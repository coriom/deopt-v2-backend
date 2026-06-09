# Mainnet custody — Cluster 2 next actions

**Posture:** READ-ONLY dependency / next-action doc. **No chain
mutation. No `.env` edit. No Safe-tx. No broadcast. No mainnet. No
KMS key creation. No vendor account creation.** Companion to
`~/DEOPT/MAINNET_CUSTODY_DECISIONS_ADDENDUM_TEMPLATE.md`,
`deopt-v2-backend/docs/MAINNET_CUSTODY_CLUSTER_2_RESOLUTION_REDACTED.md`,
and `deopt-v2-backend/docs/MAINNET_CUSTODY_DECISION_DEPENDENCY_MAP.md`.

**Date:** 2026-06-09
**Cluster 2 status:** **RESOLVED (architecture + policy) — vendor / region / provider-detail PENDING.**

---

## 0. Hard stops (this doc)

```text
no chain tx                                       ✅
no Safe tx                                        ✅
no .env edit                                      ✅
no broadcast                                      ✅
no mainnet                                        ✅
no KMS / HSM / MPC key creation                   ✅
no vendor account creation                        ✅
no IAM role provisioning                          ✅
no mainnet BE EOA address recorded                ✅
no vendor credentials / API keys recorded         ✅
```

---

## 1. Cluster 2 closure recap

| Q-CD | Closure | Authority |
|---|---|---|
| Q-CD-5 | **ARCH-PATTERN-DECIDED: Pattern C** (dedicated backend signer microservice backed by HSM/MPC or KMS, with strict signing policy layer) | Operator + Security + Backend |
| Q-CD-6 | **OPERATOR-DECIDED: distinct EOAs.** OPTION BE provisioned at launch; PERP BE deferred until perp scaffold real-broadcast path exists | Operator + Backend |
| Q-CD-14 | **RECOMMENDED-PENDING-PROVIDER: EU primary + EU/nearby secondary.** Exact regions follow vendor. | Operator + Security + DevOps |
| Q-CD-15 | **POLICY-DECIDED-PROVIDER-DETAIL-PENDING.** Disable ≠ delete; ≥ 2 approvals + governance for permanent deletion; emergency IAM revoke faster than disable. | Operator + Security |

Public redacted summary:
`deopt-v2-backend/docs/MAINNET_CUSTODY_CLUSTER_2_RESOLUTION_REDACTED.md`.
Private artefact:
`~/DEOPT/private/mainnet_custody/MAINNET_CUSTODY_CLUSTER_2_RESOLUTION.private.md` (mode 600; sha256 in `CLUSTER_HASHES.txt`).

---

## 2. What Cluster 2 unblocks

### 2.1 Planning unblocks

| Target | Status now |
|---|---|
| V2G-Y Y-F planning (NEW_OME executor migration to mainnet BE) | parameterised at the architecture layer; execution still pending impl |
| AUDIT-EXT trust-boundary review scope | crystallised — auditor reviews signer microservice + §6.6 policy + IAM minimality |
| Backend code path planning for the mainnet signer interface | clear — `RemoteSigner` trait + `KmsRemoteSigner` impl + `EXECUTOR_PRIVATE_KEY` refusal on chain_id 8453 |
| Mainnet BE custody policy (rotation / disable / delete) | clear at policy level — provider-specific procedures pending vendor |
| Mainnet manifest planning for executor + KMS handle slots | clear shape — addresses pending KMS provisioning |

### 2.2 Items NOT unblocked

| Target | Remaining gate |
|---|---|
| Y-F execution | KMS key not provisioned; signer service not built; backend `RemoteSigner` not merged; Sepolia integration test not run |
| Mainnet BE EOA address | requires Q-CD-5 vendor selected + Q-CD-14 region locked + KMS key generated inside KMS |
| TREASURY funding of BE | Cluster 3 Q-CD-7 (TREASURY Safe) and Q-CD-9 (fund thresholds) still OPEN |
| `BACKEND-SHOULD-BROADCAST-ECONOMIC-GATE` | Independent of Cluster 2; can start now per gap-list C-4 |

---

## 3. Required follow-up milestones

### 3.1 `MAINNET-BE-SIGNER-SERVICE-DESIGN` (read-only design)

