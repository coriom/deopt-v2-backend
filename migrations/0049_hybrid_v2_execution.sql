-- 0049_hybrid_v2_execution.sql
--
-- BACKEND-HYBRID-V2-SIGNER-AND-EXECUTION-V1 (Foundation package,
-- parts C–H) — persist pre-broadcast execution intent, phase
-- transitions, and executor-nonce reservations.
--
-- Additive-only. Does NOT alter or drop any existing table or column.
-- Extends the CHECK constraint on `hybrid_v2_operation_locks.operation`
-- to accept 'EXECUTION' so the unified deployment-scoped lock can
-- serialize execution vs. reorg vs. rebuild vs. reconciliation.
--
-- Frozen safety invariants newly enforced here:
--   * BROADCAST_IS_DISABLED — the only terminal success phase in the
--     `phase` CHECK constraint is BROADCAST_DISABLED. There is
--     deliberately no BROADCASTED value: this milestone's RPC layer
--     has no send_* method, and the schema mirrors that firewall.
--   * CHAIN_STATE_IS_CANONICAL_POSTGRES_IS_A_REBUILDABLE_NON_CANONICAL_PROJECTION
--     — every column in `hybrid_v2_execution_requests` is a
--     projection of intent, not a canonical economic fact. The
--     `canonical_execution_id` PK is a backend-derived hash whose
--     preimage is defined in `src/hybrid_v2/execution/identity.rs`.
--   * IMMUTABLE_PLAN_HASH — once `plan_hash` is set to a non-null
--     value it MUST NOT change. Enforced by a trigger below and by
--     the Rust `update_execution_phase` conditional update.
--   * ATOMIC_NONCE_RESERVATION — `(chain_id, signer_identity,
--     reserved_nonce)` is UNIQUE. Callers reserve via
--     `INSERT ... ON CONFLICT DO NOTHING` so no two executions ever
--     race a duplicate nonce onto the same signer.

BEGIN;

-------------------------------------------------------------------------------
-- Operation lock CHECK constraint — add 'EXECUTION'
-------------------------------------------------------------------------------
--
-- The unified `hybrid_v2_operation_locks` row is now shared with the
-- execution orchestrator. Reorg + rebuild + reconciliation + execution
-- all contend for the single row per deployment; at most one holds it.
--
-- Rewrite (drop + re-add) the constraint idempotently.

ALTER TABLE hybrid_v2_operation_locks
    DROP CONSTRAINT IF EXISTS hybrid_v2_operation_locks_operation_ck;

ALTER TABLE hybrid_v2_operation_locks
    ADD CONSTRAINT hybrid_v2_operation_locks_operation_ck
        CHECK (operation IN ('REORG', 'REBUILD', 'RECONCILIATION', 'EXECUTION'));

-------------------------------------------------------------------------------
-- Execution request table (source of truth for pre-broadcast intent)
-------------------------------------------------------------------------------
--
-- One row per canonical execution attempt. The `canonical_execution_id`
-- primary key is the deterministic hash defined in
-- `src/hybrid_v2/execution/identity.rs`; a duplicate intent-to-execute
-- maps to the same row (idempotent by construction).
--
-- Phase machine: DISCOVERED -> VALIDATING -> READY_TO_SIMULATE ->
--   SIMULATING -> SIMULATION_SUCCEEDED -> AWAITING_SIGNATURE ->
--   SIGNING -> SIGNATURE_VERIFIED -> READY_FOR_BROADCAST ->
--   BROADCAST_DISABLED (terminal success).
--
-- Additional terminal phases: SIMULATION_FAILED (funnels into FAILED
-- only), CANCELLED, STALE, FAILED. See `src/hybrid_v2/execution/state.rs`
-- for the full legal-edge matrix; the SQL `update_execution_phase`
-- function reproduces the matrix via a conditional WHERE clause.

