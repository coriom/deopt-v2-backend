# AWS KMS operator setup pack

**Posture:** DOC / RUNBOOK ONLY. No source code modified. No `.env`
edited. No AWS account, IAM role, or KMS key created by this
milestone. No `terraform apply`. No AWS CLI commands against any real
account.
**Closes milestone:** `AWS-KMS-OPERATOR-SETUP-PACK`.
**Anchors:**
- `docs/MAINNET_KMS_VENDOR_SELECTION_DECISION.md` — preferred path AWS
  KMS asymmetric secp256k1; operator commercial sign-off OPEN.
- `docs/BACKEND_KMS_VENDOR_ADAPTER_IMPLEMENTATION_AWS_KMS_RESULT.md`
  — `AwsKmsSignerProvider` + `AwsKmsTransport` trait + mock.
- `docs/BACKEND_AWS_KMS_PRODUCTION_TRANSPORT_RESULT.md` — feature-gated
  `AwsKmsSdkTransport` over `aws_sdk_kms::Client`.
- `docs/BACKEND_AWS_KMS_CLOUDTRAIL_REQUEST_ID_RESULT.md` — real
  CloudTrail RequestId extraction + sanitiser.
- `docs/MAINNET_SIGNER_STAGING_REHEARSAL_PLAN.md` — 7-phase rehearsal
  ladder.
- `docs/MAINNET_SIGNER_ROTATION_AND_INCIDENT_RUNBOOK.md` — rotation +
  incident response.
- `docs/MAINNET_BE_SIGNER_SERVICE_DESIGN.md` — Pattern C signer
  microservice topology.
- `MAINNET_CUSTODY_POLICY.md §6 BE-5` — no raw key in backend memory.

## 0. Hard rules (this doc)

```text
no real AWS account creation             ✅
no real IAM role creation                ✅
no real KMS key creation                 ✅
no terraform apply                       ✅
no live AWS CLI against real account     ✅
no .env edit                             ✅
no chain tx                              ✅
no Safe tx                               ✅
no mainnet broadcast                     ✅
no canary broadcast                      ✅
no private custody roster disclosure     ✅
no real AWS account ID in this doc       ✅
no real KMS key ID in this doc           ✅
no real ARN in this doc                  ✅
no private signer identity in this doc   ✅
```

## 1. Current backend readiness summary

The backend layer is implementation-complete for AWS KMS signing:

* `PluggableSignerProvider` trait + `AwsKmsSignerProvider` adapter +
  `AwsKmsTransport` vendor-neutral trait (mock + production
  implementations).
* `AwsKmsSdkTransport` (real `aws-sdk-kms`-backed) shipped behind the
  `aws-kms-transport` Cargo feature flag; default builds DO NOT pull
  the AWS SDK.
* Real CloudTrail `RequestId` extraction via
  `aws_sdk_kms::operation::RequestId` with 5-step sanitisation +
  synthetic UUID fallback.
* SPKI parser + DER decoder + y_parity recovery + EIP-2 low-s
  validation.
* `SignerProviderKind::AwsKms` enum variant + mainnet refusal of
  `Mock` provider + 3 concentric mainnet defences against unsafe
  signing.
* `BACKEND_SIGNER_TIMEOUT_MS` typed config (default 2500 ms; range
  100..=30000).
* `/executor/health/v2.signer.*` block surfaces every signer-relevant
  state; `not_tracked_yet=[]`.
* 1053 backend tests green (1032 default + 21 feature-gated all-features).
* **Production `RemoteSignerClient::new` continues to use
  `UnimplementedTransport`** — no operational signing capability is
  enabled by code alone; operator-side AWS resources + explicit
  wiring at the rehearsal Phase 3 cutover are required.

What's missing on the operator side:

* No real AWS account configured in this repo context.
* No real KMS key created.
* No real IAM role created.
* No real signer microservice deployed.
* No CloudTrail trail configured.
* No real production signer EVM address derived or authorised.

This pack ships the **documentation + templates + runbooks** to close
the gap safely. The pack itself does NOT create any AWS resources.

