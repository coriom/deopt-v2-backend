use crate::error::Result;
use crate::execution::{PerpTradePayload, PerpTradeSignatureBundle};
use crate::signing::eip712::{keccak256, parse_evm_address};
use alloy_primitives::{Address, Bytes, U256};
use alloy_sol_types::{sol, SolCall};

sol! {
    struct PerpTrade {
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

pub const EXECUTE_TRADE_SIGNATURE: &str =
    "executeTrade((address,address,uint256,uint128,uint128,bool,uint256,uint256,uint256),bytes,bytes)";

pub fn execute_trade_selector() -> [u8; 4] {
    executeTradeCall::SELECTOR
}

pub fn encode_execute_trade_calldata(
    payload: &PerpTradePayload,
    signatures: &PerpTradeSignatureBundle,
) -> Result<Vec<u8>> {
    payload.validate()?;
    let call = executeTradeCall {
        t: PerpTrade {
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

pub fn expected_execute_trade_selector() -> [u8; 4] {
    let hash = keccak256(EXECUTE_TRADE_SIGNATURE.as_bytes());
    [hash[0], hash[1], hash[2], hash[3]]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::execution::{PerpTradePayload, PerpTradeSignatureBundle};
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
