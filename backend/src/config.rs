use std::{env, net::SocketAddr};

use serde::Deserialize;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payment_token_decimals_are_configured_by_chain_and_token() {
        assert_eq!(
            payment_token_decimals(1, "0xdac17f958d2ee523a2206206994597c13d831ec7"),
            Some(6)
        );
        assert_eq!(
            payment_token_decimals(1, "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48"),
            Some(6)
        );
        assert_eq!(
            payment_token_decimals(56, "0x55d398326f99059ff775485246999027b3197955"),
            Some(18)
        );
        assert_eq!(
            payment_token_decimals(56, "0x8ac76a51cc950d9822d68b83fe1ad97b32cd580d"),
            Some(18)
        );
        assert_eq!(
            payment_token_decimals(56, "0xed7b83bf2862ea0f702c76064004effcd0f4b1d5"),
            Some(18)
        );
        assert_eq!(
            payment_token_decimals(56, "0xfdd9796a8ad4fa1615350e62a1a736382a005677"),
            Some(18)
        );
    }
}

fn default_confirmations() -> u64 {
    0
}

fn default_indexer_poll_interval_secs() -> u64 {
    5
}

fn default_indexer_batch_size() -> u64 {
    200
}

fn default_signin_challenge_ttl_secs() -> i64 {
    300
}

fn default_indexer_reorg_rollback_blocks() -> u64 {
    128
}

fn default_mail_max_retries() -> u32 {
    3
}

fn default_mail_retry_backoff_ms() -> u64 {
    300
}

fn default_email_access_token_ttl_secs() -> i64 {
    900
}

fn default_email_session_ttl_hours() -> i64 {
    24
}

fn default_stripe_api_version() -> String {
    "2026-04-22.dahlia".to_string()
}

fn default_stripe_currency() -> String {
    "usd".to_string()
}

fn default_stripe_api_base_url() -> String {
    "https://api.stripe.com".to_string()
}

fn default_fiat_checkout_session_ttl_secs() -> i64 {
    1800
}

fn default_signin_cleanup_interval_secs() -> u64 {
    600
}

fn default_signin_cleanup_retention_secs() -> i64 {
    86400
}

fn default_purchase_intent_ttl_secs() -> i64 {
    900
}

fn default_admin_jwt_ttl_hours() -> i64 {
    12
}

