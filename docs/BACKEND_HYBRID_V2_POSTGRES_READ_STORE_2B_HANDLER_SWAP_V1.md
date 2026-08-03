# `BACKEND-HYBRID-V2-POSTGRES-READ-STORE-2B-HANDLER-SWAP-V1`

Date landed: 2026-08-03
Predecessors:
- `BACKEND-HYBRID-V2-POSTGRES-READ-STORE-2A-STORE-AND-PG-TESTS-V1`
- `BACKEND-PG-MIGRATION-CHAIN-INTEGRITY-V1`
Status: `IMPLEMENTED_AND_VALIDATED_EXPERIMENTAL`
Safety posture: `EXPERIMENTAL — NOT SECURITY APPROVED`

## Purpose

Move the complete mounted Hybrid V2 HTTP read API from direct
`ProjectionState` / `IndexerRuntime` access to the stage-2A
`HybridV2ReadStore` abstraction so that production HTTP reads flow
through `PostgresHybridV2ReadStore`. Preserve every mounted route,
every response shape, and every existing runtime-backed test via a
compatibility adapter that binds `HybridV2ReadStore` to an
`IndexerRuntime`.

## Frozen rules

- `CHAIN_STATE_IS_CANONICAL_POSTGRES_IS_A_REBUILDABLE_NON_CANONICAL_PROJECTION`
- `PRODUCTION_HYBRID_V2_HTTP_READS_USE_POSTGRES_ONLY`
- No production handler accesses `ProjectionState`, `IndexerRuntime`,
  a runtime `RwLock`, `runtime.raw_logs`, runtime order maps, runtime
  recovery maps, or in-memory cursor / readiness fields.
- No automatic PostgreSQL-to-memory fallback is permitted; Postgres
  outages surface as structured `INTERNAL_INCONSISTENCY` responses.

## Architecture

### Store abstraction

The stage-2A `HybridV2ReadStore` trait (`src/hybrid_v2/read_store.rs`)
covers every data-bearing read family required by the 16-route
public API. Its record types already carry every auxiliary field the
pre-refactor handlers computed inline against `ProjectionState` — the
`active` boolean on `PositionRecord`, `universe_index` + `enabled` on
`CollateralRecord`, `escape` + `withdrawal_count` on `RecoveryRecord`,
and the four projection counts on `SubaccountSummaryRecord`. No trait
extension was needed.

Three concrete implementations exist:

| Implementation | Selected by | Purpose |
| --- | --- | --- |
| `PostgresHybridV2ReadStore` | `HybridV2ApiState::with_postgres(pool, entries)` | **Production.** sqlx-backed, fail-closed. |
| `RuntimeBackedHybridV2ReadStore` | `HybridV2ApiState::new(entries)` | Test / compatibility. Wraps `IndexerRuntime`s. Not production-selectable. |
| `InMemoryHybridV2ReadStore` | `HybridV2ApiState::with_store(store, entries)` | Parity fixtures / unit tests. |

An `EmptyReadStore` (private to `state.rs`) backs
`HybridV2ApiState::empty()`; combined with the empty deployment
registry, every canonical route on an unconfigured backend responds
with a structured 404 / 503, and `/subaccounts/deployments` returns
`[]`.

### `HybridV2ApiState`

Replaced with a two-part inner:
- `deployments: BTreeMap<u64, Arc<DeploymentEntry>>` — per-deployment
  metadata (`deployment_id` + immutable `ManifestParams`).
- `store: Arc<dyn HybridV2ReadStore>` — shared across every handler.

`DeploymentEntry` still exposes a `runtime: Option<Arc<RwLock<IndexerRuntime>>>`
field, but it is documented as **test/compatibility only** —
`RuntimeBackedHybridV2ReadStore` binds to it; handlers must not.
Production entries use `DeploymentEntry::from_metadata` which sets
`runtime: None`.

### Handlers

Every one of the 16 mounted routes now follows a common shape:
1. request parsing + strict address/hash validation,
2. cursor decoding + binding verification,
3. `store.get_deployment_status(deployment_id)` for readiness + head
   metadata,
