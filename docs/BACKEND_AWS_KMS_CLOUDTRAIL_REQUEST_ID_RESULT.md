# BACKEND-AWS-KMS-CLOUDTRAIL-REQUEST-ID — result

**Posture:** SHIPPED at 2026-06-10.

## 1. Goal

Promote `AwsKmsSdkTransport` correlation from synthetic-only tokens to
real AWS CloudTrail `RequestId` (when the SDK exposes it), with a
sanitiser-validated synthetic fallback when the SDK returns `None` or
a malformed value. **No live AWS calls in tests. No real AWS account /
credentials required.**

## 2. Files changed

* `src/execution/signer_adapters/aws_kms_sdk.rs`
  * Added `use aws_sdk_kms::operation::RequestId;` — the SDK
    re-exports `aws_types::request_id::RequestId` at this path,
    avoiding a top-level `aws-types` dependency.
  * Updated `get_public_key` and `sign_digest` paths to call
    `output.request_id()` before consuming the output.
  * Removed `response_request_id_for_get_public_key` / `_for_sign`
    helpers; replaced with `sanitize_request_id(Option<&str>,
    RequestIdOperation)` + `RequestIdOperation` enum + 5 new
    sanitiser-tests + 5 updated/new behavior tests.
* `docs/BACKEND_AWS_KMS_CLOUDTRAIL_REQUEST_ID_RESULT.md` — NEW (this
  document).
* `docs/BACKEND_AWS_KMS_PRODUCTION_TRANSPORT_RESULT.md` — addendum
  noting CloudTrail RequestId extraction shipped; §10 known gap
  closed.
* `docs/MAINNET_KMS_VENDOR_SELECTION_DECISION.md` — third addendum
  noting CloudTrail RequestId shipped.
* `RUN_STATE.md` — closure paragraph appended.

No `.env` edited. No `sol/` source touched. No DB schema migration. No
new dependencies. Production `RemoteSignerClient::new` UNCHANGED.

## 3. RequestId extraction method

`aws-sdk-kms` 1.110 re-exports the `aws_types::request_id::RequestId`
trait via `aws_sdk_kms::operation::RequestId`. Both `SignOutput` and
`GetPublicKeyOutput` implement it — calling `output.request_id()`
returns `Option<&str>` carrying the AWS CloudTrail correlation token
when available (typical UUID-shaped string like
`4f8a1bd9-9f1a-4f9e-9b6e-2d4f6a8c1e3b`).

The extraction is a single trait-method call per response; no
additional `customize().send_with_metadata()` plumbing required — the
trait implementation pulls the metadata from the SDK's smithy
response context internally.

## 4. Sanitiser + fallback contract

`sanitize_request_id(raw: Option<&str>, operation: RequestIdOperation) ->
String`:

| Input | Output |
|---|---|
| `Some(valid_id)` (sanitised → ≤80 chars after prefix) | `{operation.prefix()}-{id}` |
| `None` | `{operation.synthetic_prefix()}-{Uuid::new_v4()}` |
| `Some("")` or whitespace-only | `{operation.synthetic_prefix()}-{Uuid::new_v4()}` |
| `Some(id)` with control character | `{operation.synthetic_prefix()}-{Uuid::new_v4()}` |
| `Some(id)` with URL shape (`://` / `/` / `?` / `=` / `@`) | `{operation.synthetic_prefix()}-{Uuid::new_v4()}` |
| `Some(id)` where `len({prefix}-{id}) > 80` | `{operation.synthetic_prefix()}-{Uuid::new_v4()}` |

Operation prefixes are stable:
* `GetPublicKey` → real: `aws-kms-get-public-key-…`; synthetic:
  `aws-kms-get-public-key-synthetic-…`.
* `Sign` → real: `aws-kms-sign-…`; synthetic: `aws-kms-sign-synthetic-…`.

Audit consumers route by operation regardless of whether the id is
real or synthetic; the `-synthetic-` infix unambiguously identifies
the fallback branch when CloudTrail correlation is unavailable.

