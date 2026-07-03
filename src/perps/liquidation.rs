//! PERPS-LIQUIDATION-AND-RISK-V1 — isolated-margin liquidation engine.
//!
//! **Internal-only.** Public Perps mutation routes remain fail-closed;
//! liquidations are computed by an admin-gated tick endpoint
//! (`POST /admin/perps/liquidations/tick`) and read via
//! `GET /accounts/:address/perps/liquidations`. There is no public
//! mutation surface that reaches this module.
//!
//! Semantics (isolated margin, no funding accrual, no liquidation fee):
//!
//! ```text
//! equity                          = margin + unrealized_pnl
//! maintenance_margin_requirement  = notional_at_mark * mm_bps / 10_000
//! liquidatable                    = equity <= maintenance_margin_requirement
//! ```
//!
//! **Price required.** If the oracle mark is stale or unavailable,
//! the tick surfaces the candidate as `price_unavailable` and takes
//! no action. Liquidating on a stale price would be a footgun.
//!
//! **Bad debt.** If `equity < 0` at the time of liquidation
//! (e.g. mark gapped past the liquidation price faster than the
//! tick could fire), the shortfall is recorded on the liquidation
//! event as `bad_debt_1e8`. V1 does NOT touch an insurance fund;
//! that's future work.
//!
//! **Liquidation fee.** V1: always `0`. A future milestone can
//! introduce a fee/penalty; the event row already carries a field
//! for it.
//!
//! **Idempotency.** A position that's already `Liquidated` or
//! `Closed` is skipped by the tick. Repeated ticks against the
//! same account/market produce at most one event.

use crate::api::public_ws::LifecycleEventSender;
use crate::error::{BackendError, Result};
use crate::perps::config::{PerpsReadConfig, PerpsReadMarket};
use crate::perps::lifecycle::{emit_perp_order_lifecycle, emit_perp_position_lifecycle};
use crate::perps::margin;
use crate::perps::order_store::PerpOrderStore;
use crate::perps::orders::reason as order_reason;
use crate::perps::positions::{PerpPosition, PerpPositionStatus, PerpPositionsStore, PerpSide};
use crate::perps::price_reader::PerpOraclePriceReader;
use crate::types::{now_ms, AccountId, TimestampMs};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

// ---------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PerpLiquidationStatus {
    /// Position closed at mark; event persisted.
    Completed,
    /// Mark price unavailable at tick time; no action taken. Emitted
    /// so operators know a candidate was seen and skipped.
    PriceUnavailable,
}

impl PerpLiquidationStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::PriceUnavailable => "price_unavailable",
        }
    }
}

pub mod liquidation_reason {
    pub const MARGIN_BREACH: &str = "margin_breach";
    pub const PRICE_UNAVAILABLE: &str = "price_unavailable";
}

/// A liquidation event. All monetary values `1e8`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PerpLiquidationEvent {
    pub id: Uuid,
    pub account: AccountId,
    pub market_id: String,
    pub position_id: Uuid,
    pub side: PerpSide,
    pub size_1e8: u128,
    pub entry_price_1e8: u128,
    pub mark_price_1e8: u128,
    pub margin_1e8: u128,
    pub unrealized_pnl_1e8: i128,
    pub equity_1e8: i128,
    pub maintenance_margin_requirement_1e8: u128,
    pub margin_ratio_bps: u128,
    pub realized_pnl_1e8: i128,
    pub bad_debt_1e8: u128,
    pub liquidation_fee_1e8: u128,
    pub status: PerpLiquidationStatus,
    pub reason_code: String,
    pub created_at_ms: TimestampMs,
}

/// In-memory store of liquidation events. Mirrors the shape of the
/// PG-backed `perp_liquidation_events` table added in migration
/// `0035_perp_liquidations.sql`.
#[derive(Clone, Debug, Default)]
pub struct PerpLiquidationsStore {
    by_id: HashMap<Uuid, PerpLiquidationEvent>,
}

impl PerpLiquidationsStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, event: PerpLiquidationEvent) {
        self.by_id.insert(event.id, event);
    }

    pub fn list_for_account(&self, account: &AccountId) -> Vec<PerpLiquidationEvent> {
        let want = account.0.to_lowercase();
        let mut rows: Vec<PerpLiquidationEvent> = self
            .by_id
            .values()
            .filter(|e| e.account.0.to_lowercase() == want)
            .cloned()
            .collect();
        rows.sort_by(|a, b| b.created_at_ms.cmp(&a.created_at_ms));
        rows
    }

    pub fn all(&self) -> Vec<PerpLiquidationEvent> {
        let mut rows: Vec<PerpLiquidationEvent> = self.by_id.values().cloned().collect();
        rows.sort_by(|a, b| b.created_at_ms.cmp(&a.created_at_ms));
        rows
    }
}

