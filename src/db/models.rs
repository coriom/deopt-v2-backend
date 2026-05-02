use crate::error::{BackendError, Result};
use crate::execution::{ExecutionIntent, ExecutionIntentStatus};
use crate::signing::SignedOrder;
use crate::types::{
    AccountId, Order, OrderId, OrderStatus, OrderType, Side, TimeInForce, TimestampMs, TradeMatch,
};
use std::str::FromStr;
use uuid::Uuid;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DbOrder {
    pub order_id: String,
    pub market_id: i64,
    pub account: String,
    pub side: String,
    pub order_type: String,
    pub time_in_force: String,
    pub price_1e8: String,
    pub size_1e8: String,
    pub remaining_size_1e8: String,
    pub reduce_only: bool,
    pub post_only: bool,
    pub client_order_id: String,
    pub nonce: i64,
    pub deadline_ms: i64,
    pub signature: String,
    pub status: String,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

impl DbOrder {
    pub fn from_order_and_signature(order: &Order, signed_order: &SignedOrder) -> Result<Self> {
        Ok(Self {
            order_id: order.order_id.to_string(),
            market_id: u64_to_i64("market_id", order.market_id)?,
            account: order.account.0.clone(),
            side: side_to_str(order.side).to_string(),
            order_type: order_type_to_str(order.order_type).to_string(),
            time_in_force: tif_to_str(order.time_in_force).to_string(),
            price_1e8: order.price_1e8.to_string(),
            size_1e8: order.size_1e8.to_string(),
            remaining_size_1e8: order.remaining_size_1e8.to_string(),
            reduce_only: order.reduce_only,
            post_only: order.post_only,
            client_order_id: signed_order.client_order_id.clone().unwrap_or_default(),
            nonce: u64_to_i64("nonce", signed_order.nonce)?,
            deadline_ms: signed_order.deadline_ms,
            signature: signed_order.signature.clone(),
            status: order_status_to_str(order.status).to_string(),
            created_at_ms: order.created_at_ms,
            updated_at_ms: order.created_at_ms,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DbTrade {
    pub trade_id: String,
    pub market_id: i64,
    pub maker_order_id: String,
    pub taker_order_id: String,
    pub maker_account: String,
    pub taker_account: String,
    pub price_1e8: String,
    pub size_1e8: String,
    pub buyer: String,
    pub seller: String,
    pub created_at_ms: i64,
}

impl TryFrom<&TradeMatch> for DbTrade {
    type Error = BackendError;

    fn try_from(trade: &TradeMatch) -> Result<Self> {
        Ok(Self {
            trade_id: Uuid::new_v4().to_string(),
            market_id: u64_to_i64("market_id", trade.market_id)?,
            maker_order_id: trade.maker_order_id.to_string(),
            taker_order_id: trade.taker_order_id.to_string(),
            maker_account: trade.maker_account.0.clone(),
            taker_account: trade.taker_account.0.clone(),
            price_1e8: trade.price_1e8.to_string(),
            size_1e8: trade.size_1e8.to_string(),
            buyer: trade.buyer.0.clone(),
            seller: trade.seller.0.clone(),
            created_at_ms: trade.created_at_ms,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DbExecutionIntent {
    pub intent_id: String,
    pub market_id: i64,
    pub buyer: String,
    pub seller: String,
    pub price_1e8: String,
    pub size_1e8: String,
    pub buy_order_id: String,
    pub sell_order_id: String,
    pub status: String,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

impl TryFrom<&ExecutionIntent> for DbExecutionIntent {
    type Error = BackendError;

    fn try_from(intent: &ExecutionIntent) -> Result<Self> {
        Ok(Self {
            intent_id: intent.intent_id.to_string(),
            market_id: u64_to_i64("market_id", intent.market_id)?,
            buyer: intent.buyer.0.clone(),
            seller: intent.seller.0.clone(),
            price_1e8: intent.price_1e8.to_string(),
            size_1e8: intent.size_1e8.to_string(),
            buy_order_id: intent.buy_order_id.to_string(),
            sell_order_id: intent.sell_order_id.to_string(),
            status: execution_status_to_str(intent.status).to_string(),
            created_at_ms: intent.created_at_ms,
            updated_at_ms: intent.created_at_ms,
        })
    }
}

impl TryFrom<DbExecutionIntent> for ExecutionIntent {
    type Error = BackendError;

    fn try_from(value: DbExecutionIntent) -> Result<Self> {
        Ok(Self {
            intent_id: Uuid::parse_str(&value.intent_id)
                .map_err(|error| BackendError::Persistence(error.to_string()))?,
            market_id: i64_to_u64("market_id", value.market_id)?,
            buyer: AccountId::new(value.buyer),
            seller: AccountId::new(value.seller),
            price_1e8: value.price_1e8.parse().map_err(|error| {
                BackendError::Persistence(format!("invalid price_1e8: {error}"))
            })?,
            size_1e8: value
                .size_1e8
                .parse()
                .map_err(|error| BackendError::Persistence(format!("invalid size_1e8: {error}")))?,
            buy_order_id: OrderId::from_str(&value.buy_order_id)
                .map_err(|error| BackendError::Persistence(error.to_string()))?,
            sell_order_id: OrderId::from_str(&value.sell_order_id)
                .map_err(|error| BackendError::Persistence(error.to_string()))?,
            created_at_ms: value.created_at_ms,
            status: execution_status_from_str(&value.status)?,
        })
    }
}

pub fn side_to_str(side: Side) -> &'static str {
    match side {
        Side::Buy => "buy",
        Side::Sell => "sell",
    }
}

pub fn tif_to_str(time_in_force: TimeInForce) -> &'static str {
    match time_in_force {
        TimeInForce::Gtc => "gtc",
        TimeInForce::Ioc => "ioc",
        TimeInForce::Fok => "fok",
    }
}

pub fn order_type_to_str(order_type: OrderType) -> &'static str {
    match order_type {
        OrderType::Limit => "limit",
    }
}

pub fn order_status_to_str(status: OrderStatus) -> &'static str {
    match status {
        OrderStatus::New => "new",
        OrderStatus::Open => "open",
        OrderStatus::PartiallyFilled => "partially_filled",
        OrderStatus::Filled => "filled",
        OrderStatus::Cancelled => "cancelled",
        OrderStatus::Rejected => "rejected",
    }
}

pub fn execution_status_to_str(status: ExecutionIntentStatus) -> &'static str {
    match status {
        ExecutionIntentStatus::Pending => "pending",
        ExecutionIntentStatus::DryRun => "dry_run",
        ExecutionIntentStatus::Submitted => "submitted",
        ExecutionIntentStatus::Confirmed => "confirmed",
        ExecutionIntentStatus::Failed => "failed",
    }
}

fn execution_status_from_str(value: &str) -> Result<ExecutionIntentStatus> {
    match value {
        "pending" => Ok(ExecutionIntentStatus::Pending),
        "dry_run" => Ok(ExecutionIntentStatus::DryRun),
        "submitted" => Ok(ExecutionIntentStatus::Submitted),
        "confirmed" => Ok(ExecutionIntentStatus::Confirmed),
        "failed" => Ok(ExecutionIntentStatus::Failed),
        other => Err(BackendError::Persistence(format!(
            "invalid execution intent status: {other}"
        ))),
    }
}

pub fn u64_to_i64(field: &str, value: u64) -> Result<i64> {
    i64::try_from(value)
        .map_err(|_| BackendError::Persistence(format!("{field} exceeds BIGINT range")))
}

fn i64_to_u64(field: &str, value: i64) -> Result<u64> {
    u64::try_from(value)
        .map_err(|_| BackendError::Persistence(format!("{field} cannot be negative")))
}

pub fn timestamp_to_i64(value: TimestampMs) -> i64 {
    value
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{MarketId, Price1e8, Size1e8};

    fn order() -> Order {
        Order {
            order_id: OrderId::new(),
            market_id: 1 as MarketId,
            account: AccountId::new("0x0000000000000000000000000000000000000001"),
            side: Side::Sell,
            order_type: OrderType::Limit,
            time_in_force: TimeInForce::Gtc,
            price_1e8: 300_000_000_000 as Price1e8,
            size_1e8: 100_000_000 as Size1e8,
            remaining_size_1e8: 50_000_000,
            reduce_only: false,
            post_only: true,
            client_order_id: Some("client-1".to_string()),
            created_at_ms: 123,
            status: OrderStatus::PartiallyFilled,
        }
    }

    fn signed_order() -> SignedOrder {
        SignedOrder {
            account: AccountId::new("0x0000000000000000000000000000000000000001"),
            market_id: 1,
            side: Side::Sell,
            price_1e8: 300_000_000_000,
            size_1e8: 100_000_000,
            time_in_force: TimeInForce::Gtc,
            reduce_only: false,
            post_only: true,
            client_order_id: Some("client-1".to_string()),
            nonce: 7,
            deadline_ms: 456,
            signature: "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
        }
    }

    #[test]
    fn db_order_preserves_signed_order_fields() {
        let db_order = DbOrder::from_order_and_signature(&order(), &signed_order()).unwrap();

        assert_eq!(db_order.side, "sell");
        assert_eq!(db_order.time_in_force, "gtc");
        assert_eq!(db_order.status, "partially_filled");
        assert_eq!(db_order.price_1e8, "300000000000");
        assert_eq!(db_order.remaining_size_1e8, "50000000");
        assert_eq!(db_order.client_order_id, "client-1");
        assert_eq!(db_order.nonce, 7);
        assert_eq!(db_order.deadline_ms, 456);
    }
}
