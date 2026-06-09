# Mainnet custody — Cluster 3 resolution (REDACTED public summary)

**Posture:** READ-ONLY redacted public summary. **No chain mutation.
No `.env` edit. No Safe-tx. No broadcast. No mainnet broadcast.
No Treasury Safe creation. No KMS key. No IAM role. No fund
movement.** Public companion to the private resolution artefact at
`~/DEOPT/private/mainnet_custody/MAINNET_CUSTODY_CLUSTER_3_RESOLUTION.private.md`
(mode 600, outside all repo trees).

**Date validated (UTC):** 2026-06-09

This document contains **NO** Treasury signer identities, personal
contacts, bank details, account IDs, wire instructions, private
funding sources, vendor credentials, API keys, private keys, seed
phrases, mnemonics, recovery phrases, RPC secrets, admin tokens, or
DATABASE_URL values. Only policy descriptions, the BE funding
formula (with placeholder fields), status classifications, and the
sha256 anchor for the private artefact.

---

## 0. Cluster 3 closure status

| Q-CD | Status | Decision label |
|---|---|---|
| **Q-CD-7** TREASURY Safe form | **POLICY-DECIDED** | Separate Safe v1.4.1, ≥ 3-of-5 hardware-wallet signers; **HARD disjoint** from OPS_SAFE_MAINNET roster; **default partial separation** from GOV_SAFE_MAINNET roster; no DEPLOYER as owner; no hot wallet as Treasury. Identity decision (roster + deployment) deferred to `MAINNET-TREASURY-SAFE-CREATION-PACKET`. |
| **Q-CD-8** DEPLOYER form after migration | **POLICY-DECIDED** | Post-V2G-Y DEPLOYER = deployment provenance only; **MUST NOT** be protocol owner, Timelock proposer/executor/guardian, NEW_OME executor, or owner of any of OPS / GOV / TREASURY Safes. Allowed residual: historical verification + non-privileged reference. Custody-policy §9.3 retirement procedure confirmed as canonical. Pre-migration form (Q-CD-8a — Safe vs hardware-wallet EOA for the deploy ceremony) still OPEN as separate sub-decision. |
| **Q-CD-9** BE FUND_FLOOR / TARGET / CEILING | **FORMULA-DECIDED-PARAMETERS-PENDING-OPERATOR-FILL** | `FUND_FLOOR = max(7d projected gas, emergency rotation budget)`; `FUND_TARGET = 3 × FUND_FLOOR`; `FUND_CEILING = min(10 × FUND_FLOOR, operator hot-balance cap)`. Recompute monthly. Numeric values filled at provisioning time by Operator + Finance + Backend in a private addendum. |

---

## 1. Q-CD-7 — TREASURY Safe form

### 1.1 Recommended architecture

| Property | Value |
|---|---|
| Form | Safe v1.4.1 SafeL2 on Base mainnet (or current production Safe version at deployment) |
| Threshold | **≥ 3-of-5 hardware-wallet signers** (minimum acceptable) |
| Owners | ≥ 5 hardware-wallet signers |
| Owner form | Hardware-wallet only (Ledger / Trezor / equivalent). No software, no browser-extension-only, no seed-phrase-only. |
| Roster disjointness vs OPS_SAFE_MAINNET | **HARD requirement** — no overlap |
| Roster disjointness vs GOV_SAFE_MAINNET | **default partial separation**; overlap permitted only with explicit operator approval + written rationale + AUDIT-EXT sign-off |
| DEPLOYER as owner | **MUST NOT** |
| Cold-custody posture | Signs only at scheduled treasury windows; not exposed to hot path |
| Hardware-vendor diversity | recommended |
| Geographic distribution | ≥ 2 jurisdictions |

### 1.2 Treasury responsibilities (what it DOES do)

