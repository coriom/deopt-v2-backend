use crate::error::Result;
use crate::execution::ExecutionIntent;
use crate::types::{AccountId, MarketId, Price1e8, Size1e8};
use uuid::Uuid;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PreparedExecutionCall {
    pub target_contract: AccountId,
    pub intent_id: Uuid,
    pub market_id: MarketId,
    pub buyer: AccountId,
    pub seller: AccountId,
    pub price_1e8: Price1e8,
    pub size_1e8: Size1e8,
    pub calldata: Vec<u8>,
    pub is_placeholder: bool,
}

/// Builds the execution-call boundary for a future PerpMatchingEngine call.
///
/// This scaffold intentionally does not ABI encode calldata. Contract ABI
/// integration will replace the empty placeholder bytes in a later task.
pub fn build_perp_execution_call(
    intent: &ExecutionIntent,
    target_contract: &AccountId,
) -> Result<PreparedExecutionCall> {
    Ok(PreparedExecutionCall {
        target_contract: target_contract.clone(),
        intent_id: intent.intent_id,
        market_id: intent.market_id,
        buyer: intent.buyer.clone(),
        seller: intent.seller.clone(),
        price_1e8: intent.price_1e8,
        size_1e8: intent.size_1e8,
        calldata: Vec::new(),
        is_placeholder: true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::execution::ExecutionIntentStatus;
    use crate::types::OrderId;

    #[test]
    fn tx_builder_produces_placeholder_call() {
        let intent = ExecutionIntent {
            intent_id: Uuid::from_u128(1),
            market_id: 7,
            buyer: AccountId::new("0x0000000000000000000000000000000000000001"),
            seller: AccountId::new("0x0000000000000000000000000000000000000002"),
            price_1e8: 300_000_000_000,
            size_1e8: 100_000_000,
            buy_order_id: OrderId(Uuid::from_u128(2)),
            sell_order_id: OrderId(Uuid::from_u128(3)),
            created_at_ms: 123,
            status: ExecutionIntentStatus::Pending,
        };
        let target = AccountId::new("0x0000000000000000000000000000000000000009");

        let call = build_perp_execution_call(&intent, &target).unwrap();

        assert_eq!(call.target_contract, target);
        assert_eq!(call.intent_id, intent.intent_id);
        assert_eq!(call.market_id, 7);
        assert_eq!(call.buyer, intent.buyer);
        assert_eq!(call.seller, intent.seller);
        assert_eq!(call.price_1e8, "300000000000".parse::<u128>().unwrap());
        assert_eq!(call.size_1e8, "100000000".parse::<u128>().unwrap());
        assert!(call.calldata.is_empty());
        assert!(call.is_placeholder);
    }
}
