-- OPTIONS-TWAP-ORDERS-V1 — Options TWAP parent + child persistence.
--
-- One parent row per user-created TWAP; N child rows scheduled at a
-- deterministic interval computed as `running_time_ms / child_count`.
-- Children are materialised lazily by the admin tick as their
-- scheduled_at_ms comes due (see `list_due_option_twap_orders`), so
-- this table starts empty and grows as ticks fire — no fake rows.
--
-- Every mutation is off-chain; nothing here touches Solidity.
-- Perps TWAP is out of scope for V1 and remains fail-closed at the
-- route boundary.

CREATE TABLE IF NOT EXISTS option_twap_orders (
    option_twap_id TEXT PRIMARY KEY,
    account TEXT NOT NULL,
    option_series_id TEXT NOT NULL REFERENCES option_series(option_series_id),
    side TEXT NOT NULL,
    size_1e8 TEXT NOT NULL,
    remaining_size_1e8 TEXT NOT NULL,
    limit_price_1e8 TEXT NOT NULL,
    running_time_ms BIGINT NOT NULL,
    child_count INTEGER NOT NULL,
    child_interval_ms BIGINT NOT NULL,
    next_execution_at_ms BIGINT NOT NULL,
    started_at_ms BIGINT NOT NULL,
    ends_at_ms BIGINT NOT NULL,
    status TEXT NOT NULL,
    created_child_count INTEGER NOT NULL DEFAULT 0,
    filled_size_1e8 TEXT NOT NULL DEFAULT '0',
    failed_child_count INTEGER NOT NULL DEFAULT 0,
    client_order_id TEXT,
    last_error TEXT,
    created_at_ms BIGINT NOT NULL,
    updated_at_ms BIGINT NOT NULL
);

CREATE INDEX IF NOT EXISTS option_twap_orders_account_lower_idx
    ON option_twap_orders (LOWER(account));
CREATE INDEX IF NOT EXISTS option_twap_orders_status_idx
    ON option_twap_orders (status);
CREATE INDEX IF NOT EXISTS option_twap_orders_next_execution_at_ms_idx
    ON option_twap_orders (next_execution_at_ms);
CREATE INDEX IF NOT EXISTS option_twap_orders_series_id_idx
    ON option_twap_orders (option_series_id);

CREATE TABLE IF NOT EXISTS option_twap_child_orders (
    option_twap_child_id TEXT PRIMARY KEY,
    option_twap_id TEXT NOT NULL REFERENCES option_twap_orders(option_twap_id),
    sequence INTEGER NOT NULL,
    scheduled_at_ms BIGINT NOT NULL,
    submitted_at_ms BIGINT,
    child_order_id TEXT,
    status TEXT NOT NULL,
    requested_size_1e8 TEXT NOT NULL,
    filled_size_1e8 TEXT NOT NULL DEFAULT '0',
    limit_price_1e8 TEXT NOT NULL,
    error_message TEXT,
    UNIQUE (option_twap_id, sequence)
);

CREATE INDEX IF NOT EXISTS option_twap_child_orders_twap_id_idx
    ON option_twap_child_orders (option_twap_id);
CREATE INDEX IF NOT EXISTS option_twap_child_orders_child_order_id_idx
    ON option_twap_child_orders (child_order_id);
