//! `BACKEND-HYBRID-V2-PRODUCTION-SIGNER-BOOTSTRAP-AND-STARTUP-WIRING-V1`
//! Public library entry point for wiring the pre-broadcast execution
//! orchestrator + performing the Part-F bootstrap identity probe.
//!
//! Split out of `main.rs` so integration test binaries can drive the
//! same startup path the production binary uses without a re-
//! implementation drift risk. `main.rs` re-exports these functions
//! verbatim; PG integration tests that assert the wire path are
//! REQUIRED to invoke this module (never `AppState::with_hybrid_v2
//! _execution_orchestrator` directly).
//!
//! Frozen posture (unchanged from main.rs's original inline code):
//!
//! * `BROADCAST_STRICTLY_FORBIDDEN` — this module never invokes any
//!   `send_*` RPC method. Signer construction runs through
//!   `HybridV2SignerBuilder`; identity probe is a `GET /hybrid_v2
//!   /identity` — no `sign`, no `broadcast`.
//! * `BASE_MAINNET_8453_IS_FORBIDDEN` — refused up-front in
//!   `bootstrap_probe_signer_identity` (before any transport).
//! * `SIGNER_UNAVAILABLE_NEVER_FALLS_BACK_TO_LOCAL_RAW_KEY` — the
//!   signer builder either returns the real `HybridV2KmsSignerBridge`
//!   or an `Err(...)` that the caller downgrades to
//!   `orchestrator = None` + a WARN log. No local key path is wired.

use std::sync::Arc;
use std::time::Duration;

use tracing::{info, warn};

use crate::api::AppState;
use crate::hybrid_v2::config::HybridV2ExecutionConfig;
use crate::hybrid_v2::execution::broadcast_outbox::BroadcastOutbox;
use crate::hybrid_v2::execution::broadcast_rpc::{
    ExecutionBroadcastRpcClient, HttpExecutionBroadcastRpcClient,
};
use crate::hybrid_v2::execution::broadcast_worker::BroadcastConfirmationWorker;
use crate::hybrid_v2::execution::{
    ExecutionOrchestrator, HttpExecutionRpcClient, HttpSignerTransport, HybridV2SignerBuilder,
    SystemClock, TargetPolicy,
};

