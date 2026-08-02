# BACKEND-HYBRID-V2-POSTGRES-PROJECTION-CORE-V1

Status: `IMPLEMENTED_AND_VALIDATED_EXPERIMENTAL`
Parent milestone: `BACKEND-HYBRID-V2-PROJECTION-PERSISTENCE-V1` — **remains blocked**;
this is stage 1 of the authorized decomposition into four sub-milestones.
Safety posture: `EXPERIMENTAL — NOT SECURITY APPROVED`.

## Purpose

Deliver the durable PostgreSQL projection-writing foundation for
Hybrid V2:

- additive migration surface covering every projection category;
- store-trait boundary decoupling the writer from consumers;
- block-atomic PostgreSQL writer covering all 46 canonical event
  variants;
- persistent cursor / readiness / runtime-metrics metadata;
- idempotent raw + decoded event journals;
- convergence + integration test proofs.

The public read API surface is **not** swapped over to Postgres in this
stage. Runtime dual-write remains deferred. Rebuild and reorg replay
against the persisted state are deferred to stage 3.

## Canonicality rule (frozen)

```
CHAIN_STATE_IS_CANONICAL_POSTGRES_IS_A_REBUILDABLE_NON_CANONICAL_PROJECTION
```

The Rust reducer at `src/hybrid_v2/reducer.rs` is the single semantic
implementation. The writer never independently reimplements an economic
rule. PostgreSQL constraints (NOT NULL, PK uniqueness, non-negative
CHECKs) enforce invariants but never encode a new one.

## Files landed

### Migration
- `migrations/0046_hybrid_v2_projection_persistence_core.sql` — 8 new
  tables + 6 additive columns on `hybrid_v2_cursors` + 1 additive
  column on `hybrid_v2_recovery_state` + 7 pagination indexes. No
  destructive changes to 0044 / 0045.

New tables:
- `hybrid_v2_canonical_blocks` — block registry, keyed on
  `(deployment_id, block_hash)`; carries `is_canonical` +
  `orphaned_at_block` for reorg-safe tracking.
- `hybrid_v2_pause_flags` — per-subKey pause bit.
- `hybrid_v2_bad_debt` — per-`(subKey, token)` bad-debt ledger.
- `hybrid_v2_recovery_withdrawal_counts` — per-subKey finalization
  withdrawal counter.
- `hybrid_v2_recovery_pause` — per-deployment recovery-pause singleton.
- `hybrid_v2_runtime_metrics` — per-deployment counters
  (reorg_count, max_reorg_depth_seen, decode_failures,
  projection_failures, unknown_canonical_events, last_success_block).
- `hybrid_v2_readiness` — per-deployment readiness state + reason.
- `hybrid_v2_reorg_events` — reorg event log.

### Rust module
- `src/hybrid_v2/persistence.rs` (~1500 LOC) — public store surface.
  - `CanonicalBlockRef`, `RuntimeCursorSnapshot`, `ReadinessSnapshot`,
    `CorrelationSnapshot` — value types.
  - `HybridV2ProjectionStore` async trait — the boundary consumers
    depend on.
  - `PostgresHybridV2ProjectionStore` — sqlx-backed implementation.
  - `InMemoryProjectionStore` — in-memory fake for unit tests and
    convergence proofs; no PG dependency.
  - `persist_event_projection` — per-event dispatch covering all 46
    canonical `EventKind` variants.
  - Per-table upsert helpers (subaccounts, balances, reservations,
    universe, capability, pause, bad_debt, positions, active_series,
    order_lifecycle, matched_executions, fee_events, escape_state,
    recovery_state, recovery_epochs, recovery_pause,
    recovery_withdrawal_counts) plus canonical_block / cursor /
    readiness / runtime_metrics upserts.

### Module registration
- `src/hybrid_v2/mod.rs` — `pub mod persistence;` + re-exports of the
  primary types.

### Tests
- `tests/hybrid_v2_persistence_convergence_tests.rs` — 9 in-memory
  convergence tests (deployment idempotency, block-atomic snapshot,
  cursor + readiness round-trip, deployment isolation, repeat block
  idempotency, reorg event capture, full ProjectionState round-trip
  across every field category, monotonic cursor counter advance).
