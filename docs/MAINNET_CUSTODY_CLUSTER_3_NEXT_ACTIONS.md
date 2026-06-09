# Mainnet custody — Cluster 3 next actions

**Posture:** READ-ONLY dependency / next-action doc. **No chain
mutation. No `.env` edit. No Safe-tx. No broadcast. No mainnet. No
Treasury Safe creation. No fund movement.** Companion to
`~/DEOPT/MAINNET_CUSTODY_DECISIONS_ADDENDUM_TEMPLATE.md`,
`deopt-v2-backend/docs/MAINNET_CUSTODY_CLUSTER_3_RESOLUTION_REDACTED.md`,
and `deopt-v2-backend/docs/MAINNET_CUSTODY_DECISION_DEPENDENCY_MAP.md`.

**Date:** 2026-06-09
**Cluster 3 status:** **POLICY + FORMULA DECIDED**; Treasury Safe identity, numeric BE funding parameters, and pre-migration DEPLOYER form (Q-CD-8a) PENDING follow-up milestones.

---

## 0. Hard stops (this doc)

```text
no chain tx                                        ✅
no Safe tx                                         ✅
no Treasury Safe creation                          ✅
no .env edit                                       ✅
no broadcast                                       ✅
no mainnet                                         ✅
no fund movement                                   ✅
no Treasury signer identities recorded             ✅
no Treasury Safe address recorded                  ✅
no mainnet BE EOA recorded                         ✅
no numeric FUND_FLOOR / TARGET / CEILING values    ✅
no operator hot-balance cap                        ✅
no vendor credentials / API keys                   ✅
```

---

## 1. Cluster 3 closure recap

| Q-CD | Closure | Authority |
|---|---|---|
| Q-CD-7 | **POLICY-DECIDED**: separate Safe v1.4.1 ≥ 3-of-5; hard disjoint from OPS; default partial separation from GOV; no DEPLOYER as owner | Operator + Treasury |
| Q-CD-8 | **POLICY-DECIDED**: post-V2G-Y DEPLOYER = deployment provenance only; no protocol role; no Safe ownership; V2G-Y hard stops added (§2.4 in redacted) | Operator |
| Q-CD-9 | **FORMULA-DECIDED-PARAMETERS-PENDING-OPERATOR-FILL**: `FUND_FLOOR = max(7d gas, emergency rotation)`, `FUND_TARGET = 3× FLOOR`, `FUND_CEILING = min(10× FLOOR, op cap)`; monthly recompute | Operator + Finance + Backend |

Public redacted summary: `deopt-v2-backend/docs/MAINNET_CUSTODY_CLUSTER_3_RESOLUTION_REDACTED.md`.
Private artefact: `~/DEOPT/private/mainnet_custody/MAINNET_CUSTODY_CLUSTER_3_RESOLUTION.private.md` (mode 600).
sha256 anchor: `2962a46a7be7ce016cb16dca579722c62faee99ddaac8a57d22a0779eb8b416e`.

---

## 2. What Cluster 3 unblocks

### 2.1 Planning unblocks

| Target | Status |
|---|---|
| `MAINNET-TREASURY-SAFE-CREATION-PACKET` planning | architecture + policy locked; identity decisions queued |
| `MAINNET-BE-FUNDING-POLICY-PARAMETER-FILL` planning | formula locked; awaiting first numeric fill |
| Monitoring threshold ladder | per Cluster 3 §3.3 (FUND_TARGET / FUND_FLOOR / emergency_floor / FUND_CEILING + daily drift) |
| Signer-service §6.6 drain-back-to-Treasury policy | added to `MAINNET-BE-SIGNER-SERVICE-DESIGN` Cluster 2 follow-up |
| V2G-Y hard stops for DEPLOYER retirement | added to V2G-Y per-phase verification |
| Custody-policy §9.3 retirement procedure | confirmed canonical |

### 2.2 Items NOT unblocked by Cluster 3

