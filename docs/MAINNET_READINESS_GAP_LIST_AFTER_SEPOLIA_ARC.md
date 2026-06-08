# Mainnet readiness gap list — after Sepolia rehearsal arc

**Posture:** READ-ONLY audit. **No chain mutation. No `.env` edit. No
Safe-tx. No broadcast. No mainnet.** Enumerates every remaining gap
between the structurally-complete Sepolia rehearsal arc
(governance + executor cutover + first live orderbook smoke + first
live RFQ smoke) and a credible mainnet activation.

**Date (UTC):** 2026-06-08

**Scope:** survey of all three repos (`deopt-v2-sol`,
`deopt-v2-backend`, `deopt-v2-frontend`) and the governance /
operational doc corpus. Produces a domain-classified gap list plus
a P0/P1/P2/P3 prioritised roadmap.

---

## 0. Sepolia evidence baseline (DONE — recap)

| Milestone | Evidence | Locked at |
|---|---|---|
| **V2G-GOV-G** Timelock owner → OPS_MULTISIG; DEPLOYER stripped | `deopt-v2-sol/docs/V2G_GOV_G_RESULT.md` | 7 chain tx, Safe.nonce 0→4, R5 drift 0 |
| **FX-Q1 backend signer cutover** DEPLOYER PK → BACKEND_EXECUTOR PK | `BACKEND_SIGNER_CUTOVER_RUNBOOK_V2G_FX_Q1.md` + `FX_Q1_C_OPERATOR_DECISION_PRE_FLIP.md` | `EXECUTOR_FROM_ADDRESS=BE`; broadcast flags up |
| **FIRST_LIVE_SMOKE-EXEC orderbook fee-only** | `FIRST_LIVE_OPTION_EXECUTION_SMOKE_RESULT_V2_SEPOLIA.md` | tx `0xb2379a46…e800` block 42 571 249; BE.nonce 0→1; +3000 mUSDC PFV fee |
| **FIRST_LIVE_SMOKE-RFQ fee-only** | `FIRST_LIVE_RFQ_OPTION_EXECUTION_SMOKE_RESULT_SEPOLIA.md` | tx `0x8538066c…5326` block 42 581 402; BE.nonce 1→2; +3000 mUSDC PFV fee |
| **OPTION_CONFIRMATION_WORKER_ENABLED** + DB cleanup of orphan perp rows | `POST_GOV_G_OPS_CLEANUP_BEFORE_RFQ_SMOKE.md` | confirmation worker live; 0 noise lines |
| **OPTION_NONCE_SYNC_ENABLED** (remediated mid-arc after BadNonce halts) | `RFQ_SMOKE_NONCE_SYNC_REMEDIATION.md` | end-to-end nonce sync proven |
| **R5 drift = 0** preserved across 7 governance tx + 2 live trades | every closure doc | invariant intact across full arc |
| **V2G-AUDIT0** internal audit closure (sol + backend + frontend) | `~/DEOPT/AUDIT_GATE_DECISION_V2G_AUDIT0.md` | 0 Critical; 4 High (all mainnet-blocking) |

Sepolia rehearsal arc is structurally complete on both
option-execution surfaces (orderbook + RFQ). Cumulative PFV fee
balance = 6 028 mUSDC; conservation 0; no rebate path exercised.

---

## 1. Hard stops compliance for this audit

```text
no chain tx                          ✅ — read-only inspection only
no config mutation                   ✅
no Safe tx                           ✅
no backend broadcast                 ✅
no new RFQ / order                   ✅
no G-6 (setMinDelay)                 ✅
no reserve allocation                ✅
no mainnet                           ✅
no private key / DATABASE_URL / RPC printed ✅
```

---

## 2. Sepolia waivers — explicit enumeration

Every waiver-for-Sepolia that exists in the rehearsal corpus, and
the mainnet task that lifts it.

| Waiver ID | Where | What was deferred | Mainnet task name |
|---|---|---|---|
| **W-1 BE-FUND** | `FX_Q1_C_OPERATOR_DECISION_PRE_FLIP.md §5.1` | BACKEND_EXECUTOR balance below FUND_TARGET (`3.8e15 / 1e16`); DEPLOYER itself too small to top to TARGET. | `MAINNET-BE-FUND-POLICY-COMMIT` |
| **W-2 MON** | id. §5.2 | No PagerDuty / Discord wiring; "active manual watch" substitute. | `MAINNET-MONITORING-ALERTS-WIRING` |
| **W-3 REBATE-GATE** | id. §5.3 + `BACKEND_GAS_FEES_REBATES_POLICY_V1.md §4.2` | `should_broadcast §4.2` rebate-solvency hard gate spec-only; chain-side `InsufficientRebateReserve` revert is the only backstop today. | `MAINNET-BACKEND-SHOULD-BROADCAST-IMPL` |
| **W-4 SRE-ONCALL** | id. §5.4 | No SRE / Risk on-call; single-actor operator. | `MAINNET-THREE-SIGNATURE-ATTESTATION-GATE` |
| **W-5 ENV-KEY** | `BACKEND_SIGNER_CUTOVER_RUNBOOK_V2G_FX_Q1.md §13` | `EXECUTOR_PRIVATE_KEY` env-var path; no KMS/HSM interface in code. | `MAINNET-KMS-HSM-SIGNER-INTERFACE` |
| **W-6 REBATE-RESERVE** | `BACKEND_GAS_FEES_REBATES_POLICY_V1.md §3,§12` | `PFV.rebateReserve(mUSDC) = 0`; rebate-bearing trades disabled. | `MAINNET-REBATE-RESERVE-ALLOCATION-DESIGN` |
| **W-7 TEST-WALLETS** | `FX_Q1_C_OPERATOR_DECISION_PRE_FLIP.md §3` | BUYER/SELLER fixtures with locally-held PKs in `wallet.txt`. | `MAINNET-MM-COUNTERPARTY-ONBOARDING` |
| **W-8 TEST-TOKEN** | id. (`mUSDC = 0x6eAe…412E`) | mUSDC = mintable testnet token. | `MAINNET-ASSET-MIGRATION-USDC-CANONICAL` |
| **W-9 MIN-DELAY** | `V2G_GOV_G_RESULT.md §9` | Timelock `minDelay = 86 400` (24 h); mainnet candidate is 72 h. | `MAINNET-G6-MIN-DELAY-DECISION` |
| **W-10 ALARM-WAIVE-LOG** | `FX_Q1_C_OPERATOR_DECISION_PRE_FLIP.md §5.2` | Halt thresholds were operator-only manual watch (no synthetic alert firing tested). | folded into W-2 |
| **W-11 OWNER=EOA on mainnet** | `INTERNAL_AUDIT_FINDINGS_V2G_AUDIT0.md S-H1` | every owner-controlled mainnet contract `owner() == deployer EOA`; ownership migration not yet performed on mainnet. | `MAINNET-V2G-Y-OWNERSHIP-MIGRATION` |
| **W-12 ADMIN TOKEN in browser** | id. F-H1 / B-H1 | `sessionStorage["deopt.adminToken"]` + plaintext `X-Admin-Token` header. | `MAINNET-V2G-W3-SSR-PROXY` |
| **W-13 ADMIN AUDIT-LOG SINK** | id. B-H2 | `tracing::info!(target: "deopt.admin.audit", ...)` mixes with general log; no retention sink. | `MAINNET-V2G-W2-1-AUDIT-RETENTION` |
| **W-14 SLITHER/MYTHRIL** | `AUDIT_GATE_DECISION_V2G_AUDIT0.md §4` | Static analysis deferred to AUDIT-EXT environment. | `MAINNET-AUDIT-EXT-ENGAGEMENT` |

---

## 3. Domain-classified gap list

