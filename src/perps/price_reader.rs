//! PERPS-MINIMAL-MARKET-AND-PRICE-V1 — read-only OracleRouter price
//! reader.
//!
//! `getPriceSafe(base, quote) view returns (uint256, uint256, bool)`
//! returns `(price_1e8, updated_at_sec, ok)`. If `ok == false` (feed
//! paused, stale, missing, or a source read failed inside the router),
//! we surface `PerpsPriceUnavailable` and never invent a number.

use crate::error::{BackendError, Result};
use crate::execution::rpc::{EthCallProvider, EthCallRequest};
use crate::perps::config::{PerpsReadConfig, PerpsReadMarket};
use crate::types::{AccountId, TimestampMs};
use alloy_primitives::{Address, U256};
use alloy_sol_types::{sol, SolCall};
use async_trait::async_trait;

sol! {
    /// OracleRouter (0xb416406f200b2ef3d7a86a5d5877ed41d9b1a581 on
    /// Base Sepolia). `getPriceSafe` is the best-effort variant — it
    /// never reverts; on any failure it returns `(0, 0, false)`. We
    /// prefer it over the reverting `getPrice` because a read-only
    /// endpoint should never surface a chain-side revert as an HTTP
    /// 500.
    function getPriceSafe(address baseAsset, address quoteAsset)
        external view returns (uint256 price, uint256 updatedAt, bool ok);
}

/// A single oracle read for one market.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RawPriceRead {
    /// Price scaled by 1e8 (matches `OracleRouter.PRICE_SCALE = 1e8`).
    pub price_1e8: u128,
    /// Unix seconds from the oracle. Convert to ms with `* 1000` when
    /// crossing the API boundary.
    pub updated_at_sec: u64,
    /// The router's `ok` flag. False → treat as unavailable.
    pub ok: bool,
}

impl RawPriceRead {
    /// `oracle_timestamp_ms` for the wire format.
    pub fn updated_at_ms(&self) -> TimestampMs {
        (self.updated_at_sec as i64).saturating_mul(1000)
    }
}

#[async_trait]
pub trait PerpOraclePriceReader: Send + Sync {
    /// Reads the router-side `getPriceSafe(base, quote)` and returns
    /// the raw price + timestamp + ok flag. Returns
    /// `Err(BackendError::PerpsPriceUnavailable)` when the JSON-RPC
    /// call itself fails (network / decode). A `RawPriceRead { ok:
    /// false, .. }` is a successful RPC response and should be
    /// surfaced to the caller so the service layer can decide how to
    /// render it (typically: mark the snapshot unavailable).
    async fn read_price(&self, market: &PerpsReadMarket) -> Result<RawPriceRead>;
}

#[derive(Clone)]
pub struct PerpOracleRouterRpcReader<P: EthCallProvider> {
    provider: P,
    oracle_address: AccountId,
    caller: AccountId,
}

impl<P: EthCallProvider> PerpOracleRouterRpcReader<P> {
    pub fn new(provider: P, cfg: &PerpsReadConfig) -> Result<Self> {
        let oracle_address = cfg.oracle_router_address.clone().ok_or_else(|| {
            BackendError::Config(
                "PerpOracleRouterRpcReader requires PERPS_ORACLE_ROUTER_ADDRESS \
                 to be set"
                    .to_string(),
            )
        })?;
        Ok(Self {
            provider,
            oracle_address,
            caller: AccountId::new("0x0000000000000000000000000000000000000000"),
        })
    }
}

#[async_trait]
impl<P: EthCallProvider + 'static> PerpOraclePriceReader for PerpOracleRouterRpcReader<P> {
    async fn read_price(&self, market: &PerpsReadMarket) -> Result<RawPriceRead> {
        let base = parse_evm_address(&market.base_asset_address).ok_or_else(|| {
            BackendError::Config(format!(
                "market {} has malformed base_asset_address",
                market.symbol
            ))
        })?;
        let quote = parse_evm_address(&market.quote_asset_address).ok_or_else(|| {
            BackendError::Config(format!(
                "market {} has malformed quote_asset_address",
                market.symbol
            ))
        })?;
        let request = EthCallRequest {
            from: self.caller.clone(),
            to: self.oracle_address.clone(),
            data: getPriceSafeCall {
                baseAsset: base,
                quoteAsset: quote,
            }
            .abi_encode(),
            value: 0,
            gas_limit: None,
        };
        let success = self.provider.eth_call(request).await.map_err(|err| {
            BackendError::PerpsPriceUnavailable(format!(
                "OracleRouter.getPriceSafe({}) rpc failed: {err}",
                market.symbol
            ))
        })?;
        let decoded =
            getPriceSafeCall::abi_decode_returns(&success.output, true).map_err(|err| {
                BackendError::PerpsPriceUnavailable(format!(
                    "failed to decode getPriceSafe({}) return: {err}",
                    market.symbol
                ))
            })?;
        Ok(RawPriceRead {
            price_1e8: u256_to_u128_saturating(decoded.price),
            updated_at_sec: u256_to_u64_saturating(decoded.updatedAt),
            ok: decoded.ok,
        })
    }
}

