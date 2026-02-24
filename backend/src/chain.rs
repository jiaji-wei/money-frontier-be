use std::{collections::HashMap, str::FromStr, sync::Arc};

use anyhow::Context;
use async_trait::async_trait;
use ethers::{
    abi::RawLog,
    contract::EthEvent,
    providers::{Http, Middleware, Provider},
    types::{Address, BlockNumber, Filter, Log, H256, U256},
};

use crate::config::ChainConfig;

#[derive(Debug, Clone)]
pub struct DecodedPurchase {
    pub tx_hash: String,
    pub log_index: u64,
    pub block_number: u64,
    pub block_hash: Option<String>,
    pub order_id: String,
    pub buyer: String,
    pub payment_token: String,
    pub total_amount: String,
    pub level_ids: Vec<u8>,
    pub quantities: Vec<u64>,
    pub unit_prices: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ChainRuntimeConfig {
    pub chain_id: u64,
    pub start_block: Option<u64>,
    pub confirmations: u64,
}

#[async_trait]
pub trait ChainReader: Send + Sync {
    fn runtime_configs(&self) -> Vec<ChainRuntimeConfig>;
    async fn latest_finalized_block(&self, chain_id: u64) -> anyhow::Result<u64>;
    async fn block_hash(&self, chain_id: u64, block_number: u64) -> anyhow::Result<Option<String>>;
    async fn fetch_purchases(
        &self,
        chain_id: u64,
        tx_hash: &str,
    ) -> anyhow::Result<Vec<DecodedPurchase>>;
    async fn fetch_purchases_by_block_range(
        &self,
        chain_id: u64,
        from_block: u64,
        to_block: u64,
    ) -> anyhow::Result<Vec<DecodedPurchase>>;
}

#[derive(Debug, Clone)]
struct ChainClient {
    sale_contract: Address,
    provider: Arc<Provider<Http>>,
    start_block: Option<u64>,
    confirmations: u64,
}

#[derive(Clone)]
pub struct ChainService {
    clients: HashMap<u64, ChainClient>,
}

#[derive(Clone, Debug, EthEvent)]
#[ethevent(
    name = "TicketsPurchased",
    abi = "TicketsPurchased(uint256,address,address,uint256,uint8[],uint256[],uint256[],uint256)"
)]
struct TicketsPurchasedLog {
    #[ethevent(indexed)]
    pub order_id: U256,
    #[ethevent(indexed)]
    pub buyer: Address,
    #[ethevent(indexed)]
    pub payment_token: Address,
    pub total_amount: U256,
    pub level_ids: Vec<u8>,
    pub quantities: Vec<U256>,
    pub unit_prices: Vec<U256>,
    pub purchased_at: U256,
}

impl ChainService {
    pub fn new(configs: &[ChainConfig]) -> anyhow::Result<Self> {
        let mut clients = HashMap::new();
        for cfg in configs {
            let provider = Provider::try_from(cfg.rpc_url.as_str())
                .with_context(|| format!("invalid rpc url for chain {}", cfg.chain_id))?;
            let sale_contract = Address::from_str(cfg.sale_contract.as_str())
                .with_context(|| format!("invalid sale contract for chain {}", cfg.chain_id))?;

            clients.insert(
                cfg.chain_id,
                ChainClient {
                    sale_contract,
                    provider: Arc::new(provider),
                    start_block: cfg.start_block,
                    confirmations: cfg.confirmations,
                },
            );
        }

        Ok(Self { clients })
    }

    fn client(&self, chain_id: u64) -> anyhow::Result<&ChainClient> {
        self.clients
            .get(&chain_id)
            .with_context(|| format!("unsupported chain_id: {chain_id}"))
    }