For each domain: Sepolia evidence already proven; mainnet
blocker status; the exact gap; risk if ignored; required owner;
proposed next task name.

Blocker tiers (per task §6 brief):
`BLOCKING` — must close to even begin mainnet planning safely.
`REQUIRED-BEFORE-MAINNET` — must close before any mainnet broadcast.
`RECOMMENDED-BEFORE-MAINNET` — strongly advised; explicit waiver
required to skip.
`OPTIONAL` — useful hardening; defer if scope-bound.
`DONE` — already closed on Sepolia (or, where relevant, on mainnet).

---

### A. Governance

| Item | Status | Gap | Risk if ignored | Owner | Next task |
|---|---|---|---|---|---|
| A-1 Sepolia Timelock → OPS_MULTISIG (2-step) | **DONE** | — (verified `V2G_GOV_G_RESULT.md`) | — | — | — |
| A-2 Sepolia DEPLOYER stripped (proposer + executor + guardian + owner) | **DONE** | — | — | — | — |
| A-3 Mainnet `GOVERNANCE_MULTISIG` Safe deployment + roster sign-off | **REQUIRED-BEFORE-MAINNET** | OPS_MULTISIG (2-of-3) is the Sepolia rehearsal Safe; mainnet needs Model G-B target = Safe ≥ 3-of-5 + MFA per `GOVERNANCE_TIMELOCK_CLEANUP_PREP_V2G_GOV_G_PREP.md` §Models. No mainnet Safe deployed. | Single-shard custody on mainnet — same blast radius as deployer EOA. | Protocol + Security + Operator | `MAINNET-GOVERNANCE-SAFE-DEPLOY` |
| A-4 Mainnet Timelock deploy + `minDelay = 72h` policy | **REQUIRED-BEFORE-MAINNET** | Mainnet manifest `protocolTimelock = TODO_REPLACE`; G-6 minDelay bump deferred even on Sepolia. | Without 72h delay every owner-only change is single-cycle; rollback window too short for incident response. | Protocol + Security | `MAINNET-TIMELOCK-DEPLOY-MINDELAY-72H` |
| A-5 Mainnet ownership handover on all contracts (FM-V2, PFV, NEW_OME, NEW_ME, RG, etc.) | **BLOCKING** for mainnet activation | All audited-protocol owner slots on mainnet still read deployer EOA per `S-H1`; single-key compromise → full takeover. | Mainnet's worst-case is the audit's High finding made permanent. | Protocol + Security + Governance | `MAINNET-V2G-Y-OWNERSHIP-MIGRATION` |
| A-6 G-6 Sepolia minDelay 24h → 72h decision | **OPTIONAL** (Sepolia); **REQUIRED-BEFORE-MAINNET** target shape (apply on mainnet directly) | Deferred per `V2G_GOV_G_RESULT.md §9`. | Sepolia rehearsal of the 72h cadence not yet performed. | Operator | `V2G-GOV-G-G6-MIN-DELAY-DECISION-PACKET` |
| A-7 Cancel/queue/execute rehearsal on mainnet Safe before activation | **RECOMMENDED-BEFORE-MAINNET** | `FINAL_LAUNCH_CHECKLIST.md` Governance row requires "harmless governance queue/cancel/execute rehearsal in staging". | First production Timelock op is the live one; operator UX risk. | Governance + Ops | folded into `MAINNET-STAGING-REHEARSAL-FULL` |
| A-8 Forward-recovery (OPS_MULTISIG → DEPLOYER) 4-step Safe sequence documented | **DONE** | — (in `V2G_GOV_G_RESULT.md §8`). Mainnet variant should mirror with the GOVERNANCE_MULTISIG signer set. | — | — | mainnet copy in `MAINNET-INCIDENT-RUNBOOKS` |

---

### B. Smart contracts

| Item | Status | Gap | Risk if ignored | Owner | Next task |
|---|---|---|---|---|---|
| B-1 Internal audit V2G-AUDIT0 — 0 Critical, 1 High (S-H1 = W-11 above), 3 Medium | **DONE** for Sepolia rehearsal | S-H1 mainnet-blocking; S-M1/S-M2 accepted; S-M3 already closed by V2G-RX.1. | — | — | mainnet-blocked by A-5 above |
| B-2 External audit engagement (AUDIT-EXT) | **BLOCKING** for mainnet | Not engaged. Handoff package documented in `INVARIANT_FUZZ_COVERAGE_MATRIX_V2G_AUDIT.md §6`. | Any unaudited finding (oracle deviation under adversarial single-feed, yield-adapter rotation invariant, etc.) becomes mainnet incident. | Security | `MAINNET-AUDIT-EXT-ENGAGEMENT` |
| B-3 Invariant fuzz gaps closed | **RECOMMENDED-BEFORE-MAINNET** | Open per `INVARIANT_FUZZ_COVERAGE_MATRIX_V2G_AUDIT.md §1.3 / 1.4 / 1.5 / 3 / 5`: FM-V2 state-machine, OME nonce-monotonic, CV yield-adapter rotation, perp/option fuzz suites. | Subtle accounting bugs surface only at full-traffic mainnet load. | Protocol + Security | `MAINNET-INVARIANT-FUZZ-SUITE-EXPANSION` |
| B-4 `S-L4` mainnet guard on `WireProtocolFeeVaultFeesManager.s.sol` | **REQUIRED-BEFORE-MAINNET** | Wire script lacks `MAINNET_OK=true` gate parity with deploy script. | Operator could mis-broadcast wiring without dual confirm. | Protocol | `S-L4-MAINNET-GUARD-PATCH` |
| B-5 Verify all `script/*V2*.s.sol` broadcast paths re-checked under mainnet env | **REQUIRED-BEFORE-MAINNET** | All paths gate on `*_CONFIRM=true` per `S-I1`; need explicit "no mainnet placeholder remains" audit per `FINAL_LAUNCH_CHECKLIST.md` Configuration row. | Mis-configured deploy → broken wiring → forced re-deploy of partial stack. | Protocol + Ops | `MAINNET-DEPLOY-SCRIPT-AUDIT` |
| B-6 `OptionMatchingEngine` nonce-monotonic invariant suite | **RECOMMENDED-BEFORE-MAINNET** | Per `INVARIANT_FUZZ_COVERAGE_MATRIX_V2G_AUDIT.md §1.4`. | Same as B-3. | Protocol | folded into B-3 |
| B-7 Storage-layout audit on every deployed-storage contract | **REQUIRED-BEFORE-MAINNET** | `INVARIANTS.md §1.8` mandates; `FINAL_LAUNCH_CHECKLIST.md` Code Readiness row pending. | Future upgrade collides; data corruption. | Protocol + Security | folded into B-2 |

---

### C. Backend execution path

