# Prebuild → build handoff

**Posture:** READ-ONLY handoff. **No chain mutation. No `.env` edit.
No Safe-tx. No broadcast. No mainnet. No source patch in this
milestone.** Bridges the design / policy phase (all 4 custody Clusters
+ P0 foundation pack) to the implementation phase. Establishes
recommended build order, acceptance criteria per task, and hard
stops before any mainnet tx.

**Date:** 2026-06-09
**Operator side:** Operator + Backend lead + Security lead
**Builder side:** Backend implementer (developer / agent)

---

## 0. Hard stops (this doc + every derived build task)

```text
no chain tx                                            ✅
no Safe tx                                             ✅
no backend broadcast outside Sepolia rehearsal         ✅
no DB mutation outside backend integration tests       ✅
no .env edit outside operator session                  ✅
no mainnet execution                                   ✅
no Treasury Safe / InsuranceFund / KMS key creation    ✅
no rebate reserve allocation                           ✅
no PFV withdrawal                                      ✅
no ownership / guardian / Timelock mutation            ✅
no private keys / RPC URL / admin token / DATABASE_URL output ✅
no guessed mainnet addresses                           ✅
no guessed vendor credentials                          ✅
```

The build order below assumes Sepolia integration testing is OK
(rehearsal arc continues if needed); **mainnet broadcast is NOT
authorised** until the gates in §5 are met.

---

## 1. What is now complete (design + policy + Sepolia evidence)

### 1.1 Design / policy

- **Custody policy** v1.0.0-pre: `~/DEOPT/MAINNET_CUSTODY_POLICY.md` (12 principles + 7-role model + Safe architecture + KMS architecture + 8 rotation/incident procedures + monitoring + 12 cross-links).
- **All 18 Q-CD decisions CLOSED** across 4 Cluster resolutions (1: rosters + thresholds + Sepolia rehearsal; 2: Pattern C + topology + region + key-deletion; 3: Treasury + DEPLOYER + BE funding formula; 4: PFV receiver + rebates DEFER + insurance + policy versioning).
- **P0 foundation pack** complete: gap list (14 domains; 14 Sepolia waivers; P0/P1/P2/P3); AUDIT-EXT engagement package (Q-1..Q-38 auditor questions; 9-section handoff bundle); manifest TODO inventory (99 placeholders / 76 distinct slots / 6 tag classes); V2G-Y migration plan (Y-A → Y-G phases + 2 launch invariants).
- **Sepolia rehearsal arc structurally complete** on both option-execution surfaces (orderbook + RFQ) — R5 drift = 0 preserved across 7 governance tx + 2 live fee-only trades.

### 1.2 Mainnet Safes chain-anchored

- OPS_SAFE_MAINNET `0xce0e46Db1072B820CB5eCf30188ED76cb560C932` — Safe v1.4.1, 2-of-3, nonce 0, owners disjoint from GOV.
- GOV_SAFE_MAINNET `0x7C6Ce20eED2b633b4FF4A2e2387E437abc96b166` — Safe v1.4.1, 3-of-5, nonce 0, owners disjoint from OPS.

### 1.3 Audit kickoff bundle ready

- `deopt-v2-sol/docs/MAINNET_AUDIT_EXT_KICKOFF_BUNDLE.md` + `MAINNET_AUDIT_HANDOFF_INDEX.md` shipped (this milestone).

---

## 2. What remains open

### 2.1 Implementation work (this handoff's scope)

| Task | Owner | Status |
|---|---|---|
| `BACKEND-SHOULD-BROADCAST-ECONOMIC-GATE` | Backend + Risk | **NEXT** — first build task |
| `MAINNET-BE-SIGNER-SERVICE-DESIGN` | Backend + Security | follow-up (read-only design) |
| `BACKEND-SIGNER-INTERFACE-KMS-HSM-ADAPTER` | Backend | depends on signer-service design + Cluster 2 vendor sub-decision |
| `BACKEND-MONITORING-ALERTS-WIRING` | Backend + SRE | gap-list E-1..E-10 |
| `OPTION-EXECUTION-TX-VISIBILITY-FIX` | Backend | fixes `/executor/transactions/<intent>` returning `[]` for option intents (gap-list C-12) |
| `FRONTEND-V2G-W3-SSR-PROXY` | Frontend + Backend + Security | gap-list J-1 / F-H1 / B-H1 closure |
| `BACKEND-SHOULD-BROADCAST-UNIT-TESTS` | Backend | gap-list C-15 / T-10 |
| `BACKEND-DEDUPE-NONCE-STORE` | Backend | gap-list C-7 |
| `BACKEND-WASH-DETECTION` | Backend + Risk | gap-list C-6 |
| `BACKEND-SUBSIDY-BUDGET-LEDGER` | Backend + Finance | gap-list C-8 |

