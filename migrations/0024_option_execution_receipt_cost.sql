ALTER TABLE option_execution_transactions
    ADD COLUMN IF NOT EXISTS gas_used BIGINT NULL;
ALTER TABLE option_execution_transactions
    ADD COLUMN IF NOT EXISTS effective_gas_price TEXT NULL;
ALTER TABLE option_execution_transactions
    ADD COLUMN IF NOT EXISTS cumulative_gas_used BIGINT NULL;
ALTER TABLE option_execution_transactions
    ADD COLUMN IF NOT EXISTS receipt_block_hash TEXT NULL;
ALTER TABLE option_execution_transactions
    ADD COLUMN IF NOT EXISTS receipt_transaction_index BIGINT NULL;
ALTER TABLE option_execution_transactions
    ADD COLUMN IF NOT EXISTS receipt_observed_at_ms BIGINT NULL;
