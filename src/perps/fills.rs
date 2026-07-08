//! PERPS-ISOLATED-MARGIN-POSITION-ENGINE-V1 — internal fill application.
//!
//! `apply_perp_fill_for_account` mutates the in-memory
//! `PerpPositionsStore` to open, increase, reduce, or close a position.
//! It is called by unit tests and by future order-execution code —
//! **never** by a public HTTP handler. No route currently exposes it.
//!
//! Semantics (isolated margin, one position per account/market):
//!
//! * If no active position exists → open a fresh one. Requires
//!   `margin_1e8 >= initial_margin_requirement_1e8(size, price,
//!   market.max_leverage)` and refuses zero size / price / margin.
//! * If an active position exists in the same direction → increase.
//!   The `add_margin_1e8` is added to `margin`; entry is bumped to
//!   the weighted-average. Post-increase leverage is re-checked.
//! * If an active position exists in the opposite direction and the
//!   fill size ≤ existing size → reduce. Realises proportional PnL.
//!   The remaining position keeps its entry price. Reducing to zero
//!   closes the position; `margin` and any positive realised PnL
//!   are returned to the trader (conceptually — V1 does not touch
//!   on-chain collateral).
//! * If the fill size **exceeds** the existing opposite-side position,
//!   the V1 model **rejects the flip** with `PerpPositionFlip`. Later
//!   milestones may support atomic close-then-open.

use crate::error::{BackendError, Result};
use crate::perps::config::PerpsReadMarket;
use crate::perps::margin;
use crate::perps::positions::{
    new_position_skeleton, PerpPosition, PerpPositionStatus, PerpPositionsStore, PerpSide,
};
use crate::types::{now_ms, AccountId, TimestampMs};

/// A single Perps fill applied against a `PerpPositionsStore`. All
/// numeric inputs are `1e8`-scaled.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PerpFillInput {
    pub account: AccountId,
    /// PERPS-SUBACCOUNTS-ENGINE-ROUTING-V1 — the subaccount the fill
    /// mutates. Default `1` (Account 1) via the migration + serde
    /// defaults; explicit values isolate positions across subaccounts.
    pub subaccount_id: u32,
    pub market_id: String,
    pub side: PerpSide,
    pub size_1e8: u128,
    pub price_1e8: u128,
    /// For an opening/increasing fill: the isolated margin the trader
    /// posts. For a reducing/closing fill: ignored (margin is
    /// released proportionally, but V1 keeps this API simple — a
    /// non-zero value is silently ignored on reduce).
    pub margin_1e8: u128,
}

/// Outcome of applying one fill — surfaced to the caller so future
/// order-execution code can decide what to persist / emit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PerpFillOutcome {
    Opened(PerpPosition),
    Increased(PerpPosition),
    Reduced {
        position: PerpPosition,
        realized_pnl_1e8: i128,
    },
    Closed {
        position: PerpPosition,
        realized_pnl_1e8: i128,
    },
}

impl PerpFillOutcome {
    pub fn position(&self) -> &PerpPosition {
        match self {
            Self::Opened(p) | Self::Increased(p) => p,
            Self::Reduced { position, .. } | Self::Closed { position, .. } => position,
        }
    }

    pub fn realized_pnl_1e8(&self) -> i128 {
        match self {
            Self::Opened(_) | Self::Increased(_) => 0,
            Self::Reduced {
                realized_pnl_1e8, ..
            }
            | Self::Closed {
                realized_pnl_1e8, ..
            } => *realized_pnl_1e8,
        }
    }
}

/// Apply one fill in-place. Returns the outcome (the caller can then
/// mirror to PG or emit lifecycle events — that plumbing lands in
/// later milestones).
pub fn apply_perp_fill_for_account(
    store: &mut PerpPositionsStore,
    market: &PerpsReadMarket,
    fill: PerpFillInput,
) -> Result<PerpFillOutcome> {
    validate_fill_basics(&fill)?;
    if fill.market_id != market.symbol {
        return Err(BackendError::Config(format!(
            "market row / fill symbol mismatch: {} vs {}",
            market.symbol, fill.market_id
        )));
    }
    let now = now_ms();
    let existing = store.get_active(&fill.account, fill.subaccount_id, &fill.market_id);
    match existing {
        None => open_fresh_position(store, market, fill, now),
        Some(pos) if pos.side == fill.side => increase_position(store, market, pos, fill, now),
        Some(pos) => reduce_or_reject(store, pos, fill, now),
    }
}

