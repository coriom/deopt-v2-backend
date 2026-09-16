# PERPS_BASE_SEPOLIA_BACKEND_BROADCAST_RUNTIME_WIRING_V1

Runtime wiring for the Perps closed-test broadcast worker: extends
receipt handling with semantic PME event verification, adds a
background reconciler loop, and defines the fail-closed startup
ordering. Builds on
[PERPS_BASE_SEPOLIA_BACKEND_BROADCAST_DURABILITY_PG_V1](./PERPS_BASE_SEPOLIA_BACKEND_BROADCAST_DURABILITY_PG_V1.md).

## Startup ordering (fail-closed)

The deployment MUST perform the following steps IN ORDER before the
backend accepts any closed-test execution that could reach broadcast:

```
1. validate ExecutionConfig::validate_startup()            (existing)
   ├─ ensures real_broadcast_enabled + signer backend + gas caps + RPC
   └─ refuses mainnet local-signer

2. validate_signer_triad(                                   (new)
     EXECUTOR_FROM_ADDRESS,
     HV2_SIGNER_EXPECTED_ADDRESS,
     HV2_EXECUTOR_ADDRESS)
   └─ any mismatch → BackendError::Config, startup aborts

3. broadcast_reconciler::startup_preflight(policy).await   (new)
   ├─ policy.preflight_static()   — signer bind, dry_run, rpc_url
   ├─ rpc.chain_id() == EXECUTOR_CHAIN_ID
   ├─ PME.isExecutor(from) == true
   ├─ PME.paused() == false
   └─ any failure → startup aborts

4. broadcast_reconciler::initial_reconciliation(            (new)
     policy, repo, batch_size).await
   └─ resolves durable Prepared/Submitted rows from prior process
      (Confirmed / Failed / rebroadcast-attempted). No new broadcasts
      may proceed before this returns Ok.

5. broadcast_reconciler::spawn_broadcast_reconciler(        (new)
     policy, repo, ReconcilerConfig::from_env())
   └─ background tokio task; runs indefinitely until CancelToken fires.
```

Only after step 5 returns is the executor allowed to accept new
closed-test intents.

## Runtime reconciler lifecycle

`broadcast_reconciler::spawn_broadcast_reconciler` returns
`(JoinHandle, BroadcastReconcilerCancel)`. Every tick:

1. Check the cancel token; if set, exit gracefully.
2. Call `policy.reconcile_unfinalized(repo, batch_size)`.
3. Log the `ReconcileSummary` and emit an observability event
   (`crate::monitoring::observe_broadcast_reconcile_tick`).
4. Sleep `interval_ms`.

Errors on a single tick are logged and the loop continues (per-row
failure never kills the whole task).

**Env vars** (all optional; safe defaults):

| Var | Default | Range | Meaning |
|---|---|---|---|
| `PERPS_BROADCAST_RECONCILE_INTERVAL_MS` | 10 000 | 1 000 – 600 000 | Interval between passes |
| `PERPS_BROADCAST_RECONCILE_BATCH_SIZE` | 25 | 1 – 500 | Max intents per pass |

Values outside the range fall back to defaults (see
`ReconcilerConfig::from_env`).

## Semantic PME event verification

`BroadcastPolicy::finalize_receipt` now enforces (when
`policy.verify_pme_event == true`, which is expected in production):

1. `receipt.status == 1` (existing)
2. `receipt.tx_hash == derived envelope hash` (existing)
3. **NEW**: at least one log with:
   - emitter address == `config.perp_matching_engine_address` AND
   - `topic0` ∈ `{PME_TRADE_EXECUTED_TOPIC0, PME_TRADE_EXECUTED_FROM_INTENTS_TOPIC0}`

If (3) is missing on a status=1 receipt, the intent is marked
`Failed` with `failure_reason="semantic_event_verification: …"`
rather than `Confirmed`. This closes the "receipt.status = 1 but
wrong contract executed" attack surface.

`ConfirmationReceipt` now carries `logs: Vec<ReceiptLog>` populated
by `eth_getTransactionReceipt`.

## Status monotonicity

`mark_intent_submitted` / `mark_intent_confirmed` / `mark_intent_failed`
enforce `WHERE status IN ('prepared', 'submitted')` — Confirmed and
Failed rows CANNOT regress. Legal transitions:

```
Prepared  → Submitted  → Confirmed
    │           │
    │           └────→ Failed
    └────────────────→ Failed (deterministic-reject send path)
```

Illegal transitions are silently ignored at the DB layer (the
`UPDATE` returns 0 rows affected), and the pre-existing row remains
authoritative.

## Observability

Structured tracing events (target labels operators can scrape):

- `deopt_perps_broadcast_reconcile` — per-tick `ReconcileSummary`
  (inspected / confirmed / failed / still_pending /
  rebroadcast_attempted).
- `deopt_perps_broadcast_outcome` — per-intent lifecycle transition
  (status / block / error class).

No secret material appears in labels or logs. Prometheus counter
wiring can bind to these targets via the tracing → OTEL bridge.

## Real Postgres integration tests

`tests/perps_broadcast_durability_pg_integration.rs` — 10 tests
executed against a real disposable Postgres:

1. `migration_and_insert_read`
2. `duplicate_tx_hash_rejected`
3. `duplicate_executor_nonce_rejected`
4. `lifecycle_confirmed`
5. `lifecycle_failed`
6. `bump_send_attempt_monotonic`
7. `confirmed_cannot_regress` — monotonicity
8. `failed_cannot_regress` — monotonicity
9. `list_unfinalized_filter` — reconciler query correctness
10. `concurrent_nonce_race_exactly_one_wins` — 5 workers racing on the
    same executor nonce; DB serializes + rejects 4 of 5.

Env: `PERPS_CLOSED_TEST_E2E_PG_URL=postgres://deopt:deopt@127.0.0.1:15432/deopt_v2_test`.

## Access-gate posture (unchanged)

- **Public Perps remains OFF.** Fail-closed at `src/api/routes.rs:3076-3078`.
- **Funding remains OFF.**
- **Collateral activation unchanged.**
- **Base mainnet untouched.** Startup preflight refuses non-84532.
- Frontend fail-closed gate intact.

## Shutdown / restart behavior

- `BroadcastReconcilerCancel::cancel()` signals the reconciler loop
  to exit at the next tick boundary.
- On restart, the startup ordering above ensures the reconciler
  resolves any in-flight `Prepared` / `Submitted` rows BEFORE the
  broadcast worker accepts new intents.
- Byte-identical rebroadcast means the same `raw_tx_hex` is sent
  again — same `tx_hash`, same nonce, no risk of a second economic
  execution.

## Follow-on work

1. **`PERPS_BASE_SEPOLIA_CLOSED_TEST_TRADER_FIXTURES_V1`** — fund
   closed-test traders with mUSDC collateral. See below for READ-ONLY
   scope map.
2. **`PERPS_BASE_SEPOLIA_CLOSED_TEST_SMOKE_ORDER_V1`** — pre-signed
   matched-pair `PerpOrderIntent` end-to-end broadcast.
3. **Prometheus counter wiring** — bind the tracing targets to
   IntCounter/Histogram metrics via the existing OTEL adapter (if
   any) or add a `prometheus_hyper` scrape endpoint.
4. **Main.rs startup integration** — the reconciler/policy is
   library-ready; the actual `main.rs` gated block that constructs
   the policy from env + spawns the reconciler is deployment-side
   wiring (kept out of this milestone because production
   `main.rs` is already complex and full integration requires the
   trader-fixtures milestone to be exercisable).
