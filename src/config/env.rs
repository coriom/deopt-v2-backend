use crate::error::{BackendError, Result};
use crate::execution::{ExecutionConfig, PrivateKeySecret};
use crate::indexer::IndexerConfig;
use crate::reconciliation::ReconciliationConfig;
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
    pub execution: ExecutionConfig,
    pub indexer: IndexerConfig,
    pub reconciliation: ReconciliationConfig,
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
        let execution = ExecutionConfig {
            execution_enabled: parse_env(&mut lookup, "EXECUTION_ENABLED", "false")?,
            dry_run: parse_env(&mut lookup, "EXECUTOR_DRY_RUN", "true")?,
            poll_interval_ms: parse_env(&mut lookup, "EXECUTOR_POLL_INTERVAL_MS", "1000")?,
            max_batch_size: parse_env(&mut lookup, "EXECUTOR_MAX_BATCH_SIZE", "10")?,
            real_broadcast_enabled: parse_env(
                &mut lookup,
                "EXECUTOR_REAL_BROADCAST_ENABLED",
                "false",
            )?,
            executor_private_key: lookup("EXECUTOR_PRIVATE_KEY")
                .filter(|value| !value.is_empty())
                .map(PrivateKeySecret::new),
            executor_chain_id: parse_env(&mut lookup, "EXECUTOR_CHAIN_ID", "84532")?,
            max_gas_limit: parse_env(&mut lookup, "EXECUTOR_MAX_GAS_LIMIT", "1000000")?,
            max_fee_per_gas_wei: lookup("EXECUTOR_MAX_FEE_PER_GAS_WEI")
                .filter(|value| !value.is_empty()),
            max_priority_fee_per_gas_wei: lookup("EXECUTOR_MAX_PRIORITY_FEE_PER_GAS_WEI")
                .filter(|value| !value.is_empty()),
            require_simulation_ok: parse_env(
                &mut lookup,
                "EXECUTOR_REQUIRE_SIMULATION_OK",
                "true",
            )?,
            simulation_enabled: parse_env(&mut lookup, "SIMULATION_ENABLED", "false")?,
            simulation_requires_persistence: parse_env(
                &mut lookup,
                "SIMULATION_REQUIRE_PERSISTENCE",
                "true",
            )?,
            rpc_url: lookup("RPC_URL").filter(|value| !value.is_empty()),
            executor_from_address: AccountId::new(get_env(
                &mut lookup,
                "EXECUTOR_FROM_ADDRESS",
                "0x0000000000000000000000000000000000000000",
            )),
            perp_matching_engine_address: AccountId::new(get_env(
                &mut lookup,
                "PERP_MATCHING_ENGINE_ADDRESS",
                "0x0000000000000000000000000000000000000000",
            )),
            perp_engine_address: AccountId::new(get_env(
                &mut lookup,
                "PERP_ENGINE_ADDRESS",
                "0x0000000000000000000000000000000000000000",
            )),
        };
        let indexer = IndexerConfig {
            enabled: parse_env(&mut lookup, "INDEXER_ENABLED", "false")?,
            start_block: parse_env(&mut lookup, "INDEXER_START_BLOCK", "0")?,
            poll_interval_ms: parse_env(&mut lookup, "INDEXER_POLL_INTERVAL_MS", "3000")?,
            max_block_range: parse_env(&mut lookup, "INDEXER_MAX_BLOCK_RANGE", "500")?,
            require_persistence: parse_env(&mut lookup, "INDEXER_REQUIRE_PERSISTENCE", "true")?,
            rpc_url: execution.rpc_url.clone(),
            perp_matching_engine_address: execution.perp_matching_engine_address.clone(),
        };
        let reconciliation = ReconciliationConfig {
            enabled: parse_env(&mut lookup, "RECONCILIATION_ENABLED", "false")?,
            require_persistence: parse_env(
                &mut lookup,
                "RECONCILIATION_REQUIRE_PERSISTENCE",
                "true",
            )?,
            max_batch_size: parse_env(&mut lookup, "RECONCILIATION_MAX_BATCH_SIZE", "100")?,
        };
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
        execution.validate_startup(persistence_enabled)?;
        indexer.validate_startup(persistence_enabled)?;
        reconciliation.validate_startup(persistence_enabled)?;

        Ok(Self {
            host,
            port,
            rust_log,
            chain_id,
            network_name,
            execution,
            indexer,
            reconciliation,
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

    #[test]
    fn execution_disabled_uses_dry_run_defaults() {
        let config = config_from_pairs([("EXECUTION_ENABLED", "false")]).unwrap();

        assert!(!config.execution.execution_enabled);
        assert!(config.execution.dry_run);
        assert_eq!(config.execution.poll_interval_ms, 1_000);
        assert_eq!(config.execution.max_batch_size, 10);
        assert!(!config.execution.real_broadcast_enabled);
        assert!(config.execution.executor_private_key.is_none());
        assert_eq!(config.execution.executor_chain_id, 84532);
        assert_eq!(config.execution.max_gas_limit, 1_000_000);
        assert_eq!(config.execution.max_fee_per_gas_wei, None);
        assert_eq!(config.execution.max_priority_fee_per_gas_wei, None);
        assert!(config.execution.require_simulation_ok);
        assert_eq!(config.execution.rpc_url, None);
        assert!(!config.execution.simulation_enabled);
        assert!(config.execution.simulation_requires_persistence);
    }

    #[test]
    fn real_broadcast_enabled_requires_private_key() {
        let error = config_from_pairs([
            ("EXECUTOR_REAL_BROADCAST_ENABLED", "true"),
            ("PERSISTENCE_ENABLED", "true"),
            (
                "DATABASE_URL",
                "postgres://deopt:deopt@127.0.0.1:5432/deopt_v2_backend",
            ),
        ])
        .unwrap_err();

        assert!(error
            .to_string()
            .contains("EXECUTOR_PRIVATE_KEY is required"));
    }

    #[test]
    fn real_broadcast_enabled_requires_persistence() {
        let error = config_from_pairs([("EXECUTOR_REAL_BROADCAST_ENABLED", "true")]).unwrap_err();

        assert!(error
            .to_string()
            .contains("real broadcast requires persistence enabled"));
    }

    #[test]
    fn real_broadcast_enabled_requires_rpc_url() {
        let error = config_from_pairs([
            ("EXECUTOR_REAL_BROADCAST_ENABLED", "true"),
            ("PERSISTENCE_ENABLED", "true"),
            (
                "DATABASE_URL",
                "postgres://deopt:deopt@127.0.0.1:5432/deopt_v2_backend",
            ),
            (
                "EXECUTOR_PRIVATE_KEY",
                "0x4c0883a69102937d6231471b5dbb6204fe5129617082792ae468d01a3f362318",
            ),
        ])
        .unwrap_err();

        assert!(error
            .to_string()
            .contains("RPC_URL is required when EXECUTOR_REAL_BROADCAST_ENABLED=true"));
    }

    #[test]
    fn real_broadcast_enabled_requires_fee_config() {
        let error = config_from_pairs([
            ("EXECUTOR_REAL_BROADCAST_ENABLED", "true"),
            ("PERSISTENCE_ENABLED", "true"),
            (
                "DATABASE_URL",
                "postgres://deopt:deopt@127.0.0.1:5432/deopt_v2_backend",
            ),
            (
                "EXECUTOR_PRIVATE_KEY",
                "0x4c0883a69102937d6231471b5dbb6204fe5129617082792ae468d01a3f362318",
            ),
            ("RPC_URL", "https://example.invalid"),
        ])
        .unwrap_err();

        assert!(error.to_string().contains("EXECUTOR_MAX_FEE_PER_GAS_WEI"));
    }

    #[test]
    fn real_broadcast_enabled_rejects_invalid_private_key() {
        let error = config_from_pairs([
            ("EXECUTOR_REAL_BROADCAST_ENABLED", "true"),
            ("PERSISTENCE_ENABLED", "true"),
            (
                "DATABASE_URL",
                "postgres://deopt:deopt@127.0.0.1:5432/deopt_v2_backend",
            ),
            ("EXECUTOR_PRIVATE_KEY", "0xabc"),
            ("RPC_URL", "https://example.invalid"),
            ("EXECUTOR_MAX_FEE_PER_GAS_WEI", "1000000000"),
            ("EXECUTOR_MAX_PRIORITY_FEE_PER_GAS_WEI", "100000000"),
        ])
        .unwrap_err();

        assert!(error.to_string().contains("invalid EXECUTOR_PRIVATE_KEY"));
    }

    #[test]
    fn real_broadcast_enabled_accepts_complete_static_config() {
        let config = config_from_pairs([
            ("EXECUTOR_REAL_BROADCAST_ENABLED", "true"),
            ("PERSISTENCE_ENABLED", "true"),
            (
                "DATABASE_URL",
                "postgres://deopt:deopt@127.0.0.1:5432/deopt_v2_backend",
            ),
            (
                "EXECUTOR_PRIVATE_KEY",
                "0x4c0883a69102937d6231471b5dbb6204fe5129617082792ae468d01a3f362318",
            ),
            ("RPC_URL", "https://example.invalid"),
            ("EXECUTOR_MAX_FEE_PER_GAS_WEI", "1000000000"),
            ("EXECUTOR_MAX_PRIORITY_FEE_PER_GAS_WEI", "100000000"),
        ])
        .unwrap();

        assert!(config.execution.real_broadcast_enabled);
        assert!(config.execution.executor_private_key.is_some());
    }

    #[test]
    fn private_key_is_redacted_from_execution_config_debug() {
        let config = config_from_pairs([
            ("EXECUTOR_REAL_BROADCAST_ENABLED", "true"),
            ("PERSISTENCE_ENABLED", "true"),
            (
                "DATABASE_URL",
                "postgres://deopt:deopt@127.0.0.1:5432/deopt_v2_backend",
            ),
            (
                "EXECUTOR_PRIVATE_KEY",
                "0x4c0883a69102937d6231471b5dbb6204fe5129617082792ae468d01a3f362318",
            ),
            ("RPC_URL", "https://example.invalid"),
            ("EXECUTOR_MAX_FEE_PER_GAS_WEI", "1000000000"),
            ("EXECUTOR_MAX_PRIORITY_FEE_PER_GAS_WEI", "100000000"),
        ])
        .unwrap();

        let debug = format!("{:?}", config.execution);

        assert!(!debug.contains("4c0883"));
        assert!(debug.contains("<redacted>"));
    }

    #[test]
    fn dry_run_execution_requires_persistence() {
        let error = config_from_pairs([
            ("EXECUTION_ENABLED", "true"),
            ("EXECUTOR_DRY_RUN", "true"),
            ("PERSISTENCE_ENABLED", "false"),
        ])
        .unwrap_err();

        assert!(error
            .to_string()
            .contains("executor requires persistence enabled"));
    }

    #[test]
    fn real_execution_is_rejected() {
        let error = config_from_pairs([
            ("EXECUTION_ENABLED", "true"),
            ("EXECUTOR_DRY_RUN", "false"),
            ("PERSISTENCE_ENABLED", "true"),
            (
                "DATABASE_URL",
                "postgres://deopt:deopt@127.0.0.1:5432/deopt_v2_backend",
            ),
        ])
        .unwrap_err();

        assert!(error
            .to_string()
            .contains("real on-chain execution is not implemented yet"));
    }

    #[test]
    fn dry_run_execution_with_persistence_is_accepted() {
        let config = config_from_pairs([
            ("EXECUTION_ENABLED", "true"),
            ("EXECUTOR_DRY_RUN", "true"),
            ("EXECUTOR_POLL_INTERVAL_MS", "250"),
            ("EXECUTOR_MAX_BATCH_SIZE", "3"),
            ("RPC_URL", "https://example.invalid"),
            (
                "PERP_MATCHING_ENGINE_ADDRESS",
                "0x0000000000000000000000000000000000000009",
            ),
            ("PERSISTENCE_ENABLED", "true"),
            (
                "DATABASE_URL",
                "postgres://deopt:deopt@127.0.0.1:5432/deopt_v2_backend",
            ),
        ])
        .unwrap();

        assert!(config.execution.execution_enabled);
        assert!(config.execution.dry_run);
        assert_eq!(config.execution.poll_interval_ms, 250);
        assert_eq!(config.execution.max_batch_size, 3);
        assert_eq!(
            config.execution.rpc_url.as_deref(),
            Some("https://example.invalid")
        );
    }

    #[test]
    fn simulation_enabled_requires_rpc_url() {
        let error = config_from_pairs([
            ("SIMULATION_ENABLED", "true"),
            ("SIMULATION_REQUIRE_PERSISTENCE", "false"),
        ])
        .unwrap_err();

        assert!(error
            .to_string()
            .contains("RPC_URL is required when SIMULATION_ENABLED=true"));
    }

    #[test]
    fn simulation_requiring_persistence_rejects_persistence_disabled() {
        let error = config_from_pairs([
            ("SIMULATION_ENABLED", "true"),
            ("SIMULATION_REQUIRE_PERSISTENCE", "true"),
            ("RPC_URL", "https://example.invalid"),
            ("PERSISTENCE_ENABLED", "false"),
        ])
        .unwrap_err();

        assert!(error
            .to_string()
            .contains("simulation requires persistence enabled"));
    }

    #[test]
    fn indexer_disabled_does_not_require_rpc_or_persistence() {
        let config = config_from_pairs([
            ("INDEXER_ENABLED", "false"),
            ("PERSISTENCE_ENABLED", "false"),
        ])
        .unwrap();

        assert!(!config.indexer.enabled);
        assert_eq!(config.indexer.rpc_url, None);
        assert_eq!(config.indexer.start_block, 0);
        assert_eq!(config.indexer.poll_interval_ms, 3_000);
        assert_eq!(config.indexer.max_block_range, 500);
        assert!(config.indexer.require_persistence);
    }

    #[test]
    fn reconciliation_config_disabled_by_default() {
        let config = config_from_pairs([("PERSISTENCE_ENABLED", "false")]).unwrap();

        assert!(!config.reconciliation.enabled);
        assert!(config.reconciliation.require_persistence);
        assert_eq!(config.reconciliation.max_batch_size, 100);
    }

    #[test]
    fn reconciliation_requiring_persistence_rejects_persistence_disabled() {
        let error = config_from_pairs([
            ("RECONCILIATION_ENABLED", "true"),
            ("RECONCILIATION_REQUIRE_PERSISTENCE", "true"),
            ("PERSISTENCE_ENABLED", "false"),
        ])
        .unwrap_err();

        assert!(error
            .to_string()
            .contains("reconciliation requires persistence enabled"));
    }

    #[test]
    fn indexer_enabled_requires_rpc_url() {
        let error = config_from_pairs([
            ("INDEXER_ENABLED", "true"),
            ("INDEXER_REQUIRE_PERSISTENCE", "false"),
        ])
        .unwrap_err();

        assert!(error
            .to_string()
            .contains("RPC_URL is required when INDEXER_ENABLED=true"));
    }

    #[test]
    fn indexer_requiring_persistence_rejects_persistence_disabled() {
        let error = config_from_pairs([
            ("INDEXER_ENABLED", "true"),
            ("INDEXER_REQUIRE_PERSISTENCE", "true"),
            ("RPC_URL", "https://example.invalid"),
            ("PERSISTENCE_ENABLED", "false"),
        ])
        .unwrap_err();

        assert!(error
            .to_string()
            .contains("indexer requires persistence enabled"));
    }

    fn config_from_pairs<const N: usize>(pairs: [(&str, &str); N]) -> Result<AppConfig> {
        let values: HashMap<String, String> = pairs
            .into_iter()
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect();
        AppConfig::from_lookup(|key| values.get(key).cloned())
    }
}
