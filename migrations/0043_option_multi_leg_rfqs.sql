-- RFQ-MULTI-LEG-SCHEMA-V1 — schema foundation for multi-leg atomic
-- Options RFQ. Additive only. No existing table is altered; the
-- single-leg RFQ tables (`option_rfqs`, `option_rfq_quotes`,
-- `option_rfq_fills`) stay byte-identical and every existing
-- INSERT/SELECT continues to compile.
--
-- Design source: `~/DEOPT/docs/SUBACCOUNTS_RFQ_MULTI_LEG_SCOPE_V1_RESULT.md`.
--
-- What this migration lands:
--
-- * `option_multi_leg_rfqs`             — parent RFQ (taker + subaccount + status + TTL).
-- * `option_multi_leg_rfq_legs`         — leg composition, one row per leg per RFQ.
-- * `option_multi_leg_rfq_quotes`       — maker quote parent (package price + status).
-- * `option_multi_leg_rfq_quote_legs`   — per-leg maker price.
-- * `option_multi_leg_rfq_fills`        — accepted-fill parent (both sides + subaccounts).
-- * `option_multi_leg_rfq_fill_legs`    — per-leg fill detail (series + side + size + price).
-- * Composite `(LOWER(address), subaccount_id)` indexes on every side-anchored access path.
-- * Subaccount discipline mirrored from `0040_rfq_subaccounts.sql`:
--       `NOT NULL DEFAULT 1 CHECK (>= 1)`.
-- * Leg-count invariant `leg_index >= 0` enforced at every leg table.
--   Upper bound (max 8 legs) is enforced at the repository layer;
--   Postgres does not have a straightforward per-parent cardinality
--   constraint without a trigger, and the repository is the single
--   entry point for INSERTs.
-- * FK cascades **DISABLED** so the accept-time transaction is the
--   authoritative source of any parent/leg lifecycle change. Legs
--   cannot be deleted without a caller-authored DELETE inside the
--   same transaction; this preserves audit trail.
--
-- What this migration does NOT land:
--
-- * Public HTTP routes (`/options/multi-leg-rfqs`) — deferred to
--   `RFQ-MULTI-LEG-CREATE-QUOTE-V1` + `_ATOMIC-ACCEPT-V1`.
-- * Rust service functions.
-- * v2 canonical builders for the 4 new `WriteAuthAction` variants.
-- * Lifecycle WS payload variants.
-- * MM Gateway multi-leg quote message.
-- * Frontend anything. The `RfqStrategyWorkspace.tsx:160` blocker
--   stays truthful and enabled.
-- * On-chain settlement (Solidity untouched).
--
-- Off-chain only. Nothing here touches mainnet, deployments, or the
-- Perps fail-closed posture. Feature-gated behind
-- `OPTION_RFQ_MULTI_LEG_ENABLED` at the config layer; this migration
-- runs regardless of the flag so the schema is always ready when the
-- flag flips.

-- ---------------------------------------------------------------------
-- option_multi_leg_rfqs — parent request.
-- ---------------------------------------------------------------------

CREATE TABLE IF NOT EXISTS option_multi_leg_rfqs (
    option_rfq_id       TEXT PRIMARY KEY,
    taker               TEXT NOT NULL,
    taker_subaccount_id INTEGER NOT NULL DEFAULT 1
        CHECK (taker_subaccount_id >= 1),
    status              TEXT NOT NULL,
    created_at_ms       BIGINT NOT NULL,
    expires_at_ms       BIGINT NOT NULL,
    accepted_quote_id   TEXT NULL,
    accepted_fill_id    TEXT NULL
);

CREATE INDEX IF NOT EXISTS idx_option_multi_leg_rfqs_taker
    ON option_multi_leg_rfqs (LOWER(taker));
CREATE INDEX IF NOT EXISTS idx_option_multi_leg_rfqs_status
    ON option_multi_leg_rfqs (status);