/// Package A, Part I — construct the pre-broadcast execution
/// orchestrator from env + AppState's projection store. Fail-closed:
///
/// * `Ok(None)` — execution disabled (`HV2_EXECUTION_ENABLED=false`).
/// * `Ok(Some(_))` — orchestrator constructed successfully.
/// * `Err(reason)` — validation, signer, or RPC construction failed.
///   Caller downgrades to `orchestrator = None` + logs a WARN. The
///   read-side backend keeps serving.
pub async fn wire_hybrid_v2_execution_orchestrator(
    state: &AppState,
    chain_id: u64,
) -> std::result::Result<Option<(Arc<ExecutionOrchestrator>, HybridV2ExecutionConfig)>, String> {
    let execution_config = HybridV2ExecutionConfig::from_env()
        .map_err(|e| format!("HV2 execution config from_env: {e}"))?;
    if !execution_config.execution_enabled {
        return Ok(None);
    }
    execution_config
        .validate_startup(chain_id)
        .map_err(|e| format!("HV2 execution config validate_startup: {e}"))?;
    let store = state.hybrid_v2_projection_store.clone().ok_or_else(|| {
        "HV2 execution requires the projection store; HYBRID_V2_ENABLED=true?".to_string()
    })?;
    let manifest = state.hybrid_v2_manifest.clone().ok_or_else(|| {
        "HV2 execution requires a manifest bound to AppState via reconciliation wiring".to_string()
    })?;
    let rpc_url = execution_config.rpc_url.clone().ok_or_else(|| {
        "HV2 execution requires HV2_EXECUTION_RPC_URL (or HYBRID_V2_RPC_URL)".to_string()
    })?;
    let rpc_timeout = Duration::from_millis(execution_config.rpc_timeout_ms);
    let rpc: Arc<dyn crate::hybrid_v2::execution::ExecutionRpcClient> = Arc::new(
        HttpExecutionRpcClient::new(rpc_url.clone(), rpc_timeout, 3)
            .map_err(|e| format!("HV2 execution RPC construction: {e}"))?,
    );
    let target_policy = Arc::new(
        TargetPolicy::from_manifest(&manifest)
            .map_err(|e| format!("HV2 execution target policy: {e}"))?,
    );
    // Part F — bootstrap identity validation. Refuse to wire the
    // orchestrator when the vendor self-reports a different signer
    // address or a different chain_id than the config expects. A
    // transport-level failure (unreachable microservice) does NOT
    // refuse — the orchestrator is still wired and `availability()`
    // reports `Unavailable` at admin-request time (Part E).
    bootstrap_probe_signer_identity(&execution_config, chain_id).await?;
    let signer = HybridV2SignerBuilder::build(&execution_config)
        .map_err(|e| format!("HV2 execution signer builder: {e}"))?;
    let gas_policy = Arc::new(execution_config.gas_policy.clone());
    let deployment_id = state
        .hybrid_v2_read
        .list()
        .into_iter()
        .next()
        .map(|entry| entry.deployment_id as i64)
        .ok_or_else(|| "HV2 execution requires at least one deployment in AppState".to_string())?;
    let orchestrator = Arc::new(ExecutionOrchestrator {
        store,
        rpc,
        signer,
        target_policy,
        gas_policy,
        deployment_id,
        chain_id,
        executor_address: execution_config.executor_address,
        simulation_max_age_ms: execution_config.simulation_max_age_ms,
        clock: Arc::new(SystemClock),
    });
    Ok(Some((orchestrator, execution_config)))
}

/// Part F — narrow, non-transaction identity probe against the signer
/// microservice. Returns `Ok(())` in three cases:
///   1. execution disabled / provider not set → no probe (caller
///      already filtered).
///   2. probe reachable AND identity/chain match config → Ok.
///   3. probe unreachable AND caller opted into a transient bootstrap
///      (`HV2_SIGNER_BOOTSTRAP_STRICT` unset) → Ok with a WARN log;
///      the orchestrator is still wired and reports `Unavailable`
///      when the admin invokes it.
/// Returns `Err(reason)` when identity mismatch, chain mismatch, or
/// bootstrap-strict + transport failure.
pub async fn bootstrap_probe_signer_identity(
    cfg: &HybridV2ExecutionConfig,
    chain_id: u64,
) -> std::result::Result<(), String> {
    // Belt-and-suspenders: refuse Base mainnet at bootstrap. The
    // config validator already refused it — this is defence-in-depth.
    if chain_id == 8453 {
        return Err(
            "PRODUCTION_SIGNER_BOOTSTRAP_REFUSED: Base mainnet chain_id=8453 is forbidden"
                .to_string(),
        );
    }
    let Some(endpoint) = cfg.signer_endpoint.as_deref() else {
        return Ok(());
    };
    let Some(expected) = cfg.expected_signer_address else {
        return Ok(());
    };
    // Load mTLS material (optional for loopback). Errors here are
    // structural — a broken PEM MUST refuse.
    let mtls_cert =
        crate::hybrid_v2::execution::read_pem_if_configured(&cfg.signer_mtls_cert_pem_path)
            .map_err(|e| format!("signer bootstrap mTLS cert: {e}"))?;
    let mtls_key =
        crate::hybrid_v2::execution::read_pem_if_configured(&cfg.signer_mtls_key_pem_path)
            .map_err(|e| format!("signer bootstrap mTLS key: {e}"))?;
    let root_ca = crate::hybrid_v2::execution::read_pem_if_configured(&cfg.signer_root_ca_pem_path)
        .map_err(|e| format!("signer bootstrap root CA: {e}"))?;
    let transport = HttpSignerTransport::new(
        endpoint.to_string(),
        expected,
        mtls_cert,
        mtls_key,
        root_ca,
        cfg.signer_request_timeout_ms.min(30_000),
        cfg.signer_max_retries,
    )
    .map_err(|e| format!("signer bootstrap transport: {e}"))?;
    let strict = std::env::var("HV2_SIGNER_BOOTSTRAP_STRICT")
        .ok()
        .map(|v| matches!(v.trim(), "1" | "true" | "on" | "yes"))
        .unwrap_or(false);
    match transport.fetch_identity().await {
        Ok(probe) => {
            if probe.signer_address != expected {
                return Err(format!(
                    "PRODUCTION_SIGNER_BOOTSTRAP_IDENTITY_MISMATCH: endpoint reports \
                     signer_address 0x{} but config expects 0x{}",
                    hex_encode20(&probe.signer_address),
                    hex_encode20(&expected)
                ));
            }
            if probe.chain_id != chain_id {
                return Err(format!(
                    "PRODUCTION_SIGNER_BOOTSTRAP_CHAIN_MISMATCH: endpoint reports \
                     chain_id={} but deployment expects chain_id={}",
                    probe.chain_id, chain_id
                ));
            }
            if probe.chain_id == 8453 {
                return Err(
                    "PRODUCTION_SIGNER_BOOTSTRAP_REFUSED: endpoint reports Base mainnet"
                        .to_string(),
                );
            }
            info!(
                chain_id = probe.chain_id,
                "hybrid_v2 signer identity bootstrap PASSED"
            );
            Ok(())
        }
        Err(err) => {
            if strict {
                Err(format!(
                    "PRODUCTION_SIGNER_BOOTSTRAP_TRANSPORT_FAILED (strict): {}",
                    err
                ))
            } else {
                warn!(
                    reason = %err,
                    "hybrid_v2 signer identity bootstrap probe unreachable — orchestrator \
                     will be wired but admin prepare returns 503 until the microservice is up. \
                     Set HV2_SIGNER_BOOTSTRAP_STRICT=1 to refuse at startup instead."
                );
                Ok(())
            }
        }
    }
}