| Item | Status | Gap | Risk if ignored | Owner | Next task |
|---|---|---|---|---|---|
| C-1 `OPTION_CONFIRMATION_WORKER_ENABLED=true` (broadcast_submitted → broadcast_confirmed) | **DONE** | — | — | — | — |
| C-2 `OPTION_NONCE_SYNC_ENABLED=true` (pull `NEW_OME.nonces(addr)` at intent creation) | **DONE** | — | — | — | — |
| C-3 `OPTION_EXECUTION_BROADCAST_GAS_LIMIT=1500000` (raises BroadcastGate above 1.25× sim estimate) | **DONE** | — | per-product gas-profile table still recommended per `BACKEND_GAS_FEES_REBATES_POLICY_V1.md §10 T-9` | — | folded into F-3 |
| C-4 `should_broadcast` decision function (§8 of gas/fees/rebates policy) | **REQUIRED-BEFORE-MAINNET** | Spec-only in `BACKEND_GAS_FEES_REBATES_POLICY_V1.md §8`. Search `grep -rn 'fn should_broadcast\|should_broadcast(' src/` returns **0 hits** in backend source — function is unimplemented. | All §6 anti-griefing mitigations (wash, gas drain, notional inflation, rebate harvesting, replay window) are missing in the broadcast path. Backend would broadcast economically-bad trades on mainnet. | Backend + Risk | `MAINNET-BACKEND-SHOULD-BROADCAST-IMPL` |
| C-5 `should_broadcast §4.2` rebate-solvency hard gate | **REQUIRED-BEFORE-MAINNET** (W-3) | Same as C-4; rebate-bearing trade would burn gas reverting `InsufficientRebateReserve`. | Operator pays for failed broadcasts; reveals subsidisable-rebate intent in plaintext logs. | Backend | folded into C-4 |
| C-6 Wash-trade detection (same-beneficial-owner, cluster heuristics) | **REQUIRED-BEFORE-MAINNET** | Per gas/fees/rebates policy §6 + T-6. | Rebate-mining attack on the first mainnet rebate-bearing day. | Backend + Risk | folded into C-4 |
| C-7 Dedupe cache + per-address nonce window | **REQUIRED-BEFORE-MAINNET** | Spec'd in T-3. | Replay across queue prunes; double-execution post restart. | Backend | `MAINNET-BACKEND-DEDUPE-NONCE-STORE` |
| C-8 Subsidy budget registry + per-reason cap + 1h burn alert | **REQUIRED-BEFORE-MAINNET** | Spec'd in T-4 + alert in monitoring §3.4. | Compromised BE silently sponsoring attacker. | Backend + Finance | `MAINNET-BACKEND-SUBSIDY-BUDGET-LEDGER` |
| C-9 Per-product gas profile table | **RECOMMENDED-BEFORE-MAINNET** | T-9. | One-size HARD_GAS_CAP is coarse; either rejects legitimate heavy products or admits pathological ones. | Backend + Load-test | folded into F-3 |
| C-10 Persistent metric collector + chain-control probes per `BACKEND_EXECUTOR_MONITORING_ALERTS_V1.md §6` | **REQUIRED-BEFORE-MAINNET** | MON-1 still open; no Prometheus exporter wired to the 25+ §2 metrics. | Mean-time-to-detect for compromise/drift unbounded. | Backend + SRE | folded into E-1 |
| C-11 Perp scaffold real broadcast still hard-stop in `src/execution/executor.rs:54-58` | **OPTIONAL** for option-only mainnet launch; **REQUIRED-BEFORE-MAINNET** if perp surface is launched | `EXECUTOR_DRY_RUN` must stay `true`; no perp on mainnet under current code. | If perp is in mainnet scope, the engine is unfinished. | Backend | `PERP-ENGINE-EXECUTOR-REAL-BROADCAST-IMPL` (only if perp in scope) |
| C-12 `/executor/transactions/<intent>` projection bug (returns `[]` for option intents) | **OPTIONAL** | Independent DB recovery from `option_execution_transactions` works. | Operator UX only. | Backend | `OPTION-AWARE-EXECUTION-TRANSACTIONS-API` |
| C-13 RM `is_liquidatable(addr)` stable view exposed to backend | **REQUIRED-BEFORE-MAINNET** (only if liquidation in scope) | T-7 + §10 carve-out. | Liquidation path broken on day 1. | Backend + Protocol | `MAINNET-LIQUIDATION-FLAGGER-VIEW` |
| C-14 Liquidation path gating (`RG.liquidationPaused`, rebate-disable, subsidy budget) | **REQUIRED-BEFORE-MAINNET** (only if liquidation in scope) | Per gas/fees/rebates policy §7. | First mainnet liquidation either reverts or silently subsidises. | Backend + Risk | `MAINNET-LIQUIDATION-DECISION-PATH` |
| C-15 Option execution unit tests covering each §5.1/§5.2/§6/§7/§8 branch | **REQUIRED-BEFORE-MAINNET** | T-10 — required pre-mainnet per BE-PROD-4. | No regression coverage for the broadcast economics. | Backend | `MAINNET-BACKEND-BROADCAST-POLICY-TESTS` |

---

### D. Backend custody / signing

| Item | Status | Gap | Risk if ignored | Owner | Next task |
|---|---|---|---|---|---|
| D-1 `EXECUTOR_PRIVATE_KEY` env-var path | **DONE for Sepolia rehearsal**; **REQUIRED-BEFORE-MAINNET to remove** | W-5: per `BACKEND_SIGNER_CUTOVER_RUNBOOK §13`, mainnet must NOT use env-var path. | Key on disk/.env; copy-paste exfil; impossible to enforce KMS audit trail. | Backend + Security + Custody | `MAINNET-KMS-HSM-SIGNER-INTERFACE` |
| D-2 KMS-backed signing interface (per-tx call returns signature without raw-key exposure) | **REQUIRED-BEFORE-MAINNET** | Not implemented; `BACKEND_EXECUTOR_CUSTODY.md §2-3` shows custody TODOs open. Mainnet variant per `BACKEND_EXECUTOR_CUSTODY_PROFILE_V2G_GOV_F.md §2.1`. | id. | Backend + Security | folded into D-1 |
| D-3 Mainnet BACKEND_EXECUTOR address (distinct EOA, distinct KMS region) | **REQUIRED-BEFORE-MAINNET** | Per `FIRST_LIVE_SMOKE_AUTHORIZATION §13`. Today's BE EOA was used on Sepolia. | Cross-chain replay surface (different chainId in EIP-712 saves us, but key-reuse still elevates blast radius). | Operator + Custody | `MAINNET-BE-ADDRESS-PROVISION` |
| D-4 BE funding policy committed (FUND_FLOOR / FUND_TARGET / FUND_CEILING) | **W-1 WAIVED on Sepolia**; **REQUIRED-BEFORE-MAINNET** | Sepolia BE at `3.8e15 wei` (38% of FUND_TARGET). Mainnet needs hard policy with refill source + alert thresholds in fiat-anchored units. | BE silently stops signing trades on a quiet weekend; users see "no broadcasts" with no alert. | Operator + Finance | `MAINNET-BE-FUND-POLICY-COMMIT` |
| D-5 `BE_CODE_NONZERO` / `BE_OOB_TX` alerts (EIP-7702 delegation / out-of-bounds tx) | **REQUIRED-BEFORE-MAINNET** | Spec'd in `BACKEND_EXECUTOR_MONITORING_ALERTS_V1.md §3.1`; not wired. | Key compromise undetected for >5 min. | SRE + Backend | folded into E-1 |
| D-6 Rotation runbook drilled (compromise → freeze → rotate → unpause) | **REQUIRED-BEFORE-MAINNET** | `BACKEND_EXECUTOR_CUSTODY.md §7-8` is spec; no Sepolia drill of the full sequence. | First mainnet incident is the first time anyone runs the runbook. | Ops + Security | `MAINNET-COMPROMISE-DRILL-SEPOLIA` |
| D-7 BACKEND_EXECUTOR_NEXT pre-provisioned (warm spare) | **RECOMMENDED-BEFORE-MAINNET** | Per custody §6.3. | Mainnet rotation takes Timelock 72h; no warm replacement = 72h outage. | Operator + Custody | folded into D-3 |

---

### E. Monitoring / alerting

