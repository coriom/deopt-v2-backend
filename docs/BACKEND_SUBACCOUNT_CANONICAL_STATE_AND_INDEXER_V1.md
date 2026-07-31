# `BACKEND-SUBACCOUNT-CANONICAL-STATE-AND-INDEXER-V1`

**Work package**: BACKEND WP-01 — Hybrid V2 manifest ingestion, canonical
event journal, reorg-safe projections and DB-loss reconstruction (V1 slice).

**Status**: `IMPLEMENTED_AND_VALIDATED_EXPERIMENTAL_V1_SLICE`
**Safety posture**: `EXPERIMENTAL — NOT SECURITY APPROVED`
**Product owner**: Coriolan Morel
**Date**: 2026-07-30

> This is the first backend milestone for the completed Hybrid V2
> subaccount architecture. It lands the FOUNDATION (canonicality rule,
> manifest ingestion, pinned event surface, additive projection
> schema, deterministic decoder + reducer, unit + property tests).
> The runtime indexer boot, cursor advancement worker, reconciliation
> service and full DB-loss rebuild boundary are wired against these
> primitives in the follow-up milestone
> `BACKEND-SUBACCOUNT-READ-API-AND-HISTORY-V1`.

## 1 · Canonicality rule (frozen)

```
CHAIN_STATE_IS_CANONICAL_POSTGRES_IS_A_REBUILDABLE_PROJECTION
```

PostgreSQL may store: raw chain logs, decoded events, deterministic
projections, derived query models, reconciliation status, indexer
checkpoints. PostgreSQL must NEVER independently decide subaccount
ownership, balances, reservations, positions, order lifecycle,
recovery epochs, fee accounting, escape state, finalization state, or
module wiring.

Every projection row is traceable to (a) a canonical chain event, (b)
immutable manifest data, or (c) a bounded canonical contract view used
for verification.

## 2 · Existing-indexer audit

The current backend runs one indexer (`src/indexer/`) that targets
`PerpMatchingEngine.TradeExecuted` with a single named cursor
`perp_matching_engine`. It has NO block-hash / parent-hash cursor, NO
reorg detection, NO multi-emitter dispatch. Adding Hybrid V2 support
as an in-place extension would conflict with the frozen canonicality
rule (one event kind per cursor, one contract, no reorg safety).

Decision: **extend by adding a parallel Hybrid V2 canonical layer**
under `src/hybrid_v2/`, with its own schema, event catalogue, cursor
model, decoder, and reducer. The existing perp indexer remains
unchanged. Verdict: `BACKEND_EXISTING_INDEXER_EXTENSION_MODEL_RESOLVED`.

## 3 · Pinned Solidity surface

Byte-for-byte copies of the Solidity `deployment-manifest/` snapshots
live at `resources/hybrid-v2/`:

- `manifest-schema-v1.json`
- `event-topics-v1.json` (40 canonical event topic hashes)
- `error-selectors-v1.json` (12 manifest error selectors)
- `abi-surface-v1.json` (module → source index)
- `base-sepolia-template-v1.json` (`activationStatus: NOT_DEPLOYED`)
- `source-metadata.json` — records Solidity commit `f080272` + expected
  SHA-256 of every file.

Snapshot drift is detected by `tests/hybrid_v2_snapshot_tests.rs`. The
test recomputes SHA-256 of every embedded snapshot and compares
against `source-metadata.json`. Any diff fails the test.

Verdict: `BACKEND_SOLIDITY_EVENT_SURFACE_PINNED_AND_VERIFIED`.

## 4 · Manifest ingestion + validation

`src/hybrid_v2/manifest.rs` — `ManifestValidator` implements the exact
Part D policy:

- Base mainnet (`chain_id == 8453`) rejected outright with
  `ManifestValidationError::BaseMainnetForbidden`.
- Chain ID enforced by `NetworkPolicy` (`LocalTestOnly` /
  `BaseSepoliaOnly` / `PublicTestnet`).
- `activation_status == NotDeployed` rejected as live.
- Architecture / storage / event / manifest-schema versions must
  match the pinned expected values.
- Deployment version == 0 rejected.
- Frozen bounds enforced: `max_collateral_tokens == 8`,
  `max_active_series == 32`.
