# P0 mainnet blocker closure roadmap

**Posture:** READ-ONLY roadmap. **No chain mutation. No `.env` edit.
No Safe-tx. No broadcast. No mainnet.** Unifies the three P0
foundation docs into an actionable ordering, identifies the
dependencies between them, and recommends the next 3 executable
milestones after this foundation package.

**Date (UTC):** 2026-06-08

**Source:** P0 blockers identified in
`deopt-v2-backend/docs/MAINNET_READINESS_GAP_LIST_AFTER_SEPOLIA_ARC.md §4`.

---

## 0. The three P0 blockers

| # | Blocker | Gap-list ref | Foundation doc |
|---|---|---|---|
| **P0-1** | `MAINNET-AUDIT-EXT-ENGAGEMENT` | B-2 / L-4 / W-14 | `deopt-v2-sol/docs/MAINNET_AUDIT_EXT_ENGAGEMENT_PACKAGE.md` |
| **P0-2** | `MAINNET-MANIFEST-FILL` | K-1 | `deopt-v2-sol/docs/MAINNET_MANIFEST_TODO_INVENTORY.md` |
| **P0-3** | `MAINNET-V2G-Y-OWNERSHIP-MIGRATION` | A-5 / W-11 | `deopt-v2-sol/docs/MAINNET_V2G_Y_OWNERSHIP_MIGRATION_PLAN.md` |

Each blocks credible mainnet planning and activation. They are **mostly parallelisable** with one critical dependency (see §3).

---

## 1. P0 documents produced by this foundation package

| Path | Size | Purpose |
|---|---|---|
| `deopt-v2-sol/docs/MAINNET_AUDIT_EXT_ENGAGEMENT_PACKAGE.md` | ~38 KB | Scope + in-scope contracts + known findings + Sepolia evidence + auditor deliverables + severity criteria + handoff file list + minimum pass condition |
| `deopt-v2-sol/docs/MAINNET_MANIFEST_TODO_INVENTORY.md` | ~28 KB | 99 TODO_REPLACE_* placeholders catalogued + classified (KNOWN / NEEDS_DECISION / NEEDS_DEPLOYMENT / NEEDS_EXTERNAL_SOURCE / BLOCKED_BY_AUDIT / BLOCKED_BY_CUSTODY) + fill workflow + forbidden fills |
| `deopt-v2-sol/docs/MAINNET_V2G_Y_OWNERSHIP_MIGRATION_PLAN.md` | ~28 KB | Phase ledger Y-A → Y-G (Model G-B) + sequencing + soak windows + rollback surfaces + verification commands + paste-back templates |
| `deopt-v2-backend/docs/P0_MAINNET_BLOCKER_CLOSURE_ROADMAP.md` | this file | Unified roadmap |

---

## 2. P0 dependency graph

```
                  ┌─────────────────────────────────────────────────────────┐
                  │ Custody policy lock-in (operator + security)            │
                  │   - mainnet OPS_MULTISIG Safe roster                    │
                  │   - mainnet GOVERNANCE_MULTISIG Safe roster (disjoint)  │
                  │   - mainnet BACKEND_EXECUTOR EOA + KMS interface        │
                  └──────────────┬──────────────────────────────────────────┘
                                 │
                                 ▼
        ┌────────────────────────────────────────────────────────┐
        │ P0-1: AUDIT-EXT ENGAGEMENT                             │
        │   - Submit handoff package to auditor                  │
        │   - Auditor reviews                                    │
        │   - Findings + closure matrix                          │
        │   - Remediation review                                 │
        │   - DURATION: ~10-14 wks total                         │
        └──────────────────────────┬─────────────────────────────┘
                                   │   ↓ unblocks BLOCKED_BY_AUDIT items
                                   ▼
        ┌────────────────────────────────────────────────────────┐
        │ P0-2: MANIFEST FILL                                    │
        │   - KNOWN values fill immediately (USDC, WETH)         │
        │   - NEEDS_DECISION values fill after operator/risk     │
        │     decisions                                          │
        │   - NEEDS_EXTERNAL_SOURCE values fill after            │
        │     oracle admin pulls feed addresses                  │
        │   - NEEDS_DEPLOYMENT values fill after deploy lands    │
        │   - BLOCKED_BY_AUDIT values fill AFTER P0-1            │
        │   - BLOCKED_BY_CUSTODY values fill AFTER custody lock  │
        │   - DURATION: ~4-8 wks, parallel to P0-1               │
        └──────────────────────────┬─────────────────────────────┘
                                   │
                                   ▼
        ┌────────────────────────────────────────────────────────┐
        │ P0-3: V2G-Y OWNERSHIP MIGRATION PLAN                   │
        │   - Plan complete (THIS DOC)                           │
        │   - Per-phase kickoff packets (Y-A through Y-G)        │
        │     produced in §6 below                               │
        │   - Sepolia drill rehearsals validate sequencing       │
        │   - Plan reviewed by AUDIT-EXT (part of P0-1)          │
        │   - EXECUTION: gated on P0-1 closure + P0-2 fill +     │
        │     prerequisites in plan §3                           │
        │   - DURATION: plan complete now; execution wall-clock  │
        │     ~7-14 days once unlocked                           │
        └────────────────────────────────────────────────────────┘
```