| Item | Status | Gap | Risk if ignored | Owner | Next task |
|---|---|---|---|---|---|
| E-1 PagerDuty + Discord routing per `BACKEND_EXECUTOR_MONITORING_ALERTS_V1.md §3 / §4` | **W-2 WAIVED on Sepolia**; **REQUIRED-BEFORE-MAINNET** | MON-4 + MON-5 open; no synthetic-alert firing tested. | Mean-time-to-detect unbounded. | SRE + DevOps | `MAINNET-MONITORING-ALERTS-WIRING` |
| E-2 25+ Prometheus gauges + counters wired (signer / broadcast / economics / lifecycle / chain controls) | **REQUIRED-BEFORE-MAINNET** | MON-1 / MON-2 open; metric exporter not implemented. | Black-box on first mainnet day. | Backend + SRE | folded into E-1 |
| E-3 R5 drift alert (`r5_drift_musdc != 0` → critical page) | **REQUIRED-BEFORE-MAINNET** | Spec'd §3.5 [R5_DRIFT]. Sepolia tracked manually per smoke. | Accounting incident undetected. | SRE | folded into E-1 |
| E-4 Grafana dashboard `be-executor-overview` (6 rows × 30 panels) | **REQUIRED-BEFORE-MAINNET** | MON-3 open. | Investigation UX poor; longer MTTR. | SRE | folded into E-1 |
| E-5 `RUNBOOK_BACKEND_EXECUTOR.md` one-pager per alert | **REQUIRED-BEFORE-MAINNET** | MON-6 open. | On-call escalation flows undefined. | Ops + SRE | `MAINNET-BE-RUNBOOK-PUBLISH` |
| E-6 Cross-check job per `§6.3` (hourly drift detection between probes and stream) | **RECOMMENDED-BEFORE-MAINNET** | MON-7 open. | Silent monitoring-pipeline bugs. | Backend + SRE | folded into E-1 |
| E-7 Quarterly synthetic fault-injection rehearsal | **RECOMMENDED-BEFORE-MAINNET** | MON-8 open. | Alert routing rot; first real alert misroutes. | SRE | `MAINNET-SYNTHETIC-ALERT-DRILL` |
| E-8 Logging hygiene PR linter (high-cardinality / secret material) | **RECOMMENDED-BEFORE-MAINNET** | MON-9 open. | Future log addition leaks tx_hash / KMS handle / address into labels. | Backend + Security | `MAINNET-LOG-HYGIENE-LINTER` |
| E-9 Solidity-side `MONITORING_SPEC.md` (oracle, role drift, liquidation, settlement, OI, insurance) indexer | **REQUIRED-BEFORE-MAINNET** | Spec only; indexer not deployed. | Chain-side incidents go uncaught (oracle stale, OI breach, residual bad debt). | Ops + DevOps | `MAINNET-PROTOCOL-INDEXER-MONITORING` |
| E-10 Audit-log retention sink (`deopt.admin.audit`) | **REQUIRED-BEFORE-MAINNET** (B-H2) | `INTERNAL_AUDIT_FINDINGS_V2G_AUDIT0.md B-H2`. | Forensic audit trail mixed with general log; query-impossible post-incident. | Backend + Security | `MAINNET-V2G-W2-1-AUDIT-RETENTION` |

---

### F. Gas / fees / rebates / economics

| Item | Status | Gap | Risk if ignored | Owner | Next task |
|---|---|---|---|---|---|
| F-1 Fee profile tier 0 OPTION = `(maker=50ppm, taker=250ppm)` | **DONE** | proven on Sepolia: `+3000 mUSDC per $10 premium` = 300 ppm of premium per side conservation 0 across orderbook + RFQ | — | — | — |
| F-2 `PNL_FLOOR > 0` for mainnet (Sepolia = 0) | **REQUIRED-BEFORE-MAINNET** | `BACKEND_GAS_FEES_REBATES_POLICY_V1.md §9` mainnet column: `pnl_floor > 0` covering operating margin. | Backend broadcasts at-cost or near-cost trades; net protocol margin negative. | Backend + Finance | `MAINNET-BACKEND-PNL-FLOOR-COMMIT` |
| F-3 `HARD_GAS_CAP` + `MAX_MAX_FEE_PER_GAS` re-derived for mainnet economics | **REQUIRED-BEFORE-MAINNET** | Sepolia: `1.5e6 / basefee×3+2gwei`. Mainnet base fees and L1 data costs are 10-1000× larger. | Mainnet maxFee ceiling either hits during organic activity or admits attacker bidding wars. | Backend + SRE | folded into F-2 |
| F-4 `SAFETY_MARGIN`, `GAS_SAFETY_FACTOR`, `subsidy_budget[*].cap` reset for fiat-anchored units | **REQUIRED-BEFORE-MAINNET** | Per §15. | Budget consumed in mUSDC-equivalent terms instead of USDC; off-by-1e0 risk on the first liquidation subsidy. | Backend + Finance | folded into F-2 |
| F-5 `rebateReserve` allocation policy (custody, Safe-tx, refresh cadence) | **W-6 WAIVED**; **RECOMMENDED-BEFORE-MAINNET** if rebate program enabled | `BACKEND_GAS_FEES_REBATES_POLICY_V1.md §12`: requires Timelock-queued `PFV.allocateToRebateReserve(mUSDC, X)`. No allocation has occurred. | Without funded reserve, every rebate-bearing trade reverts at PFV hook. | Operator + Finance | `MAINNET-REBATE-RESERVE-ALLOCATION-DESIGN` |
| F-6 `maker_rebate_quota` per maker per 24h | **REQUIRED-BEFORE-MAINNET** if rebate program enabled | Per anti-griefing §6 + policy §9. | Rebate-budget DoS on day 1. | Backend + Risk | folded into C-4 |
| F-7 Oracle freshness (`eth_price_in_asset`) ≤ 60 s gate in `should_broadcast` | **REQUIRED-BEFORE-MAINNET** | Per policy §9 + §3 anti-griefing. | Backend's gas P&L math drifts when oracle is stale; subsidy decisions wrong. | Backend + Risk | folded into C-4 |
| F-8 PFV `withdrawRevenue` SOP (Timelock-queued) | **REQUIRED-BEFORE-MAINNET** | PFV revenue grows; no documented operator path to sweep. | Long-term: fees stranded; treasury process undefined. | Governance + Finance | `MAINNET-PFV-WITHDRAW-REVENUE-SOP` |
| F-9 Sweep PFV → fiat → BE top-up loop documented | **RECOMMENDED-BEFORE-MAINNET** | Per `BACKEND_GAS_FEES_REBATES_POLICY_V1.md §2.1`. | BE silently runs out of gas long-term. | Operator + Finance | folded into D-4 |

---

### G. RFQ / orderbook product flows

| Item | Status | Gap | Risk if ignored | Owner | Next task |
|---|---|---|---|---|---|
| G-1 Sepolia orderbook live tx | **DONE** | — | — | — | — |
| G-2 Sepolia RFQ live tx | **DONE** | — | — | — | — |
| G-3 Mainnet first-live-smoke authorisation `MAINNET_FIRST_LIVE_SMOKE_AUTHORIZATION.md` (3 + 1 signatures) | **REQUIRED-BEFORE-MAINNET** | Per `FIRST_LIVE_SMOKE_AUTHORIZATION_V2G_FX_Q1.md §13`: mainnet requires fourth signature (audit sign-off) and KMS-backed signer. | First-mainnet trade fires without combined attestation. | Operator + SRE + Risk + Audit | `MAINNET-FIRST-LIVE-SMOKE-AUTHORIZATION` |
| G-4 Initial MM accounts identified + role-separated from privileged protocol roles | **REQUIRED-BEFORE-MAINNET** | `FINAL_LAUNCH_CHECKLIST.md` Market-Maker row. | A maker account that overlaps with operator key surface is a privileged shortcut. | Market/Liquidity Lead | `MAINNET-MM-ACCOUNT-ROSTER` |
| G-5 Initial liquidity plan per launch option series + perp market (size, spread, cap) | **REQUIRED-BEFORE-MAINNET** | id. | Launch day with no liquidity; failed launch. | Market/Liquidity | folded into G-4 |
| G-6 Liquidity withdrawal / halt plan for incident response | **RECOMMENDED-BEFORE-MAINNET** | id. P2 row. | Pause requires manual MM coordination during incident. | Market/Liquidity + Ops | folded into M-x |

