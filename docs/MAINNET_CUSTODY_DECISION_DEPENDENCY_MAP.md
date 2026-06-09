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

**Cluster 2 closure update (2026-06-09):**

| Q-CD | Status | KMS / backend impact |
|---|---|---|
| **Q-CD-5** | **ARCH-PATTERN-DECIDED: Pattern C** (dedicated backend signer microservice backed by HSM/MPC or KMS) | Architecture layer of backend impl can now START. `RemoteSigner` trait + `KmsRemoteSigner::from_service_endpoint(endpoint)` impl path is decided. Vendor sub-decision (`MAINNET-KMS-VENDOR-SELECTION`) still gates KMS key generation, but `BACKEND-SIGNER-INTERFACE-KMS-HSM-ADAPTER` PR can begin against the abstract interface. |
| **Q-CD-14** | **STRUCTURE-DECIDED: EU primary + EU/nearby secondary** | Region pair shape locked; backend adapter can model failover semantics. Exact regions follow vendor; not a code-path gate at the trait layer. |
| **Q-CD-15** | **POLICY-DECIDED-PROVIDER-DETAIL-PENDING** | Custody runbook can be drafted at policy layer (disable ≠ delete; ≥ 2 approvals + governance for permanent deletion; emergency IAM revoke faster than disable). Exact IAM JSON pending vendor. Not a code-path gate. |
| **Q-CD-6** | **DECIDED: distinct EOAs** | Drives KMS key-count = 2 (OPTION at launch + warm spare; PERP deferred). Affects IAM policy structure and warm-spare provisioning. |
| **Q-CD-16** | **PRE-RESOLVED in custody policy §9.1: ≤ 30 days** | Drives backend rotation runbook + warm-spare provisioning cadence. Not a code-path gate. |
| **Q-CD-13** | **POLICY: TRUE (per Cluster 1 closure)** | Applies to signer-service Sepolia rehearsal too — backend impl PR must include a Sepolia integration test that exercises the new `RemoteSigner` path before mainnet activation. |

Pre-Cluster-2 state was "code work cannot start". Post-Cluster-2 state
is: **backend impl can begin at the abstract-trait layer**; KMS key
generation + signer-service deployment still depends on vendor + region
sub-decisions.

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
| **Q-CD-7** | **POLICY-DECIDED (Cluster 3, 2026-06-09)**: Safe v1.4.1 ≥ 3-of-5; hard disjoint OPS; default partial separation GOV; no DEPLOYER. Treasury Safe deployment via `MAINNET-TREASURY-SAFE-CREATION-PACKET`. Unblocks BE refresh path + PFV revenue cycle + rebate allocation + insurance funding planning. |
| **Q-CD-8** | **POLICY-DECIDED (Cluster 3)** post-migration policy: DEPLOYER = provenance only; V2G-Y per-phase HARD STOPS added (Y-G-5a/5b verifier; post-Y-G-6 sweep). Pre-migration DEPLOYER form Q-CD-8a still OPEN. |
| **Q-CD-9** | **FORMULA-DECIDED (Cluster 3)**: `FUND_FLOOR = max(7d gas, emergency rotation)`; `FUND_TARGET = 3× FLOOR`; `FUND_CEILING = min(10× FLOOR, op cap)`; monthly recompute. Numeric parameter fill via `MAINNET-BE-FUNDING-POLICY-PARAMETER-FILL`. Unblocks BE alert threshold ladder + Treasury refresh cadence + bounded loss limit. |
| **Q-CD-10** | **POLICY-DECIDED (Cluster 4, 2026-06-09)**: PFV revenue stays in PFV until Timelock-governed withdrawal → TREASURY_SAFE_MAINNET. Hot-wallet destination unacceptable. Future milestone `MAINNET-PFV-REVENUE-WITHDRAWAL-SOP`. |
| **Q-CD-11** | **POLICY-DECIDED (Cluster 4): DEFERRED at launch.** `rebateReserve = 0`; all active profiles effective non-negative. Launch invariant verifier sweep added to POST-Y-G-6 audit. Re-evaluation via `MAINNET-REBATE-PROGRAM-DESIGN` (post-soak, if enabled). |
| **Q-CD-12** | **FORMULA-DECIDED (Cluster 4)**: `initial_insurance_seed = OI_cap × stress_loss × coverage_ratio`. Numeric fill via `MAINNET-INSURANCE-SEEDING-PARAMETER-FILL` co-decided with launch caps. Treasury-funded; R-6 bright line preserved. |
| **Q-CD-17** | **POLICY-DECIDED (Cluster 4)**: dedicated insurance-operator Safe (≥ 2-of-3 / 3-of-5 recommended), disjoint from OPS/GOV/TREASURY rosters, Timelock owns the Fund. Fallback OPS_MULTISIG (waiver + AUDIT-EXT sign-off; upgrade ≤ 6 months). DEPLOYER unacceptable post-migration. Future milestone `MAINNET-INSURANCE-OPERATOR-POLICY-PACKET`. |
| **Q-CD-18** | **POLICY-DECIDED (Cluster 4)**: SemVer MAJOR.MINOR.PATCH; freeze v1.0.0 at first-live-smoke; quarterly review during beta + semi-annual after stable + emergency on incident; tiered approval matrix. Future milestone `MAINNET-CUSTODY-POLICY-VERSIONING-SOP`. |

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
