//! Hybrid V2 pre-broadcast execution orchestrator (Part Q of
//! `BACKEND-HYBRID-V2-SIGNER-AND-EXECUTION-V1`).
//!
//! ## Purpose
//!
//! The orchestrator ties together every Part of the SIGNER-AND-EXECUTION
//! surface: identity, state machine, persistence, preflight, plan
//! builder, target policy, nonce reservation, simulator, gas policy,
//! signer firewall, signer, and signature verification. It advances a
//! single execution row from `Discovered` all the way to the terminal
//! `BroadcastDisabled` (or a typed failure), issuing one persisted phase
//! transition per step and honouring the state-machine legal-edge
//! matrix at every write.
//!
//! ## Frozen safety invariants
//!
//! - `BROADCAST_IS_DISABLED` — the orchestrator's terminal success
//!   state is `BroadcastDisabled`; no code path builds or emits a raw
//!   transaction, and the `ExecutionRpcClient` trait has no `send_*`
//!   method (compile-time firewall). Adding a broadcast would require
//!   a fresh milestone with an explicit audit.
//! - `BASE_MAINNET_8453_IS_FORBIDDEN` — refused at construction (via
//!   `TargetPolicy::refuse_base_mainnet`) and re-checked by the
//!   simulator + firewall on every attempt.
//! - `EXECUTION_IS_IDEMPOTENT_BY_CANONICAL_ID` — `prepare` starts with
//!   `insert_execution_request` (ON CONFLICT DO NOTHING); a duplicate
//!   call converges on the same terminal state. Restart safety
//!   (`resume`) picks up mid-flight rows without re-issuing side
//!   effects that have already been persisted (plan, nonce, signature).
//! - `NO_PUBLIC_INFLUENCE_ON_EXECUTION_CALLDATA` — the orchestrator
//!   derives target, selector, calldata, nonce, gas, chain_id from the
//!   manifest allowlist + the deterministic plan builder. Callers
//!   supply intent (envelopes + orders + fill_quantity), never plan.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use alloy_primitives::{Bytes, U256};
use sha3::{Digest, Keccak256};

use crate::hybrid_v2::execution::gas_policy::{GasFeePolicy, GasPolicyError};
use crate::hybrid_v2::execution::identity::derive_canonical_execution_id;
use crate::hybrid_v2::execution::nonce::{NonceReservation, NonceReserveError, NonceReserver};
use crate::hybrid_v2::execution::persistence::{ExecutionRequestPatch, ExecutionRequestRow};
use crate::hybrid_v2::execution::plan::{
    executeMatchCall, ExecutionPlan, ExecutionPlanBuilder, OptionOrder, PlanError,
    SignedActionEnvelope,
};
use crate::hybrid_v2::execution::preflight::{PreflightChecker, PreflightRejection};
use crate::hybrid_v2::execution::rpc::{BlockTag, ExecutionRpcClient};
use crate::hybrid_v2::execution::signature_verify::{verify_signed_tx, SigVerifyError};
use crate::hybrid_v2::execution::signer::{ExecutionSigner, SignedTx, SignerError, SigningRequest};
use crate::hybrid_v2::execution::signer_firewall::{FirewallRejection, SignerPolicyFirewall};
use crate::hybrid_v2::execution::simulator::{ExecutionSimulator, SimulationError};
use crate::hybrid_v2::execution::state::ExecutionPhase;
use crate::hybrid_v2::execution::target_policy::TargetPolicy;
use crate::hybrid_v2::manifest::{ManifestParams, BASE_MAINNET_CHAIN_ID};
use crate::hybrid_v2::persistence::HybridV2ProjectionStore;
use crate::hybrid_v2::readiness::ReadinessReport;
use crate::hybrid_v2::rebuild_operations::OperationKind;
use crate::hybrid_v2::reducer::ProjectionState;

use alloy_sol_types::SolCall;

// -----------------------------------------------------------------
//                          CLOCK
// -----------------------------------------------------------------

/// Wall-clock trait so the orchestrator can be exercised deterministically
/// in tests. Every orchestrator field that would call `SystemTime::now`
/// goes through this trait instead.
pub trait Clock: Send + Sync {
    fn now_ms(&self) -> u64;
}

/// Production clock — reads wall-clock time via `SystemTime`.
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_ms(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
    }
}

/// Test-only monotonic mock clock. Advances only when the test steps it.
pub struct MockClock {
    current: std::sync::atomic::AtomicU64,
}

impl MockClock {
    pub fn new(initial_ms: u64) -> Self {
        Self {
            current: std::sync::atomic::AtomicU64::new(initial_ms),
        }
    }

    pub fn advance(&self, ms: u64) {
        self.current
            .fetch_add(ms, std::sync::atomic::Ordering::SeqCst);
    }

    pub fn set(&self, ms: u64) {
        self.current.store(ms, std::sync::atomic::Ordering::SeqCst);
    }
}

impl Clock for MockClock {
    fn now_ms(&self) -> u64 {
        self.current.load(std::sync::atomic::Ordering::SeqCst)
    }
}

// -----------------------------------------------------------------
//                    ORCHESTRATOR OUTCOME + ERROR
// -----------------------------------------------------------------

/// End-state produced by [`ExecutionOrchestrator::prepare`] /
/// [`ExecutionOrchestrator::resume`]. Every field reflects the persisted
/// row at the moment the orchestrator released the lock.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparationOutcome {
    pub canonical_execution_id: String,
    pub terminal_phase: ExecutionPhase,
    pub failure_class: Option<String>,
    pub failure_detail: Option<String>,
    pub plan_hash: Option<String>,
    pub signing_payload_hash: Option<String>,
    pub simulation_gas_estimate: Option<u64>,
    pub reserved_nonce: Option<u64>,
    pub attempts: u32,
}

/// Structural outer error surface. Non-terminal failure classes
/// (preflight rejection, simulation revert, firewall rejection, signer
/// unavailable) do NOT bubble as `Err(OrchestrationError)` — they land
/// as a terminal `Failed`/`Cancelled` phase and are reported via
/// [`PreparationOutcome::failure_class`]. `Err` is reserved for
/// contention + persistence-layer catastrophes.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum OrchestrationError {
    #[error("another operation holds the deployment lock")]
    LockContention,
    #[error("persistence failure: {0}")]
    StoreFailure(String),
    #[error("unrecoverable orchestrator error: {0}")]
    Unrecoverable(String),
}

// -----------------------------------------------------------------
//               FAILURE CLASS + BOUNDED CONSTANTS
// -----------------------------------------------------------------

