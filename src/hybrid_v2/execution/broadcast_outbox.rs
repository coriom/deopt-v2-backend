//! Transactional broadcast outbox for the Hybrid V2 pipeline (Part I
//! of `BACKEND-HYBRID-V2-BROADCAST-AND-CONFIRMATION-V1`).
//!
//! ## Flow
//!
//! 1. Insert a `hybrid_v2_broadcast_state` row (idempotent) at
//!    `BROADCAST_DISABLED`.
//! 2. Run the [`BroadcastPolicyFirewall`] — reject → transition to the
//!    appropriate failure phase.
//! 3. Serialize the signed envelope via
//!    [`serialize_signed_execution`] — derive `envelope_hash`.
//! 4. **PERSIST BEFORE SEND**: set the immutable `tx_hash` +
//!    `envelope_hash` on the row, then transition
//!    `BROADCAST_DISABLED -> BROADCASTING` (or
//!    `READY_FOR_BROADCAST -> BROADCASTING` when the row was warmed
//!    earlier), incrementing `submission_attempt_count` and setting
//!    `first_submission_at_ms` if unset. Commit.
//! 5. Call `rpc.send_raw_transaction(envelope.raw_bytes)`.
//! 6. Classify the outcome:
//!    - `Accepted { provider_tx_hash }` where hash matches → SUBMITTED.
//!    - `Accepted { provider_tx_hash }` where hash differs →
//!      MANUAL_INTERVENTION_REQUIRED with `failure_class =
//!      PROVIDER_HASH_MISMATCH` (critical).
//!    - `AlreadyKnown` → SUBMITTED (`provider_classification =
//!      ALREADY_KNOWN`) idempotent path.
//!    - `NonceTooLow` / `NonceTooHigh` / `ReplacementUnderpriced` →
//!      MANUAL_INTERVENTION_REQUIRED (`NONCE_CONFLICT`).
//!    - `ProviderRejection` → MANUAL_INTERVENTION_REQUIRED (`PROVIDER_REJECTED`).
//!    - `Timeout` / `Transport` / `Unavailable` → SUBMISSION_UNKNOWN
//!      (`TRANSPORT_AMBIGUOUS`).
//! 7. Update `last_submission_at_ms`.
//!
//! ## Frozen safety
//!
//! - **The tx hash is persisted BEFORE the network call**, so a mid-call
//!   crash resumes at `BROADCASTING` — the recovery path uses
//!   `transaction_by_hash(envelope_hash)` to determine the on-chain
//!   fate. It NEVER re-signs.
//! - **Provider hash mismatch is a hard failure**, not a warning. Any
//!   non-matching hash triggers `MANUAL_INTERVENTION_REQUIRED` and
//!   halts the pipeline for the operator to investigate.
//! - **No automatic nonce replacement / fee bump.** Deterministic
//!   rejections escalate.

use crate::hybrid_v2::execution::broadcast_firewall::{
    BroadcastFirewallRejection, BroadcastPolicyFirewall,
};
use crate::hybrid_v2::execution::broadcast_rpc::{
    BroadcastRpcError, ExecutionBroadcastRpcClient, SendOutcome,
};
use crate::hybrid_v2::execution::broadcast_state::{BroadcastPhase, BroadcastStatePatch};
use crate::hybrid_v2::execution::orchestrator::Clock;
use crate::hybrid_v2::execution::persistence::ExecutionRequestRow;
use crate::hybrid_v2::execution::plan::ExecutionPlan;
use crate::hybrid_v2::execution::signer::SignedTx;
use crate::hybrid_v2::execution::tx_serialization::{
    serialize_signed_execution, SignedExecutionEnvelope, TxSerializationError,
};
use crate::hybrid_v2::persistence::HybridV2ProjectionStore;
use crate::hybrid_v2::readiness::ReadinessReport;
use alloy_primitives::U256;
use std::sync::Arc;
use thiserror::Error;

// -----------------------------------------------------------------
//                          RESULT TYPES
// -----------------------------------------------------------------

/// Failure-class strings the outbox emits. Kept as a const-namespaced
/// list so operators can grep on either the token or the code path.
pub mod failure_class {
    pub const PROVIDER_HASH_MISMATCH: &str = "PROVIDER_HASH_MISMATCH";
    pub const NONCE_CONFLICT: &str = "NONCE_CONFLICT";
    pub const PROVIDER_REJECTED: &str = "PROVIDER_REJECTED";
    pub const TRANSPORT_AMBIGUOUS: &str = "TRANSPORT_AMBIGUOUS";
    pub const FIREWALL_REJECTED: &str = "FIREWALL_REJECTED";
    pub const SERIALIZATION_FAILED: &str = "SERIALIZATION_FAILED";
}

/// Provider-classification strings the outbox writes to
/// `provider_classification`.
pub mod provider_classification {
    pub const ACCEPTED: &str = "ACCEPTED";
    pub const ALREADY_KNOWN: &str = "ALREADY_KNOWN";
    pub const NONCE_TOO_LOW: &str = "NONCE_TOO_LOW";
    pub const NONCE_TOO_HIGH: &str = "NONCE_TOO_HIGH";
    pub const REPLACEMENT_UNDERPRICED: &str = "REPLACEMENT_UNDERPRICED";
    pub const PROVIDER_HASH_MISMATCH: &str = "PROVIDER_HASH_MISMATCH";
    pub const TIMEOUT_BEFORE_ACCEPTANCE: &str = "TIMEOUT_BEFORE_ACCEPTANCE";
    pub const TIMEOUT_AFTER_ACCEPTANCE: &str = "TIMEOUT_AFTER_ACCEPTANCE";
    pub const PROVIDER_REJECTION: &str = "PROVIDER_REJECTION";
}