4. hard-readiness gate via
   `metadata::hard_readiness_failure_from_status`,
5. store-scoped data call (`list_positions`, `query_history`, etc.),
6. response transformation with exact wire-format compatibility.

The historic runtime-lock-per-request is gone — handlers hold no
locks, do no full-projection clones, and produce no runtime SQL
strings.

### History

`build_history` remains only in the runtime-backed compatibility path.
Production `query_history` executes via
`PostgresHybridV2ReadStore::query_history` with SQL keyset pagination
over the persisted `hybrid_v2_decoded_events` (and related) tables.
Cursor binding uses the store-boundary `HistoryPageAnchor`
(`filter_hash` + `indexed_head_hash`) so a reorg produces
`ReadStoreError::StaleCursor` → HTTP 409.

The 29-variant `HistoryEventPayload` is unchanged; `HistoryRecord`
and `HistoryEvent` are structurally identical, so wire JSON is
bit-for-bit preserved. A single `record_to_event` helper in
`handlers.rs` maps the two.

### Error mapping (`ApiError::from(ReadStoreError)`)

| Variant | HTTP status | API code |
| --- | --- | --- |
| `Backend { detail }` | 500 | `INTERNAL_INCONSISTENCY` |
| `MalformedRow { detail }` | 500 | `MALFORMED_CANONICAL_DATA` |
| `InvalidCursor { detail }` | 400 | `INVALID_CURSOR` |
| `StaleCursor { … }` | 409 | `STALE_CURSOR` |
| `LimitExceeded { … }` | 400 | `PAGE_LIMIT_EXCEEDED` |
| `InvalidFilter { detail }` | 400 | `INVALID_FILTER_COMBINATION` |

Error messages never carry raw SQL, connection strings, credentials,
or database URLs.

### Production `AppState`

`AppState::with_hybrid_v2_postgres(pool, entries)` is the sole
production selection path. Under an unconfigured backend the field
defaults to `HybridV2ApiState::empty()` and every canonical route
fails closed per the pre-existing `AWAITING_FIRST_BLOCK` / 503 policy.
There is no runtime-memory fallback.

## Test surface

- `tests/hybrid_v2_read_api_tests.rs` (24 tests) — unchanged; run
  against the runtime-backed adapter via `HybridV2ApiState::new`.
- `tests/hybrid_v2_read_api_main_router_tests.rs` (17 tests) —
  unchanged; verify the top-level router path with the runtime-backed
  adapter.
- `tests/hybrid_v2_read_api_property_tests.rs` (5 tests) — property
  tests over pagination, cursor staleness, isolation.
- `tests/hybrid_v2_read_api_postgres_main_router_tests.rs` (**new**,
  21 tests) — end-to-end proof that every mounted Hybrid V2 route
  serves canonical data from real PostgreSQL through the same
  top-level router `main.rs` uses, plus `/accounts/:addr/history/v2`,
  `/health`, write-method rejection, AppState recreation identity,
  and Postgres-outage fail-closed behaviour.
- `tests/hybrid_v2_persistence_core_pg_proof.rs` (10 tests) —
  stage-2A convergence baseline; still passes 10/10.
- `tests/hybrid_v2_read_store_pg_proof.rs` (19 tests) — stage-2A
  store parity baseline; still passes 19/19.

## Route matrix (all 16 mounted paths)

