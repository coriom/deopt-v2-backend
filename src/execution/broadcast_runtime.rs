//! PERPS_BASE_SEPOLIA_BACKEND_RUNTIME_BOOT_INTEGRATION_V1 —
//! composed runtime handle for the Perps closed-test broadcast
//! subsystem.
//!
//! Owns:
//! * the [`BroadcastPolicy`] instance (RPC + signer + config);
//! * the reconciler cancel token (via
//!   [`BroadcastReconcilerCancel`]);
//! * the reconciler `JoinHandle`;
//! * a lightweight [`BroadcastReadiness`] snapshot the readiness
//!   endpoint reads to publish subsystem state without exposing
//!   internal implementation details.
//!
//! ## Fail-closed default
//!
//! `BroadcastRuntime::from_config_and_repository` is called ONLY when
//! the deployment has explicitly enabled real closed-test broadcast
//! via BOTH `EXECUTOR_REAL_BROADCAST_ENABLED=true` AND
//! `PERPS_CLOSED_TEST_ENABLED=true`. All other startup paths preserve
//! current backend behavior — no signer construction, no RPC probe.
//!
//! ## Startup sequence (called from main.rs)
//!
//! 1. `ExecutionConfig::validate_startup()` — mainnet/local-signer
//!    guards, gas caps, RPC URL presence.
//! 2. [`validate_signer_triad`] —
//!    `EXECUTOR_FROM_ADDRESS ↔ HV2_SIGNER_EXPECTED_ADDRESS ↔ HV2_EXECUTOR_ADDRESS`.
//! 3. Construct [`crate::execution::signer::ExecutorSigner`] (LocalDev)
//!    or [`crate::hybrid_v2::execution::HybridV2KmsSignerBridge`]
//!    (Remote — future wiring).
//! 4. Construct [`BroadcastPolicy`] with `verify_pme_event = true`.
//! 5. [`startup_preflight`] — chain_id + `PME.isExecutor(from)` +
//!    `PME.paused()`.
//! 6. [`initial_reconciliation`] — resolve durable in-flight rows
//!    from prior process.
//! 7. [`spawn_broadcast_reconciler`] — background tokio loop.
//! 8. Readiness advertises `broadcast_ready = true` ONLY after step 7.
//!
//! Any failure at 1–6 returns an error; the caller MUST refuse to
//! advertise readiness and MUST NOT let the execution worker call
//! `broadcast_intent`.

use crate::error::{BackendError, Result};
use crate::execution::broadcast_policy::{BroadcastPolicy, ReconcileSummary};
use crate::execution::broadcast_reconciler::{
    initial_reconciliation, spawn_broadcast_reconciler, startup_preflight,
    BroadcastReconcilerCancel, ReconcilerConfig,
};
use crate::execution::config::ExecutionConfig;
use crate::execution::executor::ExecutionIntentRepository;
use crate::execution::rpc::{
    EthCallProvider, HttpJsonRpcProvider, TransactionBroadcastProvider, TransactionReceiptProvider,
};
use crate::execution::signer::ExecutorSigner;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::task::JoinHandle;
use tracing::{info, warn};

/// Public-safe subsystem readiness snapshot. Values change only via
/// the runtime lifecycle; readers see a consistent view.
#[derive(Clone, Debug, Default)]
pub struct BroadcastReadiness {
    enabled: Arc<AtomicBool>,
    preflight_ok: Arc<AtomicBool>,
    initial_reconciliation_ok: Arc<AtomicBool>,
    reconciler_running: Arc<AtomicBool>,
}

