# `BACKEND-SUBACCOUNT-READ-API-AND-HISTORY-V1`

**Work package**: BACKEND WP-02 — Public read API, normalized account
history and subaccount query surface.

**Status**: `IMPLEMENTED_AND_VALIDATED_EXPERIMENTAL`
**Safety posture**: `EXPERIMENTAL — NOT SECURITY APPROVED`
**Product owner**: Coriolan Morel
**Predecessor**: `BACKEND-SUBACCOUNT-CANONICAL-STATE-AND-INDEXER-V1`
(backend commit `c4ddc23`).

## 1 · Non-goals

- No public trading routes; no order submission; no signing endpoint;
  no chain writes.
- No Perps subaccount API changes.
- No replacement of `/accounts/:address/history/v2` — additive only.

## 2 · Canonicality model

Every canonical response carries `CanonicalityMetadata`:

- `deployment_id`, `chain_id`, `manifest_hash`
- `indexed_block`, `indexed_block_hash`, `finalized_block`,
  `observed_head_block`
- `indexer_lag`, `reconciliation_status`
- `canonicality_level`: one of `FINALIZED`, `INDEXED_CANONICAL`,
  `INDEXER_BEHIND`, `NOT_READY`.
- `consistency_mode`: `INDEXED` (default) or `FINALIZED`.
- `generated_at_ms`.

Level is derived deterministically from readiness + lag + the requested
consistency mode. Public consumers rely on the metadata to interpret
freshness — the API never claims `FINALIZED` when data includes blocks
past the finalized cursor.

## 3 · Readiness serving policy

`src/api/hybrid_v2_read/metadata.rs::hard_readiness_failure` maps
`ReadinessReason` to serving behaviour:

- `WrongChain`, `ManifestMismatch`, `UnknownCanonicalEvent`,
  `DecodeFailure`, `ProjectionFailure`, `CursorHashMismatch`,
  `ExcessiveReorg`, `RebuildInProgress`, `RebuildFailed`,
  `ReconciliationDrift`, `MigrationSchemaMismatch`,
  `AwaitingFirstBlock` → canonical routes return 503 with a typed error.
- `Behind` → response proceeds, metadata carries `indexer_lag > 0` and
  `canonicality_level = INDEXER_BEHIND`.

The `/subaccounts/deployments/{id}/status` route is ALWAYS readable —
even under a hard-readiness failure — so operators can inspect the
current reason.

## 4 · Route catalogue

| Method | Path | Purpose |
|--------|------|---------|
| GET | `/subaccounts/deployments` | List configured Hybrid V2 deployments. |
| GET | `/subaccounts/deployments/{deployment_id}/status` | Deployment + indexer status. Always readable. |
| GET | `/accounts/{owner}/hybrid-v2/subaccounts` | Owner-scoped subaccount discovery. |
| GET | `/accounts/{owner}/hybrid-v2/history` | Owner-scoped normalized history. |
| GET | `/subaccounts/{subkey}` | Subaccount summary. |
| GET | `/subaccounts/{subkey}/collateral` | Per-token balance + aggregate reserved + available. |
| GET | `/subaccounts/{subkey}/reservations` | Per-engine reservation rows. |
| GET | `/subaccounts/{subkey}/positions` | Active Options positions. |
| GET | `/subaccounts/{subkey}/orders` | Order lifecycle rows. |
| GET | `/subaccounts/{subkey}/executions` | Complete matched executions (INCOMPLETE hidden). |
| GET | `/subaccounts/{subkey}/fees` | Fee / rebate / premium journal. |
| GET | `/subaccounts/{subkey}/recovery` | Recovery + escape state. |
| GET | `/subaccounts/{subkey}/history` | Subaccount-scoped normalized history. |
| GET | `/hybrid-v2/orders/{order_hash}` | Single order lifecycle row. |
| GET | `/hybrid-v2/history` | Normalized tagged history feed (owner and/or subkey filter). |
| GET | `/hybrid-v2/openapi.json` | OpenAPI 3.1 spec. |

Every route enforces:

- deployment scope (`deployment_id` query when multiple deployments
  are configured; single-deployment implicit resolution otherwise),
- deployment isolation (no cross-deployment leakage),
- Account 0 rejected as an ordinary user account,
- lowercase-hex address / bytes32 normalisation.

## 5 · Compatibility with `/accounts/:address/history/v2`

Verdict: `BACKEND_ACCOUNT_HISTORY_V2_PRESERVED_WITH_HYBRID_V2_ROUTE`.

The existing V1 perp/options history remains at
`/accounts/:address/history/v2`. Hybrid V2 canonical history lands at
the new route `/accounts/{owner}/hybrid-v2/history` and at the
subaccount-scoped `/subaccounts/{subkey}/history`. No V1 semantics are
altered.

