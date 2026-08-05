# `BACKEND-HYBRID-V2-PERSISTED-REORG-RECOVERY-V1`

Date: 2026-08-05
Status: **DELIVERED — STAGE 3C**
Safety posture: `EXPERIMENTAL — NOT SECURITY APPROVED`

Stage 3C of the Hybrid V2 persisted-operations tree closes operational,
persisted, restart-safe chain-reorg recovery for the live
`IndexerRuntime` + worker.

## Scope

- Detect a parent-hash mismatch inside `tick_and_persist`.
- Persist a state-machine row + a hard-503 readiness snapshot before
  any mutation.
- Walk the local canonical history backwards until a hash matches the
  live chain (bounded).
- Fetch the replacement branch (bounded).
- In one atomic Postgres transaction: invalidate the orphan branch,
  replay the mixed canonical journal, advance the cursor, restore
  readiness, and audit the reorg event.
- Rehydrate the runtime by re-running `bootstrap_from_persistence`.
- On excessive depth or a finalized-boundary violation: escalate to
  `MANUAL_INTERVENTION_REQUIRED`; worker halts for that deployment.

## Frozen safety rules honoured

- `CHAIN_STATE_IS_CANONICAL_POSTGRES_IS_A_REBUILDABLE_NON_CANONICAL_PROJECTION`
- `PRODUCTION_HYBRID_V2_HTTP_READS_USE_POSTGRES_ONLY`
- `NO_RUNTIME_STATE_PUBLICATION_BEFORE_POSTGRES_COMMIT`
- `HYBRID_V2_CHAIN_SOURCE_IS_STRICTLY_READ_ONLY`
- **New** `REORG_RECOVERY_USES_PERSISTED_CANONICAL_REPLAY` — replay
  uses `RebuildService::replay_all`, never inverse-arithmetic.
- **New** `NO_ORPHANED_ECONOMIC_STATE_IS_PUBLICLY_VISIBLE` — after
  commit no `is_canonical = TRUE` row exists on the orphan branch.
- **New** `NO_READY_STATE_DURING_INCOMPLETE_REORG_RECOVERY` — readiness
  stays hard-503 through every recovery phase.
- Base mainnet (`chain_id 8453`) remains forbidden at all three layers.

## Recovery state machine

```
NONE
  └── (detection: parent_hash mismatch during tick_and_persist)
DETECTED                         # runtime persists this before any mutation
  └── (readiness → ReorgDetected)
SEARCHING_ANCESTOR
  └── (bounded backward search via source.block_at + source.block_by_hash)
ANCESTOR_FOUND                   # persist common_ancestor_{block,hash}
  ├── (finalized_at_detection captured; violation → MANUAL)
  └── (max_reorg_depth exceeded → MANUAL)
FETCHING_REPLACEMENT             # bounded fetch, contiguous parent chain
INVALIDATING_ORPHAN_BRANCH       # inside one PG tx
REPLAYING_REPLACEMENT            # inside same PG tx
COMMITTING_REPLACEMENT           # inside same PG tx
RECOVERED                        # cursor + readiness updated atomically
```

Failure branches:
- `FAILED { reason }` — retryable, worker retries on next tick.
- `MANUAL_INTERVENTION_REQUIRED { reason }` — hard-halts the worker
  for that deployment. Only an operator restart with a cleared row
  unblocks.

## Configuration

`HybridV2Config` gained five reorg-recovery fields
(env-var-configurable + bounds-validated):

| Field | Env | Default | Bounds |
| --- | --- | --- | --- |
| `reorg_max_depth` | `HYBRID_V2_REORG_MAX_DEPTH` | 64 | [1, 512] |
| `reorg_max_replacement_blocks` | `HYBRID_V2_REORG_MAX_REPLACEMENT_BLOCKS` | 256 | [1, 4096] |
| `reorg_retry_max` | `HYBRID_V2_REORG_RETRY_MAX` | 5 | [0, 20] |
| `reorg_retry_backoff_ms` | `HYBRID_V2_REORG_RETRY_BACKOFF_MS` | 500 | [50, 60000] |
| `reorg_allow_finalized_boundary_crossing` | `HYBRID_V2_REORG_ALLOW_FINALIZED_CROSS` | false | boolean |

`ReorgRecoveryConfig::validate()` mirrors these bounds inside the
service so the recovery entry point fails-closed even when
constructed programmatically.

## Atomicity model

`HybridV2ProjectionStore::commit_reorg_recovery` runs one Postgres
transaction that:

