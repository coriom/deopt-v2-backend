use crate::execution::ExecutionIntent;
use crate::types::{Order, OrderId, TradeMatch};
use serde::Serialize;

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EngineEvent {
    OrderAccepted { order: Order },
    OrderRejected { order_id: OrderId, reason: String },
    OrderCancelled { order: Order },
    OrderPartiallyFilled { order: Order },
    OrderFilled { order: Order },
    TradeMatched { trade: TradeMatch },
    ExecutionIntentCreated { intent: ExecutionIntent },
}
