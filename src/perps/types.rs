//! PERPS-MINIMAL-MARKET-AND-PRICE-V1 — read-only Perps types.

use crate::types::TimestampMs;
use serde::{Deserialize, Serialize};

/// A read-only Perps market surfaced by the backend. Mirrors the
/// on-chain `PerpMarketRegistry.getMarket(marketId)` result plus the
/// stable human symbol (e.g. `"ETH-PERP"`) the backend maps by
/// `onchain_market_id`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PerpMarket {
    /// Stable human symbol (e.g. `"ETH-PERP"`, `"BTC-PERP"`).
    pub market_id: String,
    /// The on-chain `uint256` market id (as a decimal string to preserve
    /// full precision).
    pub onchain_market_id: String,
    /// Underlying symbol (e.g. `"ETH"`).
    pub base_asset: String,
    /// Quote symbol (e.g. `"mUSDC"`).
    pub quote_asset: String,
    /// Read-only lifecycle status.
    pub status: PerpMarketStatus,
    /// Base Sepolia chain id (84532). Included on every row so
    /// downstream consumers can double-check without trusting a request
    /// path.
    pub chain_id: u64,
    /// Where the market row was sourced from. `"onchain_registry"` for
    /// live RPC reads; `"seed"` for a fallback that mirrors the on-chain
    /// registry without an RPC round-trip (used when RPC is unavailable
    /// but the backend still wants to surface market metadata).
    pub source: PerpMarketSource,
    /// Always false in this milestone. Explicit for the frontend so
    /// there's no ambiguity about the mutation gate.
    pub trading_enabled: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PerpMarketStatus {
    /// Market exists on-chain and is active. Read endpoints return it;
    /// mutations still 503.
    ReadOnly,
    /// Market exists on-chain but its `isActive` flag is false.
    Paused,
    /// The registry read returned no row for this market id (should be
    /// impossible for the seeded ETH-PERP / BTC-PERP but included so
    /// the service can distinguish "unknown market" from RPC failure).
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PerpMarketSource {
    OnchainRegistry,
    Seed,
}

/// A single price snapshot for a Perps market. All price values are
/// normalised to `1e8`, matching the contract-side `PRICE_SCALE = 1e8`
/// convention (`PerpMarketRegistry.sol:36`, `OracleRouter.sol:36`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PerpPriceSnapshot {
    /// Stable human symbol.
    pub market_id: String,
    /// Index price from `OracleRouter.getPriceSafe(base, quote)`, `1e8`.
    pub index_price_1e8: String,
    /// V1: mirrors `index_price_1e8`. A later `PERPS-FUNDING-V1` will
    /// swap this for `PerpEngineViews.getMarkPrice(marketId)`.
    pub mark_price_1e8: String,
    /// On-chain `updatedAt` reported by the oracle (Unix ms, computed
    /// as `updatedAt_sec * 1000`).
    pub oracle_timestamp_ms: TimestampMs,
    /// `"oracle_router"` on RPC success.
    pub source: PerpPriceSource,
    /// True when `now_ms - oracle_timestamp_ms > stale_after_ms`
    /// (config-driven).
    pub stale: bool,
    /// Always false in this milestone.
    pub trading_enabled: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PerpPriceSource {
    OracleRouter,
}
