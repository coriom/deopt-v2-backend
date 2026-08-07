//! `BACKEND-HYBRID-V2-POSTGRES-PROJECTION-CORE-V1` — durable persistence
//! surface for the canonical Hybrid V2 projection.
//!
//! Canonicality rule (frozen):
//! `CHAIN_STATE_IS_CANONICAL_POSTGRES_IS_A_REBUILDABLE_NON_CANONICAL_PROJECTION`.
//!
//! Responsibilities:
//! - Register deployment identity (from the validated `ManifestParams`).
//! - Write one PostgreSQL transaction per canonical block, holding:
//!     * canonical block registration
//!     * raw log journal insert (idempotent on
//!       `(deployment, block_hash, tx_hash, log_index)`)
//!     * decoded event insert (idempotent on `raw_log_id`)
//!     * every projection mutation for the applied event set
//!     * cursor + readiness metadata advance
//! - Never independently reimplement an economic rule; SQL constraints
//!   enforce invariants (NOT NULL, PK uniqueness, non-negative counters)
//!   but the Rust reducer is the single semantic implementation.
//!
//! Boundary: this module does NOT own the runtime. `IndexerRuntime`
//! stays in-memory canonical per WP-01. The persistence engine
//! is invoked either through integration fixtures (Part K tests) or
//! through a future transitional dual-write path (Part J — deferred).
//!
//! Safety: no chain writes, no signer, no external calls apart from
//! `sqlx` against a caller-supplied `PgPool`.

use crate::error::{BackendError, Result};
use crate::hybrid_v2::correlation::ExecutionGroup;
use crate::hybrid_v2::events::{EventKind, HybridV2Event};
use crate::hybrid_v2::manifest::ManifestParams;
use crate::hybrid_v2::readiness::{ReadinessReason, ReadinessState};
use crate::hybrid_v2::reducer::{
    ApplyContext, ExecutionCompletion, FeeEventRow, ProjectionState, RecoveryStateProjection,
};
use crate::hybrid_v2::runtime::{JournaledLog, RuntimeCursor, RuntimeMetrics};
use serde_json::json;
use sqlx::postgres::PgPool;
use sqlx::{Postgres, Row, Transaction};
use std::collections::BTreeSet;

// -----------------------------------------------------------------
//                       PUBLIC TYPES
// -----------------------------------------------------------------

/// Reference to one canonical block. Persisted into
/// `hybrid_v2_canonical_blocks` and used as FK context for raw / decoded rows.
#[derive(Debug, Clone)]
pub struct CanonicalBlockRef {
    pub block_number: u64,
    pub block_hash: String,
    pub parent_hash: String,
    pub block_timestamp: u64,
}

/// Snapshot of a runtime cursor for persistence. Mirrors
/// `RuntimeCursor` + counters that live on the `hybrid_v2_cursors` row.
#[derive(Debug, Clone, Default)]
pub struct RuntimeCursorSnapshot {
    pub cursor_name: String,
    pub indexed_head_block: u64,
    pub indexed_head_hash: String,
    pub indexed_head_parent: String,
    pub observed_head_block: u64,
    pub finalized_head_block: u64,
    pub last_error: Option<String>,
    pub reorg_count: u64,
    pub max_reorg_depth_seen: u64,
    pub decode_failures: u64,
    pub projection_failures: u64,
    pub unknown_canonical_events: u64,
    pub last_success_block: u64,
}

impl RuntimeCursorSnapshot {
    pub fn from_runtime(name: &str, cursor: &RuntimeCursor, metrics: &RuntimeMetrics) -> Self {
        Self {
            cursor_name: name.to_string(),
            indexed_head_block: cursor.indexed_head_block,
            indexed_head_hash: cursor.indexed_head_hash.clone(),
            indexed_head_parent: cursor.indexed_head_parent.clone(),
            observed_head_block: cursor.observed_head_block,
            finalized_head_block: cursor.finalized_head_block,
            last_error: cursor.last_error.clone(),
            reorg_count: metrics.reorg_count,
            max_reorg_depth_seen: metrics.max_reorg_depth_seen,
            decode_failures: metrics.decode_failures,
            projection_failures: metrics.projection_failures,
            unknown_canonical_events: metrics.unknown_canonical_events,
            last_success_block: metrics.last_successful_block,
        }
    }
}

/// Snapshot of readiness for persistence.
#[derive(Debug, Clone)]
pub struct ReadinessSnapshot {
    pub ready: bool,
    pub reason: Option<String>,
    pub reason_detail: Option<String>,
}

impl ReadinessSnapshot {
    pub fn from_state(state: &ReadinessState) -> Self {
        let reason_name = state.reason.as_ref().map(reason_kind);
        let detail = state.reason.as_ref().map(reason_detail);
        Self {
            ready: state.ready,
            reason: reason_name,
            reason_detail: detail,
        }
    }
}

fn reason_kind(reason: &ReadinessReason) -> String {
    match reason {
        ReadinessReason::AwaitingFirstBlock => "AWAITING_FIRST_BLOCK".to_string(),
        ReadinessReason::WrongChain { .. } => "WRONG_CHAIN".to_string(),
        ReadinessReason::ManifestMismatch { .. } => "MANIFEST_MISMATCH".to_string(),
        ReadinessReason::UnknownCanonicalEvent { .. } => "UNKNOWN_CANONICAL_EVENT".to_string(),
        ReadinessReason::DecodeFailure { .. } => "DECODE_FAILURE".to_string(),
        ReadinessReason::ProjectionFailure { .. } => "PROJECTION_FAILURE".to_string(),
        ReadinessReason::CursorHashMismatch { .. } => "CURSOR_HASH_MISMATCH".to_string(),
        ReadinessReason::ExcessiveReorg { .. } => "EXCESSIVE_REORG".to_string(),
        ReadinessReason::RebuildInProgress => "REBUILD_IN_PROGRESS".to_string(),
        ReadinessReason::RebuildRequested { .. } => "REBUILD_REQUESTED".to_string(),
        ReadinessReason::RebuildFailed { .. } => "REBUILD_FAILED".to_string(),
        ReadinessReason::ReconciliationInProgress { .. } => {
            "RECONCILIATION_IN_PROGRESS".to_string()
        }
        ReadinessReason::ReconciliationDrift { .. } => "RECONCILIATION_DRIFT".to_string(),
        ReadinessReason::MigrationSchemaMismatch => "MIGRATION_SCHEMA_MISMATCH".to_string(),
        ReadinessReason::Behind { .. } => "BEHIND".to_string(),
        ReadinessReason::Bootstrapping => "BOOTSTRAPPING".to_string(),
        ReadinessReason::Stopping => "STOPPING".to_string(),
        ReadinessReason::ReorgDetected { .. } => "REORG_DETECTED".to_string(),
        ReadinessReason::ReorgSearching { .. } => "REORG_SEARCHING".to_string(),
        ReadinessReason::ReorgReplaying { .. } => "REORG_REPLAYING".to_string(),
        ReadinessReason::ReorgManualInterventionRequired { .. } => {
            "REORG_MANUAL_INTERVENTION_REQUIRED".to_string()
        }
    }
}

fn reason_detail(reason: &ReadinessReason) -> String {
    format!("{reason:?}")
}

/// Compact snapshot of ExecutionCorrelator pending groups suitable for
/// per-block persistence of correlation components.
#[derive(Debug, Clone, Default)]
pub struct CorrelationSnapshot {
    /// Fully-completed groups since last snapshot; the writer emits
    /// them into `hybrid_v2_matched_executions` when they arrive on the
    /// resulting `ProjectionState`.
    pub completed: Vec<String>,
    /// Currently-pending groups (unfinished correlation components). Not
    /// persisted directly — correlation state is transient — but exposed
    /// for observability and rebuild replay.
    pub pending: Vec<ExecutionGroup>,
}

// -----------------------------------------------------------------
//                       STORE TRAIT
// -----------------------------------------------------------------

/// Abstraction over the durable projection store.
///
/// Implementors must persist a block atomically: either every write in
/// the given block lands, or none does. There is no partial-block
/// state.
#[async_trait::async_trait]
pub trait HybridV2ProjectionStore: Send + Sync {
    /// Register the deployment identity from a validated manifest.
    /// Returns the deployment_id assigned by the store.
    async fn upsert_deployment(
        &self,
        manifest: &ManifestParams,
        activation_status: &str,
        now_ms: i64,
    ) -> Result<i64>;

    /// Persist one canonical block + its raw logs + decoded events +
    /// projection mutations + cursor advance in one atomic transaction.
    #[allow(clippy::too_many_arguments)]
    async fn persist_block_atomic(
        &self,
        deployment_id: i64,
        block: &CanonicalBlockRef,
        raw_logs: &[JournaledLog],
        decoded_events: &[(HybridV2Event, ApplyContext)],
        state: &ProjectionState,
        cursor: &RuntimeCursorSnapshot,
        readiness: &ReadinessSnapshot,
        now_ms: i64,
    ) -> Result<()>;

    /// Update per-deployment runtime metrics counters.
    async fn upsert_runtime_metrics(
        &self,
        deployment_id: i64,
        cursor: &RuntimeCursorSnapshot,
        now_ms: i64,
    ) -> Result<()>;

    /// Record a reorg event for observability.
    async fn record_reorg_event(
        &self,
        deployment_id: i64,
        detected_at_ms: i64,
        from_block: u64,
        from_hash: &str,
        to_block: u64,
        to_hash: &str,
        depth: u32,
        orphaned_log_count: u32,
    ) -> Result<()>;

    /// Look up the currently-persisted cursor snapshot (for restart).
    async fn read_cursor(
        &self,
        deployment_id: i64,
        cursor_name: &str,
    ) -> Result<Option<RuntimeCursorSnapshot>>;

    /// Look up the currently-persisted readiness snapshot (for restart).
    async fn read_readiness(&self, deployment_id: i64) -> Result<Option<ReadinessSnapshot>>;

    /// Read every canonical raw log for a deployment, in canonical
    /// order (`(block_number ASC, log_index ASC)`), used by
    /// `IndexerRuntime::bootstrap_from_persistence` to rehydrate the
    /// in-memory reducer state on restart via `RebuildService::replay_all`.
    ///
    /// SAFETY / RISK: currently unbounded. For deployments with
    /// millions of raw logs this will materialise every row into a
    /// `Vec<JournaledLog>`. A future stage (BOOTSTRAP-3) will add
    /// pagination; until then callers should only invoke this on
    /// restart and never inside a request handler.
    async fn read_canonical_journal(&self, deployment_id: i64) -> Result<Vec<JournaledLog>>;

    // -----------------------------------------------------------------
    //          PERSISTED REORG RECOVERY (BACKEND-HYBRID-V2-...
    //          PERSISTED-REORG-RECOVERY-V1)
    // -----------------------------------------------------------------

    /// Read the persisted reorg-recovery state row for a deployment.
    /// Returns `None` when no recovery has ever started for this
    /// deployment (fresh install).
    async fn read_reorg_recovery(
        &self,
        deployment_id: i64,
    ) -> Result<Option<crate::hybrid_v2::reorg_recovery::ReorgRecoveryState>>;

    /// Idempotent upsert of the recovery state row. Persists the
    /// full phase-machine snapshot so a subsequent bootstrap can
    /// resume from the exact same point.
    async fn upsert_reorg_recovery(
        &self,
        state: &crate::hybrid_v2::reorg_recovery::ReorgRecoveryState,
    ) -> Result<()>;

    /// Direct readiness upsert without going through the block-atomic
    /// path. The recovery service uses this to publish transient
    /// per-phase readiness reasons that map to hard 503 on the API.
    async fn write_readiness_snapshot(
        &self,
        deployment_id: i64,
        readiness: &ReadinessSnapshot,
        now_ms: i64,
    ) -> Result<()>;

    /// Atomic commit of the recovery mutation phase.
    ///
    /// One Postgres transaction that:
    ///   1. Marks orphan raw logs non-canonical (block_number > ancestor).
    ///   2. Marks orphan canonical blocks non-canonical.
    ///   3. Marks orphan matched executions INVALIDATED_BY_REORG.
    ///   4. Writes each replacement block + raw logs + decoded events +
    ///      projection mutations.
    ///   5. Advances the cursor to the replacement tip.
    ///   6. Updates readiness + metrics.
    ///   7. Marks the recovery row `phase = RECOVERED`.
    ///
    /// On any failure inside the transaction the entire batch rolls
    /// back and no persisted state changes — the caller retries.
    #[allow(clippy::too_many_arguments)]
    async fn commit_reorg_recovery(
        &self,
        deployment_id: i64,
        recovery_epoch: i64,
        ancestor_block: u64,
        ancestor_hash: &str,
        replacement_blocks: &[CanonicalBlockRef],
        replacement_raw_logs: &[JournaledLog],
        replacement_decoded_events: &[(HybridV2Event, ApplyContext)],
        projection_state: &ProjectionState,
        cursor: &RuntimeCursorSnapshot,
        readiness: &ReadinessSnapshot,
        now_ms: i64,
    ) -> Result<()>;

    // NOTE: `try_acquire_reorg_lock` + `release_reorg_lock` were
    // removed under
    // `BACKEND-HYBRID-V2-PROJECTION-PERSISTENCE-OPERATIONAL-CLOSURE-V1`.
    // Reorg now uses the unified `try_acquire_operation_lock(
    // OperationKind::Reorg, ...)` below. The legacy
    // `hybrid_v2_reorg_locks` table is retained (empty) — no code path
    // writes to it.

    // -----------------------------------------------------------------
    //  BACKEND-HYBRID-V2-PROJECTION-PERSISTENCE-CLOSURE-V1
    //  Unified deployment-scoped operation lock + persisted rebuild
    //  state machine + reconciliation results.
    //
    //  These methods land as a compatible extension: default bodies
    //  return `Err(BackendError::Persistence("unimplemented"))` so any
    //  external impl that has not been updated fails closed.
    // -----------------------------------------------------------------

    /// Attempt to acquire the unified deployment-scoped operation lock
    /// (`hybrid_v2_operation_locks`). Reorg + rebuild + reconciliation
    /// contend for the same row: at most one may hold the lock per
    /// deployment at any time.
    ///
    /// Returns `Some(guard)` on success, `None` on contention.
    async fn try_acquire_operation_lock(
        &self,
        _deployment_id: i64,
        _operation: crate::hybrid_v2::rebuild_operations::OperationKind,
        _holder_epoch: i64,
        _now_ms: i64,
    ) -> Result<Option<crate::hybrid_v2::rebuild_operations::OperationLockGuard>> {
        Err(BackendError::Persistence(
            "try_acquire_operation_lock unimplemented for this store".to_string(),
        ))
    }

    /// Release the unified deployment-scoped operation lock. Fencing:
    /// delete only when the recorded holder_epoch still matches.
    async fn release_operation_lock(&self, _deployment_id: i64, _holder_epoch: i64) -> Result<()> {
        Err(BackendError::Persistence(
            "release_operation_lock unimplemented for this store".to_string(),
        ))
    }

    /// Idempotent upsert of one row into `hybrid_v2_rebuild_operations`.
    async fn upsert_rebuild_operation(
        &self,
        _state: &crate::hybrid_v2::rebuild_operations::RebuildOperationState,
    ) -> Result<()> {
        Err(BackendError::Persistence(
            "upsert_rebuild_operation unimplemented for this store".to_string(),
        ))
    }

    /// Read one rebuild operation row by `(deployment_id, rebuild_epoch)`.
    async fn read_rebuild_operation(
        &self,
        _deployment_id: i64,
        _rebuild_epoch: i64,
    ) -> Result<Option<crate::hybrid_v2::rebuild_operations::RebuildOperationState>> {
        Err(BackendError::Persistence(
            "read_rebuild_operation unimplemented for this store".to_string(),
        ))
    }

    /// Read the most-recent rebuild operation row for a deployment
    /// (highest `rebuild_epoch`).
    async fn read_latest_rebuild_operation(
        &self,
        _deployment_id: i64,
    ) -> Result<Option<crate::hybrid_v2::rebuild_operations::RebuildOperationState>> {
        Err(BackendError::Persistence(
            "read_latest_rebuild_operation unimplemented for this store".to_string(),
        ))
    }

    /// Insert a reconciliation-result row. Returns the assigned
    /// `reconciliation_id`.
    async fn insert_reconciliation_result(
        &self,
        _record: &crate::hybrid_v2::reconciler::ReconciliationRecord,
    ) -> Result<i64> {
        Err(BackendError::Persistence(
            "insert_reconciliation_result unimplemented for this store".to_string(),
        ))
    }

    /// Read the most-recent reconciliation result for a deployment.
    async fn read_latest_reconciliation_result(
        &self,
        _deployment_id: i64,
    ) -> Result<Option<crate::hybrid_v2::reconciler::ReconciliationRecord>> {
        Err(BackendError::Persistence(
            "read_latest_reconciliation_result unimplemented for this store".to_string(),
        ))
    }

    /// Snapshot the currently-published projection state (for rebuild
    /// verification). Default returns `None` — implementors that own a
    /// projection state should override. Note the distinct name from
    /// `InMemoryProjectionStore::snapshot_state` (the inherent method
    /// on the concrete in-memory store) to avoid trait ambiguity.
    async fn snapshot_projection_state(
        &self,
        _deployment_id: i64,
    ) -> Result<Option<crate::hybrid_v2::reducer::ProjectionState>> {
        Ok(None)
    }

    /// Re-materialize the live projection tables from a rebuilt
    /// `ProjectionState`. Semantics (implemented by real stores):
    ///
    /// 1. Wipe the re-materializable projection slices for
    ///    `deployment_id` (balances, reservations, pause flags,
    ///    bad debt, recovery state, matched executions).
    /// 2. Reinsert every row from the rebuilt state.
    /// 3. Update cursor + readiness rows.
    /// 4. Mark the paired `hybrid_v2_rebuild_operations` row as
    ///    `Complete` (phase, verification_result, completed_at_ms).
    /// 5. All in a single transaction.
    ///
    /// The default stub returns
    /// `Err(BackendError::Persistence("unimplemented..."))` so any
    /// store that has not opted in fails closed.
    async fn commit_rematerialization(
        &self,
        _deployment_id: i64,
        _rebuild_epoch: i64,
        _state: &crate::hybrid_v2::reducer::ProjectionState,
        _cursor: &RuntimeCursorSnapshot,
        _readiness: &ReadinessSnapshot,
        _now_ms: i64,
    ) -> Result<()> {
        Err(BackendError::Persistence(
            "commit_rematerialization unimplemented for this store".to_string(),
        ))
    }
}

// -----------------------------------------------------------------
//                       POSTGRES ENGINE
// -----------------------------------------------------------------