---

### H. Accounting / R5 / PFV / CV invariants

| Item | Status | Gap | Risk if ignored | Owner | Next task |
|---|---|---|---|---|---|
| H-1 R5 drift (`CV(PFV) − feeBalance − rebateReserve == 0`) | **DONE** preserved across 7 GOV-G tx + 2 live trades | — | — | — | — |
| H-2 R5 drift live alert + Grafana single-stat panel | **REQUIRED-BEFORE-MAINNET** | Spec'd; not deployed (see E-3). | Accounting incident undetected. | SRE | folded into E-1 |
| H-3 PFV invariant fuzz suite (5 invariants × 128k calls) | **DONE** per V2G-R1 | — | — | — | — |
| H-4 CV yield-adapter rotation invariant test | **RECOMMENDED-BEFORE-MAINNET** | `INVARIANT_FUZZ_COVERAGE_MATRIX §1.5 / S-L2`. | Adapter rotation could silently break totals; undetected if not invariant-tested. | Protocol | folded into B-3 |
| H-5 FM-V2 rebate-budget identity invariant fuzz | **RECOMMENDED-BEFORE-MAINNET** | `§3` gap-survey item. | Budget tracking diverges from chain on edge sequences. | Protocol | folded into B-3 |
| H-6 OPTION/PERP nonce-monotonic invariant fuzz | **RECOMMENDED-BEFORE-MAINNET** | `§1.4` gap. | Nonce-reuse bug undetected pre-mainnet. | Protocol | folded into B-3 |
| H-7 Hourly cross-check job: probe-stream R5 drift vs recompute | **RECOMMENDED-BEFORE-MAINNET** | MON-7 of monitoring spec. | Probe-pipeline bug masks chain-side drift. | Backend + SRE | folded into E-6 |

---

### I. Risk / margin / liquidation

| Item | Status | Gap | Risk if ignored | Owner | Next task |
|---|---|---|---|---|---|
| I-1 `INVARIANTS.md` §1.2-§1.7 hard invariants | **DONE** as spec | — | — | — | — |
| I-2 Liquidation flow live (engine + insurance + bad-debt routing) | **REQUIRED-BEFORE-MAINNET** if liquidation in scope | Not exercised on Sepolia smoke arc (fee-only orderbook + RFQ only). | First mainnet liquidation is first time the full path runs. | Risk + Protocol | `MAINNET-LIQUIDATION-FULL-PATH-REHEARSAL` |
| I-3 Settlement flow live (`SettlementPriceProposed → Finalized → AccountSettled`) | **REQUIRED-BEFORE-MAINNET** if settlement in scope (option expiry on Sepolia is far out — `2030-01-01`) | Not exercised; first-live smoke was OPTION but not settled. | First mainnet expiry brings the whole settlement path live with real money. | Risk + Protocol | `MAINNET-SETTLEMENT-PATH-REHEARSAL` |
| I-4 `RiskGovernor.liquidationPaused` operator path tested | **REQUIRED-BEFORE-MAINNET** | Per `INVARIANTS.md §1.5` + monitoring `LIQ_PAUSED`. | Cannot pause liquidations in incident. | Risk + Ops | folded into I-2 |
| I-5 Insurance fund funded above launch threshold | **REQUIRED-BEFORE-MAINNET** | `FINAL_LAUNCH_CHECKLIST.md` Insurance row. Sepolia uses test-token; mainnet needs real funding. | Bad debt during launch can't be backstopped. | Insurance + Finance | `MAINNET-INSURANCE-FUND-FUNDING` |
| I-6 Launch caps configured per option series + per perp market | **REQUIRED-BEFORE-MAINNET** | id. Config Readiness row + multiple sub-items per `INVARIANTS.md` product baselines. | Concentration risk; size shock day-1. | Risk + Ops | `MAINNET-LAUNCH-CAPS-CONFIG` |
| I-7 Stale / zero / unavailable / future-timestamp oracle paths tested | **REQUIRED-BEFORE-MAINNET** | Solidity oracle invariants pinned by source but no end-to-end drill on Sepolia. | Liquidation path uses bad price. | Security + Oracle Admin | `MAINNET-ORACLE-FAILURE-DRILL` |

---

### J. Frontend / admin operations

| Item | Status | Gap | Risk if ignored | Owner | Next task |
|---|---|---|---|---|---|
| J-1 V2G-W3 SSR proxy (Next.js `middleware.ts` + `/api/admin/[...path]/route.ts`) | **REQUIRED-BEFORE-MAINNET** (F-H1 / B-H1 / W-12) | Browser-side `sessionStorage["deopt.adminToken"]` + plaintext `X-Admin-Token` header; XSS exfil → admin access. | High audit finding. | Frontend + Backend + Security | `MAINNET-V2G-W3-SSR-PROXY` |
| J-2 `STATIC_FACTS` refreshed post-V2G-P / V2G-R5 (UI accuracy) | **RECOMMENDED-BEFORE-MAINNET** | F-M1; UI shows stale "not deployed" for OPTION RFQ + PFV. | Operator UX confusion; risk of acting on stale data. | Frontend | `FRONTEND-STATIC-FACTS-REFRESH` |
| J-3 `<AdminDashboard>` consumes `admin-rbac-types` + hides operator panels per role | **RECOMMENDED-BEFORE-MAINNET** | F-L1. | UX nicety only; backend middleware is the security boundary. | Frontend | folded into J-1 |
| J-4 CI lint: no `dangerouslySetInnerHTML` / no wallet libs under `src/app/admin/**` | **RECOMMENDED-BEFORE-MAINNET** | F-I1 / F-I2 verified clean; lock with CI. | Future drift introduces XSS surface. | Frontend + Security | `FRONTEND-ADMIN-CI-LOCKDOWN` |
| J-5 OIDC / hardware-MFA at admin edge | **REQUIRED-BEFORE-MAINNET** | Hard-rule sweep "OIDC / hardware MFA at edge ❌". | Single-token admin auth at mainnet operator surface. | Security + DevOps | folded into J-1 |
| J-6 Strict CSP for admin pages | **REQUIRED-BEFORE-MAINNET** | Same row "Strict CSP ❌". | XSS surface broader than necessary. | Security + Frontend | folded into J-1 |

---

### K. Deployment / secrets / env management

| Item | Status | Gap | Risk if ignored | Owner | Next task |
|---|---|---|---|---|---|
| K-1 Mainnet manifest filled (no `TODO_REPLACE_*` placeholders) | **BLOCKING** for mainnet activation | `deopt-v2-sol/deployments/mainnet.template.json` has 99 `TODO/null/placeholder` matches. | Cannot deploy; obvious failure but flagged for completeness. | Deployment Owner + Ops | `MAINNET-MANIFEST-FILL` |
| K-2 `chain_id` and RPC verified at every script boundary | **REQUIRED-BEFORE-MAINNET** | `FINAL_LAUNCH_CHECKLIST.md` Deployment row + per-script wrong-chain guards `S-I1`. | Wrong-chain broadcast (script targets Sepolia while RPC points at mainnet). | Deployment Owner + Ops | folded into K-1 |
| K-3 `EXECUTOR_PRIVATE_KEY` removed from `.env`; only KMS handle | **REQUIRED-BEFORE-MAINNET** (W-5) | Per `BACKEND_SIGNER_CUTOVER_RUNBOOK §13`. | See D-1. | Backend + Security | folded into D-1 |
| K-4 `.env` audit pre-mainnet: no admin tokens, no DATABASE_URL with embedded credentials in plaintext, no RPC API keys in version control | **REQUIRED-BEFORE-MAINNET** | Custody §10 TODOs open; admin token storage policy pending. | Disk-resident credentials. | Security + DevOps | `MAINNET-ENV-SECRETS-AUDIT` |
| K-5 Mainnet RPC endpoint with dedicated quota + RPC failover | **REQUIRED-BEFORE-MAINNET** | Monitoring probe hygiene §6.2 caps RPC pool at 5; mainnet load + monitoring needs sized capacity. | Alchemy throttling → black monitoring window. | DevOps | `MAINNET-RPC-PROVIDER-CONTRACT` |
| K-6 Backup of pre-edit env (mode 0600) on every operator edit | **DONE pattern** (every Sepolia env edit has `.env.bak.*`) | Carry forward to mainnet ops. | — | Operator | mandated by `MAINNET-OPERATOR-RUNBOOK` |
| K-7 `cargo audit` in backend CI | **RECOMMENDED-BEFORE-MAINNET** | B-I1. | Supply-chain vuln undetected. | Backend + DevOps | `MAINNET-CARGO-AUDIT-CI` |
| K-8 Slither / Mythril in CI | **RECOMMENDED-BEFORE-MAINNET** | `AUDIT_GATE_DECISION §4`; deferred to AUDIT-EXT environment. | Static-analysis findings undetected. | Protocol + DevOps | folded into B-2 |

