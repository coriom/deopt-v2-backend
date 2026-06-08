# Mainnet custody decision dependency map

**Posture:** READ-ONLY dependency map. **No chain mutation. No `.env`
edit. No Safe-tx. No broadcast. No mainnet.** Companion to
`~/DEOPT/MAINNET_CUSTODY_POLICY.md §16` and
`~/DEOPT/MAINNET_CUSTODY_DECISIONS_ADDENDUM_TEMPLATE.md`. Catalogues
which Q-CDs unblock which downstream P0 / P1 milestones — so the
operator can pick the next decision to resolve based on which
workstream needs the unlock most.

**Date:** 2026-06-08

---

## 0. Hard stops (this doc)

```text
no chain tx                                                         ✅
no Safe tx                                                          ✅
no backend broadcast                                                ✅
no .env edit                                                        ✅
no mainnet                                                          ✅
no secrets printed                                                  ✅
no guessed identities / addresses recorded                          ✅
```

---

## 1. Q-CD inventory (recap)

| ID | One-liner | Recommended default (from policy §16) |
|---|---|---|
| Q-CD-1 | OPS_MULTISIG signer roster | 3-of-5, disjoint from GOV |
| Q-CD-2 | OPS_MULTISIG threshold | 2-of-3 minimum / 3-of-5 recommended |
| Q-CD-3 | GOVERNANCE_MULTISIG signer roster | 5 signers, disjoint from OPS, ≥ 3 humans / ≥ 2 jurisdictions |
| Q-CD-4 | GOVERNANCE_MULTISIG threshold | 3-of-5 |
| Q-CD-5 | KMS / HSM provider | Pattern A (cloud KMS w/ secp256k1) |
| Q-CD-6 | Same BE for OPTION + PERP or distinct | distinct |
| Q-CD-7 | TREASURY Safe form | independent Safe ≥ 3-of-5 |
| Q-CD-8 | DEPLOYER form | dedicated deploy Safe ≥ 2-of-3 |
| Q-CD-9 | BE FUND_FLOOR / TARGET / CEILING | formula-based; ≥ 30d gas / ×3 / ×5; monthly recompute |
| Q-CD-10 | PFV.revenueReceiver | Timelock (double-gate) |
| Q-CD-11 | Rebate program at launch | DEFER |
| Q-CD-12 | Insurance initial seeding | sized to ≥ 1× max single-account bad-debt |
| Q-CD-13 | Sepolia rehearsal before mainnet Safe roster | TRUE |
| Q-CD-14 | KMS region pair | 2 geographic regions (one off-continent) |
| Q-CD-15 | Key-deletion lock approval | 2 operator + 1 security + 7d wait |
| Q-CD-16 | BE rotation cadence | ≤ 30 days |
| Q-CD-17 | Insurance operator form | dedicated Safe |
| Q-CD-18 | Custody policy version + rev cadence | freeze at v1.0.0 first-live-smoke; quarterly |

---

## 2. Q-CD → manifest placeholder unlocks

Source: `deopt-v2-sol/docs/MAINNET_MANIFEST_TODO_INVENTORY.md` §3.
Each row maps a Q-CD to the manifest slots (with line number) it
structurally unblocks. The identity / value still requires the
operator-decided answer to be written; the Q-CD just shapes the slot.

