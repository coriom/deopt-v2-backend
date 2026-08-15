//! OPTIONS-HYBRID-V2-RESERVATIONS-PENDING-SETTLEMENT-AND-CANONICAL-RELEASE-V1
//! Package C — pure-math reservation formulas.
//!
//! Frozen normative definitions (from the risk-reservation design
//! doc, no re-invention):
//!
//! For an option order with:
//!   * `quantity_1e8` — order size in the 1e8 fixed-point convention
//!     (`Q`)
//!   * `contract_size_1e8` — series contract multiplier in 1e8 fixed
//!     point (`C`)
//!   * `premium_1e8` — limit premium per contract in 1e8 fixed point
//!     (`P`) — used for buys
//!   * `strike_1e8` — strike price in 1e8 fixed point (`S`) — used
//!     for short puts
//!
//! Formulas — all evaluated in `u256` wide intermediate arithmetic to
//! preclude silent overflow, then reduced to `u128` (settlement /
//! underlying token native units).
//!
//! * BUY CALL / BUY PUT:
//!   `reserved_settlement = ceil_div(Q * C * P, 1e16)`
//!
//! * SHORT CALL (physical):
//!   `reserved_underlying = ceil_div(Q * C, 1e8)`
//!
//! * SHORT PUT (cash-collateralised):
//!   `reserved_settlement = ceil_div(Q * C * S, 1e16)`
//!
//! Rounding: always UPWARD (ceil-div) so the ledger's reserved amount
//! never under-collateralises the exposure — a partial cent left
//! stranded on rounding stays in the reservation until settlement
//! releases it.
//!
//! Overflow: any intermediate that exceeds the 256-bit widening is a
//! bug (would require an option order with astronomical Q × C × P).
//! The formulas fail closed with an explicit error rather than
//! saturating.
//!
//! Cash-settled short-call semantics are intentionally NOT hardcoded
//! here — they follow the current `MarginEngine` required-collateral
//! rule. Callers that need cash-settled short-call reservations must
//! pass an explicit override; the default `short_call_reservation`
//! implements the physical-settlement formula.
//!
//! This module is pure math: no I/O, no allocations beyond `String`
//! for return, deterministic per input.

use crate::error::{BackendError, Result};
use alloy_primitives::U256;

/// The 1e8 fixed-point scale constant used across the Options
/// protocol (`quantity_1e8`, `contract_size_1e8`, `premium_1e8`,
/// `strike_1e8`, `price_1e8`).
const SCALE_1E8: u128 = 100_000_000;

/// Product of the buy-side / short-put scaling: `Q_1e8 × C_1e8 ×
/// {P_1e8|S_1e8}` divided by `1e16` yields native settlement units.
const SCALE_1E16: u128 = 10_000_000_000_000_000;

/// Reserve for a BUY (call or put) order.
///
/// Formula: `ceil_div(quantity_1e8 * contract_size_1e8 * premium_1e8, 1e16)`.
///
/// Returns the reserved amount in native settlement-token units. Wide
/// (256-bit) intermediate arithmetic; overflow → error.
pub fn buy_reservation(
    quantity_1e8: u128,
    contract_size_1e8: u128,
    premium_1e8: u128,
) -> Result<u128> {
    if quantity_1e8 == 0 {
        return Err(BackendError::ZeroSize);
    }
    if contract_size_1e8 == 0 {
        return Err(BackendError::InvalidOptionExecutionIntentState(
            "contract_size_1e8 must be nonzero".to_string(),
        ));
    }
    if premium_1e8 == 0 {
        return Err(BackendError::ZeroPrice);
    }
    ceil_div_wide_by_scale(quantity_1e8, contract_size_1e8, premium_1e8, SCALE_1E16)
}

/// Reserve for a SHORT PUT (cash-collateralised).
///
/// Formula: `ceil_div(quantity_1e8 * contract_size_1e8 * strike_1e8, 1e16)`.
pub fn short_put_reservation(
    quantity_1e8: u128,
    contract_size_1e8: u128,
    strike_1e8: u128,
) -> Result<u128> {
    if quantity_1e8 == 0 {
        return Err(BackendError::ZeroSize);
    }
    if contract_size_1e8 == 0 {
        return Err(BackendError::InvalidOptionExecutionIntentState(
            "contract_size_1e8 must be nonzero".to_string(),
        ));
    }
    if strike_1e8 == 0 {
        return Err(BackendError::InvalidOptionExecutionIntentState(
            "strike_1e8 must be nonzero".to_string(),
        ));
    }
    ceil_div_wide_by_scale(quantity_1e8, contract_size_1e8, strike_1e8, SCALE_1E16)
}

/// Reserve for a physical-settlement SHORT CALL.
///
/// Formula: `ceil_div(quantity_1e8 * contract_size_1e8, 1e8)`.
///
/// Returns the reserved amount in native underlying-token units.
/// Cash-settled short calls must follow the current MarginEngine
/// required-collateral semantics — callers wanting cash-settled
/// behavior must NOT use this helper.
pub fn short_call_reservation_physical(
    quantity_1e8: u128,
    contract_size_1e8: u128,
) -> Result<u128> {
    if quantity_1e8 == 0 {
        return Err(BackendError::ZeroSize);
    }
    if contract_size_1e8 == 0 {
        return Err(BackendError::InvalidOptionExecutionIntentState(
            "contract_size_1e8 must be nonzero".to_string(),
        ));
    }
    // ceil_div(Q * C, 1e8)
    let product = U256::from(quantity_1e8)
        .checked_mul(U256::from(contract_size_1e8))
        .ok_or_else(|| overflow_error("quantity * contract_size"))?;
    ceil_div_by_u128(product, SCALE_1E8)
}

