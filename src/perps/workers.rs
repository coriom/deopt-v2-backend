//! PERPS-FUNDING-LIQUIDATION-WORKERS-V1 — periodic Perps operational
//! workers (funding + liquidation) with layered kill-switches.
//!
//! **Safety posture (unchanged by this module):**
//!
//! * Public Perps trading remains fail-closed (`PERPS_PUBLIC_TRADING_ENABLED`
//!   default `false`).
//! * Closed-test path remains fail-closed (`PERPS_CLOSED_TEST_ENABLED`
//!   default `false`).
//! * v2 write-auth enforcement remains sealed for every mutation.
//!
//! **What this module adds:**
//!
//! * `PerpsFundingWorkerConfig` — default-off periodic runner over the
//!   existing `run_perp_funding_tick` (in-memory or PG-repository).
//! * `PerpsLiquidationWorkerConfig` — same shape over
//!   `run_perp_liquidation_tick`.
//! * A two-layer gate: `worker_enabled` starts the periodic loop, and
//!   `tick_enabled` gates each tick execution (whether periodic or
//!   admin-triggered). Flipping `tick_enabled=false` at runtime turns
//!   both the loop and the admin HTTP tick into safe no-ops without
//!   killing the process — the operator kill-switch pattern.
//! * `PerpsWorkerTickRecord` — observability record for the readiness
//!   endpoint. Never contains secrets, wallets, or signatures.
//!
//! **What this module does NOT do:**
//!
//! * No new mutation route.
//! * No new admin route (the existing `POST /admin/perps/funding/tick`
//!   and `POST /admin/perps/liquidations/tick` handlers are updated
//!   in-place to consult the kill-switch and record last-tick state).
//! * No mainnet path (`validate_startup` refuses enabled worker configs
//!   on chain ids `1` and `8453`).
//! * No liquidator public route.
//! * No cross-subaccount effect — the underlying tick paths already
//!   preserve `(account, subaccount_id, market_id)` isolation.

use crate::error::{BackendError, Result};
use crate::types::TimestampMs;
use serde::Serialize;

// Bounds for worker interval configuration. Under `MIN_INTERVAL_SEC`
// the loop starts to busy-spin; over `MAX_INTERVAL_SEC` the worker
// stops being "periodic" in any meaningful sense (funding once every
// 24h is a lifecycle event, not a worker). Both bounds enforced by
// `validate_startup` even when the worker is disabled so a dangerous
// value is caught early.
const MIN_FUNDING_INTERVAL_SEC: u64 = 30;
const MAX_FUNDING_INTERVAL_SEC: u64 = 86_400;
const MIN_LIQUIDATION_INTERVAL_SEC: u64 = 5;
const MAX_LIQUIDATION_INTERVAL_SEC: u64 = 3_600;

// Default per-tick caps. Both are advisory in V1 — the underlying tick
// paths scan every configured market or every active position; the
// values are surfaced in the readiness output for operator visibility
// and reserved for future truncation logic without a breaking API
// change.
const DEFAULT_FUNDING_INTERVAL_SEC: u64 = 3_600;
const DEFAULT_FUNDING_MAX_MARKETS_PER_TICK: u32 = 32;
const DEFAULT_LIQUIDATION_INTERVAL_SEC: u64 = 30;
const DEFAULT_LIQUIDATION_MAX_POSITIONS_PER_TICK: u32 = 500;

/// Policy for the periodic worker when the oracle for a market is
/// stale or unavailable. V1 only supports the `Skip` policy — the
/// enum exists so a future `Pause` or `Retry` variant can be added
/// without a wire break.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PerpsWorkerStaleOraclePolicy {
    /// Skip the affected market for this tick. The `skipped_*_count`
    /// on the tick response records how many markets/positions were
    /// skipped. Matches the pre-worker behaviour of the admin tick.
    Skip,
}

impl PerpsWorkerStaleOraclePolicy {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Skip => "skip",
        }
    }

    /// Parse an env value. Empty / unknown → `Skip` (the safe default).
    pub fn parse(raw: &str) -> Self {
        match raw.trim().to_ascii_lowercase().as_str() {
            "skip" | "" => Self::Skip,
            _ => Self::Skip,
        }
    }
}

impl Default for PerpsWorkerStaleOraclePolicy {
    fn default() -> Self {
        Self::Skip
    }
}

