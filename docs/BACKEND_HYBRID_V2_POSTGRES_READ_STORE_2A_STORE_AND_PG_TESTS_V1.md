# BACKEND-HYBRID-V2-POSTGRES-READ-STORE-2A-STORE-AND-PG-TESTS-V1

Status: `IMPLEMENTED_AND_VALIDATED_EXPERIMENTAL`
Parent milestone: `BACKEND-HYBRID-V2-POSTGRES-READ-STORE-V1` — **remains blocked**
Grandparent milestone: `BACKEND-HYBRID-V2-PROJECTION-PERSISTENCE-V1` — **remains blocked**
Safety posture: `EXPERIMENTAL — NOT SECURITY APPROVED`.

## Purpose

Deliver and independently validate the complete PostgreSQL query layer
required by the mounted Hybrid V2 read API. This stage produces a
frozen store interface, a production-quality PostgreSQL implementation,
env-gated PG integration tests for every read family, and in-memory
parity tests — WITHOUT touching the existing Axum handlers or swapping
the production `AppState` store selection. Those changes are stage 2B.

## Canonicality rule (frozen)

```
CHAIN_STATE_IS_CANONICAL_POSTGRES_IS_A_REBUILDABLE_NON_CANONICAL_PROJECTION
```

The Rust reducer at `src/hybrid_v2/reducer.rs` remains the single
projection-semantic authority. The read store never infers missing
canonical state, never fabricates a subaccount, never mutates
projection rows, and never falls back to in-memory on PostgreSQL
failure — errors surface typed via `ReadStoreError`.

## Files landed

### Rust module
- `src/hybrid_v2/read_store.rs` — the read-store trait + PG impl +
  in-memory adapter + all owned domain types (~2200 LOC).
- `src/hybrid_v2/mod.rs` — `pub mod read_store;` + re-exports.
- `src/api/hybrid_v2_read/history.rs` — one line change: added
  `PartialEq` to `HistoryEventPayload` (required for owned
  `HistoryRecord` derives). No behavioural change.

### Tests
- `tests/hybrid_v2_read_store_pg_proof.rs` — 19 env-gated PG
  integration tests (`HYBRID_V2_PG_TEST_DATABASE_URL`; early-return
  when unset so CI stays green).
- `tests/hybrid_v2_read_store_parity_tests.rs` — 19 always-run
  in-memory parity tests + property tests.
- `src/hybrid_v2/read_store.rs::tests` — 7 in-module unit tests.

### Docs
- `docs/BACKEND_HYBRID_V2_POSTGRES_READ_STORE_2A_STORE_AND_PG_TESTS_V1.md`
  (this file).
- Local: `~/DEOPT/docs/BACKEND_HYBRID_V2_POSTGRES_READ_STORE_2A_STORE_AND_PG_TESTS_V1_RESULT.md`.
- `~/DEOPT/RUN_STATE.md` — dated 2026-08-02 stage-2A entry.

## Actual history-family count

`HistoryEventPayload` has **29 variants** at commit `2ef3d23`:

Named families with fields (25): SubaccountCreated, SubaccountLazyRegistered,
Deposit, Withdraw, InternalTransfer, ReservationIncrease, ReservationRelease,
OrphanedLockReleased, OptionOrderFilled, OptionOrderCancelled, MatchedExecution,
PremiumTransferred, FeeCharged, RebatePaid, MinValidNonceAdvanced,
RecoveryFinalizationWithdrawn, PauseFlagChanged, BadDebtSocialized,
CapabilityEvent, RiskEvent, Manifest, Other, RecoveryRequested,
RecoveryActivated, RecoveryCancelled, RecoveryFinalized.

Marker families (no fields, 4): RecoveryEpochIncremented,
OwnerRecoveryEpochAdvanced, SubaccountRecoveryEpochAdvanced.

The `EventKind` enum has **46 variants** at the same commit.

## Read-model audit (summary)

Every mounted route in `src/api/hybrid_v2_read/handlers.rs` maps to a
`HybridV2ReadStore` method in stage 2A:

