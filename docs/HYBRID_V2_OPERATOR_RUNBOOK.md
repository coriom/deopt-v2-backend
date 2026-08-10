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

### 7.1 Production `RpcChainViewProvider` + admin trigger + periodic worker

Introduced by
`BACKEND-HYBRID-V2-CHAIN-VIEW-PROVIDER-AND-RECONCILIATION-TASK-V1`.

- Enable with `HYBRID_V2_RECONCILIATION_ENABLED=1`. The provider is
  wired on `AppState`; the admin `/reconcile` route stops returning
  `RECONCILIATION_PROVIDER_UNAVAILABLE`.
- Set `HYBRID_V2_RECONCILIATION_PERIODIC_MS` to a non-zero value to
  spawn a background worker that ticks at that cadence. `0` disables
  the worker (admin-triggered runs still work).
- `HYBRID_V2_RECONCILIATION_MAX_ITEMS_PER_RUN` (default `4096`) caps
  the number of subaccounts sampled per run.
- The provider uses block-bound `eth_call` against
  `SubaccountRegistry.ownerOf`, `CollateralVault.balanceWithYield`,
  and `RecoveryFinalizer.getRecoveryState`. Each call is validated
  against a compile-time per-module selector allowlist inside
  `RpcHybridV2ChainSource::eth_call`.
- Reservations / positions / order lifecycle / executions are
  `UNSUPPORTED_VIEW` in this milestone — the reconciler does not
  compare those categories against chain state.

Admin trigger:

```
curl -X POST -H "x-admin-token: $ADMIN_TOKEN" \
  https://backend.example/admin/hybrid_v2/deployments/42/reconcile
```

Response body:

```json
{"deployment_id": 42, "operation_id": 17, "classification": "CONVERGED",
 "status": "COMPLETED"}
```

Non-success outcomes:

- 409 `RECONCILIATION_SKIPPED` — operation lock contention (a rebuild
  or reorg is in flight).
- 503 `RECONCILIATION_PROVIDER_UNAVAILABLE` — the provider is not
  wired (missing env var, missing manifest, or invalid module address).
- 500 `RECONCILIATION_FAILED` — persistence error; the row was NOT
  written.

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


---

## Operator recovery controls — 2026-08-06

`BACKEND-HYBRID-V2-PROJECTION-PERSISTENCE-OPERATIONAL-CLOSURE-V1`
mounts three admin-gated HTTP routes for operator recovery of the
Hybrid V2 projection layer. Every route uses the same admin-token
gate as `admin/options/events/tick` — configure via `ADMIN_ENABLED=true`
and `ADMIN_TOKEN=<opaque>`.

### POST /admin/hybrid_v2/deployments/:deployment_id/rebuild

Triggers a persisted Mode-1 (JournalReplay) rebuild for the deployment.
The request body accepts:

```json
{
  "mode": "JOURNAL_REPLAY",
  "auto_rematerialize": false
}
```

- `mode` defaults to `"JOURNAL_REPLAY"`. `"FRESH_CHAIN"` is refused
  with 501 `FRESH_CHAIN_NOT_ROUTED` — it requires an operator-supplied
  ingestion closure and is only exposed via direct service calls.
- `auto_rematerialize` (default `false`) — when `true`, drift detected
  by the journal replay triggers a single-tx TRUNCATE + REINSERT of
  the re-materializable projection tables. When `false`, drift escalates
  to `MANUAL_INTERVENTION_REQUIRED` and the operator must decide how
  to proceed. **This flag is strongly recommended off during production
  first launch.**

Response codes:

- `200 OK` with `{ "outcome": "NOTHING_TO_DO" | "REBUILT", ... }`
  on success.
- `409 CONFLICT` with outcome `"MANUAL_INTERVENTION_REQUIRED"` on
  drift when `auto_rematerialize=false`, or with
  `"OPERATION_LOCK_CONTENTION"` when another operation
  (rebuild / reorg / reconciliation) is running.
- `403 FORBIDDEN` on chain-id violation (Base mainnet) or auth failure.
- `404 NOT_FOUND` on unknown `deployment_id`.
- `503 HYBRID_V2_NOT_CONFIGURED` when no projection store is attached.

### POST /admin/hybrid_v2/deployments/:deployment_id/reconcile

Reserved for a future `ChainViewProvider`-backed operator reconciliation.
Currently returns `501 RECONCILIATION_PROVIDER_UNAVAILABLE`. The
scheduler + persistence surface is fully implemented — only the
production `RpcChainViewProvider` is missing.

