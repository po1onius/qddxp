use axum::{
    Json,
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use chrono::{DateTime, Utc};
use diesel_async::RunQueryDsl;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::{
    AppState,
    db::{models::NewApiCallLog, schema::api_call_logs},
    domain::{ApiName, HttpMethod, PaymentProvider},
    error::AppError,
    payments::{
        service::{PaymentConfirmation, confirm_payment},
        wechatpay::{WechatPayClient, WechatPayNotification, WechatPayTransaction},
    },
};

const TRANSACTION_SUCCESS: &str = "TRANSACTION.SUCCESS";
const TRADE_STATE_SUCCESS: &str = "SUCCESS";
const TRADE_TYPE_NATIVE: &str = "NATIVE";
const NOTIFY_PATH: &str = "/api/payments/wechatpay/notify";

#[derive(Debug)]
enum NotifyError {
    Unauthorized(String),
    BadRequest(String),
    Business(String),
}

/// 微信支付回调必须读取原始字节，不能先经过 JSON 提取器再重新序列化，否则会破坏签名原文。
pub async fn notify(State(state): State<AppState>, headers: HeaderMap, body: Bytes) -> Response {
    let body = match String::from_utf8(body.to_vec()) {
        Ok(body) => body,
        Err(_) => {
            return notify_error_response(
                &state,
                StatusCode::BAD_REQUEST,
                "PARAM_ERROR",
                "通知报文不是 UTF-8",
                json!({ "parse_error": "invalid utf-8" }),
            )
            .await;
        }
    };
    tracing::info!(body_len = body.len(), "official WeChat Pay notify received");

    match process_notify(&state, &headers, &body).await {
        Ok(params) => {
            record_notify_call(&state, params, StatusCode::NO_CONTENT, true, "", None).await;
            tracing::info!("official WeChat Pay notify applied");
            StatusCode::NO_CONTENT.into_response()
        }
        Err(NotifyError::Unauthorized(message)) => {
            notify_error_response(
                &state,
                StatusCode::UNAUTHORIZED,
                "SIGN_ERROR",
                &message,
                json!({ "body_len": body.len() }),
            )
            .await
        }
        Err(NotifyError::BadRequest(message)) => {
            notify_error_response(
                &state,
                StatusCode::BAD_REQUEST,
                "PARAM_ERROR",
                &message,
                json!({ "body_len": body.len() }),
            )
            .await
        }
        Err(NotifyError::Business(message)) => {
            notify_error_response(
                &state,
                StatusCode::INTERNAL_SERVER_ERROR,
                "FAIL",
                &message,
                json!({ "body_len": body.len() }),
            )
            .await
        }
    }
}

async fn process_notify(
    state: &AppState,
    headers: &HeaderMap,
    body: &str,
) -> Result<Value, NotifyError> {
    let client = state
        .wechatpay
        .as_ref()
        .ok_or_else(|| NotifyError::Business("微信支付未配置".to_string()))?;
    validate_callback_timestamp(headers)?;
    client
        .verify_signed_message(headers, body)
        .map_err(|error| NotifyError::Unauthorized(error.to_string()))?;
    let notification = serde_json::from_str::<WechatPayNotification>(body)
        .map_err(|error| NotifyError::BadRequest(error.to_string()))?;
    if notification.event_type != TRANSACTION_SUCCESS
        || notification.resource.original_type != "transaction"
    {
        return Err(NotifyError::BadRequest(
            "不支持的微信支付通知类型".to_string(),
        ));
    }
    let transaction = client
        .decrypt_notification::<WechatPayTransaction>(&notification.resource)
        .map_err(|error| NotifyError::Unauthorized(error.to_string()))?;
    validate_transaction(client, &transaction).map_err(NotifyError::BadRequest)?;
    let paid_at = parse_success_time(&transaction).map_err(NotifyError::BadRequest)?;
    let expected_order_id = transaction
        .attach
        .as_deref()
        .ok_or_else(|| NotifyError::BadRequest("支付通知缺少 attach 订单号".to_string()))
        .and_then(|value| {
            Uuid::parse_str(value)
                .map_err(|_| NotifyError::BadRequest("支付通知订单号无效".to_string()))
        })?;

    confirm_payment(
        &state.pool,
        state.telegram.clone(),
        PaymentConfirmation {
            provider: PaymentProvider::Wechatpay.as_ref(),
            provider_event_id: &notification.id,
            event_type: &notification.event_type,
            merchant_trade_no: &transaction.out_trade_no,
            provider_transaction_id: &transaction.transaction_id,
            expected_order_id: Some(expected_order_id),
            amount_cents: transaction.amount.total,
            currency: &transaction.amount.currency,
            paid_at,
            request_body: body,
        },
    )
    .await
    .map_err(|error| NotifyError::Business(error.to_string()))?;

    Ok(json!({
        "notification_id": notification.id,
        "event_type": notification.event_type,
        "merchant_trade_no": transaction.out_trade_no,
        "provider_transaction_id": transaction.transaction_id,
        "amount_cents": transaction.amount.total,
        "currency": transaction.amount.currency,
    }))
}

fn validate_transaction(
    client: &WechatPayClient,
    transaction: &WechatPayTransaction,
) -> Result<(), String> {
    validate_transaction_identity(client, transaction)?;
    if transaction.trade_state != TRADE_STATE_SUCCESS {
        return Err("微信支付交易状态不是 SUCCESS".to_string());
    }
    if transaction.trade_type != TRADE_TYPE_NATIVE {
        return Err("微信支付交易类型不是 NATIVE".to_string());
    }
    if transaction.amount.currency != "CNY" {
        return Err("微信支付币种不是 CNY".to_string());
    }
    Ok(())
}

fn validate_transaction_identity(
    client: &WechatPayClient,
    transaction: &WechatPayTransaction,
) -> Result<(), String> {
    if transaction.appid != client.app_id() || transaction.mchid != client.mch_id() {
        return Err("微信支付商户身份不匹配".to_string());
    }
    Ok(())
}

fn parse_success_time(transaction: &WechatPayTransaction) -> Result<DateTime<Utc>, String> {
    let value = transaction
        .success_time
        .as_deref()
        .ok_or_else(|| "微信支付成功时间缺失".to_string())?;
    DateTime::parse_from_rfc3339(value)
        .map(|time| time.with_timezone(&Utc))
        .map_err(|_| "微信支付成功时间格式无效".to_string())
}

fn validate_callback_timestamp(headers: &HeaderMap) -> Result<(), NotifyError> {
    let timestamp = headers
        .get("Wechatpay-Timestamp")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<i64>().ok())
        .ok_or_else(|| NotifyError::Unauthorized("回调时间戳缺失或无效".to_string()))?;
    if (Utc::now().timestamp() - timestamp).abs() > 300 {
        return Err(NotifyError::Unauthorized(
            "回调时间戳超出五分钟安全窗口".to_string(),
        ));
    }
    Ok(())
}

