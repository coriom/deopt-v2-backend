-- PERPS_V2_BACKEND_RPC_SIMULATION_INTEGRATION_V1 (§§1-2)
--
-- Adds the two V2-signed price bounds to `execution_intents` so
-- the persisted row carries the exact 12-field wire shape the trader
-- signed. Before this migration `ExecutionIntent::perp_trade_payload()`
-- hardcoded `max_execution_price_1e8 = 0, min_execution_price_1e8 = 0`
-- (reproducing V1 strict-price semantics), which meant a V2 intent
-- with non-trivial bounds could not be reconstructed after DB reload
-- — the trader's signed bound values would be silently dropped.
--
-- Columns store the 1e8-scaled unsigned integer as TEXT (matching
-- the existing `price_1e8` / `size_1e8` convention on this table)
-- so u128 values round-trip losslessly through sqlx (which does not
-- natively bind u128 to a native SQL integer type wide enough).
--
-- Semantics (per Solidity `PerpMatchingEngineV2._executeSingle`):
--   max_execution_price_1e8 = 0  → NO upper bound enforced
--   min_execution_price_1e8 = 0  → NO lower bound enforced
--   both = 0                     → strict-price V1 reproduction
--
-- DEFAULT '0' back-fills every pre-migration row to strict-price
-- semantics. This is safe because:
--   1. Every pre-migration row is protocol_version = 'perp_v1'
--      (enforced by migration 0064) and V1 signatures do not commit
--      to bounds.
--   2. Any protocol_version = 'perp_v2' row that exists post-migration
--      MUST have been created via the V2 prepare path which persists
--      the trader-signed bounds explicitly.
--
-- Column type NUMERIC would be simpler but the rest of the table
-- uses TEXT for large unsigned integers, and cross-column consistency
-- outweighs the marginal type-safety win.

ALTER TABLE execution_intents
    ADD COLUMN max_execution_price_1e8 TEXT NOT NULL DEFAULT '0';

ALTER TABLE execution_intents
    ADD COLUMN min_execution_price_1e8 TEXT NOT NULL DEFAULT '0';

COMMENT ON COLUMN execution_intents.max_execution_price_1e8 IS
    'PERPS_V2_BACKEND_RPC_SIMULATION_INTEGRATION_V1 — V2 trader-signed
     upper price bound (1e8 scale). 0 = no bound. Immutable
     post-cosign. See PerpMatchingEngineV2.TRADE_TYPEHASH.';
COMMENT ON COLUMN execution_intents.min_execution_price_1e8 IS
    'PERPS_V2_BACKEND_RPC_SIMULATION_INTEGRATION_V1 — V2 trader-signed
     lower price bound (1e8 scale). 0 = no bound. Immutable
     post-cosign.';
