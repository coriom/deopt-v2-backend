CREATE TABLE IF NOT EXISTS option_execution_transactions (
    transaction_id TEXT PRIMARY KEY,
    intent_id TEXT NOT NULL REFERENCES option_execution_intents(intent_id) ON DELETE CASCADE,
    onchain_intent_id TEXT,
    sender TEXT NOT NULL,
    target TEXT NOT NULL,
    calldata TEXT NOT NULL,
    value_wei TEXT NOT NULL,
    gas_limit BIGINT NULL,
    tx_hash TEXT,
    status TEXT NOT NULL,
    error TEXT,
    created_at_ms BIGINT NOT NULL,
    updated_at_ms BIGINT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_option_execution_transactions_intent_id
    ON option_execution_transactions(intent_id);
CREATE INDEX IF NOT EXISTS idx_option_execution_transactions_onchain_intent_id
    ON option_execution_transactions(onchain_intent_id);
CREATE INDEX IF NOT EXISTS idx_option_execution_transactions_tx_hash
    ON option_execution_transactions(tx_hash);
CREATE INDEX IF NOT EXISTS idx_option_execution_transactions_status
    ON option_execution_transactions(status);
CREATE UNIQUE INDEX IF NOT EXISTS idx_option_execution_transactions_one_submitted
    ON option_execution_transactions(intent_id)
    WHERE status = 'submitted';