## 6 · Normalized history model

`src/api/hybrid_v2_read/history.rs::HistoryEvent`:

- Common envelope: `event_id`, `deployment_id`, `chain_id`,
  `block_number`, `block_hash`, `tx_hash`, `tx_index`, `log_index`,
  `timestamp_ms`, `finalized`, `direction`, `owner`, `subaccount_id`,
  `subkey`, `related_order_hash`, `related_execution_id`.
- Payload: tagged union keyed on `family`. Every canonical event kind
  maps to one variant of `HistoryEventPayload`, e.g. `DEPOSIT`,
  `WITHDRAW`, `INTERNAL_TRANSFER`, `RESERVATION_INCREASE`,
  `RESERVATION_RELEASE`, `OPTION_ORDER_FILLED`,
  `MATCHED_EXECUTION`, `PREMIUM_TRANSFERRED`, `FEE_CHARGED`,
  `REBATE_PAID`, `MIN_VALID_NONCE_ADVANCED`, `RECOVERY_REQUESTED`,
  `RECOVERY_ACTIVATED`, `RECOVERY_CANCELLED`, `RECOVERY_FINALIZED`,
  `RECOVERY_FINALIZATION_WITHDRAWN`, `RECOVERY_EPOCH_INCREMENTED`,
  `OWNER_RECOVERY_EPOCH_ADVANCED`,
  `SUBACCOUNT_RECOVERY_EPOCH_ADVANCED`, `PAUSE_FLAG_CHANGED`,
  `BAD_DEBT_SOCIALIZED`, `CAPABILITY_EVENT`, `RISK_EVENT`,
  `MANIFEST`, `OTHER`.
- Direction classified per event: `INBOUND` / `OUTBOUND` / `INTERNAL`
  / `METADATA` — for transfers, direction is derived by comparing the
  filter target against `from_subkey` / `to_subkey`.

## 7 · Filters

`HistoryQuery`:

- `deployment_id`, `owner`, `subkey`, `subaccount_id`
- `families` (comma-separated `SCREAMING_SNAKE_CASE` labels)
- `token`, `series_id`, `order_hash`, `execution_id`, `direction`
- `from_block`, `to_block`
- `consistency`, `cursor`, `limit`

Invalid filter combinations (unknown direction / excessive limit)
return typed 400 with a stable `code`.

## 8 · Pagination

`src/api/hybrid_v2_read/cursor.rs`:

- Cursor is base64url(JSON) with version `1`.
- Binding: `(deployment_id, consistency, filter_hash, indexed_head_hash)`.
- Filter hash is SHA-256 truncated to 128 bits over the filter's stable
  JSON form.
- Deployment / consistency / filter mismatch → 400 `INVALID_CURSOR`.
- Stale block hash under `consistency=indexed` → 409 `STALE_CURSOR`.
- Bounded `limit` (default 100, max 1000 via `MAX_PAGE_LIMIT`).

Ordering: block DESC → tx_index DESC → log_index DESC → `event_id` DESC
as tie-breaker.

## 9 · Reorg behaviour

For `consistency=indexed`:

- Cursor carries the indexed head hash at page-issue time.
- When the underlying runtime experiences a reorg (the raw log journal
  invalidates the old block hash), the next paginated call fails with
  409 `STALE_CURSOR`. Clients restart from page 1.

For `consistency=finalized`:

- The finalized cursor is stable under bounded reorgs because the
  finalized head only advances monotonically.

Tests: `hybrid_v2_read_api_property_tests::reorg_stale_cursor_returns_409_conflict`.

## 10 · Serialization rules

- Addresses: lowercase `0x` + 40 hex.
- bytes32: lowercase `0x` + 64 hex.
- uint128 / uint256 economic amounts: decimal String (never JSON float).
- Enums: `SCREAMING_SNAKE_CASE` explicit strings.
- Timestamps: milliseconds since epoch.

Verdict: `BACKEND_SUBACCOUNT_EXACT_SERIALIZATION_VALIDATED`.

## 11 · Structured errors

`ApiErrorBody { code, message, retryable, detail? }`. Codes:

- 400: `INVALID_ADDRESS`, `INVALID_SUBKEY`, `INVALID_DEPLOYMENT`,
  `ACCOUNT_0_INVALID`, `INVALID_CURSOR`, `INVALID_FILTER_COMBINATION`,
  `UNSUPPORTED_CONSISTENCY`, `PAGE_LIMIT_EXCEEDED`.
