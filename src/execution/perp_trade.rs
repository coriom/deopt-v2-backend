use crate::error::{BackendError, Result};
use crate::signing::eip712::parse_evm_address;
use crate::types::AccountId;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PerpTradePayload {
    pub buyer: AccountId,
    pub seller: AccountId,
    pub market_id: u128,
    pub size_delta_1e8: u128,
    pub execution_price_1e8: u128,
    pub buyer_is_maker: bool,
    pub buyer_nonce: u128,
    pub seller_nonce: u128,
    pub deadline: u128,
}

impl PerpTradePayload {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        buyer: AccountId,
        seller: AccountId,
        market_id: u128,
        size_delta_1e8: u128,
        execution_price_1e8: u128,
        buyer_is_maker: bool,
        buyer_nonce: u128,
        seller_nonce: u128,
        deadline: u128,
    ) -> Result<Self> {
        let payload = Self {
            buyer,
            seller,
            market_id,
            size_delta_1e8,
            execution_price_1e8,
            buyer_is_maker,
            buyer_nonce,
            seller_nonce,
            deadline,
        };
        payload.validate()?;
        Ok(payload)
    }

    pub fn validate(&self) -> Result<()> {
        parse_evm_address(&self.buyer)?;
        parse_evm_address(&self.seller)?;
        if self.size_delta_1e8 == 0 {
            return Err(BackendError::ZeroSize);
        }
        if self.execution_price_1e8 == 0 {
            return Err(BackendError::ZeroPrice);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PerpTradeSignatureBundle {
    pub buyer_sig: Vec<u8>,
    pub seller_sig: Vec<u8>,
}

impl PerpTradeSignatureBundle {
    pub fn new(buyer_sig: &str, seller_sig: &str) -> Result<Self> {
        Ok(Self {
            buyer_sig: decode_signature(buyer_sig)?,
            seller_sig: decode_signature(seller_sig)?,
        })
    }
}

fn decode_signature(signature: &str) -> Result<Vec<u8>> {
    let Some(hex) = signature.strip_prefix("0x") else {
        return Err(BackendError::MalformedSignature);
    };
    if hex.len() != 130 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(BackendError::MalformedSignature);
    }

    let mut bytes = vec![0u8; 65];
    decode_hex_to_slice(hex, &mut bytes).map_err(|_| BackendError::MalformedSignature)?;
    Ok(bytes)
}

fn decode_hex_to_slice(hex: &str, out: &mut [u8]) -> std::result::Result<(), ()> {
    if hex.len() != out.len() * 2 {
        return Err(());
    }

    for (index, byte) in out.iter_mut().enumerate() {
        let high = decode_hex_nibble(hex.as_bytes()[index * 2])?;
        let low = decode_hex_nibble(hex.as_bytes()[index * 2 + 1])?;
        *byte = (high << 4) | low;
    }

    Ok(())
}

fn decode_hex_nibble(byte: u8) -> std::result::Result<u8, ()> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => Err(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn perp_trade_payload_validates_addresses() {
        let payload = valid_payload();

        assert_eq!(payload.market_id, 1);
    }

    #[test]
    fn invalid_buyer_address_is_rejected() {
        let error = PerpTradePayload::new(
            AccountId::new("buyer"),
            AccountId::new("0x0000000000000000000000000000000000000002"),
            1,
            10,
            100,
            true,
            11,
            12,
            123,
        )
        .unwrap_err();

        assert!(matches!(error, BackendError::MalformedAccountAddress));
    }

    #[test]
    fn invalid_seller_address_is_rejected() {
        let error = PerpTradePayload::new(
            AccountId::new("0x0000000000000000000000000000000000000001"),
            AccountId::new("seller"),
            1,
            10,
            100,
            true,
            11,
            12,
            123,
        )
        .unwrap_err();

        assert!(matches!(error, BackendError::MalformedAccountAddress));
    }

    #[test]
    fn malformed_signature_is_rejected() {
        let error = PerpTradeSignatureBundle::new("0x1234", &signature_hex(0xbb)).unwrap_err();

        assert!(matches!(error, BackendError::MalformedSignature));
    }

    fn valid_payload() -> PerpTradePayload {
        PerpTradePayload::new(
            AccountId::new("0x0000000000000000000000000000000000000001"),
            AccountId::new("0x0000000000000000000000000000000000000002"),
            1,
            10,
            100,
            true,
            11,
            12,
            123,
        )
        .unwrap()
    }

    fn signature_hex(byte: u8) -> String {
        let mut signature = String::from("0x");
        for _ in 0..65 {
            signature.push_str(&format!("{byte:02x}"));
        }
        signature
    }
}
