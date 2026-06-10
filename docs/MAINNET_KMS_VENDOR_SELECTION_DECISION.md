# MAINNET KMS vendor selection — decision record

**Posture:** DECISION / DOC ONLY. No source code modified. No `.env`
edited. No vendor account created. No KMS/HSM/MPC key created. No
provider credential recorded. No private custody roster.

> **Addendum (2026-06-10, follow-on `BACKEND-KMS-VENDOR-ADAPTER-IMPLEMENTATION-AWS-KMS`):**
> the AWS KMS adapter (`AwsKmsSignerProvider`) shipped at
> `src/execution/signer_adapters/aws_kms.rs` against a mockable
> `AwsKmsTransport`. 27 unit + 6 env tests. No `aws-sdk-kms` crate
> added — production transport ships in a follow-on PR. Production
> `RemoteSignerClient::new` continues to use `UnimplementedTransport`.
> See `docs/BACKEND_KMS_VENDOR_ADAPTER_IMPLEMENTATION_AWS_KMS_RESULT.md`.
>
> **Addendum (2026-06-10, follow-on `BACKEND-AWS-KMS-PRODUCTION-TRANSPORT`):**
> the real `aws-sdk-kms`-backed `AwsKmsSdkTransport` shipped behind
> the `aws-kms-transport` Cargo feature flag at
> `src/execution/signer_adapters/aws_kms_sdk.rs`. AWS SDK only pulled
> when the feature is enabled. 11 new feature-gated tests with NO
> live AWS calls. Production `RemoteSignerClient::new` STILL uses
> `UnimplementedTransport`. See
> `docs/BACKEND_AWS_KMS_PRODUCTION_TRANSPORT_RESULT.md`.
>
> **Addendum (2026-06-10, follow-on `BACKEND-AWS-KMS-CLOUDTRAIL-REQUEST-ID`):**
> CloudTrail `RequestId` extraction shipped — the production transport
> now returns the real AWS RequestId (sanitised + bounded; synthetic
> UUID fallback when SDK returns `None`). Closes §10 known gap from
> the production transport milestone. 10 new feature-gated tests; no
> live AWS calls; no new dependencies. See
> `docs/BACKEND_AWS_KMS_CLOUDTRAIL_REQUEST_ID_RESULT.md`.
**Closes milestone:** `MAINNET-KMS-VENDOR-SELECTION` — closes the
Q-CD-5 vendor sub-decision opened by
`MAINNET-CUSTODY-CLUSTER-2-RESOLUTION` and matrix-laid by
`MAINNET-SIGNER-VENDOR-AND-REHEARSAL-PACK`.
**Anchors:**
- `docs/MAINNET_SIGNER_VENDOR_SELECTION_MATRIX.md` — 18-criterion ×
  10-category matrix; anti-criteria; recommended shortlist.
- `docs/MAINNET_SIGNER_VENDOR_ADAPTER_REQUIREMENTS.md` — adapter
  contract.
- `docs/BACKEND_KMS_VENDOR_ADAPTER_IMPLEMENTATION_PLUGGABLE_RESULT.md`
  — pluggable adapter surface already shipped at
  `src/execution/signer_adapters.rs`.
- `docs/MAINNET_SIGNER_STAGING_REHEARSAL_PLAN.md` — 7-phase ladder.
- `docs/MAINNET_SIGNER_ROTATION_AND_INCIDENT_RUNBOOK.md` — lifecycle.
- `MAINNET_CUSTODY_POLICY.md §6 + §7 + §8` — custody rules.
- `MAINNET_CUSTODY_CLUSTER_2_RESOLUTION_REDACTED.md` — Pattern C +
  Q-CD-5 vendor sub-decision context.

## 0. Hard rules (this doc)

```text
no source code change                    ✅
no vendor account creation               ✅
no real KMS/HSM/MPC key creation         ✅
no provider credential recorded          ✅
no guessed account ID                    ✅
no guessed mainnet executor address      ✅
no .env edit                             ✅
no chain tx                              ✅
no Safe tx                               ✅
no private custody roster                ✅
no canary broadcast                      ✅
```

## 1. Decision status

