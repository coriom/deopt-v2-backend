# BACKEND-HYBRID-V2-EXTERNAL-SIGNER-INTEGRATION-AND-LIVE-ORCHESTRATOR-V1 — Security Review

Milestone status: **Pre-broadcast surface only. Broadcast remains disabled by construction.**

External signer verdict: **`EXTERNAL_SIGNER_INTEGRATED_PRE_BROADCAST_SAFE`**.

Final verdict: **`EXTERNAL_SIGNER_SECURITY_VALIDATED`** (see closing section).

## 1. Scope and threat model

This milestone extends the pre-broadcast execution pipeline with the
integration surface for the external signer microservice (Pattern C
per `MAINNET_BE_SIGNER_SERVICE_DESIGN.md`) and a live
`ExecutionOrchestrator` wired into `AppState` so the admin
`prepare_execution` route drives a real end-to-end flow through the
bridge, the `SignerPolicyFirewall`, and the on-chain signature
verifier — landing at the terminal `BROADCAST_DISABLED` phase.

Broadcast remains **disabled** in this milestone. The
`ExecutionRpcClient` trait has no `send_*` method (compile-time
firewall); the runtime allowlist in
`src/hybrid_v2/execution/rpc.rs::ALLOWED_METHODS` and the runtime
source-scans in
`tests/hybrid_v2_execution_zero_broadcast_scan.rs` +
`tests/hybrid_v2_external_signer_no_broadcast_scan.rs` are
belt-and-braces defences on top of that.

The threat model covers:

- an operator/API caller with valid admin credentials trying to
  influence target/calldata/gas/nonce/chain_id from the request body;
- a **compromised signer microservice** returning tampered `(r, s, v)`,
  a valid signature over a different plan, a different recovered
  address, a signature bound to a different chain id, or an unavailable
  / rate-limited / auth-failed / policy-refusal error class;
- an attacker with network position between backend and signer trying
  to impersonate the signer via TLS downgrade or a rogue certificate;
- a stale simulation reused after a chain-state change (unchanged from
  V1 — restated here because the signer input carries the derived
  simulation state);
- an attacker with write access to the projection PostgreSQL trying
  to mutate the persisted `signer_request_idempotency_key` column;
- concurrent operator requests racing on the same
  `canonical_execution_id`, potentially producing two idempotency
  keys.

## 2. Trust boundary

The signer microservice is a **separate process** owned by the
platform team, not the backend team. The backend NEVER holds the
signer's private key — the backend authenticates to the signer via
IAM role / mTLS (Pattern C) and receives back a decomposed
`(r, s, v)` triple + the vendor's claim of the recovered signer
address.

**No raw private key material exists in the backend.** Search
receipts:

- No `PrivateKey`, `PrivateKeySecret`, or `SigningKey` field on
  `HybridV2KmsSignerBridge`: see
  `src/hybrid_v2/execution/signer_kms_bridge.rs:79-86`.
- No `PrivateKey` field on `HybridV2ExecutionConfig`: see
  `src/hybrid_v2/config.rs:743-797`.
- The only in-process signer is `TestEphemeralSigner`, gated behind
  `#[cfg(any(test, feature = "test-signer"))]`: see
  `src/hybrid_v2/execution/signer_ephemeral.rs` and the compile-time
  refusal in
  `src/hybrid_v2/execution/signer_builder.rs:60-72`.
- The AWS-KMS transport gate returns `SignerUnavailable` when the
  `aws-kms-transport` feature is disabled at build time: see
  `src/hybrid_v2/execution/signer_builder.rs:164-170`.

## 3. Threat / mitigation matrix

### 3.1 Signer auth (Pattern C)

The backend attaches its IAM role (or equivalent auth material) via
the signer microservice's own transport. The auth material handle is
`signer_auth_reference` on `HybridV2ExecutionConfig` — a **reference**
to auth material stored elsewhere, never the auth material itself.
The `Debug` impl on `HybridV2ExecutionConfig` collapses the field to
`<redacted>`: see `src/hybrid_v2/config.rs:840-843` and the
`debug_impl_redacts_endpoint_kms_key_id_and_auth_reference` unit test
at `src/hybrid_v2/config.rs:1220-1233`.