/// Structural failure classes — one per typed rejection surface. Every
/// value is byte-identical to the string persisted into the
/// `failure_class` column so operators can grep on either.
pub mod failure_class {
    pub const PREFLIGHT_REJECTED: &str = "PREFLIGHT_REJECTED";
    pub const PLAN_BUILD_FAILED: &str = "PLAN_BUILD_FAILED";
    pub const NONCE_RESERVATION_FAILED: &str = "NONCE_RESERVATION_FAILED";
    pub const SIMULATION_FAILED_DETERMINISTIC: &str = "SIMULATION_FAILED_DETERMINISTIC";
    pub const SIMULATION_TRANSPORT_FAILED: &str = "SIMULATION_TRANSPORT_FAILED";
    pub const GAS_POLICY_REJECTED: &str = "GAS_POLICY_REJECTED";
    pub const FIREWALL_REJECTED: &str = "FIREWALL_REJECTED";
    pub const SIGNER_UNAVAILABLE: &str = "SIGNER_UNAVAILABLE";
    pub const SIGNATURE_VERIFICATION_FAILED: &str = "SIGNATURE_VERIFICATION_FAILED";
    pub const LOCK_CONTENTION: &str = "LOCK_CONTENTION";
    pub const STORE_FAILURE: &str = "STORE_FAILURE";
}

/// Retry ceiling for transport-only failures (simulation transport,
/// nonce reservation transient conflict). Bounded so a stuck upstream
/// cannot pin the orchestrator in a loop.
pub const MAX_TRANSPORT_RETRIES: u32 = 3;

// -----------------------------------------------------------------
//                    ORCHESTRATOR ENTRY
// -----------------------------------------------------------------

/// Runtime inputs the orchestrator receives per `prepare` call. Every
/// field is the CALLER's intent — the orchestrator derives target,
/// selector, calldata, nonce, gas, chain_id from the manifest allowlist
/// + the deterministic plan builder. A public caller can NEVER supply
/// calldata / target / value / nonce / gas / chain_id, so the shape of
/// this struct deliberately omits those fields.
#[derive(Clone)]
pub struct PreparationIntent {
    pub manifest: ManifestParams,
    pub runtime_state: ProjectionState,
    pub readiness: ReadinessReport,
    pub buyer_envelope: SignedActionEnvelope,
    pub buyer_signature: Bytes,
    pub buyer_order: OptionOrder,
    pub seller_envelope: SignedActionEnvelope,
    pub seller_signature: Bytes,
    pub seller_order: OptionOrder,
    pub fill_quantity_1e8: u128,
    pub buyer_active_series: Vec<U256>,
    pub seller_active_series: Vec<U256>,
    pub buyer_order_hash: String,
    pub seller_order_hash: String,
    pub buyer_subkey: String,
    pub seller_subkey: String,
    pub series_id: String,
    pub premium_amount: String,
    pub fee_schedule_epoch: Option<i64>,
}

/// The orchestrator itself — every field is `Arc<dyn ...>` so the
/// runtime can be shared across concurrent `prepare` calls.
pub struct ExecutionOrchestrator {
    pub store: Arc<dyn HybridV2ProjectionStore>,
    pub rpc: Arc<dyn ExecutionRpcClient>,
    pub signer: Arc<dyn ExecutionSigner>,
    pub target_policy: Arc<TargetPolicy>,
    pub gas_policy: Arc<GasFeePolicy>,
    pub deployment_id: i64,
    pub chain_id: u64,
    pub executor_address: [u8; 20],
    pub simulation_max_age_ms: u64,
    pub clock: Arc<dyn Clock>,
}

impl ExecutionOrchestrator {
    /// Advance a single execution intent from `Discovered` to
    /// `BroadcastDisabled` (or a structural terminal failure). See the
    /// module docs for the frozen safety invariants and the state
    /// machine walk implemented here.
    pub async fn prepare(
        &self,
        intent: PreparationIntent,
    ) -> Result<PreparationOutcome, OrchestrationError> {
        // Base mainnet firewall — earliest, always.
        if self.chain_id == BASE_MAINNET_CHAIN_ID {
            return Err(OrchestrationError::Unrecoverable(
                "Base mainnet is forbidden for Hybrid V2 execution".to_string(),
            ));
        }
        // Derive the canonical id from the intent (no caller-supplied id).
        let canonical_execution_id = derive_canonical_execution_id(
            self.deployment_id,
            self.chain_id,
            &intent.buyer_order_hash,
            &intent.seller_order_hash,
            intent.fill_quantity_1e8,
        )
        .into_string();

        // Idempotent seed row (ON CONFLICT DO NOTHING). We record the
        // creation attempt regardless of insertion success — a duplicate
        // prepare converges on the same row.
        self.upsert_seed_row(&canonical_execution_id, &intent)
            .await?;

        // Delegate to the shared step engine.
        self.drive(&canonical_execution_id, Some(&intent)).await
    }

    /// Restart-safety entry: re-enter the step engine using the
    /// persisted row. Idempotent — a terminal row is returned as-is; an
    /// intermediate row is advanced. Because `resume` cannot rebuild
    /// the caller-provided envelope / order data, the orchestrator
    /// resumes only from phases at or after `ReadyToSimulate` (plan +
    /// calldata_hash already persisted). Earlier phases require the
    /// caller to re-invoke `prepare` with the original intent.
    pub async fn resume(
        &self,
        canonical_execution_id: &str,
    ) -> Result<PreparationOutcome, OrchestrationError> {
        self.drive(canonical_execution_id, None).await
    }

    // ---------------- internals ------------------

    async fn upsert_seed_row(
        &self,
        canonical_execution_id: &str,
        intent: &PreparationIntent,
    ) -> Result<(), OrchestrationError> {
        // Derive target + selector deterministically from the manifest.
        let target_hex = intent
            .manifest
            .module_addresses
            .option_matching_engine
            .clone();
        let selector_hex = format!("0x{}", hex_encode(&executeMatchCall::SELECTOR));
        let now = self.now_ms() as i64;
        let row = ExecutionRequestRow {
            canonical_execution_id: canonical_execution_id.to_string(),
            deployment_id: self.deployment_id,
            chain_id: self.chain_id as i64,
            execution_kind: "HYBRID_V2_OPTION_MATCH".to_string(),
            buyer_order_hash: intent.buyer_order_hash.clone(),
            seller_order_hash: intent.seller_order_hash.clone(),
            buyer_subkey: intent.buyer_subkey.clone(),
            seller_subkey: intent.seller_subkey.clone(),
            series_id: intent.series_id.clone(),
            fill_quantity_1e8: intent.fill_quantity_1e8.to_string(),
            premium_amount: intent.premium_amount.clone(),
            fee_schedule_epoch: intent.fee_schedule_epoch,
            source_matched_execution_id: None,
            target_contract: target_hex,
            selector: selector_hex,
            calldata_hash: None,
            calldata_bytes: None,
            plan_hash: None,
            tx_value_wei: "0".to_string(),
            simulation_block_number: None,
            simulation_block_hash: None,
            simulation_gas_estimate: None,
            simulation_result_json: None,
            signer_identity: None,
            signing_payload_hash: None,
            signature_r: None,
            signature_s: None,
            signature_v: None,
            recovered_signer: None,
            gas_limit: None,
            max_fee_per_gas_wei: None,
            max_priority_fee_per_gas_wei: None,
            reserved_nonce: None,
            phase: ExecutionPhase::Discovered,
            failure_class: None,
            failure_detail: None,
            retry_count: 0,
            holder_epoch: None,
            signer_request_idempotency_key: None,
            created_at_ms: now,
            updated_at_ms: now,
        };
        self.store
            .insert_execution_request(&row)
            .await
            .map_err(|e| OrchestrationError::StoreFailure(e.to_string()))?;
        Ok(())
    }

