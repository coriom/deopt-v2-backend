# Mainnet custody — Cluster 4 next actions

**Posture:** READ-ONLY dependency / next-action doc. **No chain
mutation. No `.env` edit. No Safe-tx. No broadcast. No mainnet. No
fund movement.** Companion to
`~/DEOPT/MAINNET_CUSTODY_DECISIONS_ADDENDUM_TEMPLATE.md`,
`deopt-v2-backend/docs/MAINNET_CUSTODY_CLUSTER_4_RESOLUTION_REDACTED.md`,
and `deopt-v2-backend/docs/MAINNET_CUSTODY_DECISION_DEPENDENCY_MAP.md`.

**Date:** 2026-06-09
**Cluster 4 status:** **CLOSED at policy / formula / architecture layer.**
**Custody-track status overall:** **ALL 18 Q-CDs CLOSED.** Sub-decisions, numeric fills, and provisioning are operational follow-ups.

---

## 0. Hard stops (this doc)

```text
no chain tx                                        ✅
no Safe tx                                         ✅
no Treasury Safe / Insurance operator Safe creation ✅
no InsuranceFund creation                          ✅
no PFV withdrawal                                  ✅
no rebate reserve allocation                       ✅
no fund movement                                   ✅
no .env edit                                       ✅
no broadcast                                       ✅
no mainnet                                         ✅
no numeric insurance / rebate values recorded      ✅
no personal contacts / bank details                ✅
```

---

## 1. Cluster 4 closure recap

| Q-CD | Closure | Authority |
|---|---|---|
| Q-CD-10 | **POLICY-DECIDED**: PFV revenue stays in PFV until Timelock withdrawal → TREASURY_SAFE_MAINNET. Hot-wallet destination unacceptable. | Governance + Finance |
| Q-CD-11 | **POLICY-DECIDED: DEFERRED**. `rebateReserve = 0`; all active profiles effective non-negative; re-evaluate after 3-month soak. | Operator + Risk + Finance |
| Q-CD-12 | **POLICY-DECIDED-PARAMETERS-PENDING**: formula `OI_cap × stress_loss × coverage_ratio`; Treasury-funded; staged/capped recommended. | Insurance + Finance + Risk |
| Q-CD-16 | **PRE-RESOLVED** (custody policy §9.1; ≤ 30 days or after incident) | Operator + Security |
| Q-CD-17 | **POLICY-DECIDED**: dedicated insurance-operator Safe (≥ 2-of-3 minimum, 3-of-5 recommended) recommended; fallback OPS_MULTISIG; DEPLOYER unacceptable. | Insurance + Operator |
| Q-CD-18 | **POLICY-DECIDED**: SemVer + quarterly review during beta + emergency review on incident + tiered approval matrix. | Operator + Security |

Public redacted summary: `deopt-v2-backend/docs/MAINNET_CUSTODY_CLUSTER_4_RESOLUTION_REDACTED.md`.
Private artefact: `~/DEOPT/private/mainnet_custody/MAINNET_CUSTODY_CLUSTER_4_RESOLUTION.private.md` (mode 600).
sha256 anchor: `e555bfd7e4818b8a910013d55b7e3fff985c4f041cb0b81378b7668d4353d68f`.
Hash log re-verify: 5/5 OK.

---

## 2. What Cluster 4 unblocks

### 2.1 Planning unblocks

| Target | Status |
|---|---|
| PFV revenue cycle design | three-gate flow locked; cadence pending |
| Rebate-DEFER launch posture | invariant locked; verifier sweep specified |
| Insurance funding formula | locked; numeric fill pending |
| Insurance operator architecture | locked; identity decision pending |
| Custody policy versioning governance | SemVer + cadence + approval matrix locked; changelog file seed pending |
| AUDIT-EXT review surface | crystallised; auditor questions Q-33..Q-38 added |
| All 18 custody Q-CD decisions | **CLOSED** |
| Mainnet manifest schema extensions | clear shape for `rebateLaunchPolicy` / `operatorForm` / `custodyPolicyVersion` / `custodyPolicyChangeLogSha256` |

### 2.2 Items NOT unblocked by Cluster 4

