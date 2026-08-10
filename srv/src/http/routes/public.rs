use std::collections::HashMap;

use axum::{
    Json,
    extract::{Query, State},
};
use chrono::{DateTime, Duration, Utc};
use diesel::{dsl::count_star, prelude::*};
use diesel_async::{AsyncConnection, RunQueryDsl};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::epay;
use crate::{
    AppState,
    db::{
        models::{NewOrder, NewPaymentAttempt, Order, PaymentAttempt, ProductInfo},
        schema::{orders, payment_attempts, product_info, products},
        settings::load_order_allocation_mode,
    },
    domain::{
        OrderAllocationMode, OrderStatus, PaymentAttemptState, PaymentChannel, PaymentProvider,
        ProductStatus, validate_payment_method,
    },
    error::AppError,
    http::pagination::{OffsetPageResponse, OffsetPagination, normalize_offset_page},
    security::{hash_order_password, verify_order_password},
};

#[derive(Debug, Serialize, Queryable)]
pub struct ProductListItem {
    pub id: Uuid,
    pub image_base64: Option<String>,
    pub name: String,
    pub details: String,
    pub price_cents: i64,
    pub sold_count: i64,
    pub stock: i64,
}

#[derive(Debug, Deserialize)]
pub struct CreateOrderRequest {
    pub product_info_id: Uuid,
    pub contact: String,
    pub order_password: String,
    pub payment: PaymentSelectionRequest,
}

#[derive(Debug, Deserialize)]
pub struct PaymentSelectionRequest {
    pub provider: String,
    pub channel: String,
}