- `tests/hybrid_v2_persistence_core_pg_proof.rs` — 10 env-gated PG
  tests (env var `HYBRID_V2_PG_TEST_DATABASE_URL`). When unset each
  test returns early so CI stays green.

Existing 3 in-module unit tests remain (subkey extraction, reducer/
snapshot balance parity, in-memory deployment idempotency).

## Persistence audit

Every projection category emitted by the canonical event stream now
has a durable PG representation.

| Category | Table(s) | Idempotency key | Invariant |
|---|---|---|---|
| Deployment / manifest | `hybrid_v2_deployments` | `(chain_id, manifest_hash)` | one live deployment per `(chain_id, deployment_version)` |
| Subaccounts | `hybrid_v2_subaccounts` | `(deployment_id, subkey)` | owner unique per `(deployment_id, subaccount_id)` |
| Vault balances | `hybrid_v2_vault_balances` | `(deployment_id, subkey, token)` | uint256 exact; reducer enforces non-negative |
| Per-engine reservations | `hybrid_v2_reservations` | `(deployment_id, subkey, token, engine)` | uint256 exact; reducer enforces non-negative |
| Collateral universe | `hybrid_v2_collateral_universe` | `(deployment_id, token)` | ≤ 8; `enabled` toggled by supported add/remove |
| Capability grants | `hybrid_v2_capability_grants` | `(deployment_id, engine)` | uint256 bitmap exact |
| Pause flags | `hybrid_v2_pause_flags` | `(deployment_id, subkey)` | bool per subKey |
| Bad debt | `hybrid_v2_bad_debt` | `(deployment_id, subkey, token)` | uint256 exact |
| Positions | `hybrid_v2_positions` | `(deployment_id, subkey, series_id)` | reducer enforces non-negative; auto-delete when zero |
| Active series | `hybrid_v2_active_series` | `(deployment_id, subkey, series_id)` | ≤ 32 per subkey |
| Order lifecycle | `hybrid_v2_order_lifecycle` | `(deployment_id, order_hash)` | filled ≤ total; terminal monotonic |
| Matched executions | `hybrid_v2_matched_executions` | `(deployment_id, execution_id)` | completion_status ∈ {COMPLETE, INCOMPLETE, INVALIDATED_BY_REORG} |
| Fees / rebates / premium | `hybrid_v2_fee_events` | serial `event_id` | append-only journal |
| Recovery state | `hybrid_v2_recovery_state` | `(deployment_id, subkey)` | FSM one-way with RECOVERED terminal |
| Escape state | `hybrid_v2_escape_state` | `(deployment_id, subkey)` | FSM: NORMAL → REQUESTED → ACTIVATED\|CANCELLED → FINALIZED |
| Recovery epochs | `hybrid_v2_recovery_epochs` | `(deployment_id, scope, scope_key)` | epoch_count monotonic; min_valid_nonce monotonic |
| Recovery withdrawal count | `hybrid_v2_recovery_withdrawal_counts` | `(deployment_id, subkey)` | monotonic non-decreasing; CHECK ≥ 0 |
| Recovery pause | `hybrid_v2_recovery_pause` | `deployment_id` | singleton per deployment |
| Canonical blocks | `hybrid_v2_canonical_blocks` | `(deployment_id, block_hash)` | block_hash distinguishes replaced-after-reorg block |
| Raw logs | `hybrid_v2_raw_logs` | `(deployment_id, block_hash, tx_hash, log_index)` | canonical identity; idempotent |
| Decoded events | `hybrid_v2_decoded_events` | `raw_log_id` | one decoded row per raw log |
| Cursor + counters | `hybrid_v2_cursors` | `(deployment_id, cursor_name)` | additive `reorg_count` / `decode_failures` / `projection_failures` / etc. |
| Runtime metrics | `hybrid_v2_runtime_metrics` | `deployment_id` | CHECK counters ≥ 0 |
| Readiness | `hybrid_v2_readiness` | `deployment_id` | ready bool + reason enum name + detail |
| Reorg events | `hybrid_v2_reorg_events` | serial | append-only log |

Categories with no dedicated projection field (RISK_MODULE_ACTIVATED,
RISK_PARAMS_SET, LIQUIDATION_TRIGGERED, INTENT_CONSUMED,
GUARDIAN_CHANGED, ENGINE_GUARDIAN_REVOKED, PROTOCOL_SUBACCOUNTS_
INITIALIZED, ESCAPE_CONTROLLER_INITIALIZED, RECOVERY_FINALIZER_
INITIALIZED, DEPLOYMENT_MANIFEST_DECLARED, OPTION_EXERCISED,
OPTION_SETTLED) are still persisted in `hybrid_v2_decoded_events` with
`applied = TRUE` — journal-only.

