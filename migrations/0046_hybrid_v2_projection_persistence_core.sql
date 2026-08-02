-- 0046_hybrid_v2_projection_persistence_core.sql
--
-- BACKEND-HYBRID-V2-POSTGRES-PROJECTION-CORE-V1 — additive persistence
-- surface required to close every projection category emitted by the
-- Hybrid V2 canonical event stream. NO destructive change to any prior
-- V1 table and NO destructive change to migrations 0044 / 0045.
--
-- Adds coverage for categories not represented in 0044 / 0045:
--   * per-subKey pause flags                 → hybrid_v2_pause_flags
--   * per-(subKey, token) bad debt           → hybrid_v2_bad_debt
--   * per-subKey recovery finalization
--     withdrawal count                       → hybrid_v2_recovery_withdrawal_counts
--   * per-deployment recovery-pause          → hybrid_v2_recovery_pause
--   * per-deployment runtime metrics         → hybrid_v2_runtime_metrics
--   * per-deployment readiness state         → hybrid_v2_readiness
--   * canonical block registry with reorg
--     orphan tracking                        → hybrid_v2_canonical_blocks
--   * reorg-event log                        → hybrid_v2_reorg_events
--
-- Additive columns (schema evolution only, defaults preserved):
--   * hybrid_v2_cursors.reorg_count            BIGINT DEFAULT 0
--   * hybrid_v2_cursors.max_reorg_depth_seen   BIGINT DEFAULT 0
--   * hybrid_v2_cursors.decode_failures        BIGINT DEFAULT 0
--   * hybrid_v2_cursors.projection_failures    BIGINT DEFAULT 0
--   * hybrid_v2_cursors.unknown_canonical_events BIGINT DEFAULT 0
--   * hybrid_v2_recovery_state.state_updated_block BIGINT DEFAULT 0
--
-- Canonicality rule (frozen): PostgreSQL is a REBUILDABLE PROJECTION of
-- chain state. Every row here is traceable to a raw log OR to an
-- immutable manifest field. The Rust reducer is the single semantic
-- implementation; SQL constraints enforce invariants but never encode
-- an independent economic rule.

BEGIN;

-------------------------------------------------------------------------------
-- Canonical block registry
-------------------------------------------------------------------------------
--
-- One row per block observed by the indexer, keyed on
-- (deployment_id, block_hash) so a replaced block after a reorg still
-- appears as a distinct row. Enables reorg-safe reconciliation +
-- keyset pagination over the indexer's observed head.

CREATE TABLE IF NOT EXISTS hybrid_v2_canonical_blocks (
    deployment_id   BIGINT      NOT NULL
                    REFERENCES hybrid_v2_deployments(deployment_id)
                    ON DELETE RESTRICT,
    block_hash      TEXT        NOT NULL,
    block_number    BIGINT      NOT NULL,
    parent_hash     TEXT        NOT NULL,
    block_timestamp BIGINT      NOT NULL,
    is_canonical    BOOLEAN     NOT NULL DEFAULT TRUE,
    orphaned_at_block BIGINT    NULL,
    applied_at_ms   BIGINT      NOT NULL,
    PRIMARY KEY (deployment_id, block_hash)
);

CREATE INDEX IF NOT EXISTS hybrid_v2_canonical_blocks_number_idx
    ON hybrid_v2_canonical_blocks (deployment_id, is_canonical, block_number);

CREATE INDEX IF NOT EXISTS hybrid_v2_canonical_blocks_number_desc_idx
    ON hybrid_v2_canonical_blocks (deployment_id, block_number DESC);

-------------------------------------------------------------------------------
-- Per-subKey pause flags
-------------------------------------------------------------------------------

CREATE TABLE IF NOT EXISTS hybrid_v2_pause_flags (
    deployment_id   BIGINT      NOT NULL
                    REFERENCES hybrid_v2_deployments(deployment_id)
                    ON DELETE RESTRICT,
    subkey          TEXT        NOT NULL,
    paused          BOOLEAN     NOT NULL,
    last_event_block BIGINT     NOT NULL,
    updated_at_ms   BIGINT      NOT NULL,
    PRIMARY KEY (deployment_id, subkey)
);

