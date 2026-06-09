# Mainnet custody — Cluster 4 resolution (REDACTED public summary)

**Posture:** READ-ONLY redacted public summary. **No chain mutation.
No `.env` edit. No Safe-tx. No broadcast. No mainnet broadcast. No
fund movement. No rebate reserve allocation. No PFV withdrawal. No
Treasury Safe creation. No InsuranceFund creation. No source patch.**
Public companion to the private resolution artefact at
`~/DEOPT/private/mainnet_custody/MAINNET_CUSTODY_CLUSTER_4_RESOLUTION.private.md`
(mode 600, outside all repo trees).

**Date validated (UTC):** 2026-06-09

This document contains **NO** personal signer identities, personal
contacts, bank details, account IDs, wire instructions, IBAN / SWIFT
codes, raw funding source details, vendor credentials, API keys,
private keys, seed phrases, mnemonics, recovery phrases, RPC secrets,
admin tokens, or DATABASE_URL values. Only policy descriptions,
formula expressions with placeholder fields, status classifications,
the sha256 anchor for the private artefact, and the SemVer scheme for
the custody policy itself.

---

## 0. Cluster 4 closure status

**The final custody cluster.** All 18 Q-CDs are now closed at the
policy / formula / architecture layer (with Q-CD-16 pre-resolved
since custody policy ship).

| Q-CD | Status | Decision label |
|---|---|---|
| **Q-CD-10** PFV revenue receiver | **POLICY-DECIDED** | Revenue stays in PFV until Timelock-governed withdrawal; destination = TREASURY_SAFE_MAINNET (Q-CD-7) when created. R-6 accounting bright line preserved. Recommended hot-wallet option D explicitly **UNACCEPTABLE**. |
| **Q-CD-11** Rebates at launch | **POLICY-DECIDED: DEFERRED** | `PFV.rebateReserve(asset) = 0` at launch; all active FeesManagerV2 profiles MUST be effective non-negative. No rebate-positive trades. Re-evaluation after 3-month soak via separate `MAINNET-REBATE-PROGRAM-DESIGN`. Recommended option B (enabled with zero reserve) explicitly **UNACCEPTABLE**. |
| **Q-CD-12** Insurance initial seeding | **POLICY-DECIDED-PARAMETERS-PENDING** | Formula: `initial_insurance_seed = max_open_interest_cap × stress_loss_assumption × target_insurance_coverage_ratio`. Staged/capped recommended if liquidation surface in launch scope; no-insurance acceptable if liquidation OFF. Treasury-funded. |
| **Q-CD-16** BE rotation cadence | **PRE-RESOLVED** (custody policy §9.1) | ≤ 30 days OR after incident. Referenced only. |
| **Q-CD-17** Insurance operator form | **POLICY-DECIDED** | Recommended: dedicated insurance-operator Safe (≥ 2-of-3 minimum, 3-of-5 recommended), disjoint from OPS/GOV/TREASURY rosters; InsuranceFund owner = Timelock. Fallback: OPS_MULTISIG as operator (waiver + AUDIT-EXT sign-off; upgrade within 6 months). DEPLOYER as operator explicitly **UNACCEPTABLE** (post-V2G-Y Q-CD-8 violation). |
| **Q-CD-18** Custody policy version cadence | **POLICY-DECIDED** | SemVer `MAJOR.MINOR.PATCH`; freeze at v1.0.0 at first-live-smoke; quarterly review during beta (first 6 months); semi-annual review after stable; emergency review after any incident; PATCH = 1 sig, MINOR = 2 sigs, MAJOR = 3 sigs + AUDIT-EXT notification. |

**All 18 Q-CDs now resolved.** Sub-decisions / numeric fills / provisioning still queued as named follow-up milestones.

---

## 1. Q-CD-10 — PFV revenue receiver (POLICY-DECIDED)

### 1.1 Decision summary