### 2.2 Provisioning / operator work (out of scope here; tracked elsewhere)

- `MAINNET-KMS-VENDOR-SELECTION` (Q-CD-5 sub-decision) — operator + Security + Backend.
- `MAINNET-KMS-REGION-FINALISATION` (Q-CD-14 sub-decision) — operator + DevOps.
- `MAINNET-TREASURY-SAFE-CREATION-PACKET` (Q-CD-7 identity decision) — operator + Finance + Treasury.
- `MAINNET-INSURANCE-SEEDING-PARAMETER-FILL` (Q-CD-12 numeric) — operator + Insurance + Finance + Risk.
- `MAINNET-INSURANCE-OPERATOR-POLICY-PACKET` (Q-CD-17 identity) — operator + Insurance.
- `MAINNET-CUSTODY-POLICY-VERSIONING-SOP` (Q-CD-18 changelog seed) — operator + Security.
- `MAINNET-BE-FUNDING-POLICY-PARAMETER-FILL` (Q-CD-9 numeric) — operator + Finance + Backend.
- `MAINNET-PFV-REVENUE-WITHDRAWAL-SOP` (Q-CD-10 cadence) — Governance + Finance.
- `MAINNET-DEPLOY-CEREMONY-DESIGN` (Q-CD-8a pre-migration DEPLOYER form) — operator + Security.

### 2.3 Audit / rehearsal work (out of scope here)

- AUDIT-EXT engagement execution (~10-12 wk external timeline).
- Sepolia drill rehearsals (M-1 / M-3 / D-6) — guardian pause / BE compromise / forward-recovery.
- Staging rehearsal (L-5 / L-6 / L-7).
- Liquidation / Risk / Oracle Sepolia rehearsal — only if liquidation surface in launch scope (Cluster 4 Q-CD-12 Option B).
- Mainnet fork rehearsal.
- V2G-Y mainnet ownership migration execution.

---

## 3. What MUST be implemented before mainnet

| Item | Rationale | Closure milestone |
|---|---|---|
| `should_broadcast` economic gate | gap-list C-4 / W-3 + Cluster 4 launch invariant verifier sweep | `BACKEND-SHOULD-BROADCAST-ECONOMIC-GATE` |
| KMS/HSM/MPC signer path (Pattern C) | gap-list D-1 / W-5 + Cluster 2 Q-CD-5; backend MUST refuse `EXECUTOR_PRIVATE_KEY` on `chain_id=8453` | `BACKEND-SIGNER-INTERFACE-KMS-HSM-ADAPTER` |
| Monitoring + alerts wiring | gap-list E-1..E-10 + Cluster 4 InsuranceFund metrics + Cluster 4 rebate-DEFER alerts | `BACKEND-MONITORING-ALERTS-WIRING` |
| Option execution tx visibility | gap-list C-12 (operational UX fix) | `OPTION-EXECUTION-TX-VISIBILITY-FIX` |
| V2G-W3 SSR proxy + OIDC/MFA + Strict CSP | gap-list J-1 / J-5 / J-6 + V2G-AUDIT0 F-H1 / B-H1 | `FRONTEND-V2G-W3-SSR-PROXY` |
| Manifest fill (Cluster 1-4 unlocked slots + 13 Group A + new schema fields) | gap-list K-1 | `MAINNET-MANIFEST-FILL-GOV-OPS-TREASURY-SLOTS` |
| Mainnet protocol contracts deployed | DeployCore.s.sol + downstream | separate operator milestones |
| V2G-Y mainnet ownership migration Y-A → Y-G | Cluster 1/2/3/4 chain-anchored | per-phase packet milestones |
| **POST-Y-G-6 launch invariant verifier (Cluster 4)** | rebate-DEFER + R5 = 0 + DEPLOYER fully retired | implementation in `BACKEND-SHOULD-BROADCAST-ECONOMIC-GATE` + operator-side sweep script |

