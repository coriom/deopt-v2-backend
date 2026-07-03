//! PERPS-ISOLATED-MARGIN-POSITION-ENGINE-V1 — pure margin + PnL math.
//!
//! All inputs and outputs are `1e8`-scaled integers unless a comment
//! says otherwise. Every function is total (never panics) and
//! deterministic. No floating point. No calls to `now_ms`.
//!
//! **Isolated margin model.**
//!   * initial margin requirement = `notional / max_leverage`
//!     (rounded up so an under-margined open cannot slip through).
//!   * maintenance margin requirement = `notional * maintenance_bps
//!     / 10_000` (rounded up for the same reason).
//!   * equity = `margin + unrealised_pnl` (signed).
//!   * margin ratio (bps) = `equity * 10_000 / notional`.
//!   * position eligible for liquidation when
//!     `equity <= maintenance_margin_requirement`.
//!
//! **Estimated liquidation price (isolated margin, no funding).**
//!   Long:  liq = entry - (equity - MM) / size
//!   Short: liq = entry + (equity - MM) / size
//! where `equity - MM` is the buffer above maintenance and `size` is
//! the position size in base units. Rearranging from
//! `equity(P) = margin + (P - entry) * size = MM(P)` for a long,
//! treating maintenance as a fixed fraction of notional at the
//! liquidation price yields the closed form used below. This is an
//! **estimate**; the real liquidation engine (deferred) recomputes
//! maintenance at each observed mark.

use crate::perps::positions::{PerpPosition, PerpSide};

pub const PRICE_SCALE_1E8: u128 = 100_000_000;
pub const BPS: u128 = 10_000;

/// Notional = `size * price / 1e8`, still in `1e8` quote units.
pub fn notional_1e8(size_1e8: u128, price_1e8: u128) -> u128 {
    if size_1e8 == 0 || price_1e8 == 0 {
        return 0;
    }
    // size and price are both 1e8; product has 1e16 scale so divide
    // by 1e8 to return to quote 1e8.
    mul_div_floor(size_1e8, price_1e8, PRICE_SCALE_1E8)
}

/// Initial margin requirement for opening `size` at `price` under
/// `max_leverage`. Rounded up so a strict `>=` check on the trader's
/// margin never lets an under-margined open slip through.
pub fn initial_margin_requirement_1e8(size_1e8: u128, price_1e8: u128, max_leverage: u32) -> u128 {
    if max_leverage == 0 {
        return u128::MAX;
    }
    let notional = notional_1e8(size_1e8, price_1e8);
    ceil_div(notional, max_leverage as u128)
}

/// Maintenance margin requirement given `size`, current mark, and
/// maintenance bps. Rounded up.
pub fn maintenance_margin_requirement_1e8(
    size_1e8: u128,
    price_1e8: u128,
    maintenance_bps: u32,
) -> u128 {
    let notional = notional_1e8(size_1e8, price_1e8);
    if maintenance_bps == 0 || notional == 0 {
        return 0;
    }
    ceil_div_u128(notional.saturating_mul(maintenance_bps as u128), BPS)
}

/// Unrealised PnL (signed, `1e8`). Long profits when mark > entry;
/// short profits when mark < entry.
pub fn unrealized_pnl_1e8(position: &PerpPosition, mark_price_1e8: u128) -> i128 {
    if position.size_1e8 == 0 || mark_price_1e8 == 0 {
        return 0;
    }
    let entry = position.entry_price_1e8 as i128;
    let mark = mark_price_1e8 as i128;
    let size = position.size_1e8 as i128;
    let scale = PRICE_SCALE_1E8 as i128;
    let delta_price = match position.side {
        PerpSide::Long => mark.saturating_sub(entry),
        PerpSide::Short => entry.saturating_sub(mark),
    };
    // (delta_price * size) / 1e8 — kept in i128 so signs propagate.
    delta_price.saturating_mul(size) / scale
}

