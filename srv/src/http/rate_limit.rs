use std::{
    net::{IpAddr, SocketAddr},
    num::NonZeroU32,
    sync::Arc,
};

use axum::{
    Json,
    extract::ConnectInfo,
    http::{StatusCode, request::Parts},
    response::{IntoResponse, Response},
};
use axum_governor::{
    ExtractionError, GovernorConfigBuilder, GovernorLayer, KeyExtractor, KeyOutcome, Quota,
    RejectionReason,
};
use ipnet::IpNet;
use serde_json::json;

const X_FORWARDED_FOR: &str = "x-forwarded-for";

/// 所有限流器都从同一个可信代理配置解析客户端 IP，但每个字段拥有独立的令牌桶，
/// 从而让全局、业务分组和支付回调额度可以按路由自由组合。
pub struct RateLimitLayers {
    pub api_global: GovernorLayer<IpAddr>,
    pub public_read: GovernorLayer<IpAddr>,
    pub captcha_issue: GovernorLayer<IpAddr>,
    pub order_creation: GovernorLayer<IpAddr>,
    pub order_query: GovernorLayer<IpAddr>,
    pub orders_by_contact: GovernorLayer<IpAddr>,
    pub admin_login: GovernorLayer<IpAddr>,
    pub admin_session: GovernorLayer<IpAddr>,
    pub admin_authenticated: GovernorLayer<IpAddr>,
    pub epay_notify: GovernorLayer<IpAddr>,
    pub wechatpay_notify: GovernorLayer<IpAddr>,
    pub unknown_api: GovernorLayer<IpAddr>,
    pub health: GovernorLayer<IpAddr>,
    pub static_files: GovernorLayer<IpAddr>,
}

impl RateLimitLayers {
    pub fn new(trusted_proxy_cidrs: &[IpNet]) -> Self {
        let client_ip = TrustedClientIpExtractor::new(trusted_proxy_cidrs);
        tracing::info!(
            trusted_proxy_cidrs = ?trusted_proxy_cidrs,
            "initializing IP token-bucket rate limiters"
        );

        Self {
            api_global: build_layer(
                client_ip.clone(),
                RateLimitPolicy::per_minute("api_global", 120, 40),
            ),
            public_read: build_layer(
                client_ip.clone(),
                RateLimitPolicy::per_minute("public_read", 60, 20),
            ),
            captcha_issue: build_layer(
                client_ip.clone(),
                RateLimitPolicy::spaced("captcha_issue", 20, 180, 9, 5),
            ),
            order_creation: build_layer(
                client_ip.clone(),
                RateLimitPolicy::spaced("order_creation", 5, 180, 36, 5),
            ),
            order_query: build_layer(
                client_ip.clone(),
                RateLimitPolicy::per_minute("order_query", 60, 15),
            ),
            orders_by_contact: build_layer(
                client_ip.clone(),
                RateLimitPolicy::per_minute("orders_by_contact", 10, 5),
            ),
            admin_login: build_layer(
                client_ip.clone(),
                RateLimitPolicy::per_minute("admin_login", 3, 3),
            ),
            admin_session: build_layer(
                client_ip.clone(),
                RateLimitPolicy::per_minute("admin_session", 30, 10),
            ),
            admin_authenticated: build_layer(
                client_ip.clone(),
                RateLimitPolicy::per_minute("admin_authenticated", 120, 30),
            ),
            // 支付平台可能从共享出口集中投递或重试通知，因此各平台使用独立且宽松的桶，
            // 也不参与访客 API 的全局额度。
            epay_notify: build_layer(
                client_ip.clone(),
                RateLimitPolicy::per_minute("epay_notify", 300, 100),
            ),
            wechatpay_notify: build_layer(
                client_ip.clone(),
                RateLimitPolicy::per_minute("wechatpay_notify", 300, 100),
            ),
            unknown_api: build_layer(
                client_ip.clone(),
                RateLimitPolicy::per_minute("unknown_api", 30, 10),
            ),
            health: build_layer(
                client_ip.clone(),
                RateLimitPolicy::per_minute("health", 60, 10),
            ),
            static_files: build_layer(
                client_ip,
                RateLimitPolicy::per_minute("static_files", 300, 100),
            ),
        }
    }
}

#[derive(Clone, Copy)]
struct RateLimitPolicy {
    scope: &'static str,
    quota: Quota,
    rate_requests: u32,
    rate_window_seconds: u64,
    burst: u32,
}

impl RateLimitPolicy {
    fn per_minute(scope: &'static str, requests: u32, burst: u32) -> Self {
        Self {
            scope,
            quota: Quota::requests_per_minute(non_zero(requests)).burst(non_zero(burst)),
            rate_requests: requests,
            rate_window_seconds: 60,
            burst,
        }
    }

