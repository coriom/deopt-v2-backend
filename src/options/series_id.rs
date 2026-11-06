use crate::execution::transaction::hex_0x;
use crate::error::{BackendError, Result};
use crate::signing::eip712::keccak256;
use alloy_primitives::{Address, U256};
use std::str::FromStr;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OptionSeriesIdInput<'a> {
    pub underlying: &'a str,
    pub base_asset: &'a str,
    pub quote_asset: &'a str,
    pub settlement_asset: &'a str,
    pub expiry: u64,
    pub strike_1e8: u128,
    pub is_call: bool,
    pub contract_size_1e8: u128,
}

pub fn option_series_id(input: OptionSeriesIdInput<'_>) -> String {
    let canonical = format!(
        "{}|{}|{}|{}|{}|{}|{}|{}",
        canonical_asset(input.underlying),
        canonical_asset(input.base_asset),
        canonical_asset(input.quote_asset),
        canonical_asset(input.settlement_asset),
        input.expiry,
        input.strike_1e8,
        input.is_call,
        input.contract_size_1e8
    );
    hex_0x(&keccak256(canonical.as_bytes()))
}

fn canonical_asset(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

/// Stable database/API key for a deployed option series. The raw uint256
/// remains separate for Solidity ABI and EIP-712 encoding.
pub fn onchain_option_series_key(chain_id: u64, registry: &str, option_id: &str) -> Result<String> {
    if chain_id == 0 {
        return Err(BackendError::InvalidOptionSeriesId("zero chain id".into()));
    }
    let address = Address::from_str(registry)
        .map_err(|_| BackendError::InvalidOptionSeriesId("invalid registry address".into()))?;
    if address == Address::ZERO {
        return Err(BackendError::InvalidOptionSeriesId("zero registry address".into()));
    }
    let (digits, radix) = option_id
        .strip_prefix("0x")
        .map(|hex| (hex, 16))
        .unwrap_or((option_id, 10));
    let id = U256::from_str_radix(digits, radix)
        .map_err(|_| BackendError::InvalidOptionSeriesId("invalid onchain option id".into()))?;
    // RFQ signatures and existing API routes require a bytes32 series key.
    // Hash the full ABI tuple, while retaining its three source fields in
    // PostgreSQL so provenance remains inspectable.
    let mut encoded = [0u8; 96];
    encoded[..32].copy_from_slice(&U256::from(chain_id).to_be_bytes::<32>());
    encoded[44..64].copy_from_slice(address.as_slice());
    encoded[64..96].copy_from_slice(&id.to_be_bytes::<32>());
    Ok(hex_0x(&keccak256(&encoded)))
}

#[cfg(test)]
mod v15_tests {
    use super::onchain_option_series_key;

    #[test]
    fn same_raw_option_id_cannot_alias_across_registries_or_chains() {
        let first = onchain_option_series_key(84532, "0x1111111111111111111111111111111111111111", "7").unwrap();
        let second = onchain_option_series_key(84532, "0x2222222222222222222222222222222222222222", "0x07").unwrap();
        assert_ne!(first, second);
        assert_ne!(first, onchain_option_series_key(1, "0x1111111111111111111111111111111111111111", "7").unwrap());
        assert_eq!(first, onchain_option_series_key(84532, "0x1111111111111111111111111111111111111111", "0x0007").unwrap());
        assert_eq!(first, "0x0ffd65bace87c3ecbd455fa36283257624b7a3dcdfaa610f6f37d30dd1cbbf19");
        assert_eq!(first.len(), 66);
        assert!(onchain_option_series_key(84532, "not-an-address", "7").is_err());
    }
}