fn parse_evm_address(a: &AccountId) -> Option<Address> {
    let hex = a.0.strip_prefix("0x")?;
    if hex.len() != 40 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let mut bytes = [0u8; 20];
    for (i, chunk) in hex.as_bytes().chunks(2).enumerate() {
        let s = std::str::from_utf8(chunk).ok()?;
        bytes[i] = u8::from_str_radix(s, 16).ok()?;
    }
    Some(Address::from(bytes))
}

fn u256_to_u128_saturating(v: U256) -> u128 {
    let limbs = v.into_limbs(); // [u64; 4] little-endian
    if limbs[2] != 0 || limbs[3] != 0 {
        return u128::MAX;
    }
    ((limbs[1] as u128) << 64) | (limbs[0] as u128)
}

fn u256_to_u64_saturating(v: U256) -> u64 {
    let limbs = v.into_limbs();
    if limbs[1] != 0 || limbs[2] != 0 || limbs[3] != 0 {
        return u64::MAX;
    }
    limbs[0]
}

/// Deterministic in-memory reader for tests.
#[derive(Clone, Debug, Default)]
pub struct InMemoryPerpOraclePriceReader {
    by_symbol: std::collections::HashMap<String, RawPriceRead>,
    force_error: Option<String>,
}

impl InMemoryPerpOraclePriceReader {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_price(mut self, symbol: impl Into<String>, price: RawPriceRead) -> Self {
        self.by_symbol.insert(symbol.into(), price);
        self
    }

    pub fn with_forced_error(mut self, error: impl Into<String>) -> Self {
        self.force_error = Some(error.into());
        self
    }
}

#[async_trait]
impl PerpOraclePriceReader for InMemoryPerpOraclePriceReader {
    async fn read_price(&self, market: &PerpsReadMarket) -> Result<RawPriceRead> {
        if let Some(err) = &self.force_error {
            return Err(BackendError::PerpsPriceUnavailable(err.clone()));
        }
        Ok(self
            .by_symbol
            .get(&market.symbol)
            .cloned()
            .unwrap_or(RawPriceRead {
                price_1e8: 0,
                updated_at_sec: 0,
                ok: false,
            }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eth_market() -> PerpsReadMarket {
        PerpsReadMarket {
            symbol: "ETH-PERP".to_string(),
            onchain_market_id: 1,
            base_asset_label: "ETH".to_string(),
            quote_asset_label: "mUSDC".to_string(),
            base_asset_address: AccountId::new("0x4deebc5f537f3b8ba0e3393807b4d699d72bdd02"),
            quote_asset_address: AccountId::new("0x6eae407f5640b006fac9965182e238582a3b412e"),
            max_leverage: 10,
            maintenance_margin_bps: 500,
            max_order_size_1e8: None,
            max_order_notional_1e8: None,
            max_subaccount_notional_1e8: None,
            max_open_interest_1e8: None,
        }
    }

    #[tokio::test]
    async fn in_memory_reader_returns_seeded_price() {
        let reader = InMemoryPerpOraclePriceReader::new().with_price(
            "ETH-PERP",
            RawPriceRead {
                price_1e8: 350_000_000_000,
                updated_at_sec: 1_782_000_000,
                ok: true,
            },
        );
        let read = reader.read_price(&eth_market()).await.unwrap();
        assert_eq!(read.price_1e8, 350_000_000_000);
        assert!(read.ok);
    }

    #[tokio::test]
    async fn in_memory_reader_returns_unavailable_for_unseeded_symbol() {
        let reader = InMemoryPerpOraclePriceReader::new();
        let read = reader.read_price(&eth_market()).await.unwrap();
        assert!(!read.ok);
        assert_eq!(read.price_1e8, 0);
    }

    #[test]
    fn u256_to_u128_handles_full_range() {
        assert_eq!(u256_to_u128_saturating(U256::from(0u64)), 0);
        assert_eq!(u256_to_u128_saturating(U256::from(123u64)), 123);
        assert_eq!(u256_to_u128_saturating(U256::from(u128::MAX)), u128::MAX);
        // Anything requiring the upper 128 bits saturates.
        let big = U256::from(1u64) << 130;
        assert_eq!(u256_to_u128_saturating(big), u128::MAX);
    }

    #[test]
    fn u256_to_u64_saturates_when_exceeds_u64() {
        assert_eq!(u256_to_u64_saturating(U256::from(42u64)), 42);
        let big = U256::from(1u64) << 70;
        assert_eq!(u256_to_u64_saturating(big), u64::MAX);
    }
}
