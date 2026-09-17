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
    use crate::execution::{intent_id_to_b256, ExecutionIntentStatus, StoredTradeSignatures};
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
            deadline_ms: Some(4_102_444_800_000),
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

    /// PERPS_BASE_SEPOLIA_CLOSED_TEST_LIFECYCLE_DEADLINE_UNITS_FIX_V1
    /// — the runtime calldata builder MUST encode deployed V1
    /// `PerpTrade.deadline` in Unix SECONDS (i.e. `deadline_ms / 1000`),
    /// not the raw application-layer millisecond value. Regression
    /// against the exact real-trade incident that produced
    /// `InvalidSignature (0x8baa579f)` on-chain.
    #[test]
    fn calldata_encodes_deadline_in_unix_seconds_not_milliseconds() {
        // Intent uses `deadline_ms = 4_102_444_800_000` — the shadow
        // for `deadline_sec = 4_102_444_800` (year 2100). If the
        // legacy bug returned, the encoded tuple would carry
        // `deadline = 4_102_444_800_000` and PME would reject the
        // signatures with InvalidSignature.
        let intent = intent();
        let mut signatures = StoredTradeSignatures::default();
        signatures
            .upsert(Some(signature_hex(0xaa)), Some(signature_hex(0xbb)))
            .unwrap();
        let call = build_perp_execution_call_from_intent(&intent, &target(), &signatures).unwrap();
        assert!(!call.calldata.is_empty());
        assert_eq!(&call.calldata[..4], execute_trade_selector().as_slice());

        // The tuple layout is:
        //   [4 bytes selector]
        //   [10 × 32-byte tuple fields]
        //   [2 × 32-byte offsets to buyerSig/sellerSig]
        //   [len + padded bytes for each sig]
        //
        // `deadline` is the 10th (last) tuple field. Its 32-byte word
        // starts at offset 4 + 9*32 = 292 and ends at 324.
        let deadline_word = &call.calldata[4 + 9 * 32..4 + 10 * 32];
        // Solidity encodes uint256 as big-endian 32 bytes.
        let mut buf = [0u8; 32];
        buf.copy_from_slice(deadline_word);
        let encoded_deadline_lo128 =
            u128::from_be_bytes(buf[16..].try_into().expect("32-byte slice"));
        assert_eq!(
            encoded_deadline_lo128, 4_102_444_800,
            "runtime calldata MUST encode deployed V1 deadline in Unix seconds \
             (upper 128 bits must also be zero for canonical values)"
        );
        // Upper 128 bits must be zero.
        assert_eq!(&buf[..16], &[0u8; 16]);
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
            deadline_ms: Some(4_102_444_800_000),
            created_at_ms: 123,
            status: ExecutionIntentStatus::Pending,
        }
    }

    fn payload() -> PerpTradePayload {
        PerpTradePayload::new(
            intent_id_to_b256("00000000-0000-0000-0000-000000000001").unwrap(),
            AccountId::new("0x0000000000000000000000000000000000000001"),
            AccountId::new("0x0000000000000000000000000000000000000002"),
            1,
            100_000_000,
            300_000_000_000,
            0, // max_execution_price_1e8 — strict (legacy V1 shape)
            0, // min_execution_price_1e8 — strict
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
