pub mod admin;
pub mod epay;
pub mod public;
pub mod wechatpay;

use axum::{
    Json, Router,
    http::StatusCode,
    routing::{get, patch, post},
};
use serde_json::json;
use tower_http::{
    services::{ServeDir, ServeFile},
    trace::{DefaultMakeSpan, DefaultOnFailure, DefaultOnRequest, DefaultOnResponse, TraceLayer},
};
use tracing::Level;

use crate::AppState;

pub fn router(state: AppState) -> Router {
    let web_dist_dir = state.config.web_dist_dir.clone();
    let index_file = web_dist_dir.join("index.html");
    let static_files = ServeDir::new(web_dist_dir).not_found_service(ServeFile::new(index_file));

    let api = Router::new()
        .route("/products", get(public::list_products))
        .route("/payment-methods", get(public::list_payment_methods))
        .route(
            "/order-allocation-mode",
            get(public::get_order_allocation_mode),
        )
        .route("/orders", post(public::create_order))
        .route("/orders/by-contact", post(public::list_orders_by_contact))
        .route("/orders/query", post(public::query_order))
        .route(
            "/payments/epay/notify",
            get(epay::notify_query).post(epay::notify_form),
        )
        .route("/payments/wechatpay/notify", post(wechatpay::notify))
        .route(
            "/orders/{id}/payments/wechatpay/query",
            post(wechatpay::reconcile_order),
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
            "/admin/products",
            get(admin::list_products).post(admin::create_product),
        )
        .route(
            "/admin/products/status",
            patch(admin::update_product_status),
        )
        .route("/admin/orders", get(admin::list_orders))
        .route(
            "/admin/order-allocation-mode",
            get(admin::get_order_allocation_mode).patch(admin::update_order_allocation_mode),
        )
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