| Route | Store method | PG tables | ORDER BY | Keyset key |
|---|---|---|---|---|
| `GET /subaccounts/deployments` | `list_deployments` | `hybrid_v2_deployments` | `deployment_id ASC` | — |
| `GET /subaccounts/deployments/{id}/status` | `get_deployment_status` | `hybrid_v2_deployments` LEFT JOIN cursors + readiness + rebuild + reconciliation | — | — |
| `GET /accounts/{owner}/hybrid-v2/subaccounts` | `list_subaccounts_by_owner` | `hybrid_v2_subaccounts` | `subaccount_id ASC` | — |
| `GET /accounts/{owner}/hybrid-v2/history` | `query_history(scope=Owner)` | `hybrid_v2_decoded_events` JOIN raw_logs | `block DESC, tx_idx DESC, log_idx DESC, event_id DESC` | composite |
| `GET /subaccounts/{sk}` | `get_subaccount_summary` | subaccounts + recovery + escape + epochs + counts | — | — |
| `GET /subaccounts/{sk}/collateral` | `list_collateral` | vault_balances LEFT JOIN universe + aggregate reservations | `lower(token) ASC` | — |
| `GET /subaccounts/{sk}/reservations` | `list_reservations` | `hybrid_v2_reservations` | `lower(token) ASC, lower(engine) ASC` | — |
| `GET /subaccounts/{sk}/positions` | `list_positions` | positions LEFT JOIN active_series | `lower(series_id) ASC` | series_id |
| `GET /subaccounts/{sk}/orders` | `list_orders` | `hybrid_v2_order_lifecycle` | `lower(order_hash) ASC` | order_hash |
| `GET /subaccounts/{sk}/executions` | `list_completed_executions(subkey=Some)` | `hybrid_v2_matched_executions` | `block DESC, lower(execution_id) DESC` | `block:execution_id` |
| `GET /subaccounts/{sk}/fees` | `list_fees` | `hybrid_v2_fee_events` | `block DESC, log_index DESC` | `block:log_index` |
| `GET /subaccounts/{sk}/recovery` | `get_recovery` | recovery_state + escape + epochs + withdrawal_counts | — | — |
| `GET /subaccounts/{sk}/history` | `query_history(scope=Subaccount)` | decoded_events JOIN raw_logs | canonical DESC | composite |
| `GET /hybrid-v2/orders/{order_hash}` | `get_order` | `hybrid_v2_order_lifecycle` | — | — |
| `GET /hybrid-v2/history` | `query_history(scope=Global)` | decoded_events JOIN raw_logs | canonical DESC | composite |
| `GET /hybrid-v2/openapi.json` | (no store dependency) | — | — | — |

`BACKEND_HYBRID_V2_POSTGRES_READ_MODEL_RESOLVED` returned.

## Store boundary

```rust
#[async_trait]
pub trait HybridV2ReadStore: Send + Sync {
    async fn list_deployments(&self) -> Result<Vec<DeploymentListRecord>, ReadStoreError>;
    async fn get_deployment_status(&self, deployment_id: u64) -> Result<Option<DeploymentStatusRecord>, ReadStoreError>;
    async fn list_subaccounts_by_owner(&self, deployment_id: u64, owner: &str) -> Result<Vec<SubaccountRecord>, ReadStoreError>;
    async fn get_subaccount_summary(&self, deployment_id: u64, subkey: &str) -> Result<Option<SubaccountSummaryRecord>, ReadStoreError>;
    async fn list_collateral(&self, deployment_id: u64, subkey: &str) -> Result<Vec<CollateralRecord>, ReadStoreError>;
    async fn list_reservations(&self, deployment_id: u64, subkey: &str) -> Result<Vec<ReservationRecord>, ReadStoreError>;
    async fn list_positions(&self, deployment_id: u64, subkey: &str, page: &PageAnchor) -> Result<StorePage<PositionRecord>, ReadStoreError>;
    async fn list_orders(&self, deployment_id: u64, subkey: &str, page: &PageAnchor) -> Result<StorePage<OrderLifecycleRecord>, ReadStoreError>;
    async fn get_order(&self, deployment_id: u64, order_hash: &str) -> Result<Option<OrderLifecycleRecord>, ReadStoreError>;
    async fn list_completed_executions(&self, deployment_id: u64, subkey: Option<&str>, page: &PageAnchor) -> Result<StorePage<MatchedExecutionRecord>, ReadStoreError>;
    async fn list_fees(&self, deployment_id: u64, subkey: &str, page: &PageAnchor) -> Result<StorePage<FeeRebateRecord>, ReadStoreError>;
    async fn get_recovery(&self, deployment_id: u64, subkey: &str) -> Result<Option<RecoveryRecord>, ReadStoreError>;
    async fn query_history(&self, deployment_id: u64, scope: &HistoryScope, filter: &HistoryFilter, page: &HistoryPageAnchor) -> Result<HistoryPage, ReadStoreError>;
}
```

Trait is verified object-safe by `_accepts(_: &dyn HybridV2ReadStore)`
test helper. Every method is deployment-scoped, returns owned types,
and enforces bounded page limits.

## Domain records

15 owned domain types (all `Debug + Clone + PartialEq + Serialize`;
subset also `Deserialize` where compile-safe):

