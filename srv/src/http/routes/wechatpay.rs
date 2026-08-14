use axum::{
    Json,
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use chrono::{DateTime, Utc};
use diesel::prelude::*;
use diesel_async::RunQueryDsl;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::{
    AppState,
    db::{
        models::{NewApiCallLog, Order, PaymentAttempt},
        schema::{api_call_logs, orders, payment_attempts},
    },
    domain::{ApiName, HttpMethod, PaymentProvider},
    error::AppError,
    payments::{
        service::{PaymentConfirmation, confirm_payment},
        wechatpay::{
            WechatPayClient, WechatPayNotification, WechatPayQueryTransaction, WechatPayTransaction,
        },
    },
    security::verify_order_password,
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

#[derive(Debug, Deserialize)]
pub struct ReconcileOrderRequest {
    pub order_password: String,
}

#[derive(Debug, Serialize)]
pub struct ReconcileOrderResponse {
    pub status: String,
    pub trade_state: String,
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

/// 用户点击“我已支付”时主动向微信查单。查询结果仍进入统一确认事务，不能直接修改订单。
pub async fn reconcile_order(
    State(state): State<AppState>,
    Path(order_id): Path<Uuid>,
    Json(request): Json<ReconcileOrderRequest>,
) -> Result<Json<ReconcileOrderResponse>, AppError> {
    if request.order_password.is_empty() {
        return Err(AppError::BadRequest(
            "order_password is required".to_string(),
        ));
    }
    let client = state
        .wechatpay
        .as_ref()
        .ok_or_else(|| AppError::BadRequest("WeChat Pay is not configured".to_string()))?;
    let mut conn = state.pool.get().await?;
    let order = orders::table
        .filter(orders::id.eq(order_id))
        .first::<Order>(&mut conn)
        .await
        .optional()?
        .ok_or_else(|| AppError::NotFound("order not found".to_string()))?;
    if !verify_order_password(
        &request.order_password,
        &order.order_password_hash,
        &state.config.order_password_pepper,
    )? {
        return Err(AppError::Unauthorized);
    }
    let attempt = payment_attempts::table
        .filter(payment_attempts::order_id.eq(order.id))
        .filter(payment_attempts::provider.eq(PaymentProvider::Wechatpay.as_ref()))
        .first::<PaymentAttempt>(&mut conn)
        .await
        .optional()?
        .ok_or_else(|| AppError::NotFound("WeChat Pay attempt not found".to_string()))?;
    drop(conn);

    let transaction = client
        .query_order(&attempt.merchant_trade_no)
        .await
        .map_err(|error| {
            tracing::error!(
                %order_id,
                payment_attempt_id = %attempt.id,
                error = ?error,
                "official WeChat Pay active order query failed"
            );
            AppError::Upstream("WeChat Pay order query failed".to_string())
        })?;
    validate_query_identity(client, &transaction).map_err(AppError::BadRequest)?;
    if transaction.out_trade_no != attempt.merchant_trade_no {
        return Err(AppError::BadRequest(
            "WeChat Pay query order mismatch".to_string(),
        ));
    }
    let trade_state = transaction.trade_state.clone();

    if transaction.trade_state == TRADE_STATE_SUCCESS {
        let transaction = transaction
            .into_success()
            .map_err(|error| AppError::BadRequest(error.to_string()))?;
        validate_transaction(client, &transaction).map_err(AppError::BadRequest)?;
        if parse_attach_order_id(&transaction).map_err(AppError::BadRequest)? != order.id {
            return Err(AppError::BadRequest(
                "WeChat Pay query attach order mismatch".to_string(),
            ));
        }
        let paid_at = parse_success_time(&transaction).map_err(AppError::BadRequest)?;
        let provider_event_id = format!("query:{}", transaction.transaction_id);
        let request_body = serde_json::to_string(&transaction)
            .map_err(|error| AppError::BadRequest(error.to_string()))?;
        confirm_payment(
            &state.pool,
            state.telegram.clone(),
            PaymentConfirmation {
                provider: PaymentProvider::Wechatpay.as_ref(),
                provider_event_id: &provider_event_id,
                event_type: "TRANSACTION.SUCCESS.QUERY",
                merchant_trade_no: &transaction.out_trade_no,
                provider_transaction_id: &transaction.transaction_id,
                expected_order_id: Some(order.id),
                amount_cents: transaction.amount.total,
                currency: &transaction.amount.currency,
                paid_at,
                request_body: &request_body,
            },
        )
        .await?;
    }

    let mut conn = state.pool.get().await?;
    let status = orders::table
        .filter(orders::id.eq(order.id))
        .select(orders::status)
        .first::<String>(&mut conn)
        .await?;
    tracing::info!(
        %order_id,
        payment_attempt_id = %attempt.id,
        %trade_state,
        order_status = %status,
        "official WeChat Pay active order query completed"
    );
    Ok(Json(ReconcileOrderResponse {
        status,
        trade_state,
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

fn validate_query_identity(
    client: &WechatPayClient,
    transaction: &WechatPayQueryTransaction,
) -> Result<(), String> {
    if transaction.appid != client.app_id() || transaction.mchid != client.mch_id() {
        return Err("微信支付查单商户身份不匹配".to_string());
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

fn parse_attach_order_id(transaction: &WechatPayTransaction) -> Result<Uuid, String> {
    transaction
        .attach
        .as_deref()
        .ok_or_else(|| "微信支付交易缺少 attach 订单号".to_string())
        .and_then(|value| {
            Uuid::parse_str(value).map_err(|_| "微信支付交易 attach 订单号无效".to_string())
        })
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
