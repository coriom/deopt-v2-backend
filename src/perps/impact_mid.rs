//! PERPS-FULLSTACK-RUNTIME-INTEGRATION-V1 Part B — pure impact-mid math.
//!
//! **Definition.** *Impact price* at notional `N` on one side of the book is
//! the size-weighted average price a taker of exactly `N` (quote) would pay
//! walking the book from the best price outward. *Impact mid* is the
//! midpoint between the ask-side impact price and the bid-side impact price.
//!
//! **Fail-closed guarantees.** The functions in this module NEVER fabricate
//! a price when the book is too shallow. `impact_price` returns `None` if
//! the requested notional cannot be filled by the supplied levels;
//! `impact_mid` returns `Err(InsufficientDepth::*)` variants distinguishing
//! ask-shallowness, bid-shallowness, non-monotonic (best bid >= best ask),
//! and zero-notional configuration bugs.
//!
//! **No IO.** No async, no clock, no network, no logging. Fully
//! unit-testable via literal `Level` slices. The keeper wrapper
//! (`impact_mid_keeper.rs`) is responsible for pulling levels off the
//! `PerpOrderStore` and reconciling with the oracle.
//!
//! **Overflow discipline.** Levels are `u128` scaled by `1e8`. The interior
//! math (`price * size / 1e8`) can reach ~2^80 for realistic $10k–$100M
//! books; we saturate at `u128::MAX` on overflow and treat that as a
//! computation failure (surfaced as `InsufficientDepth::Overflow`) rather
//! than silently wrapping. Silent wrap in a market-reference number would
//! be worse than "unavailable".

use serde::Serialize;

/// Fixed-point scale used across every Perps price and size (1e8).
/// Matches `OracleRouter.PRICE_SCALE`.
const SCALE_1E8: u128 = 100_000_000;

/// One flattened orderbook level. `price_1e8` is the price at which the
/// resting maker will trade; `size_1e8` is the resting base-asset size
/// (post-fill remaining, not total).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Level {
    pub price_1e8: u128,
    pub size_1e8: u128,
}

impl Level {
    pub const fn new(price_1e8: u128, size_1e8: u128) -> Self {
        Self {
            price_1e8,
            size_1e8,
        }
    }
}

/// Structured "why did impact-mid fail" reason. The keeper maps each to
/// an `ImpactMidUnavailable` cache state and (separately) to an
/// observability counter — those are one-to-one so the operator can see
/// which shallowness class is tripping.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InsufficientDepth {
    /// The requested notional (`impact_notional_1e8`) is zero — a
    /// configuration bug, not a market state. The keeper refuses to
    /// enable with `impact_notional_1e8 == 0` at startup, so this
    /// variant SHOULD be unreachable in production; kept explicit so
    /// tests can pin the boundary.
    ZeroNotional,
    /// The ask side cannot absorb the configured notional.
    NoAskDepth,
    /// The bid side cannot absorb the configured notional.
    NoBidDepth,
    /// Best bid >= best ask — either a crossed book (violated invariant)
    /// or a locked book. Either way the "mid" is undefined; we refuse
    /// to publish rather than fabricate `(bid + ask) / 2` from an
    /// inverted spread.
    NonMonotonic,
    /// Interior multiplication overflowed. Should never happen for
    /// realistic $-scale books but explicit so a sudden `u128` blow-up
    /// (e.g. an errant fixture) does not silently wrap.
    Overflow,
}

impl InsufficientDepth {
    /// Stable string form for metrics / logs. `snake_case` matches the
    /// `serde(rename_all)` on the enum so wire and log strings agree.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ZeroNotional => "zero_notional",
            Self::NoAskDepth => "no_ask_depth",
            Self::NoBidDepth => "no_bid_depth",
            Self::NonMonotonic => "non_monotonic",
            Self::Overflow => "overflow",
        }
    }
}

/// A successfully-computed impact-mid sample. `mid_1e8` is the midpoint
/// (bid_impact + ask_impact) / 2; the component impact prices are kept
/// alongside so a diagnostic surface (or a future funding worker) can
/// audit the split without recomputing.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct ImpactMidSample {
    pub mid_1e8: u128,
    pub ask_impact_1e8: u128,
    pub bid_impact_1e8: u128,
}

