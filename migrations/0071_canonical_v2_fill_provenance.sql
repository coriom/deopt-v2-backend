-- Additive audit record for each newly reconciled V2 fee. Existing rows are
-- deliberately left without a row here and read back as legacy_unverified.
-- Signed order-origin labels never imply an independent on-chain subaccount.
CREATE TABLE canonical_v2_fill_provenance (
    chain_id BIGINT NOT NULL CHECK (chain_id = 84532),
    tx_hash TEXT NOT NULL,
    trade_log_index BIGINT NOT NULL,
    block_number BIGINT NOT NULL,
    block_hash TEXT NOT NULL,
    emitter_address TEXT NOT NULL,
    buyer_wallet TEXT NOT NULL,
    seller_wallet TEXT NOT NULL,
    fee_log_index BIGINT,
    buyer_intent_hash TEXT,
    seller_intent_hash TEXT,
    trace_call_hash TEXT,
    status TEXT NOT NULL CHECK (status IN (
        'signed_direct_verified', 'signed_trace_verified',
        'wallet_only_trace_unavailable', 'wallet_only_no_intent',
        'reorg_divergent', 'evidence_conflict'
    )),
    previous_status TEXT,
    invalidated_observed_ms BIGINT,
    created_at_ms BIGINT NOT NULL,
    PRIMARY KEY (chain_id, tx_hash, trade_log_index),
    FOREIGN KEY (chain_id, tx_hash, trade_log_index)
        REFERENCES canonical_v2_perp_fees (chain_id, tx_hash, trade_log_index),
    CHECK ((buyer_intent_hash IS NULL AND seller_intent_hash IS NULL) OR
           (buyer_intent_hash IS NOT NULL AND seller_intent_hash IS NOT NULL)),
    CHECK (status != 'signed_trace_verified' OR trace_call_hash IS NOT NULL),
    CHECK (trace_call_hash IS NULL OR status IN ('signed_trace_verified', 'reorg_divergent', 'evidence_conflict')),
    CHECK (status NOT IN ('signed_direct_verified', 'signed_trace_verified') OR
           (buyer_intent_hash IS NOT NULL AND seller_intent_hash IS NOT NULL)),
    CHECK ((status IN ('reorg_divergent', 'evidence_conflict')) = (invalidated_observed_ms IS NOT NULL))
);
CREATE INDEX idx_canonical_v2_fill_provenance_tx
    ON canonical_v2_fill_provenance (chain_id, tx_hash);
