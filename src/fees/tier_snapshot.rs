//! V2G-A: deterministic tier snapshot generation.
//!
//! A tier snapshot is the off-chain artifact the operator hands to the
//! Merkle pipeline. Each row carries:
//!
//! - the trader address;
//! - the three signals fed into the OR eligibility resolver
//!   ([`tier_eligibility::EligibilityInputs`]) — 28-day OPTION and PERP
//!   volume separately (so the off-chain tooling can report each
//!   product's contribution to the venue total), 28-day venue volume
//!   share in ppm, and staked DEOPT;
//! - the *resolved* OPTION and PERP tier numbers;
//! - the canonical fee profile that those tiers map to (signed maker
//!   ppm, taker ppm, and OPTION RFQ discount bps), for human review
//!   and dashboards — these fields are **observability only** and are
//!   not consumed on-chain;
//! - `valid_from` / `valid_until` (UNIX seconds) — the same window the
//!   Merkle root will be set with on `FeesManagerV2.setMerkleRoot`.
//!
//! The leaf hash that goes into the Merkle tree is computed
//! separately from the snapshot row by
//! `tier_merkle::tier_leaf(...)`; this module only handles the
//! human-readable snapshot artifact. Determinism: rows are sorted by
//! trader address ascending so two runs of
//! [`generate_tier_snapshot`] over the same inputs produce the same
//! ordering (and therefore the same Merkle root downstream).
//!
//! See `docs/TIER_SNAPSHOT_SCHEMA_V2G_A.md` for the row schema and a
//! worked example.

use serde::{Deserialize, Serialize};

use super::schedule::{launch_tier, FeeProduct, FeeTier, BPS_PER_PCT, MICRO_BPS_PER_PPM};
use super::tier_eligibility::{resolve_tier_with_eligibility, EligibilityInputs};

/// Inputs to a single trader's snapshot row. Aggregated upstream
/// (e.g. from the persisted fee ledger or an external snapshot job);
/// this module does not query any data source.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TraderInputs {
    pub account: [u8; 20],
    /// 28-day OPTION venue volume contribution in `1e8`-scaled units.
    pub option_volume_28d_1e8: u128,
    /// 28-day PERP venue volume contribution in `1e8`-scaled units.
    pub perp_volume_28d_1e8: u128,
    /// 28-day venue share in ppm. Computed off-chain from this trader's
    /// total volume divided by the venue total.
    pub volume_share_ppm: u32,
    /// Currently-staked DEOPT in `1e8`-scaled token units.
    pub staked_deopt_1e8: u128,
}

/// Window the snapshot is valid for. Maps 1:1 to `validFrom` /
/// `validUntil` on `FeesManagerV2.setMerkleRoot`. Operator-provided.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SnapshotConfig {
    pub valid_from: u64,
    pub valid_until: u64,
}

/// One snapshot row. Field naming mirrors the schema documented in
/// `docs/TIER_SNAPSHOT_SCHEMA_V2G_A.md` so the JSON artifact is
/// self-describing.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TierSnapshotRow {
    /// 0x-prefixed lowercase hex address.
    pub trader: String,
    /// 28-day OPTION volume in `1e8`-scaled units, stringified for
    /// stable JSON serialisation.
    pub option_28d_volume_1e8: String,
    pub perp_28d_volume_1e8: String,
    pub total_28d_volume_1e8: String,
    pub volume_share_ppm: u32,
    pub staked_deopt_1e8: String,
    pub option_tier: u8,
    pub perp_tier: u8,
    /// Signed PERP maker ppm: negative for rebate tiers.
    pub perp_maker_ppm: i32,
    pub perp_taker_ppm: u32,
    /// Signed OPTION maker ppm: negative for rebate tiers.
    pub option_maker_ppm: i32,
    pub option_taker_ppm: u32,
    pub option_rfq_maker_discount_bps: u32,
    pub option_rfq_taker_discount_bps: u32,
    pub valid_from: u64,
    pub valid_until: u64,
}

