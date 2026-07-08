//! PERPS-FUNDING-V1 — perpetual funding accrual and settlement.
//!
//! **Internal-only.** Public Perps mutation routes remain fail-closed;
//! funding is settled by an admin-gated tick
//! (`POST /admin/perps/funding/tick`) and read via
//! `GET /accounts/:address/perps/funding`.
//!
//! Model:
//!
//!   * Every market has a signed cumulative funding index
//!     (`cumulativeFundingRate1e18` on the on-chain `PerpEngine.marketState`).
//!   * Each position stores the index it most recently settled at
//!     (`last_funding_index_1e18`).
//!   * Payment for one settlement:
//!
//!     ```text
//!     delta_1e18   = index_after_1e18 - index_before_1e18
//!     payment_1e8  = side_sign * size_1e8 * delta_1e18 / 1e18
//!                    where side_sign = +1 for long, -1 for short
//!     ```
//!
//!     Positive `payment_1e8` means the trader OWES funding — their
//!     `margin_1e8` is reduced (saturating to 0 with the excess
//!     recorded as `bad_debt_1e8`). Negative means the trader
//!     RECEIVES funding — their `margin_1e8` is increased.
//!
//!   * `cumulative_funding_payment_1e8` on the position accumulates the
//!     signed total for auditability.
//!
//!   * Idempotent: if `index_before == index_after`, no state changes
//!     and no event is recorded.
//!
//!   * Stale/unavailable index → the market is skipped for the tick
//!     (`skipped_source_unavailable_count` increments).
//!
//!   * Closed / liquidated positions are skipped.
//!
//!   * Funding **never** auto-liquidates. Reducing margin below
//!     maintenance is allowed; the next liquidation tick handles it.
//!
//! **On-chain flag.** As of 2026-07 the on-chain `FundingConfig.isEnabled`
//! is `false` for both ETH-PERP and BTC-PERP, so the cumulative index
//! stays at `0` and every V1 settlement is a legitimate zero-payment
//! no-op. This is honest — the engine is wired end-to-end and will
//! produce non-zero payments as soon as the on-chain flag flips.

use crate::api::public_ws::{
    LifecycleChannel, LifecycleEvent, LifecycleEventSender, LifecyclePayload,
};
use crate::error::{BackendError, Result};
use crate::perps::config::PerpsReadConfig;
use crate::perps::lifecycle::emit_perp_position_lifecycle;
use crate::perps::positions::{
    apply_funding_payment_to_margin, PerpPosition, PerpPositionStatus, PerpPositionsStore, PerpSide,
};
use crate::types::{now_ms, AccountId, TimestampMs};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

pub const FUNDING_INDEX_SCALE_1E18: u128 = 1_000_000_000_000_000_000;

pub mod funding_reason {
    /// A settlement whose `funding_delta_1e18 != 0` — produced a payment.
    pub const SETTLEMENT: &str = "funding_settlement";
    /// A tick reached the market but the on-chain source was
    /// stale/unavailable. Not persisted as a payment event; the tick
    /// response's `skipped_source_unavailable_count` reflects it.
    #[allow(dead_code)]
    pub const SOURCE_UNAVAILABLE: &str = "funding_source_unavailable";
}

// ---------------------------------------------------------------------
// Reader trait — funding-index source per market
// ---------------------------------------------------------------------

/// One read from the on-chain funding source. Signed 1e18-scaled
/// cumulative rate + timestamp + `ok` flag. `ok=false` means the source
/// was unavailable or the market is not funding-active on chain; the
/// tick treats that market as skipped.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FundingIndexRead {
    pub cumulative_index_1e18: i128,
    pub updated_at_sec: u64,
    pub ok: bool,
}

impl FundingIndexRead {
    pub fn updated_at_ms(&self) -> TimestampMs {
        (self.updated_at_sec as i64).saturating_mul(1000)
    }
}

/// Read the on-chain cumulative funding index for one market.
#[async_trait]
pub trait PerpFundingIndexReader: Send + Sync {
    async fn read_funding_index(
        &self,
        market: &crate::perps::config::PerpsReadMarket,
    ) -> Result<FundingIndexRead>;
}

/// Deterministic in-memory reader for tests.
#[derive(Clone, Debug, Default)]
pub struct InMemoryPerpFundingIndexReader {
    by_symbol: HashMap<String, FundingIndexRead>,
    force_error: Option<String>,
}

impl InMemoryPerpFundingIndexReader {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_index(mut self, symbol: impl Into<String>, read: FundingIndexRead) -> Self {
        self.by_symbol.insert(symbol.into(), read);
        self
    }

