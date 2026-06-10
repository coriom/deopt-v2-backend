# BACKEND-KMS-VENDOR-ADAPTER-IMPLEMENTATION-AWS-KMS — result

**Posture:** SHIPPED at 2026-06-10.

> **Addendum (2026-06-10, follow-on `BACKEND-AWS-KMS-PRODUCTION-TRANSPORT`):**
> the real `aws-sdk-kms`-backed transport
> (`AwsKmsSdkTransport`) shipped behind the `aws-kms-transport` Cargo
> feature flag at `src/execution/signer_adapters/aws_kms_sdk.rs`.
> Default builds remain dependency-light; AWS SDK only pulled when
> the feature is enabled. 11 new feature-gated tests; no live AWS
> calls anywhere. Production `RemoteSignerClient::new` STILL uses
> `UnimplementedTransport`. See
> `docs/BACKEND_AWS_KMS_PRODUCTION_TRANSPORT_RESULT.md`.

## 1. Goal

Implement the AWS KMS vendor-specific `PluggableSignerProvider`
behind a mockable `AwsKmsTransport`. **No real AWS account. No real
KMS keys. No real credentials. No AWS SDK dependency added by this
milestone.** Tests cover the full pipeline (SPKI parse → EVM-address
derive → DER decode → y_parity recovery → low-s validation → error
mapping → health_check) against a deterministic in-process mock.

## 2. Files changed

* `src/execution/signer_adapters.rs` — declares `pub mod aws_kms;` at
  the top of the module body. Existing pluggable code unchanged.
* `src/execution/signer_adapters/aws_kms.rs` — **NEW** (~1100 LoC + 27
  tests). The vendor-specific adapter + mock transport.
* `src/execution/config.rs` — new `ExecutionConfig.backend_signer_timeout_ms: u32`
  field (default 2500 ms; range 100..=30000) per
  `MAINNET_SIGNER_VENDOR_ADAPTER_REQUIREMENTS.md §2.10`.
* `src/config/env.rs` — `BACKEND_SIGNER_TIMEOUT_MS` env loader + range
  validator + 6 new env tests.
* `src/execution/executor.rs`, `src/execution/simulator.rs`,
  `src/execution/transaction.rs`, `tests/engine_tests.rs` — cascading
  test-fixture updates with the new `backend_signer_timeout_ms` field.
* `docs/BACKEND_KMS_VENDOR_ADAPTER_IMPLEMENTATION_AWS_KMS_RESULT.md`
  — NEW (this document).
* `docs/MAINNET_KMS_VENDOR_SELECTION_DECISION.md` — addendum noting
  the AWS KMS adapter shipped.
* `docs/BACKEND_KMS_VENDOR_ADAPTER_IMPLEMENTATION_VENDOR_SPECIFIC_NEXT_TASK.md`
  — addendum noting §2.1 default path shipped; §2.2 alternative-vendor
  override block remains usable if commercial sign-off picks a
  different vendor.
* `RUN_STATE.md` — closure paragraph appended.

No `.env` edited. No `sol/` source touched. No DB schema migration.
No vendor SDK crate added to `Cargo.toml`. Production
`RemoteSignerClient::new` continues to use `UnimplementedTransport` —
no production startup path changed.

## 3. AWS KMS adapter structure

```text
PluggableSignerProvider (existing pluggable trait)
       ▲
       │ implements
       │
AwsKmsSignerProvider
       │ holds Arc<dyn AwsKmsTransport>
       │
       ▼
AwsKmsTransport (NEW vendor-neutral trait)
       ▲
       │ implementations:
       ├── MockAwsKmsTransport (this milestone; in-process, no AWS)
       └── future: production transport backed by `aws-sdk-kms`
           crate, wired in a follow-on PR once the operator authorises
           the rehearsal Phase 3 cutover.
```

### 3.1 AwsKmsSignerProvider

Holds the vendor-neutral transport handle, the AWS KMS key id (opaque
string — ARN or key id passed to the transport untouched), and the
configured `expected_address`. Implements the three
`PluggableSignerProvider` methods:

* `provider_kind()` → `SignerProviderKind::AwsKms`.
* `derive_address()` → calls `transport.get_public_key`, parses
  SubjectPublicKeyInfo, derives the EVM address, cross-checks against
  `expected_address`.
* `sign_prehash()` → calls `transport.sign_digest`, decodes the DER
  signature, recovers `y_parity` via `secp256k1_ecdsa_recover_compact`
  matched against `expected_address`, runs the structural validator
  (`validate_signature` from the pluggable layer), returns a
  `PluggableSignResult` with `provider_request_id` + `audit_log_id`
  carrying the vendor's `CloudTrail RequestId`.

