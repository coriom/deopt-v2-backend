-- PERPS-LIQUIDATION-AND-RISK-V1
--
-- Persist Perps liquidation events. **Internal-only** — public Perps
-- mutation routes still return 503 `PerpsNotLive` at handler entry.
-- Liquidations are computed and applied by the internal service (in
-- V1: in-memory execution flow + admin-gated tick endpoint); a future
-- milestone will wire the events insert into the PG execution path
-- via `insert_perp_liquidation_event_tx`.
--
-- Note: `perp_positions.status` accepts the new `'liquidated'` value
-- without a schema change (TEXT column with application-level enum;
-- the parse fn is extended to recognise it in the position engine).
--
-- Privacy: NO signature, NO auth envelope, NO RPC URL, NO tokens.

CREATE TABLE IF NOT EXISTS perp_liquidation_events (
    id                                       UUID    PRIMARY KEY,
    account                                  TEXT    NOT NULL,
    market_id                                TEXT    NOT NULL,
    position_id                              UUID    NOT NULL REFERENCES perp_positions(id),
    side                                     TEXT    NOT NULL,     -- 'long' | 'short'
    size_1e8                                 TEXT    NOT NULL,     -- liquidated size
    entry_price_1e8                          TEXT    NOT NULL,     -- WAP at liquidation time
    mark_price_1e8                           TEXT    NOT NULL,     -- price used for the close
    margin_1e8                               TEXT    NOT NULL,     -- pre-liquidation isolated margin
    unrealized_pnl_1e8                       TEXT    NOT NULL,     -- signed
    equity_1e8                               TEXT    NOT NULL,     -- margin + uPnL (signed)
    maintenance_margin_requirement_1e8       TEXT    NOT NULL,
    margin_ratio_bps                         TEXT    NOT NULL,     -- clamped to 0 if negative
    realized_pnl_1e8                         TEXT    NOT NULL,     -- pnl booked by the close
    bad_debt_1e8                             TEXT    NOT NULL DEFAULT '0', -- >0 when equity is negative post-close
    liquidation_fee_1e8                      TEXT    NOT NULL DEFAULT '0', -- V1: always 0 (see docs)
    status                                   TEXT    NOT NULL,     -- 'completed' | 'price_unavailable'
    reason_code                              TEXT    NOT NULL,     -- 'margin_breach' | 'price_unavailable'
    created_at_ms                            BIGINT  NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_perp_liquidation_events_account_time
    ON perp_liquidation_events (lower(account), created_at_ms DESC);

CREATE INDEX IF NOT EXISTS idx_perp_liquidation_events_market_time
    ON perp_liquidation_events (market_id, created_at_ms DESC);

CREATE INDEX IF NOT EXISTS idx_perp_liquidation_events_position
    ON perp_liquidation_events (position_id);
