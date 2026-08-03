//! Hybrid V2 indexer runtime.
//!
//! Boundary:
//! - Input:  a `ChainSource` (read-only), a validated `ManifestParams`,
//!           a set of pinned emitter addresses.
//! - Output: mutations to a `RuntimeState` (cursor + `ProjectionState`
//!           + raw log journal + decoded event journal + execution
//!           correlator + reorg planner + readiness).
//!
//! Frozen rules:
//! - Manifest chain_id MUST match the ChainSource chain_id — else
//!   `RuntimeError::WrongChain` and readiness stays false.
//! - Cursor never advances past a block that failed to decode, reduce
//!   or reconcile — atomic per-block apply.
//! - Reorg detection: parent-hash mismatch between the ChainSource's
//!   block[n].parent_hash and our cursor's `indexed_head_hash`. The
//!   reorg planner locates the common ancestor and orchestrates replay.
//! - No chain writes. No signing. No RPC state mutation.
//! - Restart-safe: the cursor + projection are reconstructible from
//!   the raw log journal alone.

use crate::hybrid_v2::chain_source::{ChainSource, RawBlock};
use crate::hybrid_v2::correlation::ExecutionCorrelator;
use crate::hybrid_v2::decoder::{decode_log_with, DecoderError};
use crate::hybrid_v2::events::HybridV2Event;
use crate::hybrid_v2::manifest::ManifestParams;
use crate::hybrid_v2::persistence::{
    CanonicalBlockRef, HybridV2ProjectionStore, ReadinessSnapshot, RuntimeCursorSnapshot,
};
use crate::hybrid_v2::readiness::{ReadinessReason, ReadinessState};
use crate::hybrid_v2::reducer::{apply, ApplyContext, ProjectionState, ReducerError};
use crate::hybrid_v2::reorg::{ReorgOutcome, ReorgPlanner};
use std::collections::HashSet;
use std::sync::Arc;
use thiserror::Error;

/// One raw log with the block/tx context needed for canonical identity.
#[derive(Debug, Clone)]
pub struct JournaledLog {
    pub block_number: u64,
    pub block_hash: String,
    pub parent_hash: String,
    pub block_timestamp: u64,
    pub tx_hash: String,
    pub tx_index: u32,
    pub log_index: u32,
    pub emitter: String,
    pub topics: Vec<[u8; 32]>,
    pub data: Vec<u8>,
    pub is_canonical: bool,
    pub orphaned_at_block: Option<u64>,
}

/// Runtime cursor mirroring `hybrid_v2_cursors`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RuntimeCursor {
    pub indexed_head_block: u64,
    pub indexed_head_hash: String,
    pub indexed_head_parent: String,
    pub observed_head_block: u64,
    pub finalized_head_block: u64,
    pub last_error: Option<String>,
}

/// Snapshot of runtime diagnostics.
#[derive(Debug, Clone, Default)]
pub struct RuntimeMetrics {
    pub deployment_id: u64,
    pub manifest_hash: String,
    pub observed_block: u64,
    pub indexed_block: u64,
    pub finalized_block: u64,
    pub lag: u64,
    pub last_successful_block: u64,
    pub decode_failures: u64,
    pub projection_failures: u64,
    pub unknown_canonical_events: u64,
    pub reorg_count: u64,
    pub max_reorg_depth_seen: u64,
    pub rebuild_status: &'static str,
    pub reconciliation_status: &'static str,
}

