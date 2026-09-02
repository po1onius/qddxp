use std::collections::{HashMap, HashSet};

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use chrono::{DateTime, Utc};
use diesel::result::{DatabaseErrorKind, Error as DieselError};
use diesel::{dsl::count_star, prelude::*};
use diesel_async::{AsyncConnection, RunQueryDsl};
use serde::{Deserialize, Serialize};
use tower_sessions::Session;
use uuid::Uuid;

use crate::{
    AppState,
    db::{
        models::{
            ApiCallLog, NewProduct, NewProductInfo, ProductInfo, STOREFRONT_SETTINGS_ID,
            StorefrontSettings,
        },
        schema::{
            api_call_logs, orders, payment_attempts, product_info, products, storefront_settings,
        },
    },
    domain::{OrderStatus, ProductStatus},
    error::AppError,
    http::pagination::{OffsetPageResponse, OffsetPagination, normalize_offset_page},
    security::is_admin_session,
};

/// 每条发货内容清理首尾空白后必须保留的最少 Unicode 字符数。
///
/// 应用层使用该常量返回清晰的业务错误，数据库迁移中的同值 CHECK 约束则负责阻止
/// 绕过 HTTP 接口的无效写入。两层都按字符而不是 UTF-8 字节计算长度。
const MIN_PRODUCT_CONTENT_CHARS: usize = 4;

/// 管理后台核对库存时允许看到的发货内容比例：总字符数除以该值并向下取整。
const PRODUCT_CONTENT_VISIBLE_DIVISOR: usize = 3;

/// 管理员订单备注允许保存的最大 Unicode 字符数。
///
/// 应用层校验用于返回明确错误，首个 migration 中的 CHECK 约束是最终数据边界；两者
/// 都使用字符数而非 UTF-8 字节数，保证中文、emoji 等内容的计数符合管理员直觉。
const MAX_ORDER_REMARK_CHARS: usize = 1000;

/// 商城公告允许保存的最大 Unicode 字符数，与首个 migration 的 CHECK 约束保持一致。
const MAX_ANNOUNCEMENT_CHARS: usize = 10_000;

#[derive(Debug, Deserialize)]
pub struct UpdateAnnouncementRequest {
    pub announcement: String,
}

