use serde::{Deserialize, Serialize};

pub const MICRO_BPS_PER_BPS: u128 = 10_000;
pub const RATE_DENOMINATOR: u128 = 100_000_000;

const ONE_1E8: u128 = 100_000_000;
const VOLUME_500K_1E8: u128 = 500_000 * ONE_1E8;
const VOLUME_2_5M_1E8: u128 = 2_500_000 * ONE_1E8;
const VOLUME_10M_1E8: u128 = 10_000_000 * ONE_1E8;
const VOLUME_25M_1E8: u128 = 25_000_000 * ONE_1E8;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FeeProduct {
    PerpOrderbook,
    PerpRfq,
    OptionOrderbook,
    OptionRfq,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct FeeTier {
    pub tier: u8,
    pub min_28d_volume_1e8: u128,
    pub min_volume_share_micro_bps: u64,
    pub min_staked_deopt_1e8: u128,
    pub maker_fee_micro_bps: u64,
    pub maker_rebate_micro_bps: u64,
    pub taker_fee_micro_bps: u64,
    pub rfq_maker_discount_pct: u64,
    pub rfq_taker_discount_pct: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ResolvedFeeRates {
    pub tier: u8,
    pub maker_fee_micro_bps: u64,
    pub maker_rebate_micro_bps: u64,
    pub taker_fee_micro_bps: u64,
    pub rfq_maker_discount_pct: u64,
    pub rfq_taker_discount_pct: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct LaunchFeeSchedule {
    pub perp: Vec<FeeTier>,
    pub option: Vec<FeeTier>,
}

pub fn launch_fee_schedule() -> LaunchFeeSchedule {
    LaunchFeeSchedule {
        perp: vec![
            FeeTier {
                tier: 4,
                min_28d_volume_1e8: VOLUME_25M_1E8,
                min_volume_share_micro_bps: 5_000_000,
                min_staked_deopt_1e8: 250_000 * ONE_1E8,
                maker_fee_micro_bps: 0,
                maker_rebate_micro_bps: 10_000,
                taker_fee_micro_bps: 15_000,
                rfq_maker_discount_pct: 0,
                rfq_taker_discount_pct: 0,
            },
            FeeTier {
                tier: 3,
                min_28d_volume_1e8: VOLUME_10M_1E8,
                min_volume_share_micro_bps: 2_500_000,
                min_staked_deopt_1e8: 100_000 * ONE_1E8,
                maker_fee_micro_bps: 0,
                maker_rebate_micro_bps: 7_500,
                taker_fee_micro_bps: 17_500,
                rfq_maker_discount_pct: 0,
                rfq_taker_discount_pct: 0,
            },
            FeeTier {
                tier: 2,
                min_28d_volume_1e8: VOLUME_2_5M_1E8,
                min_volume_share_micro_bps: 1_000_000,
                min_staked_deopt_1e8: 50_000 * ONE_1E8,
                maker_fee_micro_bps: 0,
                maker_rebate_micro_bps: 5_000,
                taker_fee_micro_bps: 20_000,
                rfq_maker_discount_pct: 0,
                rfq_taker_discount_pct: 0,
            },
            FeeTier {
                tier: 1,
                min_28d_volume_1e8: VOLUME_500K_1E8,
                min_volume_share_micro_bps: 250_000,
                min_staked_deopt_1e8: 10_000 * ONE_1E8,
                maker_fee_micro_bps: 0,
                maker_rebate_micro_bps: 0,
                taker_fee_micro_bps: 25_000,
                rfq_maker_discount_pct: 0,
                rfq_taker_discount_pct: 0,
            },
            FeeTier {
                tier: 0,
                min_28d_volume_1e8: 0,
                min_volume_share_micro_bps: 0,
                min_staked_deopt_1e8: 0,
                maker_fee_micro_bps: 5_000,
                maker_rebate_micro_bps: 0,
                taker_fee_micro_bps: 30_000,
                rfq_maker_discount_pct: 0,
                rfq_taker_discount_pct: 0,
            },
        ],
        option: vec![
            FeeTier {
                tier: 4,
                min_28d_volume_1e8: VOLUME_25M_1E8,
                min_volume_share_micro_bps: 5_000_000,
                min_staked_deopt_1e8: 250_000 * ONE_1E8,
                maker_fee_micro_bps: 0,
                maker_rebate_micro_bps: 5_000,
                taker_fee_micro_bps: 7_500,
                rfq_maker_discount_pct: 100,
                rfq_taker_discount_pct: 75,
            },
            FeeTier {
                tier: 3,
                min_28d_volume_1e8: VOLUME_10M_1E8,
                min_volume_share_micro_bps: 2_500_000,
                min_staked_deopt_1e8: 100_000 * ONE_1E8,
                maker_fee_micro_bps: 0,
                maker_rebate_micro_bps: 2_500,
                taker_fee_micro_bps: 10_000,
                rfq_maker_discount_pct: 75,
                rfq_taker_discount_pct: 50,
            },
            FeeTier {
                tier: 2,
                min_28d_volume_1e8: VOLUME_2_5M_1E8,
                min_volume_share_micro_bps: 1_000_000,
                min_staked_deopt_1e8: 50_000 * ONE_1E8,
                maker_fee_micro_bps: 0,
                maker_rebate_micro_bps: 1_000,
                taker_fee_micro_bps: 12_500,
                rfq_maker_discount_pct: 50,
                rfq_taker_discount_pct: 25,
            },
            FeeTier {
                tier: 1,
                min_28d_volume_1e8: VOLUME_500K_1E8,
                min_volume_share_micro_bps: 250_000,
                min_staked_deopt_1e8: 10_000 * ONE_1E8,
                maker_fee_micro_bps: 0,
                maker_rebate_micro_bps: 0,
                taker_fee_micro_bps: 15_000,
                rfq_maker_discount_pct: 25,
                rfq_taker_discount_pct: 10,
            },
            FeeTier {
                tier: 0,
                min_28d_volume_1e8: 0,
                min_volume_share_micro_bps: 0,
                min_staked_deopt_1e8: 0,
                maker_fee_micro_bps: 5_000,
                maker_rebate_micro_bps: 0,
                taker_fee_micro_bps: 25_000,
                rfq_maker_discount_pct: 0,
                rfq_taker_discount_pct: 0,
            },
        ],
    }
}

pub fn resolve_rates_from_volume(
    product: FeeProduct,
    rolling_volume_1e8: u128,
) -> ResolvedFeeRates {
    let schedule = launch_fee_schedule();
    let tiers = match product {
        FeeProduct::PerpOrderbook | FeeProduct::PerpRfq => &schedule.perp,
        FeeProduct::OptionOrderbook | FeeProduct::OptionRfq => &schedule.option,
    };
    let tier = tiers
        .iter()
        .find(|tier| rolling_volume_1e8 >= tier.min_28d_volume_1e8)
        .unwrap_or_else(|| tiers.last().expect("launch fee schedule has tier 0"));
    let (maker_fee_micro_bps, taker_fee_micro_bps) = match product {
        FeeProduct::PerpRfq | FeeProduct::OptionRfq => (
            discount_positive_fee(tier.maker_fee_micro_bps, tier.rfq_maker_discount_pct),
            discount_positive_fee(tier.taker_fee_micro_bps, tier.rfq_taker_discount_pct),
        ),
        FeeProduct::PerpOrderbook | FeeProduct::OptionOrderbook => {
            (tier.maker_fee_micro_bps, tier.taker_fee_micro_bps)
        }
    };
    ResolvedFeeRates {
        tier: tier.tier,
        maker_fee_micro_bps,
        maker_rebate_micro_bps: tier.maker_rebate_micro_bps,
        taker_fee_micro_bps,
        rfq_maker_discount_pct: tier.rfq_maker_discount_pct,
        rfq_taker_discount_pct: tier.rfq_taker_discount_pct,
    }
}

fn discount_positive_fee(rate_micro_bps: u64, discount_pct: u64) -> u64 {
    let discount_pct = discount_pct.min(100);
    rate_micro_bps.saturating_mul(100 - discount_pct) / 100
}

/// V2G-A: canonical conversion factor between ppm (parts-per-million,
/// used by `FeesManagerV2` on-chain) and the `micro_bps` units this
/// schedule stores. `1 ppm = 1e-6` and `1 micro_bps = 1e-8`, so a rate
/// of `R ppm` corresponds to `R * MICRO_BPS_PER_PPM = R * 100` micro_bps.
pub const MICRO_BPS_PER_PPM: u64 = 100;

/// V2G-A: canonical conversion factor between bps (basis points, used
/// when stating RFQ discounts in business documents) and pct (the unit
/// the option schedule stores `rfq_*_discount_pct` in). The canonical
/// table specifies "RFQ maker discount = 2_500 bps" for tier 1; that
/// equals 25 % and is stored as `rfq_maker_discount_pct: 25`.
pub const BPS_PER_PCT: u64 = 100;

/// Look up the canonical [`FeeTier`] entry for `(product, tier)` in
/// the launch fee schedule. Panics with a clear message if the tier is
/// unknown — callers in tests pass static, table-driven inputs.
pub fn launch_tier(product: FeeProduct, tier: u8) -> FeeTier {
    let schedule = launch_fee_schedule();
    let tiers = match product {
        FeeProduct::PerpOrderbook | FeeProduct::PerpRfq => schedule.perp,
        FeeProduct::OptionOrderbook | FeeProduct::OptionRfq => schedule.option,
    };
    tiers
        .into_iter()
        .find(|entry| entry.tier == tier)
        .unwrap_or_else(|| panic!("V2G-A launch schedule missing tier {tier} for {product:?}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launch_schedule_uses_micro_bps_rates() {
        let schedule = launch_fee_schedule();
        let option_tier_0 = schedule.option.iter().find(|tier| tier.tier == 0).unwrap();
        let option_tier_4 = schedule.option.iter().find(|tier| tier.tier == 4).unwrap();

        assert_eq!(option_tier_0.maker_fee_micro_bps, 5_000);
        assert_eq!(option_tier_0.taker_fee_micro_bps, 25_000);
        assert_eq!(option_tier_4.maker_rebate_micro_bps, 5_000);
        assert_eq!(option_tier_4.taker_fee_micro_bps, 7_500);
    }

    #[test]
    fn option_rfq_discounts_positive_fees_without_discounting_rebates() {
        let rates = resolve_rates_from_volume(FeeProduct::OptionRfq, VOLUME_25M_1E8);

        assert_eq!(rates.tier, 4);
        assert_eq!(rates.maker_fee_micro_bps, 0);
        assert_eq!(rates.maker_rebate_micro_bps, 5_000);
        assert_eq!(rates.taker_fee_micro_bps, 1_875);
    }

    /// V2G-A: encode the canonical OPTION + PERP fee schedule as
    /// `(tier, maker_ppm_signed, taker_ppm, rfq_maker_discount_bps,
    /// rfq_taker_discount_bps)` and assert the launch schedule's
    /// `micro_bps` storage matches each row exactly. `maker_ppm_signed`
    /// is negative for rebate tiers; it is split into the storage's
    /// `(maker_fee_micro_bps, maker_rebate_micro_bps)` pair.
    ///
    /// This is the cross-check that any drift between the canonical
    /// `NEXT_TASK.md` table and the backend schedule fires here first.
    #[test]
    fn canonical_option_schedule_matches_launch_table() {
        // (tier, maker_ppm_signed, taker_ppm, rfq_maker_discount_bps, rfq_taker_discount_bps)
        let canonical: [(u8, i64, u64, u64, u64); 5] = [
            (4, -50, 75, 10_000, 7_500),
            (3, -25, 100, 7_500, 5_000),
            (2, -10, 125, 5_000, 2_500),
            (1, 0, 150, 2_500, 1_000),
            (0, 50, 250, 0, 0),
        ];
        for (tier, maker_ppm, taker_ppm, rfq_maker_bps, rfq_taker_bps) in canonical {
            let entry = launch_tier(FeeProduct::OptionOrderbook, tier);
            assert_canonical_maker(&entry, tier, "option", maker_ppm);
            assert_eq!(
                entry.taker_fee_micro_bps,
                taker_ppm * MICRO_BPS_PER_PPM,
                "option tier {tier} taker"
            );
            assert_eq!(
                entry.rfq_maker_discount_pct * BPS_PER_PCT,
                rfq_maker_bps,
                "option tier {tier} rfq maker discount"
            );
            assert_eq!(
                entry.rfq_taker_discount_pct * BPS_PER_PCT,
                rfq_taker_bps,
                "option tier {tier} rfq taker discount"
            );
        }
    }

    #[test]
    fn canonical_perp_schedule_matches_launch_table() {
        let canonical: [(u8, i64, u64); 5] = [
            (4, -100, 150),
            (3, -75, 175),
            (2, -50, 200),
            (1, 0, 250),
            (0, 50, 300),
        ];
        for (tier, maker_ppm, taker_ppm) in canonical {
            let entry = launch_tier(FeeProduct::PerpOrderbook, tier);
            assert_canonical_maker(&entry, tier, "perp", maker_ppm);
            assert_eq!(
                entry.taker_fee_micro_bps,
                taker_ppm * MICRO_BPS_PER_PPM,
                "perp tier {tier} taker"
            );
            // Perps have no RFQ discount: both pcts must stay zero on
            // every tier per the canonical PERP table.
            assert_eq!(
                entry.rfq_maker_discount_pct, 0,
                "perp tier {tier} rfq maker"
            );
            assert_eq!(
                entry.rfq_taker_discount_pct, 0,
                "perp tier {tier} rfq taker"
            );
        }
    }

    /// V2G-A: helper for the canonical-table cross-checks. Splits a
    /// signed `maker_ppm` into the backend's `(maker_fee_micro_bps,
    /// maker_rebate_micro_bps)` representation and asserts both halves.
    fn assert_canonical_maker(entry: &FeeTier, tier: u8, product: &str, maker_ppm_signed: i64) {
        if maker_ppm_signed >= 0 {
            assert_eq!(
                entry.maker_fee_micro_bps,
                (maker_ppm_signed as u64) * MICRO_BPS_PER_PPM,
                "{product} tier {tier} maker (positive ppm)"
            );
            assert_eq!(
                entry.maker_rebate_micro_bps, 0,
                "{product} tier {tier} maker rebate (should be zero for non-negative ppm)"
            );
        } else {
            assert_eq!(
                entry.maker_fee_micro_bps, 0,
                "{product} tier {tier} maker fee (should be zero for negative ppm)"
            );
            assert_eq!(
                entry.maker_rebate_micro_bps,
                ((-maker_ppm_signed) as u64) * MICRO_BPS_PER_PPM,
                "{product} tier {tier} maker rebate (absolute value of negative ppm)"
            );
        }
    }

    /// V2G-A: assert the thresholds stored on each tier exactly match
    /// the canonical eligibility table (volume in $1e8, share in ppm,
    /// stake in DEOPT*1e8). Tier0 must be the unconditional fallback.
    #[test]
    fn canonical_eligibility_thresholds_match_launch_table() {
        let schedule = launch_fee_schedule();
        // (tier, min_volume_1e8, min_share_micro_bps, min_staked_deopt_1e8)
        let canonical: [(u8, u128, u64, u128); 5] = [
            (4, VOLUME_25M_1E8, 5_000_000, 250_000 * ONE_1E8),
            (3, VOLUME_10M_1E8, 2_500_000, 100_000 * ONE_1E8),
            (2, VOLUME_2_5M_1E8, 1_000_000, 50_000 * ONE_1E8),
            (1, VOLUME_500K_1E8, 250_000, 10_000 * ONE_1E8),
            (0, 0, 0, 0),
        ];
        for (tier, min_vol, min_share, min_stake) in canonical {
            for product in [FeeProduct::OptionOrderbook, FeeProduct::PerpOrderbook] {
                let entry = launch_tier(product, tier);
                assert_eq!(
                    entry.min_28d_volume_1e8, min_vol,
                    "{product:?} tier {tier} vol"
                );
                assert_eq!(
                    entry.min_volume_share_micro_bps, min_share,
                    "{product:?} tier {tier} share"
                );
                assert_eq!(
                    entry.min_staked_deopt_1e8, min_stake,
                    "{product:?} tier {tier} stake"
                );
            }
        }
        // Tier0 fallback present on both products.
        assert!(schedule.option.iter().any(|tier| tier.tier == 0));
        assert!(schedule.perp.iter().any(|tier| tier.tier == 0));
    }
}
