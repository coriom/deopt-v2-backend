use crate::admin::{AdminConfig, MetricsConfig};
use crate::api::public_ws::{LifecycleEventSender, PublicWsConfig};
use crate::auth::write_authorization::memory_store::{
    InMemoryChallengeStore, InMemoryUsedNonceV2Store,
};
use crate::auth::{UsedNonceV2Store, WriteAuthChallengeStore};
use crate::confirmation::ConfirmationConfig;
use crate::db::PgRepository;
use crate::engine::EngineState;
use crate::execution::{ExecutionConfig, StoredTradeSignatures};
use crate::fees::{FeeLedgerStore, FeesConfig};
use crate::indexer::IndexerConfig;
use crate::mm::{MmGatewayConfig, MmPermissionsConfig, MmPermissionsStore, MmSessionRegistry};
use crate::nonce_sync::{OptionNonceSyncConfig, PerpNonceSyncConfig};
use crate::options::{
    OptionConfirmationConfig, OptionConfirmationTickResult, OptionEventIndexerConfig,
    OptionEventIndexerTickResult, OptionReconciliationConfig, OptionReconciliationTickResult,
    OptionSeriesStore, OptionsConfig,
};
use crate::reconciliation::ReconciliationConfig;
use crate::rfq::{RfqConfig, RfqStore};
use crate::signing::{Eip712Domain, NonceStore, SignatureVerificationMode};
use crate::subaccounts::{InMemorySubaccountStore, SubaccountStore};
use crate::types::AccountId;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

