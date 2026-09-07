//! DEOPT_MULTICHAIN_MULTICOLLATERAL_FOUNDATION_V1 — collateral
//! configuration layer.
//!
//! Canonical registry of every collateral asset the protocol may ever
//! accept. Whether the protocol actually *accepts deposits of* a given
//! asset is controlled by `deposit_enabled`; whether it accepts
//! withdrawals is controlled by `withdrawal_enabled`. The two flags
//! are independent so an asset can be paused for deposits while
//! withdrawals keep draining.
//!
//! Runtime posture (V1):
//!   * `USDC` — `deposit_enabled = true`, `withdrawal_enabled = true`.
//!     The only asset that touches user balances. `collateral_factor`
//!     is `10_000` (100%, no haircut) so the risk-adjusted value equals
//!     the raw USD value. Adding a haircut would change existing user
//!     economics and is forbidden in V1.
//!   * `WETH`, `CBBTC` — represented so future haircut-based
//!     collateralisation can be wired without a schema rewrite, but
//!     both are `deposit_enabled = false` and `collateral_factor` is
//!     deliberately set to `0` so accidentally enabling one produces a
//!     zero-value collateral (the risk engine refuses to open positions
//!     against zero collateral).
//!
//! # Settlement vs. collateral separation
//!
//! `settlement_pnl_asset()` returns the canonical PnL / settlement
//! denomination. In V1 this is always USDC regardless of what asset
//! backs the subaccount. Future users may collateralise with WETH or
//! cbBTC while PnL keeps being denominated in USDC/USD — that
//! separation is a hard V2 architectural rule and is enforced by
//! keeping this function returning a single symbol.
//!
//! # Fail-closed
//!
//! Every gate in the risk / vault / margin path that inspects
//! `deposit_enabled` MUST treat "unknown symbol" and "false" the same
//! way: reject. There is no default-open behaviour.

use std::fmt;

/// Canonical USDC symbol used across the platform.
pub const USDC_SYMBOL: &str = "USDC";
/// Canonical WETH symbol used across the platform.
pub const WETH_SYMBOL: &str = "WETH";
/// Canonical cbBTC symbol used across the platform.
pub const CBBTC_SYMBOL: &str = "cbBTC";

/// Collateral factor is stored in basis points scaled to 10_000 = 100%.
/// A factor of `10_000` means "no haircut" (raw USD value = risk-
/// adjusted value). A factor of `0` is a hard refusal to count the
/// asset toward margin, even if a balance exists.
pub const COLLATERAL_FACTOR_BPS_MAX: u16 = 10_000;

/// Canonical collateral configuration entry.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CollateralConfig {
    pub asset_symbol: &'static str,
    /// Decimals of the underlying ERC-20 (USDC=6, WETH=18, cbBTC=8).
    pub decimals: u8,
    /// Basis points, 10_000 = 100%. See `COLLATERAL_FACTOR_BPS_MAX`.
    pub collateral_factor_bps: u16,
    /// Basis points, 10_000 = 100%. Multiplier applied when the risk
    /// engine values collateral for liquidation-eligibility checks.
    /// Must be `<= collateral_factor_bps`.
    pub liquidation_factor_bps: u16,
    /// Whether the vault currently accepts deposits of this asset.
    pub deposit_enabled: bool,
    /// Whether the vault currently accepts withdrawals of this asset.
    pub withdrawal_enabled: bool,
}

impl CollateralConfig {
    /// True iff the asset can be actively used as margin backing
    /// (deposits open + collateral factor > 0).
    #[inline]
    pub fn is_active_margin_backing(&self) -> bool {
        self.deposit_enabled && self.collateral_factor_bps > 0
    }
}

impl fmt::Display for CollateralConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}(factor_bps={},deposit={},withdrawal={})",
            self.asset_symbol,
            self.collateral_factor_bps,
            self.deposit_enabled,
            self.withdrawal_enabled
        )
    }
}

/// USDC — the only V1-enabled collateral asset. Collateral factor is
/// 100% (no haircut) so the risk engine values USDC deposits at their
/// raw USD amount, preserving V1 economics exactly.
pub const USDC: CollateralConfig = CollateralConfig {
    asset_symbol: USDC_SYMBOL,
    decimals: 6,
    collateral_factor_bps: COLLATERAL_FACTOR_BPS_MAX,
    liquidation_factor_bps: COLLATERAL_FACTOR_BPS_MAX,
    deposit_enabled: true,
    withdrawal_enabled: true,
};