    fn spaced(
        scope: &'static str,
        requests: u32,
        window_seconds: u64,
        seconds_per_request: u32,
        burst: u32,
    ) -> Self {
        Self {
            scope,
            quota: Quota::seconds_per_request(non_zero(seconds_per_request)).burst(non_zero(burst)),
            rate_requests: requests,
            rate_window_seconds: window_seconds,
            burst,
        }
    }
}

fn non_zero(value: u32) -> NonZeroU32 {
    NonZeroU32::new(value).expect("rate-limit values are positive constants")
}

fn build_layer(
    client_ip: TrustedClientIpExtractor,
    policy: RateLimitPolicy,
) -> GovernorLayer<IpAddr> {
    tracing::info!(
        rate_limit_scope = policy.scope,
        rate_requests = policy.rate_requests,
        rate_window_seconds = policy.rate_window_seconds,
        burst = policy.burst,
        "IP token-bucket rate limiter initialized"
    );

    let config = GovernorConfigBuilder::default()
        .with_extractor(client_ip)
        .expect_connect_info()
        .quota_default(policy.quota)
        .error_handler(move |reason| rejection_response(policy.scope, reason))
        .finish()
        .expect("static rate-limit configuration must be valid");
    GovernorLayer::new(config)
}

#[derive(Clone, Debug)]
struct TrustedClientIpExtractor {
    trusted_proxy_cidrs: Arc<[IpNet]>,
}

impl TrustedClientIpExtractor {
    fn new(trusted_proxy_cidrs: &[IpNet]) -> Self {
        Self {
            trusted_proxy_cidrs: Arc::from(trusted_proxy_cidrs),
        }
    }

    fn is_trusted_proxy(&self, ip: IpAddr) -> bool {
        self.trusted_proxy_cidrs
            .iter()
            .any(|network| network.contains(&ip))
    }
}

impl KeyExtractor for TrustedClientIpExtractor {
    type Key = IpAddr;

    fn extract(&self, parts: &Parts) -> Result<KeyOutcome<Self::Key>, ExtractionError> {
        let peer_ip = parts
            .extensions
            .get::<ConnectInfo<SocketAddr>>()
            .map(|ConnectInfo(address)| address.ip())
            .ok_or(ExtractionError::MissingConnectInfo)?;
        if !self.is_trusted_proxy(peer_ip) {
            return Ok(KeyOutcome {
                key: peer_ip,
                quota_override: None,
            });
        }

        // X-Forwarded-For 从左到右记录客户端到代理链。只有当前一跳属于受信网段时才
        // 继续向左剥离，因此攻击者预置的伪造地址无法越过第一个非受信来源地址。
        let mut forwarded_hops = Vec::new();
        for value in parts.headers.get_all(X_FORWARDED_FOR) {
            let value = value
                .to_str()
                .map_err(|_| ExtractionError::MalformedHeader(X_FORWARDED_FOR))?;
            for hop in value.split(',') {
                forwarded_hops.push(
                    hop.trim()
                        .parse::<IpAddr>()
                        .map_err(|_| ExtractionError::MalformedHeader(X_FORWARDED_FOR))?,
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

        Ok(KeyOutcome {
            key: client_ip,
            quota_override: None,
        })
    }

    fn requires_connect_info(&self) -> bool {
        true
    }
}

fn rejection_response(scope: &'static str, reason: RejectionReason) -> Response {
    match reason {
        RejectionReason::QuotaExceeded {
            wait,
            key,
            policy_name,
            ..
        } => {
            let retry_after_seconds = wait.as_secs().max(1);
            let client_ip = key.downcast_ref::<IpAddr>().copied();
            tracing::warn!(
                rate_limit_scope = scope,
                limiter_policy = %policy_name,
                ?client_ip,
                retry_after_seconds,
                "request rejected by IP token-bucket rate limiter"
            );
            (
                StatusCode::TOO_MANY_REQUESTS,
                Json(json!({
                    "error": "too_many_requests",
                    "rate_limit_scope": scope,
                    "retry_after_seconds": retry_after_seconds,
                })),
            )
                .into_response()
        }
        RejectionReason::KeyExtractionFailed(error) => {
            let status = match &error {
                ExtractionError::MalformedHeader(_)
                | ExtractionError::MissingHeader(_)
                | ExtractionError::UntrustedProxy => StatusCode::BAD_REQUEST,
                ExtractionError::MissingConnectInfo | ExtractionError::Other(_) => {
                    StatusCode::INTERNAL_SERVER_ERROR
                }
            };
            if status.is_server_error() {
                tracing::error!(
                    rate_limit_scope = scope,
                    error = ?error,
                    "rate limiter could not determine client IP"
                );
            } else {
                tracing::warn!(
                    rate_limit_scope = scope,
                    error = ?error,
                    "rate limiter rejected invalid client IP metadata"
                );
            }
            let error_message = if status.is_server_error() {
                "unable to determine client IP"
            } else {
                "invalid client IP forwarding metadata"
            };
            (
                status,
                Json(json!({
                    "error": error_message,
                    "rate_limit_scope": scope,
                })),
            )
                .into_response()
        }
    }
}