// ---------------------------------------------------------------------
// Wire view
// ---------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PerpLiquidationEventView {
    pub liquidation_id: String,
    pub account: String,
    pub market_id: String,
    pub position_id: String,
    pub side: String,
    pub size_1e8: String,
    pub entry_price_1e8: String,
    pub mark_price_1e8: String,
    pub margin_1e8: String,
    pub unrealized_pnl_1e8: String,
    pub equity_1e8: String,
    pub maintenance_margin_requirement_1e8: String,
    pub margin_ratio_bps: String,
    pub realized_pnl_1e8: String,
    pub bad_debt_1e8: String,
    pub liquidation_fee_1e8: String,
    pub status: String,
    pub reason_code: String,
    pub created_at_ms: TimestampMs,
    pub trading_enabled: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PerpLiquidationListResponse {
    pub liquidations: Vec<PerpLiquidationEventView>,
    pub chain_id: u64,
    pub trading_enabled: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PerpLiquidationTickResponse {
    pub now_ms: TimestampMs,
    pub checked_count: u32,
    pub liquidated_count: u32,
    pub skipped_price_unavailable_count: u32,
    pub liquidation_ids: Vec<String>,
    pub chain_id: u64,
    pub trading_enabled: bool,
}

pub fn build_perp_liquidation_view(event: &PerpLiquidationEvent) -> PerpLiquidationEventView {
    PerpLiquidationEventView {
        liquidation_id: event.id.to_string(),
        account: event.account.0.clone(),
        market_id: event.market_id.clone(),
        position_id: event.position_id.to_string(),
        side: event.side.as_str().to_string(),
        size_1e8: event.size_1e8.to_string(),
        entry_price_1e8: event.entry_price_1e8.to_string(),
        mark_price_1e8: event.mark_price_1e8.to_string(),
        margin_1e8: event.margin_1e8.to_string(),
        unrealized_pnl_1e8: event.unrealized_pnl_1e8.to_string(),
        equity_1e8: event.equity_1e8.to_string(),
        maintenance_margin_requirement_1e8: event.maintenance_margin_requirement_1e8.to_string(),
        margin_ratio_bps: event.margin_ratio_bps.to_string(),
        realized_pnl_1e8: event.realized_pnl_1e8.to_string(),
        bad_debt_1e8: event.bad_debt_1e8.to_string(),
        liquidation_fee_1e8: event.liquidation_fee_1e8.to_string(),
        status: event.status.as_str().to_string(),
        reason_code: event.reason_code.clone(),
        created_at_ms: event.created_at_ms,
        trading_enabled: false,
    }
}

// ---------------------------------------------------------------------
// Pure math
// ---------------------------------------------------------------------

/// Outcome of a per-position risk evaluation. Callers decide whether
/// to act on the `Liquidatable` variant.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LiquidationEvaluation {
    Healthy,
    Liquidatable,
    PriceUnavailable,
}

/// Evaluate a single position against a mark price + market risk
/// config. `mark_price_1e8 = None` means the oracle is stale or
/// unavailable; the position is neither liquidated nor cleared.
pub fn evaluate_perp_liquidation(
    market: &PerpsReadMarket,
    position: &PerpPosition,
    mark_price_1e8: Option<u128>,
) -> LiquidationEvaluation {
    if position.status != PerpPositionStatus::Open {
        return LiquidationEvaluation::Healthy;
    }
    let Some(mark) = mark_price_1e8 else {
        return LiquidationEvaluation::PriceUnavailable;
    };
    let mm = margin::maintenance_margin_requirement_1e8(
        position.size_1e8,
        mark,
        market.maintenance_margin_bps,
    );
    let equity = margin::equity_1e8(position, mark);
    if equity <= mm as i128 {
        LiquidationEvaluation::Liquidatable
    } else {
        LiquidationEvaluation::Healthy
    }
}

// ---------------------------------------------------------------------
// Action: liquidate one position
// ---------------------------------------------------------------------