/// Structured outcome returned by `BroadcastOutbox::submit`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutboxOutcome {
    pub canonical_execution_id: String,
    pub tx_hash: Option<[u8; 32]>,
    pub phase: BroadcastPhase,
    pub provider_classification: Option<String>,
    pub failure_class: Option<String>,
    pub failure_detail: Option<String>,
}

/// Unrecoverable outbox errors — the caller cannot fix these by
/// retrying. Firewall rejections and provider errors are represented
/// on the [`OutboxOutcome`] as terminal phases; only genuine
/// programming / persistence failures surface here.
#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum OutboxError {
    #[error("operation lock contention (deployment {deployment_id})")]
    LockContention { deployment_id: i64 },
    #[error("persistence failure: {0}")]
    StoreFailure(String),
    #[error("unrecoverable: {0}")]
    Unrecoverable(String),
}

// -----------------------------------------------------------------
//                          OUTBOX
// -----------------------------------------------------------------

/// The broadcast outbox. Owned by the orchestrator; borrows the store
/// + rpc + firewall for the duration of a single `submit` call.
pub struct BroadcastOutbox {
    pub store: Arc<dyn HybridV2ProjectionStore>,
    pub rpc: Arc<dyn ExecutionBroadcastRpcClient>,
    pub clock: Arc<dyn Clock>,
    pub deployment_id: i64,
}

impl BroadcastOutbox {
    /// Submit an already-signed, already-verified execution. `firewall`
    /// is passed by reference so callers can vary its configuration
    /// per attempt (e.g. flip `pre_send_hash_probe` for operator
    /// investigation).
    #[allow(clippy::too_many_arguments)]
    pub async fn submit(
        &self,
        req: ExecutionRequestRow,
        plan: ExecutionPlan,
        signed: SignedTx,
        expected_signer: [u8; 20],
        nonce: u64,
        gas_limit: u64,
        max_fee_per_gas_wei: U256,
        max_priority_fee_per_gas_wei: U256,
        readiness: ReadinessReport,
        firewall: &BroadcastPolicyFirewall<'_>,
    ) -> Result<OutboxOutcome, OutboxError> {
        let now_ms = || self.clock.now_ms();

        // Step 1: idempotent insert.
        self.store
            .insert_broadcast_state(&req.canonical_execution_id, now_ms() as i64)
            .await
            .map_err(|e| OutboxError::StoreFailure(e.to_string()))?;

        // Step 2: firewall revalidation.
        // Serialize envelope FIRST so we can pass it to the firewall.
        let envelope = match serialize_signed_execution(
            &plan,
            &signed,
            nonce,
            gas_limit,
            max_fee_per_gas_wei,
            max_priority_fee_per_gas_wei,
        ) {
            Ok(e) => e,
            Err(err) => {
                let detail = match &err {
                    TxSerializationError::WrongTxType(_)
                    | TxSerializationError::HighSSignature
                    | TxSerializationError::MalformedSignature(_)
                    | TxSerializationError::ChainMismatch { .. }
                    | TxSerializationError::RlpEncodeError(_) => err.to_string(),
                };
                return self
                    .escalate_manual(
                        &req.canonical_execution_id,
                        BroadcastPhase::BroadcastDisabled,
                        failure_class::SERIALIZATION_FAILED,
                        &detail,
                        now_ms(),
                    )
                    .await;
            }
        };

        if let Err(rej) = firewall
            .revalidate_before_send(&req, &plan, &signed, &envelope, expected_signer, &readiness)
            .await
        {
            return self
                .handle_firewall_rejection(&req.canonical_execution_id, rej, now_ms())
                .await;
        }

        // Step 3: persist tx_hash + envelope_hash BEFORE the network
        // call. The immutability trigger blocks a divergent re-write on
        // retry, so this is safe to invoke repeatedly.
        let tx_hash_hex = envelope.envelope_hash_hex();
        self.store
            .set_broadcast_tx_hash(
                &req.canonical_execution_id,
                &tx_hash_hex,
                &tx_hash_hex,
                &tx_hash_hex,
                now_ms() as i64,
            )
            .await
            .map_err(|e| OutboxError::StoreFailure(e.to_string()))?;

        // Determine which from-phase we are advancing from. `insert`
        // above created BROADCAST_DISABLED; a warmed row may already be
        // at READY_FOR_BROADCAST — either is fine.
        let current = self
            .store
            .get_broadcast_state(&req.canonical_execution_id)
            .await
            .map_err(|e| OutboxError::StoreFailure(e.to_string()))?
            .ok_or_else(|| OutboxError::Unrecoverable("broadcast row disappeared".into()))?;
        let from_phase = current.phase;
        if !matches!(
            from_phase,
            BroadcastPhase::BroadcastDisabled | BroadcastPhase::ReadyForBroadcast
        ) {
            // A concurrent submitter got here first. Return the current
            // outcome without re-broadcasting.
            return Ok(OutboxOutcome {
                canonical_execution_id: req.canonical_execution_id,
                tx_hash: current.tx_hash.as_deref().and_then(parse_bytes32_lax),
                phase: current.phase,
                provider_classification: current.provider_classification,
                failure_class: current.failure_class,
                failure_detail: current.failure_detail,
            });
        }
        let attempt_count = current.submission_attempt_count + 1;
        let first_submission = current.first_submission_at_ms.or(Some(now_ms() as i64));
        let patch = BroadcastStatePatch {
            submission_attempt_count: Some(attempt_count),
            first_submission_at_ms: first_submission,
            last_submission_at_ms: Some(now_ms() as i64),
            ..Default::default()
        };
        let ok = self
            .store
            .update_broadcast_phase(
                &req.canonical_execution_id,
                from_phase,
                BroadcastPhase::Broadcasting,
                now_ms() as i64,
                patch,
            )
            .await
            .map_err(|e| OutboxError::StoreFailure(e.to_string()))?;
        if !ok {
            // Lost update — concurrent transition; treat as contention.
            return Err(OutboxError::LockContention {
                deployment_id: self.deployment_id,
            });
        }

        // Step 4: the actual send.
        let send_result = self.rpc.send_raw_transaction(&envelope.raw_bytes).await;

        // Step 5: classify + persist.
        self.classify_and_persist(
            &req.canonical_execution_id,
            &envelope,
            send_result,
            now_ms(),
        )
        .await
    }

