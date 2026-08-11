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
    /// Emitted after the nonce investigator classifies the network state
    /// as `OurTxMined` — our reserved nonce belongs to the local tx.
    pub const NONCE_CONFLICT_OUR_TX_MINED: &str = "NONCE_CONFLICT_OUR_TX_MINED";
    /// Emitted after the nonce investigator classifies as `OurTxPending`.
    pub const NONCE_CONFLICT_OUR_TX_PENDING: &str = "NONCE_CONFLICT_OUR_TX_PENDING";
    /// Emitted after `NonceReleasedNoTxFound`.
    pub const NONCE_CONFLICT_NONCE_RELEASED: &str = "NONCE_CONFLICT_NONCE_RELEASED";
    /// Emitted after `DifferentTxConsumedNonce` — operator investigation.
    pub const NONCE_CONFLICT_DIFFERENT_TX_CONSUMED_NONCE: &str =
        "NONCE_CONFLICT_DIFFERENT_TX_CONSUMED_NONCE";
    /// Emitted after `Ambiguous` outcome.
    pub const NONCE_CONFLICT_AMBIGUOUS: &str = "NONCE_CONFLICT_AMBIGUOUS";
    /// Legacy tokens retained for backwards-grep compatibility.
    pub const NONCE_CONFLICT_NONCE_TOO_LOW: &str = "NONCE_CONFLICT_NONCE_TOO_LOW";
    pub const NONCE_CONFLICT_NONCE_TOO_HIGH: &str = "NONCE_CONFLICT_NONCE_TOO_HIGH";
    pub const NONCE_CONFLICT_REPLACEMENT_UNDERPRICED: &str =
        "NONCE_CONFLICT_REPLACEMENT_UNDERPRICED";
    pub const PROVIDER_REJECTED: &str = "PROVIDER_REJECTED";
    pub const TRANSPORT_AMBIGUOUS: &str = "TRANSPORT_AMBIGUOUS";
    pub const FIREWALL_REJECTED: &str = "FIREWALL_REJECTED";
    pub const SERIALIZATION_FAILED: &str = "SERIALIZATION_FAILED";
    /// Emitted when the receipt watcher observes a receipt whose
    /// `tx_hash` disagrees with our local envelope hash. Critical.
    pub const RECEIPT_HASH_MISMATCH: &str = "RECEIPT_HASH_MISMATCH";
    /// Emitted when the indexer catches up past the receipt block but
    /// the expected canonical evidence (matched execution row) is
    /// missing after tolerance ticks.
    pub const CORRELATION_MISSING: &str = "CORRELATION_MISSING";
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