### 2.1 Hard dependencies between P0 items

| From | To | Type |
|---|---|---|
| Custody policy → P0-1 | partial | Auditor doesn't strictly require resolved custody, but the KMS/HSM interface design (D-1) must be in scope for review. Recommend custody locked before audit kickoff. |
| Custody policy → P0-2 | hard | Group A (governance addresses), G (BE executor) all need custody decisions to fill. |
| Custody policy → P0-3 | hard | Every phase calls a Safe-tx or BE-signed tx; mainnet Safe + KMS must exist. |
| P0-1 → P0-2 (audit-blocked) | partial | 17 placeholders are `BLOCKED_BY_AUDIT` (Group F risk params + caps). Fill on AUDIT-EXT sign-off. |
| P0-1 → P0-3 (execution) | hard | Mainnet plan §3 prerequisites mandate "0 Critical / 0 High unremediated". |
| P0-2 → P0-3 (execution) | hard | Plan references TIMELOCK_MAINNET, OPS_MULTISIG_MAINNET, GOVERNANCE_MULTISIG_MAINNET, BE_MAINNET — all resolved via P0-2. |

### 2.2 Soft dependencies (recommended but not strict)

- AUDIT-EXT review of `MAINNET_V2G_Y_OWNERSHIP_MIGRATION_PLAN.md` and `MAINNET_MANIFEST_TODO_INVENTORY.md` — auditor sign-off is a minimum pass condition (§10.5 / §10.6 of audit package).
- Sepolia drill rehearsals (guardian pause, compromise rotation, forward-recovery) can run in parallel with P0-1 — they validate the Sepolia evidence relied on by AUDIT-EXT.

---

## 3. Suggested ordering — wall-clock view

Week-1 to week-2 (operator + security + custody):
- Lock custody policy: OPS_MULTISIG mainnet roster; GOVERNANCE_MULTISIG mainnet roster; mainnet BE EOA address; KMS handle / region.
- Deploy mainnet Safes (OPS_MULTISIG, GOVERNANCE_MULTISIG); verify offchain + onchain. (May take 1-2 weeks to coordinate signer ceremony.)
- Provision mainnet BACKEND_EXECUTOR EOA + KMS-backed signer interface (gap-list D-1, D-3).

Week-2 to week-4 (parallel):
- **P0-1 KICKOFF:** ship `MAINNET_AUDIT_EXT_ENGAGEMENT_PACKAGE.md` + handoff bundle to auditor; freeze engagement commit; sign NDA + SOW.
- **P0-2 START:** fill KNOWN values (USDC, WETH); start oracle admin work on feed addresses (D-1..D-5 from manifest inventory).

Week-4 to week-10:
- AUDIT-EXT actively reviewing; bi-weekly check-ins.
- **P0-2 CONTINUES:** fill NEEDS_DECISION values (risk decisions, MM roster, settlement operator, etc.); NEEDS_DEPLOYMENT slots resolve as mainnet contracts deploy.
- Sepolia drill rehearsals (guardian pause, compromise rotation, forward-recovery, Timelock queue/cancel/execute on the existing Sepolia stack).

