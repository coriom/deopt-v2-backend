-- ATTACHED-TP-SL-ON-ENTRY-V1
--
-- Persist the trader's TP/SL attachment spec for a parent option
-- order so it survives between submit (no fill yet) and the first
-- fill batch where the conditional-orders are actually created.
--
-- The active reduce-only conditional-order rows still live in
-- `options_conditional_orders`; this table is a separate "intent"
-- table that the fill-time materializer reads + flips through its
-- own status lifecycle (pending → active | cancelled | failed).
--
-- Why a separate table (instead of reusing options_conditional_orders
-- with status=pre_armed):
--   * `create_conditional_orders` requires a non-zero reducible
--     position; arming legs against a zero-fill parent would either
--     break that precondition or need a much larger refactor of
--     the worker's quantity-check path.
--   * The attachment plan and the materialized conditional rows
--     have different lifecycles: the plan is a one-shot spec, the
--     rows are persistent legs the user can cancel independently.
--
-- Privacy: NO signature, NO raw authorization envelope, NO
-- request headers, NO RPC URL, NO token. The plan captures only
-- prices + the parent order id + the trader-meaningful failure
-- code/message.

CREATE TABLE IF NOT EXISTS option_order_attachment_plans (
    plan_id                              UUID    PRIMARY KEY,
    parent_order_id                      TEXT    NOT NULL REFERENCES option_orders(order_id),
    account                              TEXT    NOT NULL,
    option_series_id                     TEXT    NOT NULL REFERENCES option_series(option_series_id),
    -- Take-profit leg (NULL if not requested).
    tp_trigger_price_1e8                 TEXT    NULL,
    tp_limit_price_1e8                   TEXT    NULL,
    -- Stop-loss leg (NULL if not requested).
    sl_trigger_price_1e8                 TEXT    NULL,
    sl_limit_price_1e8                   TEXT    NULL,
    -- When both TP and SL are present this controls whether they
    -- materialize as an OCO pair (worker cancels the loser when
    -- the winner triggers). Single-leg plans ignore this flag.
    link_as_oco                          BOOLEAN NOT NULL DEFAULT FALSE,
    -- Optional inherited expiry for the materialized conditional
    -- rows; NULL means "use the conditional service defaults".
    expires_at_ms                        BIGINT  NULL,
    -- Status lifecycle:
    --   'pending'   — plan recorded, no materialization yet
    --                 (parent has no fills yet OR not enough
    --                 reducible position).
    --   'active'    — materialization succeeded; the conditional
    --                 rows are now live in options_conditional_orders.
    --   'cancelled' — parent order was cancelled / expired with no
    --                 fill, OR user explicitly cancelled the plan
    --                 before materialization.
    --   'failed'    — materialization attempted but the conditional
    --                 service returned an error (see failure_code +
    --                 failure_message). Parent order state is
    --                 unaffected; plan is terminal.
    status                               TEXT    NOT NULL,
    -- The size at materialization. The trader's filled exposure
    -- when the legs were created. V1 policy: materialize once on
    -- the first fill batch where reducible position exists; later
    -- fills do not extend the conditional rows.
    materialized_size_1e8                TEXT    NULL,
    tp_conditional_order_id              UUID    NULL REFERENCES options_conditional_orders(id),
    sl_conditional_order_id              UUID    NULL REFERENCES options_conditional_orders(id),
    oco_group_id                         UUID    NULL,
    failure_code                         TEXT    NULL,
    failure_message                      TEXT    NULL,
    created_at_ms                        BIGINT  NOT NULL,
    updated_at_ms                        BIGINT  NOT NULL
);

-- Look-up by parent order id is the hot path: every fill batch
-- the service finishes scans the involved order ids for a pending
-- plan. UNIQUE because each parent order has at most one plan.
CREATE UNIQUE INDEX IF NOT EXISTS idx_option_order_attachment_plans_parent
    ON option_order_attachment_plans (parent_order_id);

-- Tester-facing listing by account; mirrors the option_orders /
-- option_order_rejections account index style.
CREATE INDEX IF NOT EXISTS idx_option_order_attachment_plans_account_time
    ON option_order_attachment_plans (lower(account), created_at_ms DESC);
