-- PERPS_V2_BACKEND_COMPAT_FOUNDATION_V1
--
-- Adds a durable `protocol_version` tag to every Perps execution
-- intent so a single backend deployment can coexist with both the
-- deployed V1 stack (`PerpEngine` / `PerpMatchingEngine`, EIP-712
-- domain version "1", 10-field `PerpTrade`) and the future V2 stack
-- (`PerpEngineV2` / `PerpMatchingEngineV2`, EIP-712 domain version
-- "2", 12-field `PerpTrade`).
--
-- Invariants this column upholds:
--
--   1. A persisted intent is IMMUTABLY bound to the settlement
--      protocol it was cosigned against. Flipping the runtime
--      `PERPS_ACTIVE_ENGINE_VERSION` MUST NOT retarget an
--      already-signed intent — the trader's EIP-712 signature is
--      only valid for the exact `(name, version, verifyingContract)`
--      that was hashed at cosign time.
--
--   2. Pre-migration rows deterministically back-fill to `perp_v1`.
--      Every intent that existed before this migration was created
--      under the deployed V1 stack; no ambiguity, no operator
--      judgement call required.
--
--   3. Reconciliation / receipt-verification / history reads are
--      version-aware: a row's `protocol_version` selects which
--      emitter, ABI, and typehash the correlator expects.
--
--   4. A cross-version replay is refused at multiple layers: DB
--      CHECK constraint, backend `PerpsProtocolVersion::parse`,
--      digest reconstruction, and executor preflight all agree.
--
-- Rollback is DELETE COLUMN; unsafe if any `perp_v2` rows exist
-- (they would silently degrade to V1 shape). See
-- `docs/PERPS_V2_BACKEND_COMPAT_FOUNDATION_V1.md` §22 for the
-- Base Sepolia cutover ordering.
--
-- Column type is TEXT (not ENUM) to keep migration reversibility
-- cheap and match the existing `status` column convention on this
-- same table.

ALTER TABLE execution_intents
    ADD COLUMN protocol_version TEXT NOT NULL DEFAULT 'perp_v1'
    CHECK (protocol_version IN ('perp_v1', 'perp_v2'));

-- Existing rows already back-fill to `perp_v1` via the DEFAULT
-- clause above; the following statement is an explicit assertion
-- that ties the back-fill to the deployed V1 stack for
-- audit-log reconstruction.
COMMENT ON COLUMN execution_intents.protocol_version IS
    'PERPS_V2_BACKEND_COMPAT_FOUNDATION_V1 — settlement protocol
     version this intent belongs to. Immutable post-cosign.
     Values: `perp_v1` (deployed 10-field PerpTrade, domain
     version "1"), `perp_v2` (future 12-field PerpTrade, domain
     version "2"). Runtime flip of `PERPS_ACTIVE_ENGINE_VERSION`
     MUST NOT retarget an existing row.';

CREATE INDEX IF NOT EXISTS idx_execution_intents_protocol_version
    ON execution_intents (protocol_version);