    pub fn with_forced_error(mut self, error: impl Into<String>) -> Self {
        self.force_error = Some(error.into());
        self
    }
}

#[async_trait]
impl PerpFundingIndexReader for InMemoryPerpFundingIndexReader {
    async fn read_funding_index(
        &self,
        market: &crate::perps::config::PerpsReadMarket,
    ) -> Result<FundingIndexRead> {
        if let Some(err) = &self.force_error {
            return Err(BackendError::PerpsPriceUnavailable(err.clone()));
        }
        Ok(self
            .by_symbol
            .get(&market.symbol)
            .copied()
            .unwrap_or(FundingIndexRead {
                cumulative_index_1e18: 0,
                updated_at_sec: 0,
                ok: false,
            }))
    }
}

// ---------------------------------------------------------------------
// Pure math
// ---------------------------------------------------------------------

/// Calculate the signed funding payment for one settlement.
///
/// Positive → the trader OWES funding. Negative → the trader RECEIVES
/// funding. Returns 0 when there is no delta or the position has no
/// size.
pub fn calculate_funding_payment_1e8(
    side: PerpSide,
    size_1e8: u128,
    index_before_1e18: i128,
    index_after_1e18: i128,
) -> i128 {
    if size_1e8 == 0 {
        return 0;
    }
    let delta = index_after_1e18.saturating_sub(index_before_1e18);
    if delta == 0 {
        return 0;
    }
    let size = size_1e8 as i128;
    let scale = FUNDING_INDEX_SCALE_1E18 as i128;
    let signed_delta = match side {
        PerpSide::Long => delta,
        PerpSide::Short => -delta,
    };
    // (delta * size) / 1e18. Kept in i128 so sign propagates.
    signed_delta.saturating_mul(size) / scale
}

// ---------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PerpFundingEvent {
    pub id: Uuid,
    pub account: AccountId,
    /// PERPS-SUBACCOUNTS-ENGINE-ROUTING-V1 — subaccount of the funded
    /// position. Legacy pre-milestone payloads default to `1`.
    #[serde(default = "crate::perps::positions::one_subaccount_id")]
    pub subaccount_id: u32,
    pub market_id: String,
    pub position_id: Uuid,
    pub side: PerpSide,
    pub position_size_1e8: u128,
    pub funding_index_before_1e18: i128,
    pub funding_index_after_1e18: i128,
    pub funding_delta_1e18: i128,
    /// Signed payment. Positive = trader owes.
    pub payment_1e8: i128,
    pub margin_before_1e8: u128,
    pub margin_after_1e8: u128,
    pub bad_debt_1e8: u128,
    pub reason_code: String,
    pub created_at_ms: TimestampMs,
}

/// In-memory funding-event ledger. Mirrors the schema of
/// `perp_funding_events` (migration `0036_perp_funding.sql`).
#[derive(Clone, Debug, Default)]
pub struct PerpFundingEventsStore {
    by_id: HashMap<Uuid, PerpFundingEvent>,
}

impl PerpFundingEventsStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, event: PerpFundingEvent) {
        self.by_id.insert(event.id, event);
    }

    pub fn list_for_account(&self, account: &AccountId) -> Vec<PerpFundingEvent> {
        let want = account.0.to_lowercase();
        let mut rows: Vec<PerpFundingEvent> = self
            .by_id
            .values()
            .filter(|e| e.account.0.to_lowercase() == want)
            .cloned()
            .collect();
        rows.sort_by(|a, b| b.created_at_ms.cmp(&a.created_at_ms));
        rows
    }

    /// PERPS-SUBACCOUNTS-ENGINE-ROUTING-V1 — subaccount-scoped list.
    pub fn list_for_account_and_subaccount(
        &self,
        account: &AccountId,
        subaccount_id: u32,
    ) -> Vec<PerpFundingEvent> {
        let want = account.0.to_lowercase();
        let mut rows: Vec<PerpFundingEvent> = self
            .by_id
            .values()
            .filter(|e| e.account.0.to_lowercase() == want && e.subaccount_id == subaccount_id)
            .cloned()
            .collect();
        rows.sort_by(|a, b| b.created_at_ms.cmp(&a.created_at_ms));
        rows
    }

    pub fn all(&self) -> Vec<PerpFundingEvent> {
        let mut rows: Vec<PerpFundingEvent> = self.by_id.values().cloned().collect();
        rows.sort_by(|a, b| b.created_at_ms.cmp(&a.created_at_ms));
        rows
    }
}

