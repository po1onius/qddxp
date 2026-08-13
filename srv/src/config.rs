use std::{
    env, fmt,
    net::SocketAddr,
    path::{Path, PathBuf},
};

use ipnet::IpNet;
use thiserror::Error;

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub listen_addr: SocketAddr,
    pub database_url: String,
    pub public_base_url: String,
    pub web_return_url: String,
    pub web_dist_dir: PathBuf,
    pub shop_name: String,
    pub shop_logo_file: PathBuf,
    pub admin_key: String,
    pub order_password_pepper: String,
    /// 只有来自这些网段的直接连接才允许使用 X-Forwarded-For 识别真实客户端。
    /// 留空时始终使用 TCP 对端 IP，避免公网请求伪造代理头绕过限流。
    pub rate_limit_trusted_proxy_cidrs: Vec<IpNet>,
    /// 微信官方 Native 支付结束时间。ePay 使用固定三分钟的本地库存预占期限，二者不能共用配置。
    pub wechatpay_expire_minutes: i64,
    pub epay: Option<EpayConfig>,
    pub wechatpay: Option<WechatPayConfig>,
    pub telegram: Option<TelegramConfig>,
}

#[derive(Debug, Clone)]
pub struct EpayConfig {
    pub gateway: String,
    pub pid: String,
    pub key: String,
}

#[derive(Debug, Clone)]
pub struct WechatPayConfig {
    pub app_id: String,
    pub mch_id: String,
    pub merchant_serial_no: String,
    pub merchant_private_key_file: PathBuf,
    pub api_v3_key: String,
    pub public_key_id: String,
    pub public_key_file: PathBuf,
    pub notify_url: String,
}

#[derive(Clone)]
pub struct TelegramConfig {
    pub bot_token: String,
    pub chat_id: String,
}

