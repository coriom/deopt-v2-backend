//! PERPS-PG-EXECUTION-INTEGRATION-V1 — PG-backed internal execution.
//!
//! Mirrors the semantics of `submit_perp_order_internal` /
//! `cancel_perp_order_internal` (in-memory path) but writes to
//! Postgres under a single transaction. **Public HTTP mutation routes
//! remain fail-closed** — this module is called only by the internal
//! dispatcher (`crate::perps::service::submit_perp_order_via_state`)
//! when `state.repository.is_some()`.
//!
//! Transaction shape:
//!   1. Begin.
//!   2. Insert taker order row (status='open', filled=0).
//!   3. Load crossable maker orders via `list_open_perp_orders_tx`.
//!   4. Compute the deterministic match plan (pure).
//!   5. Apply TIF gates (post-only, FOK). If any rejects, ROLLBACK
//!      by dropping the `tx` — no COMMIT ever runs.
//!   6. Reduce-only pre-check via `get_active_perp_position_tx`.
//!   7. For each planned fill:
//!        - Self-trade check → rollback.
//!        - Update maker order via `update_perp_order_tx`.
//!        - Insert `perp_fills` row.
//!        - Upsert taker + maker positions (`insert_perp_position_tx`
//!          or `update_perp_position_tx` or `close_active_perp_position_tx`).
//!   8. Update taker order final state (status, remaining, terminal reason).
//!   9. Commit.
//!  10. Emit lifecycle frames — AFTER the commit succeeds.
//!
//! Invariant: no lifecycle frame fires unless the underlying rows are
//! durably on disk.

use crate::api::public_ws::LifecycleEventSender;
use crate::db::repository as repo;
use crate::error::{BackendError, Result};
use crate::perps::config::PerpsReadConfig;
use crate::perps::execution::SubmitPerpOrderInput;
use crate::perps::fills::PerpFillInput;
use crate::perps::lifecycle::{
    emit_perp_fill_lifecycle, emit_perp_order_lifecycle, emit_perp_position_lifecycle,
    emit_perp_rejection_lifecycle,
};
use crate::perps::margin;
use crate::perps::orderbook::crosses;
use crate::perps::orders::{
    reason, PerpFill, PerpOrder, PerpOrderSide, PerpOrderStatus, PerpTimeInForce,
};
use crate::perps::positions::{new_position_skeleton, PerpPosition, PerpPositionStatus};
use crate::perps::price_reader::PerpOraclePriceReader;
use crate::perps::SubmitPerpOrderOutcome;
use crate::types::{now_ms, AccountId, TimestampMs};
use uuid::Uuid;

/// PG-backed twin of `submit_perp_order_internal`. Requires
/// `state.repository.is_some()`; the dispatcher never calls this
/// otherwise.
///
/// PERPS-PG-HARNESS-AND-REJECTION-EMIT-V1 — on a classified error
/// (see `classify_perp_rejection`), this wrapper emits a
/// `PerpOrderRejected` lifecycle frame AFTER the tx has rolled back
/// (dropping `tx` inside the inner function). The frame carries only
/// the trader-facing scalar fields from the original submit input —
/// never a signature, nonce, envelope, header, or bearer token.
pub async fn submit_perp_order_via_repository<P: PerpOraclePriceReader + ?Sized>(
    cfg: &PerpsReadConfig,
    repository: &crate::db::PgRepository,
    price_reader: &P,
    lifecycle_sender: &LifecycleEventSender,
    input: SubmitPerpOrderInput,
) -> Result<SubmitPerpOrderOutcome> {
    // Snapshot input for the rejection frame BEFORE we move it into
    // the inner call. The snapshot carries only scalar fields — no
    // secrets exist on `SubmitPerpOrderInput` to begin with (auth
    // material lives at the HTTP layer).
    let rejection_snapshot = RejectionSnapshot::from_input(&input);
    let result = submit_perp_order_via_repository_inner(
        cfg,
        repository,
        price_reader,
        lifecycle_sender,
        input,
    )
    .await;
    if let Err(err) = &result {
        rejection_snapshot.emit_if_classified(lifecycle_sender, err);
    }
    result
}

/// A lightweight copy of the scalar fields from `SubmitPerpOrderInput`,
/// kept alive across a possible submit failure so we can emit
/// `PerpOrderRejected` with the same fields the trader submitted.
/// `pub(crate)` so the in-memory dispatcher in `service.rs` reuses
/// the exact same snapshot+emit pattern.
pub(crate) struct RejectionSnapshot {
    account: AccountId,
    market_id: String,
    side: PerpOrderSide,
    price_1e8: u128,
    size_1e8: u128,
    time_in_force: PerpTimeInForce,
    post_only: bool,
    reduce_only: bool,
    client_order_id: Option<String>,
}

