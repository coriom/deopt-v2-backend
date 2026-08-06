# BACKEND-HYBRID-V2-CHAIN-VIEW-PROVIDER-AND-RECONCILIATION-TASK-V1

Corrective milestone that lands the production `RpcChainViewProvider`,
the mounted admin reconciliation trigger, and the periodic
reconciliation worker on top of the persisted reconciliation surface
introduced by `BACKEND-HYBRID-V2-PROJECTION-PERSISTENCE-CLOSURE-V1`.

## Scope

- `src/hybrid_v2/rpc_chain_view.rs` — new `RpcChainViewProvider` struct
  that issues allowlisted, block-bound `eth_call` reads to the
  manifest's module addresses.
- `src/hybrid_v2/rpc_chain_source.rs` — `RpcHybridV2ChainSource` gains
  a narrow public `eth_call(to, data, block, allowed_selectors)` method
  that enforces the allowlist inside the source (defence in depth) and
  reuses the existing retry / timeout / redaction infrastructure.
- `src/hybrid_v2/reconciliation_worker.rs` — new periodic worker
  (`spawn_hybrid_v2_reconciliation_worker`) and a shared `tick_once`
  entry point used by both the worker and the admin route.
- `src/api/hybrid_v2_admin.rs::request_reconciliation` — the prior 501
  is replaced with a real trigger that acquires the unified operation
  lock, invokes `tick_once`, and persists the result.
- `src/hybrid_v2/config.rs` — three new fields:
  `reconciliation_enabled`, `reconciliation_periodic_ms`,
  `reconciliation_max_items_per_run`. All three are validated at
  startup with bounded ranges.
- `src/main.rs` — wires the provider + worker (and the runtime handle,
  worker config, and manifest) into `AppState` when
  `HYBRID_V2_RECONCILIATION_ENABLED=true`.

## Frozen invariants enforced

- `PRODUCTION_HYBRID_V2_HTTP_READS_USE_POSTGRES_ONLY` — the provider
  never mutates any projection row.
- `HYBRID_V2_CHAIN_SOURCE_IS_STRICTLY_READ_ONLY` — `eth_call` shares
  the same prohibited-method allowlist as every other read; write /
  sign / wallet methods are still refused inside the source.
- `RECONCILIATION_DRIFT_NEVER_AUTO_REPAIRS_PROJECTIONS` — the admin
  route + worker only *persist* a classification row; no rebuild or
  rematerialisation is triggered.
- `PRODUCTION_RECONCILIATION_USES_ALLOWLISTED_BLOCK_BOUND_ETH_CALLS`
  — every `eth_call` binds to `BlockRef::Number(indexed_block)` and
  the target address + selector are validated against the compile-time
  per-module allowlist.
- `RECONCILIATION_PROVIDER_FAILURE_IS_NOT_PROJECTION_DRIFT` —
  transport / RPC failures classify as `PROVIDER_UNAVAILABLE`. The
  admin route + worker mark the provider unavailable + persist the row
  and do *not* touch readiness (which stays clean until the underlying
  provider recovers).
- `READY_REQUIRES_NO_UNRESOLVED_RECONCILIATION_DRIFT` — the reconciler
  writes `ReadinessReason::ReconciliationDrift` when a divergent
  classification is observed.
- `ADMIN_RECONCILIATION_TRIGGER_IS_NON_PUBLIC_AND_DEPLOYMENT_SCOPED`
  — the admin route reuses `ensure_admin` (same as
  `admin/options/events/tick`), refuses Base mainnet at handler entry,
  and rejects a deployment mismatch between the URL path and the wired
  provider with a 409.
- `UNIFIED_OPERATION_LOCK_SERIALIZES_REORG_REBUILD_AND_RECONCILIATION`
  — every reconciliation run acquires
  `OperationKind::Reconciliation` on `hybrid_v2_operation_locks`.

## Supported chain view categories (this milestone)

- `manifest_hash` — pinned by the manifest embedded in the provider at
  construction. No `eth_call` required.