impl BroadcastReadiness {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn enabled(&self) -> bool {
        self.enabled.load(Ordering::SeqCst)
    }
    pub fn preflight_ok(&self) -> bool {
        self.preflight_ok.load(Ordering::SeqCst)
    }
    pub fn initial_reconciliation_ok(&self) -> bool {
        self.initial_reconciliation_ok.load(Ordering::SeqCst)
    }
    pub fn reconciler_running(&self) -> bool {
        self.reconciler_running.load(Ordering::SeqCst)
    }
    /// Whether the broadcast subsystem is fully ready to accept
    /// real closed-test broadcasts. Only true after ALL of:
    /// enabled + preflight_ok + initial_reconciliation_ok +
    /// reconciler_running.
    pub fn broadcast_ready(&self) -> bool {
        self.enabled()
            && self.preflight_ok()
            && self.initial_reconciliation_ok()
            && self.reconciler_running()
    }
    fn mark_enabled(&self) {
        self.enabled.store(true, Ordering::SeqCst);
    }
    fn mark_preflight_ok(&self) {
        self.preflight_ok.store(true, Ordering::SeqCst);
    }
    fn mark_initial_reconciliation_ok(&self) {
        self.initial_reconciliation_ok.store(true, Ordering::SeqCst);
    }
    fn mark_reconciler_running(&self) {
        self.reconciler_running.store(true, Ordering::SeqCst);
    }
    pub fn mark_reconciler_stopped(&self) {
        self.reconciler_running.store(false, Ordering::SeqCst);
    }
}

/// Composed runtime handle for the Perps broadcast subsystem.
pub struct BroadcastRuntime<R, Repo>
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
    pub policy: Arc<BroadcastPolicy<R, ExecutorSigner>>,
    pub repository: Arc<Repo>,
    pub cancel: BroadcastReconcilerCancel,
    pub join_handle: Option<JoinHandle<()>>,
    pub readiness: BroadcastReadiness,
    pub initial_reconciliation_summary: ReconcileSummary,
}

impl<R, Repo> BroadcastRuntime<R, Repo>
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
    /// Signal the reconciler task to stop and update the readiness
    /// snapshot. Safe to call multiple times.
    pub fn shutdown(&self) {
        self.cancel.cancel();
        self.readiness.mark_reconciler_stopped();
    }

    /// Await the reconciler task's clean termination. Returns Ok on
    /// join; err if the task panicked.
    pub async fn join(&mut self) -> std::result::Result<(), tokio::task::JoinError> {
        if let Some(handle) = self.join_handle.take() {
            handle.await
        } else {
            Ok(())
        }
    }
}

/// Build a [`BroadcastRuntime`] from an already-constructed policy +
/// repository + readiness handle. Runs the startup ordering:
/// preflight → initial reconciliation → reconciler spawn. Returns an
/// error if any predicate fails.
///
/// Callers that need to construct the policy from
/// `HttpJsonRpcProvider` should use [`build_broadcast_runtime`]
/// instead (which composes RPC + signer + policy in one shot from
/// [`ExecutionConfig`]).
pub async fn wire_broadcast_runtime<R, Repo>(
    policy: Arc<BroadcastPolicy<R, ExecutorSigner>>,
    repository: Arc<Repo>,
    reconciler_config: ReconcilerConfig,
    readiness: BroadcastReadiness,
) -> Result<BroadcastRuntime<R, Repo>>
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
    readiness.mark_enabled();
    // Step 5 — preflight.
    startup_preflight(policy.as_ref()).await?;
    readiness.mark_preflight_ok();
    info!("perps broadcast runtime: startup preflight OK");

    // Step 6 — initial reconciliation.
    let summary = initial_reconciliation(
        policy.as_ref(),
        repository.as_ref(),
        reconciler_config.batch_size,
    )
    .await?;
    readiness.mark_initial_reconciliation_ok();
    info!(
        confirmed = summary.confirmed,
        failed = summary.failed,
        still_pending = summary.still_pending,
        "perps broadcast runtime: initial reconciliation complete"
    );

    // Step 7 — spawn the reconciler.
    let (join_handle, cancel) =
        spawn_broadcast_reconciler(policy.clone(), repository.clone(), reconciler_config);
    readiness.mark_reconciler_running();
    info!("perps broadcast runtime: reconciler task spawned");

    Ok(BroadcastRuntime {
        policy,
        repository,
        cancel,
        join_handle: Some(join_handle),
        readiness,
        initial_reconciliation_summary: summary,
    })
}

