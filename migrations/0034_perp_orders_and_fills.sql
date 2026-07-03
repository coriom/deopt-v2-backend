-- PERPS-ORDER-EXECUTION-INTERNAL-V1
--
-- Persist Perps orders + fills. **Internal** execution surface only —
-- there is NO public HTTP mutation route that reaches this table in
-- V1. All six Perps mutation routes still return 503 PerpsNotLive at
-- handler entry; the internal `submit_perp_order_internal` /
-- `cancel_perp_order_internal` service functions are called only by
-- unit tests and future public order paths.
--
-- Privacy: NO signature, NO raw authorization envelope, NO RPC URL,
-- NO tokens, NO headers, NO private keys. The rows carry only the
-- order + matching bookkeeping the trader would see in their history.

CREATE TABLE IF NOT EXISTS perp_orders (
    id                        UUID    PRIMARY KEY,
    account                   TEXT    NOT NULL,
    market_id                 TEXT    NOT NULL,
    side                      TEXT    NOT NULL,      -- 'buy' | 'sell'
    order_type                TEXT    NOT NULL,      -- 'limit'
    price_1e8                 TEXT    NOT NULL,
    size_1e8                  TEXT    NOT NULL,      -- original size
    remaining_size_1e8        TEXT    NOT NULL,
    filled_size_1e8           TEXT    NOT NULL DEFAULT '0',
    time_in_force             TEXT    NOT NULL,      -- 'gtc' | 'ioc' | 'fok'
    post_only                 BOOLEAN NOT NULL DEFAULT FALSE,
    reduce_only               BOOLEAN NOT NULL DEFAULT FALSE,
    isolated_margin_1e8       TEXT    NOT NULL,      -- posted at submit for opens/increases
    status                    TEXT    NOT NULL,      -- 'open' | 'partially_filled' | 'filled' | 'cancelled' | 'rejected'
    client_order_id           TEXT    NULL,
    terminal_reason_code      TEXT    NULL,
    terminal_reason_message   TEXT    NULL,
    terminal_reason_source    TEXT    NULL,
    created_at_ms             BIGINT  NOT NULL,
    updated_at_ms             BIGINT  NOT NULL
);

-- Hot orderbook lookup: open/partially_filled orders sorted by
-- price then time.
CREATE INDEX IF NOT EXISTS idx_perp_orders_book
    ON perp_orders (market_id, status, price_1e8, created_at_ms);

-- Account listing.
CREATE INDEX IF NOT EXISTS idx_perp_orders_account_time
    ON perp_orders (lower(account), created_at_ms DESC);

-- Client-order idempotency per account (unique when present).
CREATE UNIQUE INDEX IF NOT EXISTS idx_perp_orders_client_order_id
    ON perp_orders (lower(account), client_order_id)
    WHERE client_order_id IS NOT NULL;

CREATE TABLE IF NOT EXISTS perp_fills (
    id                        UUID    PRIMARY KEY,
    market_id                 TEXT    NOT NULL,
    taker_order_id            UUID    NOT NULL REFERENCES perp_orders(id),
    maker_order_id            UUID    NOT NULL REFERENCES perp_orders(id),
    taker_account             TEXT    NOT NULL,
    maker_account             TEXT    NOT NULL,
    taker_side                TEXT    NOT NULL,      -- 'buy' | 'sell'
    price_1e8                 TEXT    NOT NULL,      -- fill price = maker's resting price
    size_1e8                  TEXT    NOT NULL,
    created_at_ms             BIGINT  NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_perp_fills_market_time
    ON perp_fills (market_id, created_at_ms DESC);

CREATE INDEX IF NOT EXISTS idx_perp_fills_taker
    ON perp_fills (lower(taker_account), created_at_ms DESC);

CREATE INDEX IF NOT EXISTS idx_perp_fills_maker
    ON perp_fills (lower(maker_account), created_at_ms DESC);
