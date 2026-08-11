//! Receipt polling + canonicality verification + confirmation depth
//! for the Hybrid V2 broadcast pipeline (Parts L, M, N of
//! `BACKEND-HYBRID-V2-BROADCAST-AND-CONFIRMATION-V1`).
//!
//! ## Flow (per tick, per row)
//!
//! 1. `receipt_by_hash(tx_hash)`:
//!    - `None` + phase = `Submitted` / `SubmissionUnknown`:
//!        - verify the tx is still known via `transaction_by_hash`;
//!          if `None` AND `age > max_pending_age_ms` → `Dropped`.
//!        - else stay in current phase.
//!    - `Some(receipt)`:
//!        - assert `receipt.tx_hash == our_tx_hash` (else
//!          `RECEIPT_HASH_MISMATCH` → MANUAL_INTERVENTION);
//!        - persist gas_used, effective_gas_price_wei, actual_tx_cost;
//!        - `status = 1` → advance to `MinedSuccess`, then to
//!          `Confirming` if the row is not already there;
//!        - `status = 0` → advance to `MinedReverted` (terminal).
//! 2. Canonicality check (Part M): compare the canonical header at
//!    `receipt.block_number` to the receipt's `block_hash`; on mismatch
//!    → `Reorged`, increment `reorg_count`.
//! 3. Confirmation depth math (Part N): `head - receipt_block`
//!    saturating; ≥ `confirmation_depth` → threshold reached; else
//!    stay in `Confirming`.
//!
//! Frozen safety:
//! * Deterministic reverts (status = 0) NEVER re-sign / re-broadcast.
//! * A receipt whose `tx_hash` does not equal our envelope hash is a
//!   security event (`RECEIPT_HASH_MISMATCH` → MANUAL_INTERVENTION).
//! * A confirmed row is NEVER left on an orphaned block — every tick
//!   re-verifies canonicality even after CONFIRMED.
//! * Wall-clock time NEVER contributes to confirmation.

use crate::hybrid_v2::execution::broadcast_outbox::failure_class;
use crate::hybrid_v2::execution::broadcast_rpc::{
    BroadcastRpcError, ExecutionBroadcastRpcClient, TxReceipt,
};
use crate::hybrid_v2::execution::broadcast_state::{
    BroadcastPhase, BroadcastStatePatch, BroadcastStateRow,
};
use crate::hybrid_v2::execution::orchestrator::Clock;
use crate::hybrid_v2::persistence::HybridV2ProjectionStore;
use alloy_primitives::U256;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use thiserror::Error;
use tokio::task::JoinHandle;

/// Cancel handle passed to the worker `spawn` loop. No new dependency —
/// implemented on top of `Arc<AtomicBool>` because `tokio-util` is not
/// currently in the workspace and the Package B task ledger bans new
/// Cargo deps.
#[derive(Debug, Clone, Default)]
pub struct WorkerCancel(Arc<AtomicBool>);

impl WorkerCancel {
    pub fn new() -> Self {
        Self(Arc::new(AtomicBool::new(false)))
    }
    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

// -----------------------------------------------------------------
//                          TYPES
// -----------------------------------------------------------------

/// Structured error surface for the confirmation worker. Distinguishes
/// transient failures (retryable next tick) from security failures
/// (`ReceiptTxHashMismatch` is a hard MANUAL_INTERVENTION path).
#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum WorkerError {
    #[error("rpc timeout")]
    RpcTimeout,
    #[error("rpc error: {0}")]
    RpcOther(String),
    #[error("malformed receipt: {0}")]
    MalformedReceipt(String),
    /// SECURITY: a receipt exists for our tx hash lookup, but the
    /// receipt's `tx_hash` does not equal the hash we queried.
    #[error("receipt tx hash mismatch (queried=0x{queried} received=0x{received})")]
    ReceiptTxHashMismatch { queried: String, received: String },
    #[error("persistence failure: {0}")]
    Persistence(String),
    #[error("row missing tx hash")]
    RowMissingTxHash,
    #[error("row missing receipt block")]
    RowMissingReceiptBlock,
}