CREATE INDEX IF NOT EXISTS idx_option_multi_leg_rfqs_created_at
    ON option_multi_leg_rfqs (created_at_ms);
CREATE INDEX IF NOT EXISTS idx_option_multi_leg_rfqs_expires_at
    ON option_multi_leg_rfqs (expires_at_ms);
CREATE INDEX IF NOT EXISTS idx_option_multi_leg_rfqs_taker_subaccount
    ON option_multi_leg_rfqs (LOWER(taker), taker_subaccount_id);

-- ---------------------------------------------------------------------
-- option_multi_leg_rfq_legs — leg composition, one row per leg per RFQ.
-- Stable ordering guaranteed by the composite PK on
-- (option_rfq_id, leg_index).
-- ---------------------------------------------------------------------

CREATE TABLE IF NOT EXISTS option_multi_leg_rfq_legs (
    option_rfq_id     TEXT NOT NULL
        REFERENCES option_multi_leg_rfqs (option_rfq_id),
    leg_index         INTEGER NOT NULL CHECK (leg_index >= 0),
    option_series_id  TEXT NOT NULL
        REFERENCES option_series (option_series_id),
    side              TEXT NOT NULL,
    size_1e8          TEXT NOT NULL,
    ratio_num         INTEGER NOT NULL DEFAULT 1 CHECK (ratio_num >= 1),
    ratio_den         INTEGER NOT NULL DEFAULT 1 CHECK (ratio_den >= 1),
    PRIMARY KEY (option_rfq_id, leg_index)
);

CREATE INDEX IF NOT EXISTS idx_option_multi_leg_rfq_legs_series
    ON option_multi_leg_rfq_legs (option_series_id);

-- ---------------------------------------------------------------------
-- option_multi_leg_rfq_quotes — maker quote parent. `package_price_1e8`
-- is a signed net debit / credit expressed as a decimal-string 1e8
-- integer; `size_1e8` is the package multiplier applied to every leg's
-- ratio.
-- ---------------------------------------------------------------------

CREATE TABLE IF NOT EXISTS option_multi_leg_rfq_quotes (
    quote_id            TEXT PRIMARY KEY,
    option_rfq_id       TEXT NOT NULL
        REFERENCES option_multi_leg_rfqs (option_rfq_id),
    mm_account          TEXT NOT NULL,
    maker_subaccount_id INTEGER NOT NULL DEFAULT 1
        CHECK (maker_subaccount_id >= 1),
    session_id          TEXT NULL,
    client_quote_id     TEXT NULL,
    package_price_1e8   TEXT NOT NULL,
    size_1e8            TEXT NOT NULL,
    status              TEXT NOT NULL,
    created_at_ms       BIGINT NOT NULL,
    expires_at_ms       BIGINT NOT NULL,
    signature           TEXT NULL,
    quote_digest        TEXT NULL,
    quote_nonce         TEXT NULL,
    signature_status    TEXT NOT NULL,
    recovered_signer    TEXT NULL
);

CREATE INDEX IF NOT EXISTS idx_option_multi_leg_rfq_quotes_rfq
    ON option_multi_leg_rfq_quotes (option_rfq_id);
CREATE INDEX IF NOT EXISTS idx_option_multi_leg_rfq_quotes_mm
    ON option_multi_leg_rfq_quotes (LOWER(mm_account));
CREATE INDEX IF NOT EXISTS idx_option_multi_leg_rfq_quotes_status
    ON option_multi_leg_rfq_quotes (status);
CREATE INDEX IF NOT EXISTS idx_option_multi_leg_rfq_quotes_created_at
    ON option_multi_leg_rfq_quotes (created_at_ms);
CREATE INDEX IF NOT EXISTS idx_option_multi_leg_rfq_quotes_maker_subaccount
    ON option_multi_leg_rfq_quotes (LOWER(mm_account), maker_subaccount_id);
CREATE UNIQUE INDEX IF NOT EXISTS idx_option_multi_leg_rfq_quotes_client_id
    ON option_multi_leg_rfq_quotes (option_rfq_id, LOWER(mm_account), client_quote_id)
    WHERE client_quote_id IS NOT NULL;

