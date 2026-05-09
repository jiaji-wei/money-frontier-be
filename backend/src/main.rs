mod admin;
mod admin_routes;
mod auth;
mod chain;
mod cleanup;
mod config;
mod db;
mod error;
mod indexer;
mod mailer;
mod promotions;
mod routes;

use std::sync::Arc;

use axum::{routing::get, routing::post, Router};
use ethers_signers::LocalWallet;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use auth::JwtCodec;
use chain::{ChainReader, ChainService};
use config::AppConfig;
use db::Db;
use mailer::Mailer;

#[derive(Clone)]
pub struct AppState {
    pub config: AppConfig,
    pub db: Db,
    pub chain: Arc<dyn ChainReader>,
    pub jwt: JwtCodec,
    pub mailer: Mailer,
    pub purchase_signer: Option<LocalWallet>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let config = AppConfig::from_env()?;
    let db = Db::connect(&config.database_url).await?;
    let chain: Arc<dyn ChainReader> = Arc::new(ChainService::new(&config.chains)?);
    let jwt = JwtCodec::new(&config.jwt_secret, config.jwt_ttl_days)?;
    let purchase_signer = config
        .purchase_signer_private_key
        .as_deref()
        .map(str::parse::<LocalWallet>)
        .transpose()?;
    let mailer = Mailer::new(
        config.mail_from.clone(),
        config.mail_provider.clone(),
        config.mail_webhook_url.clone(),
        config.mail_api_key.clone(),
        config.mail_max_retries,
        config.mail_retry_backoff_ms,
        config.mail_alert_webhook_url.clone(),
        config.mail_alert_api_key.clone(),
    )?;

    let state = Arc::new(AppState {
        config: config.clone(),
        db,
        chain,
        jwt,
        mailer,
        purchase_signer,
    });

    indexer::spawn(state.clone());
    cleanup::spawn(state.clone());

    let app = Router::new()
        .route("/health", get(routes::health))
        .route("/signin/challenge", post(routes::signin_challenge))
        .route("/signin", post(routes::signin_verify))
        .route("/purchase-prices", post(routes::list_ticket_prices))
        .route("/purchase-quotes", post(routes::create_purchase_quote))
        .route(
            "/purchase-referral-quotes",
            post(routes::create_referral_purchase_quote),
        )
        .route("/purchase-intents", post(routes::create_purchase_intent))
        .route("/purchase-intents/:id", get(routes::get_purchase_intent))
        .nest("/admin", admin_routes::router())
        .route(
            "/tickets",
            get(routes::list_tickets).post(routes::notify_tickets),
        )
        .route(
            "/tickets/:id",
            get(routes::get_ticket).put(routes::transfer_ticket),
        )
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(config.bind_addr).await?;
    tracing::info!("backend listening");
    axum::serve(listener, app).await?;
    Ok(())
}
