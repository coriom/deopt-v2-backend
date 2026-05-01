use crate::error::{BackendError, Result};
use std::env;
use std::net::SocketAddr;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AppConfig {
    pub host: String,
    pub port: u16,
    pub rust_log: String,
    pub chain_id: u64,
    pub network_name: String,
    pub execution_enabled: bool,
}

impl AppConfig {
    pub fn from_env() -> Result<Self> {
        dotenvy::dotenv().ok();

        let host = env::var("HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
        let port = parse_env("PORT", "8080")?;
        let rust_log = env::var("RUST_LOG").unwrap_or_else(|_| "info".to_string());
        let chain_id = parse_env("CHAIN_ID", "84532")?;
        let network_name = env::var("NETWORK_NAME").unwrap_or_else(|_| "base-sepolia".to_string());
        let execution_enabled = parse_env("EXECUTION_ENABLED", "false")?;

        Ok(Self {
            host,
            port,
            rust_log,
            chain_id,
            network_name,
            execution_enabled,
        })
    }

    pub fn socket_addr(&self) -> Result<SocketAddr> {
        format!("{}:{}", self.host, self.port)
            .parse()
            .map_err(|error| BackendError::Config(format!("invalid socket address: {error}")))
    }
}

fn parse_env<T>(key: &str, default: &str) -> Result<T>
where
    T: std::str::FromStr,
    T::Err: std::fmt::Display,
{
    let value = env::var(key).unwrap_or_else(|_| default.to_string());
    value
        .parse()
        .map_err(|error| BackendError::Config(format!("invalid {key}: {error}")))
}