- Fund BACKEND_EXECUTOR gas float (Q-CD-9 formula).
- Fund `PFV.rebateReserve` via Timelock-queued allocation if rebates are enabled at launch (Q-CD-11; recommended DEFER).
- Fund InsuranceFund if in launch scope (Q-CD-12 sizing pending).
- Receive PFV fee withdrawals if Q-CD-10 resolves to TREASURY-direct receiver (currently recommended Timelock-direct).
- Hold operational runway funds (vendor fees, audit fees, regulatory reserves).

### 1.3 Treasury restrictions (what it MUST NOT do)

- Be a protocol contract owner.
- Bypass Timelock — every chain-side protocol setter change goes via Timelock queue + 72h + execute.
- Sign BACKEND_EXECUTOR transactions.
- Hold raw BACKEND_EXECUTOR private key material.
- Be a Timelock proposer / executor / guardian.
- Be an OPS_MULTISIG or GOVERNANCE_MULTISIG owner.

### 1.4 Future milestone

**`MAINNET-TREASURY-SAFE-CREATION-PACKET`** — operator-authorised
future milestone that names signers (offline binder; placeholder labels
in repo), confirms disjointness, completes Sepolia rehearsal analogue,
pre-derives the CREATE2 address, broadcasts Safe deployment, and
records the address into manifest schema slot `governanceRoles.treasury`.

**This Cluster 3 milestone does NOT create the Treasury Safe.**

---

## 2. Q-CD-8 — DEPLOYER form after migration

### 2.1 Post-V2G-Y DEPLOYER MUST be

| Subject | Value |
|---|---|
| Protocol owner of any module | `false` (all 8 targets post-Y-E acceptOwnership) |
| Timelock proposer | `false` (post-Y-G-5a) |
| Timelock executor | `false` (post-Y-G-5b) |
| Timelock guardian | `false` (post-Y-G-2 → OPS_MULTISIG_MAINNET) |
| Timelock owner | NOT DEPLOYER (final = GOV_SAFE_MAINNET post-Y-G-4) |
| NEW_OME executor | `false` (post-Y-F-B-X-rm) |
| Module guardian on any target | `false` (post-Y-A → OPS_MULTISIG_MAINNET) |
| OPS_SAFE_MAINNET owner | `false` (verified at Cluster 1 closure) |
| GOV_SAFE_MAINNET owner | `false` (verified at Cluster 1 closure) |
| TREASURY_SAFE_MAINNET owner | `false` (Q-CD-7) |
| Backend executor (KMS-signed sender) | NOT DEPLOYER address (Cluster 2 Q-CD-5) |
| InsuranceFund operator | `false` (post Q-CD-17) |

### 2.2 Allowed residual usage

- Historical deployment verification (block explorer linkage, `forge verify-contract`).
- Non-privileged on-chain reference (event log provenance).
- Emergency re-authorization path — **DISCOURAGED**; canonical recovery uses a fresh DEPLOYER or operational Safe per `V2G_GOV_G_RESULT.md §8` forward-recovery sequence.

### 2.3 Manifest implications

- `chainMetadata.deployer` (line 11): may carry DEPLOYER provenance address; treated as non-privileged.
- All `governanceRoles.*` slots: MUST NOT contain DEPLOYER.
- **New schema field recommended:** `governanceRoles.deployerRetirementStatus` — opaque enum `{deployed, transferred, retired, archived}`.

### 2.4 V2G-Y HARD STOPS added by Q-CD-8 policy

Each phase boundary in `MAINNET_V2G_Y_OWNERSHIP_MIGRATION_PLAN.md` is augmented with the relevant DEPLOYER-not-owner / not-proposer / not-executor / not-guardian / not-OME-executor / not-Safe-owner sweep:

```text
Y-E-X-i acceptance     : DEPLOYER MUST NOT be owner of target i
Y-F-B-X-rm acceptance  : NEW_OME.isExecutor(DEPLOYER) = false
Y-G-4 acceptance       : Timelock.owner = GOV_SAFE_MAINNET; DEPLOYER not reachable as owner
Y-G-5a acceptance      : Timelock.proposers(DEPLOYER) = false
Y-G-5b acceptance      : Timelock.executors(DEPLOYER) = false
POST-Y-G-6 final sweep : DEPLOYER fully retired across 8 modules + Timelock
                         + NEW_OME + all 3 Safes + InsuranceFund. If ANY
                         check fails, mainnet activation BLOCKED.
```

