-- PERPS_BASE_SEPOLIA_BACKEND_BROADCAST_DURABILITY_PG_V1
--
-- Durable broadcast lifecycle table for the Perps closed-test settlement
-- worker (src/execution/broadcast_policy.rs). One row per execution intent
-- that reaches the "prepared to broadcast" phase.
--
-- Uniqueness:
--   * intent_id — one active broadcast per canonical execution identity.
--   * tx_hash — the derived keccak256(raw_signed_tx). No two intents may
--     share a tx_hash.
--   * (chain_id, executor_address, nonce) — no two intents can allocate
--     the same executor nonce on the same chain. Enforces multi-worker
--     nonce safety at the DB level so races cannot slip past Rust
--     in-memory checks.
--
-- Lifecycle:
--   prepared (raw envelope persisted, RPC not yet observed)
--     ├─ submitted (eth_sendRawTransaction returned OK or idempotent-replay)
--     │    ├─ confirmed (receipt.status = 1, event verified)
--     │    └─ failed (receipt.status = 0 OR event verification failed)
--     └─ failed (deterministic reject class from RPC)
--
-- Raw signed transaction bytes are retained for byte-identical
-- rebroadcast in the reconciler. This is operational protocol data, not
-- private-key material — the private key never touches this table.

CREATE TABLE IF NOT EXISTS execution_intent_broadcasts (
    intent_id                   TEXT PRIMARY KEY REFERENCES execution_intents(intent_id) ON DELETE CASCADE,
    chain_id                    BIGINT NOT NULL,
    executor_address            TEXT NOT NULL,
    target_address              TEXT NOT NULL,
    tx_hash                     TEXT NOT NULL,
    nonce                       BIGINT NOT NULL,
    raw_tx_hex                  TEXT NOT NULL,
    status                      TEXT NOT NULL,
    prepared_at_ms              BIGINT NOT NULL,
    first_submission_at_ms      BIGINT,
    last_send_at_ms             BIGINT,
    send_attempts               INTEGER NOT NULL DEFAULT 0,
    receipt_block_number        BIGINT,
    receipt_status              BIGINT,
    confirmed_at_ms             BIGINT,
    failure_class               TEXT,
    failure_reason              TEXT,
    failed_at_ms                BIGINT,
    updated_at_ms               BIGINT NOT NULL
);

-- Enforce global uniqueness of the derived transaction hash. Two intents
-- cannot share an on-chain envelope.
CREATE UNIQUE INDEX IF NOT EXISTS uq_execution_intent_broadcasts_tx_hash
    ON execution_intent_broadcasts (tx_hash);

-- Enforce executor nonce uniqueness per chain. Multi-worker safety: two
-- workers racing to broadcast will collide on this constraint rather
-- than emit two distinct transactions carrying the same nonce.
CREATE UNIQUE INDEX IF NOT EXISTS uq_execution_intent_broadcasts_executor_nonce
    ON execution_intent_broadcasts (chain_id, executor_address, nonce);

-- Reconciliation queries scan the unfinalized set frequently.
CREATE INDEX IF NOT EXISTS idx_execution_intent_broadcasts_status
    ON execution_intent_broadcasts (status);
CREATE INDEX IF NOT EXISTS idx_execution_intent_broadcasts_prepared_at_ms
    ON execution_intent_broadcasts (prepared_at_ms);