Backend + Security. Output: design doc covering:
- mTLS server / client topology (HTTPS or gRPC).
- Per-sign request / response schema.
- §6.6 transaction policy precheck layer (chainId allowlist `{8453}`, target allowlist `{NEW_OME_MAINNET}`, selector allowlist `{executeTrade, executeRfqTrade}`, max value `0`, max gas, nonce, rate limit).
- Vendor adapter interface (KMS vs HSM vs MPC behind the service).
- Structured per-sign log shape with `request_id` propagation.
- Failover client to secondary region.
- Emergency disable / pause endpoint (IAM-gated).
- Health endpoint.
- Deployment shape (VPC-isolated; no public ingress; service-to-service authn via mTLS + IAM).
- Versioning + rollback policy.
- Acceptance criteria for `BACKEND-SIGNER-INTERFACE-KMS-HSM-ADAPTER`.

### 3.2 `BACKEND-SIGNER-INTERFACE-KMS-HSM-ADAPTER` (code implementation)

Backend. Output: PR adding:
- `RemoteSigner` trait in `src/execution/signer.rs`.
- Existing `ExecutorSigner::from_private_key` retained for Sepolia / tests.
- `KmsRemoteSigner::from_service_endpoint(endpoint)` impl (or named accordingly per design doc) for mainnet.
- New env keys: `BACKEND_SIGNER_ENDPOINT`, `BACKEND_SIGNER_CLIENT_CERT_PATH`, `BACKEND_SIGNER_CLIENT_KEY_PATH`, `BACKEND_SIGNER_CA_CERT_PATH`.
- Startup guard: **REFUSE `EXECUTOR_PRIVATE_KEY` on `chain_id = 8453`**.
- Startup guard: require `BACKEND_SIGNER_ENDPOINT` when `EXECUTOR_REAL_BROADCAST_ENABLED=true` AND mainnet.
- Wire trait into option execution path (`src/options/service.rs:1166` and `:1213` call sites).
- Unit + integration tests (Sepolia end-to-end through the new path).

### 3.3 `BACKEND-SHOULD-BROADCAST-ECONOMIC-GATE` (code implementation)

Backend + Risk. Independent of Cluster 2 but required pre-mainnet (gap-list C-4 / W-3). Output: PR implementing `should_broadcast` per `BACKEND_GAS_FEES_REBATES_POLICY_V1.md §8`:
- Rebate-solvency hard gate (§4.2).
- Wash-trade detection (§6).
- Persistent dedupe cache + nonce-window store (T-3).
- Subsidy budget registry + per-reason cap + 1h burn alert (T-4).
- Unit tests for every §8 branch (T-10).

### 3.4 `BACKEND-SIGNER-AUDIT-LOGS-AND-ALERTS` (monitoring)

Backend + SRE. Output:
- Structured per-sign log emitted by backend with `request_id` from signer service.
- Sign-rate alerts (rate spike → page).
- IAM revoke event alert (audit-side).
- KMS sign-failure alert (signer service rejects per §6.6 → page).
- Failover event alert (primary → secondary region transition).
- Two-layer audit log correlation (signer service log + downstream KMS / HSM log + backend log + chain receipt).

### 3.5 `MAINNET-KMS-VENDOR-SELECTION` (operator decision)

Operator + Security + Backend. Output: vendor name + IAM tier + contract + access. **Recorded in offline binder, NOT in any tracked doc.** Triggers `MAINNET-KMS-REGION-FINALISATION`.

### 3.6 `MAINNET-KMS-REGION-FINALISATION` (operator decision)

Operator + Security + DevOps. Output: exact primary + secondary regions. **Recorded in offline binder + private addendum**, NOT in any tracked doc.

---

## 4. Dependency reference (updated)

Cluster 2 dependency status in
`deopt-v2-backend/docs/MAINNET_CUSTODY_DECISION_DEPENDENCY_MAP.md`
§4 (Q-CD → KMS / backend-implementation unlocks):