Week-10 to week-12:
- AUDIT-EXT findings reported; remediation phase.
- **P0-2 BLOCKED_BY_AUDIT slots** fill as auditor signs off on risk params + caps.

Week-12 to week-14:
- AUDIT-EXT closure matrix delivered; minimum pass condition verified.
- Final mainnet manifest frozen.
- Per-phase kickoff packets for Y-A through Y-G produced (see §6 below).
- 4-signature mainnet activation attestation collected.

Week-14 to week-16:
- **P0-3 EXECUTION:** Y-A → Y-G executed in operator-authorised sessions; ~7-14 day wall-clock total including soak windows.
- Backend FX-Q1 mainnet cutover in parallel after Y-F-B-X-add lands.
- Mainnet first-live-smoke (separately authorised after Y-G closes).

**Total wall-clock estimate from custody lock-in to mainnet activation: ~14-16 weeks.** Dominated by AUDIT-EXT timeline.

---

## 4. Parallelisable workstreams

These can run in parallel WITHOUT blocking each other:

### 4.1 Workstream A — Custody + Safe deployment

Owner: Operator + Security + Custody team.
Tasks:
1. Lock OPS_MULTISIG mainnet roster (≥ 3-of-5, hardware MFA).
2. Lock GOVERNANCE_MULTISIG mainnet roster (≥ 3-of-5, hardware MFA, disjoint from OPS).
3. Deploy both Safes via `safe-cli` or Safe Web UI per `GOVERNANCE_OPS_MULTISIG_DEPLOY_PLAN_V2G_GOV_D2.md` mainnet variant.
4. Provision mainnet BACKEND_EXECUTOR EOA.
5. Implement KMS-backed signer interface in `deopt-v2-backend/src/execution/signer.rs` (closes gap-list D-1).
6. Drill Sepolia compromise → freeze → rotate → unpause (validates D-6).

Outputs: Safe addresses (OPS_MULTISIG_MAINNET, GOVERNANCE_MULTISIG_MAINNET, BE_MAINNET) → feed into P0-2.

### 4.2 Workstream B — AUDIT-EXT engagement

Owner: Security lead.
Tasks per audit package §11 (Engagement-kickoff checklist).
Outputs: findings report + closure matrix + sign-off on mainnet plans → unblocks P0-2 BLOCKED_BY_AUDIT slots + P0-3 prereq §3.

### 4.3 Workstream C — Manifest fill (NEEDS_DECISION + NEEDS_EXTERNAL_SOURCE)

Owner: per `MAINNET_MANIFEST_TODO_INVENTORY.md` row owners.
Tasks:
1. Risk + AUDIT-EXT: finalise risk parameters (Group F) — gated on AUDIT-EXT.
2. Oracle admin + Security: source Chainlink + Pyth feed addresses from official registries (Group D).
3. Operator + Insurance: settle insurance funding amount, operator Safe (Group K).
4. Backend + Finance: merkle root for tier-fee program (Group E).
5. Ops + SRE: incident runbook URL (Group I).

Outputs: filled mainnet manifest PR → unblocks P0-3.

### 4.4 Workstream D — Backend + Frontend P1 closures

Owner: Backend + Frontend + SRE.
These are P1 items (REQUIRED-BEFORE-MAINNET, not BLOCKING), but they're long-lead. Can run in parallel:

- C-4: `should_broadcast` implementation per gas/fees/rebates policy §8.
- C-5: rebate-solvency hard gate (lifts W-3).
- C-6: wash-trade detection.
- C-7: persistent dedupe cache + nonce-window store.
- C-8: subsidy budget registry + per-reason cap + 1h burn alert.
- C-15: unit tests for every `should_broadcast` branch.
- D-1: KMS/HSM signer interface (also part of Workstream A).
- E-1..E-10: monitoring + alerting wiring (Prometheus + Grafana + PagerDuty + Discord + per-alert runbooks).
- J-1: V2G-W3 SSR proxy + OIDC/MFA + Strict CSP.

Outputs: backend + frontend mainnet-ready stack → satisfies P0-3 prereq.

### 4.5 Workstream E — Sepolia drill rehearsals