## 2. Architecture decision checkpoint

The technical recommendation in
`MAINNET_KMS_VENDOR_SELECTION_DECISION.md §3` is AWS KMS asymmetric
secp256k1. This pack now addresses the next architectural question:
**does the backend talk to AWS KMS directly, or via a dedicated signer
microservice?**

### 2.1 Option A — Backend direct to AWS KMS via IAM role

```text
                                  AWS region
┌──────────────┐    mTLS+IAM    ┌──────────┐
│  backend     │────────────────│  AWS KMS │
│  (DeOpt)     │  STS short-    │  asymmetric
└──────────────┘  lived creds   │  secp256k1│
                                └──────────┘
```

| Aspect | Value |
|---|---|
| **Pros** | Lower latency (1 network hop); lower operational complexity (no microservice to deploy/monitor); cheaper. |
| **Cons** | Backend process holds AWS IAM identity + can issue Sign directly; less isolation. Custody-policy §6 BE-5 (no raw key) is satisfied because KMS is non-exportable, but BE-3 ("BE process MUST NOT hold any private-key material that could sign value-moving txs without an external policy gate") needs careful reasoning since the policy gate is the backend itself. |
| **IAM trust target** | The backend runtime (EC2 instance / EKS pod / ECS task / Fargate). |
| **Credential delivery** | IAM Role attached to the runtime; STS short-lived creds via instance-metadata / IRSA / task-role / OIDC. **NEVER AWS_ACCESS_KEY_ID in `.env`.** |
| **CloudTrail attribution** | All `Sign` events attributed to the backend runtime's role; harder to disambiguate from operator actions if the same role is shared. |
| **Suitable for** | Initial prod / beta when operator wants the fastest path to mainnet broadcast with lowest ops cost; the operator's auditor is satisfied with KMS non-extraction + IAM least-privilege + CloudTrail logging. |

### 2.2 Option B — Backend → signer microservice → AWS KMS (Pattern C)

```text
                  mTLS         IAM
┌──────────────┐  policy  ┌──────────┐  Sign  ┌──────────┐
│  backend     │──────────│ signer μs│────────│  AWS KMS │
│  (DeOpt)     │  signed  │ (Pattern │  IAM   │
└──────────────┘ approval │   C)     │  role  └──────────┘
              fingerprint └──────────┘
```

| Aspect | Value |
|---|---|
| **Pros** | Strongest trust boundary (signer is the only IAM identity with `kms:Sign`); independent crash domain; independent rotation cadence; per-request policy layer with allowlists; clean audit attribution; portable to other KMS vendors with no backend change. |
| **Cons** | Higher latency (2 network hops); higher operational complexity (additional service to deploy, monitor, and patch); higher cost. |
| **IAM trust target** | Only the signer microservice runtime. The backend's IAM identity has ZERO KMS permissions. |
| **Credential delivery** | IAM Role attached to the signer microservice runtime ONLY. Backend ↔ microservice talks mTLS with caller identity allowlist. |
| **CloudTrail attribution** | All `Sign` events attributed to the microservice's role; operator-side actions use a separate role. Clean separation. |
| **Suitable for** | Mature prod when operator wants stronger service separation + audit independence; the operator's auditor requires "the backend can ONLY ask the signer to sign; the backend cannot bypass the signer microservice's policy layer." Aligns with `MAINNET_BE_SIGNER_SERVICE_DESIGN.md` Pattern C decision. |

### 2.3 Recommendation

| Phase | Architecture |
|---|---|
| **Initial mainnet beta / launch** | **Option A — backend direct to AWS KMS via IAM role.** Operator achieves mainnet readiness fastest; ops surface is minimal; auditor checklist is short (KMS non-extraction + IAM least-privilege + CloudTrail logging). Acceptable per custody-policy §7.1 ("Pattern A is fastest path"). |
| **Mature prod / Q2-2026+** | **Option B — extract to signer microservice (Pattern C).** Adds policy-layer attestation + clean audit separation when the operator's compliance program + audit cadence matures. Migration is a backend code change (`RemoteSignerClient` already exposes the abstraction — swap the AWS KMS provider for a microservice client provider). |