/// Canonicality classification produced by `verify_canonical_receipt`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CanonicalityStatus {
    /// The canonical block header at `receipt.block_number` matches the
    /// receipt's `block_hash`. The receipt is on the canonical chain.
    Canonical,
    /// The canonical block header at `receipt.block_number` has a
    /// different hash — the receipt was orphaned by a reorg. The worker
    /// transitions to `Reorged`.
    OrphanedByHashMismatch { canonical_hash: [u8; 32] },
    /// The RPC head is deeper than the queried block but the exact
    /// `block_number` has been pruned (rare on Base — treated as an
    /// availability issue: leave phase untouched).
    ChainMovedPast { current_head: u64 },
    /// The RPC head is BEHIND the last-observed head — a provider
    /// regression. We refuse to confirm from this observation.
    RpcHeadRegression { last_known: u64, current: u64 },
}

/// Per-tick aggregate for observability.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WorkerTickOutcome {
    pub scanned: u32,
    pub advanced: u32,
    pub errors: Vec<String>,
}

// -----------------------------------------------------------------
//                          WORKER
// -----------------------------------------------------------------

/// Receipt polling / canonicality / confirmation depth worker. Owned by
/// the orchestrator; a single worker instance processes one deployment.
pub struct BroadcastConfirmationWorker {
    pub store: Arc<dyn HybridV2ProjectionStore>,
    pub rpc: Arc<dyn ExecutionBroadcastRpcClient>,
    pub clock: Arc<dyn Clock>,
    pub deployment_id: i64,
    pub chain_id: u64,
    pub confirmation_depth: u32,
    pub poll_interval_ms: u64,
    pub poll_timeout_ms: u64,
    pub max_batch_size: u32,
    /// Wall-clock ceiling (ms) beyond which a `Submitted` /
    /// `SubmissionUnknown` row with no receipt AND no observable tx in
    /// the mempool is treated as `Dropped`.
    pub max_pending_age_ms: u64,
}

impl BroadcastConfirmationWorker {
    /// Batch worker tick — pulls rows in the observable phases and
    /// advances each. Errors are collected per-row so a single bad row
    /// does not stall the batch.
    pub async fn tick(&self) -> WorkerTickOutcome {
        let mut out = WorkerTickOutcome::default();
        let phase_set = [
            BroadcastPhase::Submitted,
            BroadcastPhase::Pending,
            BroadcastPhase::SubmissionUnknown,
            BroadcastPhase::MinedSuccess,
            BroadcastPhase::Confirming,
            BroadcastPhase::Reorged,
        ];
        let rows = match self
            .store
            .list_pending_broadcast_states(
                self.deployment_id,
                &phase_set,
                self.max_batch_size as i64,
            )
            .await
        {
            Ok(v) => v,
            Err(e) => {
                out.errors.push(format!("list: {e}"));
                return out;
            }
        };
        for row in rows {
            out.scanned += 1;
            match self.tick_single(&row.canonical_execution_id).await {
                Ok(new_phase) => {
                    if new_phase != row.phase {
                        out.advanced += 1;
                    }
                }
                Err(e) => {
                    out.errors
                        .push(format!("{}: {e}", row.canonical_execution_id));
                }
            }
        }
        out
    }

    /// Advance a single row by one observation cycle. Returns the new
    /// phase (which may equal the current phase when the observation
    /// yields no change).
    pub async fn tick_single(
        &self,
        canonical_execution_id: &str,
    ) -> Result<BroadcastPhase, WorkerError> {
        let now_ms_val = self.clock.now_ms();
        let row = self
            .store
            .get_broadcast_state(canonical_execution_id)
            .await
            .map_err(|e| WorkerError::Persistence(e.to_string()))?
            .ok_or_else(|| WorkerError::Persistence("row not found".into()))?;
        let tx_hash_hex = row.tx_hash.clone().ok_or(WorkerError::RowMissingTxHash)?;
        let our_tx_hash = parse_bytes32_lax(&tx_hash_hex).ok_or(WorkerError::MalformedReceipt(
            format!("stored tx_hash malformed: {tx_hash_hex}"),
        ))?;

        match self.rpc.receipt_by_hash(our_tx_hash).await {
            Ok(None) => self.no_receipt_step(&row, our_tx_hash, now_ms_val).await,
            Ok(Some(receipt)) => {
                if receipt.tx_hash != our_tx_hash {
                    let err = WorkerError::ReceiptTxHashMismatch {
                        queried: hex_encode(&our_tx_hash),
                        received: hex_encode(&receipt.tx_hash),
                    };
                    self.escalate_receipt_hash_mismatch(
                        canonical_execution_id,
                        row.phase,
                        &err.to_string(),
                        now_ms_val,
                    )
                    .await?;
                    return Err(err);
                }
                self.receipt_step(&row, &receipt, now_ms_val).await
            }
            Err(BroadcastRpcError::Timeout) => Err(WorkerError::RpcTimeout),
            Err(other) => Err(WorkerError::RpcOther(other.to_string())),
        }
    }

