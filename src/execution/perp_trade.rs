use crate::error::{BackendError, Result};
use crate::signing::eip712::{keccak256, parse_evm_address, EIP712_DOMAIN_TYPE};
use crate::types::AccountId;
use alloy_primitives::B256;
use serde::Serialize;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PerpTradePayload {
    pub intent_id: B256,
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
        intent_id: B256,
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
            intent_id,
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
        if self.intent_id == B256::ZERO {
            return Err(BackendError::InvalidPerpTradeIntentId);
        }
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

pub fn intent_id_to_b256(intent_id: &str) -> Result<B256> {
    let mapped = B256::from(keccak256(intent_id.as_bytes()));
    if mapped == B256::ZERO {
        return Err(BackendError::InvalidPerpTradeIntentId);
    }
    Ok(mapped)
}

pub fn intent_id_to_hex_bytes32(intent_id: &str) -> Result<String> {
    Ok(b256_to_hex_bytes32(&intent_id_to_b256(intent_id)?))
}

pub fn b256_to_hex_bytes32(intent_id: &B256) -> String {
    hex_0x(intent_id.as_slice())
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

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct StoredTradeSignatures {
    pub buyer_sig: Option<String>,
    pub seller_sig: Option<String>,
}

impl StoredTradeSignatures {
    pub fn upsert(&mut self, buyer_sig: Option<String>, seller_sig: Option<String>) -> Result<()> {
        if let Some(signature) = buyer_sig {
            validate_signature_hex(&signature)?;
            self.buyer_sig = Some(signature);
        }
        if let Some(signature) = seller_sig {
            validate_signature_hex(&signature)?;
            self.seller_sig = Some(signature);
        }
        Ok(())
    }

    pub fn buyer_signature_present(&self) -> bool {
        self.buyer_sig.is_some()
    }

    pub fn seller_signature_present(&self) -> bool {
        self.seller_sig.is_some()
    }

    pub fn calldata_ready(&self) -> bool {
        self.buyer_signature_present() && self.seller_signature_present()
    }

    pub fn missing_signatures(&self) -> bool {
        !self.calldata_ready()
    }

    pub fn bundle(&self) -> Result<Option<PerpTradeSignatureBundle>> {
        let Some(buyer_sig) = self.buyer_sig.as_deref() else {
            return Ok(None);
        };
        let Some(seller_sig) = self.seller_sig.as_deref() else {
            return Ok(None);
        };
        Ok(Some(PerpTradeSignatureBundle::new(buyer_sig, seller_sig)?))
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct TradeSignatureStatus {
    pub buyer_signature_present: bool,
    pub seller_signature_present: bool,
    pub calldata_ready: bool,
    pub missing_signatures: bool,
}

impl From<&StoredTradeSignatures> for TradeSignatureStatus {
    fn from(value: &StoredTradeSignatures) -> Self {
        Self {
            buyer_signature_present: value.buyer_signature_present(),
            seller_signature_present: value.seller_signature_present(),
            calldata_ready: value.calldata_ready(),
            missing_signatures: value.missing_signatures(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PerpTradeDomain {
    pub name: String,
    pub version: String,
    pub chain_id: u64,
    pub verifying_contract: AccountId,
}

impl PerpTradeDomain {
    pub fn new(chain_id: u64, verifying_contract: AccountId) -> Self {
        Self {
            name: "DeOptV2-PerpMatchingEngine".to_string(),
            version: "1".to_string(),
            chain_id,
            verifying_contract,
        }
    }
}

pub const PERP_TRADE_TYPE: &str = "PerpTrade(bytes32 intentId,address buyer,address seller,uint256 marketId,uint128 sizeDelta1e8,uint128 executionPrice1e8,bool buyerIsMaker,uint256 buyerNonce,uint256 sellerNonce,uint256 deadline)";

pub fn perp_trade_digest(payload: &PerpTradePayload, domain: &PerpTradeDomain) -> Result<String> {
    let domain_separator = domain_separator(domain)?;
    let trade_hash = perp_trade_hash(payload)?;
    let mut encoded = Vec::with_capacity(66);
    encoded.extend_from_slice(b"\x19\x01");
    encoded.extend_from_slice(&domain_separator);
    encoded.extend_from_slice(&trade_hash);
    Ok(hex_0x(&keccak256(&encoded)))
}

fn validate_signature_hex(signature: &str) -> Result<()> {
    decode_signature(signature).map(|_| ())
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

fn domain_separator(domain: &PerpTradeDomain) -> Result<[u8; 32]> {
    let verifying_contract = parse_evm_address(&domain.verifying_contract)?;
    let mut encoded = Vec::with_capacity(160);
    encoded.extend_from_slice(&keccak256(EIP712_DOMAIN_TYPE.as_bytes()));
    encoded.extend_from_slice(&keccak256(domain.name.as_bytes()));
    encoded.extend_from_slice(&keccak256(domain.version.as_bytes()));
    encoded.extend_from_slice(&encode_u64(domain.chain_id));
    encoded.extend_from_slice(&encode_address(&verifying_contract));
    Ok(keccak256(&encoded))
}

fn perp_trade_hash(payload: &PerpTradePayload) -> Result<[u8; 32]> {
    payload.validate()?;
    let buyer = parse_evm_address(&payload.buyer)?;
    let seller = parse_evm_address(&payload.seller)?;

    let mut encoded = Vec::with_capacity(352);
    encoded.extend_from_slice(&keccak256(PERP_TRADE_TYPE.as_bytes()));
    encoded.extend_from_slice(payload.intent_id.as_slice());
    encoded.extend_from_slice(&encode_address(&buyer));
    encoded.extend_from_slice(&encode_address(&seller));
    encoded.extend_from_slice(&encode_u128(payload.market_id));
    encoded.extend_from_slice(&encode_u128(payload.size_delta_1e8));
    encoded.extend_from_slice(&encode_u128(payload.execution_price_1e8));
    encoded.extend_from_slice(&encode_bool(payload.buyer_is_maker));
    encoded.extend_from_slice(&encode_u128(payload.buyer_nonce));
    encoded.extend_from_slice(&encode_u128(payload.seller_nonce));
    encoded.extend_from_slice(&encode_u128(payload.deadline));
    Ok(keccak256(&encoded))
}

fn encode_address(address: &[u8; 20]) -> [u8; 32] {
    let mut word = [0u8; 32];
    word[12..].copy_from_slice(address);
    word
}

fn encode_bool(value: bool) -> [u8; 32] {
    encode_u8(u8::from(value))
}

fn encode_u8(value: u8) -> [u8; 32] {
    let mut word = [0u8; 32];
    word[31] = value;
    word
}

fn encode_u64(value: u64) -> [u8; 32] {
    let mut word = [0u8; 32];
    word[24..].copy_from_slice(&value.to_be_bytes());
    word
}

fn encode_u128(value: u128) -> [u8; 32] {
    let mut word = [0u8; 32];
    word[16..].copy_from_slice(&value.to_be_bytes());
    word
}

fn hex_0x(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(2 + bytes.len() * 2);
    encoded.push_str("0x");
    for byte in bytes {
        encoded.push(HEX[(byte >> 4) as usize] as char);
        encoded.push(HEX[(byte & 0x0f) as usize] as char);
    }
    encoded
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
        assert_eq!(
            intent_id_to_hex_bytes32("00000000-0000-0000-0000-000000000001").unwrap(),
            hex_0x(payload.intent_id.as_slice())
        );
    }

    #[test]
    fn backend_intent_id_maps_deterministically_to_bytes32() {
        let intent_id = "550e8400-e29b-41d4-a716-446655440000";

        let first = intent_id_to_b256(intent_id).unwrap();
        let second = intent_id_to_b256(intent_id).unwrap();
        let hex = intent_id_to_hex_bytes32(intent_id).unwrap();

        assert_eq!(first, second);
        assert_eq!(hex.len(), 66);
        assert!(hex.starts_with("0x"));
        assert_ne!(first, B256::ZERO);
    }

    #[test]
    fn different_backend_intent_ids_map_to_different_bytes32_values() {
        let first = intent_id_to_b256("550e8400-e29b-41d4-a716-446655440000").unwrap();
        let second = intent_id_to_b256("550e8400-e29b-41d4-a716-446655440001").unwrap();

        assert_ne!(first, second);
    }

    #[test]
    fn zero_perp_trade_intent_id_is_rejected() {
        let error = PerpTradePayload::new(
            B256::ZERO,
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
        .unwrap_err();

        assert!(matches!(error, BackendError::InvalidPerpTradeIntentId));
    }

    #[test]
    fn invalid_buyer_address_is_rejected() {
        let error = PerpTradePayload::new(
            intent_id_to_b256("00000000-0000-0000-0000-000000000001").unwrap(),
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
            intent_id_to_b256("00000000-0000-0000-0000-000000000001").unwrap(),
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

    #[test]
    fn stored_signatures_report_calldata_readiness() {
        let mut signatures = StoredTradeSignatures::default();
        signatures.upsert(Some(signature_hex(0xaa)), None).unwrap();
        assert!(signatures.buyer_signature_present());
        assert!(!signatures.seller_signature_present());
        assert!(!signatures.calldata_ready());

        signatures.upsert(None, Some(signature_hex(0xbb))).unwrap();
        assert!(signatures.calldata_ready());
        assert!(signatures.bundle().unwrap().is_some());
    }

    #[test]
    fn perp_trade_digest_is_eip712_shape() {
        let digest = perp_trade_digest(
            &valid_payload(),
            &PerpTradeDomain::new(
                84532,
                AccountId::new("0x0000000000000000000000000000000000000009"),
            ),
        )
        .unwrap();

        assert_eq!(digest.len(), 66);
        assert!(digest.starts_with("0x"));
    }

    #[test]
    fn perp_trade_digest_changes_when_intent_id_changes() {
        let domain = PerpTradeDomain::new(
            84532,
            AccountId::new("0x0000000000000000000000000000000000000009"),
        );
        let first = perp_trade_digest(&valid_payload(), &domain).unwrap();
        let second = perp_trade_digest(
            &PerpTradePayload::new(
                intent_id_to_b256("00000000-0000-0000-0000-000000000002").unwrap(),
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
            .unwrap(),
            &domain,
        )
        .unwrap();

        assert_ne!(first, second);
    }

    #[test]
    fn perp_trade_digest_is_deterministic() {
        let domain = PerpTradeDomain::new(
            84532,
            AccountId::new("0x0000000000000000000000000000000000000009"),
        );

        assert_eq!(
            perp_trade_digest(&valid_payload(), &domain).unwrap(),
            perp_trade_digest(&valid_payload(), &domain).unwrap()
        );
    }

    fn valid_payload() -> PerpTradePayload {
        PerpTradePayload::new(
            intent_id_to_b256("00000000-0000-0000-0000-000000000001").unwrap(),
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
