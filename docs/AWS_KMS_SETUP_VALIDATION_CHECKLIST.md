# AWS KMS setup validation checklist

**Posture:** CHECKLIST / DOC ONLY. Operator executes against a real
AWS account in a separately-authorised milestone
(`MAINNET-SIGNER-REHEARSAL-PHASE-2-EXECUTION`). This doc creates
ZERO AWS resources, runs ZERO live AWS CLI commands, sends ZERO
transactions.

**Anchors:**
- `docs/AWS_KMS_OPERATOR_SETUP_PACK.md` — architecture decision.
- `docs/AWS_KMS_IAM_AND_KEY_POLICY_TEMPLATE.md` — IAM + key policy.
- `docs/AWS_KMS_SIGNER_RUNTIME_CONFIG_TEMPLATE.md` — runtime config.
- `docs/AWS_KMS_CLOUDTRAIL_AND_MONITORING_RUNBOOK.md` — CloudTrail
  trail + alerts.
- `docs/MAINNET_SIGNER_STAGING_REHEARSAL_PLAN.md` — 7-phase rehearsal
  ladder.

## 0. Hard rules (this checklist)

```text
no mainnet broadcast                     ✅
no Sepolia live broadcast                ✅
no Safe tx                               ✅
no .env edit                             ✅
no real credentials printed              ✅
no real AWS account ID printed           ✅
no real KMS key id / ARN printed         ✅
no governance / Timelock / ownership /   ✅
  guardian mutation
```

## 1. Preflight — AWS account / region / key

| # | Check | Pass criterion |
|---|---|---|
| P1 | AWS account selected | Operator confirms ONE account dedicated to signer infrastructure; not shared with other workloads. |
| P2 | AWS region selected | EU region preferred per `MAINNET_KMS_VENDOR_SELECTION_DECISION.md §3.1`. Recorded in operator binder. |
| P3 | KMS key created | `aws kms describe-key --key-id <KEY>` returns: `KeySpec=ECC_SECG_P256K1`, `KeyUsage=SIGN_VERIFY`, `Origin=AWS_KMS`, `KeyState=Enabled`. |
| P4 | KMS key region matches | Returned `Arn` region == operator's selected region. |
| P5 | KMS key has alias | `aws kms list-aliases --key-id <KEY>` returns expected alias. |
| P6 | KMS key auto-rotation NOT enabled | AWS KMS does NOT auto-rotate asymmetric keys; if the operator wired any rotation automation, confirm it's a no-op. |

## 2. Preflight — public key + address derivation

| # | Check | Pass criterion |
|---|---|---|
| P7 | GetPublicKey returns SPKI | `aws kms get-public-key --key-id <KEY>` returns a Blob whose ASN.1 outer SEQUENCE parses cleanly. |
| P8 | SPKI carries secp256k1 OID | The algorithm SEQUENCE includes `1.3.132.0.10` (secp256k1). |
| P9 | SEC1 uncompressed public key | BIT STRING content length == 65; first byte == `0x04`. |
| P10 | EVM address derived offline | `keccak256(pubkey[1..])[12..]` formatted as `0x<40-hex>` lowercase. |
| P11 | Derived address matches `EXECUTOR_FROM_ADDRESS` | Cross-check at backend startup `health_check` — fails closed otherwise. |
| P12 | Operator binder records key id ↔ address mapping | Offline-only; tracked docs MUST NOT contain the production address. |

## 3. Preflight — IAM identity separation

| # | Check | Pass criterion |
|---|---|---|
| P13 | `<SIGNER_RUNTIME_ROLE_NAME>` exists | `aws iam get-role --role-name <…>` returns the role. |
| P14 | `<KMS_ADMIN_ROLE_NAME>` exists | Separate from runtime role. |
| P15 | Runtime role can `kms:GetPublicKey` | Simulated via `aws iam simulate-principal-policy` with action `kms:GetPublicKey` against `<KMS_KEY_ARN>` → `allowed`. |
| P16 | Runtime role can `kms:Sign` (with EcdsaSha256 condition) | Simulated with action `kms:Sign` + context `kms:SigningAlgorithm = ECDSA_SHA_256` → `allowed`. |
| P17 | Runtime role CANNOT `kms:Sign` with another algo | Simulated with action `kms:Sign` + context `kms:SigningAlgorithm = ECDSA_SHA_384` → `denied` (condition mismatch). |
| P18 | Runtime role CANNOT `kms:DisableKey` | Simulated → `denied`. |
| P19 | Runtime role CANNOT `kms:ScheduleKeyDeletion` | Simulated → `denied`. |
| P20 | Runtime role CANNOT `kms:PutKeyPolicy` | Simulated → `denied`. |
| P21 | Runtime role CANNOT `iam:*` | Simulated → `denied`. |
| P22 | Admin role CAN `kms:*` administrative actions | Simulated → `allowed`. |
| P23 | Admin role bound to operator SSO + MFA | Operator confirms. |
| P24 | NO runtime + admin role overlap | Operator confirms — different IAM principals. |