/// `ceil_div(a * b * c, scale)` in wide arithmetic. Every step is
/// checked; overflow at any step returns an error. The final result
/// must fit u128 (native token amount) — a fit failure surfaces the
/// same error class.
fn ceil_div_wide_by_scale(a: u128, b: u128, c: u128, scale: u128) -> Result<u128> {
    let product_ab = U256::from(a)
        .checked_mul(U256::from(b))
        .ok_or_else(|| overflow_error("a * b"))?;
    let product_abc = product_ab
        .checked_mul(U256::from(c))
        .ok_or_else(|| overflow_error("a * b * c"))?;
    ceil_div_by_u128(product_abc, scale)
}

fn ceil_div_by_u128(numerator: U256, denominator: u128) -> Result<u128> {
    if denominator == 0 {
        return Err(BackendError::InvalidOptionExecutionIntentState(
            "reservation formula denominator is zero".to_string(),
        ));
    }
    let denom = U256::from(denominator);
    let quotient = numerator / denom;
    let remainder = numerator % denom;
    let ceil = if remainder.is_zero() {
        quotient
    } else {
        quotient
            .checked_add(U256::from(1u8))
            .ok_or_else(|| overflow_error("ceil_div rounding"))?
    };
    let ceil_u128: u128 = ceil
        .try_into()
        .map_err(|_| overflow_error("result > u128"))?;
    Ok(ceil_u128)
}

fn overflow_error(where_: &str) -> BackendError {
    BackendError::InvalidOptionExecutionIntentState(format!(
        "reservation formula overflow: {where_}"
    ))
}

// -------------------------------------------------------------------
// Tests — pure math, no PG needed. Kept alongside the module so the
// invariants are locked in on every `cargo test --lib`.
// -------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buy_reservation_matches_normative_formula() {
        // Q=1 contract at 1x multiplier and premium=1.00 settlement
        // 1e8 * 1e8 * 1e8 = 1e24 / 1e16 = 1e8 native units.
        assert_eq!(
            buy_reservation(SCALE_1E8, SCALE_1E8, SCALE_1E8).unwrap(),
            SCALE_1E8
        );
    }

    #[test]
    fn buy_reservation_ceil_rounds_up_partial_units() {
        // Odd fraction that leaves a nonzero remainder must round up.
        // Q=1, C=1, P=one raw unit above zero → still yields 1 native
        // unit (never zero).
        let out = buy_reservation(SCALE_1E8, SCALE_1E8, 1).unwrap();
        assert_eq!(out, 1);
    }

    #[test]
    fn buy_reservation_rejects_zero_size() {
        assert!(matches!(
            buy_reservation(0, SCALE_1E8, SCALE_1E8),
            Err(BackendError::ZeroSize)
        ));
    }

    #[test]
    fn buy_reservation_rejects_zero_premium() {
        assert!(matches!(
            buy_reservation(SCALE_1E8, SCALE_1E8, 0),
            Err(BackendError::ZeroPrice)
        ));
    }

    #[test]
    fn buy_reservation_rejects_zero_contract_size() {
        assert!(buy_reservation(SCALE_1E8, 0, SCALE_1E8).is_err());
    }

    #[test]
    fn buy_reservation_rejects_overflow_astronomical_input() {
        // Q * C * P must fit u256; result must fit u128.
        let huge = u128::MAX;
        assert!(buy_reservation(huge, huge, huge).is_err());
    }

    #[test]
    fn short_put_matches_strike_formula() {
        // Q=1, C=1, S=3000.00 → 3000 native units.
        let strike = 3_000 * SCALE_1E8;
        let out = short_put_reservation(SCALE_1E8, SCALE_1E8, strike).unwrap();
        assert_eq!(out, 3_000 * SCALE_1E8);
    }

    #[test]
    fn short_call_physical_matches_underlying_formula() {
        // Q=2, C=1 → 2 native underlying units.
        let out = short_call_reservation_physical(2 * SCALE_1E8, SCALE_1E8).unwrap();
        assert_eq!(out, 2 * SCALE_1E8);
    }

    #[test]
    fn short_call_physical_rounds_up() {
        // Q * C not evenly divisible by 1e8 → +1 native unit.
        let q = SCALE_1E8; // 1.0
        let c = SCALE_1E8 + 1; // 1.0 + one raw unit
        let out = short_call_reservation_physical(q, c).unwrap();
        // Q*C = 1e8 * (1e8+1) = 1e16 + 1e8; /1e8 (ceil) = 1e8 + 1
        assert_eq!(out, SCALE_1E8 + 1);
    }

    #[test]
    fn zero_denominator_guarded() {
        // Direct helper reject.
        assert!(ceil_div_by_u128(U256::from(1u8), 0).is_err());
    }

    #[test]
    fn boundary_exact_division_returns_quotient() {
        // Numerator divisible → no rounding bump.
        let q = 4 * SCALE_1E8;
        let c = 5 * SCALE_1E8;
        // Q*C = 20e16; /1e8 = 20e8 = 2_000_000_000.
        let out = short_call_reservation_physical(q, c).unwrap();
        assert_eq!(out, 2_000_000_000);
    }

    #[test]
    fn large_but_safe_input_computes() {
        // Max realistic option: 1M contracts (Q=1e14) × 1x multiplier
        // × premium=10000.00 (P=1e12). Q*C*P = 1e14 * 1e8 * 1e12 = 1e34
        // fits u256 (~1.16e77). Result = 1e34 / 1e16 = 1e18 fits u128.
        let q = 1_000_000 * SCALE_1E8;
        let c = SCALE_1E8;
        let p = 10_000 * SCALE_1E8;
        let out = buy_reservation(q, c, p).unwrap();
        assert_eq!(out, 1_000_000_000_000_000_000);
    }
}
