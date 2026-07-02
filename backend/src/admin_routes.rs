use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::{header, HeaderMap},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};

use crate::{
    admin::{extract_admin, require_role, AdminPrincipal, AdminRole},
    auth::{normalize_wallet_address, verify_wallet_signature},
    db::{
        AdminAuditLogRow, AdminOrderRow, AdminRedemptionClaimRow, AdminReferralBindingRow,
        AdminWalletRow, NewAdminAuditLog, NewAdminWallet, NewDiscountCode, NewInviteCode,
        NewRedemptionCode, OrderAttributionDiagnostic, OrderFilters, PurchaseIntentDiagnostic,
        PurchaseIntentFilters, RedemptionCodeRow, RedemptionCodeStatsRow, ReferralSettlementRow,
        UpdateAdminWallet, UpdateDiscountCode, UpdateInviteCode, UpdateRedemptionCode,
    },
    error::ApiError,
    promotions::{
        normalize_new_invite_code, normalize_new_promotion_code, PromotionCodeRow,
        PROMOTION_CODE_MAX_LEN, PROMOTION_CODE_MIN_LEN, SAFE_PROMOTION_CODE_ALPHABET,
    },
    AppState,
};

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/auth/challenge", post(admin_auth_challenge))
        .route("/auth/verify", post(admin_auth_verify))
        .route("/me", get(admin_me))
        .route(
            "/invite-codes",
            get(list_invite_codes).post(create_invite_code),
        )
        .route(
            "/invite-codes/:id",
            get(get_invite_code).patch(update_invite_code),
        )
        .route("/invite-codes/:id/pause", post(pause_invite_code))
        .route("/invite-codes/:id/activate", post(activate_invite_code))
        .route(
            "/discount-codes",
            get(list_discount_codes).post(create_discount_code),
        )
        .route(
            "/discount-codes/:id",
            get(get_discount_code).patch(update_discount_code),
        )
        .route("/discount-codes/:id/pause", post(pause_discount_code))
        .route("/discount-codes/:id/activate", post(activate_discount_code))
        .route(
            "/redemption-codes",
            get(list_redemption_codes).post(create_redemption_code),
        )
        .route("/redemption-codes/bulk", post(bulk_create_redemption_codes))
        .route("/redemption-codes/stats", get(get_redemption_code_stats))
        .route("/redemption-codes/claims", get(list_redemption_code_claims))
        .route(
            "/redemption-codes/:id",
            get(get_redemption_code).patch(update_redemption_code),
        )
        .route("/redemption-codes/:id/pause", post(pause_redemption_code))
        .route(
            "/redemption-codes/:id/activate",
            post(activate_redemption_code),
        )
        .route(
            "/redemption-codes/:id/claims",
            get(list_redemption_code_claims_for_code),
        )
        .route("/referral-bindings", get(list_referral_bindings))
        .route("/purchase-intents", get(list_purchase_intents))
        .route("/purchase-intents/:id", get(get_purchase_intent_diagnostic))
        .route("/orders", get(list_orders))
        .route("/orders/:id/attribution", get(get_order_attribution))
        .route("/settlements/referrals", get(list_referral_settlements))
        .route(
            "/settlements/referrals.csv",
            get(download_referral_settlements_csv),
        )
        .route("/audit-logs", get(list_audit_logs))
        .route(
            "/admin-wallets",
            get(list_admin_wallets).post(create_admin_wallet),
        )
        .route(
            "/admin-wallets/:id",
            get(get_admin_wallet)
                .patch(update_admin_wallet)
                .delete(delete_admin_wallet),
        )
}

#[derive(Debug, Deserialize)]
pub struct AdminChallengeRequest {
    pub address: String,
}

#[derive(Debug, Serialize)]
pub struct AdminChallengeResponse {
    pub challenge_id: String,
    pub challenge_message: String,
    pub expires_at: i64,
}

pub async fn admin_auth_challenge(
    State(state): State<Arc<AppState>>,
    Json(req): Json<AdminChallengeRequest>,
) -> Result<Json<AdminChallengeResponse>, ApiError> {
    let wallet = normalize_wallet_address(&req.address)?;
    let challenge = state
        .db
        .create_admin_signin_challenge(&wallet, state.config.signin_challenge_ttl_secs)
        .await
        .map_err(|err| ApiError::internal(format!("create admin challenge failed: {err}")))?;

    Ok(Json(AdminChallengeResponse {
        challenge_id: challenge.id,
        challenge_message: challenge.challenge_message,
        expires_at: challenge.expires_at,
    }))
}

#[derive(Debug, Deserialize)]
pub struct AdminVerifyRequest {
    pub address: String,
    pub challenge_id: String,
    pub signature: String,
}

#[derive(Debug, Serialize)]
pub struct AdminVerifyResponse {
    pub wallet: String,
    pub role: String,
    pub token: String,
    pub expires_at: i64,
}

pub async fn admin_auth_verify(
    State(state): State<Arc<AppState>>,
    Json(req): Json<AdminVerifyRequest>,
) -> Result<Json<AdminVerifyResponse>, ApiError> {
    let wallet = normalize_wallet_address(&req.address)?;
    let challenge_message = state
        .db
        .get_admin_signin_challenge_message(&req.challenge_id, &wallet)
        .await
        .map_err(|err| ApiError::internal(format!("load admin challenge failed: {err}")))?
        .ok_or_else(|| ApiError::unauthorized("invalid or expired admin challenge"))?;

    verify_wallet_signature(&wallet, &challenge_message, &req.signature)?;

    let role = resolve_admin_wallet_role(&state, &wallet).await?;
    let Some(role) = role else {
        return Err(ApiError::forbidden("wallet is not authorized for admin"));
    };

    let consumed = state
        .db
        .mark_admin_signin_challenge_used(&req.challenge_id, &wallet)
        .await
        .map_err(|err| ApiError::internal(format!("consume admin challenge failed: {err}")))?;
    if !consumed {
        return Err(ApiError::unauthorized("invalid or expired admin challenge"));
    }

    let role = role.as_str();
    let (token, expires_at) =
        state
            .jwt
            .issue_admin(&wallet, role, state.config.admin_jwt_ttl_hours)?;

    Ok(Json(AdminVerifyResponse {
        wallet,
        role: role.to_string(),
        token,
        expires_at,
    }))
}

async fn resolve_admin_wallet_role(
    state: &Arc<AppState>,
    wallet: &str,
) -> Result<Option<AdminRole>, ApiError> {
    if state
        .chain
        .has_default_admin_role(wallet)
        .await
        .map_err(|err| ApiError::internal(format!("check admin chain role failed: {err}")))?
    {
        return Ok(Some(AdminRole::Admin));
    }

    state
        .db
        .find_active_admin_wallet_role(wallet)
        .await
        .map_err(|err| ApiError::internal(format!("lookup admin wallet failed: {err}")))?
        .map(|role| role.parse())
        .transpose()
}

async fn authenticate_admin(
    state: &Arc<AppState>,
    headers: &HeaderMap,
) -> Result<AdminPrincipal, ApiError> {
    let token_principal = extract_admin(headers, &state.jwt)?;
    let Some(role) = resolve_admin_wallet_role(state, &token_principal.wallet).await? else {
        return Err(ApiError::forbidden("wallet is not authorized for admin"));
    };

    Ok(AdminPrincipal {
        wallet: token_principal.wallet,
        role,
    })
}

#[derive(Debug, Serialize)]
pub struct AdminMeResponse {
    pub wallet: String,
    pub role: String,
    pub scope: String,
}

