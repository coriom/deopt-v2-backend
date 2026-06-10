# BACKEND-AWS-KMS-PRODUCTION-TRANSPORT — result

**Posture:** SHIPPED at 2026-06-10.

> **Addendum (2026-06-10, follow-on `BACKEND-AWS-KMS-CLOUDTRAIL-REQUEST-ID`):**
> the §10 "Synthetic CloudTrail RequestId (known gap)" section is now
> closed. `aws_sdk_kms::operation::RequestId` trait extraction lands;
> the production transport now returns the real AWS CloudTrail
> `RequestId` (with sanitised + bounded format and synthetic UUID
> fallback when unavailable). See
> `docs/BACKEND_AWS_KMS_CLOUDTRAIL_REQUEST_ID_RESULT.md`.

## 1. Goal

Implement a real `aws-sdk-kms`-backed `AwsKmsTransport` behind an
opt-in Cargo feature. Production transport compiles when the feature
is enabled; default builds remain dependency-light and unchanged.
**Tests perform NO live AWS calls. No real AWS account / KMS key /
credentials are required by this milestone.**

## 2. Files changed

* `Cargo.toml`
  * Added `[features]` block with `default = []` and
    `aws-kms-transport = ["dep:aws-config", "dep:aws-sdk-kms",
    "dep:aws-smithy-runtime-api", "dep:aws-smithy-types"]`.
  * Added 4 optional dependencies pinned to current stable versions:
    * `aws-config = "1.5"` (rustls + rt-tokio + behavior-version-latest)
    * `aws-sdk-kms = "1.47"` (rustls + rt-tokio + behavior-version-latest)
    * `aws-smithy-runtime-api = "1.7"` (client feature)
    * `aws-smithy-types = "1.2"` (default)
* `src/execution/signer_adapters.rs` — added
  `#[cfg(feature = "aws-kms-transport")] pub mod aws_kms_sdk;`.
* `src/execution/signer_adapters/aws_kms_sdk.rs` — **NEW**
  (~370 LoC + 11 unit tests).
* `Cargo.lock` — updated for the optional deps.
* `docs/BACKEND_AWS_KMS_PRODUCTION_TRANSPORT_RESULT.md` — NEW (this
  document).
* `docs/BACKEND_KMS_VENDOR_ADAPTER_IMPLEMENTATION_AWS_KMS_RESULT.md`
  — addendum noting the production transport shipped.
* `docs/MAINNET_KMS_VENDOR_SELECTION_DECISION.md` — addendum noting
  the production transport shipped.
* `RUN_STATE.md` — closure paragraph appended.

No `.env` edited. No `sol/` source touched. No DB schema migration.
Production `RemoteSignerClient::new` UNCHANGED. No real AWS account /
KMS key / credentials referenced anywhere in source or docs.

## 3. Feature flag

| Name | Default | Pulls |
|---|---|---|
| `aws-kms-transport` | **OFF** | `aws-config`, `aws-sdk-kms`, `aws-smithy-runtime-api`, `aws-smithy-types` |

Operators enable explicitly:

```bash
cargo build --features aws-kms-transport
cargo test  --features aws-kms-transport
```

Default builds (`cargo build`, `cargo test`) do NOT compile any AWS
SDK code. The `signer_adapters::aws_kms_sdk` module is gated by
`#[cfg(feature = "aws-kms-transport")]` so its symbols are not in
scope when the feature is off.

## 4. Dependencies added

All four optional crates use default-features-OFF + selected feature
subsets to minimise the dependency graph:

* `aws-config = { version = "1.5", optional = true, default-features = false, features = ["rustls", "rt-tokio", "behavior-version-latest"] }`
* `aws-sdk-kms = { version = "1.47", optional = true, default-features = false, features = ["rustls", "rt-tokio", "behavior-version-latest"] }`
* `aws-smithy-runtime-api = { version = "1.7", optional = true, default-features = false, features = ["client"] }`
* `aws-smithy-types = { version = "1.2", optional = true, default-features = false }`

`rustls` feature on `aws-config` + `aws-sdk-kms` aligns with the
backend's existing rustls usage via `reqwest` + `sqlx` + `quinn`. No
native-TLS / OpenSSL pulled.

## 5. Real transport behavior