### GET /admin/hybrid_v2/deployments/:deployment_id/operations/latest

Returns the most recent rebuild-op row, reconciliation-result row,
and reorg-recovery row for the deployment. Use this to poll status
after triggering a rebuild.

### Operational sequence — journal-replay rebuild

1. Verify readiness: `GET /admin/hybrid_v2/deployments/:id/operations/latest`
   — confirm no active rebuild.
2. Trigger: `POST /admin/hybrid_v2/deployments/:id/rebuild`
   with `{"mode":"JOURNAL_REPLAY"}`. Response body carries the
   `rebuild_epoch` and outcome.
3. If the response is `NOTHING_TO_DO`, the projections are already
   consistent with the canonical journal — no further action.
4. If the response is `409 MANUAL_INTERVENTION_REQUIRED`, inspect the
   `detail` field, review the affected projection tables, and — after
   operator sign-off — re-trigger with `{"auto_rematerialize": true}`.
5. During any active rebuild, the runtime reports readiness
   `RebuildInProgress` (hard 503) and the worker pauses ticks.

### Unified operation lock

Reorg + rebuild + reconciliation now contend for the same row in
`hybrid_v2_operation_locks`. Inspect via:

```sql
SELECT deployment_id, operation, holder_epoch, acquired_at_ms
FROM hybrid_v2_operation_locks;
```

The legacy `hybrid_v2_reorg_locks` table remains for historical
purposes but is never written to. It can be dropped in a follow-up
migration when convenient.

### Bootstrap-time rebuild block

If a rebuild-op row exists with a non-terminal / Failed / manual-
intervention-required phase when the runtime starts,
`IndexerRuntime::bootstrap_from_persistence` returns
`BootstrapResult::RebuildBlocked` and the worker refuses to tick
until an operator clears the row. Log lines include `rebuild_epoch`
and `phase` for tracing.


---

## 15. Reconciliation-scope Policy A — 2026-08-06

Introduced by
`BACKEND-HYBRID-V2-FINAL-PERSISTENCE-MATRIX-AND-PARENT-CLOSURE-V1`.

### Supported reconciliation categories

The `Reconciler` directly compares projection against on-chain view
for these four categories only:

1. **Manifest identity** — expected hash vs
   `ChainSnapshot.manifest_hash`.
2. **Subaccount ownership** — `SubaccountRegistry.ownerOf(subKey)`.
3. **Collateral vault balance** — `CollateralVault.balanceWithYield(
   subKey, token)`.
4. **Recovery state** — `RecoveryFinalizer.getRecoveryState(subKey)`.

Every other category is `UNSUPPORTED_VIEW` at this milestone:

- Reservations per (subKey, token, engine)
- Positions per (subKey, series)
- Order lifecycle
- Matched executions
- Active-series enumeration
- Escape / withdrawal counts

### Frozen invariants

- **`UNSUPPORTED_RECONCILIATION_VIEW_IS_NEVER_REPORTED_AS_CONVERGED`.**
  The reconciler returns `ReconciliationResult::Unsupported { detail }`
  for unsupported categories; the scheduler never coerces that into
  `Converged` when persisting a row.
- **`READY_NEVER_IMPLIES_UNSUPPORTED_CATEGORIES_WERE_RECONCILED`.**
  A READY response does NOT claim the unsupported categories were
  reconciled against chain state. It claims:
  - the supported categories are convergent under the reconciler +
    provider allowlist, AND
  - the canonical event journal has been played through the
    deterministic reducer for every other category.

### What READY means

READY implies: the four supported categories were compared against
chain state at the block referenced in the latest reconciliation
row, and the classification was `CONVERGED`, `INDEXER_BEHIND`, or
`PROVIDER_UNAVAILABLE` (transient) — never `PROJECTION_DRIFT`.

READY does **NOT** imply: the unsupported categories were directly
compared against chain state. Their correctness is derived from
the canonical journal + reducer, verified separately by the
runtime persistence property suite.

### Serving policy A

Operators MUST NOT infer that a hybrid v2 read for a position /
order / execution is "chain-verified" — it is journal-derived. If
an on-chain view for one of the deferred categories becomes
required for a downstream product (e.g. a settlement UI), extend
the RPC allowlist + provider first, add a reconciler comparison,
and only then remove the category from the unsupported list. The
provider allowlist in `RpcHybridV2ChainSource::eth_call` is the
authoritative source; every entry must be a per-module compile-time
selector.

