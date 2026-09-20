use crate::error::{BackendError, Result};
use crate::execution::perp_trade::PerpsProtocolVersion;
use crate::execution::{PerpTradePayload, PerpTradeSignatureBundle};
use crate::signing::eip712::{keccak256, parse_evm_address};
use alloy_primitives::{Address, Bytes, U256};
use alloy_sol_types::{sol, SolCall, SolValue};

sol! {
    struct PerpTrade {
        bytes32 intentId;
        address buyer;
        address seller;
        uint256 marketId;
        uint128 sizeDelta1e8;
        uint128 executionPrice1e8;
        bool buyerIsMaker;
        uint256 buyerNonce;
        uint256 sellerNonce;
        uint256 deadline;
    }

    function executeTrade(PerpTrade t, bytes buyerSig, bytes sellerSig);
}

// PERPS_V2_BACKEND_EXECUTOR_PATH_V1 — V2 12-field PerpTrade tuple,
// distinct from V1 by insertion of `maxExecutionPrice1e8` +
// `minExecutionPrice1e8` between `executionPrice1e8` and
// `buyerIsMaker`. Field order MUST match
// `PerpMatchingEngineV2.PerpTrade` at sol HEAD `2e9ad6f` byte-for-byte.
// Any divergence produces a selector mismatch on the deployed V2 PME
// and the executor tx reverts with an unrecognised-function selector.
//
// The nested `mod v2` scoping is intentional: the Solidity function
// name on both V1 and V2 is `executeTrade`, so the alloy `sol!` macro
// generates the same Rust identifier in both. Wrapping the V2 codec
// in a submodule keeps the Rust identifiers namespaced
// (`v2::executeTradeCall` vs the top-level `executeTradeCall`) while
// preserving the wire-truthful `executeTrade(...)` name that drives
// the 4-byte selector.
pub mod v2 {
    use alloy_sol_types::sol;

    sol! {
        struct PerpTrade {
            bytes32 intentId;
            address buyer;
            address seller;
            uint256 marketId;
            uint128 sizeDelta1e8;
            uint128 executionPrice1e8;
            uint128 maxExecutionPrice1e8;
            uint128 minExecutionPrice1e8;
            bool buyerIsMaker;
            uint256 buyerNonce;
            uint256 sellerNonce;
            uint256 deadline;
        }

        function executeTrade(PerpTrade t, bytes buyerSig, bytes sellerSig);
    }
}

// Re-export V2 types at the module root so external references can
// use `abi::PerpTradeV2` without exposing the submodule structure.
pub use v2::PerpTrade as PerpTradeV2;
pub use v2::executeTradeCall as executeTradeV2Call;

pub const EXECUTE_TRADE_SIGNATURE: &str =
    "executeTrade((bytes32,address,address,uint256,uint128,uint128,bool,uint256,uint256,uint256),bytes,bytes)";

/// PERPS_V2_BACKEND_EXECUTOR_PATH_V1 — canonical V2 `executeTrade`
/// signature. Note: the Solidity function is also named
/// `executeTrade`, but the tuple shape differs, which produces a
/// different 4-byte selector. This name is a backend-only alias to
/// keep V1 vs V2 selectors grep-able. The wire string uses
/// `executeTrade(...)` (matching Solidity) — the `V2` suffix in
/// `executeTradeV2Call` above is a Rust type-namespace convenience
/// so both ABI codecs can coexist in one module.
pub const EXECUTE_TRADE_V2_SIGNATURE: &str =
    "executeTrade((bytes32,address,address,uint256,uint128,uint128,uint128,uint128,bool,uint256,uint256,uint256),bytes,bytes)";

pub fn execute_trade_selector() -> [u8; 4] {
    executeTradeCall::SELECTOR
}

/// PERPS_V2_BACKEND_EXECUTOR_PATH_V1 — V2 `executeTrade` 4-byte
/// selector. Distinct from V1's `execute_trade_selector()` because
/// the tuple shape differs. Wire form of the selector is
/// `keccak256(EXECUTE_TRADE_V2_SIGNATURE)[..4]` computed at compile
/// time by the `sol!` macro.
pub fn execute_trade_v2_selector() -> [u8; 4] {
    executeTradeV2Call::SELECTOR
}

pub fn expected_execute_trade_v2_selector() -> [u8; 4] {
    let hash = keccak256(EXECUTE_TRADE_V2_SIGNATURE.as_bytes());
    [hash[0], hash[1], hash[2], hash[3]]
}

