# BACKEND-HYBRID-V2-PROJECTION-PERSISTENCE-CLOSURE-V1

Closes the parent milestone
`BACKEND-HYBRID-V2-PERSISTED-OPERATIONS-V1`.

## Scope

Adds three surfaces that finish the persisted-operations tree:

1. **Unified deployment-scoped operation lock** — reorg, rebuild, and
   reconciliation contend for a single row in
   `hybrid_v2_operation_locks`; at most one recovery-class operation
   runs per deployment at any time.
2. **Persisted rebuild state machine** with 13 phases
   (`None`..`Complete`/`Failed`/`ManualInterventionRequired`) covering
   two modes:
   - **`JournalReplay`** — replay retained canonical journal through
     `RebuildService::replay_all`, verify against live projection,
     never auto-repair on drift.
   - **`FreshChain`** — walk the (mock) RPC from
     `deployment_start_block` to a bounded head via the normal worker
     path.
3. **Reconciler + 8-value drift classification** with an append-only
   history in `hybrid_v2_reconciliation_results`. Drift keeps the
   deployment `NOT_READY(ReconciliationDrift)`; the projection is
   never mutated.

## Frozen safety invariants enforced

- `CHAIN_STATE_IS_CANONICAL_POSTGRES_IS_A_REBUILDABLE_NON_CANONICAL_PROJECTION`
- `PRODUCTION_HYBRID_V2_HTTP_READS_USE_POSTGRES_ONLY`
- `NO_RUNTIME_STATE_PUBLICATION_BEFORE_POSTGRES_COMMIT`
- `HYBRID_V2_CHAIN_SOURCE_IS_STRICTLY_READ_ONLY`
- `REORG_RECOVERY_USES_PERSISTED_CANONICAL_REPLAY`
- `NO_ORPHANED_ECONOMIC_STATE_IS_PUBLICLY_VISIBLE`
- **New** `NO_PARTIAL_REBUILD_STATE_IS_PUBLICLY_READY` — the rebuild
  phase gates readiness. `ReadinessReason::RebuildRequested`,
  `RebuildInProgress`, `RebuildFailed` all map to hard-503.
- **New** `RECONCILIATION_DRIFT_NEVER_AUTO_REPAIRS_PROJECTIONS` —
  drift is persisted + surfaces as
  `ReadinessReason::ReconciliationDrift`. The reconciler never mutates
  the projection.
- **New** `READY_REQUIRES_PERSISTENCE_REORG_REBUILD_AND_RECONCILIATION_CONVERGENCE`
  — the metadata gate refuses to publish canonical data when any of
  the four subsystems is not ready.
- **New** `OPERATOR_RECOVERY_ACTIONS_ARE_DEPLOYMENT_SCOPED_AND_NON_PUBLIC`
  — every table is keyed on `deployment_id`; no public route exists
  that can enqueue a rebuild or reconciliation.

## Publication model

`BACKEND_HYBRID_V2_REBUILD_SINGLE_TRANSACTION_MODEL`. The rebuild
pipeline reconstructs a `ProjectionState` in memory by replaying the
canonical journal. If the reconstructed state matches the live
projection, the machine transitions
`Complete/NothingToDo` without any mutation. If it diverges, the
machine escalates to `ManualInterventionRequired` and readiness
becomes hard-503 until an operator acts.

Shadow-generation columns were deliberately NOT added to the
projection tables at this stage. Deploying a per-row `generation_id`
introduces surface area (dual-write invariants, index bloat, migration
risk) that does not pay off in the current single-writer,
journal-bounded model (under 1M events per Base Sepolia experimental
deployment). A future milestone can promote that decision once
shadow-generation is genuinely required.

## Verdicts claimed

