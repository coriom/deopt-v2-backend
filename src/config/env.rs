use crate::error::{BackendError, Result};
use crate::signing::signature::SignatureVerificationMode;
use crate::signing::Eip712Domain;
use crate::types::AccountId;
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
    pub signature_verification_mode: SignatureVerificationMode,
    pub eip712_domain: Eip712Domain,
    pub persistence_enabled: bool,
    pub database_url: Option<String>,
}

impl AppConfig {
    pub fn from_env() -> Result<Self> {
        dotenvy::dotenv().ok();
        Self::from_lookup(|key| env::var(key).ok())
    }

    fn from_lookup(mut lookup: impl FnMut(&str) -> Option<String>) -> Result<Self> {
        let host = get_env(&mut lookup, "HOST", "127.0.0.1");
        let port = parse_env(&mut lookup, "PORT", "8080")?;
        let rust_log = get_env(&mut lookup, "RUST_LOG", "info");
        let chain_id = parse_env(&mut lookup, "CHAIN_ID", "84532")?;
        let network_name = get_env(&mut lookup, "NETWORK_NAME", "base-sepolia");
        let execution_enabled = parse_env(&mut lookup, "EXECUTION_ENABLED", "false")?;
        let signature_verification_mode =
            parse_env(&mut lookup, "SIGNATURE_VERIFICATION_MODE", "disabled")?;
        let eip712_domain = Eip712Domain {
            name: get_env(&mut lookup, "EIP712_NAME", "DeOptV2"),
            version: get_env(&mut lookup, "EIP712_VERSION", "1"),
            chain_id: parse_env(&mut lookup, "EIP712_CHAIN_ID", "84532")?,
            verifying_contract: AccountId::new(get_env(
                &mut lookup,
                "EIP712_VERIFYING_CONTRACT",
                "0x0000000000000000000000000000000000000000",
            )),
        };
        let persistence_enabled = parse_env(&mut lookup, "PERSISTENCE_ENABLED", "false")?;
        let database_url = lookup("DATABASE_URL").filter(|value| !value.is_empty());

        if persistence_enabled && database_url.is_none() {
            return Err(BackendError::Config(
                "DATABASE_URL is required when PERSISTENCE_ENABLED=true".to_string(),
            ));
        }

        Ok(Self {
            host,
            port,
            rust_log,
            chain_id,
            network_name,
            execution_enabled,
            signature_verification_mode,
            eip712_domain,
            persistence_enabled,
            database_url,
        })
    }

    pub fn socket_addr(&self) -> Result<SocketAddr> {
        format!("{}:{}", self.host, self.port)
            .parse()
            .map_err(|error| BackendError::Config(format!("invalid socket address: {error}")))
    }
}

fn get_env(lookup: &mut impl FnMut(&str) -> Option<String>, key: &str, default: &str) -> String {
    lookup(key).unwrap_or_else(|| default.to_string())
}

fn parse_env<T>(
    lookup: &mut impl FnMut(&str) -> Option<String>,
    key: &str,
    default: &str,
) -> Result<T>
where
    T: std::str::FromStr,
    T::Err: std::fmt::Display,
{
    let value = get_env(lookup, key, default);
    value
        .parse()
        .map_err(|error| BackendError::Config(format!("invalid {key}: {error}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn persistence_disabled_does_not_require_database_url() {
        let config = config_from_pairs([("PERSISTENCE_ENABLED", "false")]).unwrap();

        assert!(!config.persistence_enabled);
        assert_eq!(config.database_url, None);
    }

    #[test]
    fn persistence_enabled_requires_database_url() {
        let error = config_from_pairs([("PERSISTENCE_ENABLED", "true")]).unwrap_err();

        assert!(error
            .to_string()
            .contains("DATABASE_URL is required when PERSISTENCE_ENABLED=true"));
    }

    #[test]
    fn persistence_enabled_accepts_database_url() {
        let config = config_from_pairs([
            ("PERSISTENCE_ENABLED", "true"),
            (
                "DATABASE_URL",
                "postgres://deopt:deopt@127.0.0.1:5432/deopt_v2_backend",
            ),
        ])
        .unwrap();

        assert!(config.persistence_enabled);
        assert_eq!(
            config.database_url.as_deref(),
            Some("postgres://deopt:deopt@127.0.0.1:5432/deopt_v2_backend")
        );
    }

    fn config_from_pairs<const N: usize>(pairs: [(&str, &str); N]) -> Result<AppConfig> {
        let values: HashMap<String, String> = pairs
            .into_iter()
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect();
        AppConfig::from_lookup(|key| values.get(key).cloned())
    }
}
