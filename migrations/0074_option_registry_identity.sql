-- Preserve existing option_series_id foreign keys. Historical rows have no
-- inferred registry; operators must verify their provenance before backfill.
ALTER TABLE option_series
    ADD COLUMN onchain_chain_id BIGINT,
    ADD COLUMN onchain_registry_address TEXT;

ALTER TABLE option_series
    ADD CONSTRAINT option_series_registry_identity_pair CHECK (
        (onchain_chain_id IS NULL AND onchain_registry_address IS NULL)
        OR (onchain_chain_id > 0 AND onchain_registry_address ~ '^0x[0-9a-f]{40}$'
            AND onchain_series_id IS NOT NULL)
    );

CREATE UNIQUE INDEX option_series_registry_option_id_unique
    ON option_series (onchain_chain_id, onchain_registry_address, onchain_series_id)
    WHERE onchain_chain_id IS NOT NULL;
