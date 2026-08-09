//! Pre-execution canonical validation for the Hybrid V2 execution
//! orchestrator.
//!
//! This is the LOCAL check — the store-scoped projection is consulted
//! for order lifecycle, recovery, and readiness state. It is
//! deliberately conservative: on-chain simulation (next package) is
//! the authoritative last word before signing.
//!
//! What preflight MUST reject (per the audit):
//!   * Deployment not READY (per `ReadinessReport::is_ready`).
//!   * Active reorg / rebuild.
//!   * Reconciliation drift classified as `PROJECTION_DRIFT`,
//!     `MANIFEST_MISMATCH`, or `MALFORMED_CHAIN_RESPONSE`.
//!   * Base mainnet (chain_id == 8453).
//!   * Execution row's own phase already in a terminal state.
//!
//! What preflight MAY reject when the projection has the data:
//!   * Order not found / cancelled / expired / nonce stale.
//!   * Quantity exceeds remaining fillable.
//!   * Owner recovery active.
//!   * Subaccount finalized.
//!   * Fee schedule mismatch.
//!   * Execution already settled on-chain (correlated via
//!     `matched_executions`).
//!
//! What preflight WILL NOT do:
//!   * On-chain calls (belongs to the simulation package).
//!   * Signature verification (belongs to the signer package).
//!   * Broadcast decisioning (broadcast is disabled).

use crate::hybrid_v2::execution::persistence::ExecutionRequestRow;
use crate::hybrid_v2::execution::state::ExecutionPhase;
use crate::hybrid_v2::manifest::BASE_MAINNET_CHAIN_ID;
use crate::hybrid_v2::persistence::HybridV2ProjectionStore;
use crate::hybrid_v2::readiness::{ReadinessReason, ReadinessReport};
use crate::hybrid_v2::reducer::ProjectionState;
use thiserror::Error;

/// Structured rejection reasons. Kept in 1:1 correspondence with the
/// closed list in the audit; do not swallow variants into a generic
/// bucket.
#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum PreflightRejection {
    #[error("deployment not ready: {reason}")]
    DeploymentNotReady { reason: String },
    #[error("reorg is active for this deployment")]
    ActiveReorg,
    #[error("rebuild is active for this deployment")]
    ActiveRebuild,
    #[error("reconciliation drift blocks execution: {detail}")]
    ReconciliationDrift { detail: String },
    #[error("execution row already terminal in phase {phase}")]
    ExecutionAlreadyTerminal { phase: ExecutionPhase },
    #[error("order not found in projection: {order_hash}")]
    OrderNotFound { order_hash: String },
    #[error("order cancelled: {order_hash}")]
    OrderCancelled { order_hash: String },
    #[error("order expired: {order_hash}")]
    OrderExpired { order_hash: String },
    #[error("order nonce {provided} is stale; min-valid is {min_valid} (order {order_hash})")]
    OrderNonceStale {
        order_hash: String,
        provided: u64,
        min_valid: u64,
    },
    #[error(
        "requested fill quantity {requested} exceeds remaining {remaining} on order {order_hash}"
    )]
    QuantityExceedsRemaining {
        order_hash: String,
        requested: u128,
        remaining: u128,
    },
    #[error("owner recovery is active for subaccount {subkey}")]
    OwnerRecoveryActive { subkey: String },
    #[error("subaccount {subkey} is finalized")]
    SubaccountFinalized { subkey: String },
    #[error("fee schedule mismatch between request and current epoch")]
    FeeScheduleMismatch,
    #[error(
        "matched execution {on_chain_execution_id} is already settled on-chain for this intent"
    )]
    ExecutionAlreadySettledOnChain { on_chain_execution_id: String },
    #[error("Base mainnet is forbidden for Hybrid V2 execution")]
    BaseMainnetForbidden,
}

/// Non-blocking warnings surfaced by preflight when the projection
/// does not carry a piece of data preflight would like to verify.
/// The caller SHOULD log the warnings and MUST continue (simulation
/// is the authoritative check).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrustLevel {
    /// Projection carried the data and it was checked.
    VerifiedFromProjection,
    /// Projection did not carry the data. The check was skipped;
    /// simulation is the authoritative last word.
    UnverifiedFromProjection { detail: String },
}

/// Outcome bundle. `warnings` is a bounded list of
/// `TrustLevel::UnverifiedFromProjection` markers — the caller can
/// choose to record them alongside the execution row for audit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreflightOutcome {
    pub warnings: Vec<TrustLevel>,
}