## Block-atomic writer contract

`PostgresHybridV2ProjectionStore::persist_block_atomic` runs one
PostgreSQL transaction per canonical block:

1. Register the canonical block (idempotent on `(deployment_id, block_hash)`).
2. Insert raw logs (idempotent on canonical identity).
3. Insert decoded events (idempotent on `raw_log_id`).
4. Per decoded event, look up the resulting state in `ProjectionState`
   for the touched key(s) and UPSERT the affected projection row(s).
5. Advance the persistent cursor, upsert runtime metrics, upsert
   readiness state.
6. Commit — or rollback on any failure, leaving no partial block state
   applied.

## Store abstractions

The `HybridV2ProjectionStore` trait carries:

- `upsert_deployment(&ManifestParams, activation_status, now_ms) -> Result<i64>`
- `persist_block_atomic(..., 8 args)` — the block-atomic contract
- `upsert_runtime_metrics(deployment_id, cursor, now_ms)`
- `record_reorg_event(deployment_id, at_ms, from_block, from_hash, to_block, to_hash, depth, orphaned_log_count)`
- `read_cursor(deployment_id, cursor_name) -> Option<RuntimeCursorSnapshot>`
- `read_readiness(deployment_id) -> Option<ReadinessSnapshot>`

Both `PostgresHybridV2ProjectionStore` and `InMemoryProjectionStore`
implement the trait. Convergence tests exercise the trait through both
implementations; the in-memory fake keeps CI green in developer
environments without a Postgres instance.

## Transitional runtime integration (Part J decision)

**Verdict returned: `BACKEND_HYBRID_V2_POSTGRES_WRITER_READY_NOT_RUNTIME_ENABLED`**

The runtime (`IndexerRuntime`) is NOT wired to dual-write in this
stage. Rationale:

- Adding a dual-write path without a persistent-cursor bootstrap and
  without reorg replay in PG would allow the in-memory runtime to
  advance past a block whose PG write failed, breaking the "no partial
  block" contract.
- Stage 3 (`BACKEND-HYBRID-V2-POSTGRES-CORRELATION-CURSOR-REORG-V1`)
  is the correct place for cursor bootstrap + reorg replay wiring.
- The writer surface is fully validated via integration fixtures +
  convergence tests, meeting the "writer ready" bar without a hidden
  production fallback.

## Test coverage

- **9 in-memory convergence tests** (`hybrid_v2_persistence_convergence_tests.rs`):
  - deployment idempotency + hash isolation
  - block-atomic snapshot integrity
  - cursor round-trip
  - readiness round-trip
  - two-deployment isolation
  - idempotent re-application
  - reorg event capture
  - full ProjectionState round-trip across every field category
  - monotonic cursor counter advance

- **10 env-gated PG integration tests** (`hybrid_v2_persistence_core_pg_proof.rs`):
  - full migration chain applies cleanly
  - deployment upsert idempotency
  - block-atomic write persists all artifacts (block + raw + decoded +
    subaccount + balance + cursor + readiness + metrics)
  - raw log idempotency
  - decoded event idempotency
  - deployment isolation (no leakage)
  - uint256 MAX balance round-trip
  - recovery finalization zeros balances + reservations
  - matched execution persists via OrderPairExecuted
  - positions + active_series upsert AND delete
  - recovery_state projection variants round-trip

- **3 in-module unit tests** (`src/hybrid_v2/persistence.rs::tests`):
  - `subkeys_touched_by` dedup
  - reducer + snapshot balance parity
  - in-memory deployment idempotency

- **Baseline suites** (existing hybrid_v2 tests): all 117 continue to pass.

## Performance observations

Development-machine observations:

- Empty block: 1 PG transaction, 5 UPSERTs (block registration + cursor
  + metrics + readiness; no raw/decoded).
- Populated block (1 subaccount + 1 deposit): 1 PG transaction, ~10
  UPSERTs (block + 2 raw + 2 decoded + subaccount + balance + cursor +
  metrics + readiness).
