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
                },
            ],
            stale_after_sec: DEFAULT_STALE_AFTER_SEC,
        }
    }

    /// Validate at startup. Returns `Err(BackendError::Config)` on
    /// any misconfiguration that would silently produce nonsense
    /// downstream (e.g. mainnet chain id, RPC missing while enabled,
    /// registry address missing while enabled).
    pub fn validate_startup(&self) -> crate::error::Result<()> {
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
        Ok(())
    }

    /// Look up one seeded market row by human symbol.
    pub fn market_by_symbol(&self, symbol: &str) -> Option<&PerpsReadMarket> {
        self.markets.iter().find(|m| m.symbol == symbol)
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