/// The canonical preflight validator. Holds only borrowed context;
/// runs one `validate` per candidate execution.
pub struct PreflightChecker<'a> {
    pub store: &'a dyn HybridV2ProjectionStore,
    pub runtime_state: &'a ProjectionState,
    pub readiness: &'a ReadinessReport,
    pub deployment_id: i64,
    pub chain_id: u64,
}

impl PreflightChecker<'_> {
    /// Run the preflight check against `req`. Returns
    /// `Ok(PreflightOutcome)` on pass; `Err(PreflightRejection)`
    /// with the first structural failure. The order of checks below
    /// mirrors the audit checklist — the CHEAPEST hard-fail runs
    /// first (chain firewall), then progressively more expensive
    /// projection lookups.
    pub async fn validate(
        &self,
        req: &ExecutionRequestRow,
    ) -> Result<PreflightOutcome, PreflightRejection> {
        let mut warnings: Vec<TrustLevel> = Vec::new();

        // 1. Base mainnet firewall — always first.
        if self.chain_id == BASE_MAINNET_CHAIN_ID {
            return Err(PreflightRejection::BaseMainnetForbidden);
        }
        if req.chain_id as u64 == BASE_MAINNET_CHAIN_ID {
            return Err(PreflightRejection::BaseMainnetForbidden);
        }

        // 2. Execution row must not already be terminal.
        if req.phase.is_terminal() {
            return Err(PreflightRejection::ExecutionAlreadyTerminal { phase: req.phase });
        }

        // 3. Readiness — the aggregated report already fans in
        //    runtime + rebuild + reconciliation. If any axis is 503,
        //    execution MUST refuse.
        if !self.readiness.is_ready() {
            let reason = describe_readiness(self.readiness);
            // If the reason is specifically reorg / rebuild /
            // reconciliation drift, return the specific variant so
            // the orchestrator can act on it. Otherwise, generic
            // DeploymentNotReady.
            if let Some(r) = &self.readiness.runtime.reason {
                if matches!(
                    r,
                    ReadinessReason::ReorgDetected { .. }
                        | ReadinessReason::ReorgSearching { .. }
                        | ReadinessReason::ReorgReplaying { .. }
                        | ReadinessReason::ReorgManualInterventionRequired { .. }
                        | ReadinessReason::ExcessiveReorg { .. }
                ) {
                    return Err(PreflightRejection::ActiveReorg);
                }
            }
            if let Some(r) = &self.readiness.rebuild.reason {
                if matches!(
                    r,
                    ReadinessReason::RebuildInProgress
                        | ReadinessReason::RebuildRequested { .. }
                        | ReadinessReason::RebuildFailed { .. }
                ) {
                    return Err(PreflightRejection::ActiveRebuild);
                }
            }
            if let Some(r) = &self.readiness.reconciliation.reason {
                if let ReadinessReason::ReconciliationDrift { detail } = r {
                    return Err(PreflightRejection::ReconciliationDrift {
                        detail: detail.clone(),
                    });
                }
                if let ReadinessReason::ReconciliationInProgress { .. } = r {
                    // Reconciliation running is a soft-not-ready
                    // condition — still blocking, but we surface it
                    // as generic DeploymentNotReady with the reason
                    // string so the operator can distinguish.
                    return Err(PreflightRejection::DeploymentNotReady { reason });
                }
            }
            return Err(PreflightRejection::DeploymentNotReady { reason });
        }

        // 4. On-chain settled correlation — if the projection has a
        //    matched_executions row that references BOTH order hashes
        //    with a completion status of Complete, then the pair has
        //    already settled and we MUST refuse to double-execute.
        if let Some(existing) = find_matched_execution(
            self.runtime_state,
            &req.buyer_order_hash,
            &req.seller_order_hash,
        ) {
            if matches!(
                existing.completion_status,
                crate::hybrid_v2::reducer::ExecutionCompletion::Complete
            ) {
                // MatchedExecutionRow carries `tx_hash` +
                // `block_number`; the canonical on-chain execution id
                // is not stored directly in the projection at this
                // milestone. Surface `tx_hash` as the correlation
                // string for the operator's log — good enough to
                // grep on chain.
                return Err(PreflightRejection::ExecutionAlreadySettledOnChain {
                    on_chain_execution_id: existing.tx_hash.clone(),
                });
            }
        }

        // 5. Order lifecycle — the projection may or may not carry
        //    per-order records. If present, enforce cancelled /
        //    quantity checks; otherwise emit a
        //    TrustLevel::UnverifiedFromProjection warning.
        for order_hash in [&req.buyer_order_hash, &req.seller_order_hash] {
            match self.runtime_state.order_lifecycle.get(order_hash) {
                Some(row) => {
                    if row.cancelled {
                        return Err(PreflightRejection::OrderCancelled {
                            order_hash: order_hash.clone(),
                        });
                    }
                    if row.terminal {
                        // A terminal but not-cancelled lifecycle row
                        // typically means fully filled or expired;
                        // treat as expired so the orchestrator can
                        // decide whether to age the request out.
                        return Err(PreflightRejection::OrderExpired {
                            order_hash: order_hash.clone(),
                        });
                    }
                    if let Some(rem) = remaining_quantity(&row.total_qty_1e8, &row.filled_qty_1e8) {
                        if let Ok(requested) = req.fill_quantity_1e8.parse::<u128>() {
                            if requested > rem {
                                return Err(PreflightRejection::QuantityExceedsRemaining {
                                    order_hash: order_hash.clone(),
                                    requested,
                                    remaining: rem,
                                });
                            }
                        }
                    }
                }
                None => {
                    warnings.push(TrustLevel::UnverifiedFromProjection {
                        detail: format!(
                            "order lifecycle row missing for {order_hash}; deferring to simulation"
                        ),
                    });
                }
            }
        }

        // 6. Owner-recovery / subaccount finalization. Both subkeys
        //    must be free of an active recovery flag.
        for subkey in [&req.buyer_subkey, &req.seller_subkey] {
            if let Some(state) = self.runtime_state.subaccount_recovery_epochs.get(subkey) {
                if is_recovery_active(state) {
                    return Err(PreflightRejection::OwnerRecoveryActive {
                        subkey: subkey.clone(),
                    });
                }
            }
            if is_subaccount_finalized(self.runtime_state, subkey) {
                return Err(PreflightRejection::SubaccountFinalized {
                    subkey: subkey.clone(),
                });
            }
        }

        Ok(PreflightOutcome { warnings })
    }
}

