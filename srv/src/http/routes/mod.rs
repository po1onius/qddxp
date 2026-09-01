pub mod admin;
pub mod admin_auth;
pub mod epay;
pub mod public;
pub mod wechatpay;

use axum::{
    Json, Router,
    http::{HeaderValue, StatusCode, header::CONTENT_TYPE},
    routing::{any, get, patch, post},
};
use serde_json::json;
use tower_http::{
    services::{ServeDir, ServeFile},
    set_header::SetResponseHeader,
    trace::{DefaultMakeSpan, DefaultOnFailure, DefaultOnRequest, DefaultOnResponse, TraceLayer},
};
use tracing::Level;

use crate::{AppState, http::rate_limit};

pub fn router(state: AppState) -> Router {
    let trusted_proxy_cidrs = &state.config.rate_limit_trusted_proxy_cidrs;
    let rate_limit::RateLimitLayers {
        api_global,
        public_read,
        captcha_issue,
        order_creation,
        order_query,
        orders_by_contact,
        admin_login,
        admin_session,
        admin_authenticated,
        epay_notify,
        wechatpay_notify,
        unknown_api,
        health,
        static_files: static_file_limit,
    } = rate_limit::RateLimitLayers::new(trusted_proxy_cidrs);

    let web_dist_dir = state.config.web_dist_dir.clone();
    let index_file = web_dist_dir.join("index.html");
    // BrowserRouter 的直接访问与刷新都应返回 index.html 的正常 200 响应；
    // `not_found_service` 会强制改成 404，因此 SPA 回退必须使用保留原状态码的 `fallback`。
    let static_files = ServeDir::new(web_dist_dir).fallback(ServeFile::new(index_file));
    // Logo 已在启动阶段解析并验证为 SVG。这里显式覆盖响应头，确保本地开发即使传入
    // 无扩展名文件也会按 SVG 提供，且响应类型不受宿主机文件名影响。
    let shop_logo = SetResponseHeader::overriding(
        ServeFile::new(state.config.shop_logo_file.clone()),
        CONTENT_TYPE,
        HeaderValue::from_static("image/svg+xml"),
    );

    let public_read_api = Router::new()
        .route("/storefront", get(public::get_storefront))
        .route_service("/storefront/logo", shop_logo)
        .route("/products", get(public::list_products))
        .route("/products/{id}", get(public::get_product))
        .route("/payment-methods", get(public::list_payment_methods))
        .layer(public_read);

    let public_action_api = Router::new()
        .route("/captcha", get(public::issue_captcha).layer(captcha_issue))
        .route("/orders", post(public::create_order).layer(order_creation))
        .route(
            "/orders/by-contact",
            post(public::list_orders_by_contact).layer(orders_by_contact),
        )
        .route(
            "/orders/query",
            post(public::query_order).layer(order_query),
        );

    let payment_callback_api = Router::new()
        .route(
            "/payments/epay/notify",
            get(epay::notify_query)
                .post(epay::notify_form)
                .layer(epay_notify),
        )
        .route(
            "/payments/wechatpay/notify",
            post(wechatpay::notify).layer(wechatpay_notify),
        );

    let admin_session_api = Router::new().route(
        "/admin/session",
        get(admin_auth::status)
            .delete(admin_auth::logout)
            .layer(admin_session)
            // 登录、状态查询与退出分别使用业务额度，避免状态轮询消耗登录配额。
            .merge(post(admin_auth::login).layer(admin_login)),
    );

    let admin_api = Router::new()
        .route(
            "/admin/product-info",
            get(admin::list_product_info).post(admin::create_product_info),
        )
        .route(
            "/admin/announcement",
            get(admin::get_announcement_settings).patch(admin::update_announcement),
        )
        .route(
            "/admin/product-info/{id}/active",
            patch(admin::update_product_info_active),
        )
        .route(
            "/admin/product-info/{id}",
            patch(admin::update_product_info),
        )
        .route(
            "/admin/products",
            get(admin::list_products).post(admin::create_product),
        )
        .route(
            "/admin/products/status",
            patch(admin::update_product_status),
        )
        .route("/admin/orders", get(admin::list_orders))
        .route(
            "/admin/orders/{id}/remark",
            patch(admin::update_order_remark),
        )
        .route("/admin/api-call-logs", get(admin::list_api_call_logs))
        .layer(admin_authenticated);

    // 支付通知拥有完全独立的计数桶，不参与访客全局额度，避免支付平台共享出口或重试
    // 流量被普通访客耗尽。其他已知 API 同时消耗业务桶和全局安全网额度。
    let visitor_api = Router::new()
        .merge(public_read_api)
        .merge(public_action_api)
        .merge(admin_session_api)
        .merge(admin_api)
        .layer(api_global);
    let api = visitor_api
        .merge(payment_callback_api)
        .fallback_service(any(api_not_found).layer(unknown_api))
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(DefaultMakeSpan::new().level(Level::INFO))
                .on_request(DefaultOnRequest::new().level(Level::INFO))
                .on_response(DefaultOnResponse::new().level(Level::INFO))
                .on_failure(DefaultOnFailure::new().level(Level::WARN)),
        )
        .with_state(state);

    Router::new()
        .route(
            "/health",
            get(|| async { Json(json!({ "ok": true })) }).layer(health),
        )
        .nest("/api", api)
        .merge(
            Router::new()
                .fallback_service(static_files)
                .layer(static_file_limit),
        )
}

async fn api_not_found() -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::NOT_FOUND,
        Json(json!({ "error": "api route not found" })),
    )
}
