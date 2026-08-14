-- OPTIONS-HYBRID-V2-ECONOMIC-EXECUTION-CORE-V1 — Part D (canonical
-- Options execution identity).
--
-- Additive schema for the canonical Options order hash and canonical
-- Hybrid V2 execution identity per fill. Both columns are NULL for
-- pre-migration rows and for legacy inserts; new inserts on the
-- canonical path populate them. Both columns are immutable once set.
--
-- Frozen safety notes:
--   * `canonical_order_hash` binds every execution-relevant field of
--     an Options order (owner + subaccount + series + side + price +
--     size + TIF + post-only + nonce + deadline + deployment/chain).
--     Two subaccounts of the same owner never collide; cross-chain
--     replay never collides; identical replay maps to the same hash.
--   * `canonical_execution_id` binds a matched Options fill to the
--     existing Hybrid V2 canonical execution identity space via
--     `derive_canonical_execution_id(deployment_id, chain_id,
--     buyer_order_hash, seller_order_hash, size_1e8)`. Same economic
--     fill → same execution id; different fill quantity → different
--     execution id.
--   * Uniqueness is enforced by a sparse index over non-NULL values
--     so back-filled legacy rows do not collide with the sparse
--     canonical-path inserts landing during rollout.
--   * Immutability triggers mirror the posture of
--     `hybrid_v2_execution_requests` (`calldata_bytes`, `plan_hash`,
--     `calldata_hash`): once set, subsequent UPDATEs cannot change
--     these values.
--   * No new grant / role change.

ALTER TABLE option_orders
    ADD COLUMN IF NOT EXISTS canonical_order_hash TEXT NULL;

ALTER TABLE option_fills
    ADD COLUMN IF NOT EXISTS canonical_execution_id TEXT NULL;

-- Sparse uniqueness — only enforced on rows that have populated the
-- canonical hash. Pre-existing legacy rows remain NULL and are
-- ignored by this index. Same posture as the sparse null indexes on
-- `hybrid_v2_execution_requests.calldata_bytes`.
CREATE UNIQUE INDEX IF NOT EXISTS ux_option_orders_canonical_order_hash
    ON option_orders (canonical_order_hash)
    WHERE canonical_order_hash IS NOT NULL;

CREATE UNIQUE INDEX IF NOT EXISTS ux_option_fills_canonical_execution_id
    ON option_fills (canonical_execution_id)
    WHERE canonical_execution_id IS NOT NULL;

-- Read index — supports lookups from the reconciliation worker or
-- admin surfaces that resolve an on-chain execution back to the
-- persisted fill row.
CREATE INDEX IF NOT EXISTS idx_option_orders_canonical_order_hash_lookup
    ON option_orders (canonical_order_hash)
    WHERE canonical_order_hash IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_option_fills_canonical_execution_id_lookup
    ON option_fills (canonical_execution_id)
    WHERE canonical_execution_id IS NOT NULL;

-- Immutability trigger: `option_orders.canonical_order_hash` cannot
-- change once set. Same posture as `hybrid_v2_execution_requests`
-- `calldata_bytes` / `plan_hash` / `calldata_hash`.
CREATE OR REPLACE FUNCTION option_orders_canonical_order_hash_immutability_trigger()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    IF OLD.canonical_order_hash IS NOT NULL
       AND NEW.canonical_order_hash IS DISTINCT FROM OLD.canonical_order_hash THEN
        RAISE EXCEPTION
            'option_orders.canonical_order_hash is immutable once set (order_id=%)',
            OLD.order_id;
    END IF;
    RETURN NEW;
END;
$$;

DROP TRIGGER IF EXISTS option_orders_canonical_order_hash_immutability
    ON option_orders;

CREATE TRIGGER option_orders_canonical_order_hash_immutability
    BEFORE UPDATE ON option_orders
    FOR EACH ROW EXECUTE FUNCTION
        option_orders_canonical_order_hash_immutability_trigger();

-- Immutability trigger: `option_fills.canonical_execution_id` cannot
-- change once set. Fills are logically immutable; this guards against
-- an accidental UPDATE overwriting the canonical execution binding.
CREATE OR REPLACE FUNCTION option_fills_canonical_execution_id_immutability_trigger()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    IF OLD.canonical_execution_id IS NOT NULL
       AND NEW.canonical_execution_id IS DISTINCT FROM OLD.canonical_execution_id THEN
        RAISE EXCEPTION
            'option_fills.canonical_execution_id is immutable once set (fill_id=%)',
            OLD.fill_id;
    END IF;
    RETURN NEW;
END;
$$;

DROP TRIGGER IF EXISTS option_fills_canonical_execution_id_immutability
    ON option_fills;

CREATE TRIGGER option_fills_canonical_execution_id_immutability
    BEFORE UPDATE ON option_fills
    FOR EACH ROW EXECUTE FUNCTION
        option_fills_canonical_execution_id_immutability_trigger();
