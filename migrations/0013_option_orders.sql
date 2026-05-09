CREATE TABLE IF NOT EXISTS option_orders (
    order_id TEXT PRIMARY KEY,
    option_series_id TEXT NOT NULL REFERENCES option_series(option_series_id),
    account TEXT NOT NULL,
    side TEXT NOT NULL,
    price_1e8 TEXT NOT NULL,
    size_1e8 TEXT NOT NULL,
    remaining_size_1e8 TEXT NOT NULL,
    time_in_force TEXT NOT NULL,
    client_order_id TEXT NULL,
    nonce TEXT NULL,
    deadline_ms BIGINT NULL,
    signature TEXT NULL,
    status TEXT NOT NULL,
    created_at_ms BIGINT NOT NULL,
    updated_at_ms BIGINT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_option_orders_series ON option_orders(option_series_id);
CREATE INDEX IF NOT EXISTS idx_option_orders_account ON option_orders(lower(account));
CREATE INDEX IF NOT EXISTS idx_option_orders_status ON option_orders(status);
CREATE INDEX IF NOT EXISTS idx_option_orders_side ON option_orders(side);
CREATE INDEX IF NOT EXISTS idx_option_orders_price ON option_orders(price_1e8);
CREATE UNIQUE INDEX IF NOT EXISTS idx_option_orders_open_account_client_id
    ON option_orders(lower(account), client_order_id)
    WHERE client_order_id IS NOT NULL AND status = 'open';
