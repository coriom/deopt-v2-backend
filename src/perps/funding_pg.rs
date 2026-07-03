//! PERPS-FUNDING-V1 — PG-backed funding tick.
//!
//! Per-position transaction, lifecycle-after-commit. Same shape as the
//! liquidation PG tick.
//!
//! Transaction shape (one per candidate position):
//!   1. Load candidate open positions (short read tx).
//!   2. For each candidate:
//!        - Skip if the market's funding index is unavailable
//!          (source-unavailable count increments; no state change).
//!        - Begin per-candidate write tx.
//!        - Re-load position inside tx (concurrent close skips
//!          gracefully — active position lookup returns None).
//!        - Compute `payment_1e8` using the pure math helper.
//!        - If `funding_delta_1e18 == 0` → idempotent no-op; drop tx.
//!        - Apply payment to `margin_1e8` (saturating), bump
//!          `last_funding_index_1e18`, accumulate
//!          `cumulative_funding_payment_1e8`.
//!        - `update_perp_position_tx` — persists all field changes.
//!        - `insert_perp_funding_event_tx` — records the event.
//!        - Commit.
//!        - Emit `PerpPositionUpdated` + `PerpFundingPaymentCreated`
//!          AFTER commit.

use crate::api::public_ws::LifecycleEventSender;
use crate::db::repository as repo;
use crate::db::PgRepository;
use crate::error::{BackendError, Result};
use crate::perps::config::PerpsReadConfig;
use crate::perps::funding::{
    calculate_funding_payment_1e8, emit_perp_funding_lifecycle, funding_reason, FundingIndexRead,
    PerpFundingEvent, PerpFundingTickResponse,
};
use crate::perps::lifecycle::emit_perp_position_lifecycle;
use crate::perps::positions::{apply_funding_payment_to_margin, PerpPosition};
use crate::types::TimestampMs;
use std::collections::HashMap;
use uuid::Uuid;

pub async fn run_perp_funding_tick_via_repository(
    cfg: &PerpsReadConfig,
    repository: &PgRepository,
    indices: &HashMap<String, Option<FundingIndexRead>>,
    lifecycle_sender: &LifecycleEventSender,
    now: TimestampMs,
) -> Result<PerpFundingTickResponse> {
    let candidates = {
        let mut tx = repository.begin().await?;
        let out = repo::list_open_perp_positions_tx(&mut tx).await?;
        drop(tx);
        out
    };

    let mut checked = 0u32;
    let mut settled = 0u32;
    let skipped_no_position = 0u32;
    let mut skipped_source = 0u32;
    let mut ids: Vec<String> = Vec::new();

    for candidate in candidates {
        if cfg.market_by_symbol(&candidate.market_id).is_none() {
            continue;
        }
        checked += 1;
        let Some(Some(read)) = indices.get(&candidate.market_id) else {
            skipped_source += 1;
            continue;
        };
        match settle_perp_position_via_repository(
            repository,
            &candidate,
            *read,
            lifecycle_sender,
            now,
        )
        .await?
        {
            Some(event) => {
                settled += 1;
                ids.push(event.id.to_string());
            }
            None => {}
        }
    }

    Ok(PerpFundingTickResponse {
        now_ms: now,
        checked_count: checked,
        settled_count: settled,
        skipped_no_position_count: skipped_no_position,
        skipped_source_unavailable_count: skipped_source,
        funding_event_ids: ids,
        chain_id: cfg.chain_id,
        trading_enabled: false,
    })
}

async fn settle_perp_position_via_repository(
    repository: &PgRepository,
    candidate: &PerpPosition,
    read: FundingIndexRead,
    lifecycle_sender: &LifecycleEventSender,
    now: TimestampMs,
) -> Result<Option<PerpFundingEvent>> {
    let mut tx = repository.begin().await?;

    // Re-load position inside tx. Concurrent close/liquidate skips
    // gracefully.
    let Some(current) =
        repo::get_active_perp_position_tx(&mut tx, &candidate.account, &candidate.market_id)
            .await?
    else {
        drop(tx);
        return Ok(None);
    };

    let index_before = current.last_funding_index_1e18;
    let index_after = read.cumulative_index_1e18;
    let delta = index_after.saturating_sub(index_before);
    if delta == 0 {
        drop(tx);
        return Ok(None);
    }

    let payment =
        calculate_funding_payment_1e8(current.side, current.size_1e8, index_before, index_after);
    let (margin_after, bad_debt) = apply_funding_payment_to_margin(current.margin_1e8, payment);

    let mut updated = current.clone();
    updated.margin_1e8 = margin_after;
    updated.last_funding_index_1e18 = index_after;
    updated.cumulative_funding_payment_1e8 = updated
        .cumulative_funding_payment_1e8
        .saturating_add(payment);
    updated.updated_at_ms = now;
    // NOTE: version is bumped inside update_perp_position_tx via the
    // `version = version + 1` clause; the WHERE requires the CURRENT
    // version so we pass `current.version`.
    let post_commit_position = repo::update_perp_position_tx(&mut tx, &updated).await?;

    let event = PerpFundingEvent {
        id: Uuid::new_v4(),
        account: current.account.clone(),
        market_id: current.market_id.clone(),
        position_id: current.id,
        side: current.side,
        position_size_1e8: current.size_1e8,
        funding_index_before_1e18: index_before,
        funding_index_after_1e18: index_after,
        funding_delta_1e18: delta,
        payment_1e8: payment,
        margin_before_1e8: current.margin_1e8,
        margin_after_1e8: margin_after,
        bad_debt_1e8: bad_debt,
        reason_code: funding_reason::SETTLEMENT.to_string(),
        created_at_ms: now,
    };
    repo::insert_perp_funding_event_tx(&mut tx, &event).await?;

    tx.commit()
        .await
        .map_err(|e| BackendError::Persistence(e.to_string()))?;

    // Post-commit lifecycle: position updated, then funding payment.
    emit_perp_position_lifecycle(lifecycle_sender, &post_commit_position);
    emit_perp_funding_lifecycle(lifecycle_sender, &event);
    Ok(Some(event))
}
