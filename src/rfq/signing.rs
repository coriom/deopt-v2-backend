use crate::error::{BackendError, Result};
use crate::execution::transaction::hex_0x;
use crate::signing::eip712::{keccak256, parse_evm_address, EIP712_DOMAIN_TYPE};
use crate::signing::Eip712Domain;
use crate::types::{AccountId, MarketId, Price1e8, Size1e8};
use alloy_primitives::B256;

pub const RFQ_QUOTE_TYPE: &str = "RFQQuote(bytes32 rfqId,address mmAccount,uint256 marketId,bool takerIsBuyer,uint128 price1e8,uint128 size1e8,uint256 quoteNonce,uint256 expiry)";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RfqQuoteSigningPayload {
    pub rfq_id: B256,
    pub mm_account: AccountId,
    pub market_id: MarketId,
    pub taker_is_buyer: bool,
    pub price_1e8: Price1e8,
    pub size_1e8: Size1e8,
    pub quote_nonce: u128,
    pub expiry: u128,
}

impl RfqQuoteSigningPayload {
    pub fn validate(&self) -> Result<()> {
        if self.rfq_id == B256::ZERO {
            return Err(BackendError::InvalidRfqId);
        }
        parse_evm_address(&self.mm_account)?;
        if self.price_1e8 == 0 {
            return Err(BackendError::ZeroPrice);
        }
        if self.size_1e8 == 0 {
            return Err(BackendError::ZeroSize);
        }
        Ok(())
    }
}

pub fn rfq_id_to_b256(rfq_id: &str) -> B256 {
    B256::from(keccak256(rfq_id.as_bytes()))
}

pub fn rfq_id_to_hex_bytes32(rfq_id: &str) -> String {
    hex_0x(rfq_id_to_b256(rfq_id).as_slice())
}

pub fn rfq_quote_digest(payload: &RfqQuoteSigningPayload, domain: &Eip712Domain) -> Result<String> {
    let digest = rfq_quote_digest_bytes(payload, domain)?;
    Ok(hex_0x(&digest))
}

pub fn rfq_quote_digest_bytes(
    payload: &RfqQuoteSigningPayload,
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

pub fn rfq_quote_typehash() -> [u8; 32] {
    keccak256(RFQ_QUOTE_TYPE.as_bytes())
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

fn quote_hash(payload: &RfqQuoteSigningPayload) -> Result<[u8; 32]> {
    payload.validate()?;
    let mm_account = parse_evm_address(&payload.mm_account)?;

    let mut encoded = Vec::with_capacity(288);
    encoded.extend_from_slice(&rfq_quote_typehash());
    encoded.extend_from_slice(payload.rfq_id.as_slice());
    encoded.extend_from_slice(&encode_address(&mm_account));
    encoded.extend_from_slice(&encode_u128(payload.market_id as u128));
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