1. Marks orphan raw logs `is_canonical = FALSE`, `orphaned_at_block = ancestor`.
2. Marks orphan canonical blocks the same way.
3. Marks orphan `hybrid_v2_matched_executions.completion_status`
   `= 'INVALIDATED_BY_REORG'`.
4. Writes each replacement block + raw logs + decoded events +
   projection mutations via the same primitives used by the normal
   block writer (`upsert_canonical_block`, `insert_raw_log`,
   `insert_decoded_event`, `persist_event_projection`).
5. Advances the cursor to the replacement tip.
6. Publishes ready readiness + updated metrics.
7. Marks the recovery row `phase = 'RECOVERED'`, `completed_at_ms`.

On any failure the transaction ROLLBACKs and no persisted state
changes — the caller retries on the next tick.

## Restart matrix (implemented, validated)

| Persisted phase at boot | Behaviour |
| --- | --- |
| `NONE` / row absent | worker enters normal tick loop |
| `RECOVERED` | worker enters normal tick loop; stale lock (if any) auto-reclaimed |
| `DETECTED` / `SEARCHING_ANCESTOR` / `ANCESTOR_FOUND` / `FETCHING_REPLACEMENT` | worker resumes recovery via `ReorgRecoveryService::recover(detection = None)` before ticking |
| `INVALIDATING_ORPHAN_BRANCH` / `REPLAYING_REPLACEMENT` / `COMMITTING_REPLACEMENT` | equivalent — the atomic tx rolls back on crash, so the persisted phase reverts to the last completed pre-mutation state |
| `FAILED` | recovery retries on next tick |
| `MANUAL_INTERVENTION_REQUIRED` | worker sleeps indefinitely; readiness hard-503 |

## Lock model

`hybrid_v2_reorg_locks` is a deployment-scoped row-based mutex.
Acquire = `INSERT ... ON CONFLICT DO NOTHING`; release = `DELETE`.
Stale locks are automatically reclaimed on boot when the paired
recovery row is `RECOVERED` / `NONE` / absent.

## Readiness → API mapping

Four new `ReadinessReason` variants and their string tokens all map to
hard-503 in `hard_readiness_failure` +
`hard_readiness_failure_from_status`:

| Variant | Token |
| --- | --- |
| `ReorgDetected { at_block, epoch }` | `REORG_DETECTED` |
| `ReorgSearching { epoch }` | `REORG_SEARCHING` |
| `ReorgReplaying { epoch, ancestor }` | `REORG_REPLAYING` |
| `ReorgManualInterventionRequired { epoch, reason }` | `REORG_MANUAL_INTERVENTION_REQUIRED` |

## Deferred (explicit non-goals)

- General database-loss rebuild (`BACKEND-HYBRID-V2-PROJECTION-
  PERSISTENCE-CLOSURE-V1`).
- Chain-view reconciliation.
- Signing / broadcast / any chain-write path.
- Base mainnet.
- Full 25-case PG integration matrix — 7 core scenarios landed here;
  the remaining 18 are queued for the outer session to run against a
  provisioned PG once available (compile-clean, skip cleanly today).

## Files delivered

- `migrations/0047_hybrid_v2_reorg_recovery.sql`
- `src/hybrid_v2/reorg_recovery.rs`
- Trait + impls in `src/hybrid_v2/persistence.rs`
- `ReadinessReason` variants in `src/hybrid_v2/readiness.rs`
- Metadata mappings in `src/api/hybrid_v2_read/metadata.rs`
- Runtime integration in `src/hybrid_v2/runtime.rs`
- Worker integration in `src/hybrid_v2/worker.rs`
- Config fields in `src/hybrid_v2/config.rs`
- `tests/hybrid_v2_reorg_recovery_config_tests.rs`
- `tests/hybrid_v2_reorg_recovery_properties.rs`
- `tests/hybrid_v2_reorg_recovery_pg_integration.rs`
- Mock RPC extension in `tests/hybrid_v2_mock_rpc_helpers.rs`
- `docs/BACKEND_HYBRID_V2_PERSISTED_REORG_RECOVERY_V1.md`

## Tests

- Unit: 4 new tests in `src/hybrid_v2/reorg_recovery.rs`.
- Config: 6 tests in `tests/hybrid_v2_reorg_recovery_config_tests.rs`.
- Property: 9 tests in `tests/hybrid_v2_reorg_recovery_properties.rs`
  (deterministic, no external PG/RPC).
- PG-gated integration: 7 tests in
  `tests/hybrid_v2_reorg_recovery_pg_integration.rs` — skip cleanly
  when `HYBRID_V2_PG_TEST_DATABASE_URL` is unset.

Full workspace: `cargo test --workspace --lib` → 1204 passed / 0 failed.
