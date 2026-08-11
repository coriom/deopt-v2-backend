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

fn hex_encode20(bytes: &[u8; 20]) -> String {
    let mut s = String::with_capacity(40);
    for b in bytes.iter() {
        s.push_str(&format!("{:02x}", b));
    }
    s
}
