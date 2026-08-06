//! `BACKEND-HYBRID-V2-CHAIN-VIEW-PROVIDER-AND-RECONCILIATION-TASK-V1`
//! — periodic reconciliation worker.
//!
//! Ticks at a configurable cadence, pre-fetches an on-chain snapshot
//! via `RpcChainViewProvider::fetch_snapshot_at`, then invokes
//! `Reconciler::reconcile` and persists the result. Skips ticks when
//! the deployment's unified operation lock is contended (a rebuild or
//! reorg is in-flight). Never mutates any projection.
//!
//! Frozen posture:
//! - `RECONCILIATION_DRIFT_NEVER_AUTO_REPAIRS_PROJECTIONS`
//! - `RECONCILIATION_PROVIDER_FAILURE_IS_NOT_PROJECTION_DRIFT`
//! - `UNIFIED_OPERATION_LOCK_SERIALIZES_REORG_REBUILD_AND_RECONCILIATION`

use crate::hybrid_v2::chain_view::Reconciler;
use crate::hybrid_v2::manifest::ManifestParams;
use crate::hybrid_v2::persistence::HybridV2ProjectionStore;
use crate::hybrid_v2::rebuild_operations::OperationKind;
use crate::hybrid_v2::reconciler::{
    DriftClassification, ProviderAvailability, ReconciliationRecord,
};
use crate::hybrid_v2::rpc_chain_view::RpcChainViewProvider;
use crate::hybrid_v2::runtime::IndexerRuntime;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{watch, RwLock};
use tokio::task::JoinHandle;

/// Static configuration for the reconciliation worker.
#[derive(Debug, Clone)]
pub struct HybridV2ReconciliationWorkerConfig {
    pub deployment_id: i64,
    /// Tick cadence in milliseconds. `0` disables the worker.
    pub periodic_ms: u64,
    /// Maximum number of subaccounts to sample per tick. Bounded upstream.
    pub max_items_per_run: u64,
    /// Manifest bound at construction — provides `manifest_hash` for
    /// the reconciler comparison + module addresses for the provider.
    pub manifest: ManifestParams,
}

impl HybridV2ReconciliationWorkerConfig {
    pub fn new(
        deployment_id: i64,
        periodic_ms: u64,
        max_items_per_run: u64,
        manifest: ManifestParams,
    ) -> Self {
        Self {
            deployment_id,
            periodic_ms,
            max_items_per_run,
            manifest,
        }
    }
}

/// Spawn the reconciliation worker. Returns immediately when
/// `periodic_ms == 0`.
pub fn spawn_hybrid_v2_reconciliation_worker(
    runtime: Arc<RwLock<IndexerRuntime>>,
    provider: Arc<RpcChainViewProvider>,
    store: Arc<dyn HybridV2ProjectionStore>,
    config: HybridV2ReconciliationWorkerConfig,
    shutdown_rx: Option<watch::Receiver<bool>>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        if config.periodic_ms == 0 {
            tracing::info!(
                target: "hybrid_v2::reconciliation_worker",
                deployment_id = config.deployment_id,
                "reconciliation worker disabled (periodic_ms=0)"
            );
            return;
        }
        let sleep_dur = Duration::from_millis(config.periodic_ms);
        let reconciler = Reconciler {
            max_pairs_per_batch: config.max_items_per_run as usize,
        };
        loop {
            if let Some(rx) = shutdown_rx.as_ref() {
                if *rx.borrow() {
                    tracing::info!(
                        target: "hybrid_v2::reconciliation_worker",
                        deployment_id = config.deployment_id,
                        "reconciliation worker shutting down"
                    );
                    break;
                }
            }
            match tick_once(
                &runtime,
                provider.as_ref(),
                store.as_ref(),
                &reconciler,
                &config,
            )
            .await
            {
                Ok(TickOutcome::Ran { classification, .. }) => {
                    tracing::debug!(
                        target: "hybrid_v2::reconciliation_worker",
                        deployment_id = config.deployment_id,
                        classification = classification.as_str(),
                        "reconciliation tick completed"
                    );
                }
                Ok(TickOutcome::Skipped { reason }) => {
                    tracing::debug!(
                        target: "hybrid_v2::reconciliation_worker",
                        deployment_id = config.deployment_id,
                        reason = %reason,
                        "reconciliation tick skipped"
                    );
                }
                Err(err) => {
                    tracing::warn!(
                        target: "hybrid_v2::reconciliation_worker",
                        deployment_id = config.deployment_id,
                        error = %err,
                        "reconciliation tick failed"
                    );
                }
            }
            if let Some(rx) = shutdown_rx.as_ref() {
                tokio::select! {
                    _ = tokio::time::sleep(sleep_dur) => {}
                    _ = wait_shutdown(rx.clone()) => break,
                }
            } else {
                tokio::time::sleep(sleep_dur).await;
            }
        }
    })
}

#[derive(Debug)]
pub enum TickOutcome {
    Ran {
        classification: DriftClassification,
        reconciliation_id: Option<i64>,
    },
    Skipped {
        reason: String,
    },
}