## 5. GetPublicKey correlation behavior

```rust
match self.client.get_public_key().key_id(key_id).send().await {
    Ok(output) => {
        let sdk_request_id = output.request_id();  // Option<&str>
        let public_key_der = output.public_key()...
        let request_id = sanitize_request_id(
            sdk_request_id,
            RequestIdOperation::GetPublicKey,
        );
        Ok(GetPublicKeyResponse { public_key_der, request_id })
    }
    ...
}
```

The real AWS `RequestId` from the SDK response now flows into the
`GetPublicKeyResponse.request_id` field — which lives in the
adapter's `provider_request_id` / `audit_log_id` chain (see
`AwsKmsSignerProvider::derive_address` / `sign_prehash` in
`aws_kms.rs`). Operators can now look up the CloudTrail event for any
backend-side `health_check` round-trip via the real id.

## 6. Sign correlation behavior

Analogous to §5 but on the `client.sign()` path. The real AWS
`RequestId` flows through to `SignDigestResponse.request_id` →
`PluggableSignResult.provider_request_id` AND
`PluggableSignResult.audit_log_id` →
`SignerResponse.kms_request_id` +
`SignerResponse.audit_log_id` +
`SignerResponse.remote_signer_request_id` → the broadcast call site's
INFO log at `src/options/service.rs::sign_option_execution_via_signer`.

The CloudTrail `Sign` event id is now end-to-end correlatable to a
backend broadcast attempt without out-of-band timestamp matching.

## 7. Redaction / security behavior

* `sanitize_request_id` rejects control characters → blocks
  log-injection of newlines or terminal escape codes from a misbehaving
  transport shim.
* `sanitize_request_id` rejects URL-shape characters (`://`, `/`,
  `?`, `=`, `@`) → blocks accidental promotion of an endpoint or
  token-bearing URL into the audit field.
* `sanitize_request_id` caps total output length at 80 chars (post
  prefix) → bounded log line + protects metric labels.
* `contains_url_shape_chars` helper is `pub(super)`-style scoped to
  the module + pinned by a dedicated test.
* Synthetic fallback uses `Uuid::new_v4()` → cryptographic-quality
  randomness; two consecutive `None` inputs produce distinct ids
  (pinned by `synthetic_fallback_uniqueness_pins_disambiguation_contract`).
* No SDK error payload leaks via the request id path. Errors flow
  through the separate `map_*_error` chain which already enforces
  80-char redaction.

## 8. CloudTrail audit implication

Before this milestone:
* Backend log line for a successful `Sign` carried `aws-kms-sign-<uuid>`
  (synthetic).
* Operator looked up CloudTrail `Sign` events by timestamp + key id +
  caller arn, then manually correlated.

After this milestone:
* Backend log line for a successful `Sign` carries
  `aws-kms-sign-<real-aws-request-id>` (real CloudTrail event id).
* Operator looks up CloudTrail event by RequestId directly — a single
  query.
* When the SDK fails to expose RequestId (e.g. transport-shim bug),
  the synthetic fallback signals "look it up by timestamp" via the
  `-synthetic-` infix so operators are not confused into searching
  CloudTrail for a non-existent id.

