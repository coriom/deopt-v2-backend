-- 0044_hybrid_v2_canonical_state.sql
--
-- BACKEND-SUBACCOUNT-CANONICAL-STATE-AND-INDEXER-V1 (WP-01 backend) — additive
-- Hybrid V2 canonical projection schema. NO destructive change to any prior
-- V1 table.
--
-- Layering (per contract-spec/13 + WP-11 result doc):
--   1. immutable / raw chain journal      → hybrid_v2_raw_logs
--   2. deployment identity                → hybrid_v2_deployments
--   3. reorg-safe cursor                  → hybrid_v2_cursors
--   4. decoded typed events               → hybrid_v2_decoded_events
--   5. canonical projections              → hybrid_v2_subaccounts /
--                                            _vault_balances /
--                                            _reservations /
--                                            _collateral_universe /
--                                            _capability_grants /
--                                            _recovery_state
--
-- Canonicality rule (frozen): PostgreSQL is a REBUILDABLE PROJECTION of
-- chain state. Every projection row is traceable to a raw log OR to the
-- immutable manifest OR to a bounded canonical contract view.

BEGIN;

-------------------------------------------------------------------------------
-- Deployment identity
-------------------------------------------------------------------------------

CREATE TABLE IF NOT EXISTS hybrid_v2_deployments (
    deployment_id           BIGSERIAL   PRIMARY KEY,
    chain_id                BIGINT      NOT NULL,
    manifest_address        TEXT        NOT NULL,
    manifest_hash           TEXT        NOT NULL,
    module_addresses_hash   TEXT        NOT NULL,
    critical_config_hash    TEXT        NOT NULL,
    architecture_version    INTEGER     NOT NULL,
    storage_version         INTEGER     NOT NULL,
    event_version           INTEGER     NOT NULL,
    deployment_version      INTEGER     NOT NULL,
    manifest_schema_version INTEGER     NOT NULL,
    environment_tag         TEXT        NOT NULL,
    deployer                TEXT        NOT NULL,
    deployment_block        BIGINT      NOT NULL,
    deployment_timestamp    BIGINT      NOT NULL,
    module_addresses_json   JSONB       NOT NULL,
    protocol_fee_subkey     TEXT        NOT NULL,
    rebate_budget_subkey    TEXT        NOT NULL,
    insurance_fund_subkey   TEXT        NOT NULL,
    max_collateral_tokens   INTEGER     NOT NULL,
    max_active_series       INTEGER     NOT NULL,
    all_capabilities_mask   TEXT        NOT NULL,
    recovery_activation_delay_seconds  BIGINT NOT NULL,
    recovery_pause_max_duration_blocks BIGINT NOT NULL,
    activation_status       TEXT        NOT NULL DEFAULT 'PENDING',
    created_at_ms           BIGINT      NOT NULL,
    updated_at_ms           BIGINT      NOT NULL,
    CONSTRAINT hybrid_v2_deployments_uniq UNIQUE (chain_id, manifest_hash)
);

-- One live deployment per (chain_id, deployment_version). Prevents two
-- incompatible manifests from sharing one deployment identity.
CREATE UNIQUE INDEX IF NOT EXISTS hybrid_v2_deployments_version_uniq
    ON hybrid_v2_deployments (chain_id, deployment_version);

-- Base mainnet (8453) is forbidden — enforced defensively at insert time
-- by the manifest ingestion module, mirrored here for belt-and-braces
-- documentation. A CHECK constraint would silently reject rows even from
-- a migration replay of historical data; the ingestion module is the
-- canonical enforcement point.
COMMENT ON TABLE hybrid_v2_deployments IS
    'One row per deployed HybridV2 manifest. Base mainnet (8453) is rejected by the ingestion module, not by a check constraint here.';

-------------------------------------------------------------------------------
-- Reorg-safe cursor
-------------------------------------------------------------------------------

CREATE TABLE IF NOT EXISTS hybrid_v2_cursors (
    deployment_id       BIGINT      NOT NULL
                        REFERENCES hybrid_v2_deployments(deployment_id)
                        ON DELETE RESTRICT,
    cursor_name         TEXT        NOT NULL,
    indexed_head_block  BIGINT      NOT NULL,
    indexed_head_hash   TEXT        NOT NULL,
    indexed_head_parent TEXT        NOT NULL,
    observed_head_block BIGINT      NOT NULL DEFAULT 0,
    finalized_head_block BIGINT     NOT NULL DEFAULT 0,
    last_advanced_at_ms BIGINT      NOT NULL,
    last_error          TEXT        NULL,
    PRIMARY KEY (deployment_id, cursor_name)
);

