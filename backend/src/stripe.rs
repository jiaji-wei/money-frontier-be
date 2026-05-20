use hmac::{Hmac, Mac};
use reqwest::Client;
use serde::Deserialize;
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

#[derive(Debug, Clone)]
pub struct StripeCheckoutLineItem {
    pub name: String,
    pub unit_amount: i64,
    pub quantity: i64,
}

#[derive(Debug, Clone)]
pub struct CreateCheckoutSession {
    pub success_url: String,
    pub cancel_url: String,
    pub currency: String,
    pub customer_email: String,
    pub client_reference_id: String,
    pub metadata: Vec<(String, String)>,
    pub line_items: Vec<StripeCheckoutLineItem>,
    pub expires_at: i64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct StripeCheckoutSession {
    pub id: String,
    pub url: String,
}

#[derive(Debug, Clone)]
pub struct StripeClient {
    client: Client,
    api_key: String,
    api_version: String,
    base_url: String,
}

impl StripeClient {
    pub fn new(api_key: String, api_version: String, base_url: String) -> Self {
        Self {
            client: Client::new(),
            api_key,
            api_version,
            base_url,
        }
    }

    pub async fn create_checkout_session(
        &self,
        input: &CreateCheckoutSession,
    ) -> anyhow::Result<StripeCheckoutSession> {
        let mut form = vec![
            ("mode".to_string(), "payment".to_string()),
            ("success_url".to_string(), input.success_url.clone()),
            ("cancel_url".to_string(), input.cancel_url.clone()),
            ("customer_email".to_string(), input.customer_email.clone()),
            (
                "client_reference_id".to_string(),
                input.client_reference_id.clone(),
            ),
            ("expires_at".to_string(), input.expires_at.to_string()),
        ];

        for (key, value) in &input.metadata {
            form.push((format!("metadata[{key}]"), value.clone()));
        }

        for (index, item) in input.line_items.iter().enumerate() {
            form.push((
                format!("line_items[{index}][price_data][currency]"),
                input.currency.clone(),
            ));
            form.push((
                format!("line_items[{index}][price_data][unit_amount]"),
                item.unit_amount.to_string(),
            ));
            form.push((
                format!("line_items[{index}][price_data][product_data][name]"),
                item.name.clone(),
            ));
            form.push((
                format!("line_items[{index}][quantity]"),
                item.quantity.to_string(),
            ));
        }

        let url = format!(
            "{}/v1/checkout/sessions",
            self.base_url.trim_end_matches('/')
        );
        let response = self
            .client
            .post(url)
            .basic_auth(&self.api_key, Some(""))
            .header("Stripe-Version", &self.api_version)
            .form(&form)
            .send()
            .await?;

        let status = response.status();
        let body = response.text().await?;
        if !status.is_success() {
            anyhow::bail!("stripe checkout session failed with status {status}: {body}");
        }

        Ok(serde_json::from_str(&body)?)
    }
}

pub fn verify_webhook_signature(
    payload: &[u8],
    signature_header: &str,
    secret: &str,
    tolerance_secs: i64,
    now_ts: i64,
) -> anyhow::Result<()> {
    let mut timestamp = None;
    let mut signatures = Vec::new();
    for part in signature_header.split(',') {
        if let Some(raw) = part.strip_prefix("t=") {
            timestamp = Some(raw.parse::<i64>()?);
        } else if let Some(raw) = part.strip_prefix("v1=") {
            signatures.push(raw.to_string());
        }
    }

    let timestamp = timestamp.ok_or_else(|| anyhow::anyhow!("missing stripe timestamp"))?;
    if (now_ts - timestamp).abs() > tolerance_secs {
        anyhow::bail!("stripe webhook timestamp outside tolerance");
    }

    let signed_payload = format!("{}.", timestamp);
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes())?;
    mac.update(signed_payload.as_bytes());
    mac.update(payload);
    let expected = hex_lower(&mac.finalize().into_bytes());

    if signatures
        .iter()
        .any(|candidate| constant_time_eq(candidate, &expected))
    {
        return Ok(());
    }

    anyhow::bail!("invalid stripe webhook signature")
}

fn constant_time_eq(left: &str, right: &str) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.bytes()
        .zip(right.bytes())
        .fold(0u8, |acc, (a, b)| acc | (a ^ b))
        == 0
}

fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}
