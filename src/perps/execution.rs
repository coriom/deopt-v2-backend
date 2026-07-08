//! PERPS-ORDER-EXECUTION-INTERNAL-V1 — internal order execution
//! service. NOT wired to any public HTTP mutation route in V1 —
//! public routes still return `503 PerpsNotLive` at handler entry.
//! Unit + integration tests call these functions directly to exercise
//! matching, position updates, TIF handling, and risk checks.
//!
//! Semantics (isolated margin, one position per account/market):
//!   * Limit orders match against resting counterparties by price-time
//!     priority; fills happen at the maker's resting price.
//!   * Taker gets a position update via `apply_perp_fill_for_account`
//!     with `side = order.side.position_side()`. The maker gets the
//!     opposite side.
//!   * Post-only rejects if any part of the taker would cross.
//!   * FOK rejects if aggregate resting size (up to the taker's limit)
//!     is less than the taker's size.
//!   * IOC cancels any un-filled remainder after crossing.
//!   * GTC rests any remainder as `open` or `partially_filled`.
//!   * Reduce-only orders may not increase net exposure — a reduce
//!     taker requires an existing opposite-side position, and the
//!     total fill size must not exceed the existing position's size.
//!   * Self-trade rejected.
//!   * Mark price required-fresh for opens/increases; a stale/
//!     unavailable mark blocks the order with `stale_mark_price`.

use crate::error::{BackendError, Result};
use crate::perps::config::{PerpsReadConfig, PerpsReadMarket};
use crate::perps::fills::{apply_perp_fill_for_account, PerpFillInput};
use crate::perps::order_store::PerpOrderStore;
use crate::perps::orderbook::{counterparties_for, crosses};
use crate::perps::orders::{
    reason, PerpFill, PerpOrder, PerpOrderSide, PerpOrderStatus, PerpTimeInForce,
};
use crate::perps::positions::PerpPositionsStore;
use crate::perps::price_reader::PerpOraclePriceReader;
use crate::types::{now_ms, AccountId, TimestampMs};
use uuid::Uuid;

/// Input to the internal submit path. Mirrors `SubmitOptionOrderInput`
/// shape without the write-auth / signature material — this service
/// is internal to the process and is never reached by an unauthed
/// caller.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SubmitPerpOrderInput {
    pub account: AccountId,
    /// PERPS-SUBACCOUNTS-ENGINE-ROUTING-V1 — the subaccount this order
    /// belongs to. Callers of the internal engine that predate this
    /// milestone default to `1`. Cross-subaccount reduce / cancel /
    /// liquidation is refused at the store layer.
    pub subaccount_id: u32,
    pub market_id: String,
    pub side: PerpOrderSide,
    pub price_1e8: u128,
    pub size_1e8: u128,
    pub time_in_force: PerpTimeInForce,
    pub post_only: bool,
    pub reduce_only: bool,
    /// Isolated collateral posted with this order. Ignored on
    /// reduce-only fills. For opens/increases, must be >= the initial
    /// margin requirement at the order's own price.
    pub isolated_margin_1e8: u128,
    pub client_order_id: Option<String>,
}

/// Outcome of a successful submit — the (possibly resting) order row
/// and any fills that landed as taker.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SubmitPerpOrderOutcome {
    pub order: PerpOrder,
    pub fills: Vec<PerpFill>,
}

/// A planned fill computed in the read-only match phase. Applied to
/// the store during the commit phase if TIF checks pass.
#[derive(Clone, Debug)]
struct PlannedFill {
    maker_order_id: Uuid,
    maker_account: AccountId,
    price_1e8: u128,
    size_1e8: u128,
}