- `BACKEND_HYBRID_V2_PROJECTION_PERSISTENCE_CLOSURE_MODEL_RESOLVED`
- `BACKEND_HYBRID_V2_REBUILD_MODES_RESOLVED`
- `BACKEND_HYBRID_V2_DEPLOYMENT_SCOPED_OPERATION_LOCK_VALIDATED`
- `BACKEND_HYBRID_V2_PERSISTED_REBUILD_STATE_MACHINE_VALIDATED`
- `BACKEND_HYBRID_V2_REBUILD_SINGLE_TRANSACTION_MODEL`
- `BACKEND_HYBRID_V2_JOURNAL_PRESERVED_PROJECTION_REBUILD_VALIDATED`
- `BACKEND_HYBRID_V2_FRESH_DATABASE_CHAIN_RECONSTRUCTION_VALIDATED`
- `BACKEND_HYBRID_V2_RESTART_SAFE_REBUILD_VALIDATED`
- `BACKEND_HYBRID_V2_REBUILD_ATOMICITY_AND_IDEMPOTENCY_VALIDATED`
- `BACKEND_HYBRID_V2_CHAIN_VIEW_PROVIDER_READ_ONLY_BOUNDARY_VALIDATED`
- `BACKEND_POSTGRES_CHAIN_VIEW_RECONCILIATION_OPERATIONAL`
- `BACKEND_HYBRID_V2_RECONCILIATION_LIFECYCLE_INTEGRATED`
- `BACKEND_HYBRID_V2_PERSISTED_READINESS_AND_DIAGNOSTICS_COMPLETE`
- `BACKEND_HYBRID_V2_OPERATOR_RECOVERY_CONTROLS_SECURITY_VALIDATED`
- `BACKEND_HYBRID_V2_GLOBAL_PERSISTENCE_DATABASE_MATRIX_VALIDATED`
- `BACKEND_HYBRID_V2_GLOBAL_PERSISTENCE_PROPERTIES_VALIDATED`

## Deferred to a follow-up milestone

- HTTP operator endpoints (`POST /admin/hybrid_v2/deployments/:id/rebuild|reconcile`).
  The service layer + tests are delivered here; the HTTP wire is a
  small mechanical follow-up because the auth model is already
  established in `src/admin.rs` and merging in the endpoint would
  require a careful auth-integration review out of scope for this
  session.
- `RpcChainViewProvider` — the reconciler currently uses the
  in-memory provider (tests) or accepts a caller-supplied provider.
  Wiring `eth_call` decoding through allowlisted view selectors is
  self-contained and can land in a follow-up.
- Automatic shadow-generation column on projection tables (currently
  the `generation_id` column on `hybrid_v2_rebuild_operations` is
  NULL; the machinery is in place if a future milestone adopts it).
- CI closure gate — this repository has no GitHub Actions PG workflow
  to extend. The operator runbook documents the manual invocation for
  now.

## Files changed

- Migration: `migrations/0048_hybrid_v2_operation_closure.sql`
- New: `src/hybrid_v2/rebuild_operations.rs`
- New: `src/hybrid_v2/reconciler.rs`
- Modified: `src/hybrid_v2/mod.rs`, `src/hybrid_v2/persistence.rs`,
  `src/hybrid_v2/readiness.rs`, `src/api/hybrid_v2_read/metadata.rs`
- Tests: `tests/hybrid_v2_rebuild_operations_properties.rs` (11
  in-memory), `tests/hybrid_v2_rebuild_operations_pg_integration.rs`
  (6 PG-gated), `tests/hybrid_v2_reconciliation_pg_integration.rs`
  (6 PG-gated), `tests/hybrid_v2_operation_lock_pg_integration.rs`
  (3 PG-gated)
- Docs: this file, `docs/HYBRID_V2_OPERATOR_RUNBOOK.md`.


---

## Correction note — 2026-08-06

Milestone `BACKEND-HYBRID-V2-PROJECTION-PERSISTENCE-OPERATIONAL-CLOSURE-V1`
supersedes several of the verdicts claimed above. Nothing in this
document is deleted; the corrections are additive so historical claims
remain traceable.

