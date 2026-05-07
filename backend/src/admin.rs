use std::str::FromStr;

use axum::http::{header, HeaderMap};

use crate::{
    auth::{AdminClaims, JwtCodec},
    error::ApiError,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdminRole {
    Viewer,
    Operator,
    Finance,
    Admin,
}

impl AdminRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Viewer => "viewer",
            Self::Operator => "operator",
            Self::Finance => "finance",
            Self::Admin => "admin",
        }
    }
}

impl FromStr for AdminRole {
    type Err = ApiError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "viewer" => Ok(Self::Viewer),
            "operator" => Ok(Self::Operator),
            "finance" => Ok(Self::Finance),
            "admin" => Ok(Self::Admin),
            _ => Err(ApiError::unauthorized("invalid admin role")),
        }
    }
}

#[derive(Debug, Clone)]
pub struct AdminPrincipal {
    pub wallet: String,
    pub role: AdminRole,
}

impl TryFrom<AdminClaims> for AdminPrincipal {
    type Error = ApiError;

    fn try_from(claims: AdminClaims) -> Result<Self, Self::Error> {
        Ok(Self {
            wallet: claims.sub,
            role: claims.role.parse()?,
        })
    }
}

pub fn extract_admin(headers: &HeaderMap, jwt: &JwtCodec) -> Result<AdminPrincipal, ApiError> {
    let raw_header = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| ApiError::unauthorized("missing authorization header"))?;

    let token = raw_header
        .strip_prefix("Bearer ")
        .ok_or_else(|| ApiError::unauthorized("invalid authorization header"))?;

    AdminPrincipal::try_from(jwt.verify_admin(token)?)
}

pub fn require_role(
    principal: &AdminPrincipal,
    allowed_roles: &[AdminRole],
) -> Result<(), ApiError> {
    if principal.role == AdminRole::Admin || allowed_roles.contains(&principal.role) {
        return Ok(());
    }

    Err(ApiError::forbidden("insufficient admin role"))
}
