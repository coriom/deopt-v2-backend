//! PERPS-MINIMAL-MARKET-AND-PRICE-V1 — read-only Perps configuration.
//!
//! **Off by default.** Every field defaults to a sentinel that keeps
//! the read endpoints returning `PerpsReadDisabled` (503) until an
//! operator explicitly wires the addresses via env. This mirrors the
//! posture of every other opt-in on-chain reader in the codebase
//! (`TradingViewsConfig::disabled()`, `PerpNonceSyncConfig::disabled()`).
//!
//! **Never activates mutations.** No field in this struct can flip the
//! Perps mutation gate. The `PerpsNotLive` handler-entry guards remain
//! in place regardless of what is configured here.

use crate::types::AccountId;

const DEFAULT_BASE_SEPOLIA_CHAIN_ID: u64 = 84532;
const DEFAULT_STALE_AFTER_SEC: u64 = 60;
const DEFAULT_ETH_ONCHAIN_MARKET_ID: u64 = 1;
const DEFAULT_BTC_ONCHAIN_MARKET_ID: u64 = 2;

// PERPS-MARGIN-ORACLE-RISK-V1 defaults.
//
// Deviation: OracleRouter guarantees `mark == index` in V1, but we
// thread the guard through the same pre-submit gate as the freshness
// check so a future divergence flips from "pass deterministically" to
// "reject at threshold". Threshold defaults to 500 bps (5 %).
const DEFAULT_MAX_DEVIATION_BPS: u32 = 500;
// Sane bounds for the deviation config — anything outside these is a
// startup-refusal because it either lets clearly unsafe prices through
// or is so tight it can never pass. `10_000` bps == 100 %.
const MAX_ALLOWED_DEVIATION_BPS: u32 = 5_000;
// Same reasoning for staleness — anything under 1s risks flapping,
// anything over an hour has no legitimate trading use case in V1.
const MIN_STALE_AFTER_SEC: u64 = 1;
const MAX_STALE_AFTER_SEC: u64 = 3_600;

/// One row of the read-only market catalogue. The (base, quote)
/// addresses tell `OracleRouter.getPriceSafe(base, quote)` which feed
/// to query; the human symbol is what the frontend renders.
///
/// PERPS-ISOLATED-MARGIN-POSITION-ENGINE-V1 — per-market risk
/// parameters live here rather than in a separate config so the
/// operator wires them alongside the addresses. Defaults are the
/// conservative V1 values from the milestone brief; every field is
/// overrideable via `PERPS_{ETH,BTC}_*` env vars.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PerpsReadMarket {
    /// Stable human symbol (e.g. `"ETH-PERP"`).
    pub symbol: String,
    /// The on-chain `uint256` market id (as a u64 — both seeded markets
    /// fit; we keep the wire format decimal-string in the API).
    pub onchain_market_id: u64,
    /// Underlying symbol (e.g. `"ETH"`).
    pub base_asset_label: String,
    /// Quote symbol (e.g. `"mUSDC"`).
    pub quote_asset_label: String,
    /// Address of the underlying ERC-20 (e.g. mWETH).
    pub base_asset_address: AccountId,
    /// Address of the quote ERC-20 (e.g. mUSDC).
    pub quote_asset_address: AccountId,
    /// Maximum permitted leverage for opening or increasing a
    /// position on this market. Example: `10` for ETH-PERP allows up
    /// to 10× notional per unit of isolated margin.
    pub max_leverage: u32,
    /// Maintenance margin as a fraction of notional, in basis points.
    /// Example: `500` = 5%. When the position's equity (margin +
    /// unrealised PnL) falls below `notional * maintenance_margin_bps
    /// / 10_000`, the position is eligible for liquidation in a
    /// later milestone.
    pub maintenance_margin_bps: u32,
    /// PERPS-MARGIN-ORACLE-RISK-V1 — per-market risk caps. Every cap
    /// is `None` for the disabled config and populated on the closed-
    /// test config. Reads default to `u128::MAX` behaviour (no cap)
    /// when `None`, matching the pre-milestone posture.
    ///
    /// Max order size (base units, `1e8` scaled). E.g. 10 ETH → `10 * 1e8 = 1_000_000_000`.
    pub max_order_size_1e8: Option<u128>,
    /// Max order notional (quote units, `1e8` scaled). E.g. $100k → `100_000 * 1e8 = 10_000_000_000_000`.
    pub max_order_notional_1e8: Option<u128>,
    /// Max aggregate open notional per (wallet, subaccount) on this
    /// market. Cross-subaccount notional is not netted.
    pub max_subaccount_notional_1e8: Option<u128>,
    /// Max market-wide open interest, in base units (`1e8`). Sums the
    /// size of every active position across every wallet + subaccount.
    pub max_open_interest_1e8: Option<u128>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PerpsReadConfig {
    /// Master switch. When false, every read endpoint returns
    /// `PerpsReadDisabled` (503).
    pub enabled: bool,
    /// Chain id the reader is bound to. Requests that surface a
    /// different chain id are rejected with `PerpsChainIdMismatch`.
    /// Defaults to Base Sepolia (84532). Mainnet ids are rejected
    /// during `validate_startup`.
    pub chain_id: u64,
    /// JSON-RPC endpoint. When `None`, the reader treats every price
    /// call as `PerpsPriceUnavailable`. Never printed to logs.
    pub rpc_url: Option<String>,
    /// Address of the deployed `PerpMarketRegistry`.
    pub market_registry_address: Option<AccountId>,
    /// Address of the deployed `OracleRouter`.
    pub oracle_router_address: Option<AccountId>,
    /// Rows we surface. Populated from env at startup.
    pub markets: Vec<PerpsReadMarket>,
    /// A price whose `updatedAt` is older than this becomes `stale=true`
    /// on the wire but is still returned. Default 60s (matches the
    /// mock-feed `maxDelay` seen in the base-sepolia deployment).
    pub stale_after_sec: u64,
    /// PERPS-MARGIN-ORACLE-RISK-V1 — max absolute deviation, in bps,
    /// between the trusted `index` price and the mark used for
    /// execution. V1 uses `mark == index` from OracleRouter, so the
    /// guard passes deterministically today; the field is validated at
    /// startup so a future divergence path (e.g. a per-market mark
    /// smoother) rejects anything above threshold rather than silently
    /// letting stale-shape prices settle risk-taking mutations.
    pub oracle_max_deviation_bps: u32,
}