Both options use the existing `AwsKmsSignerProvider` /
`AwsKmsSdkTransport` code path. The Option A → Option B migration
requires NO new backend module; only a constructor swap at
`RemoteSignerClient::new` once the microservice is operator-deployed.

### 2.4 Non-negotiables (BOTH options)

* No raw private key material in `.env`. KMS holds the key as
  non-exportable origin material.
* No `EXECUTOR_PRIVATE_KEY` set on mainnet (refused at startup by
  `validate_startup`).
* No `LocalDevSigner` on mainnet (refused at startup +
  `build_signer_for_state` runtime guard + `LocalDevSigner` runtime
  guard).
* No `Mock` provider on mainnet (refused at startup by
  `validate_signer_backend`).
* IAM role uses STS short-lived credentials via
  instance-metadata / IRSA / task-role / OIDC. NEVER long-lived
  access keys.
* CloudTrail enabled with `kms:Sign` + `kms:GetPublicKey` data
  events captured.
* Key rotation by creating NEXT key (never changing key spec).
* Emergency disable via a SEPARATE IAM role (not the signer
  runtime's role).

## 3. AWS KMS key requirements

The operator creates ONE asymmetric KMS key per environment
(sepolia + mainnet). Both keys use the SAME spec:

| Property | Required value |
|---|---|
| `KeySpec` | `ECC_SECG_P256K1` |
| `KeyUsage` | `SIGN_VERIFY` |
| `Origin` | `AWS_KMS` (non-exportable — default; do NOT use `EXTERNAL`) |
| `MultiRegion` | `false` (single-region; rotation is by creating NEXT key in same region) |
| Region (production) | EU recommended — `eu-central-1` (Frankfurt) preferred; alternatives `eu-west-1`, `eu-west-2`, `eu-west-3`, `eu-north-1` per
`MAINNET_KMS_VENDOR_SELECTION_DECISION.md §3.1` |
| Region (sepolia / sandbox) | Operator's discretion; SHOULD differ from mainnet region to prevent operational confusion |
| Auto-rotation | N/A — AWS KMS does NOT auto-rotate asymmetric keys. Rotation runbook at `MAINNET_SIGNER_ROTATION_AND_INCIDENT_RUNBOOK.md §2`. |
| Alias | Recommended: `alias/deopt-op-be-mainnet-<yyyy-mm>` (mainnet) and `alias/deopt-op-be-sepolia-<yyyy-mm>` (sepolia). Alias rotation = swap during rotation cycle. |

Expected calls from the production transport:

| API | Purpose | Frequency |
|---|---|---|
| `kms:GetPublicKey` | At startup `health_check`; at operator-side address derivation; at rotation cycle. | A few times per restart + once per rotation. |
| `kms:Sign` with `MessageType=Digest`, `SigningAlgorithm=ECDSA_SHA_256`, `Message=<32-byte prehash>` | Per broadcast attempt that reaches the signer call site. | Bounded by mainnet broadcast cadence; typically O(10-100) per day during normal operation. |

NO other KMS APIs are called by the backend at runtime.
`DescribeKey` / `ListAliases` / `EnableKey` / `DisableKey` /
`ScheduleKeyDeletion` are operator-only — see the IAM template
(`AWS_KMS_IAM_AND_KEY_POLICY_TEMPLATE.md`).

Ethereum address derivation flow:

1. Operator calls `kms:GetPublicKey` against the new KMS key (via AWS
   CLI / SDK in a controlled context, NOT from the backend at this
   stage).
2. AWS returns SPKI-encoded SubjectPublicKeyInfo containing an
   uncompressed SEC1 secp256k1 public key (`0x04 || X(32) || Y(32)`).
3. Operator extracts the 65-byte uncompressed pubkey from the SPKI's
   BIT STRING.
4. Operator computes `keccak256(pubkey[1..])` (skip the `0x04`
   prefix).
5. Operator takes the LAST 20 bytes of the keccak digest →
   `EXECUTOR_FROM_ADDRESS` (lowercase hex, `0x`-prefixed).
6. Operator records the EVM address publicly (it is NOT secret) +
   privately records the KMS key id / alias in the operator binder.

The backend adapter performs the SAME derivation at startup
`health_check` to cross-check that the configured `expected_address`
matches the KMS-derived address. Any mismatch → `PostSignFromMismatch`
→ broadcast fails closed before any signer call happens.

## 4. Files in this pack

| File | Purpose |
|---|---|
| `AWS_KMS_OPERATOR_SETUP_PACK.md` | This doc — architecture decision + readiness summary + cross-references. |
| `AWS_KMS_IAM_AND_KEY_POLICY_TEMPLATE.md` | Placeholder JSON templates: IAM role for signer runtime + KMS key policy + admin/operator separation + explicit deny list. |
| `AWS_KMS_SIGNER_RUNTIME_CONFIG_TEMPLATE.md` | Env config placeholders separated by layer (backend app / signer service / AWS IAM role); IRSA / instance-role preference; no `.env` long-lived creds. |
| `AWS_KMS_CLOUDTRAIL_AND_MONITORING_RUNBOOK.md` | CloudTrail data/management event setup + alert conditions + mapping to `/executor/health/v2` + `signer_denied_total` metric. |
| `AWS_KMS_SETUP_VALIDATION_CHECKLIST.md` | Pre-flight validations + no-broadcast rehearsal + Sepolia gate + mainnet read-only gate + go/no-go checklist. |
| `MAINNET_SIGNER_REHEARSAL_PHASE_2_NEXT_TASK.md` | Copy/paste-ready prompt for `MAINNET-SIGNER-REHEARSAL-PHASE-2-EXECUTION` — no-broadcast default + Sepolia variant. |

## 5. What's out of scope (this pack)

* Creating any real AWS account / IAM role / KMS key.
* Running any `terraform apply` against a real account.
* Running any AWS CLI command against a real account.
* Capturing the operator-selected AWS account ID / KMS key id / ARN
  in tracked docs. These live in the operator's offline binder per
  `MAINNET_CUSTODY_CLUSTER_2_RESOLUTION_REDACTED.md §3.5`.
* Final formal Q-CD-5 commercial / legal sign-off (per
  `MAINNET_KMS_VENDOR_SELECTION_DECISION.md §6`).
* Deploying any signer microservice (Pattern C track is operator-side
  per `MAINNET_BE_SIGNER_SERVICE_DESIGN.md`).
* Promoting `RemoteSignerClient::new` from `UnimplementedTransport` to
  `AwsKmsSdkTransport` — happens at rehearsal Phase 3 cutover per
  `MAINNET_SIGNER_STAGING_REHEARSAL_PLAN.md`.

## 6. Cross-links

* `docs/MAINNET_KMS_VENDOR_SELECTION_DECISION.md` — vendor decision +
  operator questions.
* `docs/MAINNET_SIGNER_VENDOR_ADAPTER_REQUIREMENTS.md` — adapter
  contract.
* `docs/MAINNET_SIGNER_STAGING_REHEARSAL_PLAN.md` — 7-phase rehearsal
  ladder.
* `docs/MAINNET_SIGNER_ROTATION_AND_INCIDENT_RUNBOOK.md` — rotation +
  incident response.
* `docs/BACKEND_KMS_VENDOR_ADAPTER_IMPLEMENTATION_AWS_KMS_RESULT.md`
  — backend adapter shape.
* `docs/BACKEND_AWS_KMS_PRODUCTION_TRANSPORT_RESULT.md` — feature-gated
  SDK transport.
* `docs/BACKEND_AWS_KMS_CLOUDTRAIL_REQUEST_ID_RESULT.md` — RequestId
  extraction.
* `docs/MAINNET_BE_SIGNER_SERVICE_DESIGN.md` — Pattern C topology.
* `MAINNET_CUSTODY_POLICY.md §6 BE-5 + §7.1 + §7.4` — custody rules.