### 3.2 Signer authz (KMS key policy)

Authorization lives inside the signer microservice — the backend
cannot bypass a `kms:Sign` deny. The `HV2_SIGNER_KMS_KEY_ID`
identifies which key the signer should use; validation of
key-policy-side allowlists is the signer's responsibility. Backend
enforces the `expected_signer_address` cross-check at the bridge
boundary so a compromised signer that produces a signature under a
different key (recovering to a different address) fails closed at
`HybridV2KmsSignerBridge::sign_execution`:
`src/hybrid_v2/execution/signer_kms_bridge.rs:244-258`.

### 3.3 Backend compromise blast radius

An attacker with full backend RCE:

- **Cannot** recover a private key from the backend (there is none —
  see §2 receipts).
- **Cannot** issue a broadcast RPC via the execution module (the
  `ExecutionRpcClient` trait exposes only the 7 read methods; there
  is no `send_*` method — see the runtime allowlist assertion in
  `tests/hybrid_v2_external_signer_no_broadcast_scan.rs:
  http_execution_rpc_client_allowlist_is_frozen_at_seven_read_methods`).
- **Can** replay a signed request to the signer up to
  `signer_max_retries + 1` times (bounded to 6 in
  `HybridV2ExecutionConfig::validate_startup`
  `src/hybrid_v2/config.rs:1017-1022`; the bridge additionally
  clamps to 5 in `HybridV2KmsSignerBridge::new`
  `src/hybrid_v2/execution/signer_kms_bridge.rs:117`).
- **Cannot** cause the signer to sign a different plan without also
  producing a matching `plan_hash`, `calldata_hash`, and
  `signing_payload_hash` — the signer's own policy would refuse the
  request whose fingerprint is not on the operator's allowlist.

### 3.4 Signer compromise blast radius

A fully-compromised signer that returns arbitrary signatures:

- Cannot cause the backend to broadcast (broadcast disabled by
  construction).
- Cannot cause the pipeline to accept a wrong-signer signature — the
  bridge cross-checks the recovered address against
  `expected_signer_address`
  (`signer_kms_bridge.rs:253-258`); if the checks pass at the bridge
  boundary, the orchestrator RE-VERIFIES via `verify_signed_tx`
  (`orchestrator.rs` STEP 4, see also
  `src/hybrid_v2/execution/signature_verify.rs`). Both must succeed
  for the row to reach `BROADCAST_DISABLED`.
- Cannot corrupt the persisted plan: SQL trigger on migration 0049
  (referenced from V1 security review §3.5) refuses UPDATEs to
  `plan_hash` / `calldata_hash` on any row that has advanced past
  the plan-persisted state. The Package A migration 0050 adds the
  same immutability trigger to `signer_request_idempotency_key`
  (`migrations/0050_hybrid_v2_signer_idempotency.sql`).

### 3.5 Replay defence (idempotency)

Every persisted execution row carries a 16-byte
`signer_request_idempotency_key` derived from
`keccak256("HV2_SIGNER_IDEMPOTENCY_V1" || expected_signer_address ||
canonical_execution_id || plan_hash || signing_payload_hash)[..16]`
(`signer_kms_bridge.rs:303-319`). Domain separation via the
tag byte string prevents cross-milestone key reuse; the SQL trigger
on migration 0050 refuses any UPDATE that would mutate the column;
and the property test
`prop_idempotency_key_differs_on_any_field_change` (see
`tests/hybrid_v2_external_signer_properties.rs`) proves the
derivation is sensitive to every field.

The signer microservice itself may honour request idempotency using
the same key (surfaced via `PerpsSignerRequest.policy_fingerprint`,
see `signer_kms_bridge.rs:234`). A retried request from the backend
converges on the same signature on the signer side.

### 3.6 Cross-chain replay