| Target | Remaining gate |
|---|---|
| Treasury Safe deployed | requires `MAINNET-TREASURY-SAFE-CREATION-PACKET` (operator-authorised broadcast) |
| BE first refresh | requires Treasury Safe deployed AND mainnet BE EOA derived (Cluster 2 vendor selection + KMS key generation) |
| `chainMetadata.deployer` manifest slot fillable | depends on Q-CD-8a pre-migration DEPLOYER form decision |
| `governanceRoles.treasury` manifest slot fillable | depends on Treasury Safe deployment |
| Insurance funding | still pending Q-CD-12 sizing + Q-CD-17 operator form |
| Rebate-reserve allocation | still pending Q-CD-11 enable/defer decision |
| `BACKEND-SHOULD-BROADCAST-ECONOMIC-GATE` | gap-list C-4; independent of Cluster 3; can start now |

---

## 3. Required follow-up milestones

### 3.1 `MAINNET-TREASURY-SAFE-CREATION-PACKET` (operator-authorised broadcast packet)

Operator + Finance + Treasury. Output:
- Roster identification (5 named human signers in offline binder; placeholder labels in repo).
- Disjointness confirmation vs OPS_SAFE_MAINNET (hard) and GOV_SAFE_MAINNET (default partial separation).
- Sepolia rehearsal analogue per Q-CD-13 — each Treasury signer test-signs on Sepolia first.
- Safe v1.4.1 SafeL2 deployment plan (CREATE2 address pre-derived).
- Pre-broadcast bytecode verification.
- Post-broadcast read-only verification: `VERSION()=1.4.1`, `getThreshold()=3`, `getOwners() count=5`, `nonce()=0`, `isOwner(DEPLOYER)=false`, roster non-overlap vs OPS confirmed on chain.
- Address recorded in private addendum + manifest schema slot `governanceRoles.treasury`.

### 3.2 `MAINNET-BE-FUNDING-POLICY-PARAMETER-FILL` (operator + Finance + Backend)

Operator + Finance + Backend. Output:
- First numeric fill of the 5 input values from Cluster 3 §3.2 formula.
- Computed FUND_FLOOR_wei / FUND_TARGET_wei / FUND_CEILING_wei.
- Monitoring threshold values committed to alert rule deployment.
- First monthly recompute scheduled.
- Private addendum stored under `~/DEOPT/private/mainnet_custody/` (mode 600).

### 3.3 `MAINNET-MANIFEST-FILL-GOV-OPS-TREASURY-SLOTS` (manifest-fill PR)

Deployment Owner + Ops. Output:
- Read-only manifest update PR filling the 13 Group A slots unblocked by Cluster 1 (governance owners + Timelock roles + module guardians) with OPS_SAFE_MAINNET / GOV_SAFE_MAINNET.
- Schema extension PR adding: `governanceRoles.treasury`, `governanceRoles.deployerRetirementStatus`, `governanceRoles.kmsKeyHandles.optionBackendExecutor`, `governanceRoles.kmsKeyHandles.optionBackendExecutorNext`, `funding.backendExecutor.fundFloorWei / .fundTargetWei / .fundCeilingWei / .recomputeCadenceMonths`, `custodyPolicyVersion`, PFV-side slots from custody policy §13.3.
- Treasury slot filled with `TODO_REPLACE_*` annotation `pending MAINNET-TREASURY-SAFE-CREATION-PACKET` until §3.1 closes.
- No actual mainnet contract addresses (those depend on `DeployCore.s.sol`).

### 3.4 `MAINNET-DEPLOY-CEREMONY-DESIGN` (operator decision)

Operator + Security. Output: pre-migration DEPLOYER form decision (Q-CD-8a) — dedicated deploy Safe (≥ 2-of-3) recommended OR hardware-wallet EOA. Recorded in offline binder + private addendum.

### 3.5 Operational track also unblocked

- Treasury operational log shape + cap-review cadence calendar + quarterly Treasury audit cadence.
- Off-chain Treasury → BE refresh SOP rehearsed in staging.
- DEPLOYER retirement archival (custody-policy §9.3) prepared offline.

---

## 4. Dependency reference (updated)

Cluster 3 status in dependency map §2 (Q-CD → manifest placeholder unlocks) + §6 (funding/rebate/insurance):

| Q-CD | Was | Now |
|---|---|---|
| Q-CD-7 | drives TREASURY Safe + funding flow | **POLICY-DECIDED** — Treasury operational flow design unblocked; identity + Safe deployment via `MAINNET-TREASURY-SAFE-CREATION-PACKET` |
| Q-CD-8 | drives DEPLOYER manifest slot + retirement plan + V2G-Y Y-E operational caller | **POLICY-DECIDED** post-migration; pre-migration form Q-CD-8a still OPEN; V2G-Y per-phase hard stops added |
| Q-CD-9 | drives BE alert thresholds + Treasury refresh cadence + bounded loss limit | **FORMULA-DECIDED**; numeric parameter fill via `MAINNET-BE-FUNDING-POLICY-PARAMETER-FILL` |

