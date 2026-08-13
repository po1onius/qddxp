use std::collections::{HashMap, HashSet};

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use chrono::{DateTime, Utc};
use diesel::{dsl::count_star, prelude::*};
use diesel_async::{AsyncConnection, RunQueryDsl};
use serde::{Deserialize, Serialize};
use tower_sessions::Session;
use uuid::Uuid;

use crate::{
    AppState,
    db::{
        models::{ApiCallLog, NewProduct, NewProductInfo, Product, ProductInfo},
        schema::{api_call_logs, orders, payment_attempts, product_info, products},
    },
    domain::{OrderStatus, ProductStatus},
    error::AppError,
    http::pagination::{OffsetPageResponse, OffsetPagination, normalize_offset_page},
    security::is_admin_session,
};

#[derive(Debug, Deserialize)]
pub struct CreateProductInfoRequest {
    pub image_base64: Option<String>,
    pub name: String,
    pub details: Option<String>,
    pub price_cents: i64,
    pub active: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct ProductInfoResponse {
    pub id: Uuid,
    pub image_base64: Option<String>,
    pub name: String,
    pub details: String,
    pub price_cents: i64,
    pub sold_count: i64,
    pub active: bool,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateProductInfoActiveRequest {
    pub active: bool,
}

#[derive(Debug, Deserialize)]
pub struct CreateProductRequest {
    pub product_info_id: Uuid,
    pub contents: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct CreateProductResponse {
    pub items: Vec<Product>,
    pub stocked: usize,
}

#[derive(Debug, Deserialize)]
pub struct UpdateProductStatusRequest {
    pub product_ids: Vec<Uuid>,
    pub status: String,
}

#[derive(Debug, Serialize)]
pub struct UpdateProductStatusResponse {
    pub selected: usize,
    pub updated: usize,
    pub ignored: usize,
    pub status: String,
}

#[derive(Debug, Deserialize, Default)]
pub struct AdminProductQuery {
    pub page: Option<i64>,
    pub page_size: Option<i64>,
    pub product_info_id: Option<String>,
    pub status: Option<String>,
}

#[derive(Debug, Serialize, Queryable)]
pub struct AdminProductResponse {
    pub id: Uuid,
    pub product_info_id: Uuid,
    pub product_name: String,
    pub price_cents: i64,
    pub product_info_active: bool,
    pub content: String,
    pub status: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, Queryable)]
pub struct AdminOrderResponse {
    pub id: Uuid,
    pub product_id: Option<Uuid>,
    pub product_info_id: Uuid,
    pub product_name: String,
    pub product_content: Option<String>,
    pub created_at: DateTime<Utc>,
    pub paid_at: Option<DateTime<Utc>>,
    pub status: String,
    pub contact: String,
    pub payment_provider: String,
    pub payment_channel: String,
    pub merchant_trade_no: String,
    pub provider_transaction_id: Option<String>,
    pub payment_state: String,
    pub amount_cents: i64,
    pub currency: String,
}

pub async fn create_product_info(
    State(state): State<AppState>,
    session: Session,
    Json(request): Json<CreateProductInfoRequest>,
) -> Result<(StatusCode, Json<ProductInfoResponse>), AppError> {
    require_admin_for(&session, "create_product_info").await?;
    validate_product_info(&request.name, request.price_cents)?;
    let active = request.active.unwrap_or(true);
    tracing::info!(
        name_len = request.name.trim().chars().count(),
        price_cents = request.price_cents,
        active,
        has_image = request.image_base64.is_some(),
        "admin creating product info"
    );

    let mut conn = state.pool.get().await?;
    let product_info = diesel::insert_into(product_info::table)
        .values(&NewProductInfo {
            id: Uuid::new_v4(),
            image_base64: request.image_base64.as_deref(),
            name: request.name.trim(),
            details: request
                .details
                .as_deref()
                .map(str::trim)
                .unwrap_or_default(),
            price_cents: request.price_cents,
            active,
        })
        .get_result::<ProductInfo>(&mut conn)
        .await?;
    tracing::info!(
        product_info_id = %product_info.id,
        price_cents = product_info.price_cents,
        active = product_info.active,
        "admin created product info"
    );

    Ok((
        StatusCode::CREATED,
        Json(product_info_response(product_info, 0)),
    ))
}

pub async fn list_product_info(
    State(state): State<AppState>,
    session: Session,
) -> Result<Json<Vec<ProductInfoResponse>>, AppError> {
    require_admin_for(&session, "list_product_info").await?;
    tracing::info!("admin listing product info");

    let mut conn = state.pool.get().await?;
    let infos = product_info::table
        .order((product_info::created_at.desc(), product_info::id.desc()))
        .load::<ProductInfo>(&mut conn)
        .await?;
    let product_info_ids = infos.iter().map(|info| info.id).collect::<Vec<_>>();
    let sold_counts = paid_order_counts(&mut conn, &product_info_ids).await?;
    let infos = infos
        .into_iter()
        .map(|info| {
            let sold_count = sold_counts.get(&info.id).copied().unwrap_or(0);
            product_info_response(info, sold_count)
        })
        .collect::<Vec<_>>();
    tracing::info!(returned = infos.len(), "admin listed product info");

    Ok(Json(infos))
}

pub async fn update_product_info_active(
    State(state): State<AppState>,
    session: Session,
    Path(id): Path<Uuid>,
    Json(request): Json<UpdateProductInfoActiveRequest>,
) -> Result<Json<ProductInfoResponse>, AppError> {
    require_admin_for(&session, "update_product_info_active").await?;
    tracing::info!(
        product_info_id = %id,
        active = request.active,
        "admin updating product info active status"
    );

    let mut conn = state.pool.get().await?;
    let product_info = diesel::update(product_info::table.filter(product_info::id.eq(id)))
        .set(product_info::active.eq(request.active))
        .get_result::<ProductInfo>(&mut conn)
        .await?;
    tracing::info!(
        product_info_id = %product_info.id,
        active = product_info.active,
        "admin updated product info active status"
    );

    let sold_count = paid_order_count(&mut conn, product_info.id).await?;
    Ok(Json(product_info_response(product_info, sold_count)))
}

fn product_info_response(info: ProductInfo, sold_count: i64) -> ProductInfoResponse {
    ProductInfoResponse {
        id: info.id,
        image_base64: info.image_base64,
        name: info.name,
        details: info.details,
        price_cents: info.price_cents,
        sold_count,
        active: info.active,
        created_at: info.created_at,
    }
}

async fn paid_order_count(
    conn: &mut diesel_async::AsyncPgConnection,
    product_info_id: Uuid,
) -> Result<i64, AppError> {
    Ok(paid_order_counts(conn, &[product_info_id])
        .await?
        .get(&product_info_id)
        .copied()
        .unwrap_or(0))
}

async fn paid_order_counts(
    conn: &mut diesel_async::AsyncPgConnection,
    product_info_ids: &[Uuid],
) -> Result<HashMap<Uuid, i64>, AppError> {
    if product_info_ids.is_empty() {
        return Ok(HashMap::new());
    }

    let counts = orders::table
        .filter(orders::product_info_id.eq_any(product_info_ids))
        .filter(orders::status.eq(OrderStatus::Paid.as_ref()))
        .group_by(orders::product_info_id)
        .select((orders::product_info_id, count_star()))
        .load::<(Uuid, i64)>(conn)
        .await?;

    Ok(counts.into_iter().collect())
}

pub async fn create_product(
    State(state): State<AppState>,
    session: Session,
    Json(request): Json<CreateProductRequest>,
) -> Result<(StatusCode, Json<CreateProductResponse>), AppError> {
    require_admin_for(&session, "create_product").await?;

    let contents = product_contents(&request)?;
    tracing::info!(
        product_info_id = %request.product_info_id,
        raw_items = request.contents.len(),
        normalized_items = contents.len(),
        "admin creating inventory products"
    );

    let mut conn = state.pool.get().await?;
    let products = conn
        .transaction::<_, AppError, _>(async move |conn| {
            product_info::table
                .filter(product_info::id.eq(request.product_info_id))
                .first::<ProductInfo>(conn)
                .await
                .optional()?
                .ok_or_else(|| AppError::NotFound("product info not found".to_string()))?;

            // 补货只新增可售库存，不再扫描或自动履约任何历史订单。
            let new_products = contents
                .iter()
                .map(|content| NewProduct {
                    id: Uuid::new_v4(),
                    product_info_id: request.product_info_id,
                    content,
                    status: ProductStatus::Available.as_ref(),
                })
                .collect::<Vec<_>>();

            diesel::insert_into(products::table)
                .values(&new_products)
                .get_results::<Product>(conn)
                .await
                .map_err(Into::into)
        })
        .await?;
    let stocked = products.len();
    tracing::info!(
        product_info_id = %request.product_info_id,
        created = products.len(),
        stocked,
        "admin created inventory products"
    );

    Ok((
        StatusCode::CREATED,
        Json(CreateProductResponse {
            items: products,
            stocked,
        }),
    ))
}

pub async fn update_product_status(
    State(state): State<AppState>,
    session: Session,
    Json(request): Json<UpdateProductStatusRequest>,
) -> Result<Json<UpdateProductStatusResponse>, AppError> {
    require_admin_for(&session, "update_product_status").await?;

    let target_status = request
        .status
        .trim()
        .parse::<ProductStatus>()
        .map_err(|_| AppError::BadRequest("status must be available or disabled".to_string()))?;
    if !matches!(
        target_status,
        ProductStatus::Available | ProductStatus::Disabled
    ) {
        tracing::warn!(
            status = target_status.as_ref(),
            "admin update inventory status rejected: unsupported target status"
        );
        return Err(AppError::BadRequest(
            "status must be available or disabled".to_string(),
        ));
    }
    if request.product_ids.is_empty() {
        tracing::warn!("admin update inventory status rejected: empty selection");
        return Err(AppError::BadRequest("product_ids is required".to_string()));
    }

    let mut seen = HashSet::with_capacity(request.product_ids.len());
    let mut product_ids = Vec::with_capacity(request.product_ids.len());
    for product_id in request.product_ids {
        if seen.insert(product_id) {
            product_ids.push(product_id);
        }
    }

    let source_status = match target_status {
        ProductStatus::Available => ProductStatus::Disabled,
        ProductStatus::Disabled => ProductStatus::Available,
        _ => unreachable!("target status already validated"),
    };
    tracing::info!(
        selected = product_ids.len(),
        source_status = source_status.as_ref(),
        target_status = target_status.as_ref(),
        "admin updating inventory product statuses"
    );

    let selected = product_ids.len();
    let mut conn = state.pool.get().await?;
    let updated = diesel::update(
        products::table
            .filter(products::id.eq_any(&product_ids))
            .filter(products::status.eq(source_status.as_ref())),
    )
    .set(products::status.eq(target_status.as_ref()))
    .execute(&mut conn)
    .await?;
    let ignored = selected.saturating_sub(updated);

    tracing::info!(
        selected,
        updated,
        ignored,
        target_status = target_status.as_ref(),
        "admin updated inventory product statuses"
    );
    Ok(Json(UpdateProductStatusResponse {
        selected,
        updated,
        ignored,
        status: target_status.as_ref().to_string(),
    }))
}

pub async fn list_products(
    State(state): State<AppState>,
    session: Session,
    Query(request): Query<AdminProductQuery>,
) -> Result<Json<OffsetPageResponse<AdminProductResponse>>, AppError> {
    require_admin_for(&session, "list_products").await?;
    let pagination = OffsetPagination {
        page: request.page,
        page_size: request.page_size,
    };
    let (page, page_size, offset) = normalize_offset_page(&pagination)?;
    let product_info_id = request
        .product_info_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| {
            Uuid::parse_str(value)
                .map_err(|_| AppError::BadRequest("product_info_id must be a uuid".to_string()))
        })
        .transpose()?;
    let status_filter = request
        .status
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| {
            value
                .parse::<ProductStatus>()
                .map(|status| status.as_ref().to_string())
                .map_err(|_| {
                    AppError::BadRequest(
                        "status must be available, reserved, delivered, or disabled".to_string(),
                    )
                })
        })
        .transpose()?;
    tracing::info!(
        product_info_id = ?product_info_id,
        status = status_filter.as_deref().unwrap_or(""),
        page,
        page_size,
        offset,
        "admin listing inventory products"
    );