```rust
#[derive(Clone, Debug)]
pub struct AwsKmsSdkTransport {
    client: aws_sdk_kms::Client,
}

impl AwsKmsSdkTransport {
    pub fn new(client: aws_sdk_kms::Client) -> Self { ... }
    pub fn client(&self) -> &aws_sdk_kms::Client { ... }
}

impl AwsKmsTransport for AwsKmsSdkTransport {
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

The caller constructs the SDK `Client` (with their own credentials +
region + endpoint + retry strategy). The backend never reads AWS env
keys directly — Pattern C compatibility intact (`AWS_ACCESS_KEY_ID` /
`AWS_SECRET_ACCESS_KEY` / `AWS_SESSION_TOKEN` / `AWS_REGION` /
`AWS_KMS_SIGNER_KEY_ID` are NOT in the backend env loader; they live
in the signer microservice secret store).

## 6. GetPublicKey behavior

* SDK call: `client.get_public_key().key_id(key_id).send().await`.
* On `Ok`: extract `output.public_key()` as `&Blob`; convert to
  `Vec<u8>`; package into the existing `GetPublicKeyResponse` struct.
* Missing `PublicKey` field on `Ok` → `AwsKmsError::MalformedResponse`.
* On `Err(SdkError)`: see §8 mapping.
* `request_id` field: returns a synthetic correlation token of the
  form `"aws-kms-get-public-key-<uuid>"` (see §10).

## 7. Sign behavior

* SDK call: `client.sign().key_id(key_id).message(Blob).message_type(MessageType::Digest).signing_algorithm(SigningAlgorithmSpec::EcdsaSha256).send().await`.
* `MessageType::Digest` + `SigningAlgorithmSpec::EcdsaSha256` are
  REQUIRED for EVM-style EIP-1559 prehash signing. Backend never asks
  KMS to hash for us; the existing
  `eip1559_transaction_prehash` helper produces the 32-byte digest
  before this call.
* `SignRequestMetadata` is accepted by the trait but NOT forwarded to
  AWS — the policy_decision_id / policy_fingerprint binding is
  enforced by the signer microservice's policy layer outside the SDK
  call (per Pattern C).
* On `Ok`: extract `output.signature()` as `&Blob`; convert to
  `Vec<u8>`; package into the existing `SignDigestResponse` struct.
* Missing `Signature` field on `Ok` → `AwsKmsError::MalformedResponse`.
* On `Err(SdkError)`: see §8 mapping.
* `request_id` field: returns a synthetic correlation token of the
  form `"aws-kms-sign-<uuid>"` (see §10).

## 8. Error mapping (SDK → AwsKmsError)

Both `map_get_public_key_error` and `map_sign_error` accept any
`SdkError<E, R>` and project onto the existing 8-variant
`AwsKmsError` taxonomy. The outer `SdkError` variants are matched
first (timeout / dispatch failure / response error); the inner
service error variants (modelled by the smithy code generator) are
matched next; anything not explicitly modelled falls through to
`map_unhandled_or_generic` which does a small case-insensitive
string match for common AWS error code names.

| Source | Maps to AwsKmsError |
|---|---|
| `SdkError::TimeoutError` | `Timeout` |
| `SdkError::DispatchFailure` | `ServiceUnavailable("dispatch-failure: …")` (redacted) |
| `SdkError::ResponseError` | `ServiceUnavailable("response-error")` |
| `DependencyTimeoutException` | `Timeout` |
| `DisabledException` | `KeyDisabled` |
| `InvalidGrantTokenException` | `AccessDenied("invalid-grant-token")` |
| `InvalidKeyUsageException` | `KeyInvalid("invalid-key-usage")` |
| `KeyUnavailableException` | `KeyInvalid("key-unavailable")` |
| `KmsInternalException` | `ServiceUnavailable("kms-internal")` |
| `KmsInvalidStateException` | `KeyInvalid("kms-invalid-state")` |
| `NotFoundException` | `KeyInvalid("not-found")` |
| `UnsupportedOperationException` (GetPublicKey) | `KeyInvalid("unsupported-operation")` |
| `DryRunOperationException` (Sign) | `KeyInvalid("dry-run")` |
| Any other (heuristic by name) | `Throttling` / `AccessDenied` / `Timeout` / `KeyDisabled` / `KeyInvalid` / `ServiceUnavailable` / `Unknown` per `map_unhandled_or_generic` |

Reason strings pass through `redact_sdk_reason` which strips control
characters + caps at 80 chars. Downstream `map_vendor_error` further
truncates to 80 chars at the VendorError → SignerError boundary.

The end-to-end pipeline preserves the existing
`SignerError::code()` taxonomy — no new variant introduced.

## 9. Config behavior

* No new backend env keys added by this milestone.
* AWS credentials / key id / region remain microservice-side per
  Pattern C.
* Default `RemoteSignerClient::new` continues to use
  `UnimplementedTransport` — production wiring of
  `AwsKmsSdkTransport` requires explicit operator code at the rehearsal
  Phase 3 cutover.
* Mainnet guards intact:
  * `LocalDev` refused at startup + runtime + defence-in-depth.
  * `Mock` provider refused at startup.
  * `SignerProviderKind::AwsKms` passes the mainnet config guard
    (`is_operational() == true`).

## 10. Synthetic CloudTrail RequestId (known gap)

`aws-sdk-kms` 1.110 does not expose the response-side
`x-amzn-RequestId` directly on the strongly-typed `*Output` structs.
The metadata is reachable via the `customize().send_with_metadata()`
+ `aws_smithy_runtime_api::client::result::RequestId` trait path, but
threading that through requires non-trivial additional surface.

This milestone returns synthetic correlation tokens
(`"aws-kms-get-public-key-<uuid>"` / `"aws-kms-sign-<uuid>"`) that
are CLEARLY non-CloudTrail (prefixed for audit disambiguation, pinned
by `synthetic_request_ids_are_prefixed_for_audit_disambiguation`
test). Operators cross-correlate via the signer microservice's own
request log against CloudTrail by timestamp + key id.

A follow-on milestone `BACKEND-AWS-KMS-CLOUDTRAIL-REQUEST-ID`
promotes this to the real AWS RequestId via the customize +
RequestId path. Non-launch-blocking; CloudTrail correlation works
manually in the meantime.

## 11. Tests added

### `src/execution/signer_adapters/aws_kms_sdk.rs::tests` (11 new, all behind `#[cfg(feature = "aws-kms-transport")]`)

