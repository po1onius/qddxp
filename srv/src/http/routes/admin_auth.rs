use axum::{Json, extract::State, http::StatusCode};
use serde::{Deserialize, Serialize};
use tower_sessions::Session;

use crate::{
    AppState,
    error::AppError,
    security::{
        authenticate_admin_session, clear_admin_session, is_admin_session, verify_admin_key,
    },
};

#[derive(Debug, Deserialize)]
pub struct AdminLoginRequest {
    pub admin_key: String,
}

#[derive(Debug, Serialize)]
pub struct AdminSessionResponse {
    pub authenticated: bool,
}

/// 使用部署时配置的 ADMIN_KEY 建立管理员会话。任何日志都不能记录密钥原文、长度或
/// 会话 ID，避免认证凭据从调试日志间接泄露。
pub async fn login(
    State(state): State<AppState>,
    session: Session,
    Json(request): Json<AdminLoginRequest>,
) -> Result<Json<AdminSessionResponse>, AppError> {
    if !verify_admin_key(&request.admin_key, &state.config.admin_key) {
        // 如果浏览器携带了旧会话或攻击者预置的 Cookie，失败登录同时清空它，确保失败
        // 响应不会继续保留任何管理员身份。
        clear_admin_session(&session).await?;
        tracing::warn!("admin login rejected: invalid credentials");
        return Err(AppError::Unauthorized);
    }

    authenticate_admin_session(&session).await?;
    tracing::info!("admin login succeeded and session id was rotated");

    Ok(Json(AdminSessionResponse {
        authenticated: true,
    }))
}

/// 前端首次进入 `/admin` 时通过此接口恢复登录状态。未登录属于正常业务状态，返回
/// `200 + authenticated=false`，便于页面稳定展示登录表单而不是制造异常日志。
pub async fn status(session: Session) -> Result<Json<AdminSessionResponse>, AppError> {
    let authenticated = is_admin_session(&session).await?;
    tracing::debug!(authenticated, "admin session status checked");
    Ok(Json(AdminSessionResponse { authenticated }))
}

pub async fn logout(session: Session) -> Result<StatusCode, AppError> {
    let was_authenticated = is_admin_session(&session).await?;
    clear_admin_session(&session).await?;
    tracing::info!(
        was_authenticated,
        "admin session logged out and invalidated"
    );
    Ok(StatusCode::NO_CONTENT)
}