| Q-CD | Manifest slot(s) unblocked | Notes |
|---|---|---|
| **Q-CD-1, Q-CD-2, Q-CD-13** | `governanceRoles.governanceOwner` (77); `.timelockProposers[0]` (83); `.timelockExecutors[0]` (86); `.governanceGuardian` (99); `.moduleGuardians.collateralVault / .oracleRouter / .marginEngine / .perpEngine / .insuranceFund / .matchingEngine / .perpMatchingEngine` (101-108) (7 slots); plus the OPS Safe schema addition for any custody schema-extension PR. | All Group A slots that read "OPS_MULTISIG" depend on OPS Safe deployment + roster lock. |
| **Q-CD-3, Q-CD-4, Q-CD-13** | `governanceRoles.finalGovernanceOwner` (78); `.timelockOwner` (79); `.riskGovernorOwner` (80, if shared). | Group A: GOVERNANCE_MULTISIG identity. |
| **Q-CD-5, Q-CD-6, Q-CD-14, Q-CD-15** | `matchingExecutors.options[0].executor` (114); `matchingExecutors.perps[0].executor` (120) (if distinct per Q-CD-6); new schema addition `governanceRoles.kmsKeyHandles.backendExecutor` and `.backendExecutorNext`. | BE EOA derives from KMS key (Q-CD-5 + Q-CD-14); rotation cadence (Q-CD-16) drives schema for warm spare. |
| **Q-CD-7** | New schema addition `governanceRoles.treasury` (custody-policy §13.3). | Group A schema extension. |
| **Q-CD-8** | `chainMetadata.deployer` (11). | Group A / J. |
| **Q-CD-9** | No direct slot — drives BE alert thresholds (`BE_BAL_LOW` / `BE_BAL_CEILING`) and TREASURY refresh policy. Affects monitoring config rather than manifest fill. | Indirect. |
| **Q-CD-10** | No direct slot in mainnet template today; affects post-deploy `PFV.setRevenueReceiver` Timelock op. | Operational. |
| **Q-CD-11** | `feesConfiguration.merkleRoot` (194) — value `0x0…0` if deferred OR a real root if enabled. | Group E. |
| **Q-CD-12** | `insuranceConfiguration.initialFundingAmountBase` (199); `.fundAddress` (197, linked); `.allowedTokens[0]` (201); `.operators[0]` (linked to Q-CD-17). | Group K. |
| **Q-CD-17** | `insuranceConfiguration.operators[0]` (204). | Group K. |
| **Q-CD-18** | New schema addition `custodyPolicyVersion` (custody-policy §13.3). | Group A schema extension. |

Q-CDs that DO NOT directly unblock a manifest slot: Q-CD-9, Q-CD-10, Q-CD-13 (operational rather than manifest-structural).

---

## 3. Q-CD → AUDIT-EXT kickoff unlocks

Source: `deopt-v2-sol/docs/MAINNET_AUDIT_EXT_ENGAGEMENT_PACKAGE.md` §7 / §11.

| Q-CD | AUDIT-EXT impact |
|---|---|
| **Q-CD-5** | Required IN SCOPE: auditor reviews KMS/HSM signer trust boundary, IAM policy, transaction policy precheck (custody-policy §6.5 / §6.6 / §7). Q-26 + Q-27 in package §7. Without Q-CD-5 resolved, auditor cannot review the BE signing path. |
| **Q-CD-1, Q-CD-3** | Required IN SCOPE: roster review per audit package §11 (engagement-kickoff checklist) and audit-package §14.1 / Q-28 in custody policy. Auditor confirms R-8 (no roster overlap). |
| **Q-CD-2, Q-CD-4** | Required IN SCOPE: threshold review. |
| **Q-CD-7** | IN SCOPE: TREASURY separation (R-6) reviewed. |
| **Q-CD-8** | IN SCOPE: DEPLOYER retirement procedure reviewed; Q-CD-8 form affects retirement-log archival. |
| **Q-CD-13** | IN SCOPE: confirms each signer has rehearsed Sepolia first. |
| **Q-CD-14** | IN SCOPE: KMS region failover reviewed. |
| **Q-CD-15** | IN SCOPE: key-deletion approval lock reviewed. |
| **Q-CD-16** | IN SCOPE: rotation cadence reviewed. |
| **Q-CD-11** | IN SCOPE: if rebate program enabled at launch, AUDIT-EXT reviews additional rebate-bearing surfaces; if deferred, auditor confirms the `rebateReserve=0` chain backstop is sufficient. |
| **Q-CD-12** | IN SCOPE: insurance funding amount and bad-debt sizing rationale reviewed. |
| **Q-CD-17** | IN SCOPE: insurance operator separation reviewed. |
| **Q-CD-18** | IN SCOPE: auditor receives the frozen v1.0.0 policy commit hash. |

**AUDIT-EXT engagement can kick off WITHOUT all Q-CDs resolved**: the auditor can review the policy + plan + manifest *shape* and queue specific Q-CD-related findings to remediation review. But:
- Q-CD-5 SHOULD be resolved before AUDIT-EXT kickoff to enable trust-boundary review.
- Q-CD-1 / Q-CD-3 / Q-CD-7 SHOULD be resolved or at least nominated before AUDIT-EXT kickoff so the auditor knows the role architecture.

---

## 4. Q-CD → KMS / backend-implementation unlocks

Source: gap-list D-1 / `MAINNET-BE-KMS-SIGNER-INTERFACE-IMPL` milestone (custody-policy §7.4 + backend source survey in §6.1).