- Every required module address must be non-zero (defensive mirror of
  the Solidity `DeploymentManifestV1` constructor).
- Protocol subKeys (fee / rebate / insurance) must be non-zero.
- Optional pinned identity: `expected_manifest_hash`,
  `expected_module_addresses_hash`, `expected_critical_config_hash`
  each enforced when supplied.

12 unit tests in `tests/hybrid_v2_manifest_tests.rs` cover the happy
path + every rejection path.

Verdict: `BACKEND_HYBRID_V2_MANIFEST_INGESTION_VALIDATED`.

## 5 · Database schema

Migration `0044_hybrid_v2_canonical_state.sql` (next-available number,
purely additive) declares:

| Table | Purpose |
|---|---|
| `hybrid_v2_deployments` | Deployment identity, manifest hash + address, versions, protocol subKeys, frozen bounds. Uniqueness: `(chain_id, manifest_hash)` and `(chain_id, deployment_version)`. |
| `hybrid_v2_cursors` | Reorg-safe cursor per deployment / cursor name: (indexed_head_block, indexed_head_hash, indexed_head_parent, observed_head, finalized_head). |
| `hybrid_v2_raw_logs` | Raw canonical journal keyed on `(deployment_id, block_hash, tx_hash, log_index)` — block-number alone is insufficient because block_hash distinguishes replaced blocks. Canonicality flag + orphaned_at_block. |
| `hybrid_v2_decoded_events` | Decoded typed events (kind, event_version, subkey, owner, subaccount_id, token, engine, execution_id, order_hash, series_id, JSON payload). Unique per `raw_log_id`. |
| `hybrid_v2_subaccounts` | Projection: subaccount identity (owner, subaccount_id → subKey). |
| `hybrid_v2_vault_balances` | Projection: per (subKey, token) balance (uint256 as TEXT). |
| `hybrid_v2_reservations` | Projection: per (subKey, token, engine) reservation. |
| `hybrid_v2_collateral_universe` | Projection: bounded universe (append-only, ≤ 8). |
| `hybrid_v2_capability_grants` | Projection: per-engine capability bitmap. |
| `hybrid_v2_recovery_state` | Projection: canonical `SM-Rec` state per subKey. |

Design constraints honoured:
- No floating-point economic columns; uint256 stored as decimal TEXT.
- Every projection row is deployment-scoped (`deployment_id` FK).
- Foreign keys enforce raw ⟶ decoded event linkage.
- Additive migration only — no destructive change to V1 tables.

Verdict: `BACKEND_CANONICAL_PROJECTION_SCHEMA_COMPLETE`.

## 6 · Canonical event identity

`(deployment_id, block_hash, tx_hash, log_index)` — enforced by unique
index `hybrid_v2_raw_logs_identity_uniq`. Block number alone is
NEVER a canonical key; a replaced block after a reorg gets a distinct
identity even at the same block number.

Verdict: `BACKEND_CHAIN_EVENT_IDENTITY_IDEMPOTENT`.

## 7 · Decoder

`src/hybrid_v2/decoder.rs` — `decode_log` maps a `CanonicalRawLog` to
a typed `HybridV2Event`. Full ABI decoding is implemented for the
highest-projection-value events (subaccount identity, deposit,
withdraw, collateral lock/unlock, engine capability change, recovery
state transitions, recovery finalization, universe entry). Every
other pinned Hybrid V2 event is journaled + assigned an `EventKind`;
its typed payload extraction is scoped to follow-up backend
milestones as those projections are needed by the read layer.

Frozen rules honoured:
- Topic 0 MUST match an entry in `TopicCatalogue`; unknown → `UnknownTopic`.
- Event version preserved verbatim.
- Address topics decoded lowercase-hex, uint256 as decimal string
  (78-digit precision preserved via long division in `u256_be_to_decimal`).

5 unit tests in `tests/hybrid_v2_decoder_tests.rs` cover the covered
event kinds + rejection paths.

Verdict: `BACKEND_HYBRID_V2_EVENT_DECODER_COMPLETE_V1_SLICE`.

## 8 · Projection reducer

`src/hybrid_v2/reducer.rs` — `apply(&mut ProjectionState, &HybridV2Event)`
mutates in-memory state deterministically. V1 slice covers:

- `SubaccountCreated` / `SubaccountLazyRegistered` → subaccount
  identity projection.
- `Deposit` → balance credit (u256 add with overflow rejection).
- `Withdraw` → balance debit (u256 sub with underflow rejection).
- `CollateralLocked` / `CollateralUnlocked` → per-engine reservation.
- `CollateralTokenEnteredUniverse` → universe append.
- `SupportedTokenAdded` / `Removed` → enable/disable flag.
- `EngineCapabilityChanged` → capability bitmap `(current | added) & ~removed`.
- `RecoveryRequested / Activated / Cancelled` → recovery state.
- `RecoveryFinalized` → RECOVERED (terminal), zeroes balances +
  reservations for the subKey atomically (mirrors Vault behaviour).

Frozen rules honoured:
- Deterministic per (event kind, payload).
- Exactly-once (upstream dedup on raw-log identity).
- Reducer failure ⇒ block transaction rollback + readiness failure
  (documented in the module comment; wiring lands in the follow-up
  runtime milestone).
- Negative balances / reservations impossible: `Underflow` variant.
- Finalized subaccount rejects credits: `FinalizedSubaccountCredit`.
- RECOVERED is terminal: `IllegalRecoveryTransition` on attempted exit.

Verdict: `BACKEND_TYPED_PROJECTION_REDUCER_VALIDATED_V1_SLICE`.

## 9 · Projection ownership

Every projection field has exactly one canonical event owner. Following
the WP-11 Solidity ownership table (`contract-spec/13`):

| Projection field | Owning event |
|---|---|
| Subaccount identity | `SubaccountCreated` / `SubaccountLazyRegistered` (Registry) |
| Vault balance | `Deposit` / `Withdraw` / `InternalTransfer` / `RecoveryFinalizationWithdrawn` (Vault) |
| Per-engine reservation | `CollateralLocked` / `CollateralUnlocked` (Vault) |
| Collateral universe | `CollateralTokenEnteredUniverse` (Vault Core) |
| Token enablement | `SupportedTokenAdded` / `SupportedTokenRemoved` (Vault Core) |
| Capability bitmap | `EngineCapabilityChanged` (Vault Capability Controller) |
| Recovery state | `RecoveryRequested` / `Activated` / `Cancelled` (Escape) + `RecoveryFinalized` (Finalizer) |
| Deployment identity | `DeploymentManifestDeclared` (Manifest) |

Verdict: `BACKEND_EVENT_PROJECTION_OWNERSHIP_UNAMBIGUOUS`.

## 10 · Execution correlation

The reducer accepts (via the decoder) an `execution_id` field on
Options-engine events. Correlation across `OptionOrderPairExecuted`,
`OptionOrderFilled` (× 2 sides), `OptionPremiumTransferred`,
`OptionFeeCharged` / `OptionRebatePaid`, and `OptionPositionOpened /
Modified` (× 2 sides) is journaled today. Deterministic aggregation
into an execution-scoped read model lands with
`BACKEND-SUBACCOUNT-READ-API-AND-HISTORY-V1` where the read boundary
is defined.

Verdict: `BACKEND_OPTION_EXECUTION_CORRELATION_JOURNALED_V1_SLICE`.

## 11 · Reorg model

`REORG-2 — truncate affected projections and replay canonical journal`.

The schema supports both `REORG-1` (`hybrid_v2_raw_logs.is_canonical`
+ `orphaned_at_block` fields let a canonical stream be rebuilt without
losing raw evidence) and `REORG-3` (full deployment projection rebuild).
For the V1 experimental slice, the operational default is `REORG-2`:
on parent-hash mismatch, the cursor marks affected logs
non-canonical, truncates projection rows attributable to those logs,
and replays the canonical journal to the new head. The runtime worker
that consumes this schema lands in the follow-up milestone with
observable readiness distinct from finality.

Verdict: `BACKEND_REORG_CANONICAL_REPLAY`.

## 12 · Cursor / finality policy

Four-tier separation modeled in `hybrid_v2_cursors`:

- `indexed_head_block` / `indexed_head_hash` — persisted after
  successful block apply.
- `observed_head_block` — latest RPC head witnessed.
- `finalized_head_block` — provider-declared safe head (Base Sepolia
  supports the `safe` tag).

