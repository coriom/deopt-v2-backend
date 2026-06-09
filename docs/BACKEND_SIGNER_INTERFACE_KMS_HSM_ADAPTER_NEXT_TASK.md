# NEXT TASK — BACKEND-SIGNER-INTERFACE-KMS-HSM-ADAPTER

**Posture:** ready-to-run prompt for the next backend implementation
milestone. Hand this file verbatim to the backend implementer once
`MAINNET-BE-SIGNER-SERVICE-DESIGN` is signed and `MAINNET-KMS-VENDOR-SELECTION`
is unblocked (vendor selection is operator-side; the trait + mock
implementation can land before vendor selection completes).
**No mainnet broadcast. No chain mutation. No `.env` secret printed.
No KMS key creation.**

**Closes milestone:** `BACKEND-SIGNER-INTERFACE-KMS-HSM-ADAPTER` (custody
policy §7.4 + Cluster 2 §5.1; gap-list D-1).

---

## Prompt (begin)

---

Workspace root is `~/DEOPT`.

Execute `BACKEND-SIGNER-INTERFACE-KMS-HSM-ADAPTER` only.

### Current state

- `BACKEND-SHOULD-BROADCAST-ECONOMIC-GATE` is closed: `should_broadcast`
  gate active in the broadcast path; structured reject codes; launch
  invariant verifier; 36 tests; cargo + forge all green.
- `MAINNET-BE-SIGNER-SERVICE-DESIGN` is closed: design at
  `deopt-v2-backend/docs/MAINNET_BE_SIGNER_SERVICE_DESIGN.md` is the
  authoritative spec for this implementation milestone.
- Current backend signer: `src/execution/signer.rs::ExecutorSigner::from_private_key`
  consumes `EXECUTOR_PRIVATE_KEY` raw bytes. Mainnet REQUIRES this be
  replaced by a remote/KMS path (custody policy §6 principle BE-5).
- No `RemoteSigner` trait exists. Call sites at
  `src/options/service.rs:1166` and `:1213` (post-`should_broadcast` swap
  points) currently construct `ExecutorSigner` directly.
- No `BACKEND_SIGNER_ENDPOINT` env key. No mainnet startup refusal of
  `EXECUTOR_PRIVATE_KEY`.
- Sepolia rehearsal regression must remain green throughout — the
  `LocalDevSigner` retention is the gate that preserves it.

### Hard stops (this task)

```text
no mainnet broadcast                                                   ✅
no chain tx by the implementer                                         ✅
no Safe tx                                                             ✅
no `.env` edit by the implementer                                      ✅
no KMS / HSM / MPC key creation                                        ✅
no vendor account creation                                             ✅
no Treasury / InsuranceFund / Ownership / Timelock / Guardian mutation ✅
no rebate reserve allocation / PFV withdrawal                          ✅
no governance mutation                                                 ✅
no real KMS provider credentials in code / tests / fixtures            ✅
no `EXECUTOR_PRIVATE_KEY` value / RPC URL / DATABASE_URL / admin token printed in PR description or commit message ✅
no `--no-verify` git flags                                             ✅
no provider account / region / ARN / IAM role string in code or docs   ✅
no fallback path that resurrects mainnet local-key signing             ✅
```

If a step requires any of the above, STOP and document the blocker for
operator review.

### Goal

Land the `RemoteSigner` trait in `src/execution/`, retain
`LocalDevSigner` (wrapping the existing `ExecutorSigner::from_private_key`)
for Sepolia + unit tests, and add a `RemoteSignerClient` mock-backed by an
in-process server for integration tests. Refuse mainnet startup when an
`EXECUTOR_PRIVATE_KEY` is configured on `chain_id == 8453`.

### Required Phase A — read the design + source

1. Read end-to-end:
   - `deopt-v2-backend/docs/MAINNET_BE_SIGNER_SERVICE_DESIGN.md` (full).
   - `deopt-v2-backend/docs/SHOULD_BROADCAST_DESIGN_NOTE.md` (call-site).
   - `deopt-v2-backend/docs/BACKEND_SHOULD_BROADCAST_ECONOMIC_GATE_RESULT.md` (boundary mode).
   - `MAINNET_CUSTODY_POLICY.md §6 + §7 + §9` (principles).
   - `MAINNET_CUSTODY_CLUSTER_2_RESOLUTION_REDACTED.md §5.1` (touch points).
   - `BACKEND_EXECUTOR_MONITORING_ALERTS_V1.md §2.1 + §7.1` (audit logs).