- 404: `DEPLOYMENT_NOT_FOUND`, `SUBACCOUNT_NOT_FOUND`,
  `ORDER_NOT_FOUND`, `UNKNOWN_ROUTE`.
- 409: `STALE_CURSOR`.
- 500: `MALFORMED_CANONICAL_DATA`, `INTERNAL_INCONSISTENCY`.
- 503: `INDEXER_NOT_READY`, `MANIFEST_MISMATCH`,
  `RECONCILIATION_DRIFT`.

No raw database / RPC / signature leak. `retryable` is true for 503 and
409.

## 12 · OpenAPI 3.1

Full static spec at `src/api/hybrid_v2_read/openapi.rs`. Served at
`GET /hybrid-v2/openapi.json`. Every route documented with parameter
types, canonicality metadata schema, error schema, decimal-string
integer note, opaque cursor note.

Snapshot enforced by `hybrid_v2_read_api_tests::openapi_route_returns_json_spec`.

## 13 · Security

- Every SQL-eligible query path routes through
  `HybridV2QueryRepository` — no user-provided column / order
  expression, no string interpolation.
- Address / bytes32 parsers reject non-hex before touching state.
- Deployment scoping enforced at handler entry; no cross-deployment
  data leakage possible.
- No signatures / Merkle proofs exposed.
- Bounded `limit` per handler; no unbounded scans.
- Runtime lock poisoning surfaces as 500 `INTERNAL_INCONSISTENCY`
  rather than panic.

## 14 · Performance

- Read handlers borrow the in-memory `ProjectionState` under a read
  lock — no per-item RPC, no N+1 SQL.
- Pagination cap: `MAX_PAGE_LIMIT = 1000`.
- History filter uses `HashSet`-backed containment for family filters
  and deterministic ordering via a single sort of the produced
  history slice.
- Deterministic observation: 20-block frozen fixture (40 canonical
  events) paginated at limit=5 → 8 pages, no duplicates, no gap.

## 15 · Tests

- `tests/hybrid_v2_read_api_tests.rs` — 24 handler / route /
  serialisation / metadata / readiness tests.
- `tests/hybrid_v2_read_api_property_tests.rs` — 5 bounded property
  tests (pagination completeness, filtered pagination, reorg stale
  cursor, sibling isolation, exact integer round-trip).

## 16 · Files landed

- `src/api/hybrid_v2_read/mod.rs`
- `src/api/hybrid_v2_read/state.rs`
- `src/api/hybrid_v2_read/metadata.rs`
- `src/api/hybrid_v2_read/errors.rs`
- `src/api/hybrid_v2_read/serialization.rs`
- `src/api/hybrid_v2_read/cursor.rs`
- `src/api/hybrid_v2_read/history.rs`
- `src/api/hybrid_v2_read/handlers.rs`
- `src/api/hybrid_v2_read/router.rs`
- `src/api/hybrid_v2_read/openapi.rs`
- `tests/hybrid_v2_read_api_tests.rs`
- `tests/hybrid_v2_read_api_property_tests.rs`

## 17 · Exact next backend milestone

`BACKEND-SUBACCOUNT-EXECUTION-AND-SIGNER-INTEGRATION-V1` — awaiting
product-owner authorization.

---

## 22 · Runtime-wiring closure — 2026-07-31

`BACKEND-SUBACCOUNT-READ-API-RUNTIME-WIRING-CLOSURE-V1`. The read API
implemented at `9d6097a` was previously reachable only via the
isolated `build_hybrid_v2_read_router` factory. This closure mounts it
into the real backend application router.

### 22.1 AppState field

`src/api/http.rs`:

```rust
pub struct AppState {
    // ... existing fields ...
    pub hybrid_v2_read: crate::api::hybrid_v2_read::HybridV2ApiState,
}
```

Default: `HybridV2ApiState::empty()` — no deployment configured.
Populated via the builder `AppState::with_hybrid_v2(state)` once an
operator wires a validated `ManifestParams` + `ChainSource`.

### 22.2 Main router merge

`src/api/routes.rs::router(state)` now calls
`.merge(build_hybrid_v2_read_router(state.hybrid_v2_read.clone()))`
AFTER `.with_state(state)`. All 16 Hybrid V2 routes are reachable at
the top level of the production HTTP server, alongside V1 routes.

### 22.3 Fail-closed contract

- No deployment configured (default): `GET /subaccounts/deployments`
  returns `[]`; canonical routes return 400 `INVALID_DEPLOYMENT`;
  `/subaccounts/deployments/{id}/status` returns 404
  `DEPLOYMENT_NOT_FOUND`.
- Configured but not ready: canonical routes return typed 503;
  `/subaccounts/deployments/{id}/status` remains readable and reports
  the exact readiness reason.