### 3.2 AwsKmsTransport trait

```rust
pub trait AwsKmsTransport: Send + Sync {
    fn get_public_key<'a>(&'a self, key_id: &'a str)
        -> AwsKmsFuture<'a, GetPublicKeyResponse>;
    fn sign_digest<'a>(
        &'a self,
        key_id: &'a str,
        digest: [u8; 32],
        metadata: SignRequestMetadata,
    ) -> AwsKmsFuture<'a, SignDigestResponse>;
}
```

`SignRequestMetadata` threads `request_id` / `intent_id` /
`policy_decision_id` / `policy_fingerprint` into the transport so the
production transport can forward them to the signer microservice's
policy layer (AWS KMS itself doesn't consume them; the microservice
uses them to bind the AWS request to the should_broadcast decision).

### 3.3 MockAwsKmsTransport

Test-only `AwsKmsTransport` with 11 modes:

| Mode | Effect |
|---|---|
| `Success` | Produce a valid SPKI + valid DER signature recoverable to the configured signer's address. |
| `AccessDenied` | Return `AwsKmsError::AccessDenied(reason)`. |
| `Throttling` | Return `AwsKmsError::Throttling`. |
| `Timeout` | Return `AwsKmsError::Timeout`. |
| `KeyDisabled` | Return `AwsKmsError::KeyDisabled`. |
| `KeyInvalid` | Return `AwsKmsError::KeyInvalid(reason)`. |
| `ServiceUnavailable` | Return `AwsKmsError::ServiceUnavailable(reason)`. |
| `MalformedResponse` | Return `AwsKmsError::MalformedResponse(reason)`. |
| `Unknown` | Return `AwsKmsError::Unknown(reason)`. |
| `WrongKey` | `get_public_key` returns an SPKI for a DIFFERENT key + `sign_digest` signs with the SAME different key — exercises the address-mismatch branch end-to-end. |
| `MalformedSignatureDer` | `sign_digest` returns a 3-byte garbage payload — exercises the DER decoder rejection. |

The mock derives valid signatures via an in-process `ExecutorSigner`
holding the same well-known test private key already used by
`src/execution/signer.rs::tests` and `src/api/routes.rs::tests`. NO
new private key bytes introduced.

## 4. Public-key derivation

The adapter accepts AWS KMS's `GetPublicKey` SubjectPublicKeyInfo
output (raw ASN.1 DER bytes). The adapter walks the structure with a
small manual ASN.1 reader (no `der`/`asn1` crate added):

```text
SubjectPublicKeyInfo ::= SEQUENCE {
  algorithm        SEQUENCE { OID ecPublicKey, OID secp256k1 },
  subjectPublicKey BIT STRING { 04 || X(32) || Y(32) }
}
```

The walker:

1. Unwraps the outer `SEQUENCE`.
2. Skips the algorithm `SEQUENCE` (the BIT STRING length check + the
   signature recovery at sign time give defence-in-depth; we do not
   bring a full ASN.1 decoder for OID verification).
3. Extracts the `BIT STRING` content.
4. Confirms `unused-bits = 0` and `length = 65`.
5. Confirms the SEC1 prefix is `0x04` (uncompressed).
6. Derives the EVM address as `keccak256(pubkey[1..])[12..]`
   formatted as a lowercase `0x…` 40-hex-char string.

Rejections:

* Truncated input → `VendorError::MalformedSignature("spki outer: …")`.
* Wrong tag → `VendorError::MalformedSignature("spki algorithm: …")`.
* Non-zero unused-bits → `VendorError::MalformedSignature`.
* SEC1 length ≠ 65 → `VendorError::MalformedSignature`.
* SEC1 prefix ≠ 0x04 → `VendorError::MalformedSignature`.
* Derived address ≠ `expected_address` → `VendorError::AddressMismatch`.

## 5. Signing flow

1. Adapter receives `PluggableSignRequest.prehash` (32 bytes).
2. Adapter calls `transport.sign_digest(key_id, prehash, metadata)`.
3. Adapter decodes the DER signature into `(r, s)` byte pairs via a
   minimal RFC 3279 reader. Accepts canonical DER + tolerates a
   leading zero sign-byte on either integer; rejects oversized
   integers (>32 bytes after trimming).
4. Adapter calls `recover_signature_with_parity(prehash, r, s,
   expected_address)`:
   * Tries `y_parity ∈ {0, 1}`.
   * Recovers the verifying key via `k256::ecdsa::VerifyingKey::recover_from_prehash`.
   * Derives the EVM address from the recovered public key.
   * Returns the matching `RecoverableSignature` (`y_parity`, `r`,
     `s`) when the recovered address case-insensitively equals
     `expected_address`.
   * Returns `VendorError::AddressMismatch` when neither candidate
     recovers to the expected address.