* `transport_constructs_without_live_aws_call` — builds the
  `AwsKmsSdkTransport` from a placeholder `aws_sdk_kms::Client`
  without invoking `.send()`. Compile-time check that the
  `AwsKmsTransport` trait is satisfied.
* `redact_sdk_reason_caps_length_and_strips_control_chars` — 80-char
  cap + control-char strip; pins the redaction contract.
* `map_unhandled_or_generic_routes_throttling`
* `map_unhandled_or_generic_routes_access_denied`
* `map_unhandled_or_generic_routes_disabled`
* `map_unhandled_or_generic_routes_internal`
* `map_unhandled_or_generic_routes_invalid_key`
* `map_unhandled_or_generic_routes_timeout`
* `map_unhandled_or_generic_routes_unknown`
* `sdk_mapping_targets_align_with_vendor_error_taxonomy` — pins the
  full AwsKmsError → VendorError → SignerError code chain for every
  variant the SDK can produce.
* `synthetic_request_ids_are_prefixed_for_audit_disambiguation` —
  pins the audit-correlation prefix contract.

All tests are credential-free + offline. No `.send()` calls. No
network access.

## 12. Tests run

* Default build path:
  * `cargo fmt --check` — clean.
  * `cargo clippy --all-targets -- -D warnings` — clean.
  * `cargo test --no-default-features --all-targets --no-fail-fast`
    — **1032 / 1032 green**. Unchanged from prior milestone.
* Feature-enabled build path:
  * `cargo clippy --all-targets --all-features -- -D warnings` — clean.
  * `cargo test --all-targets --all-features --no-fail-fast`
    — **1043 / 1043 green** (+11 vs default).
* `git diff --check` — clean.
* `forge fmt / build / test` — not re-run (no `sol/` source touched).

## 13. Remaining operator setup gaps

* **Signer microservice integration.** The `AwsKmsSdkTransport`
  expects the caller to construct the `aws_sdk_kms::Client` from
  microservice-side IAM credentials. Microservice infrastructure
  (separate repo / operator-managed) is not in scope here.
* **CloudTrail RequestId promotion.** See §10. Synthetic tokens
  ship today; real CloudTrail RequestId is a small follow-on.
* **Sandbox vendor signer rehearsal (Phase 2).** Operator provisions
  a sandbox AWS account + KMS key per
  `MAINNET_SIGNER_STAGING_REHEARSAL_PLAN.md §2.2` and runs the
  adapter against the sandbox endpoint manually (offline test
  fixtures in the backend repo never touch sandbox).
* **Production wiring of `RemoteSignerClient::new`.** Untouched.
  Operator promotes at rehearsal Phase 3 per
  `MAINNET_SIGNER_STAGING_REHEARSAL_PLAN.md`.
* **Operator commercial / legal sign-off on Q-CD-5.** Captured
  offline per `MAINNET_KMS_VENDOR_SELECTION_DECISION.md §6`.

## 14. Forbidden-list compliance

* No mainnet tx attempted. No Sepolia live broadcast.
* No Safe tx. No governance / Timelock / ownership / guardian
  mutation.
* No rebate reserve allocation. No PFV withdrawal. No fund movement.
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
* No `.send()` calls in any test (verified by grep on
  `aws_kms_sdk.rs::tests`).
* No live AWS network calls anywhere.
* No secrets printed.

## 15. Next milestone recommendation

* **`BACKEND-AWS-KMS-CLOUDTRAIL-REQUEST-ID`** — small follow-on;
  promote the synthetic correlation token to the real AWS
  `RequestId` via the SDK's customize + RequestId trait path.
  Non-launch-blocking.
* **`MAINNET-SIGNER-MICROSERVICE-DEPLOYMENT`** — operator-side;
  microservice that fronts AWS KMS per Pattern C lives outside this
  repo. Inputs from this milestone: the `AwsKmsSdkTransport` request
  shape (key_id + 32-byte digest + microservice-supplied
  `SignRequestMetadata`).

Parallel operator tracks unchanged: `MAINNET-AUDIT-EXT-KICKOFF`,
`MAINNET-TREASURY-SAFE-CREATION-PACKET`,
`MAINNET-INSURANCE-OPERATOR-POLICY-PACKET`,
`FRONTEND-V2G-W3-SSR-PROXY`.
