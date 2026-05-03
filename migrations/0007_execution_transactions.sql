CREATE TABLE IF NOT EXISTS execution_transactions (
    transaction_id TEXT PRIMARY KEY,
    intent_id TEXT NOT NULL REFERENCES execution_intents(intent_id) ON DELETE CASCADE,
    onchain_intent_id TEXT,
    target TEXT NOT NULL,
    calldata TEXT NOT NULL,
    value_wei TEXT NOT NULL,
    tx_hash TEXT,
    status TEXT NOT NULL,
    error TEXT,
    created_at_ms BIGINT NOT NULL,
    updated_at_ms BIGINT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_execution_transactions_intent_id
    ON execution_transactions(intent_id);
CREATE INDEX IF NOT EXISTS idx_execution_transactions_onchain_intent_id
    ON execution_transactions(onchain_intent_id);
CREATE INDEX IF NOT EXISTS idx_execution_transactions_tx_hash
    ON execution_transactions(tx_hash);
CREATE INDEX IF NOT EXISTS idx_execution_transactions_status
    ON execution_transactions(status);