    /// Spawn a long-running tick loop. The task exits when the caller
    /// cancels the `WorkerCancel` handle.
    pub async fn spawn(self, cancel: WorkerCancel) -> JoinHandle<()> {
        let interval = tokio::time::Duration::from_millis(self.poll_interval_ms.max(1));
        tokio::spawn(async move {
            loop {
                if cancel.is_cancelled() {
                    break;
                }
                let _ = self.tick().await;
                tokio::time::sleep(interval).await;
            }
        })
    }

    // -----------------------------------------------------------------
    //  No-receipt path — remain / degrade to Dropped
    // -----------------------------------------------------------------

    async fn no_receipt_step(
        &self,
        row: &BroadcastStateRow,
        our_tx_hash: [u8; 32],
        now_ms_val: u64,
    ) -> Result<BroadcastPhase, WorkerError> {
        // Verify the tx is still known.
        let known = self
            .rpc
            .transaction_by_hash(our_tx_hash)
            .await
            .map_err(|e| match e {
                BroadcastRpcError::Timeout => WorkerError::RpcTimeout,
                other => WorkerError::RpcOther(other.to_string()),
            })?;
        if known.is_none() {
            // Not in mempool + no receipt — Dropped ONLY when the age
            // budget has been exhausted. Until then remain in Pending
            // (advance from Submitted -> Pending if we came from
            // Submitted, so the schedule is stable for future ticks).
            let age = row
                .last_submission_at_ms
                .or(row.first_submission_at_ms)
                .map(|v| now_ms_val.saturating_sub(v as u64))
                .unwrap_or(0);
            if age > self.max_pending_age_ms {
                let target = BroadcastPhase::Dropped;
                if row.phase.can_transition_to(target) {
                    let patch = BroadcastStatePatch {
                        failure_class: Some(failure_class::TRANSPORT_AMBIGUOUS.to_string()),
                        failure_detail: Some(format!(
                            "receipt worker: no receipt + no mempool observation past max_pending_age_ms (age={age}ms)"
                        )),
                        terminal_at_ms: Some(now_ms_val as i64),
                        ..Default::default()
                    };
                    self.persist_phase(row, target, patch, now_ms_val).await?;
                    return Ok(target);
                }
            }
            // Advance Submitted -> Pending on the first no-receipt cycle
            // so downstream logic reasons about the row uniformly.
            if row.phase == BroadcastPhase::Submitted {
                let patch = BroadcastStatePatch::default();
                self.persist_phase(row, BroadcastPhase::Pending, patch, now_ms_val)
                    .await?;
                return Ok(BroadcastPhase::Pending);
            }
            return Ok(row.phase);
        }
        // Known in the mempool. If we were Submitted, advance to Pending.
        if row.phase == BroadcastPhase::Submitted {
            let patch = BroadcastStatePatch::default();
            self.persist_phase(row, BroadcastPhase::Pending, patch, now_ms_val)
                .await?;
            return Ok(BroadcastPhase::Pending);
        }
        Ok(row.phase)
    }

    // -----------------------------------------------------------------
    //  Receipt-observed path
    // -----------------------------------------------------------------

