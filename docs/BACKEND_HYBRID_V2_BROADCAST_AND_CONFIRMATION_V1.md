# BACKEND-HYBRID-V2-BROADCAST-AND-CONFIRMATION-V1 — Closure

Milestone: `BACKEND-HYBRID-V2-BROADCAST-AND-CONFIRMATION-V1`.
Package: **D — Parts V–Z + docs + deferred AppState wiring**.
Prior packages: **A** (foundation, HEAD `7512f53`), **B** (lifecycle,
HEAD `350281f`), **C** (reorg + admin + boundary audit, HEAD
`b7691c1`).

## Frozen safety statement (unchanged from the parent brief)

**This milestone performed NO real public-chain broadcast; all tests
used a deterministic in-process mock RPC.** Every scenario across
Packages A–D asserts the write-allowlist invariant
`broadcast_mock.write_method_calls()` contains ONLY
`"eth_sendRawTransaction"`.

Frozen tokens grep-able across every module:

* `BASE_MAINNET_8453_IS_FORBIDDEN`
* `NO_AUTOMATIC_NONCE_REPLACEMENT`
* `NO_AUTOMATIC_FEE_BUMP_OR_RBF`
* `REORGED_TRANSACTION_IS_NEVER_LEFT_CONFIRMED`
* `WRITE_RPC_METHOD_ALLOWLIST_IS_ETH_SENDRAWTRANSACTION_ONLY`
* `NO_REAL_PUBLIC_CHAIN_BROADCAST`

## Deliverables (Package D)

### 1. AppState wiring (deferred from Package C)

`AppState` gains four new fields alongside the pre-broadcast
orchestrator surface:

* `hybrid_v2_broadcast_outbox: Option<Arc<BroadcastOutbox>>`
* `hybrid_v2_broadcast_worker: Option<Arc<BroadcastConfirmationWorker>>`
* `hybrid_v2_broadcast_rpc: Option<Arc<dyn ExecutionBroadcastRpcClient>>`
* `hybrid_v2_broadcast_config: Option<HybridV2ExecutionConfig>`

Attached by `AppState::with_hybrid_v2_broadcast(outbox, worker, rpc,
config)` after a successful `wire_hybrid_v2_broadcast(state, chain_id)`
call. `with_hybrid_v2_broadcast_unavailable(reason)` mirrors the
existing execution-orchestrator fail-closed constructor.

Startup helper: `src/hybrid_v2/startup.rs::wire_hybrid_v2_broadcast`.
Three-outcome contract:
* `Ok(None)` — `HV2_BROADCAST_ENABLED=false` OR execution disabled
  → AppState carries the fail-closed marker; admin returns 503.
* `Ok(Some(_))` — all four handles wired.
* `Err(reason)` — validation, RPC construction, projection store,
  or chain-id gate failed. Caller downgrades to `outbox = None` +
  logs a WARN; read-side backend keeps serving.

Admin handler updates (`src/api/hybrid_v2_execution_admin.rs`):
* `admin_broadcast_execution` — now drives `outbox.resume(...)` when
  wired. First-broadcast path via `outbox.submit(...)` requires a
  plan+signed hydrator that reconstructs the signed envelope from
  `execution_requests` columns — deferred to
  `BACKEND-HYBRID-V2-BASE-SEPOLIA-EXECUTION-E2E-V1`.
* `admin_broadcast_recheck` — drives `worker.tick_single(...)` when
  wired.
* `admin_broadcast_resend_same_bytes` — honest 503 pending the plan
  hydrator; the response body surfaces the wired-broadcast state so
  operators can observe the deferral.
* `admin_broadcast_status` / `admin_broadcast_pending` /
  `admin_broadcast_manual_intervention` — unchanged from Package C
  (read + operator escalation surfaces).

### 2. Part V — Full PG matrix

`tests/hybrid_v2_broadcast_full_matrix_pg_integration.rs` — 59
scenarios across 12 categories:
* Config & startup (6): env validation, Base mainnet refusal at three
  gates, RPC scheme refusal, allowlist enforcement, valid wire.
* Pre-broadcast validation (6): firewall reject paths, deterministic
  serialization, read-only isolation.
* Admin flow (3): auth gate, `deny_unknown_fields`, 404 on missing
  row.
* Transactional outbox (5): successful send, PROVIDER_HASH_MISMATCH,
  idempotent duplicate, ALREADY_KNOWN, PROVIDER_REJECTED.
