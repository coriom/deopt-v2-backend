# DeOpt V2 — Mainnet Readiness Gap Analysis

> **Snapshot date:** 2026-06-12. **Posture:** public testnet beta. **Mainnet is permanently disabled in this build.**

This document is intentionally strict. Mainnet means **real user funds at risk**. Crossing any of these gates without explicit closure is unacceptable. The list is **deliberately longer than what's needed for audit dispatch**; an audit is necessary but not sufficient for mainnet.

---

## Hard gate model

Mainnet activation requires **all** of the following to be closed in writing:

1. External audit complete OR a documented decision NOT to engage one (with the rationale and the operator's named accountability).
2. Production signer / KMS / Safe plan complete.
3. Production monitoring + alerting complete.
4. Incident response runbook complete.
5. Pause / guardian / governance runbooks complete.
6. Deployment plan complete.
7. Liquidity / market-maker plan complete.
8. Legal / compliance / product-risk review complete.
9. Public docs adjusted from testnet to mainnet vocabulary.

A single missing closure = no mainnet.

---

## 1. External audit

### Status
**NOT STARTED.** No firm engaged. No SOW signed. No findings.

### Gate
* External audit complete (report received, findings addressed in writing, fix PRs merged).
* OR formal documented decision NOT to engage (highly discouraged; would require a written rationale and named operator accountability).

### Why blocking
* Contracts have never been reviewed by an unbiased third party.
* Internal review is not a substitute (auditors find what authors miss).

### Cross-reference
* `AUDIT_READINESS_GAP_ANALYSIS.md` (this packet)
* `PRE_AUDIT_ACTION_PLAN.md` (this packet)
* `EXTERNAL_AUDIT_DISPATCH_PREP_NEXT_TASK.md` (next-task brief)

---

## 2. Production signer / KMS / Safe plan

### Status
**DESIGN ONLY.** No KMS-backed signer deployed for production use. Local executor EOA at testnet.

### Gate
* KMS-backed executor signer deployed and round-trip tested on a mainnet **fork** (NOT live mainnet) before any mainnet broadcast.
* Vendor selected (`Q-CD-5` in custody policy — currently OPEN).
* IAM policy + CloudTrail audit log active.
* Key rotation runbook tested.
* Compromise IR runbook drafted + dry-run.

### Why blocking
* A plain testnet EOA holding a mainnet executor key is the single biggest red flag possible.
* Once mainnet funds exist, the executor key becomes a target.

### Cross-reference
* `MAINNET_CUSTODY_POLICY.md §R-3` (BACKEND_EXECUTOR role)
* `BACKEND_SIGNER_CUTOVER_RUNBOOK_V2G_FX_Q1.md`
* `AWS_KMS_OPERATOR_SETUP_PACK.md`
* `AWS_KMS_IAM_AND_KEY_POLICY_TEMPLATE.md`
* `AWS_KMS_CLOUDTRAIL_AND_MONITORING_RUNBOOK.md`
* `BACKEND_AWS_KMS_CLOUDTRAIL_REQUEST_ID_RESULT.md`
* `BACKEND_AWS_KMS_PRODUCTION_TRANSPORT_RESULT.md`
* `BACKEND_KMS_VENDOR_ADAPTER_IMPLEMENTATION_PLUGGABLE_RESULT.md`

---

## 3. Monitoring + alerting

### Status
**PARTIAL DESIGN.** `BACKEND_EXECUTOR_MONITORING_ALERTS_V1.md` drafted; not deployed.

### Gate
* Per-tx executor alerting active.
* Indexer fallback / stall alerts active.
* Signer integrity alerts active.
* Oracle-staleness alerts active.
* Vault-balance-divergence alerts active (alert if `sum(deposits) - sum(withdrawals) != vault.balanceOf`).
* `/trading/health` dashboard public OR on the operator's monitored screen 24/7.
* Pageable on-call rota exists.

### Why blocking
* Without monitoring, attacks that succeed go unnoticed until users complain.
* Without alerting, the on-call has nothing to wake up to.

### Cross-reference
* `BACKEND_EXECUTOR_MONITORING_ALERTS_V1.md`
* `BACKEND_EXECUTOR_MONITORING_ALERTS_V1_WIRING_RESULT.md`
* `ALERTING_SPEC.md`

---

## 4. Incident response runbook

### Status
**NOT YET COMPILED.** Individual runbooks (signer cutover, flag flip) exist as drafts.

### Gate
* Signer compromise runbook (rotate key, pause executor, drain executor allowance).
* Oracle compromise runbook (pause trades, switch oracle source, post status).
* Indexer poisoning runbook (rewind, re-process, alert).
* Vault drain attempt runbook (pause `executeTrade`, freeze withdrawals if possible, audit the vault state, communicate).
* Single-IR-index document linking each runbook.
* Dry-run of at least one runbook on a fork.

### Why blocking
* IR runbooks are necessary even if never used.
* Real-funds operations cannot rely on the operator improvising under pressure.

### Cross-reference
* No consolidated `INCIDENT_RESPONSE_INDEX.md` yet. Action item for `MAINNET_LAUNCH_READINESS_NEXT_TASK.md`.

---

## 5. Pause / guardian / governance runbooks

### Status
**DESIGN ONLY.** Governance contracts not deployed (`GOVERNANCE_*` docs are designs).

### Gate
* Owner role transferred to GOVERNANCE_MULTISIG (≥3-of-5, per `MAINNET_CUSTODY_POLICY.md §R-1`).
* OPS_MULTISIG deployed (≥2-of-3 OR 3-of-5 per `Q-CD-2` decision).
* TimelockController in place between governance and contract changes.
* Pause runbook tested on a fork.
* Guardian role assigned with limited revoke / pause powers per `GOVERNANCE_GUARDIAN_MIGRATION_V2G_GOV_A.md`.

### Why blocking
* A single EOA owner of a mainnet contract holding user funds is a single point of failure (key compromise = total loss).

### Cross-reference
* `MAINNET_CUSTODY_POLICY.md §R-1..R-9`
* `GOVERNANCE_OWNERSHIP_TRANSFER_PACKET_V2G_GOV_D.md`
* `GOVERNANCE_OWNERSHIP_TRANSFER_RESULT_V2G_GOV_D.md`
* `GOVERNANCE_OPS_MULTISIG_DEPLOY_PLAN_V2G_GOV_D2.md`
* `GOVERNANCE_GUARDIAN_MIGRATION_V2G_GOV_A.md`
* `GOVERNANCE_OME_GUARDIAN_MIGRATION_V2G_GOV_A_OME.md`

---

## 6. Deployment plan

### Status
**NOT WRITTEN for mainnet.** Sepolia deployment history exists.

### Gate
* Deployment script verified by independent reviewer.
* Per-contract deploy + verify (Etherscan) + ownership-assignment dry-run on a fork.
* Bidirectional wiring assertion as a hard gate (`MatchingEngine ↔ MarginEngine`).
* mUSDC analog: a real USDC instance (not a mock) registered as collateral.
* OracleRouter wired to a production oracle source (NOT `MockPriceSource`).
* Post-deploy chain-state snapshot archived.

### Why blocking
* Mainnet deployments are largely irreversible. Mistakes are expensive.

### Cross-reference
* `MAINNET_CUSTODY_POLICY.md` Q-CD-1..Q-CD-12 (some still open)
* `SEPOLIA_MATCHING_ENGINE_RETARGET_RESULT.md` (historical Sepolia lesson)
* `BACKEND_MAINNET_IMPLEMENTATION_ROADMAP.md`

---

## 7. Liquidity / market-maker plan

### Status
**NONE.** Testnet has no MM.

### Gate
* Real MM relationships secured.
* MM SLA / spread expectations documented (publicly or to MMs).
* Bootstrap-liquidity plan (if applicable).
* Risk transfer to MM is contractual, not implicit.

### Why blocking (product, not security)
* Mainnet without liquidity is a UX disaster and erodes trust.

### Cross-reference
* `KNOWN_LIMITATIONS_AND_RISKS.md` §4 (market-making absence at testnet).

---

## 8. Legal / compliance / product-risk review

### Status
**NOT REVIEWED EXTERNALLY** for mainnet.

### Gate
* Counsel review of the protocol's legal status in the operator's primary jurisdiction.
* Terms-of-service / risk-disclosure document drafted.
* KYC / sanctions screening decision (yes / no + rationale).
* Disclosures to users about regulatory status.

### Why blocking
* Without this, mainnet launch creates legal exposure regardless of whether the protocol is secure.

---

## 9. Public docs adjusted from testnet to mainnet vocabulary

### Status
**TESTNET POSTURE.** Every doc is testnet-positioned.

### Gate
* New mainnet-positioned variants OR a clear "public testnet beta → mainnet" cutover plan.
* All banners removed / replaced.
* All "no real funds" disclaimers replaced with risk-disclosure language.
* All `Base Sepolia` references updated.
* All canonical addresses point at mainnet contracts.
* Legacy testnet section preserved as a historical archive.

### Why blocking
* Misleading users is unacceptable. The current public-beta posture is the safe default; the unsafe default (and explicit closure) only happens when everything above is closed.

### Cross-reference
* `docs/public-beta/PUBLIC_TESTNET_BETA_LAUNCH_CHECKLIST.md` (testnet beta version)
* All 15 docs under `docs/public-beta/`

---

## Closing reminder

Each of the 9 gates is necessary. None alone is sufficient. Crossing them in parallel is fine; crossing none of them and pretending mainnet is "close" is not.

If at any point in the future a stakeholder pushes for mainnet without all 9 closures, the operator's job is to refuse with this document in hand.

The `MAINNET_LAUNCH_READINESS_NEXT_TASK.md` brief is the structured follow-up — it does NOT initiate mainnet, but it organises closure of these 9 gates as a multi-milestone arc.

---

**End of mainnet readiness gap analysis.**
