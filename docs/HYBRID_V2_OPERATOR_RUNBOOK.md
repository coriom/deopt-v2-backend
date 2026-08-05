# Hybrid V2 Operator Runbook

Operator-facing procedures for the Hybrid V2 experimental indexer
subsystem (Base Sepolia / local Anvil / local Hardhat only — Base
mainnet is refused at every layer).

Absent from Hybrid V2 at this stage: chain writes, signing, transaction
broadcast, or Base mainnet operation. Any procedure that would require
one of those is out of scope for this runbook.

## 1. Configuration

Env vars read by `HybridV2Config::from_env()`:

| Env var                                     | Default | Bounds        | Notes                                     |
|---------------------------------------------|---------|---------------|-------------------------------------------|
| `HYBRID_V2_ENABLED`                         | `false` | bool          | Master switch. When false everything skips |
| `HYBRID_V2_DEPLOYMENT_ID`                   | -       | > 0           | Assigned by `upsert_deployment`           |
| `HYBRID_V2_CHAIN_ID`                        | -       | 84532/31337/1337 | Base mainnet (8453) is refused          |
| `HYBRID_V2_POLL_INTERVAL_MS`                | 1000    | [100, 60000]  |                                           |
| `HYBRID_V2_MAX_BLOCK_BATCH`                 | 128     | [1, 4096]     |                                           |
| `HYBRID_V2_RPC_URL`                         | -       | http(s) only  | Path/query redacted in logs               |
| `HYBRID_V2_REORG_MAX_DEPTH`                 | 64      | [1, 512]      |                                           |
| `HYBRID_V2_REORG_MAX_REPLACEMENT_BLOCKS`    | 256     | [1, 4096]     |                                           |
| `HYBRID_V2_REORG_ALLOW_FINALIZED_CROSS`     | false   | bool          | Dev/test only                             |

Every field is validated at startup. A misconfigured value aborts the
worker rather than running with silent defaults.

## 2. Safety invariants (frozen)

- Chain is canonical. Postgres is a rebuildable, non-canonical projection.
- Production Hybrid V2 HTTP reads come from Postgres only.
- Runtime never publishes state before Postgres commit.
- Chain source is strictly read-only.
- Reorg recovery uses persisted canonical replay (no inverse arithmetic).
- Orphaned economic state is never publicly visible.
- **Rebuild** never publishes partial state; readiness is hard-503
  throughout every rebuild phase.
- **Reconciliation drift** never auto-repairs projections. Drift
  persists as `NOT_READY(ReconciliationDrift)`; operator must act.
- Ready requires all four subsystems (persistence + reorg + rebuild +
  reconciliation) to have converged.
- Operator recovery actions are deployment-scoped and non-public.

## 3. Startup checklist

1. Apply migrations 0044..0048 via `sqlx::migrate::Migrator::run` (or
   `sqlx migrate run` outside the process). The Rust runtime does
   NOT auto-migrate — that is a startup-script responsibility.
2. Register the deployment via `upsert_deployment` with the validated
   `ManifestParams`.
3. Bootstrap the runtime via `IndexerRuntime::bootstrap_from_persistence`.
   This is safe to invoke on every startup; it either resumes the
   existing cursor or reconstructs it from the raw log journal.
4. Confirm readiness — a fresh deployment reports
   `ReadinessReason::AwaitingFirstBlock` (503) until the first block
   is persisted.

## 4. Readiness interpretation

A `503` from `/hybrid_v2/*` routes is the frozen posture — the reason
column tells you why. Non-fatal reasons (`Behind`) do NOT trigger 503;
every other reason does.

| Reason                            | Meaning                                                                                          | Operator action                                     |
|-----------------------------------|--------------------------------------------------------------------------------------------------|-----------------------------------------------------|
| `AwaitingFirstBlock`              | New deployment, nothing indexed yet                                                              | Wait                                                |
| `WrongChain`                      | ChainSource chain_id ≠ manifest.chain_id                                                        | Fix RPC endpoint or manifest                        |
| `ManifestMismatch`                | Live manifest hash ≠ stored                                                                      | Investigate; do NOT force-overwrite                 |
| `UnknownCanonicalEvent`           | Topic0 not in the catalogue                                                                      | Investigate; extend decoder if the event is valid   |
| `DecodeFailure`                   | ABI decode failed                                                                                | Investigate — likely a new contract version         |
| `ProjectionFailure`               | Reducer invariant violated                                                                       | Emergency: freeze deployment, investigate log       |
| `CursorHashMismatch`              | Persisted cursor hash ≠ live cursor                                                              | Restart runtime; falls into reorg recovery          |
| `ExcessiveReorg`                  | Reorg depth ≥ `REORG_MAX_DEPTH`                                                                  | Manual intervention required                        |
| `ReorgDetected/Searching/Replaying/ManualInterventionRequired` | Reorg recovery in flight or escalated | Watch, or intervene at ManualInterventionRequired |
| `RebuildRequested`                | Operator enqueued a rebuild but state machine hasn't advanced                                    | Wait                                                |
| `RebuildInProgress`               | Rebuild is running                                                                               | Wait                                                |
| `RebuildFailed`                   | Rebuild produced drift or failed                                                                 | Manual intervention required                        |
| `ReconciliationInProgress`        | Scheduler is actively running                                                                    | Wait                                                |
| `ReconciliationDrift`             | Drift persisted; projection NOT auto-repaired                                                    | Investigate + operator-driven rebuild               |
| `MigrationSchemaMismatch`         | Runtime detected schema drift                                                                    | Re-apply migrations                                 |
| `Bootstrapping`                   | `bootstrap_from_persistence` in flight                                                           | Wait                                                |
| `Stopping`                        | Runtime received shutdown                                                                        | Terminal                                            |

