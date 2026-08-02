use std::collections::HashMap;

use axum::{
    Json,
    extract::{Query, State},
};
use chrono::{DateTime, Utc};
use diesel::{dsl::count_star, prelude::*};
use diesel_async::{AsyncConnection, RunQueryDsl};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::epay;
use crate::{
    AppState,
    db::{
        models::{NewOrder, Order, ProductInfo},
        schema::{orders, product_info, products},
        settings::load_order_allocation_mode,
    },
    domain::{OrderAllocationMode, OrderStatus, PaymentType, ProductStatus},
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
    pub payment_type: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct CreateOrderResponse {
    pub id: Uuid,
    pub status: String,
    pub payment_url: Option<String>,
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
    let contact = request.contact.trim().to_string();
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

    let payment_type = request
        .payment_type
        .as_deref()
        .unwrap_or(PaymentType::Alipay.as_ref())
        .parse::<PaymentType>()
        .map_err(|_| AppError::BadRequest("unsupported payment_type".to_string()))?;

    let password_hash =
        hash_order_password(&request.order_password, &state.config.order_password_pepper)?;
    let product_info_id = request.product_info_id;
    tracing::info!(
        %product_info_id,
        payment_type = payment_type.as_ref(),
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
            let epay_trade_no = epay::build_trade_no(order_id);
            let order = diesel::insert_into(orders::table)
                .values(&NewOrder {
                    id: order_id,
                    epay_trade_no: &epay_trade_no,
                    product_id,
                    product_info_id,
                    status: OrderStatus::Pending.as_ref(),
                    contact: &contact,
                    order_password_hash: &password_hash,
                })
                .get_result::<Order>(conn)
                .await?;
            tracing::info!(
                order_id = %order.id,
                %product_info_id,
                product_id = ?product_id,
                status = %order.status,
                allocation_mode = %allocation_mode.as_ref(),
                "pending order created"
            );

            Ok((order, product_name, product_price_cents))
        })
        .await;
    let (order, product_name, product_price_cents) = match transaction_result {
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

    let payment_url = epay::build_payment_url(
        state.config.as_ref(),
        order.id,
        &order.epay_trade_no,
        &product_name,
        product_price_cents,
        payment_type.as_ref(),
    );
    tracing::info!(
        order_id = %order.id,
        %product_info_id,
        payment_url_generated = payment_url.is_some(),
        "create order completed"
    );

    Ok(Json(CreateOrderResponse {
        id: order.id,
        status: order.status,
        payment_url,
    }))
}

pub async fn list_orders_by_contact(
    State(state): State<AppState>,
    Json(request): Json<ListOrdersByContactRequest>,
) -> Result<Json<OffsetPageResponse<OrderSummaryResponse>>, AppError> {
    let contact = request.contact.trim().to_string();
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
        .inner_join(product_info::table.on(product_info::id.eq(orders::product_info_id)))
        .filter(orders::contact.eq(contact.as_str()))
        .select(count_star())
        .first::<i64>(&mut conn)
        .await?;

    let orders = orders::table
        .inner_join(product_info::table.on(product_info::id.eq(orders::product_info_id)))
        .filter(orders::contact.eq(contact.as_str()))
        .select((
            orders::id,
            orders::product_info_id,
            product_info::name,
            product_info::price_cents,
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

    let product_name = product_info::table
        .filter(product_info::id.eq(order.product_info_id))
        .select(product_info::name)
        .first::<String>(&mut conn)
        .await?;

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
        product_name,
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
