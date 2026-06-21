-- OPTIONS-CONDITIONAL-ORDERS-TP-SL-V1
--
-- Server-side persistent TP/SL conditional exit orders bound to an
-- existing option position (account + series_id). Persistence is what
-- makes them survive a browser close — the worker evaluates `armed`
-- rows against the underlying oracle and atomically transitions one
-- row at a time.
--
-- An OCO pair is two rows sharing the same `oco_group_id`. Both rows
-- carry `reduce_only = TRUE`; the worker enforces reducible position
-- size again at trigger time.

CREATE TABLE IF NOT EXISTS options_conditional_orders (
    id                    UUID        PRIMARY KEY,
    account               TEXT        NOT NULL,
    option_series_id      TEXT        NOT NULL REFERENCES option_series(option_series_id),
    -- Stable derived position reference. Positions are reconstructed
    -- from option_fills, so we key on the (account, series_id) pair
    -- and store the side that was authoritative at arm time for
    -- audit. The worker re-derives reducible size at trigger time.
    position_side         TEXT        NOT NULL,            -- 'long' | 'short'
    option_kind           TEXT        NOT NULL,            -- 'call' | 'put'
    conditional_type      TEXT        NOT NULL,            -- 'take_profit' | 'stop_loss'
    trigger_source        TEXT        NOT NULL,            -- 'underlying_oracle'
    trigger_condition     TEXT        NOT NULL,            -- 'gte' | 'lte'
    trigger_price_1e8     TEXT        NOT NULL,            -- u128 as TEXT (project convention)
    quantity_1e8          TEXT        NOT NULL,            -- u128 as TEXT
    execution_type        TEXT        NOT NULL,            -- 'ioc_limit'
    limit_price_1e8       TEXT        NOT NULL,            -- u128 as TEXT
    reduce_only           BOOLEAN     NOT NULL DEFAULT TRUE,
    oco_group_id          UUID        NULL,
    status                TEXT        NOT NULL,            -- see ConditionalOrderStatus
    child_order_id        TEXT        NULL REFERENCES option_orders(order_id),
    failure_code          TEXT        NULL,
    failure_message       TEXT        NULL,
    expires_at_ms         BIGINT      NULL,
    triggered_at_ms       BIGINT      NULL,
    completed_at_ms       BIGINT      NULL,
    created_at_ms         BIGINT      NOT NULL,
    updated_at_ms         BIGINT      NOT NULL,
    -- `version` is an optimistic-lock counter incremented on every
    -- write. The worker uses it together with the status guard to
    -- prevent double-trigger after restart.
    version               BIGINT      NOT NULL DEFAULT 0
);

-- Active-trigger evaluation hot path: the worker queries armed rows
-- only.
CREATE INDEX IF NOT EXISTS idx_options_conditional_orders_armed
    ON options_conditional_orders(status)
    WHERE status = 'armed';

-- Per-account listing for `GET /accounts/{address}/conditional-orders`.
CREATE INDEX IF NOT EXISTS idx_options_conditional_orders_account
    ON options_conditional_orders(lower(account));

-- Per-position lookups (reduce-only checks, OCO grouping by series).
CREATE INDEX IF NOT EXISTS idx_options_conditional_orders_series
    ON options_conditional_orders(option_series_id);

-- OCO sibling lookups: cancel the partner inside the same critical
-- section as the trigger transition.
CREATE INDEX IF NOT EXISTS idx_options_conditional_orders_oco
    ON options_conditional_orders(oco_group_id)
    WHERE oco_group_id IS NOT NULL;