/// Apply a liquidation to `(account, market_id)` in the in-memory
/// ledger. Returns the persisted event on success. Fires
/// `PerpPositionUpdated` (status='liquidated') + one
/// `PerpOrderUpdated` per cancelled order + `PerpPositionLiquidated`
/// (via the caller emitting on the returned event) — the emitters
/// are chained in `liquidate_perp_position_internal` below.
///
/// **Idempotent.** A position that is not `Open`, or whose mark is
/// unavailable, produces no event and no state mutation.
pub fn liquidate_perp_position_internal(
    cfg: &PerpsReadConfig,
    positions_store: &mut PerpPositionsStore,
    order_store: &mut PerpOrderStore,
    liquidations_store: &mut PerpLiquidationsStore,
    lifecycle_sender: &LifecycleEventSender,
    account: &AccountId,
    market_id: &str,
    mark_price_1e8: Option<u128>,
    now: TimestampMs,
) -> Result<Option<PerpLiquidationEvent>> {
    let market = cfg
        .market_by_symbol(market_id)
        .ok_or_else(|| BackendError::PerpsMarketNotFound(market_id.to_string()))?;
    let Some(position) = positions_store.get_active(account, market_id) else {
        return Ok(None);
    };
    let evaluation = evaluate_perp_liquidation(market, &position, mark_price_1e8);
    match evaluation {
        LiquidationEvaluation::Healthy => Ok(None),
        LiquidationEvaluation::PriceUnavailable => {
            // Record the event with status=price_unavailable so an
            // operator can see the candidate was seen and skipped.
            let event = build_price_unavailable_event(&position, now);
            liquidations_store.insert(event.clone());
            emit_perp_position_liquidated_lifecycle(lifecycle_sender, &event);
            Ok(Some(event))
        }
        LiquidationEvaluation::Liquidatable => {
            let mark = mark_price_1e8.expect("Liquidatable implies mark is Some");
            let event = apply_liquidation(
                market,
                positions_store,
                order_store,
                liquidations_store,
                lifecycle_sender,
                &position,
                mark,
                now,
            )?;
            Ok(Some(event))
        }
    }
}

fn apply_liquidation(
    market: &PerpsReadMarket,
    positions_store: &mut PerpPositionsStore,
    order_store: &mut PerpOrderStore,
    liquidations_store: &mut PerpLiquidationsStore,
    lifecycle_sender: &LifecycleEventSender,
    position: &PerpPosition,
    mark_price_1e8: u128,
    now: TimestampMs,
) -> Result<PerpLiquidationEvent> {
    let notional = margin::notional_1e8(position.size_1e8, mark_price_1e8);
    let mm = margin::maintenance_margin_requirement_1e8(
        position.size_1e8,
        mark_price_1e8,
        market.maintenance_margin_bps,
    );
    let unrealized = margin::unrealized_pnl_1e8(position, mark_price_1e8);
    let equity = margin::equity_1e8(position, mark_price_1e8);
    let ratio = margin::margin_ratio_bps(position, mark_price_1e8);
    // Realised PnL from closing the whole size at mark. Signed.
    let realized = margin::realized_pnl_1e8(
        position.entry_price_1e8,
        mark_price_1e8,
        position.size_1e8,
        position.side,
    );
    // Bad debt: shortfall between equity and 0. Positive when the
    // trader owes more than their margin covers.
    let bad_debt: u128 = if equity < 0 { (-equity) as u128 } else { 0 };
    let liquidated_position =
        positions_store.liquidate_active(&position.account, &position.market_id, realized, now)?;
    let cancelled_orders = order_store.cancel_open_orders_for_account_market(
        &position.account,
        &position.market_id,
        order_reason::LIQUIDATED,
        order_reason::SOURCE_LIQUIDATION_TICK,
        now,
    );
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
        liquidation_fee_1e8: 0, // V1: no liquidation fee
        status: PerpLiquidationStatus::Completed,
        reason_code: liquidation_reason::MARGIN_BREACH.to_string(),
        created_at_ms: now,
    };
    liquidations_store.insert(event.clone());
    // Emit lifecycle for everything the tick changed.
    emit_perp_position_lifecycle(lifecycle_sender, &liquidated_position);
    for order in &cancelled_orders {
        emit_perp_order_lifecycle(lifecycle_sender, order);
    }
    emit_perp_position_liquidated_lifecycle(lifecycle_sender, &event);
    let _ = notional; // silence unused when the caller doesn't need it
    Ok(event)
}

/// Public re-export of the price-unavailable event builder for the
/// PG-backed tick (`crate::perps::liquidation_pg`). Same shape as the
/// in-memory helper — the mark price is `0` and the reason is
/// `price_unavailable`.
pub fn build_price_unavailable_event_public(
    position: &PerpPosition,
    now: TimestampMs,
) -> PerpLiquidationEvent {
    build_price_unavailable_event(position, now)
}

