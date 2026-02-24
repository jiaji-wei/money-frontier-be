use std::time::Duration;

use reqwest::Client;
use serde::Serialize;

#[derive(Clone)]
enum MailBackend {
    Console,
    Webhook {
        client: Client,
        url: String,
        api_key: Option<String>,
    },
}

#[derive(Clone)]
struct AlertBackend {
    client: Client,
    url: String,
    api_key: Option<String>,
}

#[derive(Clone)]
pub struct Mailer {
    from: String,
    backend: MailBackend,
    max_retries: u32,
    retry_backoff: Duration,
    alert_backend: Option<AlertBackend>,
}

#[derive(Debug, Serialize)]
struct WebhookMailRequest {
    from: String,
    to: String,
    subject: String,
    text: String,
}

#[derive(Debug, Serialize)]
struct WebhookAlertRequest {
    level: &'static str,
    event: &'static str,
    to: String,
    error: String,
    retries: u32,
}

impl Mailer {
    pub fn new(
        from: String,
        provider: String,
        webhook_url: Option<String>,
        api_key: Option<String>,
        max_retries: u32,
        retry_backoff_ms: u64,
        alert_webhook_url: Option<String>,
        alert_api_key: Option<String>,
    ) -> anyhow::Result<Self> {
        let backend = match provider.as_str() {
            "console" => MailBackend::Console,
            "webhook" => {
                let url = webhook_url.ok_or_else(|| {
                    anyhow::anyhow!("MAIL_WEBHOOK_URL is required when MAIL_PROVIDER=webhook")
                })?;
                MailBackend::Webhook {
                    client: Client::new(),
                    url,
                    api_key,
                }
            }
            other => anyhow::bail!("unsupported mail provider: {other}"),
        };

        let alert_backend = alert_webhook_url.map(|url| AlertBackend {
            client: Client::new(),
            url,
            api_key: alert_api_key,
        });

        Ok(Self {
            from,
            backend,
            max_retries,
            retry_backoff: Duration::from_millis(retry_backoff_ms),
            alert_backend,
        })
    }

    pub async fn send_ticket_qr(&self, to_email: &str, qr_payload: &str) -> anyhow::Result<()> {
        match &self.backend {
            MailBackend::Console => {
                tracing::info!(
                    from = self.from,
                    to = to_email,
                    qr_payload = qr_payload,
                    "email transfer placeholder"
                );
                Ok(())
            }
            MailBackend::Webhook { .. } => {
                let mut last_error = String::new();
                let attempts = self.max_retries + 1;

                for attempt in 1..=attempts {
                    match self.send_ticket_qr_once(to_email, qr_payload).await {
                        Ok(()) => return Ok(()),
                        Err(err) => {
                            last_error = err.to_string();
                            tracing::warn!(
                                to = to_email,
                                attempt,
                                attempts,
                                error = %last_error,
                                "mail dispatch failed, retrying"
                            );

                            if attempt < attempts {
                                tokio::time::sleep(self.retry_backoff).await;
                            }
                        }
                    }
                }

                self.send_alert(to_email, &last_error, self.max_retries)
                    .await;
                anyhow::bail!("mail delivery failed after {attempts} attempts: {last_error}");
            }
        }
    }

    async fn send_ticket_qr_once(&self, to_email: &str, qr_payload: &str) -> anyhow::Result<()> {
        let MailBackend::Webhook {
            client,
            url,
            api_key,
        } = &self.backend
        else {
            anyhow::bail!("send_ticket_qr_once only supports webhook backend");
        };

        let body = WebhookMailRequest {
            from: self.from.clone(),
            to: to_email.to_string(),
            subject: "Your transferred ticket".to_string(),
            text: format!("Your QR payload: {qr_payload}"),
        };
        Self::post_json(client, url, api_key.as_deref(), &body).await
    }