---

### L. Testing / audit / fuzz / invariants

| Item | Status | Gap | Risk if ignored | Owner | Next task |
|---|---|---|---|---|---|
| L-1 `forge test --no-match-path 'test/fork/*'` green | **DONE** at last audit close: 351/0/0 per `AUDIT_GATE_DECISION_V2G_AUDIT0.md §4` | re-run on launch commit per `FINAL_LAUNCH_CHECKLIST.md` Code Readiness | — | Protocol | `MAINNET-LAUNCH-COMMIT-TEST-PASS` |
| L-2 `cargo test --all-targets --all-features` green | **DONE**: 764/0/0 | re-run on launch commit | — | Backend | folded into L-1 |
| L-3 Fork tests against mainnet pinned block | **REQUIRED-BEFORE-MAINNET** | Sepolia tests don't cover mainnet token / oracle topology. | Mainnet-specific failure (USDC permit2, Chainlink/Pyth feeds, base fee schedule). | Protocol + Backend | `MAINNET-FORK-TEST-SUITE` |
| L-4 External audit AUDIT-EXT engagement closed (no unresolved Critical/High) | **BLOCKING** for mainnet activation | Per `FINAL_LAUNCH_CHECKLIST.md` Audit row + `AUDIT_GATE_DECISION §2.4`. | Mainnet launch with unaudited surface. | Security | folded into B-2 |
| L-5 Staging rehearsal (full deploy + handoff + smoke + drills) | **REQUIRED-BEFORE-MAINNET** | `FINAL_LAUNCH_CHECKLIST.md` Staging Rehearsal row; Sepolia ≠ staging-with-mainnet-config. | First operator pass at the full mainnet rehearsal is the production deploy. | Ops + Deployment Owner | `MAINNET-STAGING-REHEARSAL-FULL` |
| L-6 Functional smoke tests in staging for deposit/withdraw/options/perps/funding/liquidation/insurance/bad-debt | **REQUIRED-BEFORE-MAINNET** | id. Smoke row. | Whole-surface regressions undetected. | Security + Protocol | folded into L-5 |
| L-7 Incident drills (oracle stale, matching compromise, close-only, collateral cap, OI cap) | **REQUIRED-BEFORE-MAINNET** | id. Incident-drill row + custody §7. | First incident is the first drill. | Ops + Security | folded into L-5 |
| L-8 Invariant suite extensions per `INVARIANT_FUZZ_COVERAGE_MATRIX §3 / §5` | **RECOMMENDED-BEFORE-MAINNET** | Gaps catalogued; not yet implemented. | Subtle bugs missed. | Protocol + Security | folded into B-3 |
| L-9 Slither output reviewed | **RECOMMENDED-BEFORE-MAINNET** | Deferred per audit-gate. | Static-analysis findings missed. | Security | folded into B-2 |

---

### M. Incident response / rollback / pause

| Item | Status | Gap | Risk if ignored | Owner | Next task |
|---|---|---|---|---|---|
| M-1 Guardian (OPS_MULTISIG) pause path tested on Sepolia | **REQUIRED-BEFORE-MAINNET** | NEW_OME.guardian = OPS_MULTISIG since GOV-A-OME; pause not exercised in this arc. | First incident is first pause; Safe coordination overhead unknown. | Ops + Governance | `MAINNET-GUARDIAN-PAUSE-DRILL-SEPOLIA` |
| M-2 Emergency `queuePaused = true` Sepolia rehearsal | **RECOMMENDED-BEFORE-MAINNET** | Per Timelock `pauseQueueing` `onlyGuardianOrOwner`. | id. for Timelock-side incidents. | Ops + Governance | folded into M-1 |
| M-3 `BACKEND_EXECUTOR` compromise → freeze → rotate → unpause Sepolia drill | **REQUIRED-BEFORE-MAINNET** (D-6) | id. | First runbook execution is the live incident. | Ops + Security | folded into D-6 |
| M-4 Forward-recovery rehearsal OPS_MULTISIG → DEPLOYER (4-step Safe sequence) | **RECOMMENDED-BEFORE-MAINNET** | `V2G_GOV_G_RESULT.md §8` documents; not drilled. | First emergency recovery operator-untested. | Governance + Ops | `MAINNET-FORWARD-RECOVERY-DRILL` |
| M-5 Incident artifact template + storage location | **REQUIRED-BEFORE-MAINNET** | `FINAL_LAUNCH_CHECKLIST.md` Incident row P2 item. | Post-incident report quality poor. | Ops | `MAINNET-INCIDENT-ARTIFACT-TEMPLATE` |
| M-6 Recovery / unpause criteria signed by governance + security leads | **REQUIRED-BEFORE-MAINNET** | id. | Ambiguous "are we safe to unpause?" decision under stress. | Governance + Security | `MAINNET-RECOVERY-CRITERIA-SIGN-OFF` |
| M-7 Per-alert runbook one-pager (MON-6) | **REQUIRED-BEFORE-MAINNET** | (E-5). | Each alert handled ad-hoc. | Ops + SRE | folded into E-5 |
| M-8 RB-PAUSE-REBATES (`setRebateFundingAccount(0)`) operator rehearsal | **RECOMMENDED-BEFORE-MAINNET** | Per `S-M1` accepted; documented mechanism not drilled. | Rebate-incident response slow. | Ops + Risk | `MAINNET-REBATES-PAUSE-DRILL` |

---

### N. Mainnet address / config migration

