CREATE TABLE IF NOT EXISTS mm_accounts (
    mm_account TEXT PRIMARY KEY,
    enabled BOOLEAN NOT NULL,
    label TEXT NULL,
    can_submit_perp_orders BOOLEAN NOT NULL DEFAULT FALSE,
    can_quote_perp_rfq BOOLEAN NOT NULL DEFAULT FALSE,
    can_quote_option_rfq BOOLEAN NOT NULL DEFAULT FALSE,
    can_submit_option_orders BOOLEAN NOT NULL DEFAULT FALSE,
    created_at_ms BIGINT NOT NULL,
    updated_at_ms BIGINT NOT NULL
);

CREATE TABLE IF NOT EXISTS mm_market_permissions (
    id TEXT PRIMARY KEY,
    mm_account TEXT NOT NULL REFERENCES mm_accounts(mm_account) ON DELETE CASCADE,
    market_id BIGINT NULL,
    option_series_id TEXT NULL,
    enabled BOOLEAN NOT NULL,
    created_at_ms BIGINT NOT NULL,
    updated_at_ms BIGINT NOT NULL,
    CHECK (market_id IS NULL OR market_id >= 0),
    CHECK (market_id IS NULL OR option_series_id IS NULL)
);

CREATE INDEX IF NOT EXISTS idx_mm_accounts_enabled ON mm_accounts(enabled);
CREATE INDEX IF NOT EXISTS idx_mm_market_permissions_account
    ON mm_market_permissions(lower(mm_account));
CREATE INDEX IF NOT EXISTS idx_mm_market_permissions_market
    ON mm_market_permissions(market_id);
CREATE INDEX IF NOT EXISTS idx_mm_market_permissions_option_series
    ON mm_market_permissions(option_series_id);