/// Submit a Perps order into the internal execution engine.
///
/// This is a pure Rust function against explicit stores; it does not
/// touch on-chain state, the public router, or any signature layer.
/// The `price_reader` is used for stale-price gating; pass an
/// in-memory reader in tests.
pub async fn submit_perp_order_internal<P: PerpOraclePriceReader + ?Sized>(
    cfg: &PerpsReadConfig,
    order_store: &mut PerpOrderStore,
    positions_store: &mut PerpPositionsStore,
    price_reader: &P,
    input: SubmitPerpOrderInput,
) -> Result<SubmitPerpOrderOutcome> {
    validate_input_basics(&input)?;
    validate_tif_combinations(&input)?;
    let market = cfg
        .market_by_symbol(&input.market_id)
        .ok_or_else(|| BackendError::PerpsMarketNotFound(input.market_id.clone()))?;

    // Pre-fill risk gate: opens/increases require a fresh mark price.
    // Reduces don't (they only decrease exposure, so we don't need
    // the mark for margin math). This matches the milestone brief.
    let position_side_after = input.side.position_side();
    let is_reduce = input.reduce_only
        || positions_store
            .get_active(&input.account, input.subaccount_id, &input.market_id)
            .filter(|p| p.side != position_side_after)
            .is_some();
    if !is_reduce {
        // PERPS-MARGIN-ORACLE-RISK-V1 — layered pre-submit gate:
        //   * fresh oracle mark (existing guard, unchanged);
        //   * deviation guard between index + mark;
        //   * per-market risk caps (order size / notional / subaccount
        //     notional / open interest).
        ensure_mark_fresh(cfg, price_reader, market).await?;
        ensure_mark_within_deviation(cfg, price_reader, market).await?;
        enforce_pre_submit_risk_caps(market, positions_store, &input)?;
    }

    // Now insert the order (status=Open). The store dedupes by
    // client_order_id, so a duplicate here surfaces before we do any
    // matching or position work.
    let now = now_ms();
    let mut order = PerpOrder::new(
        input.account.clone(),
        input.subaccount_id,
        input.market_id.clone(),
        input.side,
        input.price_1e8,
        input.size_1e8,
        input.time_in_force,
        input.post_only,
        input.reduce_only,
        input.isolated_margin_1e8,
        input.client_order_id.clone(),
        now,
    );
    let order_id = order.id;
    let inserted = order_store.insert_order(order.clone())?;
    order = inserted;

    // Plan the fills against resting counterparties. This phase is
    // read-only over the store — nothing mutates until we commit.
    let planned = plan_fills(order_store, &order)?;

    // Post-only: no fill allowed.
    if order.post_only && !planned.is_empty() {
        mark_order_terminal(
            order_store,
            order_id,
            PerpOrderStatus::Rejected,
            reason::POST_ONLY_WOULD_MATCH,
            "post-only order would immediately match",
            reason::SOURCE_MATCHING_POLICY,
            now,
        )?;
        return Err(BackendError::PerpPostOnlyWouldMatch);
    }

    // FOK: must be entirely fillable at submit.
    let planned_total: u128 = planned.iter().map(|f| f.size_1e8).sum();
    if order.time_in_force == PerpTimeInForce::Fok && planned_total < order.size_1e8 {
        mark_order_terminal(
            order_store,
            order_id,
            PerpOrderStatus::Rejected,
            reason::FOK_NOT_FILLABLE,
            "fill-or-kill order not fully fillable at submit",
            reason::SOURCE_MATCHING_POLICY,
            now,
        )?;
        return Err(BackendError::PerpFokNotFillable);
    }

    // Reduce-only pre-check: fills must not exceed the existing
    // opposite-side position and must not switch net direction.
    if order.reduce_only {
        enforce_reduce_only(positions_store, &order, planned_total)?;
    }

    // Commit: apply each planned fill to both accounts. Every fill
    // is a paired mutation over positions_store; if a position
    // mutation ever errors mid-loop we surface the error but leave
    // the partial state (the caller sees the error via the return —
    // an internal execution surface can inspect the store). This
    // mirrors options behaviour.
    let mut fills: Vec<PerpFill> = Vec::with_capacity(planned.len());
    let mut taker_remaining = order.size_1e8;
    for plan in &planned {
        let taker_side = order.side;
        // Self-trade rejection: the account is proving both sides
        // of the fill. In V1 we reject rather than crossing the
        // book internally.
        if plan.maker_account == order.account {
            mark_order_terminal(
                order_store,
                order_id,
                PerpOrderStatus::Rejected,
                reason::SELF_TRADE,
                "self-trade rejected",
                reason::SOURCE_MATCHING_POLICY,
                now,
            )?;
            return Err(BackendError::PerpSelfTrade);
        }
        // Update the maker's row first: reduce their remaining and
        // bump their filled/status.
        order_store.update(plan.maker_order_id, now, |maker| {
            let new_remaining = maker.remaining_size_1e8.saturating_sub(plan.size_1e8);
            maker.filled_size_1e8 = maker.filled_size_1e8.saturating_add(plan.size_1e8);
            maker.remaining_size_1e8 = new_remaining;
            if new_remaining == 0 {
                maker.status = PerpOrderStatus::Filled;
                maker.terminal_reason_code = Some(reason::FILLED.to_string());
                maker.terminal_reason_source = Some(reason::SOURCE_MATCHING_POLICY.to_string());
            } else {
                maker.status = PerpOrderStatus::PartiallyFilled;
            }
            Ok(())
        })?;

        // Compute per-fill isolated margin for both sides:
        //   * taker: pro-rated share of `order.isolated_margin_1e8`.
        //   * maker: pro-rated share of the maker's isolated margin.
        // Reduce-only fills pass 0 (the applicator ignores it).
        let taker_margin_share = pro_rate(order.isolated_margin_1e8, plan.size_1e8, order.size_1e8);
        // Look up the fresh maker row for the up-to-date margin.
        let maker_after = order_store
            .get(plan.maker_order_id)
            .ok_or_else(|| BackendError::PerpOrderNotFound(plan.maker_order_id.to_string()))?;
        let maker_margin_share = pro_rate(
            maker_after.isolated_margin_1e8,
            plan.size_1e8,
            maker_after.size_1e8.max(1),
        );

        // Apply taker fill.
        apply_perp_fill_for_account(
            positions_store,
            market,
            PerpFillInput {
                account: order.account.clone(),
                subaccount_id: order.subaccount_id,
                market_id: order.market_id.clone(),
                side: taker_side.position_side(),
                size_1e8: plan.size_1e8,
                price_1e8: plan.price_1e8,
                margin_1e8: if order.reduce_only {
                    0
                } else {
                    taker_margin_share
                },
            },
        )?;

        // Apply maker fill. The maker's side is the ORDER side
        // (opposite of the taker) mapped to its position semantics.
        let maker_side = taker_side.opposite();
        apply_perp_fill_for_account(
            positions_store,
            market,
            PerpFillInput {
                account: plan.maker_account.clone(),
                subaccount_id: maker_after.subaccount_id,
                market_id: order.market_id.clone(),
                side: maker_side.position_side(),
                size_1e8: plan.size_1e8,
                price_1e8: plan.price_1e8,
                margin_1e8: if maker_after.reduce_only {
                    0
                } else {
                    maker_margin_share
                },
            },
        )?;

        let fill = PerpFill {
            id: Uuid::new_v4(),
            market_id: order.market_id.clone(),
            taker_order_id: order.id,
            maker_order_id: plan.maker_order_id,
            taker_account: order.account.clone(),
            maker_account: plan.maker_account.clone(),
            taker_subaccount_id: order.subaccount_id,
            maker_subaccount_id: maker_after.subaccount_id,
            taker_side,
            price_1e8: plan.price_1e8,
            size_1e8: plan.size_1e8,
            created_at_ms: now,
        };
        order_store.insert_fill(fill.clone());
        fills.push(fill);
        taker_remaining = taker_remaining.saturating_sub(plan.size_1e8);
    }

    // Bump the taker's post-match state.
    let taker_final_status = if taker_remaining == 0 {
        PerpOrderStatus::Filled
    } else if !fills.is_empty() {
        // Some fills happened but remainder is unfilled.
        match order.time_in_force {
            PerpTimeInForce::Ioc => PerpOrderStatus::Cancelled,
            PerpTimeInForce::Gtc => PerpOrderStatus::PartiallyFilled,
            PerpTimeInForce::Fok => {
                // Should be unreachable — the FOK gate above rejects
                // any partial. Kept as `Cancelled` for safety.
                PerpOrderStatus::Cancelled
            }
        }
    } else {
        // No fills at all.
        match order.time_in_force {
            PerpTimeInForce::Gtc => PerpOrderStatus::Open,
            PerpTimeInForce::Ioc | PerpTimeInForce::Fok => PerpOrderStatus::Cancelled,
        }
    };
    let taker_reason = match (taker_final_status, order.time_in_force) {
        (PerpOrderStatus::Filled, _) => Some((reason::FILLED, reason::SOURCE_MATCHING_POLICY)),
        (PerpOrderStatus::Cancelled, PerpTimeInForce::Ioc) => Some((
            reason::IOC_UNFILLED_REMAINDER,
            reason::SOURCE_MATCHING_POLICY,
        )),
        (PerpOrderStatus::Cancelled, PerpTimeInForce::Fok) => {
            Some((reason::FOK_NOT_FILLABLE, reason::SOURCE_MATCHING_POLICY))
        }
        _ => None,
    };
    let updated = order_store.update(order_id, now, |o| {
        o.filled_size_1e8 = o.size_1e8.saturating_sub(taker_remaining);
        o.remaining_size_1e8 = taker_remaining;
        o.status = taker_final_status;
        if let Some((code, source)) = taker_reason {
            o.terminal_reason_code = Some(code.to_string());
            o.terminal_reason_source = Some(source.to_string());
        }
        Ok(())
    })?;

    Ok(SubmitPerpOrderOutcome {
        order: updated,
        fills,
    })
}

