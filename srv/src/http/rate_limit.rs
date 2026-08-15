use std::{
    net::{IpAddr, SocketAddr},
    sync::Arc,
    time::{Duration, Instant},
};

use axum::{
    Json,
    extract::{ConnectInfo, Request, State},
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
};
use dashmap::DashMap;
use ipnet::IpNet;
use serde_json::json;

const X_FORWARDED_FOR: &str = "x-forwarded-for";
const ORDER_CREATION_LIMIT_WINDOW: Duration = Duration::from_secs(3 * 60);
const ORDER_CREATION_LIMIT_MAX_REQUESTS: u32 = 5;
const ADMIN_LOGIN_LIMIT_WINDOW: Duration = Duration::from_secs(60);
const ADMIN_LOGIN_LIMIT_MAX_REQUESTS: u32 = 3;
const CLEANUP_INTERVAL: Duration = Duration::from_secs(60);

#[derive(Clone, Copy, Debug)]
struct RateLimitPolicy {
    scope: &'static str,
    window: Duration,
    max_requests: u32,
    error_message: &'static str,
}

const ORDER_CREATION_POLICY: RateLimitPolicy = RateLimitPolicy {
    scope: "order_creation",
    window: ORDER_CREATION_LIMIT_WINDOW,
    max_requests: ORDER_CREATION_LIMIT_MAX_REQUESTS,
    error_message: "too many order creation requests",
};

const ADMIN_LOGIN_POLICY: RateLimitPolicy = RateLimitPolicy {
    scope: "admin_login",
    window: ADMIN_LOGIN_LIMIT_WINDOW,
    max_requests: ADMIN_LOGIN_LIMIT_MAX_REQUESTS,
    error_message: "too many admin login requests",
};

#[derive(Debug)]
struct FixedWindow {
    started_at: Instant,
    accepted_requests: u32,
}

/// 按客户端 IP 计数的固定窗口限流器。每个业务入口创建独立实例，因此管理员
/// 登录与创建订单不会相互消耗配额。DashMap 保证同一 IP 的并发计数更新原子化，
/// 克隆中间件状态时仍共享同一实例的计数表，避免并发请求绕过限制。
#[derive(Clone, Debug)]
pub struct FixedWindowRateLimiter {
    windows: Arc<DashMap<IpAddr, FixedWindow>>,
    trusted_proxy_cidrs: Arc<[IpNet]>,
    policy: RateLimitPolicy,
}

impl FixedWindowRateLimiter {
    pub fn for_order_creation(trusted_proxy_cidrs: &[IpNet]) -> Self {
        Self::new(trusted_proxy_cidrs, ORDER_CREATION_POLICY)
    }

    pub fn for_admin_login(trusted_proxy_cidrs: &[IpNet]) -> Self {
        Self::new(trusted_proxy_cidrs, ADMIN_LOGIN_POLICY)
    }