CREATE TABLE IF NOT EXISTS hybrid_v2_execution_requests (
    canonical_execution_id        TEXT           PRIMARY KEY,
    deployment_id                 BIGINT         NOT NULL,
    chain_id                      BIGINT         NOT NULL,
    execution_kind                TEXT           NOT NULL DEFAULT 'HYBRID_V2_OPTION_MATCH',
    buyer_order_hash              TEXT           NOT NULL,
    seller_order_hash             TEXT           NOT NULL,
    buyer_subkey                  TEXT           NOT NULL,
    seller_subkey                 TEXT           NOT NULL,
    series_id                     TEXT           NOT NULL,
    fill_quantity_1e8             NUMERIC(78,0)  NOT NULL,
    premium_amount                NUMERIC(78,0)  NOT NULL,
    fee_schedule_epoch            BIGINT         NULL,
    source_matched_execution_id   TEXT           NULL,

    -- Plan surface (populated when the deterministic plan is built)
    target_contract               TEXT           NOT NULL,
    selector                      CHAR(10)       NOT NULL,
    calldata_hash                 TEXT           NULL,
    plan_hash                     TEXT           NULL,
    tx_value_wei                  NUMERIC(78,0)  NOT NULL DEFAULT 0,

    -- Simulation surface (populated by the next package)
    simulation_block_number       BIGINT         NULL,
    simulation_block_hash         TEXT           NULL,
    simulation_gas_estimate       BIGINT         NULL,
    simulation_result_json        JSONB          NULL,

    -- Signature surface (populated by the signer package)
    signer_identity               TEXT           NULL,
    signing_payload_hash          TEXT           NULL,
    signature_r                   TEXT           NULL,
    signature_s                   TEXT           NULL,
    signature_v                   SMALLINT       NULL,
    recovered_signer              TEXT           NULL,

    -- Gas & nonce surface (populated by the orchestrator package)
    gas_limit                     BIGINT         NULL,
    max_fee_per_gas_wei           NUMERIC(78,0)  NULL,
    max_priority_fee_per_gas_wei  NUMERIC(78,0)  NULL,
    reserved_nonce                BIGINT         NULL,

    -- State machine
    phase                         TEXT           NOT NULL DEFAULT 'DISCOVERED',
    failure_class                 TEXT           NULL,
    failure_detail                TEXT           NULL,
    retry_count                   INTEGER        NOT NULL DEFAULT 0,

    -- Lock correlation (holder_epoch of the paired operation lock).
    -- Populated at the moment the execution orchestrator acquires the
    -- unified operation lock. The stale-lock cleanup in
    -- `try_acquire_operation_lock` uses this to correlate a live
    -- EXECUTION lock to a terminal execution row.
    holder_epoch                  BIGINT         NULL,

    created_at_ms                 BIGINT         NOT NULL,
    updated_at_ms                 BIGINT         NOT NULL,

    CONSTRAINT hybrid_v2_execution_requests_deployment_fk
        FOREIGN KEY (deployment_id)
        REFERENCES hybrid_v2_deployments(deployment_id)
        ON DELETE CASCADE,

    CONSTRAINT hybrid_v2_execution_requests_kind_ck
        CHECK (execution_kind IN ('HYBRID_V2_OPTION_MATCH')),

    CONSTRAINT hybrid_v2_execution_requests_phase_ck
        CHECK (phase IN (
            'DISCOVERED',
            'VALIDATING',
            'READY_TO_SIMULATE',
            'SIMULATING',
            'SIMULATION_SUCCEEDED',
            'SIMULATION_FAILED',
            'AWAITING_SIGNATURE',
            'SIGNING',
            'SIGNATURE_VERIFIED',
            'READY_FOR_BROADCAST',
            'BROADCAST_DISABLED',
            'CANCELLED',
            'STALE',
            'FAILED'
        )),

    CONSTRAINT hybrid_v2_execution_requests_selector_shape_ck
        CHECK (selector LIKE '0x________')
);

CREATE INDEX IF NOT EXISTS hybrid_v2_execution_requests_phase_idx
    ON hybrid_v2_execution_requests(deployment_id, phase);

CREATE INDEX IF NOT EXISTS hybrid_v2_execution_requests_updated_idx
    ON hybrid_v2_execution_requests(deployment_id, updated_at_ms DESC);

CREATE INDEX IF NOT EXISTS hybrid_v2_execution_requests_holder_idx
    ON hybrid_v2_execution_requests(deployment_id, holder_epoch);

COMMENT ON TABLE hybrid_v2_execution_requests IS
    'Pre-broadcast Hybrid V2 execution intent. Source of truth keyed on canonical_execution_id (see src/hybrid_v2/execution/identity.rs).';

COMMENT ON COLUMN hybrid_v2_execution_requests.plan_hash IS
    'Deterministic plan hash (keccak256 of the calldata surface). IMMUTABLE ONCE SET — enforced by the plan_hash immutability trigger.';

-------------------------------------------------------------------------------
-- Plan-hash immutability trigger
-------------------------------------------------------------------------------
--
-- The `plan_hash` and `calldata_hash` columns are derived from the
-- ABI-encoded `executeMatch` calldata. Once populated they represent
-- the exact bytes the signer will sign; any change post-hoc would
-- silently substitute the payload. Reject any UPDATE that mutates a
-- non-null hash to a different non-null value.

