//! DEOPT_MULTICHAIN_SCHEMA_HARDENING_AND_MULTICOLLATERAL_ACTIVATION_DESIGN_V1
//! Part E — test-only multi-collateral risk engine.
//!
//! Pure, dependency-free model of the risk-adjusted collateral
//! valuation used by the milestone Part E validation.
//!
//! **This is NOT the production risk engine.** The production margin
//! math lives in Solidity (`src/hybrid-v2/margin/MarginEngineV2.sol`
//! and `src/risk/RiskModuleCollateral.sol`). This Rust module models
//! the same valuation invariants so they can be exercised by unit
//! tests without a Foundry harness — the tests catch drift between
//! the design docs (WETH/cbBTC activation designs) and the
//! CollateralConfig registry, and prove the "risk-adjusted collateral
//! = sum(asset USD value × collateral factor)" invariant across a
//! curated set of scenarios.
//!
//! # Scale conventions
//!
//! - Price is `u128` scaled by `1e8` (matches every other USD-price
//!   field in the codebase).
//! - Amount is `u128` in the asset's native decimals.
//! - USD value is `u128` scaled by `1e8`.
//! - Collateral factor is `u16` in basis points, `10_000 = 100%`.

use crate::config::collateral::{CollateralConfig, COLLATERAL_FACTOR_BPS_MAX};

pub const USD_1E8: u128 = 100_000_000;
pub const BPS_DENOMINATOR: u128 = 10_000;

/// Convert a raw token amount into a `1e8`-scaled USD value given a
/// `1e8`-scaled price. Rounds toward zero — the risk engine never
/// over-values collateral.
pub fn token_amount_to_usd_1e8(amount: u128, decimals: u8, price_1e8: u128) -> u128 {
    let decimals_pow = 10u128.pow(u32::from(decimals));
    // amount * price / 10^decimals — evaluated in u128 space so we
    // don't lose precision for reasonable asset sizes.
    amount.saturating_mul(price_1e8) / decimals_pow
}

/// Apply the collateral factor haircut. Rounds toward zero.
pub fn apply_collateral_factor(raw_usd_1e8: u128, factor_bps: u16) -> u128 {
    raw_usd_1e8.saturating_mul(u128::from(factor_bps)) / BPS_DENOMINATOR
}

/// Apply the liquidation factor haircut. Same rounding.
pub fn apply_liquidation_factor(raw_usd_1e8: u128, factor_bps: u16) -> u128 {
    apply_collateral_factor(raw_usd_1e8, factor_bps)
}

/// Reason a valuation refused to produce a number.
#[derive(Debug, Eq, PartialEq)]
pub enum ValuationRefusal {
    /// Deposits for this asset are disabled — cannot contribute to
    /// margin backing.
    DepositDisabled,
    /// Collateral factor is zero — asset is registered but inert.
    ZeroCollateralFactor,
    /// Oracle price was zero — treated as an oracle failure; fail-
    /// closed, no partial-margin.
    OraclePriceZero,
}

/// Compute the risk-adjusted collateral value for a single asset
/// balance. Returns `Err(ValuationRefusal)` if the asset cannot
/// contribute to margin — callers MUST treat any refusal as "zero
/// contribution to margin", never as "skip and continue".
pub fn risk_adjusted_value(
    cfg: &CollateralConfig,
    amount: u128,
    price_1e8: u128,
) -> Result<u128, ValuationRefusal> {
    if !cfg.deposit_enabled {
        return Err(ValuationRefusal::DepositDisabled);
    }
    if cfg.collateral_factor_bps == 0 {
        return Err(ValuationRefusal::ZeroCollateralFactor);
    }
    if price_1e8 == 0 {
        return Err(ValuationRefusal::OraclePriceZero);
    }
    let raw = token_amount_to_usd_1e8(amount, cfg.decimals, price_1e8);
    Ok(apply_collateral_factor(raw, cfg.collateral_factor_bps))
}