/// Periodic Perps funding worker configuration. All defaults are safe
/// (`worker_enabled=false`, `tick_enabled=false`). Enable via env:
///
/// * `PERPS_FUNDING_WORKER_ENABLED=true` (spawns the periodic loop)
/// * `PERPS_FUNDING_TICK_ENABLED=true`   (gates each tick — kill-switch)
/// * `PERPS_FUNDING_WORKER_INTERVAL_SEC` (30..=86400)
/// * `PERPS_FUNDING_MAX_MARKETS_PER_TICK` (advisory)
/// * `PERPS_FUNDING_STALE_ORACLE_POLICY=skip`
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PerpsFundingWorkerConfig {
    /// When `false`, `spawn_perps_funding_worker` returns immediately.
    /// The admin `POST /admin/perps/funding/tick` is unaffected by this
    /// flag — the admin path always executes when the operator token
    /// is present and `tick_enabled=true`.
    pub worker_enabled: bool,
    /// Kill-switch consulted by BOTH the periodic worker and the admin
    /// HTTP tick. When `false`, both surfaces return a "disabled" tick
    /// record without touching the funding index reader.
    pub tick_enabled: bool,
    /// Seconds between periodic ticks. `MissedTickBehavior::Delay`
    /// applies: a slow tick does not queue subsequent ticks. Ignored
    /// when `worker_enabled=false`.
    pub interval_sec: u64,
    /// Advisory upper bound on markets processed per tick. Reserved
    /// for future truncation logic. Surfaced in readiness output.
    pub max_markets_per_tick: u32,
    /// Policy when the funding-index source for a market is stale
    /// beyond `PERPS_STALE_AFTER_SEC` or unavailable. V1: `Skip` only.
    pub stale_oracle_policy: PerpsWorkerStaleOraclePolicy,
}

impl PerpsFundingWorkerConfig {
    /// The safe default: everything off.
    pub fn disabled() -> Self {
        Self {
            worker_enabled: false,
            tick_enabled: false,
            interval_sec: DEFAULT_FUNDING_INTERVAL_SEC,
            max_markets_per_tick: DEFAULT_FUNDING_MAX_MARKETS_PER_TICK,
            stale_oracle_policy: PerpsWorkerStaleOraclePolicy::Skip,
        }
    }

    /// Refuses obviously-invalid configuration at startup. Runs even
    /// when `worker_enabled=false` so a dangerous knob (e.g.
    /// `interval_sec=0`) is caught before the operator later flips
    /// the enable flag. Mainnet is refused unconditionally when the
    /// worker is enabled.
    pub fn validate_startup(&self, chain_id: u64) -> Result<()> {
        if self.interval_sec < MIN_FUNDING_INTERVAL_SEC
            || self.interval_sec > MAX_FUNDING_INTERVAL_SEC
        {
            return Err(BackendError::Config(format!(
                "PERPS_FUNDING_WORKER_INTERVAL_SEC must be in [{MIN_FUNDING_INTERVAL_SEC}, \
                 {MAX_FUNDING_INTERVAL_SEC}], got {}",
                self.interval_sec
            )));
        }
        if self.max_markets_per_tick == 0 {
            return Err(BackendError::Config(
                "PERPS_FUNDING_MAX_MARKETS_PER_TICK must be > 0".to_string(),
            ));
        }
        if (self.worker_enabled || self.tick_enabled) && (chain_id == 1 || chain_id == 8453) {
            return Err(BackendError::Config(format!(
                "PERPS_FUNDING_WORKER/TICK_ENABLED refused on mainnet chain id {chain_id}"
            )));
        }
        Ok(())
    }
}

/// Periodic Perps liquidation worker configuration. All defaults are
/// safe (`worker_enabled=false`, `tick_enabled=false`).
///
/// Env:
///
/// * `PERPS_LIQUIDATION_WORKER_ENABLED=true`
/// * `PERPS_LIQUIDATION_TICK_ENABLED=true` (kill-switch)
/// * `PERPS_LIQUIDATION_WORKER_INTERVAL_SEC` (5..=3600)
/// * `PERPS_LIQUIDATION_MAX_POSITIONS_PER_TICK`
/// * `PERPS_LIQUIDATION_STALE_ORACLE_POLICY=skip`
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PerpsLiquidationWorkerConfig {
    pub worker_enabled: bool,
    pub tick_enabled: bool,
    pub interval_sec: u64,
    pub max_positions_per_tick: u32,
    pub stale_oracle_policy: PerpsWorkerStaleOraclePolicy,
}