| Q-CD | Was | Now |
|---|---|---|
| Q-CD-5 | HARD GATE for KMS impl | **ARCH-PATTERN-DECIDED: Pattern C** — backend impl can start; vendor sub-decision still gates key generation |
| Q-CD-14 | HARD GATE for KMS impl | **STRUCTURE-DECIDED: EU primary + EU/nearby secondary** — backend impl can model failover; exact regions still pending |
| Q-CD-15 | required for IAM provisioning | **POLICY-DECIDED** — IAM policy can be drafted at the structural layer; exact JSON pending vendor |
| Q-CD-6 | drives KMS key-count | **DECIDED: distinct EOAs** — provision 1 OPTION key at launch; PERP key deferred |
| Q-CD-16 (rotation cadence) | operational runbook | **PRE-RESOLVED in custody policy §9.1: ≤ 30 days** — provider-specific workflow detail still pending |
| Q-CD-13 (Sepolia rehearsal) | Sepolia integration test prior to mainnet code activation | **POLICY: TRUE per Cluster 1 closure** — applies to signer-service Sepolia rehearsal too |

---

## 5. Manifest implications (delta from Cluster 1)

Cluster 1 unblocked 13 Group A slots (governance + guardians) with OPS / GOV Safe addresses. Cluster 2 unblocks the **planning** of two more slots — values still pending vendor + KMS provisioning:

| Slot (line) | Pre-Cluster-2 status | Post-Cluster-2 status |
|---|---|---|
| `matchingExecutors.options[0].executor` (114) | `NEEDS_OPERATOR_DECISION + NEEDS_DEPLOYMENT` | architecture decided (Pattern C); value pending KMS provisioning |
| `matchingExecutors.perps[0].executor` (120) | `NEEDS_OPERATOR_DECISION + NEEDS_DEPLOYMENT` | DEFERRED per Q-CD-6 (perp scaffold not real-broadcast capable) |
| (new schema add) `governanceRoles.kmsKeyHandles.optionBackendExecutor` | proposed in custody-policy §13.3 | **structurally clear** — slot shape locked; handle value pending vendor |
| (new schema add) `governanceRoles.kmsKeyHandles.optionBackendExecutorNext` | proposed | **structurally clear** — warm spare per custody-policy D-7 |

Manifest schema-extension PR (per Cluster 1 next-actions item) should
now add the KMS handle slots in addition to the TREASURY + breakGlass
+ custodyPolicyVersion slots already identified.

---

## 6. Audit implications (delta)

`MAINNET_AUDIT_EXT_ENGAGEMENT_PACKAGE.md §4.10` and §7 Q-26..Q-30 are already in scope. Cluster 2 closure adds:

- Auditor reviews **Pattern C selection rationale** (vs A / B / D) recorded in the Cluster 2 redacted summary §1.2.
- Auditor reviews **distinct option / perp EOA decision** (Q-CD-6 reasoning §2.2).
- Auditor reviews **region failover semantics** (Q-CD-14) at the structural layer.
- Auditor reviews **key-deletion approval lock** (Q-CD-15) at the policy layer.
- Auditor confirms backend startup guard refuses `EXECUTOR_PRIVATE_KEY` on `chain_id = 8453` once the `BACKEND-SIGNER-INTERFACE-KMS-HSM-ADAPTER` PR lands.

---

## 7. V2G-Y implications (delta)

Cluster 1 fully parameterised Y-A and Y-G-1..6. Cluster 2 fully
parameterises the **planning** of Y-F at the architecture level. Y-F
**execution** is still blocked on:

- `BACKEND-SIGNER-INTERFACE-KMS-HSM-ADAPTER` merged.
- `MAINNET-BE-SIGNER-SERVICE-DESIGN` shipped.
- `MAINNET-KMS-VENDOR-SELECTION` resolved.
- `MAINNET-KMS-REGION-FINALISATION` resolved.
- KMS key provisioned inside KMS; BE EOA derived.
- Signer microservice deployed (primary + secondary regions).
- Sepolia integration test green through the new signer path.
- `MAINNET_FIRST_LIVE_SMOKE_AUTHORIZATION` 4-signature attestation (operator + security + risk + audit).

---

## 8. Cluster 3 + 4 (remaining clusters)

Cluster 3 (Q-CD-7 / Q-CD-8 / Q-CD-9) is the recommended **next** Cluster milestone after Cluster 2:

- **Q-CD-7** TREASURY Safe form — without TREASURY, BE cannot be funded.
- **Q-CD-8** DEPLOYER form — drives `chainMetadata.deployer` manifest slot + retirement plan.
- **Q-CD-9** BE FUND_FLOOR / TARGET / CEILING — drives monitoring alert thresholds and TREASURY refresh cadence.

Cluster 4 (Q-CD-10 / 11 / 12 / 16 / 17 / 18) covers PFV revenue receiver, rebates, insurance, cadences, policy version.

---

## 9. Files produced / updated by this milestone

| Path | Status |
|---|---|
| `~/DEOPT/private/mainnet_custody/MAINNET_CUSTODY_CLUSTER_2_RESOLUTION.private.md` | **CREATED** (mode 600, outside all repo trees) |
| `~/DEOPT/private/mainnet_custody/CLUSTER_HASHES.txt` | **APPENDED** (Cluster 2 sha256 entry) |
| `deopt-v2-backend/docs/MAINNET_CUSTODY_CLUSTER_2_RESOLUTION_REDACTED.md` | **CREATED** (public redacted summary) |
| `deopt-v2-backend/docs/MAINNET_CUSTODY_CLUSTER_2_NEXT_ACTIONS.md` | **CREATED** (this file) |
| `deopt-v2-backend/docs/MAINNET_CUSTODY_DECISION_DEPENDENCY_MAP.md` | **UPDATED** (Cluster 2 row status) |
| `~/DEOPT/RUN_STATE.md` | **APPENDED** (redacted closure note) |

**No source touched. No `.env` edit. No chain mutation. No Safe-tx. No KMS key. No vendor account.**

---

## 10. Next milestone recommendation

Primary recommendation: **`MAINNET-CUSTODY-CLUSTER-3-RESOLUTION`** (Q-CD-7 / Q-CD-8 / Q-CD-9). Unlocks TREASURY operational flow + DEPLOYER manifest slot + BE funding thresholds.

In parallel:
1. **`MAINNET-BE-SIGNER-SERVICE-DESIGN`** — read-only design milestone.
2. **`MAINNET-KMS-VENDOR-SELECTION`** — Q-CD-5 sub-decision.
3. **`BACKEND-SHOULD-BROADCAST-ECONOMIC-GATE`** — gap-list C-4; independent of Cluster 2.
4. **`MAINNET-AUDIT-EXT-KICKOFF`** — ship handoff bundle including Cluster 1 + Cluster 2 redacted closure summaries.

---

## 11. Cross-links

- `~/DEOPT/MAINNET_CUSTODY_POLICY.md` §6 / §7 / §13 / §14
- `~/DEOPT/MAINNET_CUSTODY_DECISIONS_ADDENDUM_TEMPLATE.md` Q-CD-5 / 6 / 14 / 15
- `~/DEOPT/deopt-v2-backend/docs/MAINNET_CUSTODY_DECISION_DEPENDENCY_MAP.md` §4
- `~/DEOPT/deopt-v2-backend/docs/MAINNET_CUSTODY_CLUSTER_2_RESOLUTION_REDACTED.md`
- `~/DEOPT/deopt-v2-backend/docs/MAINNET_CUSTODY_CLUSTER_1_RESOLUTION_REDACTED.md`
- `~/DEOPT/deopt-v2-backend/docs/MAINNET_CUSTODY_CLUSTER_1_NEXT_ACTIONS.md`
- `~/DEOPT/deopt-v2-backend/docs/P0_MAINNET_BLOCKER_CLOSURE_ROADMAP.md`
- `~/DEOPT/deopt-v2-sol/docs/MAINNET_V2G_Y_OWNERSHIP_MIGRATION_PLAN.md` §4 Y-F
- `~/DEOPT/deopt-v2-sol/docs/MAINNET_AUDIT_EXT_ENGAGEMENT_PACKAGE.md`
- `~/DEOPT/BACKEND_EXECUTOR_CUSTODY.md`
- `~/DEOPT/deopt-v2-backend/docs/BACKEND_SIGNER_CUTOVER_RUNBOOK_V2G_FX_Q1.md` §13
- `~/DEOPT/deopt-v2-backend/docs/BACKEND_EXECUTOR_MONITORING_ALERTS_V1.md`
- `~/DEOPT/RUN_STATE.md`

**End of Cluster 2 next-actions doc.**