    async fn handle_firewall_rejection(
        &self,
        canonical_execution_id: &str,
        rejection: BroadcastFirewallRejection,
        now_ms_val: u64,
    ) -> Result<OutboxOutcome, OutboxError> {
        // Map every firewall rejection to a broadcast phase + failure
        // class. Cancelled goes to CancelledBeforeBroadcast; everything
        // else goes to ManualInterventionRequired (frozen safety —
        // there is no auto-remediation).
        let (target_phase, detail) = match &rejection {
            BroadcastFirewallRejection::ExecutionCancelled => (
                BroadcastPhase::CancelledBeforeBroadcast,
                "execution cancelled".to_string(),
            ),
            other => (
                BroadcastPhase::ManualInterventionRequired,
                other.to_string(),
            ),
        };
        self.escalate_manual_typed(
            canonical_execution_id,
            BroadcastPhase::BroadcastDisabled,
            target_phase,
            failure_class::FIREWALL_REJECTED,
            &detail,
            now_ms_val,
        )
        .await
    }

    async fn classify_and_persist(
        &self,
        canonical_execution_id: &str,
        envelope: &SignedExecutionEnvelope,
        send_result: Result<SendOutcome, BroadcastRpcError>,
        now_ms_val: u64,
    ) -> Result<OutboxOutcome, OutboxError> {
        match send_result {
            Ok(SendOutcome::Accepted { provider_tx_hash }) => {
                if provider_tx_hash != envelope.envelope_hash {
                    return self
                        .critical_hash_mismatch(
                            canonical_execution_id,
                            envelope,
                            provider_tx_hash,
                            now_ms_val,
                        )
                        .await;
                }
                self.transition_to_submitted(
                    canonical_execution_id,
                    envelope,
                    provider_classification::ACCEPTED,
                    now_ms_val,
                )
                .await
            }
            Ok(SendOutcome::AlreadyKnown { provider_tx_hash }) => {
                // If the provider reports a hash it MUST equal our own
                // (all-zeros means the provider omitted it; in that
                // case we accept the message alone as ALREADY_KNOWN).
                if provider_tx_hash != [0u8; 32] && provider_tx_hash != envelope.envelope_hash {
                    return self
                        .critical_hash_mismatch(
                            canonical_execution_id,
                            envelope,
                            provider_tx_hash,
                            now_ms_val,
                        )
                        .await;
                }
                self.transition_to_submitted(
                    canonical_execution_id,
                    envelope,
                    provider_classification::ALREADY_KNOWN,
                    now_ms_val,
                )
                .await
            }
            Ok(SendOutcome::NonceTooLow) => {
                self.escalate_manual_typed(
                    canonical_execution_id,
                    BroadcastPhase::Broadcasting,
                    BroadcastPhase::ManualInterventionRequired,
                    failure_class::NONCE_CONFLICT,
                    provider_classification::NONCE_TOO_LOW,
                    now_ms_val,
                )
                .await
            }
            Ok(SendOutcome::NonceTooHigh) => {
                self.escalate_manual_typed(
                    canonical_execution_id,
                    BroadcastPhase::Broadcasting,
                    BroadcastPhase::ManualInterventionRequired,
                    failure_class::NONCE_CONFLICT,
                    provider_classification::NONCE_TOO_HIGH,
                    now_ms_val,
                )
                .await
            }
            Ok(SendOutcome::ReplacementUnderpriced) => {
                self.escalate_manual_typed(
                    canonical_execution_id,
                    BroadcastPhase::Broadcasting,
                    BroadcastPhase::ManualInterventionRequired,
                    failure_class::NONCE_CONFLICT,
                    provider_classification::REPLACEMENT_UNDERPRICED,
                    now_ms_val,
                )
                .await
            }
            Ok(SendOutcome::ProviderRejection { code, message }) => {
                let detail = format!("code={code} message={message}");
                self.escalate_manual_typed(
                    canonical_execution_id,
                    BroadcastPhase::Broadcasting,
                    BroadcastPhase::ManualInterventionRequired,
                    failure_class::PROVIDER_REJECTED,
                    &detail,
                    now_ms_val,
                )
                .await
            }
            Err(err) => match err {
                BroadcastRpcError::Timeout
                | BroadcastRpcError::Transport(_)
                | BroadcastRpcError::Unavailable(_)
                | BroadcastRpcError::RateLimited => {
                    self.transition_to_submission_unknown(
                        canonical_execution_id,
                        envelope,
                        &err,
                        now_ms_val,
                    )
                    .await
                }
                other => {
                    let detail = other.to_string();
                    self.escalate_manual_typed(
                        canonical_execution_id,
                        BroadcastPhase::Broadcasting,
                        BroadcastPhase::ManualInterventionRequired,
                        failure_class::PROVIDER_REJECTED,
                        &detail,
                        now_ms_val,
                    )
                    .await
                }
            },
        }
    }

