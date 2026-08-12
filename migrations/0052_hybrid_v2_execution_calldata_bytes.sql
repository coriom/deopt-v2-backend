-- BACKEND-HYBRID-V2-BROADCAST-LIVE-WIRING-CLOSURE-V1 (Part D)
--
-- Persist the full calldata bytes on `hybrid_v2_execution_requests` so
-- the admin fresh-submit path can hydrate a complete `ExecutionPlan`
-- from the row (needed to serialize the signed envelope for
-- `BroadcastOutbox::submit`). Prior migration 0049 only persisted
-- `calldata_hash`; the raw bytes were lost after the orchestrator
-- built the plan in memory.
--
-- Frozen safety notes:
--   * `calldata_bytes` is NULL for legacy rows and immutable once set
--     (same posture as `plan_hash` / `calldata_hash`).
--   * The admin reconstruction helper MUST recompute keccak256 of the
--     bytes and assert equality with `calldata_hash` before feeding the
--     plan to the outbox. If they disagree, the fresh-submit path
--     refuses with `CALLDATA_HASH_MISMATCH` (no RPC contact).
--   * No new grant / role change — the column inherits table ACL.

ALTER TABLE hybrid_v2_execution_requests
    ADD COLUMN IF NOT EXISTS calldata_bytes BYTEA NULL;

-- Sparse index over rows still awaiting a `calldata_bytes` back-fill.
-- Fresh rows written by an updated orchestrator carry the bytes at
-- insert-time; legacy rows (pre-migration) may remain NULL and the
-- fresh-submit path will refuse them with `CALLDATA_BYTES_MISSING`.
CREATE INDEX IF NOT EXISTS idx_hybrid_v2_execution_requests_calldata_bytes_null
    ON hybrid_v2_execution_requests (canonical_execution_id)
    WHERE calldata_bytes IS NULL;

-- Immutability trigger — once `calldata_bytes` is set to a non-NULL
-- value, subsequent UPDATEs may not change it. Mirrors the posture of
-- the existing plan_hash / calldata_hash immutability triggers.
CREATE OR REPLACE FUNCTION hybrid_v2_execution_requests_calldata_bytes_immutability_trigger()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    IF OLD.calldata_bytes IS NOT NULL
       AND NEW.calldata_bytes IS DISTINCT FROM OLD.calldata_bytes THEN
        RAISE EXCEPTION
            'hybrid_v2_execution_requests.calldata_bytes is immutable once set (canonical_execution_id=%)',
            OLD.canonical_execution_id;
    END IF;
    RETURN NEW;
END;
$$;

DROP TRIGGER IF EXISTS hybrid_v2_execution_requests_calldata_bytes_immutability
    ON hybrid_v2_execution_requests;

CREATE TRIGGER hybrid_v2_execution_requests_calldata_bytes_immutability
    BEFORE UPDATE ON hybrid_v2_execution_requests
    FOR EACH ROW EXECUTE FUNCTION
        hybrid_v2_execution_requests_calldata_bytes_immutability_trigger();
