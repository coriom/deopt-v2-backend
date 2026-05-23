use crate::admin::{AdminConfig, MetricsConfig};
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
    pub fees_config: FeesConfig,
    pub mm_gateway_config: MmGatewayConfig,
    pub mm_permissions_config: MmPermissionsConfig,
    pub admin_config: AdminConfig,
    pub metrics_config: MetricsConfig,
    pub rfq_store: Arc<Mutex<RfqStore>>,
    pub options_store: Arc<Mutex<OptionSeriesStore>>,
    pub fees_store: Arc<Mutex<FeeLedgerStore>>,
    pub mm_permissions: Arc<Mutex<MmPermissionsStore>>,
    pub mm_sessions: MmSessionRegistry,
    pub trade_signatures: Arc<Mutex<HashMap<Uuid, StoredTradeSignatures>>>,
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
            fees_config,
            mm_gateway_config: MmGatewayConfig::default(),
            mm_permissions_config: MmPermissionsConfig::disabled(),
            admin_config: AdminConfig::disabled(),
            metrics_config: MetricsConfig::enabled_by_default(),
            rfq_store: Arc::new(Mutex::new(RfqStore::new())),
            options_store: Arc::new(Mutex::new(OptionSeriesStore::new())),
            fees_store: Arc::new(Mutex::new(FeeLedgerStore::new())),
            mm_permissions: Arc::new(Mutex::new(MmPermissionsStore::new())),
            mm_sessions: MmSessionRegistry::new(),
            trade_signatures: Arc::new(Mutex::new(HashMap::new())),
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
}