#[derive(Debug, Serialize)]
pub struct CreateOrderResponse {
    pub id: Uuid,
    pub status: String,
    pub payment_action: Option<PaymentAction>,
    pub payment_error: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PaymentAction {
    Redirect {
        url: String,
    },
    QrCode {
        content: String,
        expires_at: DateTime<Utc>,
    },
}

#[derive(Debug, Serialize)]
pub struct PaymentMethodResponse {
    pub provider: String,
    pub channel: String,
    pub label: String,
    pub action_type: String,
}

#[derive(Debug, Serialize)]
pub struct OrderAllocationModeResponse {
    pub order_allocation_mode: String,
}

#[derive(Debug, Deserialize)]
pub struct QueryOrderRequest {
    pub id: Uuid,
    pub order_password: String,
}

#[derive(Debug, Deserialize)]
pub struct ListOrdersByContactRequest {
    pub contact: String,
    pub page: Option<i64>,
    pub page_size: Option<i64>,
}

#[derive(Debug, Serialize, Queryable)]
pub struct OrderSummaryResponse {
    pub id: Uuid,
    pub product_info_id: Uuid,
    pub product_name: String,
    pub price_cents: i64,
    pub status: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct OrderDetailResponse {
    pub id: Uuid,
    pub product_info_id: Uuid,
    pub product_name: String,
    pub status: String,
    pub contact: String,
    pub created_at: DateTime<Utc>,
    pub content: Option<String>,
}

pub async fn list_products(
    State(state): State<AppState>,
    Query(pagination): Query<OffsetPagination>,
) -> Result<Json<OffsetPageResponse<ProductListItem>>, AppError> {
    let (page, page_size, offset) = normalize_offset_page(&pagination)?;
    tracing::info!(page, page_size, offset, "listing public products");
    let mut conn = state.pool.get().await?;
    let total = product_info::table
        .filter(product_info::active.eq(true))
        .select(count_star())
        .first::<i64>(&mut conn)
        .await?;

    let product_infos = product_info::table
        .filter(product_info::active.eq(true))
        .order((product_info::created_at.desc(), product_info::id.desc()))
        .limit(page_size)
        .offset(offset)
        .load::<ProductInfo>(&mut conn)
        .await?;
    let product_ids = product_infos
        .iter()
        .map(|product| product.id)
        .collect::<Vec<_>>();
    let stock_counts = available_stock_counts(&mut conn, &product_ids).await?;
    let sold_counts = paid_order_counts(&mut conn, &product_ids).await?;

    let products = product_infos
        .into_iter()
        .map(|product| ProductListItem {
            id: product.id,
            image_base64: product.image_base64,
            name: product.name,
            details: product.details,
            price_cents: product.price_cents,
            sold_count: sold_counts.get(&product.id).copied().unwrap_or(0),
            stock: stock_counts.get(&product.id).copied().unwrap_or(0),
        })
        .collect::<Vec<_>>();

    tracing::info!(
        page,
        page_size,
        total,
        returned = products.len(),
        "listed public products"
    );

    Ok(Json(OffsetPageResponse {
        items: products,
        page,
        page_size,
        total,
    }))
}

pub async fn get_order_allocation_mode(
    State(state): State<AppState>,
) -> Result<Json<OrderAllocationModeResponse>, AppError> {
    let mut conn = state.pool.get().await?;
    let mode = load_order_allocation_mode(&mut conn).await?;

    Ok(Json(OrderAllocationModeResponse {
        order_allocation_mode: mode.as_ref().to_string(),
    }))
}

pub async fn list_payment_methods(
    State(state): State<AppState>,
) -> Json<Vec<PaymentMethodResponse>> {
    let mut methods = Vec::new();
    if state.config.epay.is_some() {
        methods.push(PaymentMethodResponse {
            provider: PaymentProvider::Epay.as_ref().to_string(),
            channel: PaymentChannel::Alipay.as_ref().to_string(),
            label: "支付宝（易支付）".to_string(),
            action_type: "redirect".to_string(),
        });
        methods.push(PaymentMethodResponse {
            provider: PaymentProvider::Epay.as_ref().to_string(),
            channel: PaymentChannel::Wxpay.as_ref().to_string(),
            label: "微信（易支付）".to_string(),
            action_type: "redirect".to_string(),
        });
    }
    if state.wechatpay.is_some() {
        methods.push(PaymentMethodResponse {
            provider: PaymentProvider::Wechatpay.as_ref().to_string(),
            channel: PaymentChannel::Native.as_ref().to_string(),
            label: "微信支付（官方）".to_string(),
            action_type: "qr_code".to_string(),
        });
    }
    tracing::info!(
        returned = methods.len(),
        "listed configured payment methods"
    );
    Json(methods)
}

async fn available_stock_counts(
    conn: &mut diesel_async::AsyncPgConnection,
    product_info_ids: &[Uuid],
) -> Result<HashMap<Uuid, i64>, AppError> {
    if product_info_ids.is_empty() {
        return Ok(HashMap::new());
    }

    let counts = products::table
        .filter(products::product_info_id.eq_any(product_info_ids))
        .filter(products::status.eq(ProductStatus::Available.as_ref()))
        .group_by(products::product_info_id)
        .select((products::product_info_id, count_star()))
        .load::<(Uuid, i64)>(conn)
        .await?;

    Ok(counts.into_iter().collect())
}

async fn paid_order_counts(
    conn: &mut diesel_async::AsyncPgConnection,
    product_info_ids: &[Uuid],
) -> Result<HashMap<Uuid, i64>, AppError> {
    if product_info_ids.is_empty() {
        return Ok(HashMap::new());
    }

    let paid_statuses = [OrderStatus::Paid.as_ref(), OrderStatus::Preorder.as_ref()];
    let counts = orders::table
        .filter(orders::product_info_id.eq_any(product_info_ids))
        .filter(orders::status.eq_any(paid_statuses))
        .group_by(orders::product_info_id)
        .select((orders::product_info_id, count_star()))
        .load::<(Uuid, i64)>(conn)
        .await?;

    Ok(counts.into_iter().collect())
}

pub async fn create_order(
    State(state): State<AppState>,
    Json(request): Json<CreateOrderRequest>,
) -> Result<Json<CreateOrderResponse>, AppError> {
    let contact = request.contact.trim();
    if contact.is_empty() {
        tracing::warn!(
            product_info_id = %request.product_info_id,
            "create order rejected: missing contact"
        );
        return Err(AppError::BadRequest("contact is required".to_string()));
    }

    if request.order_password.len() < 6 {
        tracing::warn!(
            product_info_id = %request.product_info_id,
            contact_len = contact.chars().count(),
            "create order rejected: password too short"
        );
        return Err(AppError::BadRequest(
            "order_password must be at least 6 characters".to_string(),
        ));
    }

    let payment_provider = request
        .payment
        .provider
        .trim()
        .parse::<PaymentProvider>()
        .map_err(|_| AppError::BadRequest("unsupported payment provider".to_string()))?;
    let payment_channel = request
        .payment
        .channel
        .trim()
        .parse::<PaymentChannel>()
        .map_err(|_| AppError::BadRequest("unsupported payment channel".to_string()))?;
    if !validate_payment_method(payment_provider, payment_channel) {
        return Err(AppError::BadRequest(
            "unsupported payment provider/channel combination".to_string(),
        ));
    }
    let provider_configured = match payment_provider {
        PaymentProvider::Epay => state.config.epay.is_some(),
        PaymentProvider::Wechatpay => state.wechatpay.is_some(),
    };
    if !provider_configured {
        return Err(AppError::BadRequest(
            "selected payment provider is not configured".to_string(),
        ));
    }

    let password_hash =
        hash_order_password(&request.order_password, &state.config.order_password_pepper)?;
    let product_info_id = request.product_info_id;
    let payment_expire_minutes = state.config.payment_expire_minutes;
    tracing::info!(
        %product_info_id,
        payment_provider = payment_provider.as_ref(),
        payment_channel = payment_channel.as_ref(),
        contact_len = contact.chars().count(),
        "creating order"
    );

    let mut conn = state.pool.get().await?;
    let transaction_result = conn
        .transaction::<_, AppError, _>(async move |conn| {
            let (product_name, product_price_cents) = product_info::table
                .filter(product_info::id.eq(product_info_id))
                .filter(product_info::active.eq(true))
                .select((product_info::name, product_info::price_cents))
                .first::<(String, i64)>(conn)
                .await
                .optional()?
                .ok_or_else(|| AppError::NotFound("product info not found".to_string()))?;
            tracing::debug!(
                %product_info_id,
                product_price_cents,
                "active product info found for order"
            );

            let allocation_mode = load_order_allocation_mode(conn).await?;
            let product_id = match allocation_mode {
                OrderAllocationMode::ReserveOnCreate => {
                    let product_id = products::table
                        .select(products::id)
                        .filter(products::product_info_id.eq(product_info_id))
                        .filter(products::status.eq(ProductStatus::Available.as_ref()))
                        .for_update()
                        .skip_locked()
                        .first::<Uuid>(conn)
                        .await
                        .optional()?
                        .ok_or_else(|| AppError::Conflict("product is out of stock".to_string()))?;
                    tracing::info!(
                        %product_info_id,
                        %product_id,
                        "available inventory product locked for order"
                    );

                    diesel::update(products::table.filter(products::id.eq(product_id)))
                        .set(products::status.eq(ProductStatus::Reserved.as_ref()))
                        .execute(conn)
                        .await?;

                    Some(product_id)
                }
                OrderAllocationMode::AllocateOnPay => {
                    tracing::info!(
                        %product_info_id,
                        "pay-time allocation order will be created without inventory product id"
                    );
                    None
                }
            };

            let order_id = Uuid::new_v4();
            let payment_attempt_id = Uuid::new_v4();
            // UUID 去掉连字符后恰好为 32 个合法字符，满足微信支付商户订单号限制。
            let merchant_trade_no = payment_attempt_id.simple().to_string();
            let expires_at = Utc::now() + Duration::minutes(payment_expire_minutes);
            let order = diesel::insert_into(orders::table)
                .values(&NewOrder {
                    id: order_id,
                    product_id,
                    product_info_id,
                    product_name_snapshot: &product_name,
                    amount_cents: product_price_cents,
                    currency: "CNY",
                    expires_at,
                    status: OrderStatus::Pending.as_ref(),
                    contact,
                    order_password_hash: &password_hash,
                })
                .get_result::<Order>(conn)
                .await?;
            let payment_attempt: PaymentAttempt = diesel::insert_into(payment_attempts::table)
                .values(&NewPaymentAttempt {
                    id: payment_attempt_id,
                    order_id,
                    provider: payment_provider.as_ref(),
                    channel: payment_channel.as_ref(),
                    merchant_trade_no: &merchant_trade_no,
                    state: PaymentAttemptState::Created.as_ref(),
                    amount_cents: product_price_cents,
                    currency: "CNY",
                    expires_at,
                })
                .get_result(conn)
                .await?;
            tracing::info!(
                order_id = %order.id,
                %product_info_id,
                product_id = ?product_id,
                status = %order.status,
                allocation_mode = %allocation_mode.as_ref(),
                "pending order created"
            );

            Ok((order, payment_attempt))
        })
        .await;
    let (order, payment_attempt) = match transaction_result {
        Ok(value) => value,
        Err(error) => {
            tracing::warn!(
                %product_info_id,
                error = ?error,
                "create order failed"
            );
            return Err(error);
        }
    };

    let (payment_action, payment_error) = match payment_provider {
        PaymentProvider::Epay => {
            let payment_url = epay::build_payment_url(
                state.config.as_ref(),
                order.id,
                &payment_attempt.merchant_trade_no,
                &order.product_name_snapshot,
                order.amount_cents,
                payment_channel.as_ref(),
            );
            (payment_url.map(|url| PaymentAction::Redirect { url }), None)
        }
        PaymentProvider::Wechatpay => {
            let client = state
                .wechatpay
                .as_ref()
                .expect("provider configuration checked before order transaction");
            match client
                .native_prepay(
                    &order.product_name_snapshot,
                    &payment_attempt.merchant_trade_no,
                    order.id,
                    order.amount_cents,
                    payment_attempt.expires_at,
                )
                .await
            {
                Ok(prepay) => {
                    let mut conn = state.pool.get().await?;
                    diesel::update(
                        payment_attempts::table.filter(payment_attempts::id.eq(payment_attempt.id)),
                    )
                    .set((
                        payment_attempts::state.eq(PaymentAttemptState::PrepayCreated.as_ref()),
                        payment_attempts::code_url.eq(Some(prepay.code_url.as_str())),
                        payment_attempts::updated_at.eq(Utc::now()),
                    ))
                    .execute(&mut conn)
                    .await?;
                    (
                        Some(PaymentAction::QrCode {
                            content: prepay.code_url,
                            expires_at: payment_attempt.expires_at,
                        }),
                        None,
                    )
                }
                Err(error) => {
                    tracing::error!(
                        order_id = %order.id,
                        payment_attempt_id = %payment_attempt.id,
                        error = ?error,
                        "official WeChat Pay Native prepay failed"
                    );
                    let mut conn = state.pool.get().await?;
                    diesel::update(
                        payment_attempts::table.filter(payment_attempts::id.eq(payment_attempt.id)),
                    )
                    .set((
                        payment_attempts::state.eq(PaymentAttemptState::Failed.as_ref()),
                        payment_attempts::updated_at.eq(Utc::now()),
                    ))
                    .execute(&mut conn)
                    .await?;
                    (
                        None,
                        Some(
                            "微信支付下单失败，请稍后重新下单；当前订单到期后会自动释放库存"
                                .to_string(),
                        ),
                    )
                }
            }
        }
    };
    tracing::info!(
        order_id = %order.id,
        %product_info_id,
        payment_action_generated = payment_action.is_some(),
        "create order completed"
    );

    Ok(Json(CreateOrderResponse {
        id: order.id,
        status: order.status,
        payment_action,
        payment_error,
    }))
}

pub async fn list_orders_by_contact(
    State(state): State<AppState>,
    Json(request): Json<ListOrdersByContactRequest>,
) -> Result<Json<OffsetPageResponse<OrderSummaryResponse>>, AppError> {
    let contact = request.contact.trim();
    if contact.is_empty() {
        tracing::warn!("list orders by contact rejected: missing contact");
        return Err(AppError::BadRequest("contact is required".to_string()));
    }

    let pagination = OffsetPagination {
        page: request.page,
        page_size: request.page_size,
    };
    let (page, page_size, offset) = normalize_offset_page(&pagination)?;
    tracing::info!(
        contact_len = contact.chars().count(),
        page,
        page_size,
        offset,
        "listing orders by contact"
    );

    let mut conn = state.pool.get().await?;
    let total = orders::table
        .filter(orders::contact.eq(contact))
        .select(count_star())
        .first::<i64>(&mut conn)
        .await?;

    let orders = orders::table
        .filter(orders::contact.eq(contact))
        .select((
            orders::id,
            orders::product_info_id,
            orders::product_name_snapshot,
            orders::amount_cents,
            orders::status,
            orders::created_at,
        ))
        .order((orders::created_at.desc(), orders::id.desc()))
        .limit(page_size)
        .offset(offset)
        .load::<OrderSummaryResponse>(&mut conn)
        .await?;

    tracing::info!(
        returned = orders.len(),
        page,
        page_size,
        total,
        "listed orders by contact"
    );

    Ok(Json(OffsetPageResponse {
        items: orders,
        page,
        page_size,
        total,
    }))
}

pub async fn query_order(
    State(state): State<AppState>,
    Json(request): Json<QueryOrderRequest>,
) -> Result<Json<OrderDetailResponse>, AppError> {
    if request.order_password.is_empty() {
        tracing::warn!(order_id = %request.id, "query order rejected: missing password");
        return Err(AppError::BadRequest(
            "order_password is required".to_string(),
        ));
    }

    let order_id = request.id;
    let password = request.order_password;
    tracing::info!(%order_id, "querying order detail");

    let mut conn = state.pool.get().await?;
    let order = orders::table
        .filter(orders::id.eq(order_id))
        .first::<Order>(&mut conn)
        .await
        .optional()?
        .ok_or_else(|| AppError::NotFound("order not found".to_string()))?;

    if !verify_order_password(
        &password,
        &order.order_password_hash,
        &state.config.order_password_pepper,
    )? {
        tracing::warn!(%order_id, "query order rejected: password verification failed");
        return Err(AppError::Unauthorized);
    }

    let content = match (order.status == OrderStatus::Paid.as_ref(), order.product_id) {
        (true, Some(product_id)) => products::table
            .filter(products::id.eq(product_id))
            .select(products::content)
            .first::<String>(&mut conn)
            .await
            .optional()?,
        _ => None,
    };

    let response = OrderDetailResponse {
        id: order.id,
        product_info_id: order.product_info_id,
        product_name: order.product_name_snapshot,
        status: order.status,
        contact: order.contact,
        created_at: order.created_at,
        content,
    };
    tracing::info!(
        %order_id,
        product_info_id = %response.product_info_id,
        status = %response.status,
        content_released = response.content.is_some(),
        "queried order detail"
    );

    Ok(Json(response))
}