| Aspect | Value |
|---|---|
| **Technical recommendation** | **READY** — preferred + fallback paths selected on the matrix's 18 criteria. |
| **Operator commercial / legal sign-off** | **OPERATOR_INPUT_REQUIRED** — final closure of Q-CD-5 vendor sub-decision waits on the inputs enumerated in §6. |
| **Backend implementation path** | **UNBLOCKED** — backend track can proceed against the technical recommendation by wiring an `AwsKmsSignerProvider: PluggableSignerProvider` per `docs/BACKEND_KMS_VENDOR_ADAPTER_IMPLEMENTATION_VENDOR_SPECIFIC_NEXT_TASK.md`. If the operator's commercial sign-off ultimately selects a different vendor, the pluggable shape lets us swap providers with minimal rework. |
| **Rehearsal phases** | **READY** — `MAINNET_SIGNER_STAGING_REHEARSAL_PLAN.md` Phases 1-2 (mock + sandbox) can begin immediately against the recommendation; Phases 3-7 gated on operator sign-off. |

The pluggable provider abstraction (`src/execution/signer_adapters.rs`)
is intentionally vendor-agnostic so that a late operator override of
the technical recommendation does NOT require throwing away the
adapter work — only a 3-method `<Vendor>SignerProvider` impl needs to
be re-written.

## 2. Current readiness summary

* `PluggableSignerProvider` trait exists (3 methods: `provider_kind` /
  `derive_address` / `sign_prehash`).
* `PluggableRemoteSignerTransport` bridges the trait to
  `SignerTransport` via `RemoteSignerClient::with_transport`.
* `MockVendorSignerProvider` covers Success + 9 error modes with
  bounded `VendorError → SignerError` mapping.
* `ExecutionConfig.backend_signer_provider: Option<SignerProviderKind>`
  with mainnet `Mock` refusal in `validate_signer_backend`.
* Production `RemoteSignerClient::new` still uses
  `UnimplementedTransport` — fail-closed; no signing capability.
* 999 / 999 backend tests green.
* **No operational vendor adapter exists.** No real key. No real
  provider account. No real credentials anywhere in tracked source or
  docs.

## 3. Technical recommendation

| Path | Choice | Provider Kind enum | Rationale |
|---|---|---|---|
| **Preferred** | **AWS KMS (asymmetric, `KeySpec=ECC_SECG_P256K1`, `SignatureAlgorithm=ECDSA_SHA_256`)** | `SignerProviderKind::AwsKms` | Lowest integration friction with the existing `PluggableSignerProvider` shape; mature secp256k1 since 2020; EU regions cover residency; lowest unit cost; auditor familiarity. |
| **Fallback A — compliance-driven** | **AWS CloudHSM (FIPS 140-2 L3 single-tenant)** | `SignerProviderKind::AwsKms` (same `SignerProviderKind` value; provider impl is distinct but behind the same enum branch unless the operator wants split observability) | If operator compliance program requires single-tenant attested HSM. PKCS#11 portable. ~10× cost of KMS. |
| **Fallback B — MPC-ergonomic** | **Turnkey** | `SignerProviderKind::Turnkey` | If operator prefers pure-play Ethereum signing with strong policy engine + enclave non-extraction + good Rust SDK. EU residency status MUST be confirmed before commit (§6 question). |
| **Deferred** | GCP KMS / Cloud HSM, Azure Managed HSM, Fireblocks, hardware/offline | — | Acceptable per the matrix; not selected because the preferred + fallback paths cover the operator's likely commercial postures without proliferating maintenance burden. May be revisited in a follow-on rotation cycle. |

### 3.1 Comparison summary against DeOpt launch needs