2. Inspect (read-only) the current signer path:
   - `src/execution/signer.rs` — existing `ExecutorSigner`.
   - `src/execution/config.rs` — `ExecutionConfig` (line 27); existing
     `executor_private_key` field (line 33); existing `validate_startup`
     check at line 110-134.
   - `src/config/env.rs` — env loader; `EXECUTOR_PRIVATE_KEY` parsing
     (line 70-72) and `OPTION_EXECUTION_BROADCAST_ENABLED` (line 562).
   - `src/options/service.rs::broadcast_option_execution_intent_with_provider`
     — current `ExecutorSigner::from_private_key` call.
   - `src/options/broadcast_policy.rs` — `should_broadcast` decision +
     `BroadcastContext` fields.

3. Confirm gap (must be true at task kickoff):
   ```
   grep -rn 'trait RemoteSigner\|RemoteSignerClient\|BACKEND_SIGNER_ENDPOINT' deopt-v2-backend/src/
   ```
   MUST return 0 hits.

### Required Phase B — design note (short)

4. Append a short impl note to
   `deopt-v2-backend/docs/MAINNET_BE_SIGNER_SERVICE_DESIGN.md` §13 (new
   section) titled "implementation note (Phase 1)" — does NOT modify the
   existing sections. Records:
   - new module path: `src/execution/remote_signer.rs`.
   - new env keys: `BACKEND_SIGNER_ENDPOINT`, `BACKEND_SIGNER_MTLS_CLIENT_CERT_PATH`,
     `BACKEND_SIGNER_MTLS_CLIENT_KEY_PATH`, `BACKEND_SIGNER_MTLS_CA_BUNDLE_PATH`,
     `BACKEND_SIGNER_STALE_POLICY_MAX_AGE_MS`, `EXECUTOR_ALLOW_LOCAL_SIGNER`.
   - default values + whether each is required on mainnet.

### Required Phase C — implement (Sepolia-safe only)

5. Create `src/execution/remote_signer.rs` exposing:
   - `pub trait RemoteSigner` with the §6.1 surface from the design doc.
   - `pub struct LocalDevSigner` wrapping the existing
     `ExecutorSigner::from_private_key`. Implementation reuses the
     existing k256 path verbatim.
   - `pub struct RemoteSignerClient { endpoint: Url, identity: MtlsIdentity, ... }`
     with `sign_option_execution_tx` issuing an HTTPS/2 mTLS POST per the
     §3 request/response schema. The client is **wire-only**; no business
     decisions. Round-trip errors map to `SignerError` variants.
   - `pub enum SignerError { ... }` with one variant per §4.2 reject code
     plus `KmsUnavailable`, `KmsTimeout`, `Transport(String)`, `Internal`.
   - `pub struct SignerRequest<'a>` matching §3.1 schema (incl.
     `policy_decision_id`, `policy_fingerprint`).
   - `pub struct SignerResponse` matching §3.2 schema.

6. Add a `pub enum SignerBackend` config:
   ```
   SignerBackend::LocalDev    // Sepolia + tests; refused on mainnet
   SignerBackend::Remote      // mainnet; mTLS client of signer microservice
   ```
   Wire selection in `src/config/env.rs`:
   - If `BACKEND_SIGNER_ENDPOINT.is_some()` → `Remote`.
   - Else if `EXECUTOR_ALLOW_LOCAL_SIGNER == true` AND `chain_id ∈ {31337, 84532}`
     → `LocalDev`.
   - Else: configuration error (no signer configured).

7. Add a startup refusal: in `ExecutionConfig::validate_startup` add a
   guard that returns `BackendError::Config(...)` when
   `chain_id == 8453 AND executor_private_key.is_some()`. The error
   message MUST NOT print the key. Reuse the existing custody-policy §7.4
   inversion shape.