/// PostgreSQL implementation of `HybridV2ProjectionStore`. Holds a
/// caller-supplied `PgPool`; each block-atomic write acquires a
/// transaction and commits or rolls back as a unit.
#[derive(Clone)]
pub struct PostgresHybridV2ProjectionStore {
    pool: PgPool,
}

impl PostgresHybridV2ProjectionStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }
}

#[async_trait::async_trait]
impl HybridV2ProjectionStore for PostgresHybridV2ProjectionStore {
    async fn upsert_deployment(
        &self,
        manifest: &ManifestParams,
        activation_status: &str,
        now_ms: i64,
    ) -> Result<i64> {
        let mut tx = self.pool.begin().await.map_err(pg_err)?;
        let existing = sqlx::query(
            "SELECT deployment_id FROM hybrid_v2_deployments
             WHERE chain_id = $1 AND manifest_hash = $2",
        )
        .bind(u64_to_i64("chain_id", manifest.chain_id)?)
        .bind(&manifest.manifest_hash)
        .fetch_optional(&mut *tx)
        .await
        .map_err(pg_err)?;
        let module_addresses_json = serde_json::to_value(&manifest.module_addresses)
            .map_err(|e| BackendError::Config(format!("module addresses json: {e}")))?;
        if let Some(row) = existing {
            let id: i64 = row.try_get("deployment_id").map_err(pg_err)?;
            sqlx::query(
                "UPDATE hybrid_v2_deployments SET
                    manifest_address = $2,
                    module_addresses_hash = $3,
                    critical_config_hash = $4,
                    architecture_version = $5,
                    storage_version = $6,
                    event_version = $7,
                    deployment_version = $8,
                    manifest_schema_version = $9,
                    environment_tag = $10,
                    deployer = $11,
                    deployment_block = $12,
                    deployment_timestamp = $13,
                    module_addresses_json = $14,
                    protocol_fee_subkey = $15,
                    rebate_budget_subkey = $16,
                    insurance_fund_subkey = $17,
                    max_collateral_tokens = $18,
                    max_active_series = $19,
                    all_capabilities_mask = $20,
                    recovery_activation_delay_seconds = $21,
                    recovery_pause_max_duration_blocks = $22,
                    activation_status = $23,
                    updated_at_ms = $24
                 WHERE deployment_id = $1",
            )
            .bind(id)
            .bind(&manifest.manifest_address)
            .bind(&manifest.module_addresses_hash)
            .bind(&manifest.critical_config_hash)
            .bind(manifest.architecture_version as i32)
            .bind(manifest.storage_version as i32)
            .bind(manifest.event_version as i32)
            .bind(manifest.deployment_version as i32)
            .bind(manifest.manifest_schema_version as i32)
            .bind(&manifest.environment_tag)
            .bind(&manifest.deployer)
            .bind(u64_to_i64("deployment_block", manifest.deployment_block)?)
            .bind(u64_to_i64(
                "deployment_timestamp",
                manifest.deployment_timestamp,
            )?)
            .bind(module_addresses_json)
            .bind(&manifest.protocol_fee_subkey)
            .bind(&manifest.rebate_budget_subkey)
            .bind(&manifest.insurance_fund_subkey)
            .bind(manifest.max_collateral_tokens as i32)
            .bind(manifest.max_active_series as i32)
            .bind(&manifest.all_capabilities_mask)
            .bind(u64_to_i64(
                "recovery_activation_delay_seconds",
                manifest.recovery_activation_delay_seconds,
            )?)
            .bind(u64_to_i64(
                "recovery_pause_max_duration_blocks",
                manifest.recovery_pause_max_duration_blocks,
            )?)
            .bind(activation_status)
            .bind(now_ms)
            .execute(&mut *tx)
            .await
            .map_err(pg_err)?;
            tx.commit().await.map_err(pg_err)?;
            return Ok(id);
        }
        let row = sqlx::query(
            "INSERT INTO hybrid_v2_deployments (
                chain_id, manifest_address, manifest_hash, module_addresses_hash,
                critical_config_hash, architecture_version, storage_version, event_version,
                deployment_version, manifest_schema_version, environment_tag, deployer,
                deployment_block, deployment_timestamp, module_addresses_json,
                protocol_fee_subkey, rebate_budget_subkey, insurance_fund_subkey,
                max_collateral_tokens, max_active_series, all_capabilities_mask,
                recovery_activation_delay_seconds, recovery_pause_max_duration_blocks,
                activation_status, created_at_ms, updated_at_ms
             ) VALUES (
                $1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,
                $19,$20,$21,$22,$23,$24,$25,$26
             ) RETURNING deployment_id",
        )
        .bind(u64_to_i64("chain_id", manifest.chain_id)?)
        .bind(&manifest.manifest_address)
        .bind(&manifest.manifest_hash)
        .bind(&manifest.module_addresses_hash)
        .bind(&manifest.critical_config_hash)
        .bind(manifest.architecture_version as i32)
        .bind(manifest.storage_version as i32)
        .bind(manifest.event_version as i32)
        .bind(manifest.deployment_version as i32)
        .bind(manifest.manifest_schema_version as i32)
        .bind(&manifest.environment_tag)
        .bind(&manifest.deployer)
        .bind(u64_to_i64("deployment_block", manifest.deployment_block)?)
        .bind(u64_to_i64(
            "deployment_timestamp",
            manifest.deployment_timestamp,
        )?)
        .bind(module_addresses_json)
        .bind(&manifest.protocol_fee_subkey)
        .bind(&manifest.rebate_budget_subkey)
        .bind(&manifest.insurance_fund_subkey)
        .bind(manifest.max_collateral_tokens as i32)
        .bind(manifest.max_active_series as i32)
        .bind(&manifest.all_capabilities_mask)
        .bind(u64_to_i64(
            "recovery_activation_delay_seconds",
            manifest.recovery_activation_delay_seconds,
        )?)
        .bind(u64_to_i64(
            "recovery_pause_max_duration_blocks",
            manifest.recovery_pause_max_duration_blocks,
        )?)
        .bind(activation_status)
        .bind(now_ms)
        .bind(now_ms)
        .fetch_one(&mut *tx)
        .await
        .map_err(pg_err)?;
        let id: i64 = row.try_get("deployment_id").map_err(pg_err)?;
        tx.commit().await.map_err(pg_err)?;
        Ok(id)
    }

    async fn persist_block_atomic(
        &self,
        deployment_id: i64,
        block: &CanonicalBlockRef,
        raw_logs: &[JournaledLog],
        decoded_events: &[(HybridV2Event, ApplyContext)],
        state: &ProjectionState,
        cursor: &RuntimeCursorSnapshot,
        readiness: &ReadinessSnapshot,
        now_ms: i64,
    ) -> Result<()> {
        let mut tx = self.pool.begin().await.map_err(pg_err)?;
        // 1. Register canonical block (idempotent on (deployment_id, block_hash)).
        upsert_canonical_block(&mut tx, deployment_id, block, now_ms).await?;
        // 2. Raw logs — capture generated ids so decoded events can FK them.
        let mut raw_ids: Vec<i64> = Vec::with_capacity(raw_logs.len());
        for log in raw_logs {
            let id = insert_raw_log(&mut tx, deployment_id, log, now_ms).await?;
            raw_ids.push(id);
        }
        // 3. Decoded events, FK on raw_log_id, matched positionally.
        for (i, (event, ctx)) in decoded_events.iter().enumerate() {
            let raw_id = raw_ids.get(i).copied().ok_or_else(|| {
                BackendError::Persistence("decoded/raw log count mismatch in block".to_string())
            })?;
            insert_decoded_event(&mut tx, deployment_id, raw_id, event, ctx, now_ms).await?;
            persist_event_projection(&mut tx, deployment_id, event, ctx, state, now_ms).await?;
        }
        // 4. Cursor advance, readiness, metrics — all keyed on deployment_id.
        upsert_cursor(&mut tx, deployment_id, cursor, now_ms).await?;
        upsert_runtime_metrics_tx(&mut tx, deployment_id, cursor, now_ms).await?;
        upsert_readiness(&mut tx, deployment_id, readiness, now_ms).await?;
        tx.commit().await.map_err(pg_err)?;
        Ok(())
    }

    async fn upsert_runtime_metrics(
        &self,
        deployment_id: i64,
        cursor: &RuntimeCursorSnapshot,
        now_ms: i64,
    ) -> Result<()> {
        let mut tx = self.pool.begin().await.map_err(pg_err)?;
        upsert_runtime_metrics_tx(&mut tx, deployment_id, cursor, now_ms).await?;
        tx.commit().await.map_err(pg_err)?;
        Ok(())
    }

    async fn record_reorg_event(
        &self,
        deployment_id: i64,
        detected_at_ms: i64,
        from_block: u64,
        from_hash: &str,
        to_block: u64,
        to_hash: &str,
        depth: u32,
        orphaned_log_count: u32,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO hybrid_v2_reorg_events (
                deployment_id, detected_at_ms, from_block, from_hash,
                to_block, to_hash, depth, orphaned_log_count
             ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8)",
        )
        .bind(deployment_id)
        .bind(detected_at_ms)
        .bind(u64_to_i64("from_block", from_block)?)
        .bind(from_hash)
        .bind(u64_to_i64("to_block", to_block)?)
        .bind(to_hash)
        .bind(depth as i32)
        .bind(orphaned_log_count as i32)
        .execute(&self.pool)
        .await
        .map_err(pg_err)?;
        Ok(())
    }

    async fn read_cursor(
        &self,
        deployment_id: i64,
        cursor_name: &str,
    ) -> Result<Option<RuntimeCursorSnapshot>> {
        let row = sqlx::query(
            "SELECT cursor_name, indexed_head_block, indexed_head_hash, indexed_head_parent,
                    observed_head_block, finalized_head_block, last_error,
                    reorg_count, max_reorg_depth_seen, decode_failures, projection_failures,
                    unknown_canonical_events, last_success_block
             FROM hybrid_v2_cursors
             WHERE deployment_id = $1 AND cursor_name = $2",
        )
        .bind(deployment_id)
        .bind(cursor_name)
        .fetch_optional(&self.pool)
        .await
        .map_err(pg_err)?;
        let Some(row) = row else { return Ok(None) };
        let indexed_head_block: i64 = row.try_get("indexed_head_block").map_err(pg_err)?;
        let observed_head_block: i64 = row.try_get("observed_head_block").map_err(pg_err)?;
        let finalized_head_block: i64 = row.try_get("finalized_head_block").map_err(pg_err)?;
        let reorg_count: i64 = row.try_get("reorg_count").map_err(pg_err)?;
        let max_reorg_depth_seen: i64 = row.try_get("max_reorg_depth_seen").map_err(pg_err)?;
        let decode_failures: i64 = row.try_get("decode_failures").map_err(pg_err)?;
        let projection_failures: i64 = row.try_get("projection_failures").map_err(pg_err)?;
        let unknown_canonical_events: i64 =
            row.try_get("unknown_canonical_events").map_err(pg_err)?;
        let last_success_block: i64 = row.try_get("last_success_block").map_err(pg_err)?;
        Ok(Some(RuntimeCursorSnapshot {
            cursor_name: row.try_get("cursor_name").map_err(pg_err)?,
            indexed_head_block: i64_to_u64("indexed_head_block", indexed_head_block)?,
            indexed_head_hash: row.try_get("indexed_head_hash").map_err(pg_err)?,
            indexed_head_parent: row.try_get("indexed_head_parent").map_err(pg_err)?,
            observed_head_block: i64_to_u64("observed_head_block", observed_head_block)?,
            finalized_head_block: i64_to_u64("finalized_head_block", finalized_head_block)?,
            last_error: row.try_get("last_error").map_err(pg_err)?,
            reorg_count: i64_to_u64("reorg_count", reorg_count)?,
            max_reorg_depth_seen: i64_to_u64("max_reorg_depth_seen", max_reorg_depth_seen)?,
            decode_failures: i64_to_u64("decode_failures", decode_failures)?,
            projection_failures: i64_to_u64("projection_failures", projection_failures)?,
            unknown_canonical_events: i64_to_u64(
                "unknown_canonical_events",
                unknown_canonical_events,
            )?,
            last_success_block: i64_to_u64("last_success_block", last_success_block)?,
        }))
    }

    async fn read_readiness(&self, deployment_id: i64) -> Result<Option<ReadinessSnapshot>> {
        let row = sqlx::query(
            "SELECT ready, reason, reason_detail
             FROM hybrid_v2_readiness WHERE deployment_id = $1",
        )
        .bind(deployment_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(pg_err)?;
        let Some(row) = row else { return Ok(None) };
        Ok(Some(ReadinessSnapshot {
            ready: row.try_get("ready").map_err(pg_err)?,
            reason: row.try_get("reason").map_err(pg_err)?,
            reason_detail: row.try_get("reason_detail").map_err(pg_err)?,
        }))
    }

    async fn read_canonical_journal(&self, deployment_id: i64) -> Result<Vec<JournaledLog>> {
        // NOTE(unbounded): this pulls every canonical raw log for the
        // deployment into memory. Bootstrap-only; not a request-path
        // primitive. See trait doc for the pagination follow-up plan.
        let rows = sqlx::query(
            "SELECT block_number, block_hash, parent_hash, block_timestamp,
                    tx_hash, tx_index, log_index, emitter, topics_json, data_hex
             FROM hybrid_v2_raw_logs
             WHERE deployment_id = $1 AND is_canonical = TRUE
             ORDER BY block_number ASC, log_index ASC",
        )
        .bind(deployment_id)
        .fetch_all(&self.pool)
        .await
        .map_err(pg_err)?;
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let block_number: i64 = row.try_get("block_number").map_err(pg_err)?;
            let block_hash: String = row.try_get("block_hash").map_err(pg_err)?;
            let parent_hash: String = row.try_get("parent_hash").map_err(pg_err)?;
            let block_timestamp: i64 = row.try_get("block_timestamp").map_err(pg_err)?;
            let tx_hash: String = row.try_get("tx_hash").map_err(pg_err)?;
            let tx_index: i32 = row.try_get("tx_index").map_err(pg_err)?;
            let log_index: i32 = row.try_get("log_index").map_err(pg_err)?;
            let emitter: String = row.try_get("emitter").map_err(pg_err)?;
            let topics_json: serde_json::Value = row.try_get("topics_json").map_err(pg_err)?;
            let data_hex: String = row.try_get("data_hex").map_err(pg_err)?;
            let topics = decode_topics_json(&topics_json)?;
            let data = decode_hex_string(&data_hex)?;
            out.push(JournaledLog {
                block_number: i64_to_u64("block_number", block_number)?,
                block_hash,
                parent_hash,
                block_timestamp: i64_to_u64("block_timestamp", block_timestamp)?,
                tx_hash,
                tx_index: tx_index.max(0) as u32,
                log_index: log_index.max(0) as u32,
                emitter,
                topics,
                data,
                is_canonical: true,
                orphaned_at_block: None,
            });
        }
        Ok(out)
    }

    // ---------------- REORG RECOVERY ----------------

    async fn read_reorg_recovery(
        &self,
        deployment_id: i64,
    ) -> Result<Option<crate::hybrid_v2::reorg_recovery::ReorgRecoveryState>> {
        let row = sqlx::query(
            "SELECT deployment_id, recovery_epoch, phase, detected_at_ms,
                    old_tip_block, old_tip_hash, conflicting_block, conflicting_hash,
                    common_ancestor_block, common_ancestor_hash,
                    replacement_tip_block, replacement_tip_hash,
                    orphan_depth, replacement_block_count,
                    finalized_at_detection, retry_count, last_failure_detail,
                    updated_at_ms, completed_at_ms
             FROM hybrid_v2_reorg_recovery WHERE deployment_id = $1",
        )
        .bind(deployment_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(pg_err)?;
        let Some(row) = row else { return Ok(None) };
        let phase_str: String = row.try_get("phase").map_err(pg_err)?;
        let phase = crate::hybrid_v2::reorg_recovery::ReorgRecoveryPhase::parse(&phase_str)
            .ok_or_else(|| BackendError::Persistence(format!("unknown phase {phase_str}")))?;
        let deployment_id: i64 = row.try_get("deployment_id").map_err(pg_err)?;
        let recovery_epoch: i64 = row.try_get("recovery_epoch").map_err(pg_err)?;
        let detected_at_ms: i64 = row.try_get("detected_at_ms").map_err(pg_err)?;
        let old_tip_block: i64 = row.try_get("old_tip_block").map_err(pg_err)?;
        let old_tip_hash: String = row.try_get("old_tip_hash").map_err(pg_err)?;
        let conflicting_block: Option<i64> = row.try_get("conflicting_block").map_err(pg_err)?;
        let conflicting_hash: Option<String> = row.try_get("conflicting_hash").map_err(pg_err)?;
        let common_ancestor_block: Option<i64> =
            row.try_get("common_ancestor_block").map_err(pg_err)?;
        let common_ancestor_hash: Option<String> =
            row.try_get("common_ancestor_hash").map_err(pg_err)?;
        let replacement_tip_block: Option<i64> =
            row.try_get("replacement_tip_block").map_err(pg_err)?;
        let replacement_tip_hash: Option<String> =
            row.try_get("replacement_tip_hash").map_err(pg_err)?;
        let orphan_depth: Option<i32> = row.try_get("orphan_depth").map_err(pg_err)?;
        let replacement_block_count: Option<i32> =
            row.try_get("replacement_block_count").map_err(pg_err)?;
        let finalized_at_detection: Option<i64> =
            row.try_get("finalized_at_detection").map_err(pg_err)?;
        let retry_count: i32 = row.try_get("retry_count").map_err(pg_err)?;
        let last_failure_detail: Option<String> =
            row.try_get("last_failure_detail").map_err(pg_err)?;
        let updated_at_ms: i64 = row.try_get("updated_at_ms").map_err(pg_err)?;
        let completed_at_ms: Option<i64> = row.try_get("completed_at_ms").map_err(pg_err)?;
        Ok(Some(crate::hybrid_v2::reorg_recovery::ReorgRecoveryState {
            deployment_id,
            recovery_epoch,
            phase,
            detected_at_ms,
            old_tip_block: i64_to_u64("old_tip_block", old_tip_block)?,
            old_tip_hash,
            conflicting_block: conflicting_block
                .map(|v| i64_to_u64("conflicting_block", v))
                .transpose()?,
            conflicting_hash,
            common_ancestor_block: common_ancestor_block
                .map(|v| i64_to_u64("common_ancestor_block", v))
                .transpose()?,
            common_ancestor_hash,
            replacement_tip_block: replacement_tip_block
                .map(|v| i64_to_u64("replacement_tip_block", v))
                .transpose()?,
            replacement_tip_hash,
            orphan_depth: orphan_depth.map(|v| v.max(0) as u32),
            replacement_block_count: replacement_block_count.map(|v| v.max(0) as u32),
            finalized_at_detection: finalized_at_detection
                .map(|v| i64_to_u64("finalized_at_detection", v))
                .transpose()?,
            retry_count: retry_count.max(0) as u32,
            last_failure_detail,
            updated_at_ms,
            completed_at_ms,
        }))
    }

    async fn upsert_reorg_recovery(
        &self,
        state: &crate::hybrid_v2::reorg_recovery::ReorgRecoveryState,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO hybrid_v2_reorg_recovery (
                deployment_id, recovery_epoch, phase, detected_at_ms,
                old_tip_block, old_tip_hash, conflicting_block, conflicting_hash,
                common_ancestor_block, common_ancestor_hash,
                replacement_tip_block, replacement_tip_hash,
                orphan_depth, replacement_block_count,
                finalized_at_detection, retry_count, last_failure_detail,
                updated_at_ms, completed_at_ms
             ) VALUES (
                $1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19
             )
             ON CONFLICT (deployment_id) DO UPDATE SET
                recovery_epoch = EXCLUDED.recovery_epoch,
                phase = EXCLUDED.phase,
                detected_at_ms = EXCLUDED.detected_at_ms,
                old_tip_block = EXCLUDED.old_tip_block,
                old_tip_hash = EXCLUDED.old_tip_hash,
                conflicting_block = EXCLUDED.conflicting_block,
                conflicting_hash = EXCLUDED.conflicting_hash,
                common_ancestor_block = EXCLUDED.common_ancestor_block,
                common_ancestor_hash = EXCLUDED.common_ancestor_hash,
                replacement_tip_block = EXCLUDED.replacement_tip_block,
                replacement_tip_hash = EXCLUDED.replacement_tip_hash,
                orphan_depth = EXCLUDED.orphan_depth,
                replacement_block_count = EXCLUDED.replacement_block_count,
                finalized_at_detection = EXCLUDED.finalized_at_detection,
                retry_count = EXCLUDED.retry_count,
                last_failure_detail = EXCLUDED.last_failure_detail,
                updated_at_ms = EXCLUDED.updated_at_ms,
                completed_at_ms = EXCLUDED.completed_at_ms",
        )
        .bind(state.deployment_id)
        .bind(state.recovery_epoch)
        .bind(state.phase.as_str())
        .bind(state.detected_at_ms)
        .bind(u64_to_i64("old_tip_block", state.old_tip_block)?)
        .bind(&state.old_tip_hash)
        .bind(
            state
                .conflicting_block
                .map(|v| u64_to_i64("conflicting_block", v))
                .transpose()?,
        )
        .bind(state.conflicting_hash.as_deref())
        .bind(
            state
                .common_ancestor_block
                .map(|v| u64_to_i64("common_ancestor_block", v))
                .transpose()?,
        )
        .bind(state.common_ancestor_hash.as_deref())
        .bind(
            state
                .replacement_tip_block
                .map(|v| u64_to_i64("replacement_tip_block", v))
                .transpose()?,
        )
        .bind(state.replacement_tip_hash.as_deref())
        .bind(state.orphan_depth.map(|v| v as i32))
        .bind(state.replacement_block_count.map(|v| v as i32))
        .bind(
            state
                .finalized_at_detection
                .map(|v| u64_to_i64("finalized_at_detection", v))
                .transpose()?,
        )
        .bind(state.retry_count as i32)
        .bind(state.last_failure_detail.as_deref())
        .bind(state.updated_at_ms)
        .bind(state.completed_at_ms)
        .execute(&self.pool)
        .await
        .map_err(pg_err)?;
        Ok(())
    }

    async fn write_readiness_snapshot(
        &self,
        deployment_id: i64,
        readiness: &ReadinessSnapshot,
        now_ms: i64,
    ) -> Result<()> {
        let mut tx = self.pool.begin().await.map_err(pg_err)?;
        upsert_readiness(&mut tx, deployment_id, readiness, now_ms).await?;
        tx.commit().await.map_err(pg_err)?;
        Ok(())
    }

    async fn commit_reorg_recovery(
        &self,
        deployment_id: i64,
        recovery_epoch: i64,
        ancestor_block: u64,
        _ancestor_hash: &str,
        replacement_blocks: &[CanonicalBlockRef],
        replacement_raw_logs: &[JournaledLog],
        replacement_decoded_events: &[(HybridV2Event, ApplyContext)],
        projection_state: &ProjectionState,
        cursor: &RuntimeCursorSnapshot,
        readiness: &ReadinessSnapshot,
        now_ms: i64,
    ) -> Result<()> {
        let mut tx = self.pool.begin().await.map_err(pg_err)?;
        let ancestor_i64 = u64_to_i64("ancestor_block", ancestor_block)?;

        // 1. Mark orphan raw logs.
        sqlx::query(
            "UPDATE hybrid_v2_raw_logs
             SET is_canonical = FALSE, orphaned_at_block = $2
             WHERE deployment_id = $1 AND block_number > $2 AND is_canonical = TRUE",
        )
        .bind(deployment_id)
        .bind(ancestor_i64)
        .execute(&mut *tx)
        .await
        .map_err(pg_err)?;

        // 2. Mark orphan canonical blocks.
        sqlx::query(
            "UPDATE hybrid_v2_canonical_blocks
             SET is_canonical = FALSE, orphaned_at_block = $2
             WHERE deployment_id = $1 AND block_number > $2 AND is_canonical = TRUE",
        )
        .bind(deployment_id)
        .bind(ancestor_i64)
        .execute(&mut *tx)
        .await
        .map_err(pg_err)?;

        // 3. Mark orphan matched executions.
        sqlx::query(
            "UPDATE hybrid_v2_matched_executions
             SET completion_status = 'INVALIDATED_BY_REORG'
             WHERE deployment_id = $1
               AND block_number > $2
               AND completion_status <> 'INVALIDATED_BY_REORG'",
        )
        .bind(deployment_id)
        .bind(ancestor_i64)
        .execute(&mut *tx)
        .await
        .map_err(pg_err)?;

        // 4. Write replacement blocks + raw logs + decoded events + projection mutations.
        let mut raw_ids: Vec<i64> = Vec::with_capacity(replacement_raw_logs.len());
        for block in replacement_blocks {
            upsert_canonical_block(&mut tx, deployment_id, block, now_ms).await?;
        }
        for log in replacement_raw_logs {
            let id = insert_raw_log(&mut tx, deployment_id, log, now_ms).await?;
            raw_ids.push(id);
        }
        for (i, (event, ctx)) in replacement_decoded_events.iter().enumerate() {
            let raw_id = raw_ids.get(i).copied().ok_or_else(|| {
                BackendError::Persistence(
                    "reorg commit: decoded/raw log count mismatch".to_string(),
                )
            })?;
            insert_decoded_event(&mut tx, deployment_id, raw_id, event, ctx, now_ms).await?;
            persist_event_projection(&mut tx, deployment_id, event, ctx, projection_state, now_ms)
                .await?;
        }

        // 5. Advance cursor + metrics + readiness.
        upsert_cursor(&mut tx, deployment_id, cursor, now_ms).await?;
        upsert_runtime_metrics_tx(&mut tx, deployment_id, cursor, now_ms).await?;
        upsert_readiness(&mut tx, deployment_id, readiness, now_ms).await?;

        // 6. Mark the recovery row RECOVERED atomically with the mutation.
        sqlx::query(
            "UPDATE hybrid_v2_reorg_recovery
             SET phase = 'RECOVERED',
                 completed_at_ms = $3,
                 updated_at_ms = $3
             WHERE deployment_id = $1 AND recovery_epoch = $2",
        )
        .bind(deployment_id)
        .bind(recovery_epoch)
        .bind(now_ms)
        .execute(&mut *tx)
        .await
        .map_err(pg_err)?;

        tx.commit().await.map_err(pg_err)?;
        Ok(())
    }

    // NOTE: The Postgres impls of `try_acquire_reorg_lock` /
    // `release_reorg_lock` were removed under
    // `BACKEND-HYBRID-V2-PROJECTION-PERSISTENCE-OPERATIONAL-CLOSURE-V1`.
    // Reorg now uses the unified `try_acquire_operation_lock(
    // OperationKind::Reorg, ...)` below. The legacy
    // `hybrid_v2_reorg_locks` table is retained (empty) so backups
    // remain readable.

    // -----------------------------------------------------------------
    //  CLOSURE V1 — unified operation lock, rebuild ops, reconciliation
    //  (BACKEND-HYBRID-V2-PROJECTION-PERSISTENCE-CLOSURE-V1)
    // -----------------------------------------------------------------

    async fn try_acquire_operation_lock(
        &self,
        deployment_id: i64,
        operation: crate::hybrid_v2::rebuild_operations::OperationKind,
        holder_epoch: i64,
        now_ms: i64,
    ) -> Result<Option<crate::hybrid_v2::rebuild_operations::OperationLockGuard>> {
        // Stale-lock cleanup: delete only when we can prove the prior
        // holder is terminal. NEVER treat any held lock as
        // unconditionally stale — that would let a competing operation
        // silently steal the lock mid-run.
        //   REORG          → paired reorg_recovery row in RECOVERED /
        //                    MANUAL_INTERVENTION_REQUIRED.
        //   REBUILD        → paired rebuild-op row in COMPLETE / FAILED /
        //                    MANUAL_INTERVENTION_REQUIRED.
        //   RECONCILIATION → no persisted state machine to consult, so
        //                    stale-lock cleanup does NOT run for this
        //                    operation. Explicit release
        //                    (`release_operation_lock`) is the only
        //                    path that clears a RECONCILIATION row. A
        //                    process crash mid-reconciliation therefore
        //                    holds the lock until operator restart —
        //                    accepted trade-off vs. silent stealing.
        sqlx::query(
            "DELETE FROM hybrid_v2_operation_locks
             WHERE deployment_id = $1
               AND (
                    (operation = 'REORG' AND EXISTS (
                        SELECT 1 FROM hybrid_v2_reorg_recovery r
                        WHERE r.deployment_id = $1
                          AND r.phase IN ('RECOVERED', 'MANUAL_INTERVENTION_REQUIRED')
                    ))
                 OR (operation = 'REBUILD' AND EXISTS (
                        SELECT 1 FROM hybrid_v2_rebuild_operations b
                        WHERE b.deployment_id = $1
                          AND b.phase IN ('COMPLETE', 'FAILED', 'MANUAL_INTERVENTION_REQUIRED')
                    ))
               )",
        )
        .bind(deployment_id)
        .execute(&self.pool)
        .await
        .map_err(pg_err)?;

        let inserted = sqlx::query(
            "INSERT INTO hybrid_v2_operation_locks
                (deployment_id, operation, holder_epoch, acquired_at_ms)
             VALUES ($1, $2, $3, $4)
             ON CONFLICT (deployment_id) DO NOTHING",
        )
        .bind(deployment_id)
        .bind(operation.as_str())
        .bind(holder_epoch)
        .bind(now_ms)
        .execute(&self.pool)
        .await
        .map_err(pg_err)?;
        if inserted.rows_affected() == 0 {
            return Ok(None);
        }
        Ok(Some(
            crate::hybrid_v2::rebuild_operations::OperationLockGuard {
                deployment_id,
                operation,
                holder_epoch,
                store: None,
            },
        ))
    }

    async fn release_operation_lock(&self, deployment_id: i64, holder_epoch: i64) -> Result<()> {
        sqlx::query(
            "DELETE FROM hybrid_v2_operation_locks
             WHERE deployment_id = $1 AND holder_epoch = $2",
        )
        .bind(deployment_id)
        .bind(holder_epoch)
        .execute(&self.pool)
        .await
        .map_err(pg_err)?;
        Ok(())
    }

    async fn upsert_rebuild_operation(
        &self,
        state: &crate::hybrid_v2::rebuild_operations::RebuildOperationState,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO hybrid_v2_rebuild_operations (
                deployment_id, rebuild_epoch, mode, phase, requested_at_ms,
                source_start_block, source_end_block, events_replayed,
                executions_correlated, verification_result, reconciliation_result,
                generation_id, retry_count, last_failure_detail,
                updated_at_ms, completed_at_ms
             ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16)
             ON CONFLICT (deployment_id, rebuild_epoch) DO UPDATE SET
                mode = EXCLUDED.mode,
                phase = EXCLUDED.phase,
                source_start_block = EXCLUDED.source_start_block,
                source_end_block = EXCLUDED.source_end_block,
                events_replayed = EXCLUDED.events_replayed,
                executions_correlated = EXCLUDED.executions_correlated,
                verification_result = EXCLUDED.verification_result,
                reconciliation_result = EXCLUDED.reconciliation_result,
                generation_id = EXCLUDED.generation_id,
                retry_count = EXCLUDED.retry_count,
                last_failure_detail = EXCLUDED.last_failure_detail,
                updated_at_ms = EXCLUDED.updated_at_ms,
                completed_at_ms = EXCLUDED.completed_at_ms",
        )
        .bind(state.deployment_id)
        .bind(state.rebuild_epoch)
        .bind(state.mode.as_str())
        .bind(state.phase.as_str())
        .bind(state.requested_at_ms)
        .bind(
            state
                .source_start_block
                .map(|v| u64_to_i64("source_start_block", v))
                .transpose()?,
        )
        .bind(
            state
                .source_end_block
                .map(|v| u64_to_i64("source_end_block", v))
                .transpose()?,
        )
        .bind(
            state
                .events_replayed
                .map(|v| u64_to_i64("events_replayed", v))
                .transpose()?,
        )
        .bind(
            state
                .executions_correlated
                .map(|v| u64_to_i64("executions_correlated", v))
                .transpose()?,
        )
        .bind(&state.verification_result)
        .bind(&state.reconciliation_result)
        .bind(state.generation_id)
        .bind(state.retry_count as i32)
        .bind(&state.last_failure_detail)
        .bind(state.updated_at_ms)
        .bind(state.completed_at_ms)
        .execute(&self.pool)
        .await
        .map_err(pg_err)?;
        Ok(())
    }

    async fn read_rebuild_operation(
        &self,
        deployment_id: i64,
        rebuild_epoch: i64,
    ) -> Result<Option<crate::hybrid_v2::rebuild_operations::RebuildOperationState>> {
        let row = sqlx::query(
            "SELECT deployment_id, rebuild_epoch, mode, phase, requested_at_ms,
                    source_start_block, source_end_block, events_replayed,
                    executions_correlated, verification_result, reconciliation_result,
                    generation_id, retry_count, last_failure_detail,
                    updated_at_ms, completed_at_ms
             FROM hybrid_v2_rebuild_operations
             WHERE deployment_id = $1 AND rebuild_epoch = $2",
        )
        .bind(deployment_id)
        .bind(rebuild_epoch)
        .fetch_optional(&self.pool)
        .await
        .map_err(pg_err)?;
        row.map(row_to_rebuild_operation).transpose()
    }

    async fn read_latest_rebuild_operation(
        &self,
        deployment_id: i64,
    ) -> Result<Option<crate::hybrid_v2::rebuild_operations::RebuildOperationState>> {
        let row = sqlx::query(
            "SELECT deployment_id, rebuild_epoch, mode, phase, requested_at_ms,
                    source_start_block, source_end_block, events_replayed,
                    executions_correlated, verification_result, reconciliation_result,
                    generation_id, retry_count, last_failure_detail,
                    updated_at_ms, completed_at_ms
             FROM hybrid_v2_rebuild_operations
             WHERE deployment_id = $1
             ORDER BY rebuild_epoch DESC
             LIMIT 1",
        )
        .bind(deployment_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(pg_err)?;
        row.map(row_to_rebuild_operation).transpose()
    }

    async fn insert_reconciliation_result(
        &self,
        record: &crate::hybrid_v2::reconciler::ReconciliationRecord,
    ) -> Result<i64> {
        let row = sqlx::query(
            "INSERT INTO hybrid_v2_reconciliation_results (
                deployment_id, ran_at_ms, block_number_checked, block_hash_checked,
                categories_checked, items_checked, converged_categories,
                divergent_categories, classification, mismatch_sample_json,
                provider_availability, failure_detail, duration_ms
             ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13)
             RETURNING reconciliation_id",
        )
        .bind(record.deployment_id)
        .bind(record.ran_at_ms)
        .bind(u64_to_i64(
            "block_number_checked",
            record.block_number_checked,
        )?)
        .bind(&record.block_hash_checked)
        .bind(record.categories_checked as i32)
        .bind(u64_to_i64("items_checked", record.items_checked)?)
        .bind(record.converged_categories as i32)
        .bind(record.divergent_categories as i32)
        .bind(record.classification.as_str())
        .bind(&record.mismatch_sample_json)
        .bind(record.provider_availability.as_str())
        .bind(&record.failure_detail)
        .bind(u64_to_i64("duration_ms", record.duration_ms)?)
        .fetch_one(&self.pool)
        .await
        .map_err(pg_err)?;
        Ok(row.try_get("reconciliation_id").map_err(pg_err)?)
    }

    async fn read_latest_reconciliation_result(
        &self,
        deployment_id: i64,
    ) -> Result<Option<crate::hybrid_v2::reconciler::ReconciliationRecord>> {
        let row = sqlx::query(
            "SELECT reconciliation_id, deployment_id, ran_at_ms, block_number_checked,
                    block_hash_checked, categories_checked, items_checked,
                    converged_categories, divergent_categories, classification,
                    mismatch_sample_json, provider_availability, failure_detail, duration_ms
             FROM hybrid_v2_reconciliation_results
             WHERE deployment_id = $1
             ORDER BY ran_at_ms DESC
             LIMIT 1",
        )
        .bind(deployment_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(pg_err)?;
        let Some(row) = row else { return Ok(None) };
        let classification_str: String = row.try_get("classification").map_err(pg_err)?;
        let availability_str: String = row.try_get("provider_availability").map_err(pg_err)?;
        let classification =
            crate::hybrid_v2::reconciler::DriftClassification::parse(&classification_str)
                .ok_or_else(|| {
                    BackendError::Persistence(format!(
                        "unknown reconciliation classification {classification_str}"
                    ))
                })?;
        let availability =
            crate::hybrid_v2::reconciler::ProviderAvailability::parse(&availability_str)
                .ok_or_else(|| {
                    BackendError::Persistence(format!(
                        "unknown provider availability {availability_str}"
                    ))
                })?;
        let block_number_checked: i64 = row.try_get("block_number_checked").map_err(pg_err)?;
        let items_checked: i64 = row.try_get("items_checked").map_err(pg_err)?;
        let categories_checked: i32 = row.try_get("categories_checked").map_err(pg_err)?;
        let converged_categories: i32 = row.try_get("converged_categories").map_err(pg_err)?;
        let divergent_categories: i32 = row.try_get("divergent_categories").map_err(pg_err)?;
        let duration_ms: i64 = row.try_get("duration_ms").map_err(pg_err)?;
        Ok(Some(crate::hybrid_v2::reconciler::ReconciliationRecord {
            reconciliation_id: Some(row.try_get("reconciliation_id").map_err(pg_err)?),
            deployment_id: row.try_get("deployment_id").map_err(pg_err)?,
            ran_at_ms: row.try_get("ran_at_ms").map_err(pg_err)?,
            block_number_checked: i64_to_u64("block_number_checked", block_number_checked)?,
            block_hash_checked: row.try_get("block_hash_checked").map_err(pg_err)?,
            categories_checked: categories_checked.max(0) as u32,
            items_checked: i64_to_u64("items_checked", items_checked)?,
            converged_categories: converged_categories.max(0) as u32,
            divergent_categories: divergent_categories.max(0) as u32,
            classification,
            mismatch_sample_json: row.try_get("mismatch_sample_json").ok(),
            provider_availability: availability,
            failure_detail: row.try_get("failure_detail").map_err(pg_err)?,
            duration_ms: i64_to_u64("duration_ms", duration_ms)?,
        }))
    }

    async fn snapshot_projection_state(
        &self,
        deployment_id: i64,
    ) -> Result<Option<crate::hybrid_v2::reducer::ProjectionState>> {
        // BACKEND-HYBRID-V2-PROJECTION-PERSISTENCE-OPERATIONAL-CLOSURE-V1:
        // Reconstruct a ProjectionState by reading the persisted
        // re-materializable projection tables. We only rebuild the
        // seven slices that own their storage in Postgres today:
        //   * hybrid_v2_vault_balances
        //   * hybrid_v2_reservations
        //   * hybrid_v2_pause_flags
        //   * hybrid_v2_bad_debt
        //   * hybrid_v2_recovery_state
        //   * hybrid_v2_matched_executions
        // Slices without per-projection tables (positions, orders,
        // capability grants, collateral universe, fee events, escape
        // state, recovery-epochs, recovery pause/withdrawal counts) are
        // read-only rederived from the journal at query time; drift on
        // them is caught by the rebuild's journal replay itself.
        use crate::hybrid_v2::reducer::{
            ExecutionCompletion, MatchedExecutionRow, ProjectionState, RecoveryStateProjection,
        };
        let mut state = ProjectionState::default();

        // Balances
        let bal_rows = sqlx::query(
            "SELECT subkey, token, balance FROM hybrid_v2_vault_balances
             WHERE deployment_id = $1",
        )
        .bind(deployment_id)
        .fetch_all(&self.pool)
        .await
        .map_err(pg_err)?;
        for row in bal_rows {
            let sk: String = row.try_get("subkey").map_err(pg_err)?;
            let tk: String = row.try_get("token").map_err(pg_err)?;
            let amt: String = row.try_get("balance").map_err(pg_err)?;
            state.balances.insert((sk, tk), amt);
        }

        // Reservations
        let res_rows = sqlx::query(
            "SELECT subkey, token, engine, reserved FROM hybrid_v2_reservations
             WHERE deployment_id = $1",
        )
        .bind(deployment_id)
        .fetch_all(&self.pool)
        .await
        .map_err(pg_err)?;
        for row in res_rows {
            let sk: String = row.try_get("subkey").map_err(pg_err)?;
            let tk: String = row.try_get("token").map_err(pg_err)?;
            let en: String = row.try_get("engine").map_err(pg_err)?;
            let amt: String = row.try_get("reserved").map_err(pg_err)?;
            state.reservations.insert((sk, tk, en), amt);
        }

        // Pause flags
        let pause_rows = sqlx::query(
            "SELECT subkey, paused FROM hybrid_v2_pause_flags
             WHERE deployment_id = $1",
        )
        .bind(deployment_id)
        .fetch_all(&self.pool)
        .await
        .map_err(pg_err)?;
        for row in pause_rows {
            let sk: String = row.try_get("subkey").map_err(pg_err)?;
            let p: bool = row.try_get("paused").map_err(pg_err)?;
            state.pause_flags.insert(sk, p);
        }

        // Bad debt
        let bd_rows = sqlx::query(
            "SELECT subkey, token, amount FROM hybrid_v2_bad_debt
             WHERE deployment_id = $1",
        )
        .bind(deployment_id)
        .fetch_all(&self.pool)
        .await
        .map_err(pg_err)?;
        for row in bd_rows {
            let sk: String = row.try_get("subkey").map_err(pg_err)?;
            let tk: String = row.try_get("token").map_err(pg_err)?;
            let amt: String = row.try_get("amount").map_err(pg_err)?;
            state.bad_debt.insert((sk, tk), amt);
        }

        // Recovery state
        let rs_rows = sqlx::query(
            "SELECT subkey, state FROM hybrid_v2_recovery_state
             WHERE deployment_id = $1",
        )
        .bind(deployment_id)
        .fetch_all(&self.pool)
        .await
        .map_err(pg_err)?;
        for row in rs_rows {
            let sk: String = row.try_get("subkey").map_err(pg_err)?;
            let st: String = row.try_get("state").map_err(pg_err)?;
            let proj = match st.as_str() {
                "NORMAL" => RecoveryStateProjection::Normal,
                "RECOVERY_PENDING" => RecoveryStateProjection::RecoveryPending,
                "RECOVERY_ACTIVE" => RecoveryStateProjection::RecoveryActive,
                "CANCELLED" => RecoveryStateProjection::Cancelled,
                "RECOVERED" => RecoveryStateProjection::Recovered,
                other => {
                    return Err(BackendError::Persistence(format!(
                        "unknown recovery_state value {other}"
                    )));
                }
            };
            state.recovery_state.insert(sk, proj);
        }

        // Matched executions
        let me_rows = sqlx::query(
            "SELECT execution_id, buyer_order_hash, seller_order_hash, buyer_subkey,
                    seller_subkey, series_id, matched_qty_1e8, premium_amount,
                    fee_amount, rebate_amount, block_number, tx_hash, completion_status
             FROM hybrid_v2_matched_executions
             WHERE deployment_id = $1",
        )
        .bind(deployment_id)
        .fetch_all(&self.pool)
        .await
        .map_err(pg_err)?;
        for row in me_rows {
            let exec_id: String = row.try_get("execution_id").map_err(pg_err)?;
            let block_number: i64 = row.try_get("block_number").map_err(pg_err)?;
            let cs: String = row.try_get("completion_status").map_err(pg_err)?;
            let completion = match cs.as_str() {
                "COMPLETE" => ExecutionCompletion::Complete,
                "INCOMPLETE" => ExecutionCompletion::Incomplete,
                "INVALIDATED_BY_REORG" => ExecutionCompletion::InvalidatedByReorg,
                other => {
                    return Err(BackendError::Persistence(format!(
                        "unknown completion_status value {other}"
                    )));
                }
            };
            state.matched_executions.insert(
                exec_id,
                MatchedExecutionRow {
                    buyer_order_hash: row.try_get("buyer_order_hash").map_err(pg_err)?,
                    seller_order_hash: row.try_get("seller_order_hash").map_err(pg_err)?,
                    buyer_subkey: row.try_get("buyer_subkey").map_err(pg_err)?,
                    seller_subkey: row.try_get("seller_subkey").map_err(pg_err)?,
                    series_id: row.try_get("series_id").map_err(pg_err)?,
                    matched_qty_1e8: row.try_get("matched_qty_1e8").map_err(pg_err)?,
                    premium_amount: row.try_get("premium_amount").map_err(pg_err)?,
                    fee_amount: row.try_get("fee_amount").map_err(pg_err)?,
                    rebate_amount: row.try_get("rebate_amount").map_err(pg_err)?,
                    block_number: i64_to_u64("block_number", block_number)?,
                    tx_hash: row.try_get("tx_hash").map_err(pg_err)?,
                    completion_status: completion,
                },
            );
        }

        Ok(Some(state))
    }

    async fn commit_rematerialization(
        &self,
        deployment_id: i64,
        rebuild_epoch: i64,
        state: &crate::hybrid_v2::reducer::ProjectionState,
        cursor: &RuntimeCursorSnapshot,
        readiness: &ReadinessSnapshot,
        now_ms: i64,
    ) -> Result<()> {
        // BACKEND-HYBRID-V2-PROJECTION-PERSISTENCE-OPERATIONAL-CLOSURE-V1:
        // Single-tx TRUNCATE-per-deployment + REINSERT + cursor/readiness/
        // rebuild-op update. Only the six re-materializable projection
        // slices are wiped. Rows in canonical tables (raw logs, decoded
        // events, canonical blocks) are untouched: they are the source
        // of truth used by the replayer.
        let mut tx = self.pool.begin().await.map_err(pg_err)?;

        // Wipe re-materializable projection rows for this deployment.
        for table in [
            "hybrid_v2_vault_balances",
            "hybrid_v2_reservations",
            "hybrid_v2_pause_flags",
            "hybrid_v2_bad_debt",
            "hybrid_v2_matched_executions",
        ] {
            let sql = format!("DELETE FROM {table} WHERE deployment_id = $1");
            sqlx::query(&sql)
                .bind(deployment_id)
                .execute(&mut *tx)
                .await
                .map_err(pg_err)?;
        }
        // Recovery state: wipe + reinsert.
        sqlx::query("DELETE FROM hybrid_v2_recovery_state WHERE deployment_id = $1")
            .bind(deployment_id)
            .execute(&mut *tx)
            .await
            .map_err(pg_err)?;

        // Reinsert balances.
        for ((subkey, token), amt) in state.balances.iter() {
            sqlx::query(
                "INSERT INTO hybrid_v2_vault_balances
                    (deployment_id, subkey, token, balance, last_event_block,
                     last_event_log_id, updated_at_ms)
                 VALUES ($1,$2,$3,$4,$5,$6,$7)",
            )
            .bind(deployment_id)
            .bind(subkey)
            .bind(token)
            .bind(amt)
            .bind(u64_to_i64("last_event_block", cursor.indexed_head_block)?)
            .bind(0_i64)
            .bind(now_ms)
            .execute(&mut *tx)
            .await
            .map_err(pg_err)?;
        }
        // Reservations.
        for ((subkey, token, engine), amt) in state.reservations.iter() {
            sqlx::query(
                "INSERT INTO hybrid_v2_reservations
                    (deployment_id, subkey, token, engine, reserved,
                     last_event_block, updated_at_ms)
                 VALUES ($1,$2,$3,$4,$5,$6,$7)",
            )
            .bind(deployment_id)
            .bind(subkey)
            .bind(token)
            .bind(engine)
            .bind(amt)
            .bind(u64_to_i64("last_event_block", cursor.indexed_head_block)?)
            .bind(now_ms)
            .execute(&mut *tx)
            .await
            .map_err(pg_err)?;
        }
        // Pause flags.
        for (subkey, paused) in state.pause_flags.iter() {
            sqlx::query(
                "INSERT INTO hybrid_v2_pause_flags
                    (deployment_id, subkey, paused, last_event_block, updated_at_ms)
                 VALUES ($1,$2,$3,$4,$5)",
            )
            .bind(deployment_id)
            .bind(subkey)
            .bind(*paused)
            .bind(u64_to_i64("last_event_block", cursor.indexed_head_block)?)
            .bind(now_ms)
            .execute(&mut *tx)
            .await
            .map_err(pg_err)?;
        }
        // Bad debt.
        for ((subkey, token), amt) in state.bad_debt.iter() {
            sqlx::query(
                "INSERT INTO hybrid_v2_bad_debt
                    (deployment_id, subkey, token, amount, last_event_block, updated_at_ms)
                 VALUES ($1,$2,$3,$4,$5,$6)",
            )
            .bind(deployment_id)
            .bind(subkey)
            .bind(token)
            .bind(amt)
            .bind(u64_to_i64("last_event_block", cursor.indexed_head_block)?)
            .bind(now_ms)
            .execute(&mut *tx)
            .await
            .map_err(pg_err)?;
        }
        // Recovery state.
        for (subkey, proj) in state.recovery_state.iter() {
            sqlx::query(
                "INSERT INTO hybrid_v2_recovery_state
                    (deployment_id, subkey, state, pending_since_ts, activation_eligible_at,
                     finalized_at_ts, tokens_withdrawn, state_updated_block, updated_at_ms)
                 VALUES ($1,$2,$3,NULL,NULL,NULL,0,$4,$5)",
            )
            .bind(deployment_id)
            .bind(subkey)
            .bind(proj.as_str())
            .bind(u64_to_i64(
                "state_updated_block",
                cursor.indexed_head_block,
            )?)
            .bind(now_ms)
            .execute(&mut *tx)
            .await
            .map_err(pg_err)?;
        }
        // Matched executions.
        for (exec_id, row) in state.matched_executions.iter() {
            let completion = match row.completion_status {
                crate::hybrid_v2::reducer::ExecutionCompletion::Complete => "COMPLETE",
                crate::hybrid_v2::reducer::ExecutionCompletion::Incomplete => "INCOMPLETE",
                crate::hybrid_v2::reducer::ExecutionCompletion::InvalidatedByReorg => {
                    "INVALIDATED_BY_REORG"
                }
            };
            sqlx::query(
                "INSERT INTO hybrid_v2_matched_executions (
                    deployment_id, execution_id, buyer_order_hash, seller_order_hash,
                    buyer_subkey, seller_subkey, series_id, matched_qty_1e8, premium_amount,
                    fee_amount, rebate_amount, block_number, tx_hash, completion_status,
                    completed_at_ms
                 ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15)",
            )
            .bind(deployment_id)
            .bind(exec_id)
            .bind(&row.buyer_order_hash)
            .bind(&row.seller_order_hash)
            .bind(&row.buyer_subkey)
            .bind(&row.seller_subkey)
            .bind(&row.series_id)
            .bind(&row.matched_qty_1e8)
            .bind(&row.premium_amount)
            .bind(&row.fee_amount)
            .bind(&row.rebate_amount)
            .bind(u64_to_i64("block_number", row.block_number)?)
            .bind(&row.tx_hash)
            .bind(completion)
            .bind(now_ms)
            .execute(&mut *tx)
            .await
            .map_err(pg_err)?;
        }

        // Update the cursor + readiness inside the same transaction.
        sqlx::query(
            "INSERT INTO hybrid_v2_cursors
                (deployment_id, cursor_name, indexed_head_block, indexed_head_hash,
                 indexed_head_parent, observed_head_block, finalized_head_block,
                 last_error, updated_at_ms, reorg_count, max_reorg_depth_seen,
                 decode_failures, projection_failures, unknown_canonical_events,
                 last_success_block)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15)
             ON CONFLICT (deployment_id, cursor_name) DO UPDATE SET
                indexed_head_block = EXCLUDED.indexed_head_block,
                indexed_head_hash = EXCLUDED.indexed_head_hash,
                indexed_head_parent = EXCLUDED.indexed_head_parent,
                observed_head_block = EXCLUDED.observed_head_block,
                finalized_head_block = EXCLUDED.finalized_head_block,
                last_error = EXCLUDED.last_error,
                updated_at_ms = EXCLUDED.updated_at_ms,
                reorg_count = EXCLUDED.reorg_count,
                max_reorg_depth_seen = EXCLUDED.max_reorg_depth_seen,
                decode_failures = EXCLUDED.decode_failures,
                projection_failures = EXCLUDED.projection_failures,
                unknown_canonical_events = EXCLUDED.unknown_canonical_events,
                last_success_block = EXCLUDED.last_success_block",
        )
        .bind(deployment_id)
        .bind(&cursor.cursor_name)
        .bind(u64_to_i64("indexed_head_block", cursor.indexed_head_block)?)
        .bind(&cursor.indexed_head_hash)
        .bind(&cursor.indexed_head_parent)
        .bind(u64_to_i64(
            "observed_head_block",
            cursor.observed_head_block,
        )?)
        .bind(u64_to_i64(
            "finalized_head_block",
            cursor.finalized_head_block,
        )?)
        .bind(&cursor.last_error)
        .bind(now_ms)
        .bind(u64_to_i64("reorg_count", cursor.reorg_count)?)
        .bind(u64_to_i64(
            "max_reorg_depth_seen",
            cursor.max_reorg_depth_seen,
        )?)
        .bind(u64_to_i64("decode_failures", cursor.decode_failures)?)
        .bind(u64_to_i64(
            "projection_failures",
            cursor.projection_failures,
        )?)
        .bind(u64_to_i64(
            "unknown_canonical_events",
            cursor.unknown_canonical_events,
        )?)
        .bind(u64_to_i64("last_success_block", cursor.last_success_block)?)
        .execute(&mut *tx)
        .await
        .map_err(pg_err)?;

        sqlx::query(
            "INSERT INTO hybrid_v2_readiness
                (deployment_id, ready, reason, reason_detail, updated_at_ms)
             VALUES ($1,$2,$3,$4,$5)
             ON CONFLICT (deployment_id) DO UPDATE SET
                ready = EXCLUDED.ready,
                reason = EXCLUDED.reason,
                reason_detail = EXCLUDED.reason_detail,
                updated_at_ms = EXCLUDED.updated_at_ms",
        )
        .bind(deployment_id)
        .bind(readiness.ready)
        .bind(&readiness.reason)
        .bind(&readiness.reason_detail)
        .bind(now_ms)
        .execute(&mut *tx)
        .await
        .map_err(pg_err)?;

        // Mark the rebuild op row Complete atomically.
        sqlx::query(
            "UPDATE hybrid_v2_rebuild_operations
             SET phase = 'COMPLETE',
                 verification_result = COALESCE(verification_result, 'REMATERIALIZED'),
                 completed_at_ms = $3,
                 updated_at_ms = $3
             WHERE deployment_id = $1 AND rebuild_epoch = $2",
        )
        .bind(deployment_id)
        .bind(rebuild_epoch)
        .bind(now_ms)
        .execute(&mut *tx)
        .await
        .map_err(pg_err)?;

        tx.commit().await.map_err(pg_err)?;
        Ok(())
    }
}

