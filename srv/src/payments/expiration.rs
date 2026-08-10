use chrono::{DateTime, Utc};
use diesel::prelude::*;
use diesel_async::{AsyncConnection, RunQueryDsl};
use uuid::Uuid;

use crate::{
    AppState,
    db::{
        models::{Order, PaymentAttempt},
        schema::{orders, payment_attempts, products},
    },
    domain::{OrderStatus, PaymentAttemptState, PaymentProvider, ProductStatus},
    error::AppError,
    payments::{
        service::{PaymentConfirmation, confirm_payment},
        wechatpay::{WechatPayError, WechatPayQueryTransaction},
    },
};

const EXPIRE_BATCH_SIZE: i64 = 50;

/// 后台处理已到期的微信 Native 订单。必须先查单/关单，再释放库存；仅凭本地时钟释放
/// 会与用户在最后一刻完成付款产生竞态。
pub async fn run(state: AppState) {
    if state.wechatpay.is_none() {
        tracing::info!("WeChat Pay expiration worker disabled because provider is not configured");
        return;
    }
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
    loop {
        interval.tick().await;
        if let Err(error) = process_batch(&state).await {
            tracing::error!(error = ?error, "WeChat Pay expiration batch failed");
        }
    }
}

async fn process_batch(state: &AppState) -> Result<(), AppError> {
    let mut conn = state.pool.get().await?;
    let attempts = payment_attempts::table
        .filter(payment_attempts::provider.eq(PaymentProvider::Wechatpay.as_ref()))
        .filter(payment_attempts::state.eq_any([
            PaymentAttemptState::Created.as_ref(),
            PaymentAttemptState::PrepayCreated.as_ref(),
            PaymentAttemptState::Failed.as_ref(),
        ]))
        .filter(payment_attempts::expires_at.le(Utc::now()))
        .order(payment_attempts::expires_at.asc())
        .limit(EXPIRE_BATCH_SIZE)
        .load::<PaymentAttempt>(&mut conn)
        .await?;
    drop(conn);
    if attempts.is_empty() {
        return Ok(());
    }
    tracing::info!(
        count = attempts.len(),
        "processing expired WeChat Pay attempts"
    );
    for attempt in attempts {
        process_attempt(state, attempt).await;
    }
    Ok(())
}

async fn process_attempt(state: &AppState, attempt: PaymentAttempt) {
    let client = state
        .wechatpay
        .as_ref()
        .expect("expiration worker starts only with configured WeChat Pay");
    match client.query_order(&attempt.merchant_trade_no).await {
        Ok(transaction) if transaction.trade_state == "SUCCESS" => {
            if let Err(error) = apply_success(state, &attempt, transaction).await {
                tracing::error!(
                    payment_attempt_id = %attempt.id,
                    error = ?error,
                    "expired WeChat Pay attempt was paid but could not be applied"
                );
            }
        }
        Ok(transaction) if transaction.trade_state == "CLOSED" => {
            if let Err(error) = expire_local_attempt(state, &attempt).await {
                tracing::error!(payment_attempt_id = %attempt.id, error = ?error, "failed to expire closed WeChat Pay attempt");
            }
        }
        Ok(transaction) => match client.close_order(&attempt.merchant_trade_no).await {
            Ok(()) => {
                if let Err(error) = expire_local_attempt(state, &attempt).await {
                    tracing::error!(payment_attempt_id = %attempt.id, error = ?error, "failed to expire WeChat Pay attempt after close");
                }
            }
            Err(error) => tracing::warn!(
                payment_attempt_id = %attempt.id,
                trade_state = %transaction.trade_state,
                error = ?error,
                "WeChat Pay close failed; attempt retained for retry"
            ),
        },
        Err(WechatPayError::Api { code, .. }) if code == "ORDER_NOT_EXIST" => {
            if let Err(error) = expire_local_attempt(state, &attempt).await {
                tracing::error!(payment_attempt_id = %attempt.id, error = ?error, "failed to expire missing WeChat Pay attempt");
            }
        }
        Err(error) => tracing::warn!(
            payment_attempt_id = %attempt.id,
            error = ?error,
            "WeChat Pay query failed; expired attempt retained for retry"
        ),
    }
}

