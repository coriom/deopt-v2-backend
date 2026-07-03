use crate::types::OrderId;
use thiserror::Error;

pub type Result<T> = std::result::Result<T, BackendError>;

#[derive(Debug, Error)]
pub enum BackendError {
    #[error("zero price is not allowed")]
    ZeroPrice,
    #[error("zero size is not allowed")]
    ZeroSize,
    #[error("post-only order would immediately match")]
    PostOnlyWouldMatch,
    #[error("self-trade is rejected")]
    SelfTrade,
    #[error("time in force is unsupported: {0}")]
    UnsupportedTimeInForce(String),
    #[error("fill-or-kill order is not fully fillable")]
    FokNotFillable,
    #[error("invalid time-in-force combination: {0}")]
    InvalidTimeInForceCombination(String),
    // ATTACHED-TP-SL-ON-ENTRY-V1
    #[error("invalid attached TP/SL: {0}")]
    InvalidAttachedTpSl(String),
    // OPTIONS-CONDITIONAL-ORDERS-TP-SL-V1
    #[error("invalid conditional order id")]
    InvalidConditionalOrderId,
    #[error("invalid conditional order state: {0}")]
    InvalidConditionalOrderState(String),
    #[error("conditional order is already terminal")]
    ConditionalOrderAlreadyTerminal,
    #[error("no reducible option position for series {0}")]
    NoReduciblePosition(String),
    #[error("conditional order quantity exceeds the reducible position size")]
    QuantityExceedsPosition,
    #[error("conditional trigger direction is inconsistent with the position")]
    InvalidTriggerDirection,
    #[error("conditional order trigger price must be > 0")]
    InvalidTriggerPrice,
    #[error("conditional order limit price must be > 0")]
    InvalidConditionalLimitPrice,
    #[error("underlying oracle price is unavailable or stale")]
    OracleUnavailable,
    #[error("duplicate conditional order idempotency key")]
    DuplicateConditionalOrderKey,
    #[error("oco sibling already triggered or terminal")]
    OcoSiblingTerminal,
    #[error("command is unsupported: {0}")]
    UnsupportedCommand(String),
    #[error("order not found: {0}")]
    OrderNotFound(OrderId),
    #[error("order is not open: {0}")]
    OrderNotOpen(OrderId),
    #[error("invalid order id")]
    InvalidOrderId,
    #[error("invalid execution intent id")]
    InvalidExecutionIntentId,
    #[error("rfq is disabled")]
    RfqDisabled,
    #[error("invalid RFQ id")]
    InvalidRfqId,
    #[error("invalid RFQ quote id")]
    InvalidRfqQuoteId,
    #[error("invalid RFQ state: {0}")]
    InvalidRfqState(String),
    #[error("invalid RFQ quote state: {0}")]
    InvalidRfqQuoteState(String),
    #[error("options are disabled")]
    OptionsDisabled,
    #[error("option RFQ is disabled")]
    OptionRfqDisabled,
    #[error("MM permission denied: {0}")]
    MmPermissionDenied(String),
    #[error("invalid option series id: {0}")]
    InvalidOptionSeriesId(String),
    #[error("invalid option series state: {0}")]
    InvalidOptionSeriesState(String),
    #[error("invalid option order id")]
    InvalidOptionOrderId,
    #[error("invalid option order state: {0}")]
    InvalidOptionOrderState(String),
    #[error("invalid option fill id")]
    InvalidOptionFillId,
    #[error("invalid option execution intent id")]
    InvalidOptionExecutionIntentId,
    #[error("invalid option execution intent state: {0}")]
    InvalidOptionExecutionIntentState(String),
    #[error("invalid option RFQ id")]
    InvalidOptionRfqId,
    #[error("invalid option RFQ quote id")]
    InvalidOptionRfqQuoteId,
    #[error("invalid option RFQ state: {0}")]
    InvalidOptionRfqState(String),
    #[error("invalid option RFQ quote state: {0}")]
    InvalidOptionRfqQuoteState(String),
    #[error("invalid PerpTrade intentId")]
    InvalidPerpTradeIntentId,
    #[error("invalid fixed-point value for {field}: {reason}")]
    InvalidFixedPoint { field: String, reason: String },
    #[error("deadline has expired")]
    DeadlineExpired,
    #[error("nonce must be nonzero")]
    InvalidNonce,
    #[error("nonce has already been used for account")]
    NonceAlreadyUsed,
    #[error("perp nonce sync is disabled")]
    PerpNonceSyncDisabled,
    #[error("option nonce sync is disabled")]
    OptionNonceSyncDisabled,
    #[error("perp nonce mismatch: expected on-chain nonce {expected}, got {got}")]
    PerpNonceMismatch { expected: u64, got: u64 },
    #[error("malformed signature")]
    MalformedSignature,
    #[error("trade signatures are required to build executable PerpMatchingEngine calldata")]
    MissingTradeSignatures,
    #[error("broadcast rejected: {0}")]
    BroadcastRejected(String),
    #[error("execution intent is missing PerpTrade metadata: {0}")]
    MissingExecutionMetadata(String),
    #[error("simulation failed: {0}")]
    Simulation(String),
    #[error("simulation failed: {0}")]
    SimulationReverted(Box<crate::execution::RevertDiagnostics>),
    #[error("indexer failed: {0}")]
    Indexer(String),
    #[error("malformed account address")]
    MalformedAccountAddress,
    #[error("unsupported signature v value")]
    UnsupportedSignatureV,
    #[error("signature recovery failed")]
    SignatureRecoveryFailed,
    #[error("signature signer does not match order account")]
    SignatureSignerMismatch,
    #[error("strict signature verification is not implemented in this phase")]
    StrictSignatureVerificationUnavailable,
    #[error("unknown market: {0}")]
    UnknownMarket(crate::types::MarketId),
    #[error("configuration error: {0}")]
    Config(String),
    #[error("persistence error: {0}")]
    Persistence(String),
    // ACCOUNT-WRITE-AUTH-HARDENING-V1
    #[error("{0}")]
    WriteAuth(#[from] crate::auth::WriteAuthError),
    #[error("perps not live")]
    PerpsNotLive,
    // PERPS-MINIMAL-MARKET-AND-PRICE-V1 — read-only Perps market/price
    // errors. Distinct from `PerpsNotLive` (mutation gate) so a
    // 503 SERVICE_UNAVAILABLE on a *read* endpoint doesn't spoof the
    // mutation gate and vice versa.
    #[error("perps read layer is disabled or not configured")]
    PerpsReadDisabled,
    #[error("perps chain id mismatch: expected {expected}, got {got}")]
    PerpsChainIdMismatch { expected: u64, got: u64 },
    #[error("perps market not found: {0}")]
    PerpsMarketNotFound(String),
    #[error("perps oracle price unavailable: {0}")]
    PerpsPriceUnavailable(String),
    // PERPS-ISOLATED-MARGIN-POSITION-ENGINE-V1
    #[error("perps position engine is disabled or not configured")]
    PerpsPositionsDisabled,
    #[error("perp market is paused for new position operations: {0}")]
    PerpMarketPaused(String),
    #[error("perp position size must be > 0")]
    PerpZeroSize,
    #[error("perp fill price must be > 0")]
    PerpZeroPrice,
    #[error("perp isolated margin must be > 0")]
    PerpZeroMargin,
    #[error("perp position flip is not supported in V1 — close first, then re-open")]
    PerpPositionFlip,
    #[error("perp reduce exceeds existing position size")]
    PerpReduceExceedsPosition,
    #[error("perp isolated margin is insufficient: {0}")]
    PerpInsufficientMargin(String),
    #[error("perp leverage exceeds the configured cap for {market}: {reason}")]
    PerpLeverageExceeded { market: String, reason: String },
    #[error("perp position not found for account/market")]
    PerpPositionNotFound,
    #[error("perp mark price is unavailable for risk computation: {0}")]
    PerpMarkPriceUnavailable(String),
    // PERPS-ORDER-EXECUTION-INTERNAL-V1
    #[error("perp order not found: {0}")]
    PerpOrderNotFound(String),
    #[error("perp order state is invalid: {0}")]
    PerpInvalidOrderState(String),
    #[error("perp post-only order would immediately match")]
    PerpPostOnlyWouldMatch,
    #[error("perp fill-or-kill order is not fully fillable")]
    PerpFokNotFillable,
    #[error("perp reduce-only order would increase exposure")]
    PerpReduceOnlyViolation,
    #[error("perp time-in-force is unsupported: {0}")]
    PerpUnsupportedTif(String),
    #[error("perp time-in-force combination is invalid: {0}")]
    PerpInvalidTifCombination(String),
    #[error("perp client order id already used for account: {0}")]
    PerpDuplicateClientOrderId(String),
    #[error("perp self-trade rejected")]
    PerpSelfTrade,
}
