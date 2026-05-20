use std::collections::HashMap;

use ethers_core::{
    types::{H256, U256},
    utils::keccak256,
};
use serde::Serialize;
use sqlx::{sqlite::SqlitePoolOptions, FromRow, QueryBuilder, SqlitePool};
use uuid::Uuid;

use crate::chain::DecodedPurchase;
use crate::config::payment_token_decimals;
use crate::promotions::{
    normalize_promotion_code, normalize_wallet_key, DiscountRedemptionRow,
    DiscountRedemptionStatus, NewDiscountRedemption, NewOrderPromotionsSnapshot, NewPurchaseIntent,
    OrderPromotionsSnapshotRow, PromotionCodeRow, PurchaseIntentRow, PurchaseIntentStatus,
    ReferralBindResult, WalletReferralBindingRow,
};

#[derive(Clone)]
pub struct Db {
    pool: SqlitePool,
}

#[derive(Debug, Clone, FromRow, serde::Serialize)]
pub struct TicketRow {
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

#[derive(Debug, Clone, FromRow, Serialize)]
pub struct AdminReferralBindingRow {
    pub wallet_address: String,
    pub referral_code_id: i64,
    pub referral_code: String,
    pub bound_at: i64,
    pub first_bound_source: String,
}

#[derive(Debug, Clone, FromRow, Serialize)]
pub struct AdminWalletRow {
    pub id: i64,
    pub wallet_address: String,
    pub role: String,
    pub status: String,
    pub notes: Option<String>,
    pub created_by: String,
    pub updated_by: String,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone)]
pub struct NewAdminWallet {
    pub wallet_address: String,
    pub role: String,
    pub status: String,
    pub notes: Option<String>,
    pub created_by: String,
}

#[derive(Debug, Clone)]
pub struct UpdateAdminWallet {
    pub role: Option<String>,
    pub status: Option<String>,
    pub notes: Option<String>,
    pub updated_by: String,
}

#[derive(Debug, Clone, FromRow, Serialize)]
struct AdminOrderRecord {
    pub id: i64,
    pub chain_id: i64,
    pub tx_hash: String,
    pub log_index: i64,
    pub block_number: i64,
    pub block_hash: String,
    pub order_id: String,
    pub buyer_address: String,
    pub payment_token: String,
    pub total_amount: String,
    pub created_at: i64,
    pub original_total_amount: Option<String>,
    pub discount_amount: Option<String>,
    pub paid_amount: Option<String>,
}

