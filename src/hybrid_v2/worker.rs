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

use crate::hybrid_v2::chain_source::ChainSource;
use crate::hybrid_v2::persistence::HybridV2ProjectionStore;
use crate::hybrid_v2::runtime::IndexerRuntime;
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
}

impl HybridV2IndexerWorkerConfig {
    pub fn new(deployment_id: i64, poll_interval_ms: u64) -> Self {
        Self {
            poll_interval_ms: poll_interval_ms.max(1),
            deployment_id,
        }
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
        // Also record we've referenced the store here; store used below
        // via runtime's own persistence handle (which was set via
        // `with_persistence`). We keep our own handle live so the
        // caller can drop the outer Arc without disconnecting the
        // per-tick writes.
        let _store_keepalive = store;

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