- **At launch:** `PFV.revenueReceiver` points at Timelock OR `0x0` (sticky in PFV). No withdrawal SOP triggered until ≥ 30 d post-launch.
- **Post-soak + post-Q-CD-7 Treasury Safe deployment:** Governance proposal sets `PFV.revenueReceiver = TREASURY_SAFE_MAINNET`; Timelock queue + 72h + execute.
- **Hot-wallet destination:** **UNACCEPTABLE** per custody policy P-2 / P-6.
- **Governance Safe destination:** **DISCOURAGED** (role overlap GOV ↔ Treasury custody violates P-1 / R-6).

### 1.2 Three-gate withdrawal flow (R-6 bright line preserved)

```
PFV.feeBalance → PFV.withdrawRevenue (onlyOwner = Timelock)
              → GOV_SAFE_MAINNET proposes; OPS_MULTISIG executes; 72h delay
              → TREASURY_SAFE_MAINNET (Q-CD-7) receives
              → Treasury Safe-tx (≥ 3-of-5) re-allocates to:
                  ├─ BE gas refill (Q-CD-9 SOP)
                  ├─ rebate reserve top-up (Q-CD-11 — DEFERRED at launch)
                  └─ insurance refresh (Q-CD-12 — staged)
```

No path short-circuits this chain. PFV revenue can NEVER reach BE.balance without operator + governance approval at every step.

### 1.3 Future milestone

**`MAINNET-PFV-REVENUE-WITHDRAWAL-SOP`** — names withdrawal cadence (recommend monthly review, first cycle ≥ 30 d post-launch), records `PFV.revenueReceiver` destination decision (Timelock-direct vs TREASURY-direct), provides Timelock op template + GOV Safe-tx template + Treasury operational log entry shape.

---

## 2. Q-CD-11 — Rebates at launch (POLICY-DECIDED: DEFERRED)

### 2.1 Decision summary

- **Default: DEFERRED at launch.** `PFV.rebateReserve(asset) = 0` mirrors Sepolia rehearsal posture (where R5 drift = 0 was preserved across 2 live fee-only trades).
- All active FeesManagerV2 profiles at launch MUST be **effective non-negative** for every `(tier, product, ORDERBOOK|RFQ)`.
- Re-evaluation after **3-month soak + first AUDIT-EXT review window** via separate `MAINNET-REBATE-PROGRAM-DESIGN`.

### 2.2 Reasoning

- Sepolia parity (R5 drift = 0 invariant proven on the fee-only path).
- `should_broadcast` rebate-solvency hard gate is spec-only (gap-list C-4 / W-3); the chain-side `InsufficientRebateReserve` revert is wasteful as the only backstop.
- Anti-wash detection (`BACKEND_GAS_FEES_REBATES_POLICY_V1.md §6` Threat Table T-6) not implemented.
- Subsidy budget registry per reason (T-4) not implemented.
- Auditor visibility: rebate flow adds review surface; punting reduces launch engagement scope.
- Operational simplicity: Treasury bandwidth consumed by Q-CD-7 Safe deployment + Q-CD-12 insurance seeding.

### 2.3 Launch invariant (chain-side enforcement)

```text
[ LAUNCH INVARIANT — Q-CD-11 ]

For every active fee profile at launch:
  raw makerPpm                  >= 0
  raw takerPpm                  >= 0
  RFQ effective_maker_ppm       >= 0
  RFQ effective_taker_ppm       >= 0

AND chain-side:
  PFV.rebateReserve(asset)      = 0
  FM-V2.rebateBudget(asset)     MAY be > 0 historically (Sepolia: 999 947); never consumed
                                because no rebate-positive profile is active.

Verifier sweep before first-live-smoke MUST confirm these. If any
active profile has effective negative ppm AND rebateReserve = 0,
HALT launch — every such trade would revert InsufficientRebateReserve.
```

### 2.4 If rebates are later enabled (post-launch)

Requires the full activation sequence — Treasury-funded reserve via Timelock-queued allocation; backend `should_broadcast` solvency gate in code; anti-wash detection; capped daily budget per maker; reserve-depletion + budget-burn alerts; subsidy budget registry; AUDIT-EXT sign-off on rebate program design.