The custody-policy §9.3 retirement procedure (drain DEPLOYER balance to TREASURY, archive deploy logs, destroy seed OR cold-storage seal) is confirmed as the canonical mainnet path.

### 2.5 Sub-decision still OPEN

- **Q-CD-8a pre-migration DEPLOYER form** (Safe vs hardware-wallet EOA for the deploy ceremony itself): default = dedicated deploy Safe (≥ 2-of-3). Still OPEN as a separate sub-decision; resolved alongside `MAINNET-DEPLOY-CEREMONY-DESIGN`.

---

## 3. Q-CD-9 — BE funding formula

### 3.1 Formula (locked)

```
projected_daily_gas_cost   = projected_daily_broadcasts
                           × avg_gas_per_broadcast
                           × expected_gas_price_wei

emergency_rotation_budget  = (revoke OLD + grant NEW + signer-service rotation ops
                              + emergency OPS_MULTISIG pause if applicable)
                           × expected_gas_price_wei

FUND_FLOOR                 = max(7 × projected_daily_gas_cost,
                                 emergency_rotation_budget)

FUND_TARGET                = 3 × FUND_FLOOR

FUND_CEILING               = min(10 × FUND_FLOOR,
                                 operator_hot_balance_cap_eth)
```

### 3.2 Example calculation template (placeholders only)

```yaml
inputs (operator-fill):
  projected_daily_broadcasts: ____
  avg_gas_per_broadcast: ____            # Sepolia evidence: ~700k (RFQ) / ~907k (orderbook)
                                          # mainnet-tuned post-staging
  expected_gas_price_gwei: ____           # conservative budgeted max
  emergency_rotation_budget_eth: ____
  operator_hot_balance_cap_eth: ____

derived:
  expected_gas_price_wei = <gwei> × 1e9
  projected_daily_gas_cost_wei = <daily_broadcasts> × <avg_gas> × <gas_price_wei>
  emergency_rotation_budget_wei = <emergency_eth> × 1e18

policy:
  FUND_FLOOR_wei = max(7 × projected_daily_gas_cost_wei,
                       emergency_rotation_budget_wei)
  FUND_TARGET_wei = 3 × FUND_FLOOR_wei
  FUND_CEILING_wei = min(10 × FUND_FLOOR_wei,
                         operator_hot_balance_cap_eth × 1e18)

recompute_cadence: monthly
recompute_owner: Operator + Finance + Backend
```

NO mainnet gas-price value is locked here. The 5 input numbers fill
into a private monthly recompute log at provisioning time.

### 3.3 Monitoring thresholds

| Threshold | Severity | Routing |
|---|---|---|
| `BE.balance < FUND_TARGET` for > 24h | warning | Discord (tighter than baseline `BE_BAL_LOW`) |
| `BE.balance < FUND_FLOOR` for > 5 min | critical | PagerDuty (existing `BE_BAL_LOW`) |
| `BE.balance < emergency_floor` (= 2 × emergency_rotation_budget) | critical + halt | PagerDuty + automatic backend halt of new broadcasts unless explicitly waived |
| `BE.balance > FUND_CEILING` for > 1h | warning | Discord (existing `BE_BAL_CEILING`) |
| `BE.balance` daily drift > expected_daily_gas_cost × 2 | warning | Discord (anomalous drain detection) |

### 3.4 Refilling policy