fn build_price_unavailable_event(
    position: &PerpPosition,
    now: TimestampMs,
) -> PerpLiquidationEvent {
    PerpLiquidationEvent {
        id: Uuid::new_v4(),
        account: position.account.clone(),
        market_id: position.market_id.clone(),
        position_id: position.id,
        side: position.side,
        size_1e8: position.size_1e8,
        entry_price_1e8: position.entry_price_1e8,
        mark_price_1e8: 0,
        margin_1e8: position.margin_1e8,
        unrealized_pnl_1e8: 0,
        equity_1e8: position.margin_1e8 as i128,
        maintenance_margin_requirement_1e8: 0,
        margin_ratio_bps: 0,
        realized_pnl_1e8: 0,
        bad_debt_1e8: 0,
        liquidation_fee_1e8: 0,
        status: PerpLiquidationStatus::PriceUnavailable,
        reason_code: liquidation_reason::PRICE_UNAVAILABLE.to_string(),
        created_at_ms: now,
    }
}

fn emit_perp_position_liquidated_lifecycle(
    sender: &LifecycleEventSender,
    event: &PerpLiquidationEvent,
) {
    use crate::api::public_ws::{LifecycleChannel, LifecycleEvent, LifecyclePayload};
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

// ---------------------------------------------------------------------
// Tick orchestration
// ---------------------------------------------------------------------

/// Prefetch mark prices for every configured market. The tick
/// splits into two phases so the async price reads never happen
/// while a `std::sync::MutexGuard` is held (axum requires
/// `Send`-safe handlers).
pub async fn prefetch_mark_prices<P: PerpOraclePriceReader + ?Sized>(
    cfg: &PerpsReadConfig,
    price_reader: &P,
    now: TimestampMs,
) -> HashMap<String, Option<u128>> {
    let mut marks = HashMap::new();
    for market in &cfg.markets {
        let mark = match price_reader.read_price(market).await {
            Ok(read) if read.ok && read.price_1e8 > 0 => {
                let stale_after_ms = (cfg.stale_after_sec as i64).saturating_mul(1000);
                let updated_ms = read.updated_at_ms();
                if updated_ms == 0 || now.saturating_sub(updated_ms) > stale_after_ms {
                    None
                } else {
                    Some(read.price_1e8)
                }
            }
            _ => None,
        };
        marks.insert(market.symbol.clone(), mark);
    }
    marks
}

/// Run the liquidation tick across every active perp position using
/// the pre-fetched mark price map from `prefetch_mark_prices`. This
/// function is synchronous — no `await` inside — so the caller can
/// hold `std::sync::MutexGuard`s across the whole call without
/// tripping axum's `Send` requirements.
pub fn run_perp_liquidation_tick(
    cfg: &PerpsReadConfig,
    positions_store: &mut PerpPositionsStore,
    order_store: &mut PerpOrderStore,
    liquidations_store: &mut PerpLiquidationsStore,
    marks: &HashMap<String, Option<u128>>,
    lifecycle_sender: &LifecycleEventSender,
    now: TimestampMs,
) -> Result<PerpLiquidationTickResponse> {
    let all_positions = positions_store.by_id_values_iter();
    let mut candidates: Vec<PerpPosition> = all_positions
        .into_iter()
        .filter(|p| p.status == PerpPositionStatus::Open)
        .filter(|p| cfg.market_by_symbol(&p.market_id).is_some())
        .collect();
    candidates.sort_by_key(|p| p.id);

    let mut checked = 0u32;
    let mut liquidated = 0u32;
    let mut skipped = 0u32;
    let mut ids: Vec<String> = Vec::new();
    for candidate in candidates {
        checked += 1;
        let mark = marks.get(&candidate.market_id).copied().unwrap_or(None);
        if let Some(event) = liquidate_perp_position_internal(
            cfg,
            positions_store,
            order_store,
            liquidations_store,
            lifecycle_sender,
            &candidate.account,
            &candidate.market_id,
            mark,
            now,
        )? {
            match event.status {
                PerpLiquidationStatus::Completed => {
                    liquidated += 1;
                    ids.push(event.id.to_string());
                }
                PerpLiquidationStatus::PriceUnavailable => {
                    skipped += 1;
                    ids.push(event.id.to_string());
                }
            }
        }
    }
    Ok(PerpLiquidationTickResponse {
        now_ms: now,
        checked_count: checked,
        liquidated_count: liquidated,
        skipped_price_unavailable_count: skipped,
        liquidation_ids: ids,
        chain_id: cfg.chain_id,
        trading_enabled: false,
    })
}

pub fn list_perp_liquidations_for_account_view(
    cfg: &PerpsReadConfig,
    store: &PerpLiquidationsStore,
    account: &AccountId,
) -> PerpLiquidationListResponse {
    let events = store.list_for_account(account);
    let liquidations: Vec<PerpLiquidationEventView> =
        events.iter().map(build_perp_liquidation_view).collect();
    PerpLiquidationListResponse {
        liquidations,
        chain_id: cfg.chain_id,
        trading_enabled: false,
    }
}