| Target | Remaining gate |
|---|---|
| PFV withdrawal cadence + first cycle | requires Treasury Safe deployed + `MAINNET-PFV-REVENUE-WITHDRAWAL-SOP` |
| Numeric insurance seed | requires `MAINNET-INSURANCE-SEEDING-PARAMETER-FILL` + launch caps committed |
| Insurance operator Safe deployment | requires `MAINNET-INSURANCE-OPERATOR-POLICY-PACKET` + identity decision |
| Custody policy changelog file (`MAINNET_CUSTODY_POLICY_CHANGELOG.md`) | requires `MAINNET-CUSTODY-POLICY-VERSIONING-SOP` + v1.0.0 seed at first-live-smoke |
| Rebate program design (if later) | requires `MAINNET-REBATE-PROGRAM-DESIGN` (post-soak) |
| Mainnet first-live-smoke | requires AUDIT-EXT + Cluster 1/2/3 provisioning + V2G-Y execution |

---

## 3. Required follow-up milestones

### 3.1 `MAINNET-PFV-REVENUE-WITHDRAWAL-SOP` (operator + Governance + Finance)

Output: PFV `revenueReceiver` destination decision (Timelock-direct vs TREASURY-direct); withdrawal cadence (recommend monthly review, first cycle ≥ 30 d post-launch); Timelock op template; GOV Safe-tx queue/execute template; Treasury operational log entry shape.

### 3.2 `MAINNET-REBATE-PROGRAM-DESIGN` (operator + Risk + Finance + Backend; **only if rebates later enabled**)

Output: rebate-positive profile design; Treasury-funded `rebateReserve` cadence; backend `should_broadcast` rebate-solvency gate implementation; anti-wash detection (T-6); per-maker rebate quota; reserve-depletion alerts; AUDIT-EXT-2 engagement scope.

**Not on the launch critical path.** Cluster 4 explicitly DEFERS rebates.

### 3.3 `MAINNET-INSURANCE-SEEDING-PARAMETER-FILL` (operator + Insurance + Finance + Risk)

Output: numeric fill of the §3.2 formula inputs (max OI cap per market, target coverage ratio, stress loss assumption, number of active markets); recorded in private addendum; manifest schema slot `insuranceConfiguration.initialFundingAmountBase` (line 199) filled. Co-decided with launch-caps milestone.

### 3.4 `MAINNET-INSURANCE-OPERATOR-POLICY-PACKET` (operator + Insurance)

Output: Option B vs Option C identity decision (dedicated operator Safe vs OPS_MULTISIG fallback); operator Safe signer roster (offline binder; placeholder labels in repo); allowed/forbidden surface enforcement test plan; manifest schema slot `insuranceConfiguration.operators[0]` (line 204) filled.

### 3.5 `MAINNET-CUSTODY-POLICY-VERSIONING-SOP` (operator + Security)

Output: `~/DEOPT/MAINNET_CUSTODY_POLICY_CHANGELOG.md` seeded with v1.0.0 entry at first-live-smoke; approval matrix wired into operator runbook; RUN_STATE reference pattern established.

---

## 4. Dependency reference (updated)

Cluster 4 status in `MAINNET_CUSTODY_DECISION_DEPENDENCY_MAP.md`:

| Q-CD | Was | Now |
|---|---|---|
| Q-CD-10 | post-launch op | **POLICY-DECIDED** — PFV → Timelock → TREASURY; hot-wallet destination rejected |
| Q-CD-11 | Group E manifest slot driver | **POLICY-DECIDED: DEFERRED** — `feesConfiguration.merkleRoot` may be `bytes32(0)` or non-rebate-bearing root at launch |
| Q-CD-12 | Group K driver; FINAL_LAUNCH_CHECKLIST Insurance row | **FORMULA-DECIDED** — `initial_insurance_seed = OI × stress × ratio`; numeric fill pending |
| Q-CD-16 | operational cadence | **PRE-RESOLVED** in custody policy §9.1 (≤ 30 days) — no Cluster 4 change |
| Q-CD-17 | Group K driver | **POLICY-DECIDED** — dedicated Safe recommended (Option C); fallback OPS (Option B) with waiver |
| Q-CD-18 | policy versioning | **POLICY-DECIDED** — SemVer + cadence + approval matrix locked |

Combined with prior closures, **ALL 18 Q-CDs are now closed** at the
policy / formula / architecture layer. Sub-decisions catalogued as
named follow-up milestones.

---

## 5. Manifest implications (delta from Cluster 3)