/// Equity = isolated margin + unrealised PnL. Returns i128 because
/// deep unrealised losses can drive equity negative on a paper
/// basis. The service layer typically clamps to zero when surfacing
/// this to the frontend.
pub fn equity_1e8(position: &PerpPosition, mark_price_1e8: u128) -> i128 {
    let margin = position.margin_1e8 as i128;
    margin.saturating_add(unrealized_pnl_1e8(position, mark_price_1e8))
}

/// Realised PnL for reducing `close_size_1e8` of the position at
/// `close_price_1e8`. Signed, `1e8`.
pub fn realized_pnl_1e8(
    entry_price_1e8: u128,
    close_price_1e8: u128,
    close_size_1e8: u128,
    side: PerpSide,
) -> i128 {
    if close_size_1e8 == 0 || close_price_1e8 == 0 || entry_price_1e8 == 0 {
        return 0;
    }
    let entry = entry_price_1e8 as i128;
    let close = close_price_1e8 as i128;
    let size = close_size_1e8 as i128;
    let scale = PRICE_SCALE_1E8 as i128;
    let delta = match side {
        PerpSide::Long => close.saturating_sub(entry),
        PerpSide::Short => entry.saturating_sub(close),
    };
    delta.saturating_mul(size) / scale
}

/// Weighted-average entry price when increasing an existing same-side
/// position by `add_size` at `add_price`. Returns the new WAP.
pub fn weighted_average_entry_1e8(
    existing_size_1e8: u128,
    existing_entry_1e8: u128,
    add_size_1e8: u128,
    add_price_1e8: u128,
) -> u128 {
    let total_size = existing_size_1e8.saturating_add(add_size_1e8);
    if total_size == 0 {
        return 0;
    }
    let existing_notional = existing_size_1e8.saturating_mul(existing_entry_1e8);
    let add_notional = add_size_1e8.saturating_mul(add_price_1e8);
    (existing_notional.saturating_add(add_notional)) / total_size
}

/// Margin ratio in basis points. Uses equity (with unrealised PnL)
/// over notional-at-mark. Returns 0 for a closed / sizeless position
/// and clamps negatives to 0 (the trader is already underwater —
/// surfacing a negative bps to the frontend is noisier than helpful).
pub fn margin_ratio_bps(position: &PerpPosition, mark_price_1e8: u128) -> u128 {
    let notional = notional_1e8(position.size_1e8, mark_price_1e8);
    if notional == 0 {
        return 0;
    }
    let equity = equity_1e8(position, mark_price_1e8);
    if equity <= 0 {
        return 0;
    }
    mul_div_floor(equity as u128, BPS, notional)
}

/// Estimated liquidation price under the isolated-margin, no-funding
/// V1 model. Returns `None` when maintenance is impossible to
/// compute (zero size, non-positive maintenance bps that makes the
/// buffer meaningless). Result is `1e8`-scaled.
///
/// Derivation for a long position at entry E, size S (both `1e8`),
/// margin M (`1e8`):
///
///     equity_at_P   = M + (P - E) * S / 1e8
///     mm_at_P       = P * S * mm_bps / (1e8 * 10_000)
///     liquidation ⟺ equity_at_P == mm_at_P
///
///  →  M + (P - E) * S / 1e8 = P * S * mm_bps / (1e8 * BPS)
///  →  M * 1e8 * BPS + (P - E) * S * BPS = P * S * mm_bps
///  →  M * 1e8 * BPS - E * S * BPS = P * S * mm_bps - P * S * BPS
///  →  M * 1e8 * BPS - E * S * BPS = P * S * (mm_bps - BPS)
///  →  P = [ M * 1e8 * BPS - E * S * BPS ] / [ S * (mm_bps - BPS) ]
///
/// Because `mm_bps < BPS`, the denominator is negative for a long
/// position, so we invert both signs to keep the arithmetic
/// unsigned:
///
///     P = [ E * S * BPS - M * 1e8 * BPS ] / [ S * (BPS - mm_bps) ]
///
/// For a short, the same construction yields:
///
///     P = [ E * S * BPS + M * 1e8 * BPS ] / [ S * (BPS + mm_bps) ]
///
/// The formula assumes maintenance scales linearly with the observed
/// mark — an approximation. A future `PERPS-LIQUIDATION-AND-RISK-V1`
/// milestone will recompute maintenance from the actual on-chain
/// `PerpMarketRegistry` config.
pub fn estimated_liquidation_price_1e8(
    position: &PerpPosition,
    maintenance_bps: u32,
) -> Option<u128> {
    if position.size_1e8 == 0 || position.entry_price_1e8 == 0 {
        return None;
    }
    let mm = maintenance_bps as u128;
    if mm >= BPS {
        return None;
    }
    let size = position.size_1e8;
    let entry = position.entry_price_1e8;
    let margin = position.margin_1e8;
    let entry_notional_bps = entry.checked_mul(size)?.checked_mul(BPS)?;
    let margin_bps = margin.checked_mul(PRICE_SCALE_1E8)?.checked_mul(BPS)?;
    match position.side {
        PerpSide::Long => {
            // Numerator: entry * size * BPS - margin * 1e8 * BPS
            // (positive when position is above liquidation).
            let num = entry_notional_bps.checked_sub(margin_bps)?;
            let denom = size.checked_mul(BPS - mm)?;
            if denom == 0 {
                return None;
            }
            Some(num / denom)
        }
        PerpSide::Short => {
            // Numerator: entry * size * BPS + margin * 1e8 * BPS
            let num = entry_notional_bps.checked_add(margin_bps)?;
            let denom = size.checked_mul(BPS + mm)?;
            if denom == 0 {
                return None;
            }
            Some(num / denom)
        }
    }
}

