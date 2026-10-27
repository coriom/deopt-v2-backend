-- Canonical D2 charges are distinct from local matching projections and the
-- legacy fee/rebate ledger. A trade has at most one nonzero taker charge under
-- the approved zero-maker, zero-rebate Base Sepolia policy.
CREATE TABLE IF NOT EXISTS canonical_v2_perp_fees (
    chain_id BIGINT NOT NULL CHECK (chain_id = 84532),
    tx_hash TEXT NOT NULL,
    trade_log_index BIGINT NOT NULL,
    fee_log_index BIGINT,
    block_number BIGINT NOT NULL,
    block_hash TEXT NOT NULL,
    market_id TEXT NOT NULL,
    payer TEXT NOT NULL,
    recipient TEXT NOT NULL,
    settlement_asset TEXT NOT NULL,
    maker_tier SMALLINT NOT NULL CHECK (maker_tier BETWEEN 0 AND 4),
    taker_tier SMALLINT NOT NULL CHECK (taker_tier BETWEEN 0 AND 4),
    basis_native TEXT NOT NULL,
    fee_ppm INTEGER NOT NULL CHECK (fee_ppm = 200),
    amount_native TEXT NOT NULL,
    created_at_ms BIGINT NOT NULL,
    PRIMARY KEY (chain_id, tx_hash, trade_log_index),
    FOREIGN KEY (chain_id, tx_hash, trade_log_index)
        REFERENCES indexed_perp_trades (chain_id, tx_hash, log_index)
);
CREATE UNIQUE INDEX IF NOT EXISTS uq_canonical_v2_perp_fee_log
    ON canonical_v2_perp_fees (chain_id, tx_hash, fee_log_index)
    WHERE fee_log_index IS NOT NULL;
