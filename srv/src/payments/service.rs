use chrono::{DateTime, Utc};
use diesel::prelude::*;
use diesel_async::{AsyncConnection, RunQueryDsl};
use uuid::Uuid;

use crate::{
    db::{
        models::{NewPaymentEvent, Order, PaymentAttempt},
        pool::DbPool,
        schema::{orders, payment_attempts, payment_events, products},
    },
    domain::{OrderStatus, PaymentAttemptState, PaymentProvider, ProductStatus},
    error::AppError,
};

/// 已经由具体支付协议完成验签、解密和商户身份校验后的统一收款事实。
/// 领域服务只接受该结构，避免业务层接触不同渠道的原始、不可信报文。
pub struct PaymentConfirmation<'a> {
    pub provider: &'a str,
    pub provider_event_id: &'a str,
    pub event_type: &'a str,
    pub merchant_trade_no: &'a str,
    pub provider_transaction_id: &'a str,
    pub expected_order_id: Option<Uuid>,
    pub amount_cents: i64,
    pub currency: &'a str,
    pub paid_at: DateTime<Utc>,
    pub request_body: &'a str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmPaymentOutcome {
    Applied,
    /// 支付发生在库存预占超时之后，仅记录支付事实，不交付库存。
    RecordedAfterExpiry,
    AlreadyApplied,
}