/// BACKEND-HYBRID-V2-BROADCAST-AND-CONFIRMATION-V1 (Package D). Construct
/// the broadcast RPC client + outbox + confirmation worker from env +
/// AppState's projection store. Fail-closed with the same three-outcome
/// contract as [`wire_hybrid_v2_execution_orchestrator`]:
///
/// * `Ok(None)` — broadcast disabled (`HV2_BROADCAST_ENABLED=false` OR
///   `HV2_EXECUTION_ENABLED=false`).
/// * `Ok(Some(_))` — outbox + worker constructed successfully.
/// * `Err(reason)` — validation, RPC construction, or config missing
///   failed. Caller downgrades to `outbox = None` + logs a WARN. The
///   read-side backend keeps serving.
///
/// Frozen safety enforced at wire time:
/// * Base mainnet chain_id `8453` is refused at every gate — env
///   validation (`validate_startup`), `HttpExecutionBroadcastRpcClient::new`
///   (via `expected_chain_id`), and here as belt-and-suspenders.
/// * `allowed_broadcast_chain_ids` containing 8453 is refused before
///   any RPC construction.
/// * The RPC client's method allowlist is enforced by
///   `HttpExecutionBroadcastRpcClient::check_method`; wiring does not
///   widen that allowlist.
pub async fn wire_hybrid_v2_broadcast(
    state: &AppState,
    chain_id: u64,
) -> std::result::Result<
    Option<(
        Arc<BroadcastOutbox>,
        Arc<BroadcastConfirmationWorker>,
        Arc<dyn ExecutionBroadcastRpcClient>,
        HybridV2ExecutionConfig,
    )>,
    String,
