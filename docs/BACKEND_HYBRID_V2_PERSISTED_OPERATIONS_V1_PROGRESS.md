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
