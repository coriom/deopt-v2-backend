-- Read-only wallet history authentication. Raw signatures and bearer tokens
-- are never stored. Issuance, nonce consumption and session creation are
-- transactional; sessions can be revoked independently of the browser.
CREATE TABLE wallet_read_challenges (
    nonce_hash BYTEA PRIMARY KEY CHECK (octet_length(nonce_hash) = 32),
    wallet TEXT NOT NULL,
    origin TEXT NOT NULL,
    chain_id BIGINT NOT NULL,
    message TEXT NOT NULL,
    issued_at_ms BIGINT NOT NULL,
    expires_at_ms BIGINT NOT NULL,
    consumed_at_ms BIGINT,
    CHECK (expires_at_ms > issued_at_ms)
);
CREATE INDEX wallet_read_challenges_wallet_expiry ON wallet_read_challenges (wallet, expires_at_ms);

CREATE TABLE wallet_read_sessions (
    token_hash BYTEA PRIMARY KEY CHECK (octet_length(token_hash) = 32),
    wallet TEXT NOT NULL,
    origin TEXT NOT NULL,
    chain_id BIGINT NOT NULL,
    issued_at_ms BIGINT NOT NULL,
    expires_at_ms BIGINT NOT NULL,
    revoked_at_ms BIGINT,
    CHECK (expires_at_ms > issued_at_ms)
);
CREATE INDEX wallet_read_sessions_wallet_expiry ON wallet_read_sessions (wallet, expires_at_ms);

CREATE TABLE wallet_read_rate (
    scope TEXT NOT NULL,
    minute_bucket BIGINT NOT NULL,
    attempts INTEGER NOT NULL CHECK (attempts > 0),
    PRIMARY KEY (scope, minute_bucket)
);
