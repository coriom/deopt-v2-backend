# BACKEND-SIGNER-INTERFACE-KMS-HSM-ADAPTER — result

**Status:** SHIPPED 2026-06-09 (Phase G close-out).
**Scope:** introduce the `RemoteSigner` trait abstraction, retain
`LocalDevSigner` for Sepolia + dev, add a mock-injectable
`RemoteSignerClient`, wire mainnet startup refusal of
`EXECUTOR_PRIVATE_KEY`, integrate into the option execution broadcast
path with structured signer rejection. **No real KMS / vendor credentials.
No mainnet tx. No live broadcast. No `.env` edit.**

---

## 1. Files changed

### New (2)

- `deopt-v2-backend/src/execution/remote_signer.rs` — ~700 LoC incl.
  16 unit tests. Defines `SignerBackendKind`, `SignerRequest`,
  `SignerResponse`, `SignerError`, `SignerHealth`, the `RemoteSigner`
  trait, the `SignerTransport` trait, `LocalDevSigner`,
  `RemoteSignerClient` + `UnimplementedTransport`, and the
  `policy_fingerprint` helper.
- `deopt-v2-backend/docs/BACKEND_SIGNER_INTERFACE_KMS_HSM_ADAPTER_RESULT.md`
  — this close-out doc.

### Modified (6)

- `src/execution/mod.rs` — `pub mod remote_signer;` + re-export of the
  public surface + new `assemble_eip1559_signed_transaction` +
  `eip1559_transaction_prehash` re-exports.
- `src/execution/transaction.rs` — factored
  `sign_eip1559_transaction` into a reusable
  `eip1559_transaction_prehash` + `assemble_eip1559_signed_transaction`
  pair so both the local-key and remote-signer paths share the EIP-1559
  RLP assembly logic.
- `src/execution/config.rs` — three new fields
  (`backend_signer_mode`, `backend_signer_endpoint`,
  `executor_allow_local_signer`); new `validate_signer_backend()`
  method; extended `validate_startup` with mainnet refusal of
  `EXECUTOR_PRIVATE_KEY` and Remote / LocalDev guards; 7 new
  startup-guard tests.
- `src/config/env.rs` — env loader plumbs `BACKEND_SIGNER_MODE`,
  `BACKEND_SIGNER_ENDPOINT`, `EXECUTOR_ALLOW_LOCAL_SIGNER`; mode
  auto-selection rule (explicit value > endpoint-presence > LocalDev
  default); 6 existing real-broadcast tests updated to include
  `EXECUTOR_ALLOW_LOCAL_SIGNER=true` so they continue to exercise the
  pre-existing per-field error paths under the new abstraction.
- `src/options/service.rs` — new `sign_option_execution_via_signer`
  helper computing the prehash + calldata_hash + policy fingerprint and
  calling `RemoteSigner::sign_option_execution_tx`; new
  `build_signer_for_state` + new public
  `broadcast_option_execution_intent_with_provider_and_signer` variant;
  existing `broadcast_option_execution_intent_with_provider` delegates
  through the new pair; 6 new integration tests; structured-log line on
  signer approve + warn-log line on signer reject (no secrets).
- `src/execution/executor.rs`, `src/execution/simulator.rs`,
  `src/execution/transaction.rs::tests`, `tests/engine_tests.rs` —
  4 literal `ExecutionConfig` constructions extended with the 3 new
  fields to keep cross-compile clean.

## 2. Signer abstraction introduced

| Type / fn                                    | Location                                  | Notes                                                                 |
| -------------------------------------------- | ----------------------------------------- | --------------------------------------------------------------------- |
| `trait RemoteSigner`                         | `execution::remote_signer`                | `signer_address`, `kind`, `sign_option_execution_tx`, `health_check`. |
| `enum SignerBackendKind { LocalDev, Remote}` | `execution::remote_signer`                | Mode discriminator + `parse`.                                         |
| `struct SignerRequest<'a>`                   | `execution::remote_signer`                | Mirrors design doc §3.1.                                              |
| `struct SignerResponse`                      | `execution::remote_signer`                | Mirrors design doc §3.2.                                              |
| `enum SignerError` (20 variants)             | `execution::remote_signer`                | Stable codes per design doc §4.2 + `Transport` / `Internal` / `ConfigRefusal`. |
| `struct SignerHealth`                        | `execution::remote_signer`                | Liveness + identity.                                                  |
| `trait SignerTransport`                      | `execution::remote_signer`                | Mockable transport for `RemoteSignerClient`.                          |
| `struct LocalDevSigner`                      | `execution::remote_signer`                | Wraps `ExecutorSigner`; refuses mainnet at runtime.                   |
| `struct RemoteSignerClient`                  | `execution::remote_signer`                | mTLS HTTPS/2 client; production transport = `UnimplementedTransport`. |
| `fn policy_fingerprint(...)`                 | `execution::remote_signer`                | `keccak256(decision_id ‖ calldata_hash ‖ nonce ‖ block ‖ deadline)`.  |
| `fn build_signer_for_state(state)`           | `options::service`                        | Selects impl from `state.execution_config`; runtime mainnet refusal.  |
| `broadcast_option_execution_intent_with_provider_and_signer` | `options::service`         | Canonical broadcast entry point accepting an external signer (mock / prod). |

