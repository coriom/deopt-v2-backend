ALTER TABLE execution_intents
    ADD COLUMN IF NOT EXISTS onchain_intent_id TEXT;

CREATE INDEX IF NOT EXISTS idx_execution_intents_onchain_intent_id
    ON execution_intents(onchain_intent_id);

CREATE TABLE IF NOT EXISTS execution_reconciliations (
    reconciliation_id TEXT PRIMARY KEY,
    onchain_intent_id TEXT NOT NULL,
    intent_id TEXT NOT NULL REFERENCES execution_intents(intent_id) ON DELETE CASCADE,
    indexed_event_id TEXT NOT NULL REFERENCES indexed_perp_trades(event_id) ON DELETE CASCADE,
    tx_hash TEXT NOT NULL,
    block_number BIGINT NOT NULL,
    log_index BIGINT NOT NULL,
    status TEXT NOT NULL,
    created_at_ms BIGINT NOT NULL,
    UNIQUE(intent_id, indexed_event_id)
);

CREATE INDEX IF NOT EXISTS idx_execution_reconciliations_onchain_intent_id
    ON execution_reconciliations(onchain_intent_id);

CREATE INDEX IF NOT EXISTS idx_execution_reconciliations_intent_id
    ON execution_reconciliations(intent_id);

CREATE INDEX IF NOT EXISTS idx_execution_reconciliations_status
    ON execution_reconciliations(status);

CREATE INDEX IF NOT EXISTS idx_execution_reconciliations_tx_hash
    ON execution_reconciliations(tx_hash);
