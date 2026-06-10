# BACKEND-KMS-VENDOR-ADAPTER-IMPLEMENTATION-PLUGGABLE — result

**Posture:** SHIPPED at 2026-06-10.

## 1. Goal

Implement a vendor-agnostic adapter skeleton behind the existing
`RemoteSigner` abstraction. Mockable, fail-closed, observable,
secret-safe — ready for a concrete AWS/Turnkey/Fireblocks/GCP/Azure
provider in a follow-on milestone once `MAINNET-KMS-VENDOR-SELECTION`
resolves. **No real vendor SDK dependency added. No vendor credentials.
No KMS/HSM/MPC keys created. No `.env` edit. No broadcast.**

## 2. Files changed

* `src/execution/signer_adapters.rs` — **NEW** module (~800 LoC + 22
  unit tests).
* `src/execution/mod.rs` — declares the new module.
* `src/execution/config.rs` — added
  `backend_signer_provider: Option<SignerProviderKind>` field +
  mainnet `Mock` refusal in `validate_signer_backend` + 4 new tests.
* `src/execution/executor.rs`, `src/execution/simulator.rs`,
  `src/execution/transaction.rs`, `tests/engine_tests.rs` — extended
  internal test fixtures with the new field (cascading field
  initializer updates).
* `src/config/env.rs` — `BACKEND_REMOTE_SIGNER_PROVIDER` env loader
  + 3 new tests.
* `docs/BACKEND_KMS_VENDOR_ADAPTER_IMPLEMENTATION_PLUGGABLE_RESULT.md`
  — NEW (this document).
* `docs/MAINNET_SIGNER_VENDOR_ADAPTER_REQUIREMENTS.md` — implementation
  addendum.
* `docs/BACKEND_KMS_VENDOR_ADAPTER_IMPLEMENTATION_NEXT_TASK.md` —
  pluggable-path addendum noting completion + the vendor-specific
  path remains.
* `RUN_STATE.md` — closure paragraph appended.

No `.env` edited. No `sol/` source touched. No DB schema migration.
No production startup-path change (the production
`RemoteSignerClient::new` continues to use `UnimplementedTransport`).

## 3. Provider abstraction

```rust
pub trait PluggableSignerProvider: Send + Sync {
    fn provider_kind(&self) -> SignerProviderKind;
    fn derive_address<'a>(&'a self) -> PluggableFuture<'a, AccountId>;
    fn sign_prehash<'a>(
        &'a self,
        request: PluggableSignRequest<'a>,
    ) -> PluggableFuture<'a, PluggableSignResult>;
}
```

* **`SignerProviderKind`** — bounded enum: `Mock`, `VendorAgnostic`,
  `AwsKms`, `Turnkey`, `Fireblocks`, `GcpKms`, `AzureHsm`. `is_operational()`
  returns true only for the concrete-vendor variants.
* **`PluggableSignRequest`** — subset of `SignerRequest` carrying every
  field the vendor SDK needs: `request_id`, `intent_id`, `chain_id`,
  `target_contract`, `calldata_hash`, `policy_decision_id`,
  `policy_fingerprint`, `signer_request_id`, `gas_limit`,
  `transaction_value_wei`, `prehash`.
* **`PluggableSignResult`** — `signature` (`RecoverableSignature`),
  `provider_request_id`, `audit_log_id`.

* **`PluggableRemoteSignerTransport`** — concrete `SignerTransport`
  implementation generic over any `PluggableSignerProvider`. Owns the
  `expected_address` for the defence-in-depth post-sign cross-check.

* **`validate_signature(&RecoverableSignature)`** — structural
  validation: `y_parity ∈ {0, 1}`, `r ∈ (0, n)`, `s ∈ (0, n/2]` (EIP-2
  low-s). Malformed vendor output produces
  `VendorError::MalformedSignature` → `SignerError::Internal("malformed-signature")`.

## 4. Mock provider behavior

