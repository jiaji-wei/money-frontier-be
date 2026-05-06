use std::{env, net::SocketAddr};

use serde::Deserialize;

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

fn default_signin_cleanup_interval_secs() -> u64 {
    600
}

fn default_signin_cleanup_retention_secs() -> i64 {
    86400
}

fn default_purchase_intent_ttl_secs() -> i64 {
    900
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
    pub mail_provider: String,
    pub mail_webhook_url: Option<String>,
    pub mail_api_key: Option<String>,
    pub mail_max_retries: u32,
    pub mail_retry_backoff_ms: u64,
    pub mail_alert_webhook_url: Option<String>,
    pub mail_alert_api_key: Option<String>,
    pub chains: Vec<ChainConfig>,
    pub indexer_poll_interval_secs: u64,
    pub indexer_batch_size: u64,
    pub indexer_reorg_rollback_blocks: u64,
    pub signin_challenge_ttl_secs: i64,
    pub signin_cleanup_interval_secs: u64,
    pub signin_cleanup_retention_secs: i64,
    pub purchase_intent_ttl_secs: i64,
    pub purchase_signer_private_key: Option<String>,
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

        Ok(Self {
            bind_addr,
            database_url,
            jwt_secret,
            jwt_ttl_days,
            mail_from,
            mail_provider,
            mail_webhook_url,
            mail_api_key,
            mail_max_retries,
            mail_retry_backoff_ms,
            mail_alert_webhook_url,
            mail_alert_api_key,
            chains,
            indexer_poll_interval_secs,
            indexer_batch_size,
            indexer_reorg_rollback_blocks,
            signin_challenge_ttl_secs,
            signin_cleanup_interval_secs,
            signin_cleanup_retention_secs,
            purchase_intent_ttl_secs,
            purchase_signer_private_key,
        })
    }
}