/// Execute one reconciliation tick. Public so the admin route can drive
/// exactly the same flow synchronously in response to a REST trigger.
///
/// Returns `TickOutcome::Skipped` when the operation lock is contended
/// (a rebuild or reorg is in flight) — this is not an error.
pub async fn tick_once(
    runtime: &Arc<RwLock<IndexerRuntime>>,
    provider: &RpcChainViewProvider,
    store: &dyn HybridV2ProjectionStore,
    reconciler: &Reconciler,
    config: &HybridV2ReconciliationWorkerConfig,
) -> Result<TickOutcome, String> {
    // Snapshot the runtime cursor + projection state under a read lock.
    let (indexed_block, indexed_hash, subkeys, tokens_per_subkey, projection) = {
        let guard = runtime.read().await;
        let cursor = guard.cursor().clone();
        let projection = guard.projection().clone();
        // Bounded subkey population — every known subKey in the
        // projection is a candidate for on-chain view sampling. We
        // deduplicate + cap at `max_items_per_run` to keep each tick
        // bounded regardless of projection size.
        let mut subkey_set: std::collections::BTreeSet<String> =
            projection.subaccount_meta.keys().cloned().collect();
        for (sk, _) in projection.balances.keys() {
            subkey_set.insert(sk.clone());
        }
        for sk in projection.recovery_state.keys() {
            subkey_set.insert(sk.clone());
        }
        let subkeys: Vec<String> = subkey_set
            .into_iter()
            .take(config.max_items_per_run as usize)
            .collect();
        let mut tokens_per_subkey: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for (subkey, token) in projection.balances.keys() {
            tokens_per_subkey
                .entry(subkey.clone())
                .or_default()
                .push(token.clone());
        }
        (
            cursor.indexed_head_block,
            cursor.indexed_head_hash.clone(),
            subkeys,
            tokens_per_subkey,
            projection,
        )
    };

    if indexed_block == 0 {
        return Ok(TickOutcome::Skipped {
            reason: "runtime cursor at genesis".to_string(),
        });
    }

    let now_ms = now_ms();
    let epoch = now_ms;
    let guard_opt = store
        .try_acquire_operation_lock(
            config.deployment_id,
            OperationKind::Reconciliation,
            epoch,
            now_ms,
        )
        .await
        .map_err(|e| format!("operation lock: {e}"))?;
    let Some(guard) = guard_opt else {
        return Ok(TickOutcome::Skipped {
            reason: "operation lock contention".to_string(),
        });
    };

    let started = Instant::now();

    // Fetch snapshot from chain. Failure → PROVIDER_UNAVAILABLE (never
    // a projection drift).
    let fetch_res = provider
        .fetch_snapshot_at(indexed_block, &subkeys, &tokens_per_subkey)
        .await;
    let (classification, failure_detail, provider_available) = match fetch_res {
        Ok(()) => {
            let result = reconciler.reconcile(
                &config.manifest.manifest_hash,
                indexed_block,
                &projection,
                provider,
            );
            let (c, detail, _sample) = crate::hybrid_v2::reconciler::classify_public(&result);
            (c, detail, true)
        }
        Err(e) => {
            provider.mark_unavailable();
            (
                DriftClassification::ProviderUnavailable,
                Some(format!("provider fetch: {e}")),
                false,
            )
        }
    };

    let availability = if provider_available {
        ProviderAvailability::Available
    } else {
        ProviderAvailability::Unavailable
    };

    let record = ReconciliationRecord {
        reconciliation_id: None,
        deployment_id: config.deployment_id,
        ran_at_ms: now_ms,
        block_number_checked: indexed_block,
        block_hash_checked: indexed_hash,
        categories_checked: 3,
        items_checked: subkeys.len() as u64,
        converged_categories: if matches!(classification, DriftClassification::Converged) {
            3
        } else {
            0
        },
        divergent_categories: if classification.is_converged_or_transient() {
            0
        } else {
            1
        },
        classification: classification.clone(),
        mismatch_sample_json: failure_detail
            .as_ref()
            .map(|d| serde_json::json!({ "detail": d })),
        provider_availability: availability,
        failure_detail: failure_detail.clone(),
        duration_ms: started.elapsed().as_millis() as u64,
    };

    let id = store
        .insert_reconciliation_result(&record)
        .await
        .map_err(|e| format!("persist result: {e}"))?;

    // Release the lock — non-fatal on error.
    if let Err(e) = guard.release().await {
        tracing::warn!(
            target: "hybrid_v2::reconciliation_worker",
            deployment_id = config.deployment_id,
            error = %e,
            "operation lock release failed (non-fatal)"
        );
    }

    Ok(TickOutcome::Ran {
        classification,
        reconciliation_id: Some(id),
    })
}

fn now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

async fn wait_shutdown(mut rx: watch::Receiver<bool>) {
    loop {
        if *rx.borrow() {
            return;
        }
        if rx.changed().await.is_err() {
            return;
        }
    }
}