## 3. Config keys added

| Env key                          | Default        | Required on mainnet | Notes                                                                                     |
| -------------------------------- | -------------- | ------------------- | ----------------------------------------------------------------------------------------- |
| `BACKEND_SIGNER_MODE`            | (auto)         | YES — must be `remote` | Explicit `local_dev` / `remote`. Auto-selects `Remote` if `BACKEND_SIGNER_ENDPOINT` is set, else `LocalDev`. |
| `BACKEND_SIGNER_ENDPOINT`        | unset          | YES                 | HTTPS URL of signer microservice. mTLS / IAM details out-of-tree.                         |
| `EXECUTOR_ALLOW_LOCAL_SIGNER`    | `false`        | NO (must be `false`)| Explicit opt-in for `LocalDev` on testnet `chain_id ≠ 31337`. Anvil exempt.               |

`EXECUTOR_PRIVATE_KEY` is REFUSED on `chain_id = 8453` even if other
fields are correct (`validate_startup` early-exit, no key dereference).

## 4. Mainnet startup guard

Three independent checks; each fails closed with a distinct message:

1. `chain_id == 8453 AND executor_private_key.is_some()` →
   `Config("EXECUTOR_PRIVATE_KEY must NOT be set on mainnet (chain_id=8453); use BACKEND_SIGNER_MODE=remote …")`.
2. `chain_id == 8453 AND backend_signer_mode == LocalDev` →
   `Config("BACKEND_SIGNER_MODE=local_dev is REFUSED on mainnet …")`.
3. `backend_signer_mode == Remote AND backend_signer_endpoint.is_empty()` →
   `Config("BACKEND_SIGNER_ENDPOINT is required when BACKEND_SIGNER_MODE=remote")`.

A runtime defence-in-depth re-check fires at the broadcast site
(`build_signer_for_state`) so any post-startup config mutation that
seats `LocalDev` on mainnet fails closed before reaching the signer.

## 5. Local-dev / testnet behavior

- `chain_id == 31337` (anvil) → `LocalDevSigner` allowed unconditionally.
- `chain_id == 84532` (Sepolia) or any non-mainnet → `LocalDevSigner`
  allowed iff `EXECUTOR_ALLOW_LOCAL_SIGNER=true`.
- `LocalDevSigner::sign_option_execution_tx` runtime-refuses
  `chain_id == 8453` even if startup guard somehow missed it
  (`SignerError::ConfigRefusal("local-dev-signer refused on mainnet chain id")`).
- Existing Sepolia rehearsal regression preserved: `state_with_broadcast`
  test fixtures continue to set `LocalDev` with chain id 84532 + test key;
  all 47 existing `options::service` tests pass unchanged.

## 6. Remote signer behavior

- `RemoteSignerClient::new(endpoint, expected_address)` uses
  `UnimplementedTransport` (returns `SignerError::Transport("production HTTPS/mTLS transport not yet wired …")`)
  — production transport will land in a follow-on track once Q-CD-5 vendor
  is selected; until then, mainnet broadcast is purely a fail-closed
  posture.
- `RemoteSignerClient::with_transport(...)` accepts a `SignerTransport`
  mock for tests + future sandbox integration.
- Post-sign cross-check: response `signer_address` MUST equal the
  configured expected EOA → mismatch produces
  `SignerError::PostSignFromMismatch` (design doc §4.A15).
- Round-trip check: response `policy_decision_id` MUST equal the request's
  decision id → mismatch produces `SignerError::Internal(...)`.

## 7. Broadcast path integration

Order at the wired call site (`broadcast_option_execution_intent_with_provider_and_signer`):

1. `ensure_option_execution_broadcast_enabled`.
2. `get_option_execution_intent`.
3. `should_broadcast` policy gate (existing; unchanged).
4. Build `ExecutionTransactionRequest`.
5. Compute `nonce` via provider.
6. Compute prehash + calldata_hash + `policy_decision_id` (UUID v4) +
   `policy_fingerprint`.
7. Construct `SignerRequest` with all metadata.
8. `signer.sign_option_execution_tx(req).await`.
9. On Reject → write `BroadcastFailed` with `signer:<code>:<detail>`;
   structured warn-log; **NO local-key fallback**.