8. Add a startup precondition: when `chain_id == 8453`, REQUIRE
   `BACKEND_SIGNER_ENDPOINT` is set. Return a structured `Config` error
   otherwise. Same redaction rules.

9. Wire `RemoteSigner` at the two `service.rs` call sites that currently
   construct `ExecutorSigner` (post-`should_broadcast` swap points):
   `src/options/service.rs:1166` and `:1213`. Pass through the
   `policy_decision_id` + `policy_fingerprint` from the
   `ShouldBroadcastDecision` (Phase C-9a sub-step) — see Phase C-9a.

9a. Extend `broadcast_policy.rs` minimally: when `should_broadcast`
    returns `Approve`, the call site computes the fingerprint per design
    §5.1. This is implementation-side glue, not a policy change. Persist
    `policy_decision_id` against the intent (new column or in-memory
    correlation table — implementer's call; document choice in §13).

10. Compute the `policy_fingerprint` per design §5.1:
    ```
    keccak256( policy_decision_id ‖ calldata_hash ‖ nonce ‖ simulation_block ‖ deadline )
    ```
    Reuse existing `signing::eip712::keccak256`. Provide a `policy_fingerprint`
    helper fn alongside the existing selector_hex / now_ms helpers.

11. Preserve existing intent state-machine: `broadcast_failed` /
    `broadcast_submitted` transitions remain unchanged. Reject paths add
    new structured reason prefixes (`signer:<code>:<detail>`); approve
    path swaps the local k256 sign for `RemoteSigner::sign_option_execution_tx`.

### Required Phase D — tests

12. Unit tests in `src/execution/remote_signer.rs::tests`:
    - `local_dev_signer_round_trips_under_sepolia_chain_id` — `LocalDevSigner`
      derives the same address as the wrapped `ExecutorSigner`; signs a
      known prehash; signature recovers to that address.
    - `local_dev_signer_refused_on_mainnet_chain_id` — `SignerBackend`
      selection logic returns `Config` error.
    - `remote_signer_client_request_round_trips_with_mock_server` — spawn
      an in-process `axum` mTLS-disabled HTTPS server (test cert);
      pre-canned response; assert the response is deserialised correctly.
    - `policy_fingerprint_vector_test` — fixed inputs → fixed output;
      asserted byte-for-byte to lock the schema.
    - `signer_error_redaction_test` — `Debug` and `Display` of
      `SignerError` and `RemoteSignerClient` never print the env key, mTLS
      key path, or any 0x40-hex / 0x64-hex blob.
    - `signer_request_default_value_is_zero` — request builder always
      sets `transaction_value = 0`.
    - Allowlist deny matrix (signer-side simulation): N tests, one per
      §4.2 reject code, each exercised by a mock-server fixture that
      returns the corresponding denial response. The backend MUST map
      each into a structured `BroadcastFailed` with `signer:<code>:<detail>`.

13. Backend startup-guard tests in `src/execution/config.rs::tests`:
    - `mainnet_with_env_private_key_refuses_startup` — `chain_id = 8453 AND
      executor_private_key = Some(...)` → `Err(Config(...))`.
    - `mainnet_without_signer_endpoint_refuses_startup` — `chain_id = 8453 AND
      backend_signer_endpoint = None` → `Err(Config(...))`.
    - `sepolia_with_env_private_key_starts` — `chain_id = 84532 AND
      executor_private_key = Some(...)` → ok.
    - `sepolia_with_signer_endpoint_starts` — both env keys present →
      `Remote` selected; warn-once that the local key is ignored.

14. Integration tests in `src/options/service.rs::tests`:
    - `broadcast_uses_remote_signer_round_trip` — variant of
      `policy_approve_preserves_existing_broadcast_state_machine` using
      `RemoteSignerClient` against an in-process mock server.
    - `broadcast_signer_rejection_transitions_to_broadcast_failed` — mock
      server returns `policy-fingerprint`; backend transitions intent to
      `broadcast_failed` with `signer:policy-fingerprint:...`.
    - `broadcast_signer_kms_timeout_marks_intent_failed_no_local_fallback`
      — mock server returns 504; backend does NOT attempt
      `ExecutorSigner::from_private_key`; intent transitions to
      `broadcast_failed` with `signer:kms-timeout:...`.

15. Regression: confirm all existing Phase D tests from
    `BACKEND-SHOULD-BROADCAST-ECONOMIC-GATE` continue to pass
    (`policy_approve_preserves_existing_broadcast_state_machine`,
    `policy_reject_transitions_cleanly_without_half_state`, and the 31+
    `broadcast_policy::tests` unit tests).

16. Run validation locally:
    ```
    cargo fmt --all -- --check
    cargo clippy --all-targets --all-features -- -D warnings
    cargo test --all-targets --all-features --no-fail-fast
    ```
    All MUST be green.

17. Run forge validation (no sol source touched in this milestone):
    ```
    cd ~/DEOPT/deopt-v2-sol
    forge fmt --check
    forge build
    forge test --no-match-path 'test/fork/*'
    ```
    All MUST be green.

### Required Phase E — PR + close-out

18. Open PR titled `BACKEND-SIGNER-INTERFACE-KMS-HSM-ADAPTER — RemoteSigner trait + mainnet env-key refusal`.
19. PR description references:
    - `MAINNET_BE_SIGNER_SERVICE_DESIGN.md` (spec).
    - `MAINNET_CUSTODY_POLICY.md §6 + §7.4` (principle BE-5 inversion).
    - `MAINNET_CUSTODY_CLUSTER_2_RESOLUTION_REDACTED.md §5.1` (touch points).
    - This NEXT_TASK file (origin).
20. PR description includes a "what this PR does NOT do" section:
    - No mainnet broadcast attempted.
    - No `.env` edited.
    - No KMS / vendor / Treasury Safe creation.
    - No vendor account / region / ARN strings in code or docs.
    - No `EXECUTOR_PRIVATE_KEY` value printed.
    - No real KMS sandbox key used in tests (mock server only).
21. After review, merge to main (operator-authorised).

### Final report shape

Return final report grouped by:
- workspace
- docs / source inspected
- gap confirmation grep result (must be 0 hits at start)
- new module path
- new env keys list (no secret values)
- backend changes (file paths)
- tests added (count + categories)
- forge + cargo validation results
- PR title + URL
- files touched
- validations
- blockers
- next milestone recommendation

---

## Prompt (end)

---

## Notes for the operator handing this prompt off

- Expected effort: ~1-2 weeks impl + ~3-5 days test hardening + review.
- This is the **D-1** gap-list closure and the first concrete step toward
  V2G-Y phase Y-F (NEW_OME executor migration to mainnet BE).
- The mock-server-driven integration tests let this PR land BEFORE
  `MAINNET-KMS-VENDOR-SELECTION` completes — the vendor adapter slots into
  `RemoteSignerClient` via a thin adapter trait once the vendor is
  selected, without re-opening the boundary surface.
- After this task closes, the next recommended task is
  `MAINNET-KMS-VENDOR-ADAPTER-IMPLEMENTATION` (gated by Q-CD-5 +
  `MAINNET-KMS-VENDOR-SELECTION`; out-of-tree work + thin in-tree adapter
  glue). Or, in parallel, `BACKEND-EXECUTOR-MONITORING-ALERTS-V1-WIRING`
  to plumb `kms_request_id` into backend structured logs.

## Cross-links

- `deopt-v2-backend/docs/MAINNET_BE_SIGNER_SERVICE_DESIGN.md`
- `deopt-v2-backend/docs/SHOULD_BROADCAST_DESIGN_NOTE.md`
- `deopt-v2-backend/docs/BACKEND_SHOULD_BROADCAST_ECONOMIC_GATE_RESULT.md`
- `deopt-v2-backend/docs/BACKEND_SIGNER_CUTOVER_RUNBOOK_V2G_FX_Q1.md`
- `deopt-v2-backend/docs/BACKEND_EXECUTOR_MONITORING_ALERTS_V1.md`
- `deopt-v2-backend/docs/MAINNET_CUSTODY_CLUSTER_2_RESOLUTION_REDACTED.md`
- `~/DEOPT/MAINNET_CUSTODY_POLICY.md`
- `~/DEOPT/RUN_STATE.md`

**End of NEXT_TASK prompt stub.**