- Configured + ready: all canonical routes serve projection data.

### 22.4 Production data-source verdict

**Verdict:** `BACKEND_HYBRID_V2_READ_STORE_GAP`.

Migrations 0044 + 0045 define the canonical projection schema, but no
code currently persists `ProjectionState` mutations into those tables.
The production canonical projection engine IS the in-memory
`IndexerRuntime` per the WP-01 architecture (Postgres is a
*rebuildable* projection, not the canonical source). A future
milestone — `BACKEND-HYBRID-V2-PROJECTION-PERSISTENCE-V1` — will add
the reducer→Postgres writer + a
`PostgresHybridV2ReadStore` reading from 0044/0045 tables, at which
point the store swap is a one-line change to the AppState builder.

Until then the runtime-backed read path is the operational production
path and IS the same `IndexerRuntime` (not a test fixture) that
maintains the canonical projection in memory. Every request goes
through `HybridV2QueryRepository` over the runtime's read-locked
`ProjectionState`, so the query surface is identical to the future
Postgres-backed path.

### 22.5 V1 API preservation

`GET /health`, `GET /markets`, `GET /accounts/:address/history/v2` and
every other pre-existing V1 route remains reachable through the merged
router. Regression-pinned in
`tests/hybrid_v2_read_api_main_router_tests.rs`.

### 22.6 OpenAPI integration

The full Hybrid V2 OpenAPI 3.1 spec is served through the real router
at `GET /hybrid-v2/openapi.json`. The backend does not currently
maintain an aggregate OpenAPI document; the Hybrid V2 spec stands on
its own as a stable per-namespace document, deterministically snapshot-
tested. When an aggregate document is introduced in a future
milestone, the Hybrid V2 schemas remain the canonical source for the
`/subaccounts/*` + `/hybrid-v2/*` + `/accounts/:owner/hybrid-v2/*`
surfaces.

### 22.7 Write-method safety

Every method other than GET on the Hybrid V2 surface resolves to 405
`Method Not Allowed` or 404 (axum's router behaviour). No POST / PUT /
PATCH / DELETE handler exists for any Hybrid V2 route.

### 22.8 Tests landed

`tests/hybrid_v2_read_api_main_router_tests.rs` — 15 tests through the
real `router(state)` path:

- default-state startup succeeds without Hybrid V2 configuration
- `/subaccounts/deployments` empty when unconfigured
- canonical route 400 when unconfigured
- `/health`, `/markets`, `/accounts/:address/history/v2` regression pins
- ready deployment: status, owner subaccounts, collateral, history
- malformed address / subkey → 400
- Account 0 → 400
- write methods rejected
- `/hybrid-v2/openapi.json` served
- manifest mismatch → 503
- status route readable when not ready

Baseline: 68 workspace suites, 2153+ tests. All hybrid_v2 tests remain
green.

### 22.9 Verdicts returned

- `BACKEND_HYBRID_V2_READ_RUNTIME_WIRING_MODEL_RESOLVED`
- `BACKEND_HYBRID_V2_READ_STORE_GAP` (see §22.4 — runtime-backed
  production path IS operational; Postgres backing is a follow-up
  milestone)
- `BACKEND_HYBRID_V2_READ_API_MOUNTED_IN_MAIN_APP`
- `BACKEND_HYBRID_V2_READ_API_FAIL_CLOSED_RUNTIME_POLICY_VALIDATED`
- `BACKEND_HYBRID_V2_READ_OPENAPI_RUNTIME_INTEGRATED`
- `BACKEND_HYBRID_V2_READ_MAIN_ROUTER_TESTS_PASS`
- `BACKEND_HYBRID_V2_READ_RUNTIME_PERFORMANCE_BOUNDED`
- `BACKEND_HYBRID_V2_READ_RUNTIME_SECURITY_VALIDATED`
- `BACKEND_SUBACCOUNT_READ_API_RUNTIME_WIRING_CLOSURE_V1_COMPLETE`
- `BACKEND_SUBACCOUNT_READ_API_AND_HISTORY_V1_OPERATIONALLY_COMPLETE`

Not returned: `BACKEND_HYBRID_V2_POSTGRES_READ_STORE_OPERATIONAL`,
`BACKEND_HYBRID_V2_READ_DATABASE_INTEGRATION_VALIDATED` — deferred to
`BACKEND-HYBRID-V2-PROJECTION-PERSISTENCE-V1`.

### 22.10 Non-goals still respected

- No public write / signing / broadcast route.
- No frontend change; no Solidity change; no chain broadcast.
- No secrets; no destructive migration; no Base mainnet.