> {
    if chain_id == 8453 {
        return Err("HV2_BROADCAST_REFUSED: Base mainnet chain_id=8453 is forbidden".to_string());
    }
    let execution_config = HybridV2ExecutionConfig::from_env()
        .map_err(|e| format!("HV2 broadcast config from_env: {e}"))?;
    if !execution_config.execution_enabled {
        return Ok(None);
    }
    if !execution_config.broadcast_enabled {
        return Ok(None);
    }
    // validate_startup runs the full bounded-config sweep including the
    // Base-mainnet refusal + allowed-chain-id + URL-scheme checks.
    execution_config
        .validate_startup(chain_id)
        .map_err(|e| format!("HV2 broadcast config validate_startup: {e}"))?;
    // Belt-and-suspenders: refuse if the allowed list carries 8453.
    if execution_config
        .allowed_broadcast_chain_ids
        .iter()
        .any(|c| *c == 8453)
    {
        return Err("HV2_BROADCAST_REFUSED: allowed_broadcast_chain_ids contains 8453".to_string());
    }
    let store = state.hybrid_v2_projection_store.clone().ok_or_else(|| {
        "HV2 broadcast requires the projection store; HYBRID_V2_ENABLED=true?".to_string()
    })?;
    let broadcast_rpc_url = execution_config
        .broadcast_rpc_url
        .clone()
        .ok_or_else(|| "HV2 broadcast requires HV2_BROADCAST_RPC_URL".to_string())?;
    let timeout = Duration::from_millis(execution_config.broadcast_rpc_timeout_ms);
    let rpc: Arc<dyn ExecutionBroadcastRpcClient> = Arc::new(
        HttpExecutionBroadcastRpcClient::new(
            broadcast_rpc_url,
            timeout,
            execution_config.broadcast_rpc_max_retries,
            Some(chain_id),
        )
        .map_err(|e| format!("HV2 broadcast RPC construction: {e}"))?,
    );
    let deployment_id = state
        .hybrid_v2_read
        .list()
        .into_iter()
        .next()
        .map(|entry| entry.deployment_id as i64)
        .ok_or_else(|| "HV2 broadcast requires at least one deployment in AppState".to_string())?;
    let clock: Arc<dyn crate::hybrid_v2::execution::orchestrator::Clock> = Arc::new(SystemClock);

    let outbox = Arc::new(BroadcastOutbox {
        store: store.clone(),
        rpc: rpc.clone(),
        clock: clock.clone(),
        deployment_id,
    });
    let worker = Arc::new(BroadcastConfirmationWorker {
        store,
        rpc: rpc.clone(),
        clock,
        deployment_id,
        chain_id,
        confirmation_depth: execution_config.confirmation_depth,
        poll_interval_ms: execution_config.receipt_poll_interval_ms,
        poll_timeout_ms: execution_config.receipt_poll_timeout_ms,
        max_batch_size: 32,
        max_pending_age_ms: execution_config.max_pending_age_ms,
    });
    Ok(Some((outbox, worker, rpc, execution_config)))
}

