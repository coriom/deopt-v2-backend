# `BACKEND-HYBRID-V2-PERSISTED-OPERATIONS-V1` — Partial-progress landing

Date: 2026-08-03
Status: **PARTIAL — SCAFFOLDING ONLY**
Safety posture: `EXPERIMENTAL — NOT SECURITY APPROVED`

Milestone `BACKEND-HYBRID-V2-PERSISTED-OPERATIONS-V1` spans 26 parts
(A–Z). This landing delivers the foundation (writer activation +
startup bootstrap + background worker skeleton + basic integration
tests) so the next session can layer reorg operational, rebuild lock,
full rebuild, reconciliation, readiness state machine, and the
30+ integration matrix on top.

**Update 2026-08-04:** stage-3A landed as
`BACKEND-HYBRID-V2-PERSISTED-RUNTIME-CORE-V1` — see
`BACKEND_HYBRID_V2_PERSISTED_RUNTIME_CORE_V1.md`. The non-reorg
lifecycle (bootstrap-2 journal replay, complete atomicity matrix,
restart matrix, idempotency matrix, baseline readiness state machine
with `Bootstrapping`/`Stopping` variants, `HybridV2Config` +
fail-closed validation, `main.rs` lifecycle wire) is complete;
Parts J/K also close on this pass. Reorg (Parts J/K/L of the parent
brief) → next stage `BACKEND-HYBRID-V2-PERSISTED-REORG-RECOVERY-V1`.
Rebuild + reconciliation → `BACKEND-HYBRID-V2-PROJECTION-PERSISTENCE-CLOSURE-V1`.

**Update 2026-08-04 (2):** stage-3B landed as
`BACKEND-HYBRID-V2-LIVE-CHAIN-SOURCE-AND-WORKER-ACTIVATION-V1` —
see `BACKEND_HYBRID_V2_LIVE_CHAIN_SOURCE_AND_WORKER_ACTIVATION_V1.md`.
The `ChainSource` trait is now async, `RpcHybridV2ChainSource` is a
strictly read-only reqwest-based JSON-RPC source, `HybridV2Config`
gained `rpc_url` + retry/timeout/log-bound fields with `Debug`
redaction, and `main.rs` now actually spawns the supervised worker
(previous log-only branch removed). Part R of the parent brief
(worker wiring into `main.rs`) is therefore closed. Operational
reorg (Parts J/K/L) still remains for
`BACKEND-HYBRID-V2-PERSISTED-REORG-RECOVERY-V1`.

**Update 2026-08-05 (stage-3C):** landed as
`BACKEND-HYBRID-V2-PERSISTED-REORG-RECOVERY-V1` — see
`BACKEND_HYBRID_V2_PERSISTED_REORG_RECOVERY_V1.md`. Adds a
persisted, restart-safe reorg-recovery state machine
(`hybrid_v2_reorg_recovery` + `hybrid_v2_reorg_locks` in
migration 0047) driven by a new `ReorgRecoveryService`. The runtime
persists a DETECTED row on parent-hash mismatch and raises
`RuntimeError::ReorgRequired`; the worker catches the error, invokes
recovery (bounded ancestor search + bounded replacement fetch +
one-transaction invalidate/replay/commit via
`RebuildService::replay_all`), then rehydrates via
`bootstrap_from_persistence`. Four new `ReadinessReason` variants
map to hard-503. Frozen safety rules
`REORG_RECOVERY_USES_PERSISTED_CANONICAL_REPLAY`,
`NO_ORPHANED_ECONOMIC_STATE_IS_PUBLICLY_VISIBLE`, and
`NO_READY_STATE_DURING_INCOMPLETE_REORG_RECOVERY` are enforced
throughout. Parts J/K/L of the parent brief close on this landing.

## Delivered in this landing

- **Part A** — preflight (worktree artifact from prior milestone
  resolved).
- **Part B** — disposable Postgres 16 harness (rootless container,
  mode-600 secrets).
- **Part C** — baseline: 51/51 pass (migration integration 1,
  persistence-core 10, read-store 19, main-router 21).
- **Part D** — operational-runtime audit; identifies the ~300 LOC
  insertion points and confirms `PostgresHybridV2ProjectionStore` is
  already fully implemented from stage 1.
- **Part E** — `IndexerRuntime::bootstrap_from_persistence()` hydrates
  cursor + readiness from Postgres on restart. BOOTSTRAP-1 (partial:
  reducer state hydration deferred to rebuild pass).
- **Part F** — `IndexerRuntime::tick_and_persist(source)` applies the
  next canonical block and persists it in a single Postgres
  transaction. On failure: cursor stays, in-memory rolls back,
  readiness = NOT_READY(ProjectionFailure). Additive — sync `tick()`
  unchanged, existing tests unaffected.
- **Part R (partial)** — `src/hybrid_v2/worker.rs` provides
  `spawn_hybrid_v2_indexer_worker()` with graceful-shutdown watch
  channel. NOT yet wired into `src/main.rs` (production activation
  requires the operator's ChainSource + validated ManifestParams which
  live in a downstream milestone).
- **Part G/H/I (basic)** — 5 env-gated integration tests exercise
  writer atomicity, persist-failure rollback, restart bootstrap,
  duplicate-block idempotency, and no-reapply-after-successful-restart
  against real Postgres.

## NOT yet delivered — remains for next session(s)