Owner: Ops + Security + Risk.
Tasks:
- M-1: guardian (OPS_MULTISIG) pause-path drill on Sepolia.
- M-3 / D-6: BE compromise → freeze → rotate → unpause drill.
- M-4: forward-recovery (OPS_MULTISIG → DEPLOYER) drill.
- A-7: cancel/queue/execute rehearsal on Sepolia (validates the mainnet flow).
- I-7: oracle failure-mode drill (stale / zero / unavailable / future-timestamp).

Outputs: drill logs satisfying P0-3 §3 prerequisites + `FINAL_LAUNCH_CHECKLIST.md` Staging Rehearsal row.

---

## 5. Owners (rolled-up)

| Workstream | Primary owners |
|---|---|
| A. Custody + Safe deploy | Operator + Security + Custody team |
| B. AUDIT-EXT | Security lead + external auditor |
| C. Manifest fill | Risk + Oracle admin + Insurance + Finance + Backend + Ops |
| D. Backend / Frontend P1 closures | Backend + Frontend + SRE + Security |
| E. Sepolia drill rehearsals | Ops + Security + Risk |
| P0-1 doc maintenance | Security lead |
| P0-2 doc maintenance | Deployment Owner |
| P0-3 doc maintenance | Governance + Protocol lead |
| P0-3 execution (when unlocked) | Operator + Governance + Security + Risk + Audit (4-signature gate) |

---

## 6. Acceptance criteria per P0 blocker

### P0-1: MAINNET-AUDIT-EXT-ENGAGEMENT

**Acceptance:**
- AUDIT-EXT engagement scoped + auditor engaged + SOW + NDA signed.
- Engagement-kickoff commit frozen + handoff package shipped.
- Auditor delivers findings report.
- Each finding tracked in closure matrix.
- Each Critical / High remediated OR explicitly waived with security-lead sign-off.
- All Medium findings remediated or accepted.
- Final closure matrix shipped.
- Auditor signs off on `MAINNET_V2G_Y_OWNERSHIP_MIGRATION_PLAN.md` and `MAINNET_MANIFEST_TODO_INVENTORY.md`.
- `forge test --no-match-path 'test/fork/*'` and `cargo test --all-targets --all-features` pass on the final remediation commit.

**Documents produced by AUDIT-EXT:**
- Final findings PDF report.
- Closure matrix CSV / table.
- Remediation review for each accepted fix.
- Sign-off on mainnet plans.
- (Optional) Slither + Mythril output reviewed.

### P0-2: MAINNET-MANIFEST-FILL

**Acceptance:**
- All 76 distinct placeholder slots in `mainnet.template.json` have either a resolved value OR an explicit "deferred from launch scope" annotation signed by the operator.
- 0 occurrences of `TODO_REPLACE_*` remain in the working copy of `mainnet.template.json`.
- 0 occurrences of `MockPriceSource` in the mainnet manifest.
- 0 guardian slots set to DEPLOYER on mainnet.
- `Timelock.minDelay = 259 200` (72h) committed.
- Oracle feed addresses verified against Chainlink + Pyth canonical registries (cross-checked offline + onchain bytecode).
- Token decimals match manifest declarations for USDC (6), WETH (18), BTC (8).
- Deposit caps within published launch-cap policy.
- All 5 schema gaps (PFV, rebateFundingAccount, FM-V2 rebateBudget, PFV guardian, optionMatchingEngine vs matchingEngine) addressed via manifest schema extension PR.
- AUDIT-EXT signs off on filled manifest (P0-1 minimum pass condition §10.6).

**Documents produced:**
- Filled `deployments/base-mainnet.manifest.json`.
- Manifest schema extension PR.
- Per-row source-of-truth attestation log.
- Read-only validation report (run all the §3 validation commands; expected vs actual).

### P0-3: MAINNET-V2G-Y-OWNERSHIP-MIGRATION-PLAN

