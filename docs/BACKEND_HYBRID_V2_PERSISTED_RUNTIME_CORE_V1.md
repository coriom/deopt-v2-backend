# `BACKEND-HYBRID-V2-PERSISTED-RUNTIME-CORE-V1`

Date landed: 2026-08-04
Milestone id: `BACKEND-HYBRID-V2-PERSISTED-RUNTIME-CORE-V1`
Predecessors:
- `BACKEND-HYBRID-V2-POSTGRES-PROJECTION-CORE-V1`
- `BACKEND-HYBRID-V2-POSTGRES-READ-STORE-V1` (stages 2A + 2B)
- `BACKEND-PG-MIGRATION-CHAIN-INTEGRITY-V1`
- session-1 partial: commits `cdcc18a`, `d56c01e`, `c7766e4`
Status: `IMPLEMENTED_AND_VALIDATED_EXPERIMENTAL`
Safety posture: `EXPERIMENTAL — NOT SECURITY APPROVED`

## Purpose

Complete the non-reorg persisted Hybrid V2 indexing lifecycle. Extend
the session-1 scaffolding to production quality for normal block
ingestion, restart safety, duplicate-delivery idempotency, block-atomic
failure rollback, baseline readiness state machine, worker + application
lifecycle wiring, and fail-closed configuration.

Explicitly excluded: reorg detection/replay, orphan invalidation,
rebuild locking + full rebuild, chain-view reconciliation, signer /
execution / broadcast integration. Those remain open for follow-up
stages.

## Frozen posture

- `CHAIN_STATE_IS_CANONICAL_POSTGRES_IS_A_REBUILDABLE_NON_CANONICAL_PROJECTION`
- `PRODUCTION_HYBRID_V2_HTTP_READS_USE_POSTGRES_ONLY`
- `NO_RUNTIME_STATE_PUBLICATION_BEFORE_POSTGRES_COMMIT` — the runtime
  serialises tick_and_persist behind a `RwLock<IndexerRuntime>` write
  lock; every HTTP reader routes through `PostgresHybridV2ReadStore`
  and never sees `IndexerRuntime.state` directly; the in-memory
  advance under the write lock is therefore not externally observable.
  Documented as a `SAFETY:` block on `IndexerRuntime::tick_and_persist`.
- No frontend / Solidity / signer / broadcast / chain-write.
- Base mainnet (chain_id 8453) refused unconditionally by
  `HybridV2Config::validate` and `IndexerRuntime::validate_manifest_binding`.

## Bootstrap model selected

**BOOTSTRAP-2 — replay canonical PostgreSQL journal.**

`IndexerRuntime::bootstrap_from_persistence()`:
1. Sets in-memory readiness to `ReadinessReason::Bootstrapping` (not
   persisted; transient).
2. Validates the manifest binding (rejects chain_id 8453).
3. Reads the persisted cursor + readiness snapshot via
   `HybridV2ProjectionStore::read_cursor` / `read_readiness`.
4. If `indexed_head_block == 0`: empty start → returns
   `BootstrapResult::EmptyStart`.
5. If `indexed_head_block > 0`: reads the canonical journal via
   `HybridV2ProjectionStore::read_canonical_journal(deployment_id)`,
   runs `RebuildService::new(event_version).replay_all(&journal)`,
   assigns the rebuilt `ProjectionState`, `ExecutionCorrelator`, and
   raw-logs journal to the runtime.
6. If replay's `final_head_block` diverges from the persisted cursor's
   `indexed_head_block`, sets readiness to `ProjectionFailure` and
   returns `BootstrapResult::Diverged { expected, replayed }`.
7. On success returns `BootstrapResult::HydratedFromJournal { blocks,
   events }`.

## Writer activation

`IndexerRuntime::tick_and_persist(source)`:
- Delegates to the sync `tick()` when no persistence is attached
  (existing test compat).
- Otherwise: applies + persists in one Postgres transaction via
  `HybridV2ProjectionStore::persist_block_atomic`. On failure: cursor
  rolls back, reducer state reverts to pre-block, correlator resets,
  readiness → `ProjectionFailure`, returns `RuntimeError::Persistence`.

## Configuration

New `hybrid_v2::config::HybridV2Config`:
- `enabled: bool` — master switch (default `false`).
- `deployment_id: i64` — must be > 0 when enabled.
- `chain_id: u64` — refused when `== 8453` (Base mainnet); only Base
  Sepolia + local dev accepted.
- `poll_interval_ms: u64` — `[100, 60_000]`.
- `confirmation_depth: u64` — `[0, 1024]`.
- `max_block_batch: u32` — `[1, 4096]`.
- `start_block: Option<u64>`.
- `cursor_name: String` — non-empty.

