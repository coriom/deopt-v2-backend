pub mod abi;
pub mod broadcast_policy;
pub mod broadcast_reconciler;
pub mod broadcast_runtime;
pub mod config;
pub mod executor;
pub mod intent;
pub mod perp_order_intent;
pub mod perp_trade;
pub mod queue;
pub mod remote_signer;
pub mod revert;
pub mod rpc;
pub mod runner;
pub mod signer;
pub mod signer_adapters;
pub mod simulator;
pub mod transaction;
pub mod tx_builder;

pub use abi::{encode_execute_trade_calldata, execute_trade_selector};
pub use broadcast_policy::{
    classify_send_error, expected_intent_hash_from_uuid, preflight_intent_filled,
    preflight_pme_state, validate_signer_triad, verify_pme_event_in_receipt, BroadcastOutcome,
    BroadcastPolicy, BroadcastSigner, ExpectedExecutionIdentity, GateMatrix, PmeState,
    ReconcileSummary, SendErrorClass, PME_INTENT_FILLED_SELECTOR, PME_IS_EXECUTOR_SELECTOR,
    PME_PAUSED_SELECTOR, PME_TRADE_EXECUTED_FROM_INTENTS_TOPIC0, PME_TRADE_EXECUTED_TOPIC0,
};
pub use broadcast_reconciler::{
    initial_reconciliation, spawn_broadcast_reconciler, startup_preflight,
    BroadcastReconcilerCancel, ReconcilerConfig,
};
pub use broadcast_runtime::{
    build_broadcast_runtime, execute_pending_batch, refuse_broadcast_runtime_disabled,
    spawn_broadcast_executor, wire_broadcast_runtime, BroadcastReadiness, BroadcastRuntime,
};
pub use config::{ExecutionConfig, ExecutionStatus, PrivateKeySecret};
pub use executor::{
    ExecutionIntentRepository, ExecutionTickResult, Executor, PreparedBroadcastRow,
    PreparedTransactionRecord, RepositoryFuture,
};
pub use intent::{ExecutionIntent, ExecutionIntentStatus};
pub use perp_order_intent::{
    perp_order_intent_digest, perp_order_intent_hash, perp_order_intent_typehash,
    validate_shape as validate_perp_order_intent_shape, verify_perp_order_intent, PerpOrderIntent,
    PERP_ORDER_INTENT_SIDE_BUY, PERP_ORDER_INTENT_SIDE_SELL, PERP_ORDER_INTENT_TYPE,
    PERP_ORDER_INTENT_TYPEHASH_HEX,
};
pub use perp_trade::{
    b256_to_hex_bytes32, intent_id_to_b256, intent_id_to_hex_bytes32, perp_trade_digest,
    perp_trade_digest_bytes_for_version, perp_trade_digest_for_version, perp_trade_v1_digest,
    perp_trade_v1_digest_bytes, perp_trade_v1_typehash, perp_trade_v2_digest,
    perp_trade_v2_digest_bytes, PerpTradeDomain, PerpTradePayload, PerpTradeSignatureBundle,
    PerpsProtocolVersion, StoredTradeSignatures, TradeSignatureStatus, PERP_TRADE_TYPE,
    PERP_TRADE_TYPEHASH_HEX, PERP_TRADE_V1_TYPE, PERP_TRADE_V1_TYPEHASH_HEX,
};
pub use queue::ExecutionQueue;
pub use remote_signer::{
    policy_fingerprint, signer_error_into_backend, LocalDevSigner, RemoteSigner,
    RemoteSignerClient, SignerBackendKind, SignerError, SignerFuture, SignerHealth, SignerRequest,
    SignerResponse, SignerTransport, ANVIL_CHAIN_ID, BASE_SEPOLIA_CHAIN_ID, MAINNET_CHAIN_ID,
};
pub use revert::{DecodedRevertError, RevertDiagnostics};
pub use rpc::{
    EstimateGasRequest, EthBalanceProvider, EthCallProvider, EthCallRequest, EthCallSuccess,
    EthGetLogsFilter, EthLogsProvider, GasEstimateProvider, HttpJsonRpcProvider, RpcFuture,
    TransactionBroadcastProvider, TransactionReceiptProvider,
};
pub use runner::spawn_executor;
pub use signer::ExecutorSigner;
pub use simulator::{simulate_execution_intent, SimulationResult};
pub use transaction::{
    assemble_eip1559_signed_transaction, build_execution_transaction_request,
    derive_signed_transaction_hash, eip1559_transaction_prehash, ensure_no_submitted_transaction,
    sign_eip1559_transaction, ExecutionTransaction, ExecutionTransactionRequest,
    ExecutionTransactionStatus,
};
pub use tx_builder::{
    build_perp_execution_call, build_perp_execution_call_from_intent,
    preview_perp_execution_call_from_intent, PreparedExecutionCall,
};
