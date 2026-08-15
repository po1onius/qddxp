pub mod admin;
pub mod admin_auth;
pub mod epay;
pub mod public;
pub mod wechatpay;

use axum::{
    Json, Router,
    http::{HeaderValue, StatusCode, header::CONTENT_TYPE},
    middleware,
    routing::{get, patch, post},
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
    let order_creation_limiter =
        rate_limit::FixedWindowRateLimiter::for_order_creation(trusted_proxy_cidrs);
    let admin_login_limiter =
        rate_limit::FixedWindowRateLimiter::for_admin_login(trusted_proxy_cidrs);

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

    let api = Router::new()
        .route("/storefront", get(public::get_storefront))
        .route_service("/storefront/logo", shop_logo)
        .route("/products", get(public::list_products))
        .route("/products/{id}", get(public::get_product))
        .route("/payment-methods", get(public::list_payment_methods))
        .route(
            "/orders",
            post(public::create_order).layer(middleware::from_fn_with_state(
                order_creation_limiter,
                rate_limit::enforce_fixed_window_limit,
            )),
        )
        .route("/orders/by-contact", post(public::list_orders_by_contact))
        .route("/orders/query", post(public::query_order))
        .route(
            "/payments/epay/notify",
            get(epay::notify_query).post(epay::notify_form),
        )
        .route("/payments/wechatpay/notify", post(wechatpay::notify))
        .route(
            "/admin/session",
            get(admin_auth::status)
                .delete(admin_auth::logout)
                // 限流只包裹 POST 登录处理器，会话状态查询和退出登录不消耗登录配额。
                .merge(
                    post(admin_auth::login).layer(middleware::from_fn_with_state(
                        admin_login_limiter,
                        rate_limit::enforce_fixed_window_limit,
                    )),
                ),
        )
        .route(
            "/admin/product-info",
            get(admin::list_product_info).post(admin::create_product_info),
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
        .route("/admin/api-call-logs", get(admin::list_api_call_logs))
        .fallback(api_not_found)
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(DefaultMakeSpan::new().level(Level::INFO))
                .on_request(DefaultOnRequest::new().level(Level::INFO))
                .on_response(DefaultOnResponse::new().level(Level::INFO))
                .on_failure(DefaultOnFailure::new().level(Level::WARN)),
        )
        .with_state(state);

    Router::new()
        .route("/health", get(|| async { Json(json!({ "ok": true })) }))
        .nest("/api", api)
        .fallback_service(static_files)
}

async fn api_not_found() -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::NOT_FOUND,
        Json(json!({ "error": "api route not found" })),
    )
}