- **Part G (full)** — inject persist failures at each of the 12+
  documented points (raw log, decoded event, per-projection kind,
  cursor, commit) and assert rollback semantics per point.
- **Part H (full)** — the full restart matrix (restart-after-failed
  block, provider retry after timeout, etc.).
- **Part I (full)** — the full idempotency suite (same-log twice,
  same-execution twice, provider retry post-timeout, batch duplicate).
- **Part J** — operational reorg detection under `tick_and_persist`
  (bounded common-ancestor search, excessive-depth fail-closed,
  restart-during-detection).
- **Part K** — orphan invalidation + canonical replay (mark orphaned
  blocks non-canonical, clear projections, replay replacement chain).
- **Part L** — restart during reorg (state-machine resumption).
- **Part M** — deployment-scoped rebuild lock (advisory lock or lock
  table with ownership/expiry).
- **Part N** — full rebuild workflow with the fixture matrix
  (multi-owner, 8-token universe, GTC/IOC/FOK, recovery epochs,
  premium/fee/rebate, escape lifecycle, disabled-token withdrawal).
- **Part O** — restart during rebuild.
- **Part P** — chain-view reconciliation persisted, drift classification.
- **Part Q** — full readiness state-machine (STARTING → INDEXING →
  READY → NOT_READY → REORG_RECOVERY → REBUILDING → RECONCILING →
  READY) with persisted transitions.
- **Part R (rest)** — wire `spawn_hybrid_v2_indexer_worker` into
  `main.rs` behind an `IndexerConfig` gate; validate deployment
  configuration on startup.
- **Part S** — full 30+ integration matrix vs real Postgres.
- **Part T** — bounded property tests (13+ properties from the brief).
- **Part U** — deterministic performance observations.
- **Part V** — security sweep (parameterized SQL, no chain-write, no
  secret leak, no readiness bypass, no unbounded reorg scan).
- **Part W** — existing behaviour regressions (workspace test run).
- **Part Z** — final consolidated documentation.

## Files landed in this session

- `src/hybrid_v2/runtime.rs` — extended with optional persistence
  fields + `tick_and_persist` + `bootstrap_from_persistence` +
  `PendingReorgAudit` + `BootstrapResult`.
- `src/hybrid_v2/worker.rs` — **new**,
  `spawn_hybrid_v2_indexer_worker`.
- `src/hybrid_v2/mod.rs` — register `worker` module + re-exports.
- `tests/hybrid_v2_runtime_persistence_integration.rs` — **new**, 5
  env-gated PG integration tests.
- `docs/BACKEND_HYBRID_V2_PERSISTED_OPERATIONS_V1_PROGRESS.md` — this
  file.

## Test surface added

- 5 new env-gated integration tests, 5/5 pass against real PG 16.14.
- 166+ existing hybrid_v2 tests all still pass (writer is optional
  and off by default).

## Frozen posture

- `CHAIN_STATE_IS_CANONICAL_POSTGRES_IS_A_REBUILDABLE_NON_CANONICAL_PROJECTION`
- `PRODUCTION_HYBRID_V2_HTTP_READS_USE_POSTGRES_ONLY`
- No auto-fallback from Postgres to memory.
- Cursor never advances before persist commit.
- No public signing / broadcast / chain write / mainnet.

## Safety

- No frontend / Solidity / signer change.
- No public write route added; no auth change.
- Disposable Postgres container + private mode-600 credential files
  torn down after commits.
- No credentials or URLs committed or logged.

**Update 2026-08-05 (closure):** parent milestone
`BACKEND-HYBRID-V2-PERSISTED-OPERATIONS-V1` is CLOSED by
`BACKEND-HYBRID-V2-PROJECTION-PERSISTENCE-CLOSURE-V1` — see
`BACKEND_HYBRID_V2_PROJECTION_PERSISTENCE_CLOSURE_V1.md` and the
operator runbook at `HYBRID_V2_OPERATOR_RUNBOOK.md`. The closure
adds the unified deployment-scoped operation lock
(`hybrid_v2_operation_locks` in migration 0048), a 13-phase
persisted rebuild state machine (`hybrid_v2_rebuild_operations`)
with both `JournalReplay` and `FreshChain` modes reusing
`RebuildService::replay_all` and the existing worker path
respectively, a reconciler + 8-value drift classification with
append-only history (`hybrid_v2_reconciliation_results`), two new
readiness variants (`RebuildRequested`, `ReconciliationInProgress`)
mapped to hard-503, and a comprehensive test matrix (11 in-memory
+ 15 PG-gated). Frozen safety rules
`NO_PARTIAL_REBUILD_STATE_IS_PUBLICLY_READY`,
`RECONCILIATION_DRIFT_NEVER_AUTO_REPAIRS_PROJECTIONS`,
`READY_REQUIRES_PERSISTENCE_REORG_REBUILD_AND_RECONCILIATION_CONVERGENCE`,
and
`OPERATOR_RECOVERY_ACTIONS_ARE_DEPLOYMENT_SCOPED_AND_NON_PUBLIC`
are enforced. Publication model:
`BACKEND_HYBRID_V2_REBUILD_SINGLE_TRANSACTION_MODEL`. Deferred to
follow-ups: HTTP operator endpoints (service layer + tests are in
place), `RpcChainViewProvider` wiring, CI closure workflow
(no existing PG workflow in this repo to extend).