---

## 4. Recommended build order

The order below is optimised for: (a) independent progress in parallel where possible; (b) minimum coupling between tasks; (c) early closure of the auditor's blocking observations; (d) mainnet broadcast safety.

```
=================  PARALLEL TRACKS  =================

[ Track 1 — Backend code ]                            [ Track 2 — Operator decisions ]
  1. BACKEND-SHOULD-BROADCAST-ECONOMIC-GATE             1a. MAINNET-KMS-VENDOR-SELECTION       (Q-CD-5)
     → unblocks all subsequent backend builds            1b. MAINNET-KMS-REGION-FINALISATION    (Q-CD-14)
                                                         2.  MAINNET-TREASURY-SAFE-CREATION-PACKET (Q-CD-7)
  2. MAINNET-BE-SIGNER-SERVICE-DESIGN                    3.  MAINNET-INSURANCE-OPERATOR-POLICY-PACKET (Q-CD-17)
     (read-only design milestone; depends on Q-CD-5      4.  MAINNET-INSURANCE-SEEDING-PARAMETER-FILL (Q-CD-12)
      vendor sub-decision being made by operator         5.  MAINNET-CUSTODY-POLICY-VERSIONING-SOP (Q-CD-18)
      in parallel)                                       6.  MAINNET-BE-FUNDING-POLICY-PARAMETER-FILL (Q-CD-9)
                                                         7.  MAINNET-PFV-REVENUE-WITHDRAWAL-SOP (Q-CD-10)
  3. BACKEND-SIGNER-INTERFACE-KMS-HSM-ADAPTER            8.  MAINNET-DEPLOY-CEREMONY-DESIGN     (Q-CD-8a)
     (depends on signer-service design + vendor)

  4. BACKEND-MONITORING-ALERTS-WIRING                   [ Track 3 — Audit engagement ]
     (depends on signer adapter for sign-rate            9.  MAINNET-AUDIT-EXT-KICKOFF
      metrics + auth + alert routing)                       (ship handoff bundle + handoff index)
                                                         10. AUDIT-EXT-ACTIVE-REVIEW (3-7 wk)
  5. OPTION-EXECUTION-TX-VISIBILITY-FIX                  11. AUDIT-EXT-REMEDIATION-AND-CLOSURE (4-6 wk)
     (operational UX; no dependency)

  6. FRONTEND-V2G-W3-SSR-PROXY
     (parallel; closes F-H1 + B-H1 mainnet blockers)

=================  SEQUENTIAL GATES  =================

  7. MAINNET-MANIFEST-FILL-GOV-OPS-TREASURY-SLOTS
     (depends on Tracks 1-2-3 producing the data;
      schema PR can land earlier)

  8. LIQUIDATION/RISK-REHEARSAL-SEPOLIA
     (only if liquidation in launch scope per Q-CD-12;
      requires staging rehearsal infrastructure)

  9. MAINNET-FORK-REHEARSAL
     (full deploy + handoff + smoke + drills against a
      mainnet fork; gates Y-A broadcast authorisation)

 10. V2G-Y-MAINNET-OWNERSHIP-MIGRATION
     (Y-A → Y-G phase packets; ≥ 7-day total wall-clock
      with two 72h Timelock cycles)
```

### 4.1 Why this order

- **Tracks 1, 2, 3 run in parallel.** Each builds on the closed Cluster 1/2/3/4 decisions and consumes them as inputs.
- **BACKEND-SHOULD-BROADCAST-ECONOMIC-GATE is the first build task** because: (a) it's the most-blocking single P1 item per gap-list / engagement-package §3; (b) it closes the auditor's Q-34 launch invariant verifier requirement; (c) it depends on no other implementation milestones; (d) it provides regression coverage for all subsequent broadcast-path PRs.
- **MAINNET-BE-SIGNER-SERVICE-DESIGN** comes second because the design output drives the subsequent `BACKEND-SIGNER-INTERFACE-KMS-HSM-ADAPTER` PR and the operator-side KMS vendor selection is in parallel.
- **MAINNET-MANIFEST-FILL** is sequential because it depends on all parallel tracks producing the data it fills.
- **V2G-Y EXECUTION is the last broadcast-side step** because every prior task gates it.

---

## 5. Acceptance criteria per task

### 5.1 `BACKEND-SHOULD-BROADCAST-ECONOMIC-GATE`