**`MAINNET-REBATE-PROGRAM-DESIGN`** is the future milestone if rebates are enabled. Cluster 4 does NOT pre-authorise it.

---

## 3. Q-CD-12 — Insurance initial seeding (POLICY-DECIDED-PARAMETERS-PENDING)

### 3.1 Decision summary

- **Recommended: staged / capped insurance** if liquidation surface is in launch scope (Option B).
- **Alternative: no insurance at launch** if liquidation surface OFF (Option A).
- **Full insurance launch with large reserve** (Option C) NOT recommended — over-provisioning ties up Treasury runway.

### 3.2 Formula (POLICY-DECIDED)

```yaml
inputs (operator-fill at provisioning):
  max_open_interest_cap_per_market: ____    # native settlement-asset units
  target_insurance_coverage_ratio:  ____    # fraction (e.g. 0.05 = 5% of OI cap)
  stress_loss_assumption:           ____    # max single-account bad-debt as fraction of OI under stress
  number_of_active_markets:         ____    # at launch

formula:
  per_market_initial_seed_units = max_open_interest_cap_per_market
                                × stress_loss_assumption
                                × target_insurance_coverage_ratio

  initial_insurance_seed_units  = sum over active markets of (per_market_initial_seed_units)
```

**NO mainnet numeric values are locked in this milestone.** Fill happens at provisioning per `MAINNET-INSURANCE-SEEDING-PARAMETER-FILL`.

### 3.3 Funding rules (R-6 preserved)

- **I-1:** Source = TREASURY_SAFE_MAINNET via `InsuranceFund.deposit`.
- **I-2:** Visible in Treasury operational log.
- **I-3:** Insurance reserve is **separated from PFV fee accounting**; independent counters; R-6 bright line prevents commingling.
- **I-4:** Insurance reserve **MUST NOT be used for BE gas**.
- **I-5:** Top-ups follow the same SOP: Treasury Safe-tx → InsuranceFund; logged + audited.
- **I-6:** Quarterly review of insurance ratio relative to live OI; recompute per formula.

### 3.4 Coupling with launch caps

Insurance seeding amount is co-decided with the launch OI caps (per `FINAL_LAUNCH_CHECKLIST.md` Configuration row). Smaller OI → smaller seed; larger seed enables wider OI but ties up runway. Numeric tuning in `MAINNET-INSURANCE-SEEDING-PARAMETER-FILL` is co-decided with launch-caps milestone.

---

## 4. Q-CD-17 — Insurance operator form (POLICY-DECIDED)

### 4.1 Decision summary

| Option | Verdict |
|---|---|
| A — Timelock-only owner; no operator allowlist | ACCEPTABLE but rigid |
| B — OPS_MULTISIG as operator + guardian | **FALLBACK** acceptable with waiver + AUDIT-EXT sign-off + upgrade within 6 months |
| **C — Dedicated insurance-operator Safe distinct from OPS / GOV / TREASURY** | **RECOMMENDED DEFAULT** |
| D — DEPLOYER as operator | **UNACCEPTABLE post-migration** (Q-CD-8 + V2G-Y HARD STOPS) |

### 4.2 Recommended architecture (Option C)

| Property | Value |
|---|---|
| Form | Safe v1.4.1 SafeL2 on Base mainnet |
| Threshold | ≥ 2-of-3 minimum (3-of-5 recommended) |
| Owners | hardware-wallet only |
| Roster | **disjoint** from OPS / GOV / TREASURY rosters |
| InsuranceFund owner | Timelock |
| InsuranceFund operator | Insurance Operator Safe (allowlist Timelock-gated) |
| Funding source | TREASURY_SAFE_MAINNET (Q-CD-7 + Q-CD-12) |

### 4.3 Allowed operator actions