    async fn send_alert(&self, to_email: &str, error: &str, retries: u32) {
        let Some(alert_backend) = &self.alert_backend else {
            return;
        };

        let body = WebhookAlertRequest {
            level: "error",
            event: "ticket_email_delivery_failed",
            to: to_email.to_string(),
            error: error.to_string(),
            retries,
        };

        if let Err(alert_err) = Self::post_json(
            &alert_backend.client,
            &alert_backend.url,
            alert_backend.api_key.as_deref(),
            &body,
        )
        .await
        {
            tracing::error!(
                to = to_email,
                error = %alert_err,
                "failed to send mail failure alert"
            );
        }
    }

    async fn post_json<T: Serialize>(
        client: &Client,
        url: &str,
        api_key: Option<&str>,
        body: &T,
    ) -> anyhow::Result<()> {
        let mut req = client.post(url).json(body);
        if let Some(key) = api_key {
            req = req.header("Authorization", format!("Bearer {key}"));
        }

        let resp = req.send().await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("webhook failed with status {status}: {body}");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    use axum::{extract::State, http::StatusCode, routing::post, Router};

    use super::Mailer;

    #[derive(Clone)]
    struct FailUntilState {
        attempts: Arc<AtomicUsize>,
        fail_until: usize,
    }

    #[derive(Clone)]
    struct CounterState {
        count: Arc<AtomicUsize>,
    }

    async fn failing_mail_handler(State(state): State<FailUntilState>) -> StatusCode {
        let attempt = state.attempts.fetch_add(1, Ordering::SeqCst) + 1;
        if attempt <= state.fail_until {
            StatusCode::INTERNAL_SERVER_ERROR
        } else {
            StatusCode::OK
        }
    }

    async fn alert_handler(State(state): State<CounterState>) -> StatusCode {
        state.count.fetch_add(1, Ordering::SeqCst);
        StatusCode::OK
    }

    async fn spawn_server(router: Router) -> (String, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener should bind");
        let addr = listener.local_addr().expect("local addr should exist");
        let handle = tokio::spawn(async move {
            axum::serve(listener, router)
                .await
                .expect("server should run");
        });
        (format!("http://{addr}"), handle)
    }

    #[tokio::test]
    async fn mailer_retries_and_eventually_succeeds() {
        let attempts = Arc::new(AtomicUsize::new(0));
        let state = FailUntilState {
            attempts: attempts.clone(),
            fail_until: 2,
        };
        let (mail_url, mail_handle) = spawn_server(
            Router::new()
                .route("/", post(failing_mail_handler))
                .with_state(state),
        )
        .await;

        let mailer = Mailer::new(
            "noreply@test.local".to_string(),
            "webhook".to_string(),
            Some(mail_url),
            None,
            3,
            1,
            None,
            None,
        )
        .expect("mailer should initialize");

        let result = mailer.send_ticket_qr("user@example.com", "payload").await;
        assert!(result.is_ok());
        assert_eq!(attempts.load(Ordering::SeqCst), 3);

        mail_handle.abort();
    }

    #[tokio::test]
    async fn mailer_sends_alert_after_retry_exhaustion() {
        let attempts = Arc::new(AtomicUsize::new(0));
        let mail_state = FailUntilState {
            attempts: attempts.clone(),
            fail_until: usize::MAX,
        };
        let (mail_url, mail_handle) = spawn_server(
            Router::new()
                .route("/", post(failing_mail_handler))
                .with_state(mail_state),
        )
        .await;

        let alert_count = Arc::new(AtomicUsize::new(0));
        let (alert_url, alert_handle) = spawn_server(
            Router::new()
                .route("/", post(alert_handler))
                .with_state(CounterState {
                    count: alert_count.clone(),
                }),
        )
        .await;

        let mailer = Mailer::new(
            "noreply@test.local".to_string(),
            "webhook".to_string(),
            Some(mail_url),
            None,
            1,
            1,
            Some(alert_url),
            None,
        )
        .expect("mailer should initialize");

        let result = mailer.send_ticket_qr("user@example.com", "payload").await;
        assert!(result.is_err());
        assert_eq!(attempts.load(Ordering::SeqCst), 2);
        assert_eq!(alert_count.load(Ordering::SeqCst), 1);

        mail_handle.abort();
        alert_handle.abort();
    }
}