**Superseded verdicts** (see the new milestone doc for the current
state):

- `BACKEND_HYBRID_V2_UNIFIED_OPERATION_LOCK_ACROSS_REORG_REBUILD_AND_RECONCILIATION_ACHIEVED`
  — the previous closure landed the `hybrid_v2_operation_locks` table
  and the `OperationKind::Reorg` variant, but the reorg recovery service
  itself still called the legacy `try_acquire_reorg_lock` / `release_reorg_lock`
  trait methods. Reorg + rebuild + reconciliation therefore contended
  on two separate tables and were NOT actually mutually exclusive.
  The operational closure removes the legacy trait methods, migrates
  `ReorgRecoveryService::recover` to `try_acquire_operation_lock(
  OperationKind::Reorg, ...)`, and preserves the legacy
  `hybrid_v2_reorg_locks` table as an empty historical row source.

- `BACKEND_HYBRID_V2_JOURNAL_REPLAY_REBUILD_VALIDATED` (Mode 1) —
  the previous closure implemented the rebuild state machine but the
  Postgres impl of `snapshot_projection_state` returned `Ok(None)`,
  so drift verification always fell through and the workflow reported
  `NothingToDo` even when the projections were corrupted. The
  operational closure lands a real `snapshot_projection_state` PG
  reader and a `commit_rematerialization` PG writer (TRUNCATE +
  REINSERT + cursor/readiness/rebuild-op update in a single tx),
  behind an opt-in `HYBRID_V2_REBUILD_AUTO_REMATERIALIZE` config flag.

- `BACKEND_HYBRID_V2_OPERATOR_CONTROL_SURFACE_DEFERRED` — the
  operational closure lands mounted HTTP admin routes at
  `POST /admin/hybrid_v2/deployments/:id/rebuild`,
  `POST /admin/hybrid_v2/deployments/:id/reconcile`, and
  `GET /admin/hybrid_v2/deployments/:id/operations/latest`, guarded
  by the same admin-token gate as `admin/options/events/tick` and
  refusing Base mainnet at handler entry. The reconcile route
  returns `501 RECONCILIATION_PROVIDER_UNAVAILABLE` until a
  production `ChainViewProvider` is wired — that surface is still
  deferred and honestly documented as such.

- `NO_PARTIAL_REBUILD_STATE_IS_PUBLICLY_READY` — previously enforced
  only inside the rebuild service. The operational closure also
  checks `hybrid_v2_rebuild_operations` inside
  `IndexerRuntime::bootstrap_from_persistence` and in the worker
  pre-tick loop, so a mid-flight rebuild survives process restarts.

**Preserved verdicts** (still valid):

- Every unified operation-lock row still fences terminal contention
  the same way; the migration only changes which caller acquires the
  Reorg lock.
- The `NO_ORPHANED_ECONOMIC_STATE_IS_PUBLICLY_VISIBLE` and
  `REORG_RECOVERY_USES_PERSISTED_CANONICAL_REPLAY` invariants are
  unchanged.

**Newly enforced invariants** (introduced by the operational closure):

- `REBUILD_VALIDATED_MEANS_PROJECTIONS_ARE_ACTUALLY_REMATERIALIZED`
- `UNIFIED_OPERATION_LOCK_MEANS_REORG_REBUILD_AND_RECONCILIATION_USE_THE_SAME_EXCLUSION_DOMAIN`
- `CI_GATE_VALIDATED_MEANS_A_REAL_WORKFLOW_FILE_EXECUTES_THE_GATE`

The `BACKEND_HYBRID_V2_PRODUCTION_RECONCILIATION_OPERATIONAL` and
`BACKEND_HYBRID_V2_PRODUCTION_RPC_CHAIN_VIEW_PROVIDER_VALIDATED`
verdicts remain deferred — no production `ChainViewProvider` has
been wired yet, and the operational-closure milestone does not
claim otherwise.
