-- Signed order-origin metadata for D6 executeTradeFromIntents. These values
-- do not create independent on-chain subaccount balances or transfer a fee.
CREATE TABLE canonical_v2_intent_proofs (
    chain_id BIGINT NOT NULL CHECK (chain_id = 84532),
    tx_hash TEXT NOT NULL,
    trade_log_index BIGINT NOT NULL,
    block_hash TEXT NOT NULL,
    buyer_intent_hash TEXT NOT NULL,
    seller_intent_hash TEXT NOT NULL,
    buyer_signed_subaccount_id BIGINT NOT NULL CHECK (buyer_signed_subaccount_id >= 0),
    seller_signed_subaccount_id BIGINT NOT NULL CHECK (seller_signed_subaccount_id >= 0),
    PRIMARY KEY (chain_id, tx_hash, trade_log_index),
    FOREIGN KEY (chain_id, tx_hash, trade_log_index)
        REFERENCES canonical_v2_perp_fees (chain_id, tx_hash, trade_log_index)
);
