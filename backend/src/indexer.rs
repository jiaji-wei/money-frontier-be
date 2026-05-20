use std::{cmp::min, sync::Arc, time::Duration};

use tokio::time::sleep;

use crate::{chain::ChainRuntimeConfig, AppState};

pub fn spawn(state: Arc<AppState>) {
    tokio::spawn(async move {
        run(state).await;
    });
}

async fn run(state: Arc<AppState>) {
    let interval = Duration::from_secs(state.config.indexer_poll_interval_secs.max(1));
    loop {
        if let Err(err) = sync_all_chains(&state).await {
            tracing::error!(error = %err, "indexer sync iteration failed");
        }
        sleep(interval).await;
    }
}

pub async fn sync_all_chains(state: &Arc<AppState>) -> anyhow::Result<()> {
    let chain_configs = state.chain.runtime_configs();
    for cfg in &chain_configs {
        if let Err(err) = sync_chain(state, cfg).await {
            tracing::error!(chain_id = cfg.chain_id, error = %err, "indexer sync failed for chain");
        }
    }
    Ok(())
}

async fn sync_chain(state: &Arc<AppState>, cfg: &ChainRuntimeConfig) -> anyhow::Result<()> {
    let finalized = state.chain.latest_finalized_block(cfg.chain_id).await?;
    let batch_size = state.config.indexer_batch_size.max(1);

    let mut last_indexed = match state.db.get_indexer_cursor(cfg.chain_id).await? {
        Some(cursor) => {
            let expected_hash = cursor.last_indexed_block_hash;
            if cursor.last_indexed_block > 0 && expected_hash.is_some() {
                let chain_hash = state
                    .chain
                    .block_hash(cfg.chain_id, cursor.last_indexed_block)
                    .await?;
                if chain_hash != expected_hash {
                    let rollback_window = state.config.indexer_reorg_rollback_blocks.max(1);
                    let start_floor = cfg.start_block.unwrap_or(0);
                    let rollback_from = cursor
                        .last_indexed_block
                        .saturating_sub(rollback_window - 1)
                        .max(start_floor);

                    let rollback_result = state
                        .db
                        .rollback_chain_from_block(cfg.chain_id, rollback_from)
                        .await?;

                    let reset_cursor = rollback_from.saturating_sub(1);
                    let reset_hash = if reset_cursor == 0 {
                        None
                    } else {
                        state.chain.block_hash(cfg.chain_id, reset_cursor).await?
                    };
                    state
                        .db
                        .set_indexer_cursor(cfg.chain_id, reset_cursor, reset_hash.as_deref())
                        .await?;

                    tracing::warn!(
                        chain_id = cfg.chain_id,
                        reorg_at = cursor.last_indexed_block,
                        rollback_from,
                        deleted_orders = rollback_result.deleted_orders,
                        deleted_tickets = rollback_result.deleted_tickets,
                        "reorg detected and rollback applied"
                    );

                    reset_cursor
                } else {
                    cursor.last_indexed_block
                }
            } else {
                cursor.last_indexed_block
            }
        }
        None => cfg
            .start_block
            .map(|value| value.saturating_sub(1))
            .unwrap_or_else(|| finalized.saturating_sub(1)),
    };

    while last_indexed < finalized {
        let from_block = last_indexed + 1;
        let to_block = min(from_block + batch_size - 1, finalized);

        let purchases = state
            .chain
            .fetch_purchases_by_block_range(cfg.chain_id, from_block, to_block)
            .await?;

        for purchase in &purchases {
            state.db.index_purchase(cfg.chain_id, purchase).await?;
        }

        let to_block_hash = if to_block == 0 {
            None
        } else {
            state.chain.block_hash(cfg.chain_id, to_block).await?
        };
        state
            .db
            .set_indexer_cursor(cfg.chain_id, to_block, to_block_hash.as_deref())
            .await?;

        tracing::info!(
            chain_id = cfg.chain_id,
            from_block,
            to_block,
            purchase_count = purchases.len(),
            "indexer synced block range"
        );

        last_indexed = to_block;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashMap,
        sync::{Arc, Mutex},
    };

    use async_trait::async_trait;

    use super::sync_all_chains;
    use crate::{
        auth::JwtCodec,
        chain::{ChainReader, ChainRuntimeConfig, DecodedPurchase, QuoteResult},
        config::AppConfig,
        db::Db,
        mailer::Mailer,
        AppState,
    };

    #[derive(Default)]
    struct MockChainState {
        runtime_configs: Vec<ChainRuntimeConfig>,
        finalized_blocks: HashMap<u64, u64>,
        block_hashes: HashMap<(u64, u64), Option<String>>,
        range_events: HashMap<(u64, u64, u64), Vec<DecodedPurchase>>,
    }

    #[derive(Default)]
    struct MockChain {
        state: Mutex<MockChainState>,
    }

    impl MockChain {
        fn set_runtime_configs(&self, configs: Vec<ChainRuntimeConfig>) {
            let mut guard = self.state.lock().expect("lock should succeed");
            guard.runtime_configs = configs;
        }

        fn set_finalized_block(&self, chain_id: u64, block_number: u64) {
            let mut guard = self.state.lock().expect("lock should succeed");
            guard.finalized_blocks.insert(chain_id, block_number);
        }

        fn set_block_hash(&self, chain_id: u64, block_number: u64, block_hash: Option<&str>) {
            let mut guard = self.state.lock().expect("lock should succeed");
            guard.block_hashes.insert(
                (chain_id, block_number),
                block_hash.map(|value| value.to_string()),
            );
        }

        fn set_range_events(
            &self,
            chain_id: u64,
            from_block: u64,
            to_block: u64,
            events: Vec<DecodedPurchase>,
        ) {
            let mut guard = self.state.lock().expect("lock should succeed");
            guard
                .range_events
                .insert((chain_id, from_block, to_block), events);
        }
    }

    #[async_trait]
    impl ChainReader for MockChain {
        fn runtime_configs(&self) -> Vec<ChainRuntimeConfig> {
            let guard = self.state.lock().expect("lock should succeed");
            guard.runtime_configs.clone()
        }

        async fn latest_finalized_block(&self, chain_id: u64) -> anyhow::Result<u64> {
            let guard = self.state.lock().expect("lock should succeed");
            Ok(*guard.finalized_blocks.get(&chain_id).unwrap_or(&0))
        }

        async fn block_hash(
            &self,
            chain_id: u64,
            block_number: u64,
        ) -> anyhow::Result<Option<String>> {
            let guard = self.state.lock().expect("lock should succeed");
            Ok(guard
                .block_hashes
                .get(&(chain_id, block_number))
                .cloned()
                .unwrap_or(None))
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
            chain_id: u64,
            from_block: u64,
            to_block: u64,
        ) -> anyhow::Result<Vec<DecodedPurchase>> {
            let guard = self.state.lock().expect("lock should succeed");
            Ok(guard
                .range_events
                .get(&(chain_id, from_block, to_block))
                .cloned()
                .unwrap_or_default())
        }

        async fn quote_purchase(
            &self,
            _chain_id: u64,
            _level_ids: &[u8],
            _quantities: &[u64],
        ) -> anyhow::Result<QuoteResult> {
            anyhow::bail!("quote purchase is not used by indexer tests")
        }

        async fn has_default_admin_role(&self, _wallet: &str) -> anyhow::Result<bool> {
            Ok(false)
        }
    }

    async fn build_state(mock_chain: Arc<MockChain>) -> Arc<AppState> {
        let database_url = "sqlite::memory:".to_string();
        let db = Db::connect(&database_url).await.expect("db should init");

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
            indexer_poll_interval_secs: 1,
            indexer_batch_size: 50,
            indexer_reorg_rollback_blocks: 16,
            signin_challenge_ttl_secs: 300,
            signin_cleanup_interval_secs: 600,
            signin_cleanup_retention_secs: 86400,
            purchase_intent_ttl_secs: 900,
            purchase_signer_private_key: None,
            admin_jwt_ttl_hours: 12,
        };

        let jwt =
            JwtCodec::new(&config.jwt_secret, config.jwt_ttl_days).expect("jwt codec should init");
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
        .expect("mailer should init");

        Arc::new(AppState {
            config,
            db,
            chain: mock_chain as Arc<dyn ChainReader>,
            jwt,
            mailer,
            purchase_signer: None,
        })
    }

    #[tokio::test]
    async fn sync_respects_finalized_progress() {
        let mock_chain = Arc::new(MockChain::default());
        mock_chain.set_runtime_configs(vec![ChainRuntimeConfig {
            chain_id: 1,
            start_block: Some(10),
            confirmations: 0,
        }]);
        mock_chain.set_finalized_block(1, 9);
        mock_chain.set_block_hash(1, 10, Some("0xblock10"));
        mock_chain.set_range_events(
            1,
            10,
            10,
            vec![DecodedPurchase {
                tx_hash: "0xtx-a".to_string(),
                log_index: 0,
                block_number: 10,
                block_hash: Some("0xblock10".to_string()),
                order_id: "order-a".to_string(),
                buyer: "0x000000000000000000000000000000000000beef".to_string(),
                payment_token: "0x0000000000000000000000000000000000001002".to_string(),
                total_amount: "100".to_string(),
                level_ids: vec![1],
                quantities: vec![1],
                unit_prices: vec!["100".to_string()],
                intent_id: None,
            }],
        );

        let state = build_state(mock_chain.clone()).await;
        sync_all_chains(&state)
            .await
            .expect("first sync should pass");
        assert_eq!(
            state.db.count_orders(1).await.expect("count should pass"),
            0
        );

        mock_chain.set_finalized_block(1, 10);
        sync_all_chains(&state)
            .await
            .expect("second sync should pass");
        assert_eq!(
            state.db.count_orders(1).await.expect("count should pass"),
            1
        );
    }

    #[tokio::test]
    async fn sync_rolls_back_on_reorg_and_reindexes() {
        let mock_chain = Arc::new(MockChain::default());
        mock_chain.set_runtime_configs(vec![ChainRuntimeConfig {
            chain_id: 1,
            start_block: Some(10),
            confirmations: 0,
        }]);
        mock_chain.set_finalized_block(1, 10);
        mock_chain.set_block_hash(1, 10, Some("0xblock-a"));
        mock_chain.set_range_events(
            1,
            10,
            10,
            vec![DecodedPurchase {
                tx_hash: "0xtx-a".to_string(),
                log_index: 0,
                block_number: 10,
                block_hash: Some("0xblock-a".to_string()),
                order_id: "order-a".to_string(),
                buyer: "0x000000000000000000000000000000000000beef".to_string(),
                payment_token: "0x0000000000000000000000000000000000001002".to_string(),
                total_amount: "100".to_string(),
                level_ids: vec![1],
                quantities: vec![1],
                unit_prices: vec!["100".to_string()],
                intent_id: None,
            }],
        );

        let state = build_state(mock_chain.clone()).await;
        sync_all_chains(&state)
            .await
            .expect("initial sync should pass");
        assert_eq!(
            state.db.list_order_ids(1).await.expect("query should pass"),
            vec!["order-a".to_string()]
        );

        mock_chain.set_block_hash(1, 10, Some("0xblock-b"));
        mock_chain.set_range_events(
            1,
            10,
            10,
            vec![DecodedPurchase {
                tx_hash: "0xtx-b".to_string(),
                log_index: 0,
                block_number: 10,
                block_hash: Some("0xblock-b".to_string()),
                order_id: "order-b".to_string(),
                buyer: "0x000000000000000000000000000000000000beef".to_string(),
                payment_token: "0x0000000000000000000000000000000000001002".to_string(),
                total_amount: "120".to_string(),
                level_ids: vec![1],
                quantities: vec![1],
                unit_prices: vec!["120".to_string()],
                intent_id: None,
            }],
        );

        sync_all_chains(&state)
            .await
            .expect("reorg sync should pass");
        assert_eq!(
            state.db.list_order_ids(1).await.expect("query should pass"),
            vec!["order-b".to_string()]
        );
        assert_eq!(
            state.db.count_orders(1).await.expect("count should pass"),
            1
        );
        assert_eq!(
            state.db.count_tickets(1).await.expect("count should pass"),
            1
        );
    }
}