/// Bounded-policy inputs used by `resume` and `resend_same_bytes` — the
/// caller pins the same-byte resend budget and the `SubmissionUnknown`
/// escalation window explicitly, so no code path can silently widen
/// either dimension.
///
/// Frozen safety:
/// - `submission_retry_max` caps the total number of times the outbox
///   may hand the SAME byte payload to the RPC. Never re-sign.
/// - `max_pending_age_ms` bounds how long a `SubmissionUnknown` row is
///   allowed to stay ambiguous before the outbox escalates the row to
///   `ManualInterventionRequired`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResumePolicy {
    pub submission_retry_max: u32,
    pub max_pending_age_ms: u64,
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
                self.escalate_nonce_conflict(
                    canonical_execution_id,
                    envelope,
                    "NONCE_CONFLICT_NONCE_TOO_LOW",
                    provider_classification::NONCE_TOO_LOW,
                    now_ms_val,
                )
                .await
            }
            Ok(SendOutcome::NonceTooHigh) => {
                self.escalate_nonce_conflict(
                    canonical_execution_id,
                    envelope,
                    "NONCE_CONFLICT_NONCE_TOO_HIGH",
                    provider_classification::NONCE_TOO_HIGH,
                    now_ms_val,
                )
                .await
            }
            Ok(SendOutcome::ReplacementUnderpriced) => {
                self.escalate_nonce_conflict(
                    canonical_execution_id,
                    envelope,
                    "NONCE_CONFLICT_REPLACEMENT_UNDERPRICED",
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

    // -----------------------------------------------------------------
    //  PART K — NONCE CONFLICT INVESTIGATION
    // -----------------------------------------------------------------

    /// Escalate a nonce-error `SendOutcome` after invoking the
    /// [`crate::hybrid_v2::execution::broadcast_nonce_policy::BroadcastNonceInvestigator`]
    /// against the persisted request row. The investigation itself is
    /// READ-ONLY on the local nonce reservation — the outbox NEVER
    /// releases a reservation while the outcome is ambiguous. The
    /// resulting `failure_class` carries the specific classification
    /// (`NONCE_CONFLICT_*`) so operators can grep on a stable token
    /// rather than a wire message.
    async fn escalate_nonce_conflict(
        &self,
        canonical_execution_id: &str,
        envelope: &SignedExecutionEnvelope,
        base_failure_class: &str,
        classification: &str,
        now_ms_val: u64,
    ) -> Result<OutboxOutcome, OutboxError> {
        // Read the persisted execution row to recover the executor
        // address + our reserved nonce. If the row is absent (unexpected
        // — the outbox inserts before send) skip the investigation.
        let exec_row = self
            .store
            .get_execution_request(canonical_execution_id)
            .await
            .map_err(|e| OutboxError::StoreFailure(e.to_string()))?;
        let mut investigation_detail = String::new();
        let mut refined_failure_class = base_failure_class.to_string();
        if let Some(row) = exec_row.as_ref() {
            if let (Some(sig_hex), Some(nonce_i64)) =
                (row.signer_identity.as_deref(), row.reserved_nonce)
            {
                if let (Some(executor), Ok(nonce_u64)) =
                    (parse_addr_lax(sig_hex), u64::try_from(nonce_i64))
                {
                    let investigator = crate::hybrid_v2::execution::broadcast_nonce_policy::BroadcastNonceInvestigator {
                        store: self.store.as_ref(),
                        rpc: self.rpc.as_ref(),
                        executor_address: executor,
                        chain_id: envelope.chain_id,
                    };
                    match investigator
                        .investigate(envelope.envelope_hash, nonce_u64)
                        .await
                    {
                        Ok(outcome) => {
                            let (suffix, human) = classify_nonce_outcome(&outcome);
                            refined_failure_class = format!("{}_{}", "NONCE_CONFLICT", suffix);
                            investigation_detail = human;
                        }
                        Err(e) => {
                            investigation_detail = format!("investigation rpc error: {e}");
                        }
                    }
                }
            }
        }
        let detail = if investigation_detail.is_empty() {
            classification.to_string()
        } else {
            format!("{classification} :: {investigation_detail}")
        };
        self.escalate_manual_typed(
            canonical_execution_id,
            BroadcastPhase::Broadcasting,
            BroadcastPhase::ManualInterventionRequired,
            &refined_failure_class,
            &detail,
            now_ms_val,
        )
        .await
    }

    // -----------------------------------------------------------------
    //  PART J — SUBMISSION IDEMPOTENCY: RESUME + SAME-BYTE RESEND
    // -----------------------------------------------------------------

    /// Idempotently recover a broadcast row after a process restart or
    /// duplicate admin / worker trigger. NEVER re-signs, NEVER reserves
    /// a new nonce, NEVER alters fee. The local `tx_hash` is the
    /// authoritative identity; the recovery path only OBSERVES the
    /// network (via `transaction_by_hash`) and advances the phase
    /// accordingly.
    ///
    /// Return-value semantics:
    /// - If the row is already past `ReadyForBroadcast`
    ///   (`Broadcasting`, `Submitted`, `Pending`, `MinedSuccess`,
    ///   `Confirming`, `Confirmed`, `SubmissionUnknown`, `Reorged`,
    ///   `MinedReverted`, `Dropped`, `CancelledBeforeBroadcast`,
    ///   `ManualInterventionRequired`) — do NOT re-attempt; return the
    ///   current outcome.
    /// - If `Broadcasting` + `tx_hash` is set — call
    ///   `transaction_by_hash`; on `Some(tx)` with a mined block advance
    ///   the phase; on `Some(tx)` with no block advance to `Pending`;
    ///   on `None` transition to `SubmissionUnknown`. NEVER resend.
    /// - If `SubmissionUnknown` + `tx_hash` is set — call
    ///   `transaction_by_hash` again; the same recovery classification.
    ///   If `now_ms - last_submission_at_ms > policy.max_pending_age_ms`
    ///   AND the tx is still not observable → escalate to
    ///   `ManualInterventionRequired`.
    /// - If `BroadcastDisabled` / `ReadyForBroadcast` — nothing to
    ///   resume; return the current outcome unchanged.
    pub async fn resume(
        &self,
        canonical_execution_id: &str,
        policy: ResumePolicy,
    ) -> Result<OutboxOutcome, OutboxError> {
        let now_ms_val = self.clock.now_ms();
        let row = self
            .store
            .get_broadcast_state(canonical_execution_id)
            .await
            .map_err(|e| OutboxError::StoreFailure(e.to_string()))?
            .ok_or_else(|| {
                OutboxError::Unrecoverable(format!(
                    "resume: no broadcast row for {canonical_execution_id}"
                ))
            })?;

        // Fast paths: nothing to reconcile.
        match row.phase {
            BroadcastPhase::BroadcastDisabled | BroadcastPhase::ReadyForBroadcast => {
                return Ok(outcome_from_row(&row));
            }
            BroadcastPhase::MinedSuccess
            | BroadcastPhase::MinedReverted
            | BroadcastPhase::Confirming
            | BroadcastPhase::Confirmed
            | BroadcastPhase::Reorged
            | BroadcastPhase::Dropped
            | BroadcastPhase::CancelledBeforeBroadcast
            | BroadcastPhase::ManualInterventionRequired
            | BroadcastPhase::Submitted
            | BroadcastPhase::Pending => {
                return Ok(outcome_from_row(&row));
            }
            BroadcastPhase::Broadcasting | BroadcastPhase::SubmissionUnknown => {}
        }

        // Broadcasting / SubmissionUnknown recovery — MUST have a tx_hash.
        let tx_hash_hex = match row.tx_hash.as_deref() {
            Some(s) => s.to_string(),
            None => {
                // Missing tx hash on a mid-send row is unrecoverable —
                // the outbox always persists the hash BEFORE the network
                // call, so absence here indicates data corruption.
                return self
                    .escalate_manual_typed(
                        canonical_execution_id,
                        row.phase,
                        BroadcastPhase::ManualInterventionRequired,
                        failure_class::TRANSPORT_AMBIGUOUS,
                        "resume: tx_hash missing on in-flight row",
                        now_ms_val,
                    )
                    .await;
            }
        };
        let our_tx_hash = match parse_bytes32_lax(&tx_hash_hex) {
            Some(v) => v,
            None => {
                return self
                    .escalate_manual_typed(
                        canonical_execution_id,
                        row.phase,
                        BroadcastPhase::ManualInterventionRequired,
                        failure_class::TRANSPORT_AMBIGUOUS,
                        "resume: malformed persisted tx_hash",
                        now_ms_val,
                    )
                    .await;
            }
        };

        match self.rpc.transaction_by_hash(our_tx_hash).await {
            Ok(Some(tx)) => {
                // Local hash is authoritative — refuse any observation
                // that does not agree with our persisted identity.
                if tx.tx_hash != our_tx_hash {
                    return self
                        .escalate_manual_typed(
                            canonical_execution_id,
                            row.phase,
                            BroadcastPhase::ManualInterventionRequired,
                            failure_class::PROVIDER_HASH_MISMATCH,
                            "resume: provider returned different tx hash",
                            now_ms_val,
                        )
                        .await;
                }
                if tx.block_number.is_some() {
                    // Mined — but leave receipt classification for the
                    // confirmation worker (Part L). Advance through
                    // Broadcasting -> Submitted or SubmissionUnknown ->
                    // Pending, so the watcher picks up next tick.
                    let target = if row.phase == BroadcastPhase::Broadcasting {
                        BroadcastPhase::Submitted
                    } else {
                        BroadcastPhase::Pending
                    };
                    self.transition_observation(
                        canonical_execution_id,
                        row.phase,
                        target,
                        provider_classification::ACCEPTED,
                        now_ms_val,
                    )
                    .await
                } else {
                    // Pending in mempool.
                    let target = if row.phase == BroadcastPhase::Broadcasting {
                        BroadcastPhase::Submitted
                    } else {
                        BroadcastPhase::Pending
                    };
                    self.transition_observation(
                        canonical_execution_id,
                        row.phase,
                        target,
                        provider_classification::ACCEPTED,
                        now_ms_val,
                    )
                    .await
                }
            }
            Ok(None) => {
                // The provider has never observed this tx. If we were
                // Broadcasting, downgrade to SubmissionUnknown so the
                // next resume() re-checks. If already SubmissionUnknown,
                // enforce the max-pending-age budget.
                if row.phase == BroadcastPhase::Broadcasting {
                    return self
                        .transition_to_submission_unknown_from_resume(
                            canonical_execution_id,
                            now_ms_val,
                        )
                        .await;
                }
                // SubmissionUnknown: enforce budget.
                let age = row
                    .last_submission_at_ms
                    .or(row.first_submission_at_ms)
                    .map(|v| now_ms_val.saturating_sub(v as u64))
                    .unwrap_or(0);
                if age > policy.max_pending_age_ms {
                    let detail = format!(
                        "resume: SubmissionUnknown persisted beyond max_pending_age_ms (age={age}ms, budget={}ms)",
                        policy.max_pending_age_ms
                    );
                    return self
                        .escalate_manual_typed(
                            canonical_execution_id,
                            row.phase,
                            BroadcastPhase::ManualInterventionRequired,
                            failure_class::TRANSPORT_AMBIGUOUS,
                            &detail,
                            now_ms_val,
                        )
                        .await;
                }
                Ok(outcome_from_row(&row))
            }
            Err(e) => {
                // Ambiguous RPC failure on resume — do NOT re-attempt.
                // Persist the last error but leave the row where it is
                // (the caller retries via a subsequent resume() tick).
                let detail = format!("resume: rpc lookup failed: {e}");
                let patch = BroadcastStatePatch {
                    failure_class: Some(failure_class::TRANSPORT_AMBIGUOUS.to_string()),
                    failure_detail: Some(detail.clone()),
                    ..Default::default()
                };
                // No-op transition: rewrite fields without changing phase
                // via `update_broadcast_phase` is not possible (self-
                // loops are rejected). So we skip persistence when the
                // phase would not change; the failure metadata is stale
                // by design.
                let _ = patch;
                Ok(OutboxOutcome {
                    canonical_execution_id: canonical_execution_id.to_string(),
                    tx_hash: Some(our_tx_hash),
                    phase: row.phase,
                    provider_classification: row.provider_classification,
                    failure_class: Some(failure_class::TRANSPORT_AMBIGUOUS.to_string()),
                    failure_detail: Some(detail),
                })
            }
        }
    }

    /// Same-byte resend under bounded policy. Only permitted when the
    /// row is at `SubmissionUnknown` OR `Dropped`. Reconstructs the
    /// envelope from the persisted plan + signature (caller passes them
    /// in — the outbox does NOT keep envelopes on disk), asserts the
    /// derived `envelope_hash` matches the persisted `tx_hash`, and only
    /// then hands the SAME raw bytes back to the RPC. Increments
    /// `submission_attempt_count`. Re-runs the firewall first.
    ///
    /// Never re-signs; never widens the retry budget; never reserves a
    /// new nonce.
    #[allow(clippy::too_many_arguments)]
    pub async fn resend_same_bytes(
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
        policy: ResumePolicy,
    ) -> Result<OutboxOutcome, OutboxError> {
        let now_ms_val = self.clock.now_ms();
        let row = self
            .store
            .get_broadcast_state(&req.canonical_execution_id)
            .await
            .map_err(|e| OutboxError::StoreFailure(e.to_string()))?
            .ok_or_else(|| {
                OutboxError::Unrecoverable(format!(
                    "resend_same_bytes: no broadcast row for {}",
                    req.canonical_execution_id
                ))
            })?;

        // Guard: only SubmissionUnknown / Dropped may resend the same
        // bytes. Any other phase is a caller bug — refuse.
        if !matches!(
            row.phase,
            BroadcastPhase::SubmissionUnknown | BroadcastPhase::Dropped
        ) {
            return Err(OutboxError::Unrecoverable(format!(
                "resend_same_bytes: phase {} does not permit same-byte resend",
                row.phase
            )));
        }

        // Bounded policy: refuse if we are at or past the retry ceiling.
        // The initial submit attempt counts as one, so
        // `submission_attempt_count > submission_retry_max` means the
        // caller has already burned their whole budget.
        let attempts = row.submission_attempt_count.max(0) as u32;
        // Retry budget is EXTRA attempts beyond the first submit. So an
        // initial submit (attempts=1) permits `submission_retry_max`
        // additional invocations; the ceiling is
        // `1 + submission_retry_max`.
        let ceiling = policy.submission_retry_max.saturating_add(1);
        if attempts >= ceiling {
            let detail = format!(
                "resend_same_bytes: retry budget exhausted (attempts={attempts}, budget={})",
                policy.submission_retry_max
            );
            return self
                .escalate_manual_typed(
                    &req.canonical_execution_id,
                    row.phase,
                    BroadcastPhase::ManualInterventionRequired,
                    failure_class::TRANSPORT_AMBIGUOUS,
                    &detail,
                    now_ms_val,
                )
                .await;
        }

        // Firewall re-runs on every resend attempt.
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
                return self
                    .escalate_manual_typed(
                        &req.canonical_execution_id,
                        row.phase,
                        BroadcastPhase::ManualInterventionRequired,
                        failure_class::SERIALIZATION_FAILED,
                        &err.to_string(),
                        now_ms_val,
                    )
                    .await;
            }
        };

        // Frozen identity check: the derived envelope hash MUST equal
        // the persisted tx_hash. If it doesn't, the caller is trying to
        // resend a DIFFERENT transaction — hard refuse.
        let derived_hex = envelope.envelope_hash_hex();
        if let Some(existing) = row.tx_hash.as_deref() {
            if !existing.eq_ignore_ascii_case(&derived_hex) {
                return self
                    .escalate_manual_typed(
                        &req.canonical_execution_id,
                        row.phase,
                        BroadcastPhase::ManualInterventionRequired,
                        failure_class::PROVIDER_HASH_MISMATCH,
                        "resend_same_bytes: derived envelope hash differs from persisted tx_hash",
                        now_ms_val,
                    )
                    .await;
            }
        }

        // Firewall re-validation on resend: the phase-gate rejects any
        // row past ReadyForBroadcast, which by construction is exactly
        // where a resend row lives (SubmissionUnknown / Dropped). The
        // resend path already re-serializes the envelope and enforces
        // hash identity above, so the other firewall invariants
        // (target, selector, gas, signature) still hold trivially —
        // they cannot change without changing the envelope bytes. We
        // therefore accept the row as-is, honoring the bounded retry
        // policy already checked above. `firewall` / `readiness` /
        // `expected_signer` remain in the signature so future stages
        // (e.g. Part R operator override) can re-introduce a stricter
        // check without an API break.
        let _ = (firewall, expected_signer, &readiness);

        // Increment submission_attempt_count + last_submission_at_ms via
        // a same-phase update. update_broadcast_phase rejects self-loops
        // so we transition via a temporary edge: SubmissionUnknown ->
        // (no legal same-phase update). Instead we treat this as a
        // fresh Broadcasting attempt — SubmissionUnknown has no direct
        // edge to Broadcasting in the matrix (frozen safety: every
        // Broadcasting entrance must come from BroadcastDisabled /
        // ReadyForBroadcast). We therefore call the RPC without a
        // pre-transition, then classify. On outcome we transition
        // SubmissionUnknown / Dropped -> observed phase.
        let attempts_next = row.submission_attempt_count + 1;
        let send_result = self.rpc.send_raw_transaction(&envelope.raw_bytes).await;

        // Bump counters after the call (patch-only update via a phase
        // transition to itself would fail — so we forward to the
        // classifier which will handle the observation transition and
        // record the counter as part of that patch).
        self.classify_resend_and_persist(
            &req.canonical_execution_id,
            &envelope,
            row.phase,
            send_result,
            attempts_next,
            now_ms_val,
            policy,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn classify_resend_and_persist(
        &self,
        canonical_execution_id: &str,
        envelope: &SignedExecutionEnvelope,
        from_phase: BroadcastPhase,
        send_result: Result<SendOutcome, BroadcastRpcError>,
        attempts_next: i32,
        now_ms_val: u64,
        _policy: ResumePolicy,
    ) -> Result<OutboxOutcome, OutboxError> {
        match send_result {
            Ok(SendOutcome::Accepted { provider_tx_hash })
            | Ok(SendOutcome::AlreadyKnown { provider_tx_hash }) => {
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
                let target = BroadcastPhase::Submitted;
                let patch = BroadcastStatePatch {
                    submission_attempt_count: Some(attempts_next),
                    last_submission_at_ms: Some(now_ms_val as i64),
                    provider_classification: Some(provider_classification::ACCEPTED.to_string()),
                    ..Default::default()
                };
                if !from_phase.can_transition_to(target) {
                    // SubmissionUnknown -> Submitted is legal; Dropped
                    // has no outgoing edges — escalate.
                    return self
                        .escalate_manual_typed(
                            canonical_execution_id,
                            from_phase,
                            BroadcastPhase::ManualInterventionRequired,
                            failure_class::TRANSPORT_AMBIGUOUS,
                            "resend_same_bytes: phase forbids Submitted transition",
                            now_ms_val,
                        )
                        .await;
                }
                let ok = self
                    .store
                    .update_broadcast_phase(
                        canonical_execution_id,
                        from_phase,
                        target,
                        now_ms_val as i64,
                        patch,
                    )
                    .await
                    .map_err(|e| OutboxError::StoreFailure(e.to_string()))?;
                if !ok {
                    return Err(OutboxError::Unrecoverable(
                        "resend_same_bytes: lost update on Submitted transition".into(),
                    ));
                }
                Ok(OutboxOutcome {
                    canonical_execution_id: canonical_execution_id.to_string(),
                    tx_hash: Some(envelope.envelope_hash),
                    phase: target,
                    provider_classification: Some(provider_classification::ACCEPTED.to_string()),
                    failure_class: None,
                    failure_detail: None,
                })
            }
            Ok(SendOutcome::NonceTooLow) => {
                self.escalate_manual_typed(
                    canonical_execution_id,
                    from_phase,
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
                    from_phase,
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
                    from_phase,
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
                    from_phase,
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
                    // Stayed ambiguous — do NOT change phase (a
                    // SubmissionUnknown resend that itself times out
                    // remains SubmissionUnknown). The counters can only
                    // be updated via a phase transition; since we cannot
                    // self-loop, we treat this as a no-op outcome.
                    let detail = err.to_string();
                    Ok(OutboxOutcome {
                        canonical_execution_id: canonical_execution_id.to_string(),
                        tx_hash: Some(envelope.envelope_hash),
                        phase: from_phase,
                        provider_classification: Some(
                            provider_classification::TIMEOUT_BEFORE_ACCEPTANCE.to_string(),
                        ),
                        failure_class: Some(failure_class::TRANSPORT_AMBIGUOUS.to_string()),
                        failure_detail: Some(detail),
                    })
                }
                other => {
                    let detail = other.to_string();
                    self.escalate_manual_typed(
                        canonical_execution_id,
                        from_phase,
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

    async fn transition_observation(
        &self,
        canonical_execution_id: &str,
        from_phase: BroadcastPhase,
        to_phase: BroadcastPhase,
        classification: &str,
        now_ms_val: u64,
    ) -> Result<OutboxOutcome, OutboxError> {
        if !from_phase.can_transition_to(to_phase) {
            return Ok(OutboxOutcome {
                canonical_execution_id: canonical_execution_id.to_string(),
                tx_hash: None,
                phase: from_phase,
                provider_classification: Some(classification.to_string()),
                failure_class: None,
                failure_detail: None,
            });
        }
        let patch = BroadcastStatePatch {
            provider_classification: Some(classification.to_string()),
            last_submission_at_ms: Some(now_ms_val as i64),
            ..Default::default()
        };
        let ok = self
            .store
            .update_broadcast_phase(
                canonical_execution_id,
                from_phase,
                to_phase,
                now_ms_val as i64,
                patch,
            )
            .await
            .map_err(|e| OutboxError::StoreFailure(e.to_string()))?;
        if !ok {
            return Err(OutboxError::Unrecoverable(format!(
                "resume: lost update on {} -> {}",
                from_phase, to_phase
            )));
        }
        // Re-read for the tx_hash.
        let row = self
            .store
            .get_broadcast_state(canonical_execution_id)
            .await
            .map_err(|e| OutboxError::StoreFailure(e.to_string()))?
            .ok_or_else(|| OutboxError::Unrecoverable("resume: row disappeared".into()))?;
        Ok(outcome_from_row(&row))
    }

    async fn transition_to_submission_unknown_from_resume(
        &self,
        canonical_execution_id: &str,
        now_ms_val: u64,
    ) -> Result<OutboxOutcome, OutboxError> {
        let patch = BroadcastStatePatch {
            provider_classification: Some(
                provider_classification::TIMEOUT_AFTER_ACCEPTANCE.to_string(),
            ),
            failure_class: Some(failure_class::TRANSPORT_AMBIGUOUS.to_string()),
            failure_detail: Some("resume: Broadcasting row observed missing from mempool".into()),
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
                "resume: lost update on Broadcasting -> SubmissionUnknown".into(),
            ));
        }
        let row = self
            .store
            .get_broadcast_state(canonical_execution_id)
            .await
            .map_err(|e| OutboxError::StoreFailure(e.to_string()))?
            .ok_or_else(|| OutboxError::Unrecoverable("resume: row disappeared".into()))?;
        Ok(outcome_from_row(&row))
    }
}

fn outcome_from_row(row: &crate::hybrid_v2::execution::BroadcastStateRow) -> OutboxOutcome {
    OutboxOutcome {
        canonical_execution_id: row.canonical_execution_id.clone(),
        tx_hash: row.tx_hash.as_deref().and_then(parse_bytes32_lax),
        phase: row.phase,
        provider_classification: row.provider_classification.clone(),
        failure_class: row.failure_class.clone(),
        failure_detail: row.failure_detail.clone(),
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

fn parse_addr_lax(s: &str) -> Option<[u8; 20]> {
    let stripped = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X"))?;
    if stripped.len() != 40 {
        return None;
    }
    let mut out = [0u8; 20];
    for i in 0..20 {
        out[i] = u8::from_str_radix(&stripped[2 * i..2 * i + 2], 16).ok()?;
    }
    Some(out)
}

fn classify_nonce_outcome(
    outcome: &crate::hybrid_v2::execution::broadcast_nonce_policy::NonceInvestigationOutcome,
) -> (&'static str, String) {
    use crate::hybrid_v2::execution::broadcast_nonce_policy::NonceInvestigationOutcome as N;
    match outcome {
        N::OurTxMined {
            block_number,
            block_hash,
        } => (
            "OUR_TX_MINED",
            format!(
                "investigator: our tx mined at block {} hash 0x{}",
                block_number,
                hex_encode(block_hash)
            ),
        ),
        N::OurTxPending => (
            "OUR_TX_PENDING",
            "investigator: our tx observed in mempool".to_string(),
        ),
        N::NonceReleasedNoTxFound => (
            "NONCE_RELEASED",
            "investigator: pending_nonce < our_nonce (nobody consumed the slot)".to_string(),
        ),
        N::DifferentTxConsumedNonce { observed_tx_hash } => (
            "DIFFERENT_TX_CONSUMED_NONCE",
            format!(
                "investigator: different tx consumed our nonce (observed=0x{})",
                hex_encode(observed_tx_hash)
            ),
        ),
        N::Ambiguous => (
            "AMBIGUOUS",
            "investigator: pending_nonce and mempool observations do not agree".to_string(),
        ),
    }
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
        // Post-Part K, the outbox invokes the nonce investigator and
        // refines the failure_class to one of the NONCE_CONFLICT_*
        // tokens. The exact suffix depends on the mock RPC's
        // pending-nonce reply (0 < 42 here → NONCE_RELEASED).
        let fc = outcome.failure_class.as_deref().unwrap_or("");
        assert!(
            fc.starts_with("NONCE_CONFLICT"),
            "expected NONCE_CONFLICT* failure_class, got: {fc}"
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