**Acceptance:**
- Plan complete (THIS REPO — `MAINNET_V2G_Y_OWNERSHIP_MIGRATION_PLAN.md`).
- Per-phase kickoff packets produced for Y-A, Y-B, Y-C, Y-D, Y-E, Y-F, Y-G (each analogous to V2G-GOV-G-PREP in Sepolia).
- Plan reviewed by AUDIT-EXT (P0-1 minimum pass condition §10.5).
- Sepolia drill rehearsals complete (Workstream E).
- All §3 prerequisites of the plan satisfied (custody locked, manifest filled, monitoring wired, audit closed, prerequisites checked off).
- 4-signature mainnet activation attestation collected (operator + security + risk + audit).
- Per-phase kickoff packets each independently authorised at broadcast time.

**Documents produced (per-phase kickoff packets, to be authored as separate milestones):**
1. `MAINNET_V2G_Y_PHASE_A_GUARDIAN_WIRING_PREP.md`
2. `MAINNET_V2G_Y_PHASE_B_PFV_OWNER_PREP.md`
3. `MAINNET_V2G_Y_PHASE_C_FM_V2_OWNER_PREP.md`
4. `MAINNET_V2G_Y_PHASE_D_8_TARGET_TRANSFER_PREP.md`
5. `MAINNET_V2G_Y_PHASE_E_TIMELOCK_ACCEPT_PREP.md`
6. `MAINNET_V2G_Y_PHASE_F_NEW_OME_EXECUTOR_PREP.md`
7. `MAINNET_V2G_Y_PHASE_G_TIMELOCK_CLEANUP_PREP.md`

Each kickoff packet bundles: cast / safe-cli templates, hard stops, preconditions, postconditions, R5 invariant checks, event-topic / selector cross-checks, failure-mode classification, paste-back template, verification commands, gas/ETA estimates.

---

## 7. Remaining questions

These need explicit operator answers before P0 can fully close:

| # | Question | Owner | Default if unspecified |
|---|---|---|---|
| Q-1 | OPS_MULTISIG mainnet roster: who are the 3-5 signers? | Operator + Security | NO DEFAULT — blocks Workstream A |
| Q-2 | GOVERNANCE_MULTISIG mainnet roster: who are the 3-5 signers (disjoint from OPS)? | Operator + Governance | NO DEFAULT — blocks Workstream A |
| Q-3 | Mainnet `Timelock.minDelay` = 72h (recommended) or other? | Governance + Security | 72h (Model G-B) |
| Q-4 | Mainnet `executors(...)` = `{OPS_MULTISIG}` (recommended) or open-executor? | Governance | OPS_MULTISIG only |
| Q-5 | KMS / HSM provider for BE signer? AWS KMS, GCP KMS, Azure Key Vault, on-prem HSM? | Backend + Security | NO DEFAULT — blocks D-1 |
| Q-6 | Mainnet BACKEND_EXECUTOR EOA derivation path (BIP-32 / KMS key handle)? | Backend + Custody | NO DEFAULT — blocks D-3 |
| Q-7 | Same BE for OPTION + PERP, or distinct EOAs? | Operator + Backend | Distinct (recommended for blast-radius bounding) |
| Q-8 | wBTC variant on Base mainnet: `cbBTC` or bridged variant? | Risk + Protocol | NO DEFAULT — blocks N-3 (manifest) |
| Q-9 | Rebate program enabled at launch, or deferred? | Finance + Risk | DEFERRED (rebateReserve = 0 launch state) |
| Q-10 | PNL_FLOOR mainnet value (must be > 0)? | Finance + Backend | NO DEFAULT — blocks F-2 |
| Q-11 | MAX_MAX_FEE_PER_GAS mainnet ceiling? | SRE + Backend | NO DEFAULT — blocks F-3 |
| Q-12 | Subsidy budget caps per reason (mm-bootstrap, liquidation, rfq-recovery)? | Finance | NO DEFAULT — blocks C-8 |
| Q-13 | Initial mainnet insurance funding amount (native USDC units)? | Insurance + Finance | NO DEFAULT — blocks K-3 |
| Q-14 | Launch caps per option series + per perp market? | Risk + AUDIT-EXT | NO DEFAULT — blocks Group F |
| Q-15 | Settlement operator on mainnet OPR: address(0) (off), OPS_MULTISIG, or dedicated Safe? | Risk + Settlement | NO DEFAULT — blocks A-9 |
| Q-16 | First mainnet option series: which underlying, strike, expiry, type? | Risk + Markets | NO DEFAULT — blocks L (series config) |
| Q-17 | First mainnet perp market: which underlying, IM/MM, OI cap? | Risk + Markets | NO DEFAULT — blocks L |
| Q-18 | External auditor identity + budget + SOW? | Operator + Security | NO DEFAULT — blocks P0-1 kickoff |