5. Adapter runs the structural validator `validate_signature` from
   the pluggable layer:
   * `y_parity ∈ {0, 1}`.
   * `r ∈ (0, n)`.
   * `s ∈ (0, n/2]` (EIP-2 low-s).
6. Adapter returns `PluggableSignResult { signature,
   provider_request_id, audit_log_id }` — both audit fields carry
   the vendor's `RequestId` for downstream correlation.

NO sign-request retry. NO digest manipulation. NO fallback to local
signing.

## 6. Error mapping

```text
AwsKmsError              → VendorError                       → SignerError.code()
─────────────────────────────────────────────────────────────────────────────────
AccessDenied(_)          → AuthFailed                        → caller-unauthorized
Throttling               → RateLimited                       → rate-limit
Timeout                  → Timeout                           → kms-timeout
KeyDisabled              → Denied("aws_kms_key_disabled")    → kms-unavailable
KeyInvalid(reason)       → Denied(format)                    → kms-unavailable
ServiceUnavailable(r)    → Unavailable(reason)               → transport
MalformedResponse(r)     → MalformedSignature(reason)        → internal
Unknown(reason)          → Unknown(reason)                   → internal
AddressMismatch (post-recovery from adapter, not transport)  → post-sign-from-mismatch
```

All projections preserve the existing 20-variant `SignerError`
taxonomy. No new `SignerError` variant introduced. The
`map_aws_kms_error` helper is exposed as `pub` so future vendor
transport implementations can call it directly when wrapping
`aws-sdk-kms` errors.

## 7. health_check behavior

`PluggableRemoteSignerTransport::send_health_check` calls
`AwsKmsSignerProvider::derive_address`. Since the adapter's
`derive_address` already cross-checks the recovered address against
`expected_address`, a key mismatch returns
`VendorError::AddressMismatch` → `SignerError::PostSignFromMismatch`
without triggering a signing operation. Transport failures
(`ServiceUnavailable` / `Timeout` / etc.) propagate as the matching
`SignerError`. NO sign-call as part of health check.

## 8. Tests added

### `src/execution/signer_adapters/aws_kms.rs::tests` (27 new)

* **SPKI parsing (3):**
  * `synthetic_spki_round_trips_to_expected_address` — full round-trip
    via the test signer.
  * `spki_rejects_truncated_input`
  * `spki_rejects_non_uncompressed_prefix`
* **DER decoding (4):**
  * `der_decoder_round_trips_a_canonical_signature`
  * `der_decoder_strips_leading_sign_byte` (canonical 33-byte INTEGER
    with sign byte)
  * `der_decoder_rejects_garbage_input`
  * `der_decoder_rejects_oversized_integer` (34-byte body fails the
    32-byte secp256k1 bound)
* **y_parity recovery + adapter round-trip (3):**
  * `adapter_round_trip_produces_recoverable_signature`
  * `adapter_returns_address_mismatch_when_vendor_signed_for_wrong_key`
  * `adapter_returns_internal_on_malformed_der`
* **Error mapping table (1):**
  * `map_aws_kms_error_table_is_complete`
* **End-to-end mode → SignerError code (8):**
  * `access_denied_routes_to_caller_unauthorized`
  * `throttling_routes_to_rate_limit`
  * `timeout_routes_to_kms_timeout`
  * `key_disabled_routes_to_kms_unavailable`
  * `key_invalid_routes_to_kms_unavailable`
  * `service_unavailable_routes_to_transport`
  * `malformed_response_routes_to_internal`
  * `unknown_routes_to_internal`
* **health_check (4):**
  * `health_check_success_returns_expected_address`
  * `health_check_wrong_key_returns_post_sign_from_mismatch`
  * `health_check_transport_failure_routes_to_transport`
  * `health_check_disabled_key_routes_to_kms_unavailable`
* **RemoteSignerClient integration + invariant pins (3):**
  * `aws_kms_provider_wired_into_remote_signer_client_round_trips`
  * `aws_kms_provider_kind_is_operational` — `AwsKms` is operational;
    `Mock`/`VendorAgnostic` remain non-operational so mainnet config
    guard keeps refusing them.
  * `aws_kms_error_codes_are_stable_taxonomy` — pins the
    `AwsKmsError::code()` strings for INFO-level tracing.
* **Module-import guard (1):**
  * `map_vendor_error_remains_in_scope_after_module_split` —
    compile-time guard against accidental removal during refactors.