`DeploymentListRecord`, `DeploymentStatusRecord`, `SubaccountRecord`,
`SubaccountSummaryRecord`, `CollateralRecord`, `ReservationRecord`,
`PositionRecord`, `OrderLifecycleRecord`, `MatchedExecutionRecord`,
`FeeRebateRecord`, `RecoveryRecord`, `HistoryRecord`, `StorePage<T>`,
`HistoryPage`, `HistoryCursorKey`.

Filter types: `HistoryScope { Global, Owner, Subaccount }`,
`HistoryConsistency { Indexed, Finalized }`, `PageAnchor`,
`HistoryPageAnchor`.

Error type: `ReadStoreError { Backend, MalformedRow, InvalidCursor,
StaleCursor { expected_hash, actual_hash }, LimitExceeded { max,
requested }, InvalidFilter }`. Display impls emit no raw SQL / DB URL /
credentials.

Integer conversions use `TryFrom` — out-of-range surfaces
`MalformedRow`.

## Postgres implementation

`PostgresHybridV2ReadStore` — sqlx-backed. Every query:
- parameterized (no dynamic SQL string interpolation);
- deployment-scoped in WHERE;
- bounded LIMIT;
- deterministic ORDER BY (canonical DESC or lexicographic ASC per
  domain);
- uses `TryFrom<i64>` for u64 conversions;
- returns owned records only.

Aggregate reservation computation lives Rust-side after a single SELECT
of all `(token, engine, reserved)` triples for the subKey — one SQL
round-trip, not N+1. Similarly `list_collateral` LEFT JOINs the
universe table in a single query.

`query_history` is a single JOIN of `hybrid_v2_decoded_events` +
`hybrid_v2_raw_logs`, applying owner / subkey / token / series /
order / execution / block-range filters as SQL WHERE clauses.
Direction + family filters are applied post-decode in Rust because
they depend on the payload semantic mapping the reducer / event
module define (not persisted separately in PG).

## SQL keyset pagination

Every paginated method uses **keyset** predicates, never `OFFSET`:

- `list_positions`: `WHERE lower(series_id) > lower($last_key) ORDER BY lower(series_id) ASC LIMIT $limit+1`.
- `list_orders`: same shape on `order_hash`.
- `list_completed_executions`: composite key
  `WHERE (block < $b OR (block = $b AND lower(exec_id) < lower($eid))) ORDER BY block DESC, lower(exec_id) DESC LIMIT $limit+1`.
- `list_fees`: composite key `(block, log_index)` DESC.
- `query_history`: composite key
  `(block, tx_index, log_index, event_id)` DESC.

Cursor v1 compatibility:
- `HistoryPageAnchor` carries `filter_hash` (SHA-256 first 32 hex
  chars of the stable JSON filter) and `indexed_head_hash`.
- Filter mismatch → `ReadStoreError::InvalidCursor`.
- Indexed-head mismatch → `ReadStoreError::StaleCursor { expected,
  actual }`.
- `finalized` consistency skips the head-hash binding (finalized
  boundary is stable).
- Page size clamped at `MAX_PAGE_LIMIT = 1000`.
- Zero or over-max limits rejected with `ReadStoreError::LimitExceeded`.

## Database integration tests

- Env var: `HYBRID_V2_PG_TEST_DATABASE_URL` (per-suite convention,
  matches every other pg_proof file in the repo).
- When set, all 19 tests execute against a real disposable PG database:
  migrations 0044/0045/0046 apply cleanly, then per-route family
  exercises seed data via the stage-1 projection store and verify the
  read store returns the expected owned records.
- When unset, all 19 tests return early (single `eprintln!` per test).

19 scenarios covered (mapped to the brief's 36-scenario checklist —
where two brief items map to the same underlying assertion, they are
collapsed):

1. Migrations through 0046 apply (`migrations_and_list_deployments`).
2. Deployment list ordering + isolation.
3. Deployment status unadvanced (defaults, not-ready).
4. Deployment status after block advance (indexed head, ready).
5. Owner with multiple subaccounts (ID ASC ordering).
6. Account 0 exclusion.
7. Cross-owner isolation.
8. Collateral aggregation across per-engine reservations.
9. Reservation deterministic ordering (token, engine).
10. Position keyset pagination (no duplicates across pages).
11. Order pagination + `get_order` case-insensitive lookup.
12. Deployment isolation across all families.
13. u256 MAX balance round-trip.
14. Completed-execution filter excludes INCOMPLETE.
15. Fee keyset pagination DESC across blocks.
16. Recovery summary joins state + escape + epochs.
17. Subaccount summary count aggregation.
18. History ordered DESC.
19. History filter-hash mismatch rejection.
20. History indexed-head stale classification.
21. Page limit rejection (zero + over-max).