// ---------------------------------------------------------------------
// Wire view + responses
// ---------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PerpFundingEventView {
    pub funding_event_id: String,
    pub account: String,
    /// PERPS-SUBACCOUNTS-ENGINE-ROUTING-V1 — subaccount metadata for
    /// frontend filtering.
    #[serde(default = "crate::perps::positions::one_subaccount_id")]
    pub subaccount_id: u32,
    pub market_id: String,
    pub position_id: String,
    pub side: String,
    pub position_size_1e8: String,
    pub funding_index_before_1e18: String,
    pub funding_index_after_1e18: String,
    pub funding_delta_1e18: String,
    pub payment_1e8: String,
    pub margin_before_1e8: String,
    pub margin_after_1e8: String,
    pub bad_debt_1e8: String,
    pub reason_code: String,
    pub created_at_ms: TimestampMs,
    pub trading_enabled: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PerpFundingListResponse {
    pub funding_events: Vec<PerpFundingEventView>,
    pub chain_id: u64,
    pub trading_enabled: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PerpFundingTickResponse {
    pub now_ms: TimestampMs,
    pub checked_count: u32,
    pub settled_count: u32,
    pub skipped_no_position_count: u32,
    pub skipped_source_unavailable_count: u32,
    pub funding_event_ids: Vec<String>,
    pub chain_id: u64,
    pub trading_enabled: bool,
}

pub fn build_perp_funding_view(event: &PerpFundingEvent) -> PerpFundingEventView {
    PerpFundingEventView {
        funding_event_id: event.id.to_string(),
        account: event.account.0.clone(),
        subaccount_id: event.subaccount_id,
        market_id: event.market_id.clone(),
        position_id: event.position_id.to_string(),
        side: event.side.as_str().to_string(),
        position_size_1e8: event.position_size_1e8.to_string(),
        funding_index_before_1e18: event.funding_index_before_1e18.to_string(),
        funding_index_after_1e18: event.funding_index_after_1e18.to_string(),
        funding_delta_1e18: event.funding_delta_1e18.to_string(),
        payment_1e8: event.payment_1e8.to_string(),
        margin_before_1e8: event.margin_before_1e8.to_string(),
        margin_after_1e8: event.margin_after_1e8.to_string(),
        bad_debt_1e8: event.bad_debt_1e8.to_string(),
        reason_code: event.reason_code.clone(),
        created_at_ms: event.created_at_ms,
        trading_enabled: false,
    }
}

pub fn list_perp_funding_events_for_account_view(
    cfg: &PerpsReadConfig,
    store: &PerpFundingEventsStore,
    account: &AccountId,
) -> PerpFundingListResponse {
    let events = store.list_for_account(account);
    let funding_events: Vec<PerpFundingEventView> =
        events.iter().map(build_perp_funding_view).collect();
    PerpFundingListResponse {
        funding_events,
        chain_id: cfg.chain_id,
        trading_enabled: false,
    }
}

// ---------------------------------------------------------------------
// In-memory funding tick
// ---------------------------------------------------------------------

/// Prefetch the funding index for every configured market. Two-phase
/// same as the liquidation tick: async prefetch (no locks), then a
/// synchronous per-position walk (safe to hold `MutexGuard`s).
pub async fn prefetch_funding_indices<R: PerpFundingIndexReader + ?Sized>(
    cfg: &PerpsReadConfig,
    reader: &R,
    now: TimestampMs,
) -> HashMap<String, Option<FundingIndexRead>> {
    let mut out = HashMap::new();
    for market in &cfg.markets {
        let read = match reader.read_funding_index(market).await {
            Ok(read) if read.ok => {
                let stale_after_ms = (cfg.stale_after_sec as i64).saturating_mul(1000);
                let updated_ms = read.updated_at_ms();
                // A cumulative index of 0 with updated_at_sec == 0 is a
                // valid on-chain state (funding disabled → cumulative
                // stays at 0). We treat that as ok=true, updated=now so
                // the tick records legitimate zero-payment no-ops.
                //
                // A non-zero `updated_at_sec` that has drifted past
                // `stale_after_ms` is treated as unavailable.
                if updated_ms != 0 && now.saturating_sub(updated_ms) > stale_after_ms {
                    None
                } else {
                    Some(read)
                }
            }
            _ => None,
        };
        out.insert(market.symbol.clone(), read);
    }
    out
}

/// Run the funding tick against the in-memory ledger. Synchronous —
/// no `await` inside — so the caller can hold `std::sync::MutexGuard`s
/// across the whole call without tripping axum's `Send` requirements.
pub fn run_perp_funding_tick(
    cfg: &PerpsReadConfig,
    positions_store: &mut PerpPositionsStore,
    events_store: &mut PerpFundingEventsStore,
    indices: &HashMap<String, Option<FundingIndexRead>>,
    lifecycle_sender: &LifecycleEventSender,
    now: TimestampMs,
) -> Result<PerpFundingTickResponse> {
    let candidates: Vec<PerpPosition> = positions_store
        .by_id_values_iter()
        .into_iter()
        .filter(|p| p.status == PerpPositionStatus::Open)
        .filter(|p| cfg.market_by_symbol(&p.market_id).is_some())
        .collect();

    let mut checked = 0u32;
    let mut settled = 0u32;
    let skipped_no_position = 0u32;
    let mut skipped_source = 0u32;
    let mut ids: Vec<String> = Vec::new();

    // Sort by id so the walk is deterministic across ticks.
    let mut candidates = candidates;
    candidates.sort_by_key(|p| p.id);
    for candidate in candidates {
        checked += 1;
        let Some(Some(read)) = indices.get(&candidate.market_id) else {
            skipped_source += 1;
            continue;
        };

        let index_before = candidate.last_funding_index_1e18;
        let index_after = read.cumulative_index_1e18;
        let delta = index_after.saturating_sub(index_before);
        if delta == 0 {
            // Idempotent no-op — same index as last settlement.
            // Not a "settlement" for the response counter, but not a
            // "skip" either; the caller should observe a stable
            // `checked_count`. We simply move on.
            continue;
        }

        let payment = calculate_funding_payment_1e8(
            candidate.side,
            candidate.size_1e8,
            index_before,
            index_after,
        );
        let (margin_after, bad_debt) =
            apply_funding_payment_to_margin(candidate.margin_1e8, payment);

        // Mutate the position in-place: new margin, bumped
        // last_funding_index_1e18, accumulated
        // cumulative_funding_payment_1e8, bumped version.
        let position_after = positions_store.update_active(
            &candidate.account,
            candidate.subaccount_id,
            &candidate.market_id,
            now,
            |p| {
                p.margin_1e8 = margin_after;
                p.last_funding_index_1e18 = index_after;
                p.cumulative_funding_payment_1e8 =
                    p.cumulative_funding_payment_1e8.saturating_add(payment);
                Ok(())
            },
        )?;

        let event = PerpFundingEvent {
            id: Uuid::new_v4(),
            account: candidate.account.clone(),
            subaccount_id: candidate.subaccount_id,
            market_id: candidate.market_id.clone(),
            position_id: candidate.id,
            side: candidate.side,
            position_size_1e8: candidate.size_1e8,
            funding_index_before_1e18: index_before,
            funding_index_after_1e18: index_after,
            funding_delta_1e18: delta,
            payment_1e8: payment,
            margin_before_1e8: candidate.margin_1e8,
            margin_after_1e8: margin_after,
            bad_debt_1e8: bad_debt,
            reason_code: funding_reason::SETTLEMENT.to_string(),
            created_at_ms: now,
        };
        events_store.insert(event.clone());

        // Lifecycle: position first (margin changed), then funding
        // payment.
        emit_perp_position_lifecycle(lifecycle_sender, &position_after);
        emit_perp_funding_lifecycle(lifecycle_sender, &event);

        settled += 1;
        ids.push(event.id.to_string());
    }

    let _ = skipped_no_position; // reserved for future account-scoped ticks
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

/// Emit `PerpFundingPaymentCreated` on `account.perp_funding`.
///
/// Called by both the in-memory tick and the PG tick AFTER durable
/// state has been mutated.
pub fn emit_perp_funding_lifecycle(sender: &LifecycleEventSender, event: &PerpFundingEvent) {
    sender.emit(LifecycleEvent {
        account: event.account.clone(),
        channel: LifecycleChannel::AccountPerpFunding,
        payload: LifecyclePayload::PerpFundingPaymentCreated {
            funding_event_id: event.id.to_string(),
            market_id: event.market_id.clone(),
            subaccount_id: event.subaccount_id,
            position_id: event.position_id.to_string(),
            side: event.side.as_str().to_string(),
            position_size_1e8: event.position_size_1e8.to_string(),
            funding_index_before_1e18: event.funding_index_before_1e18.to_string(),
            funding_index_after_1e18: event.funding_index_after_1e18.to_string(),
            funding_delta_1e18: event.funding_delta_1e18.to_string(),
            payment_1e8: event.payment_1e8.to_string(),
            margin_before_1e8: event.margin_before_1e8.to_string(),
            margin_after_1e8: event.margin_after_1e8.to_string(),
            bad_debt_1e8: event.bad_debt_1e8.to_string(),
            reason_code: event.reason_code.clone(),
            created_at_ms: event.created_at_ms,
        },
        emitted_at_ms: now_ms(),
    });
}
