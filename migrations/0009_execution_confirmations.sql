ALTER TABLE execution_transactions
    ADD COLUMN IF NOT EXISTS confirmed_at_ms BIGINT,
    ADD COLUMN IF NOT EXISTS confirmed_block_number BIGINT,
    ADD COLUMN IF NOT EXISTS confirmation_status TEXT,
    ADD COLUMN IF NOT EXISTS confirmation_error TEXT;

CREATE INDEX IF NOT EXISTS idx_execution_transactions_confirmation_status
    ON execution_transactions(confirmation_status);