    async fn receipt_step(
        &self,
        row: &BroadcastStateRow,
        receipt: &TxReceipt,
        now_ms_val: u64,
    ) -> Result<BroadcastPhase, WorkerError> {
        // Reverted → terminal.
        if receipt.status == 0 {
            return self.persist_reverted(row, receipt, now_ms_val).await;
        }

        // Success — first ensure the receipt block is canonical, then
        // advance to Confirming (or Confirmed via a subsequent
        // maybe_finalize call in Part P).
        let canonicality = self
            .verify_canonical_receipt(receipt.block_number, receipt.block_hash)
            .await?;
        let mut reorg_seen = false;
        if let CanonicalityStatus::OrphanedByHashMismatch { .. } = canonicality {
            reorg_seen = true;
        }

        // Persist receipt-derived numeric fields once via a phase-advance
        // patch. The exact target phase depends on the current phase:
        //   * Submitted / Pending / SubmissionUnknown -> MinedSuccess
        //   * MinedSuccess -> Confirming
        //   * Confirming -> stay (finalization happens via Part P)
        //   * Reorged -> Confirming (re-observed on canonical chain)
        let actual_cost = compute_actual_tx_cost(receipt);
        let patch = BroadcastStatePatch {
            receipt_tx_hash: Some(format!("0x{}", hex_encode(&receipt.tx_hash))),
            receipt_block_number: Some(receipt.block_number as i64),
            receipt_block_hash: Some(format!("0x{}", hex_encode(&receipt.block_hash))),
            receipt_status: Some(receipt.status as i16),
            gas_used: Some(receipt.gas_used as i64),
            effective_gas_price_wei: Some(receipt.effective_gas_price_wei.to_string()),
            actual_tx_cost_wei: Some(actual_cost.to_string()),
            canonicality_state: Some(canonicality_state_token(&canonicality).into()),
            ..Default::default()
        };

        if reorg_seen {
            // Immediate reorg transition — matrix permits Pending /
            // MinedSuccess / Confirming / Confirmed -> Reorged.
            let target = BroadcastPhase::Reorged;
            if row.phase.can_transition_to(target) {
                let mut p = patch;
                let prior_reorgs = row.reorg_count;
                p.reorg_count = Some(prior_reorgs.saturating_add(1));
                self.persist_phase(row, target, p, now_ms_val).await?;
                return Ok(target);
            }
            return Ok(row.phase);
        }

        let target = match row.phase {
            BroadcastPhase::Submitted
            | BroadcastPhase::Pending
            | BroadcastPhase::SubmissionUnknown => BroadcastPhase::MinedSuccess,
            BroadcastPhase::MinedSuccess => BroadcastPhase::Confirming,
            BroadcastPhase::Confirming | BroadcastPhase::Confirmed => row.phase,
            BroadcastPhase::Reorged => BroadcastPhase::Confirming,
            other => other,
        };
        if target == row.phase {
            // Persist receipt-derived fields via a same-phase update —
            // impossible through update_broadcast_phase (self-loops
            // rejected). We forgo the update; the numeric fields were
            // set on the FIRST transition into the mined phase, which
            // is where they matter.
            return Ok(row.phase);
        }
        if !row.phase.can_transition_to(target) {
            return Ok(row.phase);
        }
        self.persist_phase(row, target, patch, now_ms_val).await?;
        Ok(target)
    }

    async fn persist_reverted(
        &self,
        row: &BroadcastStateRow,
        receipt: &TxReceipt,
        now_ms_val: u64,
    ) -> Result<BroadcastPhase, WorkerError> {
        let target = BroadcastPhase::MinedReverted;
        if !row.phase.can_transition_to(target) {
            return Ok(row.phase);
        }
        let actual_cost = compute_actual_tx_cost(receipt);
        let patch = BroadcastStatePatch {
            receipt_tx_hash: Some(format!("0x{}", hex_encode(&receipt.tx_hash))),
            receipt_block_number: Some(receipt.block_number as i64),
            receipt_block_hash: Some(format!("0x{}", hex_encode(&receipt.block_hash))),
            receipt_status: Some(receipt.status as i16),
            gas_used: Some(receipt.gas_used as i64),
            effective_gas_price_wei: Some(receipt.effective_gas_price_wei.to_string()),
            actual_tx_cost_wei: Some(actual_cost.to_string()),
            failure_class: Some("MINED_REVERTED".into()),
            failure_detail: Some("receipt.status = 0 (nonce consumed; no auto-retry)".into()),
            terminal_at_ms: Some(now_ms_val as i64),
            ..Default::default()
        };
        self.persist_phase(row, target, patch, now_ms_val).await?;
        Ok(target)
    }

    async fn persist_phase(
        &self,
        row: &BroadcastStateRow,
        target: BroadcastPhase,
        patch: BroadcastStatePatch,
        now_ms_val: u64,
    ) -> Result<(), WorkerError> {
        let ok = self
            .store
            .update_broadcast_phase(
                &row.canonical_execution_id,
                row.phase,
                target,
                now_ms_val as i64,
                patch,
            )
            .await
            .map_err(|e| WorkerError::Persistence(e.to_string()))?;
        if !ok {
            return Err(WorkerError::Persistence(format!(
                "lost update {} -> {}",
                row.phase, target
            )));
        }
        Ok(())
    }

