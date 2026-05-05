ALTER TABLE execution_simulations
    ADD COLUMN IF NOT EXISTS revert_data TEXT;

ALTER TABLE execution_simulations
    ADD COLUMN IF NOT EXISTS revert_selector TEXT;

ALTER TABLE execution_simulations
    ADD COLUMN IF NOT EXISTS decoded_error TEXT;

CREATE INDEX IF NOT EXISTS idx_execution_simulations_revert_selector
    ON execution_simulations (revert_selector);