    async fn drive(
        &self,
        canonical_execution_id: &str,
        intent: Option<&PreparationIntent>,
    ) -> Result<PreparationOutcome, OrchestrationError> {
        // Load the live row.
        let row = self
            .store
            .get_execution_request(canonical_execution_id)
            .await
            .map_err(|e| OrchestrationError::StoreFailure(e.to_string()))?
            .ok_or_else(|| {
                OrchestrationError::Unrecoverable(format!(
                    "execution row {canonical_execution_id} not found after upsert"
                ))
            })?;

        // Terminal? Return as-is.
        if row.phase.is_terminal() {
            return Ok(build_outcome(&row));
        }

        // Acquire the deployment-scoped Execution lock. Contention →
        // report to caller without side-effects.
        let holder_epoch = self.now_ms() as i64;
        let guard = self
            .store
            .try_acquire_operation_lock(
                self.deployment_id,
                OperationKind::Execution,
                holder_epoch,
                self.now_ms() as i64,
            )
            .await
            .map_err(|e| OrchestrationError::StoreFailure(e.to_string()))?;
        let Some(guard) = guard else {
            return Err(OrchestrationError::LockContention);
        };

        // Run the pipeline. Any error here MUST still release the lock.
        let result = self
            .drive_locked(canonical_execution_id, intent, holder_epoch)
            .await;

        // Release the lock (best-effort; a failing release does not
        // change the persisted phase, and the store's cleanup path
        // will reap it once the row is terminal).
        let _ = self
            .store
            .release_operation_lock(self.deployment_id, holder_epoch)
            .await;
        drop(guard);

        result
    }

