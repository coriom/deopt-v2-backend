-- 0048_hybrid_v2_operation_closure.sql
--
-- BACKEND-HYBRID-V2-PROJECTION-PERSISTENCE-CLOSURE-V1 — additive schema
-- for the unified deployment-scoped operation lock, persisted rebuild
-- state machine, and reconciliation-result history.
--
-- Additive-only. Does NOT alter or drop any existing table or column.
-- The prior `hybrid_v2_reorg_locks` table introduced in 0047 is left
-- untouched (harmless orphan); reorg + rebuild + reconciliation now all
-- contend for a single row in `hybrid_v2_operation_locks`.
--
-- Frozen safety rules newly enforced by these tables:
--   * NO_PARTIAL_REBUILD_STATE_IS_PUBLICLY_READY — the rebuild state
--     row's `phase` gates readiness; nothing publishes until phase=COMPLETE.
--   * RECONCILIATION_DRIFT_NEVER_AUTO_REPAIRS_PROJECTIONS — the results
--     table is append-only; drift never triggers a rebuild.
--   * READY_REQUIRES_PERSISTENCE_REORG_REBUILD_AND_RECONCILIATION_CONVERGENCE
--     — readiness assembly reads all three phase surfaces.
--   * OPERATOR_RECOVERY_ACTIONS_ARE_DEPLOYMENT_SCOPED_AND_NON_PUBLIC
--     — every table below is keyed on deployment_id; there is no
--       cross-deployment row.
--
-- Publication model (documented decision):
--   BACKEND_HYBRID_V2_REBUILD_SINGLE_TRANSACTION_MODEL — the rebuild
--   pipeline replays the canonical journal in-memory into a fresh
--   ProjectionState. If the reconstructed state matches the live
--   projection, phase transitions Complete/NothingToDo without any
--   mutation. If it diverges, this milestone marks the rebuild
--   Failed/ManualInterventionRequired — the operator is required to act.
--   No shadow-generation columns are added to the projection tables at
--   this stage: the journal-sized (< 1M events per Base Sepolia
--   experimental deployment) rewrite path is deferred and does not
--   need per-row generation ids in the current single-writer model.

BEGIN;

-------------------------------------------------------------------------------
-- Unified deployment-scoped operation lock
-------------------------------------------------------------------------------
--
-- One row per deployment. Reorg recovery, rebuild, and reconciliation
-- all contend for this single row via
--   INSERT ... ON CONFLICT (deployment_id) DO NOTHING
-- so at most one recovery-class operation runs per deployment at any
-- time. `operation` records which subsystem holds the lock (audit +
-- observability); `holder_epoch` is the fencing token used to guard
-- late releases from stale holders.
--
-- Stale-lock cleanup: callers gate deletion on evidence that the
-- previous holder is terminal (RECOVERED / COMPLETE / drift persisted).
-- Absence of any operation row does NOT count as terminal — the
-- natural gap between INSERT into this table and the first operation
-- state row is expected.

CREATE TABLE IF NOT EXISTS hybrid_v2_operation_locks (
    deployment_id  BIGINT      PRIMARY KEY,
    operation      TEXT        NOT NULL,   -- 'REORG' | 'REBUILD' | 'RECONCILIATION'
    holder_epoch   BIGINT      NOT NULL,
    acquired_at_ms BIGINT      NOT NULL,
    CONSTRAINT hybrid_v2_operation_locks_deployment_fk
        FOREIGN KEY (deployment_id)
        REFERENCES hybrid_v2_deployments(deployment_id)
        ON DELETE CASCADE,
    CONSTRAINT hybrid_v2_operation_locks_operation_ck
        CHECK (operation IN ('REORG', 'REBUILD', 'RECONCILIATION'))
);

COMMENT ON TABLE hybrid_v2_operation_locks IS
    'Deployment-scoped row-based lock. One slot per deployment: reorg + rebuild + reconciliation are mutually exclusive.';

-------------------------------------------------------------------------------
-- Rebuild state machine
-------------------------------------------------------------------------------
--
-- One row per (deployment, rebuild_epoch). `phase` moves through:
--   NONE
--   REQUESTED
--   LOCK_ACQUIRED
--   VALIDATING_SOURCE
--   PREPARING
--   REPLAYING
--   CORRELATING
--   VERIFYING
--   RECONCILING
--   COMMITTING
--   COMPLETE
--   FAILED
--   MANUAL_INTERVENTION_REQUIRED
--
-- `mode`:
--   * JOURNAL_REPLAY — replay retained canonical journal into a fresh
--     ProjectionState + verify. No mutation on match; drift escalates.
--   * FRESH_CHAIN   — start from empty PG, walk (mock) RPC from
--     deployment_start_block to a bounded head via the normal worker
--     path, persisting journals + projections.
--
-- `generation_id` is nullable and reserved for a future shadow-
-- projection model — the current single-transaction model leaves it
-- NULL. Documented here so a future migration adding `generation_id`
-- columns on projection tables can populate this reference.