| Item | Status | Gap | Risk if ignored | Owner | Next task |
|---|---|---|---|---|---|
| N-1 USDC = canonical Base mainnet USDC (vs mUSDC test token) | **REQUIRED-BEFORE-MAINNET** (W-8) | All fee/rebate accounting tested against mUSDC; need final swap. | Mis-configured asset address → fees route to wrong vault. | Protocol + Ops | `MAINNET-ASSET-MIGRATION-USDC-CANONICAL` |
| N-2 Mainnet oracle sources (Chainlink + Pyth) configured, active, fresh, scaled `1e8` | **REQUIRED-BEFORE-MAINNET** | `FINAL_LAUNCH_CHECKLIST.md` Oracle row; mainnet manifest has `TODO_REPLACE_MAINNET_ETH_USDC_PRIMARY_SOURCE`. No mock feeds on mainnet (S-I3 risk). | Oracle is stale / zero / mock on day 1. | Oracle Admin + Security | `MAINNET-ORACLE-FEED-CONFIG` |
| N-3 Mainnet ETH/BTC/USDC token addresses + decimals + role-classification | **REQUIRED-BEFORE-MAINNET** | mainnet manifest `tokenAddresses: TODO_REPLACE_*`. | id. | Protocol + Ops | folded into K-1 |
| N-4 RG `liquidationParams` re-tuned for mainnet (penalty, spread, close-factor) | **REQUIRED-BEFORE-MAINNET** | `INVARIANTS.md §2.1` baseline vs per-product §3.x; verify on mainnet manifest. | Liquidation either too aggressive or too lenient. | Risk | `MAINNET-LIQUIDATION-PARAMS-COMMIT` |
| N-5 Fee tier matrix re-validated for mainnet asset units | **REQUIRED-BEFORE-MAINNET** | Mainnet USDC = 6 decimals (same as mUSDC), so unit math should hold; verify against canonical fee audit pack on mainnet manifest. | Off-by-1e0 fees. | Backend + Protocol | folded into N-1 |
| N-6 OLD perp engine address declared observability-only (Prometheus `FeeOldConsumer` alert green) | **REQUIRED-BEFORE-MAINNET** | `S-I4`; document in V2G-Y emergency runbook (already done — confirm mainnet variant). | Operator routes fees to legacy engine accidentally. | Ops + Backend | `MAINNET-LEGACY-ENGINE-INVENTORY` |
| N-7 Backend `OPTION_MATCHING_ENGINE_ADDRESS` (and analogous keys) committed to mainnet config | **REQUIRED-BEFORE-MAINNET** | Backend env naming parity with mainnet manifest per `B-I3` (accepted). | Mis-routing of intents to wrong engine. | Backend + Ops | folded into K-1 |

---

## 4. Prioritised roadmap

### P0 — BLOCKING (must close to even begin mainnet planning safely)

```
[ ] A-5  / W-11        V2G-Y ownership migration — mainnet owners → Timelock → Governance Safe
[ ] B-2  / L-4 / W-14  External audit AUDIT-EXT engagement complete; no Critical/High open
[ ] K-1                Mainnet manifest filled (99 placeholders → 0)
```

### P1 — REQUIRED-BEFORE-MAINNET (must close before any mainnet broadcast)

```
Governance
[ ] A-3                Mainnet GOVERNANCE_MULTISIG Safe (≥ 3-of-5 + MFA) deployed and signers attested
[ ] A-4                Mainnet ProtocolTimelock deployed with minDelay = 72h
[ ] A-7                Cancel/queue/execute rehearsal on the mainnet Safe (before activation)

Backend custody / signing
[ ] D-1 / W-5          KMS / HSM signer interface (drop EXECUTOR_PRIVATE_KEY env path)
[ ] D-3                Mainnet BACKEND_EXECUTOR address (distinct EOA, distinct KMS region)
[ ] D-4 / W-1          BE funding policy committed (FUND_FLOOR / TARGET / CEILING) in fiat units
[ ] D-6 / M-3          Compromise → freeze → rotate → unpause Sepolia drill executed

Backend execution path
[ ] C-4                should_broadcast implemented per gas/fees/rebates policy §8 (incl. anti-griefing §6)
[ ] C-5 / W-3          Rebate-solvency hard gate live in should_broadcast
[ ] C-6                Wash-trade detection live
[ ] C-7                Persistent dedupe cache + nonce window store
[ ] C-8                Subsidy budget registry + per-reason cap + 1h burn alert
[ ] C-15               Unit tests for every should_broadcast branch (BE-PROD-4)

Monitoring / alerting
[ ] E-1 / W-2          PagerDuty + Discord routing wired + synthetic alert fired
[ ] E-2                25+ Prometheus metrics exported and scraped
[ ] E-3 / H-2          R5 drift critical-page alert live
[ ] E-4                Grafana be-executor-overview dashboard live
[ ] E-5 / M-7          Per-alert runbook one-pagers published
[ ] E-9                Solidity-side indexer (oracle / role / liquidation / settlement / OI / insurance)
[ ] E-10 / W-13        Admin audit-log retention sink live

Frontend / admin
[ ] J-1 / W-12         V2G-W3 SSR proxy + drop sessionStorage admin token
[ ] J-5                OIDC / hardware-MFA at admin edge
[ ] J-6                Strict CSP on admin pages

Gas / fees / rebates / economics
[ ] F-2                PNL_FLOOR > 0 mainnet value committed
[ ] F-3                HARD_GAS_CAP / MAX_MAX_FEE_PER_GAS re-derived for mainnet
[ ] F-4                SAFETY_MARGIN / subsidy caps in fiat-anchored units
[ ] F-8                PFV.withdrawRevenue SOP

Risk / margin / liquidation (only if in launch scope)
[ ] I-2                Liquidation full path rehearsal (engine + insurance + bad-debt routing)
[ ] I-3                Settlement path rehearsal (next series with shorter expiry)
[ ] I-5                Insurance fund funded above launch threshold
[ ] I-6                Launch caps configured per option series + perp market
[ ] I-7                Oracle failure-mode drill (stale / zero / unavailable / future-timestamp)

Product flows
[ ] G-3                MAINNET_FIRST_LIVE_SMOKE_AUTHORIZATION (4 signatures)
[ ] G-4                MM roster + role-separation
[ ] G-5                Initial liquidity plan per launch series

Deployment / secrets / env
[ ] K-3                .env free of EXECUTOR_PRIVATE_KEY; KMS handle only
[ ] K-4                Mainnet env secrets audit
[ ] K-5                Mainnet RPC provider with quota + failover

Testing / audit / fuzz
[ ] L-3                Fork test suite against mainnet pinned block
[ ] L-5 / L-6 / L-7    Full staging rehearsal + smoke + incident drills

Mainnet address / config migration
[ ] N-1 / W-8          USDC canonical asset migration (mUSDC → USDC)
[ ] N-2                Oracle feed config (Chainlink + Pyth) live and fresh
[ ] N-4                Liquidation params committed for mainnet
[ ] N-6                Legacy engine inventory declared

Incident response
[ ] M-1                Guardian (OPS_MULTISIG) pause-path drill on Sepolia
[ ] M-5                Incident artifact template + storage location
[ ] M-6                Recovery / unpause criteria sign-off
```

### P2 — RECOMMENDED-BEFORE-MAINNET (strongly advised; explicit waiver to skip)

```
[ ] A-6 / W-9          V2G-GOV-G-G6 Sepolia minDelay 24h → 72h
[ ] B-3 / H-4 / H-5 / H-6 / L-8   Invariant fuzz suite expansion
[ ] B-5                Mainnet deploy-script audit pass
[ ] C-9                Per-product gas profile table
[ ] D-7                BACKEND_EXECUTOR_NEXT warm spare pre-provisioned
[ ] E-6 / H-7          Hourly probe-stream cross-check job
[ ] E-7                Quarterly synthetic-alert drill
[ ] E-8                Logging-hygiene PR linter
[ ] F-5 / W-6          Rebate-reserve allocation design (if rebate program in launch scope)
[ ] F-9                PFV → fiat → BE top-up loop documented
[ ] G-6                Liquidity withdrawal / halt plan
[ ] J-2 / J-3 / J-4    Frontend STATIC_FACTS + RBAC hide + CI lockdown
[ ] K-7 / K-8          cargo audit + Slither in CI
[ ] M-2 / M-4 / M-8    Timelock-pause / forward-recovery / rebates-pause drills
```

### P3 — OPTIONAL (defer if scope-bound)

```
[ ] C-11               Perp scaffold real broadcast (only if perp surface in mainnet launch)
[ ] C-12               Option-aware execution-transactions API (operator UX cleanup)
[ ] C-13 / C-14        Liquidation flagger view + decision path (only if liquidation in scope)
[ ] S-L1               Document the V2G-RX atomicity invariant in a consumeFees comment
[ ] B-4 / S-L4         MAINNET_OK guard on WireProtocolFeeVaultFeesManager.s.sol
```

