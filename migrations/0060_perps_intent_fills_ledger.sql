-- PERPS-CLOSED-TEST-HARDENING-V1 Part B
--
-- Cumulative-fill ledger for signed `PerpOrderIntent` submissions. Each
-- row tracks how much of the intent's signed `size_1e8` has been filled
-- so far. The row is INSERTed once per intent (idempotent by intent
-- hash) and UPDATEd atomically each time the internal engine produces
-- fills against that intent.
--
-- Two-layer accounting model:
--
--   * Layer A — nonce ledger (`perps_signed_intent_nonce_ledger`) —
--     prevents `(trader, nonce)` REPLAY of an intent submission.
--   * Layer B — this table — prevents CUMULATIVE OVERFILL of an
--     already-consumed intent, even across restarts. Guards against a
--     matching-logic bug that could double-count fills.
--
-- Perps public-trading posture unchanged: this table only backs the
-- closed-test signed-intent endpoint. Public perps mutation routes
-- remain permanently fail-closed regardless.
--
-- Uniqueness key: `(intent_hash)`. The `trader` column is denormalised
-- for forensic audit and is NOT part of the key.

CREATE TABLE IF NOT EXISTS perps_intent_fills_ledger (
    intent_hash     BYTEA         NOT NULL,     -- 32-byte keccak256 of signed intent
    trader          BYTEA         NOT NULL,     -- 20-byte address (denormalised for audit)
    signed_size_1e8 NUMERIC(39,0) NOT NULL,     -- u128 ceiling for cumulative fills
    filled_size_1e8 NUMERIC(39,0) NOT NULL DEFAULT 0,
    last_updated_ms BIGINT        NOT NULL,
    PRIMARY KEY (intent_hash),
    CONSTRAINT perps_intent_fills_ledger_intent_hash_len
        CHECK (octet_length(intent_hash) = 32),
    CONSTRAINT perps_intent_fills_ledger_trader_len
        CHECK (octet_length(trader) = 20),
    CONSTRAINT perps_intent_fills_ledger_signed_size_nonneg
        CHECK (signed_size_1e8 >= 0),
    CONSTRAINT perps_intent_fills_ledger_filled_size_nonneg
        CHECK (filled_size_1e8 >= 0),
    CONSTRAINT perps_intent_fills_ledger_filled_le_signed
        CHECK (filled_size_1e8 <= signed_size_1e8)
);

CREATE INDEX IF NOT EXISTS idx_perps_intent_fills_ledger_trader
    ON perps_intent_fills_ledger (trader);