// Bot Token 属于生产密钥，即使未来有人直接调试输出 AppConfig，也必须保持脱敏。
impl fmt::Debug for TelegramConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TelegramConfig")
            .field("bot_token", &"[REDACTED]")
            .field("chat_id", &self.chat_id)
            .finish()
    }
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("missing required environment variable: {0}")]
    MissingVar(&'static str),
    #[error("invalid LISTEN_ADDR: {0}")]
    InvalidListenAddr(#[from] std::net::AddrParseError),
    #[error("invalid positive integer environment variable: {0}")]
    InvalidPositiveInteger(&'static str),
    #[error("invalid CIDR in RATE_LIMIT_TRUSTED_PROXY_CIDRS: {0}")]
    InvalidTrustedProxyCidr(String),
    #[error("SHOP_NAME must contain between 1 and 100 characters")]
    InvalidShopName,
    #[error("SHOP_LOGO_FILE must contain a valid SVG image: {0}")]
    UnsupportedShopLogo(PathBuf),
    #[error("cannot read SHOP_LOGO_FILE {file}: {source}")]
    UnreadableShopLogo {
        file: PathBuf,
        source: std::io::Error,
    },
    #[error("incomplete payment configuration: {0}")]
    IncompletePaymentConfig(&'static str),
    #[error("PUBLIC_BASE_URL must use https when official WeChat Pay is enabled")]
    WechatPayRequiresHttps,
    #[error(
        "incomplete Telegram configuration: TELEGRAM_BOT_TOKEN and TELEGRAM_NOTIFY_CHAT_ID must be configured together"
    )]
    IncompleteTelegramConfig,
}

impl AppConfig {
    pub fn from_env() -> Result<Self, ConfigError> {
        let listen_addr = env::var("LISTEN_ADDR")
            .unwrap_or_else(|_| "0.0.0.0:3000".to_string())
            .parse()?;
        let database_url = required("DATABASE_URL")?;
        let public_base_url =
            env::var("PUBLIC_BASE_URL").unwrap_or_else(|_| "http://localhost:3000".to_string());
        let web_return_url = env::var("WEB_RETURN_URL")
            .unwrap_or_else(|_| "http://localhost:5173/orders".to_string());
        let web_dist_dir = required("WEB_DIST_DIR").map(PathBuf::from)?;
        let shop_name = required("SHOP_NAME")?.trim().to_string();
        if !(1..=100).contains(&shop_name.chars().count()) {
            return Err(ConfigError::InvalidShopName);
        }
        let shop_logo_file = PathBuf::from(required("SHOP_LOGO_FILE")?);
        validate_shop_logo(&shop_logo_file)?;
        let admin_key = required("ADMIN_KEY")?;
        let order_password_pepper = env::var("ORDER_PASSWORD_PEPPER")
            .unwrap_or_else(|_| "dev-insecure-change-me".to_string());
        let rate_limit_trusted_proxy_cidrs =
            parse_trusted_proxy_cidrs(env::var("RATE_LIMIT_TRUSTED_PROXY_CIDRS").ok().as_deref())?;
        let wechatpay_expire_minutes = env::var("WXPAY_EXPIRE_MINUTES")
            .unwrap_or_else(|_| "15".to_string())
            .parse::<i64>()
            .ok()
            .filter(|minutes| (1..=120).contains(minutes))
            .ok_or(ConfigError::InvalidPositiveInteger("WXPAY_EXPIRE_MINUTES"))?;

        let epay = match (
            optional_nonempty("EPAY_GATEWAY"),
            optional_nonempty("EPAY_PID"),
            optional_nonempty("EPAY_KEY"),
        ) {
            (Some(gateway), Some(pid), Some(key)) => Some(EpayConfig { gateway, pid, key }),
            _ => None,
        };

        // 固定的容器内密钥文件位置不应决定支付方是否启用；是否启用只由业务凭据判断。
        // 当五项业务凭据全部为空时忽略文件参数，使仅使用 ePay 的部署无需提供有效 PEM。
        let wechatpay_credentials = [
            optional_nonempty("WXPAY_APP_ID"),
            optional_nonempty("WXPAY_MCH_ID"),
            optional_nonempty("WXPAY_MERCHANT_SERIAL_NO"),
            optional_nonempty("WXPAY_API_V3_KEY"),
            optional_nonempty("WXPAY_PUBLIC_KEY_ID"),
        ];
        let configured_wechatpay_values = wechatpay_credentials
            .iter()
            .filter(|value| value.is_some())
            .count();
        let wechatpay = if configured_wechatpay_values == 0 {
            None
        } else if configured_wechatpay_values != wechatpay_credentials.len() {
            return Err(ConfigError::IncompletePaymentConfig(
                "all WXPAY credential values must be configured together",
            ));
        } else {
            if !public_base_url.starts_with("https://") {
                return Err(ConfigError::WechatPayRequiresHttps);
            }
            let [
                Some(app_id),
                Some(mch_id),
                Some(merchant_serial_no),
                Some(api_v3_key),
                Some(public_key_id),
            ] = wechatpay_credentials
            else {
                unreachable!("WeChat Pay configuration completeness already checked")
            };
            let merchant_private_key_file = optional_nonempty("WXPAY_MERCHANT_PRIVATE_KEY_FILE")
                .ok_or(ConfigError::IncompletePaymentConfig(
                    "WXPAY_MERCHANT_PRIVATE_KEY_FILE is required when WeChat Pay is enabled",
                ))?;
            let public_key_file = optional_nonempty("WXPAY_PUBLIC_KEY_FILE").ok_or(
                ConfigError::IncompletePaymentConfig(
                    "WXPAY_PUBLIC_KEY_FILE is required when WeChat Pay is enabled",
                ),
            )?;
            if api_v3_key.len() != 32 {
                return Err(ConfigError::IncompletePaymentConfig(
                    "WXPAY_API_V3_KEY must contain exactly 32 bytes",
                ));
            }
            Some(WechatPayConfig {
                app_id,
                mch_id,
                merchant_serial_no,
                merchant_private_key_file: PathBuf::from(merchant_private_key_file),
                api_v3_key,
                public_key_id,
                public_key_file: PathBuf::from(public_key_file),
                notify_url: format!(
                    "{}/api/payments/wechatpay/notify",
                    public_base_url.trim_end_matches('/')
                ),
            })
        };

        let telegram = telegram_config_from_values(
            env::var("TELEGRAM_BOT_TOKEN").ok(),
            env::var("TELEGRAM_NOTIFY_CHAT_ID").ok(),
        )?;

        Ok(Self {
            listen_addr,
            database_url,
            public_base_url,
            web_return_url,
            web_dist_dir,
            shop_name,
            shop_logo_file,
            admin_key,
            order_password_pepper,
            rate_limit_trusted_proxy_cidrs,
            wechatpay_expire_minutes,
            epay,
            wechatpay,
            telegram,
        })
    }
}

fn parse_trusted_proxy_cidrs(value: Option<&str>) -> Result<Vec<IpNet>, ConfigError> {
    value
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|cidr| !cidr.is_empty())
        .map(|cidr| {
            cidr.parse::<IpNet>()
                .map_err(|_| ConfigError::InvalidTrustedProxyCidr(cidr.to_string()))
        })
        .collect()
}

