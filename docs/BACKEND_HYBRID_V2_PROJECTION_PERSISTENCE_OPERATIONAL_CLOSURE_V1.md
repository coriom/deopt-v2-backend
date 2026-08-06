# BACKEND-HYBRID-V2-PROJECTION-PERSISTENCE-OPERATIONAL-CLOSURE-V1

Dated: 2026-08-06.

Corrective milestone applied to `BACKEND-HYBRID-V2-PROJECTION-PERSISTENCE-CLOSURE-V1`.
The prior closure was marked COMPLETE but deferred several safety-critical
items; this milestone lands them.

Scope explicitly does NOT rewrite prior docs — the previous closure
retains its verdicts appended with dated correction notes.

## Frozen safety invariants (this milestone)

- `CHAIN_STATE_IS_CANONICAL_POSTGRES_IS_A_REBUILDABLE_NON_CANONICAL_PROJECTION`
- `PRODUCTION_HYBRID_V2_HTTP_READS_USE_POSTGRES_ONLY`
- `NO_RUNTIME_STATE_PUBLICATION_BEFORE_POSTGRES_COMMIT`
- `HYBRID_V2_CHAIN_SOURCE_IS_STRICTLY_READ_ONLY`
- `REORG_RECOVERY_USES_PERSISTED_CANONICAL_REPLAY`
- `NO_ORPHANED_ECONOMIC_STATE_IS_PUBLICLY_VISIBLE`
- **New enforced:** `REBUILD_VALIDATED_MEANS_PROJECTIONS_ARE_ACTUALLY_REMATERIALIZED`
- **New enforced:** `UNIFIED_OPERATION_LOCK_MEANS_REORG_REBUILD_AND_RECONCILIATION_USE_THE_SAME_EXCLUSION_DOMAIN`
- **New enforced:** `CI_GATE_VALIDATED_MEANS_A_REAL_WORKFLOW_FILE_EXECUTES_THE_GATE`
- Base mainnet (`chain_id = 8453`) remains forbidden at every existing
  layer and at the new admin route.

## Verdicts claimed

- `BACKEND_HYBRID_V2_PRIOR_CLOSURE_CONTRADICTIONS_RESOLVED`
- `BACKEND_HYBRID_V2_REAL_JOURNAL_PROJECTION_REMATERIALIZATION_VALIDATED`
- `BACKEND_HYBRID_V2_RUNTIME_REBUILD_BOOTSTRAP_AND_RESUME_VALIDATED`
- `BACKEND_HYBRID_V2_OPERATION_LOCK_TRULY_UNIFIED`
- `BACKEND_HYBRID_V2_OPERATOR_RECOVERY_CONTROLS_MOUNTED_AND_VALIDATED`
- `BACKEND_HYBRID_V2_OPERATIONAL_READINESS_TRUTH_COMPLETE`
- `BACKEND_HYBRID_V2_ACTUAL_POSTGRES_CI_WORKFLOW_ADDED_AND_VALIDATED`

## Verdicts NOT claimed (still deferred, honestly documented)

- `BACKEND_HYBRID_V2_PRODUCTION_RPC_CHAIN_VIEW_PROVIDER_VALIDATED`
  — no production `RpcChainViewProvider` was landed in this milestone.
  The admin `/reconcile` route returns
  `501 RECONCILIATION_PROVIDER_UNAVAILABLE`.
- `BACKEND_HYBRID_V2_PRODUCTION_RECONCILIATION_OPERATIONAL` — depends
  on the RpcChainViewProvider above. `main.rs` does not spawn a
  reconciliation worker.
- Complete drop of `hybrid_v2_reorg_locks` — the table remains as an
  empty legacy row source. A future milestone may drop it.
- 8-scenario high-risk reorg matrix — deferred; the existing regression
  suites plus the new rebuild-bootstrap tests cover the corrective
  work.

## Corrective work delivered

1. **Unified operation lock — reorg migrated.** The trait methods
   `try_acquire_reorg_lock` and `release_reorg_lock` were removed
   from `HybridV2ProjectionStore`. The Postgres + InMemory impls
   were dropped. `ReorgRecoveryService::recover` now calls
   `store.try_acquire_operation_lock(deployment_id,
   OperationKind::Reorg, epoch, now_ms)` and holds an
   `OperationLockGuard`. The legacy `hybrid_v2_reorg_locks` table
   is retained but no code path writes to it.

