use crate::types::{NewOrder, OrderId};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EngineCommand {
    SubmitOrder(NewOrder),
    CancelOrder { order_id: OrderId },
    ReplaceOrder { order_id: OrderId, replacement: NewOrder },
}
