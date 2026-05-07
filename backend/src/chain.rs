use std::{collections::HashMap, str::FromStr, sync::Arc};

use anyhow::Context;
use async_trait::async_trait;
use ethers_core::{
    abi::{Function, Param, ParamType, RawLog, StateMutability, Token},
    types::{Address, BlockNumber, Bytes, Filter, Log, TransactionRequest, H256, U256},
};
use ethers_providers::{Http, Middleware, Provider};

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
    pub intent_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ChainRuntimeConfig {
    pub chain_id: u64,
    pub start_block: Option<u64>,
    pub confirmations: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuoteResult {
    pub total_amount: String,
    pub unit_prices: Vec<String>,
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
    async fn quote_purchase(
        &self,
        chain_id: u64,
        level_ids: &[u8],
        quantities: &[u64],
    ) -> anyhow::Result<QuoteResult>;
    async fn has_default_admin_role(&self, wallet: &str) -> anyhow::Result<bool>;
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

#[derive(Clone, Debug, ethers_contract_derive::EthEvent)]
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

#[derive(Clone, Debug, ethers_contract_derive::EthEvent)]
#[ethevent(
    name = "TicketsPurchasedWithIntent",
    abi = "TicketsPurchasedWithIntent(uint256,bytes32,address,address,uint256,uint8[],uint256[],uint256[],uint256)"
)]
struct TicketsPurchasedWithIntentLog {
    #[ethevent(indexed)]
    pub order_id: U256,
    #[ethevent(indexed)]
    pub intent_id: H256,
    #[ethevent(indexed)]
    pub buyer: Address,
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
        let tx_hash = log
            .transaction_hash
            .map(|v| format!("{v:#x}"))
            .unwrap_or_else(|| format!("{:#x}", H256::zero()));
        let log_index = log.log_index.unwrap_or_default().as_u64();
        let block_number = log.block_number.unwrap_or_default().as_u64();
        let block_hash = log.block_hash.map(|v| format!("{v:#x}"));
        let raw_log = RawLog {
            topics: log.topics.clone(),
            data: log.data.to_vec(),
        };

        if log.topics.first()
            == Some(&<TicketsPurchasedLog as ethers_contract::EthEvent>::signature())
        {
            let decoded = <TicketsPurchasedLog as ethers_contract::EthEvent>::decode_log(&raw_log)
                .context("failed to decode TicketsPurchased log")?;
            return build_decoded_purchase(
                tx_hash,
                log_index,
                block_number,
                block_hash,
                decoded.order_id,
                decoded.buyer,
                decoded.payment_token,
                decoded.total_amount,
                decoded.level_ids,
                decoded.quantities,
                decoded.unit_prices,
                None,
            );
        }

        if log.topics.first()
            == Some(&<TicketsPurchasedWithIntentLog as ethers_contract::EthEvent>::signature())
        {
            let decoded =
                <TicketsPurchasedWithIntentLog as ethers_contract::EthEvent>::decode_log(&raw_log)
                    .context("failed to decode TicketsPurchasedWithIntent log")?;
            return build_decoded_purchase(
                tx_hash,
                log_index,
                block_number,
                block_hash,
                decoded.order_id,
                decoded.buyer,
                decoded.payment_token,
                decoded.total_amount,
                decoded.level_ids,
                decoded.quantities,
                decoded.unit_prices,
                Some(format!("{:#x}", decoded.intent_id)),
            );
        }

        Ok(None)
    }

    async fn quote_purchase_via_rpc(
        &self,
        chain_id: u64,
        level_ids: &[u8],
        quantities: &[u64],
    ) -> anyhow::Result<QuoteResult> {
        let client = self.client(chain_id)?;
        #[allow(deprecated)]
        let function = Function {
            name: "quote".to_string(),
            inputs: vec![
                Param {
                    name: "level_ids".to_string(),
                    kind: ParamType::Array(Box::new(ParamType::Uint(8))),
                    internal_type: None,
                },
                Param {
                    name: "quantities".to_string(),
                    kind: ParamType::Array(Box::new(ParamType::Uint(256))),
                    internal_type: None,
                },
            ],
            outputs: vec![
                Param {
                    name: "total_amount".to_string(),
                    kind: ParamType::Uint(256),
                    internal_type: None,
                },
                Param {
                    name: "unit_prices".to_string(),
                    kind: ParamType::Array(Box::new(ParamType::Uint(256))),
                    internal_type: None,
                },
            ],
            constant: None,
            state_mutability: StateMutability::View,
        };

        let calldata = function.encode_input(&[
            Token::Array(
                level_ids
                    .iter()
                    .map(|value| Token::Uint(U256::from(*value)))
                    .collect(),
            ),
            Token::Array(
                quantities
                    .iter()
                    .map(|value| Token::Uint(U256::from(*value)))
                    .collect(),
            ),
        ])?;

        let tx = TransactionRequest::new()
            .to(client.sale_contract)
            .data(Bytes::from(calldata));
        let raw = client.provider.call(&tx.into(), None).await?;
        let decoded = function.decode_output(raw.as_ref())?;

        let total_amount = match &decoded[0] {
            Token::Uint(value) => value.to_string(),
            _ => anyhow::bail!("quote total_amount has unexpected type"),
        };
        let unit_prices = match &decoded[1] {
            Token::Array(values) => values
                .iter()
                .map(|value| match value {
                    Token::Uint(amount) => Ok(amount.to_string()),
                    _ => anyhow::bail!("quote unit_price has unexpected type"),
                })
                .collect::<anyhow::Result<Vec<_>>>()?,
            _ => anyhow::bail!("quote unit_prices has unexpected type"),
        };

        Ok(QuoteResult {
            total_amount,
            unit_prices,
        })
    }

    async fn has_default_admin_role_via_rpc(&self, wallet: &str) -> anyhow::Result<bool> {
        let wallet = Address::from_str(wallet).context("invalid admin wallet address")?;
        let mut last_error = None;

        for client in self.clients.values() {
            match client.has_default_admin_role(wallet).await {
                Ok(true) => return Ok(true),
                Ok(false) => {}
                Err(err) => last_error = Some(err),
            }
        }

        if let Some(err) = last_error {
            return Err(err).context("check DEFAULT_ADMIN_ROLE failed");
        }

        Ok(false)
    }
}