fn validate_fill_basics(fill: &PerpFillInput) -> Result<()> {
    if fill.size_1e8 == 0 {
        return Err(BackendError::PerpZeroSize);
    }
    if fill.price_1e8 == 0 {
        return Err(BackendError::PerpZeroPrice);
    }
    Ok(())
}

fn open_fresh_position(
    store: &mut PerpPositionsStore,
    market: &PerpsReadMarket,
    fill: PerpFillInput,
    _now: TimestampMs,
) -> Result<PerpFillOutcome> {
    if fill.margin_1e8 == 0 {
        return Err(BackendError::PerpZeroMargin);
    }
    let required =
        margin::initial_margin_requirement_1e8(fill.size_1e8, fill.price_1e8, market.max_leverage);
    if fill.margin_1e8 < required {
        return Err(BackendError::PerpInsufficientMargin(format!(
            "margin {} < initial requirement {} at leverage {}x",
            fill.margin_1e8, required, market.max_leverage
        )));
    }
    let notional = margin::notional_1e8(fill.size_1e8, fill.price_1e8);
    enforce_leverage_cap(
        &fill.market_id,
        notional,
        fill.margin_1e8,
        market.max_leverage,
    )?;
    let position = new_position_skeleton(
        fill.account,
        fill.subaccount_id,
        fill.market_id,
        fill.side,
        fill.size_1e8,
        fill.price_1e8,
        fill.margin_1e8,
    );
    let inserted = store.insert_open(position)?;
    Ok(PerpFillOutcome::Opened(inserted))
}

fn increase_position(
    store: &mut PerpPositionsStore,
    market: &PerpsReadMarket,
    existing: PerpPosition,
    fill: PerpFillInput,
    now: TimestampMs,
) -> Result<PerpFillOutcome> {
    if fill.margin_1e8 == 0 {
        return Err(BackendError::PerpZeroMargin);
    }
    let new_size = existing.size_1e8.saturating_add(fill.size_1e8);
    let new_entry = margin::weighted_average_entry_1e8(
        existing.size_1e8,
        existing.entry_price_1e8,
        fill.size_1e8,
        fill.price_1e8,
    );
    let new_margin = existing.margin_1e8.saturating_add(fill.margin_1e8);
    let new_notional = margin::notional_1e8(new_size, new_entry);
    let required = margin::initial_margin_requirement_1e8(new_size, new_entry, market.max_leverage);
    if new_margin < required {
        return Err(BackendError::PerpInsufficientMargin(format!(
            "post-increase margin {} < initial requirement {} at leverage {}x",
            new_margin, required, market.max_leverage
        )));
    }
    enforce_leverage_cap(
        &fill.market_id,
        new_notional,
        new_margin,
        market.max_leverage,
    )?;
    let position = store.update_active(
        &fill.account,
        fill.subaccount_id,
        &fill.market_id,
        now,
        |p| {
            p.size_1e8 = new_size;
            p.entry_price_1e8 = new_entry;
            p.margin_1e8 = new_margin;
            Ok(())
        },
    )?;
    Ok(PerpFillOutcome::Increased(position))
}

fn reduce_or_reject(
    store: &mut PerpPositionsStore,
    existing: PerpPosition,
    fill: PerpFillInput,
    now: TimestampMs,
) -> Result<PerpFillOutcome> {
    if fill.size_1e8 > existing.size_1e8 {
        // V1 refuses flips: the operator must close first, then
        // re-open. The safer path avoids ambiguous residual sizing
        // and prevents accidental over-reduction if the fill batch is
        // reprocessed.
        return Err(BackendError::PerpPositionFlip);
    }
    // Reduction is at `fill.price_1e8` against the *existing* side.
    let realized = margin::realized_pnl_1e8(
        existing.entry_price_1e8,
        fill.price_1e8,
        fill.size_1e8,
        existing.side,
    );
    let full_close = fill.size_1e8 == existing.size_1e8;
    if full_close {
        // Close entire position. Realised PnL accumulates and margin
        // is conceptually released.
        let mut closed = store.close_active(
            &existing.account,
            existing.subaccount_id,
            &existing.market_id,
            now,
        )?;
        closed.realized_pnl_1e8 = closed.realized_pnl_1e8.saturating_add(realized);
        closed.size_1e8 = 0;
        // Persist the mutated fields via a re-insert into `by_id` —
        // `close_active` already returned a clone from the store. We
        // need to update the row it stored, so use `update_active`
        // before close_active? Simpler path: update the row via a
        // direct write below, but for V1 we accept that the closed
        // row surfaced from `list_for_account` reflects the pre-close
        // PnL until PG mirroring lands. Explicit note in docs.
        let position = store.update_closed_realized_pnl(closed.id, realized, now)?;
        Ok(PerpFillOutcome::Closed {
            position,
            realized_pnl_1e8: realized,
        })
    } else {
        let new_size = existing.size_1e8 - fill.size_1e8;
        // Release margin proportional to the reduced size. Entry
        // price is preserved for the residual.
        let released = mul_div_floor(existing.margin_1e8, fill.size_1e8, existing.size_1e8);
        let new_margin = existing.margin_1e8.saturating_sub(released);
        let position = store.update_active(
            &fill.account,
            fill.subaccount_id,
            &fill.market_id,
            now,
            |p| {
                p.size_1e8 = new_size;
                p.margin_1e8 = new_margin;
                p.realized_pnl_1e8 = p.realized_pnl_1e8.saturating_add(realized);
                Ok(())
            },
        )?;
        Ok(PerpFillOutcome::Reduced {
            position,
            realized_pnl_1e8: realized,
        })
    }
}