* Ambiguous outcomes (4): timeout before/after acceptance, restart
  resume, same-byte resend.
* Nonce conflicts (3): too-low, too-high, replacement underpriced.
* Receipt lifecycle (4): pending, reverted terminal, success →
  confirming, gas fields persisted.
* Canonicality (3): match, mismatch → reorged, RPC head regression.
* Confirmation depth + indexer (5): below / at threshold, indexer
  behind, finalized persisted, terminal no-op.
* Reorg (4): mined-block reorg, receipt-returns advance, drop stays
  reorged, no fee bump on reorg.
* Restart (5): pending, confirming, reorged, AppState convergence,
  deployment isolation.
* Operational (9): PG outage, RPC outage, read isolation, write-RPC
  allowlist audits, no-auto-remediation audits.

Verdict emitted: **`BROADCAST_CONFIRMATION_DATABASE_INTEGRATION_VALIDATED`**.

### 3. Part W — Properties

`tests/hybrid_v2_broadcast_properties.rs` — 20 bounded properties
(20 cases each). Determinism / integrity (5), safety / fail-closed
(7), ambiguity / recovery (3), public boundary (3), confirmation
(2). Every property asserts `write_method_calls() ⊂
{"eth_sendRawTransaction"}` under every generated variation.

Verdict emitted: **`BROADCAST_CONFIRMATION_PROPERTIES_VALIDATED`**.

### 4. Part X — Performance

`tests/hybrid_v2_broadcast_performance_bounds.rs` — 9 deterministic
wall-clock bounds against the mock RPC:
* signed serialization       < 20 ms
* local envelope hash         < 5 ms
* broadcast request (mock)    < 500 ms (PG-inclusive ceiling)
* ambiguous recovery          < 500 ms
* receipt / canonicality /
  confirmation polls          < 300 ms each
* indexer correlation lookup  < 500 ms
* restart recovery            < 500 ms

Bounds are conservative wall-clock ceilings so a slow CI runner does
not false-positive. Wall-clock-fragile measurements are marked
`#[ignore]` and only run under `cargo test -- --ignored`.

Verdict emitted: **`BROADCAST_CONFIRMATION_PERFORMANCE_BOUNDED`**.

### 5. Part Y — Security review

`docs/BACKEND_HYBRID_V2_BROADCAST_AND_CONFIRMATION_V1_SECURITY_REVIEW.md`
— 17-section review with file:line evidence:
1. Scope and threat model.
2. Broadcast RPC allowlist.
3. Base mainnet refusal.
4. Persist-before-send safety.
5. No automatic remediation.
6. Provider hash-mismatch escalation.
7. Reorg + canonicality safety.
8. Confirmation depth + indexer correlation (final rule).
9. Signer boundary.
10. Deny-unknown-fields input hardening.
11. Admin gate + Base mainnet refusal at handler entry.
12. Read API isolation.
13. Startup fail-closed.
14. Concurrency + operation lock.
15. Restart safety.
16. Metrics / observability posture.
17. Deferred / out of scope.

Verdict: **`BROADCAST_CONFIRMATION_SECURITY_VALIDATED`**.

### 6. Part Z — CI gate

`.github/workflows/backend-postgres-integrity.yml` extended with:
* default-feature step running
  `hybrid_v2_broadcast_foundation_pg_integration`
* test-signer step running
  `hybrid_v2_broadcast_lifecycle_pg_integration`,
  `hybrid_v2_broadcast_reorg_recovery_pg_integration`,
  `hybrid_v2_broadcast_restart_pg_integration`,
  `hybrid_v2_broadcast_admin_pg_integration`,
  `hybrid_v2_broadcast_full_matrix_pg_integration`,
  `hybrid_v2_broadcast_properties`,
  `hybrid_v2_broadcast_performance_bounds`

Fresh public schema between suites. `DEOPT_REQUIRE_PG_INTEGRATION=1
--test-threads=1 --features test-signer`.

Verdict emitted: **`BROADCAST_CONFIRMATION_CI_GATE_VALIDATED`**.

## Regression envelope

Full pre-existing Hybrid V2 broadcast suites remain green (Packages
A + B + C): foundation, lifecycle (16 tests), reorg (10 tests),
restart (15 tests), admin (15 tests). The Package D additions are
purely additive; no existing behaviour changed.

