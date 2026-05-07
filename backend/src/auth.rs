use std::str::FromStr;

use axum::http::HeaderMap;
use chrono::{Duration, Utc};
use ethers_core::types::{Address, Signature};
use jsonwebtoken::{decode, encode, Algorithm, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};

use crate::error::ApiError;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String,
    pub iat: usize,
    pub exp: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdminClaims {
    pub sub: String,
    pub scope: String,
    pub role: String,
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

    pub fn issue_admin(
        &self,
        wallet: &str,
        role: &str,
        ttl_hours: i64,
    ) -> Result<(String, i64), ApiError> {
        if !is_valid_admin_role(role) {
            return Err(ApiError::unauthorized("invalid admin role"));
        }

        let iat = Utc::now();
        let exp = iat + Duration::hours(ttl_hours);
        let claims = AdminClaims {
            sub: wallet.to_owned(),
            scope: "admin".to_string(),
            role: role.to_string(),
            iat: iat.timestamp() as usize,
            exp: exp.timestamp() as usize,
        };

        let token = encode(&Header::new(Algorithm::HS256), &claims, &self.encoding_key)
            .map_err(|err| ApiError::internal(format!("admin jwt encode failed: {err}")))?;

        Ok((token, exp.timestamp()))
    }

    pub fn verify_admin(&self, token: &str) -> Result<AdminClaims, ApiError> {
        let validation = Validation::new(Algorithm::HS256);
        let data = decode::<AdminClaims>(token, &self.decoding_key, &validation)
            .map_err(|_| ApiError::unauthorized("invalid admin jwt"))?;

        if data.claims.scope != "admin" {
            return Err(ApiError::unauthorized("invalid admin scope"));
        }
        if !is_valid_admin_role(&data.claims.role) {
            return Err(ApiError::unauthorized("invalid admin role"));
        }

        Ok(data.claims)
    }
}

pub fn is_valid_admin_role(role: &str) -> bool {
    matches!(role, "viewer" | "operator" | "finance" | "admin")
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn admin_jwt_round_trips_admin_claims() {
        let jwt = JwtCodec::new("test-secret", 1).expect("jwt codec should initialize");

        let (token, expires_at) = jwt
            .issue_admin("0x1111111111111111111111111111111111111111", "operator", 12)
            .expect("admin jwt should issue");
        let claims = jwt.verify_admin(&token).expect("admin jwt should verify");

        assert_eq!(claims.sub, "0x1111111111111111111111111111111111111111");
        assert_eq!(claims.scope, "admin");
        assert_eq!(claims.role, "operator");
        assert!(expires_at > claims.iat as i64);
    }

    #[test]
    fn admin_jwt_rejects_buyer_tokens() {
        let jwt = JwtCodec::new("test-secret", 1).expect("jwt codec should initialize");
        let (buyer_token, _) = jwt
            .issue("0x1111111111111111111111111111111111111111")
            .expect("buyer jwt should issue");

        let err = jwt
            .verify_admin(&buyer_token)
            .expect_err("buyer jwt must not verify as admin");

        assert_eq!(err.status, axum::http::StatusCode::UNAUTHORIZED);
    }

    #[test]
    fn admin_jwt_rejects_invalid_roles() {
        let jwt = JwtCodec::new("test-secret", 1).expect("jwt codec should initialize");
        let iat = Utc::now();
        let exp = iat + Duration::hours(12);
        let claims = serde_json::json!({
            "sub": "0x1111111111111111111111111111111111111111",
            "scope": "admin",
            "role": "owner",
            "iat": iat.timestamp() as usize,
            "exp": exp.timestamp() as usize
        });
        let token = encode(&Header::new(Algorithm::HS256), &claims, &jwt.encoding_key)
            .expect("custom jwt should encode");

        let err = jwt
            .verify_admin(&token)
            .expect_err("invalid admin role must not verify");

        assert_eq!(err.status, axum::http::StatusCode::UNAUTHORIZED);
    }
}
