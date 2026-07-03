use std::sync::Arc;

use axum::{
    body::Bytes,
    extract::{Path, State},
    http::HeaderMap,
    Json,
};
use ethers_core::types::U256;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::stripe::{CreateCheckoutSession, StripeCheckoutLineItem};
use crate::{
    auth::{
        extract_email_session, extract_wallet, normalize_wallet_address, verify_wallet_signature,
    },
    config::{payment_token_decimals, ChainConfig},
    db::{FiatCheckoutSessionRow, NewFiatCheckoutSession, TicketRow},
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
pub struct EmailAccessChallengeRequest {
    pub email: String,
}

#[derive(Debug, Serialize)]
pub struct EmailAccessChallengeResponse {
    pub email: String,
    pub expires_at: i64,
}

#[derive(Debug, Deserialize)]
pub struct EmailAccessVerifyRequest {
    pub token: String,
}

#[derive(Debug, Serialize)]
pub struct EmailAccessVerifyResponse {
    pub email: String,
    pub token: String,
    pub expires_at: i64,
}

#[derive(Debug, Deserialize)]
pub struct CreateFiatCheckoutSessionRequest {
    pub email: String,
    pub level_ids: Vec<u8>,
    pub quantities: Vec<u64>,
    pub discount_code: Option<String>,
    pub referral_code: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct CreateFiatCheckoutSessionResponse {
    pub checkout_id: String,
    pub stripe_session_id: String,
    pub url: String,
    pub original_amount_cents: i64,
    pub discount_amount_cents: i64,
    pub final_amount_cents: i64,
    pub currency: String,
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

#[derive(Debug, Deserialize)]
pub struct TicketPricesRequest {
    pub chain_id: u64,
    pub level_ids: Vec<u8>,
}

#[derive(Debug, Serialize)]
pub struct TicketPriceView {
    pub level_id: u8,
    pub unit_price: String,
}

#[derive(Debug, Serialize)]
pub struct TicketPricesResponse {
    pub chain_id: u64,
    pub prices: Vec<TicketPriceView>,
}

#[derive(Debug, Deserialize)]
pub struct RedeemRedemptionCodeRequest {
    pub code: String,
}

#[derive(Debug, Deserialize)]
pub struct RedeemRedemptionCodeByEmailRequest {
    pub code: String,
    pub email: String,
}

#[derive(Debug, Serialize)]
pub struct RedeemRedemptionCodeResponse {
    pub code: String,
    pub claim_id: String,
    pub ticket: TicketView,
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

pub async fn email_access_challenge(
    State(state): State<Arc<AppState>>,
    Json(req): Json<EmailAccessChallengeRequest>,
) -> Result<Json<EmailAccessChallengeResponse>, ApiError> {
    let email = normalize_email_checked(&req.email)?;
    let challenge = state
        .db
        .create_email_access_challenge(&email, state.config.email_access_token_ttl_secs)
        .await
        .map_err(|err| {
            ApiError::internal(format!("create email access challenge failed: {err}"))
        })?;

    let access_url = build_email_access_url(&state.config.app_public_base_url, &challenge.token);
    state
        .mailer
        .send_ticket_access_link(
            &email,
            &access_url,
            state.config.email_access_token_ttl_secs,
        )
        .await
        .map_err(|err| ApiError::internal(format!("email access dispatch failed: {err}")))?;

    Ok(Json(EmailAccessChallengeResponse {
        email,
        expires_at: challenge.expires_at,
    }))
}

pub async fn email_access_verify(
    State(state): State<Arc<AppState>>,
    Json(req): Json<EmailAccessVerifyRequest>,
) -> Result<Json<EmailAccessVerifyResponse>, ApiError> {
    let consumed = state
        .db
        .consume_email_access_challenge(&req.token)
        .await
        .map_err(|err| ApiError::internal(format!("consume email access challenge failed: {err}")))?
        .ok_or_else(|| ApiError::unauthorized("invalid or expired email access token"))?;

    let (token, expires_at) = state
        .jwt
        .issue_email_session(&consumed.email, state.config.email_session_ttl_hours)?;

    Ok(Json(EmailAccessVerifyResponse {
        email: consumed.email,
        token,
        expires_at,
    }))
}

pub async fn create_fiat_checkout_session(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreateFiatCheckoutSessionRequest>,
) -> Result<Json<CreateFiatCheckoutSessionResponse>, ApiError> {
    if !state.config.stripe_enabled {
        return Err(ApiError::bad_request("stripe checkout is disabled"));
    }
    validate_purchase_items(&req.level_ids, &req.quantities)?;
    let email = normalize_email_checked(&req.email)?;
    let api_key = state
        .config
        .stripe_api_key
        .clone()
        .ok_or_else(|| ApiError::internal("stripe api key is not configured"))?;
    let payment_token = normalize_wallet_address(&state.config.fiat_price_payment_token)?;

    let quote = state
        .chain
        .quote_purchase(
            state.config.fiat_price_chain_id,
            &req.level_ids,
            &req.quantities,
        )
        .await
        .map_err(|err| ApiError::bad_request(format!("failed to quote purchase: {err}")))?;
    let original_total = parse_u256_decimal(&quote.total_amount)?;

    let referral_code_id = resolve_fiat_referral(&state, req.referral_code.as_deref()).await?;
    let discount_code_present = req
        .discount_code
        .as_deref()
        .and_then(normalize_promotion_code)
        .is_some();
    let (discount_code_id, discount_amount) = if discount_code_present {
        resolve_discount(
            &state,
            &email,
            state.config.fiat_price_chain_id,
            &payment_token,
            &req.level_ids,
            req.discount_code.as_deref(),
            referral_code_id.is_some(),
            original_total,
            DiscountResolutionMode::Preview,
        )
        .await?
    } else {
        (
            None,
            resolve_referral_auto_discount(
                &state,
                state.config.fiat_price_chain_id,
                &payment_token,
                referral_code_id,
                original_total,
            )
            .await?,
        )
    };
    let final_total = original_total.saturating_sub(discount_amount);
    let decimals = payment_token_decimals(state.config.fiat_price_chain_id, &payment_token)
        .ok_or_else(|| ApiError::bad_request("payment token decimals are not configured"))?;
    let original_amount_cents = token_amount_to_cents(original_total, decimals)?;
    let discount_amount_cents = token_amount_to_cents(discount_amount, decimals)?;
    let final_amount_cents = token_amount_to_cents(final_total, decimals)?;
    let unit_prices_cents = token_unit_prices_to_cents(&quote.unit_prices, decimals)?;
    if final_amount_cents <= 0 {
        return Err(ApiError::bad_request("checkout amount must be positive"));
    }

    let checkout_id = uuid::Uuid::new_v4().to_string();
    let expires_at = unix_now() + state.config.fiat_checkout_session_ttl_secs;
    let checkout = state
        .db
        .create_fiat_checkout_session(NewFiatCheckoutSession {
            id: checkout_id.clone(),
            email: email.clone(),
            currency: state.config.stripe_currency.clone(),
            level_ids_json: serde_json::to_string(&req.level_ids)
                .map_err(|err| ApiError::internal(format!("serialize levels failed: {err}")))?,
            quantities_json: serde_json::to_string(&req.quantities)
                .map_err(|err| ApiError::internal(format!("serialize quantities failed: {err}")))?,
            unit_prices_cents_json: serde_json::to_string(&unit_prices_cents).map_err(|err| {
                ApiError::internal(format!("serialize unit prices failed: {err}"))
            })?,
            referral_code_id,
            discount_code_id,
            original_amount_cents,
            discount_amount_cents,
            final_amount_cents,
            expires_at,
        })
        .await
        .map_err(|err| ApiError::internal(format!("create fiat checkout failed: {err}")))?;

    let line_items = build_stripe_line_items(
        &req.level_ids,
        &req.quantities,
        &quote.unit_prices,
        decimals,
        final_amount_cents,
        discount_amount_cents,
    )?;
    let stripe_client = crate::stripe::StripeClient::new(
        api_key,
        state.config.stripe_api_version.clone(),
        state.config.stripe_api_base_url.clone(),
    );
    let stripe_session = stripe_client
        .create_checkout_session(&CreateCheckoutSession {
            success_url: state.config.stripe_success_url.clone(),
            cancel_url: state.config.stripe_cancel_url.clone(),
            currency: state.config.stripe_currency.clone(),
            customer_email: email.clone(),
            client_reference_id: checkout.id.clone(),
            metadata: vec![("fiat_checkout_id".to_string(), checkout.id.clone())],
            line_items,
            expires_at,
        })
        .await
        .map_err(|err| ApiError::internal(format!("create stripe checkout failed: {err}")))?;

    let checkout = state
        .db
        .attach_stripe_checkout_session(&checkout.id, &stripe_session.id, &stripe_session.url)
        .await
        .map_err(|err| ApiError::internal(format!("attach stripe checkout failed: {err}")))?
        .ok_or_else(|| ApiError::internal("fiat checkout disappeared"))?;

    Ok(Json(CreateFiatCheckoutSessionResponse {
        checkout_id: checkout.id,
        stripe_session_id: stripe_session.id,
        url: stripe_session.url,
        original_amount_cents,
        discount_amount_cents,
        final_amount_cents,
        currency: state.config.stripe_currency.clone(),
    }))
}

pub async fn stripe_webhook(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<serde_json::Value>, ApiError> {
    let secret = state
        .config
        .stripe_webhook_secret
        .as_deref()
        .ok_or_else(|| ApiError::internal("stripe webhook secret is not configured"))?;
    let signature = headers
        .get("stripe-signature")
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| ApiError::bad_request("missing stripe signature"))?;
    crate::stripe::verify_webhook_signature(&body, signature, secret, 300, unix_now())
        .map_err(|_| ApiError::bad_request("invalid stripe signature"))?;

    let event: Value = serde_json::from_slice(&body)
        .map_err(|_| ApiError::bad_request("invalid stripe webhook payload"))?;
    let event_type = event["type"].as_str().unwrap_or_default();
    if !matches!(
        event_type,
        "checkout.session.completed" | "checkout.session.async_payment_succeeded"
    ) {
        return Ok(Json(serde_json::json!({ "received": true })));
    }
    let session = &event["data"]["object"];
    if session["payment_status"].as_str() != Some("paid") {
        return Ok(Json(serde_json::json!({ "received": true })));
    }
    let stripe_session_id = session["id"]
        .as_str()
        .ok_or_else(|| ApiError::bad_request("missing stripe session id"))?;
    let payment_intent_id = session["payment_intent"].as_str();
    let checkout = state
        .db
        .get_fiat_checkout_session_by_stripe_id(stripe_session_id)
        .await
        .map_err(|err| ApiError::internal(format!("load fiat checkout failed: {err}")))?;
    let Some(checkout) = checkout else {
        if session["metadata"]["fiat_checkout_id"].as_str().is_none() {
            return Ok(Json(serde_json::json!({ "received": true })));
        }
        return Err(ApiError::bad_request("unknown stripe checkout session"));
    };
    validate_stripe_checkout_session(session, &checkout)?;

    let confirmation = state
        .db
        .confirm_fiat_checkout_session(stripe_session_id, payment_intent_id)
        .await
        .map_err(|err| ApiError::internal(format!("confirm fiat checkout failed: {err}")))?
        .ok_or_else(|| ApiError::bad_request("unknown stripe checkout session"))?;
    let checkout = confirmation.checkout;

    if confirmation.newly_paid && checkout.created_tickets > 0 {
        let challenge = state
            .db
            .create_email_access_challenge(
                &checkout.email,
                state.config.email_access_token_ttl_secs,
            )
            .await
            .map_err(|err| {
                ApiError::internal(format!("create email access challenge failed: {err}"))
            })?;
        let access_url =
            build_email_access_url(&state.config.app_public_base_url, &challenge.token);
        state
            .mailer
            .send_ticket_access_link(
                &checkout.email,
                &access_url,
                state.config.email_access_token_ttl_secs,
            )
            .await
            .map_err(|err| ApiError::internal(format!("email access dispatch failed: {err}")))?;
    }

    Ok(Json(serde_json::json!({ "received": true })))
}

fn validate_stripe_checkout_session(
    session: &Value,
    checkout: &FiatCheckoutSessionRow,
) -> Result<(), ApiError> {
    if !matches!(checkout.status.as_str(), "pending" | "paid") {
        return Err(ApiError::bad_request("fiat checkout is not payable"));
    }

    let client_reference_id = session["client_reference_id"]
        .as_str()
        .ok_or_else(|| ApiError::bad_request("missing stripe client reference id"))?;
    if client_reference_id != checkout.id {
        return Err(ApiError::bad_request("stripe client reference id mismatch"));
    }

    let metadata_checkout_id = session["metadata"]["fiat_checkout_id"]
        .as_str()
        .ok_or_else(|| ApiError::bad_request("missing stripe checkout metadata"))?;
    if metadata_checkout_id != checkout.id {
        return Err(ApiError::bad_request("stripe checkout metadata mismatch"));
    }

    let amount_total = session["amount_total"]
        .as_i64()
        .ok_or_else(|| ApiError::bad_request("missing stripe amount total"))?;
    if amount_total != checkout.final_amount_cents {
        return Err(ApiError::bad_request("stripe amount total mismatch"));
    }

    let currency = session["currency"]
        .as_str()
        .ok_or_else(|| ApiError::bad_request("missing stripe currency"))?;
    if !currency.eq_ignore_ascii_case(&checkout.currency) {
        return Err(ApiError::bad_request("stripe currency mismatch"));
    }

    Ok(())
}

fn build_email_access_url(base_url: &str, token: &str) -> String {
    format!(
        "{}/en/tickets/email-access?token={}",
        base_url.trim_end_matches('/'),
        token
    )
}

async fn resolve_fiat_referral(
    state: &Arc<AppState>,
    referral_code: Option<&str>,
) -> Result<Option<i64>, ApiError> {
    let Some(referral_code) = referral_code.and_then(normalize_promotion_code) else {
        return Ok(None);
    };
    let code = state
        .db
        .find_promotion_code(&referral_code)
        .await
        .map_err(|err| ApiError::internal(format!("load referral code failed: {err}")))?;
    Ok(code.and_then(|code| {
        if code.kind == "referral" && code.status == "active" {
            Some(code.id)
        } else {
            None
        }
    }))
}

fn build_stripe_line_items(
    level_ids: &[u8],
    quantities: &[u64],
    unit_prices: &[String],
    decimals: u8,
    final_amount_cents: i64,
    discount_amount_cents: i64,
) -> Result<Vec<StripeCheckoutLineItem>, ApiError> {
    if level_ids.len() != quantities.len() || level_ids.len() != unit_prices.len() {
        return Err(ApiError::bad_request("quote line item count mismatch"));
    }
    if discount_amount_cents > 0 {
        return Ok(vec![StripeCheckoutLineItem {
            name: "Money Frontier Summit 2026 Ticket Order".to_string(),
            unit_amount: final_amount_cents,
            quantity: 1,
        }]);
    }

    level_ids
        .iter()
        .zip(quantities)
        .zip(unit_prices)
        .map(|((level_id, quantity), unit_price)| {
            Ok(StripeCheckoutLineItem {
                name: format!("Money Frontier Summit 2026 - Level {level_id}"),
                unit_amount: token_amount_to_cents(parse_u256_decimal(unit_price)?, decimals)?,
                quantity: i64::try_from(*quantity)
                    .map_err(|_| ApiError::bad_request("quantity is too large"))?,
            })
        })
        .collect()
}

fn token_unit_prices_to_cents(unit_prices: &[String], decimals: u8) -> Result<Vec<i64>, ApiError> {
    unit_prices
        .iter()
        .map(|unit_price| token_amount_to_cents(parse_u256_decimal(unit_price)?, decimals))
        .collect()
}

fn token_amount_to_cents(amount: U256, decimals: u8) -> Result<i64, ApiError> {
    let cents = amount
        .checked_mul(U256::from(100u64))
        .ok_or_else(|| ApiError::bad_request("amount conversion overflow"))?
        / U256::from(10u64).pow(U256::from(decimals));
    if cents > U256::from(i64::MAX as u64) {
        return Err(ApiError::bad_request("amount is too large"));
    }
    Ok(cents.as_u64() as i64)
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

    let discount_code_present = req
        .discount_code
        .as_deref()
        .and_then(normalize_promotion_code)
        .is_some();
    let (discount_code_id, discount_amount) = if discount_code_present {
        resolve_discount(
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
        .await?
    } else {
        (
            None,
            resolve_referral_auto_discount(
                &state,
                req.chain_id,
                &payment_token,
                referral_code_id,
                original_total,
            )
            .await?,
        )
    };

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

    let discount_code_present = req
        .discount_code
        .as_deref()
        .and_then(normalize_promotion_code)
        .is_some();
    let (discount_code_id, discount_amount) = if discount_code_present {
        resolve_discount(
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
        .await?
    } else {
        (
            None,
            resolve_referral_auto_discount(
                &state,
                req.chain_id,
                &payment_token,
                referral_code_id,
                original_total,
            )
            .await?,
        )
    };

    let final_total = original_total.saturating_sub(discount_amount);
    let referral_code_present = referral_code_id.is_some();
    let (discount_status, discount_message) = match (
        discount_code_id,
        discount_amount.is_zero(),
        discount_code_present,
        referral_code_present,
    ) {
        (Some(_), false, _, _) => ("applied", "Discount applied"),
        (Some(_), true, _, _) => ("no_discount", "Discount code did not reduce this order"),
        (None, false, false, _) => ("applied", "Referral discount applied"),
        (None, true, false, true) => ("no_discount", "No referral discount configured"),
        (None, _, true, _) => ("no_discount", "No discount applied"),
        (None, _, false, false) => ("none", "No discount code"),
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

pub async fn list_ticket_prices(
    State(state): State<Arc<AppState>>,
    Json(req): Json<TicketPricesRequest>,
) -> Result<Json<TicketPricesResponse>, ApiError> {
    find_chain_config(&state.config.chains, req.chain_id)?;
    validate_price_request(&req)?;

    let quantities = vec![1_u64; req.level_ids.len()];
    let quote = state
        .chain
        .quote_purchase(req.chain_id, &req.level_ids, &quantities)
        .await
        .map_err(|err| ApiError::bad_request(format!("failed to quote ticket prices: {err}")))?;

    Ok(Json(TicketPricesResponse {
        chain_id: req.chain_id,
        prices: req
            .level_ids
            .into_iter()
            .zip(quote.unit_prices)
            .map(|(level_id, unit_price)| TicketPriceView {
                level_id,
                unit_price,
            })
            .collect(),
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
    let tickets = match extract_ticket_auth_subject(&headers, &state.jwt)? {
        TicketAuthSubject::Wallet(wallet) => state
            .db
            .list_active_tickets_by_wallet(&wallet)
            .await
            .map_err(|err| ApiError::internal(format!("query tickets failed: {err}")))?,
        TicketAuthSubject::Email(email) => state
            .db
            .list_active_tickets_by_email(&email)
            .await
            .map_err(|err| ApiError::internal(format!("query tickets failed: {err}")))?,
    };

    Ok(Json(tickets.into_iter().map(TicketView::from).collect()))
}

pub async fn redeem_redemption_code(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<RedeemRedemptionCodeRequest>,
) -> Result<Json<RedeemRedemptionCodeResponse>, ApiError> {
    let (claimant_type, claimant) = match extract_ticket_auth_subject(&headers, &state.jwt)? {
        TicketAuthSubject::Wallet(wallet) => ("wallet", wallet),
        TicketAuthSubject::Email(email) => ("email", normalize_email(&email)),
    };

    let result = state
        .db
        .redeem_redemption_code(&req.code, claimant_type, &claimant)
        .await
        .map_err(map_redemption_error)?;

    let result = result.ok_or_else(|| ApiError::bad_request("redemption code is not active"))?;

    Ok(Json(RedeemRedemptionCodeResponse {
        code: result.code.code_normalized,
        claim_id: result.claim.id,
        ticket: result.ticket.into(),
    }))
}

pub async fn redeem_redemption_code_by_email(
    State(state): State<Arc<AppState>>,
    Json(req): Json<RedeemRedemptionCodeByEmailRequest>,
) -> Result<Json<RedeemRedemptionCodeResponse>, ApiError> {
    let email = normalize_email_checked(&req.email)?;
    let result = state
        .db
        .redeem_redemption_code(&req.code, "email", &email)
        .await
        .map_err(map_redemption_error)?;

    let result = result.ok_or_else(|| ApiError::bad_request("redemption code is not active"))?;

    Ok(Json(RedeemRedemptionCodeResponse {
        code: result.code.code_normalized,
        claim_id: result.claim.id,
        ticket: result.ticket.into(),
    }))
}

fn map_redemption_error(err: anyhow::Error) -> ApiError {
    if err.to_string() == "redemption code claim limit reached" {
        return ApiError::bad_request("redemption code claim limit reached");
    }
    ApiError::internal(format!("redeem code failed: {err}"))
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
    let ticket = match extract_ticket_auth_subject(&headers, &state.jwt)? {
        TicketAuthSubject::Wallet(wallet) => state
            .db
            .get_active_ticket_by_id_for_wallet(&ticket_id, &wallet)
            .await
            .map_err(|err| ApiError::internal(format!("query ticket failed: {err}")))?,
        TicketAuthSubject::Email(email) => state
            .db
            .get_active_ticket_by_id_for_email(&ticket_id, &email)
            .await
            .map_err(|err| ApiError::internal(format!("query ticket failed: {err}")))?,
    };

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

fn normalize_email_checked(email: &str) -> Result<String, ApiError> {
    let email = normalize_email(email);
    let has_single_at = email.matches('@').count() == 1;
    let has_dot_after_at = email
        .split_once('@')
        .map(|(_, domain)| {
            domain.contains('.') && !domain.starts_with('.') && !domain.ends_with('.')
        })
        .unwrap_or(false);
    if email.is_empty() || !has_single_at || !has_dot_after_at {
        return Err(ApiError::bad_request("invalid email address"));
    }
    Ok(email)
}

enum TicketAuthSubject {
    Wallet(String),
    Email(String),
}

fn extract_ticket_auth_subject(
    headers: &HeaderMap,
    jwt: &crate::auth::JwtCodec,
) -> Result<TicketAuthSubject, ApiError> {
    match extract_wallet(headers, jwt) {
        Ok(wallet) => Ok(TicketAuthSubject::Wallet(wallet)),
        Err(wallet_err) => match extract_email_session(headers, jwt) {
            Ok(email) => Ok(TicketAuthSubject::Email(normalize_email(&email))),
            Err(_) => Err(wallet_err),
        },
    }
}

fn validate_purchase_intent_request(req: &CreatePurchaseIntentRequest) -> Result<(), ApiError> {
    validate_purchase_items(&req.level_ids, &req.quantities)
}

fn validate_purchase_quote_request(req: &CreatePurchaseQuoteRequest) -> Result<(), ApiError> {
    validate_purchase_items(&req.level_ids, &req.quantities)
}

fn validate_price_request(req: &TicketPricesRequest) -> Result<(), ApiError> {
    if req.level_ids.is_empty() {
        return Err(ApiError::bad_request("level_ids must not be empty"));
    }
    Ok(())
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

async fn resolve_referral_auto_discount(
    state: &Arc<AppState>,
    chain_id: u64,
    payment_token: &str,
    referral_code_id: Option<i64>,
    original_total: U256,
) -> Result<U256, ApiError> {
    let Some(referral_code_id) = referral_code_id else {
        return Ok(U256::zero());
    };

    let Some(code) = state
        .db
        .get_invite_code_detail(referral_code_id)
        .await
        .map_err(|err| ApiError::internal(format!("load invite code failed: {err}")))?
    else {
        return Ok(U256::zero());
    };

    if code.kind != "referral" || code.status != "active" {
        return Ok(U256::zero());
    }
    let now = unix_now();
    if code.valid_from.is_some_and(|value| value > now)
        || code.valid_until.is_some_and(|value| value < now)
        || code.discount_type.is_none()
        || code.discount_value.is_none()
    {
        return Ok(U256::zero());
    }

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

    calculate_discount_amount(&code, original_total, token_decimals)
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
        extract::State,
        http::{Method, Request, StatusCode},
        routing::{get, post},
        Json, Router,
    };
    use ethers_signers::{LocalWallet, Signer};
    use hmac::{Hmac, Mac};
    use serde_json::{json, Value};
    use sha2::Sha256;
    use tower::util::ServiceExt;

    use super::{
        create_fiat_checkout_session, create_purchase_intent, create_purchase_quote,
        email_access_challenge, email_access_verify, get_purchase_intent, get_ticket, health,
        list_ticket_prices, list_tickets, notify_tickets, parse_token_amount,
        redeem_redemption_code, redeem_redemption_code_by_email, signin_challenge, signin_verify,
        stripe_webhook, transfer_ticket, unix_now, validate_transfer_request,
        TransferTicketRequest,
    };
    use crate::{
        auth::JwtCodec,
        chain::{ChainReader, ChainRuntimeConfig, DecodedPurchase, QuoteResult},
        config::{AppConfig, ChainConfig},
        db::{Db, NewRedemptionCode, PurchaseIntentFilters, UpdateInviteCode},
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

    #[derive(Clone, Default)]
    struct StripeCapture {
        body: Arc<Mutex<Option<String>>>,
    }

    #[derive(Clone, Default)]
    struct MailCapture {
        count: Arc<Mutex<usize>>,
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

    async fn stripe_checkout_capture(
        State(state): State<StripeCapture>,
        body: String,
    ) -> Json<Value> {
        *state.body.lock().expect("capture lock should succeed") = Some(body);
        Json(json!({
            "id": "cs_test_money_frontier",
            "url": "https://checkout.stripe.test/session"
        }))
    }

    async fn spawn_stripe_capture(state: StripeCapture) -> (String, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener should bind");
        let addr = listener.local_addr().expect("local addr should exist");
        let app = Router::new()
            .route("/v1/checkout/sessions", post(stripe_checkout_capture))
            .with_state(state);
        let handle = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("stripe capture should serve");
        });
        (format!("http://{addr}"), handle)
    }

    async fn mail_capture(State(state): State<MailCapture>, body: String) -> Json<Value> {
        assert!(body.contains("guest@example.com"));
        *state.count.lock().expect("capture lock should succeed") += 1;
        Json(json!({ "ok": true }))
    }

    async fn spawn_mail_capture(state: MailCapture) -> (String, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener should bind");
        let addr = listener.local_addr().expect("local addr should exist");
        let app = Router::new()
            .route("/mail", post(mail_capture))
            .with_state(state);
        let handle = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("mail capture should serve");
        });
        (format!("http://{addr}/mail"), handle)
    }

    fn stripe_signature_header(payload: &str, secret: &str, timestamp: i64) -> String {
        let signed_payload = format!("{timestamp}.{payload}");
        let mut mac =
            Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("hmac should initialize");
        mac.update(signed_payload.as_bytes());
        let bytes = mac.finalize().into_bytes();
        let signature = bytes
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        format!("t={timestamp},v1={signature}")
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
            .route("/email/access/challenge", post(email_access_challenge))
            .route("/email/access/verify", post(email_access_verify))
            .route(
                "/fiat/checkout-sessions",
                post(create_fiat_checkout_session),
            )
            .route("/stripe/webhook", post(stripe_webhook))
            .route("/purchase-prices", post(list_ticket_prices))
            .route("/purchase-quotes", post(create_purchase_quote))
            .route("/purchase-intents", post(create_purchase_intent))
            .route("/purchase-intents/:id", get(get_purchase_intent))
            .route("/redemption-codes/redeem", post(redeem_redemption_code))
            .route(
                "/redemption-codes/redeem-by-email",
                post(redeem_redemption_code_by_email),
            )
            .route("/tickets", get(list_tickets).post(notify_tickets))
            .route("/tickets/:id", get(get_ticket).put(transfer_ticket))
            .with_state(state)
    }

    async fn build_test_app(mock_chain: Arc<MockChain>) -> (Router, Arc<AppState>) {
        build_test_app_with_config(mock_chain, |_| {}).await
    }

    async fn build_test_app_with_config(
        mock_chain: Arc<MockChain>,
        configure: impl FnOnce(&mut AppConfig),
    ) -> (Router, Arc<AppState>) {
        let database_url = "sqlite::memory:".to_string();
        let db = Db::connect(&database_url)
            .await
            .expect("db connect should succeed");

        let mut config = AppConfig {
            bind_addr: "127.0.0.1:0".parse().expect("valid addr"),
            database_url,
            jwt_secret: "test-secret".to_string(),
            jwt_ttl_days: 3650,
            mail_from: "noreply@test.local".to_string(),
            mail_reply_to: None,
            mail_provider: "console".to_string(),
            mail_webhook_url: None,
            mail_api_key: None,
            mail_max_retries: 3,
            mail_retry_backoff_ms: 1,
            mail_alert_webhook_url: None,
            mail_alert_api_key: None,
            app_public_base_url: "http://127.0.0.1:3000".to_string(),
            email_access_token_ttl_secs: 900,
            email_session_ttl_hours: 24,
            stripe_enabled: false,
            stripe_api_key: None,
            stripe_webhook_secret: None,
            stripe_api_version: "2026-04-22.dahlia".to_string(),
            stripe_currency: "usd".to_string(),
            stripe_success_url: "http://127.0.0.1:3000/success".to_string(),
            stripe_cancel_url: "http://127.0.0.1:3000/cancelled".to_string(),
            stripe_api_base_url: "https://api.stripe.com".to_string(),
            fiat_price_chain_id: 56,
            fiat_price_payment_token: "0x55d398326f99059ff775485246999027b3197955".to_string(),
            fiat_checkout_session_ttl_secs: 1800,
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
        configure(&mut config);

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
    async fn list_ticket_prices_reads_current_chain_quote_without_auth() {
        let mock_chain = Arc::new(MockChain::default());
        let (app, _state) = build_test_app(mock_chain.clone()).await;
        mock_chain.set_quote(
            56,
            &[1, 2, 3],
            &[1, 1, 1],
            QuoteResult {
                total_amount: "2588000000000000000000".to_string(),
                unit_prices: vec![
                    "100000000000000000000".to_string(),
                    "499000000000000000000".to_string(),
                    "1989000000000000000000".to_string(),
                ],
            },
        );

        let (status, body) = json_request(
            &app,
            Method::POST,
            "/purchase-prices",
            None,
            Some(json!({
                "chain_id": 56,
                "level_ids": [1, 2, 3]
            })),
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["chain_id"], 56);
        assert_eq!(body["prices"][0]["level_id"], 1);
        assert_eq!(body["prices"][0]["unit_price"], "100000000000000000000");
        assert_eq!(body["prices"][1]["unit_price"], "499000000000000000000");
        assert_eq!(body["prices"][2]["unit_price"], "1989000000000000000000");
    }

    #[tokio::test]
    async fn email_access_verify_returns_session_that_lists_email_tickets() {
        let mock_chain = Arc::new(MockChain::default());
        let (app, state) = build_test_app(mock_chain.clone()).await;
        let buyer = "0x1111111111111111111111111111111111111111";
        let email = "guest@example.com";

        state
            .db
            .index_purchase(
                56,
                &DecodedPurchase {
                    tx_hash: "0xemail".to_string(),
                    log_index: 0,
                    block_number: 100,
                    block_hash: Some("0xblock".to_string()),
                    order_id: "email-order".to_string(),
                    buyer: buyer.to_string(),
                    payment_token: "0x2222222222222222222222222222222222222222".to_string(),
                    total_amount: "1000000000000000000".to_string(),
                    level_ids: vec![1],
                    quantities: vec![1],
                    unit_prices: vec!["1000000000000000000".to_string()],
                    intent_id: None,
                },
            )
            .await
            .expect("purchase should index");

        let wallet_tickets = state
            .db
            .list_active_tickets_by_wallet(buyer)
            .await
            .expect("wallet tickets should load");
        state
            .db
            .transfer_ticket(&wallet_tickets[0].id, buyer, None, Some(email))
            .await
            .expect("ticket should transfer to email");

        let challenge = state
            .db
            .create_email_access_challenge(email, 900)
            .await
            .expect("email challenge should create");

        let (status, verify_body) = json_request(
            &app,
            Method::POST,
            "/email/access/verify",
            None,
            Some(json!({ "token": challenge.token })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(verify_body["email"], email);
        let email_token = verify_body["token"].as_str().expect("token should exist");

        let (status, tickets_body) =
            json_request(&app, Method::GET, "/tickets", Some(email_token), None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(tickets_body.as_array().expect("tickets array").len(), 1);
        assert_eq!(tickets_body[0]["owner_email"], email);
    }

    #[tokio::test]
    async fn wallet_can_redeem_code_into_free_ticket() {
        let mock_chain = Arc::new(MockChain::default());
        let (app, state) = build_test_app(mock_chain).await;
        state
            .db
            .create_redemption_code(NewRedemptionCode {
                code: "VIPFREE1".to_string(),
                status: "active".to_string(),
                ticket_level: 3,
                max_claims: 1,
                valid_from: None,
                valid_until: None,
                notes: None,
                created_by: "admin".to_string(),
            })
            .await
            .expect("redemption code should create");
        let wallet = "0x0030457e79159bed97aee6eea708441d4cff579b";
        let (token, _) = state.jwt.issue(wallet).expect("wallet jwt should issue");

        let (status, body) = json_request(
            &app,
            Method::POST,
            "/redemption-codes/redeem",
            Some(&token),
            Some(json!({ "code": "vipfree1" })),
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["code"], "VIPFREE1");
        assert_eq!(body["ticket"]["ticket_level"], 3);
        assert_eq!(body["ticket"]["owner_wallet"], wallet);
        assert_eq!(body["ticket"]["owner_email"], Value::Null);
        assert_eq!(body["ticket"]["unit_price"], "0");
        assert_eq!(body["ticket"]["chain_id"], 0);

        let code = state
            .db
            .find_redemption_code("VIPFREE1")
            .await
            .expect("code lookup should succeed")
            .expect("code should exist");
        assert_eq!(code.status, "active");

        let (status, tickets_body) =
            json_request(&app, Method::GET, "/tickets", Some(&token), None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(tickets_body.as_array().expect("tickets array").len(), 1);
    }

    #[tokio::test]
    async fn email_session_redeem_is_idempotent_for_same_code_and_email() {
        let mock_chain = Arc::new(MockChain::default());
        let (app, state) = build_test_app(mock_chain).await;
        state
            .db
            .create_redemption_code(NewRedemptionCode {
                code: "EMAILVIP".to_string(),
                status: "active".to_string(),
                ticket_level: 2,
                max_claims: 1,
                valid_from: None,
                valid_until: None,
                notes: None,
                created_by: "admin".to_string(),
            })
            .await
            .expect("redemption code should create");
        let (email_token, _) = state
            .jwt
            .issue_email_session("Guest@Example.com", 24)
            .expect("email jwt should issue");

        let (status, body) = json_request(
            &app,
            Method::POST,
            "/redemption-codes/redeem",
            Some(&email_token),
            Some(json!({ "code": "EMAILVIP" })),
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["ticket"]["ticket_level"], 2);
        assert_eq!(body["ticket"]["owner_wallet"], Value::Null);
        assert_eq!(body["ticket"]["owner_email"], "guest@example.com");

        let first_claim_id = body["claim_id"].clone();
        let first_ticket_id = body["ticket"]["id"].clone();

        let (status, duplicate_body) = json_request(
            &app,
            Method::POST,
            "/redemption-codes/redeem",
            Some(&email_token),
            Some(json!({ "code": "EMAILVIP" })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(duplicate_body["claim_id"], first_claim_id);
        assert_eq!(duplicate_body["ticket"]["id"], first_ticket_id);

        let (status, tickets_body) =
            json_request(&app, Method::GET, "/tickets", Some(&email_token), None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(tickets_body.as_array().expect("tickets array").len(), 1);
    }

    #[tokio::test]
    async fn email_address_can_redeem_public_multi_use_code() {
        let mock_chain = Arc::new(MockChain::default());
        let (app, state) = build_test_app(mock_chain).await;
        state
            .db
            .create_redemption_code(NewRedemptionCode {
                code: "SHAREVIP".to_string(),
                status: "active".to_string(),
                ticket_level: 3,
                max_claims: 2,
                valid_from: None,
                valid_until: None,
                notes: None,
                created_by: "admin".to_string(),
            })
            .await
            .expect("redemption code should create");

        let (first_status, first_body) = json_request(
            &app,
            Method::POST,
            "/redemption-codes/redeem-by-email",
            None,
            Some(json!({
                "code": "sharevip",
                "email": " First.Guest@Example.com "
            })),
        )
        .await;
        let (second_status, second_body) = json_request(
            &app,
            Method::POST,
            "/redemption-codes/redeem-by-email",
            None,
            Some(json!({
                "code": "sharevip",
                "email": "second.guest@example.com"
            })),
        )
        .await;

        assert_eq!(first_status, StatusCode::OK);
        assert_eq!(second_status, StatusCode::OK);
        assert_eq!(first_body["code"], "SHAREVIP");
        assert_eq!(first_body["ticket"]["ticket_level"], 3);
        assert_eq!(first_body["ticket"]["owner_wallet"], Value::Null);
        assert_eq!(
            first_body["ticket"]["owner_email"],
            "first.guest@example.com"
        );
        assert_eq!(
            second_body["ticket"]["owner_email"],
            "second.guest@example.com"
        );

        let code = state
            .db
            .find_redemption_code("SHAREVIP")
            .await
            .expect("code lookup should succeed")
            .expect("code should exist");
        assert_eq!(code.status, "active");

        let claims = state
            .db
            .list_redemption_code_claims(Some(code.id), 1, 10)
            .await
            .expect("claims should load");
        assert_eq!(claims.len(), 2);
    }

    #[tokio::test]
    async fn public_email_redeem_rejects_new_claimants_after_max_claims() {
        let mock_chain = Arc::new(MockChain::default());
        let (app, state) = build_test_app(mock_chain).await;
        state
            .db
            .create_redemption_code(NewRedemptionCode {
                code: "ONCEONLY".to_string(),
                status: "active".to_string(),
                ticket_level: 1,
                max_claims: 1,
                valid_from: None,
                valid_until: None,
                notes: None,
                created_by: "admin".to_string(),
            })
            .await
            .expect("redemption code should create");

        let (first_status, first_body) = json_request(
            &app,
            Method::POST,
            "/redemption-codes/redeem-by-email",
            None,
            Some(json!({
                "code": "ONCEONLY",
                "email": "first@example.com"
            })),
        )
        .await;
        let (duplicate_status, duplicate_body) = json_request(
            &app,
            Method::POST,
            "/redemption-codes/redeem-by-email",
            None,
            Some(json!({
                "code": "ONCEONLY",
                "email": "FIRST@example.com"
            })),
        )
        .await;
        let (second_status, second_body) = json_request(
            &app,
            Method::POST,
            "/redemption-codes/redeem-by-email",
            None,
            Some(json!({
                "code": "ONCEONLY",
                "email": "second@example.com"
            })),
        )
        .await;

        assert_eq!(first_status, StatusCode::OK);
        assert_eq!(duplicate_status, StatusCode::OK);
        assert_eq!(duplicate_body["claim_id"], first_body["claim_id"]);
        assert_eq!(second_status, StatusCode::BAD_REQUEST);
        assert_eq!(second_body["error"], "redemption code claim limit reached");

        let code = state
            .db
            .find_redemption_code("ONCEONLY")
            .await
            .expect("code lookup should succeed")
            .expect("code should exist");
        let claims = state
            .db
            .list_redemption_code_claims(Some(code.id), 1, 10)
            .await
            .expect("claims should load");
        assert_eq!(claims.len(), 1);
    }

    #[tokio::test]
    async fn public_email_redeem_is_idempotent_for_same_code_and_email() {
        let mock_chain = Arc::new(MockChain::default());
        let (app, state) = build_test_app(mock_chain).await;
        state
            .db
            .create_redemption_code(NewRedemptionCode {
                code: "IDEMPOT".to_string(),
                status: "active".to_string(),
                ticket_level: 1,
                max_claims: 1,
                valid_from: None,
                valid_until: None,
                notes: None,
                created_by: "admin".to_string(),
            })
            .await
            .expect("redemption code should create");

        let (first_status, first_body) = json_request(
            &app,
            Method::POST,
            "/redemption-codes/redeem-by-email",
            None,
            Some(json!({
                "code": "IDEMPOT",
                "email": "guest@example.com"
            })),
        )
        .await;
        let (second_status, second_body) = json_request(
            &app,
            Method::POST,
            "/redemption-codes/redeem-by-email",
            None,
            Some(json!({
                "code": "IDEMPOT",
                "email": "GUEST@example.com"
            })),
        )
        .await;

        assert_eq!(first_status, StatusCode::OK);
        assert_eq!(second_status, StatusCode::OK);
        assert_eq!(first_body["claim_id"], second_body["claim_id"]);
        assert_eq!(first_body["ticket"]["id"], second_body["ticket"]["id"]);

        let code = state
            .db
            .find_redemption_code("IDEMPOT")
            .await
            .expect("code lookup should succeed")
            .expect("code should exist");
        let claims = state
            .db
            .list_redemption_code_claims(Some(code.id), 1, 10)
            .await
            .expect("claims should load");
        assert_eq!(claims.len(), 1);
    }

    #[tokio::test]
    async fn fiat_checkout_creates_stripe_session_without_payment_method_types() {
        let mock_chain = Arc::new(MockChain::default());
        mock_chain.set_quote(
            56,
            &[1],
            &[2],
            QuoteResult {
                total_amount: "176000000000000000000".to_string(),
                unit_prices: vec!["88000000000000000000".to_string()],
            },
        );
        let capture = StripeCapture::default();
        let (stripe_url, stripe_handle) = spawn_stripe_capture(capture.clone()).await;
        let (app, _state) = build_test_app_with_config(mock_chain, |config| {
            config.stripe_enabled = true;
            config.stripe_api_key = Some("sk_test_local".to_string());
            config.stripe_api_base_url = stripe_url;
            config.fiat_price_chain_id = 56;
            config.fiat_price_payment_token =
                "0x55d398326f99059ff775485246999027b3197955".to_string();
        })
        .await;

        let (status, body) = json_request(
            &app,
            Method::POST,
            "/fiat/checkout-sessions",
            None,
            Some(json!({
                "email": "guest@example.com",
                "level_ids": [1],
                "quantities": [2]
            })),
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["stripe_session_id"], "cs_test_money_frontier");
        assert_eq!(body["url"], "https://checkout.stripe.test/session");
        assert_eq!(body["original_amount_cents"], 17600);
        assert_eq!(body["final_amount_cents"], 17600);

        let captured_body = capture
            .body
            .lock()
            .expect("capture lock should succeed")
            .clone()
            .expect("stripe request body should be captured");
        assert!(!captured_body.contains("payment_method_types"));
        assert!(captured_body.contains("mode=payment"));
        assert!(captured_body.contains("customer_email=guest%40example.com"));
        assert!(captured_body.contains("line_items%5B0%5D%5Bprice_data%5D%5Bunit_amount%5D=8800"));
        assert!(captured_body.contains("line_items%5B0%5D%5Bquantity%5D=2"));

        stripe_handle.abort();
    }

    #[tokio::test]
    async fn fiat_checkout_charges_final_discounted_amount_in_stripe() {
        let mock_chain = Arc::new(MockChain::default());
        mock_chain.set_quote(
            56,
            &[1],
            &[1],
            QuoteResult {
                total_amount: "88000000000000000000".to_string(),
                unit_prices: vec!["88000000000000000000".to_string()],
            },
        );
        let capture = StripeCapture::default();
        let (stripe_url, stripe_handle) = spawn_stripe_capture(capture.clone()).await;
        let (app, state) = build_test_app_with_config(mock_chain, |config| {
            config.stripe_enabled = true;
            config.stripe_api_key = Some("sk_test_local".to_string());
            config.stripe_api_base_url = stripe_url;
        })
        .await;
        state
            .db
            .seed_fixed_discount_code("save8", "8")
            .await
            .expect("discount code seed should succeed");

        let (status, body) = json_request(
            &app,
            Method::POST,
            "/fiat/checkout-sessions",
            None,
            Some(json!({
                "email": "guest@example.com",
                "level_ids": [1],
                "quantities": [1],
                "discount_code": "save8"
            })),
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["original_amount_cents"], 8800);
        assert_eq!(body["discount_amount_cents"], 800);
        assert_eq!(body["final_amount_cents"], 8000);

        let captured_body = capture
            .body
            .lock()
            .expect("capture lock should succeed")
            .clone()
            .expect("stripe request body should be captured");
        assert!(captured_body.contains("line_items%5B0%5D%5Bprice_data%5D%5Bunit_amount%5D=8000"));
        assert!(!captured_body.contains("line_items%5B0%5D%5Bprice_data%5D%5Bunit_amount%5D=8800"));

        stripe_handle.abort();
    }

    #[tokio::test]
    async fn stripe_webhook_paid_session_creates_email_tickets_once() {
        let mock_chain = Arc::new(MockChain::default());
        mock_chain.set_quote(
            56,
            &[1],
            &[2],
            QuoteResult {
                total_amount: "176000000000000000000".to_string(),
                unit_prices: vec!["88000000000000000000".to_string()],
            },
        );
        let capture = StripeCapture::default();
        let (stripe_url, stripe_handle) = spawn_stripe_capture(capture).await;
        let mail_capture = MailCapture::default();
        let (mail_url, mail_handle) = spawn_mail_capture(mail_capture.clone()).await;
        let webhook_secret = "whsec_local_test";
        let (app, state) = build_test_app_with_config(mock_chain, |config| {
            config.stripe_enabled = true;
            config.stripe_api_key = Some("sk_test_local".to_string());
            config.stripe_webhook_secret = Some(webhook_secret.to_string());
            config.stripe_api_base_url = stripe_url;
            config.mail_provider = "webhook".to_string();
            config.mail_webhook_url = Some(mail_url);
        })
        .await;

        let (status, _) = json_request(
            &app,
            Method::POST,
            "/fiat/checkout-sessions",
            None,
            Some(json!({
                "email": "guest@example.com",
                "level_ids": [1],
                "quantities": [2]
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        let checkout = state
            .db
            .get_fiat_checkout_session_by_stripe_id("cs_test_money_frontier")
            .await
            .expect("checkout should load")
            .expect("checkout should exist");
        let payload = json!({
            "id": "evt_test",
            "type": "checkout.session.completed",
            "data": {
                "object": {
                    "id": "cs_test_money_frontier",
                    "client_reference_id": checkout.id,
                    "metadata": {
                        "fiat_checkout_id": checkout.id
                    },
                    "amount_total": 17600,
                    "currency": "usd",
                    "payment_status": "paid",
                    "payment_intent": "pi_test_money_frontier"
                }
            }
        })
        .to_string();
        let signature = stripe_signature_header(&payload, webhook_secret, unix_now());

        for _ in 0..2 {
            let req = Request::builder()
                .method(Method::POST)
                .uri("/stripe/webhook")
                .header("stripe-signature", &signature)
                .header("content-type", "application/json")
                .body(Body::from(payload.clone()))
                .expect("request should build");
            let response = app
                .clone()
                .oneshot(req)
                .await
                .expect("request should succeed");
            assert_eq!(response.status(), StatusCode::OK);
        }

        let tickets = state
            .db
            .list_active_tickets_by_email("guest@example.com")
            .await
            .expect("email tickets should load");
        assert_eq!(tickets.len(), 2);
        assert_eq!(tickets[0].unit_price, "8800");
        assert_eq!(tickets[1].unit_price, "8800");
        let checkout = state
            .db
            .get_fiat_checkout_session_by_stripe_id("cs_test_money_frontier")
            .await
            .expect("checkout should load")
            .expect("checkout should exist");
        assert_eq!(checkout.status, "paid");
        assert_eq!(checkout.created_tickets, 2);
        assert_eq!(
            *mail_capture
                .count
                .lock()
                .expect("capture lock should succeed"),
            1
        );

        stripe_handle.abort();
        mail_handle.abort();
    }

    #[tokio::test]
    async fn stripe_webhook_rejects_paid_session_when_signature_is_invalid() {
        let mock_chain = Arc::new(MockChain::default());
        mock_chain.set_quote(
            56,
            &[1],
            &[1],
            QuoteResult {
                total_amount: "88000000000000000000".to_string(),
                unit_prices: vec!["88000000000000000000".to_string()],
            },
        );
        let capture = StripeCapture::default();
        let (stripe_url, stripe_handle) = spawn_stripe_capture(capture).await;
        let webhook_secret = "whsec_local_test";
        let (app, state) = build_test_app_with_config(mock_chain, |config| {
            config.stripe_enabled = true;
            config.stripe_api_key = Some("sk_test_local".to_string());
            config.stripe_webhook_secret = Some(webhook_secret.to_string());
            config.stripe_api_base_url = stripe_url;
        })
        .await;

        let (status, _) = json_request(
            &app,
            Method::POST,
            "/fiat/checkout-sessions",
            None,
            Some(json!({
                "email": "guest@example.com",
                "level_ids": [1],
                "quantities": [1]
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        let payload = json!({
            "id": "evt_test",
            "type": "checkout.session.completed",
            "data": {
                "object": {
                    "id": "cs_test_money_frontier",
                    "client_reference_id": state
                        .db
                        .get_fiat_checkout_session_by_stripe_id("cs_test_money_frontier")
                        .await
                        .expect("checkout lookup should succeed")
                        .expect("checkout should exist")
                        .id,
                    "metadata": {
                        "fiat_checkout_id": state
                            .db
                            .get_fiat_checkout_session_by_stripe_id("cs_test_money_frontier")
                            .await
                            .expect("checkout lookup should succeed")
                            .expect("checkout should exist")
                            .id
                    },
                    "amount_total": 8800,
                    "currency": "usd",
                    "payment_status": "paid",
                    "payment_intent": "pi_test_money_frontier"
                }
            }
        })
        .to_string();
        let signature = stripe_signature_header(&payload, "wrong_secret", unix_now());
        let req = Request::builder()
            .method(Method::POST)
            .uri("/stripe/webhook")
            .header("stripe-signature", &signature)
            .header("content-type", "application/json")
            .body(Body::from(payload))
            .expect("request should build");
        let response = app
            .clone()
            .oneshot(req)
            .await
            .expect("request should succeed");
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        let checkout = state
            .db
            .get_fiat_checkout_session_by_stripe_id("cs_test_money_frontier")
            .await
            .expect("checkout should load")
            .expect("checkout should exist");
        assert_eq!(checkout.status, "pending");
        assert_eq!(checkout.created_tickets, 0);

        stripe_handle.abort();
    }

    #[tokio::test]
    async fn stripe_webhook_rejects_paid_session_when_amount_currency_or_metadata_mismatch() {
        let mock_chain = Arc::new(MockChain::default());
        mock_chain.set_quote(
            56,
            &[1],
            &[1],
            QuoteResult {
                total_amount: "88000000000000000000".to_string(),
                unit_prices: vec!["88000000000000000000".to_string()],
            },
        );
        let capture = StripeCapture::default();
        let (stripe_url, stripe_handle) = spawn_stripe_capture(capture).await;
        let webhook_secret = "whsec_local_test";
        let (app, state) = build_test_app_with_config(mock_chain, |config| {
            config.stripe_enabled = true;
            config.stripe_api_key = Some("sk_test_local".to_string());
            config.stripe_webhook_secret = Some(webhook_secret.to_string());
            config.stripe_api_base_url = stripe_url;
        })
        .await;

        let (status, _) = json_request(
            &app,
            Method::POST,
            "/fiat/checkout-sessions",
            None,
            Some(json!({
                "email": "guest@example.com",
                "level_ids": [1],
                "quantities": [1]
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        let checkout = state
            .db
            .get_fiat_checkout_session_by_stripe_id("cs_test_money_frontier")
            .await
            .expect("checkout should load")
            .expect("checkout should exist");

        for session in [
            json!({
                "id": "cs_test_money_frontier",
                "client_reference_id": checkout.id,
                "metadata": { "fiat_checkout_id": checkout.id },
                "amount_total": 8700,
                "currency": "usd",
                "payment_status": "paid",
                "payment_intent": "pi_test_money_frontier"
            }),
            json!({
                "id": "cs_test_money_frontier",
                "client_reference_id": checkout.id,
                "metadata": { "fiat_checkout_id": checkout.id },
                "amount_total": 8800,
                "currency": "eur",
                "payment_status": "paid",
                "payment_intent": "pi_test_money_frontier"
            }),
            json!({
                "id": "cs_test_money_frontier",
                "client_reference_id": "other-checkout",
                "metadata": { "fiat_checkout_id": "other-checkout" },
                "amount_total": 8800,
                "currency": "usd",
                "payment_status": "paid",
                "payment_intent": "pi_test_money_frontier"
            }),
        ] {
            let payload = json!({
                "id": "evt_test",
                "type": "checkout.session.completed",
                "data": { "object": session }
            })
            .to_string();
            let signature = stripe_signature_header(&payload, webhook_secret, unix_now());
            let req = Request::builder()
                .method(Method::POST)
                .uri("/stripe/webhook")
                .header("stripe-signature", &signature)
                .header("content-type", "application/json")
                .body(Body::from(payload))
                .expect("request should build");
            let response = app
                .clone()
                .oneshot(req)
                .await
                .expect("request should succeed");
            assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        }

        let checkout = state
            .db
            .get_fiat_checkout_session_by_stripe_id("cs_test_money_frontier")
            .await
            .expect("checkout should load")
            .expect("checkout should exist");
        assert_eq!(checkout.status, "pending");
        assert_eq!(checkout.created_tickets, 0);

        stripe_handle.abort();
    }

    #[tokio::test]
    async fn stripe_webhook_ignores_paid_sessions_without_money_frontier_metadata() {
        let mock_chain = Arc::new(MockChain::default());
        let webhook_secret = "whsec_local_test";
        let (app, _state) = build_test_app_with_config(mock_chain, |config| {
            config.stripe_enabled = true;
            config.stripe_webhook_secret = Some(webhook_secret.to_string());
        })
        .await;

        let payload = json!({
            "id": "evt_test",
            "type": "checkout.session.completed",
            "data": {
                "object": {
                    "id": "cs_test_other_service",
                    "amount_total": 8800,
                    "currency": "usd",
                    "payment_status": "paid",
                    "payment_intent": "pi_test_other_service"
                }
            }
        })
        .to_string();
        let signature = stripe_signature_header(&payload, webhook_secret, unix_now());
        let req = Request::builder()
            .method(Method::POST)
            .uri("/stripe/webhook")
            .header("stripe-signature", &signature)
            .header("content-type", "application/json")
            .body(Body::from(payload))
            .expect("request should build");
        let response = app
            .clone()
            .oneshot(req)
            .await
            .expect("request should succeed");
        assert_eq!(response.status(), StatusCode::OK);
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
    async fn create_purchase_quote_applies_referral_auto_discount_without_discount_code() {
        let mock_chain = Arc::new(MockChain::default());
        let (app, state) = build_test_app(mock_chain.clone()).await;

        let referral_code_id = state
            .db
            .seed_referral_code("partner")
            .await
            .expect("referral code seed should succeed");
        state
            .db
            .update_invite_code(
                referral_code_id,
                UpdateInviteCode {
                    beneficiary_wallet: None,
                    status: None,
                    commission_type: None,
                    commission_value: None,
                    discount_type: Some("percentage".to_string()),
                    discount_value: Some("1000".to_string()),
                    valid_from: None,
                    valid_until: None,
                    notes: None,
                },
            )
            .await
            .expect("referral buyer discount seed should succeed");
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
                "payment_token": "0xed7b83bf2862Ea0F702C76064004EFFCd0f4b1D5",
                "level_ids": [1],
                "quantities": [1],
                "referral_code": "partner"
            })),
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["referral_binding_status"], "would_bind");
        assert_eq!(body["discount_amount"], "10000000000000000000");
        assert_eq!(body["final_total_amount"], "90000000000000000000");
        assert_eq!(body["discount_status"], "applied");
        assert_eq!(body["discount_message"], "Referral discount applied");
    }

    #[tokio::test]
    async fn create_purchase_intent_prefers_manual_discount_over_referral_auto_discount() {
        let mock_chain = Arc::new(MockChain::default());
        let (app, state) = build_test_app(mock_chain.clone()).await;

        let referral_code_id = state
            .db
            .seed_referral_code("partner")
            .await
            .expect("referral code seed should succeed");
        state
            .db
            .update_invite_code(
                referral_code_id,
                UpdateInviteCode {
                    beneficiary_wallet: None,
                    status: None,
                    commission_type: None,
                    commission_value: None,
                    discount_type: Some("percentage".to_string()),
                    discount_value: Some("1000".to_string()),
                    valid_from: None,
                    valid_until: None,
                    notes: None,
                },
            )
            .await
            .expect("referral buyer discount seed should succeed");
        let discount_code_id = state
            .db
            .seed_fixed_discount_code("manual20", "20")
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
            "/purchase-intents",
            Some(&token),
            Some(json!({
                "chain_id": 56,
                "payment_token": "0xed7b83BF2862Ea0F702C76064004EFFCd0f4b1D5",
                "level_ids": [1],
                "quantities": [1],
                "referral_code": "partner",
                "discount_code": "manual20"
            })),
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["discount_amount"], "20000000000000000000");
        assert_eq!(body["final_total_amount"], "80000000000000000000");
        let intent = state
            .db
            .get_purchase_intent(body["intent_id"].as_str().expect("intent id should exist"))
            .await
            .expect("purchase intent lookup should succeed")
            .expect("purchase intent should exist");
        assert_eq!(intent.referral_code_id, Some(referral_code_id));
        assert_eq!(intent.discount_code_id, Some(discount_code_id));
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
                    discount_type: None,
                    discount_value: None,
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