/// WETH — registered but disabled. Collateral factor and liquidation
/// factor are both `0` so accidentally flipping `deposit_enabled` to
/// `true` still refuses to open positions against WETH balances.
pub const WETH: CollateralConfig = CollateralConfig {
    asset_symbol: WETH_SYMBOL,
    decimals: 18,
    collateral_factor_bps: 0,
    liquidation_factor_bps: 0,
    deposit_enabled: false,
    withdrawal_enabled: false,
};

/// cbBTC — registered but disabled. Same posture as WETH.
pub const CBBTC: CollateralConfig = CollateralConfig {
    asset_symbol: CBBTC_SYMBOL,
    decimals: 8,
    collateral_factor_bps: 0,
    liquidation_factor_bps: 0,
    deposit_enabled: false,
    withdrawal_enabled: false,
};

pub const KNOWN_COLLATERAL: &[CollateralConfig] = &[USDC, WETH, CBBTC];

pub fn find_collateral(symbol: &str) -> Option<&'static CollateralConfig> {
    KNOWN_COLLATERAL
        .iter()
        .find(|c| c.asset_symbol.eq_ignore_ascii_case(symbol))
}

pub fn enabled_collateral() -> Vec<&'static CollateralConfig> {
    KNOWN_COLLATERAL.iter().filter(|c| c.deposit_enabled).collect()
}

/// Canonical settlement / PnL asset. Always USDC in V1 regardless of
/// what asset backs the subaccount. Kept as a function rather than a
/// bare constant so future changes surface in every call site.
pub const fn settlement_pnl_asset() -> &'static str {
    USDC_SYMBOL
}

/// Assert the V1 invariant: exactly one collateral asset is enabled
/// for deposits, and it is USDC with a 100% collateral factor.
pub fn assert_v1_single_collateral_invariant() -> Result<(), &'static str> {
    let enabled: Vec<&CollateralConfig> = enabled_collateral();
    if enabled.len() != 1 {
        return Err("collateral registry V1 invariant: exactly one asset must be enabled");
    }
    let asset = enabled[0];
    if !asset.asset_symbol.eq_ignore_ascii_case(USDC_SYMBOL) {
        return Err("collateral registry V1 invariant: enabled asset must be USDC");
    }
    if asset.collateral_factor_bps != COLLATERAL_FACTOR_BPS_MAX {
        return Err(
            "collateral registry V1 invariant: USDC collateral factor must be 100% (10_000 bps)",
        );
    }
    if asset.liquidation_factor_bps != COLLATERAL_FACTOR_BPS_MAX {
        return Err(
            "collateral registry V1 invariant: USDC liquidation factor must be 100% (10_000 bps)",
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usdc_is_the_only_enabled_collateral() {
        let enabled = enabled_collateral();
        assert_eq!(enabled.len(), 1);
        assert_eq!(enabled[0].asset_symbol, USDC_SYMBOL);
        assert!(enabled[0].is_active_margin_backing());
    }

    #[test]
    fn v1_invariant_holds() {
        assert!(assert_v1_single_collateral_invariant().is_ok());
    }

    #[test]
    fn usdc_haircut_preserves_v1_economics() {
        // If this assertion ever fails, existing user margin balances
        // would silently change. Do not "fix" this test — fix the
        // constant.
        assert_eq!(USDC.collateral_factor_bps, COLLATERAL_FACTOR_BPS_MAX);
        assert_eq!(USDC.liquidation_factor_bps, COLLATERAL_FACTOR_BPS_MAX);
    }

    #[test]
    fn weth_and_cbbtc_are_registered_but_inert() {
        for sym in [WETH_SYMBOL, CBBTC_SYMBOL] {
            let cfg = find_collateral(sym).expect("must be in registry");
            assert!(!cfg.deposit_enabled);
            assert!(!cfg.withdrawal_enabled);
            assert!(!cfg.is_active_margin_backing());
            assert_eq!(cfg.collateral_factor_bps, 0);
            assert_eq!(cfg.liquidation_factor_bps, 0);
        }
    }

    #[test]
    fn settlement_asset_is_usdc_in_v1() {
        assert_eq!(settlement_pnl_asset(), USDC_SYMBOL);
    }

    #[test]
    fn lookup_is_case_insensitive_for_operator_ergonomics() {
        assert!(find_collateral("usdc").is_some());
        assert!(find_collateral("USDC").is_some());
        assert!(find_collateral("UsDc").is_some());
        assert!(find_collateral("cbbtc").is_some());
    }

    #[test]
    fn unknown_symbol_returns_none() {
        assert!(find_collateral("DOGE").is_none());
    }
}