- **Refill source:** TREASURY_SAFE_MAINNET only. No other source.
- **Refill trigger:** alert at `< FUND_TARGET` (event) OR weekly scheduled cap review OR operator-initiated cap response.
- **Refill amount:** refresh to `FUND_TARGET` (not `FUND_CEILING`) — preserves headroom.
- **Refill SOP:** Treasury signers convene → Safe-tx prepared (`to = BE_MAINNET`, `value = FUND_TARGET - current_balance`) → ≥ 3-of-5 signatures → broadcast → log → post-refill monitoring confirms.
- **Audit log retention:** ≥ 1y; quarterly reconciliation against on-chain history.

### 3.5 Ceiling policy

- **Max BE hot balance:** `FUND_CEILING`.
- **When exceeded:** drain back to TREASURY via Safe-signed tx **from BE itself** (signer service §6.6 policy adds a `to = TREASURY_SAFE_MAINNET` special case gated by additional operator approval).
- **Why bounded:** custody policy P-6 + F-3 — compromised BE drains at most `FUND_CEILING` in gas, not the entire Treasury.
- **Ceiling growth:** operator approves an `operator_hot_balance_cap_eth` increase; recompute cadence picks it up.

### 3.6 Accounting (R-6 bright line)

- BE gas funding = **operational treasury expense**.
- PFV.feeBalance = **protocol revenue**; NOT auto-fund for BE gas.
- InsuranceFund = **protocol risk reserve**; NOT auto-fund for BE gas.
- Rebate reserve = **subsidy budget for users**; NOT auto-fund for BE gas.

Path PFV → TREASURY: only via Timelock-queued `PFV.withdrawRevenue` (Q-CD-10 receiver choice). Post-receipt at TREASURY, internal re-allocation to BE refill bucket is a Treasury internal accounting decision; the on-chain bright line (no protocol-side auto-fund) is preserved.

---

## 4. Implementation implications

- **No backend source change in this milestone.** BE funding policy enforced via (a) monitoring alert thresholds, (b) Treasury refill SOP, (c) signer-service drain-back policy (Cluster 2 follow-up).
- Monitoring (gap-list E-1..E-10): new alert tiers per §3.3 land alongside the broader monitoring wiring milestone.
- Signer service (Pattern C, Cluster 2): §6.6 policy adds the BE-drain-to-Treasury special case for ceiling breaches.
- No code change to `should_broadcast` (gap-list C-4) beyond the existing BE-low-balance gate.

---

## 5. Manifest implications

| Slot | Pre-Cluster-3 | Post-Cluster-3 |
|---|---|---|
| `chainMetadata.deployer` (line 11) | `NEEDS_OPERATOR_DECISION` | post-migration policy locked; pre-migration form (Q-CD-8a) still OPEN |
| `governanceRoles.deployerRetirementStatus` (NEW schema) | not defined | **recommended NEW field** — opaque enum |
| `governanceRoles.treasury` (NEW schema per custody policy §13.3) | proposed | **structurally clear** — value pending `MAINNET-TREASURY-SAFE-CREATION-PACKET` |
| `insuranceConfiguration.initialFundingAmountBase` (line 199) | NEEDS_OPERATOR_DECISION | Cluster 3 locks TREASURY-funded source path; sizing pending Q-CD-12 |
| BE funding thresholds | not in schema | **recommended NEW fields**: `funding.backendExecutor.fundFloorWei/.fundTargetWei/.fundCeilingWei/.recomputeCadenceMonths` (observable; not source-of-truth) |

---

## 6. Audit implications

Per `MAINNET_AUDIT_EXT_ENGAGEMENT_PACKAGE.md`:

- Auditor reviews **Treasury role boundary** — Treasury must have NO chain-side authority over protocol contracts (Q-CD-7 §1.3).
- Auditor reviews **DEPLOYER post-migration sweep** — all hard stops in §2.4 verifiable on chain.
- Auditor reviews **BE funding policy** — formula auditable; protocol-side funds never auto-reach BE without governance-gated path (§3.6 bright line).
- **New auditor questions** to append to `MAINNET_AUDIT_EXT_ENGAGEMENT_PACKAGE.md §7`:
  - **Q-31:** Can any code path move funds from PFV / InsuranceFund / CollateralVault directly into BE.balance without traversing TREASURY?
  - **Q-32:** Confirm DEPLOYER retirement procedure (§9.3) is the only path that disables DEPLOYER; confirm post-procedure DEPLOYER cannot be silently re-enabled.

