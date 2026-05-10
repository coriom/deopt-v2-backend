CREATE TABLE IF NOT EXISTS option_fills (
    fill_id TEXT PRIMARY KEY,
    option_series_id TEXT NOT NULL REFERENCES option_series(option_series_id),
    buy_order_id TEXT NOT NULL REFERENCES option_orders(order_id),
    sell_order_id TEXT NOT NULL REFERENCES option_orders(order_id),
    buyer TEXT NOT NULL,
    seller TEXT NOT NULL,
    maker_order_id TEXT NOT NULL REFERENCES option_orders(order_id),
    taker_order_id TEXT NOT NULL REFERENCES option_orders(order_id),
    taker_side TEXT NOT NULL,
    price_1e8 TEXT NOT NULL,
    size_1e8 TEXT NOT NULL,
    created_at_ms BIGINT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_option_fills_series ON option_fills(option_series_id);
CREATE INDEX IF NOT EXISTS idx_option_fills_buy_order ON option_fills(buy_order_id);
CREATE INDEX IF NOT EXISTS idx_option_fills_sell_order ON option_fills(sell_order_id);
CREATE INDEX IF NOT EXISTS idx_option_fills_buyer ON option_fills(lower(buyer));
CREATE INDEX IF NOT EXISTS idx_option_fills_seller ON option_fills(lower(seller));
CREATE INDEX IF NOT EXISTS idx_option_fills_created_at ON option_fills(created_at_ms);

DROP INDEX IF EXISTS idx_option_orders_open_account_client_id;
CREATE UNIQUE INDEX IF NOT EXISTS idx_option_orders_live_account_client_id
    ON option_orders(lower(account), client_order_id)
    WHERE client_order_id IS NOT NULL AND status IN ('open', 'partially_filled');
