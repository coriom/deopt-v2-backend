use crate::engine::EngineEvent;
use crate::error::{BackendError, Result};
use crate::execution::ExecutionIntent;
use crate::signing::SignedOrder;
use crate::types::{
    AccountId, MarketId, NewOrder, Order, OrderId, OrderStatus, OrderType, Side, TimeInForce,
    TimestampMs, TradeMatch,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
pub struct SubmitOrderRequest {
    pub market_id: MarketId,
    pub account: AccountId,
    pub side: Side,
    pub price_1e8: String,
    pub size_1e8: String,
    pub time_in_force: TimeInForce,
    pub reduce_only: bool,
    pub post_only: bool,
    pub client_order_id: Option<String>,
    pub nonce: u64,
    pub deadline_ms: TimestampMs,
    pub signature: String,
}

impl SubmitOrderRequest {
    pub fn into_signed_order(self) -> Result<SignedOrder> {
        Ok(SignedOrder {
            account: self.account,
            market_id: self.market_id,
            side: self.side,
            price_1e8: parse_fixed_u128("price_1e8", &self.price_1e8)?,
            size_1e8: parse_fixed_u128("size_1e8", &self.size_1e8)?,
            time_in_force: self.time_in_force,
            reduce_only: self.reduce_only,
            post_only: self.post_only,
            client_order_id: self.client_order_id,
            nonce: self.nonce,
            deadline_ms: self.deadline_ms,
            signature: self.signature,
        })
    }
}

impl From<SignedOrder> for NewOrder {
    fn from(order: SignedOrder) -> Self {
        Self {
            market_id: order.market_id,
            account: order.account,
            side: order.side,
            price_1e8: order.price_1e8,
            size_1e8: order.size_1e8,
            time_in_force: order.time_in_force,
            reduce_only: order.reduce_only,
            post_only: order.post_only,
            client_order_id: order.client_order_id,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SubmitOrderResponse {
    pub status: String,
    pub order_id: Option<OrderId>,
    pub events: Vec<ApiEngineEvent>,
    pub execution_intents: Vec<ApiExecutionIntent>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ApiOrder {
    pub order_id: OrderId,
    pub market_id: MarketId,
    pub account: AccountId,
    pub side: Side,
    pub order_type: OrderType,
    pub time_in_force: TimeInForce,
    pub price_1e8: String,
    pub size_1e8: String,
    pub remaining_size_1e8: String,
    pub reduce_only: bool,
    pub post_only: bool,
    pub client_order_id: Option<String>,
    pub created_at_ms: TimestampMs,
    pub status: OrderStatus,
}

impl From<Order> for ApiOrder {
    fn from(order: Order) -> Self {
        Self {
            order_id: order.order_id,
            market_id: order.market_id,
            account: order.account,
            side: order.side,
            order_type: order.order_type,
            time_in_force: order.time_in_force,
            price_1e8: order.price_1e8.to_string(),
            size_1e8: order.size_1e8.to_string(),
            remaining_size_1e8: order.remaining_size_1e8.to_string(),
            reduce_only: order.reduce_only,
            post_only: order.post_only,
            client_order_id: order.client_order_id,
            created_at_ms: order.created_at_ms,
            status: order.status,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ApiTradeMatch {
    pub market_id: MarketId,
    pub maker_order_id: OrderId,
    pub taker_order_id: OrderId,
    pub maker_account: AccountId,
    pub taker_account: AccountId,
    pub price_1e8: String,
    pub size_1e8: String,
    pub buyer: AccountId,
    pub seller: AccountId,
    pub created_at_ms: TimestampMs,
}

impl From<TradeMatch> for ApiTradeMatch {
    fn from(trade: TradeMatch) -> Self {
        Self {
            market_id: trade.market_id,
            maker_order_id: trade.maker_order_id,
            taker_order_id: trade.taker_order_id,
            maker_account: trade.maker_account,
            taker_account: trade.taker_account,
            price_1e8: trade.price_1e8.to_string(),
            size_1e8: trade.size_1e8.to_string(),
            buyer: trade.buyer,
            seller: trade.seller,
            created_at_ms: trade.created_at_ms,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ApiExecutionIntent {
    pub intent_id: Uuid,
    pub market_id: MarketId,
    pub buyer: AccountId,
    pub seller: AccountId,
    pub price_1e8: String,
    pub size_1e8: String,
    pub buy_order_id: OrderId,
    pub sell_order_id: OrderId,
    pub created_at_ms: TimestampMs,
    pub status: crate::execution::ExecutionIntentStatus,
}

impl From<ExecutionIntent> for ApiExecutionIntent {
    fn from(intent: ExecutionIntent) -> Self {
        Self {
            intent_id: intent.intent_id,
            market_id: intent.market_id,
            buyer: intent.buyer,
            seller: intent.seller,
            price_1e8: intent.price_1e8.to_string(),
            size_1e8: intent.size_1e8.to_string(),
            buy_order_id: intent.buy_order_id,
            sell_order_id: intent.sell_order_id,
            created_at_ms: intent.created_at_ms,
            status: intent.status,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ApiEngineEvent {
    OrderAccepted { order: ApiOrder },
    OrderRejected { order_id: OrderId, reason: String },
    OrderCancelled { order: ApiOrder },
    OrderPartiallyFilled { order: ApiOrder },
    OrderFilled { order: ApiOrder },
    TradeMatched { trade: ApiTradeMatch },
    ExecutionIntentCreated { intent: ApiExecutionIntent },
}

impl From<EngineEvent> for ApiEngineEvent {
    fn from(event: EngineEvent) -> Self {
        match event {
            EngineEvent::OrderAccepted { order } => Self::OrderAccepted {
                order: order.into(),
            },
            EngineEvent::OrderRejected { order_id, reason } => {
                Self::OrderRejected { order_id, reason }
            }
            EngineEvent::OrderCancelled { order } => Self::OrderCancelled {
                order: order.into(),
            },
            EngineEvent::OrderPartiallyFilled { order } => Self::OrderPartiallyFilled {
                order: order.into(),
            },
            EngineEvent::OrderFilled { order } => Self::OrderFilled {
                order: order.into(),
            },
            EngineEvent::TradeMatched { trade } => Self::TradeMatched {
                trade: trade.into(),
            },
            EngineEvent::ExecutionIntentCreated { intent } => Self::ExecutionIntentCreated {
                intent: intent.into(),
            },
        }
    }
}

pub fn parse_fixed_u128(field: &str, value: &str) -> Result<u128> {
    if value.is_empty() {
        return Err(BackendError::InvalidFixedPoint {
            field: field.to_string(),
            reason: "empty string".to_string(),
        });
    }
    if value.starts_with('-') {
        return Err(BackendError::InvalidFixedPoint {
            field: field.to_string(),
            reason: "negative values are not allowed".to_string(),
        });
    }
    if !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(BackendError::InvalidFixedPoint {
            field: field.to_string(),
            reason: "expected an unsigned integer string".to_string(),
        });
    }

    value
        .parse::<u128>()
        .map_err(|error| BackendError::InvalidFixedPoint {
            field: field.to_string(),
            reason: error.to_string(),
        })
}
