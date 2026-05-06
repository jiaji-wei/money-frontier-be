use std::str::FromStr;

use ethers_core::{
    abi::{encode, Token},
    types::{Address, H256, U256},
    utils::keccak256,
};
use ethers_signers::{LocalWallet, Signer};
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Debug, Clone, FromRow)]
pub struct PromotionCodeRow {
    pub id: i64,
    pub code_normalized: String,
    pub kind: String,
    pub status: String,
    pub beneficiary_wallet: Option<String>,
    pub valid_from: Option<i64>,
    pub valid_until: Option<i64>,
    pub max_total_uses: Option<i64>,
    pub max_uses_per_wallet: Option<i64>,
    pub first_purchase_only: bool,
    pub stacking_policy: Option<String>,
    pub applicable_chain_ids: Option<String>,
    pub applicable_ticket_levels: Option<String>,
    pub discount_type: Option<String>,
    pub discount_value: Option<String>,
    pub max_discount_amount: Option<String>,
    pub commission_type: Option<String>,
    pub commission_value: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, FromRow)]
pub struct WalletReferralBindingRow {
    pub wallet_address: String,
    pub referral_code_id: i64,
    pub bound_at: i64,
    pub first_bound_source: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReferralBindResult {
    pub bound: bool,
    pub referral_code_id: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PurchaseIntentStatus {
    Pending,
    Submitted,
    Confirmed,
    Expired,
    Cancelled,
}

impl PurchaseIntentStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Submitted => "submitted",
            Self::Confirmed => "confirmed",
            Self::Expired => "expired",
            Self::Cancelled => "cancelled",
        }
    }
}

#[derive(Debug, Clone)]
pub struct NewPurchaseIntent {
    pub id: Option<String>,
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
    pub status: PurchaseIntentStatus,
    pub tx_hash: Option<String>,
    pub order_id: Option<String>,
}

impl NewPurchaseIntent {
    pub fn resolve_id(&self) -> String {
        self.id.clone().unwrap_or_else(generate_intent_id)
    }
}

#[derive(Debug, Clone, FromRow)]
pub struct PurchaseIntentRow {
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
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiscountRedemptionStatus {
    Reserved,
    Confirmed,
    Released,
}

impl DiscountRedemptionStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Reserved => "reserved",
            Self::Confirmed => "confirmed",
            Self::Released => "released",
        }
    }
}

#[derive(Debug, Clone)]
pub struct NewDiscountRedemption {
    pub purchase_intent_id: String,
    pub discount_code_id: i64,
    pub wallet_address: String,
    pub status: DiscountRedemptionStatus,
    pub tx_hash: Option<String>,
    pub order_id: Option<String>,
    pub reserved_at: i64,
    pub confirmed_at: Option<i64>,
    pub released_at: Option<i64>,
}

#[derive(Debug, Clone, FromRow)]
pub struct DiscountRedemptionRow {
    pub purchase_intent_id: String,
    pub discount_code_id: i64,
    pub wallet_address: String,
    pub status: String,
    pub tx_hash: Option<String>,
    pub order_id: Option<String>,
    pub reserved_at: i64,
    pub confirmed_at: Option<i64>,
    pub released_at: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct NewOrderPromotionsSnapshot {
    pub order_row_id: i64,
    pub wallet_address: String,
    pub referral_code_id: Option<i64>,
    pub discount_code_id: Option<i64>,
    pub original_total_amount: String,
    pub discount_amount: String,
    pub paid_amount: String,
    pub commission_base_amount: String,
    pub commission_amount: String,
    pub rule_version: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, FromRow)]
pub struct OrderPromotionsSnapshotRow {
    pub order_row_id: i64,
    pub wallet_address: String,
    pub referral_code_id: Option<i64>,
    pub discount_code_id: Option<i64>,
    pub original_total_amount: String,
    pub discount_amount: String,
    pub paid_amount: String,
    pub commission_base_amount: String,
    pub commission_amount: String,
    pub rule_version: String,
    pub created_at: i64,
}