/// Cancel a resting Perps order.
pub fn cancel_perp_order_internal(
    order_store: &mut PerpOrderStore,
    order_id: Uuid,
    caller: &AccountId,
) -> Result<PerpOrder> {
    let existing = order_store
        .get(order_id)
        .ok_or_else(|| BackendError::PerpOrderNotFound(order_id.to_string()))?;
    if existing.account.0.to_lowercase() != caller.0.to_lowercase() {
        return Err(BackendError::PerpInvalidOrderState(
            "cancel caller does not own the order".to_string(),
        ));
    }
    if existing.status.is_terminal() {
        return Err(BackendError::PerpInvalidOrderState(format!(
            "order is already terminal ({})",
            existing.status.as_str()
        )));
    }
    let now = now_ms();
    order_store.update(order_id, now, |o| {
        o.status = PerpOrderStatus::Cancelled;
        o.terminal_reason_code = Some(reason::USER_CANCELLED.to_string());
        o.terminal_reason_source = Some(reason::SOURCE_REQUEST_VALIDATION.to_string());
        Ok(())
    })
}

pub fn list_perp_orders_for_account(
    order_store: &PerpOrderStore,
    account: &AccountId,
) -> Vec<PerpOrder> {
    order_store.list_orders_for_account(account)
}

pub fn list_perp_fills_for_account(
    order_store: &PerpOrderStore,
    account: &AccountId,
) -> Vec<PerpFill> {
    order_store.list_fills_for_account(account)
}