## 5. Reorg recovery phases

Persisted in `hybrid_v2_reorg_recovery` (`phase`). Progression:
`None → Detected → SearchingAncestor → AncestorFound →
FetchingReplacement → InvalidatingOrphanBranch → ReplayingReplacement →
CommittingReplacement → Recovered`. Sinks: `Failed`,
`ManualInterventionRequired`.

Manual intervention required if:
- `orphan_depth > REORG_MAX_DEPTH`
- Ancestor ≤ finalized head and
  `REORG_ALLOW_FINALIZED_CROSS=false` (default)
- `retry_count > REORG_RETRY_MAX`

## 6. Rebuild — 13-phase machine

Persisted in `hybrid_v2_rebuild_operations` (`phase`). Progression:
`None → Requested → LockAcquired → ValidatingSource → Preparing →
Replaying → Correlating → Verifying → Reconciling → Committing →
Complete`. Sinks: `Failed`, `ManualInterventionRequired`.

Modes:
- **`JournalReplay`** — replays the canonical journal into a fresh
  `ProjectionState`. On match with live projection: `Complete
  (NothingToDo)`. On drift: `ManualInterventionRequired`, readiness =
  `RebuildFailed`.
- **`FreshChain`** — walks the RPC from `deployment_start_block` to
  a bounded head, ingesting through the normal worker path. Persists
  each block atomically.

### Operator-triggered rebuild

At this milestone the trigger surface is the `RebuildOperationsService`
Rust API. HTTP is deferred. Invocation example (from an admin script):

```rust
use deopt_v2_backend::hybrid_v2::rebuild_operations::{
    RebuildOperationsService, RebuildConfig,
};
let svc = RebuildOperationsService::new(deployment_id, RebuildConfig::default());
let outcome = svc.rebuild_from_journal(&store, &manifest).await?;
```

The service:
1. Refuses Base mainnet.
2. Refuses if the operation lock is held.
3. Persists every phase transition.
4. Never publishes any projection mutation on `JournalReplay`.

## 7. Reconciliation

The `ReconciliationScheduler` runs one pass at a time:
1. Acquires the operation lock.
2. Calls `Reconciler::reconcile` against the chain-view provider.
3. Persists a row into `hybrid_v2_reconciliation_results`.
4. On drifting classification: writes readiness
   `NOT_READY(ReconciliationDrift)`.
5. Releases the lock.

Classifications: `CONVERGED`, `INDEXER_BEHIND`, `NON_FINAL_DIFFERENCE`,
`MANIFEST_MISMATCH`, `PROJECTION_DRIFT`, `PROVIDER_UNAVAILABLE`,
`UNSUPPORTED_VIEW`, `MALFORMED_CHAIN_RESPONSE`.

`PROVIDER_UNAVAILABLE` is transient — it does NOT count as drift
against the projection.

## 8. Lock inspection

```sql
SELECT deployment_id, operation, holder_epoch, acquired_at_ms
  FROM hybrid_v2_operation_locks;

-- Rebuild status
SELECT deployment_id, rebuild_epoch, mode, phase, retry_count,
       last_failure_detail, updated_at_ms
  FROM hybrid_v2_rebuild_operations
 WHERE deployment_id = $1
 ORDER BY rebuild_epoch DESC
 LIMIT 5;

-- Latest reconciliation result
SELECT reconciliation_id, ran_at_ms, classification,
       divergent_categories, provider_availability, failure_detail
  FROM hybrid_v2_reconciliation_results
 WHERE deployment_id = $1
 ORDER BY ran_at_ms DESC
 LIMIT 5;
```

## 9. PG outage handling

- The runtime fails closed. Cursor never advances; readiness reports
  the persistence error.
- On PG recovery, restart the worker — bootstrap will re-hydrate
  from the last committed cursor.

## 10. Restart

Restart is always safe. The runtime:
1. Reads the persisted cursor + readiness.
2. If a reorg / rebuild / reconciliation phase is active,
   `bootstrap_from_persistence` resumes.
3. If none is active, the worker resumes from the last committed
   cursor.

## 11. Migration execution

```
sqlx migrate run --source ./migrations --database-url $DATABASE_URL
```

`0048_hybrid_v2_operation_closure.sql` is additive-only. It creates
three tables and does NOT alter or drop anything.

## 12. Smoke test

Load the operator-scoped smoke test:
```
DEOPT_REQUIRE_PG_INTEGRATION=1 \
HYBRID_V2_PG_TEST_DATABASE_URL=postgres://... \
cargo test --test hybrid_v2_rebuild_operations_pg_integration
```

## 13. Secret handling

`HYBRID_V2_RPC_URL` may embed a provider API key on the path. The
Rust `Debug` impl for `HybridV2Config` redacts this to the URL host
only. Never log `HYBRID_V2_RPC_URL` verbatim.

## 14. Absence of signing / broadcast

Hybrid V2 has no signing capability, no keys, no transaction
broadcast. If a procedure asks to "sign" or "send", it does not
apply here.