| Method + Path | Handler | Store method(s) |
| --- | --- | --- |
| GET `/subaccounts/deployments` | `get_deployments` | `list_deployments` + per-entry `get_deployment_status` |
| GET `/subaccounts/deployments/:deployment_id/status` | `get_deployment_status` | `get_deployment_status` |
| GET `/accounts/:owner/hybrid-v2/subaccounts` | `get_owner_subaccounts` | `list_subaccounts_by_owner` + per-sub `get_subaccount_summary` |
| GET `/accounts/:owner/hybrid-v2/history` | `get_owner_history` | `query_history` (scope=Owner) |
| GET `/subaccounts/:subkey` | `get_subaccount_summary` | `get_subaccount_summary` |
| GET `/subaccounts/:subkey/collateral` | `get_subaccount_collateral` | `list_collateral` |
| GET `/subaccounts/:subkey/reservations` | `get_subaccount_reservations` | `list_reservations` |
| GET `/subaccounts/:subkey/positions` | `get_subaccount_positions` | `list_positions` |
| GET `/subaccounts/:subkey/orders` | `get_subaccount_orders` | `list_orders` |
| GET `/subaccounts/:subkey/executions` | `get_subaccount_executions` | `list_completed_executions` |
| GET `/subaccounts/:subkey/fees` | `get_subaccount_fees` | `list_fees` |
| GET `/subaccounts/:subkey/recovery` | `get_subaccount_recovery` | `get_recovery` |
| GET `/subaccounts/:subkey/history` | `get_subaccount_history` | `query_history` (scope=Subaccount) |
| GET `/hybrid-v2/orders/:order_hash` | `get_order_lifecycle` | `get_order` |
| GET `/hybrid-v2/history` | `get_history` | `query_history` (scope=Global) |
| GET `/hybrid-v2/openapi.json` | `get_openapi` | store-independent (static JSON) |

## Performance & security

- No production handler acquires an `RwLock`, no full `ProjectionState`
  clone, no per-item RPC or SQL.
- Every store query uses parameterized sqlx bindings; user input is
  strictly bounded and validated before it reaches the store.
- Page limits are enforced at the store boundary (`PageAnchor` /
  `HistoryPageAnchor`) and mirror `MAX_PAGE_LIMIT`.
- Errors carry no raw SQL, credentials, or connection strings — the
  `postgres_unavailability_fails_closed_no_memory_fallback` test
  asserts this at the response boundary.
- Cross-deployment isolation is preserved by the deployment_id-scoped
  trait contract; the runtime-adapter path also filters by
  `deployment_id` before delegating.

## Files landed

- `src/api/hybrid_v2_read/state.rs` — rewrote `HybridV2ApiState` +
  `DeploymentEntry`; added `with_postgres` / `with_store` constructors
  + private `EmptyReadStore`.
- `src/api/hybrid_v2_read/handlers.rs` — full rewrite of all 16
  handlers.
- `src/api/hybrid_v2_read/errors.rs` — added `impl From<ReadStoreError>
  for ApiError`.
- `src/api/hybrid_v2_read/metadata.rs` — added
  `build_metadata_from_status` + `hard_readiness_failure_from_status`.
- `src/api/hybrid_v2_read/history.rs` — dropped an unused import.
- `src/hybrid_v2/runtime_backed_read_store.rs` — **new**. Test /
  compatibility adapter binding `HybridV2ReadStore` to
  `IndexerRuntime`.
- `src/hybrid_v2/mod.rs` — registers the new module.
- `src/api/http.rs` — added `AppState::with_hybrid_v2_postgres`.
- `tests/hybrid_v2_read_api_postgres_main_router_tests.rs` — **new**.
  21 tests covering every mounted Hybrid V2 route + auxiliary routes
  via the real top-level router + real Postgres.
- `tests/hybrid_v2_read_api_property_tests.rs` — one test updated to
  reach the runtime via `entry.runtime.as_ref()` (was direct).
- `docs/BACKEND_HYBRID_V2_POSTGRES_READ_STORE_2B_HANDLER_SWAP_V1.md`
  — this file.

## Out of scope (deferred)

- Persisted runtime cursor bootstrap on backend startup.
- Live reorg replay orchestration.
- Projection rebuild coordinator.
- Writer activation on the persisted mirror.
- The eight pre-existing product-logic failures in Perps, RFQ, and
  conditional-order suites (documented in
  `BACKEND-PG-MIGRATION-CHAIN-INTEGRITY-V1`); those remain owned by
  their respective subsystems.