CREATE OR REPLACE FUNCTION hybrid_v2_execution_requests_plan_immutability()
RETURNS TRIGGER AS $$
BEGIN
    IF OLD.plan_hash IS NOT NULL
       AND NEW.plan_hash IS NOT NULL
       AND NEW.plan_hash <> OLD.plan_hash THEN
        RAISE EXCEPTION
            'plan_hash is immutable once set for canonical_execution_id %',
            OLD.canonical_execution_id
            USING ERRCODE = 'check_violation';
    END IF;
    IF OLD.calldata_hash IS NOT NULL
       AND NEW.calldata_hash IS NOT NULL
       AND NEW.calldata_hash <> OLD.calldata_hash THEN
        RAISE EXCEPTION
            'calldata_hash is immutable once set for canonical_execution_id %',
            OLD.canonical_execution_id
            USING ERRCODE = 'check_violation';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS hybrid_v2_execution_requests_plan_immutability_tr
    ON hybrid_v2_execution_requests;

CREATE TRIGGER hybrid_v2_execution_requests_plan_immutability_tr
BEFORE UPDATE ON hybrid_v2_execution_requests
FOR EACH ROW EXECUTE FUNCTION hybrid_v2_execution_requests_plan_immutability();

-------------------------------------------------------------------------------
-- Execution attempts (bounded history — never mutated)
-------------------------------------------------------------------------------
--
-- Each phase transition observed by the orchestrator writes one row
-- here. The composite UNIQUE(canonical_execution_id, attempt_number)
-- lets restart-safe replay dedupe by attempt without the writer
-- reading first.

CREATE TABLE IF NOT EXISTS hybrid_v2_execution_attempts (
    attempt_id                    BIGSERIAL      PRIMARY KEY,
    canonical_execution_id        TEXT           NOT NULL,
    attempt_number                INTEGER        NOT NULL,
    phase                         TEXT           NOT NULL,
    entered_at_ms                 BIGINT         NOT NULL,
    detail_json                   JSONB          NULL,

    CONSTRAINT hybrid_v2_execution_attempts_execution_fk
        FOREIGN KEY (canonical_execution_id)
        REFERENCES hybrid_v2_execution_requests(canonical_execution_id)
        ON DELETE CASCADE,

    CONSTRAINT hybrid_v2_execution_attempts_uniq
        UNIQUE (canonical_execution_id, attempt_number)
);

CREATE INDEX IF NOT EXISTS hybrid_v2_execution_attempts_idx
    ON hybrid_v2_execution_attempts(canonical_execution_id, attempt_number);

COMMENT ON TABLE hybrid_v2_execution_attempts IS
    'Append-only phase transition history. Never mutated. Dedupe key: (canonical_execution_id, attempt_number).';

-------------------------------------------------------------------------------
-- Executor nonce reservations
-------------------------------------------------------------------------------
--
-- Atomic reservation primitive: the UNIQUE constraint on
-- (chain_id, signer_identity, reserved_nonce) guarantees no two
-- concurrent orchestrator workers ever race a duplicate nonce onto
-- the same signer address. Reservation is via
--   INSERT ... ON CONFLICT DO NOTHING
-- and the caller MUST observe rows_affected == 1 to know it holds the
-- reservation.
--
-- Status transitions:
--   RESERVED  → SETTLED   (successful path)
--   RESERVED  → ABANDONED (orchestrator gave up on this nonce)
-- Terminal states are historical only — the row is retained so a
-- restart can enumerate prior reservations to avoid re-issuing.

CREATE TABLE IF NOT EXISTS hybrid_v2_executor_nonces (
    id                            BIGSERIAL      PRIMARY KEY,
    chain_id                      BIGINT         NOT NULL,
    signer_identity               TEXT           NOT NULL,
    reserved_nonce                BIGINT         NOT NULL,
    canonical_execution_id        TEXT           NULL,
    reserved_at_ms                BIGINT         NOT NULL,
    status                        TEXT           NOT NULL DEFAULT 'RESERVED',

    CONSTRAINT hybrid_v2_executor_nonces_execution_fk
        FOREIGN KEY (canonical_execution_id)
        REFERENCES hybrid_v2_execution_requests(canonical_execution_id)
        ON DELETE SET NULL,

    CONSTRAINT hybrid_v2_executor_nonces_status_ck
        CHECK (status IN ('RESERVED', 'ABANDONED', 'SETTLED')),

    CONSTRAINT hybrid_v2_executor_nonces_uniq
        UNIQUE (chain_id, signer_identity, reserved_nonce)
);

CREATE INDEX IF NOT EXISTS hybrid_v2_executor_nonces_lookup_idx
    ON hybrid_v2_executor_nonces(chain_id, signer_identity, status);

COMMENT ON TABLE hybrid_v2_executor_nonces IS
    'Atomic per-signer nonce reservation. UNIQUE(chain_id, signer_identity, reserved_nonce) prevents double-reservation.';

COMMIT;