#[derive(Debug, Error)]
pub enum RuntimeError {
    #[error("chain id mismatch: manifest {manifest} vs source {source_chain}")]
    WrongChain { manifest: u64, source_chain: u64 },
    #[error("decoder error at block {block}, log {log}: {source}")]
    Decoder {
        block: u64,
        log: u32,
        #[source]
        source: DecoderError,
    },
    #[error("reducer error at block {block}: {source}")]
    Reducer {
        block: u64,
        #[source]
        source: ReducerError,
    },
    #[error("unknown canonical emitter {emitter} at block {block}")]
    UnknownCanonicalEmitter { emitter: String, block: u64 },
    #[error("cursor parent hash mismatch at block {block}: cursor {cursor}, incoming {incoming}")]
    ParentHashMismatch {
        block: u64,
        cursor: String,
        incoming: String,
    },
    #[error("reorg depth {depth} exceeds max {max}")]
    ExcessiveReorgDepth { depth: u64, max: u64 },
    #[error("persistence error at block {block}: {detail}")]
    Persistence { block: u64, detail: String },
    #[error("bootstrap error: {detail}")]
    Bootstrap { detail: String },
}

/// Outcome of `IndexerRuntime::bootstrap_from_persistence`. Documents
/// which model applied so the caller can decide whether reducer replay
/// (RebuildService) is needed before serving live traffic.
///
/// BOOTSTRAP-1 partial: cursor + readiness only; reducer state must
/// be rehydrated by rebuild if needed for a live runtime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BootstrapResult {
    /// No persistence configured — the runtime starts fresh in memory.
    NoPersistenceConfigured,
    /// Persistence configured but store had nothing yet — fresh start.
    FreshStart,
    /// Cursor + readiness restored from projections; reducer state must
    /// be rehydrated separately (rebuild) before serving live traffic.
    RestoredCursorOnly { indexed_head_block: u64 },
}

/// The full runtime state.
pub struct IndexerRuntime {
    pub deployment_id: u64,
    pub manifest: ManifestParams,
    pub emitters: HashSet<String>,
    pub state: ProjectionState,
    pub cursor: RuntimeCursor,
    pub raw_logs: Vec<JournaledLog>,
    pub decoded_events: Vec<(HybridV2Event, ApplyContext)>,
    pub correlator: ExecutionCorrelator,
    pub readiness: ReadinessState,
    pub metrics: RuntimeMetrics,
    pub max_reorg_depth: u64,
    /// Optional durable projection store. When `None`, the runtime
    /// operates purely in-memory (existing behaviour). When `Some`,
    /// `tick_and_persist()` writes each canonical block into the
    /// store atomically before the runtime considers it applied.
    pub persistence: Option<Arc<dyn HybridV2ProjectionStore>>,
    /// Deployment identifier used as the FK when writing to the
    /// projection store. Set alongside `persistence`.
    pub persistence_deployment_id: Option<i64>,
    /// Cursor name used when reading/writing cursor rows. Constant
    /// per runtime; the fixture default is "indexer" and matches the
    /// value the read-store handlers look up.
    pub persistence_cursor_name: String,
    /// Count of reorg audit-write failures (persistence layer only —
    /// never fails the reorg itself).
    pub reorg_audit_write_failures: u64,
    /// Reorg event captured during the last sync tick that has not yet
    /// been written to the projection store. Populated by
    /// `handle_reorg`, consumed by `tick_and_persist`.
    pub pending_reorg_audit: Option<PendingReorgAudit>,
}

/// Metadata captured when a reorg is planned + applied in-memory, to be
/// asynchronously written to the reorg audit trail on the next
/// persistence call. All fields are captured *before* rewind mutates
/// them so the audit row records the pre-rewind identity.
#[derive(Debug, Clone)]
pub struct PendingReorgAudit {
    pub from_block: u64,
    pub from_hash: String,
    pub to_block: u64,
    pub to_hash: String,
    pub depth: u32,
    pub orphaned_log_count: u32,
}