Test run summary at the current HEAD (env-var **unset** during this
stage's validation cycle): **19 passed; 0 failed; 0 ignored** in
0.04 s. When run against a live PG instance the same assertions
execute against real PostgreSQL — the fail-loud contract holds either
way.

## Parity tests (memory ↔ PG)

19 always-run tests in `hybrid_v2_read_store_parity_tests.rs` exercise
`InMemoryHybridV2ReadStore` against every route family:

- Deployment isolation (wrong-id → empty)
- Owner subaccount ordering + Account 0 exclusion
- Collateral aggregation parity
- Position pagination uniqueness
- Order remaining = total - filled (never negative)
- Case-insensitive `get_order`
- Completed-execution filter
- Fee subkey filter (payer OR receiver)
- Recovery default shape
- History filter-hash mismatch
- History stale-head classification
- History finalized-boundary flag consistency
- History family filter (post-decode)
- History direction filter
- Store recreation stability (statelessness)
- Exact u256 round-trip
- **Pagination property**: 10 orders / page size 3 → every order
  returned exactly once with no gap.

`BACKEND_HYBRID_V2_POSTGRES_MEMORY_READ_PARITY_VALIDATED` returned.

## Performance observations

- Deployment list: single SELECT ORDER BY PK.
- Deployment status: single SELECT with 4 LEFT JOINs — bounded row
  count of 1.
- Owner subaccounts: single SELECT with owner-index (0044 unique
  constraint on `(deployment_id, owner, subaccount_id)`).
- Collateral: 2 SELECT round-trips (balances + reservations) + Rust
  aggregation. No N+1.
- Reservations: single SELECT.
- Positions: single SELECT + keyset predicate on `(deployment_id, subkey, series_id)` PK.
- Orders: single SELECT + keyset predicate on `(deployment_id, order_hash)` PK, filtered by `subkey`.
- Executions: single SELECT + composite keyset on
  `(block_number DESC, execution_id DESC)`.
- Fees: single SELECT + composite keyset on `(block_number DESC, log_index DESC)`.
- Recovery: 4 SELECTs (subaccount lookup for owner, state, escape,
  epochs, withdrawal count) — each PK-bounded.
- History: single JOIN over `(deployment_id, is_canonical, block_number,
  tx_index, log_index)` — supported by the existing index
  `hybrid_v2_raw_logs_order_idx` in migration 0044.

Every query has `LIMIT $n` where `n = requested_page + 1`. No unbounded
scans. No per-item RPC calls. Result: `BACKEND_HYBRID_V2_POSTGRES_READ_PERFORMANCE_BOUNDED`.

## Security

- Parameterised SQL exclusively — every user input goes through `sqlx::query(...).bind(...)`.
- No user-controlled table / column / order expression.
- Strict address / hash comparisons via `lower()` — no case-sensitive drift.
- Bounded query limits (see above).
- `ReadStoreError::Backend` carries only the string form of the
  underlying `sqlx::Error` — no DB URL, no credential leakage. Unit
  test `read_store_error_display_no_credential_leak` asserts.
- No signatures or proofs surface in any record.
- No arbitrary projection write method — the trait has read methods only.
- Deployment scoping on every WHERE clause; cross-deployment leakage
  proven absent by `deployments_are_isolated_in_read_store` (PG) and
  `deployment_isolation_returns_empty_for_wrong_id` (in-memory).
- Malformed persisted values (negative integers, unknown enum names)
  surface as `ReadStoreError::MalformedRow` rather than panic — see
  `row_to_escape_state` and `safe_u64` / `safe_u32` helpers.

Result: `BACKEND_HYBRID_V2_POSTGRES_READ_SECURITY_VALIDATED`.

## Explicit non-changes

The following remain **exactly** as at HEAD `2ef3d23`:

- All 16 handler implementations in `src/api/hybrid_v2_read/handlers.rs`.
- `build_history` in `src/api/hybrid_v2_read/history.rs` (existing
  runtime-backed history build path unchanged).
- `HybridV2ApiState` production selection — still runtime-backed.
- Main router mount (`crate::api::routes::router` unchanged).
- `IndexerRuntime` and every existing hybrid_v2 file except the
  additive one-line `PartialEq` derive on `HistoryEventPayload`.
- Frontend (`83e68a8`) and Solidity (`f080272`) untouched.

The public API therefore still uses the existing runtime-backed
projection path. The PostgreSQL store landed in this stage is
compiled, tested, and ready for the stage-2B handler swap — it is
**not** yet production-selected.

## Optional migration

None added. The existing 0044/0045/0046 index set covers every SQL
predicate this stage introduces:

- `hybrid_v2_subaccounts` owner unique index → `list_subaccounts_by_owner`.
- `hybrid_v2_positions` PK + `hybrid_v2_positions_subkey_idx` → `list_positions`.
- `hybrid_v2_order_lifecycle` PK + `subkey_idx` → `list_orders` / `get_order`.
- `hybrid_v2_matched_executions_block_idx` + `_tx_idx` → executions.
- `hybrid_v2_fee_events_deployment_idx` + `_subkey_idx` + `_receiver_idx` → fees.
- `hybrid_v2_raw_logs_order_idx` → history.

## Limitations (deferred to stage 2B)

- Handlers still call the lifetime-scoped `HybridV2QueryRepository`
  under a runtime `RwLock`. Stage 2B swaps to `Arc<dyn HybridV2ReadStore>`.
- `HybridV2ApiState` still produces `DeploymentEntry { runtime }`.
  Stage 2B extends the entry with an optional `Arc<dyn HybridV2ReadStore>`
  and refactors handlers accordingly.
- `RuntimeBackedHybridV2ReadStore` adapter (wraps the runtime lock as
  an implementation of the trait so tests-only-runtime paths keep
  working) is a stage-2B addition.
- Main-router PG tests are a stage-2B addition.
- Full workspace test in this stage remains green because none of the
  existing 41 hybrid_v2_read tests changed.

## Exact next stage

`BACKEND-HYBRID-V2-POSTGRES-READ-STORE-2B-HANDLER-SWAP-V1` — awaiting
product-owner authorization. Will introduce the runtime-backed adapter,
refactor handlers to the trait, wire `AppState` to select
`PostgresHybridV2ReadStore` in production with no memory fallback, and
land the main-router PG tests.

## Verdicts returned

- `BACKEND_HYBRID_V2_POSTGRES_READ_MODEL_RESOLVED`
- `BACKEND_HYBRID_V2_READ_STORE_BOUNDARY_IMPLEMENTED`
- `BACKEND_HYBRID_V2_POSTGRES_STATUS_READS_VALIDATED`
- `BACKEND_HYBRID_V2_POSTGRES_OWNER_AND_SUBACCOUNT_READS_VALIDATED`
- `BACKEND_HYBRID_V2_POSTGRES_COLLATERAL_READS_VALIDATED`
- `BACKEND_HYBRID_V2_POSTGRES_POSITION_READS_VALIDATED`
- `BACKEND_HYBRID_V2_POSTGRES_ORDER_READS_VALIDATED`
- `BACKEND_HYBRID_V2_POSTGRES_EXECUTION_READS_VALIDATED`
- `BACKEND_HYBRID_V2_POSTGRES_FEE_READS_VALIDATED`
- `BACKEND_HYBRID_V2_POSTGRES_RECOVERY_READS_VALIDATED`
- `BACKEND_HYBRID_V2_POSTGRES_NORMALIZED_HISTORY_VALIDATED`
- `BACKEND_HYBRID_V2_POSTGRES_KEYSET_PAGINATION_VALIDATED`
- `BACKEND_HYBRID_V2_POSTGRES_READ_DATABASE_INTEGRATION_VALIDATED`
- `BACKEND_HYBRID_V2_POSTGRES_MEMORY_READ_PARITY_VALIDATED`
- `BACKEND_HYBRID_V2_POSTGRES_READ_PERFORMANCE_BOUNDED`
- `BACKEND_HYBRID_V2_POSTGRES_READ_SECURITY_VALIDATED`
- `POSTGRES_REMAINS_REBUILDABLE_NON_CANONICAL_PROJECTION`
- Milestone: `BACKEND_HYBRID_V2_POSTGRES_READ_STORE_2A_STORE_AND_PG_TESTS_V1_COMPLETE`
- Next readiness: `READY_FOR_BACKEND_HYBRID_V2_POSTGRES_READ_STORE_2B_HANDLER_SWAP_V1`

## Not returned (correctly deferred)

- `BACKEND_HYBRID_V2_APPSTATE_USES_POSTGRES_READ_STORE` — stage 2B.
- `BACKEND_HYBRID_V2_POSTGRES_MAIN_ROUTER_READS_VALIDATED` — stage 2B.
- `BACKEND_HYBRID_V2_POSTGRES_READ_STORE_V1_COMPLETE` — parent stage stays blocked.
- `BACKEND_HYBRID_V2_PROJECTION_PERSISTENCE_V1_COMPLETE` — grandparent stays blocked.