/// Convenience constructor: builds RPC + signer + policy from an
/// [`ExecutionConfig`] and hands the composed runtime back. Only
/// supports the `LocalDev` signer path (Base Sepolia closed test).
/// Remote signer wiring requires a project-specific
/// `HybridV2KmsSignerBridge` factory which is deployment-side wiring.
pub async fn build_broadcast_runtime<Repo>(
    config: ExecutionConfig,
    repository: Arc<Repo>,
    reconciler_config: ReconcilerConfig,
    readiness: BroadcastReadiness,
) -> Result<BroadcastRuntime<HttpJsonRpcProvider, Repo>>
where
    Repo: ExecutionIntentRepository + 'static,
{
    // Signer triad — the two additional addresses come from env at
    // this construction site.
    let hv2_executor = std::env::var("HV2_EXECUTOR_ADDRESS")
        .map(crate::types::AccountId::new)
        .map_err(|_| {
            BackendError::Config(
                "HV2_EXECUTOR_ADDRESS is required when EXECUTOR_REAL_BROADCAST_ENABLED=true"
                    .to_string(),
            )
        })?;
    let hv2_signer_expected = std::env::var("HV2_SIGNER_EXPECTED_ADDRESS")
        .map(crate::types::AccountId::new)
        .map_err(|_| {
            BackendError::Config(
                "HV2_SIGNER_EXPECTED_ADDRESS is required when EXECUTOR_REAL_BROADCAST_ENABLED=true"
                    .to_string(),
            )
        })?;
    crate::execution::broadcast_policy::validate_signer_triad(
        &config.executor_from_address,
        &hv2_signer_expected,
        &hv2_executor,
    )?;

    // Signer. LocalDev only in this V1 factory — Remote requires the
    // `HybridV2KmsSignerBridge` factory which is deployment-side.
    let private_key = config.executor_private_key.as_ref().ok_or_else(|| {
        BackendError::Config(
            "EXECUTOR_PRIVATE_KEY is required for LocalDev signer path in V1 boot integration"
                .to_string(),
        )
    })?;
    let signer = Arc::new(ExecutorSigner::from_private_key(private_key)?);

    // RPC.
    let rpc_url = config
        .rpc_url
        .as_ref()
        .ok_or_else(|| BackendError::Config("RPC_URL is required for real broadcast".to_string()))?
        .clone();
    let rpc = HttpJsonRpcProvider::new(rpc_url);

    // Policy.
    let mut policy = BroadcastPolicy::new(config, rpc, signer);
    policy.verify_pme_event = true;
    let policy = Arc::new(policy);

    wire_broadcast_runtime(policy, repository, reconciler_config, readiness).await
}

/// Log a warning + return None when construction is refused. Used by
/// the main.rs boot path to preserve current behavior for operators
/// who have not opted in.
pub fn refuse_broadcast_runtime_disabled(reason: &str) {
    warn!(
        reason,
        "perps broadcast runtime: disabled (fail-closed default)"
    );
}

