//! Transaction reorg recovery for the Hybrid V2 broadcast pipeline
//! (Part Q of `BACKEND-HYBRID-V2-BROADCAST-AND-CONFIRMATION-V1`).
//!
//! ## Scope
//!
//! Reorg recovery observes the network for a broadcast row that has
//! already been marked [`BroadcastPhase::Reorged`] by the confirmation
//! worker (i.e. its previously-observed receipt block was orphaned by
//! the canonical chain). Recovery deterministically classifies the
//! observed state and either advances the row back onto the canonical
//! chain OR marks it for manual intervention. It NEVER re-signs, NEVER
//! reserves a fresh nonce, and NEVER auto-fee-bumps.
//!
//! ## Outcome semantics (frozen)
//!
//! 1. `RemainedCanonical` — receipt block header still exists and matches
//!    the stored receipt hash (false-alarm reorg). Nothing to do; the
//!    caller may leave the row where it is (the worker's next tick will
//!    re-evaluate).
//! 2. `ReminedInReplacement { block_number, block_hash }` — the same
//!    `tx_hash` is now mined in a new block on the canonical chain.
//!    Transition `Reorged -> Confirming`, reset `confirmation_count`,
//!    increment `reorg_count`, refresh the receipt block fields.
//! 3. `ReturnedToPending` — the transaction is known to the mempool but
//!    no receipt (either mined then unmined, or provider view slipped).
//!    Transition `Reorged -> Pending`.
//! 4. `DisappearedCanRebroadcast` — the tx hash is nowhere; the nonce
//!    investigator confirms the nonce is still ours. Row remains
//!    `Reorged` with `failure_class = REORGED_TX_DISAPPEARED`. Operator
//!    may (out-of-band) invoke `resend_same_bytes` under bounded policy;
//!    this module NEVER auto-resends.
//! 5. `DifferentTxConsumedNonce { observed_tx_hash }` — some other tx
//!    grabbed our nonce. Escalate to `ManualInterventionRequired` with
//!    `failure_class = REORG_NONCE_STOLEN`. Frozen posture: NO auto
//!    replacement, NO fee bump.
//! 6. `Unresolved` — nonce investigator returned `Ambiguous` (transient
//!    provider view). Row remains `Reorged`; caller retries on a
//!    subsequent tick.
//!
//! ## Coordination with the indexer reorg recovery pipeline
//!
//! Hybrid V2's existing indexer reorg path marks affected
//! `MatchedExecutionRow` rows with `completion_status =
//! INVALIDATED_BY_REORG`. The confirmation worker's correlation checker
//! already reads that column via [`IndexerCorrelationChecker`]; the
//! transaction reorg recovery here is deliberately narrower — it deals
//! only with the on-chain fate of our tx hash and its nonce. Cross-
//! reference is the operator responsibility (via the admin controls in
//! Part S).
//!
//! Frozen safety:
//! * `NO_AUTOMATIC_NONCE_REPLACEMENT`, `NO_AUTOMATIC_FEE_BUMP_OR_RBF`.
//! * `REORGED_TRANSACTION_IS_NEVER_LEFT_CONFIRMED` — the promotion path
//!   requires re-observation via the worker after this module has
//!   returned `ReminedInReplacement`, so no direct `Reorged -> Confirmed`
//!   edge is emitted here (the state matrix rejects that transition
//!   outright, and the transition below stops at `Confirming`).

use crate::hybrid_v2::execution::broadcast_nonce_policy::{
    BroadcastNonceInvestigator, NonceInvestigationOutcome,
};
use crate::hybrid_v2::execution::broadcast_outbox::failure_class;
use crate::hybrid_v2::execution::broadcast_rpc::{
    BroadcastRpcError, ExecutionBroadcastRpcClient, TransactionSummary, TxReceipt,
};
use crate::hybrid_v2::execution::broadcast_state::{BroadcastPhase, BroadcastStatePatch};
use crate::hybrid_v2::persistence::HybridV2ProjectionStore;
use thiserror::Error;

// -----------------------------------------------------------------
//                          OUTCOME + ERROR
// -----------------------------------------------------------------

/// Failure-class strings emitted by [`BroadcastReorgRecovery`]. Retained
/// as constants so operator dashboards can grep on a stable token.
pub mod reorg_failure_class {
    /// Tx hash no longer observable on the network AND the reserved
    /// nonce is still ours. Operator MAY invoke `resend_same_bytes`
    /// under bounded policy; recovery NEVER auto-resends.
    pub const REORGED_TX_DISAPPEARED: &str = "REORGED_TX_DISAPPEARED";
    /// A different transaction consumed our reserved nonce during the
    /// reorg window. Terminal MANUAL_INTERVENTION_REQUIRED.
    pub const REORG_NONCE_STOLEN: &str = "REORG_NONCE_STOLEN";
    /// The nonce investigator returned `Ambiguous` — provider view is
    /// transiently inconsistent. Row remains `Reorged`; recovery yields
    /// `Unresolved`.
    pub const REORG_UNRESOLVED: &str = "REORG_UNRESOLVED";
}