/// PERPS_V2_BACKEND_EXECUTOR_PATH_V1 — version-dispatched selector.
/// Used by the runtime tx builder so a persisted intent's
/// `protocol_version` deterministically selects the correct wire
/// selector.
pub fn execute_trade_selector_for(version: PerpsProtocolVersion) -> [u8; 4] {
    match version {
        PerpsProtocolVersion::V1 => execute_trade_selector(),
        PerpsProtocolVersion::V2 => execute_trade_v2_selector(),
    }
}

pub fn encode_execute_trade_calldata(
    payload: &PerpTradePayload,
    signatures: &PerpTradeSignatureBundle,
) -> Result<Vec<u8>> {
    payload.validate()?;
    let call = executeTradeCall {
        t: PerpTrade {
            intentId: payload.intent_id,
            buyer: Address::from(parse_evm_address(&payload.buyer)?),
            seller: Address::from(parse_evm_address(&payload.seller)?),
            marketId: U256::from(payload.market_id),
            sizeDelta1e8: payload.size_delta_1e8,
            executionPrice1e8: payload.execution_price_1e8,
            buyerIsMaker: payload.buyer_is_maker,
            buyerNonce: U256::from(payload.buyer_nonce),
            sellerNonce: U256::from(payload.seller_nonce),
            deadline: U256::from(payload.deadline),
        },
        buyerSig: Bytes::from(signatures.buyer_sig.clone()),
        sellerSig: Bytes::from(signatures.seller_sig.clone()),
    };
    Ok(call.abi_encode())
}

/// PERPS_V2_BACKEND_EXECUTOR_PATH_V1 — V2 executeTrade calldata
/// encoding. The 12-field tuple carries the `maxExecutionPrice1e8` /
/// `minExecutionPrice1e8` bounds that ride ALL the way to
/// `PerpMatchingEngineV2._executeSingle` where they gate the fill.
///
/// Zeroed bounds reproduce V1 strict-price semantics — matching the
/// backend's current `PerpTradePayload::max/min_execution_price_1e8`
/// fields whose comment says "0 == strict (legacy V1 behaviour)".
pub fn encode_execute_trade_v2_calldata(
    payload: &PerpTradePayload,
    signatures: &PerpTradeSignatureBundle,
) -> Result<Vec<u8>> {
    payload.validate()?;
    let call = executeTradeV2Call {
        t: PerpTradeV2 {
            intentId: payload.intent_id,
            buyer: Address::from(parse_evm_address(&payload.buyer)?),
            seller: Address::from(parse_evm_address(&payload.seller)?),
            marketId: U256::from(payload.market_id),
            sizeDelta1e8: payload.size_delta_1e8,
            executionPrice1e8: payload.execution_price_1e8,
            maxExecutionPrice1e8: payload.max_execution_price_1e8,
            minExecutionPrice1e8: payload.min_execution_price_1e8,
            buyerIsMaker: payload.buyer_is_maker,
            buyerNonce: U256::from(payload.buyer_nonce),
            sellerNonce: U256::from(payload.seller_nonce),
            deadline: U256::from(payload.deadline),
        },
        buyerSig: Bytes::from(signatures.buyer_sig.clone()),
        sellerSig: Bytes::from(signatures.seller_sig.clone()),
    };
    Ok(call.abi_encode())
}

/// PERPS_V2_BACKEND_EXECUTOR_PATH_V1 — version-dispatched calldata
/// builder. Persisted intent's `protocol_version` deterministically
/// selects the shape.
pub fn encode_execute_trade_calldata_for_version(
    payload: &PerpTradePayload,
    signatures: &PerpTradeSignatureBundle,
    version: PerpsProtocolVersion,
) -> Result<Vec<u8>> {
    match version {
        PerpsProtocolVersion::V1 => encode_execute_trade_calldata(payload, signatures),
        PerpsProtocolVersion::V2 => encode_execute_trade_v2_calldata(payload, signatures),
    }
}

