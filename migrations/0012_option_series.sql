CREATE TABLE IF NOT EXISTS option_series (
    option_series_id TEXT PRIMARY KEY,
    underlying TEXT NOT NULL,
    base_asset TEXT NOT NULL,
    quote_asset TEXT NOT NULL,
    settlement_asset TEXT NOT NULL,
    expiry BIGINT NOT NULL,
    strike_1e8 TEXT NOT NULL,
    is_call BOOLEAN NOT NULL,
    contract_size_1e8 TEXT NOT NULL,
    status TEXT NOT NULL,
    source TEXT NOT NULL,
    onchain_product_id TEXT NULL,
    onchain_series_id TEXT NULL,
    created_at_ms BIGINT NOT NULL,
    updated_at_ms BIGINT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_option_series_status ON option_series(status);
CREATE INDEX IF NOT EXISTS idx_option_series_expiry ON option_series(expiry);
CREATE INDEX IF NOT EXISTS idx_option_series_underlying ON option_series(lower(underlying));
CREATE INDEX IF NOT EXISTS idx_option_series_strike ON option_series(strike_1e8);
CREATE INDEX IF NOT EXISTS idx_option_series_callput ON option_series(is_call);