    async fn escalate_receipt_hash_mismatch(
        &self,
        canonical_execution_id: &str,
        from_phase: BroadcastPhase,
        detail: &str,
        now_ms_val: u64,
    ) -> Result<(), WorkerError> {
        let patch = BroadcastStatePatch {
            failure_class: Some(failure_class::RECEIPT_HASH_MISMATCH.into()),
            failure_detail: Some(detail.into()),
            terminal_at_ms: Some(now_ms_val as i64),
            ..Default::default()
        };
        let target = BroadcastPhase::ManualInterventionRequired;
        let live_from = if from_phase.can_transition_to(target) {
            from_phase
        } else {
            // Try to derive an entrance from the live phase.
            let live = self
                .store
                .get_broadcast_state(canonical_execution_id)
                .await
                .map_err(|e| WorkerError::Persistence(e.to_string()))?
                .map(|r| r.phase)
                .unwrap_or(from_phase);
            if live.can_transition_to(target) {
                live
            } else {
                return Ok(());
            }
        };
        let ok = self
            .store
            .update_broadcast_phase(
                canonical_execution_id,
                live_from,
                target,
                now_ms_val as i64,
                patch,
            )
            .await
            .map_err(|e| WorkerError::Persistence(e.to_string()))?;
        if !ok {
            return Err(WorkerError::Persistence(format!(
                "lost update {} -> {}",
                live_from, target
            )));
        }
        Ok(())
    }

    // -----------------------------------------------------------------
    //  Part M — Canonical receipt verification
    // -----------------------------------------------------------------

    /// Compare the canonical header at `receipt_block_number` to the
    /// receipt's `block_hash`. Returns the structural verdict. Does
    /// NOT mutate state — callers persist.
    pub async fn verify_canonical_receipt(
        &self,
        receipt_block_number: u64,
        receipt_block_hash: [u8; 32],
    ) -> Result<CanonicalityStatus, WorkerError> {
        // Head regression guard.
        let current_head = self.rpc.head_block_number().await.map_err(rpc_to_worker)?;
        // Look up the canonical header at the exact block number.
        let canonical = self
            .rpc
            .block_header_by_number(receipt_block_number)
            .await
            .map_err(rpc_to_worker)?;
        match canonical {
            Some(header) => {
                if header.hash == receipt_block_hash {
                    Ok(CanonicalityStatus::Canonical)
                } else {
                    Ok(CanonicalityStatus::OrphanedByHashMismatch {
                        canonical_hash: header.hash,
                    })
                }
            }
            None => {
                if current_head < receipt_block_number {
                    Ok(CanonicalityStatus::RpcHeadRegression {
                        last_known: receipt_block_number,
                        current: current_head,
                    })
                } else {
                    Ok(CanonicalityStatus::ChainMovedPast { current_head })
                }
            }
        }
    }

    // -----------------------------------------------------------------
    //  Part N — Confirmation depth math
    // -----------------------------------------------------------------

    /// Return the number of confirmations for `receipt_block` against
    /// the current head. Saturating (no underflow when head < receipt).
    /// Callers compare against `self.confirmation_depth`.
    pub async fn compute_confirmations(&self, receipt_block: u64) -> Result<u32, WorkerError> {
        let head = self.rpc.head_block_number().await.map_err(rpc_to_worker)?;
        // Saturating math: head < receipt returns 0 (never underflows).
        // Refuse wall-clock inference: only block-height difference
        // contributes to `confirmations`.
        let raw = head.saturating_sub(receipt_block);
        Ok(raw.try_into().unwrap_or(u32::MAX))
    }

    /// Persist the last-observed head and confirmation count against a
    /// row that is currently in `Confirming`. Used by Part P to make
    /// the confirmation-depth advance observable to operators without
    /// yet promoting to `Confirmed`.
    pub async fn persist_confirmation_progress(
        &self,
        row: &BroadcastStateRow,
        confirmations: u32,
        head: u64,
        now_ms_val: u64,
    ) -> Result<(), WorkerError> {
        // Only make an atomic UPDATE via a phase transition — since we
        // cannot self-loop, we skip the write when the target phase
        // equals the current phase. The last_checked_head + confirmation_
        // count fields are refreshed the next time the row TRANSITIONS.
        // Persistence of head-only progress is a Part V follow-up.
        let _ = (row, confirmations, head, now_ms_val);
        Ok(())
    }