impl PerpsReadConfig {
    /// The safe default: everything disabled, no addresses wired, no
    /// markets. Used by every existing `AppState` builder.
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            chain_id: DEFAULT_BASE_SEPOLIA_CHAIN_ID,
            rpc_url: None,
            market_registry_address: None,
            oracle_router_address: None,
            markets: Vec::new(),
            stale_after_sec: DEFAULT_STALE_AFTER_SEC,
            oracle_max_deviation_bps: DEFAULT_MAX_DEVIATION_BPS,
        }
    }

    /// A test-only preset that turns the reader on with two seeded
    /// markets and no RPC. Callers inject mock readers via the service
    /// signature; the concrete `PerpMarketRegistryRpcReader` /
    /// `PerpOracleRouterRpcReader` never runs in this mode.
    pub fn enabled_in_memory_for_tests() -> Self {
        Self {
            enabled: true,
            chain_id: DEFAULT_BASE_SEPOLIA_CHAIN_ID,
            rpc_url: None,
            market_registry_address: Some(AccountId::new(
                "0xb4fcf45e57b93274441def8f0f68bd30f6d677ec",
            )),
            oracle_router_address: Some(AccountId::new(
                "0xb416406f200b2ef3d7a86a5d5877ed41d9b1a581",
            )),
            markets: vec![
                PerpsReadMarket {
                    symbol: "ETH-PERP".to_string(),
                    onchain_market_id: DEFAULT_ETH_ONCHAIN_MARKET_ID,
                    base_asset_label: "ETH".to_string(),
                    quote_asset_label: "mUSDC".to_string(),
                    base_asset_address: AccountId::new(
                        "0x4deebc5f537f3b8ba0e3393807b4d699d72bdd02",
                    ),
                    quote_asset_address: AccountId::new(
                        "0x6eae407f5640b006fac9965182e238582a3b412e",
                    ),
                    max_leverage: 10,
                    maintenance_margin_bps: 500,
                    // PERPS-MARGIN-ORACLE-RISK-V1 — closed-test caps
                    // for ETH-PERP per the scope doc.
                    max_order_size_1e8: Some(10 * 100_000_000),
                    max_order_notional_1e8: Some(100_000 * 100_000_000),
                    max_subaccount_notional_1e8: Some(500_000 * 100_000_000),
                    max_open_interest_1e8: Some(50 * 100_000_000),
                },
                PerpsReadMarket {
                    symbol: "BTC-PERP".to_string(),
                    onchain_market_id: DEFAULT_BTC_ONCHAIN_MARKET_ID,
                    base_asset_label: "BTC".to_string(),
                    quote_asset_label: "mUSDC".to_string(),
                    base_asset_address: AccountId::new(
                        "0x9d871ac7595e8da271e866608e5145252047967c",
                    ),
                    quote_asset_address: AccountId::new(
                        "0x6eae407f5640b006fac9965182e238582a3b412e",
                    ),
                    max_leverage: 5,
                    maintenance_margin_bps: 750,
                    // BTC-PERP stays deferred in V1; caps set defensively.
                    max_order_size_1e8: Some(100_000_000),
                    max_order_notional_1e8: Some(100_000 * 100_000_000),
                    max_subaccount_notional_1e8: Some(500_000 * 100_000_000),
                    max_open_interest_1e8: Some(5 * 100_000_000),
                },
            ],
            stale_after_sec: DEFAULT_STALE_AFTER_SEC,
            oracle_max_deviation_bps: DEFAULT_MAX_DEVIATION_BPS,
        }
    }

    /// Validate at startup. Returns `Err(BackendError::Config)` on
    /// any misconfiguration that would silently produce nonsense
    /// downstream (e.g. mainnet chain id, RPC missing while enabled,
    /// registry address missing while enabled).
    pub fn validate_startup(&self) -> crate::error::Result<()> {
        // PERPS-MARGIN-ORACLE-RISK-V1 — bounds checks apply even in
        // the disabled config so a dangerous knob (e.g. 0 stale) is
        // rejected regardless of whether the reader is turned on.
        if self.oracle_max_deviation_bps == 0
            || self.oracle_max_deviation_bps > MAX_ALLOWED_DEVIATION_BPS
        {
            return Err(crate::error::BackendError::Config(format!(
                "PERPS_ORACLE_MAX_DEVIATION_BPS must be in (0, {MAX_ALLOWED_DEVIATION_BPS}], \
                 got {}",
                self.oracle_max_deviation_bps
            )));
        }
        if self.stale_after_sec < MIN_STALE_AFTER_SEC || self.stale_after_sec > MAX_STALE_AFTER_SEC
        {
            return Err(crate::error::BackendError::Config(format!(
                "PERPS_ORACLE_STALE_AFTER_SEC must be in [{MIN_STALE_AFTER_SEC}, \
                 {MAX_STALE_AFTER_SEC}], got {}",
                self.stale_after_sec
            )));
        }
        if !self.enabled {
            return Ok(());
        }
        if self.chain_id == 1 || self.chain_id == 8453 {
            return Err(crate::error::BackendError::Config(format!(
                "PERPS_READ_ENABLED must not run against mainnet chain id {} \
                 (this module is Base Sepolia read-only)",
                self.chain_id
            )));
        }
        if self.rpc_url.is_none() {
            return Err(crate::error::BackendError::Config(
                "PERPS_READ_ENABLED=true requires RPC_URL to be set (used by \
                 the market and oracle readers)"
                    .to_string(),
            ));
        }
        if self.market_registry_address.is_none() {
            return Err(crate::error::BackendError::Config(
                "PERPS_READ_ENABLED=true requires PERPS_MARKET_REGISTRY_ADDRESS".to_string(),
            ));
        }
        if self.oracle_router_address.is_none() {
            return Err(crate::error::BackendError::Config(
                "PERPS_READ_ENABLED=true requires PERPS_ORACLE_ROUTER_ADDRESS".to_string(),
            ));
        }
        if self.markets.is_empty() {
            return Err(crate::error::BackendError::Config(
                "PERPS_READ_ENABLED=true requires at least one market row \
                 (PERPS_ETH_MARKET_* or PERPS_BTC_MARKET_*)"
                    .to_string(),
            ));
        }
        for market in &self.markets {
            market.validate_startup()?;
        }
        Ok(())
    }

    /// Look up one seeded market row by human symbol.
    pub fn market_by_symbol(&self, symbol: &str) -> Option<&PerpsReadMarket> {
        self.markets.iter().find(|m| m.symbol == symbol)
    }
}