impl PerpsLiquidationWorkerConfig {
    pub fn disabled() -> Self {
        Self {
            worker_enabled: false,
            tick_enabled: false,
            interval_sec: DEFAULT_LIQUIDATION_INTERVAL_SEC,
            max_positions_per_tick: DEFAULT_LIQUIDATION_MAX_POSITIONS_PER_TICK,
            stale_oracle_policy: PerpsWorkerStaleOraclePolicy::Skip,
        }
    }

    pub fn validate_startup(&self, chain_id: u64) -> Result<()> {
        if self.interval_sec < MIN_LIQUIDATION_INTERVAL_SEC
            || self.interval_sec > MAX_LIQUIDATION_INTERVAL_SEC
        {
            return Err(BackendError::Config(format!(
                "PERPS_LIQUIDATION_WORKER_INTERVAL_SEC must be in \
                 [{MIN_LIQUIDATION_INTERVAL_SEC}, {MAX_LIQUIDATION_INTERVAL_SEC}], got {}",
                self.interval_sec
            )));
        }
        if self.max_positions_per_tick == 0 {
            return Err(BackendError::Config(
                "PERPS_LIQUIDATION_MAX_POSITIONS_PER_TICK must be > 0".to_string(),
            ));
        }
        if (self.worker_enabled || self.tick_enabled) && (chain_id == 1 || chain_id == 8453) {
            return Err(BackendError::Config(format!(
                "PERPS_LIQUIDATION_WORKER/TICK_ENABLED refused on mainnet chain id {chain_id}"
            )));
        }
        Ok(())
    }
}

/// One-line record for the readiness endpoint. Written after every
/// funding or liquidation tick (periodic OR admin-triggered). Never
/// contains wallet addresses, order ids, or subaccount detail —
/// operator-facing summary only.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct PerpsWorkerTickRecord {
    /// Whether the tick actually executed. `false` when the kill-switch
    /// disabled it; the recorded timestamps still update so an operator
    /// can see the worker heartbeat.
    pub executed: bool,
    /// Whether execution completed without a `BackendError` return.
    /// Individual per-market skips (stale oracle) do NOT flip this to
    /// `false` — those are counted in `skipped`. `false` here means the
    /// tick returned an error and no state was mutated.
    pub ok: bool,
    /// Milliseconds since epoch when the tick started.
    pub started_at_ms: TimestampMs,
    /// Milliseconds since epoch when the tick finished. Equal to
    /// `started_at_ms` when the tick was a skip.
    pub finished_at_ms: TimestampMs,
    /// Positions or markets scanned. `0` when the tick was a skip.
    pub checked_count: u32,
    /// Positions settled/liquidated. `0` when the tick was a skip or
    /// there was nothing to do.
    pub applied_count: u32,
    /// Markets or positions skipped due to stale/unavailable oracle.
    pub skipped_count: u32,
}

impl PerpsWorkerTickRecord {
    /// Record when the kill-switch caused a skip. Used when the caller
    /// wants to bump the last-tick timestamp so the readiness endpoint
    /// shows a fresh heartbeat.
    pub fn skipped(now: TimestampMs) -> Self {
        Self {
            executed: false,
            ok: true,
            started_at_ms: now,
            finished_at_ms: now,
            checked_count: 0,
            applied_count: 0,
            skipped_count: 0,
        }
    }

    /// Record after a successful funding tick.
    pub fn from_funding(
        started_at_ms: TimestampMs,
        finished_at_ms: TimestampMs,
        response: &crate::perps::PerpFundingTickResponse,
    ) -> Self {
        Self {
            executed: true,
            ok: true,
            started_at_ms,
            finished_at_ms,
            checked_count: response.checked_count,
            applied_count: response.settled_count,
            skipped_count: response.skipped_source_unavailable_count,
        }
    }

    /// Record after a successful liquidation tick.
    pub fn from_liquidation(
        started_at_ms: TimestampMs,
        finished_at_ms: TimestampMs,
        response: &crate::perps::PerpLiquidationTickResponse,
    ) -> Self {
        Self {
            executed: true,
            ok: true,
            started_at_ms,
            finished_at_ms,
            checked_count: response.checked_count,
            applied_count: response.liquidated_count,
            skipped_count: response.skipped_price_unavailable_count,
        }
    }

    /// Record after a tick returned an error. The counts are all zero
    /// because we cannot trust a partial tick result — the underlying
    /// tick paths are all-or-nothing per invocation.
    pub fn errored(started_at_ms: TimestampMs, finished_at_ms: TimestampMs) -> Self {
        Self {
            executed: true,
            ok: false,
            started_at_ms,
            finished_at_ms,
            checked_count: 0,
            applied_count: 0,
            skipped_count: 0,
        }
    }
}