fn row_to_rebuild_operation(
    row: sqlx::postgres::PgRow,
) -> Result<crate::hybrid_v2::rebuild_operations::RebuildOperationState> {
    let mode_str: String = row.try_get("mode").map_err(pg_err)?;
    let phase_str: String = row.try_get("phase").map_err(pg_err)?;
    let mode = crate::hybrid_v2::rebuild_operations::RebuildMode::parse(&mode_str)
        .ok_or_else(|| BackendError::Persistence(format!("unknown rebuild mode {mode_str}")))?;
    let phase = crate::hybrid_v2::rebuild_operations::RebuildPhase::parse(&phase_str)
        .ok_or_else(|| BackendError::Persistence(format!("unknown rebuild phase {phase_str}")))?;
    let deployment_id: i64 = row.try_get("deployment_id").map_err(pg_err)?;
    let rebuild_epoch: i64 = row.try_get("rebuild_epoch").map_err(pg_err)?;
    let requested_at_ms: i64 = row.try_get("requested_at_ms").map_err(pg_err)?;
    let source_start_block: Option<i64> = row.try_get("source_start_block").map_err(pg_err)?;
    let source_end_block: Option<i64> = row.try_get("source_end_block").map_err(pg_err)?;
    let events_replayed: Option<i64> = row.try_get("events_replayed").map_err(pg_err)?;
    let executions_correlated: Option<i64> =
        row.try_get("executions_correlated").map_err(pg_err)?;
    let verification_result: Option<String> = row.try_get("verification_result").map_err(pg_err)?;
    let reconciliation_result: Option<String> =
        row.try_get("reconciliation_result").map_err(pg_err)?;
    let generation_id: Option<i64> = row.try_get("generation_id").map_err(pg_err)?;
    let retry_count: i32 = row.try_get("retry_count").map_err(pg_err)?;
    let last_failure_detail: Option<String> = row.try_get("last_failure_detail").map_err(pg_err)?;
    let updated_at_ms: i64 = row.try_get("updated_at_ms").map_err(pg_err)?;
    let completed_at_ms: Option<i64> = row.try_get("completed_at_ms").map_err(pg_err)?;
    Ok(
        crate::hybrid_v2::rebuild_operations::RebuildOperationState {
            deployment_id,
            rebuild_epoch,
            mode,
            phase,
            requested_at_ms,
            source_start_block: source_start_block
                .map(|v| i64_to_u64("source_start_block", v))
                .transpose()?,
            source_end_block: source_end_block
                .map(|v| i64_to_u64("source_end_block", v))
                .transpose()?,
            events_replayed: events_replayed
                .map(|v| i64_to_u64("events_replayed", v))
                .transpose()?,
            executions_correlated: executions_correlated
                .map(|v| i64_to_u64("executions_correlated", v))
                .transpose()?,
            verification_result,
            reconciliation_result,
            generation_id,
            retry_count: retry_count.max(0) as u32,
            last_failure_detail,
            updated_at_ms,
            completed_at_ms,
        },
    )
}

