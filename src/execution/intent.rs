use crate::error::{BackendError, Result};
use crate::execution::{intent_id_to_b256, PerpTradePayload};
use crate::types::{AccountId, MarketId, OrderId, Price1e8, Size1e8, TimestampMs};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionIntentStatus {
    Pending,
    DryRun,
    CalldataReady,
    SimulationOk,
    SimulationFailed,
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
    pub buyer_is_maker: Option<bool>,
    pub buyer_nonce: Option<u64>,
    pub seller_nonce: Option<u64>,
    pub deadline_ms: Option<TimestampMs>,
    pub created_at_ms: TimestampMs,
    pub status: ExecutionIntentStatus,
}

impl ExecutionIntent {
    pub fn perp_trade_payload(&self) -> Result<PerpTradePayload> {
        let buyer_is_maker = self
            .buyer_is_maker
            .ok_or_else(|| BackendError::MissingExecutionMetadata("buyer_is_maker".to_string()))?;
        let buyer_nonce = self
            .buyer_nonce
            .ok_or_else(|| BackendError::MissingExecutionMetadata("buyer_nonce".to_string()))?;
        let seller_nonce = self
            .seller_nonce
            .ok_or_else(|| BackendError::MissingExecutionMetadata("seller_nonce".to_string()))?;
        let deadline_ms = self
            .deadline_ms
            .ok_or_else(|| BackendError::MissingExecutionMetadata("deadline".to_string()))?;
        let deadline = u128::try_from(deadline_ms)
            .map_err(|_| BackendError::MissingExecutionMetadata("deadline".to_string()))?;

        PerpTradePayload::new(
            intent_id_to_b256(&self.intent_id.to_string())?,
            self.buyer.clone(),
            self.seller.clone(),
            u128::from(self.market_id),
            self.size_1e8,
            self.price_1e8,
            buyer_is_maker,
            u128::from(buyer_nonce),
            u128::from(seller_nonce),
            deadline,
        )
    }
}