// ---------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------

fn validate_input_basics(input: &SubmitPerpOrderInput) -> Result<()> {
    if input.size_1e8 == 0 {
        return Err(BackendError::PerpZeroSize);
    }
    if input.price_1e8 == 0 {
        return Err(BackendError::PerpZeroPrice);
    }
    Ok(())
}

fn validate_tif_combinations(input: &SubmitPerpOrderInput) -> Result<()> {
    if input.post_only && input.time_in_force != PerpTimeInForce::Gtc {
        return Err(BackendError::PerpInvalidTifCombination(format!(
            "post_only requires GTC; got {}",
            input.time_in_force.as_str()
        )));
    }
    Ok(())
}

async fn ensure_mark_fresh<P: PerpOraclePriceReader + ?Sized>(
    cfg: &PerpsReadConfig,
    price_reader: &P,
    market: &PerpsReadMarket,
) -> Result<()> {
    let read = price_reader
        .read_price(market)
        .await
        .map_err(|err| BackendError::PerpMarkPriceUnavailable(err.to_string()))?;
    if !read.ok || read.price_1e8 == 0 {
        return Err(BackendError::PerpMarkPriceUnavailable(format!(
            "oracle reported ok=false or zero price for {}",
            market.symbol
        )));
    }
    let now = now_ms();
    let updated_ms = read.updated_at_ms();
    let stale_after_ms = (cfg.stale_after_sec as i64).saturating_mul(1000);
    if updated_ms == 0 || now.saturating_sub(updated_ms) > stale_after_ms {
        return Err(BackendError::PerpMarkPriceUnavailable(format!(
            "mark price is stale for {}",
            market.symbol
        )));
    }
    Ok(())
}