- `InsuranceFund.deposit(asset, amount)` — Treasury → IF top-ups.
- `InsuranceFund.withdraw(asset, amount, to)` — post-event reconciliation; allowlist-gated.
- Yield ops (moveToStrategy / moveToIdle / sync) where authorised.
- Emergency pause (if contract supports guardian-style limited role).

### 4.4 Forbidden operator actions

- Transfer InsuranceFund ownership (Timelock only).
- Change guardian (Timelock only).
- Add/remove backstop callers (Timelock only).
- Change token allowlist (Timelock only).
- Bypass pause flags.
- Move funds outside allowlist scope.

### 4.5 Claim / liquidation path

Backstop callers = canonical `MarginEngine` + `PerpEngine` only (per `ROLE_MATRIX.md` "Insurance Backstop Caller"). Operator does NOT sign per-claim transactions; engines invoke `VaultBackstopPaid` automatically on shortfall.

### 4.6 Future milestone

**`MAINNET-INSURANCE-OPERATOR-POLICY-PACKET`** — resolves Option B vs Option C identity decision, names operator Safe signers (offline binder; placeholder labels only), drafts allowed/forbidden surface enforcement test plan.

---

## 5. Q-CD-18 — Custody policy version cadence (POLICY-DECIDED)

### 5.1 Versioning scheme: SemVer `MAJOR.MINOR.PATCH`

| Bump | Trigger |
|---|---|
| PATCH | Editorial / clarification / typo / cross-reference fix; no policy semantic change |
| MINOR | New section added; sub-clause refined; new Q-CD opened/closed; no breaking change |
| MAJOR | Existing decision overturned; new role added/removed; bright line changed; breaking change |

### 5.2 Freeze + review cadence

- **v1.0.0 freeze:** at mainnet first-live-smoke gate (4-signature attestation per `FIRST_LIVE_SMOKE_AUTHORIZATION` mainnet variant §10).
- **Beta period (launch → +6 months):** quarterly review.
- **Stable period (+6 months onward):** semi-annual review.
- **Emergency review:** triggered by any incident, independent of schedule.

### 5.3 Approval matrix

| Bump | Required signatures |
|---|---|
| PATCH | Operator OR Security (1) |
| MINOR | Operator + Security (2) |
| MAJOR | Operator + Security + Governance + AUDIT-EXT notification (3 + 1 notification) |

### 5.4 Change-log

Each bump appends a row to a new file `~/DEOPT/MAINNET_CUSTODY_POLICY_CHANGELOG.md` (to be seeded at v1.0.0) with: version, date, bump class, summary, affected sections, driving event, signer labels, AUDIT-EXT notification flag, entry sha256.

### 5.5 RUN_STATE reference

Each bump appends a concise one-liner to RUN_STATE.md analogous to the cluster-closure pattern; no sensitive details.

### 5.6 Future milestone

**`MAINNET-CUSTODY-POLICY-VERSIONING-SOP`** — produces `~/DEOPT/MAINNET_CUSTODY_POLICY_CHANGELOG.md`, publishes approval matrix, wires RUN_STATE reference into operator runbook, sets v1.0.0 seed at first-live-smoke.

---

## 6. Implementation implications

- **Backend code:** no Cluster 4-specific change required. `should_broadcast` C-4 implementation still needed independently for full pre-mainnet posture; Q-CD-11 launch-DEFER doesn't unblock it but also doesn't gate launch as long as the §2.3 invariant holds.
- **Monitoring:** new InsuranceFund metrics + rebate-DEFER invariant alerts (see §10).
- **Solidity:** no source change; all decisions execute via existing setters + Timelock paths.
- **Treasury operational:** PFV withdrawal cadence + insurance funding cadence enter Treasury cap-review calendar.
- **Policy ops:** versioning calendar wired into operator runbook.

---

## 7. Manifest implications

