-- PERPS-FUNDING-V1
--
-- Persist Perps funding settlements. **Internal-only** — public Perps
-- mutation routes still return 503 `PerpsNotLive` at handler entry.
-- The funding tick is admin-gated (POST /admin/perps/funding/tick).
--
-- Semantics (isolated margin, cumulative-funding-index model, 1e18 scale
-- for the on-chain cumulative rate, 1e8 for internal quote-denominated
-- values):
--
--     payment_1e8 = side_sign * size_1e8 * (index_after_1e18 - index_before_1e18) / 1e18
--
-- where side_sign = +1 for long, -1 for short. A positive `payment_1e8`
-- means the trader owes funding (reduces `margin_1e8`); a negative
-- `payment_1e8` means the trader receives funding (increases
-- `margin_1e8`). When margin would go negative, it saturates to 0 and
-- the residual is booked as `bad_debt_1e8` on the event.
--
-- The `funding_index_before/after` columns are stored as TEXT to hold
-- signed 1e18 integers (i128) without truncation.
--
-- Privacy: NO signature, NO auth envelope, NO RPC URL, NO tokens.

-- Additive fields on perp_positions.
-- `last_funding_index_1e18` — the signed cumulative funding rate this
--   position most recently settled against, 1e18 scale, in TEXT to
--   allow the full i128 range.
-- `cumulative_funding_payment_1e8` — the signed sum of every funding
--   payment applied to this position, 1e8 scale, in TEXT.
ALTER TABLE perp_positions
    ADD COLUMN IF NOT EXISTS last_funding_index_1e18 TEXT NOT NULL DEFAULT '0',
    ADD COLUMN IF NOT EXISTS cumulative_funding_payment_1e8 TEXT NOT NULL DEFAULT '0';

CREATE TABLE IF NOT EXISTS perp_funding_events (
    id                              UUID    PRIMARY KEY,
    account                         TEXT    NOT NULL,
    market_id                       TEXT    NOT NULL,
    position_id                     UUID    NOT NULL REFERENCES perp_positions(id),
    side                            TEXT    NOT NULL,     -- 'long' | 'short'
    position_size_1e8               TEXT    NOT NULL,     -- signed size at settlement
    funding_index_before_1e18       TEXT    NOT NULL,     -- signed
    funding_index_after_1e18        TEXT    NOT NULL,     -- signed
    funding_delta_1e18              TEXT    NOT NULL,     -- signed
    payment_1e8                     TEXT    NOT NULL,     -- signed: positive = trader owes
    margin_before_1e8               TEXT    NOT NULL,     -- unsigned
    margin_after_1e8                TEXT    NOT NULL,     -- unsigned (clamped to 0)
    bad_debt_1e8                    TEXT    NOT NULL DEFAULT '0', -- >0 if payment would drive margin negative
    reason_code                     TEXT    NOT NULL,     -- 'funding_settlement' | 'funding_source_unavailable'
    created_at_ms                   BIGINT  NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_perp_funding_events_account_time
    ON perp_funding_events (lower(account), created_at_ms DESC);

CREATE INDEX IF NOT EXISTS idx_perp_funding_events_market_time
    ON perp_funding_events (market_id, created_at_ms DESC);

CREATE INDEX IF NOT EXISTS idx_perp_funding_events_position_time
    ON perp_funding_events (position_id, created_at_ms DESC);