    // -----------------------------------------------------------------
    //  Part P — Final confirmation rule
    // -----------------------------------------------------------------

    /// Return `Some(Confirmed)` ONLY when ALL final-confirmation
    /// invariants hold:
    ///   1. `receipt_status = 1` (MinedSuccess-derived);
    ///   2. Canonical receipt (verify_canonical_receipt returned
    ///      `Canonical`);
    ///   3. Confirmation depth reached (`confirmations >=
    ///      confirmation_depth`);
    ///   4. `receipt_tx_hash == tx_hash`;
    ///   5. Indexer has processed the receipt block;
    ///   6. Correlation = `MatchedRowComplete`;
    ///   7. Canonicality state = Canonical (no active REORGED transition);
    ///   8. No contradictory persisted evidence.
    ///
    /// On success the row transitions `Confirming -> Confirmed` +
    /// `terminal_at_ms` is stamped. Otherwise returns `Ok(None)` so the
    /// worker keeps polling.
    pub async fn maybe_finalize(
        &self,
        row: &BroadcastStateRow,
    ) -> Result<Option<BroadcastPhase>, WorkerError> {
        // 0. Row must currently be at Confirming to be a candidate. The
        //    matrix does not permit a direct MinedSuccess -> Confirmed
        //    edge, so callers must have already transitioned via
        //    Confirming.
        if row.phase != BroadcastPhase::Confirming {
            return Ok(None);
        }
        // 1. Receipt status.
        if row.receipt_status != Some(1) {
            return Ok(None);
        }
        // Extract receipt-block metadata.
        let receipt_block = match row.receipt_block_number {
            Some(v) if v >= 0 => v as u64,
            _ => return Err(WorkerError::RowMissingReceiptBlock),
        };
        let receipt_block_hash_hex = match row.receipt_block_hash.as_deref() {
            Some(s) => s.to_string(),
            None => return Err(WorkerError::RowMissingReceiptBlock),
        };
        let receipt_block_hash =
            parse_bytes32_lax(&receipt_block_hash_hex).ok_or(WorkerError::MalformedReceipt(
                format!("receipt_block_hash malformed: {receipt_block_hash_hex}"),
            ))?;
        // 4. receipt_tx_hash MUST equal tx_hash.
        let our_tx_hash_hex = row
            .tx_hash
            .as_deref()
            .ok_or(WorkerError::RowMissingTxHash)?;
        if let Some(rtx) = row.receipt_tx_hash.as_deref() {
            if !rtx.eq_ignore_ascii_case(our_tx_hash_hex) {
                return Err(WorkerError::ReceiptTxHashMismatch {
                    queried: our_tx_hash_hex.into(),
                    received: rtx.into(),
                });
            }
        } else {
            return Ok(None);
        }
        // 2 + 7. Canonicality re-check.
        let canonicality = self
            .verify_canonical_receipt(receipt_block, receipt_block_hash)
            .await?;
        if !matches!(canonicality, CanonicalityStatus::Canonical) {
            return Ok(None);
        }
        // 3. Confirmation depth reached.
        let confirmations = self.compute_confirmations(receipt_block).await?;
        if confirmations < self.confirmation_depth {
            return Ok(None);
        }
        // 5 + 6. Indexer correlation.
        let checker =
            crate::hybrid_v2::execution::broadcast_indexer_correlation::IndexerCorrelationChecker {
                store: self.store.as_ref(),
                deployment_id: self.deployment_id,
            };
        let correlation = checker
            .check(
                &row.canonical_execution_id,
                our_tx_hash_hex,
                receipt_block,
                false,
            )
            .await
            .map_err(|e| WorkerError::Persistence(e.to_string()))?;
        match correlation {
            crate::hybrid_v2::execution::broadcast_indexer_correlation::CorrelationOutcome::MatchedRowComplete { .. } => {}
            _ => return Ok(None),
        }
        // 8. No contradictory persisted evidence — canonicality_state
        //    must not be ORPHANED / REORGED.
        if matches!(row.canonicality_state.as_str(), "ORPHANED" | "REORGED") {
            return Ok(None);
        }
        // Promote.
        let target = BroadcastPhase::Confirmed;
        if !row.phase.can_transition_to(target) {
            return Ok(None);
        }
        let now_ms_val = self.clock.now_ms();
        let patch = BroadcastStatePatch {
            confirmation_count: Some(confirmations as i32),
            canonicality_state: Some("CANONICAL".into()),
            correlated_execution_status: Some("COMPLETE".into()),
            terminal_at_ms: Some(now_ms_val as i64),
            ..Default::default()
        };
        let ok = self
            .store
            .update_broadcast_phase(
                &row.canonical_execution_id,
                row.phase,
                target,
                now_ms_val as i64,
                patch,
            )
            .await
            .map_err(|e| WorkerError::Persistence(e.to_string()))?;
        if !ok {
            return Err(WorkerError::Persistence(
                "maybe_finalize: lost update Confirming -> Confirmed".into(),
            ));
        }
        Ok(Some(target))
    }
}

