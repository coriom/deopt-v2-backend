pub mod broadcast_observability;
pub mod broadcast_policy;
pub mod broadcast_policy_data;
pub mod conditional_orders;
pub mod confirmation_worker;
pub mod event_indexer;
pub mod execution;
pub mod lifecycle;
pub mod reconciliation_worker;
pub mod rfq_operator_packet;
pub mod series_id;
pub mod service;
pub mod signing;
pub mod state_checks;
pub mod store;
pub mod types;

pub use broadcast_observability::{BroadcastObservability, BroadcastObservabilitySnapshot};
pub use broadcast_policy::{
    should_broadcast, verify_launch_invariant, ActiveFeeProfile, ApprovalReason, BroadcastContext,
    BroadcastMode, EffectiveProfileEntry, FeeFlow, FeeSplitSummary, LaunchInvariantReport,
    ProductKind, RejectReason, ShouldBroadcastDecision, SimulationSummary, SubsidyBudgetView,
    SubsidyReason, MAINNET_CHAIN_ID, SEPOLIA_CHAIN_ID,
};
pub use broadcast_policy_data::{
    aggregate_fee_split, decode_fee_quote, encode_quote_fees_call, quote_fees_selector_bytes,
    read_type, verify_launch_invariant_for_startup, BroadcastPolicyDataProvider,
    BroadcastPolicyInputs, DedupeReason, FeeQuoteRaw, FmV2QuoteFailureKind,
    LiveBroadcastPolicyDataProvider, PolicyDataError, PolicyDataFuture,
    StartupLaunchInvariantOutcome, StubBroadcastPolicyDataProvider, FM_V2_QUOTE_FEES_SELECTOR,
};

pub use confirmation_worker::{
    spawn_option_confirmation_worker, OptionConfirmationConfig, OptionConfirmationDecision,
    OptionConfirmationOutcome, OptionConfirmationTickResult,
};
pub use event_indexer::{
    default_option_event_counts, index_option_events_with_provider, list_option_execution_events,
    option_trade_executed_topic0, spawn_option_event_indexer, summarize_option_execution_events,
    summarize_option_execution_events_by_contract_address, OptionEventEmitterContract,
    OptionEventIndexerConfig, OptionEventIndexerTickResult, OPTION_EVENT_INDEXER_STATE_ID,
    OPTION_TRADE_EXECUTED_SIGNATURE,
};
pub use lifecycle::{
    get_option_execution_lifecycle, LifecycleBroadcast, LifecycleCalldata, LifecycleConfirmation,
    LifecycleEvents, LifecycleFees, LifecycleMetadata, LifecycleReconciliation,
    LifecycleSignatures, LifecycleSimulation, LifecycleSource, LifecycleStateChecks,
    LifecycleTrade, LifecycleTransfers, OptionExecutionLifecycle, OptionExecutionLifecycleHealth,
    OptionExecutionLifecycleStage,
};
pub use reconciliation_worker::{
    reconcile_option_executions, spawn_option_reconciliation_worker,
    summarize_option_execution_reconciliations, OptionReconciliationConfig,
    OptionReconciliationDecision, OptionReconciliationTickResult,
};
pub use state_checks::{
    evaluate_option_state_checks, OptionStateCheckEvaluation, MARGIN_COLLATERAL_VAULT_VIEW,
    MATCHING_MARGIN_ENGINE_VIEW, NONCES_VIEW, POSITIONS_VIEW, POSITION_QUANTITY_VIEW,
    SERIES_SHORT_OPEN_INTEREST_VIEW, VAULT_BALANCES_VIEW, VAULT_BALANCE_WITH_YIELD_VIEW,
};

pub use execution::{
    build_option_execution_transaction_request, compute_required_gas,
    encode_option_execute_rfq_trade_calldata, encode_option_execute_trade_calldata,
    expected_option_execute_rfq_trade_selector, expected_option_execute_trade_selector,
    normalize_u256_string, option_execute_rfq_trade_selector, option_execute_trade_selector,
    option_execution_broadcast_gas_limit, option_execution_intent_id_to_b256,
    option_execution_intent_id_to_hex_bytes32, option_execution_simulation_pending,
    option_execution_simulation_unavailable, option_product_registry_option_id,
    option_rfq_trade_digest, option_rfq_trade_digest_bytes, option_rfq_trade_typehash,
    option_trade_digest, option_trade_digest_bytes, option_trade_typehash,
    perform_option_broadcast_gas_safety_check, simulate_option_execution_intent,
    validate_simulation_intent, validate_simulation_target, OptionExecutionGasSafetyCheck,
    OptionTradePayload, OptionTradeSignatureBundle, OPTION_EXECUTE_RFQ_TRADE_SIGNATURE,
    OPTION_EXECUTE_TRADE_SIGNATURE, OPTION_RFQ_TRADE_TYPE, OPTION_TRADE_TYPE,
};
pub use rfq_operator_packet::{
    build_option_rfq_operator_packet, OptionRfqOperatorBroadcastError, OptionRfqOperatorPacket,
    OptionRfqOperatorPacketInputs, OPTION_RFQ_OPERATOR_BROADCAST_CONFIRM_ENV,
};
pub use series_id::{option_series_id, OptionSeriesIdInput};
pub use signing::{
    option_rfq_id_to_b256, option_rfq_id_to_hex_bytes32, option_rfq_quote_digest,
    option_series_id_to_b256, option_series_id_to_hex_bytes32, OptionRfqQuoteSigningPayload,
    OPTION_RFQ_QUOTE_TYPE,
};
pub use store::OptionSeriesStore;
pub use types::{
    OptionEventIndexerState, OptionExecutionConfirmationStatus, OptionExecutionEvent,
    OptionExecutionEventLink, OptionExecutionGasCheckStatus, OptionExecutionIntent,
    OptionExecutionIntentId, OptionExecutionIntentStatus, OptionExecutionReceiptCost,
    OptionExecutionReconciliation, OptionExecutionSignatureMode, OptionExecutionSimulationResult,
    OptionExecutionSimulationStatus, OptionExecutionSourceType, OptionExecutionTransaction,
    OptionFill, OptionFillFilter, OptionFillId, OptionOrder, OptionOrderFilter, OptionOrderId,
    OptionOrderStatus, OptionOrderbookLevel, OptionOrderbookSnapshot, OptionReconciliationStatus,
    OptionRfqFill, OptionRfqFillId, OptionRfqId, OptionRfqQuote, OptionRfqQuoteId,
    OptionRfqQuoteSignatureMode, OptionRfqQuoteSignatureStatus, OptionRfqQuoteStatus,
    OptionRfqRequest, OptionRfqStatus, OptionSeries, OptionSeriesFilter, OptionSeriesId,
    OptionSeriesSource, OptionSeriesStatus, OptionsConfig, OPTION_EXECUTION_GAS_SAFETY_BPS_DEFAULT,
    OPTION_EXECUTION_GAS_SAFETY_BPS_MAX, OPTION_EXECUTION_GAS_SAFETY_BPS_MIN,
};