CREATE INDEX IF NOT EXISTS hybrid_v2_pause_flags_paused_idx
    ON hybrid_v2_pause_flags (deployment_id, paused);

-------------------------------------------------------------------------------
-- Per-(subKey, token) bad-debt ledger
-------------------------------------------------------------------------------
--
-- Uint256 stored as TEXT (decimal) to preserve exact precision. Same
-- storage discipline as hybrid_v2_vault_balances.

CREATE TABLE IF NOT EXISTS hybrid_v2_bad_debt (
    deployment_id   BIGINT      NOT NULL
                    REFERENCES hybrid_v2_deployments(deployment_id)
                    ON DELETE RESTRICT,
    subkey          TEXT        NOT NULL,
    token           TEXT        NOT NULL,
    amount          TEXT        NOT NULL DEFAULT '0',
    last_event_block BIGINT     NOT NULL,
    updated_at_ms   BIGINT      NOT NULL,
    PRIMARY KEY (deployment_id, subkey, token)
);

CREATE INDEX IF NOT EXISTS hybrid_v2_bad_debt_subkey_idx
    ON hybrid_v2_bad_debt (deployment_id, subkey);

-------------------------------------------------------------------------------
-- Per-subKey recovery-finalization withdrawal count
-------------------------------------------------------------------------------

CREATE TABLE IF NOT EXISTS hybrid_v2_recovery_withdrawal_counts (
    deployment_id   BIGINT      NOT NULL
                    REFERENCES hybrid_v2_deployments(deployment_id)
                    ON DELETE RESTRICT,
    subkey          TEXT        NOT NULL,
    withdrawal_count INTEGER    NOT NULL DEFAULT 0,
    last_event_block BIGINT     NOT NULL,
    updated_at_ms   BIGINT      NOT NULL,
    PRIMARY KEY (deployment_id, subkey),
    CONSTRAINT hybrid_v2_recovery_withdrawal_counts_nonneg
        CHECK (withdrawal_count >= 0)
);

-------------------------------------------------------------------------------
-- Per-deployment recovery-pause singleton
-------------------------------------------------------------------------------

CREATE TABLE IF NOT EXISTS hybrid_v2_recovery_pause (
    deployment_id   BIGINT      NOT NULL PRIMARY KEY
                    REFERENCES hybrid_v2_deployments(deployment_id)
                    ON DELETE RESTRICT,
    paused          BOOLEAN     NOT NULL DEFAULT FALSE,
    until_ts        BIGINT      NOT NULL DEFAULT 0,
    last_event_block BIGINT     NOT NULL,
    updated_at_ms   BIGINT      NOT NULL
);

-------------------------------------------------------------------------------
-- Per-deployment runtime metrics counters
-------------------------------------------------------------------------------

CREATE TABLE IF NOT EXISTS hybrid_v2_runtime_metrics (
    deployment_id   BIGINT      NOT NULL PRIMARY KEY
                    REFERENCES hybrid_v2_deployments(deployment_id)
                    ON DELETE RESTRICT,
    reorg_count     BIGINT      NOT NULL DEFAULT 0,
    max_reorg_depth_seen BIGINT NOT NULL DEFAULT 0,
    decode_failures BIGINT      NOT NULL DEFAULT 0,
    projection_failures BIGINT  NOT NULL DEFAULT 0,
    unknown_canonical_events BIGINT NOT NULL DEFAULT 0,
    last_success_block BIGINT   NOT NULL DEFAULT 0,
    updated_at_ms   BIGINT      NOT NULL,
    CONSTRAINT hybrid_v2_runtime_metrics_nonneg CHECK (
        reorg_count >= 0
            AND max_reorg_depth_seen >= 0
            AND decode_failures >= 0
            AND projection_failures >= 0
            AND unknown_canonical_events >= 0
            AND last_success_block >= 0
    )
);