| Slot (line) | Pre-Cluster-4 | Post-Cluster-4 |
|---|---|---|
| `feesConfiguration.feeRecipient` (193) | NEEDS_DEPLOYMENT | Q-CD-10 confirms PFV (Sepolia analogue); value pending PFV mainnet deploy |
| `feesConfiguration.merkleRoot` (194) | NEEDS_OPERATOR_DECISION | Q-CD-11 = DEFER → launch root may be `bytes32(0)` if tier 0 only OR non-rebate-bearing root; rebate-positive root deferred |
| `insuranceConfiguration.initialFundingAmountBase` (199) | NEEDS_OPERATOR_DECISION | Q-CD-12 formula locked; value pending `MAINNET-INSURANCE-SEEDING-PARAMETER-FILL` |
| `insuranceConfiguration.operators[0]` (204) | NEEDS_OPERATOR_DECISION | Q-CD-17 = dedicated Safe recommended (fallback OPS); identity pending `MAINNET-INSURANCE-OPERATOR-POLICY-PACKET` |
| `insuranceConfiguration.allowedTokens[0]` (201) | mUSDC analogue | mainnet USDC per gap-list N-1 (canonical Base USDC) |
| `insuranceConfiguration.backstopCallers[0]` / `[1]` (207-208) | MARGIN_ENGINE / PERP_ENGINE | Q-CD-12 confirms canonical engines only |
| New schema fields recommended | | |
| `feesConfiguration.rebateLaunchPolicy` | not defined | enum `{deferred, enabled, capped}` — initial `deferred` |
| `insuranceConfiguration.operatorForm` | not defined | enum `{ops-multisig, dedicated-safe, timelock-only}` |
| `custodyPolicyVersion` (joined from Cluster 1 schema extension) | proposed | confirmed SemVer string; initial `v1.0.0` at first-live-smoke |
| `custodyPolicyChangeLogSha256` | not defined | latest change-log entry sha256 |

---

## 8. Audit implications (6 new auditor questions)

Append to `MAINNET_AUDIT_EXT_ENGAGEMENT_PACKAGE.md §7`:

- **Q-33:** Confirm PFV revenue receiver path (PFV → Timelock withdrawal → TREASURY → re-allocation) cannot be short-circuited; `withdrawRevenue` callable only by owner = Timelock.
- **Q-34:** Confirm launch invariant — all active FeesManagerV2 profiles produce effective non-negative ppm AND `PFV.rebateReserve = 0`; no code path credits a rebate without going through chain-side `InsufficientRebateReserve` revert if reserve is 0.
- **Q-35:** Document the activation gate for future rebate enablement (Treasury-funded reserve + Timelock allocation + backend solvency gate + anti-wash + AUDIT-EXT-2 sign-off).
- **Q-36:** Confirm `InsuranceFund.balance` and `PFV.feeBalance` are independent counters; no code path conflates them; insurance reserve cannot be transferred without operator/Timelock authorisation.
- **Q-37:** Review post-deployment InsuranceFund operator allowlist; confirm operator cannot escalate to owner/guardian; allowed actions match §4.3.
- **Q-38:** Review SemVer + change-log + approval matrix governance for custody policy bumps; confirm MAJOR bumps require AUDIT-EXT notification.

---

## 9. V2G-Y implications

| Phase | Cluster 4 contribution |
|---|---|
| Y-A through Y-G-6 | unchanged |
| **POST-Y-G-6 final state audit** | **NEW LAUNCH INVARIANT** added — verifier sweeps all active FeesManagerV2 profiles confirming effective non-negative ppm AND `rebateReserve = 0`. If violated, mainnet first-live-smoke gate BLOCKED. |
| Treasury operational onboarding | Cluster 4 §1.3 PFV withdrawal SOP joins post-Y-G-6 operational checklist |
| Insurance operational onboarding | Cluster 4 §3.5 / §4.6 milestones run parallel to Treasury onboarding |

---

## 10. Monitoring implications

- **InsuranceFund metrics** (extend `BACKEND_EXECUTOR_MONITORING_ALERTS_V1.md §2.7`):
  - `deopt_insurance_fund_balance_units` (gauge per asset).
  - `deopt_insurance_fund_target_seed_ratio` (gauge).
  - Alert `INSURANCE_BELOW_TARGET` when ratio < 1.0 for > 24h → Discord warning.
  - Alert `INSURANCE_NEAR_DEPLETION` when ratio < 0.25 → PagerDuty critical.
