CREATE TABLE IF NOT EXISTS option_execution_intents (
    intent_id TEXT PRIMARY KEY,
    onchain_intent_id TEXT NOT NULL UNIQUE,
    source_type TEXT NOT NULL,
    source_id TEXT NOT NULL,
    option_series_id TEXT NOT NULL REFERENCES option_series(option_series_id),
    onchain_option_id TEXT NOT NULL,
    buyer TEXT NOT NULL,
    seller TEXT NOT NULL,
    underlying TEXT NOT NULL,
    settlement_asset TEXT NOT NULL,
    expiry BIGINT NOT NULL,
    strike_1e8 TEXT NOT NULL,
    is_call BOOLEAN NOT NULL,
    contract_size_1e8 TEXT NOT NULL,
    quantity_contracts TEXT NOT NULL,
    source_size_1e8 TEXT NOT NULL,
    source_price_1e8 TEXT NOT NULL,
    premium_per_contract_native TEXT NOT NULL,
    buyer_is_maker BOOLEAN NOT NULL,
    buyer_nonce TEXT NULL,
    seller_nonce TEXT NULL,
    deadline BIGINT NOT NULL,
    buyer_signature TEXT NULL,
    seller_signature TEXT NULL,
    calldata TEXT NULL,
    status TEXT NOT NULL,
    error TEXT NULL,
    created_at_ms BIGINT NOT NULL,
    updated_at_ms BIGINT NOT NULL,
    UNIQUE(source_type, source_id)
);

CREATE INDEX IF NOT EXISTS idx_option_execution_intents_status
    ON option_execution_intents(status);
CREATE INDEX IF NOT EXISTS idx_option_execution_intents_source
    ON option_execution_intents(source_type, source_id);
CREATE INDEX IF NOT EXISTS idx_option_execution_intents_series
    ON option_execution_intents(option_series_id);
CREATE INDEX IF NOT EXISTS idx_option_execution_intents_buyer
    ON option_execution_intents(lower(buyer));
CREATE INDEX IF NOT EXISTS idx_option_execution_intents_seller
    ON option_execution_intents(lower(seller));
CREATE INDEX IF NOT EXISTS idx_option_execution_intents_created_at
    ON option_execution_intents(created_at_ms);