```text
[ ] should_broadcast (or equivalent policy gate) implemented per
    BACKEND_GAS_FEES_REBATES_POLICY_V1.md §8 pseudocode
[ ] each §8 branch covered by unit test (T-10)
[ ] launch invariant verifier sweep (Cluster 4):
    - all active fee profiles effective non-negative
    - PFV.rebateReserve(asset) == 0 at launch
    - returns False if violated
[ ] target/selector allowlist enforced (chainId / NEW_OME / executeTrade or executeRfqTrade)
[ ] BE.balance >= FUND_FLOOR (chain-side check) gates broadcast
[ ] gas cap enforced (≤ OPTION_EXECUTION_BROADCAST_GAS_LIMIT)
[ ] nonce / deadline / dedupe gates green
[ ] integration tests: Sepolia orderbook + RFQ pass; regression
[ ] no source side effects outside backend repo
[ ] no mainnet broadcast attempted
```

### 5.2 `MAINNET-BE-SIGNER-SERVICE-DESIGN`

```text
[ ] design doc deopt-v2-backend/docs/MAINNET_BE_SIGNER_SERVICE_DESIGN.md
[ ] mTLS server architecture; per-sign request/response schema
[ ] §6.6 transaction policy precheck specified
[ ] vendor adapter interface
[ ] per-sign log shape with request_id propagation
[ ] failover client to secondary region
[ ] emergency disable / pause endpoint
[ ] deployment shape (VPC-isolated)
[ ] versioning + rollback policy
[ ] no code in this milestone; design only
```

### 5.3 `BACKEND-SIGNER-INTERFACE-KMS-HSM-ADAPTER`

```text
[ ] RemoteSigner trait added in src/execution/signer.rs
[ ] ExecutorSigner::from_private_key retained for Sepolia/tests
[ ] KmsRemoteSigner::from_service_endpoint(endpoint) impl for mainnet
[ ] new env keys: BACKEND_SIGNER_ENDPOINT + mTLS cert paths
[ ] startup guard: REFUSE EXECUTOR_PRIVATE_KEY on chain_id=8453
[ ] startup guard: REQUIRE BACKEND_SIGNER_ENDPOINT when
    EXECUTOR_REAL_BROADCAST_ENABLED=true AND mainnet
[ ] wired into src/options/service.rs:1166 / :1213 call sites
[ ] unit + integration tests: env-keyed refused on mainnet chain id;
    mainnet path requires endpoint; mTLS handshake; from-address recovers correctly
[ ] no source side effects outside signer.rs / config.rs / env.rs / options/service.rs / new remote_signer.rs
```

### 5.4 `BACKEND-MONITORING-ALERTS-WIRING`

```text
[ ] all metrics from BACKEND_EXECUTOR_MONITORING_ALERTS_V1.md §2 exported
[ ] all alerts from §3 (PagerDuty) + §4 (Discord) wired
[ ] new Cluster 4 alerts:
    - INSURANCE_BELOW_TARGET
    - INSURANCE_NEAR_DEPLETION
    - REBATE_RESERVE_NONZERO_AT_LAUNCH
    - EFFECTIVE_NEGATIVE_PPM_AT_LAUNCH
    - PFV_FEE_BALANCE_GROWTH_STALL
[ ] new Cluster 3 BE funding ladder:
    - alert at FUND_TARGET (warning)
    - critical at FUND_FLOOR (existing BE_BAL_LOW)
    - critical + halt at emergency_floor
    - warning at FUND_CEILING (existing BE_BAL_CEILING)
    - daily drift > 2× expected anomaly
[ ] synthetic-fired alert verification documented
[ ] runbook one-pagers (RUNBOOK_BACKEND_EXECUTOR.md) published per MON-6
```

### 5.5 `OPTION-EXECUTION-TX-VISIBILITY-FIX`

```text
[ ] /executor/transactions/<intent> returns option_execution_transactions
    row for option intents (currently returns [])
[ ] route reads from option_execution_transactions DB table
    (which already has the correct row)
[ ] unit test covering option intent vs perp intent code paths
[ ] regression test against Sepolia recent intents
```

### 5.6 `MAINNET-MANIFEST-FILL-GOV-OPS-TREASURY-SLOTS`