Cross-reference:
- Closure milestone: `BACKEND_HYBRID_V2_FINAL_PERSISTENCE_MATRIX_AND_PARENT_CLOSURE_V1.md`
- Global matrix: `BACKEND_HYBRID_V2_GLOBAL_CLOSURE_MATRIX.md`

## 16. Signer & Execution (Pre-Broadcast) — 2026-08-08

Milestone: `BACKEND-HYBRID-V2-SIGNER-AND-EXECUTION-V1` — see
`BACKEND_HYBRID_V2_SIGNER_AND_EXECUTION_V1.md` and
`BACKEND_HYBRID_V2_SIGNER_AND_EXECUTION_V1_SECURITY_REVIEW.md`.

### 16.1 What the pre-broadcast layer does

The pre-broadcast pipeline is invoked when an operator (or, in a future
milestone, a live matching worker) has a pair of signed order envelopes
that a `OptionMatchingEngineV2::executeMatch` transaction should be
built for. It:

1. Derives a `canonical_execution_id` from `(deployment_id, chain_id,
   buyer_order_hash, seller_order_hash, fill_quantity_1e8)`. Deterministic;
   duplicate triggers converge on the same row.
2. Runs `PreflightChecker` (readiness, reorg/rebuild/reconciliation
   drift, order cancelled/expired, on-chain settled correlation).
3. Builds the deterministic ABI plan (target/selector from the
   manifest allowlist; calldata from the plan builder). Persists
   `plan_hash` and `calldata_hash` (immutable thereafter — SQL trigger).
4. Reserves a nonce for the signer (atomic UNIQUE INSERT).
5. Runs `eth_call` simulation bound to head block number + hash.
6. Computes gas + fees via the bounded gas policy.
7. Runs the firewall's independent revalidation of every field.
8. Invokes the signer, verifies the returned signature locally, and
   persists `(r, s, v, recovered_signer)`.
9. Lands the row at the terminal `BROADCAST_DISABLED` phase.

Broadcast is **disabled by construction** — no `send_*` method exists
anywhere in the execution module. The next milestone
(`BACKEND-HYBRID-V2-BROADCAST-AND-CONFIRMATION-V1`) will introduce it.

### 16.2 Admin routes and expected inputs

All routes are behind `x-admin-token` and are refused on Base mainnet
(8453) at handler entry.

Route: `POST /admin/hybrid_v2/deployments/:deployment_id/executions/:canonical_execution_id/prepare`

Body (JSON):

```json
{
  "buyer_envelope": { "owner": "0x...", "subaccount_id": 1, "subkey": "0x...", ... },
  "buyer_order":    { "series_id": "42", "side": 0, "quantity_1e8": "100000000", ... },
  "seller_envelope": { ... },
  "seller_order":   { ... },
  "fill_quantity_1e8": "100000000",
  "buyer_active_series": ["42"],
  "seller_active_series": ["42"],
  "buyer_order_hash": "0x...",
  "seller_order_hash": "0x...",
  "series_id": "42",
  "premium_amount": "50000000",
  "fee_schedule_epoch": null
}
```

Fields NOT in the body (and therefore NEVER caller-controllable):
`target`, `selector`, `calldata`, `value_wei`, `nonce`, `gas_limit`,
`max_fee_per_gas`, `max_priority_fee_per_gas`, `chain_id`.

This milestone returns `503 EXECUTION_ORCHESTRATOR_NOT_WIRED` — live
wiring lands with the production signer milestone.

Read routes:

- `GET /admin/hybrid_v2/deployments/:deployment_id/executions/:canonical_execution_id`
  — sanitized row (no `(r, s, v)`, no signer secret; `recovered_signer`
  is safe because it is a public address).
- `GET /admin/hybrid_v2/deployments/:deployment_id/executions?limit=N&offset=M`
  — bounded listing (`limit ≤ 1000`, page size is capped).

Control routes:

- `POST /admin/…/cancel` — refused past `AWAITING_SIGNATURE`.
- `POST /admin/…/retry` — returns 409 `RETRY_MUST_ISSUE_NEW_CANONICAL_ID`;
  terminal FAILED rows do not resurrect — the operator re-issues
  `prepare` with the original intent, which derives the same canonical
  id and converges on the same row.

### 16.3 Failure classes and remediation