/// 在单个数据库事务中完成幂等校验、订单落账与库存交付。
///
/// 支付回调和主动查单都必须调用此函数，保证任何入口都不会重复发货。
pub async fn confirm_payment(
    pool: &DbPool,
    confirmation: PaymentConfirmation<'_>,
) -> Result<ConfirmPaymentOutcome, AppError> {
    let mut conn = pool.get().await?;
    conn.transaction::<_, AppError, _>(async move |conn| {
        tracing::debug!(
            provider = confirmation.provider,
            merchant_trade_no = confirmation.merchant_trade_no,
            provider_event_id = confirmation.provider_event_id,
            "locking payment attempt for trusted payment confirmation"
        );

        let attempt = payment_attempts::table
            .filter(payment_attempts::provider.eq(confirmation.provider))
            .filter(payment_attempts::merchant_trade_no.eq(confirmation.merchant_trade_no))
            .for_update()
            .first::<PaymentAttempt>(conn)
            .await
            .optional()?
            .ok_or_else(|| AppError::NotFound("payment attempt not found".to_string()))?;

        if let Some(expected_order_id) = confirmation.expected_order_id
            && expected_order_id != attempt.order_id
        {
            tracing::warn!(
                expected_order_id = %expected_order_id,
                actual_order_id = %attempt.order_id,
                payment_attempt_id = %attempt.id,
                "trusted payment rejected: order id mismatch"
            );
            return Err(AppError::BadRequest("payment order mismatch".to_string()));
        }

        let existing_event_attempt_id = payment_events::table
            .filter(payment_events::provider.eq(confirmation.provider))
            .filter(payment_events::provider_event_id.eq(confirmation.provider_event_id))
            .select(payment_events::payment_attempt_id)
            .first::<Uuid>(conn)
            .await
            .optional()?;
        if let Some(existing_attempt_id) = existing_event_attempt_id {
            if existing_attempt_id != attempt.id {
                tracing::error!(
                    payment_attempt_id = %attempt.id,
                    existing_attempt_id = %existing_attempt_id,
                    provider_event_id = confirmation.provider_event_id,
                    "payment event id is already associated with another attempt"
                );
                return Err(AppError::Conflict(
                    "payment event belongs to another attempt".to_string(),
                ));
            }
            tracing::info!(
                payment_attempt_id = %attempt.id,
                provider_event_id = confirmation.provider_event_id,
                "duplicate payment event ignored"
            );
            return Ok(ConfirmPaymentOutcome::AlreadyApplied);
        }

        if attempt.amount_cents != confirmation.amount_cents
            || attempt.currency != confirmation.currency
        {
            tracing::warn!(
                payment_attempt_id = %attempt.id,
                expected_amount_cents = attempt.amount_cents,
                actual_amount_cents = confirmation.amount_cents,
                expected_currency = %attempt.currency,
                actual_currency = confirmation.currency,
                "trusted payment rejected: immutable amount snapshot mismatch"
            );
            return Err(AppError::BadRequest("payment amount mismatch".to_string()));
        }

        if attempt.state == PaymentAttemptState::Succeeded.as_ref() {
            let stored_transaction_id =
                attempt.provider_transaction_id.as_deref().ok_or_else(|| {
                    tracing::error!(
                        payment_attempt_id = %attempt.id,
                        "succeeded payment attempt is missing provider transaction id"
                    );
                    AppError::Conflict(
                        "succeeded payment is missing provider transaction id".to_string(),
                    )
                })?;
            if stored_transaction_id != confirmation.provider_transaction_id {
                tracing::error!(
                    payment_attempt_id = %attempt.id,
                    stored_transaction_id,
                    received_transaction_id = confirmation.provider_transaction_id,
                    "payment attempt received a second provider transaction"
                );
                return Err(AppError::Conflict(
                    "payment already applied with another transaction".to_string(),
                ));
            }

            insert_payment_event(conn, &attempt, &confirmation).await?;
            return Ok(ConfirmPaymentOutcome::AlreadyApplied);
        }

        let order = orders::table
            .filter(orders::id.eq(attempt.order_id))
            .for_update()
            .first::<Order>(conn)
            .await?;
        if order.amount_cents != confirmation.amount_cents
            || order.currency != confirmation.currency
        {
            tracing::error!(
                order_id = %order.id,
                payment_attempt_id = %attempt.id,
                order_amount_cents = order.amount_cents,
                attempt_amount_cents = attempt.amount_cents,
                "order and payment attempt snapshots are inconsistent"
            );
            return Err(AppError::Conflict(
                "order payment snapshot mismatch".to_string(),
            ));
        }

        let epay_paid_after_deadline = is_epay_paid_after_deadline(
            &attempt.provider,
            confirmation.paid_at,
            attempt.expires_at,
        );
        if epay_paid_after_deadline && order.status == OrderStatus::Pending.as_ref() {
            release_reserved_inventory_for_late_payment(conn, &order, attempt.id).await?;
        }

        if order.status == OrderStatus::Expired.as_ref() || epay_paid_after_deadline {
            // 超时任务已经释放了库存。此后即使收到验签成功的支付通知，也只能记录收款事实；
            // 禁止重新分配库存，否则同一条卡密可能已被另一张订单预占或交付。
            diesel::update(orders::table.filter(orders::id.eq(order.id)))
                .set(orders::paid_at.eq(Some(confirmation.paid_at)))
                .execute(conn)
                .await?;
            mark_attempt_succeeded(conn, attempt.id, &confirmation).await?;
            insert_payment_event(conn, &attempt, &confirmation).await?;
            tracing::error!(
                order_id = %order.id,
                payment_attempt_id = %attempt.id,
                provider = confirmation.provider,
                provider_transaction_id = confirmation.provider_transaction_id,
                paid_at = %confirmation.paid_at,
                "payment received after inventory reservation expired; payment recorded without delivery"
            );
            return Ok(ConfirmPaymentOutcome::RecordedAfterExpiry);
        }

        if order.status == OrderStatus::Paid.as_ref() {
            tracing::error!(
                order_id = %order.id,
                payment_attempt_id = %attempt.id,
                order_status = %order.status,
                "paid order references a non-succeeded payment attempt"
            );
            return Err(AppError::Conflict(
                "order already paid by another payment attempt".to_string(),
            ));
        }
        if order.status != OrderStatus::Pending.as_ref() {
            tracing::error!(
                order_id = %order.id,
                payment_attempt_id = %attempt.id,
                order_status = %order.status,
                "payment confirmation rejected for terminal order"
            );
            return Err(AppError::Conflict(
                "order cannot accept payment in its current state".to_string(),
            ));
        }

        deliver_order_inventory(conn, &order, confirmation.paid_at).await?;
        mark_attempt_succeeded(conn, attempt.id, &confirmation).await?;
        insert_payment_event(conn, &attempt, &confirmation).await?;

        tracing::info!(
            order_id = %order.id,
            payment_attempt_id = %attempt.id,
            provider = confirmation.provider,
            provider_transaction_id = confirmation.provider_transaction_id,
            amount_cents = confirmation.amount_cents,
            "trusted payment confirmation applied"
        );
        Ok(ConfirmPaymentOutcome::Applied)
    })
    .await
}

fn is_epay_paid_after_deadline(
    provider: &str,
    paid_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
) -> bool {
    provider == PaymentProvider::Epay.as_ref() && paid_at > expires_at
}

