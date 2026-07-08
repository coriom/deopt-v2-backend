-- PERPS-SUBACCOUNTS-CORE-ROUTING-V1
--
-- Additive migration that threads `subaccount_id` through every Perps
-- persistence table so a wallet operating from Account 2 has fully
-- isolated positions / orders / fills / funding / liquidation history
-- from Account 1 (and every other subaccount).
--
-- Key model:
--   * Single-side entities (positions, orders, funding events,
--     liquidation events): `subaccount_id INT NOT NULL DEFAULT 1
--     CHECK (subaccount_id >= 1)`. Existing rows backfill to Account 1
--     via the DEFAULT.
--   * Two-side entities (fills): `taker_subaccount_id` +
--     `maker_subaccount_id`, both with the same shape.
--
-- Invariants preserved:
--   * Public Perps mutation routes remain fail-closed (`PerpsNotLive`)
--     — this migration does not enable any public trading path.
--   * `perp_positions` "one active per key" invariant is upgraded to
--     `(lower(account), subaccount_id, market_id) WHERE status='open'`
--     so Account 1 and Account 2 can each hold an open ETH-PERP
--     position simultaneously.
--   * `perp_orders` client_order_id idempotency is upgraded to
--     `(lower(account), subaccount_id, client_order_id)` — the same
--     `client_order_id` can be re-used per subaccount.
--
-- Privacy: NO signature, NO auth envelope, NO RPC URL, NO token.

-- ─────────────────────────────────────────────────────────────────
-- perp_positions
-- ─────────────────────────────────────────────────────────────────
ALTER TABLE perp_positions
    ADD COLUMN IF NOT EXISTS subaccount_id INTEGER NOT NULL DEFAULT 1
    CHECK (subaccount_id >= 1);

-- Replace the "one open per (account, market)" unique index with the
-- subaccount-aware key. Drop the old index first so we don't retain a
-- stale predicate.
DROP INDEX IF EXISTS idx_perp_positions_active;

CREATE UNIQUE INDEX IF NOT EXISTS idx_perp_positions_active
    ON perp_positions (lower(account), subaccount_id, market_id)
    WHERE status = 'open';

CREATE INDEX IF NOT EXISTS idx_perp_positions_account_subaccount_time
    ON perp_positions (lower(account), subaccount_id, updated_at_ms DESC);

-- ─────────────────────────────────────────────────────────────────
-- perp_orders
-- ─────────────────────────────────────────────────────────────────
ALTER TABLE perp_orders
    ADD COLUMN IF NOT EXISTS subaccount_id INTEGER NOT NULL DEFAULT 1
    CHECK (subaccount_id >= 1);

-- Replace client_order_id idempotency index with a subaccount-aware
-- variant. Same `client_order_id` may now recur per subaccount.
DROP INDEX IF EXISTS idx_perp_orders_client_order_id;

CREATE UNIQUE INDEX IF NOT EXISTS idx_perp_orders_client_order_id
    ON perp_orders (lower(account), subaccount_id, client_order_id)
    WHERE client_order_id IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_perp_orders_account_subaccount_time
    ON perp_orders (lower(account), subaccount_id, created_at_ms DESC);

CREATE INDEX IF NOT EXISTS idx_perp_orders_account_subaccount_market_status
    ON perp_orders (lower(account), subaccount_id, market_id, status);

-- ─────────────────────────────────────────────────────────────────
-- perp_fills  (two-sided: taker + maker)
-- ─────────────────────────────────────────────────────────────────
ALTER TABLE perp_fills
    ADD COLUMN IF NOT EXISTS taker_subaccount_id INTEGER NOT NULL DEFAULT 1
    CHECK (taker_subaccount_id >= 1);

ALTER TABLE perp_fills
    ADD COLUMN IF NOT EXISTS maker_subaccount_id INTEGER NOT NULL DEFAULT 1
    CHECK (maker_subaccount_id >= 1);

CREATE INDEX IF NOT EXISTS idx_perp_fills_taker_subaccount_time
    ON perp_fills (lower(taker_account), taker_subaccount_id, created_at_ms DESC);

CREATE INDEX IF NOT EXISTS idx_perp_fills_maker_subaccount_time
    ON perp_fills (lower(maker_account), maker_subaccount_id, created_at_ms DESC);

-- ─────────────────────────────────────────────────────────────────
-- perp_liquidation_events
-- ─────────────────────────────────────────────────────────────────
ALTER TABLE perp_liquidation_events
    ADD COLUMN IF NOT EXISTS subaccount_id INTEGER NOT NULL DEFAULT 1
    CHECK (subaccount_id >= 1);

CREATE INDEX IF NOT EXISTS idx_perp_liquidation_events_account_subaccount_time
    ON perp_liquidation_events (lower(account), subaccount_id, created_at_ms DESC);

-- ─────────────────────────────────────────────────────────────────
-- perp_funding_events
-- ─────────────────────────────────────────────────────────────────
ALTER TABLE perp_funding_events
    ADD COLUMN IF NOT EXISTS subaccount_id INTEGER NOT NULL DEFAULT 1
    CHECK (subaccount_id >= 1);

CREATE INDEX IF NOT EXISTS idx_perp_funding_events_account_subaccount_time
    ON perp_funding_events (lower(account), subaccount_id, created_at_ms DESC);