// ---------------------------------------------------------------------
// Periodic runners.
// ---------------------------------------------------------------------
//
// Both spawn functions follow the existing repo pattern
// (`spawn_option_confirmation_worker` in `options/confirmation_worker.rs`):
//
//   * `tokio::spawn` an unnamed task,
//   * `tokio::time::interval` with `MissedTickBehavior::Delay`,
//   * catch tick errors via `tracing::warn!` (no panic escape).
//
// The task lives for the lifetime of the process; graceful shutdown
// piggy-backs on the existing runtime shutdown (the current backend
// has no cancellation token wired for any worker, so we match that
// posture rather than introduce a divergent one).

/// Spawn the periodic funding worker. No-op when `worker_enabled=false`.
/// The kill-switch `tick_enabled=false` is consulted per-tick — the
/// spawn function does not consult it, so an operator can flip the
/// kill-switch at runtime without restarting.
pub fn spawn_perps_funding_worker(state: crate::api::AppState) {
    let cfg = state.perps_funding_worker_config.clone();
    if !cfg.worker_enabled {
        tracing::info!(
            worker = "perps_funding",
            enabled = false,
            "perps funding worker disabled"
        );
        return;
    }
    let interval_ms = cfg.interval_sec.saturating_mul(1000);
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_millis(interval_ms));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        // Skip the immediate `tick.tick()` (which fires at t=0). The
        // first real tick fires at t=interval so a spammy restart loop
        // never spawns a tick storm before the process reaches a
        // steady state.
        interval.tick().await;
        loop {
            interval.tick().await;
            run_perps_funding_tick_once(&state).await;
        }
    });
}

/// Spawn the periodic liquidation worker. Same kill-switch posture as
/// `spawn_perps_funding_worker`.
pub fn spawn_perps_liquidation_worker(state: crate::api::AppState) {
    let cfg = state.perps_liquidation_worker_config.clone();
    if !cfg.worker_enabled {
        tracing::info!(
            worker = "perps_liquidation",
            enabled = false,
            "perps liquidation worker disabled"
        );
        return;
    }
    let interval_ms = cfg.interval_sec.saturating_mul(1000);
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_millis(interval_ms));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        interval.tick().await;
        loop {
            interval.tick().await;
            run_perps_liquidation_tick_once(&state).await;
        }
    });
}

/// Execute one funding tick honouring the kill-switch. Public so
/// `spawn_perps_funding_worker` and the admin HTTP tick can share the
/// same code path — the tick surface must never diverge between the
/// worker and the admin surface, or one could execute while the other
/// respects the kill-switch.
pub async fn run_perps_funding_tick_once(state: &crate::api::AppState) {
    use crate::types::now_ms;
    let started = now_ms();
    if !state.perps_funding_worker_config.tick_enabled {
        record_funding_tick(state, PerpsWorkerTickRecord::skipped(started));
        // PERPS-MONITORING-ALERTING-V1 — count kill-switch skips so a
        // long spell of skips is visible without inspecting logs.
        state
            .perps_observability
            .record_funding_tick_kill_switch_skip();
        return;
    }
    let reader = crate::perps::InMemoryPerpFundingIndexReader::new();
    let indices =
        crate::perps::prefetch_funding_indices(&state.perps_read_config, &reader, started).await;

    if let Some(repository) = state.repository.clone() {
        match crate::perps::run_perp_funding_tick_via_repository(
            &state.perps_read_config,
            &repository,
            &indices,
            &state.lifecycle_events,
            started,
        )
        .await
        {
            Ok(response) => {
                state
                    .perps_observability
                    .record_funding_tick_ok(response.skipped_source_unavailable_count);
                record_funding_tick(
                    state,
                    PerpsWorkerTickRecord::from_funding(started, now_ms(), &response),
                );
            }
            Err(error) => {
                tracing::warn!(%error, "perps funding worker tick (pg) failed");
                state.perps_observability.record_funding_tick_failure();
                record_funding_tick(state, PerpsWorkerTickRecord::errored(started, now_ms()));
            }
        }
        return;
    }

    // In-memory branch. `run_perp_funding_tick` is synchronous so the
    // MutexGuards stay off any `.await`.
    let outcome = {
        let mut positions = match state.perp_positions_store.lock() {
            Ok(guard) => guard,
            Err(_) => {
                record_funding_tick(state, PerpsWorkerTickRecord::errored(started, now_ms()));
                return;
            }
        };
        let mut events = match state.perp_funding_events_store.lock() {
            Ok(guard) => guard,
            Err(_) => {
                record_funding_tick(state, PerpsWorkerTickRecord::errored(started, now_ms()));
                return;
            }
        };
        crate::perps::run_perp_funding_tick(
            &state.perps_read_config,
            &mut positions,
            &mut events,
            &indices,
            &state.lifecycle_events,
            started,
        )
    };
    match outcome {
        Ok(response) => {
            state
                .perps_observability
                .record_funding_tick_ok(response.skipped_source_unavailable_count);
            record_funding_tick(
                state,
                PerpsWorkerTickRecord::from_funding(started, now_ms(), &response),
            );
        }
        Err(error) => {
            tracing::warn!(%error, "perps funding worker tick (in-memory) failed");
            state.perps_observability.record_funding_tick_failure();
            record_funding_tick(state, PerpsWorkerTickRecord::errored(started, now_ms()));
        }
    }
}

