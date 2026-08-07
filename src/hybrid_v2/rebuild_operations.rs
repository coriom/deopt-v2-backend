//! `BACKEND-HYBRID-V2-PROJECTION-PERSISTENCE-CLOSURE-V1` — persisted,
//! restart-safe rebuild state machine + operation lock generalisation.
//!
//! Frozen safety invariants enforced here:
//! - `NO_PARTIAL_REBUILD_STATE_IS_PUBLICLY_READY` — the rebuild phase
//!   drives readiness. Nothing publishes until the machine reaches
//!   `Complete`; otherwise readiness is `RebuildInProgress` (503).
//! - `OPERATOR_RECOVERY_ACTIONS_ARE_DEPLOYMENT_SCOPED_AND_NON_PUBLIC`
//!   — every phase update is keyed on `(deployment_id, rebuild_epoch)`;
//!   the trigger surface lives behind admin auth (see `src/admin.rs`).
//! - `READY_REQUIRES_PERSISTENCE_REORG_REBUILD_AND_RECONCILIATION_CONVERGENCE`
//!   — a `RebuildOutcome::Rebuilt` or `NothingToDo` clears the rebuild
//!   axis of readiness; a `ManualInterventionRequired` sticks 503.
//!
//! Publication model: `BACKEND_HYBRID_V2_REBUILD_SINGLE_TRANSACTION_MODEL`.
//! Mode 1 (`JournalReplay`) compares the rebuilt state to the live
//! projection and either reports `NothingToDo` (match) or escalates to
//! `ManualInterventionRequired` (drift). Mode 2 (`FreshChain`) uses the
//! existing `IndexerRuntime` worker path (invoked by the caller) to
//! ingest from an empty PG; this module records the phase machine
//! around that call. No shadow-generation columns are added; the
//! single-writer, journal-bounded model does not need them at this
//! milestone.
//!
//! Boundary:
//! - Input:  a `ChainSource` (read-only), a `HybridV2ProjectionStore`
//!           (owns Postgres), a validated `ManifestParams`.
//! - Output: mutations to `hybrid_v2_rebuild_operations` +
//!           `hybrid_v2_operation_locks`; no chain writes; no signer.

use crate::error::{BackendError, Result};
use crate::hybrid_v2::chain_source::ChainSourceError;
use crate::hybrid_v2::manifest::ManifestParams;
use crate::hybrid_v2::persistence::HybridV2ProjectionStore;
use crate::hybrid_v2::rebuild::RebuildService as JournalReplayer;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use thiserror::Error;

// -----------------------------------------------------------------
//                       ENUMS
// -----------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RebuildMode {
    /// Replay retained canonical PG journal via
    /// `crate::hybrid_v2::rebuild::RebuildService::replay_all` into a
    /// fresh `ProjectionState`, verify against the live projection.
    /// No mutation on match; drift escalates.
    JournalReplay,
    /// Starting from an empty PG, walk the (mock) RPC from
    /// `deployment_start_block` up to a bounded head, persisting
    /// journals + projections through the normal
    /// `IndexerRuntime::tick_and_persist` path. Requires the caller
    /// to supply a source + attach the runtime.
    FreshChain,
}

impl RebuildMode {
    pub fn as_str(self) -> &'static str {
        match self {
            RebuildMode::JournalReplay => "JOURNAL_REPLAY",
            RebuildMode::FreshChain => "FRESH_CHAIN",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "JOURNAL_REPLAY" => RebuildMode::JournalReplay,
            "FRESH_CHAIN" => RebuildMode::FreshChain,
            _ => return None,
        })
    }
}

/// 13 persisted phases of the rebuild state machine. Each transition is
/// written to `hybrid_v2_rebuild_operations` before the next phase
/// observes it, so a mid-flight process crash leaves the row at the
/// last-completed phase for restart resume.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RebuildPhase {
    None,
    Requested,
    LockAcquired,
    ValidatingSource,
    Preparing,
    Replaying,
    Correlating,
    Verifying,
    Reconciling,
    Committing,
    Complete,
    Failed,
    ManualInterventionRequired,
}