    fn new(trusted_proxy_cidrs: &[IpNet], policy: RateLimitPolicy) -> Self {
        let windows = Arc::new(DashMap::<IpAddr, FixedWindow>::new());
        let cleanup_windows = Arc::clone(&windows);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(CLEANUP_INTERVAL);
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            interval.tick().await;
            loop {
                interval.tick().await;
                let now = Instant::now();
                let entries_before = cleanup_windows.len();
                cleanup_windows.retain(|_, window| {
                    now.saturating_duration_since(window.started_at) < policy.window
                });
                tracing::debug!(
                    rate_limit_scope = policy.scope,
                    entries_before,
                    entries_after = cleanup_windows.len(),
                    "expired fixed-window rate limit entries cleaned"
                );
            }
        });

        tracing::info!(
            rate_limit_scope = policy.scope,
            window_seconds = policy.window.as_secs(),
            max_requests = policy.max_requests,
            trusted_proxy_cidrs = ?trusted_proxy_cidrs,
            "fixed-window rate limiter initialized"
        );
        Self {
            windows,
            trusted_proxy_cidrs: Arc::from(trusted_proxy_cidrs),
            policy,
        }
    }

    fn try_acquire(&self, client_ip: IpAddr) -> Result<(), Duration> {
        let now = Instant::now();
        let mut window = self.windows.entry(client_ip).or_insert(FixedWindow {
            started_at: now,
            accepted_requests: 0,
        });
        let elapsed = now.saturating_duration_since(window.started_at);

        if elapsed >= self.policy.window {
            *window = FixedWindow {
                started_at: now,
                accepted_requests: 1,
            };
            return Ok(());
        }
        if window.accepted_requests >= self.policy.max_requests {
            return Err(self.policy.window.saturating_sub(elapsed));
        }

        window.accepted_requests += 1;
        Ok(())
    }

    fn client_ip(&self, request: &Request) -> Result<IpAddr, ClientIpError> {
        let peer_ip = request
            .extensions()
            .get::<ConnectInfo<SocketAddr>>()
            .map(|ConnectInfo(address)| address.ip())
            .ok_or(ClientIpError::MissingConnectInfo)?;
        if !self.is_trusted_proxy(peer_ip) {
            return Ok(peer_ip);
        }

        // X-Forwarded-For 从左到右是客户端到代理链。只有当前一跳属于受信网段时才继续
        // 向左剥离，攻击者预置的伪造地址无法越过第一个非受信来源地址。
        let mut forwarded_hops = Vec::new();
        for value in request.headers().get_all(X_FORWARDED_FOR) {
            let value = value
                .to_str()
                .map_err(|_| ClientIpError::InvalidForwardedFor)?;
            for hop in value.split(',') {
                forwarded_hops.push(
                    hop.trim()
                        .parse::<IpAddr>()
                        .map_err(|_| ClientIpError::InvalidForwardedFor)?,
                );
            }
        }

        let mut client_ip = peer_ip;
        for hop in forwarded_hops.into_iter().rev() {
            if !self.is_trusted_proxy(client_ip) {
                break;
            }
            client_ip = hop;
        }
        Ok(client_ip)
    }

    fn is_trusted_proxy(&self, ip: IpAddr) -> bool {
        self.trusted_proxy_cidrs
            .iter()
            .any(|network| network.contains(&ip))
    }
}

#[derive(Debug)]
enum ClientIpError {
    MissingConnectInfo,
    InvalidForwardedFor,
}

/// 此中间件只挂载到明确需要限流的 MethodRouter，不会计数其他 API。
pub async fn enforce_fixed_window_limit(
    State(limiter): State<FixedWindowRateLimiter>,
    request: Request,
    next: Next,
) -> Response {
    let client_ip = match limiter.client_ip(&request) {
        Ok(client_ip) => client_ip,
        Err(error) => {
            // IP 无法识别时失败关闭，避免通过畸形代理头绕过限流。
            tracing::error!(
                rate_limit_scope = limiter.policy.scope,
                error = ?error,
                "rate limiter could not determine client IP"
            );
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "unable to determine client IP" })),
            )
                .into_response();
        }
    };

    if let Err(retry_after) = limiter.try_acquire(client_ip) {
        // Retry-After 使用向上取整的整秒数，避免不足一秒时返回 0 导致客户端立即重试。
        let retry_after_seconds = retry_after.as_secs() + u64::from(retry_after.subsec_nanos() > 0);
        tracing::warn!(
            rate_limit_scope = limiter.policy.scope,
            %client_ip,
            retry_after_seconds,
            "request rejected by fixed-window rate limiter"
        );
        let mut response = (
            StatusCode::TOO_MANY_REQUESTS,
            Json(json!({
                "error": limiter.policy.error_message,
                "retry_after_seconds": retry_after_seconds,
            })),
        )
            .into_response();
        response.headers_mut().insert(
            "retry-after",
            retry_after_seconds
                .to_string()
                .parse()
                .expect("正整数秒数一定是合法响应头值"),
        );
        return response;
    }

    next.run(request).await
}