`MockVendorSignerProvider` is a test-only `PluggableSignerProvider`
holding an in-process `ExecutorSigner` (for valid signatures) plus a
runtime-mutable `MockProviderMode`:

| Mode | Effect |
|---|---|
| `Success` | Produce a real signature recoverable to the holder's address. |
| `Denied` | Return `VendorError::Denied(reason)`. |
| `Timeout` | Return `VendorError::Timeout`. |
| `Unavailable` | Return `VendorError::Unavailable(reason)`. |
| `AuthFailed` | Return `VendorError::AuthFailed`. |
| `RateLimited` | Return `VendorError::RateLimited`. |
| `MalformedSignature` | Produce a valid signature, mangle `y_parity=7`, ship it (triggers local validator). |
| `AddressMismatch` | Return `VendorError::AddressMismatch`. |
| `Unknown` | Return `VendorError::Unknown(reason)`. |
| `HealthFailed` | `derive_address()` returns Err; `sign_prehash` also returns Unavailable. |

`MockProviderMode` is mutable via `set_mode(mode)` so a single
provider instance can be driven through every error branch in a
single test sequence.

## 5. Error mapping

`VendorError` → `SignerError` per
`MAINNET_SIGNER_VENDOR_ADAPTER_REQUIREMENTS.md §3`:

| VendorError | SignerError | code() |
|---|---|---|
| `Denied(_)` | `KmsUnavailable` | `kms-unavailable` |
| `Timeout` | `KmsTimeout` | `kms-timeout` |
| `Unavailable(reason)` | `Transport(truncate(reason))` | `transport` |
| `AuthFailed` | `CallerUnauthorized` | `caller-unauthorized` |
| `RateLimited` | `RateLimit` | `rate-limit` |
| `MalformedSignature(_)` | `Internal("malformed-signature")` | `internal` |
| `AddressMismatch` | `PostSignFromMismatch` | `post-sign-from-mismatch` |
| `Unknown(reason)` | `Internal(truncate(reason))` | `internal` |

`truncate(reason)` caps the reason string at 80 chars to enforce the
redaction contract (no provider response bodies smuggled through
error reasons).

**No new `SignerError` variant introduced.** Existing 20-variant
taxonomy covers every vendor category, preserving the
`signer_denied_total{code, signer_kind}` Prometheus label
vocabulary.

## 6. Config behavior

* New typed-config field
  `ExecutionConfig.backend_signer_provider: Option<SignerProviderKind>`.
* Env key `BACKEND_REMOTE_SIGNER_PROVIDER` accepted by the env
  loader; accepts the 7 variant names (case-insensitive plus
  `-`/`_` interchangeable).
* Default: `None` — the production `RemoteSignerClient::new`
  continues to use its `UnimplementedTransport` default. Remote-mode
  startup MAY pass with `None`; runtime broadcast attempts FAIL
  CLOSED with `SignerError::Transport("production HTTPS/mTLS transport
  not yet wired…")` — unchanged from prior milestones.
* Mainnet (`chain_id == 8453`) + `backend_signer_provider == Some(Mock)`
  → startup REFUSED with
  `"BACKEND_REMOTE_SIGNER_PROVIDER=mock is REFUSED on mainnet (chain_id=8453); configure an operational vendor adapter"`.
* Mainnet + operational provider kind (AwsKms / Turnkey / Fireblocks /
  GcpKms / AzureHsm) → startup passes (the concrete adapter wiring is
  a separate follow-on milestone; this config-level guard is the
  pre-wire pin).
* `VendorAgnostic` is treated as a non-operational placeholder
  (`is_operational() == false`) — operator-meaningful naming for "I
  understand a vendor is not selected yet"; does not bypass the
  default fail-closed posture.

## 7. Mainnet guard behavior

Three concentric defences against unsafe mainnet signing — all
intact, none weakened by this milestone:

1. **Startup guard** at `ExecutionConfig::validate_signer_backend`
   (`src/execution/config.rs:209`) — refuses `LocalDev` mode on
   mainnet AND refuses `Mock` provider on mainnet.
