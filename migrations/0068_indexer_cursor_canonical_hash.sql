-- The legacy PERP indexer must bind its durable cursor to a canonical block.
-- Existing unbound cursors intentionally remain NULL and require reconciliation
-- before the indexer may advance again.
ALTER TABLE indexer_cursors ADD COLUMN IF NOT EXISTS last_indexed_block_hash TEXT;