fn decode_topics_json(v: &serde_json::Value) -> Result<Vec<[u8; 32]>> {
    let arr = v
        .as_array()
        .ok_or_else(|| BackendError::Persistence("topics_json is not an array".to_string()))?;
    let mut out = Vec::with_capacity(arr.len());
    for entry in arr {
        let s = entry.as_str().ok_or_else(|| {
            BackendError::Persistence("topics_json entry is not a string".to_string())
        })?;
        let bytes = decode_hex_string(s)?;
        if bytes.len() != 32 {
            return Err(BackendError::Persistence(format!(
                "topics_json entry length {} != 32",
                bytes.len()
            )));
        }
        let mut arr32 = [0u8; 32];
        arr32.copy_from_slice(&bytes);
        out.push(arr32);
    }
    Ok(out)
}

fn decode_hex_string(s: &str) -> Result<Vec<u8>> {
    let stripped = s.strip_prefix("0x").unwrap_or(s);
    if stripped.len() % 2 != 0 {
        return Err(BackendError::Persistence(format!(
            "hex string has odd length: {}",
            stripped.len()
        )));
    }
    let mut out = Vec::with_capacity(stripped.len() / 2);
    let bytes = stripped.as_bytes();
    for i in (0..bytes.len()).step_by(2) {
        let hi = hex_nibble(bytes[i])?;
        let lo = hex_nibble(bytes[i + 1])?;
        out.push((hi << 4) | lo);
    }
    Ok(out)
}

