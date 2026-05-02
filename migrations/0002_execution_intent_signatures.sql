ALTER TABLE execution_intents
    ADD COLUMN IF NOT EXISTS buyer_is_maker BOOLEAN,
    ADD COLUMN IF NOT EXISTS buyer_nonce BIGINT,
    ADD COLUMN IF NOT EXISTS seller_nonce BIGINT,
    ADD COLUMN IF NOT EXISTS deadline_ms BIGINT;

CREATE TABLE IF NOT EXISTS execution_intent_signatures (
    intent_id TEXT PRIMARY KEY REFERENCES execution_intents(intent_id) ON DELETE CASCADE,
    buyer_sig TEXT,
    seller_sig TEXT,
    updated_at_ms BIGINT NOT NULL
);