| failure_class | Meaning | Operator action |
|---|---|---|
| `PREFLIGHT_REJECTED` | readiness / drift / cancelled / expired / etc. | Read `failure_detail`; wait for readiness or fix the order state |
| `PLAN_BUILD_FAILED` | manifest mismatch, bad address, wrong chain | Inspect manifest + intent — usually a config regression |
| `NONCE_RESERVATION_FAILED` | RPC + persistence disagreement | Inspect RPC health; usually retryable |
| `SIMULATION_FAILED_DETERMINISTIC` | on-chain revert | Read the decoded selector — fix state or accept as invalid |
| `SIMULATION_TRANSPORT_FAILED` | RPC transport blip | Retryable via a fresh `prepare` call |
| `GAS_POLICY_REJECTED` | estimate/fee/total-cost out of bounds | Wait for gas to normalize or widen the policy |
| `FIREWALL_REJECTED` | row tampered / plan mutated | Escalate — indicates DB tamper or bug |
| `SIGNER_UNAVAILABLE` | signer not wired | Wait for production signer milestone |
| `SIGNATURE_VERIFICATION_FAILED` | signer returned bad `(r,s,v)` | Escalate — signer compromise or bug |
| `LOCK_CONTENTION` | another op holds the deployment lock | Retry after the concurrent op completes |
| `STORE_FAILURE` | Postgres error | Check DB health |

### 16.4 Broadcast is disabled — no runbook entry for "trigger broadcast"

There is intentionally NO operator control for broadcasting a signed
row. The `BROADCAST_DISABLED` phase is a first-class terminal. The
next milestone will add the broadcast surface with its own runbook
section; until then, a signed row is an **audit artifact**, not a
transaction ready to submit.

### 16.5 Reconciliation vs execution

Reconciliation drift (`ProjectionDrift`, `ManifestMismatch`,
`MalformedChainResponse`) **blocks preflight** — no execution row can
advance until drift is resolved by an operator (rebuild, manifest fix,
RPC provider swap). `UnsupportedView` alone does NOT block if every
other axis reports Ready.

Cross-reference:

- Closure doc: `BACKEND_HYBRID_V2_SIGNER_AND_EXECUTION_V1.md`
- Security review: `BACKEND_HYBRID_V2_SIGNER_AND_EXECUTION_V1_SECURITY_REVIEW.md`

## 17. External Signer Integration + Live Orchestrator — 2026-08-09

Milestone: `BACKEND-HYBRID-V2-EXTERNAL-SIGNER-INTEGRATION-AND-LIVE-ORCHESTRATOR-V1`
— see `BACKEND_HYBRID_V2_EXTERNAL_SIGNER_INTEGRATION_AND_LIVE_ORCHESTRATOR_V1.md`
and the paired security review.

### 17.1 What the milestone added

- A live `HybridV2KmsSignerBridge` sitting between the HV2
  `ExecutionSigner` trait and the perps `RemoteSigner` stack — the
  same stack that the perps signer microservice already uses in
  staging. The bridge holds no private key; it forwards signing
  requests to the operator-owned signer microservice (Pattern C).
- A live `ExecutionOrchestrator` wired into `AppState`. When
  `HV2_EXECUTION_ENABLED=1` and the signer config is complete, the
  admin `prepare_execution` route drives an end-to-end flow through
  preflight, plan build, nonce reservation, simulation, gas policy,
  the signer firewall, the bridge, and local signature verification,
  landing at the terminal `BROADCAST_DISABLED` phase.
- A persisted `signer_request_idempotency_key` column
  (migration 0050) — 16 bytes derived from
  `keccak256(HV2_SIGNER_IDEMPOTENCY_V1 || expected_signer_address ||
  canonical_execution_id || plan_hash || signing_payload_hash)[..16]`
  — immutable via SQL trigger so a retried request converges on the
  same vendor-side signature.

### 17.2 Startup configuration

Set these env vars (see `HYBRID_V2_ADMIN_API.md § Configuration` for
bounds):

```
HV2_EXECUTION_ENABLED=1
HV2_EXECUTOR_ADDRESS=0x…
HV2_SIGNER_BACKEND=production
HV2_SIGNER_PROVIDER=kms_aws
HV2_SIGNER_ENDPOINT=https://signer.internal.example.com
HV2_SIGNER_EXPECTED_ADDRESS=0x…
HV2_SIGNER_KMS_KEY_ID=arn:aws:kms:…
HV2_SIGNER_REQUEST_TIMEOUT_MS=2500
HV2_SIGNER_MAX_RETRIES=1
HV2_SIGNER_AUTH_REFERENCE=arn:aws:iam::…:role/deopt-signer
HV2_EXECUTION_RPC_URL=https://…
HV2_EXECUTION_RPC_TIMEOUT_MS=10000
HV2_SIMULATION_MAX_AGE_MS=60000
```

