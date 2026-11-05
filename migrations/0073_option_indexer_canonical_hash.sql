-- The Options cursor must anchor to a canonical block, including empty blocks.
ALTER TABLE option_event_indexer_state
    ADD COLUMN IF NOT EXISTS last_indexed_block_hash TEXT;