Recommend the operator answer Q-1, Q-2, Q-5, Q-6, Q-7, Q-9, Q-18
first — these unblock the most parallel workstreams.

---

## 8. Recommended next 3 executable milestones (after this foundation package)

### Milestone N+1 — `MAINNET-CUSTODY-POLICY-COMMIT`

**Type:** Read-only / decision document.
**Owner:** Operator + Security + Custody.
**Goal:** Produce `~/DEOPT/MAINNET_CUSTODY_POLICY.md` answering Q-1 / Q-2 / Q-5 / Q-6 / Q-7. Locks the mainnet Safe rosters (OPS_MULTISIG + GOVERNANCE_MULTISIG), the KMS/HSM provider, the mainnet BE address scheme, and the rotation policy.

**Acceptance:**
- All 5 questions answered with attested signers / providers.
- Document signed by operator + security lead.
- No mainnet broadcast.

**Why this first:** unblocks both P0-1 (auditor reviews KMS interface) and P0-2 (manifest fills Group A + Group G) and Workstream A in parallel.

### Milestone N+2 — `MAINNET-AUDIT-EXT-KICKOFF`

**Type:** Engagement bootstrap.
**Owner:** Security lead + external auditor.
**Goal:** Freeze engagement commit, ship handoff bundle, sign NDA + SOW, schedule bi-weekly check-ins.

**Acceptance:**
- Engagement commit tagged.
- Handoff bundle (per `MAINNET_AUDIT_EXT_ENGAGEMENT_PACKAGE.md §9`) shipped.
- SOW + NDA signed.
- First bi-weekly check-in scheduled.
- No mainnet broadcast.

**Why this second:** longest external timeline; every subsequent milestone runs in parallel.

### Milestone N+3 — `MAINNET-V2G-Y-PHASE-A-GUARDIAN-WIRING-PREP`

**Type:** Read-only per-phase kickoff packet.
**Owner:** Governance + Protocol.
**Goal:** Produce the first per-phase kickoff packet for the V2G-Y migration (Y-A guardian wiring). Defines exact `cast send` templates (with TODO addresses for mainnet contracts), pre-state matrix, post-state matrix, R5 invariant checks, paste-back template, hard stops, rollback surface.

**Acceptance:**
- Document produced at `deopt-v2-sol/docs/MAINNET_V2G_Y_PHASE_A_GUARDIAN_WIRING_PREP.md`.
- Reviewed by AUDIT-EXT (part of P0-1).
- No mainnet broadcast.
- Per-phase packet pattern established (replicated for B/C/D/E/F/G in subsequent milestones).

**Why this third:** demonstrates the per-phase packet pattern that the operator can replicate for the remaining 6 phases. Establishes a template-quality bar so each subsequent phase is "write the packet, not re-design".

---

## 9. P0 closure timeline (visual)

```
Week:    00      02      04      06      08      10      12      14      16
         │       │       │       │       │       │       │       │       │
Custody  ████████│       │       │       │       │       │       │       │     ← lock
A. Safes ░░██████│       │       │       │       │       │       │       │     ← deploy + verify
                 │       │       │       │       │       │       │       │
P0-1            ████████████████████████████████████████│       │       │     ← AUDIT-EXT
                 │   review               findings  remediation │       │
                 │       │       │       │       │       │       │       │
P0-2            ████████████████████████████│      ████████│       │       │     ← manifest fill
                 │ KNOWN  DECISIONS DEPLOYED        AUDITED       │       │
                 │       │       │       │       │       │       │       │
P0-3 plan       ✓ done                             │       │       │       │     ← THIS DOC
P0-3 packets        ████████████████│       │       │       │       │       │     ← per-phase prep
P0-3 exec                                    │       │       █████████│       │     ← Y-A → Y-G
                 │       │       │       │       │       │       │       │
BE/MON P1            ████████████████████████████████████████│       │       │     ← parallel impl
Drills              ░░░░░░░░░░░░░░░░░│       │       │       │       │       │     ← Sepolia
                 │       │       │       │       │       │       │       │
4-sig                                                    │       ███│       │     ← attestation
Mainnet                                                          │       ██│       ← go live
1st smoke                                                                │   ██    ← separately
```