-------------------------------------------------------------------------------
-- Raw canonical log journal
-------------------------------------------------------------------------------

CREATE TABLE IF NOT EXISTS hybrid_v2_raw_logs (
    raw_log_id      BIGSERIAL   PRIMARY KEY,
    deployment_id   BIGINT      NOT NULL
                    REFERENCES hybrid_v2_deployments(deployment_id)
                    ON DELETE RESTRICT,
    block_number    BIGINT      NOT NULL,
    block_hash      TEXT        NOT NULL,
    parent_hash     TEXT        NOT NULL,
    block_timestamp BIGINT      NOT NULL,
    tx_hash         TEXT        NOT NULL,
    tx_index        INTEGER     NOT NULL,
    log_index       INTEGER     NOT NULL,
    emitter         TEXT        NOT NULL,
    topic0          TEXT        NOT NULL,
    topics_json     JSONB       NOT NULL,
    data_hex        TEXT        NOT NULL,
    is_canonical    BOOLEAN     NOT NULL DEFAULT TRUE,
    orphaned_at_block BIGINT    NULL,
    created_at_ms   BIGINT      NOT NULL
);

-- Canonical event identity per (deployment, block_hash, tx_hash, log_index).
-- Block-number alone is insufficient — block_hash distinguishes a
-- replaced block after a reorg.
CREATE UNIQUE INDEX IF NOT EXISTS hybrid_v2_raw_logs_identity_uniq
    ON hybrid_v2_raw_logs (deployment_id, block_hash, tx_hash, log_index);

-- Canonical-order queries: block, tx, log ascending.
CREATE INDEX IF NOT EXISTS hybrid_v2_raw_logs_order_idx
    ON hybrid_v2_raw_logs (deployment_id, is_canonical, block_number, tx_index, log_index);

-- Emitter + topic0 lookup.
CREATE INDEX IF NOT EXISTS hybrid_v2_raw_logs_emitter_topic0_idx
    ON hybrid_v2_raw_logs (deployment_id, emitter, topic0);

-------------------------------------------------------------------------------
-- Decoded typed events
-------------------------------------------------------------------------------

CREATE TABLE IF NOT EXISTS hybrid_v2_decoded_events (
    decoded_event_id BIGSERIAL  PRIMARY KEY,
    raw_log_id      BIGINT      NOT NULL
                    REFERENCES hybrid_v2_raw_logs(raw_log_id)
                    ON DELETE CASCADE,
    deployment_id   BIGINT      NOT NULL
                    REFERENCES hybrid_v2_deployments(deployment_id)
                    ON DELETE RESTRICT,
    event_kind      TEXT        NOT NULL,
    event_version   INTEGER     NOT NULL,
    module_slot     INTEGER     NULL,
    subkey          TEXT        NULL,
    owner           TEXT        NULL,
    subaccount_id   BIGINT      NULL,
    token           TEXT        NULL,
    engine          TEXT        NULL,
    execution_id    TEXT        NULL,
    order_hash      TEXT        NULL,
    series_id       TEXT        NULL,
    payload_json    JSONB       NOT NULL,
    applied         BOOLEAN     NOT NULL DEFAULT FALSE,
    decoder_version INTEGER     NOT NULL,
    created_at_ms   BIGINT      NOT NULL
);

CREATE UNIQUE INDEX IF NOT EXISTS hybrid_v2_decoded_events_raw_uniq
    ON hybrid_v2_decoded_events (raw_log_id);

CREATE INDEX IF NOT EXISTS hybrid_v2_decoded_events_kind_idx
    ON hybrid_v2_decoded_events (deployment_id, event_kind);

CREATE INDEX IF NOT EXISTS hybrid_v2_decoded_events_subkey_idx
    ON hybrid_v2_decoded_events (deployment_id, subkey);

-------------------------------------------------------------------------------
-- Canonical projections
-------------------------------------------------------------------------------