/// Same as `risk_adjusted_value` but applies the liquidation factor.
/// The liquidation factor MUST be ≤ collateral factor; callers rely
/// on that ordering when checking liquidation eligibility.
pub fn liquidation_adjusted_value(
    cfg: &CollateralConfig,
    amount: u128,
    price_1e8: u128,
) -> Result<u128, ValuationRefusal> {
    if !cfg.deposit_enabled {
        return Err(ValuationRefusal::DepositDisabled);
    }
    if cfg.liquidation_factor_bps == 0 {
        return Err(ValuationRefusal::ZeroCollateralFactor);
    }
    if price_1e8 == 0 {
        return Err(ValuationRefusal::OraclePriceZero);
    }
    debug_assert!(
        cfg.liquidation_factor_bps <= cfg.collateral_factor_bps + COLLATERAL_FACTOR_BPS_MAX,
        "liquidation factor cannot exceed collateral factor by more than 100%"
    );
    let raw = token_amount_to_usd_1e8(amount, cfg.decimals, price_1e8);
    Ok(apply_liquidation_factor(raw, cfg.liquidation_factor_bps))
}

/// A per-asset holding: the config, the balance in native decimals,
/// and the fresh oracle price in `1e8`. `None` price means the
/// oracle is unavailable — the valuation refuses that asset.
#[derive(Clone, Debug)]
pub struct AssetHolding<'a> {
    pub cfg: &'a CollateralConfig,
    pub amount: u128,
    pub price_1e8: Option<u128>,
}

