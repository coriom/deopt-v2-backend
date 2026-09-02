pub mod abi;
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
pub use config::{ExecutionConfig, ExecutionStatus, PrivateKeySecret};
pub use executor::{ExecutionIntentRepository, ExecutionTickResult, Executor, RepositoryFuture};
pub use intent::{ExecutionIntent, ExecutionIntentStatus};
pub use perp_order_intent::{
    perp_order_intent_digest, perp_order_intent_hash, perp_order_intent_typehash,
    validate_shape as validate_perp_order_intent_shape, verify_perp_order_intent, PerpOrderIntent,
    PERP_ORDER_INTENT_SIDE_BUY, PERP_ORDER_INTENT_SIDE_SELL, PERP_ORDER_INTENT_TYPE,
    PERP_ORDER_INTENT_TYPEHASH_HEX,
};
pub use perp_trade::{
    b256_to_hex_bytes32, intent_id_to_b256, intent_id_to_hex_bytes32, perp_trade_digest,
    PerpTradeDomain, PerpTradePayload, PerpTradeSignatureBundle, StoredTradeSignatures,
    TradeSignatureStatus, PERP_TRADE_TYPE,
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
