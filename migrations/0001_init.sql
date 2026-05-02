CREATE TABLE IF NOT EXISTS used_nonces (
    account TEXT NOT NULL,
    nonce BIGINT NOT NULL,
    created_at_ms BIGINT NOT NULL,
    PRIMARY KEY (account, nonce)
);

CREATE TABLE IF NOT EXISTS orders (
    order_id TEXT PRIMARY KEY,
    market_id BIGINT NOT NULL,
    account TEXT NOT NULL,
    side TEXT NOT NULL,
    order_type TEXT NOT NULL,
    time_in_force TEXT NOT NULL,
    price_1e8 TEXT NOT NULL,
    size_1e8 TEXT NOT NULL,
    remaining_size_1e8 TEXT NOT NULL,
    reduce_only BOOLEAN NOT NULL,
    post_only BOOLEAN NOT NULL,
    client_order_id TEXT NOT NULL,
    nonce BIGINT NOT NULL,
    deadline_ms BIGINT NOT NULL,
    signature TEXT NOT NULL,
    status TEXT NOT NULL,
    created_at_ms BIGINT NOT NULL,
    updated_at_ms BIGINT NOT NULL,
    UNIQUE (account, nonce)
);

CREATE INDEX IF NOT EXISTS idx_orders_account ON orders (account);
CREATE INDEX IF NOT EXISTS idx_orders_market_id ON orders (market_id);
CREATE INDEX IF NOT EXISTS idx_orders_status ON orders (status);

CREATE TABLE IF NOT EXISTS trades (
    trade_id TEXT PRIMARY KEY,
    market_id BIGINT NOT NULL,
    maker_order_id TEXT NOT NULL,
    taker_order_id TEXT NOT NULL,
    maker_account TEXT NOT NULL,
    taker_account TEXT NOT NULL,
    price_1e8 TEXT NOT NULL,
    size_1e8 TEXT NOT NULL,
    buyer TEXT NOT NULL,
    seller TEXT NOT NULL,
    created_at_ms BIGINT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_trades_market_id ON trades (market_id);
CREATE INDEX IF NOT EXISTS idx_trades_maker_account ON trades (maker_account);
CREATE INDEX IF NOT EXISTS idx_trades_taker_account ON trades (taker_account);
CREATE INDEX IF NOT EXISTS idx_trades_buyer ON trades (buyer);
CREATE INDEX IF NOT EXISTS idx_trades_seller ON trades (seller);

CREATE TABLE IF NOT EXISTS execution_intents (
    intent_id TEXT PRIMARY KEY,
    market_id BIGINT NOT NULL,
    buyer TEXT NOT NULL,
    seller TEXT NOT NULL,
    price_1e8 TEXT NOT NULL,
    size_1e8 TEXT NOT NULL,
    buy_order_id TEXT NOT NULL,
    sell_order_id TEXT NOT NULL,
    status TEXT NOT NULL,
    created_at_ms BIGINT NOT NULL,
    updated_at_ms BIGINT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_execution_intents_status ON execution_intents (status);
CREATE INDEX IF NOT EXISTS idx_execution_intents_market_id ON execution_intents (market_id);
CREATE INDEX IF NOT EXISTS idx_execution_intents_buyer ON execution_intents (buyer);
CREATE INDEX IF NOT EXISTS idx_execution_intents_seller ON execution_intents (seller);

CREATE TABLE IF NOT EXISTS engine_events (
    event_id TEXT PRIMARY KEY,
    event_type TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    created_at_ms BIGINT NOT NULL
);
