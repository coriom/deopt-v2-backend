CREATE TABLE IF NOT EXISTS option_execution_events (
    id UUID PRIMARY KEY,
    chain_id BIGINT NOT NULL,
    contract_address TEXT NOT NULL,
    tx_hash TEXT NOT NULL,
    log_index BIGINT NOT NULL,
    block_number BIGINT NOT NULL,
    block_hash TEXT,
    event_name TEXT NOT NULL,
    event_signature TEXT NOT NULL,
    intent_id TEXT NULL,
    onchain_intent_id TEXT NULL,
    option_execution_transaction_id TEXT NULL,
    buyer TEXT NULL,
    seller TEXT NULL,
    account TEXT NULL,
    option_id TEXT NULL,
    quantity_contracts TEXT NULL,
    premium_per_contract_native TEXT NULL,
    raw_topics JSONB NOT NULL,
    raw_data TEXT NOT NULL,
    decoded JSONB NULL,
    created_at_ms BIGINT NOT NULL,
    updated_at_ms BIGINT NOT NULL,
    UNIQUE (chain_id, tx_hash, log_index)
);

CREATE INDEX IF NOT EXISTS idx_option_execution_events_block_number
    ON option_execution_events(block_number);
CREATE INDEX IF NOT EXISTS idx_option_execution_events_chain_tx_log
    ON option_execution_events(chain_id, tx_hash, log_index);
CREATE INDEX IF NOT EXISTS idx_option_execution_events_event_name
    ON option_execution_events(event_name);
CREATE INDEX IF NOT EXISTS idx_option_execution_events_intent_id
    ON option_execution_events(intent_id);
CREATE INDEX IF NOT EXISTS idx_option_execution_events_onchain_intent_id
    ON option_execution_events(onchain_intent_id);
CREATE INDEX IF NOT EXISTS idx_option_execution_events_transaction_id
    ON option_execution_events(option_execution_transaction_id);
CREATE INDEX IF NOT EXISTS idx_option_execution_events_tx_hash
    ON option_execution_events(tx_hash);

CREATE TABLE IF NOT EXISTS option_event_indexer_state (
    id TEXT PRIMARY KEY,
    last_indexed_block BIGINT NOT NULL,
    updated_at_ms BIGINT NOT NULL,
    last_error TEXT NULL
);