fn hex_nibble(b: u8) -> Result<u8> {
    match b {
        b'0'..=b'9' => Ok(b - b'0'),
        b'a'..=b'f' => Ok(b - b'a' + 10),
        b'A'..=b'F' => Ok(b - b'A' + 10),
        _ => Err(BackendError::Persistence(format!(
            "invalid hex nibble: {}",
            b as char
        ))),
    }
}

// -----------------------------------------------------------------
//                       BLOCK-LEVEL WRITERS
// -----------------------------------------------------------------

async fn upsert_canonical_block(
    tx: &mut Transaction<'_, Postgres>,
    deployment_id: i64,
    block: &CanonicalBlockRef,
    now_ms: i64,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO hybrid_v2_canonical_blocks (
            deployment_id, block_hash, block_number, parent_hash, block_timestamp,
            is_canonical, applied_at_ms
         ) VALUES ($1,$2,$3,$4,$5,TRUE,$6)
         ON CONFLICT (deployment_id, block_hash) DO UPDATE SET
            block_number = EXCLUDED.block_number,
            parent_hash = EXCLUDED.parent_hash,
            block_timestamp = EXCLUDED.block_timestamp,
            is_canonical = TRUE,
            orphaned_at_block = NULL,
            applied_at_ms = EXCLUDED.applied_at_ms",
    )
    .bind(deployment_id)
    .bind(&block.block_hash)
    .bind(u64_to_i64("block_number", block.block_number)?)
    .bind(&block.parent_hash)
    .bind(u64_to_i64("block_timestamp", block.block_timestamp)?)
    .bind(now_ms)
    .execute(&mut **tx)
    .await
    .map_err(pg_err)?;
    Ok(())
}

async fn insert_raw_log(
    tx: &mut Transaction<'_, Postgres>,
    deployment_id: i64,
    log: &JournaledLog,
    now_ms: i64,
) -> Result<i64> {
    let topics_json = serde_json::Value::Array(
        log.topics
            .iter()
            .map(|t| serde_json::Value::String(format!("0x{}", encode_hex_bytes(t))))
            .collect(),
    );
    let topic0 = log
        .topics
        .first()
        .map(|t| format!("0x{}", encode_hex_bytes(t)))
        .unwrap_or_default();
    let row = sqlx::query(
        "INSERT INTO hybrid_v2_raw_logs (
            deployment_id, block_number, block_hash, parent_hash, block_timestamp,
            tx_hash, tx_index, log_index, emitter, topic0, topics_json,
            data_hex, is_canonical, orphaned_at_block, created_at_ms
         ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15)
         ON CONFLICT (deployment_id, block_hash, tx_hash, log_index)
         DO UPDATE SET
            is_canonical = EXCLUDED.is_canonical,
            orphaned_at_block = EXCLUDED.orphaned_at_block
         RETURNING raw_log_id",
    )
    .bind(deployment_id)
    .bind(u64_to_i64("block_number", log.block_number)?)
    .bind(&log.block_hash)
    .bind(&log.parent_hash)
    .bind(u64_to_i64("block_timestamp", log.block_timestamp)?)
    .bind(&log.tx_hash)
    .bind(log.tx_index as i32)
    .bind(log.log_index as i32)
    .bind(&log.emitter)
    .bind(&topic0)
    .bind(topics_json)
    .bind(format!("0x{}", encode_hex_bytes(&log.data)))
    .bind(log.is_canonical)
    .bind(
        log.orphaned_at_block
            .map(|b| u64_to_i64("orphaned_at_block", b))
            .transpose()?,
    )
    .bind(now_ms)
    .fetch_one(&mut **tx)
    .await
    .map_err(pg_err)?;
    let id: i64 = row.try_get("raw_log_id").map_err(pg_err)?;
    Ok(id)
}

async fn insert_decoded_event(
    tx: &mut Transaction<'_, Postgres>,
    deployment_id: i64,
    raw_log_id: i64,
    event: &HybridV2Event,
    _ctx: &ApplyContext,
    now_ms: i64,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO hybrid_v2_decoded_events (
            raw_log_id, deployment_id, event_kind, event_version, module_slot,
            subkey, owner, subaccount_id, token, engine, execution_id,
            order_hash, series_id, payload_json, applied, decoder_version, created_at_ms
         ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17)
         ON CONFLICT (raw_log_id) DO UPDATE SET applied = EXCLUDED.applied",
    )
    .bind(raw_log_id)
    .bind(deployment_id)
    .bind(event.kind.as_str())
    .bind(event.event_version as i32)
    .bind::<Option<i32>>(None)
    .bind(event.subkey.as_deref())
    .bind(event.owner.as_deref())
    .bind(event.subaccount_id.map(|v| v as i64))
    .bind(event.token.as_deref())
    .bind(event.engine.as_deref())
    .bind(event.execution_id.as_deref())
    .bind(event.order_hash.as_deref())
    .bind(event.series_id.as_deref())
    .bind(&event.payload)
    .bind(true)
    .bind(1_i32)
    .bind(now_ms)
    .execute(&mut **tx)
    .await
    .map_err(pg_err)?;
    Ok(())
}

async fn upsert_cursor(
    tx: &mut Transaction<'_, Postgres>,
    deployment_id: i64,
    cursor: &RuntimeCursorSnapshot,
    now_ms: i64,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO hybrid_v2_cursors (
            deployment_id, cursor_name, indexed_head_block, indexed_head_hash,
            indexed_head_parent, observed_head_block, finalized_head_block,
            last_advanced_at_ms, last_error, reorg_count, max_reorg_depth_seen,
            decode_failures, projection_failures, unknown_canonical_events,
            last_success_block
         ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15)
         ON CONFLICT (deployment_id, cursor_name) DO UPDATE SET
            indexed_head_block = EXCLUDED.indexed_head_block,
            indexed_head_hash = EXCLUDED.indexed_head_hash,
            indexed_head_parent = EXCLUDED.indexed_head_parent,
            observed_head_block = EXCLUDED.observed_head_block,
            finalized_head_block = EXCLUDED.finalized_head_block,
            last_advanced_at_ms = EXCLUDED.last_advanced_at_ms,
            last_error = EXCLUDED.last_error,
            reorg_count = EXCLUDED.reorg_count,
            max_reorg_depth_seen = EXCLUDED.max_reorg_depth_seen,
            decode_failures = EXCLUDED.decode_failures,
            projection_failures = EXCLUDED.projection_failures,
            unknown_canonical_events = EXCLUDED.unknown_canonical_events,
            last_success_block = EXCLUDED.last_success_block",
    )
    .bind(deployment_id)
    .bind(&cursor.cursor_name)
    .bind(u64_to_i64("indexed_head_block", cursor.indexed_head_block)?)
    .bind(&cursor.indexed_head_hash)
    .bind(&cursor.indexed_head_parent)
    .bind(u64_to_i64(
        "observed_head_block",
        cursor.observed_head_block,
    )?)
    .bind(u64_to_i64(
        "finalized_head_block",
        cursor.finalized_head_block,
    )?)
    .bind(now_ms)
    .bind(cursor.last_error.as_deref())
    .bind(u64_to_i64("reorg_count", cursor.reorg_count)?)
    .bind(u64_to_i64(
        "max_reorg_depth_seen",
        cursor.max_reorg_depth_seen,
    )?)
    .bind(u64_to_i64("decode_failures", cursor.decode_failures)?)
    .bind(u64_to_i64(
        "projection_failures",
        cursor.projection_failures,
    )?)
    .bind(u64_to_i64(
        "unknown_canonical_events",
        cursor.unknown_canonical_events,
    )?)
    .bind(u64_to_i64("last_success_block", cursor.last_success_block)?)
    .execute(&mut **tx)
    .await
    .map_err(pg_err)?;
    Ok(())
}

async fn upsert_runtime_metrics_tx(
    tx: &mut Transaction<'_, Postgres>,
    deployment_id: i64,
    cursor: &RuntimeCursorSnapshot,
    now_ms: i64,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO hybrid_v2_runtime_metrics (
            deployment_id, reorg_count, max_reorg_depth_seen, decode_failures,
            projection_failures, unknown_canonical_events, last_success_block, updated_at_ms
         ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8)
         ON CONFLICT (deployment_id) DO UPDATE SET
            reorg_count = EXCLUDED.reorg_count,
            max_reorg_depth_seen = EXCLUDED.max_reorg_depth_seen,
            decode_failures = EXCLUDED.decode_failures,
            projection_failures = EXCLUDED.projection_failures,
            unknown_canonical_events = EXCLUDED.unknown_canonical_events,
            last_success_block = EXCLUDED.last_success_block,
            updated_at_ms = EXCLUDED.updated_at_ms",
    )
    .bind(deployment_id)
    .bind(u64_to_i64("reorg_count", cursor.reorg_count)?)
    .bind(u64_to_i64(
        "max_reorg_depth_seen",
        cursor.max_reorg_depth_seen,
    )?)
    .bind(u64_to_i64("decode_failures", cursor.decode_failures)?)
    .bind(u64_to_i64(
        "projection_failures",
        cursor.projection_failures,
    )?)
    .bind(u64_to_i64(
        "unknown_canonical_events",
        cursor.unknown_canonical_events,
    )?)
    .bind(u64_to_i64("last_success_block", cursor.last_success_block)?)
    .bind(now_ms)
    .execute(&mut **tx)
    .await
    .map_err(pg_err)?;
    Ok(())
}

async fn upsert_readiness(
    tx: &mut Transaction<'_, Postgres>,
    deployment_id: i64,
    readiness: &ReadinessSnapshot,
    now_ms: i64,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO hybrid_v2_readiness (
            deployment_id, ready, reason, reason_detail, updated_at_ms
         ) VALUES ($1,$2,$3,$4,$5)
         ON CONFLICT (deployment_id) DO UPDATE SET
            ready = EXCLUDED.ready,
            reason = EXCLUDED.reason,
            reason_detail = EXCLUDED.reason_detail,
            updated_at_ms = EXCLUDED.updated_at_ms",
    )
    .bind(deployment_id)
    .bind(readiness.ready)
    .bind(readiness.reason.as_deref())
    .bind(readiness.reason_detail.as_deref())
    .bind(now_ms)
    .execute(&mut **tx)
    .await
    .map_err(pg_err)?;
    Ok(())
}

// -----------------------------------------------------------------
//              PER-EVENT PROJECTION DISPATCH (46 kinds)
// -----------------------------------------------------------------