/// PERPS-MARGIN-ORACLE-RISK-V1 — deviation guard between index and
/// mark. V1 OracleRouter returns `mark == index` from `getPriceSafe`,
/// so this call passes deterministically today. The guard is present
/// so a future divergence (e.g. a per-market mark smoother) rejects
/// anything above threshold rather than silently letting stale-shape
/// prices settle risk-taking mutations.
///
/// Reads a single price and compares the mark against the index. In
/// V1 they're the same field on `PerpPriceRead`, so the observed
/// deviation is always 0 bps → pass. Kept as a first-class function
/// so tests can freeze the shape.
async fn ensure_mark_within_deviation<P: PerpOraclePriceReader + ?Sized>(
    cfg: &PerpsReadConfig,
    price_reader: &P,
    market: &PerpsReadMarket,
) -> Result<()> {
    let read = price_reader
        .read_price(market)
        .await
        .map_err(|err| BackendError::PerpMarkPriceUnavailable(err.to_string()))?;
    let index = read.price_1e8;
    let mark = read.price_1e8; // V1: mark == index at OracleRouter.
    let observed_bps = deviation_bps(index, mark);
    if observed_bps > cfg.oracle_max_deviation_bps {
        return Err(BackendError::PerpOracleDeviationExceeded(format!(
            "{}: observed {}bps > max {}bps",
            market.symbol, observed_bps, cfg.oracle_max_deviation_bps
        )));
    }
    Ok(())
}

/// Absolute deviation between two 1e8 prices, expressed in basis
/// points relative to `index`. Returns 0 when index is 0 (no
/// meaningful ratio; the freshness guard already caught that).
pub fn deviation_bps(index_1e8: u128, mark_1e8: u128) -> u32 {
    if index_1e8 == 0 {
        return 0;
    }
    let diff = if mark_1e8 >= index_1e8 {
        mark_1e8 - index_1e8
    } else {
        index_1e8 - mark_1e8
    };
    // (diff / index) in bps, saturating at u32::MAX so anything wild
    // still reports as "way over".
    let bps_u128 = diff
        .saturating_mul(crate::perps::margin::BPS)
        .checked_div(index_1e8)
        .unwrap_or(0);
    if bps_u128 > u32::MAX as u128 {
        u32::MAX
    } else {
        bps_u128 as u32
    }
}

