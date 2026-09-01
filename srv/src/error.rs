use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde_json::json;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("{0}")]
    BadRequest(String),
    #[error("unauthorized")]
    Unauthorized,
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    Conflict(String),
    #[error("database error")]
    Database(#[from] diesel::result::Error),
    #[error("connection pool error")]
    Pool(#[from] diesel_async::pooled_connection::bb8::RunError),
    #[error("session error")]
    Session(#[from] tower_sessions::session::Error),
    #[error("CAPTCHA error")]
    Captcha(#[from] crate::captcha::CaptchaError),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            Self::BadRequest(message) => (StatusCode::BAD_REQUEST, message),
            Self::Unauthorized => (StatusCode::UNAUTHORIZED, "unauthorized".to_string()),
            Self::NotFound(message) => (StatusCode::NOT_FOUND, message),
            Self::Conflict(message) => (StatusCode::CONFLICT, message),
            Self::Database(diesel::result::Error::NotFound) => {
                (StatusCode::NOT_FOUND, "resource not found".to_string())
            }
            error @ (Self::Database(_) | Self::Pool(_) | Self::Session(_) | Self::Captcha(_)) => {
                tracing::error!(error = ?error, "internal error");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal server error".to_string(),
                )
            }
        };

        (status, Json(json!({ "error": message }))).into_response()
    }
}