---

## 7. V2G-Y implications

| Phase | Cluster 3 contribution |
|---|---|
| Y-A through Y-G-3 | unchanged (Cluster 1 parameterised) |
| Y-G-4 | unchanged |
| Y-G-5a / 5b | **HARD STOPS added** — verifier MUST confirm `Timelock.proposers(DEPLOYER) = false` and `Timelock.executors(DEPLOYER) = false` post-execute |
| Y-G-6 | unchanged (mainnet setMinDelay 72h) |
| POST-Y-G-6 final state audit | **NEW SWEEP** — verifies DEPLOYER fully retired across all 8 protocol modules + Timelock + NEW_OME + 3 Safes + InsuranceFund. If ANY check fails, mainnet activation BLOCKED; custody-policy §9.3 retirement HALTED. |
| Treasury operational onboarding | follows Y-G-6 final audit + `MAINNET-TREASURY-SAFE-CREATION-PACKET` + Cluster 2 BE EOA derivation |

---

## 8. Monitoring implications

- `BACKEND_EXECUTOR_MONITORING_ALERTS_V1.md §3.1 BE_BAL_LOW` threshold values become the §3.3 ladder from this resolution.
- New gauge metrics: `deopt_be_balance_vs_fund_target_ratio`, `deopt_be_balance_vs_fund_floor_ratio`, `deopt_be_balance_vs_emergency_floor_ratio`, `deopt_be_balance_vs_fund_ceiling_ratio`.
- New alert: `BE_DRAIN_PENDING` when BE > FUND_CEILING for > 1h.
- Treasury-side metrics (off-chain): refresh cadence, last refresh timestamp, drift since last refresh.

---

## 9. Remaining open decisions (post-Cluster-3)

- **Q-CD-7 identity decision** — Treasury signer roster + Safe deployment — `MAINNET-TREASURY-SAFE-CREATION-PACKET`.
- **Q-CD-8a** — pre-migration DEPLOYER form (Safe vs hardware-wallet EOA for the deploy ceremony).
- **Q-CD-9 numeric parameter fill** — 5 input values — `MAINNET-BE-FUNDING-POLICY-PARAMETER-FILL`.
- **Cluster 4:** Q-CD-10 PFV revenue receiver / Q-CD-11 rebates at launch / Q-CD-12 insurance seeding / Q-CD-16 rotation cadence (pre-resolved) / Q-CD-17 insurance operator form / Q-CD-18 policy versioning.
- AUDIT-EXT engagement, Cluster 2 vendor + region selection + KMS impl, monitoring + alerts wiring, V2G-W3 SSR proxy, Sepolia drill rehearsals, staging rehearsal.

---

## 10. Private artefact integrity anchor

```
Private artefact sha256 :
  2962a46a7be7ce016cb16dca579722c62faee99ddaac8a57d22a0779eb8b416e
Private artefact path :
  ~/DEOPT/private/mainnet_custody/MAINNET_CUSTODY_CLUSTER_3_RESOLUTION.private.md
  (mode 600; dir mode 700; outside all 3 git sub-repos)
Hash log path :
  ~/DEOPT/private/mainnet_custody/CLUSTER_HASHES.txt
  (mode 600; appended with Cluster 3 entry)
```

---

## 11. What this document does NOT contain

```text
- NO Treasury signer identities (placeholder labels only)
- NO Treasury Safe address (none deployed)
- NO bank details, account IDs, wire instructions, SWIFT/IBAN/routing
- NO mainnet BACKEND_EXECUTOR EOA address
- NO numeric FUND_FLOOR / TARGET / CEILING values (formula only; placeholders in §3.2)
- NO operator hot-balance cap value
- NO vendor credentials / API keys / IAM ARNs
- NO private keys / seed phrases / mnemonics / recovery phrases
- NO RPC API keys / admin tokens / DATABASE_URL values
- NO personal contacts / emails / phone numbers
```