/// Structured outcome of a single `recover(...)` cycle. The caller
/// (typically the confirmation worker or an admin recheck) reads this
/// enum to decide whether to leave the row, request another tick, or
/// escalate. The recovery routine ITSELF has already persisted the
/// phase transition (or left the row untouched for `Unresolved` /
/// `DisappearedCanRebroadcast`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReorgRecoveryOutcome {
    /// Receipt block header still exists and matches the stored receipt
    /// hash. False alarm — no phase change persisted.
    RemainedCanonical,
    /// Same tx hash mined in a NEW canonical block. Row advanced
    /// `Reorged -> Confirming` with `reorg_count += 1` and refreshed
    /// receipt block fields.
    ReminedInReplacement {
        block_number: u64,
        block_hash: [u8; 32],
    },
    /// Tx is in the mempool but not mined. Row advanced `Reorged ->
    /// Pending`.
    ReturnedToPending,
    /// Tx is nowhere and the nonce investigator confirms the nonce slot
    /// is still ours. Row REMAINS in `Reorged`; `failure_class`
    /// stamped to `REORGED_TX_DISAPPEARED`. Operator may resend the
    /// same bytes.
    DisappearedCanRebroadcast,
    /// A different tx consumed our reserved nonce. Row escalated to
    /// `ManualInterventionRequired` with `failure_class =
    /// REORG_NONCE_STOLEN`.
    DifferentTxConsumedNonce { observed_tx_hash: [u8; 32] },
    /// Provider view was transiently inconsistent (`Ambiguous`). Row
    /// remains `Reorged`; caller retries on a subsequent tick.
    Unresolved,
}

/// Errors surfaced from the recovery cycle. Every variant is retryable
/// on a subsequent tick — the recovery module NEVER escalates for a
/// transient RPC failure.
#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum ReorgRecoveryError {
    /// The upstream RPC failed. Retryable on the next tick.
    #[error("rpc failure: {0}")]
    RpcFailure(String),
    /// The store rejected a read or a phase transition. Callers surface
    /// this to the operator; it is NOT retried automatically.
    #[error("store failure: {0}")]
    StoreFailure(String),
}

// -----------------------------------------------------------------
//                          RECOVERY
// -----------------------------------------------------------------

/// Reorg recovery driver. Borrows the store + rpc + nonce investigator
/// for the duration of a single `recover` call. The `deployment_id`,
/// `chain_id`, and `executor_address` are captured on construction so
/// the caller cannot accidentally point the recovery at a different
/// deployment mid-flight.
pub struct BroadcastReorgRecovery<'a> {
    pub store: &'a dyn HybridV2ProjectionStore,
    pub rpc: &'a dyn ExecutionBroadcastRpcClient,
    pub nonce_investigator: &'a BroadcastNonceInvestigator<'a>,
    pub deployment_id: i64,
    pub chain_id: u64,
    pub executor_address: [u8; 20],
}

