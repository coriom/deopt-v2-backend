-- ACCOUNT-WRITE-AUTH-HARDENING-V1
--
-- Server-issued challenge / nonce table for HTTP write-authorization.
-- Each row represents one outstanding (or consumed) write-auth
-- envelope. Atomic claim semantics live in `PgRepository::claim_write_auth_challenge`.
--
-- nonce_bytes is the 32-byte CSPRNG random value the wallet signs
-- inside the EIP-712 `WriteAuthorization` message. Stored as BYTEA(32).
--
-- request_digest is keccak256(eip712_digest(envelope)) — captured on
-- claim so a second incoming envelope with the SAME nonce but a
-- DIFFERENT canonical payload is rejected (payload-mismatch), while
-- an exact retry of the same envelope returns the same resource_id
-- (idempotent replay).

CREATE TABLE IF NOT EXISTS write_auth_challenges (
    nonce_bytes      BYTEA       PRIMARY KEY,
    account          TEXT        NOT NULL,
    action           TEXT        NOT NULL,
    chain_id         BIGINT      NOT NULL,
    issued_at_ms     BIGINT      NOT NULL,
    expires_at_ms    BIGINT      NOT NULL,
    status           TEXT        NOT NULL,
    request_digest   BYTEA,
    idempotency_key  TEXT,
    resource_id      TEXT,
    consumed_at_ms   BIGINT,
    CONSTRAINT write_auth_challenges_nonce_bytes_len CHECK (octet_length(nonce_bytes) = 32),
    CONSTRAINT write_auth_challenges_request_digest_len CHECK (
        request_digest IS NULL OR octet_length(request_digest) = 32
    ),
    CONSTRAINT write_auth_challenges_status_values CHECK (
        status IN ('issued', 'consumed', 'expired')
    )
);

CREATE INDEX IF NOT EXISTS write_auth_challenges_by_account_action_status
    ON write_auth_challenges (lower(account), action, status);

CREATE INDEX IF NOT EXISTS write_auth_challenges_by_expires_at_ms
    ON write_auth_challenges (expires_at_ms);

-- Idempotency: at most one consumed row per (account, action,
-- idempotency_key) — so a retry with the same idempotency key cannot
-- create a second resource.
CREATE UNIQUE INDEX IF NOT EXISTS write_auth_challenges_idempotency
    ON write_auth_challenges (lower(account), action, idempotency_key)
    WHERE idempotency_key IS NOT NULL AND status = 'consumed';