    let mut conn = state.pool.get().await?;
    let mut count_query = products::table
        .inner_join(product_info::table.on(product_info::id.eq(products::product_info_id)))
        .into_boxed();
    if let Some(product_info_id) = product_info_id {
        count_query = count_query.filter(products::product_info_id.eq(product_info_id));
    }
    if let Some(status) = status_filter.as_deref() {
        count_query = count_query.filter(products::status.eq(status));
    }
    let total = count_query
        .select(count_star())
        .first::<i64>(&mut conn)
        .await?;

    let mut query = products::table
        .inner_join(product_info::table.on(product_info::id.eq(products::product_info_id)))
        .select((
            products::id,
            products::product_info_id,
            product_info::name,
            product_info::price_cents,
            product_info::active,
            products::content,
            products::status,
            products::created_at,
        ))
        .into_boxed();

    if let Some(product_info_id) = product_info_id {
        query = query.filter(products::product_info_id.eq(product_info_id));
    }
    if let Some(status) = status_filter.as_deref() {
        query = query.filter(products::status.eq(status));
    }

    let products = query
        .order((products::created_at.desc(), products::id.desc()))
        .limit(page_size)
        .offset(offset)
        .load::<AdminProductResponse>(&mut conn)
        .await?;
    tracing::info!(
        returned = products.len(),
        page,
        page_size,
        total,
        "admin listed inventory products"
    );