fn hex_encode20(bytes: &[u8; 20]) -> String {
    let mut s = String::with_capacity(40);
    for b in bytes.iter() {
        s.push_str(&format!("{:02x}", b));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::EngineState;

    /// Package D wiring — with_hybrid_v2_broadcast_unavailable populates
    /// the reason field and leaves the outbox/worker/rpc/config unset.
    #[test]
    fn appstate_with_hybrid_v2_broadcast_unavailable_populates_reason() {
        let state = AppState::new(EngineState::new(Vec::new()))
            .with_hybrid_v2_broadcast_unavailable("BROADCAST_DISABLED_TEST");
        assert!(state.hybrid_v2_broadcast_outbox.is_none());
        assert!(state.hybrid_v2_broadcast_worker.is_none());
        assert!(state.hybrid_v2_broadcast_rpc.is_none());
        assert!(state.hybrid_v2_broadcast_config.is_none());
        assert_eq!(
            state.hybrid_v2_broadcast_unavailable_reason.as_deref(),
            Some("BROADCAST_DISABLED_TEST")
        );
    }

    /// Package D wiring — a fresh AppState defaults to the fail-closed
    /// posture: outbox/worker/rpc absent, reason populated.
    #[test]
    fn appstate_defaults_to_broadcast_unavailable() {
        let state = AppState::new(EngineState::new(Vec::new()));
        assert!(state.hybrid_v2_broadcast_outbox.is_none());
        assert!(state.hybrid_v2_broadcast_worker.is_none());
        assert!(state.hybrid_v2_broadcast_rpc.is_none());
        assert!(state.hybrid_v2_broadcast_config.is_none());
        assert!(state
            .hybrid_v2_broadcast_unavailable_reason
            .as_deref()
            .unwrap_or_default()
            .starts_with("BROADCAST_DISABLED"));
    }

    /// Package D wiring — the broadcast wire refuses Base mainnet at
    /// chain_id = 8453 regardless of env content.
    #[tokio::test]
    async fn wire_hybrid_v2_broadcast_refuses_base_mainnet() {
        let state = AppState::new(EngineState::new(Vec::new()));
        let out = wire_hybrid_v2_broadcast(&state, 8453).await;
        match out {
            Err(err) => {
                assert!(err.contains("HV2_BROADCAST_REFUSED"), "err = {err}");
                assert!(err.contains("8453"), "err = {err}");
            }
            Ok(_) => panic!("wire must refuse base mainnet chain_id=8453"),
        }
    }

    /// Package D wiring — when neither execution nor broadcast are
    /// enabled via env, the wire returns Ok(None) (disabled).
    /// NOTE: this test does not manipulate real env vars so it relies
    /// on the from_env default (`broadcast_enabled = false`).
    #[tokio::test]
    async fn wire_hybrid_v2_broadcast_returns_none_when_disabled() {
        // Neutralize any inherited HV2_ env vars for this thread — the
        // wire path reads from process env, but if a foreign env leaks
        // execution_enabled=true we still fail closed on the projection
        // store being None.
        let state = AppState::new(EngineState::new(Vec::new()));
        // Best-effort: many CI environments do not set HV2_ vars.
        let out = wire_hybrid_v2_broadcast(&state, 84532).await;
        match out {
            Ok(None) => {}
            // Also acceptable — when execution/broadcast are enabled by
            // env, the wire fails on missing projection store (Err).
            Err(reason) => {
                assert!(
                    reason.contains("projection store")
                        || reason.contains("HV2_BROADCAST_RPC_URL")
                        || reason.contains("validate_startup")
                        || reason.contains("deployment")
                        || reason.contains("execution config"),
                    "unexpected wire err: {reason}"
                );
            }
            Ok(Some(_)) => {
                panic!("wire returned Some in a test context without a projection store");
            }
        }
    }

    /// Package D wiring — with_hybrid_v2_broadcast populates all four
    /// fields when supplied a constructed outbox / worker / rpc / cfg.
    /// We use a shared PostgresHybridV2ProjectionStore-shaped
    /// fabrication via the in-memory adapter that already satisfies
    /// the trait.
    #[tokio::test]
    async fn appstate_with_hybrid_v2_broadcast_populates_fields() {
        use crate::hybrid_v2::execution::broadcast_outbox::BroadcastOutbox;
        use crate::hybrid_v2::execution::broadcast_worker::BroadcastConfirmationWorker;
        use crate::hybrid_v2::execution::orchestrator::SystemClock;
        use crate::hybrid_v2::persistence::InMemoryProjectionStore;

        // Deterministic mock rpc — implemented behind a small stub so
        // this unit test does not require the test-signer feature.
        struct StubRpc;
        #[async_trait::async_trait]
        impl crate::hybrid_v2::execution::broadcast_rpc::ExecutionBroadcastRpcClient for StubRpc {
            async fn chain_id(
                &self,
            ) -> Result<u64, crate::hybrid_v2::execution::broadcast_rpc::BroadcastRpcError>
            {
                Ok(84532)
            }
            async fn head_block_number(
                &self,
            ) -> Result<u64, crate::hybrid_v2::execution::broadcast_rpc::BroadcastRpcError>
            {
                Ok(0)
            }
            async fn finalized_block_number(
                &self,
            ) -> Result<Option<u64>, crate::hybrid_v2::execution::broadcast_rpc::BroadcastRpcError>
            {
                Ok(None)
            }
            async fn block_header_by_number(
                &self,
                _n: u64,
            ) -> Result<
                Option<crate::hybrid_v2::execution::broadcast_rpc::BlockHeader>,
                crate::hybrid_v2::execution::broadcast_rpc::BroadcastRpcError,
            > {
                Ok(None)
            }
            async fn block_header_by_hash(
                &self,
                _h: [u8; 32],
            ) -> Result<
                Option<crate::hybrid_v2::execution::broadcast_rpc::BlockHeader>,
                crate::hybrid_v2::execution::broadcast_rpc::BroadcastRpcError,
            > {
                Ok(None)
            }
            async fn transaction_by_hash(
                &self,
                _h: [u8; 32],
            ) -> Result<
                Option<crate::hybrid_v2::execution::broadcast_rpc::TransactionSummary>,
                crate::hybrid_v2::execution::broadcast_rpc::BroadcastRpcError,
            > {
                Ok(None)
            }
            async fn receipt_by_hash(
                &self,
                _h: [u8; 32],
            ) -> Result<
                Option<crate::hybrid_v2::execution::broadcast_rpc::TxReceipt>,
                crate::hybrid_v2::execution::broadcast_rpc::BroadcastRpcError,
            > {
                Ok(None)
            }
            async fn transaction_count(
                &self,
                _a: [u8; 20],
                _t: crate::hybrid_v2::execution::rpc::BlockTag,
            ) -> Result<u64, crate::hybrid_v2::execution::broadcast_rpc::BroadcastRpcError>
            {
                Ok(0)
            }
            async fn send_raw_transaction(
                &self,
                _b: &[u8],
            ) -> Result<
                crate::hybrid_v2::execution::broadcast_rpc::SendOutcome,
                crate::hybrid_v2::execution::broadcast_rpc::BroadcastRpcError,
            > {
                Err(
                    crate::hybrid_v2::execution::broadcast_rpc::BroadcastRpcError::Unavailable(
                        "stub".into(),
                    ),
                )
            }
        }

        let store: Arc<dyn crate::hybrid_v2::HybridV2ProjectionStore> =
            Arc::new(InMemoryProjectionStore::new());
        let rpc: Arc<dyn crate::hybrid_v2::execution::broadcast_rpc::ExecutionBroadcastRpcClient> =
            Arc::new(StubRpc);
        let clock: Arc<dyn crate::hybrid_v2::execution::orchestrator::Clock> =
            Arc::new(SystemClock);
        let outbox = Arc::new(BroadcastOutbox {
            store: store.clone(),
            rpc: rpc.clone(),
            clock: clock.clone(),
            deployment_id: 42,
        });
        let worker = Arc::new(BroadcastConfirmationWorker {
            store,
            rpc: rpc.clone(),
            clock,
            deployment_id: 42,
            chain_id: 84532,
            confirmation_depth: 3,
            poll_interval_ms: 2_000,
            poll_timeout_ms: 300_000,
            max_batch_size: 32,
            max_pending_age_ms: 3_600_000,
        });
        let cfg = crate::hybrid_v2::config::HybridV2ExecutionConfig::disabled();
        let state = AppState::new(EngineState::new(Vec::new()))
            .with_hybrid_v2_broadcast(outbox, worker, rpc, cfg);
        assert!(state.hybrid_v2_broadcast_outbox.is_some());
        assert!(state.hybrid_v2_broadcast_worker.is_some());
        assert!(state.hybrid_v2_broadcast_rpc.is_some());
        assert!(state.hybrid_v2_broadcast_config.is_some());
        assert!(state.hybrid_v2_broadcast_unavailable_reason.is_none());
    }
}
