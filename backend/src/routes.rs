use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::HeaderMap,
    Json,
};
use serde::{Deserialize, Serialize};

use crate::{
    auth::{extract_wallet, normalize_wallet_address, verify_wallet_signature},
    db::TicketRow,
    error::ApiError,
    AppState,
};

pub async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "status": "ok" }))
}

#[derive(Debug, Deserialize)]
pub struct SigninChallengeRequest {
    pub address: String,
}

#[derive(Debug, Serialize)]
pub struct SigninChallengeResponse {
    pub challenge_id: String,
    pub challenge_message: String,
    pub expires_at: i64,
}

pub async fn signin_challenge(
    State(state): State<Arc<AppState>>,
    Json(req): Json<SigninChallengeRequest>,
) -> Result<Json<SigninChallengeResponse>, ApiError> {
    let wallet = normalize_wallet_address(&req.address)?;
    let challenge = state
        .db
        .create_signin_challenge(&wallet, state.config.signin_challenge_ttl_secs)
        .await
        .map_err(|err| ApiError::internal(format!("create challenge failed: {err}")))?;

    Ok(Json(SigninChallengeResponse {
        challenge_id: challenge.id,
        challenge_message: challenge.challenge_message,
        expires_at: challenge.expires_at,
    }))
}

#[derive(Debug, Deserialize)]
pub struct SigninVerifyRequest {
    pub address: String,
    pub challenge_id: String,
    pub signature: String,
}

#[derive(Debug, Serialize)]
pub struct SigninResponse {
    pub wallet: String,
    pub token: String,
    pub expires_at: i64,
}

pub async fn signin_verify(
    State(state): State<Arc<AppState>>,
    Json(req): Json<SigninVerifyRequest>,
) -> Result<Json<SigninResponse>, ApiError> {
    let wallet = normalize_wallet_address(&req.address)?;
    let challenge_message = state
        .db
        .get_signin_challenge_message(&req.challenge_id, &wallet)
        .await
        .map_err(|err| ApiError::internal(format!("load challenge failed: {err}")))?
        .ok_or_else(|| ApiError::unauthorized("invalid or expired challenge"))?;

    verify_wallet_signature(&wallet, &challenge_message, &req.signature)?;
    let consumed = state
        .db
        .mark_signin_challenge_used(&req.challenge_id, &wallet)
        .await
        .map_err(|err| ApiError::internal(format!("consume challenge failed: {err}")))?;
    if !consumed {
        return Err(ApiError::unauthorized("invalid or expired challenge"));
    }

    let (token, expires_at) = state.jwt.issue(&wallet)?;

    Ok(Json(SigninResponse {
        wallet,
        token,
        expires_at,
    }))
}

#[derive(Debug, Serialize)]
pub struct TicketView {
    pub id: String,
    pub chain_id: i64,
    pub order_id: String,
    pub owner_wallet: Option<String>,
    pub owner_email: Option<String>,
    pub ticket_level: i64,
    pub unit_price: String,
    pub qr_payload: String,
    pub qr_version: i64,
    pub status: String,
    pub created_at: i64,
    pub updated_at: i64,
}

