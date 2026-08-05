-- 0047_hybrid_v2_reorg_recovery.sql
--
-- BACKEND-HYBRID-V2-PERSISTED-REORG-RECOVERY-V1 — additive schema for
-- restart-safe, persisted chain-reorg recovery state machine. NO
-- destructive change to any prior V1 table.
--
-- Adds:
--   * hybrid_v2_reorg_recovery       — per-deployment recovery state row
--                                      with phase + progress + audit metadata
--   * hybrid_v2_reorg_locks          — deployment-scoped row-based mutex
--                                      (single writer per deployment while
--                                      a recovery is active)
--
-- Frozen safety rules:
--   * REORG_RECOVERY_USES_PERSISTED_CANONICAL_REPLAY — the recovery
--     service must reconstruct projections via the canonical replay
--     primitive (`RebuildService::replay_all`) rather than any
--     inverse-arithmetic path.
--   * NO_ORPHANED_ECONOMIC_STATE_IS_PUBLICLY_VISIBLE — once
--     recovery commits, no `is_canonical=TRUE` row exists on the
--     orphan branch.
--   * NO_READY_STATE_DURING_INCOMPLETE_REORG_RECOVERY — readiness
--     stays hard-503 through every recovery phase.
--
-- Every column in this migration is populated by the recovery service.

BEGIN;

-------------------------------------------------------------------------------
-- Recovery state machine row (one per deployment)
-------------------------------------------------------------------------------
--
-- `phase` is one of:
--   NONE, DETECTED, SEARCHING_ANCESTOR, ANCESTOR_FOUND, FETCHING_REPLACEMENT,
--   INVALIDATING_ORPHAN_BRANCH, REPLAYING_REPLACEMENT, COMMITTING_REPLACEMENT,
--   RECOVERED, FAILED, MANUAL_INTERVENTION_REQUIRED
--
-- `recovery_epoch` monotonically increases each time recovery is
-- initiated (used as the fencing token for the row-based lock).
--
-- `retry_count` counts retryable failures. Once the service escalates
-- to MANUAL_INTERVENTION_REQUIRED the worker for this deployment
-- stops advancing. Operator restart clears the phase.

CREATE TABLE IF NOT EXISTS hybrid_v2_reorg_recovery (
    deployment_id            BIGINT      PRIMARY KEY,
    recovery_epoch           BIGINT      NOT NULL DEFAULT 0,
    phase                    TEXT        NOT NULL,
    detected_at_ms           BIGINT      NOT NULL,
    old_tip_block            BIGINT      NOT NULL,
    old_tip_hash             TEXT        NOT NULL,
    conflicting_block        BIGINT      NULL,
    conflicting_hash         TEXT        NULL,
    common_ancestor_block    BIGINT      NULL,
    common_ancestor_hash     TEXT        NULL,
    replacement_tip_block    BIGINT      NULL,
    replacement_tip_hash     TEXT        NULL,
    orphan_depth             INTEGER     NULL,
    replacement_block_count  INTEGER     NULL,
    finalized_at_detection   BIGINT      NULL,
    retry_count              INTEGER     NOT NULL DEFAULT 0,
    last_failure_detail      TEXT        NULL,
    updated_at_ms            BIGINT      NOT NULL,
    completed_at_ms          BIGINT      NULL,
    CONSTRAINT hybrid_v2_reorg_recovery_deployment_fk
        FOREIGN KEY (deployment_id)
        REFERENCES hybrid_v2_deployments(deployment_id)
        ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS hybrid_v2_reorg_recovery_phase_idx
    ON hybrid_v2_reorg_recovery(phase);

COMMENT ON TABLE hybrid_v2_reorg_recovery IS
    'Per-deployment reorg recovery state machine. Populated during recovery, cleared to phase=RECOVERED on success.';

-------------------------------------------------------------------------------
-- Recovery lock (deployment-scoped mutex)
-------------------------------------------------------------------------------
--
-- Simple row-based mutex — INSERT ... ON CONFLICT DO NOTHING acquires,
-- DELETE releases. `holder_epoch` is a fencing token: a delayed
-- holder can only release the lock if its epoch still matches. On
-- worker bootstrap a stale row (recovery already completed OR the
-- corresponding recovery_epoch has advanced) is safely deleted.

CREATE TABLE IF NOT EXISTS hybrid_v2_reorg_locks (
    deployment_id  BIGINT      PRIMARY KEY,
    holder_epoch   BIGINT      NOT NULL,
    acquired_at_ms BIGINT      NOT NULL,
    CONSTRAINT hybrid_v2_reorg_locks_deployment_fk
        FOREIGN KEY (deployment_id)
        REFERENCES hybrid_v2_deployments(deployment_id)
        ON DELETE CASCADE
);

COMMENT ON TABLE hybrid_v2_reorg_locks IS
    'Deployment-scoped row-based lock protecting concurrent reorg-recovery attempts.';

COMMIT;