/// Execute one batch of pending intents: for each intent with
/// calldata-ready signatures, invoke
/// [`BroadcastPolicy::broadcast_intent`] and observe the outcome via
/// [`crate::monitoring::observe_broadcast_outcome`]. Errors on
/// individual intents are logged; the batch continues (single-row
/// failures never kill the loop).
///
/// This is the connection point the milestone requires:
///     matched execution → executor worker → BroadcastPolicy::broadcast_intent.
pub async fn execute_pending_batch<R, Repo>(
    policy: &BroadcastPolicy<R, ExecutorSigner>,
    repository: &Repo,
    batch_size: u32,
) -> Result<usize>
where
    R: EthCallProvider
        + TransactionBroadcastProvider
        + TransactionReceiptProvider
        + Clone
        + Send
        + Sync,
    Repo: ExecutionIntentRepository,
{
    // PERPS_BASE_SEPOLIA_CLOSED_TEST_RUNTIME_ARMING_AND_ACCOUNTING_V1 —
    // Arming gate. If disarmed, the worker MUST NOT initiate a NEW
    // broadcast for any intent. Durable in-flight reconciliation of
    // ALREADY-Prepared rows remains the reconciler's responsibility
    // and is unaffected by this gate — this function only starts NEW
    // broadcasts.
    if !policy.config.perps_closed_test_broadcast_armed {
        return Ok(0);
    }
    let armed_id = match policy.config.perps_closed_test_broadcast_intent_id {
        Some(id) => id,
        None => {
            // Startup validation should have caught this; log and skip.
            warn!("broadcast armed but no intent_id configured — refusing to broadcast");
            return Ok(0);
        }
    };
    let intents = repository
        .list_pending_execution_intents(batch_size)
        .await?;
    let mut processed = 0usize;
    for intent in &intents {
        // Only the exact armed UUID is eligible for a NEW broadcast.
        if intent.intent_id != armed_id {
            continue;
        }
        let sigs = repository
            .get_execution_intent_signatures(intent.intent_id)
            .await?;
        if !sigs.calldata_ready() {
            // Not yet signed by both parties; skip. The dry-run
            // preview tick will pick it up.
            continue;
        }
        match policy.broadcast_intent(repository, intent, &sigs).await {
            Ok(outcome) => {
                crate::monitoring::observe_broadcast_outcome(&outcome);
                processed += 1;
            }
            Err(error) => {
                warn!(
                    intent_id = %intent.intent_id,
                    %error,
                    "broadcast_intent failed; row remains in prior state"
                );
            }
        }
    }
    Ok(processed)
}

/// Spawn a bounded periodic executor loop that invokes
/// [`execute_pending_batch`]. Uses the same
/// [`BroadcastReconcilerCancel`] as the reconciler for coordinated
/// graceful shutdown.
pub fn spawn_broadcast_executor<R, Repo>(
    policy: Arc<BroadcastPolicy<R, ExecutorSigner>>,
    repository: Arc<Repo>,
    poll_interval_ms: u64,
    batch_size: u32,
    cancel: BroadcastReconcilerCancel,
) -> JoinHandle<()>
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
    tokio::spawn(async move {
        info!(
            poll_interval_ms,
            batch_size, "perps broadcast executor started"
        );
        loop {
            if cancel.is_cancelled() {
                info!("perps broadcast executor shutting down");
                break;
            }
            match execute_pending_batch(policy.as_ref(), repository.as_ref(), batch_size).await {
                Ok(processed) => {
                    if processed > 0 {
                        info!(processed, "broadcast executor tick processed intents");
                    }
                }
                Err(error) => {
                    warn!(%error, "broadcast executor tick failed; loop continues");
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(poll_interval_ms)).await;
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readiness_defaults_all_false() {
        let r = BroadcastReadiness::new();
        assert!(!r.enabled());
        assert!(!r.preflight_ok());
        assert!(!r.initial_reconciliation_ok());
        assert!(!r.reconciler_running());
        assert!(!r.broadcast_ready());
    }

    #[test]
    fn readiness_broadcast_ready_requires_all_flags() {
        let r = BroadcastReadiness::new();
        r.mark_enabled();
        assert!(!r.broadcast_ready());
        r.mark_preflight_ok();
        assert!(!r.broadcast_ready());
        r.mark_initial_reconciliation_ok();
        assert!(!r.broadcast_ready());
        r.mark_reconciler_running();
        assert!(r.broadcast_ready());
    }

    #[test]
    fn shutdown_flips_reconciler_running() {
        let r = BroadcastReadiness::new();
        r.mark_reconciler_running();
        assert!(r.reconciler_running());
        r.mark_reconciler_stopped();
        assert!(!r.reconciler_running());
    }
}