Env vars: `HYBRID_V2_ENABLED`, `HYBRID_V2_DEPLOYMENT_ID`,
`HYBRID_V2_CHAIN_ID`, `HYBRID_V2_POLL_INTERVAL_MS`,
`HYBRID_V2_CONFIRMATION_DEPTH`, `HYBRID_V2_MAX_BLOCK_BATCH`,
`HYBRID_V2_START_BLOCK`, `HYBRID_V2_CURSOR_NAME`.

Validated at startup by `main.rs` via `config.hybrid_v2.validate()?`.

## Application lifecycle

`src/main.rs`:
- Loads `HybridV2Config` via `HybridV2Config::from_env()`.
- Validates before any worker spawn.
- Logs `hybrid_v2_enabled` and `hybrid_v2_deployment_id` alongside
  every other startup gate.
- When `HYBRID_V2_ENABLED=true` AND `PERSISTENCE_ENABLED=true`, logs
  the configured state at `INFO` and defers the actual worker spawn
  — an RPC ChainSource implementation is required and will land in
  the next stage. The worker code, tests, and integration path are
  fully complete via `InMemoryChainSource` fixtures.
- When `HYBRID_V2_ENABLED=true` AND `PERSISTENCE_ENABLED=false`, logs
  a `WARN` refusing to spawn (persistence is mandatory).
- When `HYBRID_V2_ENABLED=false` (default), silently skips.

The `spawn_hybrid_v2_indexer_worker` function itself (from
`hybrid_v2::worker`) is unchanged: owns the runtime write lock during
each tick, releases during sleep, honours a `watch::Receiver<bool>`
shutdown signal.

## Readiness state machine

Added `ReadinessReason::Bootstrapping` and `ReadinessReason::Stopping`.
Both are hard-503 in both the enum-based
`hard_readiness_failure` and the string-prefix
`hard_readiness_failure_from_status`.

Effective transitions under the current lifecycle:
- Fresh start (no cursor): `AwaitingFirstBlock` → after first tick →
  ready (or `Behind` if lagging).
- Restart with persisted cursor: `Bootstrapping` (transient) → replay →
  ready.
- Persist failure: `ProjectionFailure(block, detail)` — persisted.
- Ready deployment catching up: `Behind` (soft-fail).

## Test surface

- `tests/hybrid_v2_runtime_persistence_integration.rs` — extended from
  5 to 22 tests (5 original + 6 atomicity + 5 restart + 4 idempotency +
  2 readiness).
- `tests/hybrid_v2_persisted_runtime_properties.rs` — **new**, 7
  bounded property tests.
- `src/hybrid_v2/config.rs` — 7 unit tests for the config parse /
  validate paths.
- All existing hybrid_v2 tests still green.

## Files landed

- `src/hybrid_v2/runtime.rs` — extended `bootstrap_from_persistence`,
  added `BootstrapResult::{HydratedFromJournal, Diverged, ChainForbidden}`
  variants + `validate_manifest_binding`.
- `src/hybrid_v2/persistence.rs` — added `read_canonical_journal` to the
  trait + Postgres + InMemory impls.
- `src/hybrid_v2/readiness.rs` — added `Bootstrapping` + `Stopping`
  variants + `#[derive(Default)]`.
- `src/hybrid_v2/config.rs` — **new** `HybridV2Config` with fail-closed
  validation.
- `src/hybrid_v2/mod.rs` — register `config` module + re-export.
- `src/api/hybrid_v2_read/metadata.rs` — map the two new readiness
  reasons in both mapping functions.
- `src/config/env.rs` — add `hybrid_v2` field on `AppConfig` + load
  via `HybridV2Config::from_env`.
- `src/main.rs` — validate at startup + log configured state.
- `tests/hybrid_v2_runtime_persistence_integration.rs` — 17 new tests.
- `tests/hybrid_v2_persisted_runtime_properties.rs` — **new**, 7 tests.
- `docs/BACKEND_HYBRID_V2_PERSISTED_RUNTIME_CORE_V1.md` — this file.

## Out of scope (deferred to follow-up stages)

- `BACKEND-HYBRID-V2-PERSISTED-REORG-RECOVERY-V1` — operational reorg
  detection, orphan invalidation, canonical replay, restart during
  reorg.
- `BACKEND-HYBRID-V2-PROJECTION-PERSISTENCE-CLOSURE-V1` — deployment
  rebuild lock, full rebuild workflow, chain-view reconciliation
  operational, final closure.
- Signer / execution / broadcast integration.
- Live RPC ChainSource implementation (required to physically activate
  the worker in production).