- **Rebate-DEFER invariant alerts (NEW Cluster 4):**
  - `REBATE_RESERVE_NONZERO_AT_LAUNCH` if `PFV.rebateReserve(asset) > 0` while custody policy in DEFER → critical (config drift).
  - `EFFECTIVE_NEGATIVE_PPM_AT_LAUNCH` if any active profile reports effective negative ppm while DEFER → critical.
- **PFV revenue (existing §2.7) augmented:**
  - `PFV_FEE_BALANCE_GROWTH_STALL` if balance flat > 1 week during active trading.
- **Policy versioning (off-chain operator runbook):**
  - Quarterly review reminder (calendar event during beta).
  - Emergency review trigger on any incident.

---

## 11. Private artefact integrity anchor

```
Private artefact sha256 :
  e555bfd7e4818b8a910013d55b7e3fff985c4f041cb0b81378b7668d4353d68f
Private artefact path :
  ~/DEOPT/private/mainnet_custody/MAINNET_CUSTODY_CLUSTER_4_RESOLUTION.private.md
  (mode 600; dir mode 700; outside all 3 git sub-repos)
Hash log path :
  ~/DEOPT/private/mainnet_custody/CLUSTER_HASHES.txt
  (mode 600; 5 entries — Cluster 1 input + Cluster 1/2/3/4 resolutions)
sha256sum --check : 5/5 OK ✓
```

---

## 12. Remaining open decisions (post all Cluster closures)

### Cluster 4 sub-decisions / numeric fills

- Q-CD-10 cadence + final destination → `MAINNET-PFV-REVENUE-WITHDRAWAL-SOP`.
- Q-CD-11 rebate program design (if later enabled) → `MAINNET-REBATE-PROGRAM-DESIGN`.
- Q-CD-12 numeric formula input fill → `MAINNET-INSURANCE-SEEDING-PARAMETER-FILL`.
- Q-CD-17 Safe identity + signers → `MAINNET-INSURANCE-OPERATOR-POLICY-PACKET`.
- Q-CD-18 changelog file seed → `MAINNET-CUSTODY-POLICY-VERSIONING-SOP`.

### Outstanding non-custody decisions / milestones

- Cluster 1 follow-up: rehearsal log + sign-off label back-fill.
- Cluster 2 follow-ups: KMS vendor + region selection, signer service design, backend KMS adapter impl.
- Cluster 3 follow-ups: Treasury Safe creation, BE funding parameter fill, deploy-ceremony design.
- AUDIT-EXT engagement (P0-1).
- Mainnet manifest fill + schema extensions.
- Mainnet protocol contract deployment.
- Backend `should_broadcast` impl (gap-list C-4).
- Monitoring + alerts wiring (E-1..E-10).
- V2G-W3 SSR proxy + admin OIDC/MFA + Strict CSP.
- Sepolia drill rehearsals (M-1 / M-3 / D-6); staging rehearsal (L-5/L-6/L-7).

---

## 13. What this document does NOT contain

```text
- NO Treasury / Insurance signer identities
- NO Treasury Safe / Insurance operator Safe addresses (none deployed)
- NO bank details / wire instructions / IBAN / SWIFT
- NO mainnet BACKEND_EXECUTOR EOA address
- NO numeric insurance seed amount (formula + placeholders only)
- NO numeric rebate budget (rebates DEFERRED)
- NO numeric PFV withdrawal cadence value
- NO operator hot-balance cap value
- NO vendor credentials / API keys / IAM ARNs
- NO private keys / seed phrases / mnemonics / recovery phrases
- NO RPC API keys / admin tokens / DATABASE_URL values
- NO personal contacts / emails / phone numbers
```

The private resolution artefact at
`~/DEOPT/private/mainnet_custody/MAINNET_CUSTODY_CLUSTER_4_RESOLUTION.private.md`
(mode 600) is the operator's binder counterpart.

