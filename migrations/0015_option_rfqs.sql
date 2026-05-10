CREATE TABLE IF NOT EXISTS option_rfqs (
    option_rfq_id TEXT PRIMARY KEY,
    taker TEXT NOT NULL,
    option_series_id TEXT NOT NULL REFERENCES option_series(option_series_id),
    side TEXT NOT NULL,
    size_1e8 TEXT NOT NULL,
    limit_price_1e8 TEXT NULL,
    status TEXT NOT NULL,
    created_at_ms BIGINT NOT NULL,
    expires_at_ms BIGINT NOT NULL,
    accepted_quote_id TEXT NULL,
    option_fill_id TEXT NULL
);

CREATE TABLE IF NOT EXISTS option_rfq_quotes (
    quote_id TEXT PRIMARY KEY,
    option_rfq_id TEXT NOT NULL REFERENCES option_rfqs(option_rfq_id),
    mm_account TEXT NOT NULL,
    session_id TEXT NULL,
    client_quote_id TEXT NULL,
    price_1e8 TEXT NOT NULL,
    size_1e8 TEXT NOT NULL,
    status TEXT NOT NULL,
    created_at_ms BIGINT NOT NULL,
    expires_at_ms BIGINT NOT NULL
);

CREATE TABLE IF NOT EXISTS option_rfq_fills (
    fill_id TEXT PRIMARY KEY,
    option_rfq_id TEXT NOT NULL REFERENCES option_rfqs(option_rfq_id),
    quote_id TEXT NOT NULL REFERENCES option_rfq_quotes(quote_id),
    option_series_id TEXT NOT NULL REFERENCES option_series(option_series_id),
    buyer TEXT NOT NULL,
    seller TEXT NOT NULL,
    taker TEXT NOT NULL,
    mm_account TEXT NOT NULL,
    taker_side TEXT NOT NULL,
    price_1e8 TEXT NOT NULL,
    size_1e8 TEXT NOT NULL,
    created_at_ms BIGINT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_option_rfqs_series ON option_rfqs(option_series_id);
CREATE INDEX IF NOT EXISTS idx_option_rfqs_taker ON option_rfqs(lower(taker));
CREATE INDEX IF NOT EXISTS idx_option_rfqs_status ON option_rfqs(status);
CREATE INDEX IF NOT EXISTS idx_option_rfqs_created_at ON option_rfqs(created_at_ms);
CREATE INDEX IF NOT EXISTS idx_option_rfqs_expires_at ON option_rfqs(expires_at_ms);

CREATE INDEX IF NOT EXISTS idx_option_rfq_quotes_rfq ON option_rfq_quotes(option_rfq_id);
CREATE INDEX IF NOT EXISTS idx_option_rfq_quotes_mm ON option_rfq_quotes(lower(mm_account));
CREATE INDEX IF NOT EXISTS idx_option_rfq_quotes_status ON option_rfq_quotes(status);
CREATE INDEX IF NOT EXISTS idx_option_rfq_quotes_created_at ON option_rfq_quotes(created_at_ms);
CREATE UNIQUE INDEX IF NOT EXISTS idx_option_rfq_quotes_client_id
    ON option_rfq_quotes(option_rfq_id, lower(mm_account), client_quote_id)
    WHERE client_quote_id IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_option_rfq_fills_rfq ON option_rfq_fills(option_rfq_id);
CREATE INDEX IF NOT EXISTS idx_option_rfq_fills_quote ON option_rfq_fills(quote_id);
CREATE INDEX IF NOT EXISTS idx_option_rfq_fills_series ON option_rfq_fills(option_series_id);
CREATE INDEX IF NOT EXISTS idx_option_rfq_fills_buyer ON option_rfq_fills(lower(buyer));
CREATE INDEX IF NOT EXISTS idx_option_rfq_fills_seller ON option_rfq_fills(lower(seller));
CREATE INDEX IF NOT EXISTS idx_option_rfq_fills_created_at ON option_rfq_fills(created_at_ms);