/// PERPS_V2_BACKEND_EXECUTOR_PATH_V1 — independent decode of V2
/// calldata for §14 round-trip proof. Given a produced calldata
/// blob, verify selector + decoded tuple fields == source payload.
/// This is the "no manually rebuilt alternate path" guarantee.
pub fn decode_execute_trade_v2_calldata(
    calldata: &[u8],
) -> Result<(PerpTradeV2, Vec<u8>, Vec<u8>)> {
    if calldata.len() < 4 {
        return Err(BackendError::Config(
            "V2 calldata too short (< 4 bytes for selector)".to_string(),
        ));
    }
    let selector = &calldata[..4];
    if selector != execute_trade_v2_selector() {
        return Err(BackendError::Config(format!(
            "V2 calldata selector mismatch: got 0x{}, expected 0x{}",
            hex_bytes(selector),
            hex_bytes(&execute_trade_v2_selector())
        )));
    }
    let args = <(PerpTradeV2, Bytes, Bytes)>::abi_decode_params(&calldata[4..], true)
        .map_err(|error| {
            BackendError::Config(format!("V2 calldata decode failed: {error}"))
        })?;
    Ok((args.0, args.1.to_vec(), args.2.to_vec()))
}

fn hex_bytes(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

pub fn expected_execute_trade_selector() -> [u8; 4] {
    let hash = keccak256(EXECUTE_TRADE_SIGNATURE.as_bytes());
    [hash[0], hash[1], hash[2], hash[3]]
}

/// PERPS_V2_BACKEND_EXECUTOR_PATH_V1 — cheap runtime keccak of the
/// V2 executeTrade signature. Should equal
/// [`execute_trade_v2_selector()`] byte-for-byte; enforced by
/// `calldata_v2_selector_matches_execute_trade_v2_signature`
/// regression test.
pub fn expected_execute_trade_v2_selector_runtime() -> [u8; 4] {
    let hash = keccak256(EXECUTE_TRADE_V2_SIGNATURE.as_bytes());
    [hash[0], hash[1], hash[2], hash[3]]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::execution::{intent_id_to_b256, PerpTradePayload, PerpTradeSignatureBundle};
    use crate::types::AccountId;

    #[test]
    fn calldata_selector_matches_execute_trade_signature() {
        assert_eq!(execute_trade_selector(), expected_execute_trade_selector());
    }

    #[test]
    fn calldata_builder_creates_non_empty_calldata_with_signatures() {
        let calldata = encode_execute_trade_calldata(&payload(), &signature_bundle()).unwrap();

        assert!(!calldata.is_empty());
        assert_eq!(&calldata[..4], execute_trade_selector().as_slice());
    }

    // ─── PERPS_V2_BACKEND_EXECUTOR_PATH_V1 (§14) V2 ABI tests ─

    #[test]
    fn v2_calldata_selector_matches_execute_trade_v2_signature() {
        assert_eq!(
            execute_trade_v2_selector(),
            expected_execute_trade_v2_selector()
        );
        assert_eq!(
            execute_trade_v2_selector(),
            expected_execute_trade_v2_selector_runtime()
        );
    }

    #[test]
    fn v1_and_v2_selectors_are_distinct() {
        assert_ne!(
            execute_trade_selector(),
            execute_trade_v2_selector(),
            "V1 and V2 executeTrade selectors MUST differ; a match would let a V1 executor \
             tx accidentally hit a V2 PME or vice versa"
        );
    }

    #[test]
    fn v2_calldata_starts_with_v2_selector() {
        let calldata = encode_execute_trade_v2_calldata(&payload(), &signature_bundle()).unwrap();
        assert!(!calldata.is_empty());
        assert_eq!(&calldata[..4], execute_trade_v2_selector().as_slice());
    }

    #[test]
    fn v2_calldata_round_trip_preserves_every_field() {
        // Independent decode proves the encoder / decoder is
        // self-consistent AND that no signed V2 field is silently
        // dropped. Not tautological — the decode goes through
        // alloy's ABI codec independently of the sol! macro's
        // encoding trait, so a mismatch surfaces here.
        let src = payload();
        let sigs = signature_bundle();
        let calldata = encode_execute_trade_v2_calldata(&src, &sigs).unwrap();
        let (decoded, buyer_sig, seller_sig) =
            decode_execute_trade_v2_calldata(&calldata).unwrap();
        assert_eq!(decoded.intentId, src.intent_id);
        assert_eq!(
            decoded.buyer,
            Address::from(parse_evm_address(&src.buyer).unwrap())
        );
        assert_eq!(
            decoded.seller,
            Address::from(parse_evm_address(&src.seller).unwrap())
        );
        assert_eq!(decoded.marketId, U256::from(src.market_id));
        assert_eq!(decoded.sizeDelta1e8, src.size_delta_1e8);
        assert_eq!(decoded.executionPrice1e8, src.execution_price_1e8);
        assert_eq!(decoded.maxExecutionPrice1e8, src.max_execution_price_1e8);
        assert_eq!(decoded.minExecutionPrice1e8, src.min_execution_price_1e8);
        assert_eq!(decoded.buyerIsMaker, src.buyer_is_maker);
        assert_eq!(decoded.buyerNonce, U256::from(src.buyer_nonce));
        assert_eq!(decoded.sellerNonce, U256::from(src.seller_nonce));
        assert_eq!(decoded.deadline, U256::from(src.deadline));
        assert_eq!(buyer_sig.as_slice(), sigs.buyer_sig.as_slice());
        assert_eq!(seller_sig.as_slice(), sigs.seller_sig.as_slice());
    }

    #[test]
    fn v2_calldata_carries_price_bounds_that_v1_would_drop() {
        // Same payload, but V1 encoder ignores the price bounds
        // (10-field tuple has no slots for them). This test proves
        // the V2 encoder actually threads them through.
        let mut src = payload();
        // Tweak the payload's bounds — build it fresh via the
        // constructor so `validate()` accepts them.
        src = PerpTradePayload::new(
            src.intent_id,
            src.buyer.clone(),
            src.seller.clone(),
            src.market_id,
            src.size_delta_1e8,
            src.execution_price_1e8,
            src.execution_price_1e8 + 1_000_000, // max — inclusive above
            src.execution_price_1e8 - 1_000_000, // min — inclusive below
            src.buyer_is_maker,
            src.buyer_nonce,
            src.seller_nonce,
            src.deadline,
        )
        .unwrap();
        let sigs = signature_bundle();

        let v1_calldata = encode_execute_trade_calldata(&src, &sigs).unwrap();
        let v2_calldata = encode_execute_trade_v2_calldata(&src, &sigs).unwrap();

        assert_ne!(v1_calldata, v2_calldata);
        assert_ne!(&v1_calldata[..4], &v2_calldata[..4]);

        // V2 decode must recover the exact bounds the source
        // payload set — bytes are byte-identical after the
        // round-trip.
        let (decoded, _, _) = decode_execute_trade_v2_calldata(&v2_calldata).unwrap();
        assert_eq!(decoded.maxExecutionPrice1e8, src.max_execution_price_1e8);
        assert_eq!(decoded.minExecutionPrice1e8, src.min_execution_price_1e8);
    }

    #[test]
    fn v2_calldata_decode_rejects_short_input() {
        // Cannot use unwrap_err() because PerpTradeV2 (via sol!)
        // does not derive Debug. Assert on the Err arm directly.
        match decode_execute_trade_v2_calldata(&[0x00, 0x11, 0x22]) {
            Err(error) => assert!(format!("{error}").contains("too short")),
            Ok(_) => panic!("expected decode to reject too-short input"),
        }
    }

    #[test]
    fn v2_calldata_decode_rejects_v1_selector() {
        let mut calldata = encode_execute_trade_calldata(&payload(), &signature_bundle()).unwrap();
        // Force the selector to V1 but keep V2-shaped body — decode
        // must refuse.
        let v1_sel = execute_trade_selector();
        calldata[..4].copy_from_slice(&v1_sel);
        match decode_execute_trade_v2_calldata(&calldata) {
            Err(error) => assert!(format!("{error}").contains("selector mismatch")),
            Ok(_) => panic!("expected decode to reject V1 selector"),
        }
    }

    #[test]
    fn version_dispatched_calldata_matches_direct_encoders() {
        let src = payload();
        let sigs = signature_bundle();

        let v1_direct = encode_execute_trade_calldata(&src, &sigs).unwrap();
        let v1_dispatched =
            encode_execute_trade_calldata_for_version(&src, &sigs, PerpsProtocolVersion::V1)
                .unwrap();
        assert_eq!(v1_direct, v1_dispatched);

        let v2_direct = encode_execute_trade_v2_calldata(&src, &sigs).unwrap();
        let v2_dispatched =
            encode_execute_trade_calldata_for_version(&src, &sigs, PerpsProtocolVersion::V2)
                .unwrap();
        assert_eq!(v2_direct, v2_dispatched);

        assert_eq!(
            execute_trade_selector_for(PerpsProtocolVersion::V1),
            execute_trade_selector()
        );
        assert_eq!(
            execute_trade_selector_for(PerpsProtocolVersion::V2),
            execute_trade_v2_selector()
        );
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