#[derive(Debug, Serialize)]
pub struct AnnouncementSettingsResponse {
    pub announcement: String,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct CreateProductInfoRequest {
    pub image_base64: Option<String>,
    pub name: String,
    pub details: Option<String>,
    pub price_cents: i64,
    pub active: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateProductInfoRequest {
    pub image_base64: Option<String>,
    pub name: String,
    pub details: Option<String>,
    pub price_cents: i64,
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
    pub submitted: usize,
    pub stocked: usize,
    pub duplicates: usize,
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
    pub payment_paid_at: Option<DateTime<Utc>>,
    pub status: String,
    pub contact: String,
    pub remark: String,
    pub payment_provider: String,
    pub payment_channel: String,
    pub merchant_trade_no: String,
    pub provider_transaction_id: Option<String>,
    pub payment_state: String,
    pub amount_cents: i64,
    pub currency: String,
}

#[derive(Debug, Deserialize)]
pub struct UpdateOrderRemarkRequest {
    pub remark: String,
}

#[derive(Debug, Serialize, Queryable)]
pub struct UpdateOrderRemarkResponse {
    pub id: Uuid,
    pub remark: String,
}

/// 管理后台只展示发货内容开头三分之一的字符用于核对库存，不能把完整卡密返回给浏览器。
///
/// 这里按 Unicode 字符而不是 UTF-8 字节截取，避免中文、emoji 等多字节内容在字符中间
/// 被切断。可见字符数向下取整；最短内容为四个字符，因此至少会展示一个字符。固定使用
/// 四个星号表示已经隐藏的剩余部分，不暴露原始内容的准确长度。
fn mask_product_content(content: &str) -> String {
    let visible_chars = content.chars().count() / PRODUCT_CONTENT_VISIBLE_DIVISOR;
    let visible_prefix = content.chars().take(visible_chars).collect::<String>();
    format!("{visible_prefix}****")
}

pub async fn get_announcement_settings(
    State(state): State<AppState>,
    session: Session,
) -> Result<Json<AnnouncementSettingsResponse>, AppError> {
    require_admin_for(&session, "get_announcement_settings").await?;
    tracing::info!("admin loading announcement settings");
    let mut conn = state.pool.get().await?;
    let settings = storefront_settings::table
        .find(STOREFRONT_SETTINGS_ID)
        .first::<StorefrontSettings>(&mut conn)
        .await?;
    tracing::info!(
        announcement_chars = settings.announcement.chars().count(),
        announcement_empty = settings.announcement.is_empty(),
        updated_at = %settings.updated_at,
        "admin loaded announcement settings"
    );
    Ok(Json(announcement_settings_response(settings)))
}

pub async fn update_announcement(
    State(state): State<AppState>,
    session: Session,
    Json(request): Json<UpdateAnnouncementRequest>,
) -> Result<Json<AnnouncementSettingsResponse>, AppError> {
    require_admin_for(&session, "update_announcement").await?;
    let announcement = request.announcement.trim();
    let announcement_chars = announcement.chars().count();
    if announcement_chars > MAX_ANNOUNCEMENT_CHARS {
        tracing::warn!(
            announcement_chars,
            max_announcement_chars = MAX_ANNOUNCEMENT_CHARS,
            "admin announcement update rejected: content is too long"
        );
        return Err(AppError::BadRequest(format!(
            "公告内容不能超过 {MAX_ANNOUNCEMENT_CHARS} 个字符"
        )));
    }

    tracing::info!(
        announcement_chars,
        announcement_empty = announcement.is_empty(),
        "admin updating announcement"
    );
    let mut conn = state.pool.get().await?;
    let settings = diesel::update(storefront_settings::table.find(STOREFRONT_SETTINGS_ID))
        .set((
            storefront_settings::announcement.eq(announcement),
            storefront_settings::updated_at.eq(Utc::now()),
        ))
        .get_result::<StorefrontSettings>(&mut conn)
        .await?;
    tracing::info!(
        announcement_chars = settings.announcement.chars().count(),
        announcement_empty = settings.announcement.is_empty(),
        updated_at = %settings.updated_at,
        "admin updated announcement"
    );
    Ok(Json(announcement_settings_response(settings)))
}

fn announcement_settings_response(settings: StorefrontSettings) -> AnnouncementSettingsResponse {
    AnnouncementSettingsResponse {
        announcement: settings.announcement,
        updated_at: settings.updated_at,
    }
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
        .await
        .map_err(map_product_info_write_error)?;
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
    let sold_counts = delivered_order_counts(&mut conn, &product_info_ids).await?;
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

/// 更新商品面向顾客展示的基础信息。
///
/// 名称、图片、详情和价格必须在同一条 SQL 中原子更新。下单事务会锁定同一条
/// `product_info` 记录，因此并发下单只能读取编辑前或编辑后的完整版本，并把当时的
/// 名称与价格保存为订单快照；已经创建的订单及其支付金额不会被本次编辑改变。
pub async fn update_product_info(
    State(state): State<AppState>,
    session: Session,
    Path(id): Path<Uuid>,
    Json(request): Json<UpdateProductInfoRequest>,
) -> Result<Json<ProductInfoResponse>, AppError> {
    require_admin_for(&session, "update_product_info").await?;
    validate_product_info(&request.name, request.price_cents)?;

    let name = request.name.trim();
    let details = request
        .details
        .as_deref()
        .map(str::trim)
        .unwrap_or_default();
    tracing::info!(
        product_info_id = %id,
        name_len = name.chars().count(),
        details_len = details.chars().count(),
        price_cents = request.price_cents,
        has_image = request.image_base64.is_some(),
        "admin updating product info"
    );

    let mut conn = state.pool.get().await?;
    let product_info = diesel::update(product_info::table.filter(product_info::id.eq(id)))
        .set((
            product_info::image_base64.eq(request.image_base64.as_deref()),
            product_info::name.eq(name),
            product_info::details.eq(details),
            product_info::price_cents.eq(request.price_cents),
        ))
        .get_result::<ProductInfo>(&mut conn)
        .await
        .map_err(map_product_info_write_error)?;
    tracing::info!(
        product_info_id = %product_info.id,
        price_cents = product_info.price_cents,
        active = product_info.active,
        "admin updated product info"
    );

    let sold_count = delivered_order_count(&mut conn, product_info.id).await?;
    Ok(Json(product_info_response(product_info, sold_count)))
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

    let sold_count = delivered_order_count(&mut conn, product_info.id).await?;
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

async fn delivered_order_count(
    conn: &mut diesel_async::AsyncPgConnection,
    product_info_id: Uuid,
) -> Result<i64, AppError> {
    Ok(delivered_order_counts(conn, &[product_info_id])
        .await?
        .get(&product_info_id)
        .copied()
        .unwrap_or(0))
}

async fn delivered_order_counts(
    conn: &mut diesel_async::AsyncPgConnection,
    product_info_ids: &[Uuid],
) -> Result<HashMap<Uuid, i64>, AppError> {
    if product_info_ids.is_empty() {
        return Ok(HashMap::new());
    }

    let counts = orders::table
        .filter(orders::product_info_id.eq_any(product_info_ids))
        .filter(orders::status.eq(OrderStatus::Delivered.as_ref()))
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

    let mut contents = product_contents(&request)?;
    let submitted = contents.len();
    // 先在进程内保持原顺序去重，减少无意义的 INSERT；数据库摘要唯一索引仍是最终并发
    // 防线，可以处理不同请求或不同应用实例同时导入相同卡密的竞争。
    let mut unique_contents = HashSet::with_capacity(contents.len());
    contents.retain(|content| unique_contents.insert(content.clone()));
    let request_duplicates = submitted.saturating_sub(contents.len());
    tracing::info!(
        product_info_id = %request.product_info_id,
        raw_items = request.contents.len(),
        normalized_items = submitted,
        unique_items = contents.len(),
        request_duplicates,
        "admin creating inventory products"
    );

    let mut conn = state.pool.get().await?;
    // 调用方只关心实际入库数量，不需要取回刚插入的整行数据。`execute` 直接返回受影响
    // 行数，既避免无用的数据库回传，也确保完整发货内容不会进入 HTTP 响应构造流程。
    let stocked = conn
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
                // 不指定冲突目标，使 PostgreSQL 的卡密摘要唯一索引负责最终裁决。这样两个
                // 管理请求并发导入相同内容时，最多只有一个请求能够创建该条库存。
                .on_conflict_do_nothing()
                .execute(conn)
                .await
                .map_err(Into::into)
        })
        .await?;
    let duplicates = submitted.saturating_sub(stocked);
    tracing::info!(
        product_info_id = %request.product_info_id,
        submitted,
        stocked,
        duplicates,
        "admin created inventory products"
    );

    Ok((
        StatusCode::CREATED,
        Json(CreateProductResponse {
            submitted,
            stocked,
            duplicates,
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

    let mut products = query
        .order((products::created_at.desc(), products::id.desc()))
        .limit(page_size)
        .offset(offset)
        .load::<AdminProductResponse>(&mut conn)
        .await?;
    for product in &mut products {
        product.content = mask_product_content(&product.content);
    }
    tracing::info!(
        returned = products.len(),
        page,
        page_size,
        total,
        content_masked = true,
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

    let mut orders = orders::table
        .inner_join(payment_attempts::table.on(payment_attempts::order_id.eq(orders::id)))
        .left_join(products::table.on(products::id.nullable().eq(orders::product_id)))
        .select((
            orders::id,
            orders::product_id,
            orders::product_info_id,
            orders::product_name_snapshot,
            products::content.nullable(),
            orders::created_at,
            payment_attempts::paid_at,
            orders::status,
            orders::contact,
            orders::remark,
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
    for order in &mut orders {
        order.product_content = order.product_content.as_deref().map(mask_product_content);
    }
    tracing::info!(
        returned = orders.len(),
        page,
        page_size,
        total,
        content_masked = true,
        "admin listed orders"
    );

    Ok(Json(OffsetPageResponse {
        items: orders,
        page,
        page_size,
        total,
    }))
}

/// 修改单张订单的管理员内部备注。
///
/// 备注不会出现在任何顾客端响应中。写入前统一清理首尾空白，同时保留正文内部的换行，
/// 方便管理员记录多行处理信息；日志刻意不包含正文，避免把联系方式等内部信息复制到日志。
pub async fn update_order_remark(
    State(state): State<AppState>,
    session: Session,
    Path(id): Path<Uuid>,
    Json(request): Json<UpdateOrderRemarkRequest>,
) -> Result<Json<UpdateOrderRemarkResponse>, AppError> {
    require_admin_for(&session, "update_order_remark").await?;

    let remark = request.remark.trim();
    let remark_chars = remark.chars().count();
    if remark_chars > MAX_ORDER_REMARK_CHARS {
        tracing::warn!(
            order_id = %id,
            remark_chars,
            max_remark_chars = MAX_ORDER_REMARK_CHARS,
            "admin order remark update rejected: remark too long"
        );
        return Err(AppError::BadRequest(format!(
            "备注不能超过 {MAX_ORDER_REMARK_CHARS} 个字符"
        )));
    }

    tracing::info!(
        order_id = %id,
        remark_chars,
        remark_empty = remark.is_empty(),
        "admin updating order remark"
    );
    let mut conn = state.pool.get().await?;
    let updated = diesel::update(orders::table.filter(orders::id.eq(id)))
        .set(orders::remark.eq(remark))
        .returning((orders::id, orders::remark))
        .get_result::<UpdateOrderRemarkResponse>(&mut conn)
        .await
        .optional()?
        .ok_or_else(|| AppError::NotFound("order not found".to_string()))?;

    tracing::info!(
        order_id = %updated.id,
        remark_chars = updated.remark.chars().count(),
        remark_empty = updated.remark.is_empty(),
        "admin updated order remark"
    );
    Ok(Json(updated))
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

/// 将商品名称唯一约束冲突转换为管理员可以直接理解的业务错误。
///
/// 唯一性必须由数据库约束保证，不能用“先查询再写入”替代，否则两个并发请求仍可能
/// 同时通过查询。其他数据库错误继续交给统一错误处理，避免掩盖真实故障。
fn map_product_info_write_error(error: DieselError) -> AppError {
    if let DieselError::DatabaseError(DatabaseErrorKind::UniqueViolation, information) = &error
        && information.constraint_name() == Some("product_info_name_unique")
    {
        tracing::warn!(
            constraint = "product_info_name_unique",
            "product info write rejected: duplicate name"
        );
        return AppError::Conflict("商品名称已存在".to_string());
    }

    AppError::Database(error)
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

    // 在整批写库前一次性拒绝所有过短内容，避免同一请求只导入部分库存。日志仅记录
    // 数量和长度，不记录卡密原文，既便于定位输入问题，也不会把敏感发货内容写入日志。
    let invalid_content_lengths = contents
        .iter()
        .map(|content| content.chars().count())
        .filter(|length| *length < MIN_PRODUCT_CONTENT_CHARS)
        .collect::<Vec<_>>();
    if !invalid_content_lengths.is_empty() {
        let shortest_content_chars = invalid_content_lengths.iter().copied().min().unwrap_or(0);
        tracing::warn!(
            product_info_id = %request.product_info_id,
            normalized_items = contents.len(),
            invalid_items = invalid_content_lengths.len(),
            shortest_content_chars,
            minimum_content_chars = MIN_PRODUCT_CONTENT_CHARS,
            "inventory content validation failed: content too short"
        );
        return Err(AppError::BadRequest(format!(
            "每条发货内容不得少于 {MIN_PRODUCT_CONTENT_CHARS} 位"
        )));
    }

    Ok(contents)
}