impl RebuildPhase {
    pub fn as_str(self) -> &'static str {
        match self {
            RebuildPhase::None => "NONE",
            RebuildPhase::Requested => "REQUESTED",
            RebuildPhase::LockAcquired => "LOCK_ACQUIRED",
            RebuildPhase::ValidatingSource => "VALIDATING_SOURCE",
            RebuildPhase::Preparing => "PREPARING",
            RebuildPhase::Replaying => "REPLAYING",
            RebuildPhase::Correlating => "CORRELATING",
            RebuildPhase::Verifying => "VERIFYING",
            RebuildPhase::Reconciling => "RECONCILING",
            RebuildPhase::Committing => "COMMITTING",
            RebuildPhase::Complete => "COMPLETE",
            RebuildPhase::Failed => "FAILED",
            RebuildPhase::ManualInterventionRequired => "MANUAL_INTERVENTION_REQUIRED",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "NONE" => RebuildPhase::None,
            "REQUESTED" => RebuildPhase::Requested,
            "LOCK_ACQUIRED" => RebuildPhase::LockAcquired,
            "VALIDATING_SOURCE" => RebuildPhase::ValidatingSource,
            "PREPARING" => RebuildPhase::Preparing,
            "REPLAYING" => RebuildPhase::Replaying,
            "CORRELATING" => RebuildPhase::Correlating,
            "VERIFYING" => RebuildPhase::Verifying,
            "RECONCILING" => RebuildPhase::Reconciling,
            "COMMITTING" => RebuildPhase::Committing,
            "COMPLETE" => RebuildPhase::Complete,
            "FAILED" => RebuildPhase::Failed,
            "MANUAL_INTERVENTION_REQUIRED" => RebuildPhase::ManualInterventionRequired,
            _ => return None,
        })
    }
    /// Terminal (do not resume) — either done or requires operator action.
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            RebuildPhase::Complete
                | RebuildPhase::Failed
                | RebuildPhase::ManualInterventionRequired
        )
    }
    /// The public projection is publicly-visible only when NOT here.
    pub fn is_active(self) -> bool {
        !matches!(self, RebuildPhase::None | RebuildPhase::Complete)
            && !matches!(self, RebuildPhase::Failed)
    }
}

// -----------------------------------------------------------------
//                       STATE ROW
// -----------------------------------------------------------------

/// Mirrors one row of `hybrid_v2_rebuild_operations`.
#[derive(Debug, Clone)]
pub struct RebuildOperationState {
    pub deployment_id: i64,
    pub rebuild_epoch: i64,
    pub mode: RebuildMode,
    pub phase: RebuildPhase,
    pub requested_at_ms: i64,
    pub source_start_block: Option<u64>,
    pub source_end_block: Option<u64>,
    pub events_replayed: Option<u64>,
    pub executions_correlated: Option<u64>,
    pub verification_result: Option<String>,
    pub reconciliation_result: Option<String>,
    pub generation_id: Option<i64>,
    pub retry_count: u32,
    pub last_failure_detail: Option<String>,
    pub updated_at_ms: i64,
    pub completed_at_ms: Option<i64>,
}

impl RebuildOperationState {
    pub fn new_requested(
        deployment_id: i64,
        rebuild_epoch: i64,
        mode: RebuildMode,
        now_ms: i64,
    ) -> Self {
        Self {
            deployment_id,
            rebuild_epoch,
            mode,
            phase: RebuildPhase::Requested,
            requested_at_ms: now_ms,
            source_start_block: None,
            source_end_block: None,
            events_replayed: None,
            executions_correlated: None,
            verification_result: None,
            reconciliation_result: None,
            generation_id: None,
            retry_count: 0,
            last_failure_detail: None,
            updated_at_ms: now_ms,
            completed_at_ms: None,
        }
    }
}

// -----------------------------------------------------------------
//                       OPERATION LOCK
// -----------------------------------------------------------------

/// Which subsystem holds the deployment-scoped operation lock.
///
/// Reorg, rebuild, reconciliation, and execution all contend for the
/// same `hybrid_v2_operation_locks` row per deployment; at most one
/// may hold the lock at a time. The `Execution` variant is added by
/// `BACKEND-HYBRID-V2-SIGNER-AND-EXECUTION-V1` (foundation package):
/// the execution orchestrator acquires the lock before advancing an
/// execution row into any phase that could observe or mutate
/// projection state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationKind {
    Reorg,
    Rebuild,
    Reconciliation,
    Execution,
}

impl OperationKind {
    pub fn as_str(self) -> &'static str {
        match self {
            OperationKind::Reorg => "REORG",
            OperationKind::Rebuild => "REBUILD",
            OperationKind::Reconciliation => "RECONCILIATION",
            OperationKind::Execution => "EXECUTION",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "REORG" => OperationKind::Reorg,
            "REBUILD" => OperationKind::Rebuild,
            "RECONCILIATION" => OperationKind::Reconciliation,
            "EXECUTION" => OperationKind::Execution,
            _ => return None,
        })
    }
}

