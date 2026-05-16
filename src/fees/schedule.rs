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
}