    async fn transition_to_submitted(
        &self,
        canonical_execution_id: &str,
        envelope: &SignedExecutionEnvelope,
        classification: &str,
        now_ms_val: u64,
    ) -> Result<OutboxOutcome, OutboxError> {
        let patch = BroadcastStatePatch {
            provider_classification: Some(classification.to_string()),
            last_submission_at_ms: Some(now_ms_val as i64),
            ..Default::default()
        };
        let ok = self
            .store
            .update_broadcast_phase(
                canonical_execution_id,
                BroadcastPhase::Broadcasting,
                BroadcastPhase::Submitted,
                now_ms_val as i64,
                patch,
            )
            .await
            .map_err(|e| OutboxError::StoreFailure(e.to_string()))?;
        if !ok {
            return Err(OutboxError::Unrecoverable(
                "post-send phase transition lost update".to_string(),
            ));
        }
        Ok(OutboxOutcome {
            canonical_execution_id: canonical_execution_id.to_string(),
            tx_hash: Some(envelope.envelope_hash),
            phase: BroadcastPhase::Submitted,
            provider_classification: Some(classification.to_string()),
            failure_class: None,
            failure_detail: None,
        })
    }

    async fn transition_to_submission_unknown(
        &self,
        canonical_execution_id: &str,
        envelope: &SignedExecutionEnvelope,
        err: &BroadcastRpcError,
        now_ms_val: u64,
    ) -> Result<OutboxOutcome, OutboxError> {
        let detail = err.to_string();
        let patch = BroadcastStatePatch {
            provider_classification: Some(
                provider_classification::TIMEOUT_BEFORE_ACCEPTANCE.to_string(),
            ),
            failure_class: Some(failure_class::TRANSPORT_AMBIGUOUS.to_string()),
            failure_detail: Some(detail.clone()),
            last_submission_at_ms: Some(now_ms_val as i64),
            ..Default::default()
        };
        let ok = self
            .store
            .update_broadcast_phase(
                canonical_execution_id,
                BroadcastPhase::Broadcasting,
                BroadcastPhase::SubmissionUnknown,
                now_ms_val as i64,
                patch,
            )
            .await
            .map_err(|e| OutboxError::StoreFailure(e.to_string()))?;
        if !ok {
            return Err(OutboxError::Unrecoverable(
                "post-send phase transition lost update".to_string(),
            ));
        }
        Ok(OutboxOutcome {
            canonical_execution_id: canonical_execution_id.to_string(),
            tx_hash: Some(envelope.envelope_hash),
            phase: BroadcastPhase::SubmissionUnknown,
            provider_classification: Some(
                provider_classification::TIMEOUT_BEFORE_ACCEPTANCE.to_string(),
            ),
            failure_class: Some(failure_class::TRANSPORT_AMBIGUOUS.to_string()),
            failure_detail: Some(detail),
        })
    }

    async fn critical_hash_mismatch(
        &self,
        canonical_execution_id: &str,
        envelope: &SignedExecutionEnvelope,
        provider_tx_hash: [u8; 32],
        now_ms_val: u64,
    ) -> Result<OutboxOutcome, OutboxError> {
        // Neutral logging — the tracing subscriber may forward this to
        // operator dashboards. NEVER include raw signature bytes.
        tracing::error!(
            canonical_execution_id = %canonical_execution_id,
            local_hash = %format!("0x{}", hex_encode(&envelope.envelope_hash)),
            provider_hash = %format!("0x{}", hex_encode(&provider_tx_hash)),
            "PROVIDER_HASH_MISMATCH: provider tx_hash differs from local envelope_hash"
        );
        let detail = format!(
            "local=0x{} provider=0x{}",
            hex_encode(&envelope.envelope_hash),
            hex_encode(&provider_tx_hash)
        );
        self.escalate_manual_typed(
            canonical_execution_id,
            BroadcastPhase::Broadcasting,
            BroadcastPhase::ManualInterventionRequired,
            failure_class::PROVIDER_HASH_MISMATCH,
            &detail,
            now_ms_val,
        )
        .await
    }

    async fn escalate_manual(
        &self,
        canonical_execution_id: &str,
        from_phase: BroadcastPhase,
        failure_class_str: &str,
        detail: &str,
        now_ms_val: u64,
    ) -> Result<OutboxOutcome, OutboxError> {
        self.escalate_manual_typed(
            canonical_execution_id,
            from_phase,
            BroadcastPhase::ManualInterventionRequired,
            failure_class_str,
            detail,
            now_ms_val,
        )
        .await
    }

    async fn escalate_manual_typed(
        &self,
        canonical_execution_id: &str,
        from_phase: BroadcastPhase,
        target_phase: BroadcastPhase,
        failure_class_str: &str,
        detail: &str,
        now_ms_val: u64,
    ) -> Result<OutboxOutcome, OutboxError> {
        let patch = BroadcastStatePatch {
            failure_class: Some(failure_class_str.to_string()),
            failure_detail: Some(detail.to_string()),
            terminal_at_ms: Some(now_ms_val as i64),
            provider_classification: if failure_class_str == failure_class::PROVIDER_HASH_MISMATCH {
                Some(provider_classification::PROVIDER_HASH_MISMATCH.to_string())
            } else {
                None
            },
            ..Default::default()
        };
        // If the from_phase is not the persisted phase, refresh it.
        let live_phase = self
            .store
            .get_broadcast_state(canonical_execution_id)
            .await
            .map_err(|e| OutboxError::StoreFailure(e.to_string()))?
            .map(|r| r.phase)
            .unwrap_or(from_phase);
        let effective_from = if live_phase.can_transition_to(target_phase) {
            live_phase
        } else {
            from_phase
        };
        let ok = self
            .store
            .update_broadcast_phase(
                canonical_execution_id,
                effective_from,
                target_phase,
                now_ms_val as i64,
                patch,
            )
            .await
            .map_err(|e| OutboxError::StoreFailure(e.to_string()))?;
        if !ok {
            return Err(OutboxError::Unrecoverable(format!(
                "escalate_manual failed: phase transition {} -> {} rejected",
                effective_from, target_phase
            )));
        }
        Ok(OutboxOutcome {
            canonical_execution_id: canonical_execution_id.to_string(),
            tx_hash: None,
            phase: target_phase,
            provider_classification: if failure_class_str == failure_class::PROVIDER_HASH_MISMATCH {
                Some(provider_classification::PROVIDER_HASH_MISMATCH.to_string())
            } else {
                None
            },
            failure_class: Some(failure_class_str.to_string()),
            failure_detail: Some(detail.to_string()),
        })
    }
}

