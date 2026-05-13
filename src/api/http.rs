use crate::admin::AdminConfig;
use crate::confirmation::ConfirmationConfig;
use crate::db::PgRepository;
use crate::engine::EngineState;
use crate::execution::{ExecutionConfig, StoredTradeSignatures};
use crate::indexer::IndexerConfig;
use crate::mm::{MmGatewayConfig, MmSessionRegistry};
use crate::nonce_sync::PerpNonceSyncConfig;
use crate::options::{OptionSeriesStore, OptionsConfig};
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
    pub confirmation_config: ConfirmationConfig,
    pub indexer_config: IndexerConfig,
    pub reconciliation_config: ReconciliationConfig,
    pub rfq_config: RfqConfig,
    pub options_config: OptionsConfig,
    pub mm_gateway_config: MmGatewayConfig,
    pub admin_config: AdminConfig,
    pub rfq_store: Arc<Mutex<RfqStore>>,
    pub options_store: Arc<Mutex<OptionSeriesStore>>,
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
            ConfirmationConfig::disabled(),
            indexer_config,
            reconciliation_config,
            RfqConfig::disabled(),
            OptionsConfig::disabled(),
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
        confirmation_config: ConfirmationConfig,
        indexer_config: IndexerConfig,
        reconciliation_config: ReconciliationConfig,
        rfq_config: RfqConfig,
        options_config: OptionsConfig,
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
            confirmation_config,
            indexer_config,
            reconciliation_config,
            rfq_config,
            options_config,
            mm_gateway_config: MmGatewayConfig::default(),
            admin_config: AdminConfig::disabled(),
            rfq_store: Arc::new(Mutex::new(RfqStore::new())),
            options_store: Arc::new(Mutex::new(OptionSeriesStore::new())),
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
            ConfirmationConfig::disabled(),
            IndexerConfig::disabled(),
            ReconciliationConfig::disabled(),
            rfq_config,
            OptionsConfig::disabled(),
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
            ConfirmationConfig::disabled(),
            IndexerConfig::disabled(),
            ReconciliationConfig::disabled(),
            RfqConfig::disabled(),
            options_config,
            84532,
        )
    }
}
