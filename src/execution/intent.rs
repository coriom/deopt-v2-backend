use crate::types::{AccountId, MarketId, OrderId, Price1e8, Size1e8, TimestampMs};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionIntentStatus {
    Pending,
    Submitted,
    Confirmed,
    Failed,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ExecutionIntent {
    pub intent_id: Uuid,
    pub market_id: MarketId,
    pub buyer: AccountId,
    pub seller: AccountId,
    pub price_1e8: Price1e8,
    pub size_1e8: Size1e8,
    pub buy_order_id: OrderId,
    pub sell_order_id: OrderId,
    pub created_at_ms: TimestampMs,
    pub status: ExecutionIntentStatus,
}
