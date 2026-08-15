-- OPTIONS-HYBRID-V2-RESERVATIONS-PENDING-SETTLEMENT-AND-CANONICAL-RELEASE-V1
-- Package B — off-chain Options risk reservation ledger.
--
-- This table is DELIBERATELY SEPARATE from `hybrid_v2_reservations`.
-- The latter is a canonical on-chain projection populated exclusively
-- from Solidity `TokenReserved` events (see migration 0044) and would
-- be corrupted by off-chain writes.
--
-- Purpose: track two kinds of off-chain risk holds for the Options
-- product:
--
--   * `OPEN_ORDER` — collateral reserved when a resting order is
--     accepted; released on cancellation, expiry, or transitioned to
--     `PENDING_SETTLEMENT` on match.
--
--   * `PENDING_SETTLEMENT` — collateral reserved from the moment a
--     match commits until the canonical Options execution event lands
--     and settles the exposure. Bound to `canonical_execution_id` so
--     the reducer's `Promoted` outcome can release exactly the right
--     rows.
--
-- Scoping (matches the frozen Options accounting identity):
--   `(deployment_id, chain_id, owner, subaccount_id, collateral_token)`
--
-- No cross-subaccount netting. No cross-token netting.
--
-- Lifecycle:
--   ACTIVE            (default; contributes to available-collateral deduction)
--   CONVERTED         (OPEN_ORDER row: matched quantity migrated to a
--                      PENDING_SETTLEMENT row inside the same tx)
--   RELEASED          (order cancelled / expired / IOC/FOK unmatched
--                      residual / post-only crossed reject)
--   SETTLED           (canonical event correlated + settled the
--                      pending exposure)
--   MANUAL_REVIEW     (operator-only escalation — reorg that cannot
--                      safely re-establish the hold, etc.)
--
-- ACTIVE uniqueness invariant:
--   at most one ACTIVE row per
--   `(purpose, owner, subaccount_id, collateral_token, source_identity)`
-- where `source_identity` is:
--   * `canonical_order_hash` for OPEN_ORDER
--   * `canonical_execution_id` for PENDING_SETTLEMENT
--
-- Terminal rows (CONVERTED / RELEASED / SETTLED / MANUAL_REVIEW)
-- remain visible for audit. Sparse UNIQUE indexes preserve
-- at-most-one-ACTIVE without blocking historical rows.

CREATE TABLE IF NOT EXISTS option_reservations (
    reservation_id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    -- Purpose discriminator
    purpose TEXT NOT NULL
        CHECK (purpose IN ('OPEN_ORDER', 'PENDING_SETTLEMENT')),
    -- Scope: deployment + chain + owner + subaccount + token
    deployment_id BIGINT NOT NULL,
    chain_id BIGINT NOT NULL,
    owner TEXT NOT NULL,
    subaccount_id INTEGER NOT NULL,
    collateral_token TEXT NOT NULL,
    -- Source identity
    canonical_order_hash TEXT NULL,
    canonical_execution_id TEXT NULL,
    -- Option economic context
    option_series_id TEXT NOT NULL,
    side TEXT NOT NULL CHECK (side IN ('buy', 'sell')),
    -- Reserved amount, stringified fixed-point in the token's own
    -- decimals. `quantity_1e8` is the option-side quantity that
    -- produced this reservation (informational; the authoritative
    -- economic weight is `reserved_amount`).
    reserved_amount TEXT NOT NULL,
    quantity_1e8 TEXT NOT NULL,
    -- Lifecycle
    status TEXT NOT NULL DEFAULT 'ACTIVE'
        CHECK (status IN ('ACTIVE', 'CONVERTED', 'RELEASED', 'SETTLED', 'MANUAL_REVIEW')),
    terminal_reason TEXT NULL,
    created_at_ms BIGINT NOT NULL,
    updated_at_ms BIGINT NOT NULL,
    -- Purpose-specific integrity: OPEN_ORDER must carry
    -- canonical_order_hash; PENDING_SETTLEMENT must carry
    -- canonical_execution_id. Enforce at insert.
    CHECK (
        (purpose = 'OPEN_ORDER'
            AND canonical_order_hash IS NOT NULL
            AND canonical_execution_id IS NULL)
        OR
        (purpose = 'PENDING_SETTLEMENT'
            AND canonical_execution_id IS NOT NULL
            AND canonical_order_hash IS NULL)
    )
);

