CREATE TABLE IF NOT EXISTS indexer_cursors (
    name TEXT PRIMARY KEY,
    last_indexed_block BIGINT NOT NULL,
    updated_at_ms BIGINT NOT NULL
);

CREATE TABLE IF NOT EXISTS indexed_perp_trades (
    event_id TEXT PRIMARY KEY,
    tx_hash TEXT NOT NULL,
    log_index BIGINT NOT NULL,
    block_number BIGINT NOT NULL,
    block_hash TEXT,
    buyer TEXT NOT NULL,
    seller TEXT NOT NULL,
    market_id TEXT NOT NULL,
    size_delta_1e8 TEXT NOT NULL,
    execution_price_1e8 TEXT NOT NULL,
    buyer_is_maker BOOLEAN NOT NULL,
    buyer_nonce TEXT NOT NULL,
    seller_nonce TEXT NOT NULL,
    created_at_ms BIGINT NOT NULL,
    UNIQUE (tx_hash, log_index)
);

CREATE INDEX IF NOT EXISTS idx_indexed_perp_trades_block_number
    ON indexed_perp_trades (block_number);
CREATE INDEX IF NOT EXISTS idx_indexed_perp_trades_buyer
    ON indexed_perp_trades (buyer);
CREATE INDEX IF NOT EXISTS idx_indexed_perp_trades_seller
    ON indexed_perp_trades (seller);
CREATE INDEX IF NOT EXISTS idx_indexed_perp_trades_market_id
    ON indexed_perp_trades (market_id);
CREATE INDEX IF NOT EXISTS idx_indexed_perp_trades_tx_hash
    ON indexed_perp_trades (tx_hash);