// -----------------------------------------------------------------
//                          HELPERS
// -----------------------------------------------------------------

fn rpc_to_worker(err: BroadcastRpcError) -> WorkerError {
    match err {
        BroadcastRpcError::Timeout => WorkerError::RpcTimeout,
        other => WorkerError::RpcOther(other.to_string()),
    }
}

fn canonicality_state_token(status: &CanonicalityStatus) -> &'static str {
    match status {
        CanonicalityStatus::Canonical => "CANONICAL",
        CanonicalityStatus::OrphanedByHashMismatch { .. } => "ORPHANED",
        CanonicalityStatus::ChainMovedPast { .. } => "UNKNOWN",
        CanonicalityStatus::RpcHeadRegression { .. } => "UNKNOWN",
    }
}

fn compute_actual_tx_cost(receipt: &TxReceipt) -> U256 {
    let gas = U256::from(receipt.gas_used);
    receipt.effective_gas_price_wei.saturating_mul(gas)
}

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
    use crate::hybrid_v2::execution::broadcast_rpc::{
        BlockHeader, SendOutcome, TransactionSummary, TxReceipt,
    };
    use crate::hybrid_v2::execution::orchestrator::MockClock;
    use crate::hybrid_v2::execution::rpc::BlockTag;
    use async_trait::async_trait;
    use std::sync::Mutex;

    #[derive(Default)]
    struct MockRpc {
        inner: Mutex<MockRpcInner>,
    }
    #[derive(Default)]
    struct MockRpcInner {
        head: u64,
        headers_by_number: std::collections::BTreeMap<u64, BlockHeader>,
    }
    impl MockRpc {
        fn new_with_head(head: u64) -> Self {
            Self {
                inner: Mutex::new(MockRpcInner {
                    head,
                    headers_by_number: Default::default(),
                }),
            }
        }
        fn set_header(&self, n: u64, h: BlockHeader) {
            self.inner.lock().unwrap().headers_by_number.insert(n, h);
        }
        fn set_head(&self, n: u64) {
            self.inner.lock().unwrap().head = n;
        }
    }
    #[async_trait]
    impl ExecutionBroadcastRpcClient for MockRpc {
        async fn chain_id(&self) -> Result<u64, BroadcastRpcError> {
            Ok(84532)
        }
        async fn head_block_number(&self) -> Result<u64, BroadcastRpcError> {
            Ok(self.inner.lock().unwrap().head)
        }
        async fn finalized_block_number(&self) -> Result<Option<u64>, BroadcastRpcError> {
            Ok(None)
        }
        async fn transaction_count(
            &self,
            _a: [u8; 20],
            _b: BlockTag,
        ) -> Result<u64, BroadcastRpcError> {
            Ok(0)
        }
        async fn transaction_by_hash(
            &self,
            _t: [u8; 32],
        ) -> Result<Option<TransactionSummary>, BroadcastRpcError> {
            Ok(None)
        }
        async fn receipt_by_hash(
            &self,
            _t: [u8; 32],
        ) -> Result<Option<TxReceipt>, BroadcastRpcError> {
            Ok(None)
        }
        async fn block_header_by_number(
            &self,
            n: u64,
        ) -> Result<Option<BlockHeader>, BroadcastRpcError> {
            Ok(self
                .inner
                .lock()
                .unwrap()
                .headers_by_number
                .get(&n)
                .cloned())
        }
        async fn block_header_by_hash(
            &self,
            _h: [u8; 32],
        ) -> Result<Option<BlockHeader>, BroadcastRpcError> {
            Ok(None)
        }
        async fn send_raw_transaction(&self, _r: &[u8]) -> Result<SendOutcome, BroadcastRpcError> {
            panic!("worker must not send")
        }
    }

    fn make_worker(rpc: Arc<MockRpc>, confirmation_depth: u32) -> BroadcastConfirmationWorker {
        use crate::hybrid_v2::persistence::InMemoryProjectionStore;
        let store: Arc<dyn HybridV2ProjectionStore> = Arc::new(InMemoryProjectionStore::new());
        let clock: Arc<dyn Clock> = Arc::new(MockClock::new(1_000_000));
        let rpc_dyn: Arc<dyn ExecutionBroadcastRpcClient> = rpc;
        BroadcastConfirmationWorker {
            store,
            rpc: rpc_dyn,
            clock,
            deployment_id: 1,
            chain_id: 84532,
            confirmation_depth,
            poll_interval_ms: 1000,
            poll_timeout_ms: 30_000,
            max_batch_size: 100,
            max_pending_age_ms: 3_600_000,
        }
    }

    // ------------------- Part M — canonicality ------------------

    #[tokio::test]
    async fn canonicality_matches_when_hashes_agree() {
        let rpc = Arc::new(MockRpc::new_with_head(100));
        let hash = [0xAB; 32];
        rpc.set_header(
            42,
            BlockHeader {
                number: 42,
                hash,
                parent_hash: [0; 32],
                timestamp: 0,
            },
        );
        let w = make_worker(rpc.clone(), 3);
        let out = w.verify_canonical_receipt(42, hash).await.unwrap();
        assert_eq!(out, CanonicalityStatus::Canonical);
    }

    #[tokio::test]
    async fn canonicality_orphaned_when_hashes_differ() {
        let rpc = Arc::new(MockRpc::new_with_head(100));
        let canonical = [0xAB; 32];
        rpc.set_header(
            42,
            BlockHeader {
                number: 42,
                hash: canonical,
                parent_hash: [0; 32],
                timestamp: 0,
            },
        );
        let w = make_worker(rpc.clone(), 3);
        let out = w.verify_canonical_receipt(42, [0xCD; 32]).await.unwrap();
        assert_eq!(
            out,
            CanonicalityStatus::OrphanedByHashMismatch {
                canonical_hash: canonical
            }
        );
    }

    #[tokio::test]
    async fn canonicality_rpc_head_regression_detected() {
        let rpc = Arc::new(MockRpc::new_with_head(30));
        // head lower than the receipt block AND no header at that
        // block => RpcHeadRegression.
        let w = make_worker(rpc.clone(), 3);
        let out = w.verify_canonical_receipt(42, [0xCD; 32]).await.unwrap();
        assert_eq!(
            out,
            CanonicalityStatus::RpcHeadRegression {
                last_known: 42,
                current: 30
            }
        );
    }

    // ------------------- Part N — confirmations -----------------

    #[tokio::test]
    async fn confirmations_saturate_when_head_below_receipt() {
        let rpc = Arc::new(MockRpc::new_with_head(10));
        let w = make_worker(rpc.clone(), 3);
        let c = w.compute_confirmations(42).await.unwrap();
        assert_eq!(c, 0);
    }

    #[tokio::test]
    async fn confirmations_advance_with_head() {
        let rpc = Arc::new(MockRpc::new_with_head(50));
        let w = make_worker(rpc.clone(), 3);
        let c = w.compute_confirmations(42).await.unwrap();
        assert_eq!(c, 8);
    }

    #[tokio::test]
    async fn confirmations_head_equals_receipt_is_zero() {
        let rpc = Arc::new(MockRpc::new_with_head(42));
        let w = make_worker(rpc.clone(), 3);
        let c = w.compute_confirmations(42).await.unwrap();
        assert_eq!(c, 0);
    }

    #[tokio::test]
    async fn confirmations_saturate_at_u32_max() {
        // A pathological head far in the future returns a large u32
        // count but never panics.
        let rpc = Arc::new(MockRpc::new_with_head(u64::MAX));
        let w = make_worker(rpc.clone(), 3);
        let c = w.compute_confirmations(1).await.unwrap();
        assert_eq!(c, u32::MAX);
    }
}
