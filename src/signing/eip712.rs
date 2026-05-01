use crate::types::{AccountId, MarketId, Side, TimeInForce, TimestampMs};

pub const EIP712_ORDER_TYPE: &str = "DeOptOrder(address account,uint64 marketId,string side,uint128 price1e8,uint128 size1e8,string timeInForce,bool reduceOnly,bool postOnly,string clientOrderId,uint64 nonce,int64 deadlineMs)";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SignedOrder {
    pub account: AccountId,
    pub market_id: MarketId,
    pub side: Side,
    pub price_1e8: u128,
    pub size_1e8: u128,
    pub time_in_force: TimeInForce,
    pub reduce_only: bool,
    pub post_only: bool,
    pub client_order_id: Option<String>,
    pub nonce: u64,
    pub deadline_ms: TimestampMs,
    pub signature: String,
}

impl SignedOrder {
    pub fn canonical_payload(&self) -> String {
        format!(
            "{}|account={}|market_id={}|side={:?}|price_1e8={}|size_1e8={}|time_in_force={:?}|reduce_only={}|post_only={}|client_order_id={}|nonce={}|deadline_ms={}",
            EIP712_ORDER_TYPE,
            self.account.0,
            self.market_id,
            self.side,
            self.price_1e8,
            self.size_1e8,
            self.time_in_force,
            self.reduce_only,
            self.post_only,
            self.client_order_id.as_deref().unwrap_or_default(),
            self.nonce,
            self.deadline_ms,
        )
    }
}