/// Generate a deterministic tier snapshot from the provided per-trader
/// inputs. Rows are sorted by trader address ascending so the Merkle
/// root downstream is stable across runs with the same input set.
pub fn generate_tier_snapshot(
    inputs: &[TraderInputs],
    config: SnapshotConfig,
) -> Vec<TierSnapshotRow> {
    let mut rows: Vec<TierSnapshotRow> = inputs
        .iter()
        .map(|input| snapshot_row_for(input, config))
        .collect();
    rows.sort_by(|left, right| left.trader.cmp(&right.trader));
    rows
}

fn snapshot_row_for(input: &TraderInputs, config: SnapshotConfig) -> TierSnapshotRow {
    let total_28d_volume_1e8 = input
        .option_volume_28d_1e8
        .saturating_add(input.perp_volume_28d_1e8);
    let eligibility = EligibilityInputs {
        volume_28d_1e8: total_28d_volume_1e8,
        volume_share_ppm: input.volume_share_ppm,
        staked_deopt_1e8: input.staked_deopt_1e8,
    };
    let option_tier = resolve_tier_with_eligibility(FeeProduct::OptionOrderbook, eligibility);
    let perp_tier = resolve_tier_with_eligibility(FeeProduct::PerpOrderbook, eligibility);
    let option_profile = launch_tier(FeeProduct::OptionOrderbook, option_tier);
    let perp_profile = launch_tier(FeeProduct::PerpOrderbook, perp_tier);

    TierSnapshotRow {
        trader: format_address(&input.account),
        option_28d_volume_1e8: input.option_volume_28d_1e8.to_string(),
        perp_28d_volume_1e8: input.perp_volume_28d_1e8.to_string(),
        total_28d_volume_1e8: total_28d_volume_1e8.to_string(),
        volume_share_ppm: input.volume_share_ppm,
        staked_deopt_1e8: input.staked_deopt_1e8.to_string(),
        option_tier,
        perp_tier,
        option_maker_ppm: signed_maker_ppm(&option_profile),
        option_taker_ppm: positive_taker_ppm(&option_profile),
        perp_maker_ppm: signed_maker_ppm(&perp_profile),
        perp_taker_ppm: positive_taker_ppm(&perp_profile),
        option_rfq_maker_discount_bps: u32::try_from(
            option_profile.rfq_maker_discount_pct * BPS_PER_PCT,
        )
        .unwrap_or(u32::MAX),
        option_rfq_taker_discount_bps: u32::try_from(
            option_profile.rfq_taker_discount_pct * BPS_PER_PCT,
        )
        .unwrap_or(u32::MAX),
        valid_from: config.valid_from,
        valid_until: config.valid_until,
    }
}

fn signed_maker_ppm(profile: &FeeTier) -> i32 {
    if profile.maker_rebate_micro_bps > 0 {
        // Rebate tier: emit a negative ppm so the JSON artifact matches
        // the canonical table's "maker = -50 ppm" notation.
        -i32::try_from(profile.maker_rebate_micro_bps / MICRO_BPS_PER_PPM).unwrap_or(0)
    } else {
        i32::try_from(profile.maker_fee_micro_bps / MICRO_BPS_PER_PPM).unwrap_or(0)
    }
}

fn positive_taker_ppm(profile: &FeeTier) -> u32 {
    u32::try_from(profile.taker_fee_micro_bps / MICRO_BPS_PER_PPM).unwrap_or(0)
}