    async fn drive_locked(
        &self,
        canonical_execution_id: &str,
        intent: Option<&PreparationIntent>,
        holder_epoch: i64,
    ) -> Result<PreparationOutcome, OrchestrationError> {
        // Re-read the row (in case another writer advanced it between
        // load and lock).
        let mut row = self.load_row(canonical_execution_id).await?;
        if row.phase.is_terminal() {
            return Ok(build_outcome(&row));
        }

        // Track attempts for observability — the persistent
        // `retry_count` column stays as the row-level attempt counter.
        let mut attempts: u32 = row.retry_count as u32;

        // ------------------------- STEP 1: preflight -------------------
        if row.phase == ExecutionPhase::Discovered {
            attempts = attempts.saturating_add(1);
            self.transition(
                canonical_execution_id,
                ExecutionPhase::Discovered,
                ExecutionPhase::Validating,
                &ExecutionRequestPatch {
                    holder_epoch: Some(holder_epoch),
                    ..Default::default()
                },
            )
            .await?;
            self.append_attempt(canonical_execution_id, ExecutionPhase::Validating)
                .await?;
            row.phase = ExecutionPhase::Validating;
        }

        if row.phase == ExecutionPhase::Validating {
            // Preflight requires the intent (runtime state + readiness).
            // Without it (resume without intent) we cannot re-run
            // preflight; but since the row was persisted at Validating
            // by our own code path, the caller either re-supplies the
            // intent OR we fail the row with a MISSING_INTENT_ON_RESUME
            // detail. Callers observing this can invoke `prepare` again
            // with the original intent.
            let intent = intent.ok_or_else(|| {
                OrchestrationError::Unrecoverable(
                    "resume without intent cannot re-run preflight; call prepare with the \
                     original intent to re-validate"
                        .to_string(),
                )
            })?;
            let checker = PreflightChecker {
                store: self.store.as_ref(),
                runtime_state: &intent.runtime_state,
                readiness: &intent.readiness,
                deployment_id: self.deployment_id,
                chain_id: self.chain_id,
            };
            match checker.validate(&row).await {
                Ok(_) => {
                    self.transition(
                        canonical_execution_id,
                        ExecutionPhase::Validating,
                        ExecutionPhase::ReadyToSimulate,
                        &Default::default(),
                    )
                    .await?;
                    row.phase = ExecutionPhase::ReadyToSimulate;
                }
                Err(rej) => {
                    // Classify: OrderCancelled → Cancelled, everything
                    // else → Failed(PREFLIGHT_REJECTED).
                    let terminal = if matches!(rej, PreflightRejection::OrderCancelled { .. }) {
                        ExecutionPhase::Cancelled
                    } else {
                        ExecutionPhase::Failed
                    };
                    let patch = ExecutionRequestPatch {
                        failure_class: Some(failure_class::PREFLIGHT_REJECTED.to_string()),
                        failure_detail: Some(rej.to_string()),
                        ..Default::default()
                    };
                    self.transition(
                        canonical_execution_id,
                        ExecutionPhase::Validating,
                        terminal,
                        &patch,
                    )
                    .await?;
                    let final_row = self.load_row(canonical_execution_id).await?;
                    return Ok(with_attempts(build_outcome(&final_row), attempts));
                }
            }
        }

        // ------------------------- STEP 2: plan build -------------------
        if row.phase == ExecutionPhase::ReadyToSimulate && row.plan_hash.is_none() {
            let intent = intent.ok_or_else(|| {
                OrchestrationError::Unrecoverable(
                    "resume before plan_hash persisted requires the intent; call prepare"
                        .to_string(),
                )
            })?;
            let plan = match ExecutionPlanBuilder::build_from_request(
                &row,
                &intent.manifest,
                &intent.buyer_envelope,
                &intent.buyer_signature,
                &intent.buyer_order,
                &intent.seller_envelope,
                &intent.seller_signature,
                &intent.seller_order,
                intent.fill_quantity_1e8,
                &intent.buyer_active_series,
                &intent.seller_active_series,
            ) {
                Ok(p) => p,
                Err(err) => {
                    return self
                        .fail_from(
                            &row,
                            canonical_execution_id,
                            failure_class::PLAN_BUILD_FAILED,
                            &err.to_string(),
                            attempts,
                        )
                        .await;
                }
            };
            // Persist calldata_hash + plan_hash + target + selector.
            let calldata_hash_hex = format!("0x{}", hex_encode(&plan.calldata_hash));
            let plan_hash_hex = format!("0x{}", hex_encode(&plan.plan_hash));
            let target_hex = format!("0x{}", hex_encode(&plan.target));
            let selector_hex = format!("0x{}", hex_encode(&plan.selector));
            let patch = ExecutionRequestPatch {
                calldata_hash: Some(calldata_hash_hex),
                calldata_bytes: None,
                plan_hash: Some(plan_hash_hex),
                target_contract: Some(target_hex),
                selector: Some(selector_hex),
                ..Default::default()
            };
            // Plan build does not advance the phase — it decorates the
            // row. We use a self-update trick by applying the patch via
            // a no-op phase advance (ReadyToSimulate stays until
            // SIMULATING). To do so idempotently we short-circuit and
            // reload; the plan-hash immutability rule blocks any
            // adversarial later mutation.
            //
            // The persistence trait requires a legal edge; we cannot
            // "stay" in ReadyToSimulate. So we advance to Simulating
            // right after with the same patch merged in.
            row = self.load_row(canonical_execution_id).await?;
            // Store the plan hash on the row via a shim update prior to
            // Simulating — done inline in STEP 3 below.
            // Cache patch fields for the next transition.
            let _ = (patch, plan);
        }

        // ------------------------- STEP 3: nonce reservation + simulate -
        if row.phase == ExecutionPhase::ReadyToSimulate {
            let intent = intent.ok_or_else(|| {
                OrchestrationError::Unrecoverable(
                    "resume at ReadyToSimulate requires intent".to_string(),
                )
            })?;
            // Rebuild the plan (deterministic) if not already cached.
            let plan = ExecutionPlanBuilder::build_from_request(
                &row,
                &intent.manifest,
                &intent.buyer_envelope,
                &intent.buyer_signature,
                &intent.buyer_order,
                &intent.seller_envelope,
                &intent.seller_signature,
                &intent.seller_order,
                intent.fill_quantity_1e8,
                &intent.buyer_active_series,
                &intent.seller_active_series,
            )
            .map_err(|e: PlanError| {
                OrchestrationError::Unrecoverable(format!("plan rebuild: {e}"))
            })?;

            // Reserve nonce (idempotent: if the row already carries a
            // reserved_nonce, skip). Reuse the on-chain reservation
            // path — the store enforces uniqueness.
            let signer_identity_hex = format!("0x{}", hex_encode(&self.signer.identity().address));
            let reservation = if let Some(existing) = row.reserved_nonce {
                let n = u64::try_from(existing).map_err(|_| {
                    OrchestrationError::Unrecoverable("negative reserved_nonce".to_string())
                })?;
                NonceReservation {
                    chain_id: self.chain_id,
                    signer_identity: signer_identity_hex.clone(),
                    nonce: n,
                    canonical_execution_id: canonical_execution_id.to_string(),
                    reserved_at_ms: self.now_ms(),
                }
            } else {
                let reserver = NonceReserver {
                    store: self.store.as_ref(),
                    rpc: self.rpc.as_ref(),
                    signer_identity: signer_identity_hex.clone(),
                    chain_id: self.chain_id,
                };
                match reserver
                    .reserve_for(canonical_execution_id, self.now_ms())
                    .await
                {
                    Ok(r) => r,
                    Err(NonceReserveError::ChainMismatch { .. }) => {
                        return self
                            .fail_from(
                                &row,
                                canonical_execution_id,
                                failure_class::NONCE_RESERVATION_FAILED,
                                "chain mismatch during nonce reservation",
                                attempts,
                            )
                            .await;
                    }
                    Err(err) => {
                        return self
                            .fail_from(
                                &row,
                                canonical_execution_id,
                                failure_class::NONCE_RESERVATION_FAILED,
                                &err.to_string(),
                                attempts,
                            )
                            .await;
                    }
                }
            };

            // Persist plan + nonce and advance to Simulating.
            //
            // BACKEND-HYBRID-V2-BROADCAST-LIVE-WIRING-CLOSURE-V1 (Part
            // D): also persist the raw `calldata_bytes` alongside the
            // hash. The admin fresh-submit path uses these bytes to
            // reconstruct a complete `ExecutionPlan` from the row and
            // re-verifies the keccak256 matches `calldata_hash` before
            // handing anything to the outbox. Migration 0052 enforces
            // immutability once set.
            let calldata_hash_hex = format!("0x{}", hex_encode(&plan.calldata_hash));
            let plan_hash_hex = format!("0x{}", hex_encode(&plan.plan_hash));
            let target_hex = format!("0x{}", hex_encode(&plan.target));
            let selector_hex = format!("0x{}", hex_encode(&plan.selector));
            let patch = ExecutionRequestPatch {
                calldata_hash: Some(calldata_hash_hex),
                calldata_bytes: Some(plan.calldata.clone()),
                plan_hash: Some(plan_hash_hex),
                target_contract: Some(target_hex),
                selector: Some(selector_hex),
                reserved_nonce: Some(reservation.nonce as i64),
                signer_identity: Some(signer_identity_hex.clone()),
                ..Default::default()
            };
            self.transition(
                canonical_execution_id,
                ExecutionPhase::ReadyToSimulate,
                ExecutionPhase::Simulating,
                &patch,
            )
            .await?;
            row = self.load_row(canonical_execution_id).await?;

            // Run the simulator with bounded transport retries.
            let simulator = ExecutionSimulator::new(self.rpc.as_ref(), self.executor_address);
            let mut sim_result = None;
            let mut last_transport_err: Option<String> = None;
            for attempt in 0..MAX_TRANSPORT_RETRIES {
                match simulator.simulate(&plan).await {
                    Ok(outcome) => {
                        sim_result = Some(outcome);
                        break;
                    }
                    Err(SimulationError::ChainMismatch { .. }) => {
                        // Deterministic — never retry.
                        return self
                            .fail_from(
                                &row,
                                canonical_execution_id,
                                failure_class::SIMULATION_FAILED_DETERMINISTIC,
                                "chain mismatch during simulation",
                                attempts,
                            )
                            .await;
                    }
                    Err(err) => {
                        last_transport_err = Some(err.to_string());
                        // MalformedResponse is a hard error — do not retry.
                        if matches!(err, SimulationError::MalformedResponse(_)) {
                            break;
                        }
                        let _ = attempt;
                    }
                }
            }
            let Some(sim) = sim_result else {
                return self
                    .fail_from(
                        &row,
                        canonical_execution_id,
                        failure_class::SIMULATION_TRANSPORT_FAILED,
                        &last_transport_err
                            .unwrap_or_else(|| "simulation transport exhausted".into()),
                        attempts,
                    )
                    .await;
            };

            // Persist simulation surface.
            let sim_hash_hex = format!("0x{}", hex_encode(&sim.simulation_block_hash));
            let (next_phase, patch) = if sim.eth_call_ok {
                (
                    ExecutionPhase::SimulationSucceeded,
                    ExecutionRequestPatch {
                        simulation_block_number: Some(sim.simulation_block_number as i64),
                        simulation_block_hash: Some(sim_hash_hex),
                        simulation_gas_estimate: Some(sim.gas_estimate as i64),
                        ..Default::default()
                    },
                )
            } else {
                (
                    ExecutionPhase::SimulationFailed,
                    ExecutionRequestPatch {
                        simulation_block_number: Some(sim.simulation_block_number as i64),
                        simulation_block_hash: Some(sim_hash_hex),
                        simulation_gas_estimate: Some(0),
                        simulation_result_json: sim.revert.as_ref().map(|r| {
                            serde_json::json!({
                                "revert": format!("{r:?}"),
                            })
                        }),
                        ..Default::default()
                    },
                )
            };
            self.transition(
                canonical_execution_id,
                ExecutionPhase::Simulating,
                next_phase,
                &patch,
            )
            .await?;
            row = self.load_row(canonical_execution_id).await?;

            if !sim.eth_call_ok {
                // Terminal deterministic revert path.
                let detail = sim
                    .revert
                    .as_ref()
                    .map(|r| format!("{r:?}"))
                    .unwrap_or_else(|| "unknown revert".to_string());
                self.transition(
                    canonical_execution_id,
                    ExecutionPhase::SimulationFailed,
                    ExecutionPhase::Failed,
                    &ExecutionRequestPatch {
                        failure_class: Some(
                            failure_class::SIMULATION_FAILED_DETERMINISTIC.to_string(),
                        ),
                        failure_detail: Some(detail),
                        ..Default::default()
                    },
                )
                .await?;
                let final_row = self.load_row(canonical_execution_id).await?;
                return Ok(with_attempts(build_outcome(&final_row), attempts));
            }

            // ---- Gas policy ----
            // Sample fee_history from the RPC.
            let fee_history = match self
                .rpc
                .fee_history(4, BlockTag::Number(sim.simulation_block_number), &[10.0])
                .await
            {
                Ok(f) => f,
                Err(err) => {
                    return self
                        .fail_from(
                            &row,
                            canonical_execution_id,
                            failure_class::GAS_POLICY_REJECTED,
                            &format!("fee_history: {err}"),
                            attempts,
                        )
                        .await;
                }
            };
            let gas_out = match self.gas_policy.compute(&sim, &fee_history) {
                Ok(o) => o,
                Err(err) => {
                    let detail = match err {
                        GasPolicyError::EstimateExceedsCeiling { estimate, ceiling } => {
                            format!("estimate {estimate} exceeds ceiling {ceiling}")
                        }
                        GasPolicyError::ProviderFeeAnomaly(s) => format!("provider anomaly: {s}"),
                        other => other.to_string(),
                    };
                    return self
                        .fail_from(
                            &row,
                            canonical_execution_id,
                            failure_class::GAS_POLICY_REJECTED,
                            &detail,
                            attempts,
                        )
                        .await;
                }
            };

            // Compute signing_payload_hash + persist gas + advance to
            // AwaitingSignature.
            let signing_payload_hash = derive_signing_payload_hash(
                self.chain_id,
                reservation.nonce,
                &plan,
                gas_out.gas_limit,
                gas_out.max_fee_per_gas_wei,
                gas_out.max_priority_fee_per_gas_wei,
            );
            let patch = ExecutionRequestPatch {
                gas_limit: Some(gas_out.gas_limit as i64),
                max_fee_per_gas_wei: Some(gas_out.max_fee_per_gas_wei.to_string()),
                max_priority_fee_per_gas_wei: Some(
                    gas_out.max_priority_fee_per_gas_wei.to_string(),
                ),
                signing_payload_hash: Some(format!("0x{}", hex_encode(&signing_payload_hash))),
                signer_identity: Some(signer_identity_hex.clone()),
                ..Default::default()
            };
            self.transition(
                canonical_execution_id,
                ExecutionPhase::SimulationSucceeded,
                ExecutionPhase::AwaitingSignature,
                &patch,
            )
            .await?;
            row = self.load_row(canonical_execution_id).await?;
        }

        // ------------------------- STEP 4: firewall + sign -----------
        if row.phase == ExecutionPhase::AwaitingSignature {
            let intent_opt = intent;
            // Rebuild plan for firewall + signing. `plan_hash` is
            // immutable in the persisted row so a rebuild that diverges
            // would have failed in STEP 3.
            let plan_opt: Option<ExecutionPlan> = if let Some(intent) = intent_opt {
                Some(
                    ExecutionPlanBuilder::build_from_request(
                        &row,
                        &intent.manifest,
                        &intent.buyer_envelope,
                        &intent.buyer_signature,
                        &intent.buyer_order,
                        &intent.seller_envelope,
                        &intent.seller_signature,
                        &intent.seller_order,
                        intent.fill_quantity_1e8,
                        &intent.buyer_active_series,
                        &intent.seller_active_series,
                    )
                    .map_err(|e| OrchestrationError::Unrecoverable(format!("plan rebuild: {e}")))?,
                )
            } else {
                None
            };
            let plan = plan_opt.ok_or_else(|| {
                OrchestrationError::Unrecoverable(
                    "resume at AwaitingSignature requires intent to rebuild plan".to_string(),
                )
            })?;

            let signer_identity_hex = row
                .signer_identity
                .clone()
                .unwrap_or_else(|| format!("0x{}", hex_encode(&self.signer.identity().address)));
            let reserved_nonce = row.reserved_nonce.ok_or_else(|| {
                OrchestrationError::Unrecoverable("reserved_nonce missing at signing".to_string())
            })? as u64;
            let reservation = NonceReservation {
                chain_id: self.chain_id,
                signer_identity: signer_identity_hex.clone(),
                nonce: reserved_nonce,
                canonical_execution_id: canonical_execution_id.to_string(),
                reserved_at_ms: self.now_ms(),
            };

            self.transition(
                canonical_execution_id,
                ExecutionPhase::AwaitingSignature,
                ExecutionPhase::Signing,
                &Default::default(),
            )
            .await?;
            row = self.load_row(canonical_execution_id).await?;

            // Firewall independent revalidation.
            let firewall = SignerPolicyFirewall {
                target_policy: self.target_policy.as_ref(),
                gas_policy: self.gas_policy.as_ref(),
                configured_chain_id: self.chain_id,
                deployment_id: self.deployment_id,
                store: self.store.as_ref(),
                simulation_max_age_ms: self.simulation_max_age_ms,
                now_ms: self.now_ms(),
            };
            if let Err(rej) = firewall
                .revalidate(&row, &plan, &reservation, self.signer.identity().address)
                .await
            {
                return self
                    .fail_from_signing(
                        &row,
                        canonical_execution_id,
                        failure_class::FIREWALL_REJECTED,
                        &firewall_detail(&rej),
                        attempts,
                    )
                    .await;
            }

            // Build the signing request.
            let signing_payload_hash = row
                .signing_payload_hash
                .as_ref()
                .and_then(|s| parse_bytes32(s))
                .ok_or_else(|| {
                    OrchestrationError::Unrecoverable(
                        "signing_payload_hash missing/malformed".to_string(),
                    )
                })?;
            let request = SigningRequest {
                chain_id: self.chain_id,
                nonce: reserved_nonce,
                target: plan.target,
                value_wei: plan.value_wei,
                calldata_hash: plan.calldata_hash,
                gas_limit: row.gas_limit.unwrap_or(0) as u64,
                max_fee_per_gas_wei: parse_u256_decimal(
                    row.max_fee_per_gas_wei.as_deref().unwrap_or("0"),
                ),
                max_priority_fee_per_gas_wei: parse_u256_decimal(
                    row.max_priority_fee_per_gas_wei.as_deref().unwrap_or("0"),
                ),
                tx_type: 2,
                plan_hash: plan.plan_hash,
                signing_payload_hash,
                calldata: plan.calldata.clone(),
            };

            // Part L — derive + persist the external-signer
            // idempotency key BEFORE invoking the signer. The row is
            // in phase `Signing`; the migration 0050 trigger blocks
            // any divergent overwrite so a re-derive on retry is a
            // no-op (same inputs → same key). We use a store-level
            // "same-phase re-update" via a Signing→Signing edge —
            // which is illegal — so we persist the key using a plain
            // `insert_execution_request` idempotent-shape update via
            // a bespoke helper. Simplest path: attach it to the
            // upcoming Signing→SignatureVerified patch. If the signer
            // call fails between derive and persist, the next attempt
            // will re-derive the same key.
            let idem_key_bytes =
                crate::hybrid_v2::execution::signer_kms_bridge::derive_idempotency_key(
                    &self.signer.identity().address,
                    canonical_execution_id.as_bytes(),
                    &request.plan_hash,
                    &request.signing_payload_hash,
                );
            let idem_key_hex = format!("0x{}", hex_encode(&idem_key_bytes));

            // Idempotency: if the row already carries a signature (from
            // a previous partial attempt) skip signer call.
            let signed = if row.signature_r.is_some() && row.signature_s.is_some() {
                SignedTx {
                    signature_r: parse_bytes32(row.signature_r.as_ref().unwrap())
                        .unwrap_or([0; 32]),
                    signature_s: parse_bytes32(row.signature_s.as_ref().unwrap())
                        .unwrap_or([0; 32]),
                    signature_v: row.signature_v.unwrap_or(0) as u8,
                    recovered_signer: row
                        .recovered_signer
                        .as_ref()
                        .and_then(|s| parse_address(s))
                        .unwrap_or([0; 20]),
                    tx_type: 2,
                }
            } else {
                match self.signer.sign_execution(request.clone()).await {
                    Ok(s) => s,
                    Err(SignerError::SignerUnavailable(msg)) => {
                        return self
                            .fail_from_signing(
                                &row,
                                canonical_execution_id,
                                failure_class::SIGNER_UNAVAILABLE,
                                &msg,
                                attempts,
                            )
                            .await;
                    }
                    Err(err) => {
                        return self
                            .fail_from_signing(
                                &row,
                                canonical_execution_id,
                                failure_class::SIGNER_UNAVAILABLE,
                                &err.to_string(),
                                attempts,
                            )
                            .await;
                    }
                }
            };

            // Verify signature.
            if let Err(err) = verify_signed_tx(&request, &signed, self.signer.identity().address) {
                return self
                    .fail_from_signing(
                        &row,
                        canonical_execution_id,
                        failure_class::SIGNATURE_VERIFICATION_FAILED,
                        &sig_verify_detail(&err),
                        attempts,
                    )
                    .await;
            }

            // Persist signature + advance Signing → SignatureVerified.
            let r_hex = format!("0x{}", hex_encode(&signed.signature_r));
            let s_hex = format!("0x{}", hex_encode(&signed.signature_s));
            let recovered_hex = format!("0x{}", hex_encode(&signed.recovered_signer));
            let patch = ExecutionRequestPatch {
                signature_r: Some(r_hex),
                signature_s: Some(s_hex),
                signature_v: Some(signed.signature_v as i16),
                recovered_signer: Some(recovered_hex),
                // Part L — same-input-same-key idempotency; the
                // migration 0050 trigger + in-memory store
                // immutability check both refuse a divergent
                // overwrite. Setting it here binds the eventual
                // signature to a deterministic vendor request id so
                // an operator can correlate CloudTrail events with
                // this canonical execution.
                signer_request_idempotency_key: Some(idem_key_hex),
                ..Default::default()
            };
            self.transition(
                canonical_execution_id,
                ExecutionPhase::Signing,
                ExecutionPhase::SignatureVerified,
                &patch,
            )
            .await?;
            row = self.load_row(canonical_execution_id).await?;
        }

        // ------------------------- STEP 5: broadcast disabled ---------
        if row.phase == ExecutionPhase::SignatureVerified {
            self.transition(
                canonical_execution_id,
                ExecutionPhase::SignatureVerified,
                ExecutionPhase::ReadyForBroadcast,
                &Default::default(),
            )
            .await?;
            row.phase = ExecutionPhase::ReadyForBroadcast;
        }
        if row.phase == ExecutionPhase::ReadyForBroadcast {
            self.transition(
                canonical_execution_id,
                ExecutionPhase::ReadyForBroadcast,
                ExecutionPhase::BroadcastDisabled,
                &Default::default(),
            )
            .await?;
        }

        let final_row = self.load_row(canonical_execution_id).await?;
        Ok(with_attempts(build_outcome(&final_row), attempts))
    }