/// ePay 回调可能在三分钟截止后、后台扫描任务执行前到达。此时回调事务直接释放库存并
/// 将订单置为过期，确保业务期限不受扫描间隔影响。
async fn release_reserved_inventory_for_late_payment(
    conn: &mut diesel_async::AsyncPgConnection,
    order: &Order,
    payment_attempt_id: Uuid,
) -> Result<(), AppError> {
    let product_id = order.product_id.ok_or_else(|| {
        tracing::error!(
            order_id = %order.id,
            %payment_attempt_id,
            "late ePay payment found pending order without reserved inventory"
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
            %payment_attempt_id,
            %product_id,
            updated,
            "late ePay payment could not release reserved inventory"
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
    tracing::warn!(
        order_id = %order.id,
        %payment_attempt_id,
        %product_id,
        "ePay payment arrived after deadline; reserved inventory released in callback transaction"
    );
    Ok(())
}

async fn mark_attempt_succeeded(
    conn: &mut diesel_async::AsyncPgConnection,
    attempt_id: Uuid,
    confirmation: &PaymentConfirmation<'_>,
) -> Result<(), AppError> {
    diesel::update(payment_attempts::table.filter(payment_attempts::id.eq(attempt_id)))
        .set((
            payment_attempts::provider_transaction_id
                .eq(Some(confirmation.provider_transaction_id)),
            payment_attempts::state.eq(PaymentAttemptState::Succeeded.as_ref()),
            payment_attempts::paid_at.eq(Some(confirmation.paid_at)),
            payment_attempts::updated_at.eq(Utc::now()),
        ))
        .execute(conn)
        .await?;
    Ok(())
}

async fn insert_payment_event(
    conn: &mut diesel_async::AsyncPgConnection,
    attempt: &PaymentAttempt,
    confirmation: &PaymentConfirmation<'_>,
) -> Result<(), AppError> {
    diesel::insert_into(payment_events::table)
        .values(&NewPaymentEvent {
            id: Uuid::new_v4(),
            provider: confirmation.provider,
            provider_event_id: confirmation.provider_event_id,
            payment_attempt_id: attempt.id,
            event_type: confirmation.event_type,
            request_body: confirmation.request_body,
            success: true,
            error_message: None,
        })
        .execute(conn)
        .await?;
    Ok(())
}

async fn deliver_order_inventory(
    conn: &mut diesel_async::AsyncPgConnection,
    order: &Order,
    paid_at: DateTime<Utc>,
) -> Result<(), AppError> {
    let product_id = order.product_id.ok_or_else(|| {
        tracing::error!(order_id = %order.id, "pending order is missing reserved inventory id");
        AppError::Conflict("pending order is missing reserved inventory".to_string())
    })?;
    let product_status = products::table
        .select(products::status)
        .filter(products::id.eq(product_id))
        .filter(products::product_info_id.eq(order.product_info_id))
        .for_update()
        .first::<String>(conn)
        .await
        .optional()?
        .ok_or_else(|| {
            tracing::error!(order_id = %order.id, %product_id, "reserved inventory record is missing");
            AppError::Conflict("reserved inventory record is missing".to_string())
        })?;
    if product_status != ProductStatus::Reserved.as_ref() {
        tracing::error!(
            order_id = %order.id,
            %product_id,
            %product_status,
            "reserved inventory is not in reserved state"
        );
        return Err(AppError::Conflict(
            "reserved inventory is not in reserved state".to_string(),
        ));
    }

    diesel::update(orders::table.filter(orders::id.eq(order.id)))
        .set((
            orders::status.eq(OrderStatus::Paid.as_ref()),
            orders::paid_at.eq(Some(paid_at)),
        ))
        .execute(conn)
        .await?;
    diesel::update(products::table.filter(products::id.eq(product_id)))
        .set(products::status.eq(ProductStatus::Delivered.as_ref()))
        .execute(conn)
        .await?;
    tracing::info!(
        order_id = %order.id,
        %product_id,
        "reserved inventory delivered"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use chrono::Duration;

    use super::*;

    #[test]
    fn only_epay_confirmed_after_deadline_is_locally_classified_as_late() {
        let expires_at = Utc::now();

        assert!(is_epay_paid_after_deadline(
            PaymentProvider::Epay.as_ref(),
            expires_at + Duration::milliseconds(1),
            expires_at,
        ));
        assert!(!is_epay_paid_after_deadline(
            PaymentProvider::Epay.as_ref(),
            expires_at,
            expires_at,
        ));
        assert!(!is_epay_paid_after_deadline(
            PaymentProvider::Wechatpay.as_ref(),
            expires_at + Duration::minutes(1),
            expires_at,
        ));
    }
}