```text
[ ] 13 Group A slots filled with OPS_SAFE_MAINNET / GOV_SAFE_MAINNET
    (per Cluster 1 redacted summary §6.1)
[ ] schema PR adds:
    - governanceRoles.treasury
    - governanceRoles.deployerRetirementStatus
    - governanceRoles.kmsKeyHandles.optionBackendExecutor / .optionBackendExecutorNext
    - feesConfiguration.rebateLaunchPolicy
    - insuranceConfiguration.operatorForm
    - custodyPolicyVersion / custodyPolicyChangeLogSha256
    - funding.backendExecutor.fundFloorWei / .fundTargetWei / .fundCeilingWei / .recomputeCadenceMonths
[ ] no actual mainnet contract addresses (those depend on DeployCore.s.sol)
[ ] no Treasury address until MAINNET-TREASURY-SAFE-CREATION-PACKET closes
[ ] no KMS handle until vendor selected + key generated
```

### 5.7 `LIQUIDATION/RISK-REHEARSAL-SEPOLIA` (if in scope)

```text
[ ] only if Cluster 4 Q-CD-12 = Option B (insurance in scope)
[ ] full liquidation + InsuranceFund coverage + bad-debt residual cycle
    executed end-to-end on Sepolia
[ ] R5 invariant preserved
[ ] auditor briefed on rehearsal evidence
```

### 5.8 `MAINNET-FORK-REHEARSAL`

```text
[ ] mainnet fork at engagement-kickoff block
[ ] DeployCore + WireCore + ConfigureCore + ConfigureMarkets + VerifyDeployment + TransferOwnerships + AcceptOwnerships
[ ] should_broadcast gate green
[ ] signer service deployed; first sign succeeds
[ ] monitoring + alerts firing in fork environment
[ ] all Cluster 4 launch invariants verified
[ ] V2G-Y phases simulated in fork
[ ] full post-deploy verification passes
```

### 5.9 `V2G-Y-MAINNET-OWNERSHIP-MIGRATION`

```text
[ ] all §3 prerequisites in MAINNET_V2G_Y_OWNERSHIP_MIGRATION_PLAN.md met
[ ] AUDIT-EXT minimum-pass condition satisfied
[ ] each per-phase packet operator-authorised separately
[ ] Y-A → Y-G phases executed; ≥ 7-day wall-clock
[ ] POST-Y-G-6 final sweep:
    - DEPLOYER fully retired (Cluster 3)
    - rebate-DEFER launch invariant (Cluster 4)
    - R5 drift = 0
    - all per-target chain reads green
[ ] mainnet first-live-smoke authorisation 4-sig attestation collected
```

---

## 6. Hard stops before any mainnet tx

Each hard stop is a chain-side or doc-side check that the operator + auditor verify before authorising any mainnet broadcast.

### 6.1 No mainnet broadcast before should_broadcast exists

```
HARD STOP: if grep -rn 'fn should_broadcast\|should_broadcast(' deopt-v2-backend/src/
           returns 0 hits, mainnet broadcast is BLOCKED.

Closure: BACKEND-SHOULD-BROADCAST-ECONOMIC-GATE merged + tests green.
```

### 6.2 No mainnet broadcast before KMS/HSM/MPC signer path exists

```
HARD STOP: if backend's effective signer path on chain_id=8453 is
           ExecutorSigner::from_private_key, mainnet broadcast is BLOCKED.

Closure: BACKEND-SIGNER-INTERFACE-KMS-HSM-ADAPTER merged + Sepolia
         integration test green AND mainnet startup guard refuses
         EXECUTOR_PRIVATE_KEY on chain_id=8453.

Waiver: explicit operator + security + audit attestation; recorded
        in offline binder; documented residual risk acceptance. Per
        custody policy P-3 / P-4, no realistic mainnet launch should
        request this waiver.
```

### 6.3 No mainnet broadcast before monitoring exists

```
HARD STOP: if `BACKEND-MONITORING-ALERTS-WIRING` is incomplete (any of
           MON-1..MON-9 from BACKEND_EXECUTOR_MONITORING_ALERTS_V1.md §9
           open), mainnet broadcast is BLOCKED.

Closure: synthetic-fired alerts confirm PagerDuty + Discord routing;
         runbook one-pagers published; on-call rotation defined.

Waiver: explicit operator + SRE + security attestation; documented
        manual-watch substitute for the unwired alerts; time-bounded
        to ≤ 14 days; reverted on alert wiring completion.
```