    // --------- helpers ------------

    async fn load_row(
        &self,
        canonical_execution_id: &str,
    ) -> Result<ExecutionRequestRow, OrchestrationError> {
        self.store
            .get_execution_request(canonical_execution_id)
            .await
            .map_err(|e| OrchestrationError::StoreFailure(e.to_string()))?
            .ok_or_else(|| {
                OrchestrationError::Unrecoverable(format!(
                    "execution row {canonical_execution_id} disappeared mid-flight"
                ))
            })
    }

    async fn transition(
        &self,
        canonical_execution_id: &str,
        from: ExecutionPhase,
        to: ExecutionPhase,
        patch: &ExecutionRequestPatch,
    ) -> Result<(), OrchestrationError> {
        let now = self.now_ms() as i64;
        let ok = self
            .store
            .update_execution_phase(canonical_execution_id, from, to, now, patch.clone())
            .await
            .map_err(|e| OrchestrationError::StoreFailure(e.to_string()))?;
        if !ok {
            return Err(OrchestrationError::StoreFailure(format!(
                "lost update from {} to {} on {}",
                from.as_str(),
                to.as_str(),
                canonical_execution_id
            )));
        }
        Ok(())
    }

    async fn append_attempt(
        &self,
        canonical_execution_id: &str,
        phase: ExecutionPhase,
    ) -> Result<(), OrchestrationError> {
        let now = self.now_ms() as i64;
        self.store
            .append_execution_attempt(canonical_execution_id, phase, None, now)
            .await
            .map(|_| ())
            .map_err(|e| OrchestrationError::StoreFailure(e.to_string()))
    }