/// PERPS-MARGIN-ORACLE-RISK-V1 — market-side risk caps applied to a
/// candidate submit BEFORE the order is inserted into the book and
/// BEFORE any margin math persists a position.
///
/// * Order size cap: `input.size_1e8 <= market.max_order_size_1e8`.
/// * Order notional cap: `size * price / 1e8 <= market.max_order_notional_1e8`.
/// * Per-subaccount notional cap: after the hypothetical fill, the
///   sum of `size * entry` for every active position for this
///   (wallet, subaccount) on this market must not exceed
///   `market.max_subaccount_notional_1e8`.
/// * Open-interest cap: after the hypothetical fill, market-wide
///   summed open size on this market must not exceed
///   `market.max_open_interest_1e8`.
///
/// Reduce-only paths skip these checks — `is_reduce` in the caller
/// already routes reduces around this gate. Cross-subaccount netting
/// is never assumed: caps are scoped strictly to `(account, subaccount)`.
pub(crate) fn enforce_pre_submit_risk_caps(
    market: &PerpsReadMarket,
    positions_store: &crate::perps::positions::PerpPositionsStore,
    input: &SubmitPerpOrderInput,
) -> Result<()> {
    // (1) order size.
    if let Some(cap) = market.max_order_size_1e8 {
        if input.size_1e8 > cap {
            return Err(BackendError::PerpOrderSizeCap(format!(
                "{}: order size {} > max {}",
                market.symbol, input.size_1e8, cap
            )));
        }
    }
    // (2) order notional.
    let order_notional = crate::perps::margin::notional_1e8(input.size_1e8, input.price_1e8);
    if let Some(cap) = market.max_order_notional_1e8 {
        if order_notional > cap {
            return Err(BackendError::PerpOrderNotionalCap(format!(
                "{}: order notional {} > max {}",
                market.symbol, order_notional, cap
            )));
        }
    }
    // (3) per-subaccount notional across this market.
    if let Some(cap) = market.max_subaccount_notional_1e8 {
        let existing_subaccount_notional: u128 = positions_store
            .list_for_account_and_subaccount(&input.account, input.subaccount_id)
            .iter()
            .filter(|p| {
                p.status == crate::perps::positions::PerpPositionStatus::Open
                    && p.market_id == market.symbol
            })
            .map(|p| crate::perps::margin::notional_1e8(p.size_1e8, p.entry_price_1e8))
            .sum();
        let after = existing_subaccount_notional.saturating_add(order_notional);
        if after > cap {
            return Err(BackendError::PerpSubaccountNotionalCap(format!(
                "{}: post-fill subaccount notional {} > max {}",
                market.symbol, after, cap
            )));
        }
    }
    // (4) market-wide open interest.
    if let Some(cap) = market.max_open_interest_1e8 {
        let current_oi: u128 = positions_store
            .by_id_values_iter()
            .iter()
            .filter(|p| {
                p.status == crate::perps::positions::PerpPositionStatus::Open
                    && p.market_id == market.symbol
            })
            .map(|p| p.size_1e8)
            .sum();
        let after = current_oi.saturating_add(input.size_1e8);
        if after > cap {
            return Err(BackendError::PerpOpenInterestCap(format!(
                "{}: post-fill open interest {} > max {}",
                market.symbol, after, cap
            )));
        }
    }
    Ok(())
}

fn plan_fills(store: &PerpOrderStore, taker: &PerpOrder) -> Result<Vec<PlannedFill>> {
    let mut plan = Vec::new();
    let mut taker_remaining = taker.size_1e8;
    let counterparties = counterparties_for(store, &taker.market_id, taker.side);
    for maker in counterparties {
        if taker_remaining == 0 {
            break;
        }
        if !crosses(taker.side, taker.price_1e8, maker.price_1e8) {
            break;
        }
        let fill = taker_remaining.min(maker.remaining_size_1e8);
        if fill == 0 {
            continue;
        }
        plan.push(PlannedFill {
            maker_order_id: maker.id,
            maker_account: maker.account.clone(),
            price_1e8: maker.price_1e8,
            size_1e8: fill,
        });
        taker_remaining -= fill;
    }
    Ok(plan)
}

fn enforce_reduce_only(
    positions_store: &PerpPositionsStore,
    order: &PerpOrder,
    planned_total: u128,
) -> Result<()> {
    let opposite = order.side.position_side().opposite();
    let existing = positions_store
        .get_active(&order.account, order.subaccount_id, &order.market_id)
        .ok_or(BackendError::PerpReduceOnlyViolation)?;
    if existing.side != opposite {
        return Err(BackendError::PerpReduceOnlyViolation);
    }
    if planned_total > existing.size_1e8 {
        return Err(BackendError::PerpReduceOnlyViolation);
    }
    Ok(())
}

fn mark_order_terminal(
    order_store: &mut PerpOrderStore,
    order_id: Uuid,
    status: PerpOrderStatus,
    code: &str,
    message: &str,
    source: &str,
    now: TimestampMs,
) -> Result<PerpOrder> {
    order_store.update(order_id, now, |o| {
        o.status = status;
        o.terminal_reason_code = Some(code.to_string());
        o.terminal_reason_message = Some(message.to_string());
        o.terminal_reason_source = Some(source.to_string());
        Ok(())
    })
}

fn pro_rate(total: u128, part: u128, whole: u128) -> u128 {
    if whole == 0 {
        return 0;
    }
    // Ceil-div so the accumulated pro-rated share never under-collateralises.
    let numerator = total.saturating_mul(part);
    numerator.div_ceil(whole)
}
