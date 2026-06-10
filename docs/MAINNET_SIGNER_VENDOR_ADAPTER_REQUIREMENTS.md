# Mainnet signer vendor adapter requirements

**Posture:** DESIGN / DOC ONLY. No source code modified. No `.env` edited.
No vendor credentials. No KMS/HSM/MPC key created. No private custody
detail.

> **Addendum (2026-06-10, follow-on `BACKEND-KMS-VENDOR-ADAPTER-IMPLEMENTATION-PLUGGABLE`):**
> the pluggable adapter shape (Provider trait + mock + error mapping +
> 22-test pin) shipped at `src/execution/signer_adapters.rs`.
> `RemoteSignerClient::new` continues to use `UnimplementedTransport`
> as its production default; the pluggable transport ships behind
> `RemoteSignerClient::with_transport`. Mainnet `BACKEND_REMOTE_SIGNER_PROVIDER=mock`
> hard-refused at `ExecutionConfig::validate_signer_backend`. See
> `docs/BACKEND_KMS_VENDOR_ADAPTER_IMPLEMENTATION_PLUGGABLE_RESULT.md`.
> Vendor-specific implementation (§3.1 of the next-task doc) remains
> the next step once `MAINNET-KMS-VENDOR-SELECTION` resolves.
**Closes milestone (in part):** `MAINNET-SIGNER-VENDOR-AND-REHEARSAL-PACK`.
**Anchors:**
- `MAINNET_BE_SIGNER_SERVICE_DESIGN.md §3 + §4 + §6.1` — request/response
  schema + reject taxonomy + RemoteSigner trait.
- `BACKEND_SIGNER_INTERFACE_KMS_HSM_ADAPTER_RESULT.md` — current Rust
  surface (`RemoteSigner` / `LocalDevSigner` / `RemoteSignerClient` /
  `SignerTransport` / `UnimplementedTransport` / `SignerError::code()`).
- `BACKEND_EXECUTOR_MONITORING_ALERTS_V1.md §2.1 + §3.1` — signer
  metrics + alert thresholds.
- `EXECUTOR_HEALTH_ENDPOINT_V2_RESULT.md` — `signer` block fields
  surfaced on `/executor/health/v2`.
- `BACKEND_OBSERVABILITY_LAST_SINGLETON_FIELDS_RESULT.md` —
  `last_signer_error_code` taxonomy.

## 0. Hard rules (this doc)

```text
no vendor account creation              ✅
no real key creation                     ✅
no credentials in tracked docs           ✅
no fallback to local-key on mainnet      ✅
no remote signer call before policy      ✅
no broadcast                             ✅
no .env edit                             ✅
no DB schema change                      ✅
```

## 1. Scope

This doc specifies the **backend adapter** behind the chosen vendor —
the Rust code that the `RemoteSignerClient`'s `SignerTransport` impl
will route requests through. It does NOT specify the signer
microservice runtime topology (that lives in
`MAINNET_BE_SIGNER_SERVICE_DESIGN.md`) and it does NOT pick the
vendor (`MAINNET_SIGNER_VENDOR_SELECTION_MATRIX.md`).

## 2. Required adapter behavior

Each behavior maps to a concrete trait method or contract on the new
adapter type.

### 2.1 `sign_transaction`

* Receives: a [`SignerRequest`](../src/execution/remote_signer.rs:88)
  carrying intent_id, source_type, chain_id, target contract,
  function selector, calldata_hash, calldata_length, transaction_to,
  transaction_value_wei (must be 0), gas_limit, max_fee_per_gas_wei,
  max_priority_fee_per_gas_wei, nonce, simulation_block, deadline_ms,
  policy_decision_id, policy_fingerprint, policy_decision_at_ms, and
  the EIP-1559 prehash (keccak256(0x02 ‖ payload)).
* Computes a vendor-specific request payload and SUBMITS it via the
  vendor SDK / HTTPS.
* Returns: [`SignerResponse`](../src/execution/remote_signer.rs:113)
  carrying signer_address, RecoverableSignature (r, s, y_parity), and
  the vendor's per-request audit identifier (`kms_request_id` /
  `audit_log_id` / `remote_signer_request_id`).

### 2.2 `sign_digest`

* Required only if the vendor's typed-data flow is used.
* Default: deferred — the backend already pre-computes the prehash and
  needs only raw secp256k1 signing.

### 2.3 `derive_address`

* Adapter MUST be able to derive the EVM address from the vendor's
  public-key blob without a sign call.
* Used by the startup health-check.

### 2.4 `health_check`