`SigningRequest.chain_id` is bound into the orchestrator's derived
`signing_payload_hash` (unchanged from V1). The bridge additionally
refuses `chain_id == 8453` (Base mainnet) at its own boundary:
`signer_kms_bridge.rs:179-181`. Property test
`prop_bridge_never_calls_vendor_on_mainnet_chain_id` covers.

### 3.7 Cross-deployment replay

`derive_canonical_execution_id` incorporates `deployment_id` (see
V1 security review §3.3). A second deployment on the same chain
produces a distinct canonical id and therefore a distinct persisted
row (test:
`hybrid_v2_external_signer_full_matrix_pg_integration::
deployment_isolation_via_bridge`).

### 3.8 Target / calldata / nonce / gas / fee substitution

The `SignerPolicyFirewall` re-derives + re-validates every one of
these at the boundary immediately upstream of the bridge. The bridge
itself has no knowledge of the manifest; it forwards the pre-
validated `SigningRequest` to the signer microservice, and the signer
microservice's own policy independently re-checks target, selector,
chain, nonce, gas cap, value, and calldata length. Any tamper is
caught at BOTH layers.

### 3.9 Response tampering

The signer may return:

- A signature over a **different prehash** — caught by
  `verify_signed_tx` in the orchestrator (STEP 4), which re-derives
  the signing payload locally and recovers the address against the
  returned `(r, s, v)`. Test:
  `hybrid_v2_external_signer_full_matrix_pg_integration::
  wrong_plan_signature_binding_rejected`.
- A signature with a **different signer address string** — caught by
  the bridge address cross-check at
  `signer_kms_bridge.rs:253-258`. Test:
  `hybrid_v2_external_signer_full_matrix_pg_integration::
  wrong_signer_response_terminates_row_as_failed`.
- A **structurally-invalid signature** (`y_parity ∉ {0, 1}`) — caught
  at the bridge boundary by `extract_signed`
  (`signer_kms_bridge.rs:321-336`) and mapped to
  `SignerError::MalformedResponse`. Property test:
  `prop_malformed_y_parity_always_rejected`.

### 3.10 TLS / impersonation

Signer endpoint scheme is enforced by
`HybridV2ExecutionConfig::validate_startup`:
`https://` mandatory, or `http://127.0.0.1` / `http://localhost` for
local dev only. Public `http://` refused
(`src/hybrid_v2/config.rs:1038-1050`). Property test:
`prop_config_validate_refuses_non_https_endpoint`.

### 3.11 Endpoint config injection

`HV2_SIGNER_ENDPOINT` is read once at startup via
`HybridV2ExecutionConfig::from_env`. There is no admin route that
mutates it at runtime — a caller with admin token cannot swap the
signer endpoint. The `Debug` impl redacts the endpoint URL to
`<host:port>` via `redact_rpc_url`
(`src/hybrid_v2/config.rs:812-815`). Property test:
`prop_endpoint_redaction_never_leaks_path`.

### 3.12 Request / signature logging

- The bridge's `Debug` impl redacts `endpoint_uri_redacted` (already
  redacted upstream) and NEVER prints the returned `(r, s, v)`:
  `signer_kms_bridge.rs:88-100`.
- The Perps `RemoteSignerClient` logs only structural
  metadata; the raw signature bytes never appear in a log line.
- Error classification via `classify_error` truncates every
  vendor-returned reason string to 80 chars
  (`signer_kms_bridge.rs:418-425`) — a rogue signer that echoes back
  an attacker-controlled long payload cannot flood the operator log.

### 3.13 Outage handling

- **Signer outage** (vendor timeout / 5xx / rate limit): retryable
  class — bridge retries up to `max_retries + 1` times. On budget
  exhaustion, orchestrator lands `SIGNER_UNAVAILABLE`. Read side
  remains healthy. Test:
  `signer_outage_read_api_still_healthy`.
- **Auth failure**: NOT retryable — one attempt, terminal
  `SIGNER_UNAVAILABLE`. Test:
  `auth_failure_not_retried_lands_signer_unavailable`.
- **Deterministic refusal** (KMS key disabled, policy fingerprint,
  vendor policy rejection): NOT retryable. Test:
  `deterministic_rejection_not_retried_even_with_budget`.