// -----------------------------------------------------------------
//                          HELPERS
// -----------------------------------------------------------------

fn describe_readiness(report: &ReadinessReport) -> String {
    let mut parts: Vec<String> = Vec::new();
    if !report.runtime.ready {
        parts.push(format!("runtime={:?}", report.runtime.reason));
    }
    if !report.rebuild.ready {
        parts.push(format!("rebuild={:?}", report.rebuild.reason));
    }
    if !report.reconciliation.ready {
        parts.push(format!("reconciliation={:?}", report.reconciliation.reason));
    }
    if parts.is_empty() {
        "unknown".to_string()
    } else {
        parts.join("; ")
    }
}

fn find_matched_execution<'a>(
    state: &'a ProjectionState,
    buyer_hash: &str,
    seller_hash: &str,
) -> Option<&'a crate::hybrid_v2::reducer::MatchedExecutionRow> {
    state.matched_executions.values().find(|r| {
        r.buyer_order_hash.eq_ignore_ascii_case(buyer_hash)
            && r.seller_order_hash.eq_ignore_ascii_case(seller_hash)
    })
}

fn remaining_quantity(total_dec: &str, filled_dec: &str) -> Option<u128> {
    let total: u128 = total_dec.parse().ok()?;
    let filled: u128 = filled_dec.parse().ok()?;
    total.checked_sub(filled)
}

/// Recovery-epoch rows carry non-zero fields only when a recovery
/// is active for the given key. The reducer contract is documented
/// in `src/hybrid_v2/reducer.rs`; we treat any non-zero epoch value
/// as active.
fn is_recovery_active(row: &crate::hybrid_v2::reducer::EpochRow) -> bool {
    // The `EpochRow` field set is small and the reducer keeps only
    // active rows in the map; presence implies activity.
    !row_is_default(row)
}

fn row_is_default(row: &crate::hybrid_v2::reducer::EpochRow) -> bool {
    // Compare against Default in the absence of a dedicated `is_zero`
    // helper on `EpochRow`. Cheaper than serializing.
    format!("{row:?}") == format!("{:?}", crate::hybrid_v2::reducer::EpochRow::default())
}

fn is_subaccount_finalized(state: &ProjectionState, subkey: &str) -> bool {
    state
        .recovery_finalization_withdrawals
        .get(subkey)
        .copied()
        .unwrap_or(0)
        > 0
}