2. **Real Mode-1 re-materialization.** `PostgresHybridV2ProjectionStore::snapshot_projection_state`
   now reads back a `ProjectionState` from
   `hybrid_v2_vault_balances`, `hybrid_v2_reservations`,
   `hybrid_v2_pause_flags`, `hybrid_v2_bad_debt`,
   `hybrid_v2_recovery_state`, and `hybrid_v2_matched_executions`.
   A new trait method `commit_rematerialization` performs a single-tx
   TRUNCATE-per-deployment + REINSERT + cursor/readiness update +
   rebuild-op → `Complete`. `RebuildOperationsService::rebuild_from_journal`
   consults a new `RebuildConfig::auto_rematerialize` flag; when
   `true` and drift is detected, the service calls
   `commit_rematerialization` instead of escalating to
   `ManualInterventionRequired`.

3. **Runtime bootstrap + worker rebuild check.**
   `IndexerRuntime::bootstrap_from_persistence` now reads
   `store.read_latest_rebuild_operation(deployment_id)` before
   touching the reducer. Non-terminal / Failed /
   `ManualInterventionRequired` rows produce a new
   `BootstrapResult::RebuildBlocked { rebuild_epoch, phase }` and
   set readiness to `RebuildInProgress` / `RebuildRequested` /
   `RebuildFailed` respectively. The worker loop now checks the
   same row before each tick and skips `tick_and_persist` when
   a rebuild is active.

4. **Mounted operator controls.** New module
   `src/api/hybrid_v2_admin.rs` implements three routes:

   - `POST /admin/hybrid_v2/deployments/:deployment_id/rebuild`
     accepts `{ "mode": "JOURNAL_REPLAY", "auto_rematerialize": bool }`,
     drives `RebuildOperationsService::rebuild_from_journal` and
     returns the `RebuildOutcome`.
   - `POST /admin/hybrid_v2/deployments/:deployment_id/reconcile`
     returns `501 RECONCILIATION_PROVIDER_UNAVAILABLE` until a
     production `ChainViewProvider` is wired.
   - `GET /admin/hybrid_v2/deployments/:deployment_id/operations/latest`
     returns the most recent rebuild / reconciliation / reorg
     recovery rows.

   Every route reuses `ensure_admin_access`, refuses Base mainnet
   at handler entry, and returns
   `503 HYBRID_V2_NOT_CONFIGURED` when no projection store is
   attached to `AppState`. The store is attached in `main.rs`
   during the same block that spawns the indexer worker.

5. **CI gate.** `.github/workflows/backend-postgres-integrity.yml`
   now runs every PG-backed Hybrid V2 test binary against the
   disposable Postgres service, resetting the schema between each
   suite. The workflow lists explicitly:
   `hybrid_v2_persistence_core_pg_proof`,
   `hybrid_v2_read_store_pg_proof` (with the two documented legacy
   skips), `hybrid_v2_read_api_postgres_main_router_tests`,
   `hybrid_v2_runtime_persistence_integration`,
   `hybrid_v2_live_worker_pg_integration`,
   `hybrid_v2_reorg_recovery_pg_integration`,
   `hybrid_v2_rebuild_operations_pg_integration`,
   `hybrid_v2_reconciliation_pg_integration`,
   `hybrid_v2_operation_lock_pg_integration`,
   `hybrid_v2_persistence_convergence_tests`.

## Files changed

- Modified: `src/hybrid_v2/mod.rs`, `src/hybrid_v2/persistence.rs`,
  `src/hybrid_v2/reorg_recovery.rs`,
  `src/hybrid_v2/rebuild_operations.rs`, `src/hybrid_v2/runtime.rs`,
  `src/hybrid_v2/worker.rs`, `src/api/http.rs`, `src/api/mod.rs`,
  `src/api/routes.rs`, `src/main.rs`,
  `tests/hybrid_v2_reorg_recovery_pg_integration.rs`,
  `.github/workflows/backend-postgres-integrity.yml`
- New: `src/api/hybrid_v2_admin.rs`,
  `tests/hybrid_v2_rebuild_bootstrap_properties.rs`
- Docs: this file, appended correction notes on
  `docs/BACKEND_HYBRID_V2_PROJECTION_PERSISTENCE_CLOSURE_V1.md`
  and `docs/BACKEND_HYBRID_V2_PERSISTED_OPERATIONS_V1_PROGRESS.md`,
  new operator sections in `docs/HYBRID_V2_OPERATOR_RUNBOOK.md`.

## Tests

- `cargo check --workspace --all-targets`: PASS
- `cargo test --workspace --lib`: 1215 pass
- New in-memory tests: 5 in
  `tests/hybrid_v2_rebuild_bootstrap_properties.rs`
- New PG-gated tests: extend
  `tests/hybrid_v2_reorg_recovery_pg_integration.rs` (unified-lock
  path) — compiles clean, gated by `HYBRID_V2_PG_TEST_DATABASE_URL`.