| Criterion | AWS KMS | AWS CloudHSM | Turnkey |
|---|---|---|---|
| Base mainnet (chainId 8453) tx signing | Yes (backend prehash + assemble; vendor signs) | Yes (PKCS#11 prehash + assemble) | Yes (native EVM tx API) |
| secp256k1 native | Yes | Yes | Yes |
| Non-exportable key | Yes (`Origin=AWS_KMS`) | Yes (single-tenant HSM) | Yes (enclave-backed) |
| EU region | eu-central-1 / eu-west-1 / eu-west-2 / eu-west-3 / eu-north-1 | eu-central-1 / eu-west-1 / eu-west-2 | US-primary; EU residency in development — **verify before commit** |
| Per-request audit log | CloudTrail | CloudHSM audit log + CloudTrail wrap | Per-request log + policy decision log |
| Emergency disable | `DisableKey` API + IAM | HSM user perms + IAM | Policy engine + key delete |
| IAM / service auth | IAM role + STS short-lived creds; VPC PrivateLink | mTLS to HSM endpoint + PKCS#11 | Public-key authenticated API requests |
| Latency p99 | ~100-180 ms | ~50-150 ms | ~500 ms - 1 s |
| Rust integration cost | Low (`aws-sdk-kms`) | Medium (PKCS#11 driver + connection mgmt) | Low-Medium (Turnkey Rust SDK) |
| Cost | $1/key/mo + $0.03/10k signs | ~$1.45/hr/HSM ≈ $1,050/mo/cluster | Pay-per-sign mid-tier |
| Operator complexity | Low (managed KMS) | Medium (cluster mgmt + on-call) | Low (managed SaaS) |
| Rotation workflow | Manual `CreateKey` + alias swap | Same as KMS pattern | Per-key activity; manual swap |
| Compatibility with `PluggableSignerProvider` | Direct: `aws-sdk-kms::Sign` → `RecoverableSignature` | Direct: PKCS#11 `C_Sign` → `RecoverableSignature` | Direct: Turnkey SDK → `RecoverableSignature` |

All three pass the matrix's anti-criteria (§6 of the matrix: key non-extraction / EU residency / emergency disable / audit log id / SOC2-equivalent attestation — modulo Turnkey EU residency confirmation).

### 3.2 Why AWS KMS as preferred

* The matrix scoring puts AWS KMS first on integration cost +
  latency + EU residency + unit cost.
* The pluggable adapter trait was designed to map 1:1 to AWS KMS's
  request/response shape (single sign call returns DER-encoded
  ECDSA signature; backend handles prehash + assemble + recovery,
  matching the existing `eip1559_transaction_prehash` +
  `assemble_eip1559_signed_transaction` helpers).
* The operator's existing AWS posture (already-used cloud
  infrastructure) is the most likely commercial starting point.
* Auditor familiarity is highest for AWS KMS (SOC2 / ISO 27001 /
  PCI / FIPS 140-2 L3 via CloudHSM backing).

### 3.3 Why dual fallbacks

* `Fallback A: AWS CloudHSM` — same Rust integration footprint as
  KMS (PKCS#11 is more code, but same trait); strongest for
  auditors requiring single-tenant attested HSM. Higher cost +
  operational burden, so it's a fallback rather than default.
* `Fallback B: Turnkey` — if the operator commercially prefers a
  pure-play Ethereum signing platform with policy-engine ergonomics
  and enclave isolation. The matrix flagged EU residency as the
  decision-blocker; if Turnkey confirms EU residency before
  commitment, this is operator-grade ready.

## 4. Implementation consequences (for the preferred path)

### 4.1 Required env / config keys (no real values written)

| Key | Type | Default | Notes |
|---|---|---|---|
| `BACKEND_SIGNER_MODE` | `local_dev` \| `remote` | inferred from `BACKEND_SIGNER_ENDPOINT` presence | Must be `remote` on mainnet. |
| `BACKEND_SIGNER_ENDPOINT` | URL (operator's signer microservice mTLS endpoint) | None | Required when `BACKEND_SIGNER_MODE=remote`. The vendor SDK does NOT connect directly to AWS — it goes through the signer microservice per Pattern C. |
| `BACKEND_REMOTE_SIGNER_PROVIDER` | `aws_kms` | None (fail-closed) | Selects the `SignerProviderKind::AwsKms` adapter behind the microservice. |
| `AWS_KMS_KEY_ID` | `arn:aws:kms:eu-…:…:key/…` | None | Operator-provisioned key. Loaded by the signer microservice, NOT by the backend. Listed here for reference — NEVER configured in the backend `.env`. |
| `AWS_KMS_REGION` | AWS region string (e.g. `eu-central-1`) | None | Signer microservice config; not backend. |
| `BACKEND_SIGNER_TIMEOUT_MS` | u32 (100..=30000) | 2500 | Adapter timeout per `MAINNET_SIGNER_VENDOR_ADAPTER_REQUIREMENTS.md §2.10`. Added by the vendor-specific milestone. |

**Operator MUST NOT** put `AWS_KMS_KEY_ID` / `AWS_KMS_REGION` /
`AWS_ACCESS_KEY_ID` / `AWS_SECRET_ACCESS_KEY` / `AWS_SESSION_TOKEN`
into the backend `.env`. Those live entirely inside the signer
microservice's secret store (IAM role + STS short-lived creds,
ideally) and never reach the backend.

### 4.2 Provider kind enum value

`SignerProviderKind::AwsKms` already exists at
`src/execution/signer_adapters.rs:69`. Parse strings:
`"aws_kms"` | `"awskms"` | `"aws-kms"` (case-insensitive).

### 4.3 Signer address derivation flow

1. Operator creates the asymmetric KMS key with
   `KeySpec=ECC_SECG_P256K1`.
2. Operator calls `kms:GetPublicKey` once via the signer microservice
   admin path; recovers DER-encoded SEC1 public key; computes the EVM
   address as `keccak256(public_key.uncompressed[1..])[12..]`.
3. Operator records the derived address publicly (it is non-secret)
   as `EXECUTOR_FROM_ADDRESS` in the operator secret store.
4. `AwsKmsSignerProvider::derive_address` at the backend adapter
   layer calls the same `GetPublicKey` path against the signer
   microservice (NOT directly against AWS) and re-derives the
   address for the startup `health_check` cross-check.

### 4.4 Request / response mapping

| Pluggable trait | AWS KMS API | Notes |
|---|---|---|
| `PluggableSignRequest.prehash` | `Sign.Message` (raw 32-byte prehash) | Set `MessageType=DIGEST` so KMS does not hash again. |
| `PluggableSignRequest.policy_decision_id` | Forwarded as `EncryptionContext` (optional) or passed via the signer microservice request body | Bound at the microservice's policy layer per design doc §5.1. |
| `PluggableSignRequest.policy_fingerprint` | Forwarded the same way. Cross-checked at the microservice. | Same as above. |
| `PluggableSignResult.signature` | KMS `Sign.Signature` (DER-encoded ECDSA) → decoded to `(r, s)`; `y_parity` recovered via `secp256k1_ecdsa_recover_compact` against the prehash + recovered public key | Standard EVM signature assembly. |
| `PluggableSignResult.provider_request_id` | KMS `RequestId` (vendor-issued audit correlation) | Lands in `SignerResponse.remote_signer_request_id`. |
| `PluggableSignResult.audit_log_id` | CloudTrail event id (looked up via the microservice's CloudTrail tail OR derived from request id) | Lands in `SignerResponse.kms_request_id` + `audit_log_id`. |

### 4.5 Timeout + retry policy

* Default `BACKEND_SIGNER_TIMEOUT_MS=2500` (per requirements doc
  §2.10).
* Adapter MUST NOT retry sign requests (re-sign risks duplicate
  intent submission with a different nonce).
* Adapter MAY transparently retry `derive_address` /
  `health_check` (read-only, idempotent).

### 4.6 `health_check` behavior

* Calls the signer microservice's health endpoint.
* Cross-checks the recovered address against the configured
  `expected_address`.
* Returns Err on any mismatch or vendor unreachable — propagated as
  `SignerError::PostSignFromMismatch` or
  `SignerError::Transport(reason)`.
* MUST NOT trigger a sign operation as part of health check.

### 4.7 Address-mismatch handling

* Adapter returns `VendorError::AddressMismatch`.
* `map_vendor_error` projects to `SignerError::PostSignFromMismatch`.
* `RemoteSignerClient`'s existing post-sign cross-check at
  `src/execution/remote_signer.rs:397-401` provides defence-in-depth.
* The transport-layer `expected_address` field provides a third
  layer.

### 4.8 Malformed-signature handling

* Adapter calls `validate_signature` from
  `src/execution/signer_adapters.rs` on every vendor response.
* `r` non-zero + `r < n`; `s` non-zero + `s < n` + `s ≤ n/2` (EIP-2
  low-s); `y_parity ∈ {0, 1}`.
* Failure → `VendorError::MalformedSignature(structural_reason)` →
  `SignerError::Internal("malformed-signature")`.

## 5. SignerError taxonomy mapping for AWS KMS

| AWS KMS error class | VendorError | SignerError variant | code() |
|---|---|---|---|
| Policy/IAM deny, `DisabledException`, `KMSInvalidStateException` | `Denied` | `KmsUnavailable` | `kms-unavailable` |
| SDK timeout / request timeout | `Timeout` | `KmsTimeout` | `kms-timeout` |
| HTTP 5xx / connection refused / DNS error | `Unavailable(short_reason)` | `Transport(reason)` | `transport` |
| 401/403 — credentials / IAM auth failed | `AuthFailed` | `CallerUnauthorized` | `caller-unauthorized` |
| `ThrottlingException` / 429 | `RateLimited` | `RateLimit` | `rate-limit` |
| Malformed signature output (structural validator fails) | `MalformedSignature(reason)` | `Internal("malformed-signature")` | `internal` |
| Recovered EVM address ≠ `expected_address` | `AddressMismatch` | `PostSignFromMismatch` | `post-sign-from-mismatch` |
| Any other AWS SDK error | `Unknown(reason)` | `Internal(reason)` | `internal` |

The 8-row table preserves the existing `SignerError::code()`
taxonomy. No new variant introduced. Reason strings are 80-char
capped at `map_vendor_error` to enforce the redaction contract.

## 6. Operator commercial / legal questions

The technical recommendation is READY. Final closure of Q-CD-5
requires the operator to resolve the following commercial / legal
inputs. Each is OPERATOR-ONLY (Backend + Security can advise but not
decide):

| # | Question | Why it matters |
|---|---|---|
| O1 | Does the operator's existing cloud posture already include AWS? | If yes, AWS KMS is the lowest-friction path. If the operator is GCP-native or Azure-native, the matrix's GCP KMS / Azure Key Vault Premium paths apply with similar shape but different SDK. |
| O2 | What is the auditor's explicit position on FIPS 140-2 L3 single-tenant HSM vs SOC2/ISO 27001 multi-tenant KMS? | Determines AWS KMS (preferred default) vs AWS CloudHSM (compliance-driven fallback). |
| O3 | Annual cost ceiling for the signer track? | AWS KMS ~$1-10/month vs CloudHSM ~$12k/year. |
| O4 | Operator preference for managed-SaaS Ethereum signing platform (Turnkey / Fireblocks) vs cloud-native primitives? | Determines whether Turnkey moves from Fallback B to Preferred. |
| O5 | If Turnkey is considered: does Turnkey have EU residency confirmed at the time of operator commitment? | Hard gate per the matrix anti-criteria. |
| O6 | Jurisdiction-specific data residency: any constraint not met by `eu-central-1` / `eu-west-1` / `eu-west-2` / `eu-west-3` / `eu-north-1`? | If yes, narrows the region choice or eliminates AWS in favor of a regional vendor. |
| O7 | Operator on-call capability: dedicated 24/7 HSM expertise available? | Determines whether AWS CloudHSM operational complexity is acceptable. |
| O8 | Insurance / fiduciary considerations forcing multi-party (MPC) signing? | If yes, escalates Fireblocks / Lit / Coinbase Cloud as Preferred over single-key KMS. |

Operator records the answers in the **offline binder** per
`MAINNET_CUSTODY_CLUSTER_2_RESOLUTION_REDACTED.md §3.5`. Public-safe
outcome is captured in a follow-on
`MAINNET_KMS_VENDOR_SELECTION_FINAL_CLOSURE.md` ONE-LINE result doc
(no vendor credentials, no roster).

## 7. Rehearsal requirements

The rehearsal ladder from
`MAINNET_SIGNER_STAGING_REHEARSAL_PLAN.md` applies unchanged. This
decision doc clarifies which phases the technical recommendation
unblocks immediately vs which phases gate on operator sign-off.

### 7.1 Phases unblocked by this decision

* **Phase 1 — Mock remote signer.** Already covered by the 22 unit
  tests at `src/execution/signer_adapters.rs::tests`.
* **Phase 2 — Sandbox vendor signer.** Begins as soon as the
  vendor-specific implementation (`AwsKmsSignerProvider`) lands per
  `docs/BACKEND_KMS_VENDOR_ADAPTER_IMPLEMENTATION_VENDOR_SPECIFIC_NEXT_TASK.md`.
  Operator provisions a sandbox AWS account + KMS key separate from
  the mainnet account. Backend adapter built against the sandbox
  endpoint. No real mainnet key.

### 7.2 Phases gated on operator sign-off

* **Phase 3 — Sepolia remote-signer rehearsal.** Requires operator
  to provision a Sepolia-only KMS key + record the derived address +
  fund with Sepolia gas. Only one phase that broadcasts (Sepolia,
  supervised per `BACKEND_SIGNER_CUTOVER_RUNBOOK_V2G_FX_Q1.md`).
* **Phase 4 — No-broadcast mainnet dry run.** Requires mainnet
  vendor key provisioned (derived address recorded; NOT yet granted
  executor role).
* **Phase 5 — Read-only mainnet preflight.** Requires mainnet
  contract addresses + RPC URL + live-provider config.
* **Phase 6 — Final Sepolia canary AGAINST production commit.**
  Re-runs Phase 3 against the deploy artifact for the mainnet
  operation.
* **Phase 7 — Mainnet canary PREPARATION.** Operator authorisation
  captured for the separately-runbook'd launch broadcast operation.
  THIS DOC does NOT itself authorise that operation.

### 7.3 Go / no-go criteria for any phase

A phase is **GO** when every acceptance criterion in
`MAINNET_SIGNER_STAGING_REHEARSAL_PLAN.md §2` is GREEN for that
phase. Specifically:

* `/executor/health/v2.signer.signer_mode == "remote"`.
* `/executor/health/v2.signer.remote_signer_configured == true`.
* `/executor/health/v2.signer.signer_address` matches the derived
  address.
* `/executor/health/v2.signer.local_signer_on_mainnet_refused_total
  == 0`.
* `/executor/health/v2.overall_status == "green"`.
* `/metrics` Prometheus scrape green; no `signer_denied_total`
  spike; no `policy_data_failures_total{*}` spike.
* No fallback to local signer — `LocalDevSigner` runtime guard at
  `src/execution/remote_signer.rs:283` MUST never fire on mainnet.

A phase is **NO-GO** if any of the above are red OR if any phase
acceptance criterion in the rehearsal plan fails. NO-GO triggers the
rollback path in `MAINNET_SIGNER_STAGING_REHEARSAL_PLAN.md §3`.

## 8. What remains out of scope (this milestone)

* Real KMS key creation.
* Real AWS account creation.
* IAM provisioning.
* Safe transactions of any kind.
* Mainnet `setExecutor` grant.
* Canary broadcast on Sepolia or mainnet.
* Vendor SDK dependency addition (deferred to the vendor-specific
  implementation milestone).
* Replacement of `UnimplementedTransport` as `RemoteSignerClient::new`
  default (deferred to the operator-authorised rehearsal Phase 3
  cutover).
* Operator commercial / legal sign-off (per §6 questions).

## 9. Cross-links

* `docs/MAINNET_SIGNER_VENDOR_SELECTION_MATRIX.md` — the matrix this
  decision builds on.
* `docs/MAINNET_SIGNER_VENDOR_ADAPTER_REQUIREMENTS.md` — adapter
  contract this implementation must satisfy.
* `docs/MAINNET_SIGNER_STAGING_REHEARSAL_PLAN.md` — rehearsal phases.
* `docs/MAINNET_SIGNER_ROTATION_AND_INCIDENT_RUNBOOK.md` — rotation
  + incident readiness.
* `docs/BACKEND_KMS_VENDOR_ADAPTER_IMPLEMENTATION_PLUGGABLE_RESULT.md`
  — the pluggable shape this decision targets.
* `docs/BACKEND_KMS_VENDOR_ADAPTER_IMPLEMENTATION_VENDOR_SPECIFIC_NEXT_TASK.md`
  — the next-task prompt for the vendor-specific implementation.
* `MAINNET_CUSTODY_POLICY.md §6.7 + §7.4` — custody rules.
* `MAINNET_CUSTODY_CLUSTER_2_RESOLUTION_REDACTED.md §1 + §3.5` —
  Pattern C + Q-CD-5 vendor sub-decision frame.