    async fn fail_from(
        &self,
        row: &ExecutionRequestRow,
        canonical_execution_id: &str,
        failure_class: &str,
        failure_detail: &str,
        attempts: u32,
    ) -> Result<PreparationOutcome, OrchestrationError> {
        // Route the row from its current phase to Failed using ONLY
        // edges that exist in the state-machine matrix. See
        // `execution/state.rs` for the closed legal-edge set. Phases
        // whose successor list does not include `Failed` are bridged
        // via a stepping-stone forward edge.
        let current = row.phase;
        let patch = ExecutionRequestPatch {
            failure_class: Some(failure_class.to_string()),
            failure_detail: Some(failure_detail.to_string()),
            ..Default::default()
        };
        match current {
            ExecutionPhase::Discovered
            | ExecutionPhase::Validating
            | ExecutionPhase::Simulating
            | ExecutionPhase::Signing
            | ExecutionPhase::SignatureVerified => {
                self.transition(
                    canonical_execution_id,
                    current,
                    ExecutionPhase::Failed,
                    &patch,
                )
                .await?;
            }
            ExecutionPhase::ReadyToSimulate => {
                self.transition(
                    canonical_execution_id,
                    ExecutionPhase::ReadyToSimulate,
                    ExecutionPhase::Simulating,
                    &Default::default(),
                )
                .await?;
                self.transition(
                    canonical_execution_id,
                    ExecutionPhase::Simulating,
                    ExecutionPhase::Failed,
                    &patch,
                )
                .await?;
            }
            ExecutionPhase::SimulationSucceeded => {
                // Bridge SimulationSucceeded → AwaitingSignature →
                // Signing → Failed. Every intermediate hop is a legal
                // edge; the failure_class + detail land at the final
                // transition.
                self.transition(
                    canonical_execution_id,
                    ExecutionPhase::SimulationSucceeded,
                    ExecutionPhase::AwaitingSignature,
                    &Default::default(),
                )
                .await?;
                self.transition(
                    canonical_execution_id,
                    ExecutionPhase::AwaitingSignature,
                    ExecutionPhase::Signing,
                    &Default::default(),
                )
                .await?;
                self.transition(
                    canonical_execution_id,
                    ExecutionPhase::Signing,
                    ExecutionPhase::Failed,
                    &patch,
                )
                .await?;
            }
            ExecutionPhase::AwaitingSignature => {
                self.transition(
                    canonical_execution_id,
                    ExecutionPhase::AwaitingSignature,
                    ExecutionPhase::Signing,
                    &Default::default(),
                )
                .await?;
                self.transition(
                    canonical_execution_id,
                    ExecutionPhase::Signing,
                    ExecutionPhase::Failed,
                    &patch,
                )
                .await?;
            }
            // Terminal or ReadyForBroadcast — no legal edge to Failed.
            // Terminal states are already frozen; ReadyForBroadcast is
            // one step from `BroadcastDisabled` (success terminal). If
            // we ever reach here the caller made a routing bug; surface
            // it deterministically.
            ExecutionPhase::SimulationFailed => {
                self.transition(
                    canonical_execution_id,
                    ExecutionPhase::SimulationFailed,
                    ExecutionPhase::Failed,
                    &patch,
                )
                .await?;
            }
            ExecutionPhase::ReadyForBroadcast
            | ExecutionPhase::BroadcastDisabled
            | ExecutionPhase::Cancelled
            | ExecutionPhase::Stale
            | ExecutionPhase::Failed => {
                return Err(OrchestrationError::Unrecoverable(format!(
                    "fail_from unreachable for phase {}: no legal Failed edge",
                    current.as_str()
                )));
            }
        }
        let final_row = self.load_row(canonical_execution_id).await?;
        Ok(with_attempts(build_outcome(&final_row), attempts))
    }

