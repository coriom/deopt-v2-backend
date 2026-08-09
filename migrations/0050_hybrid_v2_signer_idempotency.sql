-- 0050_hybrid_v2_signer_idempotency.sql
--
-- BACKEND-HYBRID-V2-EXTERNAL-SIGNER-INTEGRATION-AND-LIVE-ORCHESTRATOR-V1
-- (Package A, Part L) — persist a bounded 16-byte idempotency key for
-- every external-signer request so a re-issued sign request converges
-- on the same signature (when the signer supports idempotency) and so
-- state-machine gating can reason about which sign attempt owns the
-- persisted signature.
--
-- Additive-only:
--   * ALTER TABLE hybrid_v2_execution_requests ADD COLUMN
--     signer_request_idempotency_key TEXT NULL — nullable so pre-existing
--     rows (populated before this migration ran) are legal without a
--     backfill.
--   * Extends the plan-hash immutability trigger to enforce
--     `signer_request_idempotency_key` immutability once set (same
--     pattern as `plan_hash` and `calldata_hash`).
--
-- Frozen safety invariants preserved:
--   * BROADCAST_IS_DISABLED — this migration adds only a bookkeeping
--     column; it does NOT introduce a broadcast column, a raw-tx column,
--     or any new terminal phase.
--   * IMMUTABLE_SIGNER_REQUEST_IDEMPOTENCY_KEY — once set to a non-null
--     value, MUST NOT change. Same enforcement pattern as plan_hash.
--   * CHAIN_STATE_IS_CANONICAL_POSTGRES_IS_A_REBUILDABLE_NON_CANONICAL_PROJECTION
--     — the idempotency key is a projection derived deterministically
--     from (expected_signer_address, canonical_execution_id, plan_hash,
--     signing_payload_hash) and can be re-derived at any time.

BEGIN;

ALTER TABLE hybrid_v2_execution_requests
    ADD COLUMN IF NOT EXISTS signer_request_idempotency_key TEXT NULL;

COMMENT ON COLUMN hybrid_v2_execution_requests.signer_request_idempotency_key IS
    'External signer idempotency key: 0x-prefixed 16-byte hex derived from keccak256("HV2_SIGNER_IDEMPOTENCY_V1" || expected_signer_address || canonical_execution_id || plan_hash || signing_payload_hash). IMMUTABLE ONCE SET.';

-- Replace the plan-hash immutability trigger function so it also
-- enforces `signer_request_idempotency_key` immutability once set.
-- Uses CREATE OR REPLACE — no destructive drop needed.

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
    IF OLD.signer_request_idempotency_key IS NOT NULL
       AND NEW.signer_request_idempotency_key IS NOT NULL
       AND NEW.signer_request_idempotency_key <> OLD.signer_request_idempotency_key THEN
        RAISE EXCEPTION
            'signer_request_idempotency_key is immutable once set for canonical_execution_id %',
            OLD.canonical_execution_id
            USING ERRCODE = 'check_violation';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

COMMIT;
