ALTER TABLE option_execution_transactions
    ADD COLUMN IF NOT EXISTS estimated_gas BIGINT NULL;
ALTER TABLE option_execution_transactions
    ADD COLUMN IF NOT EXISTS required_gas BIGINT NULL;
ALTER TABLE option_execution_transactions
    ADD COLUMN IF NOT EXISTS simulation_gas_limit BIGINT NULL;
ALTER TABLE option_execution_transactions
    ADD COLUMN IF NOT EXISTS broadcast_gas_limit BIGINT NULL;
ALTER TABLE option_execution_transactions
    ADD COLUMN IF NOT EXISTS gas_safety_bps INTEGER NULL;
ALTER TABLE option_execution_transactions
    ADD COLUMN IF NOT EXISTS gas_check_status TEXT NULL;
ALTER TABLE option_execution_transactions
    ADD COLUMN IF NOT EXISTS gas_check_error TEXT NULL;