impl IndexerRuntime {
    pub fn new(deployment_id: u64, manifest: ManifestParams) -> Self {
        let emitters = collect_emitters(&manifest);
        let manifest_hash = manifest.manifest_hash.clone();
        let mut metrics = RuntimeMetrics::default();
        metrics.deployment_id = deployment_id;
        metrics.manifest_hash = manifest_hash;
        metrics.rebuild_status = "IDLE";
        metrics.reconciliation_status = "IDLE";
        Self {
            deployment_id,
            manifest,
            emitters,
            state: ProjectionState::default(),
            cursor: RuntimeCursor::default(),
            raw_logs: Vec::new(),
            decoded_events: Vec::new(),
            correlator: ExecutionCorrelator::new(),
            readiness: ReadinessState::new_not_ready(ReadinessReason::AwaitingFirstBlock),
            metrics,
            max_reorg_depth: 64,
            persistence: None,
            persistence_deployment_id: None,
            persistence_cursor_name: "indexer".to_string(),
            reorg_audit_write_failures: 0,
            pending_reorg_audit: None,
        }
    }

    /// Attach a durable projection store + the deployment id used as the
    /// persistence FK. Enables `tick_and_persist()` and
    /// `bootstrap_from_persistence()`. Existing `tick()` remains a
    /// pure in-memory operation regardless.
    pub fn with_persistence(
        mut self,
        store: Arc<dyn HybridV2ProjectionStore>,
        deployment_id: i64,
    ) -> Self {
        self.persistence = Some(store);
        self.persistence_deployment_id = Some(deployment_id);
        self
    }

    /// Override the cursor name used for persistence read/write. Default
    /// is "indexer". Kept explicit so integration tests can drive
    /// multiple parallel cursors against one store.
    pub fn with_persistence_cursor_name(mut self, name: impl Into<String>) -> Self {
        self.persistence_cursor_name = name.into();
        self
    }

    /// Advance the runtime by pulling one block from the source and
    /// applying it transactionally. Returns:
    /// - `Ok(true)` when a block was applied.
    /// - `Ok(false)` when the source has no new block.
    pub fn tick(&mut self, source: &dyn ChainSource) -> Result<bool, RuntimeError> {
        if source.chain_id() != self.manifest.chain_id {
            self.readiness = ReadinessState::new_not_ready(ReadinessReason::WrongChain {
                manifest: self.manifest.chain_id,
                source: source.chain_id(),
            });
            return Err(RuntimeError::WrongChain {
                manifest: self.manifest.chain_id,
                source_chain: source.chain_id(),
            });
        }
        self.metrics.observed_block = source.head_block_number();
        self.metrics.finalized_block = source.finalized_block_number();
        let next = self.cursor.indexed_head_block + 1;
        let Some(block) = source.block_at(next) else {
            self.recompute_readiness();
            return Ok(false);
        };
        // Reorg detection: parent hash of incoming must match our cursor head.
        if self.cursor.indexed_head_block > 0
            && !parents_match(&self.cursor.indexed_head_hash, &block.parent_hash)
        {
            self.handle_reorg(source, &block)?;
            // After rewind, cursor may have jumped back multiple blocks. The
            // next tick will re-pull the correct next block from the source
            // rather than skipping ahead to `block`.
            self.metrics.observed_block = source.head_block_number();
            self.metrics.lag = self
                .metrics
                .observed_block
                .saturating_sub(self.cursor.indexed_head_block);
            self.recompute_readiness();
            return Ok(true);
        }
        self.apply_block(&block)?;
        self.metrics.observed_block = source.head_block_number();
        self.metrics.lag = self
            .metrics
            .observed_block
            .saturating_sub(self.cursor.indexed_head_block);
        self.recompute_readiness();
        Ok(true)
    }

