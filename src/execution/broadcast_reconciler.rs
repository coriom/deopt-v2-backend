//! PERPS_BASE_SEPOLIA_BACKEND_BROADCAST_RUNTIME_WIRING_V1 — background
//! runtime task that periodically invokes
//! [`crate::execution::broadcast_policy::BroadcastPolicy::reconcile_unfinalized`].
//!
//! ## Runtime posture
//!
//! * Spawned only when the deployment has BOTH
//!   `EXECUTOR_REAL_BROADCAST_ENABLED=true` AND
//!   `PERPS_CLOSED_TEST_ENABLED=true`. If either is false the task is
//!   NOT spawned — no broadcast side-effects, no side-channel.
//! * Startup performs an INITIAL reconciliation pass before marking
//!   the worker ready. Rows persisted as `Prepared` by a prior
//!   process are inspected first, so no new state-changing broadcast
//!   can happen until the durable in-flight set has been resolved.
//! * Graceful shutdown via
//!   [`tokio_util::sync::CancellationToken`]-style semaphore built
//!   on `Arc<AtomicBool>` (no new dep — matches
//!   `hybrid_v2::execution::broadcast_worker` cancel pattern).
//! * Bounded polling: reconciliation interval defaulted from
//!   [`ReconcilerConfig`]; batch size bounded to prevent single-tick
//!   RPC storms.
//! * Failure of one row does not kill the whole task — errors are
//!   logged (non-secret ids only) and the loop continues.

use crate::error::Result;
use crate::execution::broadcast_policy::{BroadcastPolicy, ReconcileSummary};
use crate::execution::executor::ExecutionIntentRepository;
use crate::execution::rpc::{
    EthCallProvider, TransactionBroadcastProvider, TransactionReceiptProvider,
};
use crate::execution::signer::ExecutorSigner;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::task::JoinHandle;
use tokio::time::{sleep, Duration};
use tracing::{info, warn};

/// Reconciler config. Bounded defaults suit Base Sepolia block times
/// (~2s) and are conservative for a closed-test operator posture.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReconcilerConfig {
    /// Interval between reconciliation passes. Env:
    /// `PERPS_BROADCAST_RECONCILE_INTERVAL_MS`. Default 10 000 ms.
    pub interval_ms: u64,
    /// Max intents inspected per pass. Env:
    /// `PERPS_BROADCAST_RECONCILE_BATCH_SIZE`. Default 25.
    pub batch_size: u32,
}

impl Default for ReconcilerConfig {
    fn default() -> Self {
        Self {
            interval_ms: 10_000,
            batch_size: 25,
        }
    }
}

impl ReconcilerConfig {
    pub fn from_env() -> Self {
        let mut cfg = Self::default();
        if let Ok(v) = std::env::var("PERPS_BROADCAST_RECONCILE_INTERVAL_MS") {
            if let Ok(parsed) = v.parse::<u64>() {
                if (1_000..=600_000).contains(&parsed) {
                    cfg.interval_ms = parsed;
                }
            }
        }
        if let Ok(v) = std::env::var("PERPS_BROADCAST_RECONCILE_BATCH_SIZE") {
            if let Ok(parsed) = v.parse::<u32>() {
                if (1..=500).contains(&parsed) {
                    cfg.batch_size = parsed;
                }
            }
        }
        cfg
    }
}

/// Cancellation handle. Drop-safe: the loop observes
/// [`Ordering::SeqCst`] on each tick and exits gracefully. Matches
/// the `hybrid_v2::execution::broadcast_worker::CancelToken` pattern
/// so operators can shut down both workers with a single signal.
#[derive(Clone, Default)]
pub struct BroadcastReconcilerCancel(Arc<AtomicBool>);

