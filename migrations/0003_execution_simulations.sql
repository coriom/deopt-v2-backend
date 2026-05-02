CREATE TABLE IF NOT EXISTS execution_simulations (
    simulation_id TEXT PRIMARY KEY,
    intent_id TEXT NOT NULL REFERENCES execution_intents(intent_id) ON DELETE CASCADE,
    status TEXT NOT NULL,
    block_number BIGINT,
    error TEXT,
    created_at_ms BIGINT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_execution_simulations_intent_id
    ON execution_simulations (intent_id);

CREATE INDEX IF NOT EXISTS idx_execution_simulations_status
    ON execution_simulations (status);
