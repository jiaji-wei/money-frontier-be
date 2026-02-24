use std::str::FromStr;

use axum::http::HeaderMap;
use chrono::{Duration, Utc};
use ethers::types::{Address, Signature};
use jsonwebtoken::{decode, encode, Algorithm, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};

use crate::error::ApiError;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String,
    pub iat: usize,
    pub exp: usize,
}

#[derive(Clone)]
pub struct JwtCodec {
    encoding_key: EncodingKey,
    decoding_key: DecodingKey,
    ttl_days: i64,
}

impl JwtCodec {
    pub fn new(secret: &str, ttl_days: i64) -> anyhow::Result<Self> {
        if secret.is_empty() {
            anyhow::bail!("JWT secret must not be empty");
        }

        Ok(Self {
            encoding_key: EncodingKey::from_secret(secret.as_bytes()),
            decoding_key: DecodingKey::from_secret(secret.as_bytes()),
            ttl_days,
        })
    }

    pub fn issue(&self, wallet: &str) -> Result<(String, i64), ApiError> {
        let iat = Utc::now();
        let exp = iat + Duration::days(self.ttl_days);
        let claims = Claims {
            sub: wallet.to_owned(),
            iat: iat.timestamp() as usize,
            exp: exp.timestamp() as usize,
        };

        let token = encode(&Header::new(Algorithm::HS256), &claims, &self.encoding_key)
            .map_err(|err| ApiError::internal(format!("jwt encode failed: {err}")))?;

        Ok((token, exp.timestamp()))
    }

    pub fn verify(&self, token: &str) -> Result<Claims, ApiError> {
        let validation = Validation::new(Algorithm::HS256);
        let data = decode::<Claims>(token, &self.decoding_key, &validation)
            .map_err(|_| ApiError::unauthorized("invalid jwt"))?;
        Ok(data.claims)
    }
}

pub fn extract_wallet(headers: &HeaderMap, jwt: &JwtCodec) -> Result<String, ApiError> {
    let raw_header = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| ApiError::unauthorized("missing authorization header"))?;

    let token = raw_header
        .strip_prefix("Bearer ")
        .ok_or_else(|| ApiError::unauthorized("invalid authorization header"))?;

    let claims = jwt.verify(token)?;
    Ok(claims.sub)
}

pub fn normalize_wallet_address(address: &str) -> Result<String, ApiError> {
    let parsed =
        Address::from_str(address).map_err(|_| ApiError::bad_request("invalid wallet address"))?;
    Ok(format!("{parsed:#x}"))
}

pub fn verify_wallet_signature(
    address: &str,
    message: &str,
    signature: &str,
) -> Result<(), ApiError> {
    let expected =
        Address::from_str(address).map_err(|_| ApiError::bad_request("invalid wallet address"))?;
    let parsed_signature = Signature::from_str(signature)
        .map_err(|_| ApiError::bad_request("invalid signature format"))?;

    let recovered = parsed_signature
        .recover(message)
        .map_err(|_| ApiError::unauthorized("signature verification failed"))?;

    if recovered != expected {
        return Err(ApiError::unauthorized("signature mismatch"));
    }

    Ok(())
}