impl PerpsReadMarket {
    /// PERPS-MARGIN-ORACLE-RISK-V1 — per-market cross-consistency
    /// checks. Called during `PerpsReadConfig::validate_startup`.
    pub fn validate_startup(&self) -> crate::error::Result<()> {
        if self.max_leverage == 0 {
            return Err(crate::error::BackendError::Config(format!(
                "perps market {}: max_leverage must be > 0",
                self.symbol
            )));
        }
        if self.maintenance_margin_bps == 0
            || self.maintenance_margin_bps >= crate::perps::margin::BPS as u32
        {
            return Err(crate::error::BackendError::Config(format!(
                "perps market {}: maintenance_margin_bps must be in (0, {}), got {}",
                self.symbol,
                crate::perps::margin::BPS,
                self.maintenance_margin_bps
            )));
        }
        // Cross-consistency: maintenance margin must be strictly less
        // than initial margin. Otherwise every fresh open is instantly
        // liquidatable, which is a footgun. Initial margin bps at
        // `max_leverage`x is `10_000 / max_leverage`.
        let initial_bps = crate::perps::margin::BPS / (self.max_leverage as u128);
        if (self.maintenance_margin_bps as u128) >= initial_bps {
            return Err(crate::error::BackendError::Config(format!(
                "perps market {}: maintenance_margin_bps {} must be < initial-margin-at-max-leverage bps {} (max_leverage {})",
                self.symbol, self.maintenance_margin_bps, initial_bps, self.max_leverage
            )));
        }
        for (name, cap) in [
            ("max_order_size_1e8", self.max_order_size_1e8),
            ("max_order_notional_1e8", self.max_order_notional_1e8),
            (
                "max_subaccount_notional_1e8",
                self.max_subaccount_notional_1e8,
            ),
            ("max_open_interest_1e8", self.max_open_interest_1e8),
        ] {
            if let Some(0) = cap {
                return Err(crate::error::BackendError::Config(format!(
                    "perps market {}: {} must be > 0 (or None to disable)",
                    self.symbol, name
                )));
            }
        }
        Ok(())
    }
}