10. On Approve → assemble raw EIP-1559 tx using
    `assemble_eip1559_signed_transaction` + signature components;
    structured info-log carrying `kms_request_id` / `audit_log_id` /
    `remote_signer_request_id` / `signer_address`.
11. `provider.send_raw_transaction(raw)` (unchanged).

## 8. Request / response / log fields

### Request fields propagated to signer

`request_id` (fresh UUID per call) · `intent_id` · `source_type` ·
`chain_id` · `target_contract` · `function_selector` (hex) ·
`calldata_hash` (32-byte keccak) · `calldata_length` · `transaction_to` ·
`transaction_value_wei` (= 0) · `gas_limit` · `max_fee_per_gas_wei` ·
`max_priority_fee_per_gas_wei` · `nonce` · `simulation_block` ·
`deadline_ms` · `policy_decision_id` · `policy_fingerprint` ·
`policy_decision_at_ms` · `prehash`.

### Response fields persisted / logged

`request_id` · `signer_address` (cross-checked) · `signature` (recoverable
secp256k1: y_parity + r + s) · `kms_request_id` (opaque correlation) ·
`audit_log_id` (signer-side) · `remote_signer_request_id` (transport-side)
· `created_at_ms` · `policy_decision_id` (echo).

### Structured log lines (tracing target = `broadcast_signer`)

- Approve (info!): `intent_id`, `signer_kind`, `policy_decision_id`,
  `kms_request_id`, `audit_log_id`, `remote_signer_request_id`,
  `signer_address` (the AccountId string only).
- Reject (warn!): `intent_id`, `signer_kind`, `code` (stable code).

Never logged: private keys, mTLS cert bytes, KMS IAM credentials, RPC
URLs, admin tokens, DATABASE_URL, raw signed tx bytes. `LocalDevSigner`'s
`Debug` carries `<redacted>`; `RemoteSignerClient`'s `Debug` carries
`<redacted-url>`.

## 9. Tests added

### Unit (16 in `execution::remote_signer::tests`)

LocalDev: 3 (Sepolia round-trip, anvil round-trip, mainnet runtime refusal).
RemoteSignerClient: 6 (mock-approval round-trip, mock denial, post-sign
address mismatch, policy_decision_id mismatch, production stub returns
Transport error, mock health-check).
LocalDev health-check: 1.
Redaction: 3 (SignerError Display, LocalDev Debug redaction,
RemoteSignerClient Debug redaction).
SignerBackendKind parse: 1.
`policy_fingerprint` vector test: 1 (5 input-sensitivity assertions).
`signer_error_into_backend` helper: 1.

### Startup-guard (7 in `execution::config::tests`)

Mainnet + env key → refused. Mainnet + LocalDev mode → refused. Mainnet
+ Remote + endpoint → allowed. Mainnet + Remote without endpoint →
refused. Sepolia + LocalDev without allow → refused. Sepolia + LocalDev
+ allow → allowed. Anvil + LocalDev → allowed unconditionally.

### Integration (6 in `options::service::tests`)

`signer_approve_routes_through_broadcast_and_marks_submitted` — mock
signer Approve → 1 sign call → 1 chain send → `BroadcastSubmitted`.

`signer_not_called_when_policy_rejects` — wash buyer==seller → policy
Reject → 0 sign calls → 0 chain sends.

`signer_rejection_transitions_to_broadcast_failed_without_fallback` —
mock signer Reject(PolicyFingerprint) → `BroadcastFailed` with
`signer:policy-fingerprint:` reason; 1 sign call; 0 chain sends; tx row
persisted with `tx_hash=None`.

`signer_kms_timeout_marks_intent_failed_no_local_fallback` — mock signer
Reject(KmsTimeout) → `BroadcastFailed` with `signer:kms-timeout:` reason;
0 chain sends; no local-key fallback exercised.

`build_signer_refuses_local_dev_on_mainnet_at_runtime` — runtime
defence-in-depth.

`build_signer_refuses_remote_without_endpoint` — config error.

### Existing-test updates (6 in `config::env::tests`)

`real_broadcast_enabled_requires_private_key`,
`real_broadcast_enabled_requires_rpc_url`,
`real_broadcast_enabled_requires_fee_config`,
`real_broadcast_enabled_rejects_invalid_private_key`,
`real_broadcast_enabled_accepts_complete_static_config`,
`private_key_is_redacted_from_execution_config_debug` — each gets
`EXECUTOR_ALLOW_LOCAL_SIGNER=true` so the new signer-mode guard does NOT
short-circuit ahead of the per-field error each test asserts.

### Total

29 new tests + 6 existing tests updated; 0 tests deleted.