-------------------------------------------------------------------------------
-- Per-deployment readiness state
-------------------------------------------------------------------------------

CREATE TABLE IF NOT EXISTS hybrid_v2_readiness (
    deployment_id   BIGINT      NOT NULL PRIMARY KEY
                    REFERENCES hybrid_v2_deployments(deployment_id)
                    ON DELETE RESTRICT,
    ready           BOOLEAN     NOT NULL DEFAULT FALSE,
    reason          TEXT        NULL,
    reason_detail   TEXT        NULL,
    updated_at_ms   BIGINT      NOT NULL
);

-------------------------------------------------------------------------------
-- Reorg-event log (observability + reconciliation)
-------------------------------------------------------------------------------

CREATE TABLE IF NOT EXISTS hybrid_v2_reorg_events (
    reorg_event_id  BIGSERIAL   PRIMARY KEY,
    deployment_id   BIGINT      NOT NULL
                    REFERENCES hybrid_v2_deployments(deployment_id)
                    ON DELETE RESTRICT,
    detected_at_ms  BIGINT      NOT NULL,
    from_block      BIGINT      NOT NULL,
    from_hash       TEXT        NOT NULL,
    to_block        BIGINT      NOT NULL,
    to_hash         TEXT        NOT NULL,
    depth           INTEGER     NOT NULL,
    orphaned_log_count INTEGER  NOT NULL DEFAULT 0
);

CREATE INDEX IF NOT EXISTS hybrid_v2_reorg_events_deployment_idx
    ON hybrid_v2_reorg_events (deployment_id, detected_at_ms DESC);

-------------------------------------------------------------------------------
-- Additive columns on existing tables
-------------------------------------------------------------------------------

ALTER TABLE hybrid_v2_cursors
    ADD COLUMN IF NOT EXISTS reorg_count BIGINT NOT NULL DEFAULT 0,
    ADD COLUMN IF NOT EXISTS max_reorg_depth_seen BIGINT NOT NULL DEFAULT 0,
    ADD COLUMN IF NOT EXISTS decode_failures BIGINT NOT NULL DEFAULT 0,
    ADD COLUMN IF NOT EXISTS projection_failures BIGINT NOT NULL DEFAULT 0,
    ADD COLUMN IF NOT EXISTS unknown_canonical_events BIGINT NOT NULL DEFAULT 0,
    ADD COLUMN IF NOT EXISTS last_success_block BIGINT NOT NULL DEFAULT 0;

ALTER TABLE hybrid_v2_recovery_state
    ADD COLUMN IF NOT EXISTS state_updated_block BIGINT NOT NULL DEFAULT 0;

-------------------------------------------------------------------------------
-- Extra indexes on 0044 / 0045 tables for keyset-pagination read patterns
-------------------------------------------------------------------------------

CREATE INDEX IF NOT EXISTS hybrid_v2_vault_balances_block_idx
    ON hybrid_v2_vault_balances (deployment_id, subkey, last_event_block DESC);

CREATE INDEX IF NOT EXISTS hybrid_v2_reservations_block_idx
    ON hybrid_v2_reservations (deployment_id, subkey, last_event_block DESC);

CREATE INDEX IF NOT EXISTS hybrid_v2_positions_block_idx
    ON hybrid_v2_positions (deployment_id, subkey, last_event_block DESC);

CREATE INDEX IF NOT EXISTS hybrid_v2_order_lifecycle_block_idx
    ON hybrid_v2_order_lifecycle (deployment_id, subkey, last_event_block DESC);

CREATE INDEX IF NOT EXISTS hybrid_v2_matched_executions_block_idx
    ON hybrid_v2_matched_executions (deployment_id, block_number DESC);

CREATE INDEX IF NOT EXISTS hybrid_v2_fee_events_subkey_idx
    ON hybrid_v2_fee_events (deployment_id, payer_subkey, block_number DESC);

CREATE INDEX IF NOT EXISTS hybrid_v2_fee_events_receiver_idx
    ON hybrid_v2_fee_events (deployment_id, receiver_subkey, block_number DESC);

COMMIT;
