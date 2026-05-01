use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;
use uuid::Uuid;

pub type Price1e8 = u128;
pub type Size1e8 = u128;
pub type MarketId = u64;
pub type TimestampMs = i64;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Ord, PartialOrd, Serialize, Deserialize)]
pub struct OrderId(pub Uuid);

impl OrderId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for OrderId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for OrderId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for OrderId {
    type Err = uuid::Error;

    fn from_str(value: &str) -> std::result::Result<Self, Self::Err> {
        Ok(Self(Uuid::parse_str(value)?))
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Hash, Ord, PartialOrd, Serialize, Deserialize)]
pub struct AccountId(pub String);

impl AccountId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    Buy,
    Sell,
}

impl Side {
    pub fn opposite(self) -> Self {
        match self {
            Self::Buy => Self::Sell,
            Self::Sell => Self::Buy,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrderType {
    Limit,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TimeInForce {
    Gtc,
    Ioc,
    Fok,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrderStatus {
    New,
    Open,
    PartiallyFilled,
    Filled,
    Cancelled,
    Rejected,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Order {
    pub order_id: OrderId,
    pub market_id: MarketId,
    pub account: AccountId,
    pub side: Side,
    pub order_type: OrderType,
    pub time_in_force: TimeInForce,
    pub price_1e8: Price1e8,
    pub size_1e8: Size1e8,
    pub remaining_size_1e8: Size1e8,
    pub reduce_only: bool,
    pub post_only: bool,
    pub client_order_id: Option<String>,
    pub created_at_ms: TimestampMs,
    pub status: OrderStatus,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct NewOrder {
    pub market_id: MarketId,
    pub account: AccountId,
    pub side: Side,
    pub price_1e8: Price1e8,
    pub size_1e8: Size1e8,
    pub time_in_force: TimeInForce,
    pub reduce_only: bool,
    pub post_only: bool,
    pub client_order_id: Option<String>,
}

impl NewOrder {
    pub fn into_order(self, order_id: OrderId, created_at_ms: TimestampMs) -> Order {
        Order {
            order_id,
            market_id: self.market_id,
            account: self.account,
            side: self.side,
            order_type: OrderType::Limit,
            time_in_force: self.time_in_force,
            price_1e8: self.price_1e8,
            size_1e8: self.size_1e8,
            remaining_size_1e8: self.size_1e8,
            reduce_only: self.reduce_only,
            post_only: self.post_only,
            client_order_id: self.client_order_id,
            created_at_ms,
            status: OrderStatus::New,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TradeMatch {
    pub market_id: MarketId,
    pub maker_order_id: OrderId,
    pub taker_order_id: OrderId,
    pub maker_account: AccountId,
    pub taker_account: AccountId,
    pub price_1e8: Price1e8,
    pub size_1e8: Size1e8,
    pub buyer: AccountId,
    pub seller: AccountId,
    pub created_at_ms: TimestampMs,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Market {
    #[serde(rename = "marketId")]
    pub market_id: MarketId,
    pub symbol: String,
    pub kind: String,
}

pub fn now_ms() -> TimestampMs {
    Utc::now().timestamp_millis()
}