impl RejectionSnapshot {
    pub(crate) fn from_input(input: &SubmitPerpOrderInput) -> Self {
        Self {
            account: input.account.clone(),
            market_id: input.market_id.clone(),
            side: input.side,
            price_1e8: input.price_1e8,
            size_1e8: input.size_1e8,
            time_in_force: input.time_in_force,
            post_only: input.post_only,
            reduce_only: input.reduce_only,
            client_order_id: input.client_order_id.clone(),
        }
    }

    pub(crate) fn emit_if_classified(&self, sender: &LifecycleEventSender, error: &BackendError) {
        emit_perp_rejection_lifecycle(
            sender,
            &self.account,
            error,
            Some(self.market_id.clone()),
            Some(self.side),
            Some(self.price_1e8),
            Some(self.size_1e8),
            Some(self.time_in_force.as_str().to_string()),
            Some(self.post_only),
            Some(self.reduce_only),
            self.client_order_id.clone(),
        );
    }
}

async fn submit_perp_order_via_repository_inner<P: PerpOraclePriceReader + ?Sized>(
    cfg: &PerpsReadConfig,
    repository: &crate::db::PgRepository,
    price_reader: &P,
    lifecycle_sender: &LifecycleEventSender,
    input: SubmitPerpOrderInput,
) -> Result<SubmitPerpOrderOutcome> {
    // Pre-transaction validation: cheap, deterministic, no writes.
    validate_input_basics(&input)?;
    validate_tif_combinations(&input)?;
    let market = cfg
        .market_by_symbol(&input.market_id)
        .ok_or_else(|| BackendError::PerpsMarketNotFound(input.market_id.clone()))?
        .clone();

    // Mark-price staleness gate for opens/increases. We compute it
    // outside the transaction to keep the RPC call off the DB
    // connection.
    let position_side_after = input.side.position_side();
    // We can't know `is_reduce` before opening the tx (need the
    // pre-fill position), so re-check inside. For the staleness gate
    // we conservatively probe here when NOT explicitly reduce-only.
    if !input.reduce_only {
        ensure_mark_fresh(cfg, price_reader, &market).await?;
    }

    // Snapshot values needed inside the transaction.
    let now = now_ms();
    let taker_seed = PerpOrder::new(
        input.account.clone(),
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

    // ---- Transaction opens here ----------------------------------
    let mut tx = repository.begin().await?;

    // 1. Insert taker order (status=Open). This claims the client
    //    order id via the unique index; a violation bubbles as
    //    `PerpDuplicateClientOrderId`.
    repo::insert_perp_order_tx(&mut tx, &taker_seed).await?;

    // 2. Load crossable maker orders. `list_open_perp_orders_tx`
    //    returns rows sorted by price ASC, time ASC. For a Buy taker
    //    we want ascending asks; for a Sell taker we filter and sort
    //    the bid side high-to-low.
    let candidates = repo::list_open_perp_orders_tx(&mut tx, &input.market_id).await?;
    let counterparties = filter_counterparties(candidates, taker_seed.side);

    // 3. Plan the match (pure).
    let planned = plan_fills_pure(&taker_seed, &counterparties);

    // 4. Post-only + FOK gates BEFORE any position mutation.
    if taker_seed.post_only && !planned.is_empty() {
        return Err(BackendError::PerpPostOnlyWouldMatch);
    }
    let planned_total: u128 = planned.iter().map(|f| f.size_1e8).sum();
    if taker_seed.time_in_force == PerpTimeInForce::Fok && planned_total < taker_seed.size_1e8 {
        return Err(BackendError::PerpFokNotFillable);
    }

    // 5. Reduce-only pre-check.
    if taker_seed.reduce_only {
        enforce_reduce_only_pg(
            &mut tx,
            &taker_seed.account,
            &taker_seed.market_id,
            position_side_after.opposite(),
            planned_total,
        )
        .await?;
    }

    // 6. Commit each planned fill: update maker order, insert fill,
    //    upsert taker + maker positions.
    let mut fills: Vec<PerpFill> = Vec::with_capacity(planned.len());
    let mut taker_remaining = taker_seed.size_1e8;
    // Track positions we've updated so we can emit lifecycle after
    // commit. Deduped by (account, market) via a Vec of the final
    // shape.
    let mut post_commit_positions: Vec<PerpPosition> = Vec::new();

    for plan in &planned {
        if plan
            .maker_account
            .0
            .eq_ignore_ascii_case(&taker_seed.account.0)
        {
            return Err(BackendError::PerpSelfTrade);
        }

        // Load fresh maker row and mutate.
        let maker = repo::get_perp_order_tx(&mut tx, plan.maker_order_id)
            .await?
            .ok_or_else(|| BackendError::PerpOrderNotFound(plan.maker_order_id.to_string()))?;
        let mut maker_updated = maker.clone();
        let maker_new_remaining = maker_updated
            .remaining_size_1e8
            .saturating_sub(plan.size_1e8);
        maker_updated.filled_size_1e8 = maker_updated.filled_size_1e8.saturating_add(plan.size_1e8);
        maker_updated.remaining_size_1e8 = maker_new_remaining;
        if maker_new_remaining == 0 {
            maker_updated.status = PerpOrderStatus::Filled;
            maker_updated.terminal_reason_code = Some(reason::FILLED.to_string());
            maker_updated.terminal_reason_source = Some(reason::SOURCE_MATCHING_POLICY.to_string());
        } else {
            maker_updated.status = PerpOrderStatus::PartiallyFilled;
        }
        maker_updated.updated_at_ms = now;
        repo::update_perp_order_tx(&mut tx, &maker_updated).await?;

        // Pro-rate the taker & maker isolated margin to this fill.
        let taker_margin_share = pro_rate(
            taker_seed.isolated_margin_1e8,
            plan.size_1e8,
            taker_seed.size_1e8,
        );
        let maker_margin_share = pro_rate(
            maker_updated.isolated_margin_1e8,
            plan.size_1e8,
            maker_updated.size_1e8.max(1),
        );

        // Apply taker fill against the DB position.
        let taker_position = apply_perp_fill_pg(
            &mut tx,
            &market,
            PerpFillInput {
                account: taker_seed.account.clone(),
                market_id: taker_seed.market_id.clone(),
                side: taker_seed.side.position_side(),
                size_1e8: plan.size_1e8,
                price_1e8: plan.price_1e8,
                margin_1e8: if taker_seed.reduce_only {
                    0
                } else {
                    taker_margin_share
                },
            },
            now,
        )
        .await?;

        // Apply maker fill against the DB position.
        let maker_position = apply_perp_fill_pg(
            &mut tx,
            &market,
            PerpFillInput {
                account: plan.maker_account.clone(),
                market_id: taker_seed.market_id.clone(),
                side: taker_seed.side.opposite().position_side(),
                size_1e8: plan.size_1e8,
                price_1e8: plan.price_1e8,
                margin_1e8: if maker_updated.reduce_only {
                    0
                } else {
                    maker_margin_share
                },
            },
            now,
        )
        .await?;

        // Persist fill row.
        let fill = PerpFill {
            id: Uuid::new_v4(),
            market_id: taker_seed.market_id.clone(),
            taker_order_id: taker_seed.id,
            maker_order_id: plan.maker_order_id,
            taker_account: taker_seed.account.clone(),
            maker_account: plan.maker_account.clone(),
            taker_side: taker_seed.side,
            price_1e8: plan.price_1e8,
            size_1e8: plan.size_1e8,
            created_at_ms: now,
        };
        repo::insert_perp_fill_tx(&mut tx, &fill).await?;
        fills.push(fill);
        taker_remaining = taker_remaining.saturating_sub(plan.size_1e8);
        post_commit_positions.push(taker_position);
        post_commit_positions.push(maker_position);
    }

    // 7. Bump taker order final state.
    let taker_final_status = if taker_remaining == 0 {
        PerpOrderStatus::Filled
    } else if !fills.is_empty() {
        match taker_seed.time_in_force {
            PerpTimeInForce::Ioc => PerpOrderStatus::Cancelled,
            PerpTimeInForce::Gtc => PerpOrderStatus::PartiallyFilled,
            PerpTimeInForce::Fok => PerpOrderStatus::Cancelled,
        }
    } else {
        match taker_seed.time_in_force {
            PerpTimeInForce::Gtc => PerpOrderStatus::Open,
            PerpTimeInForce::Ioc | PerpTimeInForce::Fok => PerpOrderStatus::Cancelled,
        }
    };
    let taker_reason = match (taker_final_status, taker_seed.time_in_force) {
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
    let mut taker_updated = taker_seed.clone();
    taker_updated.filled_size_1e8 = taker_updated.size_1e8.saturating_sub(taker_remaining);
    taker_updated.remaining_size_1e8 = taker_remaining;
    taker_updated.status = taker_final_status;
    if let Some((code, source)) = taker_reason {
        taker_updated.terminal_reason_code = Some(code.to_string());
        taker_updated.terminal_reason_source = Some(source.to_string());
    }
    taker_updated.updated_at_ms = now;
    let final_taker = repo::update_perp_order_tx(&mut tx, &taker_updated).await?;

    // 8. COMMIT — nothing has been visible to concurrent readers
    //    until this line succeeds. Everything above is atomic.
    tx.commit()
        .await
        .map_err(|e| BackendError::Persistence(e.to_string()))?;

    // 9. Emit lifecycle frames AFTER commit. If the sender's channel
    //    has no receiver, the emitter logs and drops — never fails
    //    the parent operation.
    emit_perp_order_lifecycle(lifecycle_sender, &final_taker);
    for fill in &fills {
        emit_perp_fill_lifecycle(lifecycle_sender, fill);
    }
    for position in &post_commit_positions {
        emit_perp_position_lifecycle(lifecycle_sender, position);
    }

    Ok(SubmitPerpOrderOutcome {
        order: final_taker,
        fills,
    })
}

/// PG-backed twin of `cancel_perp_order_internal`. Wraps the read +
/// update in a transaction so the caller sees a consistent post-state.
pub async fn cancel_perp_order_via_repository(
    repository: &crate::db::PgRepository,
    lifecycle_sender: &LifecycleEventSender,
    order_id: Uuid,
    caller: &AccountId,
) -> Result<PerpOrder> {
    let mut tx = repository.begin().await?;
    let existing = repo::get_perp_order_tx(&mut tx, order_id)
        .await?
        .ok_or_else(|| BackendError::PerpOrderNotFound(order_id.to_string()))?;
    if !existing.account.0.eq_ignore_ascii_case(&caller.0) {
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
    let mut updated = existing.clone();
    updated.status = PerpOrderStatus::Cancelled;
    updated.terminal_reason_code = Some(reason::USER_CANCELLED.to_string());
    updated.terminal_reason_source = Some(reason::SOURCE_REQUEST_VALIDATION.to_string());
    updated.updated_at_ms = now;
    let final_order = repo::update_perp_order_tx(&mut tx, &updated).await?;
    tx.commit()
        .await
        .map_err(|e| BackendError::Persistence(e.to_string()))?;
    emit_perp_order_lifecycle(lifecycle_sender, &final_order);
    Ok(final_order)
}

// ---------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------

struct PlannedFill {
    maker_order_id: Uuid,
    maker_account: AccountId,
    price_1e8: u128,
    size_1e8: u128,
}

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
    market: &crate::perps::config::PerpsReadMarket,
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

fn filter_counterparties(orders: Vec<PerpOrder>, taker_side: PerpOrderSide) -> Vec<PerpOrder> {
    let mut filtered: Vec<PerpOrder> = orders
        .into_iter()
        .filter(|o| o.side == taker_side.opposite())
        .collect();
    match taker_side {
        // list_open_perp_orders_tx returns price ASC. For a Buy
        // taker we want asks low-to-high already sorted; keep as-is.
        PerpOrderSide::Buy => {}
        // For a Sell taker we want bids high-to-low.
        PerpOrderSide::Sell => filtered.sort_by(|a, b| {
            b.price_1e8
                .cmp(&a.price_1e8)
                .then_with(|| a.created_at_ms.cmp(&b.created_at_ms))
        }),
    }
    filtered
}

fn plan_fills_pure(taker: &PerpOrder, counterparties: &[PerpOrder]) -> Vec<PlannedFill> {
    let mut out = Vec::new();
    let mut remaining = taker.size_1e8;
    for maker in counterparties {
        if remaining == 0 {
            break;
        }
        if !crosses(taker.side, taker.price_1e8, maker.price_1e8) {
            break;
        }
        let fill = remaining.min(maker.remaining_size_1e8);
        if fill == 0 {
            continue;
        }
        out.push(PlannedFill {
            maker_order_id: maker.id,
            maker_account: maker.account.clone(),
            price_1e8: maker.price_1e8,
            size_1e8: fill,
        });
        remaining -= fill;
    }
    out
}

async fn enforce_reduce_only_pg(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    account: &AccountId,
    market_id: &str,
    required_side: crate::perps::PerpSide,
    planned_total: u128,
) -> Result<()> {
    let existing = repo::get_active_perp_position_tx(tx, account, market_id)
        .await?
        .ok_or(BackendError::PerpReduceOnlyViolation)?;
    if existing.side != required_side {
        return Err(BackendError::PerpReduceOnlyViolation);
    }
    if planned_total > existing.size_1e8 {
        return Err(BackendError::PerpReduceOnlyViolation);
    }
    Ok(())
}

fn pro_rate(total: u128, part: u128, whole: u128) -> u128 {
    if whole == 0 {
        return 0;
    }
    let numerator = total.saturating_mul(part);
    numerator.div_ceil(whole)
}

/// PG twin of `apply_perp_fill_for_account`. Reads existing position,
/// applies the same open/increase/reduce/close math, and persists.
async fn apply_perp_fill_pg(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    market: &crate::perps::config::PerpsReadMarket,
    fill: PerpFillInput,
    now: TimestampMs,
) -> Result<PerpPosition> {
    if fill.size_1e8 == 0 {
        return Err(BackendError::PerpZeroSize);
    }
    if fill.price_1e8 == 0 {
        return Err(BackendError::PerpZeroPrice);
    }
    if fill.market_id != market.symbol {
        return Err(BackendError::Config(format!(
            "market row / fill symbol mismatch: {} vs {}",
            market.symbol, fill.market_id
        )));
    }
    let existing = repo::get_active_perp_position_tx(tx, &fill.account, &fill.market_id).await?;
    match existing {
        None => {
            if fill.margin_1e8 == 0 {
                return Err(BackendError::PerpZeroMargin);
            }
            let required = margin::initial_margin_requirement_1e8(
                fill.size_1e8,
                fill.price_1e8,
                market.max_leverage,
            );
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
                fill.market_id,
                fill.side,
                fill.size_1e8,
                fill.price_1e8,
                fill.margin_1e8,
            );
            let mut position = position;
            position.opened_at_ms = now;
            position.updated_at_ms = now;
            repo::insert_perp_position_tx(tx, &position).await?;
            Ok(position)
        }
        Some(existing_pos) if existing_pos.side == fill.side => {
            if fill.margin_1e8 == 0 {
                return Err(BackendError::PerpZeroMargin);
            }
            let new_size = existing_pos.size_1e8.saturating_add(fill.size_1e8);
            let new_entry = margin::weighted_average_entry_1e8(
                existing_pos.size_1e8,
                existing_pos.entry_price_1e8,
                fill.size_1e8,
                fill.price_1e8,
            );
            let new_margin = existing_pos.margin_1e8.saturating_add(fill.margin_1e8);
            let new_notional = margin::notional_1e8(new_size, new_entry);
            let required =
                margin::initial_margin_requirement_1e8(new_size, new_entry, market.max_leverage);
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
            let mut updated = existing_pos;
            updated.size_1e8 = new_size;
            updated.entry_price_1e8 = new_entry;
            updated.margin_1e8 = new_margin;
            updated.updated_at_ms = now;
            repo::update_perp_position_tx(tx, &updated).await
        }
        Some(existing_pos) => {
            // Opposite side → reduce or reject flip.
            if fill.size_1e8 > existing_pos.size_1e8 {
                return Err(BackendError::PerpPositionFlip);
            }
            let realized = margin::realized_pnl_1e8(
                existing_pos.entry_price_1e8,
                fill.price_1e8,
                fill.size_1e8,
                existing_pos.side,
            );
            let full_close = fill.size_1e8 == existing_pos.size_1e8;
            if full_close {
                let mut closed = repo::close_active_perp_position_tx(
                    tx,
                    &existing_pos.account,
                    &existing_pos.market_id,
                    now,
                )
                .await?;
                closed.realized_pnl_1e8 = closed.realized_pnl_1e8.saturating_add(realized);
                closed.status = PerpPositionStatus::Closed;
                // Persist the realized PnL patch.
                repo::update_perp_position_tx(tx, &closed).await
            } else {
                let new_size = existing_pos.size_1e8 - fill.size_1e8;
                let released = mul_div_floor(
                    existing_pos.margin_1e8,
                    fill.size_1e8,
                    existing_pos.size_1e8,
                );
                let new_margin = existing_pos.margin_1e8.saturating_sub(released);
                let mut updated = existing_pos;
                updated.size_1e8 = new_size;
                updated.margin_1e8 = new_margin;
                updated.realized_pnl_1e8 = updated.realized_pnl_1e8.saturating_add(realized);
                updated.updated_at_ms = now;
                repo::update_perp_position_tx(tx, &updated).await
            }
        }
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
    let lhs = notional_1e8;
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