    Ok(Json(OffsetPageResponse {
        items: products,
        page,
        page_size,
        total,
    }))
}

pub async fn list_orders(
    State(state): State<AppState>,
    session: Session,
    Query(pagination): Query<OffsetPagination>,
) -> Result<Json<OffsetPageResponse<AdminOrderResponse>>, AppError> {
    require_admin_for(&session, "list_orders").await?;
    let (page, page_size, offset) = normalize_offset_page(&pagination)?;
    tracing::info!(page, page_size, offset, "admin listing orders");

    let mut conn = state.pool.get().await?;
    let total = orders::table
        .select(count_star())
        .first::<i64>(&mut conn)
        .await?;

    let orders = orders::table
        .inner_join(payment_attempts::table.on(payment_attempts::order_id.eq(orders::id)))
        .left_join(products::table.on(products::id.nullable().eq(orders::product_id)))
        .select((
            orders::id,
            orders::product_id,
            orders::product_info_id,
            orders::product_name_snapshot,
            products::content.nullable(),
            orders::created_at,
            orders::paid_at,
            orders::status,
            orders::contact,
            payment_attempts::provider,
            payment_attempts::channel,
            payment_attempts::merchant_trade_no,
            payment_attempts::provider_transaction_id,
            payment_attempts::state,
            orders::amount_cents,
            orders::currency,
        ))
        .order((orders::created_at.desc(), orders::id.desc()))
        .limit(page_size)
        .offset(offset)
        .load::<AdminOrderResponse>(&mut conn)
        .await?;
    tracing::info!(
        returned = orders.len(),
        page,
        page_size,
        total,
        "admin listed orders"
    );

    Ok(Json(OffsetPageResponse {
        items: orders,
        page,
        page_size,
        total,
    }))
}

