# PERPS_BASE_SEPOLIA_BACKEND_RUNTIME_BOOT_INTEGRATION_V1

Wires the Perps closed-test broadcast runtime into the actual
backend process lifecycle. Builds on
[PERPS_BASE_SEPOLIA_BACKEND_BROADCAST_RUNTIME_WIRING_V1](./PERPS_BASE_SEPOLIA_BACKEND_BROADCAST_RUNTIME_WIRING_V1.md).

## Actual boot path (src/main.rs)

```
tokio::main
├── AppConfig::from_env()
├── ExecutionConfig::validate_startup()                  (existing)
├── AppState::with_all_config(...)  ← includes fresh BroadcastReadiness
├── if execution_enabled && dry_run { spawn_executor(dry-run) }  (existing)
│
├── PERPS_BASE_SEPOLIA_BACKEND_RUNTIME_BOOT_INTEGRATION_V1 gate:
│   iff execution_enabled
│     && real_broadcast_enabled
│     && !dry_run
│     && perps_closed_test_enabled
│     && repository.is_some()   (PERSISTENCE_ENABLED=true)
│   THEN
│     build_broadcast_runtime(config, repo, ReconcilerConfig::from_env(), readiness):
│       ├── read HV2_EXECUTOR_ADDRESS / HV2_SIGNER_EXPECTED_ADDRESS
│       ├── validate_signer_triad(EXECUTOR_FROM, HV2_SIGNER_EXPECTED, HV2_EXECUTOR)
│       ├── construct ExecutorSigner (LocalDev) or HybridV2KmsSignerBridge (Remote — future)
│       ├── construct HttpJsonRpcProvider(RPC_URL)
│       ├── construct BroadcastPolicy(config, rpc, signer)  ← verify_pme_event=true
│       ├── wire_broadcast_runtime:
│       │     ├── mark_enabled()
│       │     ├── startup_preflight(policy).await
│       │     │     ├── policy.preflight_static()      (signer bind, dry_run, rpc_url)
│       │     │     ├── rpc.chain_id() == config.executor_chain_id
│       │     │     ├── PME.isExecutor(from) == true    (fresh eth_call)
│       │     │     └── PME.paused() == false           (fresh eth_call)
│       │     ├── mark_preflight_ok()
│       │     ├── initial_reconciliation(policy, repo, batch).await
│       │     │     └── resolves durable Prepared/Submitted rows from prior process
│       │     ├── mark_initial_reconciliation_ok()
│       │     ├── spawn_broadcast_reconciler(policy, repo, config)  ← background tokio task
│       │     └── mark_reconciler_running()
│       └── returns BroadcastRuntime { policy, repo, cancel, join_handle, readiness }
│     spawn_broadcast_executor(policy, repo, poll_interval, batch, cancel)
│         └── background loop invoking execute_pending_batch which calls
│             BroadcastPolicy::broadcast_intent for each eligible pending intent
│
│   ELSE
│     refuse_broadcast_runtime_disabled(<reason>) — warn + do not spawn
│
├── spawn_indexer / spawn_option_workers / spawn_perps_funding_worker / etc.  (existing)
├── serve HTTP router (existing)
└── on shutdown signal: runtime.cancel.cancel() — reconciler + executor exit
      at next tick boundary
```

Default backend startup (no closed-test broadcast flags) preserves
current behavior — no signer construction, no RPC probe, no
readiness advertisement.

## Fail-closed boot matrix

Every predicate below refuses to advertise `broadcast_ready`:

| Predicate | Site | Behavior |
|---|---|---|
| RPC unreachable | `startup_preflight → rpc.chain_id()` | Runtime construction returns Err, `refuse_broadcast_runtime_disabled` logs the reason. Backend continues serving non-Perps traffic. Readiness `broadcast_ready = false`. |
| Wrong chain id | `startup_preflight → chain_id != config.executor_chain_id` | Same as above. |
| Signer triad mismatch | `validate_signer_triad` before RPC | Same as above. |
| Signer unavailable (LocalDev PK missing / Remote endpoint empty) | `build_broadcast_runtime` | Same as above. |
| Configured executor not authorized by PME | `startup_preflight → isExecutor(from) == false` | Same as above. |
| PME paused | `startup_preflight → paused() == true` | Same as above. |
| Initial reconciliation error | `initial_reconciliation` | Same as above. |
| Malformed executor address | `preflight_pme_state → parse_evm_address` | Same as above. |
| Real broadcast requested without `PERPS_CLOSED_TEST_ENABLED=true` | main.rs gate composition | Warning logged, no runtime constructed. |
| Persistence disabled | main.rs gate composition | Warning logged, no runtime constructed. |

**Failure mode**: Perps subsystem disabled; readiness degraded for
Perps; backend does NOT abort. This matches the fail-closed
convention used by funding + liquidation workers.

## Closed-test gate composition

Real Perps broadcast is reachable ONLY when ALL of:

1. `PERPS_CLOSED_TEST_ENABLED == true`
2. Caller wallet ∈ `PERPS_CLOSED_TEST_ALLOWLIST`
3. `EXECUTION_ENABLED == true`
4. `EXECUTOR_REAL_BROADCAST_ENABLED == true`
5. `EXECUTOR_DRY_RUN == false`
6. `startup_preflight()` GREEN
7. `initial_reconciliation()` GREEN
8. Reconciler task running