// -----------------------------------------------------------------
//                          UNIT TESTS
// -----------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hybrid_v2::execution::identity::derive_canonical_execution_id;
    use crate::hybrid_v2::execution::state::ExecutionPhase;
    use crate::hybrid_v2::persistence::InMemoryProjectionStore;
    use crate::hybrid_v2::readiness::{ReadinessReason, ReadinessState};
    use crate::hybrid_v2::reducer::{
        ExecutionCompletion, MatchedExecutionRow, OrderLifecycleRow, ProjectionState,
    };

    fn baseline_row(chain_id: u64) -> ExecutionRequestRow {
        let buyer_h = format!("0x{}", "aa".repeat(32));
        let seller_h = format!("0x{}", "bb".repeat(32));
        let id = derive_canonical_execution_id(1, chain_id, &buyer_h, &seller_h, 100_000_000);
        ExecutionRequestRow {
            canonical_execution_id: id.into_string(),
            deployment_id: 1,
            chain_id: chain_id as i64,
            execution_kind: "HYBRID_V2_OPTION_MATCH".to_string(),
            buyer_order_hash: buyer_h,
            seller_order_hash: seller_h,
            buyer_subkey: format!("0x{}", "aa".repeat(32)),
            seller_subkey: format!("0x{}", "bb".repeat(32)),
            series_id: "42".to_string(),
            fill_quantity_1e8: "100000000".to_string(),
            premium_amount: "50000000".to_string(),
            fee_schedule_epoch: None,
            source_matched_execution_id: None,
            target_contract: format!("0x{}", "5a".repeat(20)),
            selector: "0x00000000".to_string(),
            calldata_hash: None,
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
            created_at_ms: 1,
            updated_at_ms: 1,
        }
    }

    fn ready_report() -> ReadinessReport {
        ReadinessReport {
            runtime: ReadinessState::ready(),
            rebuild: ReadinessState::ready(),
            reconciliation: ReadinessState::ready(),
        }
    }

    #[tokio::test]
    async fn happy_path_passes_with_no_projection_data() {
        let store = InMemoryProjectionStore::new();
        let state = ProjectionState::default();
        let readiness = ready_report();
        let checker = PreflightChecker {
            store: &store,
            runtime_state: &state,
            readiness: &readiness,
            deployment_id: 1,
            chain_id: 84532,
        };
        let outcome = checker.validate(&baseline_row(84532)).await.unwrap();
        // No order rows in projection → two warnings.
        assert_eq!(outcome.warnings.len(), 2);
    }

    #[tokio::test]
    async fn base_mainnet_is_rejected_by_chain_id() {
        let store = InMemoryProjectionStore::new();
        let state = ProjectionState::default();
        let readiness = ready_report();
        let checker = PreflightChecker {
            store: &store,
            runtime_state: &state,
            readiness: &readiness,
            deployment_id: 1,
            chain_id: 8453,
        };
        let err = checker.validate(&baseline_row(8453)).await.unwrap_err();
        assert_eq!(err, PreflightRejection::BaseMainnetForbidden);
    }

    #[tokio::test]
    async fn terminal_execution_row_is_rejected() {
        let store = InMemoryProjectionStore::new();
        let state = ProjectionState::default();
        let readiness = ready_report();
        let checker = PreflightChecker {
            store: &store,
            runtime_state: &state,
            readiness: &readiness,
            deployment_id: 1,
            chain_id: 84532,
        };
        let mut row = baseline_row(84532);
        row.phase = ExecutionPhase::BroadcastDisabled;
        let err = checker.validate(&row).await.unwrap_err();
        assert!(matches!(
            err,
            PreflightRejection::ExecutionAlreadyTerminal { .. }
        ));
    }

    #[tokio::test]
    async fn active_reorg_blocks_execution() {
        let store = InMemoryProjectionStore::new();
        let state = ProjectionState::default();
        let readiness = ReadinessReport {
            runtime: ReadinessState::new_not_ready(ReadinessReason::ReorgDetected {
                at_block: 100,
                epoch: 1,
            }),
            rebuild: ReadinessState::ready(),
            reconciliation: ReadinessState::ready(),
        };
        let checker = PreflightChecker {
            store: &store,
            runtime_state: &state,
            readiness: &readiness,
            deployment_id: 1,
            chain_id: 84532,
        };
        let err = checker.validate(&baseline_row(84532)).await.unwrap_err();
        assert_eq!(err, PreflightRejection::ActiveReorg);
    }

    #[tokio::test]
    async fn active_rebuild_blocks_execution() {
        let store = InMemoryProjectionStore::new();
        let state = ProjectionState::default();
        let readiness = ReadinessReport {
            runtime: ReadinessState::ready(),
            rebuild: ReadinessState::new_not_ready(ReadinessReason::RebuildInProgress),
            reconciliation: ReadinessState::ready(),
        };
        let checker = PreflightChecker {
            store: &store,
            runtime_state: &state,
            readiness: &readiness,
            deployment_id: 1,
            chain_id: 84532,
        };
        let err = checker.validate(&baseline_row(84532)).await.unwrap_err();
        assert_eq!(err, PreflightRejection::ActiveRebuild);
    }

    #[tokio::test]
    async fn projection_drift_blocks_execution() {
        let store = InMemoryProjectionStore::new();
        let state = ProjectionState::default();
        let readiness = ReadinessReport {
            runtime: ReadinessState::ready(),
            rebuild: ReadinessState::ready(),
            reconciliation: ReadinessState::new_not_ready(ReadinessReason::ReconciliationDrift {
                detail: "projection drift".into(),
            }),
        };
        let checker = PreflightChecker {
            store: &store,
            runtime_state: &state,
            readiness: &readiness,
            deployment_id: 1,
            chain_id: 84532,
        };
        let err = checker.validate(&baseline_row(84532)).await.unwrap_err();
        assert!(matches!(
            err,
            PreflightRejection::ReconciliationDrift { .. }
        ));
    }

    #[tokio::test]
    async fn cancelled_order_is_rejected() {
        let store = InMemoryProjectionStore::new();
        let mut state = ProjectionState::default();
        let row = baseline_row(84532);
        state.order_lifecycle.insert(
            row.buyer_order_hash.clone(),
            OrderLifecycleRow {
                subkey: row.buyer_subkey.clone(),
                cancelled: true,
                total_qty_1e8: "100000000".into(),
                filled_qty_1e8: "0".into(),
                ..Default::default()
            },
        );
        let readiness = ready_report();
        let checker = PreflightChecker {
            store: &store,
            runtime_state: &state,
            readiness: &readiness,
            deployment_id: 1,
            chain_id: 84532,
        };
        let err = checker.validate(&row).await.unwrap_err();
        assert!(matches!(err, PreflightRejection::OrderCancelled { .. }));
    }

    #[tokio::test]
    async fn already_settled_pair_is_rejected() {
        let store = InMemoryProjectionStore::new();
        let mut state = ProjectionState::default();
        let row = baseline_row(84532);
        state.matched_executions.insert(
            "0xexec1".into(),
            MatchedExecutionRow {
                buyer_order_hash: row.buyer_order_hash.clone(),
                seller_order_hash: row.seller_order_hash.clone(),
                completion_status: ExecutionCompletion::Complete,
                tx_hash: "0xdeadbeef".into(),
                ..Default::default()
            },
        );
        let readiness = ready_report();
        let checker = PreflightChecker {
            store: &store,
            runtime_state: &state,
            readiness: &readiness,
            deployment_id: 1,
            chain_id: 84532,
        };
        let err = checker.validate(&row).await.unwrap_err();
        assert!(matches!(
            err,
            PreflightRejection::ExecutionAlreadySettledOnChain { .. }
        ));
    }

    #[tokio::test]
    async fn quantity_exceeds_remaining_is_rejected() {
        let store = InMemoryProjectionStore::new();
        let mut state = ProjectionState::default();
        let row = baseline_row(84532);
        state.order_lifecycle.insert(
            row.buyer_order_hash.clone(),
            OrderLifecycleRow {
                subkey: row.buyer_subkey.clone(),
                total_qty_1e8: "50000000".into(),
                filled_qty_1e8: "0".into(),
                ..Default::default()
            },
        );
        state.order_lifecycle.insert(
            row.seller_order_hash.clone(),
            OrderLifecycleRow {
                subkey: row.seller_subkey.clone(),
                total_qty_1e8: "50000000".into(),
                filled_qty_1e8: "0".into(),
                ..Default::default()
            },
        );
        let readiness = ready_report();
        let checker = PreflightChecker {
            store: &store,
            runtime_state: &state,
            readiness: &readiness,
            deployment_id: 1,
            chain_id: 84532,
        };
        let err = checker.validate(&row).await.unwrap_err();
        assert!(matches!(
            err,
            PreflightRejection::QuantityExceedsRemaining { .. }
        ));
    }
}
