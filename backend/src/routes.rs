use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::HeaderMap,
    Json,
};
use ethers_core::types::U256;
use serde::{Deserialize, Serialize};

use crate::{
    auth::{extract_wallet, normalize_wallet_address, verify_wallet_signature},
    config::{payment_token_decimals, ChainConfig},
    db::TicketRow,
    error::ApiError,
    promotions::{
        normalize_promotion_code, sign_purchase_authorization, DiscountRedemptionStatus,
        NewDiscountRedemption, NewPurchaseIntent, PromotionCodeRow, PurchaseIntentStatus,
    },
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
    pub referral_code: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ReferralBindingStatus {
    pub status: String,
    pub referral_code: String,
}

#[derive(Debug, Serialize)]
pub struct SigninResponse {
    pub wallet: String,
    pub token: String,
    pub expires_at: i64,
    pub referral_binding: Option<ReferralBindingStatus>,
}

#[derive(Debug, Deserialize)]
pub struct CreatePurchaseIntentRequest {
    pub chain_id: u64,
    pub payment_token: String,
    pub level_ids: Vec<u8>,
    pub quantities: Vec<u64>,
    pub discount_code: Option<String>,
    pub referral_code: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct CreatePurchaseIntentResponse {
    pub intent_id: String,
    pub expires_at: i64,
    pub original_total_amount: String,
    pub discount_amount: String,
    pub final_total_amount: String,
    pub signature: String,
    pub referral_binding_status: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreatePurchaseQuoteRequest {
    pub chain_id: u64,
    pub payment_token: String,
    pub level_ids: Vec<u8>,
    pub quantities: Vec<u64>,
    pub discount_code: Option<String>,
    pub referral_code: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct CreatePurchaseQuoteResponse {
    pub original_total_amount: String,
    pub discount_amount: String,
    pub final_total_amount: String,
    pub discount_status: String,
    pub discount_message: String,
    pub referral_binding_status: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct PurchaseIntentResponse {
    pub id: String,
    pub wallet_address: String,
    pub chain_id: i64,
    pub payment_token: String,
    pub level_ids_json: String,
    pub quantities_json: String,
    pub referral_code_id: Option<i64>,
    pub discount_code_id: Option<i64>,
    pub original_total_amount: String,
    pub discount_amount: String,
    pub final_total_amount: String,
    pub expires_at: i64,
    pub status: String,
    pub tx_hash: Option<String>,
    pub order_id: Option<String>,
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

    let referral_binding = match req
        .referral_code
        .as_deref()
        .and_then(normalize_promotion_code)
    {
        Some(referral_code) => {
            if state
                .db
                .get_wallet_referral_binding(&wallet)
                .await
                .map_err(|err| ApiError::internal(format!("load referral binding failed: {err}")))?
                .is_some()
            {
                Some(ReferralBindingStatus {
                    status: "already_bound".to_string(),
                    referral_code,
                })
            } else {
                let promotion_code =
                    state
                        .db
                        .find_promotion_code(&referral_code)
                        .await
                        .map_err(|err| {
                            ApiError::internal(format!("load referral code failed: {err}"))
                        })?;

                match promotion_code {
                    Some(code) if code.kind == "referral" && code.status == "active" => {
                        let bind_result = state
                            .db
                            .bind_wallet_referral_once(&wallet, code.id, "signin")
                            .await
                            .map_err(|err| {
                                ApiError::internal(format!("bind referral failed: {err}"))
                            })?;

                        Some(ReferralBindingStatus {
                            status: if bind_result.bound {
                                "bound".to_string()
                            } else {
                                "already_bound".to_string()
                            },
                            referral_code: code.code_normalized,
                        })
                    }
                    _ => Some(ReferralBindingStatus {
                        status: "invalid".to_string(),
                        referral_code,
                    }),
                }
            }
        }
        None => None,
    };

    let (token, expires_at) = state.jwt.issue(&wallet)?;

    Ok(Json(SigninResponse {
        wallet,
        token,
        expires_at,
        referral_binding,
    }))
}

pub async fn create_purchase_intent(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<CreatePurchaseIntentRequest>,
) -> Result<Json<CreatePurchaseIntentResponse>, ApiError> {
    let wallet = extract_wallet(&headers, &state.jwt)?;
    validate_purchase_intent_request(&req)?;

    let payment_token = normalize_wallet_address(&req.payment_token)?;
    let chain_config = find_chain_config(&state.config.chains, req.chain_id)?;

    let quote = state
        .chain
        .quote_purchase(req.chain_id, &req.level_ids, &req.quantities)
        .await
        .map_err(|err| ApiError::bad_request(format!("failed to quote purchase: {err}")))?;
    let original_total = parse_u256_decimal(&quote.total_amount)?;

    let (referral_code_id, referral_binding_status) =
        resolve_purchase_referral_binding(&state, &wallet, req.referral_code.as_deref()).await?;

    let (discount_code_id, discount_amount) = resolve_discount(
        &state,
        &wallet,
        req.chain_id,
        &payment_token,
        &req.level_ids,
        req.discount_code.as_deref(),
        referral_code_id.is_some(),
        original_total,
        DiscountResolutionMode::CreateIntent,
    )
    .await?;

    let final_total = original_total.saturating_sub(discount_amount);
    let expires_at = unix_now() + state.config.purchase_intent_ttl_secs;
    let signer = state
        .purchase_signer
        .as_ref()
        .ok_or_else(|| ApiError::internal("purchase signer not configured".to_string()))?;

    let purchase_intent = state
        .db
        .create_purchase_intent(NewPurchaseIntent {
            id: None,
            wallet_address: wallet.clone(),
            chain_id: req.chain_id as i64,
            payment_token: payment_token.clone(),
            level_ids_json: serde_json::to_string(&req.level_ids)
                .map_err(|err| ApiError::internal(format!("serialize level ids failed: {err}")))?,
            quantities_json: serde_json::to_string(&req.quantities)
                .map_err(|err| ApiError::internal(format!("serialize quantities failed: {err}")))?,
            referral_code_id,
            discount_code_id,
            original_total_amount: original_total.to_string(),
            discount_amount: discount_amount.to_string(),
            final_total_amount: final_total.to_string(),
            expires_at,
            status: PurchaseIntentStatus::Pending,
            tx_hash: None,
            order_id: None,
        })
        .await
        .map_err(|err| ApiError::internal(format!("create purchase intent failed: {err}")))?;

    if let Some(discount_code_id) = discount_code_id {
        state
            .db
            .reserve_discount_redemption(NewDiscountRedemption {
                purchase_intent_id: purchase_intent.id.clone(),
                discount_code_id,
                wallet_address: wallet.clone(),
                status: DiscountRedemptionStatus::Reserved,
                tx_hash: None,
                order_id: None,
                reserved_at: unix_now(),
                confirmed_at: None,
                released_at: None,
            })
            .await
            .map_err(|err| ApiError::internal(format!("reserve discount failed: {err}")))?;
    }

    let signature = sign_purchase_authorization(
        signer,
        &chain_config.sale_contract,
        req.chain_id,
        &wallet,
        &payment_token,
        &req.level_ids,
        &req.quantities,
        &purchase_intent.id,
        &purchase_intent.final_total_amount,
        expires_at,
    )
    .await
    .map_err(|err| ApiError::internal(format!("sign purchase authorization failed: {err}")))?;

    Ok(Json(CreatePurchaseIntentResponse {
        intent_id: purchase_intent.id,
        expires_at,
        original_total_amount: purchase_intent.original_total_amount,
        discount_amount: purchase_intent.discount_amount,
        final_total_amount: purchase_intent.final_total_amount,
        signature,
        referral_binding_status,
    }))
}

pub async fn create_purchase_quote(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<CreatePurchaseQuoteRequest>,
) -> Result<Json<CreatePurchaseQuoteResponse>, ApiError> {
    let wallet = extract_wallet(&headers, &state.jwt)?;
    validate_purchase_quote_request(&req)?;

    let payment_token = normalize_wallet_address(&req.payment_token)?;
    find_chain_config(&state.config.chains, req.chain_id)?;

    let quote = state
        .chain
        .quote_purchase(req.chain_id, &req.level_ids, &req.quantities)
        .await
        .map_err(|err| ApiError::bad_request(format!("failed to quote purchase: {err}")))?;
    let original_total = parse_u256_decimal(&quote.total_amount)?;

    let (referral_code_id, referral_binding_status) =
        resolve_purchase_referral_preview(&state, &wallet, req.referral_code.as_deref()).await?;

    let (discount_code_id, discount_amount) = resolve_discount(
        &state,
        &wallet,
        req.chain_id,
        &payment_token,
        &req.level_ids,
        req.discount_code.as_deref(),
        referral_code_id.is_some(),
        original_total,
        DiscountResolutionMode::Preview,
    )
    .await?;

    let final_total = original_total.saturating_sub(discount_amount);
    let discount_code_present = req
        .discount_code
        .as_deref()
        .and_then(normalize_promotion_code)
        .is_some();
    let (discount_status, discount_message) = match (discount_code_id, discount_amount.is_zero()) {
        (Some(_), false) => ("applied", "Discount applied"),
        (Some(_), true) => ("no_discount", "Discount code did not reduce this order"),
        (None, _) if discount_code_present => ("no_discount", "No discount applied"),
        (None, _) => ("none", "No discount code"),
    };

    Ok(Json(CreatePurchaseQuoteResponse {
        original_total_amount: original_total.to_string(),
        discount_amount: discount_amount.to_string(),
        final_total_amount: final_total.to_string(),
        discount_status: discount_status.to_string(),
        discount_message: discount_message.to_string(),
        referral_binding_status,
    }))
}

pub async fn get_purchase_intent(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(intent_id): Path<String>,
) -> Result<Json<PurchaseIntentResponse>, ApiError> {
    let wallet = extract_wallet(&headers, &state.jwt)?;
    let intent = state
        .db
        .get_purchase_intent(&intent_id)
        .await
        .map_err(|err| ApiError::internal(format!("load purchase intent failed: {err}")))?;

    let intent = intent.ok_or_else(|| ApiError::not_found("purchase intent not found"))?;
    if intent.wallet_address != wallet {
        return Err(ApiError::not_found("purchase intent not found"));
    }

    Ok(Json(PurchaseIntentResponse {
        id: intent.id,
        wallet_address: intent.wallet_address,
        chain_id: intent.chain_id,
        payment_token: intent.payment_token,
        level_ids_json: intent.level_ids_json,
        quantities_json: intent.quantities_json,
        referral_code_id: intent.referral_code_id,
        discount_code_id: intent.discount_code_id,
        original_total_amount: intent.original_total_amount,
        discount_amount: intent.discount_amount,
        final_total_amount: intent.final_total_amount,
        expires_at: intent.expires_at,
        status: intent.status,
        tx_hash: intent.tx_hash,
        order_id: intent.order_id,
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
    let mut matched_purchase = false;

    for purchase in &purchases {
        if purchase.buyer != wallet {
            continue;
        }
        matched_purchase = true;

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

    if !matched_purchase {
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

fn validate_purchase_intent_request(req: &CreatePurchaseIntentRequest) -> Result<(), ApiError> {
    validate_purchase_items(&req.level_ids, &req.quantities)
}

fn validate_purchase_quote_request(req: &CreatePurchaseQuoteRequest) -> Result<(), ApiError> {
    validate_purchase_items(&req.level_ids, &req.quantities)
}

fn validate_purchase_items(level_ids: &[u8], quantities: &[u64]) -> Result<(), ApiError> {
    if level_ids.is_empty() {
        return Err(ApiError::bad_request("level_ids must not be empty"));
    }
    if level_ids.len() != quantities.len() {
        return Err(ApiError::bad_request(
            "level_ids and quantities length must match",
        ));
    }
    if quantities.iter().any(|quantity| *quantity == 0) {
        return Err(ApiError::bad_request(
            "quantities must be greater than zero",
        ));
    }
    Ok(())
}

async fn resolve_purchase_referral_preview(
    state: &Arc<AppState>,
    wallet: &str,
    referral_code: Option<&str>,
) -> Result<(Option<i64>, Option<String>), ApiError> {
    if let Some(existing) = state
        .db
        .get_wallet_referral_binding(wallet)
        .await
        .map_err(|err| ApiError::internal(format!("load referral binding failed: {err}")))?
    {
        return Ok((
            Some(existing.referral_code_id),
            Some("already_bound".to_string()),
        ));
    }

    let Some(referral_code) = referral_code.and_then(normalize_promotion_code) else {
        return Ok((None, None));
    };

    let promotion_code = state
        .db
        .find_promotion_code(&referral_code)
        .await
        .map_err(|err| ApiError::internal(format!("load referral code failed: {err}")))?;

    match promotion_code {
        Some(code) if code.kind == "referral" && code.status == "active" => {
            Ok((Some(code.id), Some("would_bind".to_string())))
        }
        _ => Ok((None, Some("invalid".to_string()))),
    }
}

async fn resolve_purchase_referral_binding(
    state: &Arc<AppState>,
    wallet: &str,
    referral_code: Option<&str>,
) -> Result<(Option<i64>, Option<String>), ApiError> {
    if let Some(existing) = state
        .db
        .get_wallet_referral_binding(wallet)
        .await
        .map_err(|err| ApiError::internal(format!("load referral binding failed: {err}")))?
    {
        return Ok((
            Some(existing.referral_code_id),
            Some("already_bound".to_string()),
        ));
    }

    let Some(referral_code) = referral_code.and_then(normalize_promotion_code) else {
        return Ok((None, None));
    };

    let promotion_code = state
        .db
        .find_promotion_code(&referral_code)
        .await
        .map_err(|err| ApiError::internal(format!("load referral code failed: {err}")))?;

    match promotion_code {
        Some(code) if code.kind == "referral" && code.status == "active" => {
            let bind_result = state
                .db
                .bind_wallet_referral_once(wallet, code.id, "purchase_intent")
                .await
                .map_err(|err| ApiError::internal(format!("bind referral failed: {err}")))?;
            Ok((
                Some(bind_result.referral_code_id),
                Some(if bind_result.bound {
                    "bound".to_string()
                } else {
                    "already_bound".to_string()
                }),
            ))
        }
        _ => Ok((None, Some("invalid".to_string()))),
    }
}

#[derive(Debug, Clone, Copy)]
enum DiscountResolutionMode {
    Preview,
    CreateIntent,
}

async fn resolve_discount(
    state: &Arc<AppState>,
    wallet: &str,
    chain_id: u64,
    payment_token: &str,
    level_ids: &[u8],
    discount_code: Option<&str>,
    has_referral: bool,
    original_total: U256,
    mode: DiscountResolutionMode,
) -> Result<(Option<i64>, U256), ApiError> {
    let Some(discount_code) = discount_code.and_then(normalize_promotion_code) else {
        return Ok((None, U256::zero()));
    };

    let code = state
        .db
        .find_promotion_code(&discount_code)
        .await
        .map_err(|err| ApiError::internal(format!("load discount code failed: {err}")))?
        .ok_or_else(|| ApiError::bad_request("discount code not found"))?;

    if matches!(mode, DiscountResolutionMode::CreateIntent) {
        state
            .db
            .release_pending_discount_redemptions_for_wallet_code(code.id, wallet, unix_now())
            .await
            .map_err(|err| {
                ApiError::internal(format!(
                    "release pending discount reservations failed: {err}"
                ))
            })?;
    }

    validate_discount_code(
        state,
        wallet,
        chain_id,
        level_ids,
        has_referral,
        &code,
        mode,
    )
    .await?;
    let token_decimals =
        if code.discount_type.as_deref() == Some("fixed") || code.max_discount_amount.is_some() {
            Some(
                payment_token_decimals(chain_id, payment_token).ok_or_else(|| {
                    ApiError::bad_request("payment token decimals are not configured")
                })?,
            )
        } else {
            None
        };
    let discount_amount = calculate_discount_amount(&code, original_total, token_decimals)?;
    Ok((Some(code.id), discount_amount))
}

async fn validate_discount_code(
    state: &Arc<AppState>,
    wallet: &str,
    chain_id: u64,
    level_ids: &[u8],
    has_referral: bool,
    code: &PromotionCodeRow,
    mode: DiscountResolutionMode,
) -> Result<(), ApiError> {
    if code.kind != "discount" || code.status != "active" {
        return Err(ApiError::bad_request("discount code is not active"));
    }

    let now = unix_now();
    if code.valid_from.is_some_and(|value| value > now) {
        return Err(ApiError::bad_request("discount code is not active yet"));
    }
    if code.valid_until.is_some_and(|value| value < now) {
        return Err(ApiError::bad_request("discount code has expired"));
    }
    if has_referral
        && code
            .stacking_policy
            .as_deref()
            .is_some_and(|value| matches!(value, "exclusive" | "discount_only" | "no_referral"))
    {
        return Err(ApiError::bad_request(
            "discount code cannot be combined with referral",
        ));
    }
    let _ = (chain_id, level_ids);
    if code.first_purchase_only {
        let order_count = state
            .db
            .count_orders_by_wallet(wallet)
            .await
            .map_err(|err| ApiError::internal(format!("count wallet orders failed: {err}")))?;
        if order_count > 0 {
            return Err(ApiError::bad_request(
                "discount code is only valid for first purchase",
            ));
        }
    }
    if let Some(max_total_uses) = code.max_total_uses {
        let current_total = match mode {
            DiscountResolutionMode::Preview => {
                state.db.count_confirmed_discount_redemptions(code.id).await
            }
            DiscountResolutionMode::CreateIntent => {
                state.db.count_active_discount_redemptions(code.id).await
            }
        }
        .map_err(|err| ApiError::internal(format!("count discount redemptions failed: {err}")))?;
        if current_total >= max_total_uses {
            return Err(ApiError::bad_request("discount code usage limit reached"));
        }
    }
    if let Some(max_uses_per_wallet) = code.max_uses_per_wallet {
        let current_wallet_uses = match mode {
            DiscountResolutionMode::Preview => {
                state
                    .db
                    .count_confirmed_discount_redemptions_for_wallet(code.id, wallet)
                    .await
            }
            DiscountResolutionMode::CreateIntent => {
                state
                    .db
                    .count_active_discount_redemptions_for_wallet(code.id, wallet)
                    .await
            }
        }
        .map_err(|err| {
            ApiError::internal(format!("count wallet discount redemptions failed: {err}"))
        })?;
        if current_wallet_uses >= max_uses_per_wallet {
            return Err(ApiError::bad_request(
                "discount code wallet usage limit reached",
            ));
        }
    }
    if code.discount_type.is_none() || code.discount_value.is_none() {
        return Err(ApiError::bad_request("discount code is misconfigured"));
    }

    Ok(())
}

fn calculate_discount_amount(
    code: &PromotionCodeRow,
    original_total: U256,
    token_decimals: Option<u8>,
) -> Result<U256, ApiError> {
    let discount_type = code
        .discount_type
        .as_deref()
        .ok_or_else(|| ApiError::bad_request("discount code is misconfigured"))?;
    let discount_value = code
        .discount_value
        .as_deref()
        .ok_or_else(|| ApiError::bad_request("discount code is misconfigured"))?;

    let mut discount_amount = match discount_type {
        "fixed" => parse_token_amount(discount_value, token_decimals)?,
        "percentage" => {
            let bps = parse_u256_decimal(discount_value)?;
            original_total
                .checked_mul(bps)
                .ok_or_else(|| ApiError::bad_request("discount calculation overflow"))?
                / U256::from(10_000u64)
        }
        _ => return Err(ApiError::bad_request("unsupported discount type")),
    };

    if let Some(max_discount_amount) = code.max_discount_amount.as_deref() {
        let max_discount_amount = parse_token_amount(max_discount_amount, token_decimals)?;
        if discount_amount > max_discount_amount {
            discount_amount = max_discount_amount;
        }
    }

    if discount_amount > original_total {
        discount_amount = original_total;
    }

    Ok(discount_amount)
}

fn parse_u256_decimal(value: &str) -> Result<U256, ApiError> {
    U256::from_dec_str(value).map_err(|_| ApiError::bad_request("invalid decimal amount"))
}

fn parse_token_amount(value: &str, decimals: Option<u8>) -> Result<U256, ApiError> {
    let decimals =
        decimals.ok_or_else(|| ApiError::bad_request("payment token decimals are required"))?;
    let normalized = value.trim();
    if normalized.is_empty() || normalized.starts_with('-') {
        return Err(ApiError::bad_request("invalid decimal amount"));
    }

    let parts = normalized.split('.').collect::<Vec<_>>();
    if parts.len() > 2 {
        return Err(ApiError::bad_request("invalid decimal amount"));
    }

    let whole = if parts[0].is_empty() {
        U256::zero()
    } else {
        parse_u256_decimal(parts[0])?
    };
    let decimals_usize = decimals as usize;
    let fraction = parts.get(1).copied().unwrap_or("");
    if fraction.len() > decimals_usize || !fraction.chars().all(|ch| ch.is_ascii_digit()) {
        return Err(ApiError::bad_request("invalid decimal amount"));
    }

    let scale = U256::from(10u64).pow(U256::from(decimals));
    let padded_fraction = format!("{fraction:0<decimals_usize$}");
    let fraction_value = if padded_fraction.is_empty() {
        U256::zero()
    } else {
        parse_u256_decimal(&padded_fraction)?
    };

    whole
        .checked_mul(scale)
        .and_then(|value| value.checked_add(fraction_value))
        .ok_or_else(|| ApiError::bad_request("discount calculation overflow"))
}

fn find_chain_config<'a>(
    chains: &'a [ChainConfig],
    chain_id: u64,
) -> Result<&'a ChainConfig, ApiError> {
    chains
        .iter()
        .find(|cfg| cfg.chain_id == chain_id)
        .ok_or_else(|| ApiError::bad_request("unsupported chain_id"))
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("time should be after unix epoch")
        .as_secs() as i64
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
    use ethers_signers::{LocalWallet, Signer};
    use serde_json::{json, Value};
    use tower::util::ServiceExt;

    use super::{
        create_purchase_intent, create_purchase_quote, get_purchase_intent, get_ticket, health,
        list_tickets, notify_tickets, parse_token_amount, signin_challenge, signin_verify,
        transfer_ticket, unix_now, validate_transfer_request, TransferTicketRequest,
    };
    use crate::{
        auth::JwtCodec,
        chain::{ChainReader, ChainRuntimeConfig, DecodedPurchase, QuoteResult},
        config::{AppConfig, ChainConfig},
        db::{Db, PurchaseIntentFilters, UpdateInviteCode},
        mailer::Mailer,
        promotions::DiscountRedemptionStatus,
        AppState,
    };

    #[derive(Default)]
    struct MockChainState {
        tx_events: HashMap<(u64, String), Vec<DecodedPurchase>>,
        quotes: HashMap<(u64, Vec<u8>, Vec<u64>), QuoteResult>,
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

        fn set_quote(
            &self,
            chain_id: u64,
            level_ids: &[u8],
            quantities: &[u64],
            quote: QuoteResult,
        ) {
            let mut guard = self.state.lock().expect("lock should succeed");
            guard
                .quotes
                .insert((chain_id, level_ids.to_vec(), quantities.to_vec()), quote);
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

        async fn quote_purchase(
            &self,
            chain_id: u64,
            level_ids: &[u8],
            quantities: &[u64],
        ) -> anyhow::Result<QuoteResult> {
            let guard = self.state.lock().expect("lock should succeed");
            guard
                .quotes
                .get(&(chain_id, level_ids.to_vec(), quantities.to_vec()))
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("missing quote response"))
        }

        async fn has_default_admin_role(&self, _wallet: &str) -> anyhow::Result<bool> {
            Ok(false)
        }
    }

    fn build_test_router(state: Arc<AppState>) -> Router {
        Router::new()
            .route("/health", get(health))
            .route("/signin/challenge", post(signin_challenge))
            .route("/signin", post(signin_verify))
            .route("/purchase-quotes", post(create_purchase_quote))
            .route("/purchase-intents", post(create_purchase_intent))
            .route("/purchase-intents/:id", get(get_purchase_intent))
            .route("/tickets", get(list_tickets).post(notify_tickets))
            .route("/tickets/:id", get(get_ticket).put(transfer_ticket))
            .with_state(state)
    }

    async fn build_test_app(mock_chain: Arc<MockChain>) -> (Router, Arc<AppState>) {
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
            chains: vec![ChainConfig {
                chain_id: 56,
                rpc_url: "http://localhost:8545".to_string(),
                sale_contract: "0x0000000000000000000000000000000000005000".to_string(),
                start_block: None,
                confirmations: 0,
            }],
            indexer_poll_interval_secs: 5,
            indexer_batch_size: 50,
            indexer_reorg_rollback_blocks: 64,
            signin_challenge_ttl_secs: 300,
            signin_cleanup_interval_secs: 600,
            signin_cleanup_retention_secs: 86400,
            purchase_intent_ttl_secs: 900,
            purchase_signer_private_key: Some(
                "0x8b3a350cf5c34c9194ca3a545d4d2ce7d9f69b17a3b2ecfacac4f2d0f6f7f204".to_string(),
            ),
            admin_jwt_ttl_hours: 12,
        };

        let jwt = JwtCodec::new(&config.jwt_secret, config.jwt_ttl_days)
            .expect("jwt init should succeed");
        let purchase_signer = config
            .purchase_signer_private_key
            .as_deref()
            .map(str::parse::<LocalWallet>)
            .transpose()
            .expect("purchase signer should parse");
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
            purchase_signer,
        });

        (build_test_router(state.clone()), state)
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
        let (app, _state) = build_test_app(mock_chain.clone()).await;

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
                intent_id: None,
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

    #[tokio::test]
    async fn signin_binds_referral_only_for_unbound_wallet() {
        let mock_chain = Arc::new(MockChain::default());
        let (app, state) = build_test_app(mock_chain).await;

        let first_code_id = state
            .db
            .seed_referral_code("alice")
            .await
            .expect("first code seed should succeed");
        let second_code_id = state
            .db
            .seed_referral_code("bob")
            .await
            .expect("second code seed should succeed");

        let wallet: LocalWallet =
            "0x59c6995e998f97a5a0044966f09453880a61fdbf87f6ea0f0f8a7ecf7f5f91f7"
                .parse()
                .expect("wallet parse should succeed");
        let wallet_address = format!("{:#x}", wallet.address());

        let (status, first_challenge_body) = json_request(
            &app,
            Method::POST,
            "/signin/challenge",
            None,
            Some(json!({ "address": wallet_address })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let first_challenge_id = first_challenge_body["challenge_id"]
            .as_str()
            .expect("first challenge id should exist");
        let first_challenge_message = first_challenge_body["challenge_message"]
            .as_str()
            .expect("first challenge message should exist");

        let first_signature = wallet
            .sign_message(first_challenge_message.to_string())
            .await
            .expect("first message signing should succeed");

        let (status, first_signin_body) = json_request(
            &app,
            Method::POST,
            "/signin",
            None,
            Some(json!({
                "address": wallet_address,
                "challenge_id": first_challenge_id,
                "signature": first_signature.to_string(),
                "referral_code": "alice"
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(first_signin_body["wallet"], wallet_address);

        let (status, second_challenge_body) = json_request(
            &app,
            Method::POST,
            "/signin/challenge",
            None,
            Some(json!({ "address": wallet_address })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let second_challenge_id = second_challenge_body["challenge_id"]
            .as_str()
            .expect("second challenge id should exist");
        let second_challenge_message = second_challenge_body["challenge_message"]
            .as_str()
            .expect("second challenge message should exist");

        let second_signature = wallet
            .sign_message(second_challenge_message.to_string())
            .await
            .expect("second message signing should succeed");

        let (status, second_signin_body) = json_request(
            &app,
            Method::POST,
            "/signin",
            None,
            Some(json!({
                "address": wallet_address,
                "challenge_id": second_challenge_id,
                "signature": second_signature.to_string(),
                "referral_code": "bob"
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(second_signin_body["wallet"], wallet_address);

        let binding = state
            .db
            .get_wallet_referral_binding(&wallet_address)
            .await
            .expect("binding lookup should succeed")
            .expect("binding should exist");

        assert_eq!(binding.referral_code_id, first_code_id);
        assert_ne!(binding.referral_code_id, second_code_id);
    }

    #[tokio::test]
    async fn create_purchase_intent_reserves_discount_and_uses_bound_referral() {
        let mock_chain = Arc::new(MockChain::default());
        let (app, state) = build_test_app(mock_chain.clone()).await;

        let referral_code_id = state
            .db
            .seed_referral_code("alice")
            .await
            .expect("referral code seed should succeed");
        let discount_code_id = state
            .db
            .seed_fixed_discount_code("save50", "50")
            .await
            .expect("discount code seed should succeed");
        mock_chain.set_quote(
            56,
            &[1],
            &[2],
            QuoteResult {
                total_amount: "200000000000000000000".to_string(),
                unit_prices: vec!["100000000000000000000".to_string()],
            },
        );
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
                "signature": signature.to_string(),
                "referral_code": "alice"
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let token = signin_body["token"]
            .as_str()
            .expect("token should exist")
            .to_string();

        let (status, intent_body) = json_request(
            &app,
            Method::POST,
            "/purchase-intents",
            Some(&token),
            Some(json!({
                "chain_id": 56,
                "payment_token": "0x55d398326f99059ff775485246999027b3197955",
                "level_ids": [1],
                "quantities": [2],
                "discount_code": "save50"
            })),
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(intent_body["referral_binding_status"], "already_bound");
        assert_eq!(
            intent_body["original_total_amount"],
            "200000000000000000000"
        );
        assert_eq!(intent_body["discount_amount"], "50000000000000000000");
        assert_eq!(intent_body["final_total_amount"], "150000000000000000000");
        let intent_id = intent_body["intent_id"]
            .as_str()
            .expect("intent id should exist");
        assert!(intent_body["signature"].as_str().is_some());

        let intent = state
            .db
            .get_purchase_intent(intent_id)
            .await
            .expect("purchase intent lookup should succeed")
            .expect("purchase intent should exist");
        assert_eq!(intent.referral_code_id, Some(referral_code_id));
        assert_eq!(intent.discount_code_id, Some(discount_code_id));

        let redemption = state
            .db
            .get_discount_redemption(intent_id)
            .await
            .expect("discount redemption lookup should succeed")
            .expect("discount redemption should exist");
        assert_eq!(redemption.discount_code_id, discount_code_id);
        assert_eq!(redemption.status, "reserved");
    }

    #[tokio::test]
    async fn create_purchase_quote_previews_discount_without_creating_intent_or_reservation() {
        let mock_chain = Arc::new(MockChain::default());
        let (app, state) = build_test_app(mock_chain.clone()).await;

        let discount_code_id = state
            .db
            .seed_fixed_discount_code("preview20", "20")
            .await
            .expect("discount code seed should succeed");
        mock_chain.set_quote(
            56,
            &[1],
            &[1],
            QuoteResult {
                total_amount: "100000000000000000000".to_string(),
                unit_prices: vec!["100000000000000000000".to_string()],
            },
        );
        let wallet_address = "0x0030457e79159bed97aee6eea708441d4cff579b";
        let (token, _) = state.jwt.issue(wallet_address).expect("jwt should issue");

        let (status, body) = json_request(
            &app,
            Method::POST,
            "/purchase-quotes",
            Some(&token),
            Some(json!({
                "chain_id": 56,
                "payment_token": "0xed7b83bf2862ea0f702c76064004effcd0f4b1d5",
                "level_ids": [1],
                "quantities": [1],
                "discount_code": "preview20"
            })),
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["original_total_amount"], "100000000000000000000");
        assert_eq!(body["discount_amount"], "20000000000000000000");
        assert_eq!(body["final_total_amount"], "80000000000000000000");
        assert_eq!(body["discount_status"], "applied");
        assert!(body.get("intent_id").is_none());
        assert!(body.get("signature").is_none());

        let intents = state
            .db
            .list_purchase_intents_admin(PurchaseIntentFilters::default(), 1, 10)
            .await
            .expect("purchase intent list should succeed");
        assert!(intents.is_empty());
        let redemption_count = state
            .db
            .count_active_discount_redemptions(discount_code_id)
            .await
            .expect("redemption count should succeed");
        assert_eq!(redemption_count, 0);
    }

    #[tokio::test]
    async fn create_purchase_quote_ignores_existing_pending_reservation_for_same_wallet() {
        let mock_chain = Arc::new(MockChain::default());
        let (app, state) = build_test_app(mock_chain.clone()).await;

        let discount_code_id = state
            .db
            .seed_fixed_discount_code("retry20", "20")
            .await
            .expect("discount code seed should succeed");
        state
            .db
            .set_discount_max_uses_per_wallet(discount_code_id, 1)
            .await
            .expect("discount max usage seed should succeed");
        let wallet_address = "0x0030457e79159bed97aee6eea708441d4cff579b";
        state
            .db
            .create_purchase_intent(crate::promotions::NewPurchaseIntent {
                id: Some("retry-preview-intent".to_string()),
                wallet_address: wallet_address.to_string(),
                chain_id: 56,
                payment_token: "0xed7b83bf2862ea0f702c76064004effcd0f4b1d5".to_string(),
                level_ids_json: "[1]".to_string(),
                quantities_json: "[1]".to_string(),
                referral_code_id: None,
                discount_code_id: Some(discount_code_id),
                original_total_amount: "100000000000000000000".to_string(),
                discount_amount: "20000000000000000000".to_string(),
                final_total_amount: "80000000000000000000".to_string(),
                expires_at: unix_now() + 900,
                status: crate::promotions::PurchaseIntentStatus::Pending,
                tx_hash: None,
                order_id: None,
            })
            .await
            .expect("purchase intent seed should succeed");
        state
            .db
            .reserve_discount_redemption(crate::promotions::NewDiscountRedemption {
                purchase_intent_id: "retry-preview-intent".to_string(),
                discount_code_id,
                wallet_address: wallet_address.to_string(),
                status: DiscountRedemptionStatus::Reserved,
                tx_hash: None,
                order_id: None,
                reserved_at: unix_now(),
                confirmed_at: None,
                released_at: None,
            })
            .await
            .expect("discount reservation seed should succeed");
        mock_chain.set_quote(
            56,
            &[1],
            &[1],
            QuoteResult {
                total_amount: "100000000000000000000".to_string(),
                unit_prices: vec!["100000000000000000000".to_string()],
            },
        );
        let (token, _) = state.jwt.issue(wallet_address).expect("jwt should issue");

        let (status, body) = json_request(
            &app,
            Method::POST,
            "/purchase-quotes",
            Some(&token),
            Some(json!({
                "chain_id": 56,
                "payment_token": "0xed7b83bf2862ea0f702c76064004effcd0f4b1d5",
                "level_ids": [1],
                "quantities": [1],
                "discount_code": "retry20"
            })),
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["discount_amount"], "20000000000000000000");
    }

    #[tokio::test]
    async fn create_purchase_intent_releases_prior_pending_reservation_for_same_wallet_discount() {
        let mock_chain = Arc::new(MockChain::default());
        let (app, state) = build_test_app(mock_chain.clone()).await;

        let discount_code_id = state
            .db
            .seed_fixed_discount_code("retry30", "30")
            .await
            .expect("discount code seed should succeed");
        state
            .db
            .set_discount_max_uses_per_wallet(discount_code_id, 1)
            .await
            .expect("discount max usage seed should succeed");
        let wallet_address = "0x0030457e79159bed97aee6eea708441d4cff579b";
        state
            .db
            .create_purchase_intent(crate::promotions::NewPurchaseIntent {
                id: Some("old-retry-intent".to_string()),
                wallet_address: wallet_address.to_string(),
                chain_id: 56,
                payment_token: "0xed7b83bf2862ea0f702c76064004effcd0f4b1d5".to_string(),
                level_ids_json: "[1]".to_string(),
                quantities_json: "[1]".to_string(),
                referral_code_id: None,
                discount_code_id: Some(discount_code_id),
                original_total_amount: "100000000000000000000".to_string(),
                discount_amount: "30000000000000000000".to_string(),
                final_total_amount: "70000000000000000000".to_string(),
                expires_at: unix_now() + 900,
                status: crate::promotions::PurchaseIntentStatus::Pending,
                tx_hash: None,
                order_id: None,
            })
            .await
            .expect("purchase intent seed should succeed");
        state
            .db
            .reserve_discount_redemption(crate::promotions::NewDiscountRedemption {
                purchase_intent_id: "old-retry-intent".to_string(),
                discount_code_id,
                wallet_address: wallet_address.to_string(),
                status: DiscountRedemptionStatus::Reserved,
                tx_hash: None,
                order_id: None,
                reserved_at: unix_now(),
                confirmed_at: None,
                released_at: None,
            })
            .await
            .expect("discount reservation seed should succeed");
        mock_chain.set_quote(
            56,
            &[1],
            &[1],
            QuoteResult {
                total_amount: "100000000000000000000".to_string(),
                unit_prices: vec!["100000000000000000000".to_string()],
            },
        );
        let (token, _) = state.jwt.issue(wallet_address).expect("jwt should issue");

        let (status, body) = json_request(
            &app,
            Method::POST,
            "/purchase-intents",
            Some(&token),
            Some(json!({
                "chain_id": 56,
                "payment_token": "0xed7b83bf2862ea0f702c76064004effcd0f4b1d5",
                "level_ids": [1],
                "quantities": [1],
                "discount_code": "retry30"
            })),
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["discount_amount"], "30000000000000000000");
        let old_redemption = state
            .db
            .get_discount_redemption("old-retry-intent")
            .await
            .expect("old redemption lookup should succeed")
            .expect("old redemption should exist");
        assert_eq!(old_redemption.status, "released");
    }

    #[tokio::test]
    async fn purchase_intent_ignores_discount_chain_and_ticket_scope_for_now() {
        let mock_chain = Arc::new(MockChain::default());
        let (app, state) = build_test_app(mock_chain.clone()).await;

        let discount_code_id = state
            .db
            .seed_fixed_discount_code("scopeoff", "20")
            .await
            .expect("discount code seed should succeed");
        state
            .db
            .set_discount_scope(discount_code_id, "[1]", "[9]")
            .await
            .expect("discount scope seed should succeed");
        mock_chain.set_quote(
            56,
            &[1],
            &[1],
            QuoteResult {
                total_amount: "100000000000000000000".to_string(),
                unit_prices: vec!["100000000000000000000".to_string()],
            },
        );

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

        let (status, body) = json_request(
            &app,
            Method::POST,
            "/purchase-intents",
            Some(&token),
            Some(json!({
                "chain_id": 56,
                "payment_token": "0x55d398326f99059ff775485246999027b3197955",
                "level_ids": [1],
                "quantities": [1],
                "discount_code": "scopeoff"
            })),
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["discount_amount"], "20000000000000000000");
    }

    #[test]
    fn fixed_discount_amounts_use_payment_token_decimals() {
        assert_eq!(
            parse_token_amount("20", Some(6))
                .expect("6 decimal amount should parse")
                .to_string(),
            "20000000"
        );
        assert_eq!(
            parse_token_amount("20", Some(18))
                .expect("18 decimal amount should parse")
                .to_string(),
            "20000000000000000000"
        );
        assert_eq!(
            parse_token_amount("12.5", Some(6))
                .expect("fractional amount should parse")
                .to_string(),
            "12500000"
        );
        assert!(parse_token_amount("0.1234567", Some(6)).is_err());
    }

    #[tokio::test]
    async fn notify_tickets_confirms_discount_and_writes_snapshot_once() {
        let mock_chain = Arc::new(MockChain::default());
        let (app, state) = build_test_app(mock_chain.clone()).await;

        let referral_code_id = state
            .db
            .seed_referral_code("alice")
            .await
            .expect("referral code seed should succeed");
        state
            .db
            .update_invite_code(
                referral_code_id,
                UpdateInviteCode {
                    beneficiary_wallet: None,
                    status: None,
                    commission_type: Some("percentage".to_string()),
                    commission_value: Some("1000".to_string()),
                    valid_from: None,
                    valid_until: None,
                    notes: None,
                },
            )
            .await
            .expect("referral commission update should succeed");
        let discount_code_id = state
            .db
            .seed_fixed_discount_code("save50", "50")
            .await
            .expect("discount code seed should succeed");
        mock_chain.set_quote(
            56,
            &[1],
            &[2],
            QuoteResult {
                total_amount: "200000000000000000000".to_string(),
                unit_prices: vec!["100000000000000000000".to_string()],
            },
        );
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
                "signature": signature.to_string(),
                "referral_code": "alice"
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let token = signin_body["token"]
            .as_str()
            .expect("token should exist")
            .to_string();

        let (status, intent_body) = json_request(
            &app,
            Method::POST,
            "/purchase-intents",
            Some(&token),
            Some(json!({
                "chain_id": 56,
                "payment_token": "0x55d398326f99059ff775485246999027b3197955",
                "level_ids": [1],
                "quantities": [2],
                "discount_code": "save50"
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let intent_id = intent_body["intent_id"]
            .as_str()
            .expect("intent id should exist")
            .to_string();

        mock_chain.set_tx_events(
            56,
            "0xintenttx",
            vec![DecodedPurchase {
                tx_hash: "0xintenttx".to_string(),
                log_index: 0,
                block_number: 11,
                block_hash: Some("0xblock11".to_string()),
                order_id: "order-intent-1".to_string(),
                buyer: wallet_address.clone(),
                payment_token: "0x55d398326f99059ff775485246999027b3197955".to_string(),
                total_amount: "150000000000000000000".to_string(),
                level_ids: vec![1],
                quantities: vec![2],
                unit_prices: vec!["100000000000000000000".to_string()],
                intent_id: Some(intent_id.clone()),
            }],
        );

        let (status, notify_body) = json_request(
            &app,
            Method::POST,
            "/tickets",
            Some(&token),
            Some(json!({
                "chain_id": 56,
                "tx_hash": "0xintenttx"
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(notify_body["indexed_orders"], 1);
        assert_eq!(notify_body["created_tickets"], 2);

        let (status, second_notify_body) = json_request(
            &app,
            Method::POST,
            "/tickets",
            Some(&token),
            Some(json!({
                "chain_id": 56,
                "tx_hash": "0xintenttx"
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(second_notify_body["indexed_orders"], 0);
        assert_eq!(second_notify_body["created_tickets"], 0);

        let redemption = state
            .db
            .get_discount_redemption(&intent_id)
            .await
            .expect("discount redemption lookup should succeed")
            .expect("discount redemption should exist");
        assert_eq!(redemption.discount_code_id, discount_code_id);
        assert_eq!(redemption.status, "confirmed");

        let order_row_id = state
            .db
            .find_order_row_id(56, "0xintenttx", 0)
            .await
            .expect("order row id lookup should succeed")
            .expect("order row id should exist");
        let snapshot = state
            .db
            .get_order_promotions_snapshot(order_row_id)
            .await
            .expect("snapshot lookup should succeed")
            .expect("snapshot should exist");
        assert_eq!(snapshot.referral_code_id, Some(referral_code_id));
        assert_eq!(snapshot.discount_code_id, Some(discount_code_id));
        assert_eq!(snapshot.paid_amount, "150000000000000000000");
        assert_eq!(snapshot.discount_amount, "50000000000000000000");
        assert_eq!(snapshot.commission_base_amount, "150000000000000000000");
        assert_eq!(snapshot.commission_amount, "15000000000000000000");
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
