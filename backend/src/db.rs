use sqlx::{sqlite::SqlitePoolOptions, FromRow, SqlitePool};
use uuid::Uuid;

use crate::chain::DecodedPurchase;
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
                    let qr_payload = format!("lili:qr:{}:1:{}", ticket_id, Uuid::new_v4().simple());

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
                    .bind("0")
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
        let new_qr_payload = format!("lili:qr:{}:1:{}", new_ticket_id, Uuid::new_v4().simple());

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
                created_at,
                updated_at
            ) VALUES (?1, ?2, 'active', NULL, NULL, NULL, NULL, NULL, 0, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, ?3, ?3)
            "#,
        )
        .bind(normalized)
        .bind(kind)
        .bind(now_ts)
        .execute(&self.pool)
        .await?;

        Ok(result.last_insert_rowid())
    }
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("time should be after unix epoch")
        .as_secs() as i64
}

#[cfg(test)]
mod tests {
    use super::Db;

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