#[derive(Clone)]
pub struct AppState {
    pub engine: Arc<Mutex<EngineState>>,
    pub nonces: Arc<Mutex<NonceStore>>,
    pub signature_verification_mode: SignatureVerificationMode,
    pub eip712_domain: Eip712Domain,
    pub chain_id: u64,
    pub network_name: String,
    pub persistence_enabled: bool,
    pub database_configured: bool,
    pub repository: Option<PgRepository>,
    pub execution_config: ExecutionConfig,
    pub perp_nonce_sync_config: PerpNonceSyncConfig,
    pub option_nonce_sync_config: OptionNonceSyncConfig,
    pub option_confirmation_config: OptionConfirmationConfig,
    pub option_confirmation_last_tick: Arc<Mutex<Option<OptionConfirmationTickResult>>>,
    pub option_event_indexer_config: OptionEventIndexerConfig,
    pub option_event_indexer_last_tick: Arc<Mutex<Option<OptionEventIndexerTickResult>>>,
    pub option_reconciliation_config: OptionReconciliationConfig,
    pub option_reconciliation_last_tick: Arc<Mutex<Option<OptionReconciliationTickResult>>>,
    pub confirmation_config: ConfirmationConfig,
    pub indexer_config: IndexerConfig,
    pub reconciliation_config: ReconciliationConfig,
    pub rfq_config: RfqConfig,
    pub options_config: OptionsConfig,
    pub conditional_orders_config: crate::options::conditional_orders::ConditionalOrdersConfig,
    pub fees_config: FeesConfig,
    /// M-P2d — Optional addresses for the trading-views read surface.
    /// All fields default to `None`; trading handlers fall back to the
    /// M-P2b partial-data path when any field is absent.
    pub trading_views: crate::api::trading_views::TradingViewsConfig,
    /// M-P4c — Local/test-only execution-intent + tx-status fixture
    /// guard. Disabled by default; runtime-refused on `chain_id == 8453`.
    /// See `crate::api::local_test_fixtures` for the full safety
    /// envelope. NEVER read in production code paths.
    pub local_test_fixtures: crate::api::local_test_fixtures::LocalTestFixturesConfig,
    /// M-P4c — In-memory synthetic intent store. Used only by handlers
    /// under `/admin/test/*` and `/trading/test/*`. Never persisted.
    pub local_test_intents:
        std::sync::Arc<std::sync::Mutex<crate::api::local_test_fixtures::LocalTestIntentStore>>,
    pub mm_gateway_config: MmGatewayConfig,
    pub mm_permissions_config: MmPermissionsConfig,
    /// BACKEND-PUBLIC-WS-API-V1 — knobs for the new `/ws` public
    /// WebSocket endpoint. The MM Gateway over WebTransport is a
    /// separate operator-whitelisted surface; its config lives in
    /// `mm_gateway_config` and is not affected by this field.
    pub public_ws_config: PublicWsConfig,
    pub admin_config: AdminConfig,
    pub metrics_config: MetricsConfig,
    pub rfq_store: Arc<Mutex<RfqStore>>,
    pub options_store: Arc<Mutex<OptionSeriesStore>>,
    pub fees_store: Arc<Mutex<FeeLedgerStore>>,
    pub mm_permissions: Arc<Mutex<MmPermissionsStore>>,
    pub mm_sessions: MmSessionRegistry,
    pub trade_signatures: Arc<Mutex<HashMap<Uuid, StoredTradeSignatures>>>,
    /// In-process observability for the option execution broadcast
    /// pipeline. Shared via `Arc`; counters increment at the broadcast
    /// call site; rendered into Prometheus text by
    /// `crate::monitoring::render_metrics` and into the readiness JSON.
    pub broadcast_observability: Arc<crate::options::BroadcastObservability>,
    /// ACCOUNT-WRITE-AUTH-HARDENING-V1 — persistent (or in-memory
    /// fallback) store for write-authorization challenges. When
    /// `repository` is `Some`, the PgRepository instance is also used
    /// here so challenges survive restarts and are atomic across
    /// concurrent processes. When no repository is configured, an
    /// in-memory store is used (suitable for unit tests only — NOT a
    /// production-safe replay-protection surface).
    pub write_auth_challenges: Arc<dyn WriteAuthChallengeStore + Send + Sync>,
    /// SUBACCOUNTS-V2-NONCE-TABLE-V1 — persistent (or in-memory
    /// fallback) v2 nonce consumption ledger. Keyed by
    /// `(lower(account), subaccount_id, action, nonce_bytes)`. Only
    /// consulted when a request supplies `AuthorizationEnvelope.version
    /// = Some(2)`; v1 requests never touch this store. Backed by
    /// `PgRepository` when persistence is on, `InMemoryUsedNonceV2Store`
    /// otherwise (tests only).
    pub used_nonces_v2: Arc<dyn UsedNonceV2Store + Send + Sync>,
    /// ORDER-LIFECYCLE-OBSERVABILITY-V1 — process-wide broadcast sink
    /// for `LifecycleEvent`s. Mutation services emit AFTER successful
    /// DB commit; per-session WS listeners filter and forward events
    /// matching their authenticated `session.account`. Bounded
    /// capacity (256) — laggy receivers get `RecvError::Lagged` and
    /// resync via the canonical REST snapshot.
    pub lifecycle_events: LifecycleEventSender,
    /// PERPS-MINIMAL-MARKET-AND-PRICE-V1 — read-only Perps market
    /// registry + OracleRouter configuration. Defaults to
    /// `PerpsReadConfig::disabled()` in every builder — the new
    /// `/perps/markets*` read routes return 503 `PerpsReadDisabled`
    /// until an operator explicitly turns them on via env. This has
    /// NO effect on the Perps mutation-route fail-closed gate; those
    /// remain unconditional `Err(PerpsNotLive)` at handler entry.
    pub perps_read_config: crate::perps::PerpsReadConfig,
    /// PERPS-ISOLATED-MARGIN-POSITION-ENGINE-V1 — in-memory Perps
    /// positions ledger. Populated by the internal
    /// `apply_perp_fill_for_account` helper (unit tests + future
    /// order-execution code). Never mutated by a public HTTP handler.
    /// The read-only `/accounts/:address/perps/positions` endpoint
    /// listing is the ONLY public consumer.
    pub perp_positions_store: std::sync::Arc<std::sync::Mutex<crate::perps::PerpPositionsStore>>,
    /// PERPS-ORDER-EXECUTION-INTERNAL-V1 — in-memory Perps order + fill
    /// ledger. Written by `submit_perp_order_internal` /
    /// `cancel_perp_order_internal` (internal service; no public
    /// route reaches these functions in V1). Public Perps mutation
    /// routes STILL return 503 `PerpsNotLive` at handler entry.
    pub perp_order_store: std::sync::Arc<std::sync::Mutex<crate::perps::PerpOrderStore>>,
    /// PERPS-LIQUIDATION-AND-RISK-V1 — in-memory Perps liquidation
    /// events ledger. Written by the admin-gated liquidation tick +
    /// the internal `liquidate_perp_position_internal` service. Read
    /// by `GET /accounts/:address/perps/liquidations`.
    pub perp_liquidations_store:
        std::sync::Arc<std::sync::Mutex<crate::perps::PerpLiquidationsStore>>,
    /// PERPS-FUNDING-V1 — in-memory Perps funding events ledger.
    /// Written by the admin-gated funding tick. Read by
    /// `GET /accounts/:address/perps/funding`. Public Perps mutation
    /// routes remain fail-closed regardless.
    pub perp_funding_events_store:
        std::sync::Arc<std::sync::Mutex<crate::perps::PerpFundingEventsStore>>,
    /// PERPS-FRONTEND-TICKET-ENABLEMENT-V1 — strict opt-in flag for
    /// the new `POST /perps/orders` and `DELETE /perps/orders/:id`
    /// mutation routes. **Default: `false`.** Every existing legacy
    /// Perps mutation route (`/orders`, `/orders/:id`, `/rfqs`,
    /// `/rfqs/:rfq_id/quotes`, `/rfqs/:rfq_id/accept/:quote_id`,
    /// `/rfqs/:rfq_id/cancel`, `/execution-intents/:intent_id/signatures`)
    /// stays permanently fail-closed regardless of this flag.
    ///
    /// Enabling this on a mainnet chain id is refused at startup by
    /// `validate_startup`. Env: `PERPS_PUBLIC_TRADING_ENABLED=true`.
    pub perps_public_trading_enabled: bool,
    /// PERPS-SUBACCOUNTS-CORE-ROUTING-V1 — closed-test opt-in flag.
    /// Independent of `perps_public_trading_enabled`. When true AND
    /// the caller is on `perps_closed_test_allowlist`, Perps mutation
    /// handlers may proceed; otherwise every handler still returns
    /// 503 `PerpsNotLive`. Refused on mainnet at startup. Default
    /// `false`. Env: `PERPS_CLOSED_TEST_ENABLED`.
    pub perps_closed_test_enabled: bool,
    /// PERPS-SUBACCOUNTS-CORE-ROUTING-V1 — allowlisted wallet
    /// addresses (lower-cased). Empty by default. Consumed by the
    /// closed-test guard on Perps mutations. Env:
    /// `PERPS_CLOSED_TEST_ALLOWLIST` (comma-separated hex).
    pub perps_closed_test_allowlist: Vec<AccountId>,
    /// SUBACCOUNTS-CORE-BACKEND-V1 — real Derive-like subaccount
    /// identity store. When `repository` is `Some`, the PgRepository
    /// is wired here so rows survive restarts. Otherwise an in-memory
    /// store is used (unit-test only). `Account 1` is lazily created
    /// on the first authenticated interaction with any listed owner
    /// (see `crate::subaccounts::ensure_default_subaccount`).
    pub subaccounts: Arc<dyn SubaccountStore + Send + Sync>,
}

