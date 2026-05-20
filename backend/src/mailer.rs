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
    Resend {
        client: Client,
        base_url: String,
        api_key: String,
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
    reply_to: Option<String>,
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
struct ResendMailRequest {
    from: String,
    to: String,
    subject: String,
    text: String,
    html: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    reply_to: Option<String>,
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
            "resend" => {
                let api_key = api_key.ok_or_else(|| {
                    anyhow::anyhow!("MAIL_API_KEY is required when MAIL_PROVIDER=resend")
                })?;
                MailBackend::Resend {
                    client: Client::new(),
                    base_url: "https://api.resend.com".to_string(),
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
            reply_to: None,
            backend,
            max_retries,
            retry_backoff: Duration::from_millis(retry_backoff_ms),
            alert_backend,
        })
    }

    pub fn with_reply_to(mut self, reply_to: Option<String>) -> Self {
        self.reply_to = reply_to;
        self
    }

    pub fn with_resend_base_url(mut self, base_url: String) -> Self {
        if let MailBackend::Resend { base_url: url, .. } = &mut self.backend {
            *url = base_url;
        }
        self
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
            MailBackend::Webhook { .. } | MailBackend::Resend { .. } => {
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

    pub async fn send_ticket_access_link(
        &self,
        to_email: &str,
        access_url: &str,
        ttl_secs: i64,
    ) -> anyhow::Result<()> {
        match &self.backend {
            MailBackend::Console => {
                tracing::info!(
                    from = self.from,
                    to = to_email,
                    access_url = access_url,
                    ttl_secs,
                    "email ticket access placeholder"
                );
                Ok(())
            }
            MailBackend::Webhook { .. } | MailBackend::Resend { .. } => {
                let ttl_minutes = (ttl_secs / 60).max(1);
                let message = EmailMessage {
                    to: to_email.to_string(),
                    subject: "Access your Money Frontier tickets".to_string(),
                    text: ticket_access_text(access_url, ttl_minutes),
                    html: ticket_access_html(access_url, ttl_minutes),
                };
                self.send_email_with_retries(&message, to_email).await
            }
        }
    }

    async fn send_email_with_retries(
        &self,
        message: &EmailMessage,
        to_email: &str,
    ) -> anyhow::Result<()> {
        let mut last_error = String::new();
        let attempts = self.max_retries + 1;

        for attempt in 1..=attempts {
            match self.send_email_once(message).await {
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

    async fn send_ticket_qr_once(&self, to_email: &str, qr_payload: &str) -> anyhow::Result<()> {
        let message = EmailMessage {
            to: to_email.to_string(),
            subject: "Your transferred ticket".to_string(),
            text: format!("Your QR payload: {qr_payload}"),
            html: format!(
                "<p>Your Money Frontier ticket QR payload:</p><p><code>{}</code></p>",
                escape_html(qr_payload)
            ),
        };
        self.send_email_once(&message).await
    }

    async fn send_email_once(&self, message: &EmailMessage) -> anyhow::Result<()> {
        match &self.backend {
            MailBackend::Webhook {
                client,
                url,
                api_key,
            } => {
                let body = WebhookMailRequest {
                    from: self.from.clone(),
                    to: message.to.clone(),
                    subject: message.subject.clone(),
                    text: message.text.clone(),
                };
                Self::post_json(client, url, api_key.as_deref(), &body).await
            }
            MailBackend::Resend {
                client,
                base_url,
                api_key,
            } => {
                let body = ResendMailRequest {
                    from: self.from.clone(),
                    to: message.to.clone(),
                    subject: message.subject.clone(),
                    text: message.text.clone(),
                    html: message.html.clone(),
                    reply_to: self.reply_to.clone(),
                };
                let url = format!("{}/emails", base_url.trim_end_matches('/'));
                Self::post_json(client, &url, Some(api_key), &body).await
            }
            MailBackend::Console => Ok(()),
        }
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

#[derive(Debug)]
struct EmailMessage {
    to: String,
    subject: String,
    text: String,
    html: String,
}

fn ticket_access_text(access_url: &str, ttl_minutes: i64) -> String {
    format!(
        "Money Frontier Summit 2026\n\nOpen My Tickets: {access_url}\n\nThis secure link expires in {ttl_minutes} minutes. If you did not request this email, you can ignore it."
    )
}

fn ticket_access_html(access_url: &str, ttl_minutes: i64) -> String {
    format!(
        r#"<!doctype html>
<html>
  <body style="margin:0;background:#000;color:#fff;font-family:Arial,Helvetica,sans-serif;">
    <div style="padding:40px 24px;">
      <div style="max-width:640px;margin:0 auto;border:1px solid #242424;border-radius:18px;background:#090909;overflow:hidden;">
        <div style="padding:34px 32px 30px;">
          <div style="display:flex;align-items:center;gap:14px;margin-bottom:42px;">
            <div style="width:44px;height:44px;border-radius:10px;background:#fff;color:#000;font-weight:800;font-size:20px;line-height:44px;text-align:center;">MF</div>
            <div style="font-weight:800;font-size:22px;line-height:1.05;">Money<br/>Frontier</div>
          </div>
          <div style="color:#a855ff;font-size:13px;letter-spacing:6px;text-transform:uppercase;margin-bottom:22px;">Money Frontier Summit 2026</div>
          <h1 style="font-size:34px;line-height:1.15;margin:0 0 20px;font-weight:800;">Access your tickets</h1>
          <p style="font-size:18px;line-height:1.65;color:#d6d6d6;margin:0 0 28px;">Use this secure link to open your Money Frontier tickets. It expires in {ttl_minutes} minutes.</p>
          <a href="{access_url}" style="display:inline-block;background:#fff;color:#000;text-decoration:none;border-radius:8px;padding:16px 24px;font-weight:800;font-size:15px;">Open My Tickets</a>
        </div>
        <div style="border-top:1px solid #242424;padding:22px 32px;color:#888;font-size:14px;line-height:1.5;">Sent by Money Frontier Tickets. If you did not request this email, you can ignore it.</div>
      </div>
    </div>
  </body>
</html>"#,
        access_url = escape_html(access_url),
        ttl_minutes = ttl_minutes
    )
}

fn escape_html(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    use axum::{
        extract::State,
        http::{HeaderMap, StatusCode},
        routing::post,
        Json, Router,
    };
    use serde_json::Value;
    use tokio::sync::Mutex;

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

    #[derive(Clone)]
    struct CaptureState {
        payloads: Arc<Mutex<Vec<Value>>>,
        auth_headers: Arc<Mutex<Vec<Option<String>>>>,
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

    async fn capture_mail_handler(
        State(state): State<CaptureState>,
        headers: HeaderMap,
        Json(payload): Json<Value>,
    ) -> StatusCode {
        state.payloads.lock().await.push(payload);
        state.auth_headers.lock().await.push(
            headers
                .get(axum::http::header::AUTHORIZATION)
                .and_then(|value| value.to_str().ok())
                .map(ToOwned::to_owned),
        );
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

    #[tokio::test]
    async fn resend_sends_branded_ticket_access_email_with_reply_to_and_text_fallback() {
        let state = CaptureState {
            payloads: Arc::new(Mutex::new(Vec::new())),
            auth_headers: Arc::new(Mutex::new(Vec::new())),
        };
        let (base_url, handle) = spawn_server(
            Router::new()
                .route("/emails", post(capture_mail_handler))
                .with_state(state.clone()),
        )
        .await;

        let mailer = Mailer::new(
            "Money Frontier Tickets <tickets@moneyfrontier.info>".to_string(),
            "resend".to_string(),
            None,
            Some("resend-test-key".to_string()),
            1,
            1,
            None,
            None,
        )
        .expect("resend mailer should initialize")
        .with_resend_base_url(base_url)
        .with_reply_to(Some("contact@moneyfrontier.info".to_string()));

        mailer
            .send_ticket_access_link(
                "guest@example.com",
                "https://www.moneyfrontier.info/en/tickets/email-access?token=abc",
                900,
            )
            .await
            .expect("access email should send");

        let payloads = state.payloads.lock().await;
        assert_eq!(payloads.len(), 1);
        let payload = &payloads[0];
        assert_eq!(
            payload["from"],
            "Money Frontier Tickets <tickets@moneyfrontier.info>"
        );
        assert_eq!(payload["to"], "guest@example.com");
        assert_eq!(payload["reply_to"], "contact@moneyfrontier.info");
        assert_eq!(payload["subject"], "Access your Money Frontier tickets");
        assert!(payload["html"].as_str().unwrap().contains("Money Frontier"));
        assert!(payload["html"]
            .as_str()
            .unwrap()
            .contains("Open My Tickets"));
        assert!(payload["html"].as_str().unwrap().contains("MF"));
        assert!(payload["text"]
            .as_str()
            .unwrap()
            .contains("https://www.moneyfrontier.info/en/tickets/email-access?token=abc"));
        assert!(payload["text"].as_str().unwrap().contains("15 minutes"));

        let auth_headers = state.auth_headers.lock().await;
        assert_eq!(auth_headers[0].as_deref(), Some("Bearer resend-test-key"));

        handle.abort();
    }
}
