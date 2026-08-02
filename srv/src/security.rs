use axum::http::HeaderMap;
use hmac::{Hmac, Mac};
use sha2::Sha256;

use crate::{config::AppConfig, error::AppError};

type HmacSha256 = Hmac<Sha256>;

pub fn require_admin(headers: &HeaderMap, config: &AppConfig) -> Result<(), AppError> {
    let Some(value) = headers.get("x-admin-key") else {
        return Err(AppError::Unauthorized);
    };

    if value.to_str().ok() == Some(config.admin_key.as_str()) {
        Ok(())
    } else {
        Err(AppError::Unauthorized)
    }
}

pub fn hash_order_password(password: &str, pepper: &str) -> Result<String, AppError> {
    let mut mac = HmacSha256::new_from_slice(pepper.as_bytes())
        .map_err(|_| AppError::BadRequest("invalid password pepper".to_string()))?;
    mac.update(password.as_bytes());
    Ok(hex::encode(mac.finalize().into_bytes()))
}

pub fn verify_order_password(
    password: &str,
    expected_hash: &str,
    pepper: &str,
) -> Result<bool, AppError> {
    Ok(hash_order_password(password, pepper)? == expected_hash)
}