pub fn payment_token_decimals(chain_id: u64, token: &str) -> Option<u8> {
    let token = token.trim().to_ascii_lowercase();
    match (chain_id, token.as_str()) {
        (1, "0xdac17f958d2ee523a2206206994597c13d831ec7") => Some(6),
        (1, "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48") => Some(6),
        (56, "0x55d398326f99059ff775485246999027b3197955") => Some(18),
        (56, "0x8ac76a51cc950d9822d68b83fe1ad97b32cd580d") => Some(18),
        (56, "0xed7b83bf2862ea0f702c76064004effcd0f4b1d5") => Some(18),
        (56, "0xfdd9796a8ad4fa1615350e62a1a736382a005677") => Some(18),
        _ => None,
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ChainConfig {
    pub chain_id: u64,
    pub rpc_url: String,
    pub sale_contract: String,
    #[serde(default)]
    pub start_block: Option<u64>,
    #[serde(default = "default_confirmations")]
    pub confirmations: u64,
}

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub bind_addr: SocketAddr,
    pub database_url: String,
    pub jwt_secret: String,
    pub jwt_ttl_days: i64,
    pub mail_from: String,
    pub mail_reply_to: Option<String>,
    pub mail_provider: String,
    pub mail_webhook_url: Option<String>,
    pub mail_api_key: Option<String>,
    pub mail_max_retries: u32,
    pub mail_retry_backoff_ms: u64,
    pub mail_alert_webhook_url: Option<String>,
    pub mail_alert_api_key: Option<String>,
    pub app_public_base_url: String,
    pub email_access_token_ttl_secs: i64,
    pub email_session_ttl_hours: i64,
    pub stripe_enabled: bool,
    pub stripe_api_key: Option<String>,
    pub stripe_webhook_secret: Option<String>,
    pub stripe_api_version: String,
    pub stripe_currency: String,
    pub stripe_success_url: String,
    pub stripe_cancel_url: String,
    pub stripe_api_base_url: String,
    pub fiat_price_chain_id: u64,
    pub fiat_price_payment_token: String,
    pub fiat_checkout_session_ttl_secs: i64,
    pub chains: Vec<ChainConfig>,
    pub indexer_poll_interval_secs: u64,
    pub indexer_batch_size: u64,
    pub indexer_reorg_rollback_blocks: u64,
    pub signin_challenge_ttl_secs: i64,
    pub signin_cleanup_interval_secs: u64,
    pub signin_cleanup_retention_secs: i64,
    pub purchase_intent_ttl_secs: i64,
    pub purchase_signer_private_key: Option<String>,
    pub admin_jwt_ttl_hours: i64,
}

impl AppConfig {
    pub fn from_env() -> anyhow::Result<Self> {
        let bind_addr: SocketAddr = env::var("BIND_ADDR")
            .unwrap_or_else(|_| "0.0.0.0:8080".to_string())
            .parse()?;

        let database_url =
            env::var("DATABASE_URL").unwrap_or_else(|_| "sqlite://ticket.db".to_string());
        let jwt_secret =
            env::var("JWT_SECRET").unwrap_or_else(|_| "change-me-in-production".to_string());

        let jwt_ttl_days = env::var("JWT_TTL_DAYS")
            .ok()
            .and_then(|raw| raw.parse::<i64>().ok())
            .unwrap_or(3650);

        let mail_from =
            env::var("MAIL_FROM").unwrap_or_else(|_| "noreply@tickets.local".to_string());
        let mail_reply_to = env::var("MAIL_REPLY_TO").ok();
        let mail_provider = env::var("MAIL_PROVIDER").unwrap_or_else(|_| "console".to_string());
        let mail_webhook_url = env::var("MAIL_WEBHOOK_URL").ok();
        let mail_api_key = env::var("MAIL_API_KEY").ok();
        let mail_max_retries = env::var("MAIL_MAX_RETRIES")
            .ok()
            .and_then(|raw| raw.parse::<u32>().ok())
            .unwrap_or_else(default_mail_max_retries);
        let mail_retry_backoff_ms = env::var("MAIL_RETRY_BACKOFF_MS")
            .ok()
            .and_then(|raw| raw.parse::<u64>().ok())
            .unwrap_or_else(default_mail_retry_backoff_ms);
        let mail_alert_webhook_url = env::var("MAIL_ALERT_WEBHOOK_URL").ok();
        let mail_alert_api_key = env::var("MAIL_ALERT_API_KEY").ok();
        let app_public_base_url =
            env::var("APP_PUBLIC_BASE_URL").unwrap_or_else(|_| "http://127.0.0.1:3000".to_string());
        let email_access_token_ttl_secs = env::var("EMAIL_ACCESS_TOKEN_TTL_SECS")
            .ok()
            .and_then(|raw| raw.parse::<i64>().ok())
            .unwrap_or_else(default_email_access_token_ttl_secs);
        let email_session_ttl_hours = env::var("EMAIL_SESSION_TTL_HOURS")
            .ok()
            .and_then(|raw| raw.parse::<i64>().ok())
            .unwrap_or_else(default_email_session_ttl_hours);
        let stripe_enabled = env::var("STRIPE_ENABLED")
            .ok()
            .is_some_and(|raw| matches!(raw.as_str(), "true" | "1" | "yes"));
        let stripe_api_key = env::var("STRIPE_API_KEY").ok();
        let stripe_webhook_secret = env::var("STRIPE_WEBHOOK_SECRET").ok();
        let stripe_api_version =
            env::var("STRIPE_API_VERSION").unwrap_or_else(|_| default_stripe_api_version());
        let stripe_currency =
            env::var("STRIPE_CURRENCY").unwrap_or_else(|_| default_stripe_currency());
        let stripe_success_url = env::var("STRIPE_SUCCESS_URL").unwrap_or_else(|_| {
            "http://127.0.0.1:3000/en/tickets/checkout/success?session_id={CHECKOUT_SESSION_ID}"
                .to_string()
        });
        let stripe_cancel_url = env::var("STRIPE_CANCEL_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:3000/en/tickets/checkout/cancelled".to_string());
        let stripe_api_base_url =
            env::var("STRIPE_API_BASE_URL").unwrap_or_else(|_| default_stripe_api_base_url());
        let fiat_price_chain_id = env::var("FIAT_PRICE_CHAIN_ID")
            .ok()
            .and_then(|raw| raw.parse::<u64>().ok())
            .unwrap_or(56);
        let fiat_price_payment_token = env::var("FIAT_PRICE_PAYMENT_TOKEN")
            .unwrap_or_else(|_| "0x55d398326f99059ff775485246999027b3197955".to_string());
        let fiat_checkout_session_ttl_secs = env::var("FIAT_CHECKOUT_SESSION_TTL_SECS")
            .ok()
            .and_then(|raw| raw.parse::<i64>().ok())
            .unwrap_or_else(default_fiat_checkout_session_ttl_secs);

        let chains_json = env::var("APP_CHAINS_JSON").unwrap_or_else(|_| "[]".to_string());
        let chains: Vec<ChainConfig> = serde_json::from_str(&chains_json)?;

        let indexer_poll_interval_secs = env::var("INDEXER_POLL_INTERVAL_SECS")
            .ok()
            .and_then(|raw| raw.parse::<u64>().ok())
            .unwrap_or_else(default_indexer_poll_interval_secs);

        let indexer_batch_size = env::var("INDEXER_BATCH_SIZE")
            .ok()
            .and_then(|raw| raw.parse::<u64>().ok())
            .unwrap_or_else(default_indexer_batch_size);

        let indexer_reorg_rollback_blocks = env::var("INDEXER_REORG_ROLLBACK_BLOCKS")
            .ok()
            .and_then(|raw| raw.parse::<u64>().ok())
            .unwrap_or_else(default_indexer_reorg_rollback_blocks);

        let signin_challenge_ttl_secs = env::var("SIGNIN_CHALLENGE_TTL_SECS")
            .ok()
            .and_then(|raw| raw.parse::<i64>().ok())
            .unwrap_or_else(default_signin_challenge_ttl_secs);
        let signin_cleanup_interval_secs = env::var("SIGNIN_CLEANUP_INTERVAL_SECS")
            .ok()
            .and_then(|raw| raw.parse::<u64>().ok())
            .unwrap_or_else(default_signin_cleanup_interval_secs);
        let signin_cleanup_retention_secs = env::var("SIGNIN_CLEANUP_RETENTION_SECS")
            .ok()
            .and_then(|raw| raw.parse::<i64>().ok())
            .unwrap_or_else(default_signin_cleanup_retention_secs);
        let purchase_intent_ttl_secs = env::var("PURCHASE_INTENT_TTL_SECS")
            .ok()
            .and_then(|raw| raw.parse::<i64>().ok())
            .unwrap_or_else(default_purchase_intent_ttl_secs);
        let purchase_signer_private_key = env::var("PURCHASE_SIGNER_PRIVATE_KEY").ok();
        let admin_jwt_ttl_hours = env::var("ADMIN_JWT_TTL_HOURS")
            .ok()
            .and_then(|raw| raw.parse::<i64>().ok())
            .unwrap_or_else(default_admin_jwt_ttl_hours);

        Ok(Self {
            bind_addr,
            database_url,
            jwt_secret,
            jwt_ttl_days,
            mail_from,
            mail_reply_to,
            mail_provider,
            mail_webhook_url,
            mail_api_key,
            mail_max_retries,
            mail_retry_backoff_ms,
            mail_alert_webhook_url,
            mail_alert_api_key,
            app_public_base_url,
            email_access_token_ttl_secs,
            email_session_ttl_hours,
            stripe_enabled,
            stripe_api_key,
            stripe_webhook_secret,
            stripe_api_version,
            stripe_currency,
            stripe_success_url,
            stripe_cancel_url,
            stripe_api_base_url,
            fiat_price_chain_id,
            fiat_price_payment_token,
            fiat_checkout_session_ttl_secs,
            chains,
            indexer_poll_interval_secs,
            indexer_batch_size,
            indexer_reorg_rollback_blocks,
            signin_challenge_ttl_secs,
            signin_cleanup_interval_secs,
            signin_cleanup_retention_secs,
            purchase_intent_ttl_secs,
            purchase_signer_private_key,
            admin_jwt_ttl_hours,
        })
    }
}