pub async fn list_api_call_logs(
    State(state): State<AppState>,
    session: Session,
    Query(pagination): Query<OffsetPagination>,
) -> Result<Json<OffsetPageResponse<ApiCallLog>>, AppError> {
    require_admin_for(&session, "list_api_call_logs").await?;
    let (page, page_size, offset) = normalize_offset_page(&pagination)?;
    tracing::info!(page, page_size, offset, "admin listing api call logs");

    let mut conn = state.pool.get().await?;
    let total = api_call_logs::table
        .select(count_star())
        .first::<i64>(&mut conn)
        .await?;

    let logs = api_call_logs::table
        .order((api_call_logs::created_at.desc(), api_call_logs::id.desc()))
        .limit(page_size)
        .offset(offset)
        .load::<ApiCallLog>(&mut conn)
        .await?;
    tracing::info!(
        returned = logs.len(),
        page,
        page_size,
        total,
        "admin listed api call logs"
    );

    Ok(Json(OffsetPageResponse {
        items: logs,
        page,
        page_size,
        total,
    }))
}

async fn require_admin_for(session: &Session, action: &'static str) -> Result<(), AppError> {
    if is_admin_session(session).await? {
        tracing::debug!(action, "admin session auth accepted");
        return Ok(());
    }

    tracing::warn!(action, "admin session auth rejected");
    Err(AppError::Unauthorized)
}

fn validate_product_info(name: &str, price_cents: i64) -> Result<(), AppError> {
    if name.trim().is_empty() {
        tracing::warn!("product info validation failed: empty name");
        return Err(AppError::BadRequest("name is required".to_string()));
    }
    if price_cents < 0 {
        tracing::warn!(price_cents, "product info validation failed: invalid price");
        return Err(AppError::BadRequest(
            "price_cents must be greater than or equal to 0".to_string(),
        ));
    }
    Ok(())
}

fn product_contents(request: &CreateProductRequest) -> Result<Vec<String>, AppError> {
    // API 的 contents 已经是完成分隔后的库存列表。这里只清理每项首尾空白，不能再按换行拆分，
    // 否则前端使用自定义分隔符时，单条发货内容中的换行会被误判成新的库存边界。
    let contents = request
        .contents
        .iter()
        .map(|item| item.trim())
        .filter(|item| !item.is_empty())
        .map(ToString::to_string)
        .collect::<Vec<_>>();

    if contents.is_empty() {
        tracing::warn!(
            product_info_id = %request.product_info_id,
            "inventory content validation failed: no normalized content"
        );
        return Err(AppError::BadRequest("contents is required".to_string()));
    }

    Ok(contents)
}