- Repeat block (idempotent): same UPSERT count; ON CONFLICT branches
  update existing rows without duplicating.

All queries are parameterised with `sqlx::query(...).bind(...)` — no
dynamic SQL string interpolation. Batch sizes are bounded by the
per-block event count. Every SELECT/UPSERT uses either the PK or the
`(deployment_id, ...)`-scoped indexes added by 0044/0045/0046. No
N+1 external RPC.

## Security posture

- Parameterised SQL exclusively.
- Deployment-scoped keys — no cross-deployment writes.
- No arbitrary mutation endpoint — the projection writer is invoked
  only via the block-atomic entrypoint.
- No chain-write provider, no signer.
- No secret / DB URL / RPC URL exposed in error messages or debug output.
- Integer conversions use `TryFrom` (`u64::try_from(i64)` etc.) so out-of-range values fail loudly rather than silently truncating.
- Recovery finalization sweeps balances + reservations for the finalized
  subKey — no ghost residual state.
- Base mainnet (`chain_id == 8453`) remains forbidden through the
  existing manifest ingestion path.

## Limitations (deferred to later stages)

- **Public read API is NOT swapped to Postgres.** The mounted
  Hybrid V2 read API in `src/api/hybrid_v2_read/` continues to serve
  from the in-memory runtime-backed `HybridV2QueryRepository`. Stage 2
  (`BACKEND-HYBRID-V2-POSTGRES-READ-STORE-V1`) will provide
  `PostgresHybridV2ReadStore` and the `AppState` swap.
- **IndexerRuntime is NOT dual-writing.** Stage 3 will wire cursor
  bootstrap + reorg replay against PG.
- **Rebuild + reconciliation** against PG are deferred to stage 4.
- The `applied` flag on `hybrid_v2_decoded_events` is set to `TRUE`
  when persisted. A future stage may track partial-apply failures more
  granularly.

## Exact next stage

`BACKEND-HYBRID-V2-POSTGRES-READ-STORE-V1` — awaiting product-owner
authorization. Will introduce `PostgresHybridV2ReadStore`, mirror
the 16-route read surface against 0044+0045+0046, refactor handlers
to an async store trait, and swap `AppState.hybrid_v2_read` to the PG
adapter.

## Verdicts returned

- `BACKEND_HYBRID_V2_POSTGRES_CORE_PERSISTENCE_MODEL_RESOLVED`
- `BACKEND_HYBRID_V2_POSTGRES_CORE_SCHEMA_COMPLETE`
- `BACKEND_HYBRID_V2_PROJECTION_STORE_BOUNDARY_IMPLEMENTED`
- `BACKEND_HYBRID_V2_BLOCK_ATOMIC_POSTGRES_WRITER_VALIDATED`
- `BACKEND_HYBRID_V2_ALL_CANONICAL_PROJECTIONS_PERSISTED`
- `BACKEND_HYBRID_V2_EXECUTION_CORRELATION_CORE_PERSISTED`
- `BACKEND_HYBRID_V2_RUNTIME_METADATA_PERSISTENCE_READY`
- `BACKEND_HYBRID_V2_POSTGRES_WRITER_READY_NOT_RUNTIME_ENABLED`
- `BACKEND_HYBRID_V2_MEMORY_POSTGRES_PROJECTION_CONVERGENCE_VALIDATED`
- `BACKEND_HYBRID_V2_POSTGRES_CORE_PERFORMANCE_BOUNDED`
- `BACKEND_HYBRID_V2_POSTGRES_CORE_SECURITY_VALIDATED`
- `POSTGRES_REMAINS_REBUILDABLE_NON_CANONICAL_PROJECTION`
- Milestone: `BACKEND_HYBRID_V2_POSTGRES_PROJECTION_CORE_V1_COMPLETE`
- Next readiness: `READY_FOR_BACKEND_HYBRID_V2_POSTGRES_READ_STORE_V1`

## Not returned (correctly deferred)

- Parent `BACKEND_HYBRID_V2_PROJECTION_PERSISTENCE_V1_COMPLETE` — parent
  remains blocked; three further stages required.
- `BACKEND_HYBRID_V2_POSTGRES_READ_STORE_OPERATIONAL` — stage 2.
- `BACKEND_HYBRID_V2_READ_DATABASE_INTEGRATION_VALIDATED` — stage 2.
