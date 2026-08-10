use std::{env, net::SocketAddr, path::PathBuf};

use thiserror::Error;

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub listen_addr: SocketAddr,
    pub database_url: String,
    pub public_base_url: String,
    pub web_return_url: String,
    pub web_dist_dir: PathBuf,
    pub admin_key: String,
    pub order_password_pepper: String,
    pub payment_expire_minutes: i64,
    pub epay: Option<EpayConfig>,
    pub wechatpay: Option<WechatPayConfig>,
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
    pub merchant_private_key_path: PathBuf,
    pub api_v3_key: String,
    pub public_key_id: String,
    pub public_key_path: PathBuf,
    pub notify_url: String,
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("missing required environment variable: {0}")]
    MissingVar(&'static str),
    #[error("invalid LISTEN_ADDR: {0}")]
    InvalidListenAddr(#[from] std::net::AddrParseError),
    #[error("invalid positive integer environment variable: {0}")]
    InvalidPositiveInteger(&'static str),
    #[error("incomplete payment configuration: {0}")]
    IncompletePaymentConfig(&'static str),
    #[error("PUBLIC_BASE_URL must use https when official WeChat Pay is enabled")]
    WechatPayRequiresHttps,
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
            .unwrap_or_else(|_| "http://localhost:5173/delivery".to_string());
        let web_dist_dir = required("WEB_DIST_DIR").map(PathBuf::from)?;
        let admin_key = required("ADMIN_KEY")?;
        let order_password_pepper = env::var("ORDER_PASSWORD_PEPPER")
            .unwrap_or_else(|_| "dev-insecure-change-me".to_string());
        let payment_expire_minutes = env::var("PAYMENT_EXPIRE_MINUTES")
            .unwrap_or_else(|_| "15".to_string())
            .parse::<i64>()
            .ok()
            .filter(|minutes| (1..=120).contains(minutes))
            .ok_or(ConfigError::InvalidPositiveInteger(
                "PAYMENT_EXPIRE_MINUTES",
            ))?;

        let epay = match (
            optional_nonempty("EPAY_GATEWAY"),
            optional_nonempty("EPAY_PID"),
            optional_nonempty("EPAY_KEY"),
        ) {
            (Some(gateway), Some(pid), Some(key)) => Some(EpayConfig { gateway, pid, key }),
            _ => None,
        };

        let wechatpay_values = [
            optional_nonempty("WXPAY_APP_ID"),
            optional_nonempty("WXPAY_MCH_ID"),
            optional_nonempty("WXPAY_MERCHANT_SERIAL_NO"),
            optional_nonempty("WXPAY_MERCHANT_PRIVATE_KEY_PATH"),
            optional_nonempty("WXPAY_API_V3_KEY"),
            optional_nonempty("WXPAY_PUBLIC_KEY_ID"),
            optional_nonempty("WXPAY_PUBLIC_KEY_PATH"),
        ];
        let configured_wechatpay_values = wechatpay_values
            .iter()
            .filter(|value| value.is_some())
            .count();
        let wechatpay = if configured_wechatpay_values == 0 {
            None
        } else if configured_wechatpay_values != wechatpay_values.len() {
            return Err(ConfigError::IncompletePaymentConfig(
                "all WXPAY_* values must be configured together",
            ));
        } else {
            if !public_base_url.starts_with("https://") {
                return Err(ConfigError::WechatPayRequiresHttps);
            }
            let [
                Some(app_id),
                Some(mch_id),
                Some(merchant_serial_no),
                Some(merchant_private_key_path),
                Some(api_v3_key),
                Some(public_key_id),
                Some(public_key_path),
            ] = wechatpay_values
            else {
                unreachable!("WeChat Pay configuration completeness already checked")
            };
            if api_v3_key.len() != 32 {
                return Err(ConfigError::IncompletePaymentConfig(
                    "WXPAY_API_V3_KEY must contain exactly 32 bytes",
                ));
            }
            Some(WechatPayConfig {
                app_id,
                mch_id,
                merchant_serial_no,
                merchant_private_key_path: PathBuf::from(merchant_private_key_path),
                api_v3_key,
                public_key_id,
                public_key_path: PathBuf::from(public_key_path),
                notify_url: format!(
                    "{}/api/payments/wechatpay/notify",
                    public_base_url.trim_end_matches('/')
                ),
            })
        };

        Ok(Self {
            listen_addr,
            database_url,
            public_base_url,
            web_return_url,
            web_dist_dir,
            admin_key,
            order_password_pepper,
            payment_expire_minutes,
            epay,
            wechatpay,
        })
    }
}

fn required(name: &'static str) -> Result<String, ConfigError> {
    env::var(name).map_err(|_| ConfigError::MissingVar(name))
}

fn optional_nonempty(name: &'static str) -> Option<String> {
    env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}