* Required by [`RemoteSigner::health_check`](../src/execution/remote_signer.rs:238).
* MUST verify, at least: (a) the vendor endpoint is reachable, (b)
  the configured key id maps to a public key, (c) the recovered
  EVM address matches the configured `executor_from_address`. MUST
  NOT trigger a sign operation.

### 2.5 `request_id` correlation

* Adapter assigns a Rust-side `Uuid::new_v4()` `request_id` per
  sign call.
* MUST log `request_id` at INFO with `target: "broadcast_signer"`.
* MUST NOT log the prehash or the signature components at INFO.
  DEBUG-level with strict policy is acceptable.

### 2.6 `policy_decision_id` correlation

* Adapter MUST pass `request.policy_decision_id` to the vendor as
  request metadata where supported.
* On response, adapter MUST persist the round-trip in the
  `BroadcastObservability` snapshot via the existing
  `record_signer_attempt` / `record_signer_success` /
  `record_signer_denied` hooks (no new hook required).

### 2.7 `policy_fingerprint` binding

* `request.policy_fingerprint` is the keccak256 binding of
  (policy_decision_id ‖ calldata_hash ‖ nonce ‖ simulation_block ‖
  deadline_ms).
* Adapter MUST forward this binding to the vendor as opaque request
  metadata. Mismatch detection happens in the signer microservice
  (not in the adapter); the adapter MUST surface
  `vendor_unknown` when the vendor returns an unrecognised error.

### 2.8 `calldata_hash` binding

* `request.calldata_hash = keccak256(request.calldata)`.
* Adapter MUST NOT log the raw calldata bytes; it MAY log
  `calldata_hash` + `calldata_length`.

### 2.9 `audit_log_id` correlation

* The vendor's per-request audit log id (e.g. AWS CloudTrail
  `RequestId`, GCP `request_id`, Azure `x-ms-request-id`) MUST land in
  the response's `kms_request_id` field.
* If the vendor surfaces a separate audit log id distinct from the
  request id, both MUST land (`kms_request_id` + `audit_log_id`).

### 2.10 Timeout behavior

* Adapter MUST honor a configurable per-request timeout, default
  `2500 ms`.
* On timeout, MUST return `SignerError::KmsTimeout` (code
  `kms-timeout`). MUST NOT retry locally.

### 2.11 Retry behavior

* The adapter does NOT retry sign requests. The broadcast pipeline
  treats a timeout as a hard failure (custody §6.7 + design doc §4.4
  rationale: re-signing risks duplicate intent submission with a
  different nonce).
* The adapter MAY transparently retry `derive_address` /
  `health_check` calls (read-only, idempotent).

### 2.12 Denial mapping

* Vendor explicit denial (policy engine reject / IAM deny / key
  disabled) maps to `SignerError::KmsUnavailable` (code
  `kms-unavailable`) when the vendor signals an inherent unavailability,
  or to `SignerError::ConfigRefusal(reason)` (code `config-refusal`)
  when the denial is operator-config in nature.

### 2.13 Transport failure mapping

* TCP/TLS / HTTP 5xx / DNS / connection-refused / abrupt disconnects
  → `SignerError::Transport(short_reason)` (code `transport`).
* Reason string MUST be redacted at the call site (no URL, no JWT, no
  request body) — only structural categories ("connection refused",
  "tls handshake failed", "http 503").

### 2.14 Malformed-signature handling

* Adapter MUST reject responses where the recovered EVM address does
  NOT match the configured `expected_address` (the
  `executor_from_address`). MUST return `SignerError::PostSignFromMismatch`
  (code `post-sign-from-mismatch`).
* Adapter MUST reject responses where `r`, `s`, or `y_parity` are
  structurally invalid (`r >= secp256k1.n` or `s >= secp256k1.n/2` —
  EIP-2 low-s — or `y_parity != 0 && y_parity != 1`). Mapped to
  `SignerError::Internal("malformed-signature")` (code `internal`)
  since this indicates vendor misbehavior, not a policy decision.

### 2.15 Signer-address mismatch

* See §2.14 — the `PostSignFromMismatch` mapping is the canonical
  surface.
* The `LocalDevSigner` runtime guard at
  `src/execution/remote_signer.rs:283` continues to refuse mainnet
  chain_id 8453 unconditionally — vendor adapter MUST be Remote
  variant only.

### 2.16 No fallback to local signer on mainnet

* The adapter is the sole transport for the Remote variant. On any
  failure mode in §2.10-2.14, the broadcast pipeline MUST surface the
  error and HALT — no local-key signing is permitted on mainnet under
  any circumstance.
* `ExecutionConfig::validate_signer_backend` (`src/execution/config.rs:197`)
  is the canonical startup guard; `build_signer_for_state`
  (`src/options/service.rs:1465`) is the runtime guard; both MUST
  remain intact when the adapter is wired in.

