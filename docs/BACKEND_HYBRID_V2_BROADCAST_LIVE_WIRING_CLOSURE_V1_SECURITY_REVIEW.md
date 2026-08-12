# Security Review — BACKEND-HYBRID-V2-BROADCAST-LIVE-WIRING-CLOSURE-V1

Every claim below is backed by a `file:line` reference to the
worktree at HEAD.

## Public-input non-influence

The admin `POST .../broadcast` body still refuses every wire-format
primitive:

* `BroadcastRequestBody` — empty struct with
  `#[serde(deny_unknown_fields)]` at
  `src/api/hybrid_v2_execution_admin.rs:926-927`. Any extraneous
  field (`raw_tx`, `nonce`, `gas`, `target`, `chain_id`, `signer`,
  `rpc_url`, `signature`, ...) is rejected by the deserializer
  before any store / RPC call.

No public-facing endpoint added by this milestone accepts a signed
envelope or a raw calldata payload from the caller.

## Fresh-submit refusal set

`src/hybrid_v2/execution/broadcast_reconstruction.rs` defines a
bounded refusal set that fires BEFORE the outbox is called:

* `CalldataBytesMissing` — line 57.
* `CalldataHashMismatch { persisted_hex, recomputed_hex }` — line 58.
* `CalldataHashMissing` — line 63.
* `PlanHashMissing`, `PlanHashMismatch` — lines 65, 67.
* `SignatureMissing`, `RecoveredSignerMissing` — lines 72, 74.
* `NonceMissing`, `GasFieldMissing` — lines 76, 78.
* `TargetContractMissing`, `SelectorMissing` — lines 80, 82.
* `ChainIdMismatch { row, manifest }` — line 84.
* `ManifestLookupFailure(_)` — line 86.
* `ValueNonZero(_)` — line 88.
* `MalformedHex(_)` — line 90.

`reconstruct_plan` runs the following checks in order
(`src/hybrid_v2/execution/broadcast_reconstruction.rs:99-181`):

1. `chain_id` sign + parity with `manifest.chain_id`.
2. keccak256(`calldata_bytes`) == stored `calldata_hash`.
3. target == `manifest.option_matching_engine`.
4. `tx_value_wei == "0"`.
5. recomputed `plan_hash` == persisted `plan_hash`.

Any failure returns an error variant WITHOUT invoking the RPC.

## Base-mainnet refusal

`refuse_mainnet(entry.manifest.chain_id)` runs at admin entry
(`src/api/hybrid_v2_execution_admin.rs:1120`). `chain_id == 8453`
returns 403 `BASE_MAINNET_FORBIDDEN` before any store or reconstruction
work.

## Outbox invariants (unchanged this milestone)

The fresh-submit path calls `BroadcastOutbox::submit` at
`src/api/hybrid_v2_execution_admin.rs:1265`, which is the same entry
the pre-broadcast orchestrator uses. All frozen invariants
(`persist-before-send`, `provider-hash-must-match`,
`envelope-hash-immutable`, `no-auto-nonce-replacement`,
`no-auto-fee-bump`, `no-auto-resign`) are enforced by the existing
outbox code:

* Firewall revalidation before send —
  `src/hybrid_v2/execution/broadcast_outbox.rs:231`.
* Persist tx_hash + envelope_hash BEFORE the network call —
  `src/hybrid_v2/execution/broadcast_outbox.rs:240-253`.
* Provider-hash mismatch escalation —
  `src/hybrid_v2/execution/broadcast_outbox.rs:358-369`.

## Worker supervision — no privileged escalation

`BroadcastConfirmationWorker::spawn_supervised` at
`src/hybrid_v2/execution/broadcast_worker.rs:250` is a pure
observation loop:

* Calls `tick` (batch listing + per-row observation) — no signer
  contact, no `send_raw_transaction`. The mock's assertion
  `write_method_calls() == ["eth_sendRawTransaction"]` (only the
  admin fresh-submit call is recorded) proves this end-to-end.
* Honours both `WorkerCancel::cancel()` AND
  `watch::Receiver<bool>.changed()`.
* No admin-token / operator secrets accessed.

## Shutdown handling (main.rs)

`src/main.rs` graceful shutdown block:

* `broadcast_worker_handle` is `Option<JoinHandle<()>>` — never
  panics if wiring failed.
* `cancel.cancel()` is idempotent.
* `tokio::time::timeout(Duration::from_secs(5), handle).await`
  prevents a stuck poll from blocking process exit; the timeout is
  observed via `warn!("broadcast worker join timeout after 5s —
  abandoning")` (visible in operator logs).

## Redaction

No signature bytes, raw calldata bytes, or provider connection
strings are surfaced on any admin JSON response added or modified
by this milestone. `SanitizedBroadcastRow`
(`src/api/hybrid_v2_execution_admin.rs:947`) intentionally omits
signature material; the fresh-submit response returns
`tx_hash` (public, recoverable from any chain observer) +
`provider_classification` (bounded enum string) only.

## Migration 0052 immutability

`migrations/0052_hybrid_v2_execution_calldata_bytes.sql:34-52`
defines `hybrid_v2_execution_requests_calldata_bytes_immutability`
which fires `BEFORE UPDATE` and raises when
`OLD.calldata_bytes IS NOT NULL AND NEW.calldata_bytes IS DISTINCT
FROM OLD.calldata_bytes`. Legacy `NULL -> Some(x)` transitions are
permitted (needed for a future back-fill); Some -> Some(different)
is blocked at the DB layer AND mirrored by the in-memory store at
`src/hybrid_v2/persistence.rs` (in-memory immutability guard).

The E2E test intentionally disables the trigger to simulate a
tampered row and confirm the application layer refuses with
`CALLDATA_HASH_MISMATCH` even when the DB is compromised.

## Test-suite proof of no public-chain traffic

`tests/hybrid_v2_broadcast_live_wiring_e2e_pg_integration.rs` uses
the in-process `MockBroadcastRpc` exclusively; the mock refuses to
be constructed with a live URL and records every write method it
observes. Every test asserts:

* On success paths: `mock.write_method_calls() ==
  ["eth_sendRawTransaction"]`.
* On refusal paths: `mock.write_method_calls().is_empty()`.

NO REAL PUBLIC-CHAIN TRANSACTION SENT.
