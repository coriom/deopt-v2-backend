use super::OptionExecutionIntent;
use crate::error::{BackendError, Result};
use crate::execution::transaction::hex_0x;
use crate::signing::eip712::{keccak256, parse_evm_address, EIP712_DOMAIN_TYPE};
use crate::signing::Eip712Domain;
use crate::types::AccountId;
use alloy_primitives::{Address, Bytes, B256, U256};
use alloy_sol_types::{sol, SolCall};

pub const OPTION_TRADE_TYPE: &str = "OptionTrade(bytes32 intentId,address buyer,address seller,uint256 optionId,address underlying,address settlementAsset,uint64 expiry,uint64 strike1e8,bool isCall,uint128 contractSize1e8,uint128 quantity,uint128 premiumPerContract,bool buyerIsMaker,uint256 buyerNonce,uint256 sellerNonce,uint256 deadline)";

pub const OPTION_EXECUTE_TRADE_SIGNATURE: &str =
    "executeTrade((bytes32,address,address,uint256,address,address,uint64,uint64,bool,uint128,uint128,uint128,bool,uint256,uint256,uint256),bytes,bytes)";

sol! {
    struct OptionTrade {
        bytes32 intentId;
        address buyer;
        address seller;
        uint256 optionId;
        address underlying;
        address settlementAsset;
        uint64 expiry;
        uint64 strike1e8;
        bool isCall;
        uint128 contractSize1e8;
        uint128 quantity;
        uint128 premiumPerContract;
        bool buyerIsMaker;
        uint256 buyerNonce;
        uint256 sellerNonce;
        uint256 deadline;
    }

    function executeTrade(OptionTrade t, bytes buyerSig, bytes sellerSig);
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OptionTradePayload {
    pub intent_id: B256,
    pub buyer: AccountId,
    pub seller: AccountId,
    pub option_id: U256,
    pub underlying: AccountId,
    pub settlement_asset: AccountId,
    pub expiry: u64,
    pub strike_1e8: u64,
    pub is_call: bool,
    pub contract_size_1e8: u128,
    pub quantity: u128,
    pub premium_per_contract: u128,
    pub buyer_is_maker: bool,
    pub buyer_nonce: u128,
    pub seller_nonce: u128,
    pub deadline: u128,
}

impl OptionTradePayload {
    pub fn from_intent(intent: &OptionExecutionIntent) -> Result<Self> {
        let buyer_nonce = intent.buyer_nonce.ok_or_else(|| {
            BackendError::MissingExecutionMetadata("option buyer_nonce".to_string())
        })?;
        let seller_nonce = intent.seller_nonce.ok_or_else(|| {
            BackendError::MissingExecutionMetadata("option seller_nonce".to_string())
        })?;
        let payload = Self {
            intent_id: parse_bytes32(&intent.onchain_intent_id, "intentId")?,
            buyer: intent.buyer.clone(),
            seller: intent.seller.clone(),
            option_id: parse_u256(&intent.onchain_option_id, "optionId")?,
            underlying: intent.underlying.clone(),
            settlement_asset: intent.settlement_asset.clone(),
            expiry: intent.expiry,
            strike_1e8: u64::try_from(intent.strike_1e8).map_err(|_| {
                BackendError::InvalidOptionExecutionIntentState(
                    "strike_1e8 exceeds uint64".to_string(),
                )
            })?,
            is_call: intent.is_call,
            contract_size_1e8: intent.contract_size_1e8,
            quantity: intent.quantity_contracts,
            premium_per_contract: intent.premium_per_contract_native,
            buyer_is_maker: intent.buyer_is_maker,
            buyer_nonce,
            seller_nonce,
            deadline: u128::from(intent.deadline),
        };
        payload.validate()?;
        Ok(payload)
    }

    pub fn validate(&self) -> Result<()> {
        if self.intent_id == B256::ZERO {
            return Err(BackendError::InvalidOptionExecutionIntentState(
                "intentId must be nonzero".to_string(),
            ));
        }
        let buyer = parse_evm_address(&self.buyer)?;
        let seller = parse_evm_address(&self.seller)?;
        if buyer == seller {
            return Err(BackendError::InvalidOptionExecutionIntentState(
                "buyer and seller must differ".to_string(),
            ));
        }
        let underlying = parse_evm_address(&self.underlying)?;
        let settlement_asset = parse_evm_address(&self.settlement_asset)?;
        if is_zero_address(&buyer) || is_zero_address(&seller) {
            return Err(BackendError::InvalidOptionExecutionIntentState(
                "buyer and seller must be nonzero".to_string(),
            ));
        }
        if is_zero_address(&underlying) || is_zero_address(&settlement_asset) {
            return Err(BackendError::InvalidOptionExecutionIntentState(
                "underlying and settlementAsset must be nonzero addresses".to_string(),
            ));
        }
        if self.option_id == U256::ZERO {
            return Err(BackendError::InvalidOptionExecutionIntentState(
                "optionId must be nonzero".to_string(),
            ));
        }
        if self.expiry == 0 || self.strike_1e8 == 0 || self.contract_size_1e8 == 0 {
            return Err(BackendError::InvalidOptionExecutionIntentState(
                "option metadata must be nonzero".to_string(),
            ));
        }
        if self.quantity == 0 {
            return Err(BackendError::ZeroSize);
        }
        if self.premium_per_contract == 0 {
            return Err(BackendError::ZeroPrice);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OptionTradeSignatureBundle {
    pub buyer_signature: Vec<u8>,
    pub seller_signature: Vec<u8>,
}

impl OptionTradeSignatureBundle {
    pub fn new(buyer_signature: &str, seller_signature: &str) -> Result<Self> {
        Ok(Self {
            buyer_signature: decode_signature(buyer_signature)?,
            seller_signature: decode_signature(seller_signature)?,
        })
    }
}

pub fn option_execution_intent_id_to_b256(intent_id: &str) -> Result<B256> {
    let mapped = B256::from(keccak256(intent_id.as_bytes()));
    if mapped == B256::ZERO {
        return Err(BackendError::InvalidOptionExecutionIntentId);
    }
    Ok(mapped)
}

pub fn option_execution_intent_id_to_hex_bytes32(intent_id: &str) -> Result<String> {
    Ok(hex_0x(
        option_execution_intent_id_to_b256(intent_id)?.as_slice(),
    ))
}

pub fn normalize_u256_string(value: &str, field: &str) -> Result<String> {
    Ok(parse_u256(value, field)?.to_string())
}

pub fn option_trade_digest(payload: &OptionTradePayload, domain: &Eip712Domain) -> Result<String> {
    Ok(hex_0x(&option_trade_digest_bytes(payload, domain)?))
}

pub fn option_trade_digest_bytes(
    payload: &OptionTradePayload,
    domain: &Eip712Domain,
) -> Result<[u8; 32]> {
    let domain_separator = domain_separator(domain)?;
    let trade_hash = option_trade_hash(payload)?;
    let mut encoded = Vec::with_capacity(66);
    encoded.extend_from_slice(b"\x19\x01");
    encoded.extend_from_slice(&domain_separator);
    encoded.extend_from_slice(&trade_hash);
    Ok(keccak256(&encoded))
}

pub fn option_trade_typehash() -> [u8; 32] {
    keccak256(OPTION_TRADE_TYPE.as_bytes())
}

pub fn option_execute_trade_selector() -> [u8; 4] {
    executeTradeCall::SELECTOR
}

pub fn expected_option_execute_trade_selector() -> [u8; 4] {
    let hash = keccak256(OPTION_EXECUTE_TRADE_SIGNATURE.as_bytes());
    [hash[0], hash[1], hash[2], hash[3]]
}

pub fn encode_option_execute_trade_calldata(
    payload: &OptionTradePayload,
    signatures: &OptionTradeSignatureBundle,
) -> Result<Vec<u8>> {
    payload.validate()?;
    let call = executeTradeCall {
        t: OptionTrade {
            intentId: payload.intent_id,
            buyer: Address::from(parse_evm_address(&payload.buyer)?),
            seller: Address::from(parse_evm_address(&payload.seller)?),
            optionId: payload.option_id,
            underlying: Address::from(parse_evm_address(&payload.underlying)?),
            settlementAsset: Address::from(parse_evm_address(&payload.settlement_asset)?),
            expiry: payload.expiry,
            strike1e8: payload.strike_1e8,
            isCall: payload.is_call,
            contractSize1e8: payload.contract_size_1e8,
            quantity: payload.quantity,
            premiumPerContract: payload.premium_per_contract,
            buyerIsMaker: payload.buyer_is_maker,
            buyerNonce: U256::from(payload.buyer_nonce),
            sellerNonce: U256::from(payload.seller_nonce),
            deadline: U256::from(payload.deadline),
        },
        buyerSig: Bytes::from(signatures.buyer_signature.clone()),
        sellerSig: Bytes::from(signatures.seller_signature.clone()),
    };
    Ok(call.abi_encode())
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

fn option_trade_hash(payload: &OptionTradePayload) -> Result<[u8; 32]> {
    payload.validate()?;
    let buyer = parse_evm_address(&payload.buyer)?;
    let seller = parse_evm_address(&payload.seller)?;
    let underlying = parse_evm_address(&payload.underlying)?;
    let settlement_asset = parse_evm_address(&payload.settlement_asset)?;

    let mut encoded = Vec::with_capacity(544);
    encoded.extend_from_slice(&option_trade_typehash());
    encoded.extend_from_slice(payload.intent_id.as_slice());
    encoded.extend_from_slice(&encode_address(&buyer));
    encoded.extend_from_slice(&encode_address(&seller));
    encoded.extend_from_slice(&encode_u256(&payload.option_id));
    encoded.extend_from_slice(&encode_address(&underlying));
    encoded.extend_from_slice(&encode_address(&settlement_asset));
    encoded.extend_from_slice(&encode_u64(payload.expiry));
    encoded.extend_from_slice(&encode_u64(payload.strike_1e8));
    encoded.extend_from_slice(&encode_bool(payload.is_call));
    encoded.extend_from_slice(&encode_u128(payload.contract_size_1e8));
    encoded.extend_from_slice(&encode_u128(payload.quantity));
    encoded.extend_from_slice(&encode_u128(payload.premium_per_contract));
    encoded.extend_from_slice(&encode_bool(payload.buyer_is_maker));
    encoded.extend_from_slice(&encode_u128(payload.buyer_nonce));
    encoded.extend_from_slice(&encode_u128(payload.seller_nonce));
    encoded.extend_from_slice(&encode_u128(payload.deadline));
    Ok(keccak256(&encoded))
}

fn parse_u256(value: &str, field: &str) -> Result<U256> {
    let value = value.trim();
    if value.is_empty() {
        return Err(BackendError::InvalidOptionExecutionIntentState(format!(
            "{field} is required"
        )));
    }
    if let Some(hex) = value.strip_prefix("0x") {
        if hex.is_empty() || hex.len() > 64 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(BackendError::InvalidOptionExecutionIntentState(format!(
                "{field} must be a uint256"
            )));
        }
        let mut bytes = vec![0u8; 32];
        let offset = 32usize.checked_sub(hex.len().div_ceil(2)).ok_or_else(|| {
            BackendError::InvalidOptionExecutionIntentState(format!("{field} exceeds uint256"))
        })?;
        decode_hex_to_right_aligned_slice(hex, &mut bytes[offset..]).map_err(|_| {
            BackendError::InvalidOptionExecutionIntentState(format!("{field} must be a uint256"))
        })?;
        return Ok(U256::from_be_slice(&bytes));
    }
    if !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(BackendError::InvalidOptionExecutionIntentState(format!(
            "{field} must be a uint256"
        )));
    }
    U256::from_str_radix(value, 10).map_err(|error| {
        BackendError::InvalidOptionExecutionIntentState(format!(
            "{field} must be a uint256: {error}"
        ))
    })
}

fn parse_bytes32(value: &str, field: &str) -> Result<B256> {
    let hex = value.strip_prefix("0x").ok_or_else(|| {
        BackendError::InvalidOptionExecutionIntentState(format!(
            "{field} must be 0x-prefixed bytes32"
        ))
    })?;
    if hex.len() != 64 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(BackendError::InvalidOptionExecutionIntentState(format!(
            "{field} must be 0x-prefixed bytes32"
        )));
    }
    let mut bytes = [0u8; 32];
    decode_hex_to_slice(hex, &mut bytes).map_err(|_| {
        BackendError::InvalidOptionExecutionIntentState(format!(
            "{field} must be 0x-prefixed bytes32"
        ))
    })?;
    Ok(B256::from(bytes))
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

fn encode_u256(value: &U256) -> [u8; 32] {
    value.to_be_bytes()
}

fn is_zero_address(address: &[u8; 20]) -> bool {
    address.iter().all(|byte| *byte == 0)
}

fn decode_hex_to_right_aligned_slice(hex: &str, out: &mut [u8]) -> std::result::Result<(), ()> {
    if hex.len() > out.len() * 2 {
        return Err(());
    }
    if hex.len() % 2 == 1 {
        let first = decode_hex_nibble(hex.as_bytes()[0])?;
        out[0] = first;
        decode_hex_to_slice(&hex[1..], &mut out[1..])
    } else {
        decode_hex_to_slice(hex, out)
    }
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
    use crate::types::AccountId;

    #[test]
    fn option_trade_typehash_is_stable() {
        assert_eq!(
            hex_0x(&option_trade_typehash()),
            "0x13e38748a4a77f7a9e8be3a50bff4ada1cf49f08922ee9091926c38fc8532e52"
        );
    }

    #[test]
    fn option_execute_trade_selector_matches_signature() {
        assert_eq!(
            option_execute_trade_selector(),
            expected_option_execute_trade_selector()
        );
    }

    #[test]
    fn option_trade_digest_is_deterministic() {
        let domain = Eip712Domain {
            name: "DeOptV2-OptionMatchingEngine".to_string(),
            version: "1".to_string(),
            chain_id: 84532,
            verifying_contract: AccountId::new("0x00000000000000000000000000000000000000ee"),
        };
        let payload = payload();

        assert_eq!(
            option_trade_digest(&payload, &domain).unwrap(),
            option_trade_digest(&payload, &domain).unwrap()
        );
    }

    #[test]
    fn option_execute_trade_calldata_has_selector() {
        let calldata =
            encode_option_execute_trade_calldata(&payload(), &signature_bundle()).unwrap();

        assert!(!calldata.is_empty());
        assert_eq!(&calldata[..4], option_execute_trade_selector().as_slice());
    }

    fn payload() -> OptionTradePayload {
        OptionTradePayload {
            intent_id: option_execution_intent_id_to_b256("00000000-0000-0000-0000-000000000001")
                .unwrap(),
            buyer: AccountId::new("0x0000000000000000000000000000000000000001"),
            seller: AccountId::new("0x0000000000000000000000000000000000000002"),
            option_id: U256::from(123u64),
            underlying: AccountId::new("0x0000000000000000000000000000000000000010"),
            settlement_asset: AccountId::new("0x0000000000000000000000000000000000000020"),
            expiry: 4_102_444_800,
            strike_1e8: 300_000_000_000,
            is_call: true,
            contract_size_1e8: 100_000_000,
            quantity: 1,
            premium_per_contract: 10_000_000,
            buyer_is_maker: false,
            buyer_nonce: 0,
            seller_nonce: 0,
            deadline: 0,
        }
    }

    fn signature_bundle() -> OptionTradeSignatureBundle {
        OptionTradeSignatureBundle::new(&signature_hex(0xaa), &signature_hex(0xbb)).unwrap()
    }

    fn signature_hex(byte: u8) -> String {
        let mut signature = String::from("0x");
        for _ in 0..65 {
            signature.push_str(&format!("{byte:02x}"));
        }
        signature
    }
}
