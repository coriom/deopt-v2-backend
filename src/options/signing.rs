use crate::error::{BackendError, Result};
use crate::execution::transaction::hex_0x;
use crate::signing::eip712::{keccak256, parse_evm_address, EIP712_DOMAIN_TYPE};
use crate::signing::Eip712Domain;
use crate::types::{AccountId, Price1e8, Size1e8};
use alloy_primitives::B256;

pub const OPTION_RFQ_QUOTE_TYPE: &str = "OptionRFQQuote(bytes32 optionRfqId,address mmAccount,bytes32 optionSeriesId,bool takerIsBuyer,uint128 price1e8,uint128 size1e8,uint256 quoteNonce,uint256 expiry)";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OptionRfqQuoteSigningPayload {
    pub option_rfq_id: B256,
    pub mm_account: AccountId,
    pub option_series_id: B256,
    pub taker_is_buyer: bool,
    pub price_1e8: Price1e8,
    pub size_1e8: Size1e8,
    pub quote_nonce: u128,
    pub expiry: u128,
}

impl OptionRfqQuoteSigningPayload {
    pub fn validate(&self) -> Result<()> {
        if self.option_rfq_id == B256::ZERO {
            return Err(BackendError::InvalidOptionRfqId);
        }
        parse_evm_address(&self.mm_account)?;
        if self.option_series_id == B256::ZERO {
            return Err(BackendError::InvalidOptionSeriesId(
                "zero bytes32".to_string(),
            ));
        }
        if self.price_1e8 == 0 {
            return Err(BackendError::ZeroPrice);
        }
        if self.size_1e8 == 0 {
            return Err(BackendError::ZeroSize);
        }
        Ok(())
    }
}

pub fn option_rfq_id_to_b256(option_rfq_id: &str) -> B256 {
    B256::from(keccak256(option_rfq_id.as_bytes()))
}

pub fn option_rfq_id_to_hex_bytes32(option_rfq_id: &str) -> String {
    hex_0x(option_rfq_id_to_b256(option_rfq_id).as_slice())
}

pub fn option_series_id_to_b256(option_series_id: &str) -> Result<B256> {
    let hex = option_series_id.strip_prefix("0x").ok_or_else(|| {
        BackendError::InvalidOptionSeriesId("expected 0x-prefixed bytes32".to_string())
    })?;
    if hex.len() != 64 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(BackendError::InvalidOptionSeriesId(
            "expected 0x-prefixed bytes32".to_string(),
        ));
    }
    let mut bytes = [0u8; 32];
    decode_hex_to_slice(hex, &mut bytes).map_err(|_| {
        BackendError::InvalidOptionSeriesId("expected 0x-prefixed bytes32".to_string())
    })?;
    Ok(B256::from(bytes))
}

pub fn option_series_id_to_hex_bytes32(option_series_id: &str) -> Result<String> {
    Ok(hex_0x(
        option_series_id_to_b256(option_series_id)?.as_slice(),
    ))
}

pub fn option_rfq_quote_digest(
    payload: &OptionRfqQuoteSigningPayload,
    domain: &Eip712Domain,
) -> Result<String> {
    let digest = option_rfq_quote_digest_bytes(payload, domain)?;
    Ok(hex_0x(&digest))
}

pub fn option_rfq_quote_digest_bytes(
    payload: &OptionRfqQuoteSigningPayload,
    domain: &Eip712Domain,
) -> Result<[u8; 32]> {
    let domain_separator = domain_separator(domain)?;
    let quote_hash = quote_hash(payload)?;
    let mut encoded = Vec::with_capacity(66);
    encoded.extend_from_slice(b"\x19\x01");
    encoded.extend_from_slice(&domain_separator);
    encoded.extend_from_slice(&quote_hash);
    Ok(keccak256(&encoded))
}

pub fn option_rfq_quote_typehash() -> [u8; 32] {
    keccak256(OPTION_RFQ_QUOTE_TYPE.as_bytes())
}

fn domain_separator(domain: &Eip712Domain) -> Result<[u8; 32]> {
    let verifying_contract = parse_evm_address(&domain.verifying_contract)?;
    let mut encoded = Vec::with_capacity(160);
    encoded.extend_from_slice(&keccak256(EIP712_DOMAIN_TYPE.as_bytes()));
    encoded.extend_from_slice(&keccak256(domain.name.as_bytes()));
    encoded.extend_from_slice(&keccak256(domain.version.as_bytes()));
    encoded.extend_from_slice(&encode_u64(domain.chain_id));
    encoded.extend_from_slice(&encode_address(&verifying_contract));
    Ok(keccak256(&encoded))
}

fn quote_hash(payload: &OptionRfqQuoteSigningPayload) -> Result<[u8; 32]> {
    payload.validate()?;
    let mm_account = parse_evm_address(&payload.mm_account)?;

    let mut encoded = Vec::with_capacity(288);
    encoded.extend_from_slice(&option_rfq_quote_typehash());
    encoded.extend_from_slice(payload.option_rfq_id.as_slice());
    encoded.extend_from_slice(&encode_address(&mm_account));
    encoded.extend_from_slice(payload.option_series_id.as_slice());
    encoded.extend_from_slice(&encode_bool(payload.taker_is_buyer));
    encoded.extend_from_slice(&encode_u128(payload.price_1e8));
    encoded.extend_from_slice(&encode_u128(payload.size_1e8));
    encoded.extend_from_slice(&encode_u128(payload.quote_nonce));
    encoded.extend_from_slice(&encode_u128(payload.expiry));
    Ok(keccak256(&encoded))
}

fn encode_address(address: &[u8; 20]) -> [u8; 32] {
    let mut word = [0u8; 32];
    word[12..].copy_from_slice(address);
    word
}

fn encode_bool(value: bool) -> [u8; 32] {
    let mut word = [0u8; 32];
    word[31] = u8::from(value);
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