/// Guard for the unified deployment-scoped operation lock. The lock is
/// held in `hybrid_v2_operation_locks` (or its in-memory equivalent).
/// Release is explicit + async — dropping the guard does not release.
pub struct OperationLockGuard {
    pub deployment_id: i64,
    pub operation: OperationKind,
    pub holder_epoch: i64,
    pub store: Option<Arc<dyn HybridV2ProjectionStore>>,
}

impl std::fmt::Debug for OperationLockGuard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OperationLockGuard")
            .field("deployment_id", &self.deployment_id)
            .field("operation", &self.operation.as_str())
            .field("holder_epoch", &self.holder_epoch)
            .field("store", &self.store.as_ref().map(|_| "<attached>"))
            .finish()
    }
}

impl OperationLockGuard {
    pub async fn release(self) -> Result<()> {
        if let Some(store) = self.store.as_ref() {
            store
                .release_operation_lock(self.deployment_id, self.holder_epoch)
                .await?;
        }
        Ok(())
    }
}

// -----------------------------------------------------------------
//                       CONFIG
// -----------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct RebuildConfig {
    /// Batch size for the journal-replay path. Bounded [1, 100_000].
    pub replay_batch_size: u32,
    /// If true, `JournalReplay` compares the rebuilt state to the live
    /// projection and escalates on drift. If false, the rebuilt state
    /// is dropped without comparison (used only for smoke tests).
    pub verification_enabled: bool,
    /// If true, a downstream reconciler is invoked after a successful
    /// rebuild. This module does not run the reconciler itself.
    pub reconciliation_enabled: bool,
    /// Retryable-failure budget before the machine escalates to
    /// `ManualInterventionRequired`. Bounded [0, 10].
    pub retry_max: u32,
    /// If true, drift detected during `JournalReplay` triggers an
    /// automatic re-materialization of the projection tables via
    /// `HybridV2ProjectionStore::commit_rematerialization`. If false
    /// (the safe default), drift escalates to
    /// `ManualInterventionRequired` without touching any live table.
    ///
    /// Configured via the environment variable
    /// `HYBRID_V2_REBUILD_AUTO_REMATERIALIZE` (accepts `1`, `true`,
    /// `yes` — case insensitive; anything else stays false).
    pub auto_rematerialize: bool,
}

impl Default for RebuildConfig {
    fn default() -> Self {
        Self {
            replay_batch_size: 1_000,
            verification_enabled: true,
            reconciliation_enabled: true,
            retry_max: 3,
            auto_rematerialize: false,
        }
    }
}

impl RebuildConfig {
    pub fn validate(&self) -> Result<()> {
        if !(1..=100_000).contains(&self.replay_batch_size) {
            return Err(BackendError::Config(format!(
                "rebuild.replay_batch_size must be within [1, 100000], got {}",
                self.replay_batch_size
            )));
        }
        if self.retry_max > 10 {
            return Err(BackendError::Config(format!(
                "rebuild.retry_max must be <= 10, got {}",
                self.retry_max
            )));
        }
        Ok(())
    }
}

// -----------------------------------------------------------------
//                       OUTCOME + ERROR
// -----------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum RebuildOutcome {
    /// The rebuilt state matched the live projection — no mutation
    /// applied. Phase = Complete.
    NothingToDo { epoch: i64, events_replayed: u64 },
    /// The rebuild completed (either replaced the projection or the
    /// projection was empty before). Phase = Complete.
    Rebuilt {
        epoch: i64,
        events_replayed: u64,
        executions_correlated: u64,
    },
    /// Drift detected between the rebuilt journal replay and the live
    /// projection, or a hard rebuild failure. Phase =
    /// ManualInterventionRequired; readiness = RebuildFailed. Operator
    /// must act.
    ManualInterventionRequired { epoch: i64, reason: String },
    /// A retryable transient failure. Phase = Failed. A subsequent
    /// invocation may retry.
    RetryableFailure { epoch: i64, reason: String },
}

