mod config;
mod db;
mod domain;
mod error;
mod http;
mod notifications;
mod payments;
mod security;

use std::{net::SocketAddr, sync::Arc};

use axum::extract::DefaultBodyLimit;
use config::AppConfig;
use db::pool::{DbPool, create_pool};
use notifications::TelegramNotifier;
use payments::wechatpay::WechatPayClient;
use time::Duration;
use tokio::net::TcpListener;
use tower_http::cors::{Any, CorsLayer};
use tower_sessions::{Expiry, SessionManagerLayer, cookie::SameSite};
use tower_sessions_moka_store::MokaStore;
use tracing_subscriber::EnvFilter;

const ADMIN_SESSION_IDLE_MINUTES: i64 = 30;
const ADMIN_SESSION_MAX_CAPACITY: u64 = 10_000;

#[derive(Clone)]
pub struct AppState {
    pub pool: DbPool,
    pub config: Arc<AppConfig>,
    pub wechatpay: Option<Arc<WechatPayClient>>,
    pub telegram: Option<Arc<TelegramNotifier>>,
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
        telegram_notifications_configured = config.telegram.is_some(),
        rate_limit_trusted_proxy_cidrs = ?config.rate_limit_trusted_proxy_cidrs,
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
    let telegram = config
        .telegram
        .as_ref()
        .map(TelegramNotifier::from_config)
        .transpose()?
        .map(Arc::new);
    let state = AppState {
        pool,
        config: Arc::clone(&config),
        wechatpay,
        telegram,
    };
    tokio::spawn(payments::expiration::run(state.clone()));
    if state.telegram.is_some() {
        tokio::spawn(notifications::run(state.clone()));
    } else {
        tracing::info!("Telegram notification worker disabled because it is not configured");
    }

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    // 管理后台与 API 在生产和本地开发时都保持同源。会话只保存在当前进程的 Moka
    // 存储中，不增加 Redis 等部署依赖；容量上限和按会话过期淘汰可防止记录无限增长。
    let secure_admin_cookie = config.public_base_url.starts_with("https://");
    let admin_session_store = MokaStore::new(Some(ADMIN_SESSION_MAX_CAPACITY));
    let admin_session_layer = SessionManagerLayer::new(admin_session_store)
        .with_name("qddxp_admin_session")
        .with_path("/api/admin")
        .with_http_only(true)
        .with_same_site(SameSite::Strict)
        .with_secure(secure_admin_cookie)
        .with_expiry(Expiry::OnInactivity(Duration::minutes(
            ADMIN_SESSION_IDLE_MINUTES,
        )))
        // 每次已认证的管理请求都延长空闲过期时间；公开接口没有提取 Session，
        // 不会触发会话读取、写入或额外的 Set-Cookie。
        .with_always_save(true);
    tracing::info!(
        idle_minutes = ADMIN_SESSION_IDLE_MINUTES,
        max_capacity = ADMIN_SESSION_MAX_CAPACITY,
        secure_cookie = secure_admin_cookie,
        cookie_path = "/api/admin",
        "admin session manager initialized"
    );
    if !secure_admin_cookie {
        tracing::warn!(
            public_base_url = %config.public_base_url,
            "admin session cookie Secure flag is disabled because PUBLIC_BASE_URL is not HTTPS"
        );
    }

    let app = http::routes::router(state)
        .layer(DefaultBodyLimit::max(10 * 1024 * 1024))
        .layer(cors)
        .layer(admin_session_layer);
    let listener = TcpListener::bind(config.listen_addr).await?;

    tracing::info!("listening on {}", config.listen_addr);
    // Peer-IP 限流依赖每条 TCP 连接的真实对端地址；不能使用不携带 ConnectInfo 的
    // 默认 make service，否则中间件无法建立安全的客户端身份键。
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await?;

    Ok(())
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
}