/// Compute the taker VWAP for consuming exactly `notional_1e8` (quote,
/// `1e8` scale) walking `side` from index 0 outward. `side` MUST already
/// be sorted best-price-first: asks ascending, bids descending.
///
/// Returns `Err(ZeroNotional)` if `notional_1e8 == 0`. Returns
/// `Err(NoAskDepth)`/`Err(NoBidDepth)` — actually just `Err(Overflow)`
/// or `Err(...)` — never; the "not enough depth" outcome is `Ok(None)`
/// so the caller can attribute it to bid vs ask. Overflow (interior
/// multiplication saturating to `u128::MAX`) surfaces as
/// `Err(Overflow)`.
///
/// The size-of-side is the caller's concern (both to make this function
/// pure and to keep the enum minimal). See `impact_mid` for the full
/// wrapper that distinguishes bid vs ask insufficiency.
pub fn impact_price_walk(
    side: &[Level],
    notional_1e8: u128,
) -> Result<Option<u128>, InsufficientDepth> {
    use alloy_primitives::U256;
    if notional_1e8 == 0 {
        return Err(InsufficientDepth::ZeroNotional);
    }
    // Precision guarantee: for the uniform-price case
    // `take_notional_i / price_i` may not divide evenly at 1e8-scale
    // (e.g. $1000 at $3000/ETH → 0.333... ETH). A per-level floor at
    // 1e8-scaled base accumulates enough drift to break the trivial
    // "VWAP of a uniform-price walk equals the price" invariant. We
    // absorb the loss by accumulating base at an extra 1e16 factor
    // (final scale 1e24), then dividing at the end. Result: uniform-
    // price walks are byte-exact; mixed-price walks lose <1e-16 per
    // level relative — orders of magnitude below the deadband, the
    // impact tolerance, and the 1e8 quantisation of the output.
    let scale_1e24 = U256::from(SCALE_1E8)
        .checked_mul(U256::from(SCALE_1E8))
        .and_then(|s| s.checked_mul(U256::from(SCALE_1E8)))
        .ok_or(InsufficientDepth::Overflow)?;
    let mut remaining = notional_1e8;
    let mut total_cost: u128 = 0;
    let mut total_base_1e24: U256 = U256::ZERO;
    for level in side {
        if level.price_1e8 == 0 || level.size_1e8 == 0 {
            // A zero-price or zero-size level would either divide by zero
            // (base conversion) or contribute nothing (skip). Skipping is
            // safe and matches the store's "active order" invariant that
            // rejects zero-priced orders upstream — this defensive
            // branch protects against a stale/torn read.
            continue;
        }
        // level_notional = price * size / 1e8. Both scaled by 1e8, so
        // the raw product is a `1e16`-scaled quantity; dividing by 1e8
        // returns it to a `1e8`-scaled notional. We saturate to catch a
        // realistic-but-unbounded book size; the wrapper turns saturation
        // into `Overflow`.
        let level_notional = mul_div_1e8(level.price_1e8, level.size_1e8)
            .ok_or(InsufficientDepth::Overflow)?;
        let take_notional = core::cmp::min(remaining, level_notional);
        // take_base_1e24 = take_notional * 1e24 / price. Absorbs the
        // per-level rounding drift; see the uniform-price precision
        // note at the top of this function.
        let take_base_1e24 = U256::from(take_notional)
            .checked_mul(scale_1e24)
            .and_then(|n| n.checked_div(U256::from(level.price_1e8)))
            .ok_or(InsufficientDepth::Overflow)?;
        total_cost = total_cost
            .checked_add(take_notional)
            .ok_or(InsufficientDepth::Overflow)?;
        total_base_1e24 = total_base_1e24
            .checked_add(take_base_1e24)
            .ok_or(InsufficientDepth::Overflow)?;
        remaining = remaining.saturating_sub(take_notional);
        if remaining == 0 {
            break;
        }
    }
    if remaining > 0 {
        // Not enough depth on this side.
        return Ok(None);
    }
    if total_base_1e24 == U256::ZERO {
        // Every level had zero base — treat as insufficient depth. This
        // is defensive; the store rejects zero-size orders upstream.
        return Ok(None);
    }
    // vwap_1e8 = total_cost_1e8 * 1e24 / total_base_1e24. Dimensionally:
    //   actual_vwap = actual_cost / actual_base
    //   vwap_1e8    = (cost_1e8 / 1e8) / (base_1e24 / 1e24) * 1e8
    //              = cost_1e8 * 1e24 / base_1e24
    let vwap_scaled = U256::from(total_cost)
        .checked_mul(scale_1e24)
        .and_then(|n| n.checked_div(total_base_1e24))
        .ok_or(InsufficientDepth::Overflow)?;
    u128::try_from(vwap_scaled)
        .map(Some)
        .map_err(|_| InsufficientDepth::Overflow)
}