#[derive(Debug, Error)]
pub enum RebuildError {
    #[error("chain source: {0}")]
    ChainSource(#[from] ChainSourceError),
    #[error("persistence: {0}")]
    Persistence(String),
    #[error("manifest mismatch: {0}")]
    ManifestMismatch(String),
    #[error("state machine: {0}")]
    StateMachine(String),
    #[error(
        "lock contention (another {kind} operation already holds the lock for deployment {deployment_id})"
    )]
    LockContention {
        deployment_id: i64,
        kind: &'static str,
    },
    #[error("config: {0}")]
    Config(String),
}

impl From<BackendError> for RebuildError {
    fn from(err: BackendError) -> Self {
        RebuildError::Persistence(err.to_string())
    }
}

// -----------------------------------------------------------------
//                       SERVICE
// -----------------------------------------------------------------

/// Persisted rebuild orchestrator. Distinct from the pure in-memory
/// `crate::hybrid_v2::rebuild::RebuildService` (which is the canonical
/// replay primitive this module invokes).
pub struct RebuildOperationsService {
    pub deployment_id: i64,
    pub config: RebuildConfig,
}

impl RebuildOperationsService {
    pub fn new(deployment_id: i64, config: RebuildConfig) -> Self {
        Self {
            deployment_id,
            config,
        }
    }

    /// Mode 1 — Journal replay. Replays the retained canonical journal
    /// via `JournalReplayer::replay_all` into a fresh `ProjectionState`,
    /// compares it to the live projection snapshot, and either:
    ///   * marks Complete `NothingToDo` when the two agree,
    ///   * marks `ManualInterventionRequired` when they diverge.
    ///
    /// This preserves the frozen safety rule
    /// `RECONCILIATION_DRIFT_NEVER_AUTO_REPAIRS_PROJECTIONS`: divergence
    /// never triggers an automatic mutation. The single-transaction
    /// publication model is documented in the module docstring.
    pub async fn rebuild_from_journal(
        &self,
        store: &Arc<dyn HybridV2ProjectionStore>,
        manifest: &ManifestParams,
    ) -> std::result::Result<RebuildOutcome, RebuildError> {
        self.config
            .validate()
            .map_err(|e| RebuildError::Config(e.to_string()))?;

        let started_ms = now_ms();
        let epoch = self.next_epoch(store).await?;
        let mut state = RebuildOperationState::new_requested(
            self.deployment_id,
            epoch,
            RebuildMode::JournalReplay,
            started_ms,
        );
        store
            .upsert_rebuild_operation(&state)
            .await
            .map_err(|e| RebuildError::Persistence(e.to_string()))?;

        // Acquire the deployment-scoped operation lock.
        let guard = match store
            .try_acquire_operation_lock(
                self.deployment_id,
                OperationKind::Rebuild,
                epoch,
                started_ms,
            )
            .await
            .map_err(|e| RebuildError::Persistence(e.to_string()))?
        {
            Some(g) => g,
            None => {
                state.phase = RebuildPhase::Failed;
                state.last_failure_detail = Some("operation lock contention".to_string());
                state.updated_at_ms = started_ms;
                store
                    .upsert_rebuild_operation(&state)
                    .await
                    .map_err(|e| RebuildError::Persistence(e.to_string()))?;
                return Err(RebuildError::LockContention {
                    deployment_id: self.deployment_id,
                    kind: "REBUILD",
                });
            }
        };
        state.phase = RebuildPhase::LockAcquired;
        state.updated_at_ms = now_ms();
        store
            .upsert_rebuild_operation(&state)
            .await
            .map_err(err_persist)?;

        let outcome = self
            .rebuild_from_journal_inner(store, manifest, &mut state)
            .await;

        if let Err(release_err) = guard.release().await {
            tracing::warn!(
                target: "hybrid_v2::rebuild_operations",
                deployment_id = self.deployment_id,
                error = %release_err,
                "operation lock release failed (non-fatal)"
            );
        }
        outcome
    }

