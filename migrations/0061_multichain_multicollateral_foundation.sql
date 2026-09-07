-- DEOPT_MULTICHAIN_MULTICOLLATERAL_FOUNDATION_V1 — safe additive
-- migration that closes the two canonical-identity collision risks in
-- the legacy tables (`indexer_cursors`, `indexed_perp_trades`) so a
-- future multi-chain deployment cannot silently overwrite state that
-- belongs to a different chain.
--
-- Design rules:
--   * Additive only — no existing constraint is dropped so nothing
--     that currently reads or writes these tables can break.
--   * Backfill uses Base Sepolia (84532) which is the only chain the
--     platform has ever indexed against.
--   * New composite unique constraints are added alongside the
--     originals so downstream code can migrate at its own pace.
--
-- Related in-code registry: `crate::config::chains::BASE_SEPOLIA` +
-- `assert_v1_single_chain_invariant`. Hybrid V2 tables are already
-- scoped by `deployment_id` (see migration 0044) so are left alone.

BEGIN;

-- --------------------------------------------------------------
-- indexer_cursors — global cursor becomes per-chain-scoped cursor.
-- --------------------------------------------------------------

ALTER TABLE indexer_cursors
    ADD COLUMN IF NOT EXISTS chain_id BIGINT NOT NULL DEFAULT 84532;

-- The historical PRIMARY KEY on `name` remains in place. We add a
-- composite unique constraint on `(chain_id, name)` so a future
-- second-chain writer cannot collide with the single-chain writer
-- that owns the current `name` row.
DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_indexes
        WHERE schemaname = current_schema()
          AND indexname = 'indexer_cursors_chain_id_name_unique'
    ) THEN
        CREATE UNIQUE INDEX indexer_cursors_chain_id_name_unique
            ON indexer_cursors (chain_id, name);
    END IF;
END $$;

CREATE INDEX IF NOT EXISTS idx_indexer_cursors_chain_id
    ON indexer_cursors (chain_id);

-- --------------------------------------------------------------
-- indexed_perp_trades — canonical event identity gains chain_id.
-- --------------------------------------------------------------

ALTER TABLE indexed_perp_trades
    ADD COLUMN IF NOT EXISTS chain_id BIGINT NOT NULL DEFAULT 84532;

-- Existing `UNIQUE (tx_hash, log_index)` is preserved. A composite
-- `(chain_id, tx_hash, log_index)` index is added so a future reader
-- can enforce cross-chain-safe uniqueness without a schema break.
DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_indexes
        WHERE schemaname = current_schema()
          AND indexname = 'indexed_perp_trades_chain_id_tx_hash_log_index_unique'
    ) THEN
        CREATE UNIQUE INDEX indexed_perp_trades_chain_id_tx_hash_log_index_unique
            ON indexed_perp_trades (chain_id, tx_hash, log_index);
    END IF;
END $$;

CREATE INDEX IF NOT EXISTS idx_indexed_perp_trades_chain_id
    ON indexed_perp_trades (chain_id);

COMMIT;
