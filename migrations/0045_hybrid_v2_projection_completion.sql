-- 0045_hybrid_v2_projection_completion.sql
--
-- BACKEND-SUBACCOUNT-CANONICAL-STATE-AND-INDEXER-V1-COMPLETION — additive
-- projection tables required to close the milestone without a `V1_SLICE`
-- suffix. NO destructive change to any prior table.
--
-- Adds coverage for:
--   * Positions per (subKey, series)       → hybrid_v2_positions
--   * Active-series membership             → hybrid_v2_active_series
--   * Order lifecycle (fill/cancel/nonce)  → hybrid_v2_order_lifecycle
--   * Matched execution groups             → hybrid_v2_matched_executions
--   * Fees / rebates aggregate journal     → hybrid_v2_fee_events
--   * Recovery epochs (owner + subaccount) → hybrid_v2_recovery_epochs
--   * Escape state per subKey              → hybrid_v2_escape_state
--   * Runtime rebuild + reconciliation     → hybrid_v2_projection_status /
--                                            _reconciliation_status
--
-- Canonicality rule (frozen): PostgreSQL is a REBUILDABLE PROJECTION. Every
-- row here is traceable to a raw log OR to an immutable manifest field.

BEGIN;

-- Positions per (subKey, series). Series is `keccak256(underlying, expiry,
-- strike, isCall)` as bytes32 hex. Quantities are `1e8`-scaled decimal.
CREATE TABLE IF NOT EXISTS hybrid_v2_positions (
    deployment_id   BIGINT      NOT NULL
                    REFERENCES hybrid_v2_deployments(deployment_id)
                    ON DELETE RESTRICT,
    subkey          TEXT        NOT NULL,
    series_id       TEXT        NOT NULL,
    long_qty_1e8    TEXT        NOT NULL DEFAULT '0',
    short_qty_1e8   TEXT        NOT NULL DEFAULT '0',
    last_event_block BIGINT     NOT NULL,
    updated_at_ms   BIGINT      NOT NULL,
    PRIMARY KEY (deployment_id, subkey, series_id)
);

CREATE INDEX IF NOT EXISTS hybrid_v2_positions_subkey_idx
    ON hybrid_v2_positions (deployment_id, subkey);

-- Active-series membership per subKey (capped at max_active_series=32).
CREATE TABLE IF NOT EXISTS hybrid_v2_active_series (
    deployment_id   BIGINT      NOT NULL
                    REFERENCES hybrid_v2_deployments(deployment_id)
                    ON DELETE RESTRICT,
    subkey          TEXT        NOT NULL,
    series_id       TEXT        NOT NULL,
    entered_block   BIGINT      NOT NULL,
    PRIMARY KEY (deployment_id, subkey, series_id)
);

-- Order lifecycle. Reusable-GTC orders accumulate `filled_qty_1e8` across
-- multiple partial fills; IOC/FOK orders reach a terminal state in one hop.
CREATE TABLE IF NOT EXISTS hybrid_v2_order_lifecycle (
    deployment_id   BIGINT      NOT NULL
                    REFERENCES hybrid_v2_deployments(deployment_id)
                    ON DELETE RESTRICT,
    order_hash      TEXT        NOT NULL,
    subkey          TEXT        NOT NULL,
    owner           TEXT        NOT NULL,
    series_id       TEXT        NULL,
    side            SMALLINT    NOT NULL, -- 0=Buy, 1=Sell
    time_in_force   SMALLINT    NOT NULL, -- 0=GTC, 1=IOC, 2=FOK
    total_qty_1e8   TEXT        NOT NULL DEFAULT '0',
    filled_qty_1e8  TEXT        NOT NULL DEFAULT '0',
    cancelled       BOOLEAN     NOT NULL DEFAULT FALSE,
    terminal        BOOLEAN     NOT NULL DEFAULT FALSE,
    first_seen_block BIGINT     NOT NULL,
    last_event_block BIGINT     NOT NULL,
    updated_at_ms   BIGINT      NOT NULL,
    PRIMARY KEY (deployment_id, order_hash)
);

CREATE INDEX IF NOT EXISTS hybrid_v2_order_lifecycle_subkey_idx
    ON hybrid_v2_order_lifecycle (deployment_id, subkey);

-- Matched executions. Each row is a completed correlation group.
CREATE TABLE IF NOT EXISTS hybrid_v2_matched_executions (
    deployment_id   BIGINT      NOT NULL
                    REFERENCES hybrid_v2_deployments(deployment_id)
                    ON DELETE RESTRICT,
    execution_id    TEXT        NOT NULL,
    buyer_order_hash TEXT       NOT NULL,
    seller_order_hash TEXT      NOT NULL,
    buyer_subkey    TEXT        NOT NULL,
    seller_subkey   TEXT        NOT NULL,
    series_id       TEXT        NOT NULL,
    matched_qty_1e8 TEXT        NOT NULL,
    premium_amount  TEXT        NOT NULL,
    fee_amount      TEXT        NOT NULL DEFAULT '0',
    rebate_amount   TEXT        NOT NULL DEFAULT '0',
    block_number    BIGINT      NOT NULL,
    tx_hash         TEXT        NOT NULL,
    completion_status TEXT      NOT NULL, -- COMPLETE, INCOMPLETE, INVALIDATED_BY_REORG
    completed_at_ms BIGINT      NOT NULL,
    PRIMARY KEY (deployment_id, execution_id)
);