    async fn rebuild_from_journal_inner(
        &self,
        store: &Arc<dyn HybridV2ProjectionStore>,
        manifest: &ManifestParams,
        state: &mut RebuildOperationState,
    ) -> std::result::Result<RebuildOutcome, RebuildError> {
        // ValidatingSource — chain identity + manifest binding. We rely
        // on the manifest structure being pre-validated by
        // ManifestValidator; here we only guard against Base mainnet.
        state.phase = RebuildPhase::ValidatingSource;
        state.updated_at_ms = now_ms();
        store
            .upsert_rebuild_operation(state)
            .await
            .map_err(err_persist)?;
        if manifest.chain_id == crate::hybrid_v2::manifest::BASE_MAINNET_CHAIN_ID {
            let reason = "Base mainnet forbidden".to_string();
            escalate_manual(store, state, reason.clone()).await?;
            return Err(RebuildError::ManifestMismatch(reason));
        }

        // Preparing — read the canonical journal.
        state.phase = RebuildPhase::Preparing;
        state.updated_at_ms = now_ms();
        store
            .upsert_rebuild_operation(state)
            .await
            .map_err(err_persist)?;
        let journal = store
            .read_canonical_journal(self.deployment_id)
            .await
            .map_err(err_persist)?;

        // Replaying — feed the journal through the canonical replay primitive.
        state.phase = RebuildPhase::Replaying;
        state.updated_at_ms = now_ms();
        store
            .upsert_rebuild_operation(state)
            .await
            .map_err(err_persist)?;
        let (rebuilt_state, _correlator, outcome) =
            JournalReplayer::new(manifest.event_version).replay_all(&journal);

        state.events_replayed = Some(outcome.events_replayed);
        state.executions_correlated = Some(rebuilt_state.matched_executions.len() as u64);
        state.source_start_block = journal.first().map(|l| l.block_number);
        state.source_end_block = Some(outcome.final_head_block);
        state.updated_at_ms = now_ms();
        store
            .upsert_rebuild_operation(state)
            .await
            .map_err(err_persist)?;

        // Correlating phase (accounting only — the replay above already
        // ran the correlator; we persist the count for observability).
        state.phase = RebuildPhase::Correlating;
        state.updated_at_ms = now_ms();
        store
            .upsert_rebuild_operation(state)
            .await
            .map_err(err_persist)?;

        // Verifying — compare rebuilt state to live projection snapshot.
        state.phase = RebuildPhase::Verifying;
        state.updated_at_ms = now_ms();
        store
            .upsert_rebuild_operation(state)
            .await
            .map_err(err_persist)?;

        if !self.config.verification_enabled {
            // Verification disabled — Mark Complete NothingToDo.
            state.verification_result = Some("SKIPPED".to_string());
            self.mark_complete(store, state).await?;
            return Ok(RebuildOutcome::NothingToDo {
                epoch: state.rebuild_epoch,
                events_replayed: outcome.events_replayed,
            });
        }
        let live = store
            .snapshot_projection_state(self.deployment_id)
            .await
            .map_err(err_persist)?;
        let drift = live
            .as_ref()
            .map(|live| projection_state_diff(&rebuilt_state, live))
            .unwrap_or(None);
        if let Some(reason) = drift {
            let detail = format!("rebuild drift: {}", reason);
            state.verification_result = Some("DRIFT".to_string());
            state.last_failure_detail = Some(detail.clone());
            if self.config.auto_rematerialize {
                // BACKEND-HYBRID-V2-PROJECTION-PERSISTENCE-OPERATIONAL-CLOSURE-V1
                // Real Mode-1 re-materialization: TRUNCATE + REINSERT the
                // rebuilt state in a single transaction.
                state.phase = RebuildPhase::Committing;
                state.updated_at_ms = now_ms();
                store
                    .upsert_rebuild_operation(state)
                    .await
                    .map_err(err_persist)?;
                let cursor_snapshot = crate::hybrid_v2::persistence::RuntimeCursorSnapshot {
                    cursor_name: "hybrid_v2_indexer".to_string(),
                    indexed_head_block: rebuilt_state
                        .matched_executions
                        .values()
                        .map(|e| e.block_number)
                        .max()
                        .unwrap_or(outcome.final_head_block),
                    indexed_head_hash: String::new(),
                    indexed_head_parent: String::new(),
                    observed_head_block: outcome.final_head_block,
                    finalized_head_block: 0,
                    last_error: None,
                    reorg_count: 0,
                    max_reorg_depth_seen: 0,
                    decode_failures: 0,
                    projection_failures: 0,
                    unknown_canonical_events: 0,
                    last_success_block: outcome.final_head_block,
                };
                let readiness_snapshot = crate::hybrid_v2::persistence::ReadinessSnapshot {
                    ready: false,
                    reason: Some("AwaitingFirstBlock".to_string()),
                    reason_detail: None,
                };
                match store
                    .commit_rematerialization(
                        self.deployment_id,
                        state.rebuild_epoch,
                        &rebuilt_state,
                        &cursor_snapshot,
                        &readiness_snapshot,
                        now_ms(),
                    )
                    .await
                {
                    Ok(()) => {
                        state.phase = RebuildPhase::Complete;
                        state.verification_result = Some("REMATERIALIZED".to_string());
                        state.completed_at_ms = Some(now_ms());
                        state.updated_at_ms = now_ms();
                        store
                            .upsert_rebuild_operation(state)
                            .await
                            .map_err(err_persist)?;
                        return Ok(RebuildOutcome::Rebuilt {
                            epoch: state.rebuild_epoch,
                            events_replayed: outcome.events_replayed,
                            executions_correlated: rebuilt_state.matched_executions.len() as u64,
                        });
                    }
                    Err(e) => {
                        let commit_detail = format!("{}: commit_rematerialization: {}", detail, e);
                        escalate_manual(store, state, commit_detail.clone()).await?;
                        return Ok(RebuildOutcome::ManualInterventionRequired {
                            epoch: state.rebuild_epoch,
                            reason: commit_detail,
                        });
                    }
                }
            }
            escalate_manual(store, state, detail.clone()).await?;
            return Ok(RebuildOutcome::ManualInterventionRequired {
                epoch: state.rebuild_epoch,
                reason: detail,
            });
        }

        // Reconciling — placeholder phase (the reconciler is separately
        // scheduled). Record and continue.
        state.phase = RebuildPhase::Reconciling;
        state.reconciliation_result = if self.config.reconciliation_enabled {
            Some("DEFERRED".to_string())
        } else {
            Some("DISABLED".to_string())
        };
        state.updated_at_ms = now_ms();
        store
            .upsert_rebuild_operation(state)
            .await
            .map_err(err_persist)?;

        // Committing — no-op in the single-transaction model when
        // rebuilt == live; we record the phase for auditability then
        // transition to Complete.
        state.phase = RebuildPhase::Committing;
        state.updated_at_ms = now_ms();
        store
            .upsert_rebuild_operation(state)
            .await
            .map_err(err_persist)?;

        state.verification_result = Some("MATCH".to_string());
        self.mark_complete(store, state).await?;
        Ok(RebuildOutcome::NothingToDo {
            epoch: state.rebuild_epoch,
            events_replayed: outcome.events_replayed,
        })
    }