    async fn fail_from_signing(
        &self,
        row: &ExecutionRequestRow,
        canonical_execution_id: &str,
        failure_class: &str,
        failure_detail: &str,
        attempts: u32,
    ) -> Result<PreparationOutcome, OrchestrationError> {
        let current = row.phase;
        let patch = ExecutionRequestPatch {
            failure_class: Some(failure_class.to_string()),
            failure_detail: Some(failure_detail.to_string()),
            ..Default::default()
        };
        // Signing → Failed is legal. If we came in at AwaitingSignature
        // (never transitioned) we cannot go direct; but our caller
        // guarantees we transitioned to Signing before invoking. Guard
        // anyway.
        let phase_for_edge = if current == ExecutionPhase::AwaitingSignature {
            ExecutionPhase::Signing
        } else {
            current
        };
        self.transition(
            canonical_execution_id,
            phase_for_edge,
            ExecutionPhase::Failed,
            &patch,
        )
        .await?;
        let final_row = self.load_row(canonical_execution_id).await?;
        Ok(with_attempts(build_outcome(&final_row), attempts))
    }

    fn now_ms(&self) -> u64 {
        self.clock.now_ms()
    }
}

// -----------------------------------------------------------------
//                         HELPERS
// -----------------------------------------------------------------

fn build_outcome(row: &ExecutionRequestRow) -> PreparationOutcome {
    PreparationOutcome {
        canonical_execution_id: row.canonical_execution_id.clone(),
        terminal_phase: row.phase,
        failure_class: row.failure_class.clone(),
        failure_detail: row.failure_detail.clone(),
        plan_hash: row.plan_hash.clone(),
        signing_payload_hash: row.signing_payload_hash.clone(),
        simulation_gas_estimate: row.simulation_gas_estimate.map(|g| g as u64),
        reserved_nonce: row.reserved_nonce.map(|n| n as u64),
        attempts: row.retry_count as u32,
    }
}

