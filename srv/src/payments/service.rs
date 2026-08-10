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
    domain::{OrderStatus, PaymentAttemptState, PaymentChannel, PaymentProvider, ProductStatus},
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

        let migrated_legacy_success = attempt.provider == PaymentProvider::Epay.as_ref()
            && attempt.channel == PaymentChannel::Legacy.as_ref()
            && attempt.state == PaymentAttemptState::Succeeded.as_ref()
            && attempt.provider_transaction_id.is_none();
        if (attempt.amount_cents != confirmation.amount_cents
            || attempt.currency != confirmation.currency)
            && !migrated_legacy_success
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
        if migrated_legacy_success
            && (attempt.amount_cents != confirmation.amount_cents
                || attempt.currency != confirmation.currency)
        {
            // 旧表没有支付金额快照，只能用迁移时的商品现价回填。已交付订单重放通知时，
            // 商品价格可能早已变化；此处只补全审计信息，绝不再次修改订单或库存。
            tracing::warn!(
                payment_attempt_id = %attempt.id,
                migrated_amount_cents = attempt.amount_cents,
                notified_amount_cents = confirmation.amount_cents,
                "legacy succeeded ePay amount differs from migration snapshot"
            );
        }

        if attempt.state == PaymentAttemptState::Succeeded.as_ref() {
            if let Some(stored_transaction_id) = attempt.provider_transaction_id.as_deref()
                && stored_transaction_id != confirmation.provider_transaction_id
            {
                tracing::error!(
                    payment_attempt_id = %attempt.id,
                    stored_transaction_id = ?attempt.provider_transaction_id,
                    received_transaction_id = confirmation.provider_transaction_id,
                    "payment attempt received a second provider transaction"
                );
                return Err(AppError::Conflict(
                    "payment already applied with another transaction".to_string(),
                ));
            }

            // 旧版订单只保存了 ePay 商户单号，没有保存平台交易号。迁移后的成功记录会在
            // 第一次重放通知时补齐平台交易号；支付尝试已被行锁保护，因此不会并发认领。
            if attempt.provider_transaction_id.is_none() {
                diesel::update(payment_attempts::table.filter(payment_attempts::id.eq(attempt.id)))
                    .set((
                        payment_attempts::provider_transaction_id
                            .eq(Some(confirmation.provider_transaction_id)),
                        payment_attempts::paid_at
                            .eq(Some(attempt.paid_at.unwrap_or(confirmation.paid_at))),
                        payment_attempts::updated_at.eq(Utc::now()),
                    ))
                    .execute(conn)
                    .await?;
                tracing::info!(
                    payment_attempt_id = %attempt.id,
                    provider_transaction_id = confirmation.provider_transaction_id,
                    "legacy succeeded payment attempt enriched from verified notification"
                );
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

        if matches!(
            order.status.as_str(),
            status if status == OrderStatus::Paid.as_ref()
                || status == OrderStatus::Preorder.as_ref()
        ) {
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

        deliver_order_inventory(conn, &order, confirmation.paid_at).await?;
        diesel::update(payment_attempts::table.filter(payment_attempts::id.eq(attempt.id)))
            .set((
                payment_attempts::provider_transaction_id
                    .eq(Some(confirmation.provider_transaction_id)),
                payment_attempts::state.eq(PaymentAttemptState::Succeeded.as_ref()),
                payment_attempts::paid_at.eq(Some(confirmation.paid_at)),
                payment_attempts::updated_at.eq(Utc::now()),
            ))
            .execute(conn)
            .await?;
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
    let reserved_product = if let Some(product_id) = order.product_id {
        products::table
            .select((products::id, products::status))
            .filter(products::id.eq(product_id))
            .filter(products::product_info_id.eq(order.product_info_id))
            .for_update()
            .first::<(Uuid, String)>(conn)
            .await
            .optional()?
    } else {
        None
    };

    if let Some((product_id, product_status)) = reserved_product {
        if product_status != ProductStatus::Reserved.as_ref() {
            return Err(AppError::Conflict(
                "reserved product is not available".to_string(),
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
            product_id = %product_id,
            "reserved inventory delivered"
        );
        return Ok(());
    }

    let product_id = products::table
        .select(products::id)
        .filter(products::product_info_id.eq(order.product_info_id))
        .filter(products::status.eq(ProductStatus::Available.as_ref()))
        .for_update()
        .skip_locked()
        .first::<Uuid>(conn)
        .await
        .optional()?;

    if let Some(product_id) = product_id {
        diesel::update(orders::table.filter(orders::id.eq(order.id)))
            .set((
                orders::status.eq(OrderStatus::Paid.as_ref()),
                orders::paid_at.eq(Some(paid_at)),
                orders::product_id.eq(Some(product_id)),
            ))
            .execute(conn)
            .await?;
        diesel::update(products::table.filter(products::id.eq(product_id)))
            .set(products::status.eq(ProductStatus::Delivered.as_ref()))
            .execute(conn)
            .await?;
        tracing::info!(
            order_id = %order.id,
            product_id = %product_id,
            "available inventory allocated and delivered"
        );
    } else {
        let preorder_product_id = Uuid::new_v4();
        diesel::update(orders::table.filter(orders::id.eq(order.id)))
            .set((
                orders::status.eq(OrderStatus::Preorder.as_ref()),
                orders::paid_at.eq(Some(paid_at)),
                orders::product_id.eq(Some(preorder_product_id)),
            ))
            .execute(conn)
            .await?;
        tracing::info!(
            order_id = %order.id,
            product_id = %preorder_product_id,
            "paid order moved to preorder because inventory is unavailable"
        );
    }
    Ok(())
}
