-- PERPS_V2_BACKEND_RECONCILIATION_V1 §12–14
--
-- Adds a durable `protocol_version` + `emitter_address` tag to every
-- indexed Perps trade so a mixed V1/V2 event stream can coexist in
-- one table without ambiguity.
--
-- Rationale:
--   V1 and V2 emit BYTE-IDENTICAL `TradeExecuted` signatures (same
--   9-parameter shape; see PerpMatchingEngine.sol:39 and
--   PerpMatchingEngineV2.sol:50). Their keccak256 topic0 is
--   therefore identical. The emitter address is the SOLE
--   generation boundary; without a persisted `protocol_version`
--   tag on each row, downstream consumers reading historical
--   trades could cross-attribute a V2 fill to V1 (or vice versa)
--   and produce mis-mapped economic state.
--
-- Invariants this migration upholds:
--   1. Every pre-migration row deterministically back-fills to
--      `perp_v1` because the deployed indexer has only ever
--      watched the V1 PerpMatchingEngine (see
--      migrations/0004_indexer.sql + PERPS_V2_BACKEND_COMPAT_
--      FOUNDATION_V1 for the historical single-emitter posture).
--   2. Future V2 rows persist with `perp_v2`; a corrupt or unknown
--      value fails closed at the DB CHECK layer.
--   3. `emitter_address` retains the exact log emitter (lower-case
--      hex) so an audit reconstruction can prove which PME
--      generation produced the fill even if the config address
--      book later changes.
--   4. Existing (`chain_id`, `tx_hash`, `log_index`) uniqueness
--      already prevents duplicate ingestion; this migration does
--      not alter that constraint.
--
-- Rollback is DROP COLUMN; safe if the indexer has not ingested
-- any V2 emitter yet.

ALTER TABLE indexed_perp_trades
    ADD COLUMN protocol_version TEXT NOT NULL DEFAULT 'perp_v1'
    CHECK (protocol_version IN ('perp_v1', 'perp_v2'));

-- Emitter address starts nullable so we can back-fill pre-existing
-- rows to a well-defined value derived from the single V1 emitter
-- ledger. In practice `IndexerConfig::perp_matching_engine_address`
-- is the ONE address that ever wrote to this table pre-migration;
-- we back-fill with a sentinel that documents that provenance
-- explicitly and then tighten the column to NOT NULL.
ALTER TABLE indexed_perp_trades
    ADD COLUMN emitter_address TEXT;

-- Every pre-migration row was ingested by the V1-only indexer, so
-- the `emitter_address` for those rows is exactly the V1 PME the
-- runtime was configured against at ingest time. We do not know
-- that address purely from SQL (it lived in env), so we back-fill
-- to a well-defined 'legacy_v1' sentinel that:
--   * satisfies NOT NULL,
--   * cannot be confused with a valid on-chain address,
--   * documents that the row predates emitter tagging.
-- Post-migration inserts persist the real lowercase hex.
UPDATE indexed_perp_trades
    SET emitter_address = 'legacy_v1'
    WHERE emitter_address IS NULL;

ALTER TABLE indexed_perp_trades
    ALTER COLUMN emitter_address SET NOT NULL;

COMMENT ON COLUMN indexed_perp_trades.protocol_version IS
    'PERPS_V2_BACKEND_RECONCILIATION_V1 — settlement generation
     this indexed trade belongs to. Derived from the log emitter
     (`emitter_address`), NEVER from a runtime config value.
     V1 and V2 emit identical TradeExecuted signatures — this
     column is the only downstream disambiguator.';

COMMENT ON COLUMN indexed_perp_trades.emitter_address IS
    'PERPS_V2_BACKEND_RECONCILIATION_V1 — the exact log emitter
     address (lower-case hex) that produced this row. Persisted so
     an audit reconstruction can prove which PME produced the fill.
     Pre-migration rows are stamped with the ''legacy_v1'' sentinel
     because their emitter was not durably captured at ingest.';

CREATE INDEX IF NOT EXISTS idx_indexed_perp_trades_protocol_version
    ON indexed_perp_trades (protocol_version);
CREATE INDEX IF NOT EXISTS idx_indexed_perp_trades_emitter_address
    ON indexed_perp_trades (emitter_address);