pub async fn admin_me(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<AdminMeResponse>, ApiError> {
    let principal = authenticate_admin(&state, &headers).await?;
    require_role(
        &principal,
        &[
            AdminRole::Viewer,
            AdminRole::Operator,
            AdminRole::Finance,
            AdminRole::Admin,
        ],
    )?;

    Ok(Json(AdminMeResponse {
        wallet: principal.wallet,
        role: principal.role.as_str().to_string(),
        scope: "admin".to_string(),
    }))
}

#[derive(Debug, Deserialize)]
pub struct ListQuery {
    pub page: Option<i64>,
    pub page_size: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct PromotionCodeListResponse {
    pub items: Vec<PromotionCodeRow>,
}

#[derive(Debug, Serialize)]
pub struct RedemptionCodeListResponse {
    pub items: Vec<RedemptionCodeRow>,
}

#[derive(Debug, Serialize)]
pub struct RedemptionClaimListResponse {
    pub items: Vec<AdminRedemptionClaimRow>,
}

#[derive(Debug, Serialize)]
pub struct ReferralBindingListResponse {
    pub items: Vec<AdminReferralBindingRow>,
}

#[derive(Debug, Serialize)]
pub struct PurchaseIntentListResponse {
    pub items: Vec<crate::promotions::PurchaseIntentRow>,
}

#[derive(Debug, Serialize)]
pub struct OrderListResponse {
    pub items: Vec<AdminOrderRow>,
}

#[derive(Debug, Serialize)]
pub struct ReferralSettlementListResponse {
    pub items: Vec<ReferralSettlementRow>,
}

#[derive(Debug, Serialize)]
pub struct AdminAuditLogListResponse {
    pub items: Vec<AdminAuditLogRow>,
}

pub async fn list_invite_codes(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<ListQuery>,
) -> Result<Json<PromotionCodeListResponse>, ApiError> {
    let principal = authenticate_admin(&state, &headers).await?;
    require_role(
        &principal,
        &[
            AdminRole::Viewer,
            AdminRole::Operator,
            AdminRole::Finance,
            AdminRole::Admin,
        ],
    )?;

    let items = state
        .db
        .list_invite_codes(query.page.unwrap_or(1), query.page_size.unwrap_or(50))
        .await
        .map_err(|err| ApiError::internal(format!("list invite codes failed: {err}")))?;

    Ok(Json(PromotionCodeListResponse { items }))
}

#[derive(Debug, Deserialize)]
pub struct CreateInviteCodeRequest {
    pub code: String,
    pub beneficiary_wallet: Option<String>,
    pub status: String,
    pub commission_type: Option<String>,
    pub commission_value: Option<String>,
    pub discount_type: Option<String>,
    pub discount_value: Option<String>,
    pub valid_from: Option<i64>,
    pub valid_until: Option<i64>,
    pub notes: Option<String>,
}

pub async fn create_invite_code(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(mut req): Json<CreateInviteCodeRequest>,
) -> Result<Json<PromotionCodeRow>, ApiError> {
    let principal = authenticate_admin(&state, &headers).await?;
    require_role(&principal, &[AdminRole::Operator])?;
    validate_promotion_status(&req.status)?;
    validate_optional_discount_value(req.discount_type.as_deref(), req.discount_value.as_deref())?;
    if req.discount_type.as_deref() == Some("") && req.discount_value.as_deref() == Some("") {
        req.discount_type = None;
        req.discount_value = None;
    }
    let code = normalize_new_invite_code(&req.code).map_err(ApiError::bad_request)?;

    if state
        .db
        .find_promotion_code(&code)
        .await
        .map_err(|err| ApiError::internal(format!("check duplicate code failed: {err}")))?
        .is_some()
    {
        return Err(ApiError::bad_request("promotion code already exists"));
    }

    let beneficiary_wallet = normalize_optional_wallet_address(req.beneficiary_wallet.as_deref())?;
    let row = state
        .db
        .create_invite_code(NewInviteCode {
            code,
            beneficiary_wallet,
            status: req.status,
            commission_type: req.commission_type,
            commission_value: req.commission_value,
            discount_type: req.discount_type,
            discount_value: req.discount_value,
            valid_from: req.valid_from,
            valid_until: req.valid_until,
            notes: req.notes,
        })
        .await
        .map_err(|err| ApiError::bad_request(format!("create invite code failed: {err}")))?;

    insert_admin_audit(
        &state,
        &principal,
        "invite_code.create",
        "invite_code",
        Some(row.id.to_string()),
        None,
        Some(&row),
    )
    .await?;

    Ok(Json(row))
}

pub async fn get_invite_code(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> Result<Json<PromotionCodeRow>, ApiError> {
    let principal = authenticate_admin(&state, &headers).await?;
    require_role(
        &principal,
        &[
            AdminRole::Viewer,
            AdminRole::Operator,
            AdminRole::Finance,
            AdminRole::Admin,
        ],
    )?;

    let row = state
        .db
        .get_invite_code_detail(id)
        .await
        .map_err(|err| ApiError::internal(format!("get invite code failed: {err}")))?
        .ok_or_else(|| ApiError::not_found("invite code not found"))?;

    Ok(Json(row))
}

#[derive(Debug, Deserialize)]
pub struct UpdateInviteCodeRequest {
    pub beneficiary_wallet: Option<String>,
    pub status: Option<String>,
    pub commission_type: Option<String>,
    pub commission_value: Option<String>,
    pub discount_type: Option<String>,
    pub discount_value: Option<String>,
    pub valid_from: Option<i64>,
    pub valid_until: Option<i64>,
    pub notes: Option<String>,
}

pub async fn update_invite_code(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<i64>,
    Json(req): Json<UpdateInviteCodeRequest>,
) -> Result<Json<PromotionCodeRow>, ApiError> {
    let principal = authenticate_admin(&state, &headers).await?;
    require_role(&principal, &[AdminRole::Operator])?;

    if let Some(status) = req.status.as_deref() {
        validate_promotion_status(status)?;
    }
    validate_optional_discount_value(req.discount_type.as_deref(), req.discount_value.as_deref())?;
    let beneficiary_wallet = normalize_optional_wallet_address(req.beneficiary_wallet.as_deref())?;
    let before = state
        .db
        .get_invite_code_detail(id)
        .await
        .map_err(|err| ApiError::internal(format!("get invite code failed: {err}")))?
        .ok_or_else(|| ApiError::not_found("invite code not found"))?;
    let row = state
        .db
        .update_invite_code(
            id,
            UpdateInviteCode {
                beneficiary_wallet,
                status: req.status,
                commission_type: req.commission_type,
                commission_value: req.commission_value,
                discount_type: req.discount_type,
                discount_value: req.discount_value,
                valid_from: req.valid_from,
                valid_until: req.valid_until,
                notes: req.notes,
            },
        )
        .await
        .map_err(|err| ApiError::internal(format!("update invite code failed: {err}")))?
        .ok_or_else(|| ApiError::not_found("invite code not found"))?;

    insert_admin_audit(
        &state,
        &principal,
        "invite_code.update",
        "invite_code",
        Some(row.id.to_string()),
        Some(&before),
        Some(&row),
    )
    .await?;

    Ok(Json(row))
}

pub async fn pause_invite_code(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> Result<Json<PromotionCodeRow>, ApiError> {
    set_invite_code_status(state, headers, id, "paused", "invite_code.pause").await
}

pub async fn activate_invite_code(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> Result<Json<PromotionCodeRow>, ApiError> {
    set_invite_code_status(state, headers, id, "active", "invite_code.activate").await
}

pub async fn list_discount_codes(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<ListQuery>,
) -> Result<Json<PromotionCodeListResponse>, ApiError> {
    let principal = authenticate_admin(&state, &headers).await?;
    require_role(
        &principal,
        &[
            AdminRole::Viewer,
            AdminRole::Operator,
            AdminRole::Finance,
            AdminRole::Admin,
        ],
    )?;

    let items = state
        .db
        .list_discount_codes(query.page.unwrap_or(1), query.page_size.unwrap_or(50))
        .await
        .map_err(|err| ApiError::internal(format!("list discount codes failed: {err}")))?;

    Ok(Json(PromotionCodeListResponse { items }))
}

#[derive(Debug, Deserialize)]
pub struct CreateDiscountCodeRequest {
    pub code: String,
    pub status: String,
    pub discount_type: String,
    pub discount_value: String,
    pub max_discount_amount: Option<String>,
    pub max_total_uses: Option<i64>,
    pub max_uses_per_wallet: Option<i64>,
    pub first_purchase_only: Option<bool>,
    pub stacking_policy: Option<String>,
    pub applicable_chain_ids: Option<Vec<i64>>,
    pub applicable_ticket_levels: Option<Vec<i64>>,
    pub valid_from: Option<i64>,
    pub valid_until: Option<i64>,
    pub notes: Option<String>,
}

pub async fn create_discount_code(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<CreateDiscountCodeRequest>,
) -> Result<Json<PromotionCodeRow>, ApiError> {
    let principal = authenticate_admin(&state, &headers).await?;
    require_role(&principal, &[AdminRole::Operator])?;
    validate_promotion_status(&req.status)?;
    let code = normalize_new_promotion_code(&req.code).map_err(ApiError::bad_request)?;
    validate_discount_value(&req.discount_type, &req.discount_value)?;
    if let Some(max_discount_amount) = req.max_discount_amount.as_deref() {
        validate_human_token_amount(max_discount_amount)?;
    }

    if state
        .db
        .find_promotion_code(&code)
        .await
        .map_err(|err| ApiError::internal(format!("check duplicate code failed: {err}")))?
        .is_some()
    {
        return Err(ApiError::bad_request("promotion code already exists"));
    }

    let row = state
        .db
        .create_discount_code(NewDiscountCode {
            code,
            status: req.status,
            discount_type: req.discount_type,
            discount_value: req.discount_value,
            max_discount_amount: req.max_discount_amount,
            max_total_uses: req.max_total_uses,
            max_uses_per_wallet: req.max_uses_per_wallet,
            first_purchase_only: req.first_purchase_only.unwrap_or(false),
            stacking_policy: req.stacking_policy,
            applicable_chain_ids: serialize_json_scope(req.applicable_chain_ids)?,
            applicable_ticket_levels: serialize_json_scope(req.applicable_ticket_levels)?,
            valid_from: req.valid_from,
            valid_until: req.valid_until,
            notes: req.notes,
        })
        .await
        .map_err(|err| ApiError::bad_request(format!("create discount code failed: {err}")))?;

    insert_admin_audit(
        &state,
        &principal,
        "discount_code.create",
        "discount_code",
        Some(row.id.to_string()),
        None,
        Some(&row),
    )
    .await?;

    Ok(Json(row))
}

pub async fn get_discount_code(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> Result<Json<PromotionCodeRow>, ApiError> {
    let principal = authenticate_admin(&state, &headers).await?;
    require_role(
        &principal,
        &[
            AdminRole::Viewer,
            AdminRole::Operator,
            AdminRole::Finance,
            AdminRole::Admin,
        ],
    )?;

    let row = state
        .db
        .get_discount_code_detail(id)
        .await
        .map_err(|err| ApiError::internal(format!("get discount code failed: {err}")))?
        .ok_or_else(|| ApiError::not_found("discount code not found"))?;

    Ok(Json(row))
}

#[derive(Debug, Deserialize)]
pub struct UpdateDiscountCodeRequest {
    pub status: Option<String>,
    pub discount_type: Option<String>,
    pub discount_value: Option<String>,
    pub max_discount_amount: Option<String>,
    pub max_total_uses: Option<i64>,
    pub max_uses_per_wallet: Option<i64>,
    pub first_purchase_only: Option<bool>,
    pub stacking_policy: Option<String>,
    pub applicable_chain_ids: Option<Vec<i64>>,
    pub applicable_ticket_levels: Option<Vec<i64>>,
    pub valid_from: Option<i64>,
    pub valid_until: Option<i64>,
    pub notes: Option<String>,
}

pub async fn update_discount_code(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<i64>,
    Json(req): Json<UpdateDiscountCodeRequest>,
) -> Result<Json<PromotionCodeRow>, ApiError> {
    let principal = authenticate_admin(&state, &headers).await?;
    require_role(&principal, &[AdminRole::Operator])?;
    if let Some(status) = req.status.as_deref() {
        validate_promotion_status(status)?;
    }
    if let (Some(discount_type), Some(discount_value)) =
        (req.discount_type.as_deref(), req.discount_value.as_deref())
    {
        validate_discount_value(discount_type, discount_value)?;
    }
    if let Some(max_discount_amount) = req.max_discount_amount.as_deref() {
        validate_human_token_amount(max_discount_amount)?;
    }

    let before = state
        .db
        .get_discount_code_detail(id)
        .await
        .map_err(|err| ApiError::internal(format!("get discount code failed: {err}")))?
        .ok_or_else(|| ApiError::not_found("discount code not found"))?;
    let applicable_chain_ids = serialize_json_scope(req.applicable_chain_ids)?;
    let applicable_ticket_levels = serialize_json_scope(req.applicable_ticket_levels)?;
    let row = state
        .db
        .update_discount_code(
            id,
            UpdateDiscountCode {
                status: req.status,
                discount_type: req.discount_type,
                discount_value: req.discount_value,
                max_discount_amount: req.max_discount_amount,
                max_total_uses: req.max_total_uses,
                max_uses_per_wallet: req.max_uses_per_wallet,
                first_purchase_only: req.first_purchase_only,
                stacking_policy: req.stacking_policy,
                applicable_chain_ids,
                applicable_ticket_levels,
                valid_from: req.valid_from,
                valid_until: req.valid_until,
                notes: req.notes,
            },
        )
        .await
        .map_err(|err| ApiError::internal(format!("update discount code failed: {err}")))?
        .ok_or_else(|| ApiError::not_found("discount code not found"))?;

    insert_admin_audit(
        &state,
        &principal,
        "discount_code.update",
        "discount_code",
        Some(row.id.to_string()),
        Some(&before),
        Some(&row),
    )
    .await?;

    Ok(Json(row))
}

pub async fn pause_discount_code(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> Result<Json<PromotionCodeRow>, ApiError> {
    set_discount_code_status(state, headers, id, "paused", "discount_code.pause").await
}

pub async fn activate_discount_code(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> Result<Json<PromotionCodeRow>, ApiError> {
    set_discount_code_status(state, headers, id, "active", "discount_code.activate").await
}

pub async fn list_redemption_codes(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<ListQuery>,
) -> Result<Json<RedemptionCodeListResponse>, ApiError> {
    require_viewer(&headers, &state).await?;
    let items = state
        .db
        .list_redemption_codes(query.page.unwrap_or(1), query.page_size.unwrap_or(50))
        .await
        .map_err(|err| ApiError::internal(format!("list redemption codes failed: {err}")))?;

    Ok(Json(RedemptionCodeListResponse { items }))
}

#[derive(Debug, Deserialize)]
pub struct CreateRedemptionCodeRequest {
    pub code: String,
    pub status: Option<String>,
    pub ticket_level: i64,
    pub valid_from: Option<i64>,
    pub valid_until: Option<i64>,
    pub notes: Option<String>,
}

pub async fn create_redemption_code(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<CreateRedemptionCodeRequest>,
) -> Result<Json<RedemptionCodeRow>, ApiError> {
    let principal = authenticate_admin(&state, &headers).await?;
    require_role(&principal, &[AdminRole::Operator])?;
    validate_ticket_level(req.ticket_level)?;
    let status = req.status.unwrap_or_else(|| "active".to_string());
    validate_redemption_status(&status)?;
    let code = normalize_new_promotion_code(&req.code).map_err(ApiError::bad_request)?;
    if state
        .db
        .find_redemption_code(&code)
        .await
        .map_err(|err| {
            ApiError::internal(format!("check duplicate redemption code failed: {err}"))
        })?
        .is_some()
    {
        return Err(ApiError::bad_request("redemption code already exists"));
    }

    let row = state
        .db
        .create_redemption_code(NewRedemptionCode {
            code,
            status,
            ticket_level: req.ticket_level,
            valid_from: req.valid_from,
            valid_until: req.valid_until,
            notes: req.notes,
            created_by: principal.wallet.clone(),
        })
        .await
        .map_err(|err| ApiError::bad_request(format!("create redemption code failed: {err}")))?;

    insert_admin_audit(
        &state,
        &principal,
        "redemption_code.create",
        "redemption_code",
        Some(row.id.to_string()),
        None,
        Some(&row),
    )
    .await?;

    Ok(Json(row))
}

#[derive(Debug, Deserialize)]
pub struct BulkCreateRedemptionCodesRequest {
    pub prefix: Option<String>,
    pub count: i64,
    pub ticket_level: i64,
    pub status: Option<String>,
    pub valid_from: Option<i64>,
    pub valid_until: Option<i64>,
    pub notes: Option<String>,
}

pub async fn bulk_create_redemption_codes(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<BulkCreateRedemptionCodesRequest>,
) -> Result<Json<RedemptionCodeListResponse>, ApiError> {
    let principal = authenticate_admin(&state, &headers).await?;
    require_role(&principal, &[AdminRole::Operator])?;
    validate_ticket_level(req.ticket_level)?;
    if !(1..=500).contains(&req.count) {
        return Err(ApiError::bad_request("bulk count must be within 1..=500"));
    }
    let status = req.status.unwrap_or_else(|| "active".to_string());
    validate_redemption_status(&status)?;
    let prefix = req
        .prefix
        .as_deref()
        .unwrap_or_default()
        .trim()
        .to_ascii_uppercase();
    if !prefix.is_empty()
        && !prefix
            .chars()
            .all(|ch| SAFE_PROMOTION_CODE_ALPHABET.contains(ch))
    {
        return Err(ApiError::bad_request(
            "redemption code prefix must avoid ambiguous characters",
        ));
    }
    if prefix.len() > PROMOTION_CODE_MAX_LEN - PROMOTION_CODE_MIN_LEN {
        return Err(ApiError::bad_request(
            "redemption code prefix must be 24 characters or shorter",
        ));
    }

    let mut items = Vec::with_capacity(req.count as usize);
    let mut attempts = 0;
    while items.len() < req.count as usize {
        attempts += 1;
        if attempts > req.count * 20 {
            return Err(ApiError::internal(
                "failed to generate unique redemption codes".to_string(),
            ));
        }
        let code = generate_redemption_code(&prefix);
        if state
            .db
            .find_redemption_code(&code)
            .await
            .map_err(|err| {
                ApiError::internal(format!("check duplicate redemption code failed: {err}"))
            })?
            .is_some()
        {
            continue;
        }
        let row = state
            .db
            .create_redemption_code(NewRedemptionCode {
                code,
                status: status.clone(),
                ticket_level: req.ticket_level,
                valid_from: req.valid_from,
                valid_until: req.valid_until,
                notes: req.notes.clone(),
                created_by: principal.wallet.clone(),
            })
            .await
            .map_err(|err| {
                ApiError::bad_request(format!("create redemption code failed: {err}"))
            })?;
        insert_admin_audit(
            &state,
            &principal,
            "redemption_code.create",
            "redemption_code",
            Some(row.id.to_string()),
            None,
            Some(&row),
        )
        .await?;
        items.push(row);
    }

    Ok(Json(RedemptionCodeListResponse { items }))
}

pub async fn get_redemption_code(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> Result<Json<RedemptionCodeRow>, ApiError> {
    require_viewer(&headers, &state).await?;
    let row = state
        .db
        .get_redemption_code_detail(id)
        .await
        .map_err(|err| ApiError::internal(format!("get redemption code failed: {err}")))?
        .ok_or_else(|| ApiError::not_found("redemption code not found"))?;

    Ok(Json(row))
}

#[derive(Debug, Deserialize)]
pub struct UpdateRedemptionCodeRequest {
    pub status: Option<String>,
    pub ticket_level: Option<i64>,
    pub valid_from: Option<i64>,
    pub valid_until: Option<i64>,
    pub notes: Option<String>,
}

pub async fn update_redemption_code(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<i64>,
    Json(req): Json<UpdateRedemptionCodeRequest>,
) -> Result<Json<RedemptionCodeRow>, ApiError> {
    let principal = authenticate_admin(&state, &headers).await?;
    require_role(&principal, &[AdminRole::Operator])?;
    if let Some(status) = req.status.as_deref() {
        validate_redemption_status(status)?;
    }
    if let Some(ticket_level) = req.ticket_level {
        validate_ticket_level(ticket_level)?;
    }
    let before = state
        .db
        .get_redemption_code_detail(id)
        .await
        .map_err(|err| ApiError::internal(format!("get redemption code failed: {err}")))?
        .ok_or_else(|| ApiError::not_found("redemption code not found"))?;
    if before.status == "redeemed" {
        return Err(ApiError::bad_request("redeemed codes cannot be edited"));
    }

    let row = state
        .db
        .update_redemption_code(
            id,
            UpdateRedemptionCode {
                status: req.status,
                ticket_level: req.ticket_level,
                valid_from: req.valid_from,
                valid_until: req.valid_until,
                notes: req.notes,
                updated_by: principal.wallet.clone(),
            },
        )
        .await
        .map_err(|err| ApiError::internal(format!("update redemption code failed: {err}")))?
        .ok_or_else(|| ApiError::not_found("redemption code not found"))?;

    insert_admin_audit(
        &state,
        &principal,
        "redemption_code.update",
        "redemption_code",
        Some(row.id.to_string()),
        Some(&before),
        Some(&row),
    )
    .await?;

    Ok(Json(row))
}

pub async fn pause_redemption_code(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> Result<Json<RedemptionCodeRow>, ApiError> {
    set_redemption_code_status(state, headers, id, "paused", "redemption_code.pause").await
}

pub async fn activate_redemption_code(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> Result<Json<RedemptionCodeRow>, ApiError> {
    set_redemption_code_status(state, headers, id, "active", "redemption_code.activate").await
}

pub async fn get_redemption_code_stats(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<RedemptionCodeStatsRow>, ApiError> {
    require_viewer(&headers, &state).await?;
    let row =
        state.db.get_redemption_code_stats().await.map_err(|err| {
            ApiError::internal(format!("get redemption code stats failed: {err}"))
        })?;
    Ok(Json(row))
}

pub async fn list_redemption_code_claims(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<ListQuery>,
) -> Result<Json<RedemptionClaimListResponse>, ApiError> {
    require_viewer(&headers, &state).await?;
    let items = state
        .db
        .list_redemption_code_claims(None, query.page.unwrap_or(1), query.page_size.unwrap_or(50))
        .await
        .map_err(|err| ApiError::internal(format!("list redemption claims failed: {err}")))?;
    Ok(Json(RedemptionClaimListResponse { items }))
}

pub async fn list_redemption_code_claims_for_code(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<i64>,
    Query(query): Query<ListQuery>,
) -> Result<Json<RedemptionClaimListResponse>, ApiError> {
    require_viewer(&headers, &state).await?;
    let items = state
        .db
        .list_redemption_code_claims(
            Some(id),
            query.page.unwrap_or(1),
            query.page_size.unwrap_or(50),
        )
        .await
        .map_err(|err| ApiError::internal(format!("list redemption claims failed: {err}")))?;
    Ok(Json(RedemptionClaimListResponse { items }))
}

pub async fn list_referral_bindings(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<ListQuery>,
) -> Result<Json<ReferralBindingListResponse>, ApiError> {
    require_viewer(&headers, &state).await?;
    let items = state
        .db
        .list_referral_bindings(query.page.unwrap_or(1), query.page_size.unwrap_or(50))
        .await
        .map_err(|err| ApiError::internal(format!("list referral bindings failed: {err}")))?;

    Ok(Json(ReferralBindingListResponse { items }))
}

#[derive(Debug, Deserialize)]
pub struct PurchaseIntentQuery {
    pub page: Option<i64>,
    pub page_size: Option<i64>,
    pub wallet: Option<String>,
    pub tx_hash: Option<String>,
    pub status: Option<String>,
    pub code: Option<String>,
}

pub async fn list_purchase_intents(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<PurchaseIntentQuery>,
) -> Result<Json<PurchaseIntentListResponse>, ApiError> {
    require_viewer(&headers, &state).await?;
    let items = state
        .db
        .list_purchase_intents_admin(
            PurchaseIntentFilters {
                wallet: query.wallet,
                tx_hash: query.tx_hash,
                status: query.status,
                code: query.code,
            },
            query.page.unwrap_or(1),
            query.page_size.unwrap_or(50),
        )
        .await
        .map_err(|err| ApiError::internal(format!("list purchase intents failed: {err}")))?;

    Ok(Json(PurchaseIntentListResponse { items }))
}

pub async fn get_purchase_intent_diagnostic(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<PurchaseIntentDiagnostic>, ApiError> {
    require_viewer(&headers, &state).await?;
    let diagnostic = state
        .db
        .get_purchase_intent_diagnostic(&id)
        .await
        .map_err(|err| ApiError::internal(format!("get purchase intent failed: {err}")))?
        .ok_or_else(|| ApiError::not_found("purchase intent not found"))?;

    Ok(Json(diagnostic))
}

#[derive(Debug, Deserialize)]
pub struct OrderQuery {
    pub page: Option<i64>,
    pub page_size: Option<i64>,
    pub wallet: Option<String>,
    pub tx_hash: Option<String>,
    pub invite_code: Option<String>,
    pub discount_code: Option<String>,
}

pub async fn list_orders(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<OrderQuery>,
) -> Result<Json<OrderListResponse>, ApiError> {
    require_viewer(&headers, &state).await?;
    let items = state
        .db
        .list_orders_admin(
            OrderFilters {
                wallet: query.wallet,
                tx_hash: query.tx_hash,
                invite_code: query.invite_code,
                discount_code: query.discount_code,
            },
            query.page.unwrap_or(1),
            query.page_size.unwrap_or(50),
        )
        .await
        .map_err(|err| ApiError::internal(format!("list orders failed: {err}")))?;

    Ok(Json(OrderListResponse { items }))
}

pub async fn get_order_attribution(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> Result<Json<OrderAttributionDiagnostic>, ApiError> {
    require_viewer(&headers, &state).await?;
    let diagnostic = state
        .db
        .get_order_attribution(id)
        .await
        .map_err(|err| ApiError::internal(format!("get order attribution failed: {err}")))?
        .ok_or_else(|| ApiError::not_found("order not found"))?;

    Ok(Json(diagnostic))
}

pub async fn list_referral_settlements(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<ReferralSettlementListResponse>, ApiError> {
    let principal = authenticate_admin(&state, &headers).await?;
    require_role(&principal, &[AdminRole::Finance])?;
    let items =
        state.db.list_referral_settlements().await.map_err(|err| {
            ApiError::internal(format!("list referral settlements failed: {err}"))
        })?;

    Ok(Json(ReferralSettlementListResponse { items }))
}

pub async fn download_referral_settlements_csv(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let principal = authenticate_admin(&state, &headers).await?;
    require_role(&principal, &[AdminRole::Finance])?;
    let rows =
        state.db.list_referral_settlements().await.map_err(|err| {
            ApiError::internal(format!("list referral settlements failed: {err}"))
        })?;

    let mut csv = String::from(
        "invite_code_id,invite_code,beneficiary_wallet,confirmed_order_count,paid_amount_total,commission_base_amount_total,commission_amount_total\n",
    );
    for row in rows {
        csv.push_str(&format!(
            "{},{},{},{},{},{},{}\n",
            row.invite_code_id,
            csv_escape(&row.invite_code),
            csv_escape(row.beneficiary_wallet.as_deref().unwrap_or("")),
            row.confirmed_order_count,
            row.paid_amount_total,
            row.commission_base_amount_total,
            row.commission_amount_total
        ));
    }

    Ok((
        [
            (header::CONTENT_TYPE, "text/csv; charset=utf-8"),
            (
                header::CONTENT_DISPOSITION,
                "attachment; filename=\"referral-settlements.csv\"",
            ),
        ],
        csv,
    )
        .into_response())
}

pub async fn list_audit_logs(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<ListQuery>,
) -> Result<Json<AdminAuditLogListResponse>, ApiError> {
    let principal = authenticate_admin(&state, &headers).await?;
    require_role(&principal, &[AdminRole::Admin])?;
    let items = state
        .db
        .list_admin_audit_logs(query.page.unwrap_or(1), query.page_size.unwrap_or(50))
        .await
        .map_err(|err| ApiError::internal(format!("list audit logs failed: {err}")))?;

    Ok(Json(AdminAuditLogListResponse { items }))
}

#[derive(Debug, Serialize)]
pub struct AdminWalletListResponse {
    pub items: Vec<AdminWalletRow>,
}

#[derive(Debug, Deserialize)]
pub struct CreateAdminWalletRequest {
    pub wallet_address: String,
    pub role: String,
    pub status: Option<String>,
    pub notes: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateAdminWalletRequest {
    pub role: Option<String>,
    pub status: Option<String>,
    pub notes: Option<String>,
}

pub async fn list_admin_wallets(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<AdminWalletListResponse>, ApiError> {
    let principal = authenticate_admin(&state, &headers).await?;
    require_role(&principal, &[AdminRole::Admin])?;

    let items = state
        .db
        .list_admin_wallets()
        .await
        .map_err(|err| ApiError::internal(format!("list admin wallets failed: {err}")))?;

    Ok(Json(AdminWalletListResponse { items }))
}

pub async fn get_admin_wallet(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> Result<Json<AdminWalletRow>, ApiError> {
    let principal = authenticate_admin(&state, &headers).await?;
    require_role(&principal, &[AdminRole::Admin])?;

    let row = state
        .db
        .get_admin_wallet(id)
        .await
        .map_err(|err| ApiError::internal(format!("get admin wallet failed: {err}")))?
        .ok_or_else(|| ApiError::not_found("admin wallet not found"))?;

    Ok(Json(row))
}

pub async fn create_admin_wallet(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<CreateAdminWalletRequest>,
) -> Result<Json<AdminWalletRow>, ApiError> {
    let principal = authenticate_admin(&state, &headers).await?;
    require_role(&principal, &[AdminRole::Admin])?;
    validate_db_admin_role(&req.role)?;
    let status = req.status.unwrap_or_else(|| "active".to_string());
    validate_admin_wallet_status(&status)?;
    let wallet_address = normalize_wallet_address(&req.wallet_address)?;
    if state
        .db
        .find_admin_wallet_by_address(&wallet_address)
        .await
        .map_err(|err| ApiError::internal(format!("check admin wallet failed: {err}")))?
        .is_some()
    {
        return Err(ApiError::bad_request("admin wallet already exists"));
    }

    let row = state
        .db
        .create_admin_wallet(NewAdminWallet {
            wallet_address,
            role: req.role,
            status,
            notes: req.notes,
            created_by: principal.wallet.clone(),
        })
        .await
        .map_err(|err| ApiError::bad_request(format!("create admin wallet failed: {err}")))?;

    insert_admin_audit(
        &state,
        &principal,
        "admin_wallet.create",
        "admin_wallet",
        Some(row.id.to_string()),
        None,
        Some(&row),
    )
    .await?;

    Ok(Json(row))
}

pub async fn update_admin_wallet(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<i64>,
    Json(req): Json<UpdateAdminWalletRequest>,
) -> Result<Json<AdminWalletRow>, ApiError> {
    let principal = authenticate_admin(&state, &headers).await?;
    require_role(&principal, &[AdminRole::Admin])?;
    if let Some(role) = req.role.as_deref() {
        validate_db_admin_role(role)?;
    }
    if let Some(status) = req.status.as_deref() {
        validate_admin_wallet_status(status)?;
    }
    let before = state
        .db
        .get_admin_wallet(id)
        .await
        .map_err(|err| ApiError::internal(format!("get admin wallet failed: {err}")))?
        .ok_or_else(|| ApiError::not_found("admin wallet not found"))?;

    let row = state
        .db
        .update_admin_wallet(
            id,
            UpdateAdminWallet {
                role: req.role,
                status: req.status,
                notes: req.notes,
                updated_by: principal.wallet.clone(),
            },
        )
        .await
        .map_err(|err| ApiError::internal(format!("update admin wallet failed: {err}")))?
        .ok_or_else(|| ApiError::not_found("admin wallet not found"))?;

    insert_admin_audit(
        &state,
        &principal,
        "admin_wallet.update",
        "admin_wallet",
        Some(row.id.to_string()),
        Some(&before),
        Some(&row),
    )
    .await?;

    Ok(Json(row))
}

pub async fn delete_admin_wallet(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> Result<Json<AdminWalletRow>, ApiError> {
    let principal = authenticate_admin(&state, &headers).await?;
    require_role(&principal, &[AdminRole::Admin])?;
    let before = state
        .db
        .delete_admin_wallet(id)
        .await
        .map_err(|err| ApiError::internal(format!("delete admin wallet failed: {err}")))?
        .ok_or_else(|| ApiError::not_found("admin wallet not found"))?;

    insert_admin_audit(
        &state,
        &principal,
        "admin_wallet.delete",
        "admin_wallet",
        Some(before.id.to_string()),
        Some(&before),
        None::<&AdminWalletRow>,
    )
    .await?;

    Ok(Json(before))
}

async fn set_invite_code_status(
    state: Arc<AppState>,
    headers: HeaderMap,
    id: i64,
    status: &str,
    action: &str,
) -> Result<Json<PromotionCodeRow>, ApiError> {
    let principal = authenticate_admin(&state, &headers).await?;
    require_role(&principal, &[AdminRole::Operator])?;
    let before = state
        .db
        .get_invite_code_detail(id)
        .await
        .map_err(|err| ApiError::internal(format!("get invite code failed: {err}")))?
        .ok_or_else(|| ApiError::not_found("invite code not found"))?;
    let row = state
        .db
        .set_invite_code_status(id, status)
        .await
        .map_err(|err| ApiError::internal(format!("set invite code status failed: {err}")))?
        .ok_or_else(|| ApiError::not_found("invite code not found"))?;

    insert_admin_audit(
        &state,
        &principal,
        action,
        "invite_code",
        Some(row.id.to_string()),
        Some(&before),
        Some(&row),
    )
    .await?;

    Ok(Json(row))
}

async fn set_discount_code_status(
    state: Arc<AppState>,
    headers: HeaderMap,
    id: i64,
    status: &str,
    action: &str,
) -> Result<Json<PromotionCodeRow>, ApiError> {
    let principal = authenticate_admin(&state, &headers).await?;
    require_role(&principal, &[AdminRole::Operator])?;
    let before = state
        .db
        .get_discount_code_detail(id)
        .await
        .map_err(|err| ApiError::internal(format!("get discount code failed: {err}")))?
        .ok_or_else(|| ApiError::not_found("discount code not found"))?;
    let row = state
        .db
        .set_discount_code_status(id, status)
        .await
        .map_err(|err| ApiError::internal(format!("set discount code status failed: {err}")))?
        .ok_or_else(|| ApiError::not_found("discount code not found"))?;

    insert_admin_audit(
        &state,
        &principal,
        action,
        "discount_code",
        Some(row.id.to_string()),
        Some(&before),
        Some(&row),
    )
    .await?;

    Ok(Json(row))
}

async fn set_redemption_code_status(
    state: Arc<AppState>,
    headers: HeaderMap,
    id: i64,
    status: &str,
    action: &str,
) -> Result<Json<RedemptionCodeRow>, ApiError> {
    let principal = authenticate_admin(&state, &headers).await?;
    require_role(&principal, &[AdminRole::Operator])?;
    let before = state
        .db
        .get_redemption_code_detail(id)
        .await
        .map_err(|err| ApiError::internal(format!("get redemption code failed: {err}")))?
        .ok_or_else(|| ApiError::not_found("redemption code not found"))?;
    if before.status == "redeemed" {
        return Err(ApiError::bad_request("redeemed codes cannot change status"));
    }
    let row = state
        .db
        .set_redemption_code_status(id, status, &principal.wallet)
        .await
        .map_err(|err| ApiError::internal(format!("set redemption code status failed: {err}")))?
        .ok_or_else(|| ApiError::not_found("redemption code not found"))?;

    insert_admin_audit(
        &state,
        &principal,
        action,
        "redemption_code",
        Some(row.id.to_string()),
        Some(&before),
        Some(&row),
    )
    .await?;

    Ok(Json(row))
}

fn validate_promotion_status(status: &str) -> Result<(), ApiError> {
    if matches!(status, "active" | "paused" | "expired" | "exhausted") {
        return Ok(());
    }

    Err(ApiError::bad_request("invalid promotion code status"))
}

fn validate_redemption_status(status: &str) -> Result<(), ApiError> {
    if matches!(status, "active" | "paused" | "expired") {
        return Ok(());
    }

    Err(ApiError::bad_request("invalid redemption code status"))
}

fn validate_ticket_level(ticket_level: i64) -> Result<(), ApiError> {
    if matches!(ticket_level, 1 | 2 | 3) {
        return Ok(());
    }

    Err(ApiError::bad_request("ticket level must be 1, 2, or 3"))
}

fn generate_redemption_code(prefix: &str) -> String {
    let alphabet = SAFE_PROMOTION_CODE_ALPHABET.as_bytes();
    let mut code = prefix.to_string();
    let target_len = (prefix.len() + PROMOTION_CODE_MIN_LEN)
        .clamp(PROMOTION_CODE_MIN_LEN, PROMOTION_CODE_MAX_LEN);
    while code.len() < target_len {
        let random = uuid::Uuid::new_v4();
        for byte in random.as_bytes() {
            if code.len() >= target_len {
                break;
            }
            let index = *byte as usize % alphabet.len();
            code.push(alphabet[index] as char);
        }
    }
    code
}

fn normalize_optional_wallet_address(value: Option<&str>) -> Result<Option<String>, ApiError> {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };

    normalize_wallet_address(value).map(Some)
}

fn validate_db_admin_role(role: &str) -> Result<(), ApiError> {
    if matches!(role, "viewer" | "operator" | "finance") {
        return Ok(());
    }

    Err(ApiError::bad_request(
        "admin wallet role must be viewer, operator, or finance",
    ))
}

fn validate_admin_wallet_status(status: &str) -> Result<(), ApiError> {
    if matches!(status, "active" | "disabled") {
        return Ok(());
    }

    Err(ApiError::bad_request(
        "admin wallet status must be active or disabled",
    ))
}

fn csv_escape(value: &str) -> String {
    if value
        .chars()
        .any(|ch| matches!(ch, ',' | '"' | '\n' | '\r'))
    {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

async fn require_viewer(headers: &HeaderMap, state: &Arc<AppState>) -> Result<(), ApiError> {
    let principal = authenticate_admin(state, headers).await?;
    require_role(
        &principal,
        &[
            AdminRole::Viewer,
            AdminRole::Operator,
            AdminRole::Finance,
            AdminRole::Admin,
        ],
    )
}

fn validate_discount_value(discount_type: &str, discount_value: &str) -> Result<(), ApiError> {
    match discount_type {
        "fixed" => validate_human_token_amount(discount_value),
        "percentage" => {
            let value = discount_value
                .parse::<i64>()
                .map_err(|_| ApiError::bad_request("percentage discount must be basis points"))?;
            if (1..=10_000).contains(&value) {
                Ok(())
            } else {
                Err(ApiError::bad_request(
                    "percentage discount must be within 1..=10000 basis points",
                ))
            }
        }
        _ => Err(ApiError::bad_request("invalid discount type")),
    }
}

fn validate_optional_discount_value(
    discount_type: Option<&str>,
    discount_value: Option<&str>,
) -> Result<(), ApiError> {
    match (discount_type, discount_value) {
        (Some(""), Some("")) => Ok(()),
        (Some(discount_type), Some(discount_value)) => {
            validate_discount_value(discount_type, discount_value)
        }
        (Some(_), None) => Err(ApiError::bad_request(
            "discount value is required when discount type is selected",
        )),
        (None, Some(_)) => Err(ApiError::bad_request(
            "discount type is required when discount value is selected",
        )),
        (None, None) => Ok(()),
    }
}

fn validate_human_token_amount(value: &str) -> Result<(), ApiError> {
    let normalized = value.trim();
    if normalized.is_empty() || normalized.starts_with('-') {
        return Err(ApiError::bad_request(
            "fixed discount requires a non-negative token amount",
        ));
    }

    let parts = normalized.split('.').collect::<Vec<_>>();
    if parts.len() > 2 || parts.iter().all(|part| part.is_empty()) {
        return Err(ApiError::bad_request(
            "fixed discount requires a non-negative token amount",
        ));
    }

    if parts
        .iter()
        .all(|part| part.chars().all(|ch| ch.is_ascii_digit()))
    {
        return Ok(());
    }

    Err(ApiError::bad_request(
        "fixed discount requires a non-negative token amount",
    ))
}

fn serialize_json_scope<T: Serialize>(value: Option<T>) -> Result<Option<String>, ApiError> {
    value
        .map(|value| {
            serde_json::to_string(&value)
                .map_err(|err| ApiError::internal(format!("serialize scope failed: {err}")))
        })
        .transpose()
}

async fn insert_admin_audit<T: Serialize>(
    state: &Arc<AppState>,
    principal: &crate::admin::AdminPrincipal,
    action: &str,
    target_type: &str,
    target_id: Option<String>,
    before: Option<&T>,
    after: Option<&T>,
) -> Result<(), ApiError> {
    let before_json = before
        .map(serde_json::to_string)
        .transpose()
        .map_err(|err| ApiError::internal(format!("serialize audit before failed: {err}")))?;
    let after_json = after
        .map(serde_json::to_string)
        .transpose()
        .map_err(|err| ApiError::internal(format!("serialize audit after failed: {err}")))?;

    state
        .db
        .insert_admin_audit_log(NewAdminAuditLog {
            actor_wallet: principal.wallet.clone(),
            actor_role: principal.role.as_str().to_string(),
            action: action.to_string(),
            target_type: target_type.to_string(),
            target_id,
            before_json,
            after_json,
            ip_address: None,
            user_agent: None,
        })
        .await
        .map_err(|err| ApiError::internal(format!("insert admin audit failed: {err}")))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use async_trait::async_trait;
    use axum::{
        body::{to_bytes, Body},
        http::{Method, Request, StatusCode},
        Router,
    };
    use ethers_signers::{LocalWallet, Signer};
    use serde_json::{json, Value};
    use tower::util::ServiceExt;

    use crate::{
        auth::JwtCodec,
        chain::{ChainReader, ChainRuntimeConfig, DecodedPurchase, QuoteResult},
        config::AppConfig,
        db::{Db, UpdateInviteCode},
        mailer::Mailer,
        promotions::{
            DiscountRedemptionStatus, NewDiscountRedemption, NewOrderPromotionsSnapshot,
            NewPurchaseIntent, PurchaseIntentStatus,
        },
        AppState,
    };

    struct NoopChain {
        default_admin_wallets: Vec<String>,
    }

    #[derive(Debug, Clone)]
    struct TestAdminWallet {
        wallet: String,
        role: String,
    }

    #[async_trait]
    impl ChainReader for NoopChain {
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
            _chain_id: u64,
            _tx_hash: &str,
        ) -> anyhow::Result<Vec<DecodedPurchase>> {
            Ok(Vec::new())
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
            _chain_id: u64,
            _level_ids: &[u8],
            _quantities: &[u64],
        ) -> anyhow::Result<QuoteResult> {
            anyhow::bail!("noop chain does not quote")
        }

        async fn has_default_admin_role(&self, wallet: &str) -> anyhow::Result<bool> {
            Ok(self
                .default_admin_wallets
                .iter()
                .any(|admin_wallet| admin_wallet == wallet))
        }
    }

    async fn build_test_app(admin_wallets: Vec<TestAdminWallet>) -> (Router, Arc<AppState>) {
        let mut db_wallets = Vec::new();
        let mut chain_admins = Vec::new();
        for wallet in admin_wallets {
            if wallet.role == "admin" {
                chain_admins.push(wallet.wallet);
            } else {
                db_wallets.push(wallet);
            }
        }
        build_test_app_with_chain_admins(db_wallets, chain_admins).await
    }

    async fn build_test_app_with_chain_admins(
        admin_wallets: Vec<TestAdminWallet>,
        default_admin_wallets: Vec<String>,
    ) -> (Router, Arc<AppState>) {
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
            chains: Vec::new(),
            indexer_poll_interval_secs: 5,
            indexer_batch_size: 50,
            indexer_reorg_rollback_blocks: 64,
            signin_challenge_ttl_secs: 300,
            signin_cleanup_interval_secs: 600,
            signin_cleanup_retention_secs: 86400,
            purchase_intent_ttl_secs: 900,
            purchase_signer_private_key: None,
            admin_jwt_ttl_hours: 12,
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
            chain: Arc::new(NoopChain {
                default_admin_wallets,
            }),
            jwt,
            mailer,
            purchase_signer: None,
        });

        for wallet in admin_wallets {
            state
                .db
                .create_admin_wallet(crate::db::NewAdminWallet {
                    wallet_address: wallet.wallet,
                    role: wallet.role,
                    status: "active".to_string(),
                    notes: Some("test seed".to_string()),
                    created_by: "0x0000000000000000000000000000000000000000".to_string(),
                })
                .await
                .expect("test admin wallet should seed");
        }

        (super::router().with_state(state.clone()), state)
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

    async fn text_request(
        app: &Router,
        method: Method,
        path: &str,
        bearer_token: Option<&str>,
    ) -> (StatusCode, String) {
        let mut req_builder = Request::builder()
            .method(method)
            .uri(path)
            .header("content-type", "application/json");

        if let Some(token) = bearer_token {
            req_builder = req_builder.header("authorization", format!("Bearer {token}"));
        }

        let req = req_builder
            .body(Body::empty())
            .expect("request should build");
        let response = app
            .clone()
            .oneshot(req)
            .await
            .expect("request should succeed");
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("response body should read");
        (
            status,
            String::from_utf8(bytes.to_vec()).expect("text response expected"),
        )
    }

    fn admin_config(wallet: &str, role: &str) -> TestAdminWallet {
        TestAdminWallet {
            wallet: wallet.to_string(),
            role: role.to_string(),
        }
    }

    fn admin_token(state: &AppState, wallet: &str, role: &str) -> String {
        state
            .jwt
            .issue_admin(wallet, role, 12)
            .expect("admin jwt should issue")
            .0
    }

    #[tokio::test]
    async fn admin_auth_flow_allowlisted_wallet_can_get_challenge_and_verify_signature() {
        let wallet: LocalWallet =
            "0x59c6995e998f97a5a0044966f09453880a61fdbf87f6ea0f0f8a7ecf7f5f91f7"
                .parse()
                .expect("wallet parse should succeed");
        let wallet_address = format!("{:#x}", wallet.address());
        let (app, _state) = build_test_app(vec![admin_config(&wallet_address, "operator")]).await;

        let (status, challenge_body) = json_request(
            &app,
            Method::POST,
            "/auth/challenge",
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
        assert!(challenge_message.starts_with("Admin Sign-In"));

        let signature = wallet
            .sign_message(challenge_message.to_string())
            .await
            .expect("message signing should succeed");

        let (status, verify_body) = json_request(
            &app,
            Method::POST,
            "/auth/verify",
            None,
            Some(json!({
                "address": wallet_address,
                "challenge_id": challenge_id,
                "signature": signature.to_string()
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(verify_body["wallet"], wallet_address);
        assert_eq!(verify_body["role"], "operator");
        assert!(verify_body["token"].as_str().is_some());
    }

    #[tokio::test]
    async fn admin_auth_flow_chain_default_admin_gets_admin_role_without_allowlist() {
        let wallet: LocalWallet =
            "0x59c6995e998f97a5a0044966f09453880a61fdbf87f6ea0f0f8a7ecf7f5f91f7"
                .parse()
                .expect("wallet parse should succeed");
        let wallet_address = format!("{:#x}", wallet.address());
        let (app, _state) =
            build_test_app_with_chain_admins(Vec::new(), vec![wallet_address.clone()]).await;

        let (status, challenge_body) = json_request(
            &app,
            Method::POST,
            "/auth/challenge",
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

        let (status, verify_body) = json_request(
            &app,
            Method::POST,
            "/auth/verify",
            None,
            Some(json!({
                "address": wallet_address,
                "challenge_id": challenge_id,
                "signature": signature.to_string()
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(verify_body["wallet"], wallet_address);
        assert_eq!(verify_body["role"], "admin");
        assert!(verify_body["token"].as_str().is_some());
    }

    #[tokio::test]
    async fn admin_auth_flow_non_allowlisted_wallet_gets_403_on_verify() {
        let wallet: LocalWallet =
            "0x8b3a350cf5c34c9194ca3a545d4d2ce7d9f69b17a3b2ecfacac4f2d0f6f7f204"
                .parse()
                .expect("wallet parse should succeed");
        let wallet_address = format!("{:#x}", wallet.address());
        let (app, _state) = build_test_app(Vec::new()).await;

        let (status, challenge_body) = json_request(
            &app,
            Method::POST,
            "/auth/challenge",
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

        let (status, _body) = json_request(
            &app,
            Method::POST,
            "/auth/verify",
            None,
            Some(json!({
                "address": wallet_address,
                "challenge_id": challenge_id,
                "signature": signature.to_string()
            })),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn admin_auth_flow_db_wallet_can_login_and_disabled_wallet_token_is_rejected() {
        let wallet: LocalWallet =
            "0x59c6995e998f97a5a0044966f09453880a61fdbf87f6ea0f0f8a7ecf7f5f91f7"
                .parse()
                .expect("wallet parse should succeed");
        let wallet_address = format!("{:#x}", wallet.address());
        let (app, state) = build_test_app(Vec::new()).await;
        state
            .db
            .create_admin_wallet(crate::db::NewAdminWallet {
                wallet_address: wallet_address.clone(),
                role: "operator".to_string(),
                status: "active".to_string(),
                notes: Some("ops".to_string()),
                created_by: "0x0000000000000000000000000000000000000000".to_string(),
            })
            .await
            .expect("admin wallet should insert");

        let (status, challenge_body) = json_request(
            &app,
            Method::POST,
            "/auth/challenge",
            None,
            Some(json!({ "address": wallet_address })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let challenge_id = challenge_body["challenge_id"].as_str().unwrap();
        let challenge_message = challenge_body["challenge_message"].as_str().unwrap();
        let signature = wallet
            .sign_message(challenge_message.to_string())
            .await
            .expect("message signing should succeed");

        let (status, verify_body) = json_request(
            &app,
            Method::POST,
            "/auth/verify",
            None,
            Some(json!({
                "address": wallet_address,
                "challenge_id": challenge_id,
                "signature": signature.to_string()
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(verify_body["role"], "operator");
        let token = verify_body["token"].as_str().unwrap();

        let (status, _body) = json_request(&app, Method::GET, "/me", Some(token), None).await;
        assert_eq!(status, StatusCode::OK);

        let row = state
            .db
            .find_admin_wallet_by_address(&wallet_address)
            .await
            .expect("admin wallet lookup should succeed")
            .expect("admin wallet should exist");
        state
            .db
            .update_admin_wallet(
                row.id,
                crate::db::UpdateAdminWallet {
                    role: None,
                    status: Some("disabled".to_string()),
                    notes: None,
                    updated_by: "0x0000000000000000000000000000000000000000".to_string(),
                },
            )
            .await
            .expect("admin wallet should update");

        let (status, _body) = json_request(&app, Method::GET, "/me", Some(token), None).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn admin_wallet_management_is_chain_admin_only_and_rejects_admin_role() {
        let admin_wallet = "0x1111111111111111111111111111111111111111";
        let operator_wallet = "0x2222222222222222222222222222222222222222";
        let managed_wallet = "0x3333333333333333333333333333333333333333";
        let (app, state) =
            build_test_app_with_chain_admins(Vec::new(), vec![admin_wallet.to_string()]).await;
        state
            .db
            .create_admin_wallet(crate::db::NewAdminWallet {
                wallet_address: operator_wallet.to_string(),
                role: "operator".to_string(),
                status: "active".to_string(),
                notes: None,
                created_by: admin_wallet.to_string(),
            })
            .await
            .expect("operator wallet should insert");
        let chain_admin_token = admin_token(&state, admin_wallet, "admin");
        let operator_token = admin_token(&state, operator_wallet, "operator");

        let (status, _body) = json_request(
            &app,
            Method::POST,
            "/admin-wallets",
            Some(&operator_token),
            Some(json!({
                "wallet_address": managed_wallet,
                "role": "viewer",
                "status": "active"
            })),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);

        let (status, _body) = json_request(
            &app,
            Method::POST,
            "/admin-wallets",
            Some(&chain_admin_token),
            Some(json!({
                "wallet_address": managed_wallet,
                "role": "admin",
                "status": "active"
            })),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);

        let (status, created) = json_request(
            &app,
            Method::POST,
            "/admin-wallets",
            Some(&chain_admin_token),
            Some(json!({
                "wallet_address": managed_wallet,
                "role": "viewer",
                "status": "active",
                "notes": "support"
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(created["wallet_address"], managed_wallet);
        assert_eq!(created["role"], "viewer");

        let (status, list) = json_request(
            &app,
            Method::GET,
            "/admin-wallets",
            Some(&chain_admin_token),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(list["items"].as_array().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn admin_auth_flow_buyer_jwt_gets_401_on_me() {
        let wallet = "0x1111111111111111111111111111111111111111";
        let (app, state) = build_test_app(vec![admin_config(wallet, "admin")]).await;
        let (buyer_token, _) = state.jwt.issue(wallet).expect("buyer jwt should issue");

        let (status, _body) =
            json_request(&app, Method::GET, "/me", Some(&buyer_token), None).await;

        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn admin_auth_flow_admin_jwt_gets_200_on_me() {
        let wallet = "0x1111111111111111111111111111111111111111";
        let (app, state) = build_test_app(vec![admin_config(wallet, "admin")]).await;
        let (admin_token, _) = state
            .jwt
            .issue_admin(wallet, "admin", 12)
            .expect("admin jwt should issue");

        let (status, body) = json_request(&app, Method::GET, "/me", Some(&admin_token), None).await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["wallet"], wallet);
        assert_eq!(body["role"], "admin");
    }

    #[tokio::test]
    async fn admin_invite_codes_operator_can_create_and_list_referral_only() {
        let wallet = "0x1111111111111111111111111111111111111111";
        let (app, state) = build_test_app(vec![admin_config(wallet, "operator")]).await;
        let token = admin_token(&state, wallet, "operator");
        state
            .db
            .seed_discount_code("SAVE10")
            .await
            .expect("discount seed should succeed");

        let (status, body) = json_request(
            &app,
            Method::POST,
            "/invite-codes",
            Some(&token),
            Some(json!({
                "code": "partnerx",
                "beneficiary_wallet": "0x2222222222222222222222222222222222222222",
                "status": "active",
                "commission_type": "percentage",
                "commission_value": "1000",
                "notes": "launch partner"
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["code_normalized"], "PARTNERX");
        assert_eq!(body["kind"], "referral");
        assert_eq!(body["notes"], "launch partner");

        let (status, list_body) =
            json_request(&app, Method::GET, "/invite-codes", Some(&token), None).await;
        assert_eq!(status, StatusCode::OK);
        let items = list_body["items"]
            .as_array()
            .expect("items should be array");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["code_normalized"], "PARTNERX");
        assert_eq!(items[0]["kind"], "referral");
    }

    #[tokio::test]
    async fn admin_invite_codes_reject_duplicate_code_and_invalid_beneficiary_wallet() {
        let wallet = "0x1111111111111111111111111111111111111111";
        let (app, state) = build_test_app(vec![admin_config(wallet, "operator")]).await;
        let token = admin_token(&state, wallet, "operator");

        let (status, _body) = json_request(
            &app,
            Method::POST,
            "/invite-codes",
            Some(&token),
            Some(json!({
                "code": "dupe2345",
                "beneficiary_wallet": "0x2222222222222222222222222222222222222222",
                "status": "active"
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        let (status, _body) = json_request(
            &app,
            Method::POST,
            "/invite-codes",
            Some(&token),
            Some(json!({
                "code": "DUPE2345",
                "beneficiary_wallet": "0x2222222222222222222222222222222222222222",
                "status": "active"
            })),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);

        let (status, _body) = json_request(
            &app,
            Method::POST,
            "/invite-codes",
            Some(&token),
            Some(json!({
                "code": "BADWAT23",
                "beneficiary_wallet": "not-a-wallet",
                "status": "active"
            })),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn admin_invite_codes_allow_missing_beneficiary_and_later_update() {
        let wallet = "0x1111111111111111111111111111111111111111";
        let (app, state) = build_test_app(vec![admin_config(wallet, "operator")]).await;
        let token = admin_token(&state, wallet, "operator");

        let (status, body) = json_request(
            &app,
            Method::POST,
            "/invite-codes",
            Some(&token),
            Some(json!({
                "code": "NOWALLET",
                "status": "active",
                "commission_type": "percentage",
                "commission_value": "1000",
                "discount_type": "percentage",
                "discount_value": "1000"
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["code_normalized"], "NOWALLET");
        assert!(body["beneficiary_wallet"].is_null());
        assert_eq!(body["discount_type"], "percentage");
        assert_eq!(body["discount_value"], "1000");

        let id = body["id"].as_i64().expect("invite id should exist");
        let (status, updated) = json_request(
            &app,
            Method::PATCH,
            &format!("/invite-codes/{id}"),
            Some(&token),
            Some(json!({
                "beneficiary_wallet": "0x2222222222222222222222222222222222222222"
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            updated["beneficiary_wallet"],
            "0x2222222222222222222222222222222222222222"
        );

        let (status, updated) = json_request(
            &app,
            Method::PATCH,
            &format!("/invite-codes/{id}"),
            Some(&token),
            Some(json!({
                "discount_type": "",
                "discount_value": ""
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(updated["discount_type"].is_null());
        assert!(updated["discount_value"].is_null());
    }

    #[tokio::test]
    async fn admin_promotion_codes_allow_custom_invite_codes_and_keep_discount_codes_safe() {
        let wallet = "0x1111111111111111111111111111111111111111";
        let (app, state) = build_test_app(vec![admin_config(wallet, "operator")]).await;
        let token = admin_token(&state, wallet, "operator");

        let (status, body) = json_request(
            &app,
            Method::POST,
            "/invite-codes",
            Some(&token),
            Some(json!({
                "code": "oil01abc",
                "beneficiary_wallet": "0x2222222222222222222222222222222222222222",
                "status": "active"
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["code_normalized"], "OIL01ABC");

        let (status, body) = json_request(
            &app,
            Method::POST,
            "/invite-codes",
            Some(&token),
            Some(json!({
                "code": "oil1",
                "beneficiary_wallet": "0x2222222222222222222222222222222222222222",
                "status": "active"
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["code_normalized"], "OIL1");

        for code in ["ABC", "SAFE-ABC"] {
            let (status, _body) = json_request(
                &app,
                Method::POST,
                "/invite-codes",
                Some(&token),
                Some(json!({
                    "code": code,
                    "beneficiary_wallet": "0x2222222222222222222222222222222222222222",
                    "status": "active"
                })),
            )
            .await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{code} should fail");
        }

        let (status, _body) = json_request(
            &app,
            Method::POST,
            "/discount-codes",
            Some(&token),
            Some(json!({
                "code": "DISC1ABC",
                "status": "active",
                "discount_type": "fixed",
                "discount_value": "100"
            })),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn admin_invite_codes_viewer_cannot_create() {
        let wallet = "0x1111111111111111111111111111111111111111";
        let (app, state) = build_test_app(vec![admin_config(wallet, "viewer")]).await;
        let token = admin_token(&state, wallet, "viewer");

        let (status, _body) = json_request(
            &app,
            Method::POST,
            "/invite-codes",
            Some(&token),
            Some(json!({
                "code": "WATCHER2",
                "beneficiary_wallet": "0x2222222222222222222222222222222222222222",
                "status": "active"
            })),
        )
        .await;

        assert_eq!(status, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn admin_invite_codes_pause_and_activate_update_status_and_audit_logs() {
        let wallet = "0x1111111111111111111111111111111111111111";
        let (app, state) = build_test_app(vec![admin_config(wallet, "operator")]).await;
        let token = admin_token(&state, wallet, "operator");

        let (status, body) = json_request(
            &app,
            Method::POST,
            "/invite-codes",
            Some(&token),
            Some(json!({
                "code": "TGM23456",
                "beneficiary_wallet": "0x2222222222222222222222222222222222222222",
                "status": "active"
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let id = body["id"].as_i64().expect("id should exist");

        let (status, paused) = json_request(
            &app,
            Method::POST,
            &format!("/invite-codes/{id}/pause"),
            Some(&token),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(paused["status"], "paused");

        let (status, activated) = json_request(
            &app,
            Method::POST,
            &format!("/invite-codes/{id}/activate"),
            Some(&token),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(activated["status"], "active");

        let audit_count = state
            .db
            .count_admin_audit_logs_for_target("invite_code", &id.to_string())
            .await
            .expect("audit count should succeed");
        assert_eq!(audit_count, 3);
    }

    #[tokio::test]
    async fn admin_discount_codes_operator_can_create_fixed_and_percentage_and_list_discount_only()
    {
        let wallet = "0x1111111111111111111111111111111111111111";
        let (app, state) = build_test_app(vec![admin_config(wallet, "operator")]).await;
        let token = admin_token(&state, wallet, "operator");
        state
            .db
            .seed_referral_code("INVITEONLY")
            .await
            .expect("referral seed should succeed");

        let (status, fixed_body) = json_request(
            &app,
            Method::POST,
            "/discount-codes",
            Some(&token),
            Some(json!({
                "code": "FEE23456",
                "status": "active",
                "discount_type": "fixed",
                "discount_value": "12.5",
                "notes": "fixed test"
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(fixed_body["kind"], "discount");
        assert_eq!(fixed_body["discount_type"], "fixed");
        assert_eq!(fixed_body["discount_value"], "12.5");

        let (status, percentage_body) = json_request(
            &app,
            Method::POST,
            "/discount-codes",
            Some(&token),
            Some(json!({
                "code": "PCT23456",
                "status": "active",
                "discount_type": "percentage",
                "discount_value": "1000"
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(percentage_body["discount_type"], "percentage");
        assert_eq!(percentage_body["discount_value"], "1000");

        let (status, list_body) =
            json_request(&app, Method::GET, "/discount-codes", Some(&token), None).await;
        assert_eq!(status, StatusCode::OK);
        let items = list_body["items"]
            .as_array()
            .expect("items should be array");
        assert_eq!(items.len(), 2);
        assert!(items.iter().all(|item| item["kind"] == "discount"));
    }

    #[tokio::test]
    async fn admin_discount_codes_validate_percentage_and_fixed_values() {
        let wallet = "0x1111111111111111111111111111111111111111";
        let (app, state) = build_test_app(vec![admin_config(wallet, "operator")]).await;
        let token = admin_token(&state, wallet, "operator");

        let (status, _body) = json_request(
            &app,
            Method::POST,
            "/discount-codes",
            Some(&token),
            Some(json!({
                "code": "PCTBAD23",
                "status": "active",
                "discount_type": "percentage",
                "discount_value": "0"
            })),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);

        let (status, _body) = json_request(
            &app,
            Method::POST,
            "/discount-codes",
            Some(&token),
            Some(json!({
                "code": "PCTMAX99",
                "status": "active",
                "discount_type": "percentage",
                "discount_value": "10001"
            })),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);

        let (status, _body) = json_request(
            &app,
            Method::POST,
            "/discount-codes",
            Some(&token),
            Some(json!({
                "code": "FEEBAD23",
                "status": "active",
                "discount_type": "fixed",
                "discount_value": "-1"
            })),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn admin_discount_codes_viewer_cannot_create() {
        let wallet = "0x1111111111111111111111111111111111111111";
        let (app, state) = build_test_app(vec![admin_config(wallet, "viewer")]).await;
        let token = admin_token(&state, wallet, "viewer");

        let (status, _body) = json_request(
            &app,
            Method::POST,
            "/discount-codes",
            Some(&token),
            Some(json!({
                "code": "WATCHDSC",
                "status": "active",
                "discount_type": "fixed",
                "discount_value": "100"
            })),
        )
        .await;

        assert_eq!(status, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn admin_discount_codes_pause_and_activate_update_status_and_audit_logs() {
        let wallet = "0x1111111111111111111111111111111111111111";
        let (app, state) = build_test_app(vec![admin_config(wallet, "operator")]).await;
        let token = admin_token(&state, wallet, "operator");

        let (status, body) = json_request(
            &app,
            Method::POST,
            "/discount-codes",
            Some(&token),
            Some(json!({
                "code": "DSCTG234",
                "status": "active",
                "discount_type": "fixed",
                "discount_value": "100"
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let id = body["id"].as_i64().expect("id should exist");

        let (status, paused) = json_request(
            &app,
            Method::POST,
            &format!("/discount-codes/{id}/pause"),
            Some(&token),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(paused["status"], "paused");

        let (status, activated) = json_request(
            &app,
            Method::POST,
            &format!("/discount-codes/{id}/activate"),
            Some(&token),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(activated["status"], "active");

        let audit_count = state
            .db
            .count_admin_audit_logs_for_target("discount_code", &id.to_string())
            .await
            .expect("audit count should succeed");
        assert_eq!(audit_count, 3);
    }

    #[tokio::test]
    async fn admin_redemption_codes_operator_can_create_bulk_list_and_toggle() {
        let wallet = "0x1111111111111111111111111111111111111111";
        let (app, state) = build_test_app(vec![admin_config(wallet, "operator")]).await;
        let token = admin_token(&state, wallet, "operator");

        let (status, created) = json_request(
            &app,
            Method::POST,
            "/redemption-codes",
            Some(&token),
            Some(json!({
                "code": "GUESTV23",
                "status": "active",
                "ticket_level": 3,
                "notes": "vip guest"
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(created["code_normalized"], "GUESTV23");
        assert_eq!(created["ticket_level"], 3);

        let (status, bulk) = json_request(
            &app,
            Method::POST,
            "/redemption-codes/bulk",
            Some(&token),
            Some(json!({
                "prefix": "STD",
                "count": 3,
                "ticket_level": 1,
                "status": "active",
                "notes": "batch guests"
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let bulk_items = bulk["items"].as_array().expect("items array");
        assert_eq!(bulk_items.len(), 3);
        let bulk_codes: std::collections::HashSet<_> = bulk_items
            .iter()
            .map(|item| item["code_normalized"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(bulk_codes.len(), 3);
        assert!(bulk_codes.iter().all(|code| code.starts_with("STD")));
        assert!(bulk_codes.iter().all(|code| code.len() == 11));

        let id = created["id"].as_i64().expect("id should exist");
        let (status, paused) = json_request(
            &app,
            Method::POST,
            &format!("/redemption-codes/{id}/pause"),
            Some(&token),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(paused["status"], "paused");

        let (status, updated) = json_request(
            &app,
            Method::PATCH,
            &format!("/redemption-codes/{id}"),
            Some(&token),
            Some(json!({ "ticket_level": 2, "notes": "changed" })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(updated["ticket_level"], 2);
        assert_eq!(updated["notes"], "changed");

        let (status, list_body) =
            json_request(&app, Method::GET, "/redemption-codes", Some(&token), None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(list_body["items"].as_array().expect("items array").len(), 4);

        let audit_count = state
            .db
            .count_admin_audit_logs_for_target("redemption_code", &id.to_string())
            .await
            .expect("audit count should succeed");
        assert_eq!(audit_count, 3);
    }

    #[tokio::test]
    async fn admin_redemption_codes_viewer_can_read_stats_and_claims() {
        let operator_wallet = "0x1111111111111111111111111111111111111111";
        let viewer_wallet = "0x2222222222222222222222222222222222222222";
        let (app, state) = build_test_app(vec![
            admin_config(operator_wallet, "operator"),
            admin_config(viewer_wallet, "viewer"),
        ])
        .await;
        let operator_token = admin_token(&state, operator_wallet, "operator");
        let viewer_token = admin_token(&state, viewer_wallet, "viewer");

        let (status, created) = json_request(
            &app,
            Method::POST,
            "/redemption-codes",
            Some(&operator_token),
            Some(json!({
                "code": "READCHM2",
                "status": "active",
                "ticket_level": 1
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let code_id = created["id"].as_i64().expect("id should exist");
        let claim_result = state
            .db
            .redeem_redemption_code("READCHM2", "email", "guest@example.com")
            .await
            .expect("redeem should succeed")
            .expect("redeem should create claim");

        let (status, stats) = json_request(
            &app,
            Method::GET,
            "/redemption-codes/stats",
            Some(&viewer_token),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(stats["redeemed_count"], 1);
        assert_eq!(stats["level_1_count"], 1);

        let (status, claims) = json_request(
            &app,
            Method::GET,
            "/redemption-codes/claims",
            Some(&viewer_token),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(claims["items"][0]["code_normalized"], "READCHM2");
        assert_eq!(claims["items"][0]["claimant"], "guest@example.com");
        assert_eq!(claims["items"][0]["ticket_id"], claim_result.ticket.id);

        let (status, code_claims) = json_request(
            &app,
            Method::GET,
            &format!("/redemption-codes/{code_id}/claims"),
            Some(&viewer_token),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            code_claims["items"]
                .as_array()
                .expect("items should be array")
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn admin_diagnostics_referral_bindings_returns_bindings_joined_to_invite_code() {
        let admin_wallet = "0x1111111111111111111111111111111111111111";
        let customer_wallet = "0x2222222222222222222222222222222222222222";
        let (app, state) = build_test_app(vec![admin_config(admin_wallet, "viewer")]).await;
        let token = admin_token(&state, admin_wallet, "viewer");
        let referral_id = state
            .db
            .seed_referral_code("INVITE-DIAG")
            .await
            .expect("referral seed should succeed");
        state
            .db
            .bind_wallet_referral_once(customer_wallet, referral_id, "signin")
            .await
            .expect("binding should succeed");

        let (status, body) =
            json_request(&app, Method::GET, "/referral-bindings", Some(&token), None).await;

        assert_eq!(status, StatusCode::OK);
        let items = body["items"].as_array().expect("items should be array");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["wallet_address"], customer_wallet);
        assert_eq!(items[0]["referral_code"], "INVITE-DIAG");
    }

    #[tokio::test]
    async fn admin_diagnostics_purchase_intents_filter_and_detail_include_redemption_and_order() {
        let admin_wallet = "0x1111111111111111111111111111111111111111";
        let customer_wallet = "0x2222222222222222222222222222222222222222";
        let (app, state) = build_test_app(vec![admin_config(admin_wallet, "viewer")]).await;
        let token = admin_token(&state, admin_wallet, "viewer");
        let referral_id = state
            .db
            .seed_referral_code("INVITE-PI")
            .await
            .expect("referral seed should succeed");
        let discount_id = state
            .db
            .seed_fixed_discount_code("SAVE-PI", "100")
            .await
            .expect("discount seed should succeed");
        state
            .db
            .create_purchase_intent(NewPurchaseIntent {
                id: Some("intent-diagnostic".to_string()),
                wallet_address: customer_wallet.to_string(),
                chain_id: 56,
                payment_token: "0x3333333333333333333333333333333333333333".to_string(),
                level_ids_json: "[1]".to_string(),
                quantities_json: "[2]".to_string(),
                referral_code_id: Some(referral_id),
                discount_code_id: Some(discount_id),
                original_total_amount: "1000".to_string(),
                discount_amount: "100".to_string(),
                final_total_amount: "900".to_string(),
                expires_at: 4_102_444_800,
                status: PurchaseIntentStatus::Submitted,
                tx_hash: Some("0xintenttx".to_string()),
                order_id: Some("order-diagnostic".to_string()),
            })
            .await
            .expect("intent seed should succeed");
        state
            .db
            .reserve_discount_redemption(NewDiscountRedemption {
                purchase_intent_id: "intent-diagnostic".to_string(),
                discount_code_id: discount_id,
                wallet_address: customer_wallet.to_string(),
                status: DiscountRedemptionStatus::Reserved,
                tx_hash: Some("0xintenttx".to_string()),
                order_id: Some("order-diagnostic".to_string()),
                reserved_at: 1,
                confirmed_at: None,
                released_at: None,
            })
            .await
            .expect("redemption seed should succeed");
        state
            .db
            .seed_order(
                56,
                "0xintenttx",
                7,
                "order-diagnostic",
                customer_wallet,
                "900",
            )
            .await
            .expect("order seed should succeed");

        let path = format!(
            "/purchase-intents?wallet={customer_wallet}&tx_hash=0xintenttx&status=submitted&code=INVITE-PI"
        );
        let (status, body) = json_request(&app, Method::GET, &path, Some(&token), None).await;
        assert_eq!(status, StatusCode::OK);
        let items = body["items"].as_array().expect("items should be array");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["id"], "intent-diagnostic");

        let (status, detail) = json_request(
            &app,
            Method::GET,
            "/purchase-intents/intent-diagnostic",
            Some(&token),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            detail["discount_redemption"]["purchase_intent_id"],
            "intent-diagnostic"
        );
        assert_eq!(detail["linked_order"]["order_id"], "order-diagnostic");
        assert_eq!(detail["linked_order"]["line_items"][0]["ticket_level"], 1);
        assert_eq!(detail["linked_order"]["line_items"][0]["quantity"], 2);
    }

    #[tokio::test]
    async fn admin_diagnostics_orders_filter_and_attribution_returns_snapshot_and_codes() {
        let admin_wallet = "0x1111111111111111111111111111111111111111";
        let customer_wallet = "0x2222222222222222222222222222222222222222";
        let (app, state) = build_test_app(vec![admin_config(admin_wallet, "viewer")]).await;
        let token = admin_token(&state, admin_wallet, "viewer");
        let referral_id = state
            .db
            .seed_referral_code("INVITE-ORDER")
            .await
            .expect("referral seed should succeed");
        let discount_id = state
            .db
            .seed_fixed_discount_code("SAVE-ORDER", "100")
            .await
            .expect("discount seed should succeed");
        let order_row_id = state
            .db
            .seed_order(
                56,
                "0xordertx",
                3,
                "order-attribution",
                customer_wallet,
                "900",
            )
            .await
            .expect("order seed should succeed");
        state
            .db
            .seed_ticket_for_order(order_row_id, 1, "300")
            .await
            .expect("level 1 ticket should seed");
        state
            .db
            .seed_ticket_for_order(order_row_id, 2, "300")
            .await
            .expect("first level 2 ticket should seed");
        state
            .db
            .seed_ticket_for_order(order_row_id, 2, "300")
            .await
            .expect("second level 2 ticket should seed");
        state
            .db
            .insert_order_promotions_snapshot(NewOrderPromotionsSnapshot {
                order_row_id,
                wallet_address: customer_wallet.to_string(),
                referral_code_id: Some(referral_id),
                discount_code_id: Some(discount_id),
                original_total_amount: "1000".to_string(),
                discount_amount: "100".to_string(),
                paid_amount: "900".to_string(),
                commission_base_amount: "900".to_string(),
                commission_amount: "90".to_string(),
                rule_version: "v1".to_string(),
                created_at: 1,
            })
            .await
            .expect("snapshot seed should succeed");

        let path = format!(
            "/orders?wallet={customer_wallet}&tx_hash=0xordertx&invite_code=INVITE-ORDER&discount_code=SAVE-ORDER"
        );
        let (status, body) = json_request(&app, Method::GET, &path, Some(&token), None).await;
        assert_eq!(status, StatusCode::OK);
        let items = body["items"].as_array().expect("items should be array");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["order_id"], "order-attribution");
        assert_eq!(items[0]["line_items"][0]["ticket_level"], 1);
        assert_eq!(items[0]["line_items"][0]["quantity"], 1);
        assert_eq!(items[0]["line_items"][1]["ticket_level"], 2);
        assert_eq!(items[0]["line_items"][1]["quantity"], 2);
        assert_eq!(items[0]["original_total_amount"], "1000");
        assert_eq!(items[0]["discount_amount"], "100");
        assert_eq!(items[0]["paid_amount"], "900");

        let (status, attribution) = json_request(
            &app,
            Method::GET,
            &format!("/orders/{order_row_id}/attribution"),
            Some(&token),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(attribution["order"]["order_id"], "order-attribution");
        assert_eq!(attribution["order"]["line_items"][1]["quantity"], 2);
        assert_eq!(attribution["snapshot"]["paid_amount"], "900");
        assert_eq!(
            attribution["invite_code"]["code_normalized"],
            "INVITE-ORDER"
        );
        assert_eq!(
            attribution["discount_code"]["code_normalized"],
            "SAVE-ORDER"
        );
    }

    #[tokio::test]
    async fn admin_settlement_finance_and_admin_can_access_summary() {
        let finance_wallet = "0x1111111111111111111111111111111111111111";
        let admin_wallet = "0x2222222222222222222222222222222222222222";
        let customer_wallet = "0x3333333333333333333333333333333333333333";
        let (app, state) = build_test_app(vec![
            admin_config(finance_wallet, "finance"),
            admin_config(admin_wallet, "admin"),
        ])
        .await;
        let finance_token = admin_token(&state, finance_wallet, "finance");
        let admin_token = admin_token(&state, admin_wallet, "admin");
        let referral_id = state
            .db
            .seed_referral_code("SETTLE")
            .await
            .expect("referral seed should succeed");
        let order_row_id = state
            .db
            .seed_order(56, "0xsettle", 1, "settle-order", customer_wallet, "900")
            .await
            .expect("order seed should succeed");
        state
            .db
            .insert_order_promotions_snapshot(NewOrderPromotionsSnapshot {
                order_row_id,
                wallet_address: customer_wallet.to_string(),
                referral_code_id: Some(referral_id),
                discount_code_id: None,
                original_total_amount: "1000".to_string(),
                discount_amount: "100".to_string(),
                paid_amount: "900".to_string(),
                commission_base_amount: "900".to_string(),
                commission_amount: "90".to_string(),
                rule_version: "v1".to_string(),
                created_at: 1,
            })
            .await
            .expect("snapshot seed should succeed");

        let (status, finance_body) = json_request(
            &app,
            Method::GET,
            "/settlements/referrals",
            Some(&finance_token),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(finance_body["items"][0]["invite_code"], "SETTLE");
        assert_eq!(finance_body["items"][0]["confirmed_order_count"], 1);
        assert_eq!(finance_body["items"][0]["commission_amount_total"], "90");

        let (status, admin_body) = json_request(
            &app,
            Method::GET,
            "/settlements/referrals",
            Some(&admin_token),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(admin_body["items"][0]["paid_amount_total"], "900");
    }

    #[tokio::test]
    async fn admin_settlement_recalculates_zero_snapshot_commission_from_invite_rule() {
        let finance_wallet = "0x1111111111111111111111111111111111111111";
        let customer_wallet = "0x3333333333333333333333333333333333333333";
        let (app, state) = build_test_app(vec![admin_config(finance_wallet, "finance")]).await;
        let finance_token = admin_token(&state, finance_wallet, "finance");
        let referral_id = state
            .db
            .seed_referral_code("SETTLE0")
            .await
            .expect("referral seed should succeed");
        state
            .db
            .update_invite_code(
                referral_id,
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
        let order_row_id = state
            .db
            .seed_order(
                56,
                "0xsettlezero",
                1,
                "settle-zero-order",
                customer_wallet,
                "80000000000000000000",
            )
            .await
            .expect("order seed should succeed");
        state
            .db
            .insert_order_promotions_snapshot(NewOrderPromotionsSnapshot {
                order_row_id,
                wallet_address: customer_wallet.to_string(),
                referral_code_id: Some(referral_id),
                discount_code_id: None,
                original_total_amount: "80000000000000000000".to_string(),
                discount_amount: "0".to_string(),
                paid_amount: "80000000000000000000".to_string(),
                commission_base_amount: "80000000000000000000".to_string(),
                commission_amount: "0".to_string(),
                rule_version: "v1".to_string(),
                created_at: 1,
            })
            .await
            .expect("snapshot seed should succeed");

        let (status, body) = json_request(
            &app,
            Method::GET,
            "/settlements/referrals",
            Some(&finance_token),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            body["items"][0]["paid_amount_total"],
            "80000000000000000000"
        );
        assert_eq!(
            body["items"][0]["commission_amount_total"],
            "8000000000000000000"
        );
    }

    #[tokio::test]
    async fn admin_settlement_viewer_and_operator_cannot_export_csv() {
        let viewer_wallet = "0x1111111111111111111111111111111111111111";
        let operator_wallet = "0x2222222222222222222222222222222222222222";
        let (app, state) = build_test_app(vec![
            admin_config(viewer_wallet, "viewer"),
            admin_config(operator_wallet, "operator"),
        ])
        .await;
        let viewer_token = admin_token(&state, viewer_wallet, "viewer");
        let operator_token = admin_token(&state, operator_wallet, "operator");

        let (status, _) = text_request(
            &app,
            Method::GET,
            "/settlements/referrals.csv",
            Some(&viewer_token),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);

        let (status, _) = text_request(
            &app,
            Method::GET,
            "/settlements/referrals.csv",
            Some(&operator_token),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn admin_settlement_summary_only_uses_confirmed_order_snapshots() {
        let finance_wallet = "0x1111111111111111111111111111111111111111";
        let customer_wallet = "0x3333333333333333333333333333333333333333";
        let (app, state) = build_test_app(vec![admin_config(finance_wallet, "finance")]).await;
        let token = admin_token(&state, finance_wallet, "finance");
        let referral_id = state
            .db
            .seed_referral_code("ONLYSNAP")
            .await
            .expect("referral seed should succeed");
        state
            .db
            .create_purchase_intent(NewPurchaseIntent {
                id: Some("unconfirmed-intent".to_string()),
                wallet_address: customer_wallet.to_string(),
                chain_id: 56,
                payment_token: "0x4444444444444444444444444444444444444444".to_string(),
                level_ids_json: "[1]".to_string(),
                quantities_json: "[1]".to_string(),
                referral_code_id: Some(referral_id),
                discount_code_id: None,
                original_total_amount: "500".to_string(),
                discount_amount: "0".to_string(),
                final_total_amount: "500".to_string(),
                expires_at: 4_102_444_800,
                status: PurchaseIntentStatus::Submitted,
                tx_hash: None,
                order_id: None,
            })
            .await
            .expect("intent seed should succeed");

        let (status, body) = json_request(
            &app,
            Method::GET,
            "/settlements/referrals",
            Some(&token),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            body["items"]
                .as_array()
                .expect("items should be array")
                .len(),
            0
        );
    }

    #[tokio::test]
    async fn admin_audit_list_is_admin_only() {
        let admin_wallet = "0x1111111111111111111111111111111111111111";
        let viewer_wallet = "0x2222222222222222222222222222222222222222";
        let (app, state) = build_test_app(vec![
            admin_config(admin_wallet, "admin"),
            admin_config(viewer_wallet, "viewer"),
        ])
        .await;
        let admin_auth_token = admin_token(&state, admin_wallet, "admin");
        let viewer_token = admin_token(&state, viewer_wallet, "viewer");
        state
            .db
            .insert_admin_audit_log(crate::db::NewAdminAuditLog {
                actor_wallet: admin_wallet.to_string(),
                actor_role: "admin".to_string(),
                action: "invite_code.create".to_string(),
                target_type: "invite_code".to_string(),
                target_id: Some("1".to_string()),
                before_json: None,
                after_json: Some("{}".to_string()),
                ip_address: None,
                user_agent: None,
            })
            .await
            .expect("audit seed should succeed");

        let (status, _) =
            json_request(&app, Method::GET, "/audit-logs", Some(&viewer_token), None).await;
        assert_eq!(status, StatusCode::FORBIDDEN);

        let (status, body) = json_request(
            &app,
            Method::GET,
            "/audit-logs",
            Some(&admin_auth_token),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["items"][0]["action"], "invite_code.create");
    }
}