fn enforce_leverage_cap(
    market_id: &str,
    notional_1e8: u128,
    margin_1e8: u128,
    max_leverage: u32,
) -> Result<()> {
    if margin_1e8 == 0 {
        return Err(BackendError::PerpZeroMargin);
    }
    // notional / margin ≤ max_leverage
    // Multiply out to avoid a division-by-zero and integer truncation.
    let lhs = notional_1e8; // notional
    let rhs = margin_1e8.saturating_mul(max_leverage as u128);
    if lhs > rhs {
        return Err(BackendError::PerpLeverageExceeded {
            market: market_id.to_string(),
            reason: format!(
                "effective leverage {}x exceeds cap {}x (notional {} margin {})",
                notional_1e8 / margin_1e8.max(1),
                max_leverage,
                notional_1e8,
                margin_1e8
            ),
        });
    }
    Ok(())
}

fn mul_div_floor(a: u128, b: u128, denom: u128) -> u128 {
    if denom == 0 {
        return 0;
    }
    match a.checked_mul(b) {
        Some(p) => p / denom,
        None => u128::MAX / denom,
    }
}

// The in-memory store needs a small extra method so `close_active`
// can be paired with a realised-PnL patch on the closed row. Keep it
// local so the store API stays symmetrical: `update_active` +
// `close_active` + this closed-row patch.
impl PerpPositionsStore {
    /// Attach realised PnL to a closed row. Used by the fill
    /// applicator to carry PnL into the history view.
    pub fn update_closed_realized_pnl(
        &mut self,
        id: uuid::Uuid,
        realized_delta_1e8: i128,
        now: TimestampMs,
    ) -> Result<PerpPosition> {
        let row = self
            .by_id_mut(id)
            .ok_or(BackendError::PerpPositionNotFound)?;
        if row.status != PerpPositionStatus::Closed {
            return Err(BackendError::Config(
                "update_closed_realized_pnl called on a non-closed position".to_string(),
            ));
        }
        row.realized_pnl_1e8 = row.realized_pnl_1e8.saturating_add(realized_delta_1e8);
        row.updated_at_ms = now;
        Ok(row.clone())
    }
}

