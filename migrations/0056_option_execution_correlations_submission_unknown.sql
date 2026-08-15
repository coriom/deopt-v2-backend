-- OPTIONS-HYBRID-V2-BACKEND-CLOSURE-SPRINT-V1 Part A1 — extend the
-- correlation state machine with `SUBMISSION_UNKNOWN`.
--
-- Motivation: the pre-existing state machine went
--   AWAITING_CHAIN_EVIDENCE -> SUBMITTED
-- only AFTER `provider.send_raw_transaction` returned successfully.
-- This left a crash window between "RPC accepted the raw tx" (which
-- means the tx may be on chain) and "backend persisted the tx_hash"
-- (which is what the correlation needs to bind to canonical
-- evidence).
--
-- The new state `SUBMISSION_UNKNOWN` is entered BEFORE calling
-- `eth_sendRawTransaction` with the tx_hash the backend computed
-- LOCALLY from the exact signed raw transaction bytes
-- (`derive_signed_transaction_hash`). If the process crashes after
-- this point but before the RPC completes, the correlation still
-- carries the authoritative tx_hash and the reducer can bind
-- canonical evidence via `(tx_hash, log_index)` — no evidence is
-- ever lost.
--
-- Full transition graph:
--   AWAITING_CHAIN_EVIDENCE
--       -> SUBMISSION_UNKNOWN  (local tx_hash persisted; RPC not yet
--                               known to have accepted)
--       -> SUBMITTED           (either directly on RPC ack + hash
--                               agreement, or from SUBMISSION_UNKNOWN
--                               on subsequent confirmation)
--   SUBMISSION_UNKNOWN
--       -> SUBMITTED
--       -> CORRELATED_CANONICAL (reducer ingest can promote directly
--                               if canonical evidence arrives
--                               before local ack — the crash-window
--                               case this migration enables)
--   [rest unchanged]
--
-- Sparse UNIQUE index scope widens to include SUBMISSION_UNKNOWN so
-- concurrent AWAITING/SUBMISSION_UNKNOWN inserts remain mutually
-- exclusive per canonical_execution_id.
--
-- Immutability trigger unchanged — the new state has the same
-- immutability posture as the others (canonical_execution_id and
-- identity fields cannot mutate; on-chain fingerprints follow the
-- NULL -> set once rule).

-- Widen the CHECK constraint on correlation_status.
ALTER TABLE option_execution_correlations
    DROP CONSTRAINT IF EXISTS option_execution_correlations_correlation_status_check;

ALTER TABLE option_execution_correlations
    ADD CONSTRAINT option_execution_correlations_correlation_status_check
    CHECK (correlation_status IN (
        'AWAITING_CHAIN_EVIDENCE',
        'SUBMISSION_UNKNOWN',
        'SUBMITTED',
        'CORRELATED_CANONICAL',
        'ORPHANED',
        'CONFLICT',
        'MANUAL_REVIEW'
    ));

-- Widen the sparse UNIQUE index on canonical_execution_id to include
-- SUBMISSION_UNKNOWN. The index enforces at-most-one ACTIVE
-- correlation per canonical execution — SUBMISSION_UNKNOWN is
-- ACTIVE (identity still binds to the pending settlement).
DROP INDEX IF EXISTS ux_option_execution_correlations_canonical_execution;
CREATE UNIQUE INDEX IF NOT EXISTS ux_option_execution_correlations_canonical_execution
    ON option_execution_correlations (canonical_execution_id)
    WHERE correlation_status IN (
        'AWAITING_CHAIN_EVIDENCE',
        'SUBMISSION_UNKNOWN',
        'SUBMITTED',
        'CORRELATED_CANONICAL'
    );

-- The `(tx_hash, log_index)` sparse UNIQUE index intentionally stays
-- scoped to CORRELATED_CANONICAL only — SUBMISSION_UNKNOWN and
-- SUBMITTED rows may share a tx_hash across deployments (see Part H
-- policy in the atomic-wiring milestone doc). Only fully-correlated
-- canonical rows are guaranteed to be pinned to a single log.
--
-- Deployment_status lookup index is unchanged (it does not filter by
-- status).