At startup, `wire_hybrid_v2_execution_orchestrator` runs
`HybridV2ExecutionConfig::validate_startup(chain_id)`. Any validation
failure logs at WARN and leaves `AppState.hybrid_v2_execution_
orchestrator = None`; the admin route then returns
`503 EXECUTION_ORCHESTRATOR_NOT_WIRED` with the availability reason
in the body. This is the honest fail-closed posture.

### 17.3 Common startup-failure availability reasons

- `EXECUTION_DISABLED` — `HV2_EXECUTION_ENABLED` is false.
- `IncompleteProductionSignerConfig: HV2_SIGNER_EXPECTED_ADDRESS
  required` (or endpoint/provider) — a required env var is unset.
- `aws-kms-transport feature not enabled at build time` — the build
  was compiled without `--features aws-kms-transport`; rebuild with
  the feature to enable the KMS transport.
- `HV2_SIGNER_ENDPOINT must start with https://…` — the endpoint is
  a public non-https host (localhost http is allowed only for dev).
- `HV2_SIGNER_REQUEST_TIMEOUT_MS must be within [100, 30000]` — see
  the bounds table in `HYBRID_V2_ADMIN_API.md`.

### 17.4 Signer failure classes and remediation

| `failure_class` | Root cause | Operator action |
|---|---|---|
| `SIGNER_UNAVAILABLE (vendor timeout)` | Signer microservice slow / DNS blip | Wait a beat; the bridge already retried once. Check signer's own logs. |
| `SIGNER_UNAVAILABLE (vendor 5xx)` | Signer microservice unhealthy | Check the signer's health endpoint + Prometheus signals. |
| `SIGNER_UNAVAILABLE (vendor auth failed)` | IAM role expired or KMS key policy revoked | Rotate `HV2_SIGNER_AUTH_REFERENCE` / verify KMS key policy. NOT retried automatically. |
| `SIGNER_UNAVAILABLE (vendor policy fingerprint)` | Signer's own policy rejected the request | Inspect the signer's audit log — usually indicates a plan mutation between backend and signer, or a stale operator allowlist. |
| `SIGNATURE_VERIFICATION_FAILED` | Signer returned a signature that does not recover to `HV2_SIGNER_EXPECTED_ADDRESS` OR does not cover the orchestrator-derived signing payload | **Escalate** — this is either a signer compromise or a code bug. Do NOT auto-retry. |

The bridge is deterministic on retryable vs non-retryable
classification: transport / KmsTimeout / KmsUnavailable / RateLimit
retry up to `HV2_SIGNER_MAX_RETRIES + 1` times. Everything else
fails on the first attempt.

### 17.5 Idempotency + restart safety

Every persisted row that reaches `SignatureVerified` carries the
`signer_request_idempotency_key` column. A retried `prepare` request
that lands on the same canonical id short-circuits — the row is
already terminal. A restart of the backend process picks up the row
via `resume(canonical_execution_id)` and yields the same terminal
outcome without re-signing.

The operator visibility for this is the
`GET /admin/hybrid_v2/deployments/:id/executions/:canonical_execution_id`
route, which returns the sanitized row including
`signer_request_idempotency_key` for correlation with signer-side
audit logs.

### 17.6 Broadcast is STILL disabled

There is intentionally no operator control to submit a signed row
on-chain. The `BROADCAST_DISABLED` phase remains a first-class
terminal. The next milestone
(`BACKEND-HYBRID-V2-BROADCAST-AND-CONFIRMATION-V1`) adds the
broadcast surface.

### 17.7 Cross-references

- External signer closure:
  `BACKEND_HYBRID_V2_EXTERNAL_SIGNER_INTEGRATION_AND_LIVE_ORCHESTRATOR_V1.md`.
- External signer security review:
  `BACKEND_HYBRID_V2_EXTERNAL_SIGNER_INTEGRATION_AND_LIVE_ORCHESTRATOR_V1_SECURITY_REVIEW.md`.
- Admin API reference: `HYBRID_V2_ADMIN_API.md`.
- Pattern C signer design: `MAINNET_BE_SIGNER_SERVICE_DESIGN.md`.

