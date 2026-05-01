use crate::types::{AccountId, MarketId, Price1e8, Side, Size1e8, TimestampMs};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MarketMakerSession {
    pub session_id: Uuid,
    pub account: AccountId,
    pub connected_at_ms: TimestampMs,
    pub last_heartbeat_ms: TimestampMs,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Heartbeat {
    pub session_id: Uuid,
    pub received_at_ms: TimestampMs,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BulkQuoteUpdate {
    pub session_id: Uuid,
    pub market_id: MarketId,
    pub quotes: Vec<QuoteUpdate>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct QuoteUpdate {
    pub side: Side,
    pub price_1e8: Price1e8,
    pub size_1e8: Size1e8,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BulkCancel {
    pub session_id: Uuid,
    pub market_id: Option<MarketId>,
}
