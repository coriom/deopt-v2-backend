//! PERPS-LIQUIDATION-PG-EXECUTION-V1 — durable Postgres liquidation.
//!
//! **Internal-only.** Public Perps mutation routes remain fail-closed;
//! liquidations are computed by the admin-gated tick endpoint
//! (`POST /admin/perps/liquidations/tick`) and read via
//! `GET /accounts/:address/perps/liquidations`. There is no public
//! mutation surface that reaches this module.
//!
//! Transaction shape (one per candidate position):
//!   1. Begin.
//!   2. Re-load the active position INSIDE the transaction — a
//!      concurrent close/liquidation would have flipped its status by
//!      now, and we honor that.
//!   3. Recompute eligibility against the same mark price prefetched
//!      out-of-band.
//!   4. If healthy → no state mutation, no event, no lifecycle.
//!   5. If mark unavailable → insert a `price_unavailable` event, emit
//!      `PerpPositionLiquidated{status=price_unavailable}` after commit,
//!      no position/order mutation.
//!   6. If liquidatable →
//!        a. `cancel_open_perp_orders_for_account_market_tx` — returns
//!           rows updated to `cancelled` with terminal reason
//!           `liquidated` / `liquidation_tick`.
//!        b. `liquidate_active_perp_position_tx` — flips
//!           `status='liquidated'`, adds realized PnL delta, stamps
//!           `closed_at_ms`.
//!        c. `insert_perp_liquidation_event_tx` — persists the event.
//!        d. Commit.
//!        e. AFTER commit, emit `PerpPositionUpdated` +
//!           `PerpOrderUpdated × N` + `PerpPositionLiquidated`.
//!
//! Invariants:
//!   * No lifecycle frame fires before a successful `tx.commit()`.
//!   * Already-liquidated (or otherwise non-`open`) positions produce
//!     no event and no state mutation — the tick is idempotent.
//!   * The tick uses one PG transaction per candidate so a single
//!     liquidation failure doesn't roll back a healthy prior liquidation
//!     in the same tick.

use crate::api::public_ws::{
    LifecycleChannel, LifecycleEvent, LifecycleEventSender, LifecyclePayload,
};
use crate::db::repository as repo;
use crate::db::PgRepository;
use crate::error::{BackendError, Result};
use crate::perps::config::{PerpsReadConfig, PerpsReadMarket};
use crate::perps::lifecycle::{emit_perp_order_lifecycle, emit_perp_position_lifecycle};
use crate::perps::liquidation::{
    build_price_unavailable_event_public, evaluate_perp_liquidation, liquidation_reason,
    LiquidationEvaluation, PerpLiquidationEvent, PerpLiquidationStatus,
    PerpLiquidationTickResponse,
};
use crate::perps::margin;
use crate::perps::orders::reason as order_reason;
use crate::perps::positions::{PerpPosition, PerpPositionStatus};
use crate::types::{now_ms, TimestampMs};
use std::collections::HashMap;
use uuid::Uuid;

/// PG-backed twin of `run_perp_liquidation_tick`. See module docs for
/// the per-candidate transaction shape.
///
/// Response fields mirror the in-memory tick — the admin caller can't
/// tell which store executed the tick from the response.
pub async fn run_perp_liquidation_tick_via_repository(
    cfg: &PerpsReadConfig,
    repository: &PgRepository,
    marks: &HashMap<String, Option<u128>>,
    lifecycle_sender: &LifecycleEventSender,
    now: TimestampMs,
) -> Result<PerpLiquidationTickResponse> {
    let candidates = {
        let mut tx = repository.begin().await?;
        let out = repo::list_open_perp_positions_tx(&mut tx).await?;
        // Read-only tx — drop.
        drop(tx);
        out
    };

    let mut checked = 0u32;
    let mut liquidated = 0u32;
    let mut skipped = 0u32;
    let mut liquidation_ids: Vec<String> = Vec::new();
    for candidate in candidates {
        let Some(market) = cfg.market_by_symbol(&candidate.market_id) else {
            continue;
        };
        checked += 1;
        let mark = marks.get(&candidate.market_id).copied().unwrap_or(None);
        match liquidate_perp_position_via_repository(
            cfg,
            repository,
            market,
            &candidate,
            mark,
            lifecycle_sender,
            now,
        )
        .await?
        {
            Some(event) => match event.status {
                PerpLiquidationStatus::Completed => {
                    liquidated += 1;
                    liquidation_ids.push(event.id.to_string());
                }
                PerpLiquidationStatus::PriceUnavailable => {
                    skipped += 1;
                    liquidation_ids.push(event.id.to_string());
                }
            },
            None => {}
        }
    }
    Ok(PerpLiquidationTickResponse {
        now_ms: now,
        checked_count: checked,
        liquidated_count: liquidated,
        skipped_price_unavailable_count: skipped,
        liquidation_ids,
        chain_id: cfg.chain_id,
        trading_enabled: false,
    })
}

