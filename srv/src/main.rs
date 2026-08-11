mod config;
mod db;
mod domain;
mod error;
mod http;
mod payments;
mod security;

use std::sync::Arc;

use axum::extract::DefaultBodyLimit;
use config::AppConfig;
use db::pool::{DbPool, create_pool};
use payments::wechatpay::WechatPayClient;
use tokio::net::TcpListener;
use tower_http::cors::{Any, CorsLayer};
use tracing_subscriber::EnvFilter;

#[derive(Clone)]
pub struct AppState {
    pub pool: DbPool,
    pub config: Arc<AppConfig>,
    pub wechatpay: Option<Arc<WechatPayClient>>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let config = Arc::new(AppConfig::from_env()?);
    tracing::info!(
        listen_addr = %config.listen_addr,
        public_base_url = %config.public_base_url,
        web_return_url = %config.web_return_url,
        web_dist_dir = %config.web_dist_dir.display(),
        shop_name = %config.shop_name,
        shop_logo_file = %config.shop_logo_file.display(),
        epay_configured = config.epay.is_some(),
        wechatpay_configured = config.wechatpay.is_some(),
        "application config loaded"
    );
    db::migrate::run_pending(&config.database_url)?;
    let pool = create_pool(&config.database_url).await?;
    tracing::info!("database pool initialized");
    let wechatpay = config
        .wechatpay
        .as_ref()
        .map(WechatPayClient::from_config)
        .transpose()?
        .map(Arc::new);
    let state = AppState {
        pool,
        config: Arc::clone(&config),
        wechatpay,
    };
    tokio::spawn(payments::expiration::run(state.clone()));

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    let app = http::routes::router(state)
        .layer(DefaultBodyLimit::max(10 * 1024 * 1024))
        .layer(cors);
    let listener = TcpListener::bind(config.listen_addr).await?;

    tracing::info!("listening on {}", config.listen_addr);
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    Ok(())
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
}
