//! V2G-A: off-chain tier-eligibility resolver.
//!
//! The canonical V2G launch fee schedule (see `schedule.rs` and
//! `docs/TIER_MERKLE_REBATE_SYSTEM_V2G_A.md`) supports five tiers
//! per product, with **OR-based eligibility** across three independent
//! signals:
//!
//! - 28-day taker+maker volume in `1e8` units of the venue's reference
//!   notional (USD-pegged at launch);
//! - 28-day share of total venue volume in **ppm** (parts-per-million);
//! - currently-staked DEOPT in `1e8`-scaled token units.
//!
//! A trader qualifies for a tier `T` if **any** of the three thresholds
//! at tier `T` is met. The highest qualifying tier wins. When no
//! threshold is met the trader is assigned Tier0 (the default
//! non-discounted profile). This matches the on-chain
//! `FeesManagerV2.currentTier()` semantics: the off-chain Merkle
//! pipeline computes the eligible tier and the trader claims it via
//! `claimTier`; only the claimed tier governs fees once the proof
//! verifies.
//!
//! Eligibility is computed off-chain by design (NEXT_TASK.md V2G-A
//! Part 3 / Phase 3): volume, share, and stake all require historical
//! data or external token-balance reads that have no place on the hot
//! path of `MarginEngineV2` / `PerpEngineV2`.

use super::schedule::{launch_fee_schedule, FeeProduct, FeeTier, MICRO_BPS_PER_PPM};

/// Trader signals fed into [`resolve_tier_with_eligibility`].
///
/// Units:
/// - `volume_28d_1e8`: 28-day volume in `1e8`-scaled venue notional.
/// - `volume_share_ppm`: 28-day share of venue volume in **ppm**
///   (1 % = 10_000 ppm; 0.25 % = 2_500 ppm).
/// - `staked_deopt_1e8`: staked DEOPT in `1e8`-scaled token units.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EligibilityInputs {
    pub volume_28d_1e8: u128,
    pub volume_share_ppm: u32,
    pub staked_deopt_1e8: u128,
}

impl EligibilityInputs {
    pub const ZERO: Self = Self {
        volume_28d_1e8: 0,
        volume_share_ppm: 0,
        staked_deopt_1e8: 0,
    };
}

/// Resolve the highest qualifying tier for a trader against the launch
/// schedule for `product`. OR-logic semantics: a tier is reached if
/// **any** of its three thresholds is met. Tier ordering follows the
/// schedule's natural high-to-low ordering (Tier 4 → 3 → 2 → 1 → 0) so
/// the first match is also the highest qualifying tier.
///
/// Returns the resolved tier number (0..=4); Tier 0 is the unconditional
/// fallback when no threshold is met.
pub fn resolve_tier_with_eligibility(product: FeeProduct, inputs: EligibilityInputs) -> u8 {
    let schedule = launch_fee_schedule();
    let tiers: &[FeeTier] = match product {
        FeeProduct::PerpOrderbook | FeeProduct::PerpRfq => &schedule.perp,
        FeeProduct::OptionOrderbook | FeeProduct::OptionRfq => &schedule.option,
    };
    for tier in tiers {
        if qualifies(tier, inputs) {
            return tier.tier;
        }
    }
    // Schedule always contains a Tier 0 fallback (see canonical-schedule
    // tests). Reaching here implies the schedule was misconfigured.
    0
}

