-- PERPS-ISOLATED-MARGIN-POSITION-ENGINE-V1
--
-- Persist Perps isolated-margin positions. V1 policy:
--   * one position per (account, market_id) with status='open'
--     enforced by a partial unique index;
--   * closed positions persist so future history/lifecycle can list
--     them, but no fresh operations can reference them;
--   * no funding accrual is stored here (that lands in
--     `PERPS-FUNDING-V1`);
--   * no on-chain collateral movement is implied — this is a
--     backend-internal ledger, and all Perps public mutation routes
--     remain fail-closed at handler entry (`PerpsNotLive`).
--
-- Scale conventions (matches project + contract conventions):
--   * size_1e8: base-asset contract size, `1e8`.
--   * entry_price_1e8: quote-per-base, `1e8`.
--   * margin_1e8: quote-asset (mUSDC) collateral locked, `1e8`.
--     mUSDC has 6 decimals on-chain; the backend normalises to `1e8`
--     for internal math consistency with every other money field.
--   * realized_pnl_1e8: quote-asset PnL realised on reduce/close.
--
-- Privacy: NO signature, NO raw auth envelope, NO RPC URL, NO token,
-- NO header. The row captures only the accounting facts.

CREATE TABLE IF NOT EXISTS perp_positions (
    id                       UUID    PRIMARY KEY,
    account                  TEXT    NOT NULL,
    market_id                TEXT    NOT NULL,
    side                     TEXT    NOT NULL,      -- 'long' | 'short'
    size_1e8                 TEXT    NOT NULL,      -- remaining position size
    entry_price_1e8          TEXT    NOT NULL,      -- weighted-average entry
    margin_1e8               TEXT    NOT NULL,      -- isolated collateral locked
    realized_pnl_1e8         TEXT    NOT NULL DEFAULT '0',
    status                   TEXT    NOT NULL,      -- 'open' | 'closed'
    opened_at_ms             BIGINT  NOT NULL,
    updated_at_ms            BIGINT  NOT NULL,
    closed_at_ms             BIGINT  NULL,
    version                  BIGINT  NOT NULL DEFAULT 1
);

-- Enforce one active position per (account, market_id). Closed rows
-- can coexist historically because the predicate filters them out.
CREATE UNIQUE INDEX IF NOT EXISTS idx_perp_positions_active
    ON perp_positions (lower(account), market_id)
    WHERE status = 'open';

CREATE INDEX IF NOT EXISTS idx_perp_positions_account_time
    ON perp_positions (lower(account), updated_at_ms DESC);

CREATE INDEX IF NOT EXISTS idx_perp_positions_market_time
    ON perp_positions (market_id, updated_at_ms DESC);