async fn liquidate_perp_position_via_repository(
    _cfg: &PerpsReadConfig,
    repository: &PgRepository,
    market: &PerpsReadMarket,
    candidate: &PerpPosition,
    mark_price_1e8: Option<u128>,
    lifecycle_sender: &LifecycleEventSender,
    now: TimestampMs,
) -> Result<Option<PerpLiquidationEvent>> {
    let mut tx = repository.begin().await?;

    // Re-load the position inside the tx. A concurrent close would have
    // flipped status by now; we honor that and skip.
    let Some(current) =
        repo::get_active_perp_position_tx(&mut tx, &candidate.account, &candidate.market_id)
            .await?
    else {
        return Ok(None);
    };

    // Recompute eligibility using the same prefetched mark.
    let evaluation = evaluate_perp_liquidation(market, &current, mark_price_1e8);
    match evaluation {
        LiquidationEvaluation::Healthy => {
            drop(tx);
            Ok(None)
        }
        LiquidationEvaluation::PriceUnavailable => {
            let event = build_price_unavailable_event_public(&current, now);
            repo::insert_perp_liquidation_event_tx(&mut tx, &event).await?;
            tx.commit()
                .await
                .map_err(|e| BackendError::Persistence(e.to_string()))?;
            emit_perp_position_liquidated_lifecycle(lifecycle_sender, &event);
            Ok(Some(event))
        }
        LiquidationEvaluation::Liquidatable => {
            let mark =
                mark_price_1e8.expect("Liquidatable implies mark price is Some by evaluator");
            let event = apply_pg_liquidation(&mut tx, market, &current, mark, now).await?;
            tx.commit()
                .await
                .map_err(|e| BackendError::Persistence(e.to_string()))?;
            // Re-fetch post-commit rows so lifecycle carries the durable
            // versions. Do a fresh read tx — the important thing is
            // that the tick's write tx has ALREADY committed.
            emit_post_commit_lifecycle_after_liquidation(
                repository,
                lifecycle_sender,
                &current,
                &event,
            )
            .await?;
            Ok(Some(event))
        }
    }
}

