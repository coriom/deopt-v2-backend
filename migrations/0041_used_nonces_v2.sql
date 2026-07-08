-- SUBACCOUNTS-V2-NONCE-TABLE-V1
--
-- Formal v2 write-authorization nonce-consumption namespace, keyed by
-- (account, subaccount_id, action, nonce). Independent of and additive
-- to `write_auth_challenges` (migration 0029) which continues to serve
-- v1 nonce/idempotency and v2 signature verification.
--
-- Rationale: v2 canonical payloads embed `subaccount_id` and rely on
-- digest divergence for cross-subaccount replay safety. This table
-- adds a database-enforced barrier: a v2 nonce may be consumed at most
-- once per (account, subaccount_id, action) tuple, even if the
-- upstream challenge store were bypassed, purged, or otherwise
-- compromised. It also permits — without false-collision — the case
-- where the same nonce is legitimately used across different
-- subaccounts or actions, since each combination has a distinct row.
--
-- v1 behavior is unchanged: v1 handlers never touch this table.
-- Perps is out of scope: no Perps action currently issues a v2
-- envelope, so no Perps row can appear here.

CREATE TABLE IF NOT EXISTS used_nonces_v2 (
    account        TEXT   NOT NULL,
    subaccount_id  INT    NOT NULL,
    action         TEXT   NOT NULL,
    nonce_bytes    BYTEA  NOT NULL,
    request_digest BYTEA  NOT NULL,
    used_at_ms     BIGINT NOT NULL,
    CONSTRAINT used_nonces_v2_subaccount_id_positive
        CHECK (subaccount_id >= 1),
    CONSTRAINT used_nonces_v2_nonce_bytes_len
        CHECK (octet_length(nonce_bytes) = 32),
    CONSTRAINT used_nonces_v2_request_digest_len
        CHECK (octet_length(request_digest) = 32)
);

-- Composite uniqueness on the case-insensitive account key. Serves as
-- the deterministic replay guard: any duplicate insert raises SQLSTATE
-- 23505 which the store maps to `NonceAlreadyUsed`.
CREATE UNIQUE INDEX IF NOT EXISTS used_nonces_v2_key
    ON used_nonces_v2 (lower(account), subaccount_id, action, nonce_bytes);

-- Lookup helper for audit/observability by (account, subaccount, action).
CREATE INDEX IF NOT EXISTS used_nonces_v2_by_account_action
    ON used_nonces_v2 (lower(account), subaccount_id, action);
