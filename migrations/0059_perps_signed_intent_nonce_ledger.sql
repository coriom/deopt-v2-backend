-- PERPS-CLOSED-TEST-HARDENING-V1 Part A
--
-- Durable ledger of consumed `PerpOrderIntent` nonces for the closed-test
-- signed-intent endpoint (`POST /perps/orders/signed`). Backs the
-- `PerpOrderIntentNonceLedger` trait's PG implementation.
--
-- Rationale: PERPS-FULLSTACK-RUNTIME-INTEGRATION-V1 Part D left the
-- nonce store as a process-local `HashSet` — a restart cleared it. Under
-- the closed-test posture this was tolerable (the endpoint is fail-closed
-- to non-allowlisted callers, EIP-712 verify still runs, etc.) but a
-- restart-window replay was theoretically possible. This migration adds
-- a database-enforced replay barrier that survives restarts AND is
-- atomic under concurrent submissions (PG's own UNIQUE constraint does
-- the serialisation).
--
-- Uniqueness key: `(trader, nonce_hex)`. `intent_hash` is stored for
-- forensic replay-audit but is NOT part of the uniqueness key — the
-- same intent CAN be resubmitted under a fresh nonce (that's a new
-- authorisation from the trader).
--
-- Perps public-trading posture unchanged: this table only backs the
-- closed-test signed-intent endpoint. Public perps mutation routes
-- remain permanently fail-closed regardless.

CREATE TABLE IF NOT EXISTS perps_signed_intent_nonce_ledger (
    trader         BYTEA        NOT NULL,     -- 20-byte address, lowercased canonical
    nonce_hex      TEXT         NOT NULL,     -- u128 as decimal string (matches wire)
    intent_hash    BYTEA        NOT NULL,     -- keccak256 of the signed struct, for audit
    consumed_at_ms BIGINT       NOT NULL,
    PRIMARY KEY (trader, nonce_hex),
    CONSTRAINT perps_signed_intent_nonce_ledger_trader_len
        CHECK (octet_length(trader) = 20),
    CONSTRAINT perps_signed_intent_nonce_ledger_intent_hash_len
        CHECK (octet_length(intent_hash) = 32)
);

CREATE INDEX IF NOT EXISTS idx_perps_nonce_intent_hash
    ON perps_signed_intent_nonce_ledger (intent_hash);
