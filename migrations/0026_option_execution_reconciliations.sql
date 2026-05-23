CREATE TABLE IF NOT EXISTS option_execution_reconciliations (
    id UUID PRIMARY KEY,
    intent_id UUID NOT NULL,
    onchain_intent_id TEXT NOT NULL,
    option_execution_transaction_id TEXT NOT NULL,
    tx_hash TEXT NOT NULL,
    chain_id BIGINT NOT NULL,
    status TEXT NOT NULL,
    strict BOOLEAN NOT NULL,
    requires_events BOOLEAN NOT NULL,
    trade_executed_event_id UUID NULL,
    margin_trade_event_id UUID NULL,
    trading_fee_event_count BIGINT NOT NULL,
    internal_transfer_event_count BIGINT NOT NULL,
    decoded_event_count BIGINT NOT NULL,
    mismatch_reason TEXT NULL,
    missing_required TEXT NULL,
    details JSONB NOT NULL,
    reconciled_at_ms BIGINT NOT NULL,
    created_at_ms BIGINT NOT NULL,
    updated_at_ms BIGINT NOT NULL,
    UNIQUE (option_execution_transaction_id)
);

CREATE INDEX IF NOT EXISTS idx_option_execution_reconciliations_intent_id
    ON option_execution_reconciliations(intent_id);
CREATE INDEX IF NOT EXISTS idx_option_execution_reconciliations_onchain_intent_id
    ON option_execution_reconciliations(onchain_intent_id);
CREATE INDEX IF NOT EXISTS idx_option_execution_reconciliations_tx_hash
    ON option_execution_reconciliations(tx_hash);
CREATE INDEX IF NOT EXISTS idx_option_execution_reconciliations_status
    ON option_execution_reconciliations(status);