// -----------------------------------------------------------------
//                          HELPERS
// -----------------------------------------------------------------

fn hex_encode(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{:02x}", b));
    }
    s
}

fn parse_bytes32_lax(s: &str) -> Option<[u8; 32]> {
    let stripped = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X"))?;
    if stripped.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for i in 0..32 {
        out[i] = u8::from_str_radix(&stripped[2 * i..2 * i + 2], 16).ok()?;
    }
    Some(out)
}

// -----------------------------------------------------------------
//                          UNIT TESTS
// -----------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hybrid_v2::execution::broadcast_firewall::BroadcastFirewallConfig;
    use crate::hybrid_v2::execution::broadcast_rpc::{BlockHeader, TransactionSummary, TxReceipt};
    use crate::hybrid_v2::execution::gas_policy::GasFeePolicy;
    use crate::hybrid_v2::execution::identity::CanonicalExecutionId;
    use crate::hybrid_v2::execution::orchestrator::MockClock;
    use crate::hybrid_v2::execution::plan::executeMatchCall;
    use crate::hybrid_v2::execution::rpc::BlockTag;
    use crate::hybrid_v2::execution::state::ExecutionPhase;
    use crate::hybrid_v2::execution::target_policy::TargetPolicy;
    use crate::hybrid_v2::manifest::{ActivationStatus, ManifestModuleAddresses, ManifestParams};
    use crate::hybrid_v2::persistence::InMemoryProjectionStore;
    use crate::hybrid_v2::readiness::{ReadinessReport, ReadinessState};
    use alloy_sol_types::SolCall;
    use async_trait::async_trait;
    use std::sync::Mutex;

    const ENGINE_HEX: &str = "0x5a5EBF9A9CCd7c012518569DE8283982982670f6";

    fn engine_bytes() -> [u8; 20] {
        let mut out = [0u8; 20];
        let s = ENGINE_HEX.trim_start_matches("0x");
        for i in 0..20 {
            out[i] = u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap();
        }
        out
    }

    // --------------------- mock RPC ----------------------------------

    #[derive(Default)]
    struct MockRpc {
        inner: Mutex<MockRpcInner>,
    }

    #[derive(Default)]
    struct MockRpcInner {
        send_responses: Vec<Result<SendOutcome, BroadcastRpcError>>,
        written_methods: Vec<&'static str>,
    }

    impl MockRpc {
        fn new_accept_matching() -> Self {
            let mut inner = MockRpcInner::default();
            // The outcome is filled in per-test by push().
            inner.send_responses.push(Ok(SendOutcome::Accepted {
                provider_tx_hash: [0u8; 32],
            }));
            Self {
                inner: Mutex::new(inner),
            }
        }

        fn push(&self, o: Result<SendOutcome, BroadcastRpcError>) {
            let mut g = self.inner.lock().unwrap();
            g.send_responses.clear();
            g.send_responses.push(o);
        }

        fn recorded_write_methods(&self) -> Vec<&'static str> {
            self.inner.lock().unwrap().written_methods.clone()
        }
    }

    #[async_trait]
    impl ExecutionBroadcastRpcClient for MockRpc {
        async fn chain_id(&self) -> Result<u64, BroadcastRpcError> {
            Ok(84532)
        }
        async fn head_block_number(&self) -> Result<u64, BroadcastRpcError> {
            Ok(100)
        }
        async fn finalized_block_number(&self) -> Result<Option<u64>, BroadcastRpcError> {
            Ok(Some(50))
        }
        async fn transaction_count(
            &self,
            _address: [u8; 20],
            _block_tag: BlockTag,
        ) -> Result<u64, BroadcastRpcError> {
            Ok(0)
        }
        async fn transaction_by_hash(
            &self,
            _tx_hash: [u8; 32],
        ) -> Result<Option<TransactionSummary>, BroadcastRpcError> {
            Ok(None)
        }
        async fn receipt_by_hash(
            &self,
            _tx_hash: [u8; 32],
        ) -> Result<Option<TxReceipt>, BroadcastRpcError> {
            Ok(None)
        }
        async fn block_header_by_number(
            &self,
            _number: u64,
        ) -> Result<Option<BlockHeader>, BroadcastRpcError> {
            Ok(None)
        }
        async fn block_header_by_hash(
            &self,
            _hash: [u8; 32],
        ) -> Result<Option<BlockHeader>, BroadcastRpcError> {
            Ok(None)
        }
        async fn send_raw_transaction(
            &self,
            _raw_tx_bytes: &[u8],
        ) -> Result<SendOutcome, BroadcastRpcError> {
            let mut g = self.inner.lock().unwrap();
            g.written_methods.push("eth_sendRawTransaction");
            g.send_responses.remove(0)
        }
    }

    // --------------------- fixture builders --------------------------

    fn make_manifest() -> ManifestParams {
        ManifestParams {
            chain_id: 84532,
            manifest_address: "0x000000000000000000000000000000000000d001".into(),
            manifest_hash: format!("0x{}", "11".repeat(32)),
            module_addresses_hash: format!("0x{}", "22".repeat(32)),
            critical_config_hash: format!("0x{}", "33".repeat(32)),
            architecture_version: 1,
            storage_version: 1,
            event_version: 1,
            deployment_version: 1,
            manifest_schema_version: 1,
            environment_tag: format!("0x{}", "00".repeat(32)),
            deployer: format!("0x{}", "de".repeat(20)),
            deployment_block: 1,
            deployment_timestamp: 1,
            module_addresses: ManifestModuleAddresses {
                subaccount_registry: format!("0x{}", "01".repeat(20)),
                collateral_vault: format!("0x{}", "02".repeat(20)),
                options_positions_ledger: format!("0x{}", "03".repeat(20)),
                risk_module: format!("0x{}", "04".repeat(20)),
                margin_engine: format!("0x{}", "05".repeat(20)),
                option_matching_engine: ENGINE_HEX.into(),
                escape_controller: format!("0x{}", "07".repeat(20)),
                recovery_finalizer: format!("0x{}", "08".repeat(20)),
                oracle_adapter: format!("0x{}", "09".repeat(20)),
                options_risk_provider: format!("0x{}", "0a".repeat(20)),
                quote_token: format!("0x{}", "0b".repeat(20)),
                fees_manager_v2: None,
                option_execution_fee_adapter: None,
                protocol_timelock: None,
                governance: None,
                guardian: None,
            },
            protocol_fee_subkey: format!("0x{}", "01".repeat(32)),
            rebate_budget_subkey: format!("0x{}", "02".repeat(32)),
            insurance_fund_subkey: format!("0x{}", "03".repeat(32)),
            max_collateral_tokens: 8,
            max_active_series: 32,
            all_capabilities_mask: format!("0x{}", "ff".repeat(32)),
            recovery_activation_delay_seconds: 3600,
            recovery_pause_max_duration_blocks: 1000,
            activation_status: ActivationStatus::Active,
        }
    }

    fn make_plan(chain_id: u64) -> ExecutionPlan {
        let calldata = vec![0xde, 0xad, 0xbe, 0xef];
        let calldata_hash = {
            use sha3::{Digest, Keccak256};
            let mut h = [0u8; 32];
            let d = Keccak256::digest(&calldata);
            h.copy_from_slice(&d[..]);
            h
        };
        ExecutionPlan {
            canonical_execution_id: CanonicalExecutionId(format!("0x{}", "aa".repeat(32))),
            chain_id,
            deployment_id: 1,
            target: engine_bytes(),
            selector: executeMatchCall::SELECTOR,
            calldata,
            calldata_hash,
            value_wei: U256::ZERO,
            expected_module_version: "OptionMatchingEngineV2".into(),
            deadline_ms: None,
            plan_hash: [0xee; 32],
        }
    }

    fn make_row(chain_id: u64, plan: &ExecutionPlan, signer: [u8; 20]) -> ExecutionRequestRow {
        ExecutionRequestRow {
            canonical_execution_id: plan.canonical_execution_id.as_str().to_string(),
            deployment_id: 1,
            chain_id: chain_id as i64,
            execution_kind: "HYBRID_V2_OPTION_MATCH".into(),
            buyer_order_hash: format!("0x{}", "aa".repeat(32)),
            seller_order_hash: format!("0x{}", "bb".repeat(32)),
            buyer_subkey: format!("0x{}", "aa".repeat(32)),
            seller_subkey: format!("0x{}", "bb".repeat(32)),
            series_id: "42".into(),
            fill_quantity_1e8: "100000000".into(),
            premium_amount: "50000000".into(),
            fee_schedule_epoch: None,
            source_matched_execution_id: None,
            target_contract: format!("0x{}", hex_encode(&plan.target)),
            selector: format!("0x{}", hex_encode(&plan.selector)),
            calldata_hash: Some(format!("0x{}", hex_encode(&plan.calldata_hash))),
            plan_hash: Some(format!("0x{}", hex_encode(&plan.plan_hash))),
            tx_value_wei: "0".into(),
            simulation_block_number: Some(100),
            simulation_block_hash: Some(format!("0x{}", "cc".repeat(32))),
            simulation_gas_estimate: Some(500_000),
            simulation_result_json: Some(serde_json::json!({})),
            signer_identity: Some(format!("0x{}", hex_encode(&signer))),
            signing_payload_hash: Some(format!("0x{}", "ff".repeat(32))),
            signature_r: Some(format!("0x{}", "11".repeat(32))),
            signature_s: Some(format!("0x{}", "22".repeat(32))),
            signature_v: Some(0),
            recovered_signer: Some(format!("0x{}", hex_encode(&signer))),
            gas_limit: Some(1_000_000),
            max_fee_per_gas_wei: Some("2000000000".into()),
            max_priority_fee_per_gas_wei: Some("500000000".into()),
            reserved_nonce: Some(42),
            phase: ExecutionPhase::SignatureVerified,
            failure_class: None,
            failure_detail: None,
            retry_count: 0,
            holder_epoch: None,
            signer_request_idempotency_key: None,
            created_at_ms: 1,
            updated_at_ms: 1,
        }
    }

    fn make_signed(signer: [u8; 20]) -> SignedTx {
        SignedTx {
            signature_r: [0x11; 32],
            signature_s: [0x22; 32],
            signature_v: 0,
            recovered_signer: signer,
            tx_type: 2,
        }
    }

    fn make_readiness_ready() -> ReadinessReport {
        ReadinessReport {
            runtime: ReadinessState::ready(),
            rebuild: ReadinessState::ready(),
            reconciliation: ReadinessState::ready(),
        }
    }

    fn gas_policy() -> GasFeePolicy {
        GasFeePolicy {
            max_gas_limit: 5_000_000,
            gas_limit_multiplier_bps: 12_000,
            max_fee_per_gas_wei: U256::from(10_000_000_000u64),
            max_priority_fee_per_gas_wei: U256::from(2_000_000_000u64),
            max_total_native_cost_wei: U256::from(10u64).pow(U256::from(18u64)),
            abnormal_estimate_reject_threshold: 10,
        }
    }

    async fn seed(store: &InMemoryProjectionStore, row: &ExecutionRequestRow) {
        store.insert_execution_request(row).await.unwrap();
    }

    async fn happy_setup() -> (
        Arc<InMemoryProjectionStore>,
        Arc<MockRpc>,
        TargetPolicy,
        GasFeePolicy,
        ExecutionRequestRow,
        ExecutionPlan,
        SignedTx,
        [u8; 20],
    ) {
        let store = Arc::new(InMemoryProjectionStore::new());
        let manifest = make_manifest();
        let plan = make_plan(84532);
        let signer = [0x77u8; 20];
        let row = make_row(84532, &plan, signer);
        seed(&store, &row).await;
        let target_policy = TargetPolicy::from_manifest(&manifest).unwrap();
        let gp = gas_policy();
        let rpc = Arc::new(MockRpc::new_accept_matching());
        (
            store,
            rpc,
            target_policy,
            gp,
            row,
            plan,
            make_signed(signer),
            signer,
        )
    }

    #[tokio::test]
    async fn outbox_happy_path_transitions_to_submitted() {
        let (store, rpc, target_policy, gp, row, plan, signed, signer) = happy_setup().await;
        // Compute envelope hash to seed the mock RPC's Accepted reply
        // with a matching value.
        let env = serialize_signed_execution(
            &plan,
            &signed,
            42,
            1_000_000,
            U256::from(2_000_000_000u64),
            U256::from(500_000_000u64),
        )
        .unwrap();
        rpc.push(Ok(SendOutcome::Accepted {
            provider_tx_hash: env.envelope_hash,
        }));
        let clock = Arc::new(MockClock::new(1000));
        let allowed = vec![84532u64];
        let firewall = BroadcastPolicyFirewall {
            store: store.as_ref(),
            target_policy: &target_policy,
            gas_policy: &gp,
            broadcast_config: BroadcastFirewallConfig {
                broadcast_enabled: true,
                pre_send_hash_probe: false,
            },
            configured_chain_id: 84532,
            deployment_id: 1,
            simulation_max_age_ms: 3_600_000,
            allowed_broadcast_chain_ids: &allowed,
            now_ms: 1_000,
            rpc: None,
        };
        let outbox = BroadcastOutbox {
            store: store.clone(),
            rpc: rpc.clone(),
            clock,
            deployment_id: 1,
        };
        let outcome = outbox
            .submit(
                row,
                plan,
                signed,
                signer,
                42,
                1_000_000,
                U256::from(2_000_000_000u64),
                U256::from(500_000_000u64),
                make_readiness_ready(),
                &firewall,
            )
            .await
            .expect("submit");
        assert_eq!(outcome.phase, BroadcastPhase::Submitted);
        assert_eq!(
            outcome.provider_classification.as_deref(),
            Some(provider_classification::ACCEPTED)
        );
        assert_eq!(outcome.tx_hash, Some(env.envelope_hash));
        assert!(outcome.failure_class.is_none());

        // Only the write method is eth_sendRawTransaction.
        let writes = rpc.recorded_write_methods();
        assert_eq!(writes, vec!["eth_sendRawTransaction"]);
    }

    #[tokio::test]
    async fn outbox_provider_hash_mismatch_escalates_manual() {
        let (store, rpc, target_policy, gp, row, plan, signed, signer) = happy_setup().await;
        rpc.push(Ok(SendOutcome::Accepted {
            provider_tx_hash: [0xEEu8; 32], // deliberately wrong
        }));
        let clock = Arc::new(MockClock::new(1000));
        let allowed = vec![84532u64];
        let firewall = BroadcastPolicyFirewall {
            store: store.as_ref(),
            target_policy: &target_policy,
            gas_policy: &gp,
            broadcast_config: BroadcastFirewallConfig {
                broadcast_enabled: true,
                pre_send_hash_probe: false,
            },
            configured_chain_id: 84532,
            deployment_id: 1,
            simulation_max_age_ms: 3_600_000,
            allowed_broadcast_chain_ids: &allowed,
            now_ms: 1_000,
            rpc: None,
        };
        let outbox = BroadcastOutbox {
            store: store.clone(),
            rpc: rpc.clone(),
            clock,
            deployment_id: 1,
        };
        let outcome = outbox
            .submit(
                row,
                plan,
                signed,
                signer,
                42,
                1_000_000,
                U256::from(2_000_000_000u64),
                U256::from(500_000_000u64),
                make_readiness_ready(),
                &firewall,
            )
            .await
            .expect("submit");
        assert_eq!(outcome.phase, BroadcastPhase::ManualInterventionRequired);
        assert_eq!(
            outcome.failure_class.as_deref(),
            Some(failure_class::PROVIDER_HASH_MISMATCH)
        );
    }

    #[tokio::test]
    async fn outbox_timeout_transitions_to_submission_unknown() {
        let (store, rpc, target_policy, gp, row, plan, signed, signer) = happy_setup().await;
        rpc.push(Err(BroadcastRpcError::Timeout));
        let clock = Arc::new(MockClock::new(1000));
        let allowed = vec![84532u64];
        let firewall = BroadcastPolicyFirewall {
            store: store.as_ref(),
            target_policy: &target_policy,
            gas_policy: &gp,
            broadcast_config: BroadcastFirewallConfig {
                broadcast_enabled: true,
                pre_send_hash_probe: false,
            },
            configured_chain_id: 84532,
            deployment_id: 1,
            simulation_max_age_ms: 3_600_000,
            allowed_broadcast_chain_ids: &allowed,
            now_ms: 1_000,
            rpc: None,
        };
        let outbox = BroadcastOutbox {
            store: store.clone(),
            rpc: rpc.clone(),
            clock,
            deployment_id: 1,
        };
        let outcome = outbox
            .submit(
                row,
                plan,
                signed,
                signer,
                42,
                1_000_000,
                U256::from(2_000_000_000u64),
                U256::from(500_000_000u64),
                make_readiness_ready(),
                &firewall,
            )
            .await
            .expect("submit");
        assert_eq!(outcome.phase, BroadcastPhase::SubmissionUnknown);
        assert_eq!(
            outcome.failure_class.as_deref(),
            Some(failure_class::TRANSPORT_AMBIGUOUS)
        );
    }

    #[tokio::test]
    async fn outbox_broadcast_disabled_never_sends() {
        let (store, rpc, target_policy, gp, row, plan, signed, signer) = happy_setup().await;
        let clock = Arc::new(MockClock::new(1000));
        let allowed = vec![84532u64];
        let firewall = BroadcastPolicyFirewall {
            store: store.as_ref(),
            target_policy: &target_policy,
            gas_policy: &gp,
            broadcast_config: BroadcastFirewallConfig {
                broadcast_enabled: false,
                pre_send_hash_probe: false,
            },
            configured_chain_id: 84532,
            deployment_id: 1,
            simulation_max_age_ms: 3_600_000,
            allowed_broadcast_chain_ids: &allowed,
            now_ms: 1_000,
            rpc: None,
        };
        let outbox = BroadcastOutbox {
            store: store.clone(),
            rpc: rpc.clone(),
            clock,
            deployment_id: 1,
        };
        let outcome = outbox
            .submit(
                row,
                plan,
                signed,
                signer,
                42,
                1_000_000,
                U256::from(2_000_000_000u64),
                U256::from(500_000_000u64),
                make_readiness_ready(),
                &firewall,
            )
            .await
            .expect("submit");
        assert_eq!(outcome.phase, BroadcastPhase::ManualInterventionRequired);
        assert_eq!(
            outcome.failure_class.as_deref(),
            Some(failure_class::FIREWALL_REJECTED)
        );
        // Zero writes.
        assert!(rpc.recorded_write_methods().is_empty());
    }

    #[tokio::test]
    async fn outbox_nonce_too_low_escalates_manual() {
        let (store, rpc, target_policy, gp, row, plan, signed, signer) = happy_setup().await;
        rpc.push(Ok(SendOutcome::NonceTooLow));
        let clock = Arc::new(MockClock::new(1000));
        let allowed = vec![84532u64];
        let firewall = BroadcastPolicyFirewall {
            store: store.as_ref(),
            target_policy: &target_policy,
            gas_policy: &gp,
            broadcast_config: BroadcastFirewallConfig {
                broadcast_enabled: true,
                pre_send_hash_probe: false,
            },
            configured_chain_id: 84532,
            deployment_id: 1,
            simulation_max_age_ms: 3_600_000,
            allowed_broadcast_chain_ids: &allowed,
            now_ms: 1_000,
            rpc: None,
        };
        let outbox = BroadcastOutbox {
            store: store.clone(),
            rpc: rpc.clone(),
            clock,
            deployment_id: 1,
        };
        let outcome = outbox
            .submit(
                row,
                plan,
                signed,
                signer,
                42,
                1_000_000,
                U256::from(2_000_000_000u64),
                U256::from(500_000_000u64),
                make_readiness_ready(),
                &firewall,
            )
            .await
            .expect("submit");
        assert_eq!(outcome.phase, BroadcastPhase::ManualInterventionRequired);
        assert_eq!(
            outcome.failure_class.as_deref(),
            Some(failure_class::NONCE_CONFLICT)
        );
    }

    #[tokio::test]
    async fn outbox_already_known_is_idempotent_submitted() {
        let (store, rpc, target_policy, gp, row, plan, signed, signer) = happy_setup().await;
        let env = serialize_signed_execution(
            &plan,
            &signed,
            42,
            1_000_000,
            U256::from(2_000_000_000u64),
            U256::from(500_000_000u64),
        )
        .unwrap();
        rpc.push(Ok(SendOutcome::AlreadyKnown {
            provider_tx_hash: env.envelope_hash,
        }));
        let clock = Arc::new(MockClock::new(1000));
        let allowed = vec![84532u64];
        let firewall = BroadcastPolicyFirewall {
            store: store.as_ref(),
            target_policy: &target_policy,
            gas_policy: &gp,
            broadcast_config: BroadcastFirewallConfig {
                broadcast_enabled: true,
                pre_send_hash_probe: false,
            },
            configured_chain_id: 84532,
            deployment_id: 1,
            simulation_max_age_ms: 3_600_000,
            allowed_broadcast_chain_ids: &allowed,
            now_ms: 1_000,
            rpc: None,
        };
        let outbox = BroadcastOutbox {
            store: store.clone(),
            rpc: rpc.clone(),
            clock,
            deployment_id: 1,
        };
        let outcome = outbox
            .submit(
                row,
                plan,
                signed,
                signer,
                42,
                1_000_000,
                U256::from(2_000_000_000u64),
                U256::from(500_000_000u64),
                make_readiness_ready(),
                &firewall,
            )
            .await
            .expect("submit");
        assert_eq!(outcome.phase, BroadcastPhase::Submitted);
        assert_eq!(
            outcome.provider_classification.as_deref(),
            Some(provider_classification::ALREADY_KNOWN)
        );
    }
}