/// Execute one liquidation tick honouring the kill-switch. Shared by
/// the periodic worker and the admin HTTP tick.
pub async fn run_perps_liquidation_tick_once(state: &crate::api::AppState) {
    use crate::types::now_ms;
    let started = now_ms();
    if !state.perps_liquidation_worker_config.tick_enabled {
        record_liquidation_tick(state, PerpsWorkerTickRecord::skipped(started));
        state
            .perps_observability
            .record_liquidation_tick_kill_switch_skip();
        return;
    }
    let reader = match build_worker_price_reader(state) {
        Ok(reader) => reader,
        Err(error) => {
            tracing::warn!(%error, "perps liquidation worker could not build price reader");
            state.perps_observability.record_liquidation_tick_failure();
            record_liquidation_tick(state, PerpsWorkerTickRecord::errored(started, now_ms()));
            return;
        }
    };
    let marks =
        crate::perps::prefetch_mark_prices(&state.perps_read_config, &reader, started).await;

    if let Some(repository) = state.repository.clone() {
        match crate::perps::run_perp_liquidation_tick_via_repository(
            &state.perps_read_config,
            &repository,
            &marks,
            &state.lifecycle_events,
            started,
        )
        .await
        {
            Ok(response) => {
                state.perps_observability.record_liquidation_tick_ok(
                    response.skipped_price_unavailable_count,
                    response.liquidated_count,
                );
                record_liquidation_tick(
                    state,
                    PerpsWorkerTickRecord::from_liquidation(started, now_ms(), &response),
                );
            }
            Err(error) => {
                tracing::warn!(%error, "perps liquidation worker tick (pg) failed");
                state.perps_observability.record_liquidation_tick_failure();
                record_liquidation_tick(state, PerpsWorkerTickRecord::errored(started, now_ms()));
            }
        }
        return;
    }

    let outcome = {
        let mut positions = match state.perp_positions_store.lock() {
            Ok(guard) => guard,
            Err(_) => {
                record_liquidation_tick(state, PerpsWorkerTickRecord::errored(started, now_ms()));
                return;
            }
        };
        let mut orders = match state.perp_order_store.lock() {
            Ok(guard) => guard,
            Err(_) => {
                record_liquidation_tick(state, PerpsWorkerTickRecord::errored(started, now_ms()));
                return;
            }
        };
        let mut liquidations = match state.perp_liquidations_store.lock() {
            Ok(guard) => guard,
            Err(_) => {
                record_liquidation_tick(state, PerpsWorkerTickRecord::errored(started, now_ms()));
                return;
            }
        };
        crate::perps::run_perp_liquidation_tick(
            &state.perps_read_config,
            &mut positions,
            &mut orders,
            &mut liquidations,
            &marks,
            &state.lifecycle_events,
            started,
        )
    };
    match outcome {
        Ok(response) => {
            state.perps_observability.record_liquidation_tick_ok(
                response.skipped_price_unavailable_count,
                response.liquidated_count,
            );
            record_liquidation_tick(
                state,
                PerpsWorkerTickRecord::from_liquidation(started, now_ms(), &response),
            );
        }
        Err(error) => {
            tracing::warn!(%error, "perps liquidation worker tick (in-memory) failed");
            state.perps_observability.record_liquidation_tick_failure();
            record_liquidation_tick(state, PerpsWorkerTickRecord::errored(started, now_ms()));
        }
    }
}