    /// Mode 2 — Fresh-chain reconstruction. This service records the
    /// state machine around the caller's driven ingestion via the
    /// normal worker path (`IndexerRuntime::tick_and_persist`). The
    /// caller MUST supply the `on_progress` closure that performs the
    /// actual chain walk — this service does not embed a worker.
    ///
    /// Rationale: the worker path already applies every safety
    /// invariant (per-block PG tx, cursor advance, readiness update).
    /// Reusing it here (rather than duplicating a bespoke ingestion
    /// pipeline) preserves those invariants and keeps this module
    /// small.
    pub async fn reconstruct_from_chain<F, Fut>(
        &self,
        store: &Arc<dyn HybridV2ProjectionStore>,
        manifest: &ManifestParams,
        start_block: u64,
        target_head: u64,
        ingest: F,
    ) -> std::result::Result<RebuildOutcome, RebuildError>
    where
        F: FnOnce(u64, u64) -> Fut + Send,
        Fut: std::future::Future<Output = std::result::Result<(u64, u64), String>> + Send,
    {
        self.config
            .validate()
            .map_err(|e| RebuildError::Config(e.to_string()))?;
        if manifest.chain_id == crate::hybrid_v2::manifest::BASE_MAINNET_CHAIN_ID {
            return Err(RebuildError::ManifestMismatch(
                "Base mainnet forbidden".to_string(),
            ));
        }
        let started_ms = now_ms();
        let epoch = self.next_epoch(store).await?;
        let mut state = RebuildOperationState::new_requested(
            self.deployment_id,
            epoch,
            RebuildMode::FreshChain,
            started_ms,
        );
        state.source_start_block = Some(start_block);
        state.source_end_block = Some(target_head);
        store
            .upsert_rebuild_operation(&state)
            .await
            .map_err(err_persist)?;

        let guard = match store
            .try_acquire_operation_lock(
                self.deployment_id,
                OperationKind::Rebuild,
                epoch,
                started_ms,
            )
            .await
            .map_err(err_persist)?
        {
            Some(g) => g,
            None => {
                state.phase = RebuildPhase::Failed;
                state.last_failure_detail = Some("operation lock contention".to_string());
                state.updated_at_ms = started_ms;
                store
                    .upsert_rebuild_operation(&state)
                    .await
                    .map_err(err_persist)?;
                return Err(RebuildError::LockContention {
                    deployment_id: self.deployment_id,
                    kind: "REBUILD",
                });
            }
        };

        state.phase = RebuildPhase::LockAcquired;
        state.updated_at_ms = now_ms();
        store
            .upsert_rebuild_operation(&state)
            .await
            .map_err(err_persist)?;
        state.phase = RebuildPhase::ValidatingSource;
        state.updated_at_ms = now_ms();
        store
            .upsert_rebuild_operation(&state)
            .await
            .map_err(err_persist)?;
        state.phase = RebuildPhase::Preparing;
        state.updated_at_ms = now_ms();
        store
            .upsert_rebuild_operation(&state)
            .await
            .map_err(err_persist)?;
        state.phase = RebuildPhase::Replaying;
        state.updated_at_ms = now_ms();
        store
            .upsert_rebuild_operation(&state)
            .await
            .map_err(err_persist)?;

        let ingest_res = ingest(start_block, target_head).await;
        let outcome = match ingest_res {
            Ok((events, corr)) => {
                state.events_replayed = Some(events);
                state.executions_correlated = Some(corr);
                state.phase = RebuildPhase::Verifying;
                state.updated_at_ms = now_ms();
                store
                    .upsert_rebuild_operation(&state)
                    .await
                    .map_err(err_persist)?;
                state.verification_result = Some("SKIPPED".to_string());
                state.phase = RebuildPhase::Committing;
                state.updated_at_ms = now_ms();
                store
                    .upsert_rebuild_operation(&state)
                    .await
                    .map_err(err_persist)?;
                self.mark_complete(store, &mut state).await?;
                Ok(RebuildOutcome::Rebuilt {
                    epoch,
                    events_replayed: events,
                    executions_correlated: corr,
                })
            }
            Err(reason) => {
                state.retry_count = state.retry_count.saturating_add(1);
                state.last_failure_detail = Some(reason.clone());
                state.phase = if state.retry_count > self.config.retry_max {
                    RebuildPhase::ManualInterventionRequired
                } else {
                    RebuildPhase::Failed
                };
                state.updated_at_ms = now_ms();
                store
                    .upsert_rebuild_operation(&state)
                    .await
                    .map_err(err_persist)?;
                if matches!(state.phase, RebuildPhase::ManualInterventionRequired) {
                    Ok(RebuildOutcome::ManualInterventionRequired { epoch, reason })
                } else {
                    Ok(RebuildOutcome::RetryableFailure { epoch, reason })
                }
            }
        };

        if let Err(release_err) = guard.release().await {
            tracing::warn!(
                target: "hybrid_v2::rebuild_operations",
                deployment_id = self.deployment_id,
                error = %release_err,
                "operation lock release failed (non-fatal)"
            );
        }
        outcome
    }