impl ChainClient {
    async fn has_default_admin_role(&self, wallet: Address) -> anyhow::Result<bool> {
        #[allow(deprecated)]
        let function = Function {
            name: "hasRole".to_string(),
            inputs: vec![
                Param {
                    name: "role".to_string(),
                    kind: ParamType::FixedBytes(32),
                    internal_type: None,
                },
                Param {
                    name: "account".to_string(),
                    kind: ParamType::Address,
                    internal_type: None,
                },
            ],
            outputs: vec![Param {
                name: "enabled".to_string(),
                kind: ParamType::Bool,
                internal_type: None,
            }],
            constant: None,
            state_mutability: StateMutability::View,
        };

        let calldata =
            function.encode_input(&[Token::FixedBytes(vec![0; 32]), Token::Address(wallet)])?;
        let tx = TransactionRequest::new()
            .to(self.sale_contract)
            .data(Bytes::from(calldata));
        let raw = self.provider.call(&tx.into(), None).await?;
        let decoded = function.decode_output(raw.as_ref())?;

        match &decoded[0] {
            Token::Bool(value) => Ok(*value),
            _ => anyhow::bail!("hasRole returned unexpected type"),
        }
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

        let base_filter = Filter::new()
            .address(client.sale_contract)
            .from_block(BlockNumber::Number(from_block.into()))
            .to_block(BlockNumber::Number(to_block.into()));

        let legacy_filter = base_filter
            .clone()
            .topic0(<TicketsPurchasedLog as ethers_contract::EthEvent>::signature());
        let intent_filter = base_filter
            .clone()
            .topic0(<TicketsPurchasedWithIntentLog as ethers_contract::EthEvent>::signature());

        let mut logs = client.provider.get_logs(&legacy_filter).await?;
        logs.extend(client.provider.get_logs(&intent_filter).await?);
        let mut purchases = Vec::with_capacity(logs.len());
        for log in logs {
            if let Some(decoded) = self.decode_purchase_log(client, &log)? {
                purchases.push(decoded);
            }
        }

        purchases.sort_by_key(|item| (item.block_number, item.log_index));
        Ok(purchases)
    }

    async fn quote_purchase(
        &self,
        chain_id: u64,
        level_ids: &[u8],
        quantities: &[u64],
    ) -> anyhow::Result<QuoteResult> {
        self.quote_purchase_via_rpc(chain_id, level_ids, quantities)
            .await
    }

    async fn has_default_admin_role(&self, wallet: &str) -> anyhow::Result<bool> {
        self.has_default_admin_role_via_rpc(wallet).await
    }
}

fn u256_to_u64(value: U256) -> anyhow::Result<u64> {
    if value > U256::from(u64::MAX) {
        anyhow::bail!("value out of u64 range")
    }
    Ok(value.as_u64())
}

fn build_decoded_purchase(
    tx_hash: String,
    log_index: u64,
    block_number: u64,
    block_hash: Option<String>,
    order_id: U256,
    buyer: Address,
    payment_token: Address,
    total_amount: U256,
    level_ids: Vec<u8>,
    quantities: Vec<U256>,
    unit_prices: Vec<U256>,
    intent_id: Option<String>,
) -> anyhow::Result<Option<DecodedPurchase>> {
    if level_ids.len() != quantities.len() || level_ids.len() != unit_prices.len() {
        anyhow::bail!("event line-item lengths mismatch");
    }

    let mut decoded_quantities = Vec::with_capacity(quantities.len());
    for quantity in &quantities {
        decoded_quantities.push(u256_to_u64(*quantity).context("quantity exceeds u64 range")?);
    }

    let decoded_unit_prices = unit_prices
        .iter()
        .map(|value| value.to_string())
        .collect::<Vec<_>>();

    Ok(Some(DecodedPurchase {
        tx_hash,
        log_index,
        block_number,
        block_hash,
        order_id: order_id.to_string(),
        buyer: format!("{:#x}", buyer),
        payment_token: format!("{:#x}", payment_token),
        total_amount: total_amount.to_string(),
        level_ids,
        quantities: decoded_quantities,
        unit_prices: decoded_unit_prices,
        intent_id,
    }))
}
