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

const EPAY_SCAN_INTERVAL_SECONDS: u64 = 5;
const WXPAY_SCAN_INTERVAL_SECONDS: u64 = 60;

/// 分别执行 ePay 本地超时与微信官方远端关单策略。
///
/// ePay 没有商户可控的支付结束时间，到达本地三分钟期限后直接释放库存；微信官方必须
/// 先查单并关单，只有确认无法继续支付后才能释放库存。
pub async fn run(state: AppState) {
    let epay_enabled = state.config.epay.is_some();
    let wechatpay_enabled = state.wechatpay.is_some();
    if !epay_enabled && !wechatpay_enabled {
        tracing::info!("payment expiration worker disabled because no provider is configured");
        return;
    }
    tracing::info!(
        epay_enabled,
        wechatpay_enabled,
        epay_scan_interval_seconds = EPAY_SCAN_INTERVAL_SECONDS,
        wechatpay_scan_interval_seconds = WXPAY_SCAN_INTERVAL_SECONDS,
        "payment expiration worker started"
    );

    let mut epay_interval =
        tokio::time::interval(std::time::Duration::from_secs(EPAY_SCAN_INTERVAL_SECONDS));
    let mut wechatpay_interval =
        tokio::time::interval(std::time::Duration::from_secs(WXPAY_SCAN_INTERVAL_SECONDS));
    epay_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    wechatpay_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            _ = epay_interval.tick(), if epay_enabled => {
                if let Err(error) = process_epay_batch(&state).await {
                    tracing::error!(error = ?error, "ePay expiration batch failed");
                }
            }
            _ = wechatpay_interval.tick(), if wechatpay_enabled => {
                if let Err(error) = process_wechatpay_batch(&state).await {
                    tracing::error!(error = ?error, "WeChat Pay expiration batch failed");
                }
            }
        }
    }
}

async fn process_epay_batch(state: &AppState) -> Result<(), AppError> {
    let mut conn = state.pool.get().await?;
    // 不限制候选数量，确保少量永久失败的旧订单不会持续占据固定批次，阻塞后续订单。
    // 查询完成后立即归还连接，实际过期处理仍逐笔开启短事务，避免长事务锁住整批订单。
    let attempts = payment_attempts::table
        .filter(payment_attempts::provider.eq(PaymentProvider::Epay.as_ref()))
        .filter(payment_attempts::state.eq_any([
            PaymentAttemptState::Created.as_ref(),
            PaymentAttemptState::Ready.as_ref(),
        ]))
        .filter(payment_attempts::expires_at.le(Utc::now()))
        .order(payment_attempts::expires_at.asc())
        .load::<PaymentAttempt>(&mut conn)
        .await?;
    drop(conn);
    if attempts.is_empty() {
        return Ok(());
    }
    tracing::info!(count = attempts.len(), "processing expired ePay attempts");
    for attempt in attempts {
        if let Err(error) = expire_reserved_attempt(state, &attempt, "epay_local_timeout").await {
            tracing::error!(
                payment_attempt_id = %attempt.id,
                order_id = %attempt.order_id,
                error = ?error,
                "failed to expire ePay attempt"
            );
        }
    }
    Ok(())
}

async fn process_wechatpay_batch(state: &AppState) -> Result<(), AppError> {
    let mut conn = state.pool.get().await?;
    // 微信支付同样一次读取全部已过期候选，避免查询或关单永久失败的旧订单挤占固定批次。
    // 远端查单和关单在连接归还后执行，不会在网络请求期间占用数据库连接。
    let attempts = payment_attempts::table
        .filter(payment_attempts::provider.eq(PaymentProvider::Wechatpay.as_ref()))
        .filter(payment_attempts::state.eq_any([
            PaymentAttemptState::Created.as_ref(),
            PaymentAttemptState::Ready.as_ref(),
            PaymentAttemptState::Failed.as_ref(),
        ]))
        .filter(payment_attempts::expires_at.le(Utc::now()))
        .order(payment_attempts::expires_at.asc())
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
        process_wechatpay_attempt(state, attempt).await;
    }
    Ok(())
}

async fn process_wechatpay_attempt(state: &AppState, attempt: PaymentAttempt) {
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
            if let Err(error) =
                expire_reserved_attempt(state, &attempt, "wechatpay_already_closed").await
            {
                tracing::error!(payment_attempt_id = %attempt.id, error = ?error, "failed to expire closed WeChat Pay attempt");
            }
        }
        Ok(transaction) => match client.close_order(&attempt.merchant_trade_no).await {
            Ok(()) => {
                if let Err(error) =
                    expire_reserved_attempt(state, &attempt, "wechatpay_closed").await
                {
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
            if let Err(error) =
                expire_reserved_attempt(state, &attempt, "wechatpay_order_not_exist").await
            {
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
            notifications_enabled: state.telegram.is_some(),
        },
    )
    .await?;
    Ok(())
}

/// 在事务中锁定支付尝试和订单后释放其唯一预占库存。支付确认事务使用相同的锁定顺序，
/// 因此超时与回调并发时只会有一方完成状态迁移。
async fn expire_reserved_attempt(
    state: &AppState,
    attempt: &PaymentAttempt,
    reason: &'static str,
) -> Result<(), AppError> {
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
        if locked_attempt.expires_at > Utc::now() {
            tracing::debug!(
                payment_attempt_id = %locked_attempt.id,
                expires_at = %locked_attempt.expires_at,
                "stale expiration candidate skipped"
            );
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

        let product_id = order.product_id.ok_or_else(|| {
            tracing::error!(
                order_id = %order.id,
                payment_attempt_id = %locked_attempt.id,
                reason,
                "pending order is missing reserved inventory during expiration"
            );
            AppError::Conflict("pending order is missing reserved inventory".to_string())
        })?;
        let updated = diesel::update(
            products::table
                .filter(products::id.eq(product_id))
                .filter(products::product_info_id.eq(order.product_info_id))
                .filter(products::status.eq(ProductStatus::Reserved.as_ref())),
        )
        .set(products::status.eq(ProductStatus::Available.as_ref()))
        .execute(conn)
        .await?;
        if updated != 1 {
            tracing::error!(
                order_id = %order.id,
                payment_attempt_id = %locked_attempt.id,
                %product_id,
                reason,
                updated,
                "reserved inventory could not be released; expiration rolled back"
            );
            return Err(AppError::Conflict(
                "reserved inventory could not be released".to_string(),
            ));
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
            %product_id,
            provider = %locked_attempt.provider,
            reason,
            "payment attempt expired and reserved inventory released"
        );
        Ok(())
    })
    .await
}