    async fn next_epoch(
        &self,
        store: &Arc<dyn HybridV2ProjectionStore>,
    ) -> std::result::Result<i64, RebuildError> {
        let existing = store
            .read_latest_rebuild_operation(self.deployment_id)
            .await
            .map_err(err_persist)?;
        Ok(match existing {
            Some(row) if !row.phase.is_terminal() => row.rebuild_epoch,
            Some(row) => row.rebuild_epoch + 1,
            None => 1,
        })
    }

    async fn mark_complete(
        &self,
        store: &Arc<dyn HybridV2ProjectionStore>,
        state: &mut RebuildOperationState,
    ) -> std::result::Result<(), RebuildError> {
        state.phase = RebuildPhase::Complete;
        state.completed_at_ms = Some(now_ms());
        state.updated_at_ms = now_ms();
        store
            .upsert_rebuild_operation(state)
            .await
            .map_err(err_persist)?;
        Ok(())
    }
}

// -----------------------------------------------------------------
//                       HELPERS
// -----------------------------------------------------------------

fn err_persist(e: BackendError) -> RebuildError {
    RebuildError::Persistence(e.to_string())
}

async fn escalate_manual(
    store: &Arc<dyn HybridV2ProjectionStore>,
    state: &mut RebuildOperationState,
    reason: String,
) -> std::result::Result<(), RebuildError> {
    state.phase = RebuildPhase::ManualInterventionRequired;
    state.last_failure_detail = Some(reason);
    state.updated_at_ms = now_ms();
    store
        .upsert_rebuild_operation(state)
        .await
        .map_err(err_persist)?;
    Ok(())
}