2. **Runtime guard** at `build_signer_for_state`
   (`src/options/service.rs:1465`) — refuses to build a `LocalDev`
   signer on mainnet at the broadcast call site.
3. **Defence-in-depth guard** at
   `LocalDevSigner::sign_option_execution_tx`
   (`src/execution/remote_signer.rs:283`) — refuses to sign with
   `chain_id == 8453` even if a signer instance was somehow seated.

`PluggableRemoteSignerTransport` operates exclusively as a
`Remote`-mode transport — no code path can have it satisfy a
`LocalDev` request. The mock provider returns a `Mock` kind which the
startup guard refuses on mainnet; defence-in-depth on top.

## 8. RemoteSignerClient integration

No production startup path changed. The pluggable transport can be
constructed and injected via the existing
`RemoteSignerClient::with_transport(endpoint, expected_address, transport)`
constructor (mock-injection point at
`src/execution/remote_signer.rs:364`). The pluggable transport
implements the existing `SignerTransport` trait, so the broadcast
call site at `src/options/service.rs:1438` continues to invoke it
through the unchanged `RemoteSigner::sign_option_execution_tx` future.

Acceptance:

* `should_broadcast` still runs before any signer call.
* Remote signer is NOT called on policy reject.
* Remote signer failure blocks broadcast (no local-key fallback).
* `request_id` / `audit_log_id` / `provider_request_id` flow through
  to the broadcast call site's existing INFO log.

## 9. Observability / health behavior

No new metrics. No new label keys. No new `/executor/health/v2`
fields. The existing observability surface
(`signer_attempted_total{signer_kind}` /
`signer_success_total{signer_kind}` /
`signer_denied_total{code, signer_kind}` /
`last_signer_error_code` / `last_signer_kind` /
`last_broadcast_submitted_ms` /
`local_signer_on_mainnet_refused_total`) already covers every
adapter event because the `SignerError::code()` taxonomy is
preserved.

The mock provider's success path populates
`PluggableSignResult.provider_request_id` and
`audit_log_id` with deterministic strings (`"mock-req-id"` /
`"mock-audit-id"`) so test assertions can pin the round-trip.

## 10. Tests added

### `src/execution/signer_adapters.rs::tests` (22 new)

* `mock_success_returns_recoverable_signature_and_address` — happy
  path; structural validation passes.
* `mock_denial_maps_to_kms_unavailable`
* `mock_timeout_maps_to_kms_timeout`
* `mock_unavailable_maps_to_transport`
* `mock_auth_failed_maps_to_caller_unauthorized`
* `mock_rate_limited_maps_to_rate_limit`
* `mock_malformed_signature_rejected_by_local_validator` —
  defence-in-depth structural-validation pin.
* `mock_address_mismatch_maps_to_post_sign_from_mismatch`
* `mock_unknown_maps_to_internal`
* `mock_health_check_success_returns_signer_address`
* `mock_health_check_failed_maps_to_transport`
* `health_check_address_mismatch_rejects_post_sign` — health-check
  pins the cross-check even without a sign attempt.
* `vendor_error_codes_are_stable_taxonomy` — code-string contract.
* `map_vendor_error_routes_onto_existing_signer_error_taxonomy`
* `unavailable_reason_string_is_length_bounded` — 80-char cap.
* `validate_signature_accepts_well_formed`
* `validate_signature_rejects_zero_r`
* `validate_signature_rejects_high_s` — EIP-2 low-s + curve-order
  bound.
* `validate_signature_rejects_invalid_y_parity`
* `provider_kind_parse_accepts_each_variant_and_rejects_unknown`
* `provider_kind_is_operational_excludes_mock_and_vendor_agnostic`
* `remote_signer_client_with_pluggable_transport_round_trip` —
  integration construction pin.

### `src/execution/config.rs::tests` (4 new)

* `mainnet_with_mock_pluggable_provider_refuses_startup` — pin the
  config-level mainnet refusal.
