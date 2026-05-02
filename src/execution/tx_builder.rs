use crate::error::Result;
use crate::execution::abi::encode_execute_trade_calldata;
use crate::execution::{
    ExecutionIntent, PerpTradePayload, PerpTradeSignatureBundle, StoredTradeSignatures,
};
use crate::signing::eip712::parse_evm_address;
use crate::types::AccountId;
use uuid::Uuid;

pub const EXECUTE_TRADE_FUNCTION_NAME: &str = "executeTrade";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PreparedExecutionCall {
    pub target: AccountId,
    pub function_name: &'static str,
    pub intent_id: Uuid,
    pub market_id: u128,
    pub buyer: AccountId,
    pub seller: AccountId,
    pub value: u128,
    pub calldata: Vec<u8>,
    pub is_broadcastable: bool,
    pub missing_signatures: bool,
}

pub fn build_perp_execution_call(
    target: &AccountId,
    intent_id: Uuid,
    payload: &PerpTradePayload,
    signatures: Option<&PerpTradeSignatureBundle>,
) -> Result<PreparedExecutionCall> {
    parse_evm_address(target)?;
    payload.validate()?;

    let (calldata, missing_signatures) = match signatures {
        Some(signatures) => (encode_execute_trade_calldata(payload, signatures)?, false),
        None => (Vec::new(), true),
    };

    Ok(PreparedExecutionCall {
        target: target.clone(),
        function_name: EXECUTE_TRADE_FUNCTION_NAME,
        intent_id,
        market_id: payload.market_id,
        buyer: payload.buyer.clone(),
        seller: payload.seller.clone(),
        value: 0,
        calldata,
        is_broadcastable: false,
        missing_signatures,
    })
}

pub fn preview_perp_execution_call_from_intent(
    intent: &ExecutionIntent,
    target: &AccountId,
) -> Result<PreparedExecutionCall> {
    parse_evm_address(target)?;
    parse_evm_address(&intent.buyer)?;
    parse_evm_address(&intent.seller)?;

    Ok(PreparedExecutionCall {
        target: target.clone(),
        function_name: EXECUTE_TRADE_FUNCTION_NAME,
        intent_id: intent.intent_id,
        market_id: u128::from(intent.market_id),
        buyer: intent.buyer.clone(),
        seller: intent.seller.clone(),
        value: 0,
        calldata: Vec::new(),
        is_broadcastable: false,
        missing_signatures: true,
    })
}

pub fn build_perp_execution_call_from_intent(
    intent: &ExecutionIntent,
    target: &AccountId,
    signatures: &StoredTradeSignatures,
) -> Result<PreparedExecutionCall> {
    let Some(bundle) = signatures.bundle()? else {
        return preview_perp_execution_call_from_intent(intent, target);
    };
    let payload = intent.perp_trade_payload()?;
    build_perp_execution_call(target, intent.intent_id, &payload, Some(&bundle))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::execution::abi::execute_trade_selector;
    use crate::execution::{ExecutionIntentStatus, StoredTradeSignatures};
    use crate::types::OrderId;

    #[test]
    fn missing_signatures_create_non_executable_preview() {
        let call =
            build_perp_execution_call(&target(), Uuid::from_u128(1), &payload(), None).unwrap();

        assert_eq!(call.target, target());
        assert_eq!(call.function_name, "executeTrade");
        assert_eq!(call.value, 0);
        assert!(call.calldata.is_empty());
        assert!(call.missing_signatures);
        assert!(!call.is_broadcastable);
    }

    #[test]
    fn prepared_call_target_equals_configured_matching_engine_address() {
        let call = build_perp_execution_call(
            &target(),
            Uuid::from_u128(1),
            &payload(),
            Some(&signature_bundle()),
        )
        .unwrap();

        assert_eq!(call.target, target());
        assert!(!call.calldata.is_empty());
        assert_eq!(&call.calldata[..4], execute_trade_selector().as_slice());
        assert!(!call.missing_signatures);
        assert!(!call.is_broadcastable);
    }

    #[test]
    fn intent_preview_marks_missing_trade_payload_fields_and_signatures() {
        let intent = ExecutionIntent {
            intent_id: Uuid::from_u128(1),
            market_id: 7,
            buyer: AccountId::new("0x0000000000000000000000000000000000000001"),
            seller: AccountId::new("0x0000000000000000000000000000000000000002"),
            price_1e8: 300_000_000_000,
            size_1e8: 100_000_000,
            buy_order_id: OrderId(Uuid::from_u128(2)),
            sell_order_id: OrderId(Uuid::from_u128(3)),
            buyer_is_maker: Some(false),
            buyer_nonce: Some(11),
            seller_nonce: Some(12),
            deadline_ms: Some(4_102_444_800),
            created_at_ms: 123,
            status: ExecutionIntentStatus::Pending,
        };

        let call = preview_perp_execution_call_from_intent(&intent, &target()).unwrap();

        assert_eq!(call.intent_id, intent.intent_id);
        assert_eq!(call.market_id, 7);
        assert_eq!(call.buyer, intent.buyer);
        assert_eq!(call.seller, intent.seller);
        assert!(call.missing_signatures);
        assert!(call.calldata.is_empty());
        assert!(!call.is_broadcastable);
    }

    #[test]
    fn tx_builder_builds_calldata_only_when_both_signatures_exist() {
        let intent = intent();
        let mut signatures = StoredTradeSignatures::default();
        signatures.upsert(Some(signature_hex(0xaa)), None).unwrap();
        let preview =
            build_perp_execution_call_from_intent(&intent, &target(), &signatures).unwrap();
        assert!(preview.missing_signatures);
        assert!(preview.calldata.is_empty());

        signatures.upsert(None, Some(signature_hex(0xbb))).unwrap();
        let call = build_perp_execution_call_from_intent(&intent, &target(), &signatures).unwrap();
        assert!(!call.missing_signatures);
        assert!(!call.calldata.is_empty());
        assert!(!call.is_broadcastable);
    }

    fn intent() -> ExecutionIntent {
        ExecutionIntent {
            intent_id: Uuid::from_u128(1),
            market_id: 7,
            buyer: AccountId::new("0x0000000000000000000000000000000000000001"),
            seller: AccountId::new("0x0000000000000000000000000000000000000002"),
            price_1e8: 300_000_000_000,
            size_1e8: 100_000_000,
            buy_order_id: OrderId(Uuid::from_u128(2)),
            sell_order_id: OrderId(Uuid::from_u128(3)),
            buyer_is_maker: Some(false),
            buyer_nonce: Some(11),
            seller_nonce: Some(12),
            deadline_ms: Some(4_102_444_800),
            created_at_ms: 123,
            status: ExecutionIntentStatus::Pending,
        }
    }

    fn payload() -> PerpTradePayload {
        PerpTradePayload::new(
            AccountId::new("0x0000000000000000000000000000000000000001"),
            AccountId::new("0x0000000000000000000000000000000000000002"),
            1,
            100_000_000,
            300_000_000_000,
            true,
            11,
            12,
            4_102_444_800,
        )
        .unwrap()
    }

    fn target() -> AccountId {
        AccountId::new("0x0000000000000000000000000000000000000009")
    }

    fn signature_bundle() -> PerpTradeSignatureBundle {
        PerpTradeSignatureBundle::new(&signature_hex(0xaa), &signature_hex(0xbb)).unwrap()
    }

    fn signature_hex(byte: u8) -> String {
        let mut signature = String::from("0x");
        for _ in 0..65 {
            signature.push_str(&format!("{byte:02x}"));
        }
        signature
    }
}