async fn notify_error_response(
    state: &AppState,
    status: StatusCode,
    code: &str,
    message: &str,
    request_params: Value,
) -> Response {
    tracing::warn!(
        status = status.as_u16(),
        code,
        message,
        "WeChat Pay notify rejected"
    );
    let response = json!({ "code": code, "message": message });
    record_notify_call(
        state,
        request_params,
        status,
        false,
        &response.to_string(),
        Some(message.to_string()),
    )
    .await;
    (status, Json(response)).into_response()
}

async fn record_notify_call(
    state: &AppState,
    request_params: Value,
    status: StatusCode,
    success: bool,
    response_body: &str,
    error_message: Option<String>,
) {
    let result = async {
        let mut conn = state.pool.get().await?;
        diesel::insert_into(api_call_logs::table)
            .values(&NewApiCallLog {
                id: Uuid::new_v4(),
                api_name: ApiName::WechatpayNotify.as_ref(),
                http_method: HttpMethod::Post.as_ref(),
                path: NOTIFY_PATH,
                request_params: &request_params,
                response_status: i32::from(status.as_u16()),
                response_body,
                success,
                error_message: error_message.as_deref(),
            })
            .execute(&mut conn)
            .await?;
        Ok::<(), AppError>(())
    }
    .await;
    if let Err(error) = result {
        tracing::error!(error = ?error, "failed to record WeChat Pay notify API log");
    }
}
