ALTER TABLE indexed_perp_trades
    ADD COLUMN IF NOT EXISTS onchain_intent_id TEXT;

CREATE INDEX IF NOT EXISTS idx_indexed_perp_trades_onchain_intent_id
    ON indexed_perp_trades(onchain_intent_id);