The private resolution artefact at
`~/DEOPT/private/mainnet_custody/MAINNET_CUSTODY_CLUSTER_3_RESOLUTION.private.md`
(mode 600) is the operator's binder counterpart. Numeric values + roster + Safe address fill into the offline binder at provisioning.

---

## 12. Next milestone

`MAINNET-CUSTODY-CLUSTER-4-RESOLUTION` — operator + Risk + Finance + Insurance leads resolve **Q-CD-10** (PFV revenue receiver), **Q-CD-11** (rebates at launch), **Q-CD-12** (insurance initial seeding), **Q-CD-17** (insurance operator form), **Q-CD-18** (policy version cadence). Q-CD-16 (BE rotation cadence ≤ 30 days) was pre-resolved in custody policy §9.1.

In parallel:
1. **`MAINNET-TREASURY-SAFE-CREATION-PACKET`** — operator + Finance + Treasury; deploys Treasury Safe.
2. **`MAINNET-BE-FUNDING-POLICY-PARAMETER-FILL`** — operator + Finance + Backend; first numeric fill of the Q-CD-9 formula.
3. **`MAINNET-DEPLOY-CEREMONY-DESIGN`** — operator; resolves Q-CD-8a (pre-migration DEPLOYER form).
4. **`MAINNET-MANIFEST-FILL-GOV-OPS-TREASURY-SLOTS`** — read-only manifest-fill PR for Cluster 1 (already unblocked) + Cluster 3 (Treasury slot pending TREASURY Safe creation).
5. **`MAINNET-AUDIT-EXT-KICKOFF`** (P0-1) — ship handoff bundle.
6. **`MAINNET-BE-SIGNER-SERVICE-DESIGN`** + **`MAINNET-KMS-VENDOR-SELECTION`** (Cluster 2 follow-ups).
7. **`BACKEND-SHOULD-BROADCAST-ECONOMIC-GATE`** — gap-list C-4.

---

## 13. Cross-links

- `~/DEOPT/MAINNET_CUSTODY_POLICY.md` §3.1 R-6, §8, §9.3, §13.3
- `~/DEOPT/MAINNET_CUSTODY_DECISIONS_ADDENDUM_TEMPLATE.md` Q-CD-7 / 8 / 9
- `~/DEOPT/deopt-v2-backend/docs/MAINNET_CUSTODY_DECISION_DEPENDENCY_MAP.md`
- `~/DEOPT/deopt-v2-backend/docs/MAINNET_CUSTODY_CLUSTER_1_RESOLUTION_REDACTED.md`
- `~/DEOPT/deopt-v2-backend/docs/MAINNET_CUSTODY_CLUSTER_2_RESOLUTION_REDACTED.md`
- `~/DEOPT/deopt-v2-backend/docs/MAINNET_CUSTODY_CLUSTER_3_NEXT_ACTIONS.md` (companion — created by this milestone)
- `~/DEOPT/deopt-v2-backend/docs/BACKEND_EXECUTOR_MONITORING_ALERTS_V1.md`
- `~/DEOPT/deopt-v2-backend/docs/BACKEND_GAS_FEES_REBATES_POLICY_V1.md`
- `~/DEOPT/deopt-v2-sol/docs/MAINNET_V2G_Y_OWNERSHIP_MIGRATION_PLAN.md`
- `~/DEOPT/deopt-v2-sol/docs/MAINNET_AUDIT_EXT_ENGAGEMENT_PACKAGE.md`
- `~/DEOPT/deopt-v2-sol/ROLE_MATRIX.md`
- `~/DEOPT/deopt-v2-sol/FINAL_LAUNCH_CHECKLIST.md`
- `~/DEOPT/RUN_STATE.md`

**End of public redacted Cluster 3 resolution summary.**