async fn persist_event_projection(
    tx: &mut Transaction<'_, Postgres>,
    deployment_id: i64,
    event: &HybridV2Event,
    ctx: &ApplyContext,
    state: &ProjectionState,
    now_ms: i64,
) -> Result<()> {
    match event.kind {
        // ---- IDENTITY ----
        EventKind::SubaccountCreated => {
            let subkey = req(event.subkey.as_deref(), "SubaccountCreated.subkey")?;
            let owner = req(event.owner.as_deref(), "SubaccountCreated.owner")?;
            let sid = event
                .subaccount_id
                .ok_or_else(|| miss("SubaccountCreated.subaccount_id"))?;
            upsert_subaccount(
                tx,
                deployment_id,
                subkey,
                owner,
                sid,
                "CREATED",
                ctx,
                now_ms,
            )
            .await?;
        }
        EventKind::SubaccountLazyRegistered => {
            let subkey = req(event.subkey.as_deref(), "SubaccountLazyRegistered.subkey")?;
            let owner = req(event.owner.as_deref(), "SubaccountLazyRegistered.owner")?;
            let sid = event
                .subaccount_id
                .ok_or_else(|| miss("SubaccountLazyRegistered.subaccount_id"))?;
            upsert_subaccount(tx, deployment_id, subkey, owner, sid, "LAZY", ctx, now_ms).await?;
        }

        // ---- VAULT BALANCES ----
        EventKind::Deposit | EventKind::Withdraw => {
            let subkey = req(event.subkey.as_deref(), "Deposit/Withdraw.subkey")?;
            let token = req(event.token.as_deref(), "Deposit/Withdraw.token")?;
            let balance = state
                .balances
                .get(&(subkey.to_string(), token.to_string()))
                .cloned()
                .unwrap_or_else(|| "0".into());
            upsert_vault_balance(
                tx,
                deployment_id,
                subkey,
                token,
                &balance,
                ctx.block_number,
                0,
                now_ms,
            )
            .await?;
        }
        EventKind::InternalTransfer => {
            let token = req(event.token.as_deref(), "InternalTransfer.token")?;
            let from = event.payload.get("from_subkey").and_then(|v| v.as_str());
            let to = event.payload.get("to_subkey").and_then(|v| v.as_str());
            for subkey_opt in [from, to] {
                if let Some(subkey) = subkey_opt {
                    let balance = state
                        .balances
                        .get(&(subkey.to_string(), token.to_string()))
                        .cloned()
                        .unwrap_or_else(|| "0".into());
                    upsert_vault_balance(
                        tx,
                        deployment_id,
                        subkey,
                        token,
                        &balance,
                        ctx.block_number,
                        0,
                        now_ms,
                    )
                    .await?;
                }
            }
        }

        // ---- RESERVATIONS ----
        EventKind::CollateralLocked
        | EventKind::CollateralUnlocked
        | EventKind::OrphanedLockReleased => {
            let subkey = req(event.subkey.as_deref(), "reservation.subkey")?;
            let token = req(event.token.as_deref(), "reservation.token")?;
            let engine = req(event.engine.as_deref(), "reservation.engine")?;
            let reserved = state
                .reservations
                .get(&(subkey.to_string(), token.to_string(), engine.to_string()))
                .cloned()
                .unwrap_or_else(|| "0".into());
            upsert_reservation(
                tx,
                deployment_id,
                subkey,
                token,
                engine,
                &reserved,
                ctx.block_number,
                now_ms,
            )
            .await?;
        }

        // ---- COLLATERAL UNIVERSE ----
        EventKind::CollateralTokenEnteredUniverse => {
            let token = req(
                event.token.as_deref(),
                "CollateralTokenEnteredUniverse.token",
            )?;
            let idx = state
                .collateral_universe
                .get(token)
                .copied()
                .unwrap_or_default();
            let enabled = !state.disabled_tokens.contains(token);
            upsert_collateral_universe(
                tx,
                deployment_id,
                token,
                idx,
                enabled,
                ctx.block_number,
                now_ms,
            )
            .await?;
        }
        EventKind::SupportedTokenAdded => {
            if let Some(token) = event.token.as_deref() {
                let enabled = !state.disabled_tokens.contains(token);
                if let Some(idx) = state.collateral_universe.get(token) {
                    upsert_collateral_universe(
                        tx,
                        deployment_id,
                        token,
                        *idx,
                        enabled,
                        ctx.block_number,
                        now_ms,
                    )
                    .await?;
                } else {
                    // Token was flagged supported but never entered the universe yet.
                    // The universe insert is deferred until CollateralTokenEnteredUniverse.
                }
            }
        }
        EventKind::SupportedTokenRemoved => {
            if let Some(token) = event.token.as_deref() {
                if let Some(idx) = state.collateral_universe.get(token) {
                    upsert_collateral_universe(
                        tx,
                        deployment_id,
                        token,
                        *idx,
                        false,
                        ctx.block_number,
                        now_ms,
                    )
                    .await?;
                }
            }
        }

        // ---- CAPABILITY / GUARDIAN ----
        EventKind::EngineCapabilityChanged => {
            let engine = req(event.engine.as_deref(), "EngineCapabilityChanged.engine")?;
            let bits = state
                .capability_grants
                .get(engine)
                .cloned()
                .unwrap_or_else(|| "0".into());
            upsert_capability(tx, deployment_id, engine, &bits, now_ms).await?;
        }
        EventKind::EngineGuardianRevoked | EventKind::GuardianChanged => {
            // Audit-trail-only in ProjectionState; journalled in decoded_events already.
        }

        // ---- PAUSE / BAD DEBT ----
        EventKind::PauseFlagChanged => {
            let subkey = req(event.subkey.as_deref(), "PauseFlagChanged.subkey")?;
            let paused = state.pause_flags.get(subkey).copied().unwrap_or(false);
            upsert_pause_flag(tx, deployment_id, subkey, paused, ctx.block_number, now_ms).await?;
        }
        EventKind::BadDebtSocialized => {
            let subkey = req(event.subkey.as_deref(), "BadDebtSocialized.subkey")?;
            let token = req(event.token.as_deref(), "BadDebtSocialized.token")?;
            let amount = state
                .bad_debt
                .get(&(subkey.to_string(), token.to_string()))
                .cloned()
                .unwrap_or_else(|| "0".into());
            upsert_bad_debt(
                tx,
                deployment_id,
                subkey,
                token,
                &amount,
                ctx.block_number,
                now_ms,
            )
            .await?;
        }

        // ---- INITIALISATION EVIDENCE / RISK MODULE (journal-only) ----
        EventKind::ProtocolSubaccountsInitialized
        | EventKind::EscapeControllerInitialized
        | EventKind::RecoveryFinalizerInitialized
        | EventKind::DeploymentManifestDeclared
        | EventKind::RiskParamsSet
        | EventKind::RiskModuleActivated
        | EventKind::LiquidationTriggered
        | EventKind::IntentConsumed => {
            // Persisted via hybrid_v2_decoded_events only; no projection mutation.
        }

        // ---- POSITIONS ----
        EventKind::OptionPositionOpened | EventKind::OptionPositionModified => {
            let subkey = req(event.subkey.as_deref(), "position.subkey")?;
            let series = req(event.series_id.as_deref(), "position.series_id")?;
            let key = (subkey.to_string(), series.to_string());
            if let Some(row) = state.positions.get(&key) {
                upsert_position(
                    tx,
                    deployment_id,
                    subkey,
                    series,
                    &row.long_qty_1e8,
                    &row.short_qty_1e8,
                    ctx.block_number,
                    now_ms,
                )
                .await?;
                if state
                    .active_series
                    .get(subkey)
                    .map(|s| s.contains(series))
                    .unwrap_or(false)
                {
                    upsert_active_series(tx, deployment_id, subkey, series, ctx.block_number)
                        .await?;
                } else {
                    delete_active_series(tx, deployment_id, subkey, series).await?;
                }
            } else {
                delete_position(tx, deployment_id, subkey, series).await?;
                delete_active_series(tx, deployment_id, subkey, series).await?;
            }
        }
        EventKind::OptionPositionClosed | EventKind::OptionPositionLiquidated => {
            let subkey = req(event.subkey.as_deref(), "position-close.subkey")?;
            let series = req(event.series_id.as_deref(), "position-close.series_id")?;
            let key = (subkey.to_string(), series.to_string());
            if let Some(row) = state.positions.get(&key) {
                upsert_position(
                    tx,
                    deployment_id,
                    subkey,
                    series,
                    &row.long_qty_1e8,
                    &row.short_qty_1e8,
                    ctx.block_number,
                    now_ms,
                )
                .await?;
            } else {
                delete_position(tx, deployment_id, subkey, series).await?;
            }
            if state
                .active_series
                .get(subkey)
                .map(|s| s.contains(series))
                .unwrap_or(false)
            {
                upsert_active_series(tx, deployment_id, subkey, series, ctx.block_number).await?;
            } else {
                delete_active_series(tx, deployment_id, subkey, series).await?;
            }
        }
        EventKind::OptionExercised | EventKind::OptionSettled => {
            // Exercise/settle burns are journal-only; the followup
            // OptionPositionModified + Deposit/Withdraw carry the numeric
            // mutation. Persisted via decoded_events.
        }

        // ---- ORDERS ----
        EventKind::OptionOrderFilled | EventKind::OptionOrderCancelled => {
            let hash = req(event.order_hash.as_deref(), "order.order_hash")?;
            if let Some(row) = state.order_lifecycle.get(hash) {
                upsert_order_lifecycle(tx, deployment_id, hash, row, now_ms).await?;
            }
            // If not present, decoded_events retains the raw evidence.
        }
        EventKind::OptionOrderPairExecuted => {
            // Completion may commit a matched execution row via the correlator
            // and reducer working in tandem. Persist whichever matched execution
            // rows carry this event's execution_id.
            if let Some(exec_id) = event.execution_id.as_deref() {
                if let Some(row) = state.matched_executions.get(exec_id) {
                    upsert_matched_execution(tx, deployment_id, exec_id, row, now_ms).await?;
                }
            }
        }
        EventKind::OptionSubaccountMinValidOrderNonceAdvanced => {
            let subkey = req(event.subkey.as_deref(), "min-nonce.subkey")?;
            let epoch = state
                .subaccount_recovery_epochs
                .get(subkey)
                .cloned()
                .unwrap_or_default();
            let min_nonce = state
                .min_valid_nonce
                .get(subkey)
                .cloned()
                .unwrap_or_else(|| epoch.min_valid_nonce.clone());
            upsert_recovery_epoch(
                tx,
                deployment_id,
                "SUBACCOUNT",
                subkey,
                epoch.epoch_count,
                &min_nonce,
                ctx.block_number,
                now_ms,
            )
            .await?;
        }

        // ---- FEES / REBATES / PREMIUM (append-only) ----
        EventKind::OptionPremiumTransferred
        | EventKind::OptionFeeCharged
        | EventKind::OptionRebatePaid => {
            // Find the exact newly-appended fee_events entry for this event
            // via (tx_hash, log_index). fee_events append-only per event.
            if let Some(row) = state
                .fee_events
                .iter()
                .rev()
                .find(|r| r.tx_hash == ctx.tx_hash && r.log_index == ctx.log_index)
            {
                insert_fee_event(tx, deployment_id, row, now_ms).await?;
            }
        }

        // ---- RECOVERY / ESCAPE ----
        EventKind::RecoveryRequested
        | EventKind::RecoveryActivated
        | EventKind::RecoveryCancelled
        | EventKind::RecoveryFinalized => {
            let subkey = req(event.subkey.as_deref(), "recovery.subkey")?;
            if let Some(row) = state.escape_state.get(subkey) {
                upsert_escape_state(tx, deployment_id, subkey, row, now_ms).await?;
            }
            if let Some(rec) = state.recovery_state.get(subkey).copied() {
                upsert_recovery_state(tx, deployment_id, subkey, rec, ctx.block_number, now_ms)
                    .await?;
            }
            if matches!(event.kind, EventKind::RecoveryFinalized) {
                // Recovery finalization zeros the subaccount's balances +
                // reservations. Mirror that in PG by deleting the projection
                // rows so read paths do not surface stale amounts.
                delete_balances_and_reservations_for_subkey(tx, deployment_id, subkey).await?;
            }
        }
        EventKind::RecoveryEpochIncremented | EventKind::SubaccountRecoveryEpochAdvanced => {
            let subkey = req(event.subkey.as_deref(), "recovery-epoch.subkey")?;
            let epoch = state
                .subaccount_recovery_epochs
                .get(subkey)
                .cloned()
                .unwrap_or_default();
            upsert_recovery_epoch(
                tx,
                deployment_id,
                "SUBACCOUNT",
                subkey,
                epoch.epoch_count,
                &epoch.min_valid_nonce,
                ctx.block_number,
                now_ms,
            )
            .await?;
        }
        EventKind::OwnerRecoveryEpochAdvanced | EventKind::NonceCancelled => {
            let owner = req(event.owner.as_deref(), "owner-epoch.owner")?;
            let epoch = state
                .owner_recovery_epochs
                .get(owner)
                .cloned()
                .unwrap_or_default();
            upsert_recovery_epoch(
                tx,
                deployment_id,
                "OWNER",
                owner,
                epoch.epoch_count,
                &epoch.min_valid_nonce,
                ctx.block_number,
                now_ms,
            )
            .await?;
        }
        EventKind::RecoveryPauseSet => {
            if let Some(row) = state.recovery_pause.as_ref() {
                upsert_recovery_pause(
                    tx,
                    deployment_id,
                    row.paused,
                    row.until_ts,
                    row.last_event_block,
                    now_ms,
                )
                .await?;
            }
        }
        EventKind::RecoveryFinalizationWithdrawn => {
            let subkey = req(event.subkey.as_deref(), "withdraw.subkey")?;
            let count = state
                .recovery_finalization_withdrawals
                .get(subkey)
                .copied()
                .unwrap_or(0);
            upsert_recovery_withdrawal_count(
                tx,
                deployment_id,
                subkey,
                count,
                ctx.block_number,
                now_ms,
            )
            .await?;
        }
    }
    Ok(())
}

// -----------------------------------------------------------------
//                    PER-TABLE UPSERT HELPERS
// -----------------------------------------------------------------

async fn upsert_subaccount(
    tx: &mut Transaction<'_, Postgres>,
    deployment_id: i64,
    subkey: &str,
    owner: &str,
    subaccount_id: u32,
    materialised_via: &str,
    ctx: &ApplyContext,
    now_ms: i64,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO hybrid_v2_subaccounts (
            deployment_id, subkey, owner, subaccount_id, materialised_via,
            created_block, created_tx_hash, created_at_ms
         ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8)
         ON CONFLICT (deployment_id, subkey) DO UPDATE SET
            owner = EXCLUDED.owner,
            subaccount_id = EXCLUDED.subaccount_id,
            materialised_via = CASE
                WHEN hybrid_v2_subaccounts.materialised_via = EXCLUDED.materialised_via
                THEN hybrid_v2_subaccounts.materialised_via
                ELSE 'BOTH' END",
    )
    .bind(deployment_id)
    .bind(subkey)
    .bind(owner)
    .bind(subaccount_id as i64)
    .bind(materialised_via)
    .bind(u64_to_i64("created_block", ctx.block_number)?)
    .bind(&ctx.tx_hash)
    .bind(now_ms)
    .execute(&mut **tx)
    .await
    .map_err(pg_err)?;
    Ok(())
}

async fn upsert_vault_balance(
    tx: &mut Transaction<'_, Postgres>,
    deployment_id: i64,
    subkey: &str,
    token: &str,
    balance: &str,
    block: u64,
    log_id: i64,
    now_ms: i64,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO hybrid_v2_vault_balances (
            deployment_id, subkey, token, balance, last_event_block,
            last_event_log_id, updated_at_ms
         ) VALUES ($1,$2,$3,$4,$5,$6,$7)
         ON CONFLICT (deployment_id, subkey, token) DO UPDATE SET
            balance = EXCLUDED.balance,
            last_event_block = EXCLUDED.last_event_block,
            last_event_log_id = EXCLUDED.last_event_log_id,
            updated_at_ms = EXCLUDED.updated_at_ms",
    )
    .bind(deployment_id)
    .bind(subkey)
    .bind(token)
    .bind(balance)
    .bind(u64_to_i64("last_event_block", block)?)
    .bind(log_id)
    .bind(now_ms)
    .execute(&mut **tx)
    .await
    .map_err(pg_err)?;
    Ok(())
}

async fn upsert_reservation(
    tx: &mut Transaction<'_, Postgres>,
    deployment_id: i64,
    subkey: &str,
    token: &str,
    engine: &str,
    reserved: &str,
    block: u64,
    now_ms: i64,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO hybrid_v2_reservations (
            deployment_id, subkey, token, engine, reserved, last_event_block, updated_at_ms
         ) VALUES ($1,$2,$3,$4,$5,$6,$7)
         ON CONFLICT (deployment_id, subkey, token, engine) DO UPDATE SET
            reserved = EXCLUDED.reserved,
            last_event_block = EXCLUDED.last_event_block,
            updated_at_ms = EXCLUDED.updated_at_ms",
    )
    .bind(deployment_id)
    .bind(subkey)
    .bind(token)
    .bind(engine)
    .bind(reserved)
    .bind(u64_to_i64("last_event_block", block)?)
    .bind(now_ms)
    .execute(&mut **tx)
    .await
    .map_err(pg_err)?;
    Ok(())
}

async fn upsert_collateral_universe(
    tx: &mut Transaction<'_, Postgres>,
    deployment_id: i64,
    token: &str,
    universe_index: u32,
    enabled: bool,
    block: u64,
    now_ms: i64,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO hybrid_v2_collateral_universe (
            deployment_id, token, universe_index, entered_block, entered_at_ms, enabled
         ) VALUES ($1,$2,$3,$4,$5,$6)
         ON CONFLICT (deployment_id, token) DO UPDATE SET
            enabled = EXCLUDED.enabled",
    )
    .bind(deployment_id)
    .bind(token)
    .bind(universe_index as i32)
    .bind(u64_to_i64("entered_block", block)?)
    .bind(now_ms)
    .bind(enabled)
    .execute(&mut **tx)
    .await
    .map_err(pg_err)?;
    Ok(())
}

async fn upsert_capability(
    tx: &mut Transaction<'_, Postgres>,
    deployment_id: i64,
    engine: &str,
    bits: &str,
    now_ms: i64,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO hybrid_v2_capability_grants (
            deployment_id, engine, capability_bits, updated_at_ms
         ) VALUES ($1,$2,$3,$4)
         ON CONFLICT (deployment_id, engine) DO UPDATE SET
            capability_bits = EXCLUDED.capability_bits,
            updated_at_ms = EXCLUDED.updated_at_ms",
    )
    .bind(deployment_id)
    .bind(engine)
    .bind(bits)
    .bind(now_ms)
    .execute(&mut **tx)
    .await
    .map_err(pg_err)?;
    Ok(())
}

async fn upsert_pause_flag(
    tx: &mut Transaction<'_, Postgres>,
    deployment_id: i64,
    subkey: &str,
    paused: bool,
    block: u64,
    now_ms: i64,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO hybrid_v2_pause_flags (
            deployment_id, subkey, paused, last_event_block, updated_at_ms
         ) VALUES ($1,$2,$3,$4,$5)
         ON CONFLICT (deployment_id, subkey) DO UPDATE SET
            paused = EXCLUDED.paused,
            last_event_block = EXCLUDED.last_event_block,
            updated_at_ms = EXCLUDED.updated_at_ms",
    )
    .bind(deployment_id)
    .bind(subkey)
    .bind(paused)
    .bind(u64_to_i64("last_event_block", block)?)
    .bind(now_ms)
    .execute(&mut **tx)
    .await
    .map_err(pg_err)?;
    Ok(())
}

async fn upsert_bad_debt(
    tx: &mut Transaction<'_, Postgres>,
    deployment_id: i64,
    subkey: &str,
    token: &str,
    amount: &str,
    block: u64,
    now_ms: i64,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO hybrid_v2_bad_debt (
            deployment_id, subkey, token, amount, last_event_block, updated_at_ms
         ) VALUES ($1,$2,$3,$4,$5,$6)
         ON CONFLICT (deployment_id, subkey, token) DO UPDATE SET
            amount = EXCLUDED.amount,
            last_event_block = EXCLUDED.last_event_block,
            updated_at_ms = EXCLUDED.updated_at_ms",
    )
    .bind(deployment_id)
    .bind(subkey)
    .bind(token)
    .bind(amount)
    .bind(u64_to_i64("last_event_block", block)?)
    .bind(now_ms)
    .execute(&mut **tx)
    .await
    .map_err(pg_err)?;
    Ok(())
}

async fn upsert_position(
    tx: &mut Transaction<'_, Postgres>,
    deployment_id: i64,
    subkey: &str,
    series: &str,
    long_qty: &str,
    short_qty: &str,
    block: u64,
    now_ms: i64,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO hybrid_v2_positions (
            deployment_id, subkey, series_id, long_qty_1e8, short_qty_1e8,
            last_event_block, updated_at_ms
         ) VALUES ($1,$2,$3,$4,$5,$6,$7)
         ON CONFLICT (deployment_id, subkey, series_id) DO UPDATE SET
            long_qty_1e8 = EXCLUDED.long_qty_1e8,
            short_qty_1e8 = EXCLUDED.short_qty_1e8,
            last_event_block = EXCLUDED.last_event_block,
            updated_at_ms = EXCLUDED.updated_at_ms",
    )
    .bind(deployment_id)
    .bind(subkey)
    .bind(series)
    .bind(long_qty)
    .bind(short_qty)
    .bind(u64_to_i64("last_event_block", block)?)
    .bind(now_ms)
    .execute(&mut **tx)
    .await
    .map_err(pg_err)?;
    Ok(())
}

async fn delete_position(
    tx: &mut Transaction<'_, Postgres>,
    deployment_id: i64,
    subkey: &str,
    series: &str,
) -> Result<()> {
    sqlx::query(
        "DELETE FROM hybrid_v2_positions
         WHERE deployment_id = $1 AND subkey = $2 AND series_id = $3",
    )
    .bind(deployment_id)
    .bind(subkey)
    .bind(series)
    .execute(&mut **tx)
    .await
    .map_err(pg_err)?;
    Ok(())
}

async fn upsert_active_series(
    tx: &mut Transaction<'_, Postgres>,
    deployment_id: i64,
    subkey: &str,
    series: &str,
    block: u64,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO hybrid_v2_active_series (
            deployment_id, subkey, series_id, entered_block
         ) VALUES ($1,$2,$3,$4)
         ON CONFLICT (deployment_id, subkey, series_id) DO NOTHING",
    )
    .bind(deployment_id)
    .bind(subkey)
    .bind(series)
    .bind(u64_to_i64("entered_block", block)?)
    .execute(&mut **tx)
    .await
    .map_err(pg_err)?;
    Ok(())
}