/// Compute the impact mid given both sides. `asks` MUST be ascending by
/// price (best first). `bids` MUST be descending by price (best first).
/// See `crate::perps::orderbook::active_asks_sorted` /
/// `active_bids_sorted` for the derivation helpers used by the keeper.
///
/// The `notional_1e8` argument is the taker notional (in quote, `1e8`
/// scale) at which the impact is measured — e.g. `$10k → 10_000 * 1e8`.
pub fn impact_mid(
    asks: &[Level],
    bids: &[Level],
    notional_1e8: u128,
) -> Result<ImpactMidSample, InsufficientDepth> {
    let ask_impact = impact_price_walk(asks, notional_1e8)?
        .ok_or(InsufficientDepth::NoAskDepth)?;
    let bid_impact = impact_price_walk(bids, notional_1e8)?
        .ok_or(InsufficientDepth::NoBidDepth)?;
    // Non-monotonic guard: after walking, the ask VWAP must strictly
    // exceed the bid VWAP. A crossed or locked book is undefined for
    // a mid — refuse to publish.
    if ask_impact <= bid_impact {
        return Err(InsufficientDepth::NonMonotonic);
    }
    let mid = ask_impact
        .checked_add(bid_impact)
        .ok_or(InsufficientDepth::Overflow)?
        / 2;
    Ok(ImpactMidSample {
        mid_1e8: mid,
        ask_impact_1e8: ask_impact,
        bid_impact_1e8: bid_impact,
    })
}

/// `a * b / 1e8` with overflow detection. Returns `None` on overflow.
fn mul_div_1e8(a: u128, b: u128) -> Option<u128> {
    mul_div(a, b, SCALE_1E8)
}

