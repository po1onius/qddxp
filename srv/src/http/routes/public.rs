use std::collections::HashMap;

use axum::{
    Json,
    extract::{Path, Query, State},
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
    },
    domain::{
        OrderStatus, PaymentAttemptState, PaymentChannel, PaymentProvider, ProductStatus,
        validate_payment_method,
    },
    error::AppError,
    http::pagination::{OffsetPageResponse, OffsetPagination, normalize_offset_page},
    notifications,
    payments::EPAY_RESERVATION_MINUTES,
    security::{hash_order_password, verify_order_password},
};

/// 联系方式会直接写入订单并用于精确查单，限制长度可以避免异常大输入进入日志、数据库和索引。
/// 使用 Unicode 字符数而不是 UTF-8 字节数，确保中文等多字节字符仍按一个字符计算。
const CONTACT_MAX_LENGTH: usize = 50;

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

#[derive(Debug, Serialize)]
pub struct StorefrontResponse {
    pub shop_name: String,
    pub logo_url: &'static str,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PublishPaymentAttemptOutcome {
    Ready,
    AlreadyDelivered,
    Unavailable,
}

#[derive(Debug, Serialize)]
pub struct PaymentMethodResponse {
    pub provider: String,
    pub channel: String,
    pub label: String,
    pub action_type: String,
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
    pub payment_paid_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct OrderDetailResponse {
    pub id: Uuid,
    pub product_info_id: Uuid,
    pub product_name: String,
    pub status: String,
    pub payment_paid_at: Option<DateTime<Utc>>,
    pub contact: String,
    pub created_at: DateTime<Utc>,
    pub content: Option<String>,
}

pub async fn get_storefront(State(state): State<AppState>) -> Json<StorefrontResponse> {
    tracing::debug!(shop_name = %state.config.shop_name, "getting public storefront configuration");
    Json(StorefrontResponse {
        shop_name: state.config.shop_name.clone(),
        logo_url: "/api/storefront/logo",
    })
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
    let sold_counts = delivered_order_counts(&mut conn, &product_ids).await?;

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

/// 获取创建订单页面所需的单个在售商品快照。
///
/// 创建订单是独立前端路由，浏览器刷新后不能再依赖商品列表页保存在内存中的对象，因此
/// 这里按商品定义 ID 重新查询名称、价格、详情以及实时库存。已下架商品对公共端表现为不存在。
pub async fn get_product(
    State(state): State<AppState>,
    Path(product_id): Path<Uuid>,
) -> Result<Json<ProductListItem>, AppError> {
    tracing::info!(%product_id, "getting public product for order creation");
    let mut conn = state.pool.get().await?;
    let product = product_info::table
        .filter(product_info::id.eq(product_id))
        .filter(product_info::active.eq(true))
        .first::<ProductInfo>(&mut conn)
        .await
        .optional()?
        .ok_or_else(|| {
            tracing::warn!(%product_id, "public product not found or inactive");
            AppError::NotFound("product info not found".to_string())
        })?;

    let product_ids = [product.id];
    let stock_counts = available_stock_counts(&mut conn, &product_ids).await?;
    let sold_counts = delivered_order_counts(&mut conn, &product_ids).await?;
    let response = ProductListItem {
        id: product.id,
        image_base64: product.image_base64,
        name: product.name,
        details: product.details,
        price_cents: product.price_cents,
        sold_count: sold_counts.get(&product.id).copied().unwrap_or(0),
        stock: stock_counts.get(&product.id).copied().unwrap_or(0),
    };
    tracing::info!(
        %product_id,
        stock = response.stock,
        sold_count = response.sold_count,
        "got public product for order creation"
    );

    Ok(Json(response))
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

pub async fn create_order(
    State(state): State<AppState>,
    Json(request): Json<CreateOrderRequest>,
) -> Result<Json<CreateOrderResponse>, AppError> {
    let contact = request.contact.trim();
    let contact_len = contact.chars().count();
    if contact.is_empty() {
        tracing::warn!(
            product_info_id = %request.product_info_id,
            "create order rejected: missing contact"
        );
        return Err(AppError::BadRequest("contact is required".to_string()));
    }
    if contact_len > CONTACT_MAX_LENGTH {
        tracing::warn!(
            product_info_id = %request.product_info_id,
            contact_len,
            contact_max_length = CONTACT_MAX_LENGTH,
            "create order rejected: contact is too long"
        );
        return Err(AppError::BadRequest(format!(
            "contact must be at most {CONTACT_MAX_LENGTH} characters"
        )));
    }

    if request.order_password.len() < 6 {
        tracing::warn!(
            product_info_id = %request.product_info_id,
            contact_len,
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
    let reservation_minutes = match payment_provider {
        PaymentProvider::Epay => EPAY_RESERVATION_MINUTES,
        PaymentProvider::Wechatpay => state.config.wechatpay_expire_minutes,
    };
    tracing::info!(
        %product_info_id,
        payment_provider = payment_provider.as_ref(),
        payment_channel = payment_channel.as_ref(),
        contact_len,
        "creating order"
    );

    let mut conn = state.pool.get().await?;
    let transaction_result = conn
        .transaction::<_, AppError, _>(async move |conn| {
            // 锁定商品定义直到订单事务提交，使下单与后台上下架操作严格串行：
            // 下单先取得锁时，下架必须等待订单落库；下架先提交时，本查询无法再命中
            // active=true。微信等远端支付准备发生在事务外，因此不会长期持有该行锁。
            let (product_name, product_price_cents) = product_info::table
                .filter(product_info::id.eq(product_info_id))
                .filter(product_info::active.eq(true))
                .select((product_info::name, product_info::price_cents))
                .for_update()
                .first::<(String, i64)>(conn)
                .await
                .optional()?
                .ok_or_else(|| AppError::NotFound("product info not found".to_string()))?;
            tracing::debug!(
                %product_info_id,
                product_price_cents,
                "active product info found for order"
            );

            // 所有订单都必须在创建事务内锁定一条真实库存；缺货时不创建订单，更不接受预购。
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

            let order_id = Uuid::new_v4();
            let payment_attempt_id = Uuid::new_v4();
            // UUID 去掉连字符后恰好为 32 个合法字符，满足微信支付商户订单号限制。
            let merchant_trade_no = payment_attempt_id.simple().to_string();
            let expires_at = Utc::now() + Duration::minutes(reservation_minutes);
            let order = diesel::insert_into(orders::table)
                .values(&NewOrder {
                    id: order_id,
                    product_id: Some(product_id),
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
                })
                .get_result(conn)
                .await?;
            tracing::info!(
                order_id = %order.id,
                %product_info_id,
                %product_id,
                status = %order.status,
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
    // Telegram 是非关键旁路：只有订单事务成功提交后才异步发送一次，发送结果不影响订单。
    notifications::notify_order_created(state.telegram.clone(), &order, &payment_attempt);

    let mut response_status = order.status.clone();
    let (payment_action, payment_error) = match payment_provider {
        PaymentProvider::Epay => {
            match publish_payment_attempt_ready(&state, &payment_attempt, None).await? {
                PublishPaymentAttemptOutcome::Ready => {
                    let payment_url = epay::build_payment_url(
                        state.config.as_ref(),
                        order.id,
                        &payment_attempt.merchant_trade_no,
                        &order.product_name_snapshot,
                        order.amount_cents,
                        payment_channel.as_ref(),
                    )
                    .expect("ePay configuration was checked before creating the order");
                    tracing::info!(
                        order_id = %order.id,
                        payment_attempt_id = %payment_attempt.id,
                        provider = payment_provider.as_ref(),
                        channel = payment_channel.as_ref(),
                        "ePay payment attempt is ready for redirect"
                    );
                    (Some(PaymentAction::Redirect { url: payment_url }), None)
                }
                PublishPaymentAttemptOutcome::AlreadyDelivered => {
                    response_status = OrderStatus::Delivered.as_ref().to_string();
                    (None, None)
                }
                PublishPaymentAttemptOutcome::Unavailable => (
                    None,
                    Some("订单已过期，支付入口未生成，请重新下单".to_string()),
                ),
            }
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
                    order.expires_at,
                )
                .await
            {
                Ok(prepay) => {
                    match publish_payment_attempt_ready(
                        &state,
                        &payment_attempt,
                        Some(&prepay.code_url),
                    )
                    .await?
                    {
                        PublishPaymentAttemptOutcome::Ready => (
                            Some(PaymentAction::QrCode {
                                content: prepay.code_url,
                                expires_at: order.expires_at,
                            }),
                            None,
                        ),
                        PublishPaymentAttemptOutcome::AlreadyDelivered => {
                            response_status = OrderStatus::Delivered.as_ref().to_string();
                            (None, None)
                        }
                        PublishPaymentAttemptOutcome::Unavailable => {
                            // 微信远端下单已经成功，但本地订单可能在网络请求期间到期并释放
                            // 库存。此时立即关单，禁止把仍可扫码的二维码暴露给用户。
                            if let Err(error) =
                                client.close_order(&payment_attempt.merchant_trade_no).await
                            {
                                tracing::error!(
                                    order_id = %order.id,
                                    payment_attempt_id = %payment_attempt.id,
                                    error = ?error,
                                    "failed to close unpublished WeChat Pay order"
                                );
                            }
                            (
                                None,
                                Some("订单已过期，微信支付入口已关闭，请重新下单".to_string()),
                            )
                        }
                    }
                }
                Err(error) => {
                    tracing::error!(
                        order_id = %order.id,
                        payment_attempt_id = %payment_attempt.id,
                        error = ?error,
                        "official WeChat Pay Native prepay failed"
                    );
                    mark_payment_attempt_failed_if_created(&state, payment_attempt.id).await?;
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
        status: response_status,
        payment_action,
        payment_error,
    }))
}

/// 支付方准备工作完成后，按超时 worker 相同的“支付尝试 → 订单”顺序加锁并发布入口。
/// 只有尚未到期的 created 尝试和 pending 订单能够进入 ready，杜绝超时释放后被请求线程
/// 重新写回 ready 的状态复活竞争。
async fn publish_payment_attempt_ready(
    state: &AppState,
    attempt: &PaymentAttempt,
    code_url: Option<&str>,
) -> Result<PublishPaymentAttemptOutcome, AppError> {
    let mut conn = state.pool.get().await?;
    conn.transaction::<_, AppError, _>(async move |conn| {
        let locked_attempt = payment_attempts::table
            .filter(payment_attempts::id.eq(attempt.id))
            .for_update()
            .first::<PaymentAttempt>(conn)
            .await?;
        let order = orders::table
            .filter(orders::id.eq(locked_attempt.order_id))
            .for_update()
            .first::<Order>(conn)
            .await?;

        if locked_attempt.state == PaymentAttemptState::Succeeded.as_ref()
            || order.status == OrderStatus::Delivered.as_ref()
        {
            tracing::info!(
                order_id = %order.id,
                payment_attempt_id = %locked_attempt.id,
                "payment completed before prepared entry was published"
            );
            return Ok(PublishPaymentAttemptOutcome::AlreadyDelivered);
        }

        let now = Utc::now();
        if locked_attempt.state != PaymentAttemptState::Created.as_ref()
            || order.status != OrderStatus::Pending.as_ref()
            || order.expires_at <= now
        {
            tracing::warn!(
                order_id = %order.id,
                payment_attempt_id = %locked_attempt.id,
                attempt_state = %locked_attempt.state,
                order_status = %order.status,
                expires_at = %order.expires_at,
                "prepared payment entry was not published because local reservation is unavailable"
            );
            return Ok(PublishPaymentAttemptOutcome::Unavailable);
        }

        let updated = diesel::update(
            payment_attempts::table
                .filter(payment_attempts::id.eq(locked_attempt.id))
                .filter(payment_attempts::state.eq(PaymentAttemptState::Created.as_ref())),
        )
        .set((
            payment_attempts::state.eq(PaymentAttemptState::Ready.as_ref()),
            payment_attempts::code_url.eq(code_url),
            payment_attempts::updated_at.eq(now),
        ))
        .execute(conn)
        .await?;
        if updated != 1 {
            tracing::error!(
                order_id = %order.id,
                payment_attempt_id = %locked_attempt.id,
                updated,
                "payment entry publication lost its expected state"
            );
            return Err(AppError::Conflict(
                "payment attempt state changed while publishing".to_string(),
            ));
        }
        Ok(PublishPaymentAttemptOutcome::Ready)
    })
    .await
}

/// 远端下单失败只能把仍处于 created 的尝试标成 failed；若超时任务已经关闭该尝试，
/// 这里必须保持 closed，不能把终态重新改成 worker 会继续处理的 failed。
async fn mark_payment_attempt_failed_if_created(
    state: &AppState,
    attempt_id: Uuid,
) -> Result<(), AppError> {
    let mut conn = state.pool.get().await?;
    let updated = diesel::update(
        payment_attempts::table
            .filter(payment_attempts::id.eq(attempt_id))
            .filter(payment_attempts::state.eq(PaymentAttemptState::Created.as_ref())),
    )
    .set((
        payment_attempts::state.eq(PaymentAttemptState::Failed.as_ref()),
        payment_attempts::updated_at.eq(Utc::now()),
    ))
    .execute(&mut conn)
    .await?;
    tracing::info!(
        %attempt_id,
        updated,
        "recorded WeChat Pay preparation failure when attempt remained created"
    );
    Ok(())
}

pub async fn list_orders_by_contact(
    State(state): State<AppState>,
    Json(request): Json<ListOrdersByContactRequest>,
) -> Result<Json<OffsetPageResponse<OrderSummaryResponse>>, AppError> {
    let contact = request.contact.trim();
    let contact_len = contact.chars().count();
    if contact.is_empty() {
        tracing::warn!("list orders by contact rejected: missing contact");
        return Err(AppError::BadRequest("contact is required".to_string()));
    }
    if contact_len > CONTACT_MAX_LENGTH {
        tracing::warn!(
            contact_len,
            contact_max_length = CONTACT_MAX_LENGTH,
            "list orders by contact rejected: contact is too long"
        );
        return Err(AppError::BadRequest(format!(
            "contact must be at most {CONTACT_MAX_LENGTH} characters"
        )));
    }

    let pagination = OffsetPagination {
        page: request.page,
        page_size: request.page_size,
    };
    let (page, page_size, offset) = normalize_offset_page(&pagination)?;
    tracing::info!(
        contact_len,
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
        .inner_join(payment_attempts::table.on(payment_attempts::order_id.eq(orders::id)))
        .filter(orders::contact.eq(contact))
        .select((
            orders::id,
            orders::product_info_id,
            orders::product_name_snapshot,
            orders::amount_cents,
            orders::status,
            payment_attempts::paid_at,
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
    // 订单交付状态与支付时间来自两个职责清晰的表，必须在同一条 SQL 中读取，避免支付
    // 确认事务恰好并发提交时向客户端返回跨时刻拼接的状态。
    let (order, payment_paid_at) = orders::table
        .inner_join(payment_attempts::table.on(payment_attempts::order_id.eq(orders::id)))
        .filter(orders::id.eq(order_id))
        .select((orders::all_columns, payment_attempts::paid_at))
        .first::<(Order, Option<DateTime<Utc>>)>(&mut conn)
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

    let content = match (
        order.status == OrderStatus::Delivered.as_ref(),
        order.product_id,
    ) {
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
        payment_paid_at,
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
