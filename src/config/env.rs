use crate::admin::{AdminConfig, MetricsConfig};
use crate::confirmation::ConfirmationConfig;
use crate::error::{BackendError, Result};
use crate::execution::{ExecutionConfig, PrivateKeySecret};
use crate::fees::{FeesConfig, OptionFeeBasis};
use crate::indexer::IndexerConfig;
use crate::mm::transport::webtransport::validate_webtransport_startup;
use crate::mm::{MmGatewayConfig, MmPermissionsConfig};
use crate::nonce_sync::{OptionNonceSyncConfig, PerpNonceSyncConfig};
use crate::options::{OptionEventIndexerConfig, OptionReconciliationConfig, OptionsConfig};
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
    pub option_nonce_sync: OptionNonceSyncConfig,
    pub option_confirmation: crate::options::OptionConfirmationConfig,
    pub option_event_indexer: OptionEventIndexerConfig,
    pub option_reconciliation: OptionReconciliationConfig,
    pub confirmation: ConfirmationConfig,
    pub indexer: IndexerConfig,
    pub reconciliation: ReconciliationConfig,
    pub rfq: RfqConfig,
    pub options: OptionsConfig,
    pub fees: FeesConfig,
    pub mm_gateway: MmGatewayConfig,
    pub mm_permissions: MmPermissionsConfig,
    pub admin: AdminConfig,
    pub metrics: MetricsConfig,
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
            challenge_ttl_ms: parse_env(&mut lookup, "MM_GATEWAY_CHALLENGE_TTL_MS", "60000")?,
        };
        let mm_permissions = MmPermissionsConfig {
            enabled: parse_env(&mut lookup, "MM_PERMISSIONS_ENABLED", "false")?,
            require_persistence: parse_env(
                &mut lookup,
                "MM_PERMISSIONS_REQUIRE_PERSISTENCE",
                "true",
            )?,
        };
        let fees = FeesConfig {
            enabled: parse_env(&mut lookup, "FEES_ENABLED", "false")?,
            require_persistence: parse_env(&mut lookup, "FEES_REQUIRE_PERSISTENCE", "true")?,
            rebates_enabled: parse_env(&mut lookup, "FEES_REBATES_ENABLED", "false")?,
            protocol_fee_recipient: get_env(&mut lookup, "FEES_PROTOCOL_FEE_RECIPIENT", "treasury"),
            default_fee_asset: get_env(&mut lookup, "FEES_DEFAULT_FEE_ASSET", "USDC"),
            option_fee_basis: parse_env(
                &mut lookup,
                "FEES_OPTION_FEE_BASIS",
                OptionFeeBasis::PremiumOrUnderlyingCapped.as_str(),
            )?,
            option_premium_cap_pct: parse_env(&mut lookup, "FEES_OPTION_PREMIUM_CAP_PCT", "10")?,
        };
        let admin = AdminConfig::new(
            parse_env(&mut lookup, "ADMIN_API_ENABLED", "false")?,
            parse_env(&mut lookup, "ADMIN_API_REQUIRE_TOKEN", "false")?,
            lookup("ADMIN_API_TOKEN").filter(|value| !value.is_empty()),
        );
        let metrics = MetricsConfig {
            enabled: parse_env(&mut lookup, "METRICS_ENABLED", "true")?,
            require_admin_token: parse_env(&mut lookup, "METRICS_REQUIRE_ADMIN_TOKEN", "false")?,
        };
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
        let option_matching_engine_address =
            AccountId::new(get_env(&mut lookup, "OPTION_MATCHING_ENGINE_ADDRESS", ""));
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
            execution_enabled: parse_env(&mut lookup, "OPTION_EXECUTION_ENABLED", "false")?,
            execution_require_persistence: parse_env(
                &mut lookup,
                "OPTION_EXECUTION_REQUIRE_PERSISTENCE",
                "true",
            )?,
            matching_engine_address: option_matching_engine_address.clone(),
            execution_signature_mode: parse_env(
                &mut lookup,
                "OPTION_EXECUTION_SIGNATURE_MODE",
                "disabled",
            )?,
            execution_eip712_domain: Eip712Domain {
                name: get_env(
                    &mut lookup,
                    "OPTION_EXECUTION_EIP712_NAME",
                    "DeOptV2-OptionMatchingEngine",
                ),
                version: get_env(&mut lookup, "OPTION_EXECUTION_EIP712_VERSION", "1"),
                chain_id: parse_env(&mut lookup, "OPTION_EXECUTION_CHAIN_ID", "84532")?,
                verifying_contract: option_matching_engine_address,
            },
            execution_default_settlement_decimals: parse_env(
                &mut lookup,
                "OPTION_EXECUTION_DEFAULT_SETTLEMENT_DECIMALS",
                "6",
            )?,
            execution_simulation_enabled: parse_env(
                &mut lookup,
                "OPTION_EXECUTION_SIMULATION_ENABLED",
                "false",
            )?,
            execution_require_rpc_for_simulation: parse_env(
                &mut lookup,
                "OPTION_EXECUTION_REQUIRE_RPC_FOR_SIMULATION",
                "true",
            )?,
            execution_simulation_gas_limit: parse_env(
                &mut lookup,
                "OPTION_EXECUTION_SIMULATION_GAS_LIMIT",
                "0",
            )?,
            execution_simulation_from: lookup("OPTION_EXECUTION_SIMULATION_FROM")
                .filter(|value| !value.is_empty())
                .map(AccountId::new),
            execution_simulation_rpc_url: execution.rpc_url.clone(),
            execution_broadcast_enabled: parse_env(
                &mut lookup,
                "OPTION_EXECUTION_BROADCAST_ENABLED",
                "false",
            )?,
            execution_require_simulation_ok: parse_env(
                &mut lookup,
                "OPTION_EXECUTION_REQUIRE_SIMULATION_OK",
                "true",
            )?,
            execution_broadcast_gas_limit: parse_env(
                &mut lookup,
                "OPTION_EXECUTION_BROADCAST_GAS_LIMIT",
                "0",
            )?,
            execution_gas_safety_bps: parse_env(
                &mut lookup,
                "OPTION_EXECUTION_GAS_SAFETY_BPS",
                &crate::options::OPTION_EXECUTION_GAS_SAFETY_BPS_DEFAULT.to_string(),
            )?,
        };
        let perp_nonce_sync = PerpNonceSyncConfig {
            enabled: parse_env(&mut lookup, "PERP_NONCE_SYNC_ENABLED", "false")?,
            require_rpc: parse_env(&mut lookup, "PERP_NONCE_SYNC_REQUIRE_RPC", "true")?,
            strict: parse_env(&mut lookup, "PERP_NONCE_SYNC_STRICT", "true")?,
            rpc_url: execution.rpc_url.clone(),
            perp_matching_engine_address: execution.perp_matching_engine_address.clone(),
        };
        let option_nonce_sync = OptionNonceSyncConfig {
            enabled: parse_env(&mut lookup, "OPTION_NONCE_SYNC_ENABLED", "false")?,
            require_rpc: parse_env(&mut lookup, "OPTION_NONCE_SYNC_REQUIRE_RPC", "true")?,
            strict: parse_env(&mut lookup, "OPTION_NONCE_SYNC_STRICT", "true")?,
            rpc_url: execution.rpc_url.clone(),
            option_matching_engine_address: options.matching_engine_address.clone(),
        };
        let option_confirmation = crate::options::OptionConfirmationConfig {
            enabled: parse_env(&mut lookup, "OPTION_CONFIRMATION_WORKER_ENABLED", "false")?,
            poll_interval_ms: parse_env(
                &mut lookup,
                "OPTION_CONFIRMATION_POLL_INTERVAL_MS",
                "15000",
            )?,
            finality_blocks: parse_env(&mut lookup, "OPTION_CONFIRMATION_FINALITY_BLOCKS", "3")?,
            batch_size: parse_env(&mut lookup, "OPTION_CONFIRMATION_BATCH_SIZE", "25")?,
            require_rpc: parse_env(&mut lookup, "OPTION_CONFIRMATION_REQUIRE_RPC", "true")?,
            rpc_url: execution.rpc_url.clone(),
        };
        let option_event_indexer_matching_engine_address =
            optional_env(&mut lookup, "OPTION_EVENT_INDEXER_MATCHING_ENGINE_ADDRESS")
                .map(AccountId::new)
                .unwrap_or_else(|| options.matching_engine_address.clone());
        let option_event_indexer_margin_engine_address =
            optional_env(&mut lookup, "OPTION_EVENT_INDEXER_MARGIN_ENGINE_ADDRESS")
                .or_else(|| optional_env(&mut lookup, "MARGIN_ENGINE"))
                .map(AccountId::new)
                .unwrap_or_else(|| AccountId::new(""));
        let option_event_indexer_collateral_vault_address =
            optional_env(&mut lookup, "OPTION_EVENT_INDEXER_COLLATERAL_VAULT_ADDRESS")
                .or_else(|| optional_env(&mut lookup, "COLLATERAL_VAULT"))
                .map(AccountId::new)
                .unwrap_or_else(|| AccountId::new(""));
        let option_event_indexer_fees_manager_address =
            optional_env(&mut lookup, "OPTION_EVENT_INDEXER_FEES_MANAGER_ADDRESS")
                .or_else(|| optional_env(&mut lookup, "FEES_MANAGER"))
                .map(AccountId::new);
        let option_reconciliation = OptionReconciliationConfig {
            enabled: parse_env(&mut lookup, "OPTION_RECONCILIATION_WORKER_ENABLED", "false")?,
            poll_interval_ms: parse_env(
                &mut lookup,
                "OPTION_RECONCILIATION_POLL_INTERVAL_MS",
                "15000",
            )?,
            batch_size: parse_env(&mut lookup, "OPTION_RECONCILIATION_BATCH_SIZE", "25")?,
            require_events: parse_env(&mut lookup, "OPTION_RECONCILIATION_REQUIRE_EVENTS", "true")?,
            require_rpc: parse_env(&mut lookup, "OPTION_RECONCILIATION_REQUIRE_RPC", "true")?,
            strict: parse_env(&mut lookup, "OPTION_RECONCILIATION_STRICT", "true")?,
            rpc_url: execution.rpc_url.clone(),
        };
        let option_event_indexer = OptionEventIndexerConfig {
            enabled: parse_env(&mut lookup, "OPTION_EVENT_INDEXER_ENABLED", "false")?,
            poll_interval_ms: parse_env(
                &mut lookup,
                "OPTION_EVENT_INDEXER_POLL_INTERVAL_MS",
                "15000",
            )?,
            from_block: parse_env(&mut lookup, "OPTION_EVENT_INDEXER_FROM_BLOCK", "0")?,
            batch_blocks: parse_env(&mut lookup, "OPTION_EVENT_INDEXER_BATCH_BLOCKS", "1000")?,
            confirmation_blocks: parse_env(
                &mut lookup,
                "OPTION_EVENT_INDEXER_CONFIRMATION_BLOCKS",
                "3",
            )?,
            require_rpc: parse_env(&mut lookup, "OPTION_EVENT_INDEXER_REQUIRE_RPC", "true")?,
            rpc_url: execution.rpc_url.clone(),
            matching_engine_address: option_event_indexer_matching_engine_address,
            margin_engine_address: option_event_indexer_margin_engine_address,
            collateral_vault_address: option_event_indexer_collateral_vault_address,
            fees_manager_address: option_event_indexer_fees_manager_address,
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
        option_nonce_sync.validate_startup()?;
        option_confirmation.validate_startup(persistence_enabled)?;
        option_event_indexer.validate_startup(persistence_enabled)?;
        option_reconciliation.validate_startup(persistence_enabled)?;
        indexer.validate_startup(persistence_enabled)?;
        reconciliation.validate_startup(persistence_enabled)?;
        confirmation.validate_startup(persistence_enabled)?;
        rfq.validate_startup(persistence_enabled)?;
        options.validate_startup(persistence_enabled)?;
        validate_option_execution_broadcast_startup(&options, &execution)?;
        mm_permissions.validate_startup(persistence_enabled)?;
        fees.validate_startup(persistence_enabled)?;
        validate_webtransport_startup(&mm_gateway)?;
        admin.validate_startup()?;
        metrics.validate_startup(&admin)?;

        Ok(Self {
            host,
            port,
            rust_log,
            chain_id,
            network_name,
            execution,
            perp_nonce_sync,
            option_nonce_sync,
            option_confirmation,
            option_event_indexer,
            option_reconciliation,
            confirmation,
            indexer,
            reconciliation,
            rfq,
            options,
            fees,
            mm_gateway,
            mm_permissions,
            admin,
            metrics,
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

fn optional_env(lookup: &mut impl FnMut(&str) -> Option<String>, key: &str) -> Option<String> {
    lookup(key).filter(|value| !value.is_empty())
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

fn validate_option_execution_broadcast_startup(
    options: &OptionsConfig,
    execution: &ExecutionConfig,
) -> Result<()> {
    if !options.execution_broadcast_enabled {
        return Ok(());
    }
    if !execution.execution_enabled {
        return Err(BackendError::Config(
            "OPTION_EXECUTION_BROADCAST_ENABLED=true requires EXECUTION_ENABLED=true".to_string(),
        ));
    }
    if !execution.real_broadcast_enabled {
        return Err(BackendError::Config(
            "OPTION_EXECUTION_BROADCAST_ENABLED=true requires EXECUTOR_REAL_BROADCAST_ENABLED=true"
                .to_string(),
        ));
    }
    if execution.executor_private_key.is_none() {
        return Err(BackendError::Config(
            "EXECUTOR_PRIVATE_KEY is required when OPTION_EXECUTION_BROADCAST_ENABLED=true"
                .to_string(),
        ));
    }
    if execution.rpc_url.is_none() {
        return Err(BackendError::Config(
            "RPC_URL is required when OPTION_EXECUTION_BROADCAST_ENABLED=true".to_string(),
        ));
    }
    Ok(())
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
    fn fees_use_safe_disabled_defaults() {
        let config = config_from_pairs([("FEES_ENABLED", "false")]).unwrap();

        assert!(!config.fees.enabled);
        assert!(config.fees.require_persistence);
        assert!(!config.fees.rebates_enabled);
        assert_eq!(config.fees.protocol_fee_recipient, "treasury");
        assert_eq!(config.fees.default_fee_asset, "USDC");
        assert_eq!(
            config.fees.option_fee_basis,
            OptionFeeBasis::PremiumOrUnderlyingCapped
        );
        assert_eq!(config.fees.option_premium_cap_pct, 10);
    }

    #[test]
    fn fees_enabled_requires_persistence_by_default() {
        let error = config_from_pairs([("FEES_ENABLED", "true")]).unwrap_err();

        assert!(error
            .to_string()
            .contains("Fees require persistence enabled"));
    }

    #[test]
    fn fees_enabled_can_run_in_memory_when_requirement_disabled() {
        let config = config_from_pairs([
            ("FEES_ENABLED", "true"),
            ("FEES_REQUIRE_PERSISTENCE", "false"),
            ("FEES_REBATES_ENABLED", "true"),
            ("FEES_PROTOCOL_FEE_RECIPIENT", "local-treasury"),
            ("FEES_DEFAULT_FEE_ASSET", "USDC"),
            ("FEES_OPTION_PREMIUM_CAP_PCT", "10"),
        ])
        .unwrap();

        assert!(config.fees.enabled);
        assert!(!config.fees.require_persistence);
        assert!(config.fees.rebates_enabled);
        assert_eq!(config.fees.protocol_fee_recipient, "local-treasury");
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
    fn option_nonce_sync_uses_safe_defaults() {
        let config = config_from_pairs([("OPTION_NONCE_SYNC_ENABLED", "false")]).unwrap();

        assert!(!config.option_nonce_sync.enabled);
        assert!(config.option_nonce_sync.require_rpc);
        assert!(config.option_nonce_sync.strict);
        assert_eq!(config.option_nonce_sync.rpc_url, None);
        assert_eq!(
            config.option_nonce_sync.option_matching_engine_address.0,
            ""
        );
    }

    #[test]
    fn option_execution_broadcast_uses_safe_defaults() {
        let config = config_from_pairs([("OPTION_EXECUTION_BROADCAST_ENABLED", "false")]).unwrap();

        assert!(!config.options.execution_broadcast_enabled);
        assert!(config.options.execution_require_simulation_ok);
        assert_eq!(config.options.execution_broadcast_gas_limit, 0);
        assert_eq!(
            config.options.execution_gas_safety_bps,
            crate::options::OPTION_EXECUTION_GAS_SAFETY_BPS_DEFAULT
        );
    }

    #[test]
    fn option_execution_gas_safety_bps_parses_override() {
        let config = config_from_pairs([("OPTION_EXECUTION_GAS_SAFETY_BPS", "13000")]).unwrap();
        assert_eq!(config.options.execution_gas_safety_bps, 13_000);
    }

    #[test]
    fn option_confirmation_worker_uses_safe_defaults() {
        let config = config_from_pairs([("OPTION_CONFIRMATION_WORKER_ENABLED", "false")]).unwrap();
        assert!(!config.option_confirmation.enabled);
        assert_eq!(config.option_confirmation.poll_interval_ms, 15_000);
        assert_eq!(config.option_confirmation.finality_blocks, 3);
        assert_eq!(config.option_confirmation.batch_size, 25);
        assert!(config.option_confirmation.require_rpc);
    }

    #[test]
    fn option_confirmation_worker_parses_overrides() {
        let config = config_from_pairs([
            ("OPTION_CONFIRMATION_WORKER_ENABLED", "true"),
            ("OPTION_CONFIRMATION_POLL_INTERVAL_MS", "5000"),
            ("OPTION_CONFIRMATION_FINALITY_BLOCKS", "7"),
            ("OPTION_CONFIRMATION_BATCH_SIZE", "50"),
            ("OPTION_CONFIRMATION_REQUIRE_RPC", "true"),
            ("RPC_URL", "https://example.invalid"),
            ("PERSISTENCE_ENABLED", "true"),
            ("DATABASE_URL", "postgres://example.invalid/db"),
        ])
        .unwrap();
        assert!(config.option_confirmation.enabled);
        assert_eq!(config.option_confirmation.poll_interval_ms, 5_000);
        assert_eq!(config.option_confirmation.finality_blocks, 7);
        assert_eq!(config.option_confirmation.batch_size, 50);
        assert!(config.option_confirmation.require_rpc);
        assert_eq!(
            config.option_confirmation.rpc_url.as_deref(),
            Some("https://example.invalid")
        );
    }

    #[test]
    fn option_confirmation_worker_rejects_when_persistence_disabled() {
        let error = config_from_pairs([
            ("OPTION_CONFIRMATION_WORKER_ENABLED", "true"),
            ("OPTION_CONFIRMATION_REQUIRE_RPC", "false"),
            ("PERSISTENCE_ENABLED", "false"),
        ])
        .unwrap_err();
        assert!(error
            .to_string()
            .contains("option confirmation worker requires persistence"));
    }

    #[test]
    fn option_confirmation_worker_rejects_when_rpc_required_but_missing() {
        let error = config_from_pairs([
            ("OPTION_CONFIRMATION_WORKER_ENABLED", "true"),
            ("OPTION_CONFIRMATION_REQUIRE_RPC", "true"),
            ("PERSISTENCE_ENABLED", "true"),
            ("DATABASE_URL", "postgres://example.invalid/db"),
        ])
        .unwrap_err();
        assert!(error
            .to_string()
            .contains("RPC_URL is required when OPTION_CONFIRMATION_WORKER_ENABLED=true"));
    }

    #[test]
    fn option_event_indexer_uses_safe_defaults() {
        let config = config_from_pairs([("OPTION_EVENT_INDEXER_ENABLED", "false")]).unwrap();

        assert!(!config.option_event_indexer.enabled);
        assert_eq!(config.option_event_indexer.poll_interval_ms, 15_000);
        assert_eq!(config.option_event_indexer.from_block, 0);
        assert_eq!(config.option_event_indexer.batch_blocks, 1_000);
        assert_eq!(config.option_event_indexer.confirmation_blocks, 3);
        assert!(config.option_event_indexer.require_rpc);
        assert_eq!(config.option_event_indexer.matching_engine_address.0, "");
        assert_eq!(config.option_event_indexer.margin_engine_address.0, "");
        assert_eq!(config.option_event_indexer.collateral_vault_address.0, "");
        assert!(config.option_event_indexer.fees_manager_address.is_none());
    }

    #[test]
    fn option_event_indexer_parses_overrides() {
        let config = config_from_pairs([
            ("OPTION_EVENT_INDEXER_ENABLED", "true"),
            ("OPTION_EVENT_INDEXER_POLL_INTERVAL_MS", "5000"),
            ("OPTION_EVENT_INDEXER_FROM_BLOCK", "41856960"),
            ("OPTION_EVENT_INDEXER_BATCH_BLOCKS", "25"),
            ("OPTION_EVENT_INDEXER_CONFIRMATION_BLOCKS", "7"),
            ("OPTION_EVENT_INDEXER_REQUIRE_RPC", "true"),
            (
                "OPTION_MATCHING_ENGINE_ADDRESS",
                "0x00000000000000000000000000000000000000ee",
            ),
            (
                "OPTION_EVENT_INDEXER_MARGIN_ENGINE_ADDRESS",
                "0x00000000000000000000000000000000000000aa",
            ),
            (
                "OPTION_EVENT_INDEXER_COLLATERAL_VAULT_ADDRESS",
                "0x00000000000000000000000000000000000000bb",
            ),
            (
                "OPTION_EVENT_INDEXER_FEES_MANAGER_ADDRESS",
                "0x00000000000000000000000000000000000000cc",
            ),
            ("RPC_URL", "https://example.invalid"),
            ("PERSISTENCE_ENABLED", "true"),
            ("DATABASE_URL", "postgres://example.invalid/db"),
        ])
        .unwrap();
        assert!(config.option_event_indexer.enabled);
        assert_eq!(config.option_event_indexer.poll_interval_ms, 5_000);
        assert_eq!(config.option_event_indexer.from_block, 41_856_960);
        assert_eq!(config.option_event_indexer.batch_blocks, 25);
        assert_eq!(config.option_event_indexer.confirmation_blocks, 7);
        assert_eq!(
            config.option_event_indexer.rpc_url.as_deref(),
            Some("https://example.invalid")
        );
        assert_eq!(
            config.option_event_indexer.matching_engine_address.0,
            "0x00000000000000000000000000000000000000ee"
        );
        assert_eq!(
            config.option_event_indexer.margin_engine_address.0,
            "0x00000000000000000000000000000000000000aa"
        );
        assert_eq!(
            config.option_event_indexer.collateral_vault_address.0,
            "0x00000000000000000000000000000000000000bb"
        );
        assert_eq!(
            config
                .option_event_indexer
                .fees_manager_address
                .as_ref()
                .map(|address| address.0.as_str()),
            Some("0x00000000000000000000000000000000000000cc")
        );
    }

    #[test]
    fn option_event_indexer_accepts_generic_core_address_fallbacks() {
        let config = config_from_pairs([
            ("OPTION_EVENT_INDEXER_ENABLED", "true"),
            ("OPTION_EVENT_INDEXER_REQUIRE_RPC", "false"),
            (
                "OPTION_EVENT_INDEXER_MATCHING_ENGINE_ADDRESS",
                "0x00000000000000000000000000000000000000ee",
            ),
            (
                "MARGIN_ENGINE",
                "0x00000000000000000000000000000000000000aa",
            ),
            (
                "COLLATERAL_VAULT",
                "0x00000000000000000000000000000000000000bb",
            ),
            ("FEES_MANAGER", "0x00000000000000000000000000000000000000cc"),
            ("PERSISTENCE_ENABLED", "true"),
            ("DATABASE_URL", "postgres://example.invalid/db"),
        ])
        .unwrap();

        assert_eq!(
            config.option_event_indexer.margin_engine_address.0,
            "0x00000000000000000000000000000000000000aa"
        );
        assert_eq!(
            config.option_event_indexer.collateral_vault_address.0,
            "0x00000000000000000000000000000000000000bb"
        );
        assert!(config.option_event_indexer.fees_manager_address.is_some());
    }

    #[test]
    fn option_event_indexer_enabled_requires_required_emitters() {
        let error = config_from_pairs([
            ("OPTION_EVENT_INDEXER_ENABLED", "true"),
            ("OPTION_EVENT_INDEXER_REQUIRE_RPC", "false"),
            (
                "OPTION_EVENT_INDEXER_MATCHING_ENGINE_ADDRESS",
                "0x00000000000000000000000000000000000000ee",
            ),
            ("PERSISTENCE_ENABLED", "true"),
            ("DATABASE_URL", "postgres://example.invalid/db"),
        ])
        .unwrap_err();

        assert!(error
            .to_string()
            .contains("OPTION_EVENT_INDEXER_MARGIN_ENGINE_ADDRESS is required"));
    }

    #[test]
    fn option_event_indexer_rejects_when_persistence_disabled() {
        let error = config_from_pairs([
            ("OPTION_EVENT_INDEXER_ENABLED", "true"),
            ("OPTION_EVENT_INDEXER_REQUIRE_RPC", "false"),
            (
                "OPTION_MATCHING_ENGINE_ADDRESS",
                "0x00000000000000000000000000000000000000ee",
            ),
            ("PERSISTENCE_ENABLED", "false"),
        ])
        .unwrap_err();
        assert!(error
            .to_string()
            .contains("option event indexer requires persistence"));
    }

    #[test]
    fn option_event_indexer_rejects_when_rpc_required_but_missing() {
        let error = config_from_pairs([
            ("OPTION_EVENT_INDEXER_ENABLED", "true"),
            ("OPTION_EVENT_INDEXER_REQUIRE_RPC", "true"),
            (
                "OPTION_MATCHING_ENGINE_ADDRESS",
                "0x00000000000000000000000000000000000000ee",
            ),
            ("PERSISTENCE_ENABLED", "true"),
            ("DATABASE_URL", "postgres://example.invalid/db"),
        ])
        .unwrap_err();
        assert!(error
            .to_string()
            .contains("RPC_URL is required when OPTION_EVENT_INDEXER_ENABLED=true"));
    }

    #[test]
    fn option_execution_gas_safety_bps_rejects_below_no_margin_floor() {
        let error = config_from_pairs([
            ("OPTIONS_ENABLED", "true"),
            ("OPTIONS_REQUIRE_PERSISTENCE", "false"),
            ("OPTION_EXECUTION_ENABLED", "true"),
            ("OPTION_EXECUTION_REQUIRE_PERSISTENCE", "false"),
            (
                "OPTION_MATCHING_ENGINE_ADDRESS",
                "0x00000000000000000000000000000000000000ee",
            ),
            ("OPTION_EXECUTION_GAS_SAFETY_BPS", "9999"),
            ("PERSISTENCE_ENABLED", "false"),
        ])
        .unwrap_err();
        assert!(error
            .to_string()
            .contains("OPTION_EXECUTION_GAS_SAFETY_BPS must be >="));
    }

    #[test]
    fn option_execution_gas_safety_bps_rejects_above_ceiling() {
        let error = config_from_pairs([
            ("OPTIONS_ENABLED", "true"),
            ("OPTIONS_REQUIRE_PERSISTENCE", "false"),
            ("OPTION_EXECUTION_ENABLED", "true"),
            ("OPTION_EXECUTION_REQUIRE_PERSISTENCE", "false"),
            (
                "OPTION_MATCHING_ENGINE_ADDRESS",
                "0x00000000000000000000000000000000000000ee",
            ),
            ("OPTION_EXECUTION_GAS_SAFETY_BPS", "50001"),
            ("PERSISTENCE_ENABLED", "false"),
        ])
        .unwrap_err();
        assert!(error
            .to_string()
            .contains("OPTION_EXECUTION_GAS_SAFETY_BPS must be <="));
    }

    #[test]
    fn option_execution_broadcast_enabled_requires_execution_flags() {
        let error = config_from_pairs([
            ("OPTIONS_ENABLED", "true"),
            ("OPTIONS_REQUIRE_PERSISTENCE", "false"),
            ("OPTION_EXECUTION_ENABLED", "true"),
            ("OPTION_EXECUTION_REQUIRE_PERSISTENCE", "false"),
            (
                "OPTION_MATCHING_ENGINE_ADDRESS",
                "0x00000000000000000000000000000000000000ee",
            ),
            ("OPTION_EXECUTION_BROADCAST_ENABLED", "true"),
        ])
        .unwrap_err();

        assert!(error
            .to_string()
            .contains("requires EXECUTION_ENABLED=true"));
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
    fn option_nonce_sync_enabled_requires_rpc_when_required() {
        let error = config_from_pairs([
            ("OPTION_NONCE_SYNC_ENABLED", "true"),
            ("OPTION_NONCE_SYNC_REQUIRE_RPC", "true"),
            (
                "OPTION_MATCHING_ENGINE_ADDRESS",
                "0x00000000000000000000000000000000000000ee",
            ),
        ])
        .unwrap_err();

        assert!(error
            .to_string()
            .contains("RPC_URL is required for option nonce sync"));
    }

    #[test]
    fn option_nonce_sync_enabled_requires_matching_engine_when_required() {
        let error = config_from_pairs([
            ("OPTION_NONCE_SYNC_ENABLED", "true"),
            ("OPTION_NONCE_SYNC_REQUIRE_RPC", "true"),
            ("RPC_URL", "https://example.invalid"),
        ])
        .unwrap_err();

        assert!(error
            .to_string()
            .contains("OPTION_MATCHING_ENGINE_ADDRESS is required for option nonce sync"));
    }

    #[test]
    fn option_nonce_sync_enabled_can_defer_rpc_validation() {
        let config = config_from_pairs([
            ("OPTION_NONCE_SYNC_ENABLED", "true"),
            ("OPTION_NONCE_SYNC_REQUIRE_RPC", "false"),
        ])
        .unwrap();

        assert!(config.option_nonce_sync.enabled);
        assert!(!config.option_nonce_sync.require_rpc);
        assert!(config.option_nonce_sync.strict);
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
    fn real_execution_without_dry_run_is_accepted_for_manual_broadcast_paths() {
        let config = config_from_pairs([
            ("EXECUTION_ENABLED", "true"),
            ("EXECUTOR_DRY_RUN", "false"),
            ("PERSISTENCE_ENABLED", "true"),
            (
                "DATABASE_URL",
                "postgres://deopt:deopt@127.0.0.1:5432/deopt_v2_backend",
            ),
        ])
        .unwrap();

        assert!(config.execution.execution_enabled);
        assert!(!config.execution.dry_run);
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
        assert_eq!(config.mm_gateway.challenge_ttl_ms, 60_000);
    }

    #[test]
    fn mm_gateway_auth_mode_accepts_wallet_challenge() {
        let config = config_from_pairs([
            ("PERSISTENCE_ENABLED", "false"),
            ("MM_GATEWAY_AUTH_MODE", "wallet_challenge"),
            ("MM_GATEWAY_CHALLENGE_TTL_MS", "120000"),
        ])
        .unwrap();

        assert_eq!(
            config.mm_gateway.auth_mode,
            crate::mm::AuthMode::WalletChallenge
        );
        assert_eq!(config.mm_gateway.challenge_ttl_ms, 120_000);
    }

    #[test]
    fn mm_gateway_auth_mode_rejects_invalid_mode() {
        let error = config_from_pairs([("MM_GATEWAY_AUTH_MODE", "token")]).unwrap_err();

        assert!(error
            .to_string()
            .contains("unsupported MM_GATEWAY_AUTH_MODE"));
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
    fn mm_permissions_use_safe_defaults() {
        let config = config_from_pairs([("PERSISTENCE_ENABLED", "false")]).unwrap();

        assert!(!config.mm_permissions.enabled);
        assert!(config.mm_permissions.require_persistence);
    }

    #[test]
    fn mm_permissions_requiring_persistence_rejects_persistence_disabled() {
        let error = config_from_pairs([
            ("PERSISTENCE_ENABLED", "false"),
            ("MM_PERMISSIONS_ENABLED", "true"),
        ])
        .unwrap_err();

        assert!(error
            .to_string()
            .contains("MM permissions require persistence enabled"));
    }

    #[test]
    fn mm_permissions_can_run_without_persistence_when_requirement_disabled() {
        let config = config_from_pairs([
            ("PERSISTENCE_ENABLED", "false"),
            ("MM_PERMISSIONS_ENABLED", "true"),
            ("MM_PERMISSIONS_REQUIRE_PERSISTENCE", "false"),
        ])
        .unwrap();

        assert!(config.mm_permissions.enabled);
        assert!(!config.mm_permissions.require_persistence);
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
    fn metrics_use_safe_defaults() {
        let config = config_from_pairs([("PERSISTENCE_ENABLED", "false")]).unwrap();

        assert!(config.metrics.enabled);
        assert!(!config.metrics.require_admin_token);
    }

    #[test]
    fn metrics_can_be_disabled() {
        let config = config_from_pairs([("METRICS_ENABLED", "false")]).unwrap();

        assert!(!config.metrics.enabled);
        assert!(!config.metrics.require_admin_token);
    }

    #[test]
    fn metrics_token_requirement_reuses_admin_token() {
        let config = config_from_pairs([
            ("METRICS_ENABLED", "true"),
            ("METRICS_REQUIRE_ADMIN_TOKEN", "true"),
            ("ADMIN_API_TOKEN", "metrics-admin-token"),
        ])
        .unwrap();

        assert!(config.metrics.enabled);
        assert!(config.metrics.require_admin_token);
        assert!(config.admin.token_configured());
    }

    #[test]
    fn metrics_token_requirement_requires_admin_token() {
        let error = config_from_pairs([
            ("METRICS_ENABLED", "true"),
            ("METRICS_REQUIRE_ADMIN_TOKEN", "true"),
        ])
        .unwrap_err();

        assert!(error
            .to_string()
            .contains("ADMIN_API_TOKEN is required when METRICS_REQUIRE_ADMIN_TOKEN=true"));
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
        assert!(!config.options.execution_enabled);
        assert!(config.options.execution_require_persistence);
        assert_eq!(config.options.matching_engine_address.0, "");
        assert_eq!(
            config.options.execution_signature_mode,
            crate::options::OptionExecutionSignatureMode::Disabled
        );
        assert_eq!(
            config.options.execution_eip712_domain.name,
            "DeOptV2-OptionMatchingEngine"
        );
        assert_eq!(config.options.execution_eip712_domain.version, "1");
        assert_eq!(config.options.execution_eip712_domain.chain_id, 84532);
        assert_eq!(config.options.execution_default_settlement_decimals, 6);
        assert!(!config.options.execution_simulation_enabled);
        assert!(config.options.execution_require_rpc_for_simulation);
        assert_eq!(config.options.execution_simulation_gas_limit, 0);
        assert_eq!(config.options.execution_simulation_from, None);
        assert_eq!(config.options.execution_simulation_rpc_url, None);
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
    fn option_execution_requiring_persistence_rejects_persistence_disabled() {
        let error = config_from_pairs([
            ("OPTIONS_ENABLED", "true"),
            ("OPTIONS_REQUIRE_PERSISTENCE", "false"),
            ("OPTION_EXECUTION_ENABLED", "true"),
            ("OPTION_EXECUTION_REQUIRE_PERSISTENCE", "true"),
            (
                "OPTION_MATCHING_ENGINE_ADDRESS",
                "0x00000000000000000000000000000000000000ee",
            ),
            ("PERSISTENCE_ENABLED", "false"),
        ])
        .unwrap_err();

        assert!(error
            .to_string()
            .contains("Option execution requires persistence enabled"));
    }

    #[test]
    fn option_execution_requires_nonzero_matching_engine_address() {
        let error = config_from_pairs([
            ("OPTIONS_ENABLED", "true"),
            ("OPTIONS_REQUIRE_PERSISTENCE", "false"),
            ("OPTION_EXECUTION_ENABLED", "true"),
            ("OPTION_EXECUTION_REQUIRE_PERSISTENCE", "false"),
            (
                "OPTION_MATCHING_ENGINE_ADDRESS",
                "0x0000000000000000000000000000000000000000",
            ),
            ("PERSISTENCE_ENABLED", "false"),
        ])
        .unwrap_err();

        assert!(error
            .to_string()
            .contains("OPTION_MATCHING_ENGINE_ADDRESS must be nonzero"));
    }

    #[test]
    fn option_execution_can_run_without_persistence_when_requirement_disabled() {
        let config = config_from_pairs([
            ("OPTIONS_ENABLED", "true"),
            ("OPTIONS_REQUIRE_PERSISTENCE", "false"),
            ("OPTION_EXECUTION_ENABLED", "true"),
            ("OPTION_EXECUTION_REQUIRE_PERSISTENCE", "false"),
            ("OPTION_EXECUTION_SIGNATURE_MODE", "strict"),
            ("OPTION_EXECUTION_CHAIN_ID", "31337"),
            ("OPTION_EXECUTION_DEFAULT_SETTLEMENT_DECIMALS", "18"),
            ("OPTION_EXECUTION_SIMULATION_ENABLED", "true"),
            ("OPTION_EXECUTION_REQUIRE_RPC_FOR_SIMULATION", "true"),
            ("OPTION_EXECUTION_SIMULATION_GAS_LIMIT", "500000"),
            (
                "OPTION_EXECUTION_SIMULATION_FROM",
                "0x00000000000000000000000000000000000000aa",
            ),
            ("RPC_URL", "https://example.invalid"),
            (
                "OPTION_MATCHING_ENGINE_ADDRESS",
                "0x00000000000000000000000000000000000000ee",
            ),
            ("PERSISTENCE_ENABLED", "false"),
        ])
        .unwrap();

        assert!(config.options.execution_enabled);
        assert!(!config.options.execution_require_persistence);
        assert_eq!(
            config.options.execution_signature_mode,
            crate::options::OptionExecutionSignatureMode::Strict
        );
        assert_eq!(config.options.execution_eip712_domain.chain_id, 31337);
        assert_eq!(
            config.options.execution_eip712_domain.verifying_contract.0,
            "0x00000000000000000000000000000000000000ee"
        );
        assert_eq!(config.options.execution_default_settlement_decimals, 18);
        assert!(config.options.execution_simulation_enabled);
        assert!(config.options.execution_require_rpc_for_simulation);
        assert_eq!(config.options.execution_simulation_gas_limit, 500_000);
        assert_eq!(
            config
                .options
                .execution_simulation_from
                .as_ref()
                .map(|account| account.0.as_str()),
            Some("0x00000000000000000000000000000000000000aa")
        );
        assert_eq!(
            config.options.execution_simulation_rpc_url.as_deref(),
            Some("https://example.invalid")
        );
    }

    #[test]
    fn option_execution_simulation_requires_rpc_when_required() {
        let error = config_from_pairs([
            ("OPTIONS_ENABLED", "true"),
            ("OPTIONS_REQUIRE_PERSISTENCE", "false"),
            ("OPTION_EXECUTION_SIMULATION_ENABLED", "true"),
            ("OPTION_EXECUTION_REQUIRE_RPC_FOR_SIMULATION", "true"),
            ("PERSISTENCE_ENABLED", "false"),
        ])
        .unwrap_err();

        assert!(error
            .to_string()
            .contains("RPC_URL is required when OPTION_EXECUTION_SIMULATION_ENABLED=true"));
    }

    #[test]
    fn option_execution_simulation_can_defer_rpc_requirement() {
        let config = config_from_pairs([
            ("OPTIONS_ENABLED", "true"),
            ("OPTIONS_REQUIRE_PERSISTENCE", "false"),
            ("OPTION_EXECUTION_SIMULATION_ENABLED", "true"),
            ("OPTION_EXECUTION_REQUIRE_RPC_FOR_SIMULATION", "false"),
            ("PERSISTENCE_ENABLED", "false"),
        ])
        .unwrap();

        assert!(config.options.execution_simulation_enabled);
        assert!(!config.options.execution_require_rpc_for_simulation);
        assert_eq!(config.options.execution_simulation_rpc_url, None);
    }

    #[test]
    fn option_execution_simulation_rejects_invalid_from_address() {
        let error = config_from_pairs([
            ("OPTIONS_ENABLED", "true"),
            ("OPTIONS_REQUIRE_PERSISTENCE", "false"),
            ("OPTION_EXECUTION_SIMULATION_ENABLED", "true"),
            ("OPTION_EXECUTION_REQUIRE_RPC_FOR_SIMULATION", "false"),
            ("OPTION_EXECUTION_SIMULATION_FROM", "not-an-address"),
            ("PERSISTENCE_ENABLED", "false"),
        ])
        .unwrap_err();

        assert!(error
            .to_string()
            .contains("OPTION_EXECUTION_SIMULATION_FROM must be a valid address"));
    }

    #[test]
    fn option_execution_rejects_invalid_signature_mode() {
        let error =
            config_from_pairs([("OPTION_EXECUTION_SIGNATURE_MODE", "optional")]).unwrap_err();

        assert!(error
            .to_string()
            .contains("invalid OPTION_EXECUTION_SIGNATURE_MODE"));
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