| Q-CD | KMS / backend impact |
|---|---|
| **Q-CD-5** | **HARD GATE.** Backend KMS interface implementation cannot start until vendor + Pattern A/B/C is chosen. Drives the `RemoteSigner` adapter (`from_kms_handle(handle)` impl). |
| **Q-CD-14** | **HARD GATE.** Region failover design drives the backend KMS adapter's retry / failover semantics. Cannot ship without region pair locked. |
| **Q-CD-15** | Required for IAM policy provisioning (custody-policy §6.5); not a code-path gate, but a deployment-time gate. |
| **Q-CD-6** | Drives KMS key-count: 1 (shared BE for option + perp) vs 2 (distinct). Affects IAM policy structure and warm-spare provisioning. |
| **Q-CD-16** | Drives backend rotation runbook + warm-spare provisioning cadence. Not a code-path gate. |
| **Q-CD-13** | Drives Sepolia integration test prior to mainnet code path activation. Not a code-path gate, but a gate on "can we trust the new code in production". |

KMS code work cannot complete without Q-CD-5 AND Q-CD-14 AND Q-CD-15
resolved. Q-CD-6 + Q-CD-16 + Q-CD-13 inform the operational runbook
but do not gate the code.

---

## 5. Q-CD → V2G-Y phase unlocks

Source: `deopt-v2-sol/docs/MAINNET_V2G_Y_OWNERSHIP_MIGRATION_PLAN.md` §3 prereqs + §1 final target shape.

| Phase | Depends on Q-CD(s) | Reason |
|---|---|---|
| **Y-A** (guardian wiring on 9 targets) | Q-CD-1, Q-CD-2, Q-CD-13 | Sets guardians to OPS_MULTISIG_MAINNET; OPS Safe must exist + verified |
| **Y-B** (PFV.transferOwnership(Timelock)) | none directly; Timelock exists post-deploy | But all phases gate on §3 prerequisites = Q-CD-1/3/5 |
| **Y-C** (FM_V2.transferOwnership(Timelock)) | none directly | same |
| **Y-D** (8-target transferOwnership) | none directly | same |
| **Y-E** (8 Timelock-queued acceptOwnership) | Q-CD-8 (DEPLOYER form must support queue/execute caller) | DEPLOYER still proposer/executor until Y-G-5; Q-CD-8 affects retirement plan |
| **Y-F** (NEW_OME executor migration) | **Q-CD-5, Q-CD-6, Q-CD-14, Q-CD-15** | BE_MAINNET must be live + signing via KMS before Y-F-B-X-add lands (otherwise Y-F-B-X-add leaves OME with an authorised executor that cannot sign). Also Q-CD-9 because BE must be funded before signing. |
| **Y-G-1, 2, 3** | Q-CD-1, Q-CD-2, Q-CD-13 (OPS Safe exists) | Wires OPS_MULTISIG as proposer/executor/guardian; transfers Timelock to GOV |
| **Y-G-4** (GOV Safe accepts Timelock) | Q-CD-3, Q-CD-4, Q-CD-13 (GOV Safe exists) | **Point of no return.** GOV Safe must be verified + roster locked. |
| **Y-G-5, 6** | Q-CD-3, Q-CD-4 (GOV signs strip + minDelay bump) | Final cleanup |

**Conclusion:** V2G-Y mainnet execution cannot start until at minimum:
- Q-CD-1, Q-CD-2 (OPS Safe deployed)
- Q-CD-3, Q-CD-4 (GOV Safe deployed)
- Q-CD-5, Q-CD-14, Q-CD-15 (KMS interface live)
- Q-CD-6 (BE topology fixed)
- Q-CD-8 (DEPLOYER form locked)
- Q-CD-9 (BE funded per policy)
- Q-CD-13 (signers rehearsed)

Q-CD-10, Q-CD-11, Q-CD-12, Q-CD-16, Q-CD-17, Q-CD-18 do NOT directly gate V2G-Y; they gate downstream operational milestones.

---

## 6. Q-CD → funding / rebate / insurance task unlocks

Source: custody-policy §8 + `MAINNET_MANIFEST_TODO_INVENTORY.md` Groups E + K + gap-list F-5 / F-8 / K-3.