-- Subaccount identity — one row per (deployment, subKey). Materialised
-- from SubaccountCreated / SubaccountLazyRegistered events.
CREATE TABLE IF NOT EXISTS hybrid_v2_subaccounts (
    deployment_id   BIGINT      NOT NULL
                    REFERENCES hybrid_v2_deployments(deployment_id)
                    ON DELETE RESTRICT,
    subkey          TEXT        NOT NULL,
    owner           TEXT        NOT NULL,
    subaccount_id   BIGINT      NOT NULL,
    materialised_via TEXT       NOT NULL,
    created_block   BIGINT      NOT NULL,
    created_tx_hash TEXT        NOT NULL,
    created_at_ms   BIGINT      NOT NULL,
    PRIMARY KEY (deployment_id, subkey)
);

CREATE UNIQUE INDEX IF NOT EXISTS hybrid_v2_subaccounts_owner_uniq
    ON hybrid_v2_subaccounts (deployment_id, owner, subaccount_id);

-- Vault balance per (subKey, token). Uint256 in TEXT (decimal).
CREATE TABLE IF NOT EXISTS hybrid_v2_vault_balances (
    deployment_id   BIGINT      NOT NULL
                    REFERENCES hybrid_v2_deployments(deployment_id)
                    ON DELETE RESTRICT,
    subkey          TEXT        NOT NULL,
    token           TEXT        NOT NULL,
    balance         TEXT        NOT NULL DEFAULT '0',
    last_event_block BIGINT     NOT NULL,
    last_event_log_id BIGINT    NOT NULL,
    updated_at_ms   BIGINT      NOT NULL,
    PRIMARY KEY (deployment_id, subkey, token)
);

-- Per-engine reservation per (subKey, token, engine).
CREATE TABLE IF NOT EXISTS hybrid_v2_reservations (
    deployment_id   BIGINT      NOT NULL
                    REFERENCES hybrid_v2_deployments(deployment_id)
                    ON DELETE RESTRICT,
    subkey          TEXT        NOT NULL,
    token           TEXT        NOT NULL,
    engine          TEXT        NOT NULL,
    reserved        TEXT        NOT NULL DEFAULT '0',
    last_event_block BIGINT     NOT NULL,
    updated_at_ms   BIGINT      NOT NULL,
    PRIMARY KEY (deployment_id, subkey, token, engine)
);

-- Collateral universe entry (append-only per deployment, ≤ 8).
CREATE TABLE IF NOT EXISTS hybrid_v2_collateral_universe (
    deployment_id   BIGINT      NOT NULL
                    REFERENCES hybrid_v2_deployments(deployment_id)
                    ON DELETE RESTRICT,
    token           TEXT        NOT NULL,
    universe_index  INTEGER     NOT NULL,
    entered_block   BIGINT      NOT NULL,
    entered_at_ms   BIGINT      NOT NULL,
    enabled         BOOLEAN     NOT NULL DEFAULT TRUE,
    PRIMARY KEY (deployment_id, token)
);

CREATE UNIQUE INDEX IF NOT EXISTS hybrid_v2_collateral_universe_index_uniq
    ON hybrid_v2_collateral_universe (deployment_id, universe_index);

-- Capability bitmap per engine.
CREATE TABLE IF NOT EXISTS hybrid_v2_capability_grants (
    deployment_id   BIGINT      NOT NULL
                    REFERENCES hybrid_v2_deployments(deployment_id)
                    ON DELETE RESTRICT,
    engine          TEXT        NOT NULL,
    capability_bits TEXT        NOT NULL DEFAULT '0',
    updated_at_ms   BIGINT      NOT NULL,
    PRIMARY KEY (deployment_id, engine)
);

-- Recovery state per subKey — canonical `SM-Rec` reflection.
CREATE TABLE IF NOT EXISTS hybrid_v2_recovery_state (
    deployment_id   BIGINT      NOT NULL
                    REFERENCES hybrid_v2_deployments(deployment_id)
                    ON DELETE RESTRICT,
    subkey          TEXT        NOT NULL,
    state           TEXT        NOT NULL,
    pending_since_ts BIGINT     NULL,
    activation_eligible_at BIGINT NULL,
    finalized_at_ts BIGINT      NULL,
    tokens_withdrawn INTEGER    NOT NULL DEFAULT 0,
    updated_at_ms   BIGINT      NOT NULL,
    PRIMARY KEY (deployment_id, subkey)
);

COMMIT;