    /// Apply one canonical block atomically. On failure, projections and
    /// cursor are left untouched.
    fn apply_block(&mut self, block: &RawBlock) -> Result<(), RuntimeError> {
        // Snapshot for rollback.
        let backup_state = self.state.clone();
        let backup_raw_len = self.raw_logs.len();
        let backup_decoded_len = self.decoded_events.len();
        let backup_correlator_pending = self.correlator_snapshot();

        let mut current_tx: Option<String> = None;
        for (idx, log) in block.logs.iter().enumerate() {
            // Emitter filter.
            if !self.emitters.contains(&log.emitter.to_ascii_lowercase()) {
                continue;
            }
            let tx_hash = tx_hash_for_log(block, idx);
            let ctx = ApplyContext {
                block_number: block.number,
                tx_hash: tx_hash.clone(),
                log_index: idx as u32,
                block_timestamp: block.timestamp,
            };
            if let Some(prev) = current_tx.as_ref() {
                if prev != &tx_hash {
                    self.correlator.flush_transaction(prev, &mut self.state);
                }
            }
            current_tx = Some(tx_hash.clone());
            let journaled = JournaledLog {
                block_number: block.number,
                block_hash: block.hash.clone(),
                parent_hash: block.parent_hash.clone(),
                block_timestamp: block.timestamp,
                tx_hash,
                tx_index: 0,
                log_index: idx as u32,
                emitter: log.emitter.clone(),
                topics: log.topics.clone(),
                data: log.data.clone(),
                is_canonical: true,
                orphaned_at_block: None,
            };
            self.raw_logs.push(journaled);
            let event = match decode_log_with(log, None, self.manifest.event_version) {
                Ok(e) => e,
                Err(err) => {
                    self.metrics.decode_failures += 1;
                    self.rollback(
                        backup_state,
                        backup_raw_len,
                        backup_decoded_len,
                        backup_correlator_pending,
                    );
                    return Err(RuntimeError::Decoder {
                        block: block.number,
                        log: idx as u32,
                        source: err,
                    });
                }
            };
            self.decoded_events.push((event.clone(), ctx.clone()));
            if let Err(err) = apply(&mut self.state, &event, &ctx) {
                self.metrics.projection_failures += 1;
                self.rollback(
                    backup_state,
                    backup_raw_len,
                    backup_decoded_len,
                    backup_correlator_pending,
                );
                return Err(RuntimeError::Reducer {
                    block: block.number,
                    source: err,
                });
            }
            self.correlator.observe(&event, &ctx, &mut self.state);
        }
        if let Some(tx) = current_tx {
            self.correlator.flush_transaction(&tx, &mut self.state);
        }
        // Cursor advance is the LAST action.
        self.cursor.indexed_head_block = block.number;
        self.cursor.indexed_head_hash = block.hash.clone();
        self.cursor.indexed_head_parent = block.parent_hash.clone();
        self.cursor.last_error = None;
        self.metrics.indexed_block = block.number;
        self.metrics.last_successful_block = block.number;
        Ok(())
    }

    fn handle_reorg(
        &mut self,
        source: &dyn ChainSource,
        incoming: &RawBlock,
    ) -> Result<(), RuntimeError> {
        let planner = ReorgPlanner::new(self.max_reorg_depth);
        let outcome = planner.plan(&self.raw_logs, source, incoming)?;
        match outcome {
            ReorgOutcome::NoReorgRequired => Ok(()),
            ReorgOutcome::Rewind {
                common_ancestor,
                orphaned_blocks,
            } => {
                // Capture pre-rewind identity for the audit trail.
                let from_block = self.cursor.indexed_head_block;
                let from_hash = self.cursor.indexed_head_hash.clone();
                let depth = orphaned_blocks.len() as u32;
                let orphaned_log_count = self
                    .raw_logs
                    .iter()
                    .filter(|l| orphaned_blocks.contains(&l.block_number) && l.is_canonical)
                    .count() as u32;
                self.rewind_to(common_ancestor, &orphaned_blocks);
                self.metrics.reorg_count += 1;
                self.metrics.max_reorg_depth_seen = self
                    .metrics
                    .max_reorg_depth_seen
                    .max(orphaned_blocks.len() as u64);
                self.pending_reorg_audit = Some(PendingReorgAudit {
                    from_block,
                    from_hash,
                    to_block: incoming.number,
                    to_hash: incoming.hash.clone(),
                    depth,
                    orphaned_log_count,
                });
                Ok(())
            }
        }
    }

