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
    pub epay: Option<EpayConfig>,
}

#[derive(Debug, Clone)]
pub struct EpayConfig {
    pub gateway: String,
    pub pid: String,
    pub key: String,
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("missing required environment variable: {0}")]
    MissingVar(&'static str),
    #[error("invalid LISTEN_ADDR: {0}")]
    InvalidListenAddr(#[from] std::net::AddrParseError),
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

        let epay = match (
            optional_nonempty("EPAY_GATEWAY"),
            optional_nonempty("EPAY_PID"),
            optional_nonempty("EPAY_KEY"),
        ) {
            (Some(gateway), Some(pid), Some(key)) => Some(EpayConfig { gateway, pid, key }),
            _ => None,
        };

        Ok(Self {
            listen_addr,
            database_url,
            public_base_url,
            web_return_url,
            web_dist_dir,
            admin_key,
            order_password_pepper,
            epay,
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