CREATE TABLE IF NOT EXISTS hybrid_v2_rebuild_operations (
    deployment_id           BIGINT      NOT NULL,
    rebuild_epoch           BIGINT      NOT NULL,
    mode                    TEXT        NOT NULL,
    phase                   TEXT        NOT NULL,
    requested_at_ms         BIGINT      NOT NULL,
    source_start_block      BIGINT      NULL,
    source_end_block        BIGINT      NULL,
    events_replayed         BIGINT      NULL,
    executions_correlated   BIGINT      NULL,
    verification_result     TEXT        NULL,
    reconciliation_result   TEXT        NULL,
    generation_id           BIGINT      NULL,
    retry_count             INTEGER     NOT NULL DEFAULT 0,
    last_failure_detail     TEXT        NULL,
    updated_at_ms           BIGINT      NOT NULL,
    completed_at_ms         BIGINT      NULL,
    PRIMARY KEY (deployment_id, rebuild_epoch),
    CONSTRAINT hybrid_v2_rebuild_operations_deployment_fk
        FOREIGN KEY (deployment_id)
        REFERENCES hybrid_v2_deployments(deployment_id)
        ON DELETE CASCADE,
    CONSTRAINT hybrid_v2_rebuild_operations_mode_ck
        CHECK (mode IN ('JOURNAL_REPLAY', 'FRESH_CHAIN'))
);

CREATE INDEX IF NOT EXISTS hybrid_v2_rebuild_operations_phase_idx
    ON hybrid_v2_rebuild_operations(deployment_id, phase);

CREATE INDEX IF NOT EXISTS hybrid_v2_rebuild_operations_updated_idx
    ON hybrid_v2_rebuild_operations(deployment_id, updated_at_ms DESC);

COMMENT ON TABLE hybrid_v2_rebuild_operations IS
    'Persisted rebuild state machine. One row per (deployment_id, rebuild_epoch).';

-------------------------------------------------------------------------------
-- Reconciliation results (append-only history)
-------------------------------------------------------------------------------
--
-- Every reconciliation run appends a row. Readers use the most-recent
-- row to derive readiness (drift keeps the deployment NOT_READY until
-- a subsequent run returns CONVERGED / IndexerBehind / ProviderUnavailable).
--
-- `classification` values (mirror `chain_view::ReconciliationResult`):
--   CONVERGED
--   INDEXER_BEHIND
--   NON_FINAL_DIFFERENCE
--   MANIFEST_MISMATCH
--   PROJECTION_DRIFT
--   PROVIDER_UNAVAILABLE
--   UNSUPPORTED_VIEW
--   MALFORMED_CHAIN_RESPONSE
--
-- `provider_availability` records whether the underlying chain-view
-- provider was available, so drift caused by an unavailable provider
-- is never treated as a projection defect.

CREATE TABLE IF NOT EXISTS hybrid_v2_reconciliation_results (
    reconciliation_id       BIGSERIAL   PRIMARY KEY,
    deployment_id           BIGINT      NOT NULL,
    ran_at_ms               BIGINT      NOT NULL,
    block_number_checked    BIGINT      NOT NULL,
    block_hash_checked      TEXT        NOT NULL,
    categories_checked      INTEGER     NOT NULL,
    items_checked           BIGINT      NOT NULL,
    converged_categories    INTEGER     NOT NULL,
    divergent_categories    INTEGER     NOT NULL,
    classification          TEXT        NOT NULL,
    mismatch_sample_json    JSONB       NULL,
    provider_availability   TEXT        NOT NULL,
    failure_detail          TEXT        NULL,
    duration_ms             BIGINT      NOT NULL,
    CONSTRAINT hybrid_v2_reconciliation_results_deployment_fk
        FOREIGN KEY (deployment_id)
        REFERENCES hybrid_v2_deployments(deployment_id)
        ON DELETE CASCADE,
    CONSTRAINT hybrid_v2_reconciliation_results_classification_ck
        CHECK (classification IN (
            'CONVERGED',
            'INDEXER_BEHIND',
            'NON_FINAL_DIFFERENCE',
            'MANIFEST_MISMATCH',
            'PROJECTION_DRIFT',
            'PROVIDER_UNAVAILABLE',
            'UNSUPPORTED_VIEW',
            'MALFORMED_CHAIN_RESPONSE'
        )),
    CONSTRAINT hybrid_v2_reconciliation_results_availability_ck
        CHECK (provider_availability IN ('AVAILABLE', 'DEGRADED', 'UNAVAILABLE'))
);

CREATE INDEX IF NOT EXISTS hybrid_v2_reconciliation_results_recent_idx
    ON hybrid_v2_reconciliation_results(deployment_id, ran_at_ms DESC);

CREATE INDEX IF NOT EXISTS hybrid_v2_reconciliation_results_classification_idx
    ON hybrid_v2_reconciliation_results(deployment_id, classification);

COMMENT ON TABLE hybrid_v2_reconciliation_results IS
    'Append-only history of reconciliation runs. Latest row per deployment gates readiness.';

COMMIT;