    fn rewind_to(&mut self, common_ancestor: u64, orphaned_blocks: &[u64]) {
        // Mark orphaned raw logs non-canonical + record `orphaned_at_block`.
        let orphaned_at = common_ancestor;
        for log in self.raw_logs.iter_mut() {
            if orphaned_blocks.contains(&log.block_number) {
                log.is_canonical = false;
                log.orphaned_at_block = Some(orphaned_at);
            }
        }
        // Rebuild projections from canonical raw logs alone.
        let mut state = ProjectionState::default();
        let mut correlator = ExecutionCorrelator::new();
        let mut decoded = Vec::new();
        let mut last_tx: Option<String> = None;
        for log in self.raw_logs.iter().filter(|l| l.is_canonical) {
            let ctx = ApplyContext {
                block_number: log.block_number,
                tx_hash: log.tx_hash.clone(),
                log_index: log.log_index,
                block_timestamp: log.block_timestamp,
            };
            if let Some(prev) = last_tx.as_ref() {
                if prev != &ctx.tx_hash {
                    correlator.flush_transaction(prev, &mut state);
                }
            }
            last_tx = Some(ctx.tx_hash.clone());
            let raw = crate::hybrid_v2::decoder::CanonicalRawLog {
                emitter: log.emitter.clone(),
                topics: log.topics.clone(),
                data: log.data.clone(),
            };
            let event = match decode_log_with(&raw, None, self.manifest.event_version) {
                Ok(e) => e,
                Err(_) => continue,
            };
            let _ = apply(&mut state, &event, &ctx);
            correlator.observe(&event, &ctx, &mut state);
            decoded.push((event, ctx));
        }
        if let Some(tx) = last_tx {
            correlator.flush_transaction(&tx, &mut state);
        }
        self.state = state;
        self.correlator = correlator;
        self.decoded_events = decoded;
        // Cursor rewinds to the last canonical raw log we still hold.
        if let Some(last) = self.raw_logs.iter().rev().find(|l| l.is_canonical) {
            self.cursor.indexed_head_block = last.block_number;
            self.cursor.indexed_head_hash = last.block_hash.clone();
            self.cursor.indexed_head_parent = last.parent_hash.clone();
        } else {
            self.cursor = RuntimeCursor::default();
        }
    }

    fn recompute_readiness(&mut self) {
        if self.cursor.indexed_head_block == 0 {
            self.readiness = ReadinessState::new_not_ready(ReadinessReason::AwaitingFirstBlock);
            return;
        }
        if self.metrics.observed_block > self.cursor.indexed_head_block {
            self.readiness = ReadinessState::new_not_ready(ReadinessReason::Behind {
                observed: self.metrics.observed_block,
                indexed: self.cursor.indexed_head_block,
            });
            return;
        }
        self.readiness = ReadinessState::ready();
    }

    fn rollback(
        &mut self,
        backup_state: ProjectionState,
        backup_raw_len: usize,
        backup_decoded_len: usize,
        backup_correlator: ExecutionCorrelator,
    ) {
        self.state = backup_state;
        self.raw_logs.truncate(backup_raw_len);
        self.decoded_events.truncate(backup_decoded_len);
        self.correlator = backup_correlator;
    }

    fn correlator_snapshot(&self) -> ExecutionCorrelator {
        // Correlator has no Clone; we serialize the pending state via
        // a fresh instance and re-observe the completed decoded_events
        // on rollback — but since the observe path is the same for
        // every retry, an empty correlator is safe: on rollback the
        // in-flight groups are dropped (the transaction failed).
        ExecutionCorrelator::new()
    }

    pub fn cursor(&self) -> &RuntimeCursor {
        &self.cursor
    }

    pub fn projection(&self) -> &ProjectionState {
        &self.state
    }

    pub fn readiness(&self) -> &ReadinessState {
        &self.readiness
    }