/// True if **any** of the three thresholds on this tier is met by the
/// inputs. Thresholds of zero are treated as "no minimum on this axis"
/// and only qualify when the inputs are also zero (Tier 0).
fn qualifies(tier: &FeeTier, inputs: EligibilityInputs) -> bool {
    let share_micro_bps_from_ppm = u64::from(inputs.volume_share_ppm) * MICRO_BPS_PER_PPM;
    inputs.volume_28d_1e8 >= tier.min_28d_volume_1e8 && tier.min_28d_volume_1e8 != 0
        || share_micro_bps_from_ppm >= tier.min_volume_share_micro_bps
            && tier.min_volume_share_micro_bps != 0
        || inputs.staked_deopt_1e8 >= tier.min_staked_deopt_1e8 && tier.min_staked_deopt_1e8 != 0
        || (tier.min_28d_volume_1e8 == 0
            && tier.min_volume_share_micro_bps == 0
            && tier.min_staked_deopt_1e8 == 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ONE_1E8: u128 = 100_000_000;
    const ONE_DEOPT_1E8: u128 = ONE_1E8;

    fn vol(usd: u128) -> u128 {
        usd * ONE_1E8
    }

    fn stake(deopt: u128) -> u128 {
        deopt * ONE_DEOPT_1E8
    }

    /// V2G-A boundary: every canonical volume threshold exactly hits
    /// the tier it names — Tier1 at $500k, Tier2 at $2.5M, Tier3 at
    /// $10M, Tier4 at $25M. Holds for both OPTION and PERP because
    /// the volume schedule is identical across products.
    #[test]
    fn exact_volume_boundaries_qualify_each_tier() {
        let cases: [(u128, u8); 4] = [
            (vol(500_000), 1),
            (vol(2_500_000), 2),
            (vol(10_000_000), 3),
            (vol(25_000_000), 4),
        ];
        for (volume_28d_1e8, expected_tier) in cases {
            let inputs = EligibilityInputs {
                volume_28d_1e8,
                volume_share_ppm: 0,
                staked_deopt_1e8: 0,
            };
            assert_eq!(
                resolve_tier_with_eligibility(FeeProduct::OptionOrderbook, inputs),
                expected_tier,
                "OPTION at ${} 1e8 should qualify Tier{expected_tier}",
                volume_28d_1e8 / ONE_1E8
            );
            assert_eq!(
                resolve_tier_with_eligibility(FeeProduct::PerpOrderbook, inputs),
                expected_tier,
                "PERP at ${} 1e8 should qualify Tier{expected_tier}",
                volume_28d_1e8 / ONE_1E8
            );
        }
    }

    /// V2G-A boundary: every canonical volume-share threshold (in ppm)
    /// exactly hits the tier it names — Tier1 at 0.25 %, Tier2 at 1 %,
    /// Tier3 at 2.5 %, Tier4 at 5 %.
    #[test]
    fn exact_share_boundaries_qualify_each_tier() {
        let cases: [(u32, u8); 4] = [(2_500, 1), (10_000, 2), (25_000, 3), (50_000, 4)];
        for (volume_share_ppm, expected_tier) in cases {
            let inputs = EligibilityInputs {
                volume_28d_1e8: 0,
                volume_share_ppm,
                staked_deopt_1e8: 0,
            };
            assert_eq!(
                resolve_tier_with_eligibility(FeeProduct::OptionOrderbook, inputs),
                expected_tier,
                "OPTION at {volume_share_ppm} ppm share → Tier{expected_tier}"
            );
            assert_eq!(
                resolve_tier_with_eligibility(FeeProduct::PerpOrderbook, inputs),
                expected_tier,
                "PERP at {volume_share_ppm} ppm share → Tier{expected_tier}"
            );
        }
    }

    /// V2G-A boundary: every canonical staked-DEOPT threshold exactly
    /// hits the tier it names — Tier1 at 10k, Tier2 at 50k, Tier3 at
    /// 100k, Tier4 at 250k.
    #[test]
    fn exact_stake_boundaries_qualify_each_tier() {
        let cases: [(u128, u8); 4] = [
            (stake(10_000), 1),
            (stake(50_000), 2),
            (stake(100_000), 3),
            (stake(250_000), 4),
        ];
        for (staked_deopt_1e8, expected_tier) in cases {
            let inputs = EligibilityInputs {
                volume_28d_1e8: 0,
                volume_share_ppm: 0,
                staked_deopt_1e8,
            };
            assert_eq!(
                resolve_tier_with_eligibility(FeeProduct::OptionOrderbook, inputs),
                expected_tier,
                "OPTION at {} DEOPT stake → Tier{expected_tier}",
                staked_deopt_1e8 / ONE_DEOPT_1E8
            );
            assert_eq!(
                resolve_tier_with_eligibility(FeeProduct::PerpOrderbook, inputs),
                expected_tier,
                "PERP at {} DEOPT stake → Tier{expected_tier}",
                staked_deopt_1e8 / ONE_DEOPT_1E8
            );
        }
    }

    /// V2G-A: when only the volume axis is below Tier1 ($499_999), the
    /// trader falls through to Tier0 regardless of how close they are.
    /// Same for share at 2_499 ppm and stake at 9_999 DEOPT.
    #[test]
    fn just_below_tier1_thresholds_falls_back_to_tier0() {
        let cases: [EligibilityInputs; 3] = [
            EligibilityInputs {
                volume_28d_1e8: vol(499_999),
                volume_share_ppm: 0,
                staked_deopt_1e8: 0,
            },
            EligibilityInputs {
                volume_28d_1e8: 0,
                volume_share_ppm: 2_499,
                staked_deopt_1e8: 0,
            },
            EligibilityInputs {
                volume_28d_1e8: 0,
                volume_share_ppm: 0,
                staked_deopt_1e8: stake(9_999),
            },
        ];
        for inputs in cases {
            assert_eq!(
                resolve_tier_with_eligibility(FeeProduct::OptionOrderbook, inputs),
                0
            );
            assert_eq!(
                resolve_tier_with_eligibility(FeeProduct::PerpOrderbook, inputs),
                0
            );
        }
    }

    /// V2G-A OR semantics: a trader meeting **just one** of the three
    /// thresholds at Tier3 qualifies for Tier3 even when the other two
    /// axes are zero. Each axis is exercised independently.
    #[test]
    fn or_logic_qualifies_when_any_single_axis_meets_threshold() {
        let only_volume = EligibilityInputs {
            volume_28d_1e8: vol(10_000_000),
            volume_share_ppm: 0,
            staked_deopt_1e8: 0,
        };
        let only_share = EligibilityInputs {
            volume_28d_1e8: 0,
            volume_share_ppm: 25_000, // 2.5 % in ppm
            staked_deopt_1e8: 0,
        };
        let only_stake = EligibilityInputs {
            volume_28d_1e8: 0,
            volume_share_ppm: 0,
            staked_deopt_1e8: stake(100_000),
        };
        for inputs in [only_volume, only_share, only_stake] {
            assert_eq!(
                resolve_tier_with_eligibility(FeeProduct::OptionOrderbook, inputs),
                3
            );
            assert_eq!(
                resolve_tier_with_eligibility(FeeProduct::PerpOrderbook, inputs),
                3
            );
        }
    }

    /// V2G-A: when a trader is eligible by multiple axes that point at
    /// different tiers, the highest qualifying tier wins. Here volume
    /// would only qualify Tier2 but share qualifies Tier4 — Tier4 wins.
    #[test]
    fn highest_qualifying_tier_wins_when_multiple_axes_match() {
        let inputs = EligibilityInputs {
            volume_28d_1e8: vol(2_500_000),  // would be Tier2 alone
            volume_share_ppm: 50_000,        // qualifies Tier4 (5 %)
            staked_deopt_1e8: stake(10_000), // would be Tier1 alone
        };
        assert_eq!(
            resolve_tier_with_eligibility(FeeProduct::OptionOrderbook, inputs),
            4
        );
        assert_eq!(
            resolve_tier_with_eligibility(FeeProduct::PerpOrderbook, inputs),
            4
        );
    }

    /// V2G-A: zero on every axis must resolve to Tier 0 (the
    /// unconditional fallback) for both products and both flows.
    #[test]
    fn zero_inputs_resolve_to_tier_zero() {
        for product in [
            FeeProduct::OptionOrderbook,
            FeeProduct::OptionRfq,
            FeeProduct::PerpOrderbook,
            FeeProduct::PerpRfq,
        ] {
            assert_eq!(
                resolve_tier_with_eligibility(product, EligibilityInputs::ZERO),
                0
            );
        }
    }

    /// V2G-A: a trader hovering one wei (one 1e8 unit) above the
    /// Tier3 boundary qualifies Tier3, not Tier4 — guards against
    /// off-by-one in the threshold inequality (`>=` vs `>`).
    #[test]
    fn one_unit_above_tier3_volume_does_not_promote_to_tier4() {
        let inputs = EligibilityInputs {
            volume_28d_1e8: vol(10_000_000) + 1,
            volume_share_ppm: 0,
            staked_deopt_1e8: 0,
        };
        assert_eq!(
            resolve_tier_with_eligibility(FeeProduct::OptionOrderbook, inputs),
            3
        );
        assert_eq!(
            resolve_tier_with_eligibility(FeeProduct::PerpOrderbook, inputs),
            3
        );
    }
}