* `mainnet_with_operational_provider_kind_passes` — AwsKms passes.
* `mainnet_without_provider_passes_but_runtime_remains_fail_closed`
  — None passes startup; runtime stays fail-closed.
* `sepolia_with_mock_provider_allowed` — non-mainnet pin.

### `src/config/env.rs::tests` (3 new)

* `backend_remote_signer_provider_absent_yields_none`
* `backend_remote_signer_provider_parses_each_variant` —
  case-insensitive + `-`/`_` interchangeable for all 7 variants.
* `backend_remote_signer_provider_unknown_value_rejects_at_startup`
  — startup refusal pin.

## 11. Tests run

* `cargo fmt --check` — clean.
* `cargo clippy --all-targets --all-features -- -D warnings` — clean.
* `cargo test --all-targets --all-features --no-fail-fast` — **999 /
  999 green** (+29 from prior baseline of 970: 22 adapter + 4
  config + 3 env).
* `git diff --check` — clean.
* `forge fmt / build / test` — not re-run (no `sol/` source touched).

## 12. Remaining vendor-specific implementation gaps

* **Concrete vendor adapter** — no `AwsKmsSignerProvider` /
  `TurnkeySignerProvider` / etc. implementation exists. Tracked under
  the vendor-specific path of
  `docs/BACKEND_KMS_VENDOR_ADAPTER_IMPLEMENTATION_NEXT_TASK.md §3.1`.
  Lands once `MAINNET-KMS-VENDOR-SELECTION` resolves.
* **`RemoteSignerClient::with_pluggable_provider` factory** — the
  current implementation uses the existing
  `with_transport(endpoint, expected_address, transport)`
  constructor. A small convenience factory that takes
  `provider + endpoint + expected_address` and returns a fully-wired
  `RemoteSignerClient` lands in the same vendor-specific milestone.
* **`BACKEND_SIGNER_TIMEOUT_MS` typed config** — adapter timeout is
  currently a documentation contract (`2500 ms` default per the
  requirements doc); a typed config field lands with the vendor-specific
  adapter.
* **`tracing` redaction tests** — the adapter doesn't log signature
  bytes or the prehash, but a dedicated capture-tracing-output
  test pinning the absence lands with the vendor-specific milestone.
* **Production startup wiring** — the production `RemoteSignerClient::new`
  must remain on `UnimplementedTransport` until the operator-approved
  rehearsal Phase 3 cutover per
  `MAINNET_SIGNER_STAGING_REHEARSAL_PLAN.md`.

## 13. Forbidden-list compliance

* No mainnet tx attempted. No Sepolia live broadcast.
* No Safe tx. No governance / Timelock / ownership / guardian
  mutation.
* No rebate reserve allocation. No PFV withdrawal. No fund movement.
* No RFQ / order smoke.
* No `.env` edit.
* No real KMS/HSM/MPC key creation. No vendor account creation.
* No real vendor SDK dependency added.
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

## 14. Next milestone recommendation

* **`BACKEND-KMS-VENDOR-ADAPTER-IMPLEMENTATION`** — vendor-specific
  path §3.1 of
  `docs/BACKEND_KMS_VENDOR_ADAPTER_IMPLEMENTATION_NEXT_TASK.md`;
  implements a concrete `PluggableSignerProvider` for the operator's
  selected vendor (e.g. `AwsKmsSignerProvider`). Gated on
  `MAINNET-KMS-VENDOR-SELECTION`.
* **`MAINNET-KMS-VENDOR-SELECTION`** — operator + Security
  resolves the Q-CD-5 vendor sub-decision per
  `docs/MAINNET_SIGNER_VENDOR_SELECTION_MATRIX.md`.

Parallel operator tracks unchanged: `MAINNET-AUDIT-EXT-KICKOFF`,
`MAINNET-TREASURY-SAFE-CREATION-PACKET`,
`MAINNET-INSURANCE-OPERATOR-POLICY-PACKET`,
`FRONTEND-V2G-W3-SSR-PROXY`.
