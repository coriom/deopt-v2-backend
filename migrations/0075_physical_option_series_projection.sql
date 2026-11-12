-- Registry events are indexed only after the configured confirmation depth.
-- Keep this projection separate from historical/manual option_series rows.
CREATE TABLE physical_option_series_events (
    chain_id BIGINT NOT NULL,
    registry_address TEXT NOT NULL,
    option_id TEXT NOT NULL,
    tx_hash TEXT NOT NULL,
    block_number BIGINT NOT NULL,
    block_hash TEXT NOT NULL,
    log_index BIGINT NOT NULL,
    event_name TEXT NOT NULL CHECK (event_name IN ('SeriesCreated', 'SeriesStatusUpdated')),
    underlying TEXT,
    settlement_asset TEXT,
    expiry BIGINT,
    strike_1e8 TEXT,
    contract_size_1e8 TEXT,
    is_call BOOLEAN,
    is_european BOOLEAN,
    is_active BOOLEAN NOT NULL,
    PRIMARY KEY (chain_id, registry_address, tx_hash, log_index)
);
CREATE INDEX physical_option_series_events_order
    ON physical_option_series_events (chain_id, registry_address, block_number, log_index);

CREATE TABLE physical_option_series_projection (
    chain_id BIGINT NOT NULL,
    registry_address TEXT NOT NULL,
    option_id TEXT NOT NULL,
    underlying TEXT NOT NULL,
    settlement_asset TEXT NOT NULL,
    expiry BIGINT NOT NULL,
    strike_1e8 TEXT NOT NULL,
    contract_size_1e8 TEXT NOT NULL,
    is_call BOOLEAN NOT NULL,
    is_european BOOLEAN NOT NULL,
    is_active BOOLEAN NOT NULL,
    creation_tx_hash TEXT NOT NULL,
    creation_block_number BIGINT NOT NULL,
    creation_block_hash TEXT NOT NULL,
    creation_log_index BIGINT NOT NULL,
    last_tx_hash TEXT NOT NULL,
    last_block_number BIGINT NOT NULL,
    last_block_hash TEXT NOT NULL,
    last_log_index BIGINT NOT NULL,
    confirmed_at_head BIGINT NOT NULL,
    PRIMARY KEY (chain_id, registry_address, option_id)
);

CREATE TABLE physical_option_series_cursor (
    chain_id BIGINT NOT NULL,
    registry_address TEXT NOT NULL,
    block_number BIGINT NOT NULL,
    block_hash TEXT NOT NULL,
    PRIMARY KEY (chain_id, registry_address)
);