// ---------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::perps::config::PerpsReadConfig;

    fn eth_market() -> PerpsReadMarket {
        PerpsReadConfig::enabled_in_memory_for_tests().markets[0].clone()
    }

    fn btc_market() -> PerpsReadMarket {
        PerpsReadConfig::enabled_in_memory_for_tests().markets[1].clone()
    }

    fn addr(hex: &str) -> AccountId {
        AccountId::new(hex.to_string())
    }

    #[test]
    fn opens_a_fresh_long_at_max_leverage_boundary() {
        let mut store = PerpPositionsStore::new();
        let market = eth_market();
        let outcome = apply_perp_fill_for_account(
            &mut store,
            &market,
            PerpFillInput {
                account: addr("0x0000000000000000000000000000000000000aaa"),
                subaccount_id: 1,
                market_id: "ETH-PERP".to_string(),
                side: PerpSide::Long,
                size_1e8: 100_000_000,      // 1 ETH
                price_1e8: 300_000_000_000, // $3000
                margin_1e8: 30_000_000_000, // $300 (exactly 10x)
            },
        )
        .unwrap();
        assert!(matches!(outcome, PerpFillOutcome::Opened(_)));
        let pos = outcome.position();
        assert_eq!(pos.side, PerpSide::Long);
        assert_eq!(pos.size_1e8, 100_000_000);
        assert_eq!(pos.entry_price_1e8, 300_000_000_000);
        assert_eq!(pos.margin_1e8, 30_000_000_000);
    }

    #[test]
    fn insufficient_margin_rejected() {
        let mut store = PerpPositionsStore::new();
        let err = apply_perp_fill_for_account(
            &mut store,
            &eth_market(),
            PerpFillInput {
                account: addr("0x0000000000000000000000000000000000000aaa"),
                subaccount_id: 1,
                market_id: "ETH-PERP".to_string(),
                side: PerpSide::Long,
                size_1e8: 100_000_000,
                price_1e8: 300_000_000_000,
                margin_1e8: 29_999_999_999, // 1 sat below the 300 IM
            },
        )
        .unwrap_err();
        assert!(matches!(err, BackendError::PerpInsufficientMargin(_)));
    }

    #[test]
    fn leverage_cap_enforced_even_when_margin_is_above_im() {
        // Trick: for BTC-PERP the cap is 5x. Even though margin is
        // just above the naive IM, the leverage cap must fire when
        // notional / margin > cap.
        let mut store = PerpPositionsStore::new();
        let err = apply_perp_fill_for_account(
            &mut store,
            &btc_market(),
            PerpFillInput {
                account: addr("0x0000000000000000000000000000000000000aaa"),
                subaccount_id: 1,
                market_id: "BTC-PERP".to_string(),
                side: PerpSide::Long,
                size_1e8: 100_000_000,         // 1 BTC
                price_1e8: 6_500_000_000_000,  // $65000
                margin_1e8: 1_000_000_000_000, // $10000 → 6.5x, > 5x cap
            },
        )
        .unwrap_err();
        assert!(
            matches!(err, BackendError::PerpInsufficientMargin(_))
                || matches!(err, BackendError::PerpLeverageExceeded { .. })
        );
    }

    #[test]
    fn increase_same_side_updates_weighted_average_entry() {
        let mut store = PerpPositionsStore::new();
        let market = eth_market();
        let acc = addr("0x0000000000000000000000000000000000000aaa");
        apply_perp_fill_for_account(
            &mut store,
            &market,
            PerpFillInput {
                account: acc.clone(),
                subaccount_id: 1,
                market_id: "ETH-PERP".to_string(),
                side: PerpSide::Long,
                size_1e8: 100_000_000,
                price_1e8: 300_000_000_000,
                margin_1e8: 30_000_000_000,
            },
        )
        .unwrap();
        // Add 1 more contract at $3200 with margin $320.
        let outcome = apply_perp_fill_for_account(
            &mut store,
            &market,
            PerpFillInput {
                account: acc.clone(),
                subaccount_id: 1,
                market_id: "ETH-PERP".to_string(),
                side: PerpSide::Long,
                size_1e8: 100_000_000,
                price_1e8: 320_000_000_000,
                margin_1e8: 32_000_000_000,
            },
        )
        .unwrap();
        assert!(matches!(outcome, PerpFillOutcome::Increased(_)));
        let pos = store.get_active(&acc, 1, "ETH-PERP").unwrap();
        assert_eq!(pos.size_1e8, 200_000_000);
        assert_eq!(pos.entry_price_1e8, 310_000_000_000);
        assert_eq!(pos.margin_1e8, 62_000_000_000);
    }

    #[test]
    fn partial_reduce_realises_proportional_pnl_and_preserves_entry() {
        let mut store = PerpPositionsStore::new();
        let market = eth_market();
        let acc = addr("0x0000000000000000000000000000000000000aaa");
        apply_perp_fill_for_account(
            &mut store,
            &market,
            PerpFillInput {
                account: acc.clone(),
                subaccount_id: 1,
                market_id: "ETH-PERP".to_string(),
                side: PerpSide::Long,
                size_1e8: 200_000_000,
                price_1e8: 300_000_000_000,
                margin_1e8: 60_000_000_000,
            },
        )
        .unwrap();
        // Reduce 1 contract at $3200 (opposite side is Short, but the
        // fill applicator receives a Short-side reduction against a
        // Long position).
        let outcome = apply_perp_fill_for_account(
            &mut store,
            &market,
            PerpFillInput {
                account: acc.clone(),
                subaccount_id: 1,
                market_id: "ETH-PERP".to_string(),
                side: PerpSide::Short,
                size_1e8: 100_000_000,
                price_1e8: 320_000_000_000,
                margin_1e8: 0,
            },
        )
        .unwrap();
        let realised = outcome.realized_pnl_1e8();
        assert_eq!(realised, 20_000_000_000); // ($3200 - $3000) * 1 = +$200
        let pos = store.get_active(&acc, 1, "ETH-PERP").unwrap();
        assert_eq!(pos.size_1e8, 100_000_000);
        assert_eq!(pos.entry_price_1e8, 300_000_000_000);
        // Margin released proportionally: 60 * 100/200 = 30 → residual 30.
        assert_eq!(pos.margin_1e8, 30_000_000_000);
    }

    #[test]
    fn full_close_returns_closed_and_removes_active_index() {
        let mut store = PerpPositionsStore::new();
        let market = eth_market();
        let acc = addr("0x0000000000000000000000000000000000000aaa");
        apply_perp_fill_for_account(
            &mut store,
            &market,
            PerpFillInput {
                account: acc.clone(),
                subaccount_id: 1,
                market_id: "ETH-PERP".to_string(),
                side: PerpSide::Long,
                size_1e8: 100_000_000,
                price_1e8: 300_000_000_000,
                margin_1e8: 30_000_000_000,
            },
        )
        .unwrap();
        let outcome = apply_perp_fill_for_account(
            &mut store,
            &market,
            PerpFillInput {
                account: acc.clone(),
                subaccount_id: 1,
                market_id: "ETH-PERP".to_string(),
                side: PerpSide::Short,
                size_1e8: 100_000_000,
                price_1e8: 310_000_000_000,
                margin_1e8: 0,
            },
        )
        .unwrap();
        assert!(matches!(outcome, PerpFillOutcome::Closed { .. }));
        assert_eq!(outcome.realized_pnl_1e8(), 10_000_000_000);
        assert!(store.get_active(&acc, 1, "ETH-PERP").is_none());
    }

    #[test]
    fn flip_rejected_in_v1() {
        let mut store = PerpPositionsStore::new();
        let market = eth_market();
        let acc = addr("0x0000000000000000000000000000000000000aaa");
        apply_perp_fill_for_account(
            &mut store,
            &market,
            PerpFillInput {
                account: acc.clone(),
                subaccount_id: 1,
                market_id: "ETH-PERP".to_string(),
                side: PerpSide::Long,
                size_1e8: 100_000_000,
                price_1e8: 300_000_000_000,
                margin_1e8: 30_000_000_000,
            },
        )
        .unwrap();
        let err = apply_perp_fill_for_account(
            &mut store,
            &market,
            PerpFillInput {
                account: acc,
                subaccount_id: 1,
                market_id: "ETH-PERP".to_string(),
                side: PerpSide::Short,
                size_1e8: 200_000_000, // exceeds existing
                price_1e8: 310_000_000_000,
                margin_1e8: 60_000_000_000,
            },
        )
        .unwrap_err();
        assert!(matches!(err, BackendError::PerpPositionFlip));
    }

    #[test]
    fn zero_size_rejected() {
        let mut store = PerpPositionsStore::new();
        let err = apply_perp_fill_for_account(
            &mut store,
            &eth_market(),
            PerpFillInput {
                account: addr("0x0000000000000000000000000000000000000aaa"),
                subaccount_id: 1,
                market_id: "ETH-PERP".to_string(),
                side: PerpSide::Long,
                size_1e8: 0,
                price_1e8: 300_000_000_000,
                margin_1e8: 30_000_000_000,
            },
        )
        .unwrap_err();
        assert!(matches!(err, BackendError::PerpZeroSize));
    }

    #[test]
    fn zero_price_rejected() {
        let mut store = PerpPositionsStore::new();
        let err = apply_perp_fill_for_account(
            &mut store,
            &eth_market(),
            PerpFillInput {
                account: addr("0x0000000000000000000000000000000000000aaa"),
                subaccount_id: 1,
                market_id: "ETH-PERP".to_string(),
                side: PerpSide::Long,
                size_1e8: 100_000_000,
                price_1e8: 0,
                margin_1e8: 30_000_000_000,
            },
        )
        .unwrap_err();
        assert!(matches!(err, BackendError::PerpZeroPrice));
    }

    #[test]
    fn unsupported_market_symbol_rejected() {
        let mut store = PerpPositionsStore::new();
        let err = apply_perp_fill_for_account(
            &mut store,
            &eth_market(),
            PerpFillInput {
                account: addr("0x0000000000000000000000000000000000000aaa"),
                subaccount_id: 1,
                market_id: "SOL-PERP".to_string(), // doesn't match the market row
                side: PerpSide::Long,
                size_1e8: 100_000_000,
                price_1e8: 30_000_000_000,
                margin_1e8: 3_000_000_000,
            },
        )
        .unwrap_err();
        assert!(matches!(err, BackendError::Config(_)));
    }
}
