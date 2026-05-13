use crate::admin::AdminConfig;
use crate::confirmation::ConfirmationConfig;
use crate::error::{BackendError, Result};
use crate::execution::{ExecutionConfig, PrivateKeySecret};
use crate::indexer::IndexerConfig;
use crate::mm::transport::webtransport::validate_webtransport_startup;
use crate::mm::MmGatewayConfig;
use crate::nonce_sync::PerpNonceSyncConfig;
use crate::options::OptionsConfig;
use crate::reconciliation::ReconciliationConfig;
use crate::rfq::RfqConfig;
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
    pub perp_nonce_sync: PerpNonceSyncConfig,
    pub confirmation: ConfirmationConfig,
    pub indexer: IndexerConfig,
    pub reconciliation: ReconciliationConfig,
    pub rfq: RfqConfig,
    pub options: OptionsConfig,
    pub mm_gateway: MmGatewayConfig,
    pub admin: AdminConfig,
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
        let mm_gateway = MmGatewayConfig {
            enabled: parse_env(&mut lookup, "MM_GATEWAY_ENABLED", "false")?,
            transport: parse_env(&mut lookup, "MM_GATEWAY_TRANSPORT", "webtransport")?,
            host: get_env(&mut lookup, "MM_GATEWAY_HOST", "127.0.0.1"),
            port: parse_env(&mut lookup, "MM_GATEWAY_PORT", "8443")?,
            cert_path: lookup("MM_GATEWAY_CERT_PATH").filter(|value| !value.is_empty()),
            key_path: lookup("MM_GATEWAY_KEY_PATH").filter(|value| !value.is_empty()),
            max_sessions: parse_env(&mut lookup, "MM_GATEWAY_MAX_SESSIONS", "100")?,
            max_in_flight_per_session: parse_env(
                &mut lookup,
                "MM_GATEWAY_MAX_IN_FLIGHT_PER_SESSION",
                "128",
            )?,
            rate_limit_per_sec: parse_env(&mut lookup, "MM_GATEWAY_RATE_LIMIT_PER_SEC", "100")?,
            heartbeat_timeout_ms: parse_env(
                &mut lookup,
                "MM_GATEWAY_HEARTBEAT_TIMEOUT_MS",
                "15000",
            )?,
            max_orders_per_bulk: parse_env(&mut lookup, "MM_GATEWAY_MAX_ORDERS_PER_BULK", "50")?,
            max_cancels_per_bulk: parse_env(&mut lookup, "MM_GATEWAY_MAX_CANCELS_PER_BULK", "100")?,
            max_open_orders_per_account: parse_env(
                &mut lookup,
                "MM_GATEWAY_MAX_OPEN_ORDERS_PER_ACCOUNT",
                "500",
            )?,
            cancel_on_disconnect: parse_env(
                &mut lookup,
                "MM_GATEWAY_CANCEL_ON_DISCONNECT",
                "true",
            )?,
            auth_mode: parse_env(&mut lookup, "MM_GATEWAY_AUTH_MODE", "disabled")?,
            require_auth: parse_env(&mut lookup, "MM_GATEWAY_REQUIRE_AUTH", "false")?,
        };
        let admin = AdminConfig::new(
            parse_env(&mut lookup, "ADMIN_API_ENABLED", "false")?,
            parse_env(&mut lookup, "ADMIN_API_REQUIRE_TOKEN", "false")?,
            lookup("ADMIN_API_TOKEN").filter(|value| !value.is_empty()),
        );
        let confirmation = ConfirmationConfig {
            enabled: parse_env(&mut lookup, "CONFIRMATION_ENABLED", "false")?,
            require_persistence: parse_env(
                &mut lookup,
                "CONFIRMATION_REQUIRE_PERSISTENCE",
                "true",
            )?,
            required_blocks: parse_env(&mut lookup, "CONFIRMATION_REQUIRED_BLOCKS", "2")?,
            max_batch_size: parse_env(&mut lookup, "CONFIRMATION_MAX_BATCH_SIZE", "50")?,
            require_reconciliation: parse_env(
                &mut lookup,
                "CONFIRMATION_REQUIRE_RECONCILIATION",
                "true",
            )?,
            rpc_url: execution.rpc_url.clone(),
        };
        let rfq = RfqConfig {
            enabled: parse_env(&mut lookup, "RFQ_ENABLED", "false")?,
            require_persistence: parse_env(&mut lookup, "RFQ_REQUIRE_PERSISTENCE", "true")?,
            default_ttl_ms: parse_env(&mut lookup, "RFQ_DEFAULT_TTL_MS", "5000")?,
            max_ttl_ms: parse_env(&mut lookup, "RFQ_MAX_TTL_MS", "30000")?,
            min_quote_ttl_ms: parse_env(&mut lookup, "RFQ_MIN_QUOTE_TTL_MS", "500")?,
            max_quote_ttl_ms: parse_env(&mut lookup, "RFQ_MAX_QUOTE_TTL_MS", "10000")?,
            max_quotes_per_rfq: parse_env(&mut lookup, "RFQ_MAX_QUOTES_PER_RFQ", "50")?,
            quote_signature_mode: parse_env(&mut lookup, "RFQ_QUOTE_SIGNATURE_MODE", "disabled")?,
            eip712_domain: Eip712Domain {
                name: get_env(&mut lookup, "RFQ_EIP712_NAME", "DeOptV2RFQ"),
                version: get_env(&mut lookup, "RFQ_EIP712_VERSION", "1"),
                chain_id: parse_env(&mut lookup, "RFQ_EIP712_CHAIN_ID", "84532")?,
                verifying_contract: AccountId::new(get_env(
                    &mut lookup,
                    "RFQ_EIP712_VERIFYING_CONTRACT",
                    "0x0000000000000000000000000000000000000000",
                )),
            },
        };
        let options = OptionsConfig {
            enabled: parse_env(&mut lookup, "OPTIONS_ENABLED", "false")?,
            require_persistence: parse_env(&mut lookup, "OPTIONS_REQUIRE_PERSISTENCE", "true")?,
            allow_manual_series: parse_env(&mut lookup, "OPTIONS_ALLOW_MANUAL_SERIES", "true")?,
            sync_onchain_registry: parse_env(
                &mut lookup,
                "OPTIONS_SYNC_ONCHAIN_REGISTRY",
                "false",
            )?,
            default_contract_size_1e8: parse_env(
                &mut lookup,
                "OPTIONS_DEFAULT_CONTRACT_SIZE_1E8",
                "100000000",
            )?,
            rfq_enabled: parse_env(&mut lookup, "OPTION_RFQ_ENABLED", "false")?,
            rfq_require_persistence: parse_env(
                &mut lookup,
                "OPTION_RFQ_REQUIRE_PERSISTENCE",
                "true",
            )?,
            rfq_default_ttl_ms: parse_env(&mut lookup, "OPTION_RFQ_DEFAULT_TTL_MS", "5000")?,
            rfq_max_ttl_ms: parse_env(&mut lookup, "OPTION_RFQ_MAX_TTL_MS", "30000")?,
            rfq_min_quote_ttl_ms: parse_env(&mut lookup, "OPTION_RFQ_MIN_QUOTE_TTL_MS", "500")?,
            rfq_max_quote_ttl_ms: parse_env(&mut lookup, "OPTION_RFQ_MAX_QUOTE_TTL_MS", "10000")?,
            rfq_max_quotes_per_rfq: parse_env(&mut lookup, "OPTION_RFQ_MAX_QUOTES_PER_RFQ", "50")?,
            rfq_quote_signature_mode: parse_env(
                &mut lookup,
                "OPTION_RFQ_QUOTE_SIGNATURE_MODE",
                "disabled",
            )?,
            rfq_eip712_domain: Eip712Domain {
                name: get_env(&mut lookup, "OPTION_RFQ_EIP712_NAME", "DeOptV2OptionRFQ"),
                version: get_env(&mut lookup, "OPTION_RFQ_EIP712_VERSION", "1"),
                chain_id: parse_env(&mut lookup, "OPTION_RFQ_EIP712_CHAIN_ID", "84532")?,
                verifying_contract: AccountId::new(get_env(
                    &mut lookup,
                    "OPTION_RFQ_EIP712_VERIFYING_CONTRACT",
                    "0x0000000000000000000000000000000000000000",
                )),
            },
        };
        let perp_nonce_sync = PerpNonceSyncConfig {
            enabled: parse_env(&mut lookup, "PERP_NONCE_SYNC_ENABLED", "false")?,
            require_rpc: parse_env(&mut lookup, "PERP_NONCE_SYNC_REQUIRE_RPC", "true")?,
            strict: parse_env(&mut lookup, "PERP_NONCE_SYNC_STRICT", "true")?,
            rpc_url: execution.rpc_url.clone(),
            perp_matching_engine_address: execution.perp_matching_engine_address.clone(),
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
        perp_nonce_sync.validate_startup()?;
        indexer.validate_startup(persistence_enabled)?;
        reconciliation.validate_startup(persistence_enabled)?;
        confirmation.validate_startup(persistence_enabled)?;
        rfq.validate_startup(persistence_enabled)?;
        options.validate_startup(persistence_enabled)?;
        validate_webtransport_startup(&mm_gateway)?;
        admin.validate_startup()?;

        Ok(Self {
            host,
            port,
            rust_log,
            chain_id,
            network_name,
            execution,
            perp_nonce_sync,
            confirmation,
            indexer,
            reconciliation,
            rfq,
            options,
            mm_gateway,
            admin,
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
    fn perp_nonce_sync_uses_safe_defaults() {
        let config = config_from_pairs([("PERP_NONCE_SYNC_ENABLED", "false")]).unwrap();

        assert!(!config.perp_nonce_sync.enabled);
        assert!(config.perp_nonce_sync.require_rpc);
        assert!(config.perp_nonce_sync.strict);
    }

    #[test]
    fn perp_nonce_sync_enabled_requires_rpc_when_required() {
        let error = config_from_pairs([
            ("PERP_NONCE_SYNC_ENABLED", "true"),
            ("PERP_NONCE_SYNC_REQUIRE_RPC", "true"),
            (
                "PERP_MATCHING_ENGINE_ADDRESS",
                "0x774d96E5739bffadEE91508b4D3D74F5BE29F165",
            ),
        ])
        .unwrap_err();

        assert!(error
            .to_string()
            .contains("RPC_URL is required for perp nonce sync"));
    }

    #[test]
    fn perp_nonce_sync_enabled_requires_matching_engine_when_required() {
        let error = config_from_pairs([
            ("PERP_NONCE_SYNC_ENABLED", "true"),
            ("PERP_NONCE_SYNC_REQUIRE_RPC", "true"),
            ("RPC_URL", "https://example.invalid"),
        ])
        .unwrap_err();

        assert!(error
            .to_string()
            .contains("PERP_MATCHING_ENGINE_ADDRESS is required for perp nonce sync"));
    }

    #[test]
    fn perp_nonce_sync_enabled_can_defer_rpc_validation() {
        let config = config_from_pairs([
            ("PERP_NONCE_SYNC_ENABLED", "true"),
            ("PERP_NONCE_SYNC_REQUIRE_RPC", "false"),
        ])
        .unwrap();

        assert!(config.perp_nonce_sync.enabled);
        assert!(!config.perp_nonce_sync.require_rpc);
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
    fn confirmation_config_uses_safe_defaults() {
        let config = config_from_pairs([("PERSISTENCE_ENABLED", "false")]).unwrap();

        assert!(!config.confirmation.enabled);
        assert!(config.confirmation.require_persistence);
        assert_eq!(config.confirmation.required_blocks, 2);
        assert_eq!(config.confirmation.max_batch_size, 50);
        assert!(config.confirmation.require_reconciliation);
        assert_eq!(config.confirmation.rpc_url, None);
    }

    #[test]
    fn mm_gateway_uses_safe_v1a_defaults() {
        let config = config_from_pairs([("PERSISTENCE_ENABLED", "false")]).unwrap();

        assert!(!config.mm_gateway.enabled);
        assert_eq!(
            config.mm_gateway.transport,
            crate::mm::MmGatewayTransport::WebTransport
        );
        assert_eq!(config.mm_gateway.host, "127.0.0.1");
        assert_eq!(config.mm_gateway.port, 8443);
        assert_eq!(config.mm_gateway.cert_path, None);
        assert_eq!(config.mm_gateway.key_path, None);
        assert_eq!(config.mm_gateway.max_sessions, 100);
        assert_eq!(config.mm_gateway.max_in_flight_per_session, 128);
        assert_eq!(config.mm_gateway.rate_limit_per_sec, 100);
        assert_eq!(config.mm_gateway.heartbeat_timeout_ms, 15_000);
        assert_eq!(config.mm_gateway.max_orders_per_bulk, 50);
        assert_eq!(config.mm_gateway.max_cancels_per_bulk, 100);
        assert_eq!(config.mm_gateway.max_open_orders_per_account, 500);
        assert!(config.mm_gateway.cancel_on_disconnect);
        assert_eq!(config.mm_gateway.auth_mode, crate::mm::AuthMode::Disabled);
        assert!(!config.mm_gateway.require_auth);
    }

    #[test]
    fn mm_gateway_enabled_requires_cert_path() {
        let error = config_from_pairs([
            ("PERSISTENCE_ENABLED", "false"),
            ("MM_GATEWAY_ENABLED", "true"),
        ])
        .unwrap_err();

        assert!(error.to_string().contains("MM_GATEWAY_CERT_PATH"));
    }

    #[test]
    fn mm_gateway_enabled_requires_key_path() {
        let error = config_from_pairs([
            ("PERSISTENCE_ENABLED", "false"),
            ("MM_GATEWAY_ENABLED", "true"),
            ("MM_GATEWAY_CERT_PATH", "/tmp/deopt-mm-gateway/cert.pem"),
        ])
        .unwrap_err();

        assert!(error.to_string().contains("MM_GATEWAY_KEY_PATH"));
    }

    #[test]
    fn mm_gateway_rejects_unsupported_transport_string() {
        let error = config_from_pairs([
            ("PERSISTENCE_ENABLED", "false"),
            ("MM_GATEWAY_TRANSPORT", "websocket"),
        ])
        .unwrap_err();

        assert!(error
            .to_string()
            .contains("unsupported MM_GATEWAY_TRANSPORT"));
    }

    #[test]
    fn mm_gateway_config_does_not_change_http_socket_config() {
        let config = config_from_pairs([
            ("PERSISTENCE_ENABLED", "false"),
            ("HOST", "127.0.0.1"),
            ("PORT", "18080"),
            ("MM_GATEWAY_ENABLED", "true"),
            ("MM_GATEWAY_HOST", "127.0.0.1"),
            ("MM_GATEWAY_PORT", "18443"),
            ("MM_GATEWAY_CERT_PATH", "/tmp/deopt-mm-gateway/cert.pem"),
            ("MM_GATEWAY_KEY_PATH", "/tmp/deopt-mm-gateway/key.pem"),
        ])
        .unwrap();

        assert_eq!(
            config.socket_addr().unwrap(),
            "127.0.0.1:18080".parse().unwrap()
        );
        assert_eq!(config.mm_gateway.host, "127.0.0.1");
        assert_eq!(config.mm_gateway.port, 18443);
    }

    #[test]
    fn admin_api_uses_safe_defaults() {
        let config = config_from_pairs([("PERSISTENCE_ENABLED", "false")]).unwrap();

        assert!(!config.admin.enabled);
        assert!(!config.admin.require_token);
        assert!(!config.admin.token_configured());
    }

    #[test]
    fn admin_api_token_required_needs_token() {
        let error = config_from_pairs([
            ("ADMIN_API_ENABLED", "true"),
            ("ADMIN_API_REQUIRE_TOKEN", "true"),
        ])
        .unwrap_err();

        assert!(error.to_string().contains("ADMIN_API_TOKEN is required"));
    }

    #[test]
    fn admin_api_token_is_redacted_from_debug() {
        let config = config_from_pairs([
            ("ADMIN_API_ENABLED", "true"),
            ("ADMIN_API_REQUIRE_TOKEN", "true"),
            ("ADMIN_API_TOKEN", "super-secret-admin-token"),
        ])
        .unwrap();

        let debug = format!("{:?}", config.admin);

        assert!(!debug.contains("super-secret-admin-token"));
        assert!(debug.contains("<redacted>"));
    }

    #[test]
    fn confirmation_enabled_requires_rpc_url() {
        let error = config_from_pairs([
            ("CONFIRMATION_ENABLED", "true"),
            ("CONFIRMATION_REQUIRE_PERSISTENCE", "false"),
        ])
        .unwrap_err();

        assert!(error
            .to_string()
            .contains("RPC_URL is required when CONFIRMATION_ENABLED=true"));
    }

    #[test]
    fn confirmation_requiring_persistence_rejects_persistence_disabled() {
        let error = config_from_pairs([
            ("CONFIRMATION_ENABLED", "true"),
            ("CONFIRMATION_REQUIRE_PERSISTENCE", "true"),
            ("RPC_URL", "https://example.invalid"),
            ("PERSISTENCE_ENABLED", "false"),
        ])
        .unwrap_err();

        assert!(error
            .to_string()
            .contains("confirmation requires persistence enabled"));
    }

    #[test]
    fn confirmation_enabled_rejects_reconciliation_disabled() {
        let error = config_from_pairs([
            ("CONFIRMATION_ENABLED", "true"),
            ("CONFIRMATION_REQUIRE_PERSISTENCE", "false"),
            ("CONFIRMATION_REQUIRE_RECONCILIATION", "false"),
            ("RPC_URL", "https://example.invalid"),
        ])
        .unwrap_err();

        assert!(error
            .to_string()
            .contains("CONFIRMATION_REQUIRE_RECONCILIATION must be true"));
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
    fn rfq_uses_safe_defaults() {
        let config = config_from_pairs([("RFQ_ENABLED", "false")]).unwrap();

        assert!(!config.rfq.enabled);
        assert!(config.rfq.require_persistence);
        assert_eq!(config.rfq.default_ttl_ms, 5_000);
        assert_eq!(config.rfq.max_ttl_ms, 30_000);
        assert_eq!(config.rfq.min_quote_ttl_ms, 500);
        assert_eq!(config.rfq.max_quote_ttl_ms, 10_000);
        assert_eq!(config.rfq.max_quotes_per_rfq, 50);
        assert_eq!(
            config.rfq.quote_signature_mode,
            crate::rfq::RfqQuoteSignatureMode::Disabled
        );
        assert_eq!(config.rfq.eip712_domain.name, "DeOptV2RFQ");
    }

    #[test]
    fn rfq_quote_signature_mode_accepts_strict() {
        let config = config_from_pairs([
            ("RFQ_QUOTE_SIGNATURE_MODE", "strict"),
            ("PERSISTENCE_ENABLED", "false"),
        ])
        .unwrap();

        assert_eq!(
            config.rfq.quote_signature_mode,
            crate::rfq::RfqQuoteSignatureMode::Strict
        );
    }

    #[test]
    fn rfq_quote_signature_mode_rejects_invalid_mode() {
        let error = config_from_pairs([("RFQ_QUOTE_SIGNATURE_MODE", "loose")]).unwrap_err();

        assert!(error
            .to_string()
            .contains("invalid RFQ_QUOTE_SIGNATURE_MODE"));
    }

    #[test]
    fn rfq_requiring_persistence_rejects_persistence_disabled() {
        let error = config_from_pairs([
            ("RFQ_ENABLED", "true"),
            ("RFQ_REQUIRE_PERSISTENCE", "true"),
            ("PERSISTENCE_ENABLED", "false"),
        ])
        .unwrap_err();

        assert!(error
            .to_string()
            .contains("RFQ requires persistence enabled"));
    }

    #[test]
    fn rfq_can_run_without_persistence_when_requirement_disabled() {
        let config = config_from_pairs([
            ("RFQ_ENABLED", "true"),
            ("RFQ_REQUIRE_PERSISTENCE", "false"),
            ("PERSISTENCE_ENABLED", "false"),
        ])
        .unwrap();

        assert!(config.rfq.enabled);
        assert!(!config.rfq.require_persistence);
    }

    #[test]
    fn options_use_safe_defaults() {
        let config = config_from_pairs([("OPTIONS_ENABLED", "false")]).unwrap();

        assert!(!config.options.enabled);
        assert!(config.options.require_persistence);
        assert!(config.options.allow_manual_series);
        assert!(!config.options.sync_onchain_registry);
        assert_eq!(config.options.default_contract_size_1e8, 100_000_000);
        assert!(!config.options.rfq_enabled);
        assert!(config.options.rfq_require_persistence);
        assert_eq!(config.options.rfq_default_ttl_ms, 5_000);
        assert_eq!(config.options.rfq_max_ttl_ms, 30_000);
        assert_eq!(config.options.rfq_min_quote_ttl_ms, 500);
        assert_eq!(config.options.rfq_max_quote_ttl_ms, 10_000);
        assert_eq!(config.options.rfq_max_quotes_per_rfq, 50);
        assert_eq!(
            config.options.rfq_quote_signature_mode,
            crate::options::OptionRfqQuoteSignatureMode::Disabled
        );
        assert_eq!(config.options.rfq_eip712_domain.name, "DeOptV2OptionRFQ");
    }

    #[test]
    fn option_rfq_quote_signature_mode_accepts_strict() {
        let config = config_from_pairs([("OPTION_RFQ_QUOTE_SIGNATURE_MODE", "strict")]).unwrap();

        assert_eq!(
            config.options.rfq_quote_signature_mode,
            crate::options::OptionRfqQuoteSignatureMode::Strict
        );
    }

    #[test]
    fn option_rfq_quote_signature_mode_rejects_invalid_mode() {
        let error =
            config_from_pairs([("OPTION_RFQ_QUOTE_SIGNATURE_MODE", "optional")]).unwrap_err();

        assert!(error
            .to_string()
            .contains("invalid OPTION_RFQ_QUOTE_SIGNATURE_MODE"));
    }

    #[test]
    fn options_requiring_persistence_rejects_persistence_disabled() {
        let error = config_from_pairs([
            ("OPTIONS_ENABLED", "true"),
            ("OPTIONS_REQUIRE_PERSISTENCE", "true"),
            ("PERSISTENCE_ENABLED", "false"),
        ])
        .unwrap_err();

        assert!(error
            .to_string()
            .contains("Options require persistence enabled"));
    }

    #[test]
    fn options_can_run_without_persistence_when_requirement_disabled() {
        let config = config_from_pairs([
            ("OPTIONS_ENABLED", "true"),
            ("OPTIONS_REQUIRE_PERSISTENCE", "false"),
            ("PERSISTENCE_ENABLED", "false"),
        ])
        .unwrap();

        assert!(config.options.enabled);
        assert!(!config.options.require_persistence);
    }

    #[test]
    fn option_rfq_requiring_persistence_rejects_persistence_disabled() {
        let error = config_from_pairs([
            ("OPTIONS_ENABLED", "true"),
            ("OPTIONS_REQUIRE_PERSISTENCE", "false"),
            ("OPTION_RFQ_ENABLED", "true"),
            ("OPTION_RFQ_REQUIRE_PERSISTENCE", "true"),
            ("PERSISTENCE_ENABLED", "false"),
        ])
        .unwrap_err();

        assert!(error
            .to_string()
            .contains("Option RFQ requires persistence enabled"));
    }

    #[test]
    fn option_rfq_can_run_without_persistence_when_requirement_disabled() {
        let config = config_from_pairs([
            ("OPTIONS_ENABLED", "true"),
            ("OPTIONS_REQUIRE_PERSISTENCE", "false"),
            ("OPTION_RFQ_ENABLED", "true"),
            ("OPTION_RFQ_REQUIRE_PERSISTENCE", "false"),
            ("PERSISTENCE_ENABLED", "false"),
        ])
        .unwrap();

        assert!(config.options.rfq_enabled);
        assert!(!config.options.rfq_require_persistence);
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
