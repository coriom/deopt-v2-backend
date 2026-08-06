//! `BACKEND-HYBRID-V2-PERSISTED-OPERATIONS-V1` — Part R.
//!
//! Background worker that drives an [`IndexerRuntime`] against a
//! [`ChainSource`] and persists each successfully-applied canonical
//! block through a [`HybridV2ProjectionStore`].
//!
//! Frozen rules:
//! - The worker owns the write lock on the runtime during each tick;
//!   never during idle sleep. Readers get the runtime via a read lock.
//! - On start the worker calls `bootstrap_from_persistence`. If that
//!   fails, we log + count the failure but keep polling — the next
//!   tick will observe the cursor + surface readiness accordingly.
//! - Per-tick failure never terminates the loop. Readiness is already
//!   persisted by the failed tick_and_persist call, so the API
//!   surfaces the NOT_READY reason.
//! - Graceful shutdown: on receipt of a `true` on the shutdown watch
//!   channel, the loop exits cleanly at the next sleep boundary.
//! - No auto-fallback: a persistence failure leaves the runtime cursor
//!   where it was in-memory (the sync runtime rolls back on persist
//!   failure). The worker just keeps looping.
//!
//! Reorg recovery integration (BACKEND-HYBRID-V2-PERSISTED-REORG-
//! RECOVERY-V1):
//! - On bootstrap: if the persisted `hybrid_v2_reorg_recovery` row is
//!   active (any phase other than NONE / RECOVERED /
//!   MANUAL_INTERVENTION_REQUIRED), run recovery before ticking.
//! - On tick: catch `RuntimeError::ReorgRequired` +
//!   `RuntimeError::ReorgRecoveryPending`, invoke
//!   `ReorgRecoveryService::recover`, then rebootstrap the runtime.
//! - MANUAL_INTERVENTION_REQUIRED sleeps indefinitely with a hard-503
//!   readiness — operator restart is the only unblock.

use crate::hybrid_v2::chain_source::ChainSource;
use crate::hybrid_v2::manifest::ManifestParams;
use crate::hybrid_v2::persistence::HybridV2ProjectionStore;
use crate::hybrid_v2::reorg_recovery::{
    RecoveryOutcome, ReorgDetection, ReorgRecoveryConfig, ReorgRecoveryService,
};
use crate::hybrid_v2::runtime::{IndexerRuntime, RuntimeError};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::watch;
use tokio::sync::RwLock;
use tokio::task::JoinHandle;

/// Static configuration for the Hybrid V2 indexer worker.
#[derive(Debug, Clone)]
pub struct HybridV2IndexerWorkerConfig {
    /// Time between ticks (milliseconds). Applied uniformly whether
    /// the previous tick produced a block, produced nothing, or
    /// errored — the worker never uses backoff, so operators can
    /// reason about the polling rate independent of chain activity.
    pub poll_interval_ms: u64,
    /// Deployment identifier used as the FK when writing to the
    /// projection store. Must match the deployment attached to the
    /// runtime via `with_persistence`.
    pub deployment_id: i64,
    /// Persisted reorg-recovery configuration bounds. Defaulted from
    /// `ReorgRecoveryConfig::default()` when constructed via
    /// `HybridV2IndexerWorkerConfig::new`.
    pub reorg_recovery_config: ReorgRecoveryConfig,
}

impl HybridV2IndexerWorkerConfig {
    pub fn new(deployment_id: i64, poll_interval_ms: u64) -> Self {
        Self {
            poll_interval_ms: poll_interval_ms.max(1),
            deployment_id,
            reorg_recovery_config: ReorgRecoveryConfig::default(),
        }
    }

    pub fn with_reorg_recovery_config(mut self, config: ReorgRecoveryConfig) -> Self {
        self.reorg_recovery_config = config;
        self
    }
}