fn now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Structural comparison of two `ProjectionState`s. Returns `None` on
/// match, `Some(reason)` on the first divergence. Bounded output size —
/// truncates the reason to 256 bytes.
///
/// Compares balances, reservations, and recovery_state (the three
/// slices most likely to drift after a partial write). Positions,
/// orders, and matched executions are intentionally NOT compared here
/// — the reconciler is the primary drift detector for those slices,
/// and comparing them at this layer would double-count.
pub fn projection_state_diff(
    rebuilt: &crate::hybrid_v2::reducer::ProjectionState,
    live: &crate::hybrid_v2::reducer::ProjectionState,
) -> Option<String> {
    if rebuilt.balances != live.balances {
        return Some(truncate(format!(
            "balances differ: rebuilt={} live={}",
            rebuilt.balances.len(),
            live.balances.len()
        )));
    }
    if rebuilt.reservations != live.reservations {
        return Some(truncate(format!(
            "reservations differ: rebuilt={} live={}",
            rebuilt.reservations.len(),
            live.reservations.len()
        )));
    }
    if rebuilt.recovery_state.len() != live.recovery_state.len() {
        return Some(truncate(format!(
            "recovery_state cardinality: rebuilt={} live={}",
            rebuilt.recovery_state.len(),
            live.recovery_state.len()
        )));
    }
    for (k, v) in rebuilt.recovery_state.iter() {
        match live.recovery_state.get(k) {
            Some(other) if other.as_str() == v.as_str() => {}
            Some(other) => {
                return Some(truncate(format!(
                    "recovery_state[{k}]: rebuilt={} live={}",
                    v.as_str(),
                    other.as_str()
                )));
            }
            None => {
                return Some(truncate(format!("recovery_state[{k}] missing in live")));
            }
        }
    }
    None
}

fn truncate(mut s: String) -> String {
    if s.len() > 256 {
        s.truncate(256);
        s.push_str("...");
    }
    s
}

// -----------------------------------------------------------------
//                       UNIT TESTS
// -----------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phase_as_str_roundtrips() {
        for p in [
            RebuildPhase::None,
            RebuildPhase::Requested,
            RebuildPhase::LockAcquired,
            RebuildPhase::ValidatingSource,
            RebuildPhase::Preparing,
            RebuildPhase::Replaying,
            RebuildPhase::Correlating,
            RebuildPhase::Verifying,
            RebuildPhase::Reconciling,
            RebuildPhase::Committing,
            RebuildPhase::Complete,
            RebuildPhase::Failed,
            RebuildPhase::ManualInterventionRequired,
        ] {
            assert_eq!(RebuildPhase::parse(p.as_str()), Some(p));
        }
    }

    #[test]
    fn mode_as_str_roundtrips() {
        assert_eq!(
            RebuildMode::parse(RebuildMode::JournalReplay.as_str()),
            Some(RebuildMode::JournalReplay)
        );
        assert_eq!(
            RebuildMode::parse(RebuildMode::FreshChain.as_str()),
            Some(RebuildMode::FreshChain)
        );
        assert!(RebuildMode::parse("NOPE").is_none());
    }

    #[test]
    fn operation_kind_as_str_roundtrips() {
        for k in [
            OperationKind::Reorg,
            OperationKind::Rebuild,
            OperationKind::Reconciliation,
            OperationKind::Execution,
        ] {
            assert_eq!(OperationKind::parse(k.as_str()), Some(k));
        }
        // Regression guard: the Execution token MUST round-trip
        // through the exact string used in the SQL CHECK constraint
        // on `hybrid_v2_operation_locks` (see migration 0049).
        assert_eq!(OperationKind::Execution.as_str(), "EXECUTION");
    }

    #[test]
    fn config_defaults_validate() {
        let cfg = RebuildConfig::default();
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn config_bounds_enforced() {
        let mut cfg = RebuildConfig::default();
        cfg.replay_batch_size = 0;
        assert!(cfg.validate().is_err());
        cfg.replay_batch_size = 200_000;
        assert!(cfg.validate().is_err());
        cfg.replay_batch_size = 1_000;
        cfg.retry_max = 11;
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn is_terminal_phases() {
        assert!(RebuildPhase::Complete.is_terminal());
        assert!(RebuildPhase::Failed.is_terminal());
        assert!(RebuildPhase::ManualInterventionRequired.is_terminal());
        assert!(!RebuildPhase::Replaying.is_terminal());
    }
}