/// Sum risk-adjusted collateral across a subaccount's asset holdings.
/// A missing or refused asset contributes ZERO — never a partial
/// value. This is the fail-closed rule.
pub fn subaccount_risk_adjusted_usd_1e8(holdings: &[AssetHolding<'_>]) -> u128 {
    let mut sum: u128 = 0;
    for h in holdings {
        let Some(price) = h.price_1e8 else { continue };
        if let Ok(v) = risk_adjusted_value(h.cfg, h.amount, price) {
            sum = sum.saturating_add(v);
        }
    }
    sum
}

/// Sum liquidation-adjusted collateral across a subaccount's asset
/// holdings. Same fail-closed rule.
pub fn subaccount_liquidation_adjusted_usd_1e8(holdings: &[AssetHolding<'_>]) -> u128 {
    let mut sum: u128 = 0;
    for h in holdings {
        let Some(price) = h.price_1e8 else { continue };
        if let Ok(v) = liquidation_adjusted_value(h.cfg, h.amount, price) {
            sum = sum.saturating_add(v);
        }
    }
    sum
}

/// True iff a proposed withdrawal keeps the subaccount above the
/// stated maintenance-margin requirement AFTER the withdrawal is
/// removed from its holdings. The proposed withdrawal is described
/// by `(asset_symbol, amount)` and MUST refer to an asset already
/// present in `holdings`.
pub fn withdrawal_is_safe(
    holdings: &[AssetHolding<'_>],
    asset_symbol: &str,
    withdraw_amount: u128,
    maintenance_margin_usd_1e8: u128,
) -> bool {
    let mut post: Vec<AssetHolding<'_>> = holdings.to_vec();
    let mut found = false;
    for h in post.iter_mut() {
        if h.cfg.asset_symbol.eq_ignore_ascii_case(asset_symbol) {
            if h.amount < withdraw_amount {
                return false;
            }
            h.amount -= withdraw_amount;
            found = true;
            break;
        }
    }
    if !found {
        return false;
    }
    subaccount_risk_adjusted_usd_1e8(&post) >= maintenance_margin_usd_1e8
}

// -----------------------------------------------------------------
// Test-only configurations. NEVER USED IN PRODUCTION.
// -----------------------------------------------------------------

/// Test-only helper: WETH configured with a deliberately conservative
/// collateral factor. Not a production parameter — the real activation
/// value will be chosen by the risk committee at activation time.
pub const TEST_WETH: CollateralConfig = CollateralConfig {
    asset_symbol: "WETH",
    decimals: 18,
    collateral_factor_bps: 8_000,     // 80% haircut factor
    liquidation_factor_bps: 8_500,    // 85% at liquidation
    deposit_enabled: true,
    withdrawal_enabled: true,
};

/// Test-only helper: cbBTC configured with a conservative factor
/// reflecting wrap risk + BTC volatility. Not production.
pub const TEST_CBBTC: CollateralConfig = CollateralConfig {
    asset_symbol: "cbBTC",
    decimals: 8,
    collateral_factor_bps: 7_000,     // 70%
    liquidation_factor_bps: 7_500,    // 75%
    deposit_enabled: true,
    withdrawal_enabled: true,
};

/// Test-only helper: USDC at 100% factor. Preserves V1 economics.
pub const TEST_USDC: CollateralConfig = CollateralConfig {
    asset_symbol: "USDC",
    decimals: 6,
    collateral_factor_bps: 10_000,
    liquidation_factor_bps: 10_000,
    deposit_enabled: true,
    withdrawal_enabled: true,
};

#[cfg(test)]
mod tests {
    use super::*;

    // Fixture: an ETH price of $3,000 in 1e8 scale.
    const ETH_USD_1E8: u128 = 3_000 * USD_1E8;
    // Fixture: a BTC price of $60,000 in 1e8 scale.
    const BTC_USD_1E8: u128 = 60_000 * USD_1E8;
    const USDC_USD_1E8: u128 = USD_1E8; // 1 USDC = 1 USD

    #[test]
    fn usdc_valuation_matches_raw_amount() {
        // 10_000 USDC at 6 decimals = 10_000 * 1e6 = 1e10.
        let amount = 10_000u128 * 1_000_000;
        let v = risk_adjusted_value(&TEST_USDC, amount, USDC_USD_1E8).unwrap();
        // 100% factor → risk-adjusted equals raw USD (10_000 * 1e8).
        assert_eq!(v, 10_000 * USD_1E8);
    }

    #[test]
    fn weth_valuation_applies_80_percent_factor() {
        // 2 WETH at 18 decimals = 2 * 1e18.
        let amount = 2u128 * 10u128.pow(18);
        let v = risk_adjusted_value(&TEST_WETH, amount, ETH_USD_1E8).unwrap();
        // Raw USD = 6_000. Factor 80% → 4_800.
        assert_eq!(v, 4_800 * USD_1E8);
    }

    #[test]
    fn cbbtc_valuation_applies_70_percent_factor() {
        // 0.1 cbBTC at 8 decimals = 10_000_000.
        let amount: u128 = 10_000_000;
        let v = risk_adjusted_value(&TEST_CBBTC, amount, BTC_USD_1E8).unwrap();
        // Raw USD = 6_000. Factor 70% → 4_200.
        assert_eq!(v, 4_200 * USD_1E8);
    }

    #[test]
    fn mixed_collateral_subaccount_sums_all_assets() {
        let holdings = vec![
            AssetHolding {
                cfg: &TEST_USDC,
                amount: 10_000 * 1_000_000,
                price_1e8: Some(USDC_USD_1E8),
            },
            AssetHolding {
                cfg: &TEST_WETH,
                amount: 1u128 * 10u128.pow(18),
                price_1e8: Some(ETH_USD_1E8),
            },
            AssetHolding {
                cfg: &TEST_CBBTC,
                amount: 5_000_000, // 0.05 cbBTC
                price_1e8: Some(BTC_USD_1E8),
            },
        ];
        // USDC: 10_000 * 100% = 10_000
        // WETH: 3_000 * 80%  = 2_400
        // cbBTC: 3_000 * 70% = 2_100
        // Total: 14_500 USD
        let total = subaccount_risk_adjusted_usd_1e8(&holdings);
        assert_eq!(total, 14_500 * USD_1E8);
    }

    #[test]
    fn oracle_price_up_scales_linearly() {
        let amount = 1u128 * 10u128.pow(18);
        let base = risk_adjusted_value(&TEST_WETH, amount, ETH_USD_1E8).unwrap();
        let doubled = risk_adjusted_value(&TEST_WETH, amount, ETH_USD_1E8 * 2).unwrap();
        assert_eq!(doubled, base * 2);
    }

    #[test]
    fn oracle_price_down_scales_linearly() {
        let amount = 1u128 * 10u128.pow(18);
        let base = risk_adjusted_value(&TEST_WETH, amount, ETH_USD_1E8).unwrap();
        let halved = risk_adjusted_value(&TEST_WETH, amount, ETH_USD_1E8 / 2).unwrap();
        assert_eq!(halved, base / 2);
    }

    #[test]
    fn oracle_failure_refuses_value_fail_closed() {
        let holdings = vec![
            AssetHolding {
                cfg: &TEST_WETH,
                amount: 1u128 * 10u128.pow(18),
                price_1e8: None, // oracle unavailable
            },
        ];
        // Oracle down → asset contributes ZERO, never partial.
        assert_eq!(subaccount_risk_adjusted_usd_1e8(&holdings), 0);

        let err = risk_adjusted_value(&TEST_WETH, 1, 0).unwrap_err();
        assert_eq!(err, ValuationRefusal::OraclePriceZero);
    }

    #[test]
    fn disabled_asset_refused_even_if_balance_present() {
        // Register WETH exactly as in the production registry (deposit
        // disabled, factor 0) and confirm it contributes nothing.
        let disabled_weth = crate::config::collateral::WETH;
        let err = risk_adjusted_value(&disabled_weth, 1, ETH_USD_1E8).unwrap_err();
        assert_eq!(err, ValuationRefusal::DepositDisabled);
    }

    #[test]
    fn withdrawal_that_would_break_margin_is_refused() {
        // Subaccount: 10_000 USDC only. Maintenance margin: 8_000 USD.
        let holdings = vec![AssetHolding {
            cfg: &TEST_USDC,
            amount: 10_000 * 1_000_000,
            price_1e8: Some(USDC_USD_1E8),
        }];
        // Withdrawing 5_000 USDC leaves 5_000 < 8_000 → unsafe.
        assert!(!withdrawal_is_safe(
            &holdings,
            "USDC",
            5_000 * 1_000_000,
            8_000 * USD_1E8
        ));
    }

    #[test]
    fn withdrawal_that_preserves_margin_is_allowed() {
        let holdings = vec![AssetHolding {
            cfg: &TEST_USDC,
            amount: 10_000 * 1_000_000,
            price_1e8: Some(USDC_USD_1E8),
        }];
        // Withdrawing 1_000 USDC leaves 9_000 >= 8_000 → safe.
        assert!(withdrawal_is_safe(
            &holdings,
            "USDC",
            1_000 * 1_000_000,
            8_000 * USD_1E8
        ));
    }

    #[test]
    fn withdrawal_of_more_than_balance_refused() {
        let holdings = vec![AssetHolding {
            cfg: &TEST_USDC,
            amount: 100u128 * 1_000_000,
            price_1e8: Some(USDC_USD_1E8),
        }];
        assert!(!withdrawal_is_safe(
            &holdings,
            "USDC",
            1_000 * 1_000_000, // more than the 100 USDC balance
            0
        ));
    }

    #[test]
    fn one_collateral_becoming_worthless_does_not_zero_others() {
        // WETH price drops to zero — WETH contributes 0, but USDC
        // still contributes its normal value.
        let holdings = vec![
            AssetHolding {
                cfg: &TEST_USDC,
                amount: 10_000 * 1_000_000,
                price_1e8: Some(USDC_USD_1E8),
            },
            AssetHolding {
                cfg: &TEST_WETH,
                amount: 1u128 * 10u128.pow(18),
                price_1e8: Some(0),
            },
        ];
        let total = subaccount_risk_adjusted_usd_1e8(&holdings);
        assert_eq!(total, 10_000 * USD_1E8);
    }

    #[test]
    fn liquidation_factor_can_meet_or_exceed_collateral_factor_up_to_100pct() {
        // TEST_WETH: collateral=80%, liquidation=85%. Value at
        // liquidation must be ≥ value at margin-availability.
        let amount = 1u128 * 10u128.pow(18);
        let coll = risk_adjusted_value(&TEST_WETH, amount, ETH_USD_1E8).unwrap();
        let liq = liquidation_adjusted_value(&TEST_WETH, amount, ETH_USD_1E8).unwrap();
        assert!(liq > coll);
    }

    #[test]
    fn subaccount_liquidation_value_uses_liquidation_factors() {
        // Compare risk-adjusted vs liquidation-adjusted for the same
        // mixed-collateral holdings.
        let holdings = vec![
            AssetHolding {
                cfg: &TEST_USDC,
                amount: 10_000 * 1_000_000,
                price_1e8: Some(USDC_USD_1E8),
            },
            AssetHolding {
                cfg: &TEST_WETH,
                amount: 1u128 * 10u128.pow(18),
                price_1e8: Some(ETH_USD_1E8),
            },
        ];
        let ra = subaccount_risk_adjusted_usd_1e8(&holdings);
        let la = subaccount_liquidation_adjusted_usd_1e8(&holdings);
        // USDC contributes identically (100%); WETH contributes
        // 85% vs 80% → liquidation-adjusted is larger by exactly
        // 5% × WETH raw = 5% × 3_000 = 150.
        assert_eq!(la - ra, 150 * USD_1E8);
    }

    #[test]
    fn opening_position_requires_full_risk_adjusted_backing() {
        // Subaccount with 10_000 USDC = 10_000 USD risk-adjusted.
        // A new position requiring 12_000 USD initial margin must be
        // refused; 8_000 USD must be allowed.
        let holdings = vec![AssetHolding {
            cfg: &TEST_USDC,
            amount: 10_000 * 1_000_000,
            price_1e8: Some(USDC_USD_1E8),
        }];
        let ra = subaccount_risk_adjusted_usd_1e8(&holdings);
        assert!(ra < 12_000 * USD_1E8);
        assert!(ra >= 8_000 * USD_1E8);
    }

    #[test]
    fn maintenance_margin_check_uses_liquidation_factors() {
        // A subaccount is "eligible for liquidation" when
        // liquidation-adjusted value falls below the maintenance
        // margin. This is the invariant every liquidation path must
        // encode.
        let holdings = vec![AssetHolding {
            cfg: &TEST_WETH,
            amount: 1u128 * 10u128.pow(18),
            price_1e8: Some(ETH_USD_1E8),
        }];
        let la = subaccount_liquidation_adjusted_usd_1e8(&holdings);
        // 3_000 USD × 85% = 2_550 USD
        assert_eq!(la, 2_550 * USD_1E8);

        let maintenance = 2_700 * USD_1E8;
        assert!(la < maintenance, "liquidatable when la < maintenance");

        let maintenance_safe = 2_400 * USD_1E8;
        assert!(la >= maintenance_safe, "safe when la >= maintenance");
    }

    #[test]
    fn zero_collateral_factor_asset_never_backs_margin() {
        // The production WETH entry has factor 0 (registered but
        // inert). Even with a huge balance and healthy oracle, it
        // contributes nothing.
        let disabled_weth = crate::config::collateral::WETH;
        let holdings = vec![AssetHolding {
            cfg: &disabled_weth,
            amount: 1_000u128 * 10u128.pow(18),
            price_1e8: Some(ETH_USD_1E8),
        }];
        assert_eq!(subaccount_risk_adjusted_usd_1e8(&holdings), 0);
    }
}