### `src/config/env.rs::tests` (6 new)

* `backend_signer_timeout_ms_default_is_2500`
* `backend_signer_timeout_ms_parses_override`
* `backend_signer_timeout_ms_rejects_zero`
* `backend_signer_timeout_ms_rejects_below_floor`
* `backend_signer_timeout_ms_rejects_above_ceiling`
* `backend_signer_timeout_ms_accepts_boundary_values`

## 9. Tests run

* `cargo fmt --check` — clean.
* `cargo clippy --all-targets --all-features -- -D warnings` — clean.
* `cargo test --all-targets --all-features --no-fail-fast` —
  **1032 / 1032 green** (+33 from prior baseline of 999: 27 AWS KMS
  + 6 env timeout).
* `git diff --check` — clean.
* `forge fmt / build / test` — not re-run (no `sol/` source touched).

## 10. Remaining real-AWS integration gaps

* **`aws-sdk-kms` crate wire-up.** No vendor SDK dependency was added
  by this milestone. The real production transport (lands in
  `BACKEND-AWS-KMS-PRODUCTION-TRANSPORT`) wraps `aws_sdk_kms::Client`
  and maps SDK errors to the `AwsKmsError` taxonomy via
  `map_aws_kms_error`.
* **`RemoteSignerClient::new` production wiring.** Untouched.
  Operator promotes the adapter to production at rehearsal Phase 3 per
  `MAINNET_SIGNER_STAGING_REHEARSAL_PLAN.md`.
* **AWS region + key id wiring.** Belongs to the **signer
  microservice** secret store per Pattern C, NOT the backend `.env`.
  Backend never reads `AWS_KMS_SIGNER_KEY_ID` / `AWS_KMS_SIGNER_REGION`
  / `AWS_ACCESS_KEY_ID` / `AWS_SECRET_ACCESS_KEY` / `AWS_SESSION_TOKEN`.
* **CloudTrail audit-log linkage.** The adapter forwards the
  CloudTrail `RequestId` via `audit_log_id` + `provider_request_id`;
  hooking these into a structured operator dashboard is operator-side
  work (per `MAINNET_SIGNER_ROTATION_AND_INCIDENT_RUNBOOK.md §5
  retention table`).
* **Operator commercial / legal sign-off on Q-CD-5.** Captured offline
  per `MAINNET_KMS_VENDOR_SELECTION_DECISION.md §6`. The backend
  adapter is implementation-ready; the formal Q-CD-5 closure happens
  in the operator binder.

## 11. Forbidden-list compliance

* No mainnet tx attempted. No Sepolia live broadcast.
* No Safe tx. No governance / Timelock / ownership / guardian
  mutation.
* No rebate reserve allocation. No PFV withdrawal. No fund movement.
* No RFQ / order smoke.
* No `.env` edit.
* No real AWS account creation. No real KMS key creation.
* No AWS access keys / secret keys / session tokens in source or
  output.
* No real AWS account IDs. No real KMS key IDs. No real KMS ARNs.
* No private key / admin token / RPC secret / `DATABASE_URL` / API
  key in source or output.
* No guessed credentials. No guessed mainnet executor address.
* No webhook secret creation.
* No private custody roster disclosure.
* No high-cardinality metric labels added (none touched).
* No fallback path that allows mainnet local-key signing.
* No bypass flag weakening mainnet policy.
* No removal of `UnimplementedTransport` from
  `RemoteSignerClient::new`.
* No secrets printed.
* `aws-sdk-kms` crate NOT added to `Cargo.toml`.

## 12. Next milestone recommendation

* **`BACKEND-AWS-KMS-PRODUCTION-TRANSPORT`** — wire `aws_sdk_kms`
  crate behind a `production` feature flag; implement
  `AwsKmsTransport` for the real client. Tests still use the mock —
  the production transport ships disabled by default. No real AWS
  account / KMS key required.
* **`MAINNET-SIGNER-MICROSERVICE-DEPLOYMENT`** — operator-side; the
  microservice that fronts AWS KMS per Pattern C lives outside this
  repo. Inputs from this milestone: the AWS KMS adapter's
  `AwsKmsTransport` request/response shape + `SignRequestMetadata`
  carry every field the microservice needs to bind to the policy
  layer.

Parallel operator tracks unchanged: `MAINNET-AUDIT-EXT-KICKOFF`,
`MAINNET-TREASURY-SAFE-CREATION-PACKET`,
`MAINNET-INSURANCE-OPERATOR-POLICY-PACKET`,
`FRONTEND-V2G-W3-SSR-PROXY`.