---

## 5. Remaining Sepolia waivers — explicit closure status

| Waiver | Status now (Sepolia) | Status required for mainnet | Closer task |
|---|---|---|---|
| W-1  BE-FUND | Open (3.8e15 < FUND_TARGET 1e16) | CLOSED (in fiat-anchored units) | D-4 |
| W-2  MON | Open (manual watch) | CLOSED (wired + synthetic-fired) | E-1 |
| W-3  REBATE-GATE | Open (chain backstop only) | CLOSED (backend hard gate) | C-4 / C-5 |
| W-4  SRE-ONCALL | Open (single actor) | CLOSED (3 + audit signatures) | G-3 |
| W-5  ENV-KEY | Open (env-var path) | CLOSED (KMS/HSM only) | D-1 |
| W-6  REBATE-RESERVE | Open (`rebateReserve = 0`) | Either CLOSED (funded) OR explicitly deferred from launch scope | F-5 |
| W-7  TEST-WALLETS | Open (local fixtures) | CLOSED (MM roster) | G-4 |
| W-8  TEST-TOKEN | Open (mUSDC) | CLOSED (canonical USDC) | N-1 |
| W-9  MIN-DELAY | Open (24h) | CLOSED (72h applied direct on mainnet) | A-4 / A-6 |
| W-10 ALARM-WAIVE-LOG | Open (manual watch) | folded into W-2 | E-1 |
| W-11 OWNER=EOA on mainnet | N/A on Sepolia (already migrated) | BLOCKING — CLOSED via V2G-Y on mainnet | A-5 |
| W-12 ADMIN TOKEN browser | Open (sessionStorage) | CLOSED (V2G-W3 SSR proxy) | J-1 |
| W-13 ADMIN AUDIT-LOG SINK | Open (mixed with general log) | CLOSED (retention sink) | E-10 |
| W-14 SLITHER/MYTHRIL | Open (deferred) | CLOSED (external audit run) | B-2 |

---

## 6. Recommended immediate next milestone

The single most informative milestone to run next is **the P0 cluster**, in this order:

1. **`MAINNET-AUDIT-EXT-ENGAGEMENT`** (B-2 / L-4 / W-14) — scope and engage the external audit using the handoff package in `INVARIANT_FUZZ_COVERAGE_MATRIX §6`. This unblocks the longest external timeline and runs in parallel to everything else.
2. **`MAINNET-MANIFEST-FILL`** (K-1) — read-only design task to nail down every `TODO_REPLACE_*` placeholder in `deopt-v2-sol/deployments/mainnet.template.json` and produce the mainnet equivalents of: oracle sources, USDC/ETH/BTC token addresses, governance roles, mainnet BACKEND_EXECUTOR target, Safe roster commitment. Output is a draft manifest with named owners per row.
3. **`MAINNET-V2G-Y-OWNERSHIP-MIGRATION-PLAN`** (A-5 / W-11) — read-only design task: produce the mainnet equivalent of the V2G-A/B/C/F/G arc, parameterised by the mainnet Safe + mainnet Timelock from items 1 and 2.

These three are read-only design + external-engagement tasks. None broadcasts. None edits `.env`. They unblock every other P1 item by establishing the mainnet target addresses, the audit timeline, and the ownership migration plan.

If the operator prefers an immediately actionable Sepolia step instead:

- **`V2G-GOV-G-G6-MIN-DELAY-DECISION-PACKET`** (A-6 / W-9) — single Safe-tx (operator-authorised, separately) to rehearse the 72h minDelay shape on Sepolia. Output is a decision packet + optional one OPS_MULTISIG Safe-tx.

**Status: Sepolia rehearsal arc structurally complete. Mainnet readiness gap list documented in this file. No chain mutation, no `.env` edit, no broadcast, no governance touch performed by this audit.**

---

## 7. Files touched (this milestone)

| Path | Change |
|---|---|
| `deopt-v2-backend/docs/MAINNET_READINESS_GAP_LIST_AFTER_SEPOLIA_ARC.md` | **CREATED** (this document) |

No source touched. No `.env` edit. No chain mutation. No DB mutation. No Safe-tx. No broadcast. No mainnet.

---

## 8. Validations

```
chain_id (when probed)                         : not probed (read-only doc audit)
governance state                               : no mutation by this milestone ✓
NEW_OME / PFV / NEW_FM_V2 / RG / CV state      : no mutation ✓
Timelock state                                 : no mutation ✓
BE.nonce                                       : no broadcast ✓
.env edits                                     : 0 ✓
DB writes                                      : 0 ✓
chain mutations                                : 0 ✓
Safe-tx                                        : 0 ✓
G-6 minDelay bump                              : not executed ✓
rebate reserve allocation                      : not executed ✓
RFQ / orderbook smoke                          : not run ✓
mainnet                                        : not touched ✓
secrets printed (PK / DATABASE_URL / RPC / admin token) : 0 ✓
docs created                                   : 1 (this file) ✓
docs updated                                   : 0
```

---

## 9. Blockers

None for this milestone. The audit is complete and read-only.

External blockers tracked above:
- P0 cluster blocks any credible mainnet launch.
- P1 cluster blocks any mainnet broadcast.

---

## 10. Cross-links

- `deopt-v2-sol/docs/V2G_GOV_G_RESULT.md`
- `deopt-v2-backend/docs/FIRST_LIVE_OPTION_EXECUTION_SMOKE_RESULT_V2_SEPOLIA.md`
- `deopt-v2-backend/docs/FIRST_LIVE_RFQ_OPTION_EXECUTION_SMOKE_RESULT_SEPOLIA.md`
- `deopt-v2-backend/docs/FX_Q1_C_OPERATOR_DECISION_PRE_FLIP.md`
- `deopt-v2-backend/docs/BACKEND_SIGNER_CUTOVER_RUNBOOK_V2G_FX_Q1.md`
- `deopt-v2-backend/docs/BACKEND_EXECUTOR_MONITORING_ALERTS_V1.md`
- `deopt-v2-backend/docs/BACKEND_GAS_FEES_REBATES_POLICY_V1.md`
- `deopt-v2-backend/docs/FIRST_LIVE_SMOKE_AUTHORIZATION_V2G_FX_Q1.md`
- `deopt-v2-backend/docs/RFQ_SMOKE_NONCE_SYNC_REMEDIATION.md`
- `deopt-v2-backend/docs/POST_GOV_G_OPS_CLEANUP_BEFORE_RFQ_SMOKE.md`
- `~/DEOPT/BACKEND_EXECUTOR_CUSTODY.md`
- `~/DEOPT/AUDIT_GATE_DECISION_V2G_AUDIT0.md`
- `deopt-v2-sol/docs/INTERNAL_AUDIT_FINDINGS_V2G_AUDIT0.md`
- `deopt-v2-sol/docs/INVARIANT_FUZZ_COVERAGE_MATRIX_V2G_AUDIT.md`
- `deopt-v2-backend/docs/INTERNAL_AUDIT_FINDINGS_V2G_AUDIT0.md`
- `deopt-v2-frontend/docs/INTERNAL_AUDIT_FINDINGS_V2G_AUDIT0.md`
- `deopt-v2-sol/FINAL_LAUNCH_CHECKLIST.md`
- `deopt-v2-sol/DEPLOYMENT_PLAN.md`
- `deopt-v2-sol/INVARIANTS.md`
- `deopt-v2-sol/MONITORING_SPEC.md`
- `deopt-v2-sol/deployments/mainnet.template.json`
- `~/DEOPT/RUN_STATE.md`

**End of gap list document.**
