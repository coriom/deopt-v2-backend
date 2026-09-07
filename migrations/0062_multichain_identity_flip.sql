-- DEOPT_MULTICHAIN_SCHEMA_HARDENING_AND_MULTICOLLATERAL_ACTIVATION_DESIGN_V1
-- Part A — flip legacy uniqueness constraints on the two tables where
-- `chain_id` was added additively in migration 0061 so they no longer
-- block cross-chain coexistence.
--
-- Rationale:
--   Migration 0061 was intentionally additive — it added a `chain_id`
--   column (defaulted + backfilled to 84532) and a composite unique
--   index alongside the historical constraint. That closed the write
--   path against future-mistake collisions but the *legacy* constraint
--   still prevented the same `tx_hash+log_index` on two different
--   chains, and the legacy `PRIMARY KEY (name)` on `indexer_cursors`
--   prevented the same cursor name on two different chains.
--
--   This migration replaces those legacy constraints with their
--   chain-scoped equivalents. It is safe because:
--     (a) Every existing row was backfilled to chain_id = 84532 by
--         migration 0061, so no row is ambiguous under the new PK.
--     (b) The V1 runtime writes exactly one chain (Base Sepolia) so no
--         current writer needs updating for correctness — only the
--         `ON CONFLICT (...)` column list in the repository layer has
--         to name the new column set, which ships in the same commit
--         as this migration.
--     (c) The redundant composite unique index created by 0061 is
--         dropped after the new PRIMARY KEY subsumes it, keeping the
--         final schema minimal.
--
-- Rollback posture:
--   In the event a rollback is required, migration 0061 already
--   preserves both the `chain_id` column AND the historical unique
--   index (until this migration drops it), so recovering to the pre-
--   flip state is a matter of recreating the legacy constraints and
--   redeploying the previous Rust binary. That rollback path is
--   documented in `docs/DEOPT_MULTICHAIN_SCHEMA_HARDENING_V1.md`.

BEGIN;

-- --------------------------------------------------------------
-- indexer_cursors — promote (chain_id, name) to PRIMARY KEY.
-- --------------------------------------------------------------

-- Drop the redundant unique index added by 0061; the new PK below
-- provides the same uniqueness.
DROP INDEX IF EXISTS indexer_cursors_chain_id_name_unique;

ALTER TABLE indexer_cursors
    DROP CONSTRAINT indexer_cursors_pkey;

ALTER TABLE indexer_cursors
    ADD CONSTRAINT indexer_cursors_pkey PRIMARY KEY (chain_id, name);

-- --------------------------------------------------------------
-- indexed_perp_trades — drop legacy uniqueness on (tx_hash, log_index).
-- --------------------------------------------------------------

-- The old UNIQUE(tx_hash, log_index) constraint from migration 0004
-- must be dropped so the same tx_hash on a future chain can coexist
-- with the current 84532 rows. The chain-scoped equivalent from 0061
-- (`indexed_perp_trades_chain_id_tx_hash_log_index_unique`) is
-- retained as the sole cross-chain-safe uniqueness guarantee.
--
-- The named auto-index on the constraint is 0004's `indexed_perp_trades_tx_hash_log_index_key`.
-- Dropping it via the constraint is the safe form because it also
-- removes the underlying index atomically.

DO $$
DECLARE
    constraint_name TEXT;
BEGIN
    SELECT conname INTO constraint_name
    FROM pg_constraint
    WHERE conrelid = 'indexed_perp_trades'::regclass
      AND contype = 'u'
      AND pg_get_constraintdef(oid) LIKE 'UNIQUE (tx_hash, log_index)%';

    IF constraint_name IS NOT NULL THEN
        EXECUTE format('ALTER TABLE indexed_perp_trades DROP CONSTRAINT %I', constraint_name);
    END IF;
END $$;

COMMIT;