| Q-CD | Funding / rebate / insurance task |
|---|---|
| **Q-CD-7** | TREASURY Safe deployment → unblocks every BE refresh + PFV revenue cycle + rebate allocation + insurance funding. **Critical for any post-V2G-Y operational tx.** |
| **Q-CD-9** | BE FUND_FLOOR / TARGET / CEILING → unblocks BE first refresh + monitoring alert thresholds + bounded loss limit. **Critical for first-live-smoke.** |
| **Q-CD-10** | PFV.revenueReceiver → unblocks the design of the PFV revenue cycle (Timelock-queued vs TREASURY-direct). Post-launch task. |
| **Q-CD-11** | Rebate program at launch → if YES, drives `feesConfiguration.merkleRoot` fill + reserve allocation milestone + wash-detection deadlines. If DEFER (recommended), no immediate downstream task. |
| **Q-CD-12** | Insurance seeding amount → unblocks `MAINNET-INSURANCE-FUND-FUNDING` milestone (gap-list I-5). |
| **Q-CD-17** | Insurance operator form → unblocks `MAINNET-INSURANCE-OPERATOR-PROVISION` operational task. |

---

## 7. Cross-cutting blocker classification

For each Q-CD, this maps **which broad category** it blocks. Useful
for "I want to unblock X — which Q-CDs do I need to resolve?"

| Q-CD | Manifest fill | AUDIT-EXT kickoff | KMS impl | V2G-Y phases | Funding / rebate / insurance | Backend code | Notes |
|---|---|---|---|---|---|---|---|
| Q-CD-1 | ✅ | ⚠ (soft) | — | ✅ Y-A, Y-G-1..3 | — | — | Highest leverage (OPS Safe) |
| Q-CD-2 | ✅ | ⚠ | — | ✅ | — | — | bundled with Q-CD-1 |
| Q-CD-3 | ✅ | ⚠ | — | ✅ Y-G-4..6 | — | — | GOV Safe identity |
| Q-CD-4 | ✅ | ⚠ | — | ✅ | — | — | bundled with Q-CD-3 |
| Q-CD-5 | ✅ | ✅ **HARD** | ✅ **HARD** | ✅ Y-F | — | ✅ | Highest leverage for backend code |
| Q-CD-6 | ✅ | — | ✅ | ✅ Y-F | — | ✅ | Drives KMS key-count |
| Q-CD-7 | ✅ (schema) | ⚠ | — | — | ✅ ALL | — | TREASURY identity |
| Q-CD-8 | ✅ | ⚠ | — | ✅ Y-E (operationally) | — | — | DEPLOYER form |
| Q-CD-9 | — | — | — | ✅ Y-F (must fund BE before signing) | ✅ BE funding | — | Indirect manifest |
| Q-CD-10 | — | — | — | — | ✅ revenue cycle | — | Post-launch op |
| Q-CD-11 | ✅ | ⚠ | — | — | ✅ rebate decision | — | Group E |
| Q-CD-12 | ✅ | ⚠ | — | — | ✅ insurance seeding | — | Group K |
| Q-CD-13 | — | ✅ | — | ✅ | — | — | Onboarding gate |
| Q-CD-14 | — | ✅ | ✅ **HARD** | ✅ Y-F | — | ✅ | KMS failover |
| Q-CD-15 | — | ✅ | ⚠ | — | — | — | IAM policy |
| Q-CD-16 | — | ⚠ | — | — | — | — | Operational cadence |
| Q-CD-17 | ✅ | ⚠ | — | — | ✅ insurance op | — | Group K |
| Q-CD-18 | ✅ (schema) | ✅ | — | — | — | — | policy versioning |

**Legend:** ✅ = direct unblock; ⚠ = soft / scope dependency; **HARD** = absolutely cannot proceed without.

---

## 8. Priority unlock order (recommended)

Based on the dependency map, the resolution order that maximises parallel unblocks:

### 8.1 Priority cluster 1 — within 2 weeks

```
Q-CD-1, Q-CD-2 (OPS_MULTISIG roster + threshold)
Q-CD-3, Q-CD-4 (GOVERNANCE_MULTISIG roster + threshold)
Q-CD-13       (Sepolia rehearsal commitment — drives signer onboarding)
```

**Unblocks:** all Group A manifest slots (governance + guardians); V2G-Y Y-A through Y-G-6; Workstream A from `P0_MAINNET_BLOCKER_CLOSURE_ROADMAP.md`.

### 8.2 Priority cluster 2 — within 4 weeks (parallel)

```
Q-CD-5         (KMS provider — Pattern A/B/C)
Q-CD-14        (KMS region pair)
Q-CD-15        (Key-deletion approval lock)
Q-CD-6         (Same vs distinct BE for option + perp)
```

**Unblocks:** backend KMS interface implementation (gap-list D-1 / `MAINNET-BE-KMS-SIGNER-INTERFACE-IMPL`); V2G-Y Y-F; AUDIT-EXT trust-boundary review.