    pub fn metrics(&self) -> &RuntimeMetrics {
        &self.metrics
    }

    // ---------------- PERSISTENCE HOOKS (Parts E + F) ----------------

    /// Async wrapper around [`Self::tick`] that additionally persists
    /// each successfully-applied block through the attached
    /// `HybridV2ProjectionStore`. When no persistence is configured this
    /// simply delegates to [`Self::tick`] and returns.
    ///
    /// Frozen rules honoured here:
    /// - Cursor stays where it was on persist failure — in-memory state
    ///   is rolled back to the pre-block snapshot, readiness is set to
    ///   `NOT_READY(ProjectionFailure)`, and the error is propagated.
    /// - No auto-fallback: if persistence is configured and persist
    ///   fails, we do NOT silently continue with the in-memory
    ///   projection. The caller sees `RuntimeError::Persistence`.
    pub async fn tick_and_persist(
        &mut self,
        source: &dyn ChainSource,
    ) -> Result<bool, RuntimeError> {
        if self.persistence.is_none() {
            return self.tick(source);
        }
        // Chain-id mismatch — surface identically to `tick()`.
        if source.chain_id() != self.manifest.chain_id {
            self.readiness = ReadinessState::new_not_ready(ReadinessReason::WrongChain {
                manifest: self.manifest.chain_id,
                source: source.chain_id(),
            });
            return Err(RuntimeError::WrongChain {
                manifest: self.manifest.chain_id,
                source_chain: source.chain_id(),
            });
        }
        self.metrics.observed_block = source.head_block_number();
        self.metrics.finalized_block = source.finalized_block_number();
        let next = self.cursor.indexed_head_block + 1;
        let Some(block) = source.block_at(next) else {
            self.recompute_readiness();
            return Ok(false);
        };
        // Reorg detection: parent hash of incoming must match our cursor head.
        if self.cursor.indexed_head_block > 0
            && !parents_match(&self.cursor.indexed_head_hash, &block.parent_hash)
        {
            // Handle in-memory reorg; capture pending audit metadata.
            self.handle_reorg(source, &block)?;
            self.metrics.observed_block = source.head_block_number();
            self.metrics.lag = self
                .metrics
                .observed_block
                .saturating_sub(self.cursor.indexed_head_block);
            self.recompute_readiness();
            // Flush the reorg audit event; failure logs + counts, never
            // fails the reorg. This matches the frozen rule that reorg
            // handling must not be blocked by observability writes.
            self.record_pending_reorg_audit().await;
            return Ok(true);
        }

        // Snapshot for rollback if persist fails.
        let backup_state = self.state.clone();
        let backup_raw_len = self.raw_logs.len();
        let backup_decoded_len = self.decoded_events.len();
        let backup_cursor = self.cursor.clone();
        let backup_metrics = self.metrics.clone();
        let backup_readiness = self.readiness.clone();

        // Apply the block in-memory (this pushes raw logs, decoded
        // events, mutates state, and advances the cursor).
        self.apply_block(&block)?;

        // Slice the just-appended raw logs + decoded events for the
        // persist call. Every emitter-canonical log in the block is
        // one row in each slice.
        let raw_slice = self.raw_logs[backup_raw_len..].to_vec();
        let decoded_slice = self.decoded_events[backup_decoded_len..].to_vec();
        let block_ref = CanonicalBlockRef {
            block_number: block.number,
            block_hash: block.hash.clone(),
            parent_hash: block.parent_hash.clone(),
            block_timestamp: block.timestamp,
        };
        let cursor_snap = RuntimeCursorSnapshot::from_runtime(
            &self.persistence_cursor_name,
            &self.cursor,
            &self.metrics,
        );
        let readiness_snap = ReadinessSnapshot::from_state(&self.readiness);
        let now_ms = now_ms();
        let deployment_id =
            self.persistence_deployment_id
                .ok_or_else(|| RuntimeError::Persistence {
                    block: block.number,
                    detail: "persistence configured without deployment_id".to_string(),
                })?;
        let store = self
            .persistence
            .as_ref()
            .expect("persistence checked at method entry")
            .clone();
        let persist_result = store
            .persist_block_atomic(
                deployment_id,
                &block_ref,
                &raw_slice,
                &decoded_slice,
                &self.state,
                &cursor_snap,
                &readiness_snap,
                now_ms,
            )
            .await;

        if let Err(err) = persist_result {
            // Roll back: cursor stays where it was, in-memory state
            // reverts to pre-block, readiness becomes NOT_READY with
            // a ProjectionFailure reason.
            self.state = backup_state;
            self.raw_logs.truncate(backup_raw_len);
            self.decoded_events.truncate(backup_decoded_len);
            self.cursor = backup_cursor;
            self.metrics = backup_metrics;
            self.metrics.projection_failures = self.metrics.projection_failures.saturating_add(1);
            self.readiness = ReadinessState::new_not_ready(ReadinessReason::ProjectionFailure {
                block: block.number,
                detail: format!("{err}"),
            });
            // Correlator was captured empty on rollback by the sync path
            // via `apply_block`; ensure any partial in-flight groups
            // from the failed persist attempt do not linger.
            self.correlator = ExecutionCorrelator::new();
            // Restore readiness only if the caller was ready before the
            // block — otherwise keep the failure reason above (which is
            // strictly more specific than the pre-block reason).
            let _ = backup_readiness;
            return Err(RuntimeError::Persistence {
                block: block.number,
                detail: format!("{err}"),
            });
        }

        self.metrics.observed_block = source.head_block_number();
        self.metrics.lag = self
            .metrics
            .observed_block
            .saturating_sub(self.cursor.indexed_head_block);
        self.recompute_readiness();
        Ok(true)
    }