Combined with prior closures:
- Cluster 1 (Q-CD-1/2/3/4/13): resolved + chain-anchored.
- Cluster 2 (Q-CD-5/6/14/15): architecture + policy decided.
- Cluster 3 (Q-CD-7/8/9): policy + formula decided.

Cluster 4 (Q-CD-10/11/12/17/18 — Q-CD-16 pre-resolved) remains the
final custody cluster.

---

## 5. Manifest implications (delta from Cluster 2)

| Slot (line) | Pre-Cluster-3 | Post-Cluster-3 |
|---|---|---|
| `chainMetadata.deployer` (11) | `NEEDS_OPERATOR_DECISION` | post-migration policy locked; pre-migration form (Q-CD-8a) still OPEN |
| `governanceRoles.deployerRetirementStatus` (NEW) | not defined | **recommended NEW field** |
| `governanceRoles.treasury` (NEW per custody policy §13.3) | proposed | **structurally clear**; value pending `MAINNET-TREASURY-SAFE-CREATION-PACKET` |
| `insuranceConfiguration.fundAddress` / `.custodyToken` / `.initialFundingAmountBase` / `.operators[0]` | NEEDS_OPERATOR_DECISION | source path = TREASURY (Cluster 3 R-6 confirmed); sizing pending Q-CD-12; operator form pending Q-CD-17 |
| `funding.backendExecutor.*` (NEW) | not defined | **recommended NEW fields** for observable manifest record of FUND_FLOOR / TARGET / CEILING / recompute cadence |

---

## 6. Audit implications (delta)

`MAINNET_AUDIT_EXT_ENGAGEMENT_PACKAGE.md §7` to add Q-31 + Q-32 from Cluster 3 §6:
- Q-31: Can any code path move funds from PFV / IF / CV directly into BE.balance without traversing TREASURY?
- Q-32: Confirm DEPLOYER retirement procedure (§9.3) is the only path that disables DEPLOYER; confirm post-procedure DEPLOYER cannot be silently re-enabled.

Auditor review surface expanded:
- Treasury role boundary verifiable on chain post-deployment.
- DEPLOYER post-migration sweep verifiable on chain post-V2G-Y.
- BE funding policy + protocol-side bright line auditable per formula + accounting §3.6.

---

## 7. V2G-Y implications (delta)

| Phase | Cluster 3 contribution |
|---|---|
| Y-G-5a / 5b | HARD STOPS added — chain-verifiable DEPLOYER strip |
| POST-Y-G-6 final sweep | NEW comprehensive sweep across 8 modules + Timelock + NEW_OME + 3 Safes + InsuranceFund |
| Treasury operational onboarding | follows Y-G-6 + Treasury Safe creation + Cluster 2 BE EOA derivation |

Cluster 1 already parameterised Y-A and Y-G-1..6 (architecture).
Cluster 2 parameterised Y-F planning. **Cluster 3 adds the
DEPLOYER-retirement verification layer to every relevant phase
boundary.**

---

## 8. Cluster 4 (remaining cluster)

Cluster 4 covers:
- Q-CD-10 PFV revenue receiver — recommended Timelock-direct.
- Q-CD-11 rebates at launch — recommended DEFER.
- Q-CD-12 insurance initial seeding — sizing decision.
- Q-CD-16 BE rotation cadence — pre-resolved at custody policy §9.1 (≤ 30 days).
- Q-CD-17 insurance operator form — operator decision.
- Q-CD-18 custody policy version cadence — freeze at v1.0.0 first-live-smoke + quarterly review.

Cluster 4 is the last custody cluster. After Cluster 4 + Cluster 1 +
Cluster 2 + Cluster 3, custody decisions are fully decided at the
policy / formula / architecture layer; provisioning (Treasury Safe
deploy, KMS vendor + region selection, numeric fund fill, etc.) is
the next operational track.

---

## 9. Files produced / updated by this milestone

