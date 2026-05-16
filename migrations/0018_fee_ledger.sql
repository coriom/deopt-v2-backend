CREATE TABLE IF NOT EXISTS fee_events (
    fee_event_id TEXT PRIMARY KEY,
    source_type TEXT NOT NULL,
    source_id TEXT NOT NULL,
    market_type TEXT NOT NULL,
    flow_type TEXT NOT NULL,
    market_id BIGINT,
    option_series_id TEXT,
    maker TEXT,
    taker TEXT,
    payer TEXT NOT NULL,
    recipient TEXT NOT NULL,
    fee_asset TEXT NOT NULL,
    notional_1e8 TEXT NOT NULL,
    fee_rate_micro_bps BIGINT NOT NULL,
    fee_amount_1e8 TEXT NOT NULL,
    rebate_rate_micro_bps BIGINT NOT NULL,
    rebate_amount_1e8 TEXT NOT NULL,
    protocol_amount_1e8 TEXT NOT NULL,
    status TEXT NOT NULL,
    created_at_ms BIGINT NOT NULL,
    UNIQUE (source_type, source_id, payer, recipient)
);

CREATE INDEX IF NOT EXISTS idx_fee_events_created_at_ms
    ON fee_events (created_at_ms DESC);
CREATE INDEX IF NOT EXISTS idx_fee_events_payer_created_at_ms
    ON fee_events (lower(payer), created_at_ms DESC);
CREATE INDEX IF NOT EXISTS idx_fee_events_source
    ON fee_events (source_type, source_id);
CREATE INDEX IF NOT EXISTS idx_fee_events_option_series
    ON fee_events (option_series_id)
    WHERE option_series_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_fee_events_market
    ON fee_events (market_id)
    WHERE market_id IS NOT NULL;

CREATE TABLE IF NOT EXISTS volume_buckets (
    bucket_id TEXT PRIMARY KEY,
    account TEXT NOT NULL,
    bucket_day TEXT NOT NULL,
    market_type TEXT NOT NULL,
    maker_volume_1e8 TEXT NOT NULL,
    taker_volume_1e8 TEXT NOT NULL,
    total_volume_1e8 TEXT NOT NULL,
    updated_at_ms BIGINT NOT NULL,
    UNIQUE (account, bucket_day, market_type)
);

CREATE INDEX IF NOT EXISTS idx_volume_buckets_account_day
    ON volume_buckets (lower(account), bucket_day DESC);
CREATE INDEX IF NOT EXISTS idx_volume_buckets_day_market
    ON volume_buckets (bucket_day DESC, market_type);

CREATE TABLE IF NOT EXISTS rebate_accruals (
    rebate_id TEXT PRIMARY KEY,
    fee_event_id TEXT NOT NULL REFERENCES fee_events (fee_event_id),
    account TEXT NOT NULL,
    source_type TEXT NOT NULL,
    source_id TEXT NOT NULL,
    rebate_asset TEXT NOT NULL,
    rebate_amount_1e8 TEXT NOT NULL,
    status TEXT NOT NULL,
    created_at_ms BIGINT NOT NULL,
    UNIQUE (fee_event_id, account)
);

CREATE INDEX IF NOT EXISTS idx_rebate_accruals_account_created_at_ms
    ON rebate_accruals (lower(account), created_at_ms DESC);
CREATE INDEX IF NOT EXISTS idx_rebate_accruals_source
    ON rebate_accruals (source_type, source_id);