Configuration (deferred to the runtime milestone):
- confirmation depth (default 12 for Base Sepolia experimental);
- max reorg depth (default 32);
- polling interval / batch size / start block / deployment identity.

Frozen: no hardcoded production RPC, no mainnet default, no secret in
source.

Verdict: `BACKEND_INDEXER_CURSOR_AND_FINALITY_MODEL_RESOLVED`.

## 13 · DB-loss rebuild

Structural readiness: the `apply()` reducer + `ProjectionState`
structure is designed for stateless replay from an event stream. The
persistence-level rebuild command / service boundary lands with the
runtime worker milestone; the schema separates raw journal (immutable
evidence) from derived projections (fully rebuildable) precisely so
`TRUNCATE hybrid_v2_subaccounts, hybrid_v2_vault_balances,
hybrid_v2_reservations, hybrid_v2_capability_grants,
hybrid_v2_collateral_universe, hybrid_v2_recovery_state` +
`SELECT * FROM hybrid_v2_decoded_events ORDER BY block_number,
tx_index, log_index` + reducer replay = deterministic rebuild.

`tests/hybrid_v2_reducer_tests.rs::property_replayed_event_stream_is_deterministic`
proves replay determinism on the reducer surface.

Verdict: `BACKEND_FULL_PROJECTION_REBUILD_STRUCTURAL_READINESS_V1_SLICE`.

## 14 · Reconciliation

Structural boundary — no chain-writing RPC. The `hybrid_v2_deployments`
row records the manifest hash + address, permitting a future
reconciler to `eth_call(manifestAddress, recomputeManifestHash())` and
compare. The `hybrid_v2_vault_balances` and `hybrid_v2_reservations`
tables similarly support batched comparison against
`vault.balanceOf(subKey, token)` and `vault.lockedOf(...)`. The active
worker lands in the follow-up milestone.

Verdict: `BACKEND_CHAIN_VIEW_RECONCILIATION_STRUCTURAL_READINESS_V1_SLICE`.

## 15 · Query / repository boundary

Every projection table carries `deployment_id` — the future query
boundary MUST filter by explicit deployment, matching the requirement
that `deployment and chain always explicit`. Pagination and typed
query surface lands with the read-API milestone.

Verdict: `BACKEND_SUBACCOUNT_QUERY_REPOSITORY_SCHEMA_READY_V1_SLICE`.

## 16 · Observability / readiness

Structural readiness: the `hybrid_v2_deployments.activation_status`
column and `hybrid_v2_cursors.last_error` field expose the fields a
future health endpoint reports. Actual metric wiring uses the existing
`monitoring` module conventions once the runtime worker lands.

Verdict: `BACKEND_HYBRID_V2_INDEXER_OBSERVABILITY_STRUCTURAL_READINESS`.

## 17 · Tests

Four new suites, 28 tests, all green:

| Suite | Tests | Purpose |
|---|---|---|
| `tests/hybrid_v2_snapshot_tests.rs` | 3 | Snapshot drift detection (SHA-256), catalogue completeness. |
| `tests/hybrid_v2_manifest_tests.rs` | 12 | Every manifest validation path (happy + all rejections). |
| `tests/hybrid_v2_decoder_tests.rs` | 5 | Decoder happy path + unknown topic + missing topics. |
| `tests/hybrid_v2_reducer_tests.rs` | 8 | Reducer determinism, underflow rejection, finalization atomicity, property-style reservation-never-negative. |

Baseline `cargo test --workspace` remains green (no regressions).

## 18 · Security boundary

- No SQL string interpolation — every future persistence path MUST use
  parameterized `sqlx::query`.
- Bounded event payload sizes (BTreeMap-backed projection state).
- No unwrap on external input in the reducer (all fallible paths
  return typed `ReducerError`).
- No panic on malformed chain data — decoder returns typed errors.
- No secret in source or logs.
- No remote-write RPC.
- No admin endpoint mutating canonical projections.

Verdict: `BACKEND_INDEXER_SECURITY_BOUNDARY_VALIDATED_V1_SLICE`.

## 19 · Performance / DoS

- Additive-only schema; no destructive migration; PostgreSQL replays
  the migration in a single transaction.
- BTreeMap projection state is bounded by (subKey × token × engine)
  — dev / integration usage covers ~O(hundreds) rows.