- `subaccount_owners` — `SubaccountRegistry.ownerOf(bytes32 subKey)`.
- `balances` — `CollateralVault.balanceWithYield(bytes32 subKey,
  address token)` per (subKey, token) pair.
- `recovery_state` — `RecoveryFinalizer.getRecoveryState(bytes32
  subKey)`, decoded to the symbolic label used by the reducer.

## UNSUPPORTED categories (returned as `UNSUPPORTED_VIEW`)

- `reservations` — no pinned view signature yet.
- `positions` — no pinned view signature yet.
- `order_lifecycle` — off-chain state.
- `matched_executions` — off-chain state.
- `active_series` — no pinned view signature yet.
- `fee_events` — off-chain journal.

The reducer + reconciler still walk these categories; the production
provider simply does not fetch a value for them. Because the reconciler
looks up chain values via `snap.balances.get(...)` etc. and defaults
missing entries to `"0"`, a projection with a non-zero reservation
would be reported as `PROJECTION_DRIFT`. To avoid false positives, the
periodic worker restricts its comparison universe to the categories
this provider supports; a future milestone will add the reservation /
position selectors and re-enable those comparisons.

## Admin API

`POST /admin/hybrid_v2/deployments/:deployment_id/reconcile`

- Headers: `x-admin-token` (as with every other admin route).
- Body: none.
- Response 200 body:
  ```json
  {
    "deployment_id": 42,
    "operation_id": 17,
    "classification": "CONVERGED",
    "status": "COMPLETED"
  }
  ```
- Errors:
  - 403 `ADMIN_DISABLED` / `ADMIN_TOKEN_REQUIRED` / `INVALID_ADMIN_TOKEN`
  - 404 `DEPLOYMENT_NOT_FOUND`
  - 403 `BASE_MAINNET_FORBIDDEN`
  - 503 `HYBRID_V2_NOT_CONFIGURED` (no projection store attached)
  - 503 `RECONCILIATION_PROVIDER_UNAVAILABLE` (provider not wired)
  - 503 `RECONCILIATION_RUNTIME_UNAVAILABLE`
  - 503 `RECONCILIATION_CONFIG_UNAVAILABLE`
  - 409 `RECONCILIATION_DEPLOYMENT_MISMATCH` (URL deployment_id
    disagrees with the wired provider — misconfiguration guardrail)
  - 409 `RECONCILIATION_SKIPPED` (operation lock contention with a
    rebuild or reorg)
  - 500 `RECONCILIATION_FAILED` (internal error persisting the row)

## Periodic worker configuration

- `HYBRID_V2_RECONCILIATION_ENABLED` — `1`/`true` to opt in. Default
  `false`.
- `HYBRID_V2_RECONCILIATION_PERIODIC_MS` — cadence, `0` disables the
  worker even when the surface is enabled. Bounded `[0, 86400000]`.
  Default `0`.
- `HYBRID_V2_RECONCILIATION_MAX_ITEMS_PER_RUN` — subaccount cap per
  tick. Bounded `[1, 1000000]`. Default `4096`.

## Readiness behaviour

- `CONVERGED`, `INDEXER_BEHIND`, `NON_FINAL_DIFFERENCE`,
  `PROVIDER_UNAVAILABLE`, `UNSUPPORTED_VIEW` are considered
  "converged or transient" — readiness is NOT flipped to not-ready.
- `PROJECTION_DRIFT`, `MANIFEST_MISMATCH`, `MALFORMED_CHAIN_RESPONSE`
  cause the reconciler to persist a `ReadinessSnapshot` with
  `ReadinessReason::ReconciliationDrift{ detail }`. The readiness
  surface stays not-ready until an operator restart clears it. No
  projection row is ever mutated by this path.

## No auto-repair

Every drift classification is persisted as-is. The worker never calls
`commit_rematerialization` or any other mutation entrypoint. The
operator inspects the persisted `mismatch_sample_json` on
`hybrid_v2_reconciliation_results` and decides whether to invoke the
admin rebuild route (`FRESH_CHAIN` requires an explicit ingestion
closure per the operational closure notes).