-- ---------------------------------------------------------------------
-- option_multi_leg_rfq_quote_legs — per-leg price on the maker quote.
-- `leg_index` refers to the same index as
-- `option_multi_leg_rfq_legs.leg_index` for the parent RFQ (composed by
-- the service layer, verified at quote-submit time).
-- ---------------------------------------------------------------------

CREATE TABLE IF NOT EXISTS option_multi_leg_rfq_quote_legs (
    quote_id   TEXT NOT NULL
        REFERENCES option_multi_leg_rfq_quotes (quote_id),
    leg_index  INTEGER NOT NULL CHECK (leg_index >= 0),
    price_1e8  TEXT NOT NULL,
    PRIMARY KEY (quote_id, leg_index)
);

-- ---------------------------------------------------------------------
-- option_multi_leg_rfq_fills — parent accepted-fill row. One row per
-- successful accept. Both parties' subaccounts pinned here so downstream
-- fills feeds can filter side-aware without joining the RFQ or quote
-- tables.
-- ---------------------------------------------------------------------

CREATE TABLE IF NOT EXISTS option_multi_leg_rfq_fills (
    fill_id             TEXT PRIMARY KEY,
    option_rfq_id       TEXT NOT NULL
        REFERENCES option_multi_leg_rfqs (option_rfq_id),
    quote_id            TEXT NOT NULL
        REFERENCES option_multi_leg_rfq_quotes (quote_id),
    taker               TEXT NOT NULL,
    taker_subaccount_id INTEGER NOT NULL DEFAULT 1
        CHECK (taker_subaccount_id >= 1),
    mm_account          TEXT NOT NULL,
    maker_subaccount_id INTEGER NOT NULL DEFAULT 1
        CHECK (maker_subaccount_id >= 1),
    package_price_1e8   TEXT NOT NULL,
    size_1e8            TEXT NOT NULL,
    created_at_ms       BIGINT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_option_multi_leg_rfq_fills_rfq
    ON option_multi_leg_rfq_fills (option_rfq_id);
CREATE INDEX IF NOT EXISTS idx_option_multi_leg_rfq_fills_quote
    ON option_multi_leg_rfq_fills (quote_id);
CREATE INDEX IF NOT EXISTS idx_option_multi_leg_rfq_fills_created_at
    ON option_multi_leg_rfq_fills (created_at_ms);
CREATE INDEX IF NOT EXISTS idx_option_multi_leg_rfq_fills_taker_subaccount
    ON option_multi_leg_rfq_fills (LOWER(taker), taker_subaccount_id);
CREATE INDEX IF NOT EXISTS idx_option_multi_leg_rfq_fills_maker_subaccount
    ON option_multi_leg_rfq_fills (LOWER(mm_account), maker_subaccount_id);

-- ---------------------------------------------------------------------
-- option_multi_leg_rfq_fill_legs — per-leg fill detail. `option_series_id`,
-- `side`, `size_1e8`, `price_1e8` snapshotted at accept time so the
-- fill row is self-contained for downstream reporting.
-- ---------------------------------------------------------------------

CREATE TABLE IF NOT EXISTS option_multi_leg_rfq_fill_legs (
    fill_id           TEXT NOT NULL
        REFERENCES option_multi_leg_rfq_fills (fill_id),
    leg_index         INTEGER NOT NULL CHECK (leg_index >= 0),
    option_series_id  TEXT NOT NULL
        REFERENCES option_series (option_series_id),
    side              TEXT NOT NULL,
    size_1e8          TEXT NOT NULL,
    price_1e8         TEXT NOT NULL,
    PRIMARY KEY (fill_id, leg_index)
);

CREATE INDEX IF NOT EXISTS idx_option_multi_leg_rfq_fill_legs_series
    ON option_multi_leg_rfq_fill_legs (option_series_id);