#[derive(Debug, Clone, FromRow, Serialize)]
pub struct AdminOrderLineItemRow {
    pub ticket_level: i64,
    pub quantity: i64,
    pub unit_price: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct AdminOrderRow {
    pub id: i64,
    pub chain_id: i64,
    pub tx_hash: String,
    pub log_index: i64,
    pub block_number: i64,
    pub block_hash: String,
    pub order_id: String,
    pub buyer_address: String,
    pub payment_token: String,
    pub total_amount: String,
    pub created_at: i64,
    pub original_total_amount: Option<String>,
    pub discount_amount: Option<String>,
    pub paid_amount: Option<String>,
    pub line_items: Vec<AdminOrderLineItemRow>,
}

impl AdminOrderRecord {
    fn with_line_items(self, line_items: Vec<AdminOrderLineItemRow>) -> AdminOrderRow {
        AdminOrderRow {
            id: self.id,
            chain_id: self.chain_id,
            tx_hash: self.tx_hash,
            log_index: self.log_index,
            block_number: self.block_number,
            block_hash: self.block_hash,
            order_id: self.order_id,
            buyer_address: self.buyer_address,
            payment_token: self.payment_token,
            total_amount: self.total_amount,
            created_at: self.created_at,
            original_total_amount: self.original_total_amount,
            discount_amount: self.discount_amount,
            paid_amount: self.paid_amount,
            line_items,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct PurchaseIntentDiagnostic {
    pub intent: PurchaseIntentRow,
    pub discount_redemption: Option<DiscountRedemptionRow>,
    pub linked_order: Option<AdminOrderRow>,
}

#[derive(Debug, Clone, Serialize)]
pub struct OrderAttributionDiagnostic {
    pub order: AdminOrderRow,
    pub snapshot: Option<OrderPromotionsSnapshotRow>,
    pub invite_code: Option<PromotionCodeRow>,
    pub discount_code: Option<PromotionCodeRow>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReferralSettlementRow {
    pub invite_code_id: i64,
    pub invite_code: String,
    pub beneficiary_wallet: Option<String>,
    pub confirmed_order_count: i64,
    pub paid_amount_total: String,
    pub commission_base_amount_total: String,
    pub commission_amount_total: String,
}

#[derive(Debug, Clone, FromRow, Serialize)]
pub struct AdminAuditLogRow {
    pub id: i64,
    pub actor_wallet: String,
    pub actor_role: String,
    pub action: String,
    pub target_type: String,
    pub target_id: Option<String>,
    pub before_json: Option<String>,
    pub after_json: Option<String>,
    pub ip_address: Option<String>,
    pub user_agent: Option<String>,
    pub created_at: i64,
}

#[derive(Debug, Clone, FromRow)]
struct SettlementSourceRow {
    invite_code_id: i64,
    invite_code: String,
    beneficiary_wallet: Option<String>,
    chain_id: i64,
    payment_token: String,
    paid_amount: String,
    commission_base_amount: String,
    commission_amount: String,
    commission_type: Option<String>,
    commission_value: Option<String>,
}

struct ReferralSettlementAccumulator {
    invite_code_id: i64,
    invite_code: String,
    beneficiary_wallet: Option<String>,
    confirmed_order_count: i64,
    paid_amount_total: U256,
    commission_base_amount_total: U256,
    commission_amount_total: U256,
}

impl ReferralSettlementAccumulator {
    fn into_row(self) -> ReferralSettlementRow {
        ReferralSettlementRow {
            invite_code_id: self.invite_code_id,
            invite_code: self.invite_code,
            beneficiary_wallet: self.beneficiary_wallet,
            confirmed_order_count: self.confirmed_order_count,
            paid_amount_total: self.paid_amount_total.to_string(),
            commission_base_amount_total: self.commission_base_amount_total.to_string(),
            commission_amount_total: self.commission_amount_total.to_string(),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct PurchaseIntentFilters {
    pub wallet: Option<String>,
    pub tx_hash: Option<String>,
    pub status: Option<String>,
    pub code: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct OrderFilters {
    pub wallet: Option<String>,
    pub tx_hash: Option<String>,
    pub invite_code: Option<String>,
    pub discount_code: Option<String>,
}

#[derive(Debug, Clone)]
pub struct NotifyResult {
    pub created_order: bool,
    pub created_tickets: usize,
}

#[derive(Debug, Clone)]
pub struct SigninChallenge {
    pub id: String,
    pub wallet: String,
    pub challenge_message: String,
    pub expires_at: i64,
}

#[derive(Debug, Clone)]
pub struct EmailAccessChallenge {
    pub id: String,
    pub email: String,
    pub token: String,
    pub token_hash: String,
    pub expires_at: i64,
}

#[derive(Debug, Clone, FromRow)]
pub struct ConsumedEmailAccessChallenge {
    pub id: String,
    pub email: String,
    pub expires_at: i64,
}

#[derive(Debug, Clone)]
pub struct NewAdminAuditLog {
    pub actor_wallet: String,
    pub actor_role: String,
    pub action: String,
    pub target_type: String,
    pub target_id: Option<String>,
    pub before_json: Option<String>,
    pub after_json: Option<String>,
    pub ip_address: Option<String>,
    pub user_agent: Option<String>,
}

#[derive(Debug, Clone)]
pub struct NewInviteCode {
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

#[derive(Debug, Clone)]
pub struct UpdateInviteCode {
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

#[derive(Debug, Clone)]
pub struct NewDiscountCode {
    pub code: String,
    pub status: String,
    pub discount_type: String,
    pub discount_value: String,
    pub max_discount_amount: Option<String>,
    pub max_total_uses: Option<i64>,
    pub max_uses_per_wallet: Option<i64>,
    pub first_purchase_only: bool,
    pub stacking_policy: Option<String>,
    pub applicable_chain_ids: Option<String>,
    pub applicable_ticket_levels: Option<String>,
    pub valid_from: Option<i64>,
    pub valid_until: Option<i64>,
    pub notes: Option<String>,
}

#[derive(Debug, Clone)]
pub struct NewFiatCheckoutSession {
    pub id: String,
    pub email: String,
    pub currency: String,
    pub level_ids_json: String,
    pub quantities_json: String,
    pub unit_prices_cents_json: String,
    pub referral_code_id: Option<i64>,
    pub discount_code_id: Option<i64>,
    pub original_amount_cents: i64,
    pub discount_amount_cents: i64,
    pub final_amount_cents: i64,
    pub expires_at: i64,
}

#[derive(Debug, Clone, FromRow, Serialize)]
pub struct FiatCheckoutSessionRow {
    pub id: String,
    pub stripe_session_id: Option<String>,
    pub email: String,
    pub currency: String,
    pub level_ids_json: String,
    pub quantities_json: String,
    pub unit_prices_cents_json: String,
    pub referral_code_id: Option<i64>,
    pub discount_code_id: Option<i64>,
    pub original_amount_cents: i64,
    pub discount_amount_cents: i64,
    pub final_amount_cents: i64,
    pub status: String,
    pub stripe_url: Option<String>,
    pub payment_intent_id: Option<String>,
    pub internal_order_row_id: Option<i64>,
    pub created_tickets: i64,
    pub expires_at: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone)]
pub struct FiatCheckoutConfirmation {
    pub checkout: FiatCheckoutSessionRow,
    pub newly_paid: bool,
}

#[derive(Debug, Clone)]
pub struct UpdateDiscountCode {
    pub status: Option<String>,
    pub discount_type: Option<String>,
    pub discount_value: Option<String>,
    pub max_discount_amount: Option<String>,
    pub max_total_uses: Option<i64>,
    pub max_uses_per_wallet: Option<i64>,
    pub first_purchase_only: Option<bool>,
    pub stacking_policy: Option<String>,
    pub applicable_chain_ids: Option<String>,
    pub applicable_ticket_levels: Option<String>,
    pub valid_from: Option<i64>,
    pub valid_until: Option<i64>,
    pub notes: Option<String>,
}

#[derive(Debug, Clone)]
pub struct IndexerCursor {
    pub last_indexed_block: u64,
    pub last_indexed_block_hash: Option<String>,
}

#[derive(Debug, Clone)]
pub struct RollbackResult {
    pub deleted_orders: u64,
    pub deleted_tickets: u64,
}

impl Db {
    pub async fn connect(database_url: &str) -> anyhow::Result<Self> {
        let max_connections = if database_url.starts_with("sqlite::memory:") {
            1
        } else {
            10
        };

        let pool = SqlitePoolOptions::new()
            .max_connections(max_connections)
            .connect(database_url)
            .await?;
        sqlx::migrate!("./migrations").run(&pool).await?;
        Ok(Self { pool })
    }

    pub async fn create_signin_challenge(
        &self,
        wallet: &str,
        ttl_secs: i64,
    ) -> anyhow::Result<SigninChallenge> {
        let id = Uuid::new_v4().to_string();
        let nonce = Uuid::new_v4().simple().to_string();
        let now_ts = unix_now();
        let expires_at = now_ts + ttl_secs;

        let challenge_message = format!(
            "Sign-In\n\
Purpose: Sign in to the ticketing service.\n\
Safety: This signature does not create a blockchain transaction and does not cost gas.\n\
Wallet: {wallet}\n\
Nonce: {nonce}\n\
IssuedAt: {now_ts}\n\
ExpiresAt: {expires_at}"
        );

        sqlx::query(
            r#"
            INSERT INTO signin_challenges (id, wallet, challenge_message, nonce, expires_at, used_at, created_at)
            VALUES (?1, ?2, ?3, ?4, ?5, NULL, ?6)
            "#,
        )
        .bind(&id)
        .bind(wallet)
        .bind(&challenge_message)
        .bind(nonce)
        .bind(expires_at)
        .bind(now_ts)
        .execute(&self.pool)
        .await?;

        Ok(SigninChallenge {
            id,
            wallet: wallet.to_string(),
            challenge_message,
            expires_at,
        })
    }

    pub async fn get_signin_challenge_message(
        &self,
        challenge_id: &str,
        wallet: &str,
    ) -> anyhow::Result<Option<String>> {
        let now_ts = unix_now();
        let message = sqlx::query_scalar::<_, String>(
            r#"
            SELECT challenge_message
            FROM signin_challenges
            WHERE id = ?1
              AND wallet = ?2
              AND used_at IS NULL
              AND expires_at >= ?3
            "#,
        )
        .bind(challenge_id)
        .bind(wallet)
        .bind(now_ts)
        .fetch_optional(&self.pool)
        .await?;

        Ok(message)
    }

    pub async fn mark_signin_challenge_used(
        &self,
        challenge_id: &str,
        wallet: &str,
    ) -> anyhow::Result<bool> {
        let now_ts = unix_now();
        let update_result = sqlx::query(
            r#"
            UPDATE signin_challenges
            SET used_at = ?2
            WHERE id = ?1
              AND wallet = ?3
              AND used_at IS NULL
              AND expires_at >= ?2
            "#,
        )
        .bind(challenge_id)
        .bind(now_ts)
        .bind(wallet)
        .execute(&self.pool)
        .await?;

        Ok(update_result.rows_affected() == 1)
    }

    pub async fn purge_signin_challenges(&self, delete_before_ts: i64) -> anyhow::Result<u64> {
        let result = sqlx::query(
            r#"
            DELETE FROM signin_challenges
            WHERE expires_at < ?1
               OR (used_at IS NOT NULL AND used_at < ?1)
            "#,
        )
        .bind(delete_before_ts)
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected())
    }

    pub async fn create_email_access_challenge(
        &self,
        email: &str,
        ttl_secs: i64,
    ) -> anyhow::Result<EmailAccessChallenge> {
        let id = Uuid::new_v4().to_string();
        let token = Uuid::new_v4().simple().to_string();
        let token_hash = email_access_token_hash(&token);
        let now_ts = unix_now();
        let expires_at = now_ts + ttl_secs;

        sqlx::query(
            r#"
            INSERT INTO email_access_challenges (id, email, token_hash, expires_at, used_at, created_at, updated_at)
            VALUES (?1, ?2, ?3, ?4, NULL, ?5, ?5)
            "#,
        )
        .bind(&id)
        .bind(email)
        .bind(&token_hash)
        .bind(expires_at)
        .bind(now_ts)
        .execute(&self.pool)
        .await?;

        Ok(EmailAccessChallenge {
            id,
            email: email.to_string(),
            token,
            token_hash,
            expires_at,
        })
    }

    pub async fn consume_email_access_challenge(
        &self,
        token: &str,
    ) -> anyhow::Result<Option<ConsumedEmailAccessChallenge>> {
        let token_hash = email_access_token_hash(token);
        let now_ts = unix_now();
        let mut tx = self.pool.begin().await?;

        let row = sqlx::query_as::<_, ConsumedEmailAccessChallenge>(
            r#"
            SELECT id, email, expires_at
            FROM email_access_challenges
            WHERE token_hash = ?1
              AND used_at IS NULL
              AND expires_at >= ?2
            "#,
        )
        .bind(&token_hash)
        .bind(now_ts)
        .fetch_optional(&mut *tx)
        .await?;

        let Some(row) = row else {
            tx.rollback().await?;
            return Ok(None);
        };

        let update = sqlx::query(
            r#"
            UPDATE email_access_challenges
            SET used_at = ?2,
                updated_at = ?2
            WHERE id = ?1
              AND used_at IS NULL
              AND expires_at >= ?2
            "#,
        )
        .bind(&row.id)
        .bind(now_ts)
        .execute(&mut *tx)
        .await?;

        if update.rows_affected() != 1 {
            tx.rollback().await?;
            return Ok(None);
        }

        tx.commit().await?;
        Ok(Some(row))
    }

    pub async fn create_fiat_checkout_session(
        &self,
        input: NewFiatCheckoutSession,
    ) -> anyhow::Result<FiatCheckoutSessionRow> {
        let now_ts = unix_now();
        sqlx::query(
            r#"
            INSERT INTO fiat_checkout_sessions (
                id,
                stripe_session_id,
                email,
                currency,
                level_ids_json,
                quantities_json,
                unit_prices_cents_json,
                referral_code_id,
                discount_code_id,
                original_amount_cents,
                discount_amount_cents,
                final_amount_cents,
                status,
                stripe_url,
                payment_intent_id,
                internal_order_row_id,
                created_tickets,
                expires_at,
                created_at,
                updated_at
            ) VALUES (?1, NULL, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 'pending', NULL, NULL, NULL, 0, ?12, ?13, ?13)
            "#,
        )
        .bind(&input.id)
        .bind(input.email)
        .bind(input.currency)
        .bind(input.level_ids_json)
        .bind(input.quantities_json)
        .bind(input.unit_prices_cents_json)
        .bind(input.referral_code_id)
        .bind(input.discount_code_id)
        .bind(input.original_amount_cents)
        .bind(input.discount_amount_cents)
        .bind(input.final_amount_cents)
        .bind(input.expires_at)
        .bind(now_ts)
        .execute(&self.pool)
        .await?;

        self.get_fiat_checkout_session(&input.id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("fiat checkout session should exist after insert"))
    }

    pub async fn attach_stripe_checkout_session(
        &self,
        id: &str,
        stripe_session_id: &str,
        stripe_url: &str,
    ) -> anyhow::Result<Option<FiatCheckoutSessionRow>> {
        let now_ts = unix_now();
        sqlx::query(
            r#"
            UPDATE fiat_checkout_sessions
            SET stripe_session_id = ?2,
                stripe_url = ?3,
                updated_at = ?4
            WHERE id = ?1
            "#,
        )
        .bind(id)
        .bind(stripe_session_id)
        .bind(stripe_url)
        .bind(now_ts)
        .execute(&self.pool)
        .await?;

        self.get_fiat_checkout_session(id).await
    }

    pub async fn get_fiat_checkout_session(
        &self,
        id: &str,
    ) -> anyhow::Result<Option<FiatCheckoutSessionRow>> {
        let row = sqlx::query_as::<_, FiatCheckoutSessionRow>(
            r#"
            SELECT id, stripe_session_id, email, currency, level_ids_json, quantities_json,
                   unit_prices_cents_json,
                   referral_code_id, discount_code_id, original_amount_cents,
                   discount_amount_cents, final_amount_cents, status, stripe_url,
                   payment_intent_id, internal_order_row_id, created_tickets,
                   expires_at, created_at, updated_at
            FROM fiat_checkout_sessions
            WHERE id = ?1
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn get_fiat_checkout_session_by_stripe_id(
        &self,
        stripe_session_id: &str,
    ) -> anyhow::Result<Option<FiatCheckoutSessionRow>> {
        let row = sqlx::query_as::<_, FiatCheckoutSessionRow>(
            r#"
            SELECT id, stripe_session_id, email, currency, level_ids_json, quantities_json,
                   unit_prices_cents_json,
                   referral_code_id, discount_code_id, original_amount_cents,
                   discount_amount_cents, final_amount_cents, status, stripe_url,
                   payment_intent_id, internal_order_row_id, created_tickets,
                   expires_at, created_at, updated_at
            FROM fiat_checkout_sessions
            WHERE stripe_session_id = ?1
            "#,
        )
        .bind(stripe_session_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn confirm_fiat_checkout_session(
        &self,
        stripe_session_id: &str,
        payment_intent_id: Option<&str>,
    ) -> anyhow::Result<Option<FiatCheckoutConfirmation>> {
        let mut tx = self.pool.begin().await?;
        let now_ts = unix_now();

        let checkout = sqlx::query_as::<_, FiatCheckoutSessionRow>(
            r#"
            SELECT id, stripe_session_id, email, currency, level_ids_json, quantities_json,
                   unit_prices_cents_json,
                   referral_code_id, discount_code_id, original_amount_cents,
                   discount_amount_cents, final_amount_cents, status, stripe_url,
                   payment_intent_id, internal_order_row_id, created_tickets,
                   expires_at, created_at, updated_at
            FROM fiat_checkout_sessions
            WHERE stripe_session_id = ?1
            "#,
        )
        .bind(stripe_session_id)
        .fetch_optional(&mut *tx)
        .await?;

        let Some(checkout) = checkout else {
            tx.rollback().await?;
            return Ok(None);
        };

        if checkout.status == "paid" {
            tx.rollback().await?;
            return Ok(Some(FiatCheckoutConfirmation {
                checkout,
                newly_paid: false,
            }));
        }

        let order_id = format!("fiat:{}", checkout.id);
        let tx_hash = format!("stripe:{stripe_session_id}");
        let order_insert = sqlx::query(
            r#"
            INSERT OR IGNORE INTO orders (
                chain_id,
                tx_hash,
                log_index,
                block_number,
                block_hash,
                order_id,
                buyer_address,
                payment_token,
                total_amount,
                created_at
            ) VALUES (0, ?1, 0, 0, 'stripe', ?2, ?3, ?4, ?5, ?6)
            "#,
        )
        .bind(&tx_hash)
        .bind(&order_id)
        .bind(&checkout.email)
        .bind(format!("stripe:{}", checkout.currency))
        .bind(checkout.final_amount_cents.to_string())
        .bind(now_ts)
        .execute(&mut *tx)
        .await?;

        let order_row_id: i64 = sqlx::query_scalar("SELECT id FROM orders WHERE tx_hash = ?1")
            .bind(&tx_hash)
            .fetch_one(&mut *tx)
            .await?;

        let mut created_tickets = 0i64;
        if order_insert.rows_affected() == 1 {
            let level_ids: Vec<i64> = serde_json::from_str(&checkout.level_ids_json)?;
            let quantities: Vec<i64> = serde_json::from_str(&checkout.quantities_json)?;
            let unit_prices: Vec<i64> = serde_json::from_str(&checkout.unit_prices_cents_json)?;
            for ((index, level), quantity) in level_ids.into_iter().enumerate().zip(quantities) {
                let unit_price = unit_prices
                    .get(index)
                    .copied()
                    .unwrap_or_else(|| fallback_fiat_unit_price(&checkout, quantity));
                for _ in 0..quantity {
                    let ticket_id = Uuid::new_v4().to_string();
                    let qr_payload = format!(
                        "money-frontier:qr:{}:1:{}",
                        ticket_id,
                        Uuid::new_v4().simple()
                    );
                    sqlx::query(
                        r#"
                        INSERT INTO tickets (
                            id,
                            chain_id,
                            order_id,
                            source_order_row_id,
                            owner_wallet,
                            owner_email,
                            ticket_level,
                            unit_price,
                            qr_payload,
                            qr_version,
                            status,
                            created_at,
                            updated_at
                        ) VALUES (?1, 0, ?2, ?3, NULL, ?4, ?5, ?6, ?7, 1, 'active', ?8, ?8)
                        "#,
                    )
                    .bind(&ticket_id)
                    .bind(&order_id)
                    .bind(order_row_id)
                    .bind(&checkout.email)
                    .bind(level)
                    .bind(unit_price.to_string())
                    .bind(qr_payload)
                    .bind(now_ts)
                    .execute(&mut *tx)
                    .await?;
                    created_tickets += 1;
                }
            }
        }

        sqlx::query(
            r#"
            UPDATE fiat_checkout_sessions
            SET status = 'paid',
                payment_intent_id = ?2,
                internal_order_row_id = ?3,
                created_tickets = CASE WHEN created_tickets > 0 THEN created_tickets ELSE ?4 END,
                updated_at = ?5
            WHERE id = ?1
            "#,
        )
        .bind(&checkout.id)
        .bind(payment_intent_id)
        .bind(order_row_id)
        .bind(created_tickets)
        .bind(now_ts)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok(self
            .get_fiat_checkout_session(&checkout.id)
            .await?
            .map(|checkout| FiatCheckoutConfirmation {
                checkout,
                newly_paid: created_tickets > 0,
            }))
    }

    pub async fn create_admin_signin_challenge(
        &self,
        wallet: &str,
        ttl_secs: i64,
    ) -> anyhow::Result<SigninChallenge> {
        let id = Uuid::new_v4().to_string();
        let nonce = Uuid::new_v4().simple().to_string();
        let now_ts = unix_now();
        let expires_at = now_ts + ttl_secs;

        let challenge_message = format!(
            "Admin Sign-In\n\
Purpose: Sign in to the operations admin console.\n\
Safety: This signature does not create a blockchain transaction and does not cost gas.\n\
Wallet: {wallet}\n\
Nonce: {nonce}\n\
IssuedAt: {now_ts}\n\
ExpiresAt: {expires_at}"
        );

        sqlx::query(
            r#"
            INSERT INTO admin_signin_challenges (id, wallet, challenge_message, nonce, expires_at, used_at, created_at)
            VALUES (?1, ?2, ?3, ?4, ?5, NULL, ?6)
            "#,
        )
        .bind(&id)
        .bind(wallet)
        .bind(&challenge_message)
        .bind(nonce)
        .bind(expires_at)
        .bind(now_ts)
        .execute(&self.pool)
        .await?;

        Ok(SigninChallenge {
            id,
            wallet: wallet.to_string(),
            challenge_message,
            expires_at,
        })
    }

    pub async fn get_admin_signin_challenge_message(
        &self,
        challenge_id: &str,
        wallet: &str,
    ) -> anyhow::Result<Option<String>> {
        let now_ts = unix_now();
        let message = sqlx::query_scalar::<_, String>(
            r#"
            SELECT challenge_message
            FROM admin_signin_challenges
            WHERE id = ?1
              AND wallet = ?2
              AND used_at IS NULL
              AND expires_at >= ?3
            "#,
        )
        .bind(challenge_id)
        .bind(wallet)
        .bind(now_ts)
        .fetch_optional(&self.pool)
        .await?;

        Ok(message)
    }

    pub async fn mark_admin_signin_challenge_used(
        &self,
        challenge_id: &str,
        wallet: &str,
    ) -> anyhow::Result<bool> {
        let now_ts = unix_now();
        let update_result = sqlx::query(
            r#"
            UPDATE admin_signin_challenges
            SET used_at = ?2
            WHERE id = ?1
              AND wallet = ?3
              AND used_at IS NULL
              AND expires_at >= ?2
            "#,
        )
        .bind(challenge_id)
        .bind(now_ts)
        .bind(wallet)
        .execute(&self.pool)
        .await?;

        Ok(update_result.rows_affected() == 1)
    }

    pub async fn purge_admin_signin_challenges(
        &self,
        delete_before_ts: i64,
    ) -> anyhow::Result<u64> {
        let result = sqlx::query(
            r#"
            DELETE FROM admin_signin_challenges
            WHERE expires_at < ?1
               OR (used_at IS NOT NULL AND used_at < ?1)
            "#,
        )
        .bind(delete_before_ts)
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected())
    }

    pub async fn insert_admin_audit_log(&self, input: NewAdminAuditLog) -> anyhow::Result<i64> {
        let now_ts = unix_now();
        let result = sqlx::query(
            r#"
            INSERT INTO admin_audit_logs (
                actor_wallet,
                actor_role,
                action,
                target_type,
                target_id,
                before_json,
                after_json,
                ip_address,
                user_agent,
                created_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
            "#,
        )
        .bind(input.actor_wallet)
        .bind(input.actor_role)
        .bind(input.action)
        .bind(input.target_type)
        .bind(input.target_id)
        .bind(input.before_json)
        .bind(input.after_json)
        .bind(input.ip_address)
        .bind(input.user_agent)
        .bind(now_ts)
        .execute(&self.pool)
        .await?;

        Ok(result.last_insert_rowid())
    }

    pub async fn list_admin_audit_logs(
        &self,
        page: i64,
        page_size: i64,
    ) -> anyhow::Result<Vec<AdminAuditLogRow>> {
        let page = page.max(1);
        let page_size = page_size.clamp(1, 200);
        let offset = (page - 1) * page_size;
        let rows = sqlx::query_as::<_, AdminAuditLogRow>(
            r#"
            SELECT
                id,
                actor_wallet,
                actor_role,
                action,
                target_type,
                target_id,
                before_json,
                after_json,
                ip_address,
                user_agent,
                created_at
            FROM admin_audit_logs
            ORDER BY created_at DESC, id DESC
            LIMIT ?1 OFFSET ?2
            "#,
        )
        .bind(page_size)
        .bind(offset)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    pub async fn list_admin_wallets(&self) -> anyhow::Result<Vec<AdminWalletRow>> {
        let rows = sqlx::query_as::<_, AdminWalletRow>(
            r#"
            SELECT
                id,
                wallet_address,
                role,
                status,
                notes,
                created_by,
                updated_by,
                created_at,
                updated_at
            FROM admin_wallets
            ORDER BY created_at DESC, id DESC
            "#,
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    pub async fn find_admin_wallet_by_address(
        &self,
        wallet: &str,
    ) -> anyhow::Result<Option<AdminWalletRow>> {
        let wallet = normalize_wallet_key(wallet);
        let row = sqlx::query_as::<_, AdminWalletRow>(
            r#"
            SELECT
                id,
                wallet_address,
                role,
                status,
                notes,
                created_by,
                updated_by,
                created_at,
                updated_at
            FROM admin_wallets
            WHERE wallet_address = ?1
            "#,
        )
        .bind(wallet)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn find_active_admin_wallet_role(
        &self,
        wallet: &str,
    ) -> anyhow::Result<Option<String>> {
        let wallet = normalize_wallet_key(wallet);
        let role = sqlx::query_scalar::<_, String>(
            r#"
            SELECT role
            FROM admin_wallets
            WHERE wallet_address = ?1
              AND status = 'active'
            "#,
        )
        .bind(wallet)
        .fetch_optional(&self.pool)
        .await?;

        Ok(role)
    }

    pub async fn create_admin_wallet(
        &self,
        input: NewAdminWallet,
    ) -> anyhow::Result<AdminWalletRow> {
        let now_ts = unix_now();
        let wallet = normalize_wallet_key(&input.wallet_address);
        sqlx::query(
            r#"
            INSERT INTO admin_wallets (
                wallet_address,
                role,
                status,
                notes,
                created_by,
                updated_by,
                created_at,
                updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?5, ?6, ?6)
            "#,
        )
        .bind(&wallet)
        .bind(input.role)
        .bind(input.status)
        .bind(input.notes)
        .bind(input.created_by)
        .bind(now_ts)
        .execute(&self.pool)
        .await?;

        self.find_admin_wallet_by_address(&wallet)
            .await?
            .ok_or_else(|| anyhow::anyhow!("created admin wallet not found"))
    }

    pub async fn update_admin_wallet(
        &self,
        id: i64,
        input: UpdateAdminWallet,
    ) -> anyhow::Result<Option<AdminWalletRow>> {
        let now_ts = unix_now();
        sqlx::query(
            r#"
            UPDATE admin_wallets
            SET role = COALESCE(?2, role),
                status = COALESCE(?3, status),
                notes = COALESCE(?4, notes),
                updated_by = ?5,
                updated_at = ?6
            WHERE id = ?1
            "#,
        )
        .bind(id)
        .bind(input.role)
        .bind(input.status)
        .bind(input.notes)
        .bind(input.updated_by)
        .bind(now_ts)
        .execute(&self.pool)
        .await?;

        self.get_admin_wallet(id).await
    }

    pub async fn get_admin_wallet(&self, id: i64) -> anyhow::Result<Option<AdminWalletRow>> {
        let row = sqlx::query_as::<_, AdminWalletRow>(
            r#"
            SELECT
                id,
                wallet_address,
                role,
                status,
                notes,
                created_by,
                updated_by,
                created_at,
                updated_at
            FROM admin_wallets
            WHERE id = ?1
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn delete_admin_wallet(&self, id: i64) -> anyhow::Result<Option<AdminWalletRow>> {
        let before = self.get_admin_wallet(id).await?;
        sqlx::query(
            r#"
            DELETE FROM admin_wallets
            WHERE id = ?1
            "#,
        )
        .bind(id)
        .execute(&self.pool)
        .await?;

        Ok(before)
    }

    pub async fn find_promotion_code(
        &self,
        code: &str,
    ) -> anyhow::Result<Option<PromotionCodeRow>> {
        let Some(normalized) = normalize_promotion_code(code) else {
            return Ok(None);
        };

        let row = sqlx::query_as::<_, PromotionCodeRow>(
            r#"
            SELECT
                id,
                code_normalized,
                kind,
                status,
                beneficiary_wallet,
                valid_from,
                valid_until,
                max_total_uses,
                max_uses_per_wallet,
                first_purchase_only,
                stacking_policy,
                applicable_chain_ids,
                applicable_ticket_levels,
                discount_type,
                discount_value,
                max_discount_amount,
                commission_type,
                commission_value,
                notes,
                created_at,
                updated_at
            FROM promotion_codes
            WHERE code_normalized = ?1
            "#,
        )
        .bind(normalized)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn list_invite_codes(
        &self,
        page: i64,
        page_size: i64,
    ) -> anyhow::Result<Vec<PromotionCodeRow>> {
        self.list_promotion_codes_by_kind("referral", page, page_size)
            .await
    }

    pub async fn create_invite_code(
        &self,
        input: NewInviteCode,
    ) -> anyhow::Result<PromotionCodeRow> {
        let now_ts = unix_now();
        let normalized = normalize_promotion_code(&input.code)
            .ok_or_else(|| anyhow::anyhow!("code is required"))?;

        sqlx::query(
            r#"
            INSERT INTO promotion_codes (
                code_normalized,
                kind,
                status,
                beneficiary_wallet,
                valid_from,
                valid_until,
                max_total_uses,
                max_uses_per_wallet,
                first_purchase_only,
                stacking_policy,
                applicable_chain_ids,
                applicable_ticket_levels,
                discount_type,
                discount_value,
                max_discount_amount,
                commission_type,
                commission_value,
                notes,
                created_at,
                updated_at
            ) VALUES (?1, 'referral', ?2, ?3, ?4, ?5, NULL, NULL, 0, NULL, NULL, NULL, ?6, ?7, NULL, ?8, ?9, ?10, ?11, ?11)
            "#,
        )
        .bind(&normalized)
        .bind(input.status)
        .bind(input.beneficiary_wallet)
        .bind(input.valid_from)
        .bind(input.valid_until)
        .bind(input.discount_type)
        .bind(input.discount_value)
        .bind(input.commission_type)
        .bind(input.commission_value)
        .bind(input.notes)
        .bind(now_ts)
        .execute(&self.pool)
        .await?;

        self.find_promotion_code(&normalized)
            .await?
            .ok_or_else(|| anyhow::anyhow!("invite code should exist after insert"))
    }

    pub async fn get_invite_code_detail(
        &self,
        id: i64,
    ) -> anyhow::Result<Option<PromotionCodeRow>> {
        self.get_promotion_code_by_id_kind(id, "referral").await
    }

    pub async fn update_invite_code(
        &self,
        id: i64,
        input: UpdateInviteCode,
    ) -> anyhow::Result<Option<PromotionCodeRow>> {
        let now_ts = unix_now();
        let result = sqlx::query(
            r#"
            UPDATE promotion_codes
            SET beneficiary_wallet = COALESCE(?2, beneficiary_wallet),
                status = COALESCE(?3, status),
                commission_type = COALESCE(?4, commission_type),
                commission_value = COALESCE(?5, commission_value),
                discount_type = CASE WHEN ?6 = '' THEN NULL ELSE COALESCE(?6, discount_type) END,
                discount_value = CASE WHEN ?7 = '' THEN NULL ELSE COALESCE(?7, discount_value) END,
                valid_from = COALESCE(?8, valid_from),
                valid_until = COALESCE(?9, valid_until),
                notes = COALESCE(?10, notes),
                updated_at = ?11
            WHERE id = ?1
              AND kind = 'referral'
            "#,
        )
        .bind(id)
        .bind(input.beneficiary_wallet)
        .bind(input.status)
        .bind(input.commission_type)
        .bind(input.commission_value)
        .bind(input.discount_type)
        .bind(input.discount_value)
        .bind(input.valid_from)
        .bind(input.valid_until)
        .bind(input.notes)
        .bind(now_ts)
        .execute(&self.pool)
        .await?;

        if result.rows_affected() == 0 {
            return Ok(None);
        }

        self.get_invite_code_detail(id).await
    }

    pub async fn set_invite_code_status(
        &self,
        id: i64,
        status: &str,
    ) -> anyhow::Result<Option<PromotionCodeRow>> {
        let now_ts = unix_now();
        let result = sqlx::query(
            r#"
            UPDATE promotion_codes
            SET status = ?2,
                updated_at = ?3
            WHERE id = ?1
              AND kind = 'referral'
            "#,
        )
        .bind(id)
        .bind(status)
        .bind(now_ts)
        .execute(&self.pool)
        .await?;

        if result.rows_affected() == 0 {
            return Ok(None);
        }

        self.get_invite_code_detail(id).await
    }

    pub async fn list_discount_codes(
        &self,
        page: i64,
        page_size: i64,
    ) -> anyhow::Result<Vec<PromotionCodeRow>> {
        self.list_promotion_codes_by_kind("discount", page, page_size)
            .await
    }

    pub async fn create_discount_code(
        &self,
        input: NewDiscountCode,
    ) -> anyhow::Result<PromotionCodeRow> {
        let now_ts = unix_now();
        let normalized = normalize_promotion_code(&input.code)
            .ok_or_else(|| anyhow::anyhow!("code is required"))?;

        sqlx::query(
            r#"
            INSERT INTO promotion_codes (
                code_normalized,
                kind,
                status,
                beneficiary_wallet,
                valid_from,
                valid_until,
                max_total_uses,
                max_uses_per_wallet,
                first_purchase_only,
                stacking_policy,
                applicable_chain_ids,
                applicable_ticket_levels,
                discount_type,
                discount_value,
                max_discount_amount,
                commission_type,
                commission_value,
                notes,
                created_at,
                updated_at
            ) VALUES (?1, 'discount', ?2, NULL, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, NULL, NULL, ?14, ?15, ?15)
            "#,
        )
        .bind(&normalized)
        .bind(input.status)
        .bind(input.valid_from)
        .bind(input.valid_until)
        .bind(input.max_total_uses)
        .bind(input.max_uses_per_wallet)
        .bind(input.first_purchase_only)
        .bind(input.stacking_policy)
        .bind(input.applicable_chain_ids)
        .bind(input.applicable_ticket_levels)
        .bind(input.discount_type)
        .bind(input.discount_value)
        .bind(input.max_discount_amount)
        .bind(input.notes)
        .bind(now_ts)
        .execute(&self.pool)
        .await?;

        self.find_promotion_code(&normalized)
            .await?
            .ok_or_else(|| anyhow::anyhow!("discount code should exist after insert"))
    }

    pub async fn get_discount_code_detail(
        &self,
        id: i64,
    ) -> anyhow::Result<Option<PromotionCodeRow>> {
        self.get_promotion_code_by_id_kind(id, "discount").await
    }

    pub async fn update_discount_code(
        &self,
        id: i64,
        input: UpdateDiscountCode,
    ) -> anyhow::Result<Option<PromotionCodeRow>> {
        let now_ts = unix_now();
        let result = sqlx::query(
            r#"
            UPDATE promotion_codes
            SET status = COALESCE(?2, status),
                discount_type = COALESCE(?3, discount_type),
                discount_value = COALESCE(?4, discount_value),
                max_discount_amount = COALESCE(?5, max_discount_amount),
                max_total_uses = COALESCE(?6, max_total_uses),
                max_uses_per_wallet = COALESCE(?7, max_uses_per_wallet),
                first_purchase_only = COALESCE(?8, first_purchase_only),
                stacking_policy = COALESCE(?9, stacking_policy),
                applicable_chain_ids = COALESCE(?10, applicable_chain_ids),
                applicable_ticket_levels = COALESCE(?11, applicable_ticket_levels),
                valid_from = COALESCE(?12, valid_from),
                valid_until = COALESCE(?13, valid_until),
                notes = COALESCE(?14, notes),
                updated_at = ?15
            WHERE id = ?1
              AND kind = 'discount'
            "#,
        )
        .bind(id)
        .bind(input.status)
        .bind(input.discount_type)
        .bind(input.discount_value)
        .bind(input.max_discount_amount)
        .bind(input.max_total_uses)
        .bind(input.max_uses_per_wallet)
        .bind(input.first_purchase_only)
        .bind(input.stacking_policy)
        .bind(input.applicable_chain_ids)
        .bind(input.applicable_ticket_levels)
        .bind(input.valid_from)
        .bind(input.valid_until)
        .bind(input.notes)
        .bind(now_ts)
        .execute(&self.pool)
        .await?;

        if result.rows_affected() == 0 {
            return Ok(None);
        }

        self.get_discount_code_detail(id).await
    }

    pub async fn set_discount_code_status(
        &self,
        id: i64,
        status: &str,
    ) -> anyhow::Result<Option<PromotionCodeRow>> {
        let now_ts = unix_now();
        let result = sqlx::query(
            r#"
            UPDATE promotion_codes
            SET status = ?2,
                updated_at = ?3
            WHERE id = ?1
              AND kind = 'discount'
            "#,
        )
        .bind(id)
        .bind(status)
        .bind(now_ts)
        .execute(&self.pool)
        .await?;

        if result.rows_affected() == 0 {
            return Ok(None);
        }

        self.get_discount_code_detail(id).await
    }

    async fn list_promotion_codes_by_kind(
        &self,
        kind: &str,
        page: i64,
        page_size: i64,
    ) -> anyhow::Result<Vec<PromotionCodeRow>> {
        let page = page.max(1);
        let page_size = page_size.clamp(1, 200);
        let offset = (page - 1) * page_size;
        let rows = sqlx::query_as::<_, PromotionCodeRow>(
            r#"
            SELECT
                id,
                code_normalized,
                kind,
                status,
                beneficiary_wallet,
                valid_from,
                valid_until,
                max_total_uses,
                max_uses_per_wallet,
                first_purchase_only,
                stacking_policy,
                applicable_chain_ids,
                applicable_ticket_levels,
                discount_type,
                discount_value,
                max_discount_amount,
                commission_type,
                commission_value,
                notes,
                created_at,
                updated_at
            FROM promotion_codes
            WHERE kind = ?1
            ORDER BY id DESC
            LIMIT ?2 OFFSET ?3
            "#,
        )
        .bind(kind)
        .bind(page_size)
        .bind(offset)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    async fn get_promotion_code_by_id_kind(
        &self,
        id: i64,
        kind: &str,
    ) -> anyhow::Result<Option<PromotionCodeRow>> {
        let row = sqlx::query_as::<_, PromotionCodeRow>(
            r#"
            SELECT
                id,
                code_normalized,
                kind,
                status,
                beneficiary_wallet,
                valid_from,
                valid_until,
                max_total_uses,
                max_uses_per_wallet,
                first_purchase_only,
                stacking_policy,
                applicable_chain_ids,
                applicable_ticket_levels,
                discount_type,
                discount_value,
                max_discount_amount,
                commission_type,
                commission_value,
                notes,
                created_at,
                updated_at
            FROM promotion_codes
            WHERE id = ?1
              AND kind = ?2
            "#,
        )
        .bind(id)
        .bind(kind)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn get_wallet_referral_binding(
        &self,
        wallet_address: &str,
    ) -> anyhow::Result<Option<WalletReferralBindingRow>> {
        let wallet_key = normalize_wallet_key(wallet_address);
        let row = sqlx::query_as::<_, WalletReferralBindingRow>(
            r#"
            SELECT wallet_address, referral_code_id, bound_at, first_bound_source
            FROM wallet_referral_bindings
            WHERE wallet_address = ?1
            "#,
        )
        .bind(wallet_key)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn list_referral_bindings(
        &self,
        page: i64,
        page_size: i64,
    ) -> anyhow::Result<Vec<AdminReferralBindingRow>> {
        let page = page.max(1);
        let page_size = page_size.clamp(1, 200);
        let offset = (page - 1) * page_size;
        let rows = sqlx::query_as::<_, AdminReferralBindingRow>(
            r#"
            SELECT
                wrb.wallet_address,
                wrb.referral_code_id,
                pc.code_normalized AS referral_code,
                wrb.bound_at,
                wrb.first_bound_source
            FROM wallet_referral_bindings wrb
            JOIN promotion_codes pc ON pc.id = wrb.referral_code_id
            WHERE pc.kind = 'referral'
            ORDER BY wrb.bound_at DESC
            LIMIT ?1 OFFSET ?2
            "#,
        )
        .bind(page_size)
        .bind(offset)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    pub async fn bind_wallet_referral_once(
        &self,
        wallet_address: &str,
        referral_code_id: i64,
        source: &str,
    ) -> anyhow::Result<ReferralBindResult> {
        anyhow::ensure!(
            matches!(source, "signin" | "purchase_intent"),
            "invalid referral bind source"
        );

        let wallet_key = normalize_wallet_key(wallet_address);
        let now_ts = unix_now();
        let result = sqlx::query(
            r#"
            INSERT OR IGNORE INTO wallet_referral_bindings (
                wallet_address,
                referral_code_id,
                bound_at,
                first_bound_source
            ) VALUES (?1, ?2, ?3, ?4)
            "#,
        )
        .bind(&wallet_key)
        .bind(referral_code_id)
        .bind(now_ts)
        .bind(source)
        .execute(&self.pool)
        .await?;

        let effective_referral_code_id = sqlx::query_scalar::<_, i64>(
            r#"
            SELECT referral_code_id
            FROM wallet_referral_bindings
            WHERE wallet_address = ?1
            "#,
        )
        .bind(&wallet_key)
        .fetch_one(&self.pool)
        .await?;

        Ok(ReferralBindResult {
            bound: result.rows_affected() == 1,
            referral_code_id: effective_referral_code_id,
        })
    }

    pub async fn create_purchase_intent(
        &self,
        input: NewPurchaseIntent,
    ) -> anyhow::Result<PurchaseIntentRow> {
        let id = input.resolve_id();
        let wallet_key = normalize_wallet_key(&input.wallet_address);
        let now_ts = unix_now();

        sqlx::query(
            r#"
            INSERT INTO purchase_intents (
                id,
                wallet_address,
                chain_id,
                payment_token,
                level_ids_json,
                quantities_json,
                referral_code_id,
                discount_code_id,
                original_total_amount,
                discount_amount,
                final_total_amount,
                expires_at,
                status,
                tx_hash,
                order_id,
                created_at,
                updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)
            "#,
        )
        .bind(&id)
        .bind(wallet_key)
        .bind(input.chain_id)
        .bind(input.payment_token)
        .bind(input.level_ids_json)
        .bind(input.quantities_json)
        .bind(input.referral_code_id)
        .bind(input.discount_code_id)
        .bind(input.original_total_amount)
        .bind(input.discount_amount)
        .bind(input.final_total_amount)
        .bind(input.expires_at)
        .bind(input.status.as_str())
        .bind(input.tx_hash)
        .bind(input.order_id)
        .bind(now_ts)
        .bind(now_ts)
        .execute(&self.pool)
        .await?;

        let row = self
            .get_purchase_intent(&id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("purchase intent should exist after insert"))?;
        Ok(row)
    }

    pub async fn get_purchase_intent(
        &self,
        intent_id: &str,
    ) -> anyhow::Result<Option<PurchaseIntentRow>> {
        let row = sqlx::query_as::<_, PurchaseIntentRow>(
            r#"
            SELECT
                id,
                wallet_address,
                chain_id,
                payment_token,
                level_ids_json,
                quantities_json,
                referral_code_id,
                discount_code_id,
                original_total_amount,
                discount_amount,
                final_total_amount,
                expires_at,
                status,
                tx_hash,
                order_id,
                created_at,
                updated_at
            FROM purchase_intents
            WHERE id = ?1
            "#,
        )
        .bind(intent_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn list_purchase_intents_admin(
        &self,
        filters: PurchaseIntentFilters,
        page: i64,
        page_size: i64,
    ) -> anyhow::Result<Vec<PurchaseIntentRow>> {
        let page = page.max(1);
        let page_size = page_size.clamp(1, 200);
        let offset = (page - 1) * page_size;
        let mut qb = QueryBuilder::new(
            r#"
            SELECT DISTINCT
                pi.id,
                pi.wallet_address,
                pi.chain_id,
                pi.payment_token,
                pi.level_ids_json,
                pi.quantities_json,
                pi.referral_code_id,
                pi.discount_code_id,
                pi.original_total_amount,
                pi.discount_amount,
                pi.final_total_amount,
                pi.expires_at,
                pi.status,
                pi.tx_hash,
                pi.order_id,
                pi.created_at,
                pi.updated_at
            FROM purchase_intents pi
            LEFT JOIN promotion_codes rc ON rc.id = pi.referral_code_id
            LEFT JOIN promotion_codes dc ON dc.id = pi.discount_code_id
            WHERE 1 = 1
            "#,
        );

        if let Some(wallet) = filters.wallet {
            qb.push(" AND pi.wallet_address = ");
            qb.push_bind(normalize_wallet_key(&wallet));
        }
        if let Some(tx_hash) = filters.tx_hash {
            qb.push(" AND pi.tx_hash = ");
            qb.push_bind(tx_hash);
        }
        if let Some(status) = filters.status {
            qb.push(" AND pi.status = ");
            qb.push_bind(status);
        }
        if let Some(code) = filters
            .code
            .and_then(|value| normalize_promotion_code(&value))
        {
            qb.push(" AND (rc.code_normalized = ");
            qb.push_bind(code.clone());
            qb.push(" OR dc.code_normalized = ");
            qb.push_bind(code);
            qb.push(")");
        }

        qb.push(" ORDER BY pi.created_at DESC LIMIT ");
        qb.push_bind(page_size);
        qb.push(" OFFSET ");
        qb.push_bind(offset);

        let rows = qb.build_query_as().fetch_all(&self.pool).await?;
        Ok(rows)
    }

    pub async fn get_purchase_intent_diagnostic(
        &self,
        intent_id: &str,
    ) -> anyhow::Result<Option<PurchaseIntentDiagnostic>> {
        let Some(intent) = self.get_purchase_intent(intent_id).await? else {
            return Ok(None);
        };
        let discount_redemption = self.get_discount_redemption(intent_id).await?;
        let linked_order = self.find_order_for_purchase_intent(&intent).await?;

        Ok(Some(PurchaseIntentDiagnostic {
            intent,
            discount_redemption,
            linked_order,
        }))
    }

    pub async fn reserve_discount_redemption(
        &self,
        input: NewDiscountRedemption,
    ) -> anyhow::Result<DiscountRedemptionRow> {
        let wallet_key = normalize_wallet_key(&input.wallet_address);
        sqlx::query(
            r#"
            INSERT INTO discount_redemptions (
                purchase_intent_id,
                discount_code_id,
                wallet_address,
                status,
                tx_hash,
                order_id,
                reserved_at,
                confirmed_at,
                released_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
            ON CONFLICT(purchase_intent_id) DO UPDATE SET
                discount_code_id = excluded.discount_code_id,
                wallet_address = excluded.wallet_address,
                status = excluded.status,
                tx_hash = excluded.tx_hash,
                order_id = excluded.order_id,
                reserved_at = excluded.reserved_at,
                confirmed_at = excluded.confirmed_at,
                released_at = excluded.released_at
            "#,
        )
        .bind(&input.purchase_intent_id)
        .bind(input.discount_code_id)
        .bind(wallet_key)
        .bind(input.status.as_str())
        .bind(input.tx_hash)
        .bind(input.order_id)
        .bind(input.reserved_at)
        .bind(input.confirmed_at)
        .bind(input.released_at)
        .execute(&self.pool)
        .await?;

        let row = self
            .get_discount_redemption(&input.purchase_intent_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("discount redemption should exist after reserve"))?;
        Ok(row)
    }

    pub async fn get_discount_redemption(
        &self,
        purchase_intent_id: &str,
    ) -> anyhow::Result<Option<DiscountRedemptionRow>> {
        let row = sqlx::query_as::<_, DiscountRedemptionRow>(
            r#"
            SELECT
                purchase_intent_id,
                discount_code_id,
                wallet_address,
                status,
                tx_hash,
                order_id,
                reserved_at,
                confirmed_at,
                released_at
            FROM discount_redemptions
            WHERE purchase_intent_id = ?1
            "#,
        )
        .bind(purchase_intent_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn count_active_discount_redemptions(
        &self,
        discount_code_id: i64,
    ) -> anyhow::Result<i64> {
        let count = sqlx::query_scalar::<_, i64>(
            r#"
            SELECT COUNT(1)
            FROM discount_redemptions
            WHERE discount_code_id = ?1
              AND status IN ('reserved', 'confirmed')
            "#,
        )
        .bind(discount_code_id)
        .fetch_one(&self.pool)
        .await?;

        Ok(count)
    }

    pub async fn count_confirmed_discount_redemptions(
        &self,
        discount_code_id: i64,
    ) -> anyhow::Result<i64> {
        let count = sqlx::query_scalar::<_, i64>(
            r#"
            SELECT COUNT(1)
            FROM discount_redemptions
            WHERE discount_code_id = ?1
              AND status = 'confirmed'
            "#,
        )
        .bind(discount_code_id)
        .fetch_one(&self.pool)
        .await?;

        Ok(count)
    }

    pub async fn count_active_discount_redemptions_for_wallet(
        &self,
        discount_code_id: i64,
        wallet_address: &str,
    ) -> anyhow::Result<i64> {
        let wallet_key = normalize_wallet_key(wallet_address);
        let count = sqlx::query_scalar::<_, i64>(
            r#"
            SELECT COUNT(1)
            FROM discount_redemptions
            WHERE discount_code_id = ?1
              AND wallet_address = ?2
              AND status IN ('reserved', 'confirmed')
            "#,
        )
        .bind(discount_code_id)
        .bind(wallet_key)
        .fetch_one(&self.pool)
        .await?;

        Ok(count)
    }

    pub async fn count_confirmed_discount_redemptions_for_wallet(
        &self,
        discount_code_id: i64,
        wallet_address: &str,
    ) -> anyhow::Result<i64> {
        let wallet_key = normalize_wallet_key(wallet_address);
        let count = sqlx::query_scalar::<_, i64>(
            r#"
            SELECT COUNT(1)
            FROM discount_redemptions
            WHERE discount_code_id = ?1
              AND wallet_address = ?2
              AND status = 'confirmed'
            "#,
        )
        .bind(discount_code_id)
        .bind(wallet_key)
        .fetch_one(&self.pool)
        .await?;

        Ok(count)
    }

    pub async fn release_pending_discount_redemptions_for_wallet_code(
        &self,
        discount_code_id: i64,
        wallet_address: &str,
        released_at: i64,
    ) -> anyhow::Result<u64> {
        let wallet_key = normalize_wallet_key(wallet_address);
        let result = sqlx::query(
            r#"
            UPDATE discount_redemptions
            SET status = ?3,
                released_at = ?4
            WHERE discount_code_id = ?1
              AND wallet_address = ?2
              AND status = 'reserved'
              AND purchase_intent_id IN (
                SELECT id
                FROM purchase_intents
                WHERE wallet_address = ?2
                  AND discount_code_id = ?1
                  AND status = 'pending'
                  AND tx_hash IS NULL
                  AND order_id IS NULL
              )
            "#,
        )
        .bind(discount_code_id)
        .bind(wallet_key)
        .bind(DiscountRedemptionStatus::Released.as_str())
        .bind(released_at)
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected())
    }

    pub async fn confirm_discount_redemption(
        &self,
        purchase_intent_id: &str,
        tx_hash: Option<&str>,
        order_id: Option<&str>,
    ) -> anyhow::Result<bool> {
        let now_ts = unix_now();
        let result = sqlx::query(
            r#"
            UPDATE discount_redemptions
            SET status = ?2,
                tx_hash = ?3,
                order_id = ?4,
                confirmed_at = ?5,
                released_at = NULL
            WHERE purchase_intent_id = ?1
            "#,
        )
        .bind(purchase_intent_id)
        .bind(DiscountRedemptionStatus::Confirmed.as_str())
        .bind(tx_hash)
        .bind(order_id)
        .bind(now_ts)
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected() == 1)
    }

    pub async fn release_discount_redemption(
        &self,
        purchase_intent_id: &str,
    ) -> anyhow::Result<bool> {
        let now_ts = unix_now();
        let result = sqlx::query(
            r#"
            UPDATE discount_redemptions
            SET status = ?2,
                released_at = ?3
            WHERE purchase_intent_id = ?1
            "#,
        )
        .bind(purchase_intent_id)
        .bind(DiscountRedemptionStatus::Released.as_str())
        .bind(now_ts)
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected() == 1)
    }

    pub async fn insert_order_promotions_snapshot(
        &self,
        input: NewOrderPromotionsSnapshot,
    ) -> anyhow::Result<bool> {
        let result = sqlx::query(
            r#"
            INSERT OR IGNORE INTO order_promotions_snapshot (
                order_row_id,
                wallet_address,
                referral_code_id,
                discount_code_id,
                original_total_amount,
                discount_amount,
                paid_amount,
                commission_base_amount,
                commission_amount,
                rule_version,
                created_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
            "#,
        )
        .bind(input.order_row_id)
        .bind(normalize_wallet_key(&input.wallet_address))
        .bind(input.referral_code_id)
        .bind(input.discount_code_id)
        .bind(input.original_total_amount)
        .bind(input.discount_amount)
        .bind(input.paid_amount)
        .bind(input.commission_base_amount)
        .bind(input.commission_amount)
        .bind(input.rule_version)
        .bind(input.created_at)
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected() == 1)
    }

    pub async fn get_order_promotions_snapshot(
        &self,
        order_row_id: i64,
    ) -> anyhow::Result<Option<OrderPromotionsSnapshotRow>> {
        let row = sqlx::query_as::<_, OrderPromotionsSnapshotRow>(
            r#"
            SELECT
                order_row_id,
                wallet_address,
                referral_code_id,
                discount_code_id,
                original_total_amount,
                discount_amount,
                paid_amount,
                commission_base_amount,
                commission_amount,
                rule_version,
                created_at
            FROM order_promotions_snapshot
            WHERE order_row_id = ?1
            "#,
        )
        .bind(order_row_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn list_orders_admin(
        &self,
        filters: OrderFilters,
        page: i64,
        page_size: i64,
    ) -> anyhow::Result<Vec<AdminOrderRow>> {
        let page = page.max(1);
        let page_size = page_size.clamp(1, 200);
        let offset = (page - 1) * page_size;
        let mut qb = QueryBuilder::new(
            r#"
            SELECT DISTINCT
                o.id,
                o.chain_id,
                o.tx_hash,
                o.log_index,
                o.block_number,
                o.block_hash,
                o.order_id,
                o.buyer_address,
                o.payment_token,
                o.total_amount,
                o.created_at,
                ops.original_total_amount,
                ops.discount_amount,
                ops.paid_amount
            FROM orders o
            LEFT JOIN order_promotions_snapshot ops ON ops.order_row_id = o.id
            LEFT JOIN promotion_codes rc ON rc.id = ops.referral_code_id
            LEFT JOIN promotion_codes dc ON dc.id = ops.discount_code_id
            WHERE 1 = 1
            "#,
        );

        if let Some(wallet) = filters.wallet {
            qb.push(" AND o.buyer_address = ");
            qb.push_bind(normalize_wallet_key(&wallet));
        }
        if let Some(tx_hash) = filters.tx_hash {
            qb.push(" AND o.tx_hash = ");
            qb.push_bind(tx_hash);
        }
        if let Some(invite_code) = filters
            .invite_code
            .and_then(|value| normalize_promotion_code(&value))
        {
            qb.push(" AND rc.code_normalized = ");
            qb.push_bind(invite_code);
        }
        if let Some(discount_code) = filters
            .discount_code
            .and_then(|value| normalize_promotion_code(&value))
        {
            qb.push(" AND dc.code_normalized = ");
            qb.push_bind(discount_code);
        }

        qb.push(" ORDER BY o.created_at DESC LIMIT ");
        qb.push_bind(page_size);
        qb.push(" OFFSET ");
        qb.push_bind(offset);

        let rows = qb
            .build_query_as::<AdminOrderRecord>()
            .fetch_all(&self.pool)
            .await?;
        self.hydrate_admin_order_rows(rows).await
    }

    pub async fn get_order_attribution(
        &self,
        order_row_id: i64,
    ) -> anyhow::Result<Option<OrderAttributionDiagnostic>> {
        let Some(order) = self.get_order_admin(order_row_id).await? else {
            return Ok(None);
        };
        let snapshot = self.get_order_promotions_snapshot(order_row_id).await?;
        let invite_code = match snapshot.as_ref().and_then(|row| row.referral_code_id) {
            Some(id) => self.get_invite_code_detail(id).await?,
            None => None,
        };
        let discount_code = match snapshot.as_ref().and_then(|row| row.discount_code_id) {
            Some(id) => self.get_discount_code_detail(id).await?,
            None => None,
        };

        Ok(Some(OrderAttributionDiagnostic {
            order,
            snapshot,
            invite_code,
            discount_code,
        }))
    }

    pub async fn list_referral_settlements(&self) -> anyhow::Result<Vec<ReferralSettlementRow>> {
        let rows = sqlx::query_as::<_, SettlementSourceRow>(
            r#"
            SELECT
                pc.id AS invite_code_id,
                pc.code_normalized AS invite_code,
                pc.beneficiary_wallet AS beneficiary_wallet,
                o.chain_id AS chain_id,
                o.payment_token AS payment_token,
                ops.paid_amount AS paid_amount,
                ops.commission_base_amount AS commission_base_amount,
                ops.commission_amount AS commission_amount,
                pc.commission_type AS commission_type,
                pc.commission_value AS commission_value
            FROM order_promotions_snapshot ops
            JOIN orders o ON o.id = ops.order_row_id
            JOIN promotion_codes pc ON pc.id = ops.referral_code_id
            WHERE pc.kind = 'referral'
            ORDER BY pc.code_normalized ASC
            "#,
        )
        .fetch_all(&self.pool)
        .await?;

        let mut grouped: HashMap<i64, ReferralSettlementAccumulator> = HashMap::new();
        for row in rows {
            let entry = grouped.entry(row.invite_code_id).or_insert_with(|| {
                ReferralSettlementAccumulator {
                    invite_code_id: row.invite_code_id,
                    invite_code: row.invite_code,
                    beneficiary_wallet: row.beneficiary_wallet,
                    confirmed_order_count: 0,
                    paid_amount_total: U256::zero(),
                    commission_base_amount_total: U256::zero(),
                    commission_amount_total: U256::zero(),
                }
            });

            let commission_base_amount = U256::from_dec_str(&row.commission_base_amount)?;
            let mut commission_amount = U256::from_dec_str(&row.commission_amount)?;
            if commission_amount.is_zero() {
                commission_amount = calculate_commission_amount_from_rule(
                    row.commission_type.as_deref(),
                    row.commission_value.as_deref(),
                    row.chain_id as u64,
                    &row.payment_token,
                    commission_base_amount,
                )?;
            }

            entry.confirmed_order_count += 1;
            entry.paid_amount_total += U256::from_dec_str(&row.paid_amount)?;
            entry.commission_base_amount_total += commission_base_amount;
            entry.commission_amount_total += commission_amount;
        }

        let mut output = grouped
            .into_values()
            .map(ReferralSettlementAccumulator::into_row)
            .collect::<Vec<_>>();
        output.sort_by(|a, b| a.invite_code.cmp(&b.invite_code));
        Ok(output)
    }

    async fn hydrate_admin_order_rows(
        &self,
        rows: Vec<AdminOrderRecord>,
    ) -> anyhow::Result<Vec<AdminOrderRow>> {
        let mut hydrated = Vec::with_capacity(rows.len());
        for row in rows {
            let line_items = self.list_admin_order_line_items(&row).await?;
            hydrated.push(row.with_line_items(line_items));
        }
        Ok(hydrated)
    }

    async fn list_admin_order_line_items(
        &self,
        order: &AdminOrderRecord,
    ) -> anyhow::Result<Vec<AdminOrderLineItemRow>> {
        let rows = sqlx::query_as::<_, AdminOrderLineItemRow>(
            r#"
            SELECT
                ticket_level,
                COUNT(1) AS quantity,
                unit_price
            FROM tickets
            WHERE source_order_row_id = ?1
            GROUP BY ticket_level, unit_price
            ORDER BY ticket_level ASC, unit_price ASC
            "#,
        )
        .bind(order.id)
        .fetch_all(&self.pool)
        .await?;

        if !rows.is_empty() {
            return Ok(rows);
        }

        if let Some(rows) = self
            .list_admin_order_line_items_from_purchase_intent(order)
            .await?
        {
            return Ok(rows);
        }

        Ok(rows)
    }

    async fn list_admin_order_line_items_from_purchase_intent(
        &self,
        order: &AdminOrderRecord,
    ) -> anyhow::Result<Option<Vec<AdminOrderLineItemRow>>> {
        let Some((level_ids_json, quantities_json)) = sqlx::query_as::<_, (String, String)>(
            r#"
            SELECT level_ids_json, quantities_json
            FROM purchase_intents
            WHERE order_id = ?1
               OR tx_hash = ?2
            ORDER BY updated_at DESC, created_at DESC
            LIMIT 1
            "#,
        )
        .bind(&order.order_id)
        .bind(&order.tx_hash)
        .fetch_optional(&self.pool)
        .await?
        else {
            return Ok(None);
        };

        let level_ids: Vec<i64> = serde_json::from_str(&level_ids_json)?;
        let quantities: Vec<i64> = serde_json::from_str(&quantities_json)?;
        if level_ids.len() != quantities.len() {
            return Ok(None);
        }

        let mut grouped: HashMap<i64, i64> = HashMap::new();
        for (level, quantity) in level_ids.into_iter().zip(quantities) {
            *grouped.entry(level).or_insert(0) += quantity;
        }

        let mut rows = grouped
            .into_iter()
            .map(|(ticket_level, quantity)| AdminOrderLineItemRow {
                ticket_level,
                quantity,
                unit_price: "0".to_string(),
            })
            .collect::<Vec<_>>();
        rows.sort_by_key(|row| row.ticket_level);

        Ok(Some(rows))
    }

    async fn get_order_admin(&self, order_row_id: i64) -> anyhow::Result<Option<AdminOrderRow>> {
        let row = sqlx::query_as::<_, AdminOrderRecord>(
            r#"
            SELECT
                id,
                chain_id,
                tx_hash,
                log_index,
                block_number,
                block_hash,
                order_id,
                buyer_address,
                payment_token,
                total_amount,
                created_at,
                NULL AS original_total_amount,
                NULL AS discount_amount,
                NULL AS paid_amount
            FROM orders
            WHERE id = ?1
            "#,
        )
        .bind(order_row_id)
        .fetch_optional(&self.pool)
        .await?;

        let Some(row) = row else {
            return Ok(None);
        };
        let line_items = self.list_admin_order_line_items(&row).await?;
        Ok(Some(row.with_line_items(line_items)))
    }

    async fn find_order_for_purchase_intent(
        &self,
        intent: &PurchaseIntentRow,
    ) -> anyhow::Result<Option<AdminOrderRow>> {
        let row = sqlx::query_as::<_, AdminOrderRecord>(
            r#"
            SELECT
                id,
                chain_id,
                tx_hash,
                log_index,
                block_number,
                block_hash,
                order_id,
                buyer_address,
                payment_token,
                total_amount,
                created_at,
                NULL AS original_total_amount,
                NULL AS discount_amount,
                NULL AS paid_amount
            FROM orders
            WHERE (?1 IS NOT NULL AND order_id = ?1)
               OR (?2 IS NOT NULL AND tx_hash = ?2)
            ORDER BY id DESC
            LIMIT 1
            "#,
        )
        .bind(intent.order_id.as_deref())
        .bind(intent.tx_hash.as_deref())
        .fetch_optional(&self.pool)
        .await?;

        let Some(row) = row else {
            return Ok(None);
        };
        let line_items = self.list_admin_order_line_items(&row).await?;
        Ok(Some(row.with_line_items(line_items)))
    }

    pub async fn mark_purchase_intent_confirmed(
        &self,
        intent_id: &str,
        tx_hash: Option<&str>,
        order_id: Option<&str>,
    ) -> anyhow::Result<bool> {
        let now_ts = unix_now();
        let result = sqlx::query(
            r#"
            UPDATE purchase_intents
            SET status = ?2,
                tx_hash = ?3,
                order_id = ?4,
                updated_at = ?5
            WHERE id = ?1
            "#,
        )
        .bind(intent_id)
        .bind(PurchaseIntentStatus::Confirmed.as_str())
        .bind(tx_hash)
        .bind(order_id)
        .bind(now_ts)
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected() == 1)
    }

    pub async fn list_active_tickets_by_wallet(
        &self,
        wallet: &str,
    ) -> anyhow::Result<Vec<TicketRow>> {
        let rows = sqlx::query_as::<_, TicketRow>(
            r#"
            SELECT id, chain_id, order_id, owner_wallet, owner_email, ticket_level, unit_price, qr_payload, qr_version, status, created_at, updated_at
            FROM tickets
            WHERE owner_wallet = ?1 AND status = 'active'
            ORDER BY created_at DESC
            "#,
        )
        .bind(wallet)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    pub async fn list_active_tickets_by_email(
        &self,
        email: &str,
    ) -> anyhow::Result<Vec<TicketRow>> {
        let rows = sqlx::query_as::<_, TicketRow>(
            r#"
            SELECT id, chain_id, order_id, owner_wallet, owner_email, ticket_level, unit_price, qr_payload, qr_version, status, created_at, updated_at
            FROM tickets
            WHERE owner_email = ?1 AND status = 'active'
            ORDER BY created_at DESC
            "#,
        )
        .bind(email)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    pub async fn get_active_ticket_by_id_for_wallet(
        &self,
        ticket_id: &str,
        wallet: &str,
    ) -> anyhow::Result<Option<TicketRow>> {
        let row = sqlx::query_as::<_, TicketRow>(
            r#"
            SELECT id, chain_id, order_id, owner_wallet, owner_email, ticket_level, unit_price, qr_payload, qr_version, status, created_at, updated_at
            FROM tickets
            WHERE id = ?1 AND owner_wallet = ?2 AND status = 'active'
            "#,
        )
        .bind(ticket_id)
        .bind(wallet)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn get_active_ticket_by_id_for_email(
        &self,
        ticket_id: &str,
        email: &str,
    ) -> anyhow::Result<Option<TicketRow>> {
        let row = sqlx::query_as::<_, TicketRow>(
            r#"
            SELECT id, chain_id, order_id, owner_wallet, owner_email, ticket_level, unit_price, qr_payload, qr_version, status, created_at, updated_at
            FROM tickets
            WHERE id = ?1 AND owner_email = ?2 AND status = 'active'
            "#,
        )
        .bind(ticket_id)
        .bind(email)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn index_purchase(
        &self,
        chain_id: u64,
        purchase: &DecodedPurchase,
    ) -> anyhow::Result<NotifyResult> {
        let mut tx = self.pool.begin().await?;
        let now_ts = unix_now();

        let insert_result = sqlx::query(
            r#"
            INSERT OR IGNORE INTO orders (
                chain_id,
                tx_hash,
                log_index,
                block_number,
                block_hash,
                order_id,
                buyer_address,
                payment_token,
                total_amount,
                created_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
            "#,
        )
        .bind(chain_id as i64)
        .bind(&purchase.tx_hash)
        .bind(purchase.log_index as i64)
        .bind(purchase.block_number as i64)
        .bind(purchase.block_hash.clone().unwrap_or_default())
        .bind(&purchase.order_id)
        .bind(&purchase.buyer)
        .bind(&purchase.payment_token)
        .bind(&purchase.total_amount)
        .bind(now_ts)
        .execute(&mut *tx)
        .await?;

        let created_order = insert_result.rows_affected() == 1;

        let order_row_id: i64 = sqlx::query_scalar(
            r#"
            SELECT id
            FROM orders
            WHERE chain_id = ?1 AND tx_hash = ?2 AND log_index = ?3
            "#,
        )
        .bind(chain_id as i64)
        .bind(&purchase.tx_hash)
        .bind(purchase.log_index as i64)
        .fetch_one(&mut *tx)
        .await?;

        let existing_tickets: i64 =
            sqlx::query_scalar("SELECT COUNT(1) FROM tickets WHERE source_order_row_id = ?1")
                .bind(order_row_id)
                .fetch_one(&mut *tx)
                .await?;

        let mut created_tickets = 0usize;
        if existing_tickets == 0 {
            let line_item_len = purchase.level_ids.len();
            for i in 0..line_item_len {
                let quantity = purchase.quantities[i];
                let level = purchase.level_ids[i] as i64;
                let unit_price = purchase.unit_prices[i].clone();

                for _ in 0..quantity {
                    let ticket_id = Uuid::new_v4().to_string();
                    let qr_payload = format!(
                        "money-frontier:qr:{}:1:{}",
                        ticket_id,
                        Uuid::new_v4().simple()
                    );

                    sqlx::query(
                        r#"
                        INSERT INTO tickets (
                            id,
                            chain_id,
                            order_id,
                            source_order_row_id,
                            owner_wallet,
                            owner_email,
                            ticket_level,
                            unit_price,
                            qr_payload,
                            qr_version,
                            status,
                            created_at,
                            updated_at
                        ) VALUES (?1, ?2, ?3, ?4, ?5, NULL, ?6, ?7, ?8, 1, 'active', ?9, ?9)
                        "#,
                    )
                    .bind(&ticket_id)
                    .bind(chain_id as i64)
                    .bind(&purchase.order_id)
                    .bind(order_row_id)
                    .bind(&purchase.buyer)
                    .bind(level)
                    .bind(unit_price.clone())
                    .bind(qr_payload)
                    .bind(now_ts)
                    .execute(&mut *tx)
                    .await?;

                    created_tickets += 1;
                }
            }
        }

        if let Some(intent_id) = purchase.intent_id.as_deref() {
            let intent = sqlx::query_as::<_, PurchaseIntentRow>(
                r#"
                SELECT
                    id,
                    wallet_address,
                    chain_id,
                    payment_token,
                    level_ids_json,
                    quantities_json,
                    referral_code_id,
                    discount_code_id,
                    original_total_amount,
                    discount_amount,
                    final_total_amount,
                    expires_at,
                    status,
                    tx_hash,
                    order_id,
                    created_at,
                    updated_at
                FROM purchase_intents
                WHERE id = ?1
                "#,
            )
            .bind(intent_id)
            .fetch_optional(&mut *tx)
            .await?;

            if let Some(intent) = intent {
                if intent.wallet_address == normalize_wallet_key(&purchase.buyer) {
                    sqlx::query(
                        r#"
                        UPDATE purchase_intents
                        SET status = ?2,
                            tx_hash = ?3,
                            order_id = ?4,
                            updated_at = ?5
                        WHERE id = ?1
                        "#,
                    )
                    .bind(intent_id)
                    .bind(PurchaseIntentStatus::Confirmed.as_str())
                    .bind(&purchase.tx_hash)
                    .bind(&purchase.order_id)
                    .bind(now_ts)
                    .execute(&mut *tx)
                    .await?;

                    if intent.discount_code_id.is_some() {
                        sqlx::query(
                            r#"
                            UPDATE discount_redemptions
                            SET status = ?2,
                                tx_hash = ?3,
                                order_id = ?4,
                                confirmed_at = ?5,
                                released_at = NULL
                            WHERE purchase_intent_id = ?1
                            "#,
                        )
                        .bind(intent_id)
                        .bind(DiscountRedemptionStatus::Confirmed.as_str())
                        .bind(&purchase.tx_hash)
                        .bind(&purchase.order_id)
                        .bind(now_ts)
                        .execute(&mut *tx)
                        .await?;
                    }

                    let commission_base_amount = U256::from_dec_str(&purchase.total_amount)?;
                    let commission_amount = calculate_referral_commission_amount(
                        &mut tx,
                        intent.referral_code_id,
                        chain_id,
                        &purchase.payment_token,
                        commission_base_amount,
                    )
                    .await?;

                    sqlx::query(
                        r#"
                        INSERT OR IGNORE INTO order_promotions_snapshot (
                            order_row_id,
                            wallet_address,
                            referral_code_id,
                            discount_code_id,
                            original_total_amount,
                            discount_amount,
                            paid_amount,
                            commission_base_amount,
                            commission_amount,
                            rule_version,
                            created_at
                        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
                        "#,
                    )
                    .bind(order_row_id)
                    .bind(normalize_wallet_key(&purchase.buyer))
                    .bind(intent.referral_code_id)
                    .bind(intent.discount_code_id)
                    .bind(intent.original_total_amount)
                    .bind(intent.discount_amount)
                    .bind(&purchase.total_amount)
                    .bind(&purchase.total_amount)
                    .bind(commission_amount.to_string())
                    .bind("v1")
                    .bind(now_ts)
                    .execute(&mut *tx)
                    .await?;
                }
            }
        }

        tx.commit().await?;

        Ok(NotifyResult {
            created_order,
            created_tickets,
        })
    }

    pub async fn transfer_ticket(
        &self,
        ticket_id: &str,
        from_wallet: &str,
        to_wallet: Option<&str>,
        to_email: Option<&str>,
    ) -> anyhow::Result<Option<TicketRow>> {
        let mut tx = self.pool.begin().await?;
        let now_ts = unix_now();

        let current_ticket = sqlx::query_as::<_, TicketRow>(
            r#"
            SELECT id, chain_id, order_id, owner_wallet, owner_email, ticket_level, unit_price, qr_payload, qr_version, status, created_at, updated_at
            FROM tickets
            WHERE id = ?1 AND owner_wallet = ?2 AND status = 'active'
            "#,
        )
        .bind(ticket_id)
        .bind(from_wallet)
        .fetch_optional(&mut *tx)
        .await?;

        let Some(current_ticket) = current_ticket else {
            tx.rollback().await?;
            return Ok(None);
        };

        sqlx::query("UPDATE tickets SET status = 'transferred_out', updated_at = ?2 WHERE id = ?1")
            .bind(&current_ticket.id)
            .bind(now_ts)
            .execute(&mut *tx)
            .await?;

        let source_order_row_id: i64 =
            sqlx::query_scalar("SELECT source_order_row_id FROM tickets WHERE id = ?1")
                .bind(&current_ticket.id)
                .fetch_one(&mut *tx)
                .await?;

        let new_ticket_id = Uuid::new_v4().to_string();
        let new_qr_payload = format!(
            "money-frontier:qr:{}:1:{}",
            new_ticket_id,
            Uuid::new_v4().simple()
        );

        sqlx::query(
            r#"
            INSERT INTO tickets (
                id,
                chain_id,
                order_id,
                source_order_row_id,
                owner_wallet,
                owner_email,
                ticket_level,
                unit_price,
                qr_payload,
                qr_version,
                status,
                created_at,
                updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 1, 'active', ?10, ?10)
            "#,
        )
        .bind(&new_ticket_id)
        .bind(current_ticket.chain_id)
        .bind(&current_ticket.order_id)
        .bind(source_order_row_id)
        .bind(to_wallet)
        .bind(to_email)
        .bind(current_ticket.ticket_level)
        .bind(&current_ticket.unit_price)
        .bind(new_qr_payload)
        .bind(now_ts)
        .execute(&mut *tx)
        .await?;

        let new_ticket = sqlx::query_as::<_, TicketRow>(
            r#"
            SELECT id, chain_id, order_id, owner_wallet, owner_email, ticket_level, unit_price, qr_payload, qr_version, status, created_at, updated_at
            FROM tickets
            WHERE id = ?1
            "#,
        )
        .bind(&new_ticket_id)
        .fetch_one(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok(Some(new_ticket))
    }

    pub async fn get_indexer_cursor(&self, chain_id: u64) -> anyhow::Result<Option<IndexerCursor>> {
        let row = sqlx::query_as::<_, (i64, Option<String>)>(
            r#"
            SELECT last_indexed_block, last_indexed_block_hash
            FROM indexer_cursors
            WHERE chain_id = ?1
            "#,
        )
        .bind(chain_id as i64)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(
            |(last_indexed_block, last_indexed_block_hash)| IndexerCursor {
                last_indexed_block: last_indexed_block as u64,
                last_indexed_block_hash,
            },
        ))
    }

    pub async fn set_indexer_cursor(
        &self,
        chain_id: u64,
        last_indexed_block: u64,
        last_indexed_block_hash: Option<&str>,
    ) -> anyhow::Result<()> {
        let now_ts = unix_now();
        sqlx::query(
            r#"
            INSERT INTO indexer_cursors (chain_id, last_indexed_block, last_indexed_block_hash, updated_at)
            VALUES (?1, ?2, ?3, ?4)
            ON CONFLICT(chain_id)
            DO UPDATE SET
                last_indexed_block = excluded.last_indexed_block,
                last_indexed_block_hash = excluded.last_indexed_block_hash,
                updated_at = excluded.updated_at
            "#,
        )
        .bind(chain_id as i64)
        .bind(last_indexed_block as i64)
        .bind(last_indexed_block_hash)
        .bind(now_ts)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn rollback_chain_from_block(
        &self,
        chain_id: u64,
        from_block: u64,
    ) -> anyhow::Result<RollbackResult> {
        let mut tx = self.pool.begin().await?;

        let deleted_tickets = sqlx::query(
            r#"
            DELETE FROM tickets
            WHERE source_order_row_id IN (
                SELECT id
                FROM orders
                WHERE chain_id = ?1 AND block_number >= ?2
            )
            "#,
        )
        .bind(chain_id as i64)
        .bind(from_block as i64)
        .execute(&mut *tx)
        .await?
        .rows_affected();

        let deleted_orders = sqlx::query(
            r#"
            DELETE FROM orders
            WHERE chain_id = ?1 AND block_number >= ?2
            "#,
        )
        .bind(chain_id as i64)
        .bind(from_block as i64)
        .execute(&mut *tx)
        .await?
        .rows_affected();

        tx.commit().await?;

        Ok(RollbackResult {
            deleted_orders,
            deleted_tickets,
        })
    }

    pub async fn count_orders_by_wallet(&self, wallet_address: &str) -> anyhow::Result<i64> {
        let wallet_key = normalize_wallet_key(wallet_address);
        let count = sqlx::query_scalar::<_, i64>(
            r#"
            SELECT COUNT(1)
            FROM orders
            WHERE buyer_address = ?1
            "#,
        )
        .bind(wallet_key)
        .fetch_one(&self.pool)
        .await?;

        Ok(count)
    }

    #[cfg(test)]
    pub async fn count_orders(&self, chain_id: u64) -> anyhow::Result<i64> {
        let count = sqlx::query_scalar::<_, i64>("SELECT COUNT(1) FROM orders WHERE chain_id = ?1")
            .bind(chain_id as i64)
            .fetch_one(&self.pool)
            .await?;
        Ok(count)
    }

    #[cfg(test)]
    pub async fn find_order_row_id(
        &self,
        chain_id: u64,
        tx_hash: &str,
        log_index: u64,
    ) -> anyhow::Result<Option<i64>> {
        let row = sqlx::query_scalar::<_, i64>(
            r#"
            SELECT id
            FROM orders
            WHERE chain_id = ?1
              AND tx_hash = ?2
              AND log_index = ?3
            "#,
        )
        .bind(chain_id as i64)
        .bind(tx_hash)
        .bind(log_index as i64)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    #[cfg(test)]
    pub async fn count_tickets(&self, chain_id: u64) -> anyhow::Result<i64> {
        let count =
            sqlx::query_scalar::<_, i64>("SELECT COUNT(1) FROM tickets WHERE chain_id = ?1")
                .bind(chain_id as i64)
                .fetch_one(&self.pool)
                .await?;
        Ok(count)
    }

    #[cfg(test)]
    pub async fn list_order_ids(&self, chain_id: u64) -> anyhow::Result<Vec<String>> {
        let rows = sqlx::query_scalar::<_, String>(
            r#"
            SELECT order_id
            FROM orders
            WHERE chain_id = ?1
            ORDER BY order_id ASC
            "#,
        )
        .bind(chain_id as i64)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    #[cfg(test)]
    pub async fn seed_order(
        &self,
        chain_id: u64,
        tx_hash: &str,
        log_index: u64,
        order_id: &str,
        buyer_address: &str,
        total_amount: &str,
    ) -> anyhow::Result<i64> {
        let now_ts = unix_now();
        let result = sqlx::query(
            r#"
            INSERT INTO orders (
                chain_id,
                tx_hash,
                log_index,
                block_number,
                block_hash,
                order_id,
                buyer_address,
                payment_token,
                total_amount,
                created_at
            ) VALUES (?1, ?2, ?3, 100, '0xblock', ?4, ?5, '0x3333333333333333333333333333333333333333', ?6, ?7)
            "#,
        )
        .bind(chain_id as i64)
        .bind(tx_hash)
        .bind(log_index as i64)
        .bind(order_id)
        .bind(normalize_wallet_key(buyer_address))
        .bind(total_amount)
        .bind(now_ts)
        .execute(&self.pool)
        .await?;

        Ok(result.last_insert_rowid())
    }

    #[cfg(test)]
    pub async fn seed_ticket_for_order(
        &self,
        order_row_id: i64,
        ticket_level: i64,
        unit_price: &str,
    ) -> anyhow::Result<String> {
        let now_ts = unix_now();
        let ticket_id = Uuid::new_v4().to_string();
        let order_id = sqlx::query_scalar::<_, String>(
            r#"
            SELECT order_id
            FROM orders
            WHERE id = ?1
            "#,
        )
        .bind(order_row_id)
        .fetch_one(&self.pool)
        .await?;

        sqlx::query(
            r#"
            INSERT INTO tickets (
                id,
                chain_id,
                order_id,
                source_order_row_id,
                owner_wallet,
                owner_email,
                ticket_level,
                unit_price,
                qr_payload,
                qr_version,
                status,
                created_at,
                updated_at
            ) VALUES (?1, 56, ?2, ?3, NULL, NULL, ?4, ?5, ?6, 1, 'active', ?7, ?7)
            "#,
        )
        .bind(&ticket_id)
        .bind(order_id)
        .bind(order_row_id)
        .bind(ticket_level)
        .bind(unit_price)
        .bind(format!("qr-{ticket_id}"))
        .bind(now_ts)
        .execute(&self.pool)
        .await?;

        Ok(ticket_id)
    }

    #[cfg(test)]
    pub async fn seed_referral_code(&self, code: &str) -> anyhow::Result<i64> {
        self.seed_promotion_code(code, "referral").await
    }

    #[cfg(test)]
    pub async fn seed_discount_code(&self, code: &str) -> anyhow::Result<i64> {
        self.seed_promotion_code(code, "discount").await
    }

    #[cfg(test)]
    pub async fn seed_fixed_discount_code(&self, code: &str, amount: &str) -> anyhow::Result<i64> {
        let id = self.seed_promotion_code(code, "discount").await?;
        sqlx::query(
            r#"
            UPDATE promotion_codes
            SET discount_type = 'fixed',
                discount_value = ?2
            WHERE id = ?1
            "#,
        )
        .bind(id)
        .bind(amount)
        .execute(&self.pool)
        .await?;
        Ok(id)
    }

    #[cfg(test)]
    pub async fn set_discount_scope(
        &self,
        id: i64,
        applicable_chain_ids: &str,
        applicable_ticket_levels: &str,
    ) -> anyhow::Result<()> {
        sqlx::query(
            r#"
            UPDATE promotion_codes
            SET applicable_chain_ids = ?2,
                applicable_ticket_levels = ?3
            WHERE id = ?1
            "#,
        )
        .bind(id)
        .bind(applicable_chain_ids)
        .bind(applicable_ticket_levels)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    #[cfg(test)]
    pub async fn set_discount_max_uses_per_wallet(
        &self,
        id: i64,
        max_uses_per_wallet: i64,
    ) -> anyhow::Result<()> {
        sqlx::query(
            r#"
            UPDATE promotion_codes
            SET max_uses_per_wallet = ?2
            WHERE id = ?1
            "#,
        )
        .bind(id)
        .bind(max_uses_per_wallet)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    #[cfg(test)]
    async fn seed_promotion_code(&self, code: &str, kind: &str) -> anyhow::Result<i64> {
        let now_ts = unix_now();
        let normalized =
            normalize_promotion_code(code).ok_or_else(|| anyhow::anyhow!("code is required"))?;

        let result = sqlx::query(
            r#"
            INSERT INTO promotion_codes (
                code_normalized,
                kind,
                status,
                beneficiary_wallet,
                valid_from,
                valid_until,
                max_total_uses,
                max_uses_per_wallet,
                first_purchase_only,
                stacking_policy,
                applicable_chain_ids,
                applicable_ticket_levels,
                discount_type,
                discount_value,
                max_discount_amount,
                commission_type,
                commission_value,
                notes,
                created_at,
                updated_at
            ) VALUES (?1, ?2, 'active', NULL, NULL, NULL, NULL, NULL, 0, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, ?3, ?3)
            "#,
        )
        .bind(normalized)
        .bind(kind)
        .bind(now_ts)
        .execute(&self.pool)
        .await?;

        Ok(result.last_insert_rowid())
    }

    #[cfg(test)]
    pub async fn count_admin_audit_logs_for_target(
        &self,
        target_type: &str,
        target_id: &str,
    ) -> anyhow::Result<i64> {
        let count = sqlx::query_scalar::<_, i64>(
            r#"
            SELECT COUNT(1)
            FROM admin_audit_logs
            WHERE target_type = ?1
              AND target_id = ?2
            "#,
        )
        .bind(target_type)
        .bind(target_id)
        .fetch_one(&self.pool)
        .await?;

        Ok(count)
    }
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("time should be after unix epoch")
        .as_secs() as i64
}

fn fallback_fiat_unit_price(checkout: &FiatCheckoutSessionRow, quantity: i64) -> i64 {
    if quantity <= 0 {
        return 0;
    }
    checkout.original_amount_cents / quantity
}

async fn calculate_referral_commission_amount(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    referral_code_id: Option<i64>,
    chain_id: u64,
    payment_token: &str,
    commission_base_amount: U256,
) -> anyhow::Result<U256> {
    let Some(referral_code_id) = referral_code_id else {
        return Ok(U256::zero());
    };

    let Some((commission_type, commission_value)) =
        sqlx::query_as::<_, (Option<String>, Option<String>)>(
            r#"
            SELECT commission_type, commission_value
            FROM promotion_codes
            WHERE id = ?1
              AND kind = 'referral'
            "#,
        )
        .bind(referral_code_id)
        .fetch_optional(&mut **tx)
        .await?
    else {
        return Ok(U256::zero());
    };

    let (Some(commission_type), Some(commission_value)) = (commission_type, commission_value)
    else {
        return Ok(U256::zero());
    };

    calculate_commission_amount_from_rule(
        Some(&commission_type),
        Some(&commission_value),
        chain_id,
        payment_token,
        commission_base_amount,
    )
}

fn calculate_commission_amount_from_rule(
    commission_type: Option<&str>,
    commission_value: Option<&str>,
    chain_id: u64,
    payment_token: &str,
    commission_base_amount: U256,
) -> anyhow::Result<U256> {
    let (Some(commission_type), Some(commission_value)) = (commission_type, commission_value)
    else {
        return Ok(U256::zero());
    };

    let mut commission_amount = match commission_type {
        "percentage" => {
            let bps = U256::from_dec_str(commission_value)?;
            commission_base_amount
                .checked_mul(bps)
                .ok_or_else(|| anyhow::anyhow!("commission calculation overflow"))?
                / U256::from(10_000u64)
        }
        "fixed" => parse_human_token_amount(
            commission_value,
            payment_token_decimals(chain_id, payment_token),
        )?,
        _ => U256::zero(),
    };

    if commission_amount > commission_base_amount {
        commission_amount = commission_base_amount;
    }

    Ok(commission_amount)
}

fn parse_human_token_amount(value: &str, decimals: Option<u8>) -> anyhow::Result<U256> {
    let decimals =
        decimals.ok_or_else(|| anyhow::anyhow!("payment token decimals are required"))?;
    let normalized = value.trim();
    if normalized.is_empty() || normalized.starts_with('-') {
        return Err(anyhow::anyhow!("invalid decimal amount"));
    }

    let mut parts = normalized.split('.');
    let whole = parts.next().unwrap_or_default();
    let fraction = parts.next();
    if parts.next().is_some() {
        return Err(anyhow::anyhow!("invalid decimal amount"));
    }
    if !whole.chars().all(|ch| ch.is_ascii_digit()) {
        return Err(anyhow::anyhow!("invalid decimal amount"));
    }

    let whole_amount = U256::from_dec_str(if whole.is_empty() { "0" } else { whole })?;
    let base = U256::from(10u64).pow(U256::from(decimals));
    let mut amount = whole_amount
        .checked_mul(base)
        .ok_or_else(|| anyhow::anyhow!("token amount overflow"))?;

    if let Some(fraction) = fraction {
        if fraction.len() > decimals as usize || !fraction.chars().all(|ch| ch.is_ascii_digit()) {
            return Err(anyhow::anyhow!("invalid decimal amount"));
        }
        let padded_fraction = format!("{fraction:0<width$}", width = decimals as usize);
        if !padded_fraction.is_empty() {
            amount += U256::from_dec_str(&padded_fraction)?;
        }
    }

    Ok(amount)
}

fn email_access_token_hash(token: &str) -> String {
    format!("{:#x}", H256::from(keccak256(token.as_bytes())))
}

#[cfg(test)]
mod tests {
    use super::Db;
    use crate::chain::DecodedPurchase;

    #[tokio::test]
    async fn purge_signin_challenges_removes_only_old_rows() {
        let db = Db::connect("sqlite::memory:")
            .await
            .expect("db should initialize");
        let now = super::unix_now();
        let cutoff = now - 60;

        sqlx::query(
            r#"
            INSERT INTO signin_challenges (id, wallet, challenge_message, nonce, expires_at, used_at, created_at)
            VALUES
                ('expired-old', '0x1', 'm1', 'n1', ?1, NULL, ?2),
                ('used-old', '0x2', 'm2', 'n2', ?3, ?4, ?2),
                ('active', '0x3', 'm3', 'n3', ?3, NULL, ?2),
                ('used-recent', '0x4', 'm4', 'n4', ?3, ?5, ?2)
            "#,
        )
        .bind(cutoff - 10)
        .bind(now)
        .bind(now + 3600)
        .bind(cutoff - 5)
        .bind(cutoff + 5)
        .execute(&db.pool)
        .await
        .expect("seed data should succeed");

        let deleted = db
            .purge_signin_challenges(cutoff)
            .await
            .expect("purge should succeed");
        assert_eq!(deleted, 2);

        let remaining: Vec<String> =
            sqlx::query_scalar("SELECT id FROM signin_challenges ORDER BY id ASC")
                .fetch_all(&db.pool)
                .await
                .expect("query should succeed");

        assert_eq!(
            remaining,
            vec!["active".to_string(), "used-recent".to_string()]
        );
    }

    #[tokio::test]
    async fn create_signin_challenge_uses_human_readable_message() {
        let db = Db::connect("sqlite::memory:")
            .await
            .expect("db should initialize");

        let challenge = db
            .create_signin_challenge("0x1111111111111111111111111111111111111111", 300)
            .await
            .expect("challenge should be created");

        assert!(challenge.challenge_message.contains("Sign-In"));
        assert!(challenge
            .challenge_message
            .contains("Sign in to the ticketing service."));
        assert!(challenge
            .challenge_message
            .contains("does not create a blockchain transaction"));
        assert!(challenge.challenge_message.contains("does not cost gas"));
        assert!(challenge
            .challenge_message
            .contains("Wallet: 0x1111111111111111111111111111111111111111"));
        assert!(challenge.challenge_message.contains("Nonce: "));
        assert!(challenge.challenge_message.contains("IssuedAt: "));
        assert!(challenge.challenge_message.contains("ExpiresAt: "));
    }

    #[tokio::test]
    async fn index_and_transfer_ticket_use_money_frontier_qr_payload_prefix() {
        let db = Db::connect("sqlite::memory:")
            .await
            .expect("db should initialize");
        let buyer = "0x1111111111111111111111111111111111111111";

        db.index_purchase(
            56,
            &DecodedPurchase {
                tx_hash: "0xaaa".to_string(),
                log_index: 0,
                block_number: 100,
                block_hash: Some("0xblock".to_string()),
                order_id: "1".to_string(),
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

        let tickets = db
            .list_active_tickets_by_wallet(buyer)
            .await
            .expect("tickets should load");
        assert_eq!(tickets.len(), 1);
        assert!(tickets[0].qr_payload.starts_with("money-frontier:qr:"));

        let transferred = db
            .transfer_ticket(
                &tickets[0].id,
                buyer,
                Some("0x3333333333333333333333333333333333333333"),
                None,
            )
            .await
            .expect("transfer should succeed")
            .expect("ticket should transfer");
        assert!(transferred.qr_payload.starts_with("money-frontier:qr:"));
    }

    #[tokio::test]
    async fn email_access_challenge_is_one_time_and_lists_email_tickets() {
        let db = Db::connect("sqlite::memory:")
            .await
            .expect("db should initialize");
        let buyer = "0x1111111111111111111111111111111111111111";
        let email = "guest@example.com";

        db.index_purchase(
            56,
            &DecodedPurchase {
                tx_hash: "0xbbbb".to_string(),
                log_index: 0,
                block_number: 101,
                block_hash: Some("0xblock".to_string()),
                order_id: "2".to_string(),
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

        let wallet_tickets = db
            .list_active_tickets_by_wallet(buyer)
            .await
            .expect("wallet tickets should load");

        db.transfer_ticket(&wallet_tickets[0].id, buyer, None, Some(email))
            .await
            .expect("ticket should transfer to email")
            .expect("ticket should exist");

        let email_tickets = db
            .list_active_tickets_by_email(email)
            .await
            .expect("email tickets should load");
        assert_eq!(email_tickets.len(), 1);
        assert_eq!(email_tickets[0].owner_email.as_deref(), Some(email));

        let challenge = db
            .create_email_access_challenge(email, 900)
            .await
            .expect("email access challenge should be created");
        assert_ne!(challenge.token, challenge.token_hash);
        assert_eq!(challenge.email, email);

        let consumed = db
            .consume_email_access_challenge(&challenge.token)
            .await
            .expect("email access challenge should consume")
            .expect("email access challenge should exist");
        assert_eq!(consumed.email, email);

        let consumed_again = db
            .consume_email_access_challenge(&challenge.token)
            .await
            .expect("consuming twice should not error");
        assert!(consumed_again.is_none());
    }

    #[tokio::test]
    async fn connect_applies_promotions_migration() {
        let db = Db::connect("sqlite::memory:")
            .await
            .expect("db should initialize");
        let columns: Vec<String> = sqlx::query_scalar(
            "SELECT name FROM pragma_table_info('purchase_intents') ORDER BY name ASC",
        )
        .fetch_all(&db.pool)
        .await
        .expect("schema query should succeed");

        assert!(columns.contains(&"wallet_address".to_string()));
    }

    #[tokio::test]
    async fn admin_challenge_lifecycle_uses_admin_message_and_one_time_consumption() {
        let db = Db::connect("sqlite::memory:")
            .await
            .expect("db should initialize");
        let wallet = "0x1111111111111111111111111111111111111111";

        let challenge = db
            .create_admin_signin_challenge(wallet, 300)
            .await
            .expect("admin challenge should be created");

        assert!(challenge.challenge_message.starts_with("Admin Sign-In"));
        assert!(challenge
            .challenge_message
            .contains("Wallet: 0x1111111111111111111111111111111111111111"));

        let message = db
            .get_admin_signin_challenge_message(&challenge.id, wallet)
            .await
            .expect("admin challenge lookup should succeed");
        assert_eq!(message, Some(challenge.challenge_message));

        assert!(db
            .mark_admin_signin_challenge_used(&challenge.id, wallet)
            .await
            .expect("admin challenge consume should succeed"));
        assert!(!db
            .mark_admin_signin_challenge_used(&challenge.id, wallet)
            .await
            .expect("admin challenge cannot be consumed twice"));
    }

    #[tokio::test]
    async fn admin_challenge_purge_removes_expired_and_used_rows() {
        let db = Db::connect("sqlite::memory:")
            .await
            .expect("db should initialize");
        let now = super::unix_now();
        let cutoff = now - 60;

        sqlx::query(
            r#"
            INSERT INTO admin_signin_challenges (id, wallet, challenge_message, nonce, expires_at, used_at, created_at)
            VALUES
                ('expired-old', '0x1', 'm1', 'n1', ?1, NULL, ?2),
                ('used-old', '0x2', 'm2', 'n2', ?3, ?4, ?2),
                ('active', '0x3', 'm3', 'n3', ?3, NULL, ?2),
                ('used-recent', '0x4', 'm4', 'n4', ?3, ?5, ?2)
            "#,
        )
        .bind(cutoff - 10)
        .bind(now)
        .bind(now + 3600)
        .bind(cutoff - 5)
        .bind(cutoff + 5)
        .execute(&db.pool)
        .await
        .expect("admin challenge seed data should succeed");

        let deleted = db
            .purge_admin_signin_challenges(cutoff)
            .await
            .expect("admin challenge purge should succeed");
        assert_eq!(deleted, 2);

        let remaining: Vec<String> =
            sqlx::query_scalar("SELECT id FROM admin_signin_challenges ORDER BY id ASC")
                .fetch_all(&db.pool)
                .await
                .expect("admin challenge query should succeed");

        assert_eq!(
            remaining,
            vec!["active".to_string(), "used-recent".to_string()]
        );
    }

    #[tokio::test]
    async fn admin_audit_insert_persists_actor_target_and_payloads() {
        let db = Db::connect("sqlite::memory:")
            .await
            .expect("db should initialize");

        let id = db
            .insert_admin_audit_log(super::NewAdminAuditLog {
                actor_wallet: "0x1111111111111111111111111111111111111111".to_string(),
                actor_role: "operator".to_string(),
                action: "invite_code.create".to_string(),
                target_type: "invite_code".to_string(),
                target_id: Some("42".to_string()),
                before_json: None,
                after_json: Some(r#"{"code":"INVITE"}"#.to_string()),
                ip_address: Some("127.0.0.1".to_string()),
                user_agent: Some("test-agent".to_string()),
            })
            .await
            .expect("admin audit log should insert");

        let row: (String, String, String, String, Option<String>) = sqlx::query_as(
            r#"
            SELECT actor_wallet, actor_role, action, target_type, after_json
            FROM admin_audit_logs
            WHERE id = ?1
            "#,
        )
        .bind(id)
        .fetch_one(&db.pool)
        .await
        .expect("admin audit row should exist");

        assert_eq!(row.0, "0x1111111111111111111111111111111111111111");
        assert_eq!(row.1, "operator");
        assert_eq!(row.2, "invite_code.create");
        assert_eq!(row.3, "invite_code");
        assert_eq!(row.4, Some(r#"{"code":"INVITE"}"#.to_string()));
    }

    #[tokio::test]
    async fn bind_referral_first_time_only() {
        let db = Db::connect("sqlite::memory:")
            .await
            .expect("db should initialize");
        let code_id = db
            .seed_referral_code("alice")
            .await
            .expect("seed should succeed");

        let first = db
            .bind_wallet_referral_once("0xabc", code_id, "signin")
            .await
            .expect("first bind should succeed");
        let second = db
            .bind_wallet_referral_once("0xabc", code_id, "purchase_intent")
            .await
            .expect("second bind should not error");

        assert!(first.bound);
        assert!(!second.bound);
    }
}