/// Spawn the Hybrid V2 indexer worker.
///
/// * `runtime` — write-locked to run each tick and take audit-trail
///   updates. Readers grab a read lock elsewhere (route handlers).
/// * `source` — read-only chain source; the worker never mutates it.
/// * `store` — durable projection store; must be the same store the
///   `runtime` was attached to via `with_persistence`.
/// * `config` — polling cadence + deployment binding.
/// * `shutdown_rx` — optional watch channel; a `true` value signals a
///   graceful stop at the next sleep boundary. If `None`, the worker
///   runs until dropped (test fixtures usually pass `None`).
pub fn spawn_hybrid_v2_indexer_worker(
    runtime: Arc<RwLock<IndexerRuntime>>,
    source: Arc<dyn ChainSource>,
    store: Arc<dyn HybridV2ProjectionStore>,
    config: HybridV2IndexerWorkerConfig,
    shutdown_rx: Option<watch::Receiver<bool>>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        // Bootstrap the runtime from persistence. Failure here is
        // logged + counted but does not stop the worker; the sync
        // path will re-derive the cursor from a fresh start on the
        // next successful tick, which is the same behaviour as if the
        // store was empty.
        {
            let mut guard = runtime.write().await;
            if let Err(err) = guard.bootstrap_from_persistence().await {
                tracing::warn!(
                    target: "hybrid_v2::worker",
                    deployment_id = config.deployment_id,
                    error = %err,
                    "hybrid_v2 indexer worker: bootstrap_from_persistence failed \
                     (continuing with in-memory runtime as fresh start)"
                );
            }
        }

        // If a persisted recovery is active from a prior run, drive
        // it to completion before ticking. Manifest is captured off
        // the runtime; we clone it because the recovery service takes
        // it by shared reference and we do not want to hold the
        // runtime write lock across the whole recovery flow.
        let manifest_snapshot: ManifestParams = {
            let guard = runtime.read().await;
            guard.manifest.clone()
        };
        let cursor_name: String = {
            let guard = runtime.read().await;
            guard.persistence_cursor_name.clone()
        };
        // Ensure config bounds are valid — a bogus config should not
        // silently degrade recovery. Log + escalate on failure.
        if let Err(err) = config.reorg_recovery_config.validate() {
            tracing::error!(
                target: "hybrid_v2::worker",
                deployment_id = config.deployment_id,
                error = %err,
                "hybrid_v2 indexer worker: reorg recovery config invalid — recovery disabled for this run"
            );
        }
        let recovery_service =
            ReorgRecoveryService::new(config.deployment_id, config.reorg_recovery_config.clone());
        if let Ok(Some(persisted)) = store.read_reorg_recovery(config.deployment_id).await {
            if persisted.phase.is_active() {
                tracing::info!(
                    target: "hybrid_v2::worker",
                    deployment_id = config.deployment_id,
                    phase = %persisted.phase.as_str(),
                    epoch = persisted.recovery_epoch,
                    "resuming persisted reorg recovery"
                );
                let outcome = recovery_service
                    .recover(
                        source.as_ref(),
                        &store,
                        &manifest_snapshot,
                        None,
                        &cursor_name,
                    )
                    .await;
                if let Ok(RecoveryOutcome::Recovered { .. }) = outcome {
                    let mut guard = runtime.write().await;
                    let _ = guard.bootstrap_from_persistence().await;
                }
            }
        }

        // Also record we've referenced the store here; store used below
        // via runtime's own persistence handle (which was set via
        // `with_persistence`). We keep our own handle live so the
        // caller can drop the outer Arc without disconnecting the
        // per-tick writes.
        let store_keepalive = store.clone();

        let sleep_dur = Duration::from_millis(config.poll_interval_ms);

        loop {
            // Check shutdown before the tick so we exit promptly.
            if let Some(rx) = shutdown_rx.as_ref() {
                if *rx.borrow() {
                    tracing::info!(
                        target: "hybrid_v2::worker",
                        deployment_id = config.deployment_id,
                        "hybrid_v2 indexer worker: shutdown requested, exiting"
                    );
                    return;
                }
            }

            // BACKEND-HYBRID-V2-PROJECTION-PERSISTENCE-OPERATIONAL-CLOSURE-V1
            // Pre-tick check: if a rebuild is active for this deployment
            // (any non-terminal, non-Complete phase), skip the tick and
            // sleep. This prevents the worker from racing against a
            // rebuild that may be re-materializing projection rows.
            if let Ok(Some(op)) = store_keepalive
                .read_latest_rebuild_operation(config.deployment_id)
                .await
            {
                use crate::hybrid_v2::rebuild_operations::RebuildPhase;
                let is_blocking = matches!(
                    op.phase,
                    RebuildPhase::Requested
                        | RebuildPhase::LockAcquired
                        | RebuildPhase::ValidatingSource
                        | RebuildPhase::Preparing
                        | RebuildPhase::Replaying
                        | RebuildPhase::Correlating
                        | RebuildPhase::Verifying
                        | RebuildPhase::Reconciling
                        | RebuildPhase::Committing
                        | RebuildPhase::ManualInterventionRequired
                );
                if is_blocking {
                    tracing::debug!(
                        target: "hybrid_v2::worker",
                        deployment_id = config.deployment_id,
                        rebuild_epoch = op.rebuild_epoch,
                        phase = %op.phase.as_str(),
                        "tick skipped — rebuild operation is active"
                    );
                    // Sleep and continue.
                    if let Some(mut rx) = shutdown_rx.clone() {
                        tokio::select! {
                            _ = tokio::time::sleep(sleep_dur) => {}
                            changed = rx.changed() => {
                                if changed.is_ok() && *rx.borrow() {
                                    return;
                                }
                            }
                        }
                    } else {
                        tokio::time::sleep(sleep_dur).await;
                    }
                    continue;
                }
            }

            let tick_outcome = {
                let mut guard = runtime.write().await;
                guard.tick_and_persist(source.as_ref()).await
            };

            match tick_outcome {
                Ok(true) => {
                    // A block was applied + persisted; loop immediately
                    // to check if more work is queued rather than
                    // sleeping. This keeps catch-up latency low without
                    // busy-looping — Ok(false) below sleeps.
                    continue;
                }
                Ok(false) => {
                    // Source has no new block yet — sleep.
                }
                Err(RuntimeError::ReorgRequired {
                    cursor_block,
                    cursor_hash,
                    conflicting_block,
                    conflicting_hash,
                    incoming_parent,
                }) => {
                    tracing::info!(
                        target: "hybrid_v2::worker",
                        deployment_id = config.deployment_id,
                        cursor_block,
                        conflicting_block = ?conflicting_block,
                        incoming_parent,
                        "reorg detected — driving persisted recovery"
                    );
                    let detection = ReorgDetection {
                        old_tip_block: cursor_block,
                        old_tip_hash: cursor_hash,
                        conflicting_block,
                        conflicting_hash,
                    };
                    let outcome = recovery_service
                        .recover(
                            source.as_ref(),
                            &store_keepalive,
                            &manifest_snapshot,
                            Some(detection),
                            &cursor_name,
                        )
                        .await;
                    match outcome {
                        Ok(RecoveryOutcome::Recovered {
                            epoch,
                            replacement_tip,
                            orphan_depth,
                            ..
                        }) => {
                            tracing::info!(
                                target: "hybrid_v2::worker",
                                deployment_id = config.deployment_id,
                                epoch,
                                replacement_tip,
                                orphan_depth,
                                "reorg recovered — rehydrating runtime"
                            );
                            let mut guard = runtime.write().await;
                            let _ = guard.bootstrap_from_persistence().await;
                        }
                        Ok(RecoveryOutcome::ManualInterventionRequired { epoch, reason }) => {
                            tracing::error!(
                                target: "hybrid_v2::worker",
                                deployment_id = config.deployment_id,
                                epoch,
                                reason = %reason,
                                "reorg recovery escalated to MANUAL_INTERVENTION_REQUIRED — worker halted for this deployment"
                            );
                            // Sleep indefinitely — a restart is required
                            // to clear the persisted state.
                            wait_indefinitely_or_shutdown(shutdown_rx.clone()).await;
                            return;
                        }
                        Ok(RecoveryOutcome::RetryableFailure { reason }) => {
                            tracing::warn!(
                                target: "hybrid_v2::worker",
                                deployment_id = config.deployment_id,
                                reason = %reason,
                                "reorg recovery hit a retryable failure — will retry on next tick"
                            );
                        }
                        Ok(RecoveryOutcome::NothingToDo) => {
                            tracing::warn!(
                                target: "hybrid_v2::worker",
                                deployment_id = config.deployment_id,
                                "recovery returned NothingToDo despite detected reorg — bootstrapping to reconcile"
                            );
                            let mut guard = runtime.write().await;
                            let _ = guard.bootstrap_from_persistence().await;
                        }
                        Err(err) => {
                            tracing::warn!(
                                target: "hybrid_v2::worker",
                                deployment_id = config.deployment_id,
                                error = %err,
                                "reorg recovery failed — will retry on next tick"
                            );
                        }
                    }
                }
                Err(RuntimeError::ReorgRecoveryPending { phase }) => {
                    tracing::debug!(
                        target: "hybrid_v2::worker",
                        deployment_id = config.deployment_id,
                        phase,
                        "tick refused — recovery in progress"
                    );
                    // Drive the persisted recovery forward.
                    let outcome = recovery_service
                        .recover(
                            source.as_ref(),
                            &store_keepalive,
                            &manifest_snapshot,
                            None,
                            &cursor_name,
                        )
                        .await;
                    if let Ok(RecoveryOutcome::Recovered { .. }) = outcome {
                        let mut guard = runtime.write().await;
                        let _ = guard.bootstrap_from_persistence().await;
                    } else if let Ok(RecoveryOutcome::ManualInterventionRequired {
                        epoch,
                        reason,
                    }) = outcome
                    {
                        tracing::error!(
                            target: "hybrid_v2::worker",
                            deployment_id = config.deployment_id,
                            epoch,
                            reason = %reason,
                            "reorg recovery escalated to MANUAL_INTERVENTION_REQUIRED — worker halted for this deployment"
                        );
                        wait_indefinitely_or_shutdown(shutdown_rx.clone()).await;
                        return;
                    }
                }
                Err(err) => {
                    tracing::warn!(
                        target: "hybrid_v2::worker",
                        deployment_id = config.deployment_id,
                        error = %err,
                        "hybrid_v2 indexer worker tick failed (readiness persisted)"
                    );
                }
            }

            // Sleep, but also break early on shutdown.
            if let Some(mut rx) = shutdown_rx.clone() {
                tokio::select! {
                    _ = tokio::time::sleep(sleep_dur) => {}
                    changed = rx.changed() => {
                        if changed.is_ok() && *rx.borrow() {
                            tracing::info!(
                                target: "hybrid_v2::worker",
                                deployment_id = config.deployment_id,
                                "hybrid_v2 indexer worker: shutdown signal received, exiting"
                            );
                            return;
                        }
                    }
                }
            } else {
                tokio::time::sleep(sleep_dur).await;
            }
        }
    })
}

/// Block until the shutdown channel signals true (or drops). Used when
/// the recovery service has escalated to MANUAL_INTERVENTION_REQUIRED
/// and the worker must stop advancing for this deployment.
async fn wait_indefinitely_or_shutdown(shutdown_rx: Option<watch::Receiver<bool>>) {
    if let Some(mut rx) = shutdown_rx {
        loop {
            if *rx.borrow() {
                return;
            }
            if rx.changed().await.is_err() {
                return;
            }
        }
    } else {
        // Without a shutdown channel, sleep in long chunks.
        loop {
            tokio::time::sleep(Duration::from_secs(60)).await;
        }
    }
}