- **PG outage**: pool-level failure surfaces as
  `OrchestrationError::StoreFailure` at the prepare/resume entry
  point. Test: `pg_outage_surfaces_store_failure_on_prepare`.

### 3.14 Public-signing / broadcast / chain-write action

There is NO public route or admin route that produces a broadcast.
There is NO public route or admin route that submits arbitrary bytes
to the signer. The admin `prepare_execution` route is the ONLY
signer-adjacent surface (`src/api/hybrid_v2_execution_admin.rs`), it
is behind `x-admin-token`, refuses mainnet at handler entry, and
its request body is `#[serde(deny_unknown_fields)]` — a caller
cannot introduce a `target` / `calldata` / `nonce` / `chain_id`
field. Every downstream execution field is derived deterministically
from the manifest allowlist + the plan builder.

## 4. Evidence file:line for each frozen invariant

| Invariant | Evidence |
|---|---|
| **No raw private key in production code** | `src/hybrid_v2/execution/signer_kms_bridge.rs:79-86` — the bridge struct has no key field. `src/hybrid_v2/execution/signer_builder.rs:60-72` — the TestEphemeral variant is refused outside `#[cfg(test)]`/`test-signer`. |
| **No mnemonic** | Grep receipt: `rg -n "mnemonic|MnemonicSigner" src/hybrid_v2/` returns zero matches. |
| **No signer secret in config** | `src/hybrid_v2/config.rs:794-796` — `signer_auth_reference` is documented as "NEVER a raw secret". `Debug` collapses to `<redacted>` at `src/hybrid_v2/config.rs:840-843`. |
| **No arbitrary signing endpoint** | `src/api/hybrid_v2_execution_admin.rs:170-193` — `PrepareRequestBody` uses `#[serde(deny_unknown_fields)]`; there is no `signer_endpoint` field. |
| **No broadcast in module** | `src/hybrid_v2/execution/rpc.rs:368-376` — `ALLOWED_METHODS` has exactly seven read methods. `src/hybrid_v2/execution/rpc.rs::check_method` refuses any other. |
| **No broadcast in bridge/config/admin** | `tests/hybrid_v2_external_signer_no_broadcast_scan.rs` scans `signer_kms_bridge.rs`, `signer_builder.rs`, `config.rs`, `hybrid_v2_execution_admin.rs`, `signer.rs`, `orchestrator.rs` for forbidden verbs and fails loud on any match. |
| **No Base mainnet** | `src/hybrid_v2/config.rs:999-1004` — chain-side check refuses `chain_id == BASE_MAINNET_CHAIN_ID`. `src/hybrid_v2/execution/signer_kms_bridge.rs:179-181` — bridge boundary refuses `chain_id == 8453`. `src/api/hybrid_v2_execution_admin.rs` — `refuse_mainnet` at handler entry. |
| **No public signer route** | Every route in `src/api/hybrid_v2_execution_admin.rs` runs `ensure_admin` first (line references at handler bodies). No route is mounted under a public prefix. |

## 5. Change log vs V1

V1 landed the signer INTERFACE + a `ProductionSignerUnavailable`
default. Package A of this milestone landed the bridge + live
orchestrator wiring behind the same fail-closed defaults. Package B
completes the audit surface: zero-broadcast reaffirmation across the
new files, external-signer test harness, full-matrix PG suite,
bounded properties, performance bounds, this security review, CI
gate, and closure documentation.

No new frozen invariants are introduced; every V1 invariant remains
in effect and is reaffirmed by the new test suites listed above.

## 6. Closing verdict

`EXTERNAL_SIGNER_SECURITY_VALIDATED`.

The external-signer integration surface has been reviewed against
Pattern C, its threat model, and the frozen safety posture. Every
mitigation surface has a corresponding test (unit / PG matrix /
property) and a file:line receipt. The pre-broadcast pipeline
remains free of chain-write capability by construction, and every
signer-adjacent artifact (bridge, config, admin route, orchestrator)
has been source-scanned for forbidden verbs and structurally proven
not to leak signer secrets or produce a broadcast.