impl AppState {
    pub fn new(engine: EngineState) -> Self {
        Self::with_signature_mode(engine, SignatureVerificationMode::Disabled)
    }

    pub fn with_signature_mode(
        engine: EngineState,
        signature_verification_mode: SignatureVerificationMode,
    ) -> Self {
        Self::with_signature_mode_and_domain(
            engine,
            signature_verification_mode,
            Eip712Domain::default(),
        )
    }

    pub fn with_signature_mode_and_domain(
        engine: EngineState,
        signature_verification_mode: SignatureVerificationMode,
        eip712_domain: Eip712Domain,
    ) -> Self {
        Self::with_signature_mode_domain_and_repository(
            engine,
            signature_verification_mode,
            eip712_domain,
            None,
        )
    }

    pub fn with_signature_mode_domain_and_repository(
        engine: EngineState,
        signature_verification_mode: SignatureVerificationMode,
        eip712_domain: Eip712Domain,
        repository: Option<PgRepository>,
    ) -> Self {
        Self::with_signature_mode_domain_repository_and_execution_config(
            engine,
            signature_verification_mode,
            eip712_domain,
            repository,
            ExecutionConfig::disabled(),
            84532,
        )
    }

    pub fn with_signature_mode_domain_repository_and_execution_config(
        engine: EngineState,
        signature_verification_mode: SignatureVerificationMode,
        eip712_domain: Eip712Domain,
        repository: Option<PgRepository>,
        execution_config: ExecutionConfig,
        chain_id: u64,
    ) -> Self {
        Self::with_signature_mode_domain_repository_execution_indexer_and_reconciliation_config(
            engine,
            signature_verification_mode,
            eip712_domain,
            repository,
            execution_config,
            IndexerConfig::disabled(),
            ReconciliationConfig::disabled(),
            chain_id,
        )
    }