impl BroadcastReconcilerCancel {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

/// Spawn the reconciler background loop. Returns a `JoinHandle` and
/// the [`BroadcastReconcilerCancel`] handle. Caller MUST have run
/// [`initial_reconciliation`] first as part of startup ordering (see
/// this module's crate-level doc).
pub fn spawn_broadcast_reconciler<R, Repo>(
    policy: Arc<BroadcastPolicy<R, ExecutorSigner>>,
    repository: Arc<Repo>,
    config: ReconcilerConfig,
) -> (JoinHandle<()>, BroadcastReconcilerCancel)
where
    R: EthCallProvider
        + TransactionBroadcastProvider
        + TransactionReceiptProvider
        + Clone
        + Send
        + Sync
        + 'static,
    Repo: ExecutionIntentRepository + 'static,
{
    let cancel = BroadcastReconcilerCancel::new();
    let cancel_clone = cancel.clone();
    let handle = tokio::spawn(async move {
        info!(
            interval_ms = config.interval_ms,
            batch_size = config.batch_size,
            "perps broadcast reconciler started"
        );
        loop {
            if cancel_clone.is_cancelled() {
                info!("perps broadcast reconciler shutting down");
                break;
            }
            match policy
                .reconcile_unfinalized(repository.as_ref(), config.batch_size)
                .await
            {
                Ok(summary) => {
                    if summary.inspected > 0 {
                        info!(
                            inspected = summary.inspected,
                            confirmed = summary.confirmed,
                            failed = summary.failed,
                            still_pending = summary.still_pending,
                            rebroadcast_attempted = summary.rebroadcast_attempted,
                            "reconcile tick complete"
                        );
                    }
                    crate::monitoring::observe_broadcast_reconcile_tick(&summary);
                }
                Err(error) => {
                    warn!(%error, "reconcile tick failed; loop continues");
                }
            }
            sleep(Duration::from_millis(config.interval_ms)).await;
        }
    });
    (handle, cancel)
}

/// One-shot reconciliation pass invoked during startup ordering
/// BEFORE new broadcasts are accepted. Fails closed on any RPC or
/// repository error — the operator must inspect before the worker
/// is allowed to accept new closed-test broadcasts.
pub async fn initial_reconciliation<R, Repo>(
    policy: &BroadcastPolicy<R, ExecutorSigner>,
    repository: &Repo,
    batch_size: u32,
) -> Result<ReconcileSummary>
where
    R: EthCallProvider
        + TransactionBroadcastProvider
        + TransactionReceiptProvider
        + Clone
        + Send
        + Sync,
    Repo: ExecutionIntentRepository,
{
    let summary = policy.reconcile_unfinalized(repository, batch_size).await?;
    info!(
        inspected = summary.inspected,
        confirmed = summary.confirmed,
        failed = summary.failed,
        still_pending = summary.still_pending,
        "initial broadcast reconciliation complete"
    );
    Ok(summary)
}

/// Fail-closed startup preflight. Validates chain identity, PME
/// address, executor authorization, and PME paused state via
/// on-chain reads. Returns Ok(()) only if every predicate passes.
/// Caller is responsible for [`crate::execution::broadcast_policy::validate_signer_triad`]
/// BEFORE invoking this (the triad check does not require RPC).
pub async fn startup_preflight<R>(policy: &BroadcastPolicy<R, ExecutorSigner>) -> Result<()>
where
    R: EthCallProvider
        + TransactionBroadcastProvider
        + TransactionReceiptProvider
        + Clone
        + Send
        + Sync,
{
    policy.preflight_static()?;
    // Chain id
    let rpc_chain_id = policy.rpc.chain_id().await?;
    if rpc_chain_id != policy.config.executor_chain_id {
        return Err(crate::error::BackendError::Config(format!(
            "startup preflight: RPC chain_id ({rpc_chain_id}) != EXECUTOR_CHAIN_ID ({})",
            policy.config.executor_chain_id
        )));
    }
    // PERPS_V2_BACKEND_ANVIL_BROADCAST_E2E_V1 §1 — preflight the
    // ACTIVE version's PME (the one new intents will target). If
    // `PERPS_ACTIVE_ENGINE_VERSION=v2`, preflight the V2 PME
    // executor authorization/pause; if V1, preflight V1. Never
    // hardcode the V1 field here, or a V2 boot would silently
    // validate the wrong engine and mask a mis-authorized executor.
    let active_pme = policy.config.active_perp_matching_engine_address()?;
    let pme_state = crate::execution::broadcast_policy::preflight_pme_state(
        &policy.rpc,
        &policy.config.executor_from_address,
        active_pme,
    )
    .await?;
    if !pme_state.is_executor {
        return Err(crate::error::BackendError::Config(format!(
            "startup preflight: PME.isExecutor({}) == false",
            policy.config.executor_from_address.0
        )));
    }
    if pme_state.paused {
        return Err(crate::error::BackendError::Config(
            "startup preflight: PME.paused() == true".to_string(),
        ));
    }
    info!(
        chain_id = rpc_chain_id,
        executor = %policy.config.executor_from_address.0,
        active_version = %policy.config.perps_active_engine_version.as_persisted_str(),
        pme = %active_pme.0,
        "startup preflight OK"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_defaults_within_bounds() {
        let c = ReconcilerConfig::default();
        assert!((1_000..=600_000).contains(&c.interval_ms));
        assert!((1..=500).contains(&c.batch_size));
    }

    #[test]
    fn config_from_env_clamps_out_of_range() {
        std::env::set_var("PERPS_BROADCAST_RECONCILE_INTERVAL_MS", "999999999");
        std::env::set_var("PERPS_BROADCAST_RECONCILE_BATCH_SIZE", "999999");
        let c = ReconcilerConfig::from_env();
        // Out-of-range values fall back to defaults.
        assert_eq!(c.interval_ms, 10_000);
        assert_eq!(c.batch_size, 25);
        std::env::remove_var("PERPS_BROADCAST_RECONCILE_INTERVAL_MS");
        std::env::remove_var("PERPS_BROADCAST_RECONCILE_BATCH_SIZE");
    }

    #[test]
    fn cancel_token_signals() {
        let c = BroadcastReconcilerCancel::new();
        assert!(!c.is_cancelled());
        c.cancel();
        assert!(c.is_cancelled());
    }

    #[test]
    fn config_from_env_accepts_valid_values() {
        std::env::set_var("PERPS_BROADCAST_RECONCILE_INTERVAL_MS", "5000");
        std::env::set_var("PERPS_BROADCAST_RECONCILE_BATCH_SIZE", "10");
        let c = ReconcilerConfig::from_env();
        assert_eq!(c.interval_ms, 5000);
        assert_eq!(c.batch_size, 10);
        std::env::remove_var("PERPS_BROADCAST_RECONCILE_INTERVAL_MS");
        std::env::remove_var("PERPS_BROADCAST_RECONCILE_BATCH_SIZE");
    }
}