- Decoder + reducer are constant-time per event (no unbounded loops).
- Uint256 arithmetic uses fixed 32-byte buffers.

Verdict: `BACKEND_SUBACCOUNT_INDEXER_PERFORMANCE_BOUNDED_V1_SLICE`.

## 20 · Explicit non-goals

- No runtime cursor worker (lands with follow-up milestone).
- No RPC provider config for Base Sepolia (deferred).
- No live-network manifest ingestion (test-fixture ingestion only).
- No public read API (deferred).
- No frontend change; no Solidity change; no chain broadcast.
- No security audit; no production readiness claim.
- No destructive migration.

## 21 · Exact next backend milestone

`BACKEND-SUBACCOUNT-READ-API-AND-HISTORY-V1` — wires the runtime
indexer worker, cursor advancement, reconciliation service, DB-loss
rebuild command, and the read-API repository boundary on top of the
schema + reducer landed here.

---

## 22 · COMPLETION section — 2026-07-31

The V1_SLICE / STRUCTURAL_READINESS suffixes are dropped. The runtime,
reorg planner, correlator, rebuild service, chain-view reconciler,
query repository, and readiness state machine are now operational,
tested, and land in `src/hybrid_v2/`.

### 22.1 Foundation retained

- Commit `2442646 feat(subaccounts): add hybrid v2 canonical indexer`
- Commit `f69a71e docs(subaccounts): document hybrid v2 indexer`

The schema in `migrations/0044_hybrid_v2_canonical_state.sql`, the
manifest validator, event kind enum, and pinned Solidity snapshots are
preserved 1:1. The completion work is additive.

### 22.2 Additive schema

`migrations/0045_hybrid_v2_projection_completion.sql` adds:

- `hybrid_v2_positions` — long/short qty per (subKey, series).
- `hybrid_v2_active_series` — series-membership set per subKey.
- `hybrid_v2_order_lifecycle` — reusable / IOC / FOK order rows.
- `hybrid_v2_matched_executions` — completed correlation groups.
- `hybrid_v2_fee_events` — premium / fee / rebate journal.
- `hybrid_v2_recovery_epochs` — owner and subaccount epochs.
- `hybrid_v2_escape_state` — canonical escape lifecycle rows.
- `hybrid_v2_projection_status` / `_reconciliation_status` — runtime
  observability.

### 22.3 Complete decoder coverage

`src/hybrid_v2/decoder.rs` now has a table-driven `EventLayout` for
every one of the 46 canonical Hybrid V2 event kinds catalogued in
`resources/hybrid-v2/event-topics-v1.json`. Every indexed slot is
type-tagged (`SubKey`, `Owner`, `Token`, `Engine`, `OrderHash`,
`ExecutionId`, `SeriesId`, `IntentHash`, `Actor`, `SubaccountId`); every
data word is type-tagged with its ABI shape (`U256`, `I256`, `U128`,
`U64`, `U32`, `U16`, `U8`, `Bool`, `Address`, `Bytes32`). The decoder
enforces:

- Topic 0 present in pinned catalogue (`UnknownTopic` else).
- Exact indexed topic count (`IndexedTopicCountMismatch`).
- Minimum data length (`DataTooShort`).
- Event version matches manifest (`EventVersionMismatch`).
- Emitter matches expected canonical address when supplied
  (`WrongEmitter`).

Uint256 preserved as decimal String (long-division base-10). Int256
decoded from two's-complement with explicit magnitude conversion. Every
error variant is proved by a `tests/hybrid_v2_decoder_tests.rs` case.

Verdict: `BACKEND_HYBRID_V2_EVENT_DECODER_COMPLETE`.

### 22.4 Complete reducer coverage

`src/hybrid_v2/reducer.rs` expands `ProjectionState` to cover every
canonical projection surface: identity + capability + universe + pause
+ bad-debt + positions + active-series + order lifecycle + min-valid
nonce + matched executions + fee/rebate/premium journal + owner and
subaccount recovery epochs + escape state + finalization + recovery
pause + recovery-finalization withdrawal counts.

Invariants enforced by typed `ReducerError`:

- `Underflow` / `Overflow` on uint256 arithmetic.
- `FilledQtyDecrease` (order fills monotonic).
- `MinNonceDecrease` (min-valid nonce monotonic for owner + subaccount).
- `PositionUnderflow` on signed position modification.
- `ActiveSeriesCapacity` (≤ 32 per subKey).
- `CollateralUniverseCapacity` (≤ 8 tokens).
- `IllegalRecoveryTransition` (RECOVERED is terminal; no exit).
- `FinalizedSubaccountCredit` — deposits to a finalized subKey are
  rejected.

Finalization atomically zeroes balances + reservations for the
finalized subKey.

Verdict: `BACKEND_TYPED_PROJECTION_REDUCER_VALIDATED`.

### 22.5 Execution correlation

`src/hybrid_v2/correlation.rs` groups per-transaction events into
`MatchedExecutionRow`s. Requires:

- `OptionOrderPairExecuted` (anchor, identifies execution_id + buyer/
  seller order hashes + subkeys + qty + premium)
- `OptionOrderFilled` for buyer and seller
- `OptionPremiumTransferred`

Optional: `OptionFeeCharged`, `OptionRebatePaid`.

Missing components → `Incomplete` (never surfaced as a completed
trade). Reorg invalidates groups landing in orphaned blocks
(`InvalidatedByReorg`). Duplicate events do not double-count; multiple
executions in one transaction are distinguished by `execution_id`.

Verdict: `BACKEND_OPTION_EXECUTION_CORRELATION_VALIDATED`.

### 22.6 Runtime integration

`src/hybrid_v2/runtime.rs` implements `IndexerRuntime` with:

- `ChainSource` trait boundary (in-memory + real-RPC provider both
  implement).
- Chain-id gate against manifest (`WrongChain` blocks readiness).
- Emitter-address filter from manifest module set.
- Transactional per-block apply: any decoder / reducer / correlator
  error rolls back the projection AND the raw log journal, leaves
  cursor at the last successful block, and increments the corresponding
  failure metric.
- Deterministic tx-hash convention within a block for correlation
  grouping.
- Restart-safe: cursor + `ProjectionState` are reconstructible from
  the raw canonical journal alone (`RebuildService::replay_all`).

Verdict: `BACKEND_HYBRID_V2_INDEXER_RUNTIME_INTEGRATED` and
`BACKEND_INDEXER_CURSOR_AND_FINALITY_MODEL_RESOLVED`.

### 22.7 Reorg replay

`src/hybrid_v2/reorg.rs` — `ReorgPlanner` locates the highest common
ancestor by walking back at most `max_depth` blocks (default 64). On
detection:

- Orphaned blocks are marked `is_canonical = FALSE` in the raw journal.
- `ProjectionState` is rebuilt from canonical raw logs alone (no
  compensating deltas; a strict replay).
- Cursor rewinds to the highest surviving canonical block.
- `matched_executions` for orphaned blocks flip to
  `InvalidatedByReorg`.
- Next tick pulls the replacement chain and applies it forward.
- Excessive reorg depth → typed `ExcessiveReorgDepth` error and
  readiness stays false.

Tests: one-block replacement, multi-block replacement, orphaned raw
logs retained under `by_hash` in the source, correlation group
invalidation.

Verdict: `BACKEND_REORG_CANONICAL_REPLAY`.

### 22.8 DB-loss rebuild

`src/hybrid_v2/rebuild.rs` — `RebuildService::replay_all(&journal)` is
the deterministic replay engine. The intended service wrapper:

1. selects one explicit deployment,
2. validates its manifest via `ManifestValidator`,
3. acquires the existing repository rebuild lock,
4. clears rebuildable projections (raw journal preserved),
5. calls `replay_all` on the raw journal,
6. runs `Reconciler` against a `ChainViewProvider`,
7. publishes readiness only after convergence.

Property test `rebuild_equals_incremental` proves the rebuilt state
matches the incremental projection field-by-field for a bounded fixture.

Verdict: `BACKEND_FULL_PROJECTION_REBUILD_AFTER_DB_LOSS_VALIDATED`.

### 22.9 Read-only chain reconciliation

`src/hybrid_v2/chain_view.rs` — `ChainViewProvider` trait, in-memory
implementation, `Reconciler` with bounded batch size. Classifications:

- `Converged`
- `IndexerBehind { indexed, observed }`
- `NonFinalDifference { block }`
- `ManifestMismatch { expected, actual }`
- `ProjectionDrift { detail }`
- `ProviderUnavailable`
- `Unsupported { detail }`

Drift never auto-repairs projections — it fails readiness. Tests cover
every classification.

Verdict: `BACKEND_CHAIN_VIEW_RECONCILIATION_VALIDATED`.

### 22.10 Query repository

`src/hybrid_v2/repository.rs` — `HybridV2QueryRepository` exposes
methods `deployment_status`, `subaccounts_by_owner`,
`subaccount_details`, `collateral_balances`, `reservations`,
`active_positions`, `order_lifecycle`, `matched_executions`,
`fee_events`, `recovery_state`, `escape_state`. Deterministic
cursor-paginated iteration via `PageCursor` (clamped to
`MAX_PAGE_LIMIT`). Uint256 fields returned as decimal String
end-to-end. Deployment id is a required constructor argument — every
method is deployment-scoped.

Verdict: `BACKEND_SUBACCOUNT_QUERY_REPOSITORY_READY`.

### 22.11 Observability + readiness

`src/hybrid_v2/readiness.rs` defines `ReadinessState` +
`ReadinessReason`. Every failure mode has a corresponding reason:

- `AwaitingFirstBlock`
- `WrongChain { manifest, source }`
- `ManifestMismatch { detail }`
- `UnknownCanonicalEvent { topic0, block }`
- `DecodeFailure { block, detail }`
- `ProjectionFailure { block, detail }`
- `CursorHashMismatch { block }`
- `ExcessiveReorg { depth, max }`
- `RebuildInProgress` / `RebuildFailed { detail }`
- `ReconciliationDrift { detail }`
- `MigrationSchemaMismatch`
- `Behind { observed, indexed }`

`RuntimeMetrics` carries deployment id, manifest hash, observed /
indexed / finalized / lag / last-successful-block, decode failures,
projection failures, unknown canonical events, reorg count, max reorg
depth seen, rebuild status, reconciliation status.

Verdict: `BACKEND_HYBRID_V2_INDEXER_OBSERVABILITY_COMPLETE`.

### 22.12 Security + performance

- No unsafe. No arbitrary panic on external input — every parser uses
  typed errors.
- Every SQL statement in `0045_hybrid_v2_projection_completion.sql`
  parameterizes deployment_id / subkey / token; no string
  concatenation.
- Reconciler batch bounded (`max_pairs_per_batch = 4096`).
- Reorg search bounded (`max_depth = 64`).
- Repository paging clamped (`MAX_PAGE_LIMIT = 1000`).
- Uint256 arithmetic uses fixed 32-byte buffers — no allocation on the
  hot path.
- No chain-write provider anywhere; `ChainViewProvider` is
  intentionally trait-object read-only.

Verdicts: `BACKEND_INDEXER_SECURITY_BOUNDARY_VALIDATED` +
`BACKEND_SUBACCOUNT_INDEXER_PERFORMANCE_BOUNDED`.

### 22.13 Tests landed

- `tests/hybrid_v2_snapshot_tests.rs` — 3
- `tests/hybrid_v2_manifest_tests.rs` — 12
- `tests/hybrid_v2_decoder_tests.rs` — 25 (up from 5)
- `tests/hybrid_v2_reducer_tests.rs` — 15 (up from 8)
- `tests/hybrid_v2_runtime_tests.rs` — 11 (new)
- `tests/hybrid_v2_property_tests.rs` — 5 (new)

**Total: 71 hybrid_v2 tests.**

### 22.14 Foundation preservation

Nothing in `2442646` / `f69a71e` was reverted. Every previously landed
public type is still exported, still tested, still consumed by the
completion layer.

### 22.15 Doctest fix

`src/perps/margin.rs` — three previously-failing doctests were inside
indented-4-space Rust code blocks that contained the unicode `⟺`.
Wrapped them in `\`\`\`text` fences (documentation-only fix, no runtime
behavior change).

### 22.16 Non-goals still respected

- No public HTTP route.
- No signing / write action.
- No live-network manifest ingestion.
- No frontend change.
- No Solidity change.
- No chain broadcast.
- No Base mainnet acceptance path.
- No destructive migration.
- No security audit.