| Slot (line) | Pre-Cluster-4 | Post-Cluster-4 |
|---|---|---|
| `feesConfiguration.feeRecipient` (193) | NEEDS_DEPLOYMENT | confirms PFV (Sepolia analogue); pending PFV mainnet deploy |
| `feesConfiguration.merkleRoot` (194) | NEEDS_OPERATOR_DECISION | DEFER → `bytes32(0)` OR non-rebate-bearing root |
| `insuranceConfiguration.initialFundingAmountBase` (199) | NEEDS_OPERATOR_DECISION | formula locked; value pending fill |
| `insuranceConfiguration.operators[0]` (204) | NEEDS_OPERATOR_DECISION | architecture decided; identity pending |
| `insuranceConfiguration.allowedTokens[0]` (201) | mUSDC analogue | mainnet USDC (gap-list N-1) |
| `insuranceConfiguration.backstopCallers[*]` (207-208) | MARGIN_ENGINE / PERP_ENGINE | confirmed canonical engines only |
| **New schema fields recommended:** | | |
| `feesConfiguration.rebateLaunchPolicy` | not defined | enum, initial `deferred` |
| `insuranceConfiguration.operatorForm` | not defined | enum |
| `custodyPolicyVersion` | proposed (Cluster 1) | SemVer; initial `v1.0.0` |
| `custodyPolicyChangeLogSha256` | not defined | latest entry sha256 |

---

## 6. Audit implications (delta)

`MAINNET_AUDIT_EXT_ENGAGEMENT_PACKAGE.md §7` to add 6 new auditor questions (Q-33..Q-38) per Cluster 4 redacted §8. Auditor review surface expanded to PFV receiver path, launch invariant, rebate activation gate, insurance reserve separation, operator authority limits, policy versioning governance.

---

## 7. V2G-Y implications (delta)

| Phase | Cluster 4 contribution |
|---|---|
| Y-A through Y-G-6 | unchanged |
| **POST-Y-G-6 final state audit** | **NEW LAUNCH INVARIANT** — verifier sweeps active FeesManagerV2 profiles confirming effective non-negative ppm AND `rebateReserve = 0`. If violated, first-live-smoke gate BLOCKED. |
| Treasury operational onboarding | PFV withdrawal SOP + insurance funding cadence join post-Y-G-6 checklist |

---

## 8. Monitoring implications (delta)

New metrics + alerts per Cluster 4 §10:
- `deopt_insurance_fund_balance_units` (gauge)
- `deopt_insurance_fund_target_seed_ratio` (gauge)
- `INSURANCE_BELOW_TARGET` (warning at ratio < 1.0 / 24h)
- `INSURANCE_NEAR_DEPLETION` (critical at ratio < 0.25)
- `REBATE_RESERVE_NONZERO_AT_LAUNCH` (critical during DEFER state)
- `EFFECTIVE_NEGATIVE_PPM_AT_LAUNCH` (critical during DEFER state)
- `PFV_FEE_BALANCE_GROWTH_STALL` (warning if flat > 1 week during active trading)
- Operator runbook calendar reminders for policy review cadence

---

## 9. Custody-track overall state

| Cluster | Status | Notes |
|---|---|---|
| Cluster 1 | RESOLVED PRIVATELY + chain-anchored | OPS + GOV Safes deployed and verified on Base mainnet |
| Cluster 2 | ARCH + POLICY DECIDED | Pattern C selected; vendor / region pending |
| Cluster 3 | POLICY + FORMULA DECIDED | Treasury Safe form locked; DEPLOYER post-migration policy; BE funding formula |
| **Cluster 4** | **POLICY DECIDED (final cluster)** | PFV receiver / rebates DEFER / insurance seeding / operator form / policy versioning |

**ALL 18 Q-CDs CLOSED.** Custody-track design phase is complete.
Remaining work is operational provisioning + parallel implementation
+ AUDIT-EXT engagement.

---

## 10. Files produced / updated by this milestone

| Path | Status |
|---|---|
| `~/DEOPT/private/mainnet_custody/MAINNET_CUSTODY_CLUSTER_4_RESOLUTION.private.md` | **CREATED** (mode 600, outside all repo trees) |
| `~/DEOPT/private/mainnet_custody/CLUSTER_HASHES.txt` | **APPENDED** (Cluster 4 sha256 entry; 5/5 sha256 --check OK) |
| `deopt-v2-backend/docs/MAINNET_CUSTODY_CLUSTER_4_RESOLUTION_REDACTED.md` | **CREATED** (public redacted summary) |
| `deopt-v2-backend/docs/MAINNET_CUSTODY_CLUSTER_4_NEXT_ACTIONS.md` | **CREATED** (this file) |
| `deopt-v2-backend/docs/MAINNET_CUSTODY_DECISION_DEPENDENCY_MAP.md` | **UPDATED** (Cluster 4 row status) |
| `deopt-v2-sol/docs/MAINNET_AUDIT_EXT_ENGAGEMENT_PACKAGE.md` | **UPDATED** (auditor questions Q-33..Q-38 appended) |
| `~/DEOPT/RUN_STATE.md` | **APPENDED** (redacted closure note) |

