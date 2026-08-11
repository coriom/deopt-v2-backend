-- BACKEND-HYBRID-V2-BROADCAST-AND-CONFIRMATION-V1 (Part E)
--
-- Broadcast state table for the Hybrid V2 broadcast pipeline. One row
-- per canonical_execution_id — enforced by PRIMARY KEY + FOREIGN KEY.
--
-- Frozen safety invariants:
--   * tx_hash + envelope_hash are IMMUTABLE once set (enforced by
--     `hybrid_v2_broadcast_state_immutability_trigger` below). The
--     outbox persists these BEFORE calling the network so a restart
--     mid-call sees the same row + hash and knows to investigate via
--     `transaction_by_hash` — never to re-sign.
--   * phase CHECK constraint keeps the SQL layer as a second-line
--     firewall against illegal phase tokens (the Rust
--     `BroadcastPhase::can_transition_to` matrix is the first line).
--   * canonicality_state CHECK guards the receipt-vs-canonical-chain
--     column.
--   * tx_hash is UNIQUE globally (no two executions may claim the
--     same hash — that would be a nonce collision).
CREATE TABLE hybrid_v2_broadcast_state (
    canonical_execution_id      TEXT PRIMARY KEY
        REFERENCES hybrid_v2_execution_requests (canonical_execution_id),
    tx_hash                     TEXT NULL UNIQUE,
    envelope_hash               TEXT NULL,
    envelope_bytes_hash         TEXT NULL,
    phase                       TEXT NOT NULL DEFAULT 'BROADCAST_DISABLED'
        CHECK (phase IN (
            'BROADCAST_DISABLED',
            'READY_FOR_BROADCAST',
            'BROADCASTING',
            'SUBMISSION_UNKNOWN',
            'SUBMITTED',
            'PENDING',
            'MINED_SUCCESS',
            'MINED_REVERTED',
            'CONFIRMING',
            'CONFIRMED',
            'REORGED',
            'DROPPED',
            'MANUAL_INTERVENTION_REQUIRED',
            'CANCELLED_BEFORE_BROADCAST'
        )),
    submission_attempt_count    INT NOT NULL DEFAULT 0,
    first_submission_at_ms      BIGINT NULL,
    last_submission_at_ms       BIGINT NULL,
    provider_classification     TEXT NULL,
    receipt_tx_hash             TEXT NULL,
    receipt_block_number        BIGINT NULL,
    receipt_block_hash          TEXT NULL,
    receipt_status              SMALLINT NULL,
    gas_used                    BIGINT NULL,
    effective_gas_price_wei     NUMERIC(78, 0) NULL,
    actual_tx_cost_wei          NUMERIC(78, 0) NULL,
    confirmation_count          INT NOT NULL DEFAULT 0,
    canonicality_state          TEXT NOT NULL DEFAULT 'UNKNOWN'
        CHECK (canonicality_state IN ('UNKNOWN', 'CANONICAL', 'ORPHANED', 'REORGED')),
    correlated_execution_status TEXT NULL,
    last_checked_head           BIGINT NULL,
    last_checked_finalized_head BIGINT NULL,
    reorg_count                 INT NOT NULL DEFAULT 0,
    failure_class               TEXT NULL,
    failure_detail              TEXT NULL,
    terminal_at_ms              BIGINT NULL,
    created_at_ms               BIGINT NOT NULL,
    updated_at_ms               BIGINT NOT NULL
);

-- Indexes for hash lookup + phase scans.
CREATE INDEX hybrid_v2_broadcast_state_tx_hash_idx
    ON hybrid_v2_broadcast_state (tx_hash)
    WHERE tx_hash IS NOT NULL;
CREATE INDEX hybrid_v2_broadcast_state_phase_updated_idx
    ON hybrid_v2_broadcast_state (phase, updated_at_ms DESC);

-- Immutability trigger for tx_hash + envelope_hash once set. Modelled
-- on migration 0049's plan_hash / calldata_hash trigger — an attempted
-- overwrite is a hard RAISE EXCEPTION (fail closed).
CREATE OR REPLACE FUNCTION hybrid_v2_broadcast_state_immutability_trigger()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    IF OLD.tx_hash IS NOT NULL AND NEW.tx_hash IS DISTINCT FROM OLD.tx_hash THEN
        RAISE EXCEPTION
            'hybrid_v2_broadcast_state.tx_hash is immutable once set (canonical_execution_id=%): old=% new=%',
            OLD.canonical_execution_id, OLD.tx_hash, NEW.tx_hash;
    END IF;
    IF OLD.envelope_hash IS NOT NULL
       AND NEW.envelope_hash IS DISTINCT FROM OLD.envelope_hash THEN
        RAISE EXCEPTION
            'hybrid_v2_broadcast_state.envelope_hash is immutable once set (canonical_execution_id=%): old=% new=%',
            OLD.canonical_execution_id, OLD.envelope_hash, NEW.envelope_hash;
    END IF;
    IF OLD.envelope_bytes_hash IS NOT NULL
       AND NEW.envelope_bytes_hash IS DISTINCT FROM OLD.envelope_bytes_hash THEN
        RAISE EXCEPTION
            'hybrid_v2_broadcast_state.envelope_bytes_hash is immutable once set (canonical_execution_id=%): old=% new=%',
            OLD.canonical_execution_id, OLD.envelope_bytes_hash, NEW.envelope_bytes_hash;
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER hybrid_v2_broadcast_state_immutability_tr
    BEFORE UPDATE ON hybrid_v2_broadcast_state
    FOR EACH ROW EXECUTE FUNCTION hybrid_v2_broadcast_state_immutability_trigger();