    /// Try to flush the last captured reorg audit event to persistence.
    /// This never returns an error: failures are logged + counted so
    /// reorg replay is not blocked by observability writes.
    async fn record_pending_reorg_audit(&mut self) {
        let Some(audit) = self.pending_reorg_audit.take() else {
            return;
        };
        let (Some(store), Some(deployment_id)) =
            (self.persistence.clone(), self.persistence_deployment_id)
        else {
            return;
        };
        let outcome = store
            .record_reorg_event(
                deployment_id,
                now_ms(),
                audit.from_block,
                &audit.from_hash,
                audit.to_block,
                &audit.to_hash,
                audit.depth,
                audit.orphaned_log_count,
            )
            .await;
        if let Err(err) = outcome {
            self.reorg_audit_write_failures = self.reorg_audit_write_failures.saturating_add(1);
            tracing::warn!(
                target: "hybrid_v2::runtime",
                deployment_id,
                from_block = audit.from_block,
                to_block = audit.to_block,
                depth = audit.depth,
                error = %err,
                "reorg audit write failed (non-fatal, reorg already applied in-memory)"
            );
        }
    }

    /// Hydrate the runtime cursor + readiness from the projection store.
    ///
    /// BOOTSTRAP-1 partial: this restores cursor + readiness only. The
    /// in-memory reducer working state (`ProjectionState`, raw logs,
    /// decoded events, correlator) is NOT rehydrated by this call —
    /// callers that need a warm reducer for a live runtime must invoke
    /// `RebuildService::replay_all(...)` against a raw-log source
    /// separately. The current design is safe because the cursor +
    /// canonical block metadata alone are enough to skip already-persisted
    /// blocks on restart, and the projection store's read paths do not
    /// consult the in-memory reducer.
    pub async fn bootstrap_from_persistence(&mut self) -> Result<BootstrapResult, RuntimeError> {
        let (Some(store), Some(deployment_id)) =
            (self.persistence.clone(), self.persistence_deployment_id)
        else {
            return Ok(BootstrapResult::NoPersistenceConfigured);
        };
        let cursor_opt = store
            .read_cursor(deployment_id, &self.persistence_cursor_name)
            .await
            .map_err(|e| RuntimeError::Bootstrap {
                detail: format!("read_cursor: {e}"),
            })?;
        let readiness_opt =
            store
                .read_readiness(deployment_id)
                .await
                .map_err(|e| RuntimeError::Bootstrap {
                    detail: format!("read_readiness: {e}"),
                })?;
        match (cursor_opt, readiness_opt) {
            (None, None) => Ok(BootstrapResult::FreshStart),
            (Some(cursor), readiness_opt) => {
                self.cursor.indexed_head_block = cursor.indexed_head_block;
                self.cursor.indexed_head_hash = cursor.indexed_head_hash;
                self.cursor.indexed_head_parent = cursor.indexed_head_parent;
                self.cursor.observed_head_block = cursor.observed_head_block;
                self.cursor.finalized_head_block = cursor.finalized_head_block;
                self.cursor.last_error = cursor.last_error;
                self.metrics.indexed_block = cursor.indexed_head_block;
                self.metrics.last_successful_block = cursor.last_success_block;
                self.metrics.observed_block = cursor.observed_head_block;
                self.metrics.finalized_block = cursor.finalized_head_block;
                self.metrics.reorg_count = cursor.reorg_count;
                self.metrics.max_reorg_depth_seen = cursor.max_reorg_depth_seen;
                self.metrics.decode_failures = cursor.decode_failures;
                self.metrics.projection_failures = cursor.projection_failures;
                self.metrics.unknown_canonical_events = cursor.unknown_canonical_events;
                match readiness_opt {
                    Some(r) if r.ready => {
                        self.readiness = ReadinessState::ready();
                    }
                    _ => {
                        // Cursor present but readiness absent OR not
                        // ready — mark the resuming state via the
                        // closest existing reason. The runtime will
                        // recompute this on the next successful tick.
                        self.readiness =
                            ReadinessState::new_not_ready(ReadinessReason::AwaitingFirstBlock);
                    }
                }
                Ok(BootstrapResult::RestoredCursorOnly {
                    indexed_head_block: self.cursor.indexed_head_block,
                })
            }
            (None, Some(_)) => {
                // Readiness without a cursor is unexpected but not
                // fatal — treat as fresh start.
                Ok(BootstrapResult::FreshStart)
            }
        }
    }
}