No source touched. No `.env` edit. No chain mutation. No Safe-tx. No fund movement. No Treasury / Insurance Safe / Fund creation.

---

## 11. Next milestone recommendation

**All 18 custody Q-CDs are closed.** The next milestones are operational provisioning + external engagement, not new custody policy decisions.

Primary recommendation: **`MAINNET-AUDIT-EXT-KICKOFF`** (P0-1) — ship the full Cluster 1 + 2 + 3 + 4 redacted closure summaries in the AUDIT-EXT handoff bundle. Longest external timeline (~10-12 weeks); gates the 17 BLOCKED_BY_AUDIT manifest slots.

Parallel tracks (can run concurrently):
1. **`MAINNET-TREASURY-SAFE-CREATION-PACKET`** — Q-CD-7 identity + Safe deployment.
2. **`MAINNET-INSURANCE-SEEDING-PARAMETER-FILL`** — Q-CD-12 numeric fill.
3. **`MAINNET-INSURANCE-OPERATOR-POLICY-PACKET`** — Q-CD-17 identity.
4. **`MAINNET-CUSTODY-POLICY-VERSIONING-SOP`** — Q-CD-18 changelog seed.
5. **`MAINNET-BE-FUNDING-POLICY-PARAMETER-FILL`** — Q-CD-9 numeric fill.
6. **`MAINNET-BE-SIGNER-SERVICE-DESIGN`** + **`MAINNET-KMS-VENDOR-SELECTION`** — Cluster 2 follow-ups.
7. **`BACKEND-SHOULD-BROADCAST-ECONOMIC-GATE`** — gap-list C-4.
8. **`MAINNET-MANIFEST-FILL-GOV-OPS-TREASURY-SLOTS`** — manifest fill PR with schema extensions.
9. **`MAINNET-PFV-REVENUE-WITHDRAWAL-SOP`** — Q-CD-10 cadence + Timelock op template.

---

## 12. Cross-links

- `~/DEOPT/MAINNET_CUSTODY_POLICY.md`
- `~/DEOPT/MAINNET_CUSTODY_DECISIONS_ADDENDUM_TEMPLATE.md` Q-CD-10 / 11 / 12 / 17 / 18
- `~/DEOPT/deopt-v2-backend/docs/MAINNET_CUSTODY_DECISION_DEPENDENCY_MAP.md`
- `~/DEOPT/deopt-v2-backend/docs/MAINNET_CUSTODY_CLUSTER_4_RESOLUTION_REDACTED.md`
- `~/DEOPT/deopt-v2-backend/docs/MAINNET_CUSTODY_CLUSTER_3_RESOLUTION_REDACTED.md`
- `~/DEOPT/deopt-v2-backend/docs/MAINNET_CUSTODY_CLUSTER_2_RESOLUTION_REDACTED.md`
- `~/DEOPT/deopt-v2-backend/docs/MAINNET_CUSTODY_CLUSTER_1_RESOLUTION_REDACTED.md`
- `~/DEOPT/deopt-v2-backend/docs/P0_MAINNET_BLOCKER_CLOSURE_ROADMAP.md`
- `~/DEOPT/deopt-v2-backend/docs/BACKEND_GAS_FEES_REBATES_POLICY_V1.md`
- `~/DEOPT/deopt-v2-backend/docs/BACKEND_EXECUTOR_MONITORING_ALERTS_V1.md`
- `~/DEOPT/deopt-v2-sol/docs/MAINNET_V2G_Y_OWNERSHIP_MIGRATION_PLAN.md`
- `~/DEOPT/deopt-v2-sol/docs/MAINNET_AUDIT_EXT_ENGAGEMENT_PACKAGE.md`
- `~/DEOPT/deopt-v2-sol/ROLE_MATRIX.md`
- `~/DEOPT/deopt-v2-sol/FINAL_LAUNCH_CHECKLIST.md`
- `~/DEOPT/RUN_STATE.md`

**End of Cluster 4 next-actions doc.**
