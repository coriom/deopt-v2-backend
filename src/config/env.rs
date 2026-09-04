use crate::admin::{AdminConfig, MetricsConfig};
use crate::api::trading_views::TradingViewsConfig;
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
    /// BACKEND-PUBLIC-WS-API-V1 — knobs for the public `/ws`
    /// WebSocket endpoint mounted on the same Axum HTTP listener.
    /// Distinct from `mm_gateway` (operator-whitelisted WebTransport).
    pub public_ws: crate::api::public_ws::PublicWsConfig,
    pub admin: AdminConfig,
    pub metrics: MetricsConfig,
    pub signature_verification_mode: SignatureVerificationMode,
    pub eip712_domain: Eip712Domain,
    pub persistence_enabled: bool,
    pub database_url: Option<String>,
    /// M-P2e — Optional public contract addresses for the read-only
    /// trading_views surface. All fields are optional; missing addresses
    /// route the trading handlers to the M-P2b partial-data path.
    /// Malformed addresses fail config validation at startup with a
    /// clear `BackendError::Config`.
    pub trading_views: TradingViewsConfig,
    /// PERPS-MINIMAL-MARKET-AND-PRICE-V1 — read-only Perps market
    /// registry + oracle price configuration. Defaults to
    /// `PerpsReadConfig::disabled()`; operator wires `PERPS_*` env vars
    /// to turn the new `/perps/markets*` read routes on. Never affects
    /// the Perps mutation fail-closed gate.
    pub perps_read: crate::perps::PerpsReadConfig,
    /// PERPS-FRONTEND-TICKET-ENABLEMENT-V1 — strict opt-in flag for
    /// the new `POST /perps/orders` + `DELETE /perps/orders/:id`
    /// mutation routes. Default: `false`. Enabling on any mainnet
    /// chain id is refused at startup by `validate_startup`.
    /// Env: `PERPS_PUBLIC_TRADING_ENABLED=true` (default `false`).
    pub perps_public_trading_enabled: bool,
    /// PERPS-SUBACCOUNTS-CORE-ROUTING-V1 — closed-test opt-in flag. When
    /// `true`, allowlisted wallets can reach the Perps mutation surface
    /// (still 503 for everyone else). When `false`, every mutation
    /// returns 503 regardless of `perps_public_trading_enabled`.
    /// Refused on mainnet chain ids at startup, mirroring the public
    /// trading flag. Env: `PERPS_CLOSED_TEST_ENABLED` (default `false`).
    pub perps_closed_test_enabled: bool,
    /// PERPS-SUBACCOUNTS-CORE-ROUTING-V1 — comma-separated allowlist
    /// of wallet addresses that may reach Perps mutations while
    /// `perps_closed_test_enabled` is true. Addresses are lower-cased
    /// on parse. Empty list means "no wallets allowed" — a closed test
    /// with no allowlist is honest but useless. Env:
    /// `PERPS_CLOSED_TEST_ALLOWLIST` (default empty).
    pub perps_closed_test_allowlist: Vec<AccountId>,
    /// PERPS-FUNDING-LIQUIDATION-WORKERS-V1 — periodic funding worker
    /// configuration. Defaults `disabled()`. Env:
    ///
    /// * `PERPS_FUNDING_WORKER_ENABLED=true` (starts the periodic loop)
    /// * `PERPS_FUNDING_TICK_ENABLED=true`   (kill-switch — consulted by
    ///   both the periodic worker AND the admin HTTP tick)
    /// * `PERPS_FUNDING_WORKER_INTERVAL_SEC` (30..=86400, default 3600)
    /// * `PERPS_FUNDING_MAX_MARKETS_PER_TICK` (default 32)
    /// * `PERPS_FUNDING_STALE_ORACLE_POLICY=skip` (V1 only supports skip)
    pub perps_funding_worker: crate::perps::PerpsFundingWorkerConfig,
    /// PERPS-FUNDING-LIQUIDATION-WORKERS-V1 — periodic liquidation
    /// worker configuration. Defaults `disabled()`. Env:
    ///
    /// * `PERPS_LIQUIDATION_WORKER_ENABLED=true`
    /// * `PERPS_LIQUIDATION_TICK_ENABLED=true` (kill-switch)
    /// * `PERPS_LIQUIDATION_WORKER_INTERVAL_SEC` (5..=3600, default 30)
    /// * `PERPS_LIQUIDATION_MAX_POSITIONS_PER_TICK` (default 500)
    /// * `PERPS_LIQUIDATION_STALE_ORACLE_POLICY=skip`
    pub perps_liquidation_worker: crate::perps::PerpsLiquidationWorkerConfig,
    /// PERPS-FULLSTACK-RUNTIME-INTEGRATION-V1 Part B — impact-mid
    /// keeper configuration. Defaults `disabled()`. Env:
    ///
    /// * `PERPS_IMPACT_MID_KEEPER_ENABLED=true` (spawns the periodic loop)
    /// * `PERPS_IMPACT_MID_KEEPER_INTERVAL_MS` (default 5000)
    /// * `PERPS_ETH_IMPACT_NOTIONAL_1E8` (non-zero to enable ETH-PERP)
    /// * `PERPS_BTC_IMPACT_NOTIONAL_1E8` (non-zero to enable BTC-PERP)
    /// * `PERPS_ETH_IMPACT_MAX_INDEX_DEVIATION_BPS` (default 500 = 5%)
    /// * `PERPS_BTC_IMPACT_MAX_INDEX_DEVIATION_BPS` (default 500 = 5%)
    ///
    /// Refused on mainnet (chain_id ∈ {1, 8453}) when enabled — same
    /// posture as the funding + liquidation workers.
    pub perps_impact_mid_keeper: crate::perps::PerpsImpactMidKeeperConfig,
    /// BACKEND-HYBRID-V2-PERSISTED-RUNTIME-CORE-V1 — Hybrid V2 indexer
    /// worker configuration. Defaults `disabled()`. When
    /// `HYBRID_V2_ENABLED=true`, `HybridV2Config::from_env` also reads
    /// deployment_id, chain_id, poll_interval_ms, confirmation_depth,
    /// max_block_batch, start_block, cursor_name. Base mainnet
    /// (chain_id=8453) is refused unconditionally at parse time. The
    /// worker requires `PERSISTENCE_ENABLED=true` and (in a future
    /// stage) a live RPC ChainSource; stage 3A validates the config
    /// and logs the wire state without spawning a real chain-driven
    /// loop.
    pub hybrid_v2: crate::hybrid_v2::config::HybridV2Config,
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
        let chain_id: u64 = parse_env(&mut lookup, "CHAIN_ID", "84532")?;
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
            old_perp_engine_address: optional_env(&mut lookup, "OLD_PERP_ENGINE_ADDRESS")
                .filter(|value| !value.is_empty())
                .map(AccountId::new),
            backend_signer_mode: {
                let endpoint = lookup("BACKEND_SIGNER_ENDPOINT").filter(|value| !value.is_empty());
                match lookup("BACKEND_SIGNER_MODE").filter(|value| !value.is_empty()) {
                    Some(value) => crate::execution::SignerBackendKind::parse(&value)
                        .map_err(crate::error::BackendError::Config)?,
                    None if endpoint.is_some() => crate::execution::SignerBackendKind::Remote,
                    None => crate::execution::SignerBackendKind::LocalDev,
                }
            },
            backend_signer_endpoint: lookup("BACKEND_SIGNER_ENDPOINT")
                .filter(|value| !value.is_empty()),
            executor_allow_local_signer: parse_env(
                &mut lookup,
                "EXECUTOR_ALLOW_LOCAL_SIGNER",
                "false",
            )?,
            backend_signer_provider: match lookup("BACKEND_REMOTE_SIGNER_PROVIDER")
                .filter(|value| !value.is_empty())
            {
                Some(value) => Some(
                    crate::execution::signer_adapters::SignerProviderKind::parse(&value)
                        .map_err(crate::error::BackendError::Config)?,
                ),
                None => None,
            },
            backend_signer_timeout_ms: {
                let value: u32 = parse_env(&mut lookup, "BACKEND_SIGNER_TIMEOUT_MS", "2500")?;
                if !(100..=30_000).contains(&value) {
                    return Err(crate::error::BackendError::Config(format!(
                        "BACKEND_SIGNER_TIMEOUT_MS must be in 100..=30000 (got {value})"
                    )));
                }
                value
            },
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
            rfq_multi_leg_enabled: parse_env(&mut lookup, "OPTION_RFQ_MULTI_LEG_ENABLED", "false")?,
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
            twap_enabled: parse_env(&mut lookup, "OPTION_TWAP_ENABLED", "false")?,
            twap_max_child_count: parse_env(&mut lookup, "OPTION_TWAP_MAX_CHILD_COUNT", "50")?,
            twap_max_running_time_ms: parse_env(
                &mut lookup,
                "OPTION_TWAP_MAX_RUNNING_TIME_MS",
                "86400000",
            )?,
            twap_min_child_interval_ms: parse_env(
                &mut lookup,
                "OPTION_TWAP_MIN_CHILD_INTERVAL_MS",
                "10000",
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
        let option_event_indexer_fees_manager_v2_address =
            optional_env(&mut lookup, "OPTION_EVENT_INDEXER_FEES_MANAGER_V2_ADDRESS")
                .or_else(|| optional_env(&mut lookup, "FEES_MANAGER_V2"))
                .map(AccountId::new);
        // V2G-F: optional legacy MarginEngine address. Used solely by
        // the `deopt_option_fee_*_v2_total{consumer="old"}` metrics;
        // never used to route broadcast or execution traffic. Unset by
        // default — non-NEW consumers bucket as `"unknown"`.
        let option_event_indexer_old_margin_engine_address =
            optional_env(&mut lookup, "OLD_MARGIN_ENGINE_ADDRESS")
                .filter(|value| !value.is_empty())
                .map(AccountId::new);
        // ProtocolFeeVault address — fed into the runtime
        // `LiveBroadcastPolicyDataProvider` so `should_broadcast`'s
        // PFV-side chain reads (`feeBalance(asset)` +
        // `rebateReserve(asset)`) fire. Defaults to `None` (no
        // permissive default — `should_broadcast` fails closed on
        // mainnet when the reserve read is missing).
        let option_event_indexer_protocol_fee_vault_address =
            optional_env(&mut lookup, "PROTOCOL_FEE_VAULT_ADDRESS")
                .or_else(|| {
                    optional_env(
                        &mut lookup,
                        "OPTION_EVENT_INDEXER_PROTOCOL_FEE_VAULT_ADDRESS",
                    )
                })
                .or_else(|| optional_env(&mut lookup, "PROTOCOL_FEE_VAULT"))
                .filter(|value| !value.is_empty())
                .map(AccountId::new);
        // Validate address shape if provided; reject malformed input
        // at startup so it never reaches the LiveProvider eth_call.
        if let Some(addr) = option_event_indexer_protocol_fee_vault_address.as_ref() {
            crate::signing::eip712::parse_evm_address(addr).map_err(|err| {
                crate::error::BackendError::Config(format!(
                    "invalid PROTOCOL_FEE_VAULT_ADDRESS: {err}"
                ))
            })?;
        }
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
            state_checks_enabled: parse_env(
                &mut lookup,
                "OPTION_RECONCILIATION_STATE_CHECKS_ENABLED",
                "false",
            )?,
            state_checks_require_rpc: parse_env(
                &mut lookup,
                "OPTION_RECONCILIATION_STATE_CHECKS_REQUIRE_RPC",
                "true",
            )?,
            state_checks_strict: parse_env(
                &mut lookup,
                "OPTION_RECONCILIATION_STATE_CHECKS_STRICT",
                "false",
            )?,
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
            fees_manager_v2_address: option_event_indexer_fees_manager_v2_address,
            old_margin_engine_address: option_event_indexer_old_margin_engine_address,
            protocol_fee_vault_address: option_event_indexer_protocol_fee_vault_address,
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

        // M-P2e — Optional public contract addresses for the
        // read-only trading_views surface. Disabled by default;
        // upgrades handler envelopes to "ok" when configured.
        // NEVER touches signer / AWS / KMS / broadcast paths.
        let public_ws_defaults = crate::api::public_ws::PublicWsConfig::default_testnet();
        let public_ws = crate::api::public_ws::PublicWsConfig {
            enabled: parse_env(
                &mut lookup,
                "PUBLIC_WS_ENABLED",
                if public_ws_defaults.enabled {
                    "true"
                } else {
                    "false"
                },
            )?,
            path: get_env(&mut lookup, "PUBLIC_WS_PATH", &public_ws_defaults.path),
            max_connections: parse_env(
                &mut lookup,
                "PUBLIC_WS_MAX_CONNECTIONS",
                &public_ws_defaults.max_connections.to_string(),
            )?,
            max_subscriptions_per_connection: parse_env(
                &mut lookup,
                "PUBLIC_WS_MAX_SUBSCRIPTIONS_PER_CONNECTION",
                &public_ws_defaults
                    .max_subscriptions_per_connection
                    .to_string(),
            )?,
            max_frame_bytes: parse_env(
                &mut lookup,
                "PUBLIC_WS_MAX_FRAME_BYTES",
                &public_ws_defaults.max_frame_bytes.to_string(),
            )?,
            client_rate_limit_per_sec: parse_env(
                &mut lookup,
                "PUBLIC_WS_CLIENT_RATE_LIMIT_PER_SEC",
                &public_ws_defaults.client_rate_limit_per_sec.to_string(),
            )?,
            heartbeat_interval_ms: parse_env(
                &mut lookup,
                "PUBLIC_WS_HEARTBEAT_INTERVAL_MS",
                &public_ws_defaults.heartbeat_interval_ms.to_string(),
            )?,
            snapshot_interval_ms: parse_env(
                &mut lookup,
                "PUBLIC_WS_SNAPSHOT_INTERVAL_MS",
                &public_ws_defaults.snapshot_interval_ms.to_string(),
            )?,
            challenge_ttl_ms: parse_env(
                &mut lookup,
                "PUBLIC_WS_CHALLENGE_TTL_MS",
                &public_ws_defaults.challenge_ttl_ms.to_string(),
            )?,
        };
        let trading_views = TradingViewsConfig {
            margin_engine_lens_address: parse_optional_address_env(
                &mut lookup,
                "OPTION_MARGIN_ENGINE_LENS_ADDRESS",
            )?,
            collateral_vault_views_address: parse_optional_address_env(
                &mut lookup,
                "OPTION_COLLATERAL_VAULT_VIEWS_ADDRESS",
            )?,
            collateral_vault_address: parse_optional_address_env(
                &mut lookup,
                "OPTION_COLLATERAL_VAULT_ADDRESS",
            )?,
            oracle_router_address: parse_optional_address_env(
                &mut lookup,
                "OPTION_ORACLE_ROUTER_ADDRESS",
            )?,
            margin_engine_address: parse_optional_address_env(
                &mut lookup,
                "OPTION_MARGIN_ENGINE_ADDRESS",
            )?,
        };
        // PERPS-MINIMAL-MARKET-AND-PRICE-V1 — read-only Perps market
        // registry + oracle price config. Off by default. Reuses
        // `execution.rpc_url` for the JSON-RPC endpoint; adding a
        // separate `PERPS_RPC_URL` would fragment the operator's
        // network wiring for no gain.
        let perps_read = {
            let enabled = parse_env(&mut lookup, "PERPS_READ_ENABLED", "false")?;
            let perps_chain_id = parse_env(&mut lookup, "PERPS_CHAIN_ID", &chain_id.to_string())?;
            let market_registry_address =
                parse_optional_address_env(&mut lookup, "PERPS_MARKET_REGISTRY_ADDRESS")?;
            let oracle_router_address =
                parse_optional_address_env(&mut lookup, "PERPS_ORACLE_ROUTER_ADDRESS")?;
            let stale_after_sec = parse_env(&mut lookup, "PERPS_STALE_AFTER_SEC", "60")?;
            let mut markets: Vec<crate::perps::config::PerpsReadMarket> = Vec::new();
            // ETH-PERP row: enrol only when both asset addresses are
            // configured. Missing addresses → row skipped (we do NOT
            // fabricate). Onchain market id defaults to 1 (Base Sepolia
            // seed).
            if let (Some(base), Some(quote)) = (
                parse_optional_address_env(&mut lookup, "PERPS_ETH_BASE_ADDRESS")?,
                parse_optional_address_env(&mut lookup, "PERPS_ETH_QUOTE_ADDRESS")?,
            ) {
                markets.push(crate::perps::config::PerpsReadMarket {
                    symbol: get_env(&mut lookup, "PERPS_ETH_SYMBOL", "ETH-PERP"),
                    onchain_market_id: parse_env(&mut lookup, "PERPS_ETH_ONCHAIN_MARKET_ID", "1")?,
                    base_asset_label: get_env(&mut lookup, "PERPS_ETH_BASE_LABEL", "ETH"),
                    quote_asset_label: get_env(&mut lookup, "PERPS_ETH_QUOTE_LABEL", "mUSDC"),
                    base_asset_address: base,
                    quote_asset_address: quote,
                    max_leverage: parse_env(&mut lookup, "PERPS_ETH_MAX_LEVERAGE", "10")?,
                    maintenance_margin_bps: parse_env(
                        &mut lookup,
                        "PERPS_ETH_MAINTENANCE_MARGIN_BPS",
                        "500",
                    )?,
                    // PERPS-MARGIN-ORACLE-RISK-V1 — per-market risk
                    // caps. Defaults match the closed-test scope doc.
                    max_order_size_1e8: Some(parse_env::<u128>(
                        &mut lookup,
                        "PERPS_ETH_MAX_ORDER_SIZE_1E8",
                        "1000000000",
                    )?),
                    max_order_notional_1e8: Some(parse_env::<u128>(
                        &mut lookup,
                        "PERPS_ETH_MAX_ORDER_NOTIONAL_1E8",
                        "10000000000000",
                    )?),
                    max_subaccount_notional_1e8: Some(parse_env::<u128>(
                        &mut lookup,
                        "PERPS_ETH_MAX_SUBACCOUNT_NOTIONAL_1E8",
                        "50000000000000",
                    )?),
                    max_open_interest_1e8: Some(parse_env::<u128>(
                        &mut lookup,
                        "PERPS_ETH_MAX_OPEN_INTEREST_1E8",
                        "5000000000",
                    )?),
                });
            }
            if let (Some(base), Some(quote)) = (
                parse_optional_address_env(&mut lookup, "PERPS_BTC_BASE_ADDRESS")?,
                parse_optional_address_env(&mut lookup, "PERPS_BTC_QUOTE_ADDRESS")?,
            ) {
                markets.push(crate::perps::config::PerpsReadMarket {
                    symbol: get_env(&mut lookup, "PERPS_BTC_SYMBOL", "BTC-PERP"),
                    onchain_market_id: parse_env(&mut lookup, "PERPS_BTC_ONCHAIN_MARKET_ID", "2")?,
                    base_asset_label: get_env(&mut lookup, "PERPS_BTC_BASE_LABEL", "BTC"),
                    quote_asset_label: get_env(&mut lookup, "PERPS_BTC_QUOTE_LABEL", "mUSDC"),
                    base_asset_address: base,
                    quote_asset_address: quote,
                    max_leverage: parse_env(&mut lookup, "PERPS_BTC_MAX_LEVERAGE", "5")?,
                    maintenance_margin_bps: parse_env(
                        &mut lookup,
                        "PERPS_BTC_MAINTENANCE_MARGIN_BPS",
                        "750",
                    )?,
                    max_order_size_1e8: Some(parse_env::<u128>(
                        &mut lookup,
                        "PERPS_BTC_MAX_ORDER_SIZE_1E8",
                        "100000000",
                    )?),
                    max_order_notional_1e8: Some(parse_env::<u128>(
                        &mut lookup,
                        "PERPS_BTC_MAX_ORDER_NOTIONAL_1E8",
                        "10000000000000",
                    )?),
                    max_subaccount_notional_1e8: Some(parse_env::<u128>(
                        &mut lookup,
                        "PERPS_BTC_MAX_SUBACCOUNT_NOTIONAL_1E8",
                        "50000000000000",
                    )?),
                    max_open_interest_1e8: Some(parse_env::<u128>(
                        &mut lookup,
                        "PERPS_BTC_MAX_OPEN_INTEREST_1E8",
                        "500000000",
                    )?),
                });
            }
            // PERPS-MARGIN-ORACLE-RISK-V1 — deviation guard threshold.
            // Default 500 bps (5%). Validated by
            // `PerpsReadConfig::validate_startup`.
            let oracle_max_deviation_bps: u32 =
                parse_env(&mut lookup, "PERPS_ORACLE_MAX_DEVIATION_BPS", "500")?;
            let cfg = crate::perps::PerpsReadConfig {
                enabled,
                chain_id: perps_chain_id,
                rpc_url: execution.rpc_url.clone(),
                market_registry_address,
                oracle_router_address,
                markets,
                stale_after_sec,
                oracle_max_deviation_bps,
            };
            cfg.validate_startup()?;
            cfg
        };

        // PERPS-FRONTEND-TICKET-ENABLEMENT-V1 — strict opt-in flag for
        // the new /perps/orders + /perps/orders/:id mutation routes.
        // Default false. Never enable on any mainnet chain id.
        let perps_public_trading_enabled: bool =
            parse_env(&mut lookup, "PERPS_PUBLIC_TRADING_ENABLED", "false")?;
        if perps_public_trading_enabled && (chain_id == 1 || chain_id == 8453) {
            return Err(BackendError::Config(format!(
                "PERPS_PUBLIC_TRADING_ENABLED=true is refused on mainnet chain id {chain_id}"
            )));
        }

        // PERPS-SUBACCOUNTS-CORE-ROUTING-V1 — closed-test opt-in flag +
        // allowlist. Both default off / empty. Refused on mainnet chain
        // ids at startup, same as the public trading flag.
        let perps_closed_test_enabled: bool =
            parse_env(&mut lookup, "PERPS_CLOSED_TEST_ENABLED", "false")?;
        if perps_closed_test_enabled && (chain_id == 1 || chain_id == 8453) {
            return Err(BackendError::Config(format!(
                "PERPS_CLOSED_TEST_ENABLED=true is refused on mainnet chain id {chain_id}"
            )));
        }
        let perps_closed_test_allowlist: Vec<AccountId> = lookup("PERPS_CLOSED_TEST_ALLOWLIST")
            .filter(|value| !value.is_empty())
            .map(|value| {
                value
                    .split(',')
                    .map(|entry| entry.trim().to_lowercase())
                    .filter(|entry| !entry.is_empty())
                    .map(AccountId::new)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        // PERPS-FUNDING-LIQUIDATION-WORKERS-V1 — periodic funding
        // worker + kill-switch. All defaults safe (worker off, tick
        // off, 1h interval). Mainnet refusal enforced by
        // `PerpsFundingWorkerConfig::validate_startup(chain_id)`.
        let perps_funding_worker = crate::perps::PerpsFundingWorkerConfig {
            worker_enabled: parse_env(&mut lookup, "PERPS_FUNDING_WORKER_ENABLED", "false")?,
            tick_enabled: parse_env(&mut lookup, "PERPS_FUNDING_TICK_ENABLED", "false")?,
            interval_sec: parse_env(&mut lookup, "PERPS_FUNDING_WORKER_INTERVAL_SEC", "3600")?,
            max_markets_per_tick: parse_env(
                &mut lookup,
                "PERPS_FUNDING_MAX_MARKETS_PER_TICK",
                "32",
            )?,
            stale_oracle_policy: crate::perps::PerpsWorkerStaleOraclePolicy::parse(&get_env(
                &mut lookup,
                "PERPS_FUNDING_STALE_ORACLE_POLICY",
                "skip",
            )),
        };
        perps_funding_worker.validate_startup(chain_id)?;

        // PERPS-FUNDING-LIQUIDATION-WORKERS-V1 — periodic liquidation
        // worker + kill-switch. All defaults safe.
        let perps_liquidation_worker = crate::perps::PerpsLiquidationWorkerConfig {
            worker_enabled: parse_env(&mut lookup, "PERPS_LIQUIDATION_WORKER_ENABLED", "false")?,
            tick_enabled: parse_env(&mut lookup, "PERPS_LIQUIDATION_TICK_ENABLED", "false")?,
            interval_sec: parse_env(&mut lookup, "PERPS_LIQUIDATION_WORKER_INTERVAL_SEC", "30")?,
            max_positions_per_tick: parse_env(
                &mut lookup,
                "PERPS_LIQUIDATION_MAX_POSITIONS_PER_TICK",
                "500",
            )?,
            stale_oracle_policy: crate::perps::PerpsWorkerStaleOraclePolicy::parse(&get_env(
                &mut lookup,
                "PERPS_LIQUIDATION_STALE_ORACLE_POLICY",
                "skip",
            )),
        };
        perps_liquidation_worker.validate_startup(chain_id)?;

        // PERPS-FULLSTACK-RUNTIME-INTEGRATION-V1 Part B — impact-mid
        // keeper. All defaults safe (`enabled=false`, no markets).
        // Per-market rows are populated ONLY when the operator sets a
        // non-zero notional env var for that market — an empty
        // notional is treated as "market not configured for the
        // keeper" (silent skip in the tick loop), matching the
        // fail-closed posture where turning the keeper on for a single
        // market must be an explicit action.
        let impact_mid_enabled: bool =
            parse_env(&mut lookup, "PERPS_IMPACT_MID_KEEPER_ENABLED", "false")?;
        let impact_mid_interval_ms: u64 = parse_env(
            &mut lookup,
            "PERPS_IMPACT_MID_KEEPER_INTERVAL_MS",
            "5000",
        )?;
        let impact_mid_markets = collect_impact_mid_markets(&mut lookup)?;
        let perps_impact_mid_keeper = crate::perps::PerpsImpactMidKeeperConfig {
            enabled: impact_mid_enabled,
            tick_interval_ms: impact_mid_interval_ms,
            markets: impact_mid_markets,
            // PERPS-CLOSED-TEST-HARDENING-V1 Part E — publisher is
            // wired POST-config in main.rs when
            // `PERPS_IMPACT_MID_PUBLISHER != none`. Env-parse layer
            // leaves this as `None`; the safe cache-only default.
            publisher: None,
        };
        perps_impact_mid_keeper.validate_startup(chain_id)?;

        // BACKEND-HYBRID-V2-PERSISTED-RUNTIME-CORE-V1 — load + validate
        // the Hybrid V2 indexer worker config. `HybridV2Config::from_env`
        // reads its own `HYBRID_V2_*` vars via `std::env::var` (dotenv
        // has already loaded the file), refuses Base mainnet
        // unconditionally, and returns `disabled()` when
        // `HYBRID_V2_ENABLED` is unset or false.
        let hybrid_v2 = crate::hybrid_v2::config::HybridV2Config::from_env()?;

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
            public_ws,
            admin,
            metrics,
            signature_verification_mode,
            eip712_domain,
            persistence_enabled,
            database_url,
            trading_views,
            perps_read,
            perps_public_trading_enabled,
            perps_closed_test_enabled,
            perps_closed_test_allowlist,
            perps_funding_worker,
            perps_liquidation_worker,
            perps_impact_mid_keeper,
            hybrid_v2,
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

/// M-P2e — Parse an optional EVM address env var. Empty / unset →
/// `Ok(None)`. Malformed → `Err(BackendError::Config(…))` with a
/// human-readable reason that NEVER reveals the configured value.
///
/// Accepts canonical `0x` + 40-hex-char addresses, case-insensitive.
fn parse_optional_address_env(
    lookup: &mut impl FnMut(&str) -> Option<String>,
    key: &str,
) -> Result<Option<AccountId>> {
    let Some(raw) = optional_env(lookup, key) else {
        return Ok(None);
    };
    let trimmed = raw.trim();
    let hex = trimmed.strip_prefix("0x").ok_or_else(|| {
        BackendError::Config(format!(
            "{key} must be a 0x-prefixed EVM address (40 hex chars)"
        ))
    })?;
    if hex.len() != 40 {
        return Err(BackendError::Config(format!(
            "{key} must have exactly 40 hex characters after the 0x prefix"
        )));
    }
    if !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(BackendError::Config(format!(
            "{key} contains non-hex characters"
        )));
    }
    Ok(Some(AccountId::new(format!(
        "0x{}",
        hex.to_ascii_lowercase()
    ))))
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

/// PERPS-FULLSTACK-RUNTIME-INTEGRATION-V1 Part B — collect per-market
/// impact-mid keeper rows from env. A market is included ONLY when its
/// `PERPS_{ETH,BTC}_IMPACT_NOTIONAL_1E8` env var is set to a non-empty,
/// non-zero value. Missing / empty / `0` → the market is silently
/// omitted from the keeper's configured markets vec (which in turn
/// means the keeper tick loop skips it — no cache write, no metric).
/// Per-market deviation defaults to
/// `PerpsImpactMidKeeperConfig::DEFAULT_MAX_INDEX_DEVIATION_BPS`.
fn collect_impact_mid_markets(
    lookup: &mut impl FnMut(&str) -> Option<String>,
) -> Result<Vec<crate::perps::PerpsImpactMidMarketConfig>> {
    // (symbol, notional-env-var, deviation-env-var). The prefix order
    // is (ETH, BTC) to match the read-config's `enabled_in_memory_for_tests`.
    let entries: &[(&str, &str, &str)] = &[
        (
            "ETH-PERP",
            "PERPS_ETH_IMPACT_NOTIONAL_1E8",
            "PERPS_ETH_IMPACT_MAX_INDEX_DEVIATION_BPS",
        ),
        (
            "BTC-PERP",
            "PERPS_BTC_IMPACT_NOTIONAL_1E8",
            "PERPS_BTC_IMPACT_MAX_INDEX_DEVIATION_BPS",
        ),
    ];
    let mut markets = Vec::new();
    for (symbol, notional_key, deviation_key) in entries {
        let notional_raw = match lookup(notional_key).filter(|v| !v.is_empty()) {
            Some(v) => v,
            None => continue,
        };
        let notional: u128 = notional_raw.parse().map_err(|error| {
            BackendError::Config(format!("invalid {notional_key}: {error}"))
        })?;
        if notional == 0 {
            continue;
        }
        let deviation_default =
            crate::perps::impact_mid_keeper::DEFAULT_MAX_INDEX_DEVIATION_BPS.to_string();
        let deviation: u32 = parse_env(lookup, deviation_key, &deviation_default)?;
        markets.push(crate::perps::PerpsImpactMidMarketConfig {
            symbol: (*symbol).to_string(),
            impact_notional_1e8: notional,
            max_index_deviation_bps: deviation,
        });
    }
    Ok(markets)
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
    fn option_reconciliation_state_checks_use_safe_defaults() {
        let config = config_from_pairs([("PERSISTENCE_ENABLED", "false")]).unwrap();

        assert!(!config.option_reconciliation.state_checks_enabled);
        assert!(config.option_reconciliation.state_checks_require_rpc);
        assert!(!config.option_reconciliation.state_checks_strict);
    }

    #[test]
    fn option_reconciliation_state_checks_parse_overrides() {
        let config = config_from_pairs([
            ("OPTION_RECONCILIATION_WORKER_ENABLED", "true"),
            ("OPTION_RECONCILIATION_REQUIRE_RPC", "false"),
            ("OPTION_RECONCILIATION_STATE_CHECKS_ENABLED", "true"),
            ("OPTION_RECONCILIATION_STATE_CHECKS_REQUIRE_RPC", "false"),
            ("OPTION_RECONCILIATION_STATE_CHECKS_STRICT", "true"),
            ("PERSISTENCE_ENABLED", "true"),
            ("DATABASE_URL", "postgres://example.invalid/db"),
        ])
        .unwrap();

        assert!(config.option_reconciliation.enabled);
        assert!(config.option_reconciliation.state_checks_enabled);
        assert!(!config.option_reconciliation.state_checks_require_rpc);
        assert!(config.option_reconciliation.state_checks_strict);
    }

    #[test]
    fn option_reconciliation_state_checks_require_rpc_when_configured() {
        let error = config_from_pairs([
            ("OPTION_RECONCILIATION_WORKER_ENABLED", "true"),
            ("OPTION_RECONCILIATION_REQUIRE_RPC", "false"),
            ("OPTION_RECONCILIATION_STATE_CHECKS_ENABLED", "true"),
            ("OPTION_RECONCILIATION_STATE_CHECKS_REQUIRE_RPC", "true"),
            ("PERSISTENCE_ENABLED", "true"),
            ("DATABASE_URL", "postgres://example.invalid/db"),
        ])
        .unwrap_err();

        assert!(error
            .to_string()
            .contains("RPC_URL is required when OPTION_RECONCILIATION_STATE_CHECKS_ENABLED=true"));
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
            ("EXECUTOR_ALLOW_LOCAL_SIGNER", "true"),
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
            ("EXECUTOR_ALLOW_LOCAL_SIGNER", "true"),
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
            ("EXECUTOR_ALLOW_LOCAL_SIGNER", "true"),
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
            ("EXECUTOR_ALLOW_LOCAL_SIGNER", "true"),
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
            ("EXECUTOR_ALLOW_LOCAL_SIGNER", "true"),
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
            ("EXECUTOR_ALLOW_LOCAL_SIGNER", "true"),
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
        assert!(!config.options.rfq_multi_leg_enabled);
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

    // ----------------------------------------------------------------------------
    // M-P2e env-loader tests — TradingViewsConfig
    // ----------------------------------------------------------------------------

    #[test]
    fn trading_views_addresses_all_absent_yields_disabled_config() {
        let config = config_from_pairs([]).unwrap();
        assert_eq!(config.trading_views, TradingViewsConfig::disabled());
        assert!(config.trading_views.margin_engine_lens_address.is_none());
        assert!(config
            .trading_views
            .collateral_vault_views_address
            .is_none());
        assert!(config.trading_views.collateral_vault_address.is_none());
        assert!(config.trading_views.oracle_router_address.is_none());
        assert!(config.trading_views.margin_engine_address.is_none());
    }

    #[test]
    fn trading_views_addresses_all_present_parsed_to_lowercase_canonical_form() {
        let config = config_from_pairs([
            (
                "OPTION_MARGIN_ENGINE_LENS_ADDRESS",
                "0xAaAaAaAaAaAaAaAaAaAaAaAaAaAaAaAaAaAaAaAa",
            ),
            (
                "OPTION_COLLATERAL_VAULT_VIEWS_ADDRESS",
                "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            ),
            (
                "OPTION_COLLATERAL_VAULT_ADDRESS",
                "0xCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCC",
            ),
            (
                "OPTION_ORACLE_ROUTER_ADDRESS",
                "0xdddddddddddddddddddddddddddddddddddddddd",
            ),
            (
                "OPTION_MARGIN_ENGINE_ADDRESS",
                "0xeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
            ),
        ])
        .unwrap();
        assert_eq!(
            config
                .trading_views
                .margin_engine_lens_address
                .as_ref()
                .unwrap()
                .0,
            "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        );
        assert_eq!(
            config
                .trading_views
                .collateral_vault_views_address
                .as_ref()
                .unwrap()
                .0,
            "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
        );
        assert_eq!(
            config
                .trading_views
                .collateral_vault_address
                .as_ref()
                .unwrap()
                .0,
            "0xcccccccccccccccccccccccccccccccccccccccc"
        );
        assert_eq!(
            config
                .trading_views
                .oracle_router_address
                .as_ref()
                .unwrap()
                .0,
            "0xdddddddddddddddddddddddddddddddddddddddd"
        );
        assert_eq!(
            config
                .trading_views
                .margin_engine_address
                .as_ref()
                .unwrap()
                .0,
            "0xeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee"
        );
    }

    #[test]
    fn trading_views_address_missing_0x_prefix_rejected() {
        let err = config_from_pairs([(
            "OPTION_MARGIN_ENGINE_LENS_ADDRESS",
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        )])
        .unwrap_err();
        assert!(err
            .to_string()
            .contains("OPTION_MARGIN_ENGINE_LENS_ADDRESS"));
        assert!(err.to_string().contains("0x-prefixed"));
    }

    #[test]
    fn trading_views_address_wrong_length_rejected() {
        let err = config_from_pairs([("OPTION_ORACLE_ROUTER_ADDRESS", "0xabc")]).unwrap_err();
        assert!(err.to_string().contains("40 hex characters"));
    }

    #[test]
    fn trading_views_address_non_hex_character_rejected() {
        let err = config_from_pairs([(
            "OPTION_COLLATERAL_VAULT_ADDRESS",
            "0xzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz",
        )])
        .unwrap_err();
        assert!(err.to_string().contains("non-hex"));
    }

    #[test]
    fn trading_views_empty_string_treated_as_absent() {
        let config = config_from_pairs([
            ("OPTION_MARGIN_ENGINE_LENS_ADDRESS", ""),
            ("OPTION_COLLATERAL_VAULT_ADDRESS", ""),
        ])
        .unwrap();
        assert!(config.trading_views.margin_engine_lens_address.is_none());
        assert!(config.trading_views.collateral_vault_address.is_none());
    }

    #[test]
    fn trading_views_partial_config_only_populates_supplied_fields() {
        let config = config_from_pairs([(
            "OPTION_ORACLE_ROUTER_ADDRESS",
            "0x1234567890abcdef1234567890abcdef12345678",
        )])
        .unwrap();
        assert!(config.trading_views.oracle_router_address.is_some());
        assert!(config.trading_views.margin_engine_lens_address.is_none());
        assert!(config.trading_views.collateral_vault_address.is_none());
    }

    #[test]
    fn trading_views_error_message_never_echoes_the_configured_value() {
        // Defence-in-depth: a malformed key must never leak its contents
        // into the error string (since logs may capture startup errors).
        let secret = "0xCAFEBABECAFEBABECAFEBABECAFEBABECAFEBABEzz"; // last two chars non-hex
        let err = config_from_pairs([("OPTION_MARGIN_ENGINE_ADDRESS", secret)]).unwrap_err();
        let msg = err.to_string();
        assert!(!msg.contains("CAFEBABE"));
        assert!(!msg.contains(secret));
    }

    // ----------------------------------------------------------------------------
    // BACKEND-LIVE-PROVIDER-PFV-CONFIG env-loader tests
    // ----------------------------------------------------------------------------

    #[test]
    fn protocol_fee_vault_address_absent_yields_none() {
        let config = config_from_pairs([]).unwrap();
        assert_eq!(
            config
                .option_event_indexer
                .protocol_fee_vault_address
                .as_ref(),
            None,
            "no env key set → typed field stays None"
        );
    }

    #[test]
    fn protocol_fee_vault_address_canonical_env_key_parses() {
        let config = config_from_pairs([(
            "PROTOCOL_FEE_VAULT_ADDRESS",
            "0x7C0a3B6feBd5BFFc164f37738299AeB453181886",
        )])
        .unwrap();
        assert!(config
            .option_event_indexer
            .protocol_fee_vault_address
            .is_some());
    }

    #[test]
    fn protocol_fee_vault_address_namespaced_env_key_parses() {
        let config = config_from_pairs([(
            "OPTION_EVENT_INDEXER_PROTOCOL_FEE_VAULT_ADDRESS",
            "0x7C0a3B6feBd5BFFc164f37738299AeB453181886",
        )])
        .unwrap();
        assert!(config
            .option_event_indexer
            .protocol_fee_vault_address
            .is_some());
    }

    #[test]
    fn protocol_fee_vault_address_short_alias_parses() {
        let config = config_from_pairs([(
            "PROTOCOL_FEE_VAULT",
            "0x7C0a3B6feBd5BFFc164f37738299AeB453181886",
        )])
        .unwrap();
        assert!(config
            .option_event_indexer
            .protocol_fee_vault_address
            .is_some());
    }

    #[test]
    fn protocol_fee_vault_address_invalid_hex_rejects() {
        let error =
            config_from_pairs([("PROTOCOL_FEE_VAULT_ADDRESS", "not-a-valid-address")]).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("invalid PROTOCOL_FEE_VAULT_ADDRESS"),
            "expected config rejection, got: {error}"
        );
    }

    #[test]
    fn protocol_fee_vault_address_short_hex_rejects() {
        let error = config_from_pairs([("PROTOCOL_FEE_VAULT_ADDRESS", "0xabc")]).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("invalid PROTOCOL_FEE_VAULT_ADDRESS"),
            "expected length rejection, got: {error}"
        );
    }

    #[test]
    fn protocol_fee_vault_address_empty_string_yields_none() {
        let config = config_from_pairs([("PROTOCOL_FEE_VAULT_ADDRESS", "")]).unwrap();
        assert_eq!(
            config.option_event_indexer.protocol_fee_vault_address, None,
            "empty string must be treated as not set, not as a malformed address"
        );
    }

    #[test]
    fn backend_remote_signer_provider_absent_yields_none() {
        let config = config_from_pairs([]).unwrap();
        assert_eq!(config.execution.backend_signer_provider, None);
    }

    #[test]
    fn backend_remote_signer_provider_parses_each_variant() {
        for (input, expected) in [
            (
                "mock",
                crate::execution::signer_adapters::SignerProviderKind::Mock,
            ),
            (
                "aws_kms",
                crate::execution::signer_adapters::SignerProviderKind::AwsKms,
            ),
            (
                "turnkey",
                crate::execution::signer_adapters::SignerProviderKind::Turnkey,
            ),
            (
                "fireblocks",
                crate::execution::signer_adapters::SignerProviderKind::Fireblocks,
            ),
            (
                "gcp_kms",
                crate::execution::signer_adapters::SignerProviderKind::GcpKms,
            ),
            (
                "azure_hsm",
                crate::execution::signer_adapters::SignerProviderKind::AzureHsm,
            ),
            (
                "vendor_agnostic",
                crate::execution::signer_adapters::SignerProviderKind::VendorAgnostic,
            ),
        ] {
            let config = config_from_pairs([("BACKEND_REMOTE_SIGNER_PROVIDER", input)]).unwrap();
            assert_eq!(
                config.execution.backend_signer_provider,
                Some(expected),
                "input `{input}` should parse to {expected:?}"
            );
        }
    }

    #[test]
    fn backend_remote_signer_provider_unknown_value_rejects_at_startup() {
        let error = config_from_pairs([("BACKEND_REMOTE_SIGNER_PROVIDER", "xxx")]).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("invalid BACKEND_REMOTE_SIGNER_PROVIDER"),
            "expected unknown-vendor rejection, got: {error}"
        );
    }

    #[test]
    fn backend_signer_timeout_ms_default_is_2500() {
        let config = config_from_pairs([]).unwrap();
        assert_eq!(config.execution.backend_signer_timeout_ms, 2500);
    }

    #[test]
    fn backend_signer_timeout_ms_parses_override() {
        let config = config_from_pairs([("BACKEND_SIGNER_TIMEOUT_MS", "5000")]).unwrap();
        assert_eq!(config.execution.backend_signer_timeout_ms, 5000);
    }

    #[test]
    fn backend_signer_timeout_ms_rejects_zero() {
        let error = config_from_pairs([("BACKEND_SIGNER_TIMEOUT_MS", "0")]).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("BACKEND_SIGNER_TIMEOUT_MS must be in 100..=30000"),
            "expected lower-bound rejection, got: {error}"
        );
    }

    #[test]
    fn backend_signer_timeout_ms_rejects_below_floor() {
        let error = config_from_pairs([("BACKEND_SIGNER_TIMEOUT_MS", "50")]).unwrap_err();
        assert!(error
            .to_string()
            .contains("BACKEND_SIGNER_TIMEOUT_MS must be in 100..=30000"));
    }

    #[test]
    fn backend_signer_timeout_ms_rejects_above_ceiling() {
        let error = config_from_pairs([("BACKEND_SIGNER_TIMEOUT_MS", "60000")]).unwrap_err();
        assert!(error
            .to_string()
            .contains("BACKEND_SIGNER_TIMEOUT_MS must be in 100..=30000"));
    }

    #[test]
    fn backend_signer_timeout_ms_accepts_boundary_values() {
        let lo = config_from_pairs([("BACKEND_SIGNER_TIMEOUT_MS", "100")]).unwrap();
        assert_eq!(lo.execution.backend_signer_timeout_ms, 100);
        let hi = config_from_pairs([("BACKEND_SIGNER_TIMEOUT_MS", "30000")]).unwrap();
        assert_eq!(hi.execution.backend_signer_timeout_ms, 30000);
    }

    // ----------------------------------------------------------------------------
    // PERPS-PUBLIC-ROUTE-UNLOCK-V1 — mainnet guard for the strict opt-in
    // `PERPS_PUBLIC_TRADING_ENABLED` flag. Enabling on any mainnet chain
    // id must fail startup with a clear config error. Base Sepolia
    // (84532) must succeed.
    // ----------------------------------------------------------------------------

    #[test]
    fn perps_public_trading_enabled_default_is_false() {
        let cfg = config_from_pairs([]).unwrap();
        assert!(!cfg.perps_public_trading_enabled);
    }

    #[test]
    fn perps_public_trading_enabled_ok_on_base_sepolia() {
        let cfg = config_from_pairs([
            ("CHAIN_ID", "84532"),
            ("PERPS_PUBLIC_TRADING_ENABLED", "true"),
        ])
        .unwrap();
        assert!(cfg.perps_public_trading_enabled);
        assert_eq!(cfg.chain_id, 84532);
    }

    #[test]
    fn perps_public_trading_enabled_refused_on_eth_mainnet() {
        let err = config_from_pairs([("CHAIN_ID", "1"), ("PERPS_PUBLIC_TRADING_ENABLED", "true")])
            .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("PERPS_PUBLIC_TRADING_ENABLED"), "got: {msg}");
        assert!(msg.contains("mainnet"), "got: {msg}");
        assert!(msg.contains(" 1"), "got: {msg}");
    }

    #[test]
    fn perps_public_trading_enabled_refused_on_base_mainnet() {
        let err = config_from_pairs([
            ("CHAIN_ID", "8453"),
            ("PERPS_PUBLIC_TRADING_ENABLED", "true"),
        ])
        .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("PERPS_PUBLIC_TRADING_ENABLED"), "got: {msg}");
        assert!(msg.contains("8453"), "got: {msg}");
    }

    #[test]
    fn perps_public_trading_flag_false_on_mainnet_is_ok() {
        // The mainnet guard is only tripped when the flag is `true` —
        // an operator who did NOT flip the flag can still run against
        // any chain id (though the safety envelope higher up in
        // `main.rs` will independently refuse mainnet).
        let cfg = config_from_pairs([("CHAIN_ID", "1")]).unwrap();
        assert!(!cfg.perps_public_trading_enabled);
        assert_eq!(cfg.chain_id, 1);
    }
}
