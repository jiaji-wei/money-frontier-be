use sqlx::{sqlite::SqlitePoolOptions, FromRow, SqlitePool};
use uuid::Uuid;

use crate::chain::DecodedPurchase;

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

        if existing_tickets > 0 {
            tx.commit().await?;
            return Ok(NotifyResult {
                created_order,
                created_tickets: 0,
            });
        }

        let mut created_tickets = 0usize;
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

    #[cfg(test)]
    pub async fn count_orders(&self, chain_id: u64) -> anyhow::Result<i64> {
        let count = sqlx::query_scalar::<_, i64>("SELECT COUNT(1) FROM orders WHERE chain_id = ?1")
            .bind(chain_id as i64)
            .fetch_one(&self.pool)
            .await?;
        Ok(count)
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
            .create_signin_challenge(
                "0x1111111111111111111111111111111111111111",
                300,
            )
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
}
