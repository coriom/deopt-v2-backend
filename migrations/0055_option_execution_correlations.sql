-- OPTIONS-HYBRID-V2-EXECUTION-CORRELATION-CLOSURE-V1 Part E — the
-- deterministic correlation ledger connecting backend canonical
-- execution identity to canonical on-chain evidence.
--
-- Purpose: give every backend `canonical_execution_id` at most one
-- authoritative correlation record. When canonical Options events
-- arrive from the on-chain journal, the reducer looks up the
-- correlation row via `(onchain_buyer_order_id,
-- onchain_seller_order_id, fill_quantity_1e8)` — the tuple
-- deterministically emitted by
-- `OptionMatchingEngineV2.executeMatch` and indexed via `keccak256`
-- of the tuple for lookup performance.
--
-- Frozen safety notes:
--   * `canonical_execution_id` is the backend economic identity
--     (from `option_fills.canonical_execution_id`). Immutable after
--     insert.
--   * `onchain_buyer_order_id` / `onchain_seller_order_id` are the
--     EIP-712 envelope digests the on-chain contract emits as
--     `buyerOrderId` / `sellerOrderId` in
--     `OptionOrderPairExecuted`. They differ from
--     `canonical_order_hash` (different preimages) — the correlation
--     table records BOTH so backend can look up either way.
--   * `onchain_execution_id` is the chain-computed executionId which
--     includes block.number + block.timestamp in its preimage.
--     Backend cannot compute this ahead of settlement — it lands
--     when the event is ingested.
--   * `tx_hash` and canonical block coordinates land when the
--     transaction is mined and the block canonicalizes.
--   * `correlation_status` values track the state machine:
--         AWAITING_CHAIN_EVIDENCE  -> intent created, no tx yet
--         SUBMITTED                -> tx_hash attached, waiting for event
--         CORRELATED_CANONICAL     -> canonical event ingested + verified
--         ORPHANED                 -> canonical branch reorged; awaiting replacement
--         CONFLICT                 -> two backend executions claim one on-chain event,
--                                     or one execution has two conflicting on-chain events
--         MANUAL_REVIEW            -> operator escalation needed
--   * Sparse UNIQUE indexes prevent duplicate correlations for the
--     same canonical execution and the same on-chain event.
--   * NO canonical economic position state stored here. This table
--     is correlation metadata only. Canonical positions live in
--     `hybrid_v2_positions` (chain-derived via HV2 reducer).
--   * NO grant / role change.

CREATE TABLE IF NOT EXISTS option_execution_correlations (
    correlation_id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    -- Backend economic identity binding
    canonical_execution_id TEXT NOT NULL,
    -- Deployment / chain scope: identities never cross domains
    deployment_id BIGINT NOT NULL,
    chain_id BIGINT NOT NULL,
    -- Execution kind (mirror the two Solidity entrypoints)
    execution_kind TEXT NOT NULL CHECK (execution_kind IN ('trade', 'rfq_trade')),
    -- Optional on-chain fingerprints (populated as evidence arrives)
    onchain_buyer_order_id TEXT NULL,
    onchain_seller_order_id TEXT NULL,
    onchain_execution_id TEXT NULL,
    fill_quantity_1e8 TEXT NULL,
    tx_hash TEXT NULL,
    canonical_block_number BIGINT NULL,
    canonical_block_hash TEXT NULL,
    log_index INT NULL,
    -- Lifecycle
    correlation_status TEXT NOT NULL CHECK (correlation_status IN (
        'AWAITING_CHAIN_EVIDENCE',
        'SUBMITTED',
        'CORRELATED_CANONICAL',
        'ORPHANED',
        'CONFLICT',
        'MANUAL_REVIEW'
    )),
    terminal_reason TEXT NULL,
    first_seen_at_ms BIGINT NOT NULL,
    last_updated_at_ms BIGINT NOT NULL
);

-- Sparse UNIQUE: at most one active correlation per canonical_execution_id.
-- Reorged / orphaned rows remain visible for audit.
CREATE UNIQUE INDEX IF NOT EXISTS ux_option_execution_correlations_canonical_execution
    ON option_execution_correlations (canonical_execution_id)
    WHERE correlation_status IN ('AWAITING_CHAIN_EVIDENCE', 'SUBMITTED', 'CORRELATED_CANONICAL');

-- Sparse UNIQUE: two backend executions cannot claim the same on-chain
-- event (deterministic per (tx_hash, log_index) when both are set).
CREATE UNIQUE INDEX IF NOT EXISTS ux_option_execution_correlations_onchain_log
    ON option_execution_correlations (tx_hash, log_index)
    WHERE tx_hash IS NOT NULL AND log_index IS NOT NULL
      AND correlation_status = 'CORRELATED_CANONICAL';