## 4. Preflight — CloudTrail

| # | Check | Pass criterion |
|---|---|---|
| P25 | CloudTrail trail exists | `aws cloudtrail describe-trails --trail-name-list <CLOUDTRAIL_TRAIL_NAME>` returns the trail. |
| P26 | Trail captures management events | `IsMultiRegionTrail`/`IsOrganizationTrail` set per operator policy; `IncludeManagementEvents=true`. |
| P27 | Trail captures KMS data events | `aws cloudtrail get-event-selectors --trail-name <…>` shows `dataResources.type = AWS::KMS::Key` + `values = <KMS_KEY_ARN>`. |
| P28 | Log file validation enabled | `LogFileValidationEnabled=true`. |
| P29 | S3 bucket has Object Lock | `aws s3api get-object-lock-configuration --bucket <…>` returns config. |
| P30 | S3 bucket KMS-encrypted with DIFFERENT key | `aws s3api get-bucket-encryption` returns SSE-KMS with a key id != `<KMS_KEY_ID_OR_ALIAS>`. |
| P31 | Retention ≥ 7 years | Lifecycle policy confirmed. |
| P32 | CloudWatch / SIEM forwarding active | Operator confirms by injecting a no-op management event (`DescribeKey`) and observing it land in CloudWatch Logs within 5 minutes. |

## 5. Preflight — backend / signer-runtime wiring

| # | Check | Pass criterion |
|---|---|---|
| P33 | Backend runtime has IAM role attached | EC2 instance profile / EKS IRSA / ECS task role. |
| P34 | Backend env DOES NOT contain AWS keys | `grep -E 'AWS_ACCESS_KEY_ID\|AWS_SECRET_ACCESS_KEY\|AWS_SESSION_TOKEN' <env>` returns nothing. |
| P35 | Backend env DOES NOT contain `EXECUTOR_PRIVATE_KEY` | `grep EXECUTOR_PRIVATE_KEY <env>` returns nothing. |
| P36 | Backend env contains `BACKEND_SIGNER_MODE=remote` | Confirmed. |
| P37 | Backend env contains `BACKEND_REMOTE_SIGNER_PROVIDER=aws_kms` | Confirmed. |
| P38 | Backend env contains `BACKEND_SIGNER_ENDPOINT=<URL>` | Non-empty; no embedded credentials. |
| P39 | Backend env contains `BACKEND_SIGNER_TIMEOUT_MS=2500` | Default; or operator-tuned value in 100..=30000. |
| P40 | Backend env contains correct `EXECUTOR_FROM_ADDRESS` | Matches the KMS-derived address. |
| P41 | Backend startup passes `validate_signer_backend` | No `BackendError::Config` on startup; LocalDev refused on mainnet; Mock refused on mainnet. |
| P42 | Backend `cargo build --features aws-kms-transport` succeeds | Compiles cleanly. |
| P43 | Production `RemoteSignerClient::new` continues using `UnimplementedTransport` | Confirmed via code review; promotion happens at rehearsal Phase 3. |

## 6. No-broadcast rehearsal — Phase 2 dry run

This is the canonical Phase 2 acceptance from
`MAINNET_SIGNER_STAGING_REHEARSAL_PLAN.md §2.2`. **NO transactions
sent on any chain.**

| # | Step | Pass criterion |
|---|---|---|
| R1 | Operator calls `GetPublicKey` via the AWS SDK (NOT from production backend) | Returns the SPKI bytes; operator records. |
| R2 | Operator derives EVM address offline | Matches `EXECUTOR_FROM_ADDRESS`. |
| R3 | Operator calls `Sign` against a fixed-bytes test prehash (e.g. `keccak256("deopt-no-broadcast-rehearsal")`) | Returns a DER signature; operator records. |
| R4 | Operator parses DER + recovers y_parity offline | Verifies recovered address matches `EXECUTOR_FROM_ADDRESS`. |
| R5 | CloudTrail records both events with attributable IAM principal | `Sign` event userIdentity.arn == `<SIGNER_RUNTIME_PRINCIPAL_ARN>`. `GetPublicKey` event recorded. |
| R6 | Backend `cargo test --all-targets --all-features` green | 1053 tests; no regression. |
| R7 | Backend `/executor/health/v2.signer.signer_address` (when running with feature + adapter wired) matches derived address | Cross-check pin. |
| R8 | Backend `/metrics` shows no `signer_denied_total` increments | Clean snapshot. |
| R9 | NO Sepolia broadcast | Confirmed. |
| R10 | NO mainnet broadcast | Confirmed. |
| R11 | NO Safe-tx execution | Confirmed. |
| R12 | NO `.env` edit | Confirmed. |
| R13 | No fallback to `LocalDevSigner` triggered | `signer.local_signer_on_mainnet_refused_total == 0`. |
| R14 | No `Mock` provider seated on mainnet | Startup refused; operator confirms by intentionally setting `BACKEND_REMOTE_SIGNER_PROVIDER=mock` + `EXECUTOR_CHAIN_ID=8453` in a test instance and observing the refusal. |

