-- OPTIONS-HYBRID-V2-EXECUTION-CORRELATION-CLOSURE-V1 Part D — link
-- user-signed execution intents to canonical Options fill identity.
--
-- Adds `option_execution_intents.canonical_execution_id` so every
-- new user-signed execution intent references exactly one canonical
-- fill via its backend-derived economic identity. Historical rows
-- remain NULL.
--
-- Frozen safety notes:
--   * `canonical_execution_id` binds the intent to the backend
--     canonical economic identity — NOT to a user signature. The
--     intent still carries its own EIP-712 signatures for the
--     `OptionTrade` payload; those remain the authorization proof
--     that the two counterparties agreed to the specific trade
--     economics.
--   * Uniqueness is enforced by a sparse index over non-NULL values
--     so back-filled legacy rows do not collide with sparse
--     canonical-path inserts landing during rollout. One canonical
--     execution id points to at most one active intent — this
--     prevents a second intent from spawning off the same fill.
--   * Immutability trigger mirrors the posture of migration 0053:
--     once a canonical_execution_id is set, subsequent UPDATEs may
--     not change it. Intents are logically immutable once signed;
--     this guards against an accidental mutation invalidating the
--     correlation chain.
--   * No new grant / role change — the column inherits table ACL.

ALTER TABLE option_execution_intents
    ADD COLUMN IF NOT EXISTS canonical_execution_id TEXT NULL;

CREATE UNIQUE INDEX IF NOT EXISTS ux_option_execution_intents_canonical_execution_id
    ON option_execution_intents (canonical_execution_id)
    WHERE canonical_execution_id IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_option_execution_intents_canonical_execution_id_lookup
    ON option_execution_intents (canonical_execution_id)
    WHERE canonical_execution_id IS NOT NULL;

CREATE OR REPLACE FUNCTION option_execution_intents_canonical_execution_id_immutability_trigger()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    IF OLD.canonical_execution_id IS NOT NULL
       AND NEW.canonical_execution_id IS DISTINCT FROM OLD.canonical_execution_id THEN
        RAISE EXCEPTION
            'option_execution_intents.canonical_execution_id is immutable once set (intent_id=%)',
            OLD.intent_id;
    END IF;
    RETURN NEW;
END;
$$;

DROP TRIGGER IF EXISTS option_execution_intents_canonical_execution_id_immutability
    ON option_execution_intents;

CREATE TRIGGER option_execution_intents_canonical_execution_id_immutability
    BEFORE UPDATE ON option_execution_intents
    FOR EACH ROW EXECUTE FUNCTION
        option_execution_intents_canonical_execution_id_immutability_trigger();