/// PERPS-MARGIN-ORACLE-RISK-V1 — market safety status. Composed at
/// read time from `(config, current price snapshot)`. Never persisted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PerpsMarketRiskStatus {
    /// Fresh oracle within deviation threshold; new orders allowed.
    Active,
    /// Oracle mark is stale beyond `stale_after_sec` or unavailable;
    /// new risk-increasing orders reject.
    StaleOracle { reason: &'static str },
    /// Absolute deviation between `index` and `mark` exceeded the
    /// configured `oracle_max_deviation_bps`; new orders reject.
    DeviationExceeded {
        observed_bps: u32,
        threshold_bps: u32,
    },
    /// Reserved — operator-driven pause. Currently only reachable via
    /// tests / the internal admin surface (not yet an env-driven knob
    /// in V1). New orders reject; reduce-only cancel remains allowed.
    Paused,
}

impl PerpsMarketRiskStatus {
    /// True when new risk-increasing orders may proceed. Reduce-only
    /// callers ignore this flag; the fill applicator computes its own
    /// reduce-vs-increase view at commit time.
    pub fn allows_new_risk(&self) -> bool {
        matches!(self, PerpsMarketRiskStatus::Active)
    }

    /// Structured reason string for API/WS surfaces. Stable — pinned
    /// by `perps_margin_oracle_risk_v1_tests.rs`.
    pub fn reason_code(&self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::StaleOracle { .. } => "stale_oracle",
            Self::DeviationExceeded { .. } => "deviation_exceeded",
            Self::Paused => "paused",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_config_validates_ok() {
        assert!(PerpsReadConfig::disabled().validate_startup().is_ok());
    }

    #[test]
    fn enabled_without_rpc_fails_validation() {
        let mut cfg = PerpsReadConfig::enabled_in_memory_for_tests();
        cfg.rpc_url = None;
        assert!(cfg.validate_startup().is_err());
    }

    #[test]
    fn enabled_on_mainnet_fails_validation() {
        let mut cfg = PerpsReadConfig::enabled_in_memory_for_tests();
        cfg.rpc_url = Some("https://example.invalid".to_string());
        cfg.chain_id = 1;
        assert!(cfg.validate_startup().is_err());
        cfg.chain_id = 8453;
        assert!(cfg.validate_startup().is_err());
    }

    #[test]
    fn enabled_without_market_registry_fails_validation() {
        let mut cfg = PerpsReadConfig::enabled_in_memory_for_tests();
        cfg.rpc_url = Some("https://example.invalid".to_string());
        cfg.market_registry_address = None;
        assert!(cfg.validate_startup().is_err());
    }

    #[test]
    fn seeded_test_config_finds_markets_by_symbol() {
        let cfg = PerpsReadConfig::enabled_in_memory_for_tests();
        assert_eq!(
            cfg.market_by_symbol("ETH-PERP")
                .map(|m| m.onchain_market_id),
            Some(1)
        );
        assert_eq!(
            cfg.market_by_symbol("BTC-PERP")
                .map(|m| m.onchain_market_id),
            Some(2)
        );
        assert!(cfg.market_by_symbol("NOPE-PERP").is_none());
    }
}