fn ceil_div(a: u128, b: u128) -> u128 {
    if b == 0 {
        return u128::MAX;
    }
    a.div_ceil(b)
}

fn ceil_div_u128(a: u128, b: u128) -> u128 {
    ceil_div(a, b)
}

fn mul_div_floor(a: u128, b: u128, denom: u128) -> u128 {
    if denom == 0 {
        return 0;
    }
    // Guard against overflow on the product by widening via a u256
    // proxy: for the values we see (prices + sizes bounded by
    // realistic market caps), the product fits u128 comfortably.
    // If it ever overflows we'd rather saturate than panic.
    match a.checked_mul(b) {
        Some(product) => product / denom,
        None => u128::MAX / denom,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::perps::positions::new_position_skeleton;
    use crate::types::AccountId;

    fn long_position(size: u128, entry: u128, margin: u128) -> PerpPosition {
        new_position_skeleton(
            AccountId::new("0x0000000000000000000000000000000000000aaa"),
            "ETH-PERP".to_string(),
            PerpSide::Long,
            size,
            entry,
            margin,
        )
    }

    fn short_position(size: u128, entry: u128, margin: u128) -> PerpPosition {
        new_position_skeleton(
            AccountId::new("0x0000000000000000000000000000000000000aaa"),
            "ETH-PERP".to_string(),
            PerpSide::Short,
            size,
            entry,
            margin,
        )
    }

    #[test]
    fn notional_of_one_contract_at_3000_is_3000() {
        // size = 1e8 (one contract), price = 3000 * 1e8
        // notional_1e8 = 3000 * 1e8 (i.e. $3000 in 1e8 units)
        let n = notional_1e8(100_000_000, 300_000_000_000);
        assert_eq!(n, 300_000_000_000);
    }

    #[test]
    fn initial_margin_ten_x_leverage() {
        let n = initial_margin_requirement_1e8(
            100_000_000,     // 1 contract
            300_000_000_000, // $3000
            10,
        );
        assert_eq!(n, 30_000_000_000); // $300
    }

    #[test]
    fn initial_margin_rounds_up_so_tight_open_cant_slip() {
        // 3 contracts at $101 with leverage 3 → notional 303, IM = 101.
        // Bump size to 3 contracts +1 sat at $101 → notional 303.00000001,
        // IM must be > 101 (round up).
        let n = initial_margin_requirement_1e8(300_000_001, 10_100_000_000, 3);
        assert!(n > 10_100_000_000);
    }

    #[test]
    fn maintenance_margin_five_percent_of_notional() {
        let n = maintenance_margin_requirement_1e8(
            100_000_000,
            300_000_000_000,
            500, // 5%
        );
        assert_eq!(n, 15_000_000_000); // 5% of $3000 = $150
    }

    #[test]
    fn unrealized_pnl_long_positive_when_mark_above_entry() {
        // 1 contract long from $3000, mark $3100 → +$100
        let p = long_position(100_000_000, 300_000_000_000, 30_000_000_000);
        assert_eq!(unrealized_pnl_1e8(&p, 310_000_000_000), 10_000_000_000);
    }

    #[test]
    fn unrealized_pnl_long_negative_when_mark_below_entry() {
        // 1 contract long from $3000, mark $2900 → -$100
        let p = long_position(100_000_000, 300_000_000_000, 30_000_000_000);
        assert_eq!(unrealized_pnl_1e8(&p, 290_000_000_000), -10_000_000_000);
    }

    #[test]
    fn unrealized_pnl_short_positive_when_mark_below_entry() {
        let p = short_position(100_000_000, 300_000_000_000, 30_000_000_000);
        assert_eq!(unrealized_pnl_1e8(&p, 290_000_000_000), 10_000_000_000);
    }

    #[test]
    fn unrealized_pnl_short_negative_when_mark_above_entry() {
        let p = short_position(100_000_000, 300_000_000_000, 30_000_000_000);
        assert_eq!(unrealized_pnl_1e8(&p, 310_000_000_000), -10_000_000_000);
    }

    #[test]
    fn realized_pnl_matches_unrealized_over_full_close() {
        // Long 1 @ 3000 → close 1 @ 3100 → realised = +100
        let realised = realized_pnl_1e8(
            300_000_000_000,
            310_000_000_000,
            100_000_000,
            PerpSide::Long,
        );
        assert_eq!(realised, 10_000_000_000);
    }

    #[test]
    fn weighted_average_entry_across_two_fills() {
        // Existing 1 @ 3000, add 1 @ 3200 → WAP = 3100
        let wap =
            weighted_average_entry_1e8(100_000_000, 300_000_000_000, 100_000_000, 320_000_000_000);
        assert_eq!(wap, 310_000_000_000);
    }

    #[test]
    fn margin_ratio_100pc_when_no_pnl_and_notional_equals_margin() {
        let p = long_position(100_000_000, 100_000_000, 100_000_000);
        // notional = 1, margin = 1 → ratio = 10_000 bps = 100%
        assert_eq!(margin_ratio_bps(&p, 100_000_000), BPS);
    }

    #[test]
    fn margin_ratio_zero_when_equity_underwater() {
        let p = long_position(100_000_000, 300_000_000_000, 10_000_000);
        // margin $0.10, entry $3000, mark $2000 → equity deeply negative.
        assert_eq!(margin_ratio_bps(&p, 200_000_000_000), 0);
    }

    #[test]
    fn liquidation_price_long_below_entry() {
        // Long 1 @ $3000 with margin $300 (10x), maintenance 5%
        // Numerator: 3000 * 1 * BPS - 300 * BPS = (3000 - 300) * BPS ~ 2700 * BPS
        // Denominator: 1 * (BPS - 500) = 9_500
        // → liq ≈ 2700 * BPS / 9500 ≈ 2842.10
        let p = long_position(100_000_000, 300_000_000_000, 30_000_000_000);
        let liq = estimated_liquidation_price_1e8(&p, 500).unwrap();
        assert!(liq < p.entry_price_1e8);
        assert!(liq > 200_000_000_000);
    }

    #[test]
    fn liquidation_price_short_above_entry() {
        let p = short_position(100_000_000, 300_000_000_000, 30_000_000_000);
        let liq = estimated_liquidation_price_1e8(&p, 500).unwrap();
        assert!(liq > p.entry_price_1e8);
    }

    #[test]
    fn liquidation_price_none_for_zero_size() {
        let p = long_position(0, 300_000_000_000, 0);
        assert!(estimated_liquidation_price_1e8(&p, 500).is_none());
    }
}