async fn delete_active_series(
    tx: &mut Transaction<'_, Postgres>,
    deployment_id: i64,
    subkey: &str,
    series: &str,
) -> Result<()> {
    sqlx::query(
        "DELETE FROM hybrid_v2_active_series
         WHERE deployment_id = $1 AND subkey = $2 AND series_id = $3",
    )
    .bind(deployment_id)
    .bind(subkey)
    .bind(series)
    .execute(&mut **tx)
    .await
    .map_err(pg_err)?;
    Ok(())
}

async fn upsert_order_lifecycle(
    tx: &mut Transaction<'_, Postgres>,
    deployment_id: i64,
    order_hash: &str,
    row: &crate::hybrid_v2::reducer::OrderLifecycleRow,
    now_ms: i64,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO hybrid_v2_order_lifecycle (
            deployment_id, order_hash, subkey, owner, series_id, side, time_in_force,
            total_qty_1e8, filled_qty_1e8, cancelled, terminal,
            first_seen_block, last_event_block, updated_at_ms
         ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14)
         ON CONFLICT (deployment_id, order_hash) DO UPDATE SET
            subkey = EXCLUDED.subkey,
            owner = EXCLUDED.owner,
            series_id = EXCLUDED.series_id,
            side = EXCLUDED.side,
            time_in_force = EXCLUDED.time_in_force,
            total_qty_1e8 = EXCLUDED.total_qty_1e8,
            filled_qty_1e8 = EXCLUDED.filled_qty_1e8,
            cancelled = EXCLUDED.cancelled,
            terminal = EXCLUDED.terminal,
            first_seen_block = LEAST(hybrid_v2_order_lifecycle.first_seen_block,
                                     EXCLUDED.first_seen_block),
            last_event_block = EXCLUDED.last_event_block,
            updated_at_ms = EXCLUDED.updated_at_ms",
    )
    .bind(deployment_id)
    .bind(order_hash)
    .bind(&row.subkey)
    .bind(&row.owner)
    .bind(row.series_id.as_deref())
    .bind(row.side as i16)
    .bind(row.time_in_force as i16)
    .bind(&row.total_qty_1e8)
    .bind(&row.filled_qty_1e8)
    .bind(row.cancelled)
    .bind(row.terminal)
    .bind(u64_to_i64("first_seen_block", row.first_seen_block.max(1))?)
    .bind(u64_to_i64("last_event_block", row.last_event_block)?)
    .bind(now_ms)
    .execute(&mut **tx)
    .await
    .map_err(pg_err)?;
    Ok(())
}

async fn upsert_matched_execution(
    tx: &mut Transaction<'_, Postgres>,
    deployment_id: i64,
    execution_id: &str,
    row: &crate::hybrid_v2::reducer::MatchedExecutionRow,
    now_ms: i64,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO hybrid_v2_matched_executions (
            deployment_id, execution_id, buyer_order_hash, seller_order_hash,
            buyer_subkey, seller_subkey, series_id, matched_qty_1e8, premium_amount,
            fee_amount, rebate_amount, block_number, tx_hash, completion_status,
            completed_at_ms
         ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15)
         ON CONFLICT (deployment_id, execution_id) DO UPDATE SET
            buyer_order_hash = EXCLUDED.buyer_order_hash,
            seller_order_hash = EXCLUDED.seller_order_hash,
            buyer_subkey = EXCLUDED.buyer_subkey,
            seller_subkey = EXCLUDED.seller_subkey,
            series_id = EXCLUDED.series_id,
            matched_qty_1e8 = EXCLUDED.matched_qty_1e8,
            premium_amount = EXCLUDED.premium_amount,
            fee_amount = EXCLUDED.fee_amount,
            rebate_amount = EXCLUDED.rebate_amount,
            block_number = EXCLUDED.block_number,
            tx_hash = EXCLUDED.tx_hash,
            completion_status = EXCLUDED.completion_status,
            completed_at_ms = EXCLUDED.completed_at_ms",
    )
    .bind(deployment_id)
    .bind(execution_id)
    .bind(&row.buyer_order_hash)
    .bind(&row.seller_order_hash)
    .bind(&row.buyer_subkey)
    .bind(&row.seller_subkey)
    .bind(&row.series_id)
    .bind(&row.matched_qty_1e8)
    .bind(&row.premium_amount)
    .bind(&row.fee_amount)
    .bind(&row.rebate_amount)
    .bind(u64_to_i64("block_number", row.block_number)?)
    .bind(&row.tx_hash)
    .bind(exec_completion_str(row.completion_status))
    .bind(now_ms)
    .execute(&mut **tx)
    .await
    .map_err(pg_err)?;
    Ok(())
}

fn exec_completion_str(c: ExecutionCompletion) -> &'static str {
    match c {
        ExecutionCompletion::Complete => "COMPLETE",
        ExecutionCompletion::Incomplete => "INCOMPLETE",
        ExecutionCompletion::InvalidatedByReorg => "INVALIDATED_BY_REORG",
    }
}

async fn insert_fee_event(
    tx: &mut Transaction<'_, Postgres>,
    deployment_id: i64,
    row: &FeeEventRow,
    now_ms: i64,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO hybrid_v2_fee_events (
            deployment_id, kind, payer_subkey, receiver_subkey, token, amount,
            block_number, tx_hash, log_index, execution_id, recorded_at_ms
         ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)",
    )
    .bind(deployment_id)
    .bind(row.kind)
    .bind(row.payer_subkey.as_deref())
    .bind(row.receiver_subkey.as_deref())
    .bind(&row.token)
    .bind(&row.amount)
    .bind(u64_to_i64("block_number", row.block_number)?)
    .bind(&row.tx_hash)
    .bind(row.log_index as i32)
    .bind(row.execution_id.as_deref())
    .bind(now_ms)
    .execute(&mut **tx)
    .await
    .map_err(pg_err)?;
    Ok(())
}

async fn upsert_escape_state(
    tx: &mut Transaction<'_, Postgres>,
    deployment_id: i64,
    subkey: &str,
    row: &crate::hybrid_v2::reducer::EscapeStateRow,
    now_ms: i64,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO hybrid_v2_escape_state (
            deployment_id, subkey, state, requested_ts, activation_eligible_at,
            activated_ts, cancelled_ts, finalized_ts, last_event_block, updated_at_ms
         ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)
         ON CONFLICT (deployment_id, subkey) DO UPDATE SET
            state = EXCLUDED.state,
            requested_ts = EXCLUDED.requested_ts,
            activation_eligible_at = EXCLUDED.activation_eligible_at,
            activated_ts = EXCLUDED.activated_ts,
            cancelled_ts = EXCLUDED.cancelled_ts,
            finalized_ts = EXCLUDED.finalized_ts,
            last_event_block = EXCLUDED.last_event_block,
            updated_at_ms = EXCLUDED.updated_at_ms",
    )
    .bind(deployment_id)
    .bind(subkey)
    .bind(row.state)
    .bind(row.requested_ts.map(|v| v as i64))
    .bind(row.activation_eligible_at.map(|v| v as i64))
    .bind(row.activated_ts.map(|v| v as i64))
    .bind(row.cancelled_ts.map(|v| v as i64))
    .bind(row.finalized_ts.map(|v| v as i64))
    .bind(u64_to_i64("last_event_block", row.last_event_block)?)
    .bind(now_ms)
    .execute(&mut **tx)
    .await
    .map_err(pg_err)?;
    Ok(())
}

async fn upsert_recovery_state(
    tx: &mut Transaction<'_, Postgres>,
    deployment_id: i64,
    subkey: &str,
    state: RecoveryStateProjection,
    block: u64,
    now_ms: i64,
) -> Result<()> {
    let pending_ts: Option<i64> = None;
    let activation_ts: Option<i64> = None;
    let finalized_ts: Option<i64> = if matches!(state, RecoveryStateProjection::Recovered) {
        Some(now_ms / 1000)
    } else {
        None
    };
    sqlx::query(
        "INSERT INTO hybrid_v2_recovery_state (
            deployment_id, subkey, state, pending_since_ts, activation_eligible_at,
            finalized_at_ts, tokens_withdrawn, state_updated_block, updated_at_ms
         ) VALUES ($1,$2,$3,$4,$5,$6,0,$7,$8)
         ON CONFLICT (deployment_id, subkey) DO UPDATE SET
            state = EXCLUDED.state,
            pending_since_ts = COALESCE(EXCLUDED.pending_since_ts,
                                        hybrid_v2_recovery_state.pending_since_ts),
            activation_eligible_at = COALESCE(EXCLUDED.activation_eligible_at,
                                              hybrid_v2_recovery_state.activation_eligible_at),
            finalized_at_ts = COALESCE(EXCLUDED.finalized_at_ts,
                                       hybrid_v2_recovery_state.finalized_at_ts),
            state_updated_block = EXCLUDED.state_updated_block,
            updated_at_ms = EXCLUDED.updated_at_ms",
    )
    .bind(deployment_id)
    .bind(subkey)
    .bind(state.as_str())
    .bind(pending_ts)
    .bind(activation_ts)
    .bind(finalized_ts)
    .bind(u64_to_i64("state_updated_block", block)?)
    .bind(now_ms)
    .execute(&mut **tx)
    .await
    .map_err(pg_err)?;
    Ok(())
}

async fn upsert_recovery_epoch(
    tx: &mut Transaction<'_, Postgres>,
    deployment_id: i64,
    scope: &str,
    scope_key: &str,
    epoch_count: u64,
    min_valid_nonce: &str,
    block: u64,
    now_ms: i64,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO hybrid_v2_recovery_epochs (
            deployment_id, scope, scope_key, epoch_count, min_valid_nonce,
            last_event_block, updated_at_ms
         ) VALUES ($1,$2,$3,$4,$5,$6,$7)
         ON CONFLICT (deployment_id, scope, scope_key) DO UPDATE SET
            epoch_count = GREATEST(hybrid_v2_recovery_epochs.epoch_count,
                                   EXCLUDED.epoch_count),
            min_valid_nonce = EXCLUDED.min_valid_nonce,
            last_event_block = EXCLUDED.last_event_block,
            updated_at_ms = EXCLUDED.updated_at_ms",
    )
    .bind(deployment_id)
    .bind(scope)
    .bind(scope_key)
    .bind(u64_to_i64("epoch_count", epoch_count)?)
    .bind(min_valid_nonce)
    .bind(u64_to_i64("last_event_block", block)?)
    .bind(now_ms)
    .execute(&mut **tx)
    .await
    .map_err(pg_err)?;
    Ok(())
}

async fn upsert_recovery_pause(
    tx: &mut Transaction<'_, Postgres>,
    deployment_id: i64,
    paused: bool,
    until_ts: u64,
    block: u64,
    now_ms: i64,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO hybrid_v2_recovery_pause (
            deployment_id, paused, until_ts, last_event_block, updated_at_ms
         ) VALUES ($1,$2,$3,$4,$5)
         ON CONFLICT (deployment_id) DO UPDATE SET
            paused = EXCLUDED.paused,
            until_ts = EXCLUDED.until_ts,
            last_event_block = EXCLUDED.last_event_block,
            updated_at_ms = EXCLUDED.updated_at_ms",
    )
    .bind(deployment_id)
    .bind(paused)
    .bind(u64_to_i64("until_ts", until_ts)?)
    .bind(u64_to_i64("last_event_block", block)?)
    .bind(now_ms)
    .execute(&mut **tx)
    .await
    .map_err(pg_err)?;
    Ok(())
}

async fn upsert_recovery_withdrawal_count(
    tx: &mut Transaction<'_, Postgres>,
    deployment_id: i64,
    subkey: &str,
    count: u32,
    block: u64,
    now_ms: i64,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO hybrid_v2_recovery_withdrawal_counts (
            deployment_id, subkey, withdrawal_count, last_event_block, updated_at_ms
         ) VALUES ($1,$2,$3,$4,$5)
         ON CONFLICT (deployment_id, subkey) DO UPDATE SET
            withdrawal_count = GREATEST(
                hybrid_v2_recovery_withdrawal_counts.withdrawal_count,
                EXCLUDED.withdrawal_count),
            last_event_block = EXCLUDED.last_event_block,
            updated_at_ms = EXCLUDED.updated_at_ms",
    )
    .bind(deployment_id)
    .bind(subkey)
    .bind(count as i32)
    .bind(u64_to_i64("last_event_block", block)?)
    .bind(now_ms)
    .execute(&mut **tx)
    .await
    .map_err(pg_err)?;
    Ok(())
}

async fn delete_balances_and_reservations_for_subkey(
    tx: &mut Transaction<'_, Postgres>,
    deployment_id: i64,
    subkey: &str,
) -> Result<()> {
    sqlx::query(
        "DELETE FROM hybrid_v2_vault_balances
         WHERE deployment_id = $1 AND subkey = $2",
    )
    .bind(deployment_id)
    .bind(subkey)
    .execute(&mut **tx)
    .await
    .map_err(pg_err)?;
    sqlx::query(
        "DELETE FROM hybrid_v2_reservations
         WHERE deployment_id = $1 AND subkey = $2",
    )
    .bind(deployment_id)
    .bind(subkey)
    .execute(&mut **tx)
    .await
    .map_err(pg_err)?;
    Ok(())
}

// -----------------------------------------------------------------
//                       IN-MEMORY FAKE STORE
// -----------------------------------------------------------------
//
// Enables unit-test coverage of the block-atomic contract without a
// live PostgreSQL instance. Every write is applied to an internal
// snapshot only when the block "commits" — a failure inside a block
// aborts and leaves prior committed state intact. This mirrors the PG
// transaction contract at the API level, giving convergence tests a
// fixture they can exercise anywhere.

#[derive(Debug, Default, Clone)]
pub struct InMemoryProjectionStore {
    inner: std::sync::Arc<std::sync::Mutex<InMemoryInner>>,
}

#[derive(Debug, Default)]
struct InMemoryInner {
    deployments: std::collections::BTreeMap<(u64, String), i64>,
    next_deployment_id: i64,
    blocks: std::collections::BTreeMap<(i64, String), CanonicalBlockRef>,
    raw_logs: std::collections::BTreeMap<(i64, String, String, i32), (i64, JournaledLog)>,
    decoded_events: std::collections::BTreeMap<i64, HybridV2Event>,
    next_raw_id: i64,
    state_snapshots: std::collections::BTreeMap<i64, ProjectionState>,
    cursors: std::collections::BTreeMap<(i64, String), RuntimeCursorSnapshot>,
    readiness: std::collections::BTreeMap<i64, ReadinessSnapshot>,
    reorg_events: Vec<serde_json::Value>,
    reorg_recovery:
        std::collections::BTreeMap<i64, crate::hybrid_v2::reorg_recovery::ReorgRecoveryState>,
    reorg_locks: std::collections::BTreeMap<i64, (i64, i64)>,
    // Unified operation-lock table: deployment → (operation, epoch, acquired_at_ms).
    // Reorg + rebuild + reconciliation contend for the single row per deployment.
    operation_locks: std::collections::BTreeMap<
        i64,
        (
            crate::hybrid_v2::rebuild_operations::OperationKind,
            i64,
            i64,
        ),
    >,
    // Persisted rebuild state machine: (deployment_id, rebuild_epoch) → row.
    rebuild_operations: std::collections::BTreeMap<
        (i64, i64),
        crate::hybrid_v2::rebuild_operations::RebuildOperationState,
    >,
    // Append-only reconciliation results (in-memory).
    reconciliation_results:
        std::collections::BTreeMap<i64, crate::hybrid_v2::reconciler::ReconciliationRecord>,
    next_reconciliation_id: i64,
}

impl InMemoryProjectionStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn snapshot_state(&self, deployment_id: i64) -> Option<ProjectionState> {
        let inner = self.inner.lock().unwrap();
        inner.state_snapshots.get(&deployment_id).cloned()
    }

    pub fn raw_log_count(&self, deployment_id: i64) -> usize {
        let inner = self.inner.lock().unwrap();
        inner
            .raw_logs
            .keys()
            .filter(|(d, _, _, _)| *d == deployment_id)
            .count()
    }

    pub fn block_count(&self, deployment_id: i64) -> usize {
        let inner = self.inner.lock().unwrap();
        inner
            .blocks
            .keys()
            .filter(|(d, _)| *d == deployment_id)
            .count()
    }

    pub fn reorg_events(&self) -> Vec<serde_json::Value> {
        self.inner.lock().unwrap().reorg_events.clone()
    }
}

#[async_trait::async_trait]
impl HybridV2ProjectionStore for InMemoryProjectionStore {
    async fn upsert_deployment(
        &self,
        manifest: &ManifestParams,
        _activation_status: &str,
        _now_ms: i64,
    ) -> Result<i64> {
        let mut inner = self.inner.lock().unwrap();
        let key = (manifest.chain_id, manifest.manifest_hash.clone());
        if let Some(id) = inner.deployments.get(&key).copied() {
            return Ok(id);
        }
        inner.next_deployment_id += 1;
        let id = inner.next_deployment_id;
        inner.deployments.insert(key, id);
        Ok(id)
    }

    async fn persist_block_atomic(
        &self,
        deployment_id: i64,
        block: &CanonicalBlockRef,
        raw_logs: &[JournaledLog],
        decoded_events: &[(HybridV2Event, ApplyContext)],
        state: &ProjectionState,
        cursor: &RuntimeCursorSnapshot,
        readiness: &ReadinessSnapshot,
        _now_ms: i64,
    ) -> Result<()> {
        if raw_logs.len() != decoded_events.len() {
            return Err(BackendError::Persistence(
                "in-memory: raw log count must equal decoded event count for block".to_string(),
            ));
        }
        let mut inner = self.inner.lock().unwrap();
        inner
            .blocks
            .insert((deployment_id, block.block_hash.clone()), block.clone());
        for log in raw_logs {
            inner.next_raw_id += 1;
            let raw_id = inner.next_raw_id;
            inner.raw_logs.insert(
                (
                    deployment_id,
                    log.block_hash.clone(),
                    log.tx_hash.clone(),
                    log.log_index as i32,
                ),
                (raw_id, log.clone()),
            );
            inner.decoded_events.insert(
                raw_id,
                HybridV2Event {
                    kind: EventKind::Deposit, // placeholder — decoded events tracked below
                    event_version: 1,
                    subkey: None,
                    owner: None,
                    subaccount_id: None,
                    token: None,
                    engine: None,
                    execution_id: None,
                    order_hash: None,
                    series_id: None,
                    payload: json!({}),
                },
            );
        }
        for (event, _ctx) in decoded_events {
            // Overwrite the placeholder with the real event using the same raw_log_id.
            // We use insertion order equivalence — matches persist_block_atomic contract.
            let last_id = inner.next_raw_id
                - (raw_logs.len() as i64
                    - decoded_events
                        .iter()
                        .position(|(e, _)| std::ptr::eq(e, event))
                        .unwrap() as i64
                    - 1);
            inner.decoded_events.insert(last_id, event.clone());
        }
        inner.state_snapshots.insert(deployment_id, state.clone());
        inner
            .cursors
            .insert((deployment_id, cursor.cursor_name.clone()), cursor.clone());
        inner.readiness.insert(deployment_id, readiness.clone());
        Ok(())
    }