async fn apply_success(
    state: &AppState,
    attempt: &PaymentAttempt,
    transaction: WechatPayQueryTransaction,
) -> Result<(), AppError> {
    let client = state.wechatpay.as_ref().expect("client checked by worker");
    if transaction.appid != client.app_id()
        || transaction.mchid != client.mch_id()
        || transaction.out_trade_no != attempt.merchant_trade_no
    {
        return Err(AppError::BadRequest(
            "expired WeChat Pay query identity mismatch".to_string(),
        ));
    }
    let transaction = transaction
        .into_success()
        .map_err(|error| AppError::BadRequest(error.to_string()))?;
    if transaction.trade_type != "NATIVE" || transaction.amount.currency != "CNY" {
        return Err(AppError::BadRequest(
            "expired WeChat Pay query payment details mismatch".to_string(),
        ));
    }
    let attached_order_id = transaction
        .attach
        .as_deref()
        .ok_or_else(|| AppError::BadRequest("WeChat Pay attach missing".to_string()))
        .and_then(|value| {
            Uuid::parse_str(value)
                .map_err(|_| AppError::BadRequest("invalid WeChat Pay attach".to_string()))
        })?;
    if attached_order_id != attempt.order_id {
        return Err(AppError::BadRequest(
            "expired WeChat Pay query attach mismatch".to_string(),
        ));
    }
    let paid_at = transaction
        .success_time
        .as_deref()
        .ok_or_else(|| AppError::BadRequest("WeChat Pay success_time missing".to_string()))
        .and_then(|value| {
            DateTime::parse_from_rfc3339(value)
                .map(|value| value.with_timezone(&Utc))
                .map_err(|_| AppError::BadRequest("invalid WeChat Pay success_time".to_string()))
        })?;
    let event_id = format!("expiration-query:{}", transaction.transaction_id);
    let body = serde_json::to_string(&transaction)
        .map_err(|error| AppError::BadRequest(error.to_string()))?;
    confirm_payment(
        &state.pool,
        PaymentConfirmation {
            provider: PaymentProvider::Wechatpay.as_ref(),
            provider_event_id: &event_id,
            event_type: "TRANSACTION.SUCCESS.EXPIRATION_QUERY",
            merchant_trade_no: &transaction.out_trade_no,
            provider_transaction_id: &transaction.transaction_id,
            expected_order_id: Some(attempt.order_id),
            amount_cents: transaction.amount.total,
            currency: &transaction.amount.currency,
            paid_at,
            request_body: &body,
        },
    )
    .await?;
    Ok(())
}

async fn expire_local_attempt(state: &AppState, attempt: &PaymentAttempt) -> Result<(), AppError> {
    let mut conn = state.pool.get().await?;
    conn.transaction::<_, AppError, _>(async move |conn| {
        let locked_attempt = payment_attempts::table
            .filter(payment_attempts::id.eq(attempt.id))
            .for_update()
            .first::<PaymentAttempt>(conn)
            .await?;
        if locked_attempt.state == PaymentAttemptState::Succeeded.as_ref() {
            return Ok(());
        }
        let order = orders::table
            .filter(orders::id.eq(locked_attempt.order_id))
            .for_update()
            .first::<Order>(conn)
            .await?;
        if order.status != OrderStatus::Pending.as_ref() {
            return Ok(());
        }

        if let Some(product_id) = order.product_id {
            let updated = diesel::update(
                products::table
                    .filter(products::id.eq(product_id))
                    .filter(products::status.eq(ProductStatus::Reserved.as_ref())),
            )
            .set(products::status.eq(ProductStatus::Available.as_ref()))
            .execute(conn)
            .await?;
            tracing::info!(
                order_id = %order.id,
                product_id = %product_id,
                inventory_released = updated == 1,
                "released inventory after confirmed WeChat Pay close"
            );
        }
        diesel::update(orders::table.filter(orders::id.eq(order.id)))
            .set((
                orders::status.eq(OrderStatus::Expired.as_ref()),
                orders::product_id.eq(Option::<Uuid>::None),
            ))
            .execute(conn)
            .await?;
        diesel::update(payment_attempts::table.filter(payment_attempts::id.eq(locked_attempt.id)))
            .set((
                payment_attempts::state.eq(PaymentAttemptState::Closed.as_ref()),
                payment_attempts::updated_at.eq(Utc::now()),
            ))
            .execute(conn)
            .await?;
        tracing::info!(
            order_id = %order.id,
            payment_attempt_id = %locked_attempt.id,
            "WeChat Pay attempt expired after remote state confirmation"
        );
        Ok(())
    })
    .await
}
