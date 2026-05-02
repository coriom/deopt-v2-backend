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
    #[error("command is unsupported: {0}")]
    UnsupportedCommand(String),
    #[error("order not found: {0}")]
    OrderNotFound(OrderId),
    #[error("order is not open: {0}")]
    OrderNotOpen(OrderId),
    #[error("invalid order id")]
    InvalidOrderId,
    #[error("invalid fixed-point value for {field}: {reason}")]
    InvalidFixedPoint { field: String, reason: String },
    #[error("deadline has expired")]
    DeadlineExpired,
    #[error("nonce must be nonzero")]
    InvalidNonce,
    #[error("nonce has already been used for account")]
    NonceAlreadyUsed,
    #[error("malformed signature")]
    MalformedSignature,
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
}
