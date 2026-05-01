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
    #[error("configuration error: {0}")]
    Config(String),
}