    pub fn with_signature_mode_domain_repository_execution_and_indexer_config(
        engine: EngineState,
        signature_verification_mode: SignatureVerificationMode,
        eip712_domain: Eip712Domain,
        repository: Option<PgRepository>,
        execution_config: ExecutionConfig,
        indexer_config: IndexerConfig,
        chain_id: u64,
    ) -> Self {
        Self::with_signature_mode_domain_repository_execution_indexer_and_reconciliation_config(
            engine,
            signature_verification_mode,
            eip712_domain,
            repository,
            execution_config,
            indexer_config,
            ReconciliationConfig::disabled(),
            chain_id,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn with_signature_mode_domain_repository_execution_indexer_and_reconciliation_config(
        engine: EngineState,
        signature_verification_mode: SignatureVerificationMode,
        eip712_domain: Eip712Domain,
        repository: Option<PgRepository>,
        execution_config: ExecutionConfig,
        indexer_config: IndexerConfig,
        reconciliation_config: ReconciliationConfig,
        chain_id: u64,
    ) -> Self {
        Self::with_all_config(
            engine,
            signature_verification_mode,
            eip712_domain,
            repository,
            execution_config,
            PerpNonceSyncConfig::disabled(),
            OptionNonceSyncConfig::disabled(),
            ConfirmationConfig::disabled(),
            indexer_config,
            reconciliation_config,
            RfqConfig::disabled(),
            OptionsConfig::disabled(),
            FeesConfig::disabled(),
            chain_id,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn with_all_config(
        engine: EngineState,
        signature_verification_mode: SignatureVerificationMode,
        eip712_domain: Eip712Domain,
        repository: Option<PgRepository>,
        execution_config: ExecutionConfig,
        perp_nonce_sync_config: PerpNonceSyncConfig,
        option_nonce_sync_config: OptionNonceSyncConfig,
        confirmation_config: ConfirmationConfig,
        indexer_config: IndexerConfig,
        reconciliation_config: ReconciliationConfig,
        rfq_config: RfqConfig,
        options_config: OptionsConfig,
        fees_config: FeesConfig,
        chain_id: u64,
    ) -> Self {
        let write_auth_challenges: Arc<dyn WriteAuthChallengeStore + Send + Sync> =
            match repository.as_ref() {
                Some(repo) => Arc::new(repo.clone()),
                None => Arc::new(InMemoryChallengeStore::new()),
            };
        let used_nonces_v2: Arc<dyn UsedNonceV2Store + Send + Sync> = match repository.as_ref() {
            Some(repo) => Arc::new(repo.clone()),
            None => Arc::new(InMemoryUsedNonceV2Store::new()),
        };
        let subaccounts: Arc<dyn SubaccountStore + Send + Sync> = match repository.as_ref() {
            Some(repo) => Arc::new(repo.clone()),
            None => Arc::new(InMemorySubaccountStore::new()),
        };
        Self {
            engine: Arc::new(Mutex::new(engine)),
            nonces: Arc::new(Mutex::new(NonceStore::new())),
            signature_verification_mode,
            eip712_domain,
            chain_id,
            network_name: "base-sepolia".to_string(),
            persistence_enabled: repository.is_some(),
            database_configured: repository.is_some(),
            repository,
            execution_config,
            perp_nonce_sync_config,
            option_nonce_sync_config,
            option_confirmation_config: OptionConfirmationConfig::disabled(),
            option_confirmation_last_tick: Arc::new(Mutex::new(None)),
            option_event_indexer_config: OptionEventIndexerConfig::disabled(),
            option_event_indexer_last_tick: Arc::new(Mutex::new(None)),
            option_reconciliation_config: OptionReconciliationConfig::disabled(),
            option_reconciliation_last_tick: Arc::new(Mutex::new(None)),
            confirmation_config,
            indexer_config,
            reconciliation_config,
            rfq_config,
            options_config,
            conditional_orders_config:
                crate::options::conditional_orders::ConditionalOrdersConfig::default(),
            fees_config,
            mm_gateway_config: MmGatewayConfig::default(),
            mm_permissions_config: MmPermissionsConfig::disabled(),
            public_ws_config: PublicWsConfig::default_testnet(),
            admin_config: AdminConfig::disabled(),
            metrics_config: MetricsConfig::enabled_by_default(),
            rfq_store: Arc::new(Mutex::new(RfqStore::new())),
            options_store: Arc::new(Mutex::new(OptionSeriesStore::new())),
            fees_store: Arc::new(Mutex::new(FeeLedgerStore::new())),
            mm_permissions: Arc::new(Mutex::new(MmPermissionsStore::new())),
            mm_sessions: MmSessionRegistry::new(),
            trade_signatures: Arc::new(Mutex::new(HashMap::new())),
            broadcast_observability: Arc::new(crate::options::BroadcastObservability::new()),
            trading_views: crate::api::trading_views::TradingViewsConfig::disabled(),
            local_test_fixtures: crate::api::local_test_fixtures::LocalTestFixturesConfig::disabled(
            ),
            local_test_intents: crate::api::local_test_fixtures::shared_store(),
            write_auth_challenges,
            used_nonces_v2,
            lifecycle_events: LifecycleEventSender::default(),
            perps_read_config: crate::perps::PerpsReadConfig::disabled(),
            perp_positions_store: std::sync::Arc::new(std::sync::Mutex::new(
                crate::perps::PerpPositionsStore::new(),
            )),
            perp_order_store: std::sync::Arc::new(std::sync::Mutex::new(
                crate::perps::PerpOrderStore::new(),
            )),
            perp_liquidations_store: std::sync::Arc::new(std::sync::Mutex::new(
                crate::perps::PerpLiquidationsStore::new(),
            )),
            perp_funding_events_store: std::sync::Arc::new(std::sync::Mutex::new(
                crate::perps::PerpFundingEventsStore::new(),
            )),
            perps_public_trading_enabled: false,
            // PERPS-SUBACCOUNTS-CORE-ROUTING-V1 — default off; empty
            // allowlist. Constructed AppStates start with Perps
            // mutation surface fail-closed for every wallet.
            perps_closed_test_enabled: false,
            perps_closed_test_allowlist: Vec::new(),
            subaccounts,
        }
    }

    pub fn with_rfq_config(engine: EngineState, rfq_config: RfqConfig) -> Self {
        Self::with_all_config(
            engine,
            SignatureVerificationMode::Disabled,
            Eip712Domain::default(),
            None,
            ExecutionConfig::disabled(),
            PerpNonceSyncConfig::disabled(),
            OptionNonceSyncConfig::disabled(),
            ConfirmationConfig::disabled(),
            IndexerConfig::disabled(),
            ReconciliationConfig::disabled(),
            rfq_config,
            OptionsConfig::disabled(),
            FeesConfig::disabled(),
            84532,
        )
    }

    pub fn with_options_config(engine: EngineState, options_config: OptionsConfig) -> Self {
        Self::with_all_config(
            engine,
            SignatureVerificationMode::Disabled,
            Eip712Domain::default(),
            None,
            ExecutionConfig::disabled(),
            PerpNonceSyncConfig::disabled(),
            OptionNonceSyncConfig::disabled(),
            ConfirmationConfig::disabled(),
            IndexerConfig::disabled(),
            ReconciliationConfig::disabled(),
            RfqConfig::disabled(),
            options_config,
            FeesConfig::disabled(),
            84532,
        )
    }

    /// OPTIONS-CONDITIONAL-ORDERS-LIVE-POSTGRES-PROOF-V1 — test-only
    /// builder that wires a real `PgRepository` onto the AppState so
    /// the conditional-orders service routes through the DB mirror
    /// (rather than the in-memory store) under integration tests.
    pub fn with_options_config_and_repository(
        engine: EngineState,
        options_config: OptionsConfig,
        repository: PgRepository,
    ) -> Self {
        let mut state = Self::with_all_config(
            engine,
            SignatureVerificationMode::Disabled,
            Eip712Domain::default(),
            Some(repository.clone()),
            ExecutionConfig::disabled(),
            PerpNonceSyncConfig::disabled(),
            OptionNonceSyncConfig::disabled(),
            ConfirmationConfig::disabled(),
            IndexerConfig::disabled(),
            ReconciliationConfig::disabled(),
            RfqConfig::disabled(),
            options_config,
            FeesConfig::disabled(),
            84532,
        );
        state.persistence_enabled = true;
        state.database_configured = true;
        state.write_auth_challenges = Arc::new(repository.clone());
        state.used_nonces_v2 = Arc::new(repository.clone());
        state.subaccounts = Arc::new(repository.clone());
        state.repository = Some(repository);
        state
    }

    pub fn with_options_and_fees_config(
        engine: EngineState,
        options_config: OptionsConfig,
        fees_config: FeesConfig,
    ) -> Self {
        Self::with_all_config(
            engine,
            SignatureVerificationMode::Disabled,
            Eip712Domain::default(),
            None,
            ExecutionConfig::disabled(),
            PerpNonceSyncConfig::disabled(),
            OptionNonceSyncConfig::disabled(),
            ConfirmationConfig::disabled(),
            IndexerConfig::disabled(),
            ReconciliationConfig::disabled(),
            RfqConfig::disabled(),
            options_config,
            fees_config,
            84532,
        )
    }

    /// PERPS-SUBACCOUNTS-CORE-ROUTING-V1 — the layered closed-test
    /// gate. Returns `true` when the caller wallet may reach the Perps
    /// mutation surface under the current config. Callers still must
    /// invoke this AFTER the fail-closed public trading gate — this
    /// helper does not open a bypass around `perps_public_trading_enabled`.
    ///
    /// Semantics:
    /// * `perps_closed_test_enabled == false` → always `false`.
    /// * Allowlist empty → always `false` (an honest closed test with
    ///   no allowlisted wallets is a well-defined "nobody in").
    /// * Address match is case-insensitive.
    pub fn perps_closed_test_allows(&self, caller: &AccountId) -> bool {
        if !self.perps_closed_test_enabled {
            return false;
        }
        let want = caller.0.to_lowercase();
        self.perps_closed_test_allowlist
            .iter()
            .any(|allowed| allowed.0.to_lowercase() == want)
    }
}