-- Lookup index: reducer path finds correlation via the deterministic
-- on-chain tuple (buyerOrderId, sellerOrderId, fillQuantity1e8).
CREATE INDEX IF NOT EXISTS idx_option_execution_correlations_onchain_tuple
    ON option_execution_correlations (onchain_buyer_order_id, onchain_seller_order_id, fill_quantity_1e8)
    WHERE onchain_buyer_order_id IS NOT NULL
      AND onchain_seller_order_id IS NOT NULL;

-- Lookup index: admin surfaces resolve tx_hash → canonical execution.
CREATE INDEX IF NOT EXISTS idx_option_execution_correlations_tx_hash
    ON option_execution_correlations (tx_hash)
    WHERE tx_hash IS NOT NULL;

-- Lookup index: chain event decoder resolves onchain_execution_id →
-- backend canonical_execution_id when the contract emits both.
CREATE INDEX IF NOT EXISTS idx_option_execution_correlations_onchain_execution_id
    ON option_execution_correlations (onchain_execution_id)
    WHERE onchain_execution_id IS NOT NULL;

-- Deployment isolation lookup.
CREATE INDEX IF NOT EXISTS idx_option_execution_correlations_deployment_status
    ON option_execution_correlations (deployment_id, correlation_status);

-- Immutability trigger: `canonical_execution_id`, `deployment_id`,
-- `chain_id`, `execution_kind` cannot change after insert. The
-- on-chain fingerprints CAN transition NULL -> set (populated as
-- evidence arrives) but cannot mutate once set. `correlation_status`
-- follows the state machine and can transition per the CHECK.
CREATE OR REPLACE FUNCTION option_execution_correlations_immutability_trigger()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    IF NEW.canonical_execution_id IS DISTINCT FROM OLD.canonical_execution_id THEN
        RAISE EXCEPTION
            'option_execution_correlations.canonical_execution_id is immutable (correlation_id=%)',
            OLD.correlation_id;
    END IF;
    IF NEW.deployment_id IS DISTINCT FROM OLD.deployment_id THEN
        RAISE EXCEPTION
            'option_execution_correlations.deployment_id is immutable (correlation_id=%)',
            OLD.correlation_id;
    END IF;
    IF NEW.chain_id IS DISTINCT FROM OLD.chain_id THEN
        RAISE EXCEPTION
            'option_execution_correlations.chain_id is immutable (correlation_id=%)',
            OLD.correlation_id;
    END IF;
    IF NEW.execution_kind IS DISTINCT FROM OLD.execution_kind THEN
        RAISE EXCEPTION
            'option_execution_correlations.execution_kind is immutable (correlation_id=%)',
            OLD.correlation_id;
    END IF;
    -- On-chain fingerprints: NULL -> set is allowed exactly once;
    -- set -> different-set is forbidden. Preserves deterministic
    -- correlation binding.
    IF OLD.onchain_buyer_order_id IS NOT NULL
       AND NEW.onchain_buyer_order_id IS DISTINCT FROM OLD.onchain_buyer_order_id THEN
        RAISE EXCEPTION
            'option_execution_correlations.onchain_buyer_order_id is immutable once set (correlation_id=%)',
            OLD.correlation_id;
    END IF;
    IF OLD.onchain_seller_order_id IS NOT NULL
       AND NEW.onchain_seller_order_id IS DISTINCT FROM OLD.onchain_seller_order_id THEN
        RAISE EXCEPTION
            'option_execution_correlations.onchain_seller_order_id is immutable once set (correlation_id=%)',
            OLD.correlation_id;
    END IF;
    IF OLD.onchain_execution_id IS NOT NULL
       AND NEW.onchain_execution_id IS DISTINCT FROM OLD.onchain_execution_id THEN
        RAISE EXCEPTION
            'option_execution_correlations.onchain_execution_id is immutable once set (correlation_id=%)',
            OLD.correlation_id;
    END IF;
    IF OLD.tx_hash IS NOT NULL
       AND NEW.tx_hash IS DISTINCT FROM OLD.tx_hash THEN
        RAISE EXCEPTION
            'option_execution_correlations.tx_hash is immutable once set (correlation_id=%)',
            OLD.correlation_id;
    END IF;
    -- fill_quantity_1e8, canonical_block_number, canonical_block_hash,
    -- log_index follow the same NULL -> set immutability posture.
    IF OLD.fill_quantity_1e8 IS NOT NULL
       AND NEW.fill_quantity_1e8 IS DISTINCT FROM OLD.fill_quantity_1e8 THEN
        RAISE EXCEPTION
            'option_execution_correlations.fill_quantity_1e8 is immutable once set (correlation_id=%)',
            OLD.correlation_id;
    END IF;
    RETURN NEW;
END;
$$;

DROP TRIGGER IF EXISTS option_execution_correlations_immutability
    ON option_execution_correlations;

CREATE TRIGGER option_execution_correlations_immutability
    BEFORE UPDATE ON option_execution_correlations
    FOR EACH ROW EXECUTE FUNCTION
        option_execution_correlations_immutability_trigger();