### 6.4 No mainnet broadcast while rebate profiles can create negative effective fees with rebateReserve=0

```
HARD STOP: if any active FeesManagerV2 profile produces effective
           negative (makerPpm × (PPM - makerDiscountPpm) / PPM) for
           any (tier, product, RFQ flow) AND PFV.rebateReserve(asset) == 0
           on the mainnet target chain, mainnet broadcast is BLOCKED.

Closure: Cluster 4 launch invariant verifier sweep (per
         BACKEND-SHOULD-BROADCAST-ECONOMIC-GATE acceptance criteria)
         returns green at the Y-G-6 chain state.

Auditor anchor: Q-34 in MAINNET_AUDIT_EXT_ENGAGEMENT_PACKAGE.md §7.8.

No waiver path. This is the central rebate-DEFER protection.
```

### 6.5 No mainnet broadcast before V2G-Y mainnet HARD STOPS verified

```
HARD STOP: POST-Y-G-6 final state audit per Cluster 3 §3.5 sweep:
  - DEPLOYER not owner of any module (×8)
  - Timelock.proposers(DEPLOYER) = false
  - Timelock.executors(DEPLOYER) = false
  - NEW_OME.isExecutor(DEPLOYER) = false
  - All 3 Safes: isOwner(DEPLOYER) = false
  - InsuranceFund: isOperator(DEPLOYER) = false
  - R5 drift = 0

If ANY check fails, mainnet first-live-smoke BLOCKED and custody-policy
§9.3 retirement HALTED until remediation.

Auditor anchor: Q-32 in MAINNET_AUDIT_EXT_ENGAGEMENT_PACKAGE.md §7.8.

No waiver path. Mandatory.
```

### 6.6 No mainnet broadcast without AUDIT-EXT minimum-pass

```
HARD STOP: 0 Critical + 0 High AUDIT-EXT findings unremediated.
           Final closure matrix delivered.
           Auditor sign-off on V2G-Y plan + manifest + 4 Cluster closures.

Waiver: only for High findings, only with operator-attested waiver +
        security-lead sign-off + documented residual-risk acceptance.
        Per custody policy P-9 + minimum-pass condition #2 in
        engagement-package §10.
```

### 6.7 No mainnet broadcast without 4-signature MAINNET_FIRST_LIVE_SMOKE_AUTHORIZATION

```
HARD STOP: per FIRST_LIVE_SMOKE_AUTHORIZATION_V2G_FX_Q1.md §13
           mainnet variant: operator + security + risk + audit
           four-signature attestation collected before first mainnet tx.

No waiver path. Mandatory.
```

---

## 7. First build prompt

The first build task is **`BACKEND-SHOULD-BROADCAST-ECONOMIC-GATE`**. The full prompt stub is at:

**`deopt-v2-backend/docs/NEXT_TASK_BACKEND_SHOULD_BROADCAST_ECONOMIC_GATE.md`**

Operator hands this prompt to the backend implementer (developer or agent) once Cluster 4 closure is signed and AUDIT-EXT engagement is kicked off (in parallel).

---

## 8. Cross-links

- `deopt-v2-backend/docs/BACKEND_MAINNET_IMPLEMENTATION_ROADMAP.md` — companion implementation roadmap
- `deopt-v2-backend/docs/NEXT_TASK_BACKEND_SHOULD_BROADCAST_ECONOMIC_GATE.md` — first build prompt stub
- `deopt-v2-sol/docs/MAINNET_AUDIT_EXT_KICKOFF_BUNDLE.md` — audit kickoff bundle
- `deopt-v2-sol/docs/MAINNET_AUDIT_HANDOFF_INDEX.md` — audit handoff index
- `deopt-v2-sol/docs/MAINNET_MANIFEST_DEPENDENCY_SNAPSHOT_AFTER_CUSTODY_CLUSTERS.md` — manifest blocker view
- All Cluster 1/2/3/4 redacted summaries
- `deopt-v2-backend/docs/MAINNET_READINESS_GAP_LIST_AFTER_SEPOLIA_ARC.md`
- `deopt-v2-backend/docs/P0_MAINNET_BLOCKER_CLOSURE_ROADMAP.md`
- `~/DEOPT/MAINNET_CUSTODY_POLICY.md`
- `~/DEOPT/RUN_STATE.md`

**End of prebuild → build handoff.**