## 10. Tests run

```
cargo fmt --all -- --check                                          : ok
cargo clippy --all-targets --all-features -- -D warnings            : ok
cargo test --all-targets --all-features --no-fail-fast              :
  lib                                                                : 582 / 582 ✓
  integration suites (api / nonce_sync / fees / rfq / options / signatures / mm-protocol) : 8 + 12 + 43 + 67 + 76 + 13 + 37 = 256 ✓
  grand total                                                        : 838 / 838 ✓
forge fmt --check                                                    : ok
forge build                                                          : ok (pre-existing lint warnings only)
forge test                                                           : not re-run; no sol source touched (367/367 in prior milestone)
```

## 11. Remaining work (deferred; follow-on PRs)

Documented as "out of scope this milestone" — not blockers for this PR:

- **Real KMS / HSM / MPC transport wiring.** `RemoteSignerClient`
  ships with `UnimplementedTransport`; the production HTTPS/2 + mTLS
  wire transport lands in a follow-on track gated on
  `MAINNET-KMS-VENDOR-SELECTION` (Q-CD-5). Mainnet broadcast is
  fail-closed until then.
- **Persistent `policy_decision_id` storage.** The decision id is
  currently a fresh UUID per call; a follow-on track adds a
  `broadcast_policy_decisions` table per design doc §5.1.
- **Audit log sink.** Per-sign structured logs are emitted on
  `tracing target = "broadcast_signer"`; long-term append-only sink
  (S3 Object Lock / GCS / CloudWatch) is operator-side per design §7.1.
- **`kms_request_id` propagation to confirmation receipts.** The id
  is captured at sign time; receipt-side correlation lands when the
  monitoring-alerts wiring track runs
  (`BACKEND-EXECUTOR-MONITORING-ALERTS-V1-WIRING`).
- **mTLS issuance topology.** Whether private CA, SPIFFE/SPIRE, or
  cloud-managed cert is operator + DevOps decision (design §10).
- **Stale-policy window tracking.** `STALE_POLICY_MAX_AGE_MS`
  enforcement currently sits in the signer service (per design); the
  backend timestamps `policy_decision_at_ms` but does not yet enforce
  the window itself.

## 12. Out of scope (explicitly NOT done)

- No mainnet broadcast attempted.
- No `.env` edited.
- No KMS / vendor / Treasury Safe creation.
- No vendor account / region / ARN / IAM role string in code or docs.
- No `EXECUTOR_PRIVATE_KEY` value printed in code, tests, docs, or
  commit messages.
- No real KMS sandbox key used in tests (mock-server `SignerTransport`
  only).
- No sol/ source touched.
- No DB schema migration.

## 13. Cross-references

- Spec: `deopt-v2-backend/docs/MAINNET_BE_SIGNER_SERVICE_DESIGN.md`.
- Origin prompt: `deopt-v2-backend/docs/BACKEND_SIGNER_INTERFACE_KMS_HSM_ADAPTER_NEXT_TASK.md`.
- Custody principle: `~/DEOPT/MAINNET_CUSTODY_POLICY.md §6 (BE-5) + §7.4`.
- Cluster 2 touch points: `MAINNET_CUSTODY_CLUSTER_2_RESOLUTION_REDACTED.md §5.1`.
- Predecessor milestone: `BACKEND_SHOULD_BROADCAST_ECONOMIC_GATE_RESULT.md`.
- Auditor anchors unlocked: Q-26 (key non-extractable from KMS), Q-27
  (§6.6 transaction policy precheck), Q-29 (mTLS authn between backend
  and signer service).
- Gap-list: closes **D-1** boundary; mainnet wire transport remains
  open under `MAINNET-KMS-VENDOR-SELECTION` + a follow-on adapter
  implementation track.

## 14. Next milestone recommendation

**Primary follow-on (operator-side, parallel):**
`MAINNET-KMS-VENDOR-SELECTION` — Q-CD-5 sub-decision; once a vendor is
chosen, the next backend impl track is `MAINNET-KMS-VENDOR-ADAPTER-IMPLEMENTATION`
that swaps `UnimplementedTransport` for the real HTTPS/2 + mTLS client
adapter for the selected vendor.

**Primary follow-on (backend-side, can run in parallel):**
`BACKEND-EXECUTOR-MONITORING-ALERTS-V1-WIRING` — plumb
`kms_request_id` into the existing monitoring alerts spec
(`BACKEND_EXECUTOR_MONITORING_ALERTS_V1.md §7.1`); add metrics for
`signer:<code>` reject-rate alerts; wire the launch-invariant verifier
into the operator-side debug route per `BACKEND_SHOULD_BROADCAST_ECONOMIC_GATE_RESULT.md §7`.
