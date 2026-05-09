use crate::execution::transaction::hex_0x;
use crate::signing::eip712::keccak256;

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