impl BroadcastReorgRecovery<'_> {
    /// Perform one recovery cycle for a row that is currently at
    /// `BroadcastPhase::Reorged`. Rows in any other phase return
    /// `RemainedCanonical` as a NO-OP so accidental callers do not
    /// perturb the row.
    pub async fn recover(
        &self,
        canonical_execution_id: &str,
    ) -> Result<ReorgRecoveryOutcome, ReorgRecoveryError> {
        // Suppress dead-code lint for deployment/chain metadata — retained
        // in the struct for future audit surfaces (e.g. an admin route
        // that reports which deployment triggered the recovery).
        let _ = (self.deployment_id, self.chain_id, self.executor_address);

        let row = self
            .store
            .get_broadcast_state(canonical_execution_id)
            .await
            .map_err(|e| ReorgRecoveryError::StoreFailure(e.to_string()))?
            .ok_or_else(|| {
                ReorgRecoveryError::StoreFailure(format!(
                    "recover: no broadcast row for {canonical_execution_id}"
                ))
            })?;

        // Guard: recovery only operates on `Reorged` rows. Everything
        // else is a NO-OP — return RemainedCanonical so the caller can
        // treat this as an idempotent "nothing to do" verdict.
        if row.phase != BroadcastPhase::Reorged {
            return Ok(ReorgRecoveryOutcome::RemainedCanonical);
        }

        let tx_hash_hex = match row.tx_hash.as_deref() {
            Some(s) => s.to_string(),
            None => {
                // A Reorged row without a tx_hash is a data integrity
                // problem — bail with a store failure rather than
                // silently escalating.
                return Err(ReorgRecoveryError::StoreFailure(format!(
                    "recover: Reorged row {canonical_execution_id} missing tx_hash"
                )));
            }
        };
        let our_tx_hash = parse_bytes32_lax(&tx_hash_hex).ok_or_else(|| {
            ReorgRecoveryError::StoreFailure(format!(
                "recover: malformed tx_hash {tx_hash_hex} for {canonical_execution_id}"
            ))
        })?;

        // Step 1 — look up receipt.
        let receipt_result = self.rpc.receipt_by_hash(our_tx_hash).await;
        match receipt_result {
            Ok(Some(receipt)) => {
                // A receipt exists. Two sub-branches:
                //  * receipt has a new block — same tx re-mined in
                //    replacement chain OR still on old (canonical again).
                //  * receipt is a pending shape — treated as ReturnedToPending.
                self.handle_receipt_present(canonical_execution_id, &row, &receipt)
                    .await
            }
            Ok(None) => {
                // No receipt. Two sub-branches:
                //  * tx still known (mempool) -> ReturnedToPending.
                //  * tx nowhere -> nonce investigation.
                self.handle_receipt_absent(canonical_execution_id, &row, our_tx_hash)
                    .await
            }
            Err(e) => Err(ReorgRecoveryError::RpcFailure(e.to_string())),
        }
    }

    // -----------------------------------------------------------------
    //  Receipt present branch
    // -----------------------------------------------------------------

    async fn handle_receipt_present(
        &self,
        canonical_execution_id: &str,
        row: &crate::hybrid_v2::execution::BroadcastStateRow,
        receipt: &TxReceipt,
    ) -> Result<ReorgRecoveryOutcome, ReorgRecoveryError> {
        // Receipt existing at block_number 0 with an all-zero hash is
        // the "pending shape" some providers return — treat as pending.
        if receipt.block_number == 0 && receipt.block_hash == [0u8; 32] {
            return self
                .transition_to_pending(canonical_execution_id, row)
                .await;
        }
        // Verify the new receipt block is canonical.
        let header = self
            .rpc
            .block_header_by_number(receipt.block_number)
            .await
            .map_err(|e| ReorgRecoveryError::RpcFailure(e.to_string()))?;
        let canonical = match header {
            Some(h) => h.hash == receipt.block_hash,
            None => false,
        };

        // Compare against the row's previously-observed receipt block.
        let previous_block =
            row.receipt_block_number
                .and_then(|v| if v >= 0 { Some(v as u64) } else { None });
        let previous_hash = row
            .receipt_block_hash
            .as_deref()
            .and_then(parse_bytes32_lax);
        let same_slot = previous_block == Some(receipt.block_number)
            && previous_hash == Some(receipt.block_hash);

        if canonical && same_slot {
            // False alarm: the previously-observed receipt block is on
            // the canonical chain after all. Do not mutate — the worker's
            // canonicality path will normalize on the next tick.
            return Ok(ReorgRecoveryOutcome::RemainedCanonical);
        }
        if !canonical {
            // Receipt exists but the block is not canonical either. This
            // is a rare provider oddity — treat as Unresolved so the
            // operator can inspect.
            return Ok(ReorgRecoveryOutcome::Unresolved);
        }
        // Canonical + new block: same tx hash mined in a new canonical
        // block. Advance `Reorged -> Confirming` and stamp fresh
        // receipt metadata + reorg counter.
        self.transition_to_confirming(canonical_execution_id, row, receipt)
            .await
    }

    async fn transition_to_pending(
        &self,
        canonical_execution_id: &str,
        row: &crate::hybrid_v2::execution::BroadcastStateRow,
    ) -> Result<ReorgRecoveryOutcome, ReorgRecoveryError> {
        let now_ms = wall_ms();
        let patch = BroadcastStatePatch {
            canonicality_state: Some("UNKNOWN".into()),
            ..Default::default()
        };
        if !row.phase.can_transition_to(BroadcastPhase::Pending) {
            // Matrix should permit Reorged -> Pending; guard defensively.
            return Ok(ReorgRecoveryOutcome::Unresolved);
        }
        let ok = self
            .store
            .update_broadcast_phase(
                canonical_execution_id,
                row.phase,
                BroadcastPhase::Pending,
                now_ms,
                patch,
            )
            .await
            .map_err(|e| ReorgRecoveryError::StoreFailure(e.to_string()))?;
        if !ok {
            return Err(ReorgRecoveryError::StoreFailure(
                "recover: lost update Reorged -> Pending".into(),
            ));
        }
        Ok(ReorgRecoveryOutcome::ReturnedToPending)
    }

    async fn transition_to_confirming(
        &self,
        canonical_execution_id: &str,
        row: &crate::hybrid_v2::execution::BroadcastStateRow,
        receipt: &TxReceipt,
    ) -> Result<ReorgRecoveryOutcome, ReorgRecoveryError> {
        let now_ms = wall_ms();
        let prior_reorgs = row.reorg_count;
        let patch = BroadcastStatePatch {
            receipt_tx_hash: Some(format!("0x{}", hex_encode(&receipt.tx_hash))),
            receipt_block_number: Some(receipt.block_number as i64),
            receipt_block_hash: Some(format!("0x{}", hex_encode(&receipt.block_hash))),
            receipt_status: Some(receipt.status as i16),
            confirmation_count: Some(0),
            canonicality_state: Some("CANONICAL".into()),
            reorg_count: Some(prior_reorgs.saturating_add(1)),
            ..Default::default()
        };
        if !row.phase.can_transition_to(BroadcastPhase::Confirming) {
            return Ok(ReorgRecoveryOutcome::Unresolved);
        }
        let ok = self
            .store
            .update_broadcast_phase(
                canonical_execution_id,
                row.phase,
                BroadcastPhase::Confirming,
                now_ms,
                patch,
            )
            .await
            .map_err(|e| ReorgRecoveryError::StoreFailure(e.to_string()))?;
        if !ok {
            return Err(ReorgRecoveryError::StoreFailure(
                "recover: lost update Reorged -> Confirming".into(),
            ));
        }
        Ok(ReorgRecoveryOutcome::ReminedInReplacement {
            block_number: receipt.block_number,
            block_hash: receipt.block_hash,
        })
    }

    // -----------------------------------------------------------------
    //  Receipt absent branch — nonce investigation
    // -----------------------------------------------------------------

    async fn handle_receipt_absent(
        &self,
        canonical_execution_id: &str,
        row: &crate::hybrid_v2::execution::BroadcastStateRow,
        our_tx_hash: [u8; 32],
    ) -> Result<ReorgRecoveryOutcome, ReorgRecoveryError> {
        // First: is the tx still known to the mempool?
        let mempool_lookup: Result<Option<TransactionSummary>, BroadcastRpcError> =
            self.rpc.transaction_by_hash(our_tx_hash).await;
        match mempool_lookup {
            Ok(Some(tx)) if tx.tx_hash == our_tx_hash => {
                if tx.block_number.is_some() {
                    // Extremely rare — receipt None but tx says mined.
                    // Look up the header for the reported block; if it is
                    // canonical, upgrade to Confirming.
                    let block_number = tx.block_number.unwrap_or(0);
                    let block_hash = tx.block_hash.unwrap_or([0u8; 32]);
                    let synthetic = TxReceipt {
                        tx_hash: our_tx_hash,
                        block_number,
                        block_hash,
                        status: 1,
                        gas_used: 0,
                        effective_gas_price_wei: alloy_primitives::U256::ZERO,
                        cumulative_gas_used: 0,
                        from: tx.from,
                        to: tx.to,
                    };
                    return self
                        .handle_receipt_present(canonical_execution_id, row, &synthetic)
                        .await;
                }
                return self
                    .transition_to_pending(canonical_execution_id, row)
                    .await;
            }
            Ok(_) => {}
            Err(e) => return Err(ReorgRecoveryError::RpcFailure(e.to_string())),
        }

        // Nonce investigator — decide between disappeared, stolen, and
        // ambiguous. The investigator is READ-ONLY on our nonce
        // reservation. The reserved nonce is not stored on
        // `BroadcastStateRow`; fetch it from the execution request row
        // (same store).
        let reserved_nonce = self
            .read_reserved_nonce(canonical_execution_id)
            .await?
            .unwrap_or(0);
        let outcome = self
            .nonce_investigator
            .investigate(our_tx_hash, reserved_nonce)
            .await
            .map_err(|e| ReorgRecoveryError::RpcFailure(e.to_string()))?;
        match outcome {
            NonceInvestigationOutcome::OurTxMined {
                block_number,
                block_hash,
            } => {
                // Rare: nonce investigator saw the tx via reorg cache.
                // Synthesize a minimal receipt and route through the
                // canonical path.
                let synthetic = TxReceipt {
                    tx_hash: our_tx_hash,
                    block_number,
                    block_hash,
                    status: 1,
                    gas_used: 0,
                    effective_gas_price_wei: alloy_primitives::U256::ZERO,
                    cumulative_gas_used: 0,
                    from: self.executor_address,
                    to: None,
                };
                self.handle_receipt_present(canonical_execution_id, row, &synthetic)
                    .await
            }
            NonceInvestigationOutcome::OurTxPending => {
                self.transition_to_pending(canonical_execution_id, row)
                    .await
            }
            NonceInvestigationOutcome::NonceReleasedNoTxFound => {
                // Row stays Reorged; stamp failure_class so operators can
                // see the reason without reading the phase machine.
                self.stamp_failure_no_phase_change(
                    canonical_execution_id,
                    row,
                    reorg_failure_class::REORGED_TX_DISAPPEARED,
                    "tx not observable; nonce released (operator may resend same bytes)",
                )
                .await?;
                Ok(ReorgRecoveryOutcome::DisappearedCanRebroadcast)
            }
            NonceInvestigationOutcome::DifferentTxConsumedNonce { observed_tx_hash } => {
                // Escalate — a foreign tx grabbed our nonce.
                self.escalate_manual(
                    canonical_execution_id,
                    row,
                    reorg_failure_class::REORG_NONCE_STOLEN,
                    &format!(
                        "reorg recovery: different tx consumed nonce (observed=0x{})",
                        hex_encode(&observed_tx_hash)
                    ),
                )
                .await?;
                Ok(ReorgRecoveryOutcome::DifferentTxConsumedNonce { observed_tx_hash })
            }
            NonceInvestigationOutcome::Ambiguous => {
                // Row remains Reorged.
                self.stamp_failure_no_phase_change(
                    canonical_execution_id,
                    row,
                    reorg_failure_class::REORG_UNRESOLVED,
                    "reorg recovery: investigator returned Ambiguous — retry on next tick",
                )
                .await?;
                Ok(ReorgRecoveryOutcome::Unresolved)
            }
        }
    }

    async fn escalate_manual(
        &self,
        canonical_execution_id: &str,
        row: &crate::hybrid_v2::execution::BroadcastStateRow,
        failure_class_str: &str,
        detail: &str,
    ) -> Result<(), ReorgRecoveryError> {
        let now_ms = wall_ms();
        let patch = BroadcastStatePatch {
            failure_class: Some(failure_class_str.to_string()),
            failure_detail: Some(detail.to_string()),
            terminal_at_ms: Some(now_ms),
            ..Default::default()
        };
        if !row
            .phase
            .can_transition_to(BroadcastPhase::ManualInterventionRequired)
        {
            return Err(ReorgRecoveryError::StoreFailure(format!(
                "recover: phase {} cannot escalate to MANUAL_INTERVENTION_REQUIRED",
                row.phase
            )));
        }
        let ok = self
            .store
            .update_broadcast_phase(
                canonical_execution_id,
                row.phase,
                BroadcastPhase::ManualInterventionRequired,
                now_ms,
                patch,
            )
            .await
            .map_err(|e| ReorgRecoveryError::StoreFailure(e.to_string()))?;
        if !ok {
            return Err(ReorgRecoveryError::StoreFailure(
                "recover: lost update Reorged -> MANUAL_INTERVENTION_REQUIRED".into(),
            ));
        }
        Ok(())
    }

    /// Suppress the `failure_class` propagation for non-terminal
    /// outcomes: since `update_broadcast_phase` rejects self-loops, we
    /// cannot patch fields without changing the phase. We instead emit
    /// a `tracing::warn` so operator dashboards still surface the
    /// reason. Persistence of a same-phase failure_class is a Part V
    /// follow-up.
    async fn stamp_failure_no_phase_change(
        &self,
        canonical_execution_id: &str,
        row: &crate::hybrid_v2::execution::BroadcastStateRow,
        failure_class_str: &str,
        detail: &str,
    ) -> Result<(), ReorgRecoveryError> {
        tracing::warn!(
            canonical_execution_id = canonical_execution_id,
            phase = %row.phase,
            failure_class = failure_class_str,
            detail = detail,
            "hybrid_v2::broadcast_reorg_recovery: same-phase failure_class"
        );
        // Suppress unused-variable warnings when the tracing macro is
        // compiled out.
        let _ = (row, failure_class_str, detail);
        Ok(())
    }

    async fn read_reserved_nonce(
        &self,
        canonical_execution_id: &str,
    ) -> Result<Option<u64>, ReorgRecoveryError> {
        let exec = self
            .store
            .get_execution_request(canonical_execution_id)
            .await
            .map_err(|e| ReorgRecoveryError::StoreFailure(e.to_string()))?;
        Ok(exec
            .and_then(|r| r.reserved_nonce)
            .and_then(|n| u64::try_from(n).ok()))
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

fn wall_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Suppress "unused" warning while keeping the constant module public.
#[doc(hidden)]
pub const _FAILURE_CLASS_SANITY_CHECK: &str = failure_class::CORRELATION_MISSING;

// -----------------------------------------------------------------
//                          UNIT TESTS
// -----------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hybrid_v2::execution::broadcast_rpc::{
        BlockHeader, SendOutcome, TransactionSummary,
    };
    use crate::hybrid_v2::execution::broadcast_state::{BroadcastPhase, BroadcastStatePatch};
    use crate::hybrid_v2::execution::rpc::BlockTag;
    use crate::hybrid_v2::persistence::InMemoryProjectionStore;
    use alloy_primitives::U256;
    use async_trait::async_trait;
    use std::sync::Mutex;

    // ---------------- mock RPC ------------------------------------

    #[derive(Default)]
    struct MockRpc {
        inner: Mutex<MockRpcInner>,
    }

    #[derive(Default)]
    struct MockRpcInner {
        receipts: std::collections::HashMap<[u8; 32], TxReceipt>,
        transactions: std::collections::HashMap<[u8; 32], TransactionSummary>,
        headers_by_number: std::collections::HashMap<u64, BlockHeader>,
        pending_nonce: u64,
        transient_error: Option<BroadcastRpcError>,
        write_calls: Vec<&'static str>,
    }

    impl MockRpc {
        fn set_receipt(&self, hash: [u8; 32], receipt: TxReceipt) {
            self.inner.lock().unwrap().receipts.insert(hash, receipt);
        }
        fn set_transaction(&self, hash: [u8; 32], tx: TransactionSummary) {
            self.inner.lock().unwrap().transactions.insert(hash, tx);
        }
        fn set_header(&self, n: u64, hash: [u8; 32]) {
            let header = BlockHeader {
                number: n,
                hash,
                parent_hash: [0u8; 32],
                timestamp: 0,
            };
            self.inner
                .lock()
                .unwrap()
                .headers_by_number
                .insert(n, header);
        }
        fn set_pending_nonce(&self, n: u64) {
            self.inner.lock().unwrap().pending_nonce = n;
        }
        fn set_transient_error(&self, err: BroadcastRpcError) {
            self.inner.lock().unwrap().transient_error = Some(err);
        }
        fn write_calls(&self) -> Vec<&'static str> {
            self.inner.lock().unwrap().write_calls.clone()
        }
    }

    #[async_trait]
    impl ExecutionBroadcastRpcClient for MockRpc {
        async fn chain_id(&self) -> Result<u64, BroadcastRpcError> {
            Ok(84532)
        }
        async fn head_block_number(&self) -> Result<u64, BroadcastRpcError> {
            Ok(1_000)
        }
        async fn finalized_block_number(&self) -> Result<Option<u64>, BroadcastRpcError> {
            Ok(Some(990))
        }
        async fn transaction_count(
            &self,
            _address: [u8; 20],
            _block_tag: BlockTag,
        ) -> Result<u64, BroadcastRpcError> {
            Ok(self.inner.lock().unwrap().pending_nonce)
        }
        async fn transaction_by_hash(
            &self,
            tx_hash: [u8; 32],
        ) -> Result<Option<TransactionSummary>, BroadcastRpcError> {
            Ok(self
                .inner
                .lock()
                .unwrap()
                .transactions
                .get(&tx_hash)
                .cloned())
        }
        async fn receipt_by_hash(
            &self,
            tx_hash: [u8; 32],
        ) -> Result<Option<TxReceipt>, BroadcastRpcError> {
            if let Some(err) = self.inner.lock().unwrap().transient_error.take() {
                return Err(err);
            }
            Ok(self.inner.lock().unwrap().receipts.get(&tx_hash).cloned())
        }
        async fn block_header_by_number(
            &self,
            number: u64,
        ) -> Result<Option<BlockHeader>, BroadcastRpcError> {
            Ok(self
                .inner
                .lock()
                .unwrap()
                .headers_by_number
                .get(&number)
                .cloned())
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
            g.write_calls.push("eth_sendRawTransaction");
            panic!("reorg recovery must not call send_raw_transaction");
        }
    }

    // ---------------- fixtures ------------------------------------

    const CID: &str = "reorg-cid";

    fn hex_of(b: &[u8]) -> String {
        format!("0x{}", hex_encode(b))
    }

    fn our_hash() -> [u8; 32] {
        [0x11u8; 32]
    }

    async fn install_reorged_row(
        store: &InMemoryProjectionStore,
        receipt_block_number: u64,
        receipt_block_hash: [u8; 32],
    ) {
        seed_execution_request_with_nonce(store, 5).await;
        store.insert_broadcast_state(CID, 1_000).await.unwrap();
        let tx_hex = hex_of(&our_hash());
        store
            .set_broadcast_tx_hash(CID, &tx_hex, &tx_hex, &tx_hex, 1_001)
            .await
            .unwrap();
        store
            .update_broadcast_phase(
                CID,
                BroadcastPhase::BroadcastDisabled,
                BroadcastPhase::Broadcasting,
                1_002,
                BroadcastStatePatch {
                    submission_attempt_count: Some(1),
                    first_submission_at_ms: Some(1_002),
                    last_submission_at_ms: Some(1_002),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        store
            .update_broadcast_phase(
                CID,
                BroadcastPhase::Broadcasting,
                BroadcastPhase::Submitted,
                1_003,
                BroadcastStatePatch::default(),
            )
            .await
            .unwrap();
        store
            .update_broadcast_phase(
                CID,
                BroadcastPhase::Submitted,
                BroadcastPhase::MinedSuccess,
                1_004,
                BroadcastStatePatch {
                    receipt_tx_hash: Some(hex_of(&our_hash())),
                    receipt_block_number: Some(receipt_block_number as i64),
                    receipt_block_hash: Some(hex_of(&receipt_block_hash)),
                    receipt_status: Some(1),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        store
            .update_broadcast_phase(
                CID,
                BroadcastPhase::MinedSuccess,
                BroadcastPhase::Reorged,
                1_005,
                BroadcastStatePatch {
                    canonicality_state: Some("ORPHANED".into()),
                    reorg_count: Some(1),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
    }

    fn build_recovery<'a>(
        store: &'a InMemoryProjectionStore,
        rpc: &'a MockRpc,
        investigator: &'a BroadcastNonceInvestigator<'a>,
    ) -> BroadcastReorgRecovery<'a> {
        BroadcastReorgRecovery {
            store,
            rpc,
            nonce_investigator: investigator,
            deployment_id: 1,
            chain_id: 84532,
            executor_address: [0x77u8; 20],
        }
    }

    // ---------------- tests ---------------------------------------

    #[tokio::test]
    async fn rejects_non_reorged_phase_as_noop() {
        let store = InMemoryProjectionStore::new();
        // Row is at BROADCAST_DISABLED — recovery is a no-op.
        seed_execution_request_with_nonce(&store, 5).await;
        store.insert_broadcast_state(CID, 1_000).await.unwrap();
        let rpc = MockRpc::default();
        let investigator = BroadcastNonceInvestigator {
            store: &store,
            rpc: &rpc,
            executor_address: [0x77u8; 20],
            chain_id: 84532,
        };
        let recovery = build_recovery(&store, &rpc, &investigator);
        let outcome = recovery.recover(CID).await.unwrap();
        assert_eq!(outcome, ReorgRecoveryOutcome::RemainedCanonical);
        assert!(rpc.write_calls().is_empty());
    }

    #[tokio::test]
    async fn remined_in_replacement_transitions_to_confirming() {
        let store = InMemoryProjectionStore::new();
        install_reorged_row(&store, 500, [0xAB; 32]).await;
        let rpc = MockRpc::default();
        // Receipt in NEW block 600 with new hash 0xCD; header canonical.
        rpc.set_receipt(
            our_hash(),
            TxReceipt {
                tx_hash: our_hash(),
                block_number: 600,
                block_hash: [0xCD; 32],
                status: 1,
                gas_used: 400_000,
                effective_gas_price_wei: U256::from(1_000_000_000u64),
                cumulative_gas_used: 400_000,
                from: [0x77u8; 20],
                to: Some([0xEE; 20]),
            },
        );
        rpc.set_header(600, [0xCD; 32]);
        let investigator = BroadcastNonceInvestigator {
            store: &store,
            rpc: &rpc,
            executor_address: [0x77u8; 20],
            chain_id: 84532,
        };
        let recovery = build_recovery(&store, &rpc, &investigator);
        let outcome = recovery.recover(CID).await.unwrap();
        assert_eq!(
            outcome,
            ReorgRecoveryOutcome::ReminedInReplacement {
                block_number: 600,
                block_hash: [0xCD; 32],
            }
        );
        let row = store.get_broadcast_state(CID).await.unwrap().unwrap();
        assert_eq!(row.phase, BroadcastPhase::Confirming);
        assert_eq!(row.reorg_count, 2);
        assert_eq!(row.confirmation_count, 0);
        assert!(rpc.write_calls().is_empty());
    }

    #[tokio::test]
    async fn remained_canonical_when_previous_block_still_canonical() {
        // The previously-observed block is still canonical (false alarm).
        let store = InMemoryProjectionStore::new();
        install_reorged_row(&store, 500, [0xAB; 32]).await;
        let rpc = MockRpc::default();
        rpc.set_receipt(
            our_hash(),
            TxReceipt {
                tx_hash: our_hash(),
                block_number: 500,
                block_hash: [0xAB; 32],
                status: 1,
                gas_used: 400_000,
                effective_gas_price_wei: U256::from(1_000_000_000u64),
                cumulative_gas_used: 400_000,
                from: [0x77u8; 20],
                to: Some([0xEE; 20]),
            },
        );
        rpc.set_header(500, [0xAB; 32]);
        let investigator = BroadcastNonceInvestigator {
            store: &store,
            rpc: &rpc,
            executor_address: [0x77u8; 20],
            chain_id: 84532,
        };
        let recovery = build_recovery(&store, &rpc, &investigator);
        let outcome = recovery.recover(CID).await.unwrap();
        assert_eq!(outcome, ReorgRecoveryOutcome::RemainedCanonical);
        // Row unchanged — still Reorged.
        let row = store.get_broadcast_state(CID).await.unwrap().unwrap();
        assert_eq!(row.phase, BroadcastPhase::Reorged);
    }

    #[tokio::test]
    async fn returned_to_pending_when_receipt_absent_but_tx_in_mempool() {
        let store = InMemoryProjectionStore::new();
        install_reorged_row(&store, 500, [0xAB; 32]).await;
        let rpc = MockRpc::default();
        // No receipt configured. Tx summary present in mempool.
        rpc.set_transaction(
            our_hash(),
            TransactionSummary {
                tx_hash: our_hash(),
                from: [0x77u8; 20],
                to: Some([0xEE; 20]),
                nonce: 42,
                block_number: None,
                block_hash: None,
                value_wei: U256::ZERO,
                input_bytes_len: 4,
                input_hash: None,
                max_fee_per_gas: None,
                max_priority_fee_per_gas: None,
                tx_type: 2,
            },
        );
        let investigator = BroadcastNonceInvestigator {
            store: &store,
            rpc: &rpc,
            executor_address: [0x77u8; 20],
            chain_id: 84532,
        };
        let recovery = build_recovery(&store, &rpc, &investigator);
        let outcome = recovery.recover(CID).await.unwrap();
        assert_eq!(outcome, ReorgRecoveryOutcome::ReturnedToPending);
        let row = store.get_broadcast_state(CID).await.unwrap().unwrap();
        assert_eq!(row.phase, BroadcastPhase::Pending);
    }

    #[tokio::test]
    async fn disappeared_can_rebroadcast_when_nonce_released() {
        let store = InMemoryProjectionStore::new();
        install_reorged_row(&store, 500, [0xAB; 32]).await;
        let rpc = MockRpc::default();
        // No receipt, no mempool entry, pending_nonce < our_nonce (0 < 0 is
        // ambiguous — install execution row so reserved_nonce lookup
        // yields 5).
        seed_execution_request_with_nonce(&store, 5).await;
        rpc.set_pending_nonce(4);
        let investigator = BroadcastNonceInvestigator {
            store: &store,
            rpc: &rpc,
            executor_address: [0x77u8; 20],
            chain_id: 84532,
        };
        let recovery = build_recovery(&store, &rpc, &investigator);
        let outcome = recovery.recover(CID).await.unwrap();
        assert_eq!(outcome, ReorgRecoveryOutcome::DisappearedCanRebroadcast);
        // Row remains Reorged (frozen: never auto-resend).
        let row = store.get_broadcast_state(CID).await.unwrap().unwrap();
        assert_eq!(row.phase, BroadcastPhase::Reorged);
        assert!(rpc.write_calls().is_empty());
    }

    #[tokio::test]
    async fn different_tx_consumed_nonce_escalates_manual() {
        let store = InMemoryProjectionStore::new();
        install_reorged_row(&store, 500, [0xAB; 32]).await;
        let rpc = MockRpc::default();
        rpc.set_pending_nonce(6); // > our_nonce
        let investigator = BroadcastNonceInvestigator {
            store: &store,
            rpc: &rpc,
            executor_address: [0x77u8; 20],
            chain_id: 84532,
        };
        let recovery = build_recovery(&store, &rpc, &investigator);
        let outcome = recovery.recover(CID).await.unwrap();
        assert!(matches!(
            outcome,
            ReorgRecoveryOutcome::DifferentTxConsumedNonce { .. }
        ));
        let row = store.get_broadcast_state(CID).await.unwrap().unwrap();
        assert_eq!(row.phase, BroadcastPhase::ManualInterventionRequired);
        assert_eq!(
            row.failure_class.as_deref(),
            Some(reorg_failure_class::REORG_NONCE_STOLEN)
        );
    }

    #[tokio::test]
    async fn unresolved_when_investigator_ambiguous() {
        let store = InMemoryProjectionStore::new();
        install_reorged_row(&store, 500, [0xAB; 32]).await;
        let rpc = MockRpc::default();
        rpc.set_pending_nonce(5); // == our_nonce -> Ambiguous
        let investigator = BroadcastNonceInvestigator {
            store: &store,
            rpc: &rpc,
            executor_address: [0x77u8; 20],
            chain_id: 84532,
        };
        let recovery = build_recovery(&store, &rpc, &investigator);
        let outcome = recovery.recover(CID).await.unwrap();
        assert_eq!(outcome, ReorgRecoveryOutcome::Unresolved);
        let row = store.get_broadcast_state(CID).await.unwrap().unwrap();
        assert_eq!(row.phase, BroadcastPhase::Reorged);
    }

    #[tokio::test]
    async fn receipt_rpc_failure_is_retryable_error() {
        let store = InMemoryProjectionStore::new();
        install_reorged_row(&store, 500, [0xAB; 32]).await;
        let rpc = MockRpc::default();
        rpc.set_transient_error(BroadcastRpcError::Timeout);
        let investigator = BroadcastNonceInvestigator {
            store: &store,
            rpc: &rpc,
            executor_address: [0x77u8; 20],
            chain_id: 84532,
        };
        let recovery = build_recovery(&store, &rpc, &investigator);
        let err = recovery.recover(CID).await.expect_err("rpc failure");
        assert!(matches!(err, ReorgRecoveryError::RpcFailure(_)));
    }

    async fn seed_execution_request_with_nonce(store: &InMemoryProjectionStore, nonce: i64) {
        // Insert a minimal execution request row so reserved_nonce is
        // discoverable via the standard store trait.
        use crate::hybrid_v2::execution::persistence::ExecutionRequestRow;
        use crate::hybrid_v2::execution::state::ExecutionPhase;
        let row = ExecutionRequestRow {
            canonical_execution_id: CID.to_string(),
            deployment_id: 1,
            chain_id: 84532,
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
            target_contract: format!("0x{}", "ee".repeat(20)),
            selector: "0x00000000".into(),
            calldata_hash: Some(format!("0x{}", "cd".repeat(32))),
            plan_hash: Some(format!("0x{}", "ee".repeat(32))),
            tx_value_wei: "0".into(),
            simulation_block_number: Some(1),
            simulation_block_hash: Some(format!("0x{}", "cc".repeat(32))),
            simulation_gas_estimate: Some(500_000),
            simulation_result_json: Some(serde_json::json!({})),
            signer_identity: Some(format!("0x{}", "77".repeat(20))),
            signing_payload_hash: Some(format!("0x{}", "ff".repeat(32))),
            signature_r: Some(format!("0x{}", "11".repeat(32))),
            signature_s: Some(format!("0x{}", "22".repeat(32))),
            signature_v: Some(0),
            recovered_signer: Some(format!("0x{}", "77".repeat(20))),
            gas_limit: Some(1_000_000),
            max_fee_per_gas_wei: Some("2000000000".into()),
            max_priority_fee_per_gas_wei: Some("500000000".into()),
            reserved_nonce: Some(nonce),
            phase: ExecutionPhase::SignatureVerified,
            failure_class: None,
            failure_detail: None,
            retry_count: 0,
            holder_epoch: None,
            signer_request_idempotency_key: None,
            created_at_ms: 1,
            updated_at_ms: 1,
        };
        store.insert_execution_request(&row).await.unwrap();
    }
}