    async fn upsert_runtime_metrics(
        &self,
        _deployment_id: i64,
        _cursor: &RuntimeCursorSnapshot,
        _now_ms: i64,
    ) -> Result<()> {
        Ok(())
    }

    async fn record_reorg_event(
        &self,
        deployment_id: i64,
        detected_at_ms: i64,
        from_block: u64,
        from_hash: &str,
        to_block: u64,
        to_hash: &str,
        depth: u32,
        orphaned_log_count: u32,
    ) -> Result<()> {
        let mut inner = self.inner.lock().unwrap();
        inner.reorg_events.push(json!({
            "deployment_id": deployment_id,
            "detected_at_ms": detected_at_ms,
            "from_block": from_block,
            "from_hash": from_hash,
            "to_block": to_block,
            "to_hash": to_hash,
            "depth": depth,
            "orphaned_log_count": orphaned_log_count,
        }));
        Ok(())
    }

    async fn read_cursor(
        &self,
        deployment_id: i64,
        cursor_name: &str,
    ) -> Result<Option<RuntimeCursorSnapshot>> {
        let inner = self.inner.lock().unwrap();
        Ok(inner
            .cursors
            .get(&(deployment_id, cursor_name.to_string()))
            .cloned())
    }

    async fn read_readiness(&self, deployment_id: i64) -> Result<Option<ReadinessSnapshot>> {
        let inner = self.inner.lock().unwrap();
        Ok(inner.readiness.get(&deployment_id).cloned())
    }

    async fn read_canonical_journal(&self, deployment_id: i64) -> Result<Vec<JournaledLog>> {
        let inner = self.inner.lock().unwrap();
        let mut out: Vec<JournaledLog> = inner
            .raw_logs
            .iter()
            .filter_map(|((d, _bh, _tx, _idx), (_id, log))| {
                if *d == deployment_id && log.is_canonical {
                    Some(log.clone())
                } else {
                    None
                }
            })
            .collect();
        out.sort_by(|a, b| (a.block_number, a.log_index).cmp(&(b.block_number, b.log_index)));
        Ok(out)
    }

    async fn read_reorg_recovery(
        &self,
        deployment_id: i64,
    ) -> Result<Option<crate::hybrid_v2::reorg_recovery::ReorgRecoveryState>> {
        let inner = self.inner.lock().unwrap();
        Ok(inner
            .reorg_recovery
            .get(&deployment_id)
            .cloned()
            .map(|s| crate::hybrid_v2::reorg_recovery::ReorgRecoveryState { ..s }))
    }

    async fn upsert_reorg_recovery(
        &self,
        state: &crate::hybrid_v2::reorg_recovery::ReorgRecoveryState,
    ) -> Result<()> {
        let mut inner = self.inner.lock().unwrap();
        inner
            .reorg_recovery
            .insert(state.deployment_id, state.clone());
        Ok(())
    }

    async fn write_readiness_snapshot(
        &self,
        deployment_id: i64,
        readiness: &ReadinessSnapshot,
        _now_ms: i64,
    ) -> Result<()> {
        let mut inner = self.inner.lock().unwrap();
        inner.readiness.insert(deployment_id, readiness.clone());
        Ok(())
    }

    async fn commit_reorg_recovery(
        &self,
        deployment_id: i64,
        recovery_epoch: i64,
        ancestor_block: u64,
        ancestor_hash: &str,
        replacement_blocks: &[CanonicalBlockRef],
        replacement_raw_logs: &[JournaledLog],
        replacement_decoded_events: &[(HybridV2Event, ApplyContext)],
        projection_state: &ProjectionState,
        cursor: &RuntimeCursorSnapshot,
        readiness: &ReadinessSnapshot,
        _now_ms: i64,
    ) -> Result<()> {
        let mut inner = self.inner.lock().unwrap();
        // 1. Mark orphan raw logs non-canonical.
        for ((d, _bh, _tx, _idx), (_id, log)) in inner.raw_logs.iter_mut() {
            if *d == deployment_id && log.is_canonical && log.block_number > ancestor_block {
                log.is_canonical = false;
                log.orphaned_at_block = Some(ancestor_block);
            }
        }
        // 2. Mark orphan blocks by deletion (in-memory has no separate is_canonical column
        //    beyond the map itself; test asserts rely on block_count/head).
        //    We keep the block ref but preserve the ancestor block; a "swap" is implicit
        //    by writing the new blocks below and leaving the old under a different hash.
        // 3. Mark orphan matched executions.
        if let Some(state) = inner.state_snapshots.get_mut(&deployment_id) {
            for (_id, row) in state.matched_executions.iter_mut() {
                if row.block_number > ancestor_block
                    && !matches!(
                        row.completion_status,
                        crate::hybrid_v2::reducer::ExecutionCompletion::InvalidatedByReorg
                    )
                {
                    row.completion_status =
                        crate::hybrid_v2::reducer::ExecutionCompletion::InvalidatedByReorg;
                }
            }
        }
        // 4. Write replacement blocks + raw logs.
        for block in replacement_blocks {
            inner
                .blocks
                .insert((deployment_id, block.block_hash.clone()), block.clone());
        }
        for (i, log) in replacement_raw_logs.iter().enumerate() {
            inner.next_raw_id += 1;
            let raw_id = inner.next_raw_id;
            inner.raw_logs.insert(
                (
                    deployment_id,
                    log.block_hash.clone(),
                    log.tx_hash.clone(),
                    log.log_index as i32,
                ),
                (raw_id, log.clone()),
            );
            if let Some((ev, _)) = replacement_decoded_events.get(i) {
                inner.decoded_events.insert(raw_id, ev.clone());
            }
        }
        // 5. Publish the new projection state.
        inner
            .state_snapshots
            .insert(deployment_id, projection_state.clone());
        inner
            .cursors
            .insert((deployment_id, cursor.cursor_name.clone()), cursor.clone());
        inner.readiness.insert(deployment_id, readiness.clone());
        // 6. Mark the recovery row as RECOVERED atomically.
        if let Some(row) = inner.reorg_recovery.get_mut(&deployment_id) {
            if row.recovery_epoch == recovery_epoch {
                row.phase = crate::hybrid_v2::reorg_recovery::ReorgRecoveryPhase::Recovered;
                row.completed_at_ms = Some(row.updated_at_ms);
            }
        }
        let _ = ancestor_hash;
        Ok(())
    }

    // NOTE: The InMemory impls of `try_acquire_reorg_lock` /
    // `release_reorg_lock` were removed under
    // `BACKEND-HYBRID-V2-PROJECTION-PERSISTENCE-OPERATIONAL-CLOSURE-V1`.
    // Reorg now goes through `try_acquire_operation_lock(OperationKind::Reorg)`
    // below.

    // -----------------------------------------------------------------
    //  CLOSURE V1 — unified operation lock + rebuild ops + reconciliation
    // -----------------------------------------------------------------

    async fn try_acquire_operation_lock(
        &self,
        deployment_id: i64,
        operation: crate::hybrid_v2::rebuild_operations::OperationKind,
        holder_epoch: i64,
        now_ms: i64,
    ) -> Result<Option<crate::hybrid_v2::rebuild_operations::OperationLockGuard>> {
        let mut inner = self.inner.lock().unwrap();
        // Stale-lock cleanup: only delete when we can prove the prior
        // holder is terminal. Absence of any paired state row is NOT
        // treated as terminal — that condition holds during the natural
        // window between lock acquisition and the first upsert of the
        // paired state row. Matching the PG semantics.
        let should_delete = match inner.operation_locks.get(&deployment_id) {
            Some((prev_op, _prev_epoch, _)) => match prev_op {
                crate::hybrid_v2::rebuild_operations::OperationKind::Reorg => {
                    matches!(
                        inner.reorg_recovery.get(&deployment_id).map(|r| r.phase),
                        Some(crate::hybrid_v2::reorg_recovery::ReorgRecoveryPhase::Recovered)
                            | Some(
                                crate::hybrid_v2::reorg_recovery::ReorgRecoveryPhase::ManualInterventionRequired
                            )
                    )
                }
                crate::hybrid_v2::rebuild_operations::OperationKind::Rebuild => inner
                    .rebuild_operations
                    .iter()
                    .filter(|((d, _), _)| *d == deployment_id)
                    .map(|(_, v)| v.phase)
                    .filter(|p| p.is_terminal())
                    .last()
                    .is_some(),
                crate::hybrid_v2::rebuild_operations::OperationKind::Reconciliation => true,
            },
            None => false,
        };
        if should_delete {
            inner.operation_locks.remove(&deployment_id);
        }
        if inner.operation_locks.contains_key(&deployment_id) {
            return Ok(None);
        }
        inner
            .operation_locks
            .insert(deployment_id, (operation, holder_epoch, now_ms));
        Ok(Some(
            crate::hybrid_v2::rebuild_operations::OperationLockGuard {
                deployment_id,
                operation,
                holder_epoch,
                store: None,
            },
        ))
    }

    async fn release_operation_lock(&self, deployment_id: i64, holder_epoch: i64) -> Result<()> {
        let mut inner = self.inner.lock().unwrap();
        if let Some((_, epoch, _)) = inner.operation_locks.get(&deployment_id).cloned() {
            if epoch == holder_epoch {
                inner.operation_locks.remove(&deployment_id);
            }
        }
        Ok(())
    }

    async fn upsert_rebuild_operation(
        &self,
        state: &crate::hybrid_v2::rebuild_operations::RebuildOperationState,
    ) -> Result<()> {
        let mut inner = self.inner.lock().unwrap();
        inner
            .rebuild_operations
            .insert((state.deployment_id, state.rebuild_epoch), state.clone());
        Ok(())
    }

    async fn read_rebuild_operation(
        &self,
        deployment_id: i64,
        rebuild_epoch: i64,
    ) -> Result<Option<crate::hybrid_v2::rebuild_operations::RebuildOperationState>> {
        let inner = self.inner.lock().unwrap();
        Ok(inner
            .rebuild_operations
            .get(&(deployment_id, rebuild_epoch))
            .cloned())
    }

    async fn read_latest_rebuild_operation(
        &self,
        deployment_id: i64,
    ) -> Result<Option<crate::hybrid_v2::rebuild_operations::RebuildOperationState>> {
        let inner = self.inner.lock().unwrap();
        Ok(inner
            .rebuild_operations
            .iter()
            .filter(|((d, _), _)| *d == deployment_id)
            .map(|(_, v)| v.clone())
            .max_by_key(|v| v.rebuild_epoch))
    }

    async fn insert_reconciliation_result(
        &self,
        record: &crate::hybrid_v2::reconciler::ReconciliationRecord,
    ) -> Result<i64> {
        let mut inner = self.inner.lock().unwrap();
        inner.next_reconciliation_id += 1;
        let id = inner.next_reconciliation_id;
        let mut with_id = record.clone();
        with_id.reconciliation_id = Some(id);
        inner.reconciliation_results.insert(id, with_id);
        Ok(id)
    }

    async fn read_latest_reconciliation_result(
        &self,
        deployment_id: i64,
    ) -> Result<Option<crate::hybrid_v2::reconciler::ReconciliationRecord>> {
        let inner = self.inner.lock().unwrap();
        Ok(inner
            .reconciliation_results
            .values()
            .filter(|r| r.deployment_id == deployment_id)
            .max_by_key(|r| r.ran_at_ms)
            .cloned())
    }

    async fn snapshot_projection_state(
        &self,
        deployment_id: i64,
    ) -> Result<Option<crate::hybrid_v2::reducer::ProjectionState>> {
        let inner = self.inner.lock().unwrap();
        Ok(inner.state_snapshots.get(&deployment_id).cloned())
    }

    async fn commit_rematerialization(
        &self,
        deployment_id: i64,
        rebuild_epoch: i64,
        state: &crate::hybrid_v2::reducer::ProjectionState,
        cursor: &RuntimeCursorSnapshot,
        readiness: &ReadinessSnapshot,
        now_ms: i64,
    ) -> Result<()> {
        let mut inner = self.inner.lock().unwrap();
        // Mirror the PG semantics: wipe + republish the projection
        // snapshot, then update cursor + readiness + rebuild-op row.
        inner.state_snapshots.insert(deployment_id, state.clone());
        inner
            .cursors
            .insert((deployment_id, cursor.cursor_name.clone()), cursor.clone());
        inner.readiness.insert(deployment_id, readiness.clone());
        if let Some(row) = inner
            .rebuild_operations
            .get_mut(&(deployment_id, rebuild_epoch))
        {
            row.phase = crate::hybrid_v2::rebuild_operations::RebuildPhase::Complete;
            row.verification_result = Some(
                row.verification_result
                    .clone()
                    .unwrap_or_else(|| "REMATERIALIZED".to_string()),
            );
            row.completed_at_ms = Some(now_ms);
            row.updated_at_ms = now_ms;
        }
        Ok(())
    }
}

// -----------------------------------------------------------------
//                       HELPERS
// -----------------------------------------------------------------

fn pg_err<E: std::fmt::Display>(e: E) -> BackendError {
    BackendError::Persistence(e.to_string())
}

fn u64_to_i64(field: &'static str, value: u64) -> Result<i64> {
    i64::try_from(value)
        .map_err(|_| BackendError::Persistence(format!("field {field} exceeds i64::MAX: {value}")))
}

fn i64_to_u64(field: &'static str, value: i64) -> Result<u64> {
    u64::try_from(value)
        .map_err(|_| BackendError::Persistence(format!("field {field} negative in PG: {value}")))
}

fn req<'a, T: ?Sized>(v: Option<&'a T>, field: &'static str) -> Result<&'a T> {
    v.ok_or_else(|| miss(field))
}

fn miss(field: &'static str) -> BackendError {
    BackendError::Persistence(format!("event missing required field {field}"))
}

/// Lowercase-hex encode a byte slice (no `0x` prefix). Local to the
/// persistence layer so the module has no external hex-crate dependency.
fn encode_hex_bytes(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(char::from_digit(u32::from(b >> 4), 16).unwrap());
        out.push(char::from_digit(u32::from(b & 0x0f), 16).unwrap());
    }
    out
}

// Public helper for callers who need the currently-touched subkeys
// derived from an event set (used by property tests + observability).
pub fn subkeys_touched_by(events: &[(HybridV2Event, ApplyContext)]) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for (event, _) in events {
        if let Some(s) = event.subkey.as_deref() {
            out.insert(s.to_string());
        }
    }
    out
}

// -----------------------------------------------------------------
//                       UNIT TESTS
// -----------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hybrid_v2::reducer::apply;

    fn ctx(block: u64, log_index: u32) -> ApplyContext {
        ApplyContext {
            block_number: block,
            tx_hash: format!("0xtx{block}"),
            log_index,
            block_timestamp: 1_700_000_000 + block,
        }
    }

    fn deposit_event(subkey: &str, token: &str, amount: &str) -> HybridV2Event {
        HybridV2Event {
            kind: EventKind::Deposit,
            event_version: 1,
            subkey: Some(subkey.to_string()),
            owner: None,
            subaccount_id: None,
            token: Some(token.to_string()),
            engine: None,
            execution_id: None,
            order_hash: None,
            series_id: None,
            payload: json!({ "amount": amount }),
        }
    }

    #[tokio::test]
    async fn subkey_extractor_dedups() {
        let e1 = deposit_event("0xsub", "0xtoken", "100");
        let e2 = deposit_event("0xsub", "0xtoken", "200");
        let touched = subkeys_touched_by(&[(e1, ctx(1, 0)), (e2, ctx(1, 1))]);
        assert_eq!(touched.len(), 1);
        assert!(touched.contains("0xsub"));
    }

    #[tokio::test]
    async fn reducer_and_snapshot_agree_on_balance() {
        let mut state = ProjectionState::default();
        let e = deposit_event("0xa1", "0xtoken", "500");
        let c = ctx(1, 0);
        apply(&mut state, &e, &c).unwrap();
        assert_eq!(
            state
                .balances
                .get(&("0xa1".to_string(), "0xtoken".to_string())),
            Some(&"500".to_string())
        );
    }

    #[tokio::test]
    async fn in_memory_store_records_deployment_once() {
        use crate::hybrid_v2::manifest::{ManifestModuleAddresses, ManifestParams};
        let manifest = ManifestParams {
            chain_id: 84532,
            manifest_address: "0xm".into(),
            manifest_hash: "0xh1".into(),
            module_addresses_hash: "0x".into(),
            critical_config_hash: "0x".into(),
            architecture_version: 1,
            storage_version: 1,
            event_version: 1,
            deployment_version: 1,
            manifest_schema_version: 1,
            environment_tag: "TESTNET".into(),
            deployer: "0xdep".into(),
            deployment_block: 1,
            deployment_timestamp: 1_700_000_000,
            module_addresses: ManifestModuleAddresses {
                subaccount_registry: "0xreg".into(),
                collateral_vault: "0xvlt".into(),
                options_positions_ledger: "0xopl".into(),
                risk_module: "0xrsk".into(),
                margin_engine: "0xmg".into(),
                option_matching_engine: "0xmm".into(),
                escape_controller: "0xesc".into(),
                recovery_finalizer: "0xrec".into(),
                oracle_adapter: "0xoc".into(),
                options_risk_provider: "0xorp".into(),
                quote_token: "0xqt".into(),
                fees_manager_v2: None,
                option_execution_fee_adapter: None,
                protocol_timelock: None,
                governance: None,
                guardian: None,
            },
            protocol_fee_subkey: "0xpf".into(),
            rebate_budget_subkey: "0xrb".into(),
            insurance_fund_subkey: "0xif".into(),
            max_collateral_tokens: 8,
            max_active_series: 32,
            all_capabilities_mask: "0".into(),
            recovery_activation_delay_seconds: 3600,
            recovery_pause_max_duration_blocks: 100,
            activation_status: crate::hybrid_v2::manifest::ActivationStatus::Pending,
        };
        let store = InMemoryProjectionStore::new();
        let id1 = store
            .upsert_deployment(&manifest, "PENDING", 1)
            .await
            .unwrap();
        let id2 = store
            .upsert_deployment(&manifest, "PENDING", 2)
            .await
            .unwrap();
        assert_eq!(id1, id2);
    }
}