## 3. Error taxonomy

The adapter projects vendor-specific errors onto the bounded set
below. These extend the existing
`SignerError::code()` taxonomy
(`src/execution/remote_signer.rs:155-179`); no new variant is required
for vendor-specific codes — existing variants cover every vendor
category.

| Adapter code | Existing `SignerError` variant | Existing `code()` string | Trigger |
|---|---|---|---|
| `vendor_denied` | `KmsUnavailable` | `kms-unavailable` | Vendor policy engine reject; key disabled; IAM deny. |
| `vendor_timeout` | `KmsTimeout` | `kms-timeout` | Vendor SDK / HTTPS timeout exceeded. |
| `vendor_unavailable` | `Transport("…")` | `transport` | Vendor returns HTTP 5xx or connection failure. |
| `vendor_auth_failed` | `CallerUnauthorized` | `caller-unauthorized` | Vendor auth token / mTLS cert rejected. |
| `vendor_rate_limited` | `RateLimit` | `rate-limit` | Vendor 429 / quota exhausted. |
| `vendor_malformed_signature` | `Internal("malformed-signature")` | `internal` | Recovered signature components fail structural validation. |
| `vendor_address_mismatch` | `PostSignFromMismatch` | `post-sign-from-mismatch` | Recovered EVM address != `expected_address`. |
| `vendor_unknown` | `Internal(short)` | `internal` | Any vendor error not in the above categories. |

The mapping is **stable**: future adapter changes MUST preserve the
existing `SignerError::code()` taxonomy so the
`policy_rejected_total{code, source_type}` /
`signer_denied_total{code, signer_kind}` Prometheus labels remain
backward-compatible.

## 4. Observability

### 4.1 Metrics (already wired)

The existing observability surface in
`src/options/broadcast_observability.rs` covers every adapter event:

* `signer_attempted_total{signer_kind="remote"}` — bumped before the
  sign call.
* `signer_success_total{signer_kind="remote"}` — bumped on Ok.
* `signer_denied_total{code, signer_kind="remote"}` — bumped on Err;
  `code` is the bounded `SignerError::code()` string.
* `last_signer_error_code` (singleton) — most-recent denial code.
* `last_signer_kind` (singleton) — most-recent signer kind.
* `last_broadcast_submitted_ms` (singleton) — most-recent successful
  sign timestamp.
* `local_signer_on_mainnet_refused_total` — defence-in-depth counter;
  MUST remain 0 on a healthy mainnet runtime.

No new counters required. The adapter MUST NOT introduce new label
keys; the `signer_kind` whitelist is fixed at `{"local_dev", "remote"}`
and the `code` whitelist is fixed by `SignerError::code()`.

### 4.2 Latency

The adapter MAY emit per-request latency via `tracing` events at INFO
with `target: "broadcast_signer.latency"`. Prometheus latency
histograms are deferred to
`BACKEND-OBSERVABILITY-PROMETHEUS-FOR-HEALTH-V2-SINGLETONS`. The
existing snapshot field `last_broadcast_submitted_ms` lets operators
derive a coarse "time since last success".

### 4.3 Health endpoint fields

`/executor/health/v2` already surfaces every signer-relevant field
(per `EXECUTOR_HEALTH_ENDPOINT_V2_RESULT.md §3` and follow-on
singleton milestones):

* `signer.signer_mode` — `"local_dev"` | `"remote"`.
* `signer.remote_signer_configured` — bool (endpoint configured).
* `signer.signer_address` — configured executor address.
* `signer.last_signer_kind` — most-recent kind.
* `signer.last_signer_success_at_ms` — most-recent success ms.
* `signer.last_signer_error_code` — bounded code or null.
* `signer.local_signer_on_mainnet_refused_total` — defence-in-depth.

No new fields required. The adapter MUST NOT add free-form strings to
this block.

### 4.4 Signer request id

* Surfaced via `SignerResponse.remote_signer_request_id`.
* The broadcast call site at `src/options/service.rs:1438-1448` already
  logs it under `target: "broadcast_signer"` at INFO.

### 4.5 Audit log id

* Surfaced via `SignerResponse.kms_request_id` and
  `SignerResponse.audit_log_id`.
* Logged at INFO by the same call site.

## 5. Redaction requirements

The adapter MUST NEVER log or surface:

* Raw RPC URLs (`https://…/some-provider-key`).
* Vendor API keys, JWT tokens, mTLS private keys, SSH keys.
* Provider raw auth response bodies.
* `DATABASE_URL`.
* Admin tokens.
* Webhook secrets.
* Private custody roster details, signer human identities, or
  jurisdictional residency markers.