---

## 14. Next milestone

**All 18 custody Q-CDs are now closed at the policy / formula / architecture layer.** The next custody-track milestones are operational provisioning (not new policy decisions).

Primary recommendation: **`MAINNET-AUDIT-EXT-KICKOFF`** (P0-1) — ship Cluster 1 + 2 + 3 + 4 redacted closure summaries in the AUDIT-EXT handoff bundle. This is the longest external timeline (~10-12 weeks) and gates 17 BLOCKED_BY_AUDIT manifest slots.

In parallel:
1. **`MAINNET-TREASURY-SAFE-CREATION-PACKET`** — Q-CD-7 identity decision + Safe deployment (Cluster 3 follow-up).
2. **`MAINNET-INSURANCE-SEEDING-PARAMETER-FILL`** — Q-CD-12 numeric fill (this Cluster 4 follow-up).
3. **`MAINNET-INSURANCE-OPERATOR-POLICY-PACKET`** — Q-CD-17 identity decision (this Cluster 4 follow-up).
4. **`MAINNET-CUSTODY-POLICY-VERSIONING-SOP`** — Q-CD-18 changelog file + v1.0.0 seed (this Cluster 4 follow-up).
5. **`MAINNET-BE-FUNDING-POLICY-PARAMETER-FILL`** — Q-CD-9 numeric fill (Cluster 3 follow-up).
6. **`MAINNET-BE-SIGNER-SERVICE-DESIGN`** + **`MAINNET-KMS-VENDOR-SELECTION`** — Cluster 2 follow-ups.
7. **`BACKEND-SHOULD-BROADCAST-ECONOMIC-GATE`** — gap-list C-4.
8. **`MAINNET-MANIFEST-FILL-GOV-OPS-TREASURY-SLOTS`** — read-only manifest fill PR with schema extensions.

---

## 15. Cross-links

- `~/DEOPT/MAINNET_CUSTODY_POLICY.md` §2 / §3 / §7 / §8 / §9 / §13
- `~/DEOPT/MAINNET_CUSTODY_DECISIONS_ADDENDUM_TEMPLATE.md` Q-CD-10 / 11 / 12 / 17 / 18
- `~/DEOPT/deopt-v2-backend/docs/MAINNET_CUSTODY_DECISION_DEPENDENCY_MAP.md`
- `~/DEOPT/deopt-v2-backend/docs/MAINNET_CUSTODY_CLUSTER_1_RESOLUTION_REDACTED.md`
- `~/DEOPT/deopt-v2-backend/docs/MAINNET_CUSTODY_CLUSTER_2_RESOLUTION_REDACTED.md`
- `~/DEOPT/deopt-v2-backend/docs/MAINNET_CUSTODY_CLUSTER_3_RESOLUTION_REDACTED.md`
- `~/DEOPT/deopt-v2-backend/docs/MAINNET_CUSTODY_CLUSTER_4_NEXT_ACTIONS.md` (companion — created by this milestone)
- `~/DEOPT/deopt-v2-backend/docs/BACKEND_GAS_FEES_REBATES_POLICY_V1.md`
- `~/DEOPT/deopt-v2-backend/docs/BACKEND_EXECUTOR_MONITORING_ALERTS_V1.md`
- `~/DEOPT/deopt-v2-sol/docs/MAINNET_V2G_Y_OWNERSHIP_MIGRATION_PLAN.md`
- `~/DEOPT/deopt-v2-sol/docs/MAINNET_AUDIT_EXT_ENGAGEMENT_PACKAGE.md`
- `~/DEOPT/deopt-v2-sol/ROLE_MATRIX.md`
- `~/DEOPT/deopt-v2-sol/PARAMETERS.md`
- `~/DEOPT/deopt-v2-sol/FINAL_LAUNCH_CHECKLIST.md`
- `~/DEOPT/RUN_STATE.md`

**End of public redacted Cluster 4 resolution summary.**
