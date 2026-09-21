-- PERPS_V2_BACKEND_RECONCILIATION_V1 §§2, 5, 8
--
-- Makes an already-prepared broadcast row durably identify its
-- settlement generation. Before this migration a persisted row
-- in `execution_intent_broadcasts` carried only `target_address`
-- (byte-identical to the on-chain `to`), which was sourced from
-- `ExecutionConfig.perp_matching_engine_address` at prepare time
-- and — critically — is not tagged with which PME generation
-- (V1 vs V2) the row belongs to. That coupling was safe only
-- because the backend had a single active version. A future
-- runtime cutover to V2 (or a rollback back to V1) would flip
-- `perp_matching_engine_address` under a persisted row and let
-- the reconciler re-derive an expected receipt emitter from a
-- config value that no longer represents the generation the
-- row was signed and prepared against.
--
-- This migration closes that gap by persisting two additional
-- fields directly on the durable broadcast row:
--
--   * `protocol_version` — the settlement generation (`perp_v1`
--     or `perp_v2`) as of prepare time. Must equal the parent
--     intent's `execution_intents.protocol_version` at insert.
--
--   * `expected_emitter` — the exact PME address the row expects
--     to see in receipt logs (matched case-insensitively by
--     `verify_pme_event_in_receipt`). Historically this was
--     identical to `target_address` because the executor's `to`
--     is always the same PME that emits `TradeExecuted`; we
--     persist it as its own column to make the receipt-binding
--     contract explicit and to keep the door open for future
--     PME topologies where `to` != emitter (e.g., a router in
--     front of PME).
--
-- Invariants this migration upholds:
--
--   1. A persisted broadcast row is IMMUTABLY bound to the
--      generation and emitter it was prepared against. A
--      runtime flip of `PERPS_ACTIVE_ENGINE_VERSION` MUST NOT
--      retarget an existing row's expected receipt emitter or
--      reinterpret its target.
--
--   2. Every pre-migration row deterministically back-fills to
--      `protocol_version = 'perp_v1'` and
--      `expected_emitter = target_address`. Both defaults are
--      correct: pre-migration all broadcasts belonged to the
--      deployed V1 stack, and for the deployed PME `to` is
--      always the same address that emits the settlement event.
--
--   3. Reconciliation / receipt-verification / rebroadcast paths
--      read `expected_emitter` from the persisted row, never
--      from the runtime config. A flip of active version can
--      never retarget the correlation surface.
--
--   4. Unknown / corrupt persisted `protocol_version` fails
--      closed at the DB CHECK layer.
--
-- Rollback is DROP COLUMN; safe if no V2 rows have been
-- broadcast yet (see PERPS_V2_BACKEND_ANVIL_BROADCAST_E2E_V1).

ALTER TABLE execution_intent_broadcasts
    ADD COLUMN protocol_version TEXT NOT NULL DEFAULT 'perp_v1'
    CHECK (protocol_version IN ('perp_v1', 'perp_v2'));

-- `expected_emitter` starts as NULL to allow the two-phase
-- back-fill below without introducing a placeholder value that
-- could later leak into a real reconciliation branch. After the
-- back-fill the column is tightened to NOT NULL.
ALTER TABLE execution_intent_broadcasts
    ADD COLUMN expected_emitter TEXT;

UPDATE execution_intent_broadcasts
    SET expected_emitter = target_address
    WHERE expected_emitter IS NULL;

ALTER TABLE execution_intent_broadcasts
    ALTER COLUMN expected_emitter SET NOT NULL;

COMMENT ON COLUMN execution_intent_broadcasts.protocol_version IS
    'PERPS_V2_BACKEND_RECONCILIATION_V1 — settlement protocol
     generation (`perp_v1` | `perp_v2`) this broadcast row was
     prepared against. Immutable post-insert. A runtime flip of
     PERPS_ACTIVE_ENGINE_VERSION MUST NOT retarget this row.';

COMMENT ON COLUMN execution_intent_broadcasts.expected_emitter IS
    'PERPS_V2_BACKEND_RECONCILIATION_V1 — exact on-chain address
     the reconciler expects to see in receipt logs
     (`TradeExecuted` emitter). Persisted separately from
     `target_address` so a future `to != emitter` topology can be
     represented; today the two are equal for PerpMatchingEngine
     calls. Compared case-insensitively.';

CREATE INDEX IF NOT EXISTS idx_execution_intent_broadcasts_protocol_version
    ON execution_intent_broadcasts (protocol_version);
