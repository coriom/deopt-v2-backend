use crate::error::{BackendError, Result};
use crate::types::{AccountId, MarketId, Side, TimeInForce, TimestampMs};
use sha3::{Digest, Keccak256};

pub const EIP712_DOMAIN_TYPE: &str =
    "EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)";
pub const EIP712_ORDER_TYPE: &str = "DeOptOrder(address account,uint64 marketId,uint8 side,uint128 price1e8,uint128 size1e8,uint8 timeInForce,bool reduceOnly,bool postOnly,string clientOrderId,uint64 nonce,uint64 deadlineMs)";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Eip712Domain {
    pub name: String,
    pub version: String,
    pub chain_id: u64,
    pub verifying_contract: AccountId,
}

impl Default for Eip712Domain {
    fn default() -> Self {
        Self {
            name: "DeOptV2".to_string(),
            version: "1".to_string(),
            chain_id: 84532,
            verifying_contract: AccountId::new("0x0000000000000000000000000000000000000000"),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SignedOrder {
    pub account: AccountId,
    pub market_id: MarketId,
    pub side: Side,
    pub price_1e8: u128,
    pub size_1e8: u128,
    pub time_in_force: TimeInForce,
    pub reduce_only: bool,
    pub post_only: bool,
    pub client_order_id: Option<String>,
    pub nonce: u64,
    pub deadline_ms: TimestampMs,
    pub signature: String,
}

impl SignedOrder {
    pub fn eip712_digest(&self, domain: &Eip712Domain) -> Result<[u8; 32]> {
        eip712_digest(self, domain)
    }
}

pub fn eip712_digest(order: &SignedOrder, domain: &Eip712Domain) -> Result<[u8; 32]> {
    let domain_separator = domain_separator(domain)?;
    let order_hash = order_hash(order)?;

    let mut encoded = Vec::with_capacity(66);
    encoded.extend_from_slice(b"\x19\x01");
    encoded.extend_from_slice(&domain_separator);
    encoded.extend_from_slice(&order_hash);

    Ok(keccak256(&encoded))
}

pub fn parse_evm_address(value: &AccountId) -> Result<[u8; 20]> {
    let Some(hex) = value.0.strip_prefix("0x") else {
        return Err(BackendError::MalformedAccountAddress);
    };

    if hex.len() != 40 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(BackendError::MalformedAccountAddress);
    }

    let mut address = [0u8; 20];
    decode_hex_to_slice(hex, &mut address).map_err(|_| BackendError::MalformedAccountAddress)?;
    Ok(address)
}

pub fn keccak256(bytes: &[u8]) -> [u8; 32] {
    let mut hasher = Keccak256::new();
    hasher.update(bytes);
    hasher.finalize().into()
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

fn order_hash(order: &SignedOrder) -> Result<[u8; 32]> {
    let account = parse_evm_address(&order.account)?;
    let deadline_ms =
        u64::try_from(order.deadline_ms).map_err(|_| BackendError::DeadlineExpired)?;
    let client_order_id_hash = keccak256(
        order
            .client_order_id
            .as_deref()
            .unwrap_or_default()
            .as_bytes(),
    );

    let mut encoded = Vec::with_capacity(384);
    encoded.extend_from_slice(&keccak256(EIP712_ORDER_TYPE.as_bytes()));
    encoded.extend_from_slice(&encode_address(&account));
    encoded.extend_from_slice(&encode_u64(order.market_id));
    encoded.extend_from_slice(&encode_u8(side_value(order.side)));
    encoded.extend_from_slice(&encode_u128(order.price_1e8));
    encoded.extend_from_slice(&encode_u128(order.size_1e8));
    encoded.extend_from_slice(&encode_u8(time_in_force_value(order.time_in_force)));
    encoded.extend_from_slice(&encode_bool(order.reduce_only));
    encoded.extend_from_slice(&encode_bool(order.post_only));
    encoded.extend_from_slice(&client_order_id_hash);
    encoded.extend_from_slice(&encode_u64(order.nonce));
    encoded.extend_from_slice(&encode_u64(deadline_ms));

    Ok(keccak256(&encoded))
}

fn side_value(side: Side) -> u8 {
    match side {
        Side::Buy => 0,
        Side::Sell => 1,
    }
}

fn time_in_force_value(time_in_force: TimeInForce) -> u8 {
    match time_in_force {
        TimeInForce::Gtc => 0,
        TimeInForce::Ioc => 1,
        TimeInForce::Fok => 2,
    }
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