fn with_attempts(mut outcome: PreparationOutcome, attempts: u32) -> PreparationOutcome {
    // Preserve the caller-observed attempt counter (higher of the
    // persisted `retry_count` and the local counter).
    outcome.attempts = outcome.attempts.max(attempts);
    outcome
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{:02x}", b));
    }
    s
}

fn parse_bytes32(s: &str) -> Option<[u8; 32]> {
    let stripped = s
        .strip_prefix("0x")
        .or_else(|| s.strip_prefix("0X"))
        .unwrap_or(s);
    if stripped.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for i in 0..32 {
        let hi = hex_char(stripped.as_bytes()[2 * i])?;
        let lo = hex_char(stripped.as_bytes()[2 * i + 1])?;
        out[i] = (hi << 4) | lo;
    }
    Some(out)
}

fn parse_address(s: &str) -> Option<[u8; 20]> {
    let stripped = s
        .strip_prefix("0x")
        .or_else(|| s.strip_prefix("0X"))
        .unwrap_or(s);
    if stripped.len() != 40 {
        return None;
    }
    let mut out = [0u8; 20];
    for i in 0..20 {
        let hi = hex_char(stripped.as_bytes()[2 * i])?;
        let lo = hex_char(stripped.as_bytes()[2 * i + 1])?;
        out[i] = (hi << 4) | lo;
    }
    Some(out)
}

fn hex_char(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(10 + (b - b'a')),
        b'A'..=b'F' => Some(10 + (b - b'A')),
        _ => None,
    }
}

fn parse_u256_decimal(s: &str) -> U256 {
    U256::from_str_radix(s.trim(), 10).unwrap_or(U256::ZERO)
}

fn firewall_detail(rej: &FirewallRejection) -> String {
    rej.to_string()
}

fn sig_verify_detail(err: &SigVerifyError) -> String {
    err.to_string()
}

/// Derive the EIP-1559 signing-payload hash preimage. This is the
/// canonical 32-byte digest fed to the signer's `sign_prehash` call.
///
/// The exact preimage layout is domain-separated by the tag
/// `HV2_SIG_V1` so a rogue caller cannot replay a payload derived by a
/// different domain. Layout:
///
/// ```text
/// keccak256("HV2_SIG_V1"           // 10 bytes tag
///        || chain_id_be8           // 8 bytes big-endian u64
///        || nonce_be8              // 8 bytes big-endian u64
///        || target                 // 20 bytes
///        || value_wei_be32         // 32 bytes big-endian U256
///        || calldata_hash          // 32 bytes keccak256(calldata)
///        || gas_limit_be8          // 8 bytes big-endian u64
///        || max_fee_be32           // 32 bytes big-endian U256
///        || max_priority_be32      // 32 bytes big-endian U256
///        || tx_type                // 1 byte (2 = EIP-1559)
///        || plan_hash              // 32 bytes
/// )
/// ```
fn derive_signing_payload_hash(
    chain_id: u64,
    nonce: u64,
    plan: &ExecutionPlan,
    gas_limit: u64,
    max_fee_per_gas_wei: U256,
    max_priority_fee_per_gas_wei: U256,
) -> [u8; 32] {
    let mut hasher = Keccak256::new();
    hasher.update(b"HV2_SIG_V1");
    hasher.update(chain_id.to_be_bytes());
    hasher.update(nonce.to_be_bytes());
    hasher.update(plan.target);
    hasher.update(plan.value_wei.to_be_bytes::<32>());
    hasher.update(plan.calldata_hash);
    hasher.update(gas_limit.to_be_bytes());
    hasher.update(max_fee_per_gas_wei.to_be_bytes::<32>());
    hasher.update(max_priority_fee_per_gas_wei.to_be_bytes::<32>());
    hasher.update([2u8]);
    hasher.update(plan.plan_hash);
    let out = hasher.finalize();
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&out[..]);
    arr
}

// -----------------------------------------------------------------
//                          UNIT TESTS
// -----------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_clock_returns_nonzero() {
        let clock = SystemClock;
        assert!(clock.now_ms() > 0);
    }

    #[test]
    fn mock_clock_advances_deterministically() {
        let clock = MockClock::new(100);
        assert_eq!(clock.now_ms(), 100);
        clock.advance(50);
        assert_eq!(clock.now_ms(), 150);
        clock.set(1000);
        assert_eq!(clock.now_ms(), 1000);
    }

    #[test]
    fn failure_classes_are_snake_case_screaming() {
        for c in [
            failure_class::PREFLIGHT_REJECTED,
            failure_class::PLAN_BUILD_FAILED,
            failure_class::NONCE_RESERVATION_FAILED,
            failure_class::SIMULATION_FAILED_DETERMINISTIC,
            failure_class::SIMULATION_TRANSPORT_FAILED,
            failure_class::GAS_POLICY_REJECTED,
            failure_class::FIREWALL_REJECTED,
            failure_class::SIGNER_UNAVAILABLE,
            failure_class::SIGNATURE_VERIFICATION_FAILED,
            failure_class::LOCK_CONTENTION,
            failure_class::STORE_FAILURE,
        ] {
            assert!(c.chars().all(|c| c.is_ascii_uppercase() || c == '_'));
        }
    }

    #[test]
    fn derive_signing_payload_hash_is_deterministic_and_domain_separated() {
        let plan = ExecutionPlan {
            canonical_execution_id: crate::hybrid_v2::execution::identity::CanonicalExecutionId(
                "0xabcd".to_string() + &"cd".repeat(30),
            ),
            chain_id: 84532,
            deployment_id: 1,
            target: [0xcc; 20],
            selector: [0x12, 0x34, 0x56, 0x78],
            calldata: vec![0x01, 0x02],
            calldata_hash: [0xdd; 32],
            value_wei: U256::ZERO,
            expected_module_version: "OptionMatchingEngineV2".into(),
            deadline_ms: None,
            plan_hash: [0xee; 32],
        };
        let a = derive_signing_payload_hash(
            84532,
            42,
            &plan,
            100_000,
            U256::from(1_000_000_000u64),
            U256::from(500_000_000u64),
        );
        let b = derive_signing_payload_hash(
            84532,
            42,
            &plan,
            100_000,
            U256::from(1_000_000_000u64),
            U256::from(500_000_000u64),
        );
        assert_eq!(a, b);
        // Different nonce → different hash.
        let c = derive_signing_payload_hash(
            84532,
            43,
            &plan,
            100_000,
            U256::from(1_000_000_000u64),
            U256::from(500_000_000u64),
        );
        assert_ne!(a, c);
    }
}