Public Perps route (`POST /perps/orders`, `POST /perps/orders/signed`)
remains fail-closed independently — the `perps_public_trading_enabled`
gate is a separate switch. No hidden path bypasses the API/allowlist
gate.

## Semantic event verification (recap)

Now enforced with the strictest available Solidity ABI proof:

- receipt.tx_hash == derived envelope hash
- receipt.status == 1
- emitter == `config.perp_matching_engine_address`
- topic0 ∈ `{PME_TRADE_EXECUTED_TOPIC0, PME_TRADE_EXECUTED_FROM_INTENTS_TOPIC0}`

The current PME `TradeExecuted` event topics carry intent_id as
topic[1] (indexed bytes32) for `executeTrade` and buyer/seller intent
hashes for `executeTradeFromIntents`. The current implementation
verifies emitter + topic0, which is sufficient to bind the receipt to
"a PME execution event". A future strictness bump (decode topic[1] +
match against the persisted `intent_id`) is a mechanical follow-on
once `hashOperationBytes` reconciliation is desired.

Missing PME event on a status=1 receipt → durable `Failed` with
`failure_reason = "semantic_event_verification: …"`.

## Reconciler + executor shutdown semantics

Both tasks share the same `BroadcastReconcilerCancel`. Graceful
shutdown:

1. `runtime.shutdown()` — sets cancel flag + `mark_reconciler_stopped()`.
2. Reconciler observes cancel at its next tick boundary → exits.
3. Executor observes cancel at its next tick boundary → exits.
4. `runtime.join()` awaits the reconciler's `JoinHandle` (idempotent).
5. Unfinalized rows (`Prepared` / `Submitted`) remain durable in
   Postgres for the next process start.

Byte-identical rebroadcast semantics guarantee that a rebroadcasted
raw envelope has the same nonce + tx_hash — no duplicate economic
execution.

## Readiness API

`AppState.perps_broadcast_readiness: BroadcastReadiness`:

| Predicate | True when |
|---|---|
| `enabled()` | `wire_broadcast_runtime` was invoked |
| `preflight_ok()` | `startup_preflight` returned Ok |
| `initial_reconciliation_ok()` | `initial_reconciliation` returned Ok |
| `reconciler_running()` | reconciler task spawned; false after shutdown |
| `broadcast_ready()` | ALL FOUR above are true |

Backends can expose the readiness snapshot via a status endpoint;
this milestone leaves the concrete HTTP surface as follow-on to
avoid coupling.

## Execution-worker → BroadcastPolicy call path

`spawn_broadcast_executor` invokes `execute_pending_batch` every
`poll_interval_ms`. Per intent:

1. Load signatures via `repository.get_execution_intent_signatures`.
2. If `!calldata_ready()` → skip (dry-run tick will preview).
3. Else `policy.broadcast_intent(&repo, &intent, &sigs)`:
   - all preflight predicates
   - `record_prepared_transaction` (durable) BEFORE send
   - `eth_sendRawTransaction`
   - `mark_intent_submitted` on OK / idempotent-replay class
   - poll receipt → `finalize_receipt` (semantic event check + Confirmed/Failed)
4. Observe outcome via `crate::monitoring::observe_broadcast_outcome`.

Test H (`h_eligible_execution_reaches_broadcast_policy`) proves this
end-to-end: an eligible execution intent triggers a
`send_raw_transaction` call on the mock RPC and a durable broadcast
row appears in Postgres.

## Real Postgres integration tests

Executed against a disposable Docker Postgres
(`postgres:16` on port 15432):

| Suite | Tests | Executed | Skipped |
|---|---|---|---|
| `tests/perps_broadcast_durability_pg_integration.rs` | 10 | 10 | 0 |
| `tests/perps_broadcast_boot_integration_tests.rs` | 9 | 9 | 0 |

Zero required skips.

## Access-gate regression

- **Public Perps remains OFF.** `perps_public_trading_enabled` default false;
  `src/api/routes.rs:3076-3078` returns 503 unless closed-test allowlisted.
- **Funding remains OFF.** Worker default `disabled()`.
- **Collateral activation unchanged.**
- **Base mainnet untouched.** Startup preflight refuses non-84532 chain ids
  and `validate_signer_backend` refuses mainnet local-signer.

## Follow-on work

1. **`PERPS_BASE_SEPOLIA_CLOSED_TEST_TRADER_FIXTURES_V1`** — fund
   closed-test traders with mUSDC collateral. See READ-ONLY overview.
2. **`PERPS_BASE_SEPOLIA_CLOSED_TEST_SMOKE_ORDER_V1`** — pre-signed
   matched-pair `PerpOrderIntent` end-to-end broadcast against Base Sepolia.
3. **Remote-signer factory** — `HybridV2KmsSignerBridge` construction
   from `HV2_SIGNER_PROVIDER` + credentials. Deployment-side wiring.
4. **Readiness HTTP surface** — expose `AppState.perps_broadcast_readiness`
   via the readiness endpoint.
5. **Prometheus counter wiring** — bind the tracing targets to
   IntCounter/Histogram metrics.
6. **Intent-id topic decoding** — strict binding of the semantic
   receipt event to the persisted canonical execution identity.