/// `a * b / d` with a `U256` intermediate. Returns `None` on divisor
/// zero or if the exact result exceeds `u128::MAX`.
///
/// The `U256` intermediate is required for VWAP precision: a naive
/// `u128 * u128 -> u128 / u128` sequence loses sub-unit precision on
/// each `take_base = take_notional * 1e8 / price` conversion, and the
/// error accumulates across levels. The impact-mid tests pin exact
/// equality when all fills happen at the same price (VWAP = P), which
/// only holds if the intermediate math is exact.
fn mul_div(a: u128, b: u128, d: u128) -> Option<u128> {
    use alloy_primitives::U256;
    if d == 0 {
        return None;
    }
    let prod = U256::from(a).checked_mul(U256::from(b))?;
    let quot = prod / U256::from(d);
    u128::try_from(quot).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const ONE: u128 = SCALE_1E8; // 1.00000000

    fn ask(price: u128, size: u128) -> Level {
        Level::new(price, size)
    }
    fn bid(price: u128, size: u128) -> Level {
        Level::new(price, size)
    }

    // -----------------------------------------------------------------
    // impact_price_walk (single-side)
    // -----------------------------------------------------------------

    #[test]
    fn zero_notional_returns_zero_notional_error() {
        let asks = &[ask(3000 * ONE, 10 * ONE)];
        let err = impact_price_walk(asks, 0).unwrap_err();
        assert_eq!(err, InsufficientDepth::ZeroNotional);
    }

    #[test]
    fn single_level_fully_fills_returns_that_price() {
        // 1 ETH @ $3000 → notional = $3000
        let asks = &[ask(3000 * ONE, 1 * ONE)];
        // Take $1500 of it (half the level).
        let vwap = impact_price_walk(asks, 1_500 * ONE).unwrap().unwrap();
        assert_eq!(vwap, 3000 * ONE);
    }

    #[test]
    fn insufficient_depth_returns_ok_none() {
        // Only $3000 on the ask; ask for $4000 → None (caller distinguishes
        // as `NoAskDepth`).
        let asks = &[ask(3000 * ONE, 1 * ONE)];
        let out = impact_price_walk(asks, 4_000 * ONE).unwrap();
        assert!(out.is_none());
    }

    #[test]
    fn multi_level_vwap_hand_computed() {
        // Book: 1 ETH @ $3000 (=$3000 notional), then 1 ETH @ $3010 (=$3010).
        // Take $4500. Level 1 fully consumed ($3000, base = 1 ETH).
        // Remaining $1500 from level 2:
        //   exact take_base = 1500 / 3010 = 0.4983388704318936877076411960... ETH
        //   → at 1e24 scale: 498_338_870_431_893_687_707_641 (floor)
        // total_cost_1e8 = 450_000_000_000
        // total_base_1e24 = 1e24 + 498_338_870_431_893_687_707_641
        //                 = 1_498_338_870_431_893_687_707_641
        // vwap_1e8 = 450_000_000_000 * 1e24 / 1_498_338_870_431_893_687_707_641
        //          = 300_332_594_235 (u256-exact, u128 floor)
        //          → $3003.32594235...
        let asks = &[ask(3000 * ONE, 1 * ONE), ask(3010 * ONE, 1 * ONE)];
        let vwap = impact_price_walk(asks, 4_500 * ONE).unwrap().unwrap();
        // Hand-recompute using the same U256-1e24 accumulator the
        // production code uses. A naive u128-only recompute would
        // drop precision at the per-level `take_base = take_notional
        // * 1e8 / price` floor and drift by ~1e-7 %, which would
        // desync from `impact_price_walk`.
        use alloy_primitives::U256;
        let scale_1e24 = U256::from(SCALE_1E8)
            .checked_mul(U256::from(SCALE_1E8))
            .unwrap()
            .checked_mul(U256::from(SCALE_1E8))
            .unwrap();
        let take2_base_1e24 = U256::from(1_500u128 * ONE)
            .checked_mul(scale_1e24)
            .unwrap()
            .checked_div(U256::from(3010u128 * ONE))
            .unwrap();
        let total_cost_1e8 = 4_500u128 * ONE;
        // Level 1 is 1 ETH fully consumed → base_1e24 = 1 * 1e24 = scale_1e24.
        // (Not `ONE * scale_1e24` — that would over-scale by 1e8; `ONE` is
        // itself the 1e8 encoding of the value `1`.)
        let total_base_1e24 = scale_1e24.checked_add(take2_base_1e24).unwrap();
        let expected = u128::try_from(
            U256::from(total_cost_1e8)
                .checked_mul(scale_1e24)
                .unwrap()
                .checked_div(total_base_1e24)
                .unwrap(),
        )
        .unwrap();
        assert_eq!(vwap, expected);
        // Sanity: the VWAP must lie strictly between the two level prices.
        assert!(vwap > 3000 * ONE);
        assert!(vwap < 3010 * ONE);
    }

    #[test]
    fn deep_book_takes_only_from_first_levels() {
        // 100 levels of $3000 for 1 ETH each ($300k total). Take $10k →
        // must exactly equal $3000.
        let asks: Vec<Level> = (0..100).map(|_| ask(3000 * ONE, 1 * ONE)).collect();
        let vwap = impact_price_walk(&asks, 10_000 * ONE).unwrap().unwrap();
        assert_eq!(vwap, 3000 * ONE);
    }

    #[test]
    fn zero_priced_level_is_skipped() {
        // Defensive: a torn/stale level with price==0 must not divide by
        // zero. It is skipped; the second level fills.
        let asks = &[ask(0, 10 * ONE), ask(3000 * ONE, 1 * ONE)];
        let vwap = impact_price_walk(asks, 1_000 * ONE).unwrap().unwrap();
        assert_eq!(vwap, 3000 * ONE);
    }

    // -----------------------------------------------------------------
    // impact_mid (both sides)
    // -----------------------------------------------------------------

    #[test]
    fn happy_path_mid_between_bid_and_ask() {
        let asks = &[ask(3010 * ONE, 10 * ONE)];
        let bids = &[bid(2990 * ONE, 10 * ONE)];
        let sample = impact_mid(asks, bids, 1_000 * ONE).unwrap();
        assert_eq!(sample.ask_impact_1e8, 3010 * ONE);
        assert_eq!(sample.bid_impact_1e8, 2990 * ONE);
        assert_eq!(sample.mid_1e8, 3000 * ONE);
    }

    #[test]
    fn ask_shallow_reports_no_ask_depth() {
        let asks = &[ask(3010 * ONE, 1 * ONE)]; // only $3010 notional
        let bids = &[bid(2990 * ONE, 100 * ONE)];
        let err = impact_mid(asks, bids, 100_000 * ONE).unwrap_err();
        assert_eq!(err, InsufficientDepth::NoAskDepth);
    }

    #[test]
    fn bid_shallow_reports_no_bid_depth() {
        let asks = &[ask(3010 * ONE, 100 * ONE)];
        let bids = &[bid(2990 * ONE, 1 * ONE)]; // only $2990 notional
        let err = impact_mid(asks, bids, 100_000 * ONE).unwrap_err();
        assert_eq!(err, InsufficientDepth::NoBidDepth);
    }

    #[test]
    fn crossed_book_reports_non_monotonic() {
        // Bid above ask — inverted spread.
        let asks = &[ask(2990 * ONE, 10 * ONE)];
        let bids = &[bid(3010 * ONE, 10 * ONE)];
        let err = impact_mid(asks, bids, 1_000 * ONE).unwrap_err();
        assert_eq!(err, InsufficientDepth::NonMonotonic);
    }

    #[test]
    fn locked_book_reports_non_monotonic() {
        // Bid == ask — locked. Mid is not undefined mathematically, but
        // we treat it as an invariant violation because a healthy book
        // never locks; if it does the tick should refuse to publish.
        let asks = &[ask(3000 * ONE, 10 * ONE)];
        let bids = &[bid(3000 * ONE, 10 * ONE)];
        let err = impact_mid(asks, bids, 1_000 * ONE).unwrap_err();
        assert_eq!(err, InsufficientDepth::NonMonotonic);
    }

    #[test]
    fn zero_notional_bubbles_up() {
        let asks = &[ask(3010 * ONE, 10 * ONE)];
        let bids = &[bid(2990 * ONE, 10 * ONE)];
        let err = impact_mid(asks, bids, 0).unwrap_err();
        assert_eq!(err, InsufficientDepth::ZeroNotional);
    }

    #[test]
    fn empty_side_is_insufficient_depth() {
        let asks: &[Level] = &[];
        let bids = &[bid(2990 * ONE, 10 * ONE)];
        let err = impact_mid(asks, bids, 1_000 * ONE).unwrap_err();
        assert_eq!(err, InsufficientDepth::NoAskDepth);

        let asks2 = &[ask(3010 * ONE, 10 * ONE)];
        let bids2: &[Level] = &[];
        let err2 = impact_mid(asks2, bids2, 1_000 * ONE).unwrap_err();
        assert_eq!(err2, InsufficientDepth::NoBidDepth);
    }

    #[test]
    fn deep_book_mid_equals_touch_when_depth_dominates() {
        // Very deep book at the best price on both sides → impact
        // collapses to the touch prices → mid == (best_ask + best_bid) / 2.
        let asks: Vec<Level> = std::iter::once(ask(3010 * ONE, 1_000 * ONE)).collect();
        let bids: Vec<Level> = std::iter::once(bid(2990 * ONE, 1_000 * ONE)).collect();
        let sample = impact_mid(&asks, &bids, 10_000 * ONE).unwrap();
        assert_eq!(sample.mid_1e8, 3000 * ONE);
    }

    #[test]
    fn insufficient_depth_str_labels_are_stable() {
        // The labels feed the observability counters; pin them so a
        // rename triggers a compile-time test failure the operator will
        // catch before deploying a broken metrics dashboard.
        assert_eq!(InsufficientDepth::ZeroNotional.as_str(), "zero_notional");
        assert_eq!(InsufficientDepth::NoAskDepth.as_str(), "no_ask_depth");
        assert_eq!(InsufficientDepth::NoBidDepth.as_str(), "no_bid_depth");
        assert_eq!(InsufficientDepth::NonMonotonic.as_str(), "non_monotonic");
        assert_eq!(InsufficientDepth::Overflow.as_str(), "overflow");
    }
}
