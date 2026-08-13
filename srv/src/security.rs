use hmac::{Hmac, Mac};
use sha2::Sha256;
use tower_sessions::Session;

use crate::error::AppError;

type HmacSha256 = Hmac<Sha256>;
const ADMIN_AUTHENTICATED_SESSION_KEY: &str = "admin_authenticated";
const ADMIN_KEY_VERIFICATION_CONTEXT: &[u8] = b"qddxp-admin-key-verification-v1";

/// 使用 HMAC 标签的恒定时间校验比较管理员密钥，避免直接字符串比较过早返回。
/// 管理员密钥只允许在登录接口出现，认证后的业务请求只读取服务端会话。
pub fn verify_admin_key(candidate: &str, expected: &str) -> bool {
    let mut expected_mac = HmacSha256::new_from_slice(expected.as_bytes())
        .expect("HMAC-SHA256 accepts keys of any length");
    expected_mac.update(ADMIN_KEY_VERIFICATION_CONTEXT);
    let expected_tag = expected_mac.finalize().into_bytes();

    let mut candidate_mac = HmacSha256::new_from_slice(candidate.as_bytes())
        .expect("HMAC-SHA256 accepts keys of any length");
    candidate_mac.update(ADMIN_KEY_VERIFICATION_CONTEXT);
    candidate_mac.verify_slice(expected_tag.as_slice()).is_ok()
}

/// 登录成功后轮换会话 ID，防止攻击者预先植入固定会话 ID；Cookie 只保存随机 ID，
/// 认证标记始终留在服务端的会话存储中。
pub async fn authenticate_admin_session(session: &Session) -> Result<(), AppError> {
    session.cycle_id().await?;
    session
        .insert(ADMIN_AUTHENTICATED_SESSION_KEY, true)
        .await?;
    Ok(())
}

pub async fn is_admin_session(session: &Session) -> Result<bool, AppError> {
    Ok(session
        .get::<bool>(ADMIN_AUTHENTICATED_SESSION_KEY)
        .await?
        .unwrap_or(false))
}

pub async fn clear_admin_session(session: &Session) -> Result<(), AppError> {
    session.flush().await?;
    Ok(())
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