    fn decode_purchase_log(
        &self,
        client: &ChainClient,
        log: &Log,
    ) -> anyhow::Result<Option<DecodedPurchase>> {
        if log.address != client.sale_contract {
            return Ok(None);
        }
        if log.topics.first() != Some(&TicketsPurchasedLog::signature()) {
            return Ok(None);
        }

        let decoded = <TicketsPurchasedLog as EthEvent>::decode_log(&RawLog {
            topics: log.topics.clone(),
            data: log.data.to_vec(),
        })
        .context("failed to decode TicketsPurchased log")?;

        if decoded.level_ids.len() != decoded.quantities.len()
            || decoded.level_ids.len() != decoded.unit_prices.len()
        {
            anyhow::bail!("event line-item lengths mismatch");
        }

        let mut quantities = Vec::with_capacity(decoded.quantities.len());
        for quantity in &decoded.quantities {
            quantities.push(u256_to_u64(*quantity).context("quantity exceeds u64 range")?);
        }

        let unit_prices = decoded
            .unit_prices
            .iter()
            .map(|v: &U256| v.to_string())
            .collect::<Vec<_>>();

        let tx_hash = log
            .transaction_hash
            .map(|v| format!("{v:#x}"))
            .unwrap_or_else(|| format!("{:#x}", H256::zero()));

        Ok(Some(DecodedPurchase {
            tx_hash,
            log_index: log.log_index.unwrap_or_default().as_u64(),
            block_number: log.block_number.unwrap_or_default().as_u64(),
            block_hash: log.block_hash.map(|v| format!("{v:#x}")),
            order_id: decoded.order_id.to_string(),
            buyer: format!("{:#x}", decoded.buyer),
            payment_token: format!("{:#x}", decoded.payment_token),
            total_amount: decoded.total_amount.to_string(),
            level_ids: decoded.level_ids,
            quantities,
            unit_prices,
        }))
    }
}

#[async_trait]
impl ChainReader for ChainService {
    fn runtime_configs(&self) -> Vec<ChainRuntimeConfig> {
        let mut values = self
            .clients
            .iter()
            .map(|(chain_id, client)| ChainRuntimeConfig {
                chain_id: *chain_id,
                start_block: client.start_block,
                confirmations: client.confirmations,
            })
            .collect::<Vec<_>>();

        values.sort_by_key(|cfg| cfg.chain_id);
        values
    }

    async fn latest_finalized_block(&self, chain_id: u64) -> anyhow::Result<u64> {
        let client = self.client(chain_id)?;
        let latest = client.provider.get_block_number().await?.as_u64();
        if latest < client.confirmations {
            return Ok(0);
        }
        Ok(latest - client.confirmations)
    }

    async fn block_hash(&self, chain_id: u64, block_number: u64) -> anyhow::Result<Option<String>> {
        let client = self.client(chain_id)?;
        let block = client.provider.get_block(block_number).await?;
        Ok(block.and_then(|b| b.hash.map(|v| format!("{v:#x}"))))
    }

    async fn fetch_purchases(
        &self,
        chain_id: u64,
        tx_hash: &str,
    ) -> anyhow::Result<Vec<DecodedPurchase>> {
        let client = self.client(chain_id)?;

        let tx_hash = H256::from_str(tx_hash).context("invalid tx hash format")?;
        let receipt = client
            .provider
            .get_transaction_receipt(tx_hash)
            .await?
            .context("transaction receipt not found")?;

        let mut purchases = Vec::new();
        for log in receipt.logs {
            if let Some(mut decoded) = self.decode_purchase_log(client, &log)? {
                if decoded.tx_hash == format!("{:#x}", H256::zero()) {
                    decoded.tx_hash = format!("{tx_hash:#x}");
                }
                purchases.push(decoded);
            }
        }

        purchases.sort_by_key(|item| (item.block_number, item.log_index));
        Ok(purchases)
    }

    async fn fetch_purchases_by_block_range(
        &self,
        chain_id: u64,
        from_block: u64,
        to_block: u64,
    ) -> anyhow::Result<Vec<DecodedPurchase>> {
        let client = self.client(chain_id)?;
        if from_block > to_block {
            return Ok(Vec::new());
        }

        let filter = Filter::new()
            .address(client.sale_contract)
            .from_block(BlockNumber::Number(from_block.into()))
            .to_block(BlockNumber::Number(to_block.into()))
            .topic0(TicketsPurchasedLog::signature());

        let logs = client.provider.get_logs(&filter).await?;
        let mut purchases = Vec::with_capacity(logs.len());
        for log in logs {
            if let Some(decoded) = self.decode_purchase_log(client, &log)? {
                purchases.push(decoded);
            }
        }

        purchases.sort_by_key(|item| (item.block_number, item.log_index));
        Ok(purchases)
    }
}

fn u256_to_u64(value: U256) -> anyhow::Result<u64> {
    if value > U256::from(u64::MAX) {
        anyhow::bail!("value out of u64 range")
    }
    Ok(value.as_u64())
}