pub fn normalize_promotion_code(value: &str) -> Option<String> {
    let normalized = value.trim().to_uppercase();
    if normalized.is_empty() {
        None
    } else {
        Some(normalized)
    }
}

pub fn normalize_wallet_key(value: &str) -> String {
    value.trim().to_lowercase()
}

pub fn generate_intent_id() -> String {
    let high = Uuid::new_v4().as_u128();
    let low = Uuid::new_v4().as_u128();
    format!("0x{high:032x}{low:032x}")
}

pub fn build_purchase_authorization_digest(
    sale_contract: &str,
    chain_id: u64,
    buyer: &str,
    payment_token: &str,
    level_ids: &[u8],
    quantities: &[u64],
    intent_id: &str,
    final_total_amount: &str,
    expires_at: i64,
) -> anyhow::Result<[u8; 32]> {
    let sale_contract = Address::from_str(sale_contract)?;
    let buyer = Address::from_str(buyer)?;
    let payment_token = Address::from_str(payment_token)?;
    let intent_id = H256::from_str(intent_id)?;
    let final_total_amount = U256::from_dec_str(final_total_amount)?;
    let expires_at = U256::from(u64::try_from(expires_at)?);

    let encoded = encode(&[
        Token::Address(sale_contract),
        Token::Uint(U256::from(chain_id)),
        Token::Address(buyer),
        Token::Address(payment_token),
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
        Token::FixedBytes(intent_id.as_bytes().to_vec()),
        Token::Uint(final_total_amount),
        Token::Uint(expires_at),
    ]);

    Ok(keccak256(encoded))
}

pub async fn sign_purchase_authorization(
    signer: &LocalWallet,
    sale_contract: &str,
    chain_id: u64,
    buyer: &str,
    payment_token: &str,
    level_ids: &[u8],
    quantities: &[u64],
    intent_id: &str,
    final_total_amount: &str,
    expires_at: i64,
) -> anyhow::Result<String> {
    let digest = build_purchase_authorization_digest(
        sale_contract,
        chain_id,
        buyer,
        payment_token,
        level_ids,
        quantities,
        intent_id,
        final_total_amount,
        expires_at,
    )?;

    let signature = signer.sign_message(digest).await?;
    Ok(signature.to_string())
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use ethers_core::types::Signature;
    use ethers_signers::{LocalWallet, Signer};

    use super::{build_purchase_authorization_digest, sign_purchase_authorization};

    #[tokio::test]
    async fn purchase_authorization_signature_recovers_signer_from_eth_signed_digest() {
        let signer = LocalWallet::from_str(
            "0x8b3a350cf5c34c9194ca3a545d4d2ce7d9f69b17a3b2ecfacac4f2d0f6f7f204",
        )
        .expect("wallet should parse");
        let digest = build_purchase_authorization_digest(
            "0x0000000000000000000000000000000000005000",
            31337,
            "0x70997970C51812dc3A010C7d01b50e0d17dc79C8",
            "0x5FbDB2315678afecb367f032d93F642f64180aa3",
            &[1, 2],
            &[1, 3],
            "0x1111111111111111111111111111111111111111111111111111111111111111",
            "400000000000000000",
            1_900_000_000,
        )
        .expect("digest should build");

        let signature = sign_purchase_authorization(
            &signer,
            "0x0000000000000000000000000000000000005000",
            31337,
            "0x70997970C51812dc3A010C7d01b50e0d17dc79C8",
            "0x5FbDB2315678afecb367f032d93F642f64180aa3",
            &[1, 2],
            &[1, 3],
            "0x1111111111111111111111111111111111111111111111111111111111111111",
            "400000000000000000",
            1_900_000_000,
        )
        .await
        .expect("signature should build");
        let signature = Signature::from_str(&signature).expect("signature should parse");

        let recovered = signature
            .recover(digest.to_vec())
            .expect("signature should recover");

        assert_eq!(recovered, signer.address());
    }
}