-- Sparse UNIQUE: at most one ACTIVE OPEN_ORDER row per canonical order
CREATE UNIQUE INDEX IF NOT EXISTS ux_option_reservations_active_open_order
    ON option_reservations (canonical_order_hash)
    WHERE status = 'ACTIVE' AND purpose = 'OPEN_ORDER';

-- Sparse UNIQUE: at most one ACTIVE PENDING_SETTLEMENT per
-- (canonical_execution_id, owner, subaccount_id, collateral_token).
-- The tuple form (not just canonical_execution_id alone) supports
-- the buyer-and-seller-share-execution-id case where both sides
-- need distinct reservation rows tied to the same execution.
CREATE UNIQUE INDEX IF NOT EXISTS ux_option_reservations_active_pending_settlement
    ON option_reservations (canonical_execution_id, owner, subaccount_id, collateral_token)
    WHERE status = 'ACTIVE' AND purpose = 'PENDING_SETTLEMENT';

-- Lookup indexes for the available-collateral read path.
CREATE INDEX IF NOT EXISTS idx_option_reservations_available_lookup
    ON option_reservations (deployment_id, owner, subaccount_id, collateral_token, status);

-- Lookup index for cancellation / settlement release: locate a
-- specific OPEN_ORDER by its canonical_order_hash.
CREATE INDEX IF NOT EXISTS idx_option_reservations_by_order_hash
    ON option_reservations (canonical_order_hash)
    WHERE canonical_order_hash IS NOT NULL;

-- Lookup index for canonical event settlement release.
CREATE INDEX IF NOT EXISTS idx_option_reservations_by_execution_id
    ON option_reservations (canonical_execution_id)
    WHERE canonical_execution_id IS NOT NULL;

-- Immutability trigger: canonical_order_hash, canonical_execution_id,
-- owner, subaccount_id, collateral_token, deployment_id, chain_id,
-- purpose and reserved_amount cannot change after insert. Status
-- follows the state machine (enforced by CHECK).
CREATE OR REPLACE FUNCTION option_reservations_immutability_trigger()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    IF NEW.reservation_id IS DISTINCT FROM OLD.reservation_id THEN
        RAISE EXCEPTION 'option_reservations.reservation_id is immutable';
    END IF;
    IF NEW.purpose IS DISTINCT FROM OLD.purpose THEN
        RAISE EXCEPTION 'option_reservations.purpose is immutable (reservation_id=%)',
            OLD.reservation_id;
    END IF;
    IF NEW.deployment_id IS DISTINCT FROM OLD.deployment_id THEN
        RAISE EXCEPTION 'option_reservations.deployment_id is immutable (reservation_id=%)',
            OLD.reservation_id;
    END IF;
    IF NEW.chain_id IS DISTINCT FROM OLD.chain_id THEN
        RAISE EXCEPTION 'option_reservations.chain_id is immutable (reservation_id=%)',
            OLD.reservation_id;
    END IF;
    IF NEW.owner IS DISTINCT FROM OLD.owner THEN
        RAISE EXCEPTION 'option_reservations.owner is immutable (reservation_id=%)',
            OLD.reservation_id;
    END IF;
    IF NEW.subaccount_id IS DISTINCT FROM OLD.subaccount_id THEN
        RAISE EXCEPTION 'option_reservations.subaccount_id is immutable (reservation_id=%)',
            OLD.reservation_id;
    END IF;
    IF NEW.collateral_token IS DISTINCT FROM OLD.collateral_token THEN
        RAISE EXCEPTION 'option_reservations.collateral_token is immutable (reservation_id=%)',
            OLD.reservation_id;
    END IF;
    IF NEW.canonical_order_hash IS DISTINCT FROM OLD.canonical_order_hash THEN
        RAISE EXCEPTION 'option_reservations.canonical_order_hash is immutable (reservation_id=%)',
            OLD.reservation_id;
    END IF;
    IF NEW.canonical_execution_id IS DISTINCT FROM OLD.canonical_execution_id THEN
        RAISE EXCEPTION 'option_reservations.canonical_execution_id is immutable (reservation_id=%)',
            OLD.reservation_id;
    END IF;
    IF NEW.reserved_amount IS DISTINCT FROM OLD.reserved_amount THEN
        RAISE EXCEPTION 'option_reservations.reserved_amount is immutable (reservation_id=%)',
            OLD.reservation_id;
    END IF;
    RETURN NEW;
END;
$$;

DROP TRIGGER IF EXISTS option_reservations_immutability ON option_reservations;

CREATE TRIGGER option_reservations_immutability
    BEFORE UPDATE ON option_reservations
    FOR EACH ROW EXECUTE FUNCTION option_reservations_immutability_trigger();