## 7. Sepolia rehearsal gate (Phase 3 — LATER milestone)

This is OUT OF SCOPE for the current pack. The Phase 3 milestone
(`MAINNET-SIGNER-REHEARSAL-PHASE-3-EXECUTION`, future) explicitly
authorises a single Sepolia broadcast through the production-shape
AWS KMS signing path. Until that milestone fires:

* NO Sepolia transaction sent by THIS pack or by Phase 2.
* If the operator wants an early Sepolia smoke against the AWS KMS
  path, they invoke the existing Sepolia rehearsal arc separately
  (sepolia rehearsal is closed per current state) with a SEPOLIA-only
  KMS key — never the mainnet key.

## 8. Mainnet read-only gate

Before any mainnet broadcast (Phase 7 of the staging plan):

| # | Check | Pass criterion |
|---|---|---|
| M1 | Mainnet RPC URL configured (read-only) | Confirmed. |
| M2 | `GetPublicKey` against MAINNET KMS key returns correct address | Matches derived. |
| M3 | `/executor/health/v2.overall_status == "green"` | Confirmed. |
| M4 | `live_provider_config.protocol_fee_vault_configured == true` | Per `BACKEND_VAULT_OBSERVABILITY_USE_TYPED_CONFIG_RESULT.md`. |
| M5 | `live_provider_config.fees_manager_v2_configured == true` | Confirmed. |
| M6 | `live_provider_config.collateral_vault_configured == true` | Confirmed. |
| M7 | `last_r5_drift_zero == Some(true)` | R5 launch invariant satisfied. |
| M8 | NO mainnet transaction sent by THIS gate | Confirmed. |
| M9 | OPS Safe ready to execute `setExecutor(<address>)` in a SEPARATE authorised operation | Operator confirms. |

## 9. Go / no-go checklist

A phase is **GO** when every check in the relevant section above is
GREEN. A single RED check → **NO-GO**; operator rolls back per
`MAINNET_SIGNER_STAGING_REHEARSAL_PLAN.md §3`.

### 9.1 Phase 2 GO

* §1 P1-P6 all GREEN.
* §2 P7-P12 all GREEN.
* §3 P13-P24 all GREEN.
* §4 P25-P32 all GREEN.
* §5 P33-P43 all GREEN.
* §6 R1-R14 all GREEN.
* Operator authorisation captured.

### 9.2 Phase 3 GO (Sepolia broadcast — LATER)

* Phase 2 GO captured.
* §7 conditions met.
* Sepolia-only KMS key + IAM role provisioned distinct from mainnet.
* Operator authorisation captured per
  `BACKEND_SIGNER_CUTOVER_RUNBOOK_V2G_FX_Q1.md`.

### 9.3 Phase 7 GO (mainnet preparation — LATEST)

* All prior phases GO.
* §8 M1-M9 all GREEN.
* OPS Safe signers staffed + on-call rotation staffed.
* Rollback rehearsed.
* Operator final authorisation captured.

## 10. Cross-links

* `docs/AWS_KMS_OPERATOR_SETUP_PACK.md` — architecture decision.
* `docs/AWS_KMS_IAM_AND_KEY_POLICY_TEMPLATE.md` — IAM + key policy.
* `docs/AWS_KMS_SIGNER_RUNTIME_CONFIG_TEMPLATE.md` — runtime config.
* `docs/AWS_KMS_CLOUDTRAIL_AND_MONITORING_RUNBOOK.md` — CloudTrail
  trail.
* `docs/MAINNET_SIGNER_STAGING_REHEARSAL_PLAN.md` — 7-phase rehearsal.
* `docs/MAINNET_SIGNER_REHEARSAL_PHASE_2_NEXT_TASK.md` — Phase 2
  prompt.