* The raw private key material (the whole point: it doesn't leave the
  provider).
* The EIP-1559 prehash or the assembled raw transaction at INFO. At
  DEBUG, only with explicit operator override.

`SignerError::Transport(reason)` / `SignerError::Internal(reason)` /
`SignerError::ConfigRefusal(reason)` carry short caller-provided
reason strings — the adapter MUST sanitize these to structural
categories only (no provider response bodies pasted into the reason).

## 6. Required tests for implementation

The implementation milestone MUST land at least the following test
classes. None require a live vendor; all use a `SignerTransport`
mock injected via `RemoteSignerClient::with_transport`
(`src/execution/remote_signer.rs:364`).

| Test | Pin |
|---|---|
| `adapter_round_trip_ok_returns_signer_response` | Happy-path: prehash → r/s/y_parity → recovered address matches expected → response Ok. |
| `adapter_returns_address_mismatch_when_recovery_diverges` | Mocked signature recovers to a DIFFERENT address → `PostSignFromMismatch`. |
| `adapter_returns_kms_timeout_on_transport_timeout` | Mock returns a timeout future → `KmsTimeout`. Adapter MUST NOT retry locally. |
| `adapter_returns_transport_on_http_5xx` | Mock returns HTTP 503 / connection refused → `Transport(…)`. |
| `adapter_returns_caller_unauthorized_on_403` | Mock returns auth failure → `CallerUnauthorized`. |
| `adapter_returns_rate_limit_on_429` | Mock returns 429 → `RateLimit`. |
| `adapter_returns_kms_unavailable_on_policy_deny` | Mock returns explicit vendor policy reject → `KmsUnavailable`. |
| `adapter_returns_internal_on_malformed_signature` | Mock returns r ≥ secp256k1.n (or invalid y_parity) → `Internal("malformed-signature")`. |
| `adapter_mainnet_with_localdev_kind_panics` | `RemoteSignerClient::with_transport` constructed on chain_id 8453 with kind `LocalDev` → runtime refusal (already pinned by existing tests; this milestone re-asserts). |
| `health_check_no_op_when_endpoint_unreachable` | `health_check` returns Err but does NOT trigger a sign call. |
| `health_check_passes_when_endpoint_returns_expected_address` | Mock returns the configured address → Ok. |
| `health_check_fails_when_endpoint_returns_different_address` | Mock returns a different address → Err. |
| `observability_signer_attempt_then_success_increments_both_counters` | End-to-end via the broadcast call site mock — pre-existing test pattern. |
| `observability_signer_attempt_then_denial_increments_denied_only` | End-to-end — pre-existing test pattern. |
| `adapter_does_not_log_prehash_or_signature_at_info` | Capture tracing output at INFO → confirm absence. |
| `adapter_redacts_transport_error_reason` | Mock returns a URL-shaped error body → `Transport(reason)` short string, no URL. |

## 7. Implementation deliverables (next milestone)

Per `BACKEND_KMS_VENDOR_ADAPTER_IMPLEMENTATION_NEXT_TASK.md`:

* New module `src/execution/signer_adapters/{vendor}.rs` (or
  `vendor_agnostic.rs` if a vendor is not yet selected).
* New `SignerTransport` impl wired into `RemoteSignerClient::with_transport`.
* Constructor reads vendor configuration from `ExecutionConfig`
  (existing `backend_signer_endpoint` field is the URL; vendor SDK
  / API key is read from environment via the typed config loader —
  NOT directly via `std::env::var`).
* Tests from §6.
* No new env var allowed unless the brief explicitly authorises it.
* No `.env` edit.

## 8. Cross-links

* `MAINNET_SIGNER_VENDOR_SELECTION_MATRIX.md` — vendor decision input.
* `MAINNET_SIGNER_STAGING_REHEARSAL_PLAN.md` — rehearsal plan that the
  implementation MUST pass through.
* `MAINNET_SIGNER_ROTATION_AND_INCIDENT_RUNBOOK.md` — rotation +
  incident response readiness.
* `BACKEND_KMS_VENDOR_ADAPTER_IMPLEMENTATION_NEXT_TASK.md` — the
  copy-paste prompt for the implementation milestone.
* `MAINNET_BE_SIGNER_SERVICE_DESIGN.md` — signer microservice topology
  this adapter calls.
* `MAINNET_CUSTODY_POLICY.md §6.7 + §7.4` — custody rules.
* `EXECUTOR_HEALTH_ENDPOINT_V2_RESULT.md` — health surface this adapter
  populates.
* `BACKEND_EXECUTOR_MONITORING_ALERTS_V1.md` — alert thresholds.