fn format_address(account: &[u8; 20]) -> String {
    let mut out = String::with_capacity(2 + 40);
    out.push_str("0x");
    for byte in account {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const ONE_1E8: u128 = 100_000_000;
    const SECONDS_PER_DAY: u64 = 86_400;

    fn addr(byte: u8) -> [u8; 20] {
        let mut buf = [0u8; 20];
        buf[19] = byte;
        buf
    }

    fn config() -> SnapshotConfig {
        SnapshotConfig {
            valid_from: 1_700_000_000,
            valid_until: 1_700_000_000 + 7 * SECONDS_PER_DAY,
        }
    }

    fn trader_qualifying_tier_4_via_volume(byte: u8) -> TraderInputs {
        TraderInputs {
            account: addr(byte),
            option_volume_28d_1e8: 12_500_000 * ONE_1E8,
            perp_volume_28d_1e8: 12_500_000 * ONE_1E8,
            volume_share_ppm: 0,
            staked_deopt_1e8: 0,
        }
    }

    fn trader_qualifying_tier_0(byte: u8) -> TraderInputs {
        TraderInputs {
            account: addr(byte),
            option_volume_28d_1e8: 100 * ONE_1E8,
            perp_volume_28d_1e8: 100 * ONE_1E8,
            volume_share_ppm: 0,
            staked_deopt_1e8: 0,
        }
    }

    /// V2G-A: snapshot is sorted by trader address ascending so the
    /// downstream Merkle root is stable across runs with the same set.
    #[test]
    fn snapshot_rows_are_sorted_by_trader_address() {
        let inputs = vec![
            trader_qualifying_tier_4_via_volume(0x42),
            trader_qualifying_tier_0(0x01),
            trader_qualifying_tier_4_via_volume(0x99),
        ];
        let snapshot = generate_tier_snapshot(&inputs, config());
        let sorted: Vec<&str> = snapshot.iter().map(|row| row.trader.as_str()).collect();
        assert_eq!(
            sorted,
            vec![
                "0x0000000000000000000000000000000000000001",
                "0x0000000000000000000000000000000000000042",
                "0x0000000000000000000000000000000000000099",
            ]
        );
    }

    /// V2G-A: a trader whose summed OPTION + PERP volume reaches the
    /// Tier4 threshold ($25M) is bucketed at Tier4 for **both**
    /// products, with the canonical ppm and RFQ discount values
    /// surfaced for observability.
    #[test]
    fn tier_4_via_combined_volume_resolves_both_products_to_tier_4() {
        let input = trader_qualifying_tier_4_via_volume(0xAB);
        let rows = generate_tier_snapshot(&[input], config());
        assert_eq!(rows.len(), 1);
        let row = &rows[0];
        assert_eq!(row.option_tier, 4);
        assert_eq!(row.perp_tier, 4);
        assert_eq!(row.option_maker_ppm, -50);
        assert_eq!(row.option_taker_ppm, 75);
        assert_eq!(row.perp_maker_ppm, -100);
        assert_eq!(row.perp_taker_ppm, 150);
        assert_eq!(row.option_rfq_maker_discount_bps, 10_000);
        assert_eq!(row.option_rfq_taker_discount_bps, 7_500);
    }

    /// V2G-A: a trader who fails all three thresholds resolves to
    /// Tier0 on both products with the canonical Tier0 ppm values.
    #[test]
    fn tier_0_fallback_resolves_canonical_default_profile() {
        let input = trader_qualifying_tier_0(0x07);
        let rows = generate_tier_snapshot(&[input], config());
        let row = &rows[0];
        assert_eq!(row.option_tier, 0);
        assert_eq!(row.perp_tier, 0);
        assert_eq!(row.option_maker_ppm, 50);
        assert_eq!(row.option_taker_ppm, 250);
        assert_eq!(row.perp_maker_ppm, 50);
        assert_eq!(row.perp_taker_ppm, 300);
        assert_eq!(row.option_rfq_maker_discount_bps, 0);
        assert_eq!(row.option_rfq_taker_discount_bps, 0);
    }

    /// V2G-A determinism: two runs over the same inputs produce
    /// byte-identical row sequences.
    #[test]
    fn snapshot_is_deterministic_across_runs() {
        let inputs = vec![
            trader_qualifying_tier_4_via_volume(0xAA),
            trader_qualifying_tier_0(0xBB),
            trader_qualifying_tier_4_via_volume(0xCC),
        ];
        let first = generate_tier_snapshot(&inputs, config());
        let second = generate_tier_snapshot(&inputs, config());
        assert_eq!(first, second);
    }

    /// V2G-A: each row carries the operator-provided `valid_from` /
    /// `valid_until` verbatim so the snapshot JSON and the
    /// `setMerkleRoot` call stay in sync.
    #[test]
    fn each_row_carries_the_snapshot_window() {
        let cfg = SnapshotConfig {
            valid_from: 1_710_000_000,
            valid_until: 1_710_000_000 + 14 * SECONDS_PER_DAY,
        };
        let rows = generate_tier_snapshot(&[trader_qualifying_tier_0(1)], cfg);
        assert_eq!(rows[0].valid_from, cfg.valid_from);
        assert_eq!(rows[0].valid_until, cfg.valid_until);
    }
}