/// Logo 只允许 SVG。不能仅相信宿主机文件名，因为 Compose 会把任意源文件映射到固定的
/// 容器路径；因此启动时使用 XML 解析器检查真实内容和 SVG 根元素，失败时直接拒绝启动。
fn validate_shop_logo(file: &Path) -> Result<(), ConfigError> {
    let bytes = std::fs::read(file).map_err(|source| ConfigError::UnreadableShopLogo {
        file: file.to_path_buf(),
        source,
    })?;
    is_svg(&bytes)
        .then_some(())
        .ok_or_else(|| ConfigError::UnsupportedShopLogo(file.to_path_buf()))
}

fn is_svg(bytes: &[u8]) -> bool {
    let Ok(xml) = std::str::from_utf8(bytes) else {
        return false;
    };
    let Ok(document) = roxmltree::Document::parse(xml) else {
        return false;
    };
    let root = document.root_element();
    let tag = root.tag_name();
    tag.name() == "svg" && matches!(tag.namespace(), None | Some("http://www.w3.org/2000/svg"))
}

fn required(name: &'static str) -> Result<String, ConfigError> {
    env::var(name).map_err(|_| ConfigError::MissingVar(name))
}

fn optional_nonempty(name: &'static str) -> Option<String> {
    optional_nonempty_value(env::var(name).ok())
}

fn optional_nonempty_value(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn telegram_config_from_values(
    bot_token: Option<String>,
    chat_id: Option<String>,
) -> Result<Option<TelegramConfig>, ConfigError> {
    match (
        optional_nonempty_value(bot_token),
        optional_nonempty_value(chat_id),
    ) {
        (None, None) => Ok(None),
        (Some(bot_token), Some(chat_id)) => Ok(Some(TelegramConfig { bot_token, chat_id })),
        _ => Err(ConfigError::IncompleteTelegramConfig),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shop_logo_content_accepts_svg_without_relying_on_extension() {
        assert!(is_svg(
            br#"<?xml version="1.0"?><svg xmlns="http://www.w3.org/2000/svg"></svg>"#
        ));
        assert!(is_svg(br#"<svg viewBox="0 0 10 10"></svg>"#));
        assert!(is_svg(include_bytes!("../../deploy/assets/shop-logo.svg")));
    }

    #[test]
    fn shop_logo_content_rejects_non_svg_or_invalid_xml() {
        assert!(!is_svg(&[0x89, 0x50, 0x4E, 0x47]));
        assert!(!is_svg(b"<html></html>"));
        assert!(!is_svg(b"<svg>"));
    }

    #[test]
    fn trusted_proxy_cidrs_are_trimmed_and_validated() {
        assert_eq!(
            parse_trusted_proxy_cidrs(Some("127.0.0.1/32, 10.0.0.0/8"))
                .expect("合法 CIDR 应当可以解析"),
            vec![
                "127.0.0.1/32".parse().unwrap(),
                "10.0.0.0/8".parse().unwrap()
            ]
        );
        assert!(parse_trusted_proxy_cidrs(Some("10.0.0.1")).is_err());
        assert!(parse_trusted_proxy_cidrs(Some("")).unwrap().is_empty());
    }
}