async fn apply_pg_liquidation(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    market: &PerpsReadMarket,
    position: &PerpPosition,
    mark_price_1e8: u128,
    now: TimestampMs,
) -> Result<PerpLiquidationEvent> {
    let mm = margin::maintenance_margin_requirement_1e8(
        position.size_1e8,
        mark_price_1e8,
        market.maintenance_margin_bps,
    );
    let unrealized = margin::unrealized_pnl_1e8(position, mark_price_1e8);
    let equity = margin::equity_1e8(position, mark_price_1e8);
    let ratio = margin::margin_ratio_bps(position, mark_price_1e8);
    let realized = margin::realized_pnl_1e8(
        position.entry_price_1e8,
        mark_price_1e8,
        position.size_1e8,
        position.side,
    );
    let bad_debt: u128 = if equity < 0 { (-equity) as u128 } else { 0 };

    // 1. Cancel every open order for (account, market). Returned rows
    // are the updated post-cancel state, ready to feed lifecycle
    // emitters after commit.
    repo::cancel_open_perp_orders_for_account_market_tx(
        tx,
        &position.account,
        &position.market_id,
        order_reason::LIQUIDATED,
        order_reason::SOURCE_LIQUIDATION_TICK,
        now,
    )
    .await?;

    // 2. Flip the position to `liquidated`, add realized PnL delta,
    // stamp `closed_at_ms`, bump version.
    repo::liquidate_active_perp_position_tx(
        tx,
        &position.account,
        &position.market_id,
        realized,
        now,
    )
    .await?;

    // 3. Insert the liquidation event.
    let event = PerpLiquidationEvent {
        id: Uuid::new_v4(),
        account: position.account.clone(),
        market_id: position.market_id.clone(),
        position_id: position.id,
        side: position.side,
        size_1e8: position.size_1e8,
        entry_price_1e8: position.entry_price_1e8,
        mark_price_1e8,
        margin_1e8: position.margin_1e8,
        unrealized_pnl_1e8: unrealized,
        equity_1e8: equity,
        maintenance_margin_requirement_1e8: mm,
        margin_ratio_bps: ratio,
        realized_pnl_1e8: realized,
        bad_debt_1e8: bad_debt,
        liquidation_fee_1e8: 0,
        status: PerpLiquidationStatus::Completed,
        reason_code: liquidation_reason::MARGIN_BREACH.to_string(),
        created_at_ms: now,
    };
    repo::insert_perp_liquidation_event_tx(tx, &event).await?;
    Ok(event)
}

/// AFTER commit, re-read the durable position + cancelled orders so
/// lifecycle carries the post-commit state, then emit
/// `PerpPositionUpdated` → per-order `PerpOrderUpdated` →
/// `PerpPositionLiquidated`.
async fn emit_post_commit_lifecycle_after_liquidation(
    repository: &PgRepository,
    lifecycle_sender: &LifecycleEventSender,
    original: &PerpPosition,
    event: &PerpLiquidationEvent,
) -> Result<()> {
    // Position status is now 'liquidated'; use the client-side patched
    // version to avoid another PG round-trip if the row hasn't already
    // been captured. The row's realized/version/closed_at values must
    // match what we just committed.
    let mut post = original.clone();
    post.status = PerpPositionStatus::Liquidated;
    post.realized_pnl_1e8 = post.realized_pnl_1e8.saturating_add(event.realized_pnl_1e8);
    post.updated_at_ms = event.created_at_ms;
    post.closed_at_ms = Some(event.created_at_ms);
    post.version = post.version.saturating_add(1);
    emit_perp_position_lifecycle(lifecycle_sender, &post);

    // Cancelled orders — fresh read filtered to this market.
    let orders = repository
        .list_perp_orders_for_account(&event.account)
        .await?;
    for order in orders.iter().filter(|o| {
        o.market_id == event.market_id
            && o.terminal_reason_code.as_deref() == Some(order_reason::LIQUIDATED)
            && o.terminal_reason_source.as_deref() == Some(order_reason::SOURCE_LIQUIDATION_TICK)
            && o.updated_at_ms == event.created_at_ms
    }) {
        emit_perp_order_lifecycle(lifecycle_sender, order);
    }

    emit_perp_position_liquidated_lifecycle(lifecycle_sender, event);
    Ok(())
}

fn emit_perp_position_liquidated_lifecycle(
    sender: &LifecycleEventSender,
    event: &PerpLiquidationEvent,
) {
    sender.emit(LifecycleEvent {
        account: event.account.clone(),
        channel: LifecycleChannel::AccountPerpPositions,
        payload: LifecyclePayload::PerpPositionLiquidated {
            liquidation_id: event.id.to_string(),
            market_id: event.market_id.clone(),
            position_id: event.position_id.to_string(),
            side: event.side.as_str().to_string(),
            size_1e8: event.size_1e8.to_string(),
            mark_price_1e8: event.mark_price_1e8.to_string(),
            realized_pnl_1e8: event.realized_pnl_1e8.to_string(),
            bad_debt_1e8: event.bad_debt_1e8.to_string(),
            liquidation_fee_1e8: event.liquidation_fee_1e8.to_string(),
            reason_code: event.reason_code.clone(),
            created_at_ms: event.created_at_ms,
        },
        emitted_at_ms: now_ms(),
    });
}
