CREATE TABLE IF NOT EXISTS rfqs (
    rfq_id TEXT PRIMARY KEY,
    taker TEXT NOT NULL,
    market_id BIGINT NOT NULL,
    side TEXT NOT NULL,
    size_1e8 TEXT NOT NULL,
    limit_price_1e8 TEXT NULL,
    status TEXT NOT NULL,
    created_at_ms BIGINT NOT NULL,
    expires_at_ms BIGINT NOT NULL,
    accepted_quote_id TEXT NULL,
    execution_intent_id TEXT NULL REFERENCES execution_intents(intent_id)
);

CREATE INDEX IF NOT EXISTS idx_rfqs_status ON rfqs(status);
CREATE INDEX IF NOT EXISTS idx_rfqs_taker ON rfqs(lower(taker));
CREATE INDEX IF NOT EXISTS idx_rfqs_market_id ON rfqs(market_id);

CREATE TABLE IF NOT EXISTS rfq_quotes (
    quote_id TEXT PRIMARY KEY,
    rfq_id TEXT NOT NULL REFERENCES rfqs(rfq_id),
    mm_account TEXT NOT NULL,
    session_id TEXT NULL,
    client_quote_id TEXT NULL,
    price_1e8 TEXT NOT NULL,
    size_1e8 TEXT NOT NULL,
    status TEXT NOT NULL,
    created_at_ms BIGINT NOT NULL,
    expires_at_ms BIGINT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_rfq_quotes_rfq_id ON rfq_quotes(rfq_id);
CREATE INDEX IF NOT EXISTS idx_rfq_quotes_mm ON rfq_quotes(lower(mm_account));
CREATE INDEX IF NOT EXISTS idx_rfq_quotes_status ON rfq_quotes(status);
CREATE UNIQUE INDEX IF NOT EXISTS idx_rfq_quotes_client_quote_id
    ON rfq_quotes(rfq_id, lower(mm_account), client_quote_id)
    WHERE client_quote_id IS NOT NULL;