impl From<TicketRow> for TicketView {
    fn from(row: TicketRow) -> Self {
        Self {
            id: row.id,
            chain_id: row.chain_id,
            order_id: row.order_id,
            owner_wallet: row.owner_wallet,
            owner_email: row.owner_email,
            ticket_level: row.ticket_level,
            unit_price: row.unit_price,
            qr_payload: row.qr_payload,
            qr_version: row.qr_version,
            status: row.status,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

pub async fn list_tickets(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<Vec<TicketView>>, ApiError> {
    let wallet = extract_wallet(&headers, &state.jwt)?;
    let tickets = state
        .db
        .list_active_tickets_by_wallet(&wallet)
        .await
        .map_err(|err| ApiError::internal(format!("query tickets failed: {err}")))?;

    Ok(Json(tickets.into_iter().map(TicketView::from).collect()))
}

#[derive(Debug, Deserialize)]
pub struct NotifyRequest {
    pub chain_id: u64,
    pub tx_hash: String,
}

#[derive(Debug, Serialize)]
pub struct NotifyResponse {
    pub indexed_orders: usize,
    pub created_tickets: usize,
}

pub async fn notify_tickets(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<NotifyRequest>,
) -> Result<Json<NotifyResponse>, ApiError> {
    let wallet = extract_wallet(&headers, &state.jwt)?;

    let purchases = state
        .chain
        .fetch_purchases(req.chain_id, &req.tx_hash)
        .await
        .map_err(|err| ApiError::bad_request(format!("failed to fetch chain events: {err}")))?;

    if purchases.is_empty() {
        return Err(ApiError::bad_request(
            "no purchase events found in transaction",
        ));
    }

    let mut indexed_orders = 0usize;
    let mut created_tickets = 0usize;

    for purchase in &purchases {
        if purchase.buyer != wallet {
            continue;
        }

        let result = state
            .db
            .index_purchase(req.chain_id, purchase)
            .await
            .map_err(|err| ApiError::internal(format!("persist purchase failed: {err}")))?;

        if result.created_order {
            indexed_orders += 1;
        }
        created_tickets += result.created_tickets;
    }

    if indexed_orders == 0 && created_tickets == 0 {
        return Err(ApiError::forbidden(
            "no matching purchase event owned by current wallet",
        ));
    }

    Ok(Json(NotifyResponse {
        indexed_orders,
        created_tickets,
    }))
}

pub async fn get_ticket(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(ticket_id): Path<String>,
) -> Result<Json<TicketView>, ApiError> {
    let wallet = extract_wallet(&headers, &state.jwt)?;
    let ticket = state
        .db
        .get_active_ticket_by_id_for_wallet(&ticket_id, &wallet)
        .await
        .map_err(|err| ApiError::internal(format!("query ticket failed: {err}")))?;

    let ticket = ticket.ok_or_else(|| ApiError::not_found("ticket not found"))?;
    Ok(Json(ticket.into()))
}

#[derive(Debug, Deserialize)]
pub struct TransferTicketRequest {
    pub to_wallet: Option<String>,
    pub to_email: Option<String>,
}

pub async fn transfer_ticket(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(ticket_id): Path<String>,
    Json(req): Json<TransferTicketRequest>,
) -> Result<Json<TicketView>, ApiError> {
    let wallet = extract_wallet(&headers, &state.jwt)?;
    validate_transfer_request(&req)?;

    let target_wallet = req
        .to_wallet
        .as_deref()
        .map(normalize_wallet_address)
        .transpose()?;

    let target_email = req.to_email.as_deref().map(normalize_email);

    let transferred = state
        .db
        .transfer_ticket(
            &ticket_id,
            &wallet,
            target_wallet.as_deref(),
            target_email.as_deref(),
        )
        .await
        .map_err(|err| ApiError::internal(format!("transfer failed: {err}")))?;

    let transferred = transferred.ok_or_else(|| ApiError::not_found("ticket not found"))?;

    if let Some(email) = target_email {
        state
            .mailer
            .send_ticket_qr(&email, &transferred.qr_payload)
            .await
            .map_err(|err| ApiError::internal(format!("email dispatch failed: {err}")))?;
    }

    Ok(Json(transferred.into()))
}

fn validate_transfer_request(req: &TransferTicketRequest) -> Result<(), ApiError> {
    match (req.to_wallet.is_some(), req.to_email.is_some()) {
        (true, false) => Ok(()),
        (false, true) => Ok(()),
        _ => Err(ApiError::bad_request(
            "exactly one of to_wallet or to_email must be provided",
        )),
    }
}

fn normalize_email(email: &str) -> String {
    email.trim().to_lowercase()
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashMap,
        sync::{Arc, Mutex},
    };

    use async_trait::async_trait;
    use axum::{
        body::{to_bytes, Body},
        http::{Method, Request, StatusCode},
        routing::{get, post},
        Router,
    };
    use ethers::signers::{LocalWallet, Signer};
    use serde_json::{json, Value};
    use tower::util::ServiceExt;

    use super::{
        get_ticket, health, list_tickets, notify_tickets, signin_challenge, signin_verify,
        transfer_ticket, validate_transfer_request, TransferTicketRequest,
    };
    use crate::{
        auth::JwtCodec,
        chain::{ChainReader, ChainRuntimeConfig, DecodedPurchase},
        config::AppConfig,
        db::Db,
        mailer::Mailer,
        AppState,
    };

    #[derive(Default)]
    struct MockChainState {
        tx_events: HashMap<(u64, String), Vec<DecodedPurchase>>,
    }

    #[derive(Default)]
    struct MockChain {
        state: Mutex<MockChainState>,
    }

    impl MockChain {
        fn set_tx_events(&self, chain_id: u64, tx_hash: &str, events: Vec<DecodedPurchase>) {
            let mut guard = self.state.lock().expect("lock should succeed");
            guard
                .tx_events
                .insert((chain_id, tx_hash.to_string()), events);
        }
    }

    #[async_trait]
    impl ChainReader for MockChain {
        fn runtime_configs(&self) -> Vec<ChainRuntimeConfig> {
            Vec::new()
        }

        async fn latest_finalized_block(&self, _chain_id: u64) -> anyhow::Result<u64> {
            Ok(0)
        }

        async fn block_hash(
            &self,
            _chain_id: u64,
            _block_number: u64,
        ) -> anyhow::Result<Option<String>> {
            Ok(None)
        }

        async fn fetch_purchases(
            &self,
            chain_id: u64,
            tx_hash: &str,
        ) -> anyhow::Result<Vec<DecodedPurchase>> {
            let guard = self.state.lock().expect("lock should succeed");
            Ok(guard
                .tx_events
                .get(&(chain_id, tx_hash.to_string()))
                .cloned()
                .unwrap_or_default())
        }

        async fn fetch_purchases_by_block_range(
            &self,
            _chain_id: u64,
            _from_block: u64,
            _to_block: u64,
        ) -> anyhow::Result<Vec<DecodedPurchase>> {
            Ok(Vec::new())
        }
    }

    async fn build_test_app(mock_chain: Arc<MockChain>) -> Router {
        let database_url = "sqlite::memory:".to_string();
        let db = Db::connect(&database_url)
            .await
            .expect("db connect should succeed");

        let config = AppConfig {
            bind_addr: "127.0.0.1:0".parse().expect("valid addr"),
            database_url,
            jwt_secret: "test-secret".to_string(),
            jwt_ttl_days: 3650,
            mail_from: "noreply@test.local".to_string(),
            mail_provider: "console".to_string(),
            mail_webhook_url: None,
            mail_api_key: None,
            mail_max_retries: 3,
            mail_retry_backoff_ms: 1,
            mail_alert_webhook_url: None,
            mail_alert_api_key: None,
            chains: Vec::new(),
            indexer_poll_interval_secs: 5,
            indexer_batch_size: 50,
            indexer_reorg_rollback_blocks: 64,
            signin_challenge_ttl_secs: 300,
            signin_cleanup_interval_secs: 600,
            signin_cleanup_retention_secs: 86400,
        };

        let jwt = JwtCodec::new(&config.jwt_secret, config.jwt_ttl_days)
            .expect("jwt init should succeed");
        let mailer = Mailer::new(
            config.mail_from.clone(),
            config.mail_provider.clone(),
            config.mail_webhook_url.clone(),
            config.mail_api_key.clone(),
            config.mail_max_retries,
            config.mail_retry_backoff_ms,
            config.mail_alert_webhook_url.clone(),
            config.mail_alert_api_key.clone(),
        )
        .expect("mailer init should succeed");

        let state = Arc::new(AppState {
            config,
            db,
            chain: mock_chain as Arc<dyn ChainReader>,
            jwt,
            mailer,
        });

        Router::new()
            .route("/health", get(health))
            .route("/signin/challenge", post(signin_challenge))
            .route("/signin", post(signin_verify))
            .route("/tickets", get(list_tickets).post(notify_tickets))
            .route("/tickets/:id", get(get_ticket).put(transfer_ticket))
            .with_state(state)
    }

    async fn json_request(
        app: &Router,
        method: Method,
        path: &str,
        bearer_token: Option<&str>,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let mut req_builder = Request::builder()
            .method(method)
            .uri(path)
            .header("content-type", "application/json");

        if let Some(token) = bearer_token {
            req_builder = req_builder.header("authorization", format!("Bearer {token}"));
        }

        let req_body = body
            .map(|value| Body::from(value.to_string()))
            .unwrap_or_else(Body::empty);

        let req = req_builder.body(req_body).expect("request should build");
        let response = app
            .clone()
            .oneshot(req)
            .await
            .expect("request should succeed");
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("response body should read");
        if bytes.is_empty() {
            return (status, Value::Null);
        }
        let json_body = serde_json::from_slice(&bytes).expect("json response expected");
        (status, json_body)
    }

    #[tokio::test]
    async fn e2e_signin_notify_and_transfer_flow() {
        let mock_chain = Arc::new(MockChain::default());
        let app = build_test_app(mock_chain.clone()).await;

        let wallet: LocalWallet =
            "0x59c6995e998f97a5a0044966f09453880a61fdbf87f6ea0f0f8a7ecf7f5f91f7"
                .parse()
                .expect("wallet parse should succeed");
        let wallet_address = format!("{:#x}", wallet.address());

        let (status, challenge_body) = json_request(
            &app,
            Method::POST,
            "/signin/challenge",
            None,
            Some(json!({ "address": wallet_address })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let challenge_id = challenge_body["challenge_id"]
            .as_str()
            .expect("challenge id should exist");
        let challenge_message = challenge_body["challenge_message"]
            .as_str()
            .expect("challenge message should exist");

        let signature = wallet
            .sign_message(challenge_message.to_string())
            .await
            .expect("message signing should succeed");

        let (status, signin_body) = json_request(
            &app,
            Method::POST,
            "/signin",
            None,
            Some(json!({
                "address": wallet_address,
                "challenge_id": challenge_id,
                "signature": signature.to_string()
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let token = signin_body["token"]
            .as_str()
            .expect("token should exist")
            .to_string();

        let (status, _) = json_request(
            &app,
            Method::POST,
            "/signin",
            None,
            Some(json!({
                "address": wallet_address,
                "challenge_id": challenge_id,
                "signature": signature.to_string()
            })),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);

        mock_chain.set_tx_events(
            11155111,
            "0xabc",
            vec![DecodedPurchase {
                tx_hash: "0xabc".to_string(),
                log_index: 0,
                block_number: 10,
                block_hash: Some("0xblocka".to_string()),
                order_id: "order-1".to_string(),
                buyer: wallet_address.clone(),
                payment_token: "0x0000000000000000000000000000000000001002".to_string(),
                total_amount: "200000000".to_string(),
                level_ids: vec![1],
                quantities: vec![2],
                unit_prices: vec!["100000000".to_string()],
            }],
        );

        let (status, notify_body) = json_request(
            &app,
            Method::POST,
            "/tickets",
            Some(&token),
            Some(json!({
                "chain_id": 11155111,
                "tx_hash": "0xabc"
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(notify_body["indexed_orders"], 1);
        assert_eq!(notify_body["created_tickets"], 2);

        let (status, list_body) =
            json_request(&app, Method::GET, "/tickets", Some(&token), None).await;
        assert_eq!(status, StatusCode::OK);
        let tickets = list_body.as_array().expect("ticket list expected");
        assert_eq!(tickets.len(), 2);
        let transfer_ticket_id = tickets[0]["id"]
            .as_str()
            .expect("ticket id should exist")
            .to_string();

        let (status, transfer_body) = json_request(
            &app,
            Method::PUT,
            &format!("/tickets/{transfer_ticket_id}"),
            Some(&token),
            Some(json!({ "to_email": "Receiver@Example.com" })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            transfer_body["owner_email"]
                .as_str()
                .expect("owner email should exist"),
            "receiver@example.com"
        );

        let (status, list_after_transfer) =
            json_request(&app, Method::GET, "/tickets", Some(&token), None).await;
        assert_eq!(status, StatusCode::OK);
        let remaining = list_after_transfer
            .as_array()
            .expect("ticket list expected");
        assert_eq!(remaining.len(), 1);
    }

    #[test]
    fn transfer_request_requires_exactly_one_target() {
        let only_wallet = TransferTicketRequest {
            to_wallet: Some("0x0000000000000000000000000000000000000001".to_string()),
            to_email: None,
        };
        assert!(validate_transfer_request(&only_wallet).is_ok());

        let only_email = TransferTicketRequest {
            to_wallet: None,
            to_email: Some("a@example.com".to_string()),
        };
        assert!(validate_transfer_request(&only_email).is_ok());

        let both = TransferTicketRequest {
            to_wallet: Some("0x0000000000000000000000000000000000000001".to_string()),
            to_email: Some("a@example.com".to_string()),
        };
        assert!(validate_transfer_request(&both).is_err());

        let none = TransferTicketRequest {
            to_wallet: None,
            to_email: None,
        };
        assert!(validate_transfer_request(&none).is_err());
    }
}
