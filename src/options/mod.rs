pub mod confirmation_worker;
pub mod execution;
pub mod series_id;
pub mod service;
pub mod signing;
pub mod store;
pub mod types;

pub use confirmation_worker::{
    spawn_option_confirmation_worker, OptionConfirmationConfig, OptionConfirmationDecision,
    OptionConfirmationOutcome, OptionConfirmationTickResult,
};

pub use execution::{
    build_option_execution_transaction_request, compute_required_gas,
    encode_option_execute_trade_calldata, expected_option_execute_trade_selector,
    normalize_u256_string, option_execute_trade_selector, option_execution_broadcast_gas_limit,
    option_execution_intent_id_to_b256, option_execution_intent_id_to_hex_bytes32,
    option_execution_simulation_pending, option_execution_simulation_unavailable,
    option_product_registry_option_id, option_trade_digest, option_trade_digest_bytes,
    perform_option_broadcast_gas_safety_check, simulate_option_execution_intent,
    validate_simulation_intent, validate_simulation_target, OptionExecutionGasSafetyCheck,
    OptionTradePayload, OptionTradeSignatureBundle, OPTION_TRADE_TYPE,
};
pub use series_id::{option_series_id, OptionSeriesIdInput};
pub use signing::{
    option_rfq_id_to_b256, option_rfq_id_to_hex_bytes32, option_rfq_quote_digest,
    option_series_id_to_b256, option_series_id_to_hex_bytes32, OptionRfqQuoteSigningPayload,
    OPTION_RFQ_QUOTE_TYPE,
};
pub use store::OptionSeriesStore;
pub use types::{
    OptionExecutionConfirmationStatus, OptionExecutionGasCheckStatus, OptionExecutionIntent,
    OptionExecutionIntentId, OptionExecutionIntentStatus, OptionExecutionReceiptCost,
    OptionExecutionSignatureMode, OptionExecutionSimulationResult, OptionExecutionSimulationStatus,
    OptionExecutionSourceType, OptionExecutionTransaction, OptionFill, OptionFillFilter,
    OptionFillId, OptionOrder, OptionOrderFilter, OptionOrderId, OptionOrderStatus,
    OptionOrderbookLevel, OptionOrderbookSnapshot, OptionRfqFill, OptionRfqFillId, OptionRfqId,
    OptionRfqQuote, OptionRfqQuoteId, OptionRfqQuoteSignatureMode, OptionRfqQuoteSignatureStatus,
    OptionRfqQuoteStatus, OptionRfqRequest, OptionRfqStatus, OptionSeries, OptionSeriesFilter,
    OptionSeriesId, OptionSeriesSource, OptionSeriesStatus, OptionsConfig,
    OPTION_EXECUTION_GAS_SAFETY_BPS_DEFAULT, OPTION_EXECUTION_GAS_SAFETY_BPS_MAX,
    OPTION_EXECUTION_GAS_SAFETY_BPS_MIN,
};