| Path | Status |
|---|---|
| `~/DEOPT/private/mainnet_custody/MAINNET_CUSTODY_CLUSTER_3_RESOLUTION.private.md` | **CREATED** (mode 600, outside all repo trees) |
| `~/DEOPT/private/mainnet_custody/CLUSTER_HASHES.txt` | **APPENDED** (Cluster 3 sha256 entry) |
| `deopt-v2-backend/docs/MAINNET_CUSTODY_CLUSTER_3_RESOLUTION_REDACTED.md` | **CREATED** (public redacted summary) |
| `deopt-v2-backend/docs/MAINNET_CUSTODY_CLUSTER_3_NEXT_ACTIONS.md` | **CREATED** (this file) |
| `deopt-v2-backend/docs/MAINNET_CUSTODY_DECISION_DEPENDENCY_MAP.md` | **UPDATED** (Cluster 3 row status) |
| `~/DEOPT/RUN_STATE.md` | **APPENDED** (redacted closure note) |

No source touched. No `.env` edit. No chain mutation. No Safe-tx. No Treasury creation. No fund movement.

---

## 10. Next milestone recommendation

Primary recommendation: **`MAINNET-CUSTODY-CLUSTER-4-RESOLUTION`** (Q-CD-10/11/12/17/18). Closes the final custody cluster.

In parallel:
1. **`MAINNET-TREASURY-SAFE-CREATION-PACKET`** — operator-authorised Treasury Safe deployment packet.
2. **`MAINNET-BE-FUNDING-POLICY-PARAMETER-FILL`** — first numeric fill of the Q-CD-9 formula.
3. **`MAINNET-DEPLOY-CEREMONY-DESIGN`** — resolves Q-CD-8a pre-migration DEPLOYER form.
4. **`MAINNET-MANIFEST-FILL-GOV-OPS-TREASURY-SLOTS`** — read-only manifest fill PR (Cluster 1 13 slots + Cluster 3 schema extension).
5. **`MAINNET-BE-SIGNER-SERVICE-DESIGN`** + **`MAINNET-KMS-VENDOR-SELECTION`** — Cluster 2 follow-ups.
6. **`BACKEND-SHOULD-BROADCAST-ECONOMIC-GATE`** — gap-list C-4.
7. **`MAINNET-AUDIT-EXT-KICKOFF`** (P0-1) — ship Cluster 1 + 2 + 3 redacted summaries in handoff bundle.

---

## 11. Cross-links

- `~/DEOPT/MAINNET_CUSTODY_POLICY.md` §3.1 R-6, §8, §9.3, §13.3
- `~/DEOPT/MAINNET_CUSTODY_DECISIONS_ADDENDUM_TEMPLATE.md` Q-CD-7 / 8 / 9
- `~/DEOPT/deopt-v2-backend/docs/MAINNET_CUSTODY_DECISION_DEPENDENCY_MAP.md`
- `~/DEOPT/deopt-v2-backend/docs/MAINNET_CUSTODY_CLUSTER_3_RESOLUTION_REDACTED.md`
- `~/DEOPT/deopt-v2-backend/docs/MAINNET_CUSTODY_CLUSTER_2_RESOLUTION_REDACTED.md`
- `~/DEOPT/deopt-v2-backend/docs/MAINNET_CUSTODY_CLUSTER_1_RESOLUTION_REDACTED.md`
- `~/DEOPT/deopt-v2-backend/docs/P0_MAINNET_BLOCKER_CLOSURE_ROADMAP.md`
- `~/DEOPT/deopt-v2-backend/docs/BACKEND_EXECUTOR_MONITORING_ALERTS_V1.md`
- `~/DEOPT/deopt-v2-backend/docs/BACKEND_GAS_FEES_REBATES_POLICY_V1.md`
- `~/DEOPT/deopt-v2-sol/docs/MAINNET_V2G_Y_OWNERSHIP_MIGRATION_PLAN.md`
- `~/DEOPT/deopt-v2-sol/docs/MAINNET_AUDIT_EXT_ENGAGEMENT_PACKAGE.md`
- `~/DEOPT/deopt-v2-sol/ROLE_MATRIX.md`
- `~/DEOPT/deopt-v2-sol/FINAL_LAUNCH_CHECKLIST.md`
- `~/DEOPT/RUN_STATE.md`

**End of Cluster 3 next-actions doc.**