/// Build the mark-price reader for the periodic liquidation worker.
/// Returns `BackendError::PerpsReadDisabled` when no RPC URL is
/// configured — the worker then records an errored tick rather than
/// silently fabricating a mark. Never dereferences a secret; the
/// `rpc_url` string is passed into the `HttpJsonRpcProvider` and never
/// logged.
fn build_worker_price_reader(
    state: &crate::api::AppState,
) -> Result<crate::perps::PerpOracleRouterRpcReader<crate::execution::rpc::HttpJsonRpcProvider>> {
    let rpc_url = state
        .perps_read_config
        .rpc_url
        .clone()
        .ok_or(BackendError::PerpsReadDisabled)?;
    let provider = crate::execution::rpc::HttpJsonRpcProvider::new(rpc_url);
    crate::perps::PerpOracleRouterRpcReader::new(provider, &state.perps_read_config)
}

fn record_funding_tick(state: &crate::api::AppState, record: PerpsWorkerTickRecord) {
    if let Ok(mut slot) = state.perp_funding_last_tick.lock() {
        *slot = Some(record);
    }
}

fn record_liquidation_tick(state: &crate::api::AppState, record: PerpsWorkerTickRecord) {
    if let Ok(mut slot) = state.perp_liquidation_last_tick.lock() {
        *slot = Some(record);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn funding_disabled_config_validates_ok() {
        assert!(PerpsFundingWorkerConfig::disabled()
            .validate_startup(84532)
            .is_ok());
    }

    #[test]
    fn funding_interval_out_of_range_rejects() {
        let mut cfg = PerpsFundingWorkerConfig::disabled();
        cfg.interval_sec = 0;
        assert!(cfg.validate_startup(84532).is_err());
        cfg.interval_sec = MAX_FUNDING_INTERVAL_SEC + 1;
        assert!(cfg.validate_startup(84532).is_err());
    }

    #[test]
    fn funding_enabled_on_mainnet_refused() {
        let mut cfg = PerpsFundingWorkerConfig::disabled();
        cfg.worker_enabled = true;
        assert!(cfg.validate_startup(1).is_err());
        assert!(cfg.validate_startup(8453).is_err());
    }

    #[test]
    fn funding_tick_enabled_on_mainnet_refused() {
        let mut cfg = PerpsFundingWorkerConfig::disabled();
        cfg.tick_enabled = true;
        assert!(cfg.validate_startup(1).is_err());
        assert!(cfg.validate_startup(8453).is_err());
    }

    #[test]
    fn liquidation_disabled_config_validates_ok() {
        assert!(PerpsLiquidationWorkerConfig::disabled()
            .validate_startup(84532)
            .is_ok());
    }

    #[test]
    fn liquidation_interval_out_of_range_rejects() {
        let mut cfg = PerpsLiquidationWorkerConfig::disabled();
        cfg.interval_sec = 0;
        assert!(cfg.validate_startup(84532).is_err());
        cfg.interval_sec = MAX_LIQUIDATION_INTERVAL_SEC + 1;
        assert!(cfg.validate_startup(84532).is_err());
    }

    #[test]
    fn liquidation_enabled_on_mainnet_refused() {
        let mut cfg = PerpsLiquidationWorkerConfig::disabled();
        cfg.worker_enabled = true;
        assert!(cfg.validate_startup(1).is_err());
        assert!(cfg.validate_startup(8453).is_err());
    }

    #[test]
    fn max_zero_rejects() {
        let mut fc = PerpsFundingWorkerConfig::disabled();
        fc.max_markets_per_tick = 0;
        assert!(fc.validate_startup(84532).is_err());
        let mut lc = PerpsLiquidationWorkerConfig::disabled();
        lc.max_positions_per_tick = 0;
        assert!(lc.validate_startup(84532).is_err());
    }

    #[test]
    fn stale_policy_parses() {
        assert_eq!(
            PerpsWorkerStaleOraclePolicy::parse("skip"),
            PerpsWorkerStaleOraclePolicy::Skip
        );
        assert_eq!(
            PerpsWorkerStaleOraclePolicy::parse("SKIP"),
            PerpsWorkerStaleOraclePolicy::Skip
        );
        assert_eq!(
            PerpsWorkerStaleOraclePolicy::parse(""),
            PerpsWorkerStaleOraclePolicy::Skip
        );
        assert_eq!(
            PerpsWorkerStaleOraclePolicy::parse("nonsense"),
            PerpsWorkerStaleOraclePolicy::Skip
        );
    }
}