CREATE INDEX IF NOT EXISTS hybrid_v2_matched_executions_tx_idx
    ON hybrid_v2_matched_executions (deployment_id, tx_hash);

-- Fee / rebate journal (canonical vault mutations, one row per event).
CREATE TABLE IF NOT EXISTS hybrid_v2_fee_events (
    deployment_id   BIGINT      NOT NULL
                    REFERENCES hybrid_v2_deployments(deployment_id)
                    ON DELETE RESTRICT,
    event_id        BIGSERIAL   PRIMARY KEY,
    kind            TEXT        NOT NULL, -- OPTION_FEE_CHARGED, OPTION_REBATE_PAID, OPTION_PREMIUM_TRANSFERRED
    payer_subkey    TEXT        NULL,
    receiver_subkey TEXT        NULL,
    token           TEXT        NOT NULL,
    amount          TEXT        NOT NULL,
    block_number    BIGINT      NOT NULL,
    tx_hash         TEXT        NOT NULL,
    log_index       INTEGER     NOT NULL,
    execution_id    TEXT        NULL,
    recorded_at_ms  BIGINT      NOT NULL
);

CREATE INDEX IF NOT EXISTS hybrid_v2_fee_events_deployment_idx
    ON hybrid_v2_fee_events (deployment_id, block_number, log_index);

-- Owner / subaccount recovery epochs.
CREATE TABLE IF NOT EXISTS hybrid_v2_recovery_epochs (
    deployment_id   BIGINT      NOT NULL
                    REFERENCES hybrid_v2_deployments(deployment_id)
                    ON DELETE RESTRICT,
    scope           TEXT        NOT NULL, -- 'OWNER' or 'SUBACCOUNT'
    scope_key       TEXT        NOT NULL, -- owner addr for OWNER, subKey for SUBACCOUNT
    epoch_count     BIGINT      NOT NULL DEFAULT 0,
    min_valid_nonce TEXT        NOT NULL DEFAULT '0',
    last_event_block BIGINT     NOT NULL,
    updated_at_ms   BIGINT      NOT NULL,
    PRIMARY KEY (deployment_id, scope, scope_key)
);

-- Escape / recovery lifecycle state per subKey.
CREATE TABLE IF NOT EXISTS hybrid_v2_escape_state (
    deployment_id   BIGINT      NOT NULL
                    REFERENCES hybrid_v2_deployments(deployment_id)
                    ON DELETE RESTRICT,
    subkey          TEXT        NOT NULL,
    state           TEXT        NOT NULL, -- NORMAL / REQUESTED / ACTIVATED / CANCELLED / FINALIZED
    requested_ts    BIGINT      NULL,
    activation_eligible_at BIGINT NULL,
    activated_ts    BIGINT      NULL,
    cancelled_ts    BIGINT      NULL,
    finalized_ts    BIGINT      NULL,
    last_event_block BIGINT     NOT NULL,
    updated_at_ms   BIGINT      NOT NULL,
    PRIMARY KEY (deployment_id, subkey)
);

-- Rebuild + reconciliation runtime status. One row per deployment for each.
CREATE TABLE IF NOT EXISTS hybrid_v2_projection_status (
    deployment_id   BIGINT      NOT NULL
                    REFERENCES hybrid_v2_deployments(deployment_id)
                    ON DELETE RESTRICT,
    status          TEXT        NOT NULL, -- IDLE, REBUILDING, READY, FAILED
    started_at_ms   BIGINT      NULL,
    finished_at_ms  BIGINT      NULL,
    events_replayed BIGINT      NOT NULL DEFAULT 0,
    last_error      TEXT        NULL,
    PRIMARY KEY (deployment_id)
);

CREATE TABLE IF NOT EXISTS hybrid_v2_reconciliation_status (
    deployment_id   BIGINT      NOT NULL
                    REFERENCES hybrid_v2_deployments(deployment_id)
                    ON DELETE RESTRICT,
    last_run_at_ms  BIGINT      NOT NULL,
    classification  TEXT        NOT NULL, -- CONVERGED, BEHIND, NON_FINAL, MANIFEST_MISMATCH, DRIFT, PROVIDER_UNAVAILABLE, UNSUPPORTED
    detail_json     JSONB       NOT NULL,
    at_block        BIGINT      NOT NULL,
    PRIMARY KEY (deployment_id)
);

COMMIT;
