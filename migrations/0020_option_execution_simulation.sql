ALTER TABLE option_execution_intents
    ADD COLUMN IF NOT EXISTS simulation_status TEXT NULL;

ALTER TABLE option_execution_intents
    ADD COLUMN IF NOT EXISTS simulation_error TEXT NULL;

ALTER TABLE option_execution_intents
    ADD COLUMN IF NOT EXISTS simulation_block_number BIGINT NULL;

ALTER TABLE option_execution_intents
    ADD COLUMN IF NOT EXISTS simulation_revert_data TEXT NULL;

ALTER TABLE option_execution_intents
    ADD COLUMN IF NOT EXISTS simulation_revert_selector TEXT NULL;

ALTER TABLE option_execution_intents
    ADD COLUMN IF NOT EXISTS simulated_at_ms BIGINT NULL;

CREATE INDEX IF NOT EXISTS idx_option_execution_intents_simulation_status
    ON option_execution_intents(simulation_status);