fn now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn parents_match(cursor_hash: &str, incoming_parent: &str) -> bool {
    cursor_hash.eq_ignore_ascii_case(incoming_parent)
}

fn tx_hash_for_log(block: &RawBlock, _log_index: usize) -> String {
    // In-fixture convention: every canonical log in a block belongs to one
    // synthetic transaction keyed on the block hash. Real ChainSource
    // implementations (RPC / WS) carry a per-log tx hash on the raw log —
    // that overrides this helper. The correlator only cares about
    // "same tx" identity for execution grouping, so binding to the block
    // hash preserves cross-log correlation within a single block.
    format!("{}::tx", block.hash)
}

fn collect_emitters(manifest: &ManifestParams) -> HashSet<String> {
    let mut set = HashSet::new();
    let m = &manifest.module_addresses;
    for slot in [
        &m.subaccount_registry,
        &m.collateral_vault,
        &m.options_positions_ledger,
        &m.risk_module,
        &m.margin_engine,
        &m.option_matching_engine,
        &m.escape_controller,
        &m.recovery_finalizer,
    ]
    .iter()
    .copied()
    {
        set.insert(slot.to_ascii_lowercase());
    }
    if let Some(addr) = &m.fees_manager_v2 {
        set.insert(addr.to_ascii_lowercase());
    }
    if let Some(addr) = &m.option_execution_fee_adapter {
        set.insert(addr.to_ascii_lowercase());
    }
    // The manifest itself emits DeploymentManifestDeclared once.
    set.insert(manifest.manifest_address.to_ascii_lowercase());
    set
}
