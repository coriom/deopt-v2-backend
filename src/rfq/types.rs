use crate::types::{AccountId, MarketId, Price1e8, Side, Size1e8, TimestampMs};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub type RfqId = Uuid;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RfqStatus {
    Open,
    Quoted,
    Accepted,
    Expired,
    Executed,
    Failed,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RfqRequest {
    pub rfq_id: RfqId,
    pub market_id: MarketId,
    pub requester: AccountId,
    pub side: Side,
    pub size_1e8: Size1e8,
    pub status: RfqStatus,
    pub created_at_ms: TimestampMs,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RfqQuote {
    pub rfq_id: RfqId,
    pub market_maker: AccountId,
    pub price_1e8: Price1e8,
    pub size_1e8: Size1e8,
    pub created_at_ms: TimestampMs,
}