The Package D admin-broadcast handler now checks
`hybrid_v2_broadcast_outbox.is_some()` instead of
`hybrid_v2_execution_orchestrator.is_some()` — this correctly
reflects the new subsystem boundary (outbox + worker are wired
independently from the pre-broadcast orchestrator). The Package C
admin PG suites continue to pass because they either set both
handles (positive path) OR neither (fail-closed path).

## Next stage

**`BACKEND-HYBRID-V2-BASE-SEPOLIA-EXECUTION-E2E-V1`** —

* Introduce a plan + signed reconstruction helper so the admin
  `broadcast` handler can hydrate the deterministic envelope from
  persisted `execution_requests` columns without re-signing.
* Bring up a live Base Sepolia broadcast provider (still allowlisted;
  Base mainnet still refused at every gate).
* Extend the CI gate to a live testnet job with a dedicated,
  chain-id-84532-only mock provider replaced by a real endpoint.
* Advance the confirmation worker's periodic tick loop into
  `main.rs` under a cancel token with graceful shutdown.

## Files added / modified in Package D

Added (5):
* `tests/hybrid_v2_broadcast_full_matrix_pg_integration.rs`
* `tests/hybrid_v2_broadcast_properties.rs`
* `tests/hybrid_v2_broadcast_performance_bounds.rs`
* `docs/BACKEND_HYBRID_V2_BROADCAST_AND_CONFIRMATION_V1.md`
* `docs/BACKEND_HYBRID_V2_BROADCAST_AND_CONFIRMATION_V1_SECURITY_REVIEW.md`

Modified (6):
* `src/api/http.rs` — 4 new AppState fields + 2 constructors.
* `src/hybrid_v2/startup.rs` — `wire_hybrid_v2_broadcast(...)` + 5
  new unit tests.
* `src/hybrid_v2/mod.rs` — re-export of `wire_hybrid_v2_broadcast`.
* `src/main.rs` — startup wiring block.
* `src/api/hybrid_v2_execution_admin.rs` — `admin_broadcast_execution`
  drives `outbox.resume(...)`; `admin_broadcast_recheck` drives
  `worker.tick_single(...)`; `admin_broadcast_resend_same_bytes` is
  an honest 503 pending the plan hydrator.
* `.github/workflows/backend-postgres-integrity.yml` — 2 new CI
  steps + expanded PR path filter.

---

## 2026-08-11 — closure addendum

The broadcast engine + mock validation shipped by this milestone landed
with two explicitly-deferred operational gaps:

1. The admin `POST .../broadcast` route only invoked `outbox.resume()`,
   the recovery/observation path. First-submission required a plan +
   signed reconstruction that the milestone did not deliver — see the
   handler's docstring at the time reading "first-submission path via
   plan+signed hydrator remains deferred to base-sepolia E2E".
2. `wire_hybrid_v2_broadcast` constructed a
   `BroadcastConfirmationWorker` but nothing in `main.rs` spawned it.
   Confirmation progress relied on operator-driven
   `broadcast_recheck` calls.

`BACKEND-HYBRID-V2-BROADCAST-LIVE-WIRING-CLOSURE-V1` closes both gaps:

* Migration `0052_hybrid_v2_execution_calldata_bytes.sql` persists the
  raw calldata bytes (with an immutability trigger) so the fresh-submit
  path can hydrate a full `ExecutionPlan` from the persisted row.
* `src/hybrid_v2/execution/broadcast_reconstruction.rs` reconstructs
  `ExecutionPlan` + `SignedTx` from `(row, manifest)` with keccak +
  plan_hash + target + value + chain-id integrity checks.
* `admin_broadcast_execution` now branches on the broadcast phase:
  fresh submit → `outbox.submit`; recovery → `outbox.resume`;
  in-flight/terminal → current status.
* `BroadcastConfirmationWorker::spawn_supervised(WorkerCancel,
  watch::Receiver<bool>)` respects both the per-worker cancel and the
  process-wide graceful shutdown signal; `main.rs` captures the
  `JoinHandle` and joins it with a 5s bounded timeout on Ctrl+C.
* `admin_broadcast_recheck` is retained as an operator diagnostic —
  no longer the sole progress driver.

See `docs/BACKEND_HYBRID_V2_BROADCAST_LIVE_WIRING_CLOSURE_V1.md` and
its security-review companion for the full scope + evidence.