### 8.3 Priority cluster 3 — within 6 weeks (parallel)

```
Q-CD-7         (TREASURY Safe form)
Q-CD-9         (BE FUND_FLOOR / TARGET / CEILING formula)
Q-CD-8         (DEPLOYER form)
```

**Unblocks:** BE first refresh + monitoring alerts; TREASURY operational flow; DEPLOYER manifest slot + retirement plan.

### 8.4 Priority cluster 4 — within 8-10 weeks (can defer)

```
Q-CD-10        (PFV.revenueReceiver)
Q-CD-11        (Rebate at launch — recommended DEFER)
Q-CD-12        (Insurance seeding amount + sizing methodology)
Q-CD-17        (Insurance operator form)
Q-CD-16        (BE rotation cadence)
Q-CD-18        (Custody policy version cadence)
```

**Unblocks:** Group E + Group K manifest slots; post-launch operational milestones.

### 8.5 Critical path summary

The shortest path to "all P0 prerequisites resolved" is:

```
Cluster 1 → Cluster 2 → AUDIT-EXT kickoff (parallel) → backend KMS impl
                                                     → V2G-Y phase A prep
                          → Cluster 3 (BE funding, treasury)
                          → Cluster 4 (insurance, rebates)
```

Cluster 1 and Cluster 2 are the unblock bottlenecks. Once both close,
every other workstream can proceed in parallel.

---

## 9. Dependency graph (visual)

```
                    Custody policy MAINNET_CUSTODY_POLICY.md
                                 │
                  ┌──────────────┼──────────────┬──────────────┐
                  │              │              │              │
                  ▼              ▼              ▼              ▼
              Cluster 1      Cluster 2      Cluster 3      Cluster 4
              (Safes)        (KMS)          (Treas/Fund)   (Insurance/Misc)
                  │              │              │              │
                  ▼              ▼              ▼              ▼
        ┌────────────────┐ ┌──────────────┐ ┌────────────┐ ┌──────────────┐
        │ Q-CD-1,2,13    │ │ Q-CD-5,14,15 │ │ Q-CD-7,9   │ │ Q-CD-10..18  │
        │ Q-CD-3,4       │ │ Q-CD-6       │ │ Q-CD-8     │ │              │
        └────────┬───────┘ └──────┬───────┘ └─────┬──────┘ └──────┬───────┘
                 │                │               │               │
                 ▼                ▼               ▼               ▼
            ┌───────────────────────────────────────────────────────────┐
            │  Manifest fill (76 distinct slots; 99 raw)                │
            │  AUDIT-EXT kickoff                                         │
            │  Backend KMS implementation                                │
            │  V2G-Y phase packets                                       │
            │  Funding / Rebate / Insurance operational milestones       │
            └───────────────────────────────────────────────────────────┘
                              │
                              ▼
                         Mainnet ready
```

---

## 10. What this dependency map does NOT do

```text
- Does NOT lock signer identities or mainnet addresses
- Does NOT broadcast any tx
- Does NOT edit any .env
- Does NOT touch mainnet
- Does NOT mark Q-CDs as OPERATOR-DECIDED
- Does NOT obligate auditor / security / governance leads
- Does NOT replace the addendum template — operator still fills each Q-CD per section
```

---

## 11. Cross-links

- `~/DEOPT/MAINNET_CUSTODY_POLICY.md` §16 OPEN DECISIONS
- `~/DEOPT/MAINNET_CUSTODY_DECISIONS_ADDENDUM_TEMPLATE.md` (companion)
- `~/DEOPT/deopt-v2-sol/docs/MAINNET_MANIFEST_TODO_INVENTORY.md` §3 placeholder tables
- `~/DEOPT/deopt-v2-sol/docs/MAINNET_V2G_Y_OWNERSHIP_MIGRATION_PLAN.md` §3 prereqs + phase ledger
- `~/DEOPT/deopt-v2-sol/docs/MAINNET_AUDIT_EXT_ENGAGEMENT_PACKAGE.md` §7 + §11
- `~/DEOPT/deopt-v2-backend/docs/P0_MAINNET_BLOCKER_CLOSURE_ROADMAP.md` §4 workstreams + §8 next milestones
- `~/DEOPT/deopt-v2-backend/docs/BACKEND_SIGNER_CUTOVER_RUNBOOK_V2G_FX_Q1.md` §13 mainnet KMS lift
- `~/DEOPT/RUN_STATE.md`

**End of mainnet custody decision dependency map.**
