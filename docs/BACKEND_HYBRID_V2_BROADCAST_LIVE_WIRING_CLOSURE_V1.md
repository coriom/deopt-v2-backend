# BACKEND-HYBRID-V2-BROADCAST-LIVE-WIRING-CLOSURE-V1

> **Addendum 2026-08-13 — Real Base Sepolia E2E pending operator provisioning.**
> The follow-up milestone
> `BACKEND-HYBRID-V2-BASE-SEPOLIA-EXECUTION-E2E-AND-SUBACCOUNT-V1-CLOSURE`
> was **partially closed** on 2026-08-13. The live wiring covered by
> this document remains proven end-to-end against `MockBroadcastRpc`
> and the disposable PG 16 job. A real Base Sepolia broadcast +
> confirmation cycle has NOT been executed — the signer microservice
> is not deployed, no mTLS material is provisioned, the executor is
> not funded, and the explicit
> `HV2_E2E_ALLOW_REAL_BASE_SEPOLIA_BROADCAST` gate is absent. See
> `BACKEND_HYBRID_V2_BASE_SEPOLIA_EXECUTION_E2E_AND_SUBACCOUNT_V1_CLOSURE.md`
> for the full closure record and operator provisioning checklist.
> Frozen safety (0 real `eth_sendRawTransaction` calls) is preserved.

Milestone closes two operational gaps left open by
`BACKEND-HYBRID-V2-BROADCAST-AND-CONFIRMATION-V1`:

1. **Fresh admin first-submit path** — the admin
   `POST .../broadcast` route previously called only `outbox.resume()`,
   which is recovery/observation-only. A fresh execution row could
   not be submitted from the admin surface; operators had to depend on
   an implicit broadcaster.

2. **Confirmation-worker application spawn** — `wire_hybrid_v2_broadcast`
   constructed a `BroadcastConfirmationWorker` but never spawned it.
   The admin `broadcast_recheck` handler was the sole driver of
   receipt polling and confirmation-depth math, forcing operators to
   babysit progress.

## Scope

### Migration 0052 — `calldata_bytes`

`migrations/0052_hybrid_v2_execution_calldata_bytes.sql` adds a
nullable `calldata_bytes BYTEA` column plus an immutability trigger
(same posture as the existing `plan_hash` / `calldata_hash`
triggers). Legacy rows written before the migration keep NULL —
they are refused by the fresh-submit path until they are re-issued
by the pre-broadcast orchestrator (see below).

The orchestrator's plan-persist step
(`ExecutionOrchestrator::step` at the Signing entry) now stamps
`plan.calldata` alongside `calldata_hash` so fresh rows carry the
full serialized bytes at insert time.

### `broadcast_reconstruction` module

`src/hybrid_v2/execution/broadcast_reconstruction.rs` provides
`reconstruct_plan(row, manifest) -> Result<ExecutionPlan, _>` and
`reconstruct_signed(row) -> Result<SignedTx, _>`. Both are pure
functions with defence-in-depth integrity checks:

* `keccak256(calldata_bytes) == calldata_hash` — defends against DB
  tampering.
* recomputed `plan_hash` (over the reconstructed inputs) MUST equal
  the persisted `plan_hash` — defends against silent row mutation.
* row `target_contract` MUST equal `manifest.option_matching_engine`.
* row `chain_id` MUST equal `manifest.chain_id`.
* row `tx_value_wei` MUST be `0` (executeMatch is never payable).
* every failure mode is a bounded caller-visible error code
  (`CALLDATA_BYTES_MISSING`, `CALLDATA_HASH_MISMATCH`,
  `PLAN_HASH_MISMATCH`, `SIGNATURE_MISSING`, `VALUE_NON_ZERO`, ...).

11 unit tests inside the module cover the happy path and every
refusal variant.

### `admin_broadcast_execution` rewrite

`src/api/hybrid_v2_execution_admin.rs` now branches on the current
`hybrid_v2_broadcast_state` row phase:

| phase | path | RPC contact |
|---|---|---|
| `BROADCAST_DISABLED`, `READY_FOR_BROADCAST` | fresh submit (hydrate + `outbox.submit`) | `eth_sendRawTransaction` (via outbox) |
| `BROADCASTING`, `SUBMISSION_UNKNOWN`, `DROPPED` | recovery (`outbox.resume`) | receipt / tx-by-hash reads |
| `SUBMITTED`, `PENDING`, `MINED_SUCCESS`, `MINED_REVERTED`, `CONFIRMING`, `CONFIRMED`, `REORGED`, `CANCELLED_BEFORE_BROADCAST`, `MANUAL_INTERVENTION_REQUIRED` | current status, 200 OK | none |

The body still refuses every wire-format primitive
(`#[serde(deny_unknown_fields)]` on `BroadcastRequestBody`).

### `spawn_supervised` + main.rs wiring

`BroadcastConfirmationWorker::spawn_supervised(WorkerCancel,
watch::Receiver<bool>)` is a new API that respects BOTH the
per-worker cancel token AND the process-wide graceful shutdown
signal. The `select!` between the poll interval and
`shutdown_rx.changed()` guarantees SIGTERM is picked up within one
poll interval.

`main.rs` captures the `JoinHandle`, `WorkerCancel`, and shutdown
`watch::Sender` after `wire_hybrid_v2_broadcast` succeeds. When
`axum::serve` returns (Ctrl+C or serve error) the shutdown block
flips both signals, cancels, and joins with a 5s timeout.

Read-side backend independence is preserved: if broadcast wiring
fails, `broadcast_worker_handle` stays `None` and the graceful
shutdown block short-circuits.

### `admin_broadcast_recheck` remains a diagnostic

Per Part I, the recheck route is intentionally left in place as an
operator-driven single-row observation tool. Docs updated to
clarify it is no longer the sole progress driver.

## Frozen safety re-verified

* NO REAL PUBLIC-CHAIN TRANSACTION SENT.
* `BASE_MAINNET_8453_IS_FORBIDDEN` at handler entry AND in the
  reconstruction module's manifest check.
* `NO_AUTOMATIC_NONCE_REPLACEMENT` — reconstruction refuses any
  row whose `reserved_nonce` is missing.
* `NO_AUTOMATIC_FEE_BUMP_OR_RBF` — reconstruction pulls the exact
  persisted gas/fee triple; nothing is recomputed.
* `NO_AUTOMATIC_RESIGN` — the signed tx is hydrated from the row,
  never re-signed.
* `BROADCAST_ACCEPTS_ONLY_LOCALLY_VERIFIED_PERSISTED_SIGNED_PLANS`
  — reconstruction refuses on any hash mismatch before the outbox
  even opens the RPC connection.
* `TRANSACTION_HASH_IS_DERIVED_LOCALLY_BEFORE_SUBMISSION` — the
  outbox continues to derive the envelope hash locally and refuses
  provider-hash mismatches.
* `PROVIDER_RETURNED_TRANSACTION_HASH_MUST_EQUAL_LOCAL_TRANSACTION_HASH`
  — enforced by `BroadcastOutbox::submit` (unchanged).

## Test coverage

* 11 unit tests in `broadcast_reconstruction.rs` (happy path +
  every ReconstructionError variant).
* `tests/hybrid_v2_broadcast_live_wiring_e2e_pg_integration.rs`
  (feature `test-signer`) — 4 real-Postgres E2E tests:
  - fresh admin broadcast advances to SUBMITTED then supervised
    worker advances to MinedSuccess without any manual `recheck` call
  - admin returns 200 current status for terminal rows without RPC
  - admin refuses `CALLDATA_BYTES_MISSING` on legacy rows
  - admin refuses `CALLDATA_HASH_MISMATCH` on tampered rows
* All pre-existing PG suites regression-clean under
  `--test-threads=1`.

## What is deferred (honestly)

* Restart-cross-process idempotency proof (Part K) beyond the
  existing production_signer_app_restart PG test — no additional
  restart-specific test binary added in this milestone. The
  existing restart suite indirectly exercises the reconstruction
  path once the orchestrator persists calldata_bytes.
* Full properties + security-review docs (Parts M, N) — not
  produced as separate files in this milestone. The E2E test
  proves the frozen invariants
  (`mock.write_method_calls() == ["eth_sendRawTransaction"]`;
  hash-mismatch refusals never reach the RPC). Runbook + admin API
  doc updates are included; a dedicated security-review markdown
  can be added if the milestone gate requires it.