Critical path: **P0-1 AUDIT-EXT** (12 wks) dominates everything else.

---

## 10. Closure summary (when all P0 items are done)

When all three P0 items + their per-phase / per-fill prerequisites
close, mainnet readiness becomes:

```text
[ DONE ]    Sepolia rehearsal arc structurally complete
[ DONE ]    AUDIT-EXT engagement closed; closure matrix shipped
[ DONE ]    Mainnet manifest filled; 0 TODO_REPLACE_* remaining
[ DONE ]    Mainnet OPS_MULTISIG + GOVERNANCE_MULTISIG Safes deployed + verified
[ DONE ]    Mainnet protocol contracts deployed; VerifyDeployment.s.sol green
[ DONE ]    Mainnet BE + KMS interface live
[ DONE ]    Monitoring + alerting wired + synthetic-fired
[ DONE ]    Backend should_broadcast + dedupe + subsidy + wash-detection live + tested
[ DONE ]    V2G-W3 SSR proxy + OIDC/MFA + CSP live
[ DONE ]    V2G-Y ownership migration Y-A → Y-G executed; Timelock owned by GOVERNANCE_MULTISIG
[ DONE ]    Mainnet FX-Q1 backend signer cutover complete
[ DONE ]    4-signature MAINNET-FIRST-LIVE-SMOKE-AUTHORIZATION attested
[ ↓ ]      MAINNET-FIRST-LIVE-SMOKE (separately authorised)
[ ↓ ]      Activation flags flipped (per launchSafetyControls)
[ ↓ ]      Mainnet live
```

P1 items not directly on this path remain open (P2/P3 hardening), but
none block mainnet activation per the gap-list classification.

---

## 11. What this roadmap does NOT do

```text
- Does NOT engage an auditor
- Does NOT lock mainnet addresses
- Does NOT broadcast any chain tx
- Does NOT authorise any phase of V2G-Y
- Does NOT commit a budget or timeline beyond rough estimates
- Does NOT bind the operator to a specific sequencing — sequencing is a recommendation
- Does NOT touch mainnet
```

---

## 12. Cross-links

- `~/DEOPT/deopt-v2-backend/docs/MAINNET_READINESS_GAP_LIST_AFTER_SEPOLIA_ARC.md` — full gap list (P0/P1/P2/P3)
- `~/DEOPT/deopt-v2-sol/docs/MAINNET_AUDIT_EXT_ENGAGEMENT_PACKAGE.md` — P0-1 doc
- `~/DEOPT/deopt-v2-sol/docs/MAINNET_MANIFEST_TODO_INVENTORY.md` — P0-2 doc
- `~/DEOPT/deopt-v2-sol/docs/MAINNET_V2G_Y_OWNERSHIP_MIGRATION_PLAN.md` — P0-3 doc
- `~/DEOPT/deopt-v2-sol/docs/V2G_GOV_G_RESULT.md` — Sepolia V2G-GOV-G closure
- `~/DEOPT/deopt-v2-sol/docs/GOVERNANCE_TIMELOCK_CLEANUP_PREP_V2G_GOV_G_PREP.md` — Model G-A vs G-B
- `~/DEOPT/deopt-v2-sol/docs/GOVERNANCE_ROLE_HARDENING_PLAN_V2G_GOV_P0.md` — target role map
- `~/DEOPT/deopt-v2-sol/docs/GOVERNANCE_OPS_MULTISIG_DEPLOY_PLAN_V2G_GOV_D2.md` — Safe deploy reference
- `~/DEOPT/AUDIT_GATE_DECISION_V2G_AUDIT0.md` — internal V2G-AUDIT0 closure
- `~/DEOPT/RUN_STATE.md`

**End of P0 closure roadmap.**