The audit acceptance criterion in
`MAINNET_SIGNER_ROTATION_AND_INCIDENT_RUNBOOK.md §3.4`
("Pull vendor audit log for the compromise window. … Reconstruct the
timeline.") is now a single-lookup operation instead of a
timestamp-bracket scan.

## 9. Tests added

### `src/execution/signer_adapters/aws_kms_sdk.rs::tests` (10 new + 1 removed)

* `sanitize_request_id_preserves_real_cloudtrail_id_with_operation_prefix`
  — typical UUID-shaped AWS id → preserved + operation-prefixed.
* `sanitize_request_id_get_public_key_prefix_routes_correctly` —
  cross-operation pin: `GetPublicKey` prefix differs from `Sign`.
* `sanitize_request_id_missing_falls_back_to_synthetic` — `None` →
  synthetic.
* `sanitize_request_id_empty_string_falls_back_to_synthetic` — `""` →
  synthetic.
* `sanitize_request_id_whitespace_only_falls_back_to_synthetic` —
  `"   "` → synthetic.
* `sanitize_request_id_rejects_control_characters` — newline +
  log-injection-shaped payload → synthetic.
* `sanitize_request_id_rejects_url_shape` — 5 URL-shape adversarial
  inputs (`https://example.com/eviltoken` / `?accessKey=…` /
  `evil/path` / `user@host` / `scheme://target`) → all fall back to
  synthetic.
* `sanitize_request_id_caps_length_to_80_chars` — boundary test for
  the `REQUEST_ID_MAX_LEN` cap.
* `synthetic_fallback_uniqueness_pins_disambiguation_contract` — two
  consecutive `None` inputs produce distinct synthetic ids.
* `synthetic_fallback_distinct_per_operation` — `Sign` and
  `GetPublicKey` fallbacks carry different prefixes.
* `contains_url_shape_chars_helper_pins_url_patterns` — direct test
  of the URL-shape predicate.

Removed (replaced by the new sanitiser contract):
* `synthetic_request_ids_are_prefixed_for_audit_disambiguation` (the
  prior synthetic-only test).

All tests are credential-free + offline. No `.send()` calls. No
network access.

## 10. Tests run

* Default build path:
  * `cargo fmt --check` — clean.
  * `cargo clippy --all-targets -- -D warnings` — clean.
  * `cargo test --no-default-features --all-targets --no-fail-fast`
    — **1032 / 1032 green**. Unchanged from prior baseline.
* Feature-enabled build path:
  * `cargo clippy --all-targets --all-features -- -D warnings` — clean.
  * `cargo test --all-targets --all-features --no-fail-fast`
    — **1053 / 1053 green** (+10 vs prior all-features baseline of 1043;
    +21 vs default baseline of 1032).
* `git diff --check` — clean.
* `forge fmt / build / test` — not re-run (no `sol/` source touched).

## 11. Remaining operator setup gaps

* **Signer microservice integration.** Unchanged. The
  microservice constructs `aws_sdk_kms::Client` from microservice-side
  IAM credentials.
* **Sandbox vendor signer rehearsal (Phase 2).** Unchanged.
* **Production wiring of `RemoteSignerClient::new`.** Unchanged.
  Operator promotes at rehearsal Phase 3 per
  `MAINNET_SIGNER_STAGING_REHEARSAL_PLAN.md`.
* **Operator commercial / legal sign-off on Q-CD-5.** Unchanged.
  Captured offline per
  `MAINNET_KMS_VENDOR_SELECTION_DECISION.md §6`.

## 12. Forbidden-list compliance

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
* No new dependencies added.
* No live AWS network calls.
* No secrets printed.

## 13. Next milestone recommendation

The AWS KMS implementation track is now feature-complete at the
backend layer. Suggested next milestones:

* **`MAINNET-SIGNER-MICROSERVICE-DEPLOYMENT`** — operator-side;
  microservice that fronts AWS KMS per Pattern C lives outside this
  repo. Inputs from this milestone: the `AwsKmsSdkTransport` request
  + response shape with real CloudTrail RequestId pass-through.
* **`MAINNET-SIGNER-REHEARSAL-PHASE-2-EXECUTION`** — operator-side;
  sandbox AWS vendor signer rehearsal per
  `MAINNET_SIGNER_STAGING_REHEARSAL_PLAN.md §2.2`.

Parallel operator tracks unchanged: `MAINNET-AUDIT-EXT-KICKOFF`,
`MAINNET-TREASURY-SAFE-CREATION-PACKET`,
`MAINNET-INSURANCE-OPERATOR-POLICY-PACKET`,
`FRONTEND-V2G-W3-SSR-PROXY`.
