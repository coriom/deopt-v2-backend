ALTER TABLE option_execution_transactions
    ADD COLUMN IF NOT EXISTS confirmation_status TEXT NULL;
ALTER TABLE option_execution_transactions
    ADD COLUMN IF NOT EXISTS confirmed_at_ms BIGINT NULL;
ALTER TABLE option_execution_transactions
    ADD COLUMN IF NOT EXISTS confirmed_block_number BIGINT NULL;
ALTER TABLE option_execution_transactions
    ADD COLUMN IF NOT EXISTS receipt_status BIGINT NULL;
ALTER TABLE option_execution_transactions
    ADD COLUMN IF NOT EXISTS confirmation_error TEXT NULL;
