# MAINNET-LAUNCH-READINESS — Next Task Brief

**Date written:** 2026-06-12
**Origin:** `PRODUCT_FREEZE_AND_SECURITY_REANCHOR_NEXT_TASK.md` + `docs/security-reanchor/MAINNET_READINESS_GAP_ANALYSIS.md`
**Target:** organise the closure of the 9 mainnet hard gates as a multi-milestone arc. **This brief does NOT activate mainnet.** Mainnet remains deliberately disabled. This brief structures the sequence of milestones that would precede any future mainnet decision.
**Posture:** **Planning + docs only. NEVER deploy contracts to mainnet under this brief. NEVER use mainnet RPC. NEVER edit backend `.env` to flip any production flag. NEVER touch private keys. NEVER claim mainnet is close. The output is a coordination doc; the actual closures happen in separate approval-gated milestones.**

> **This task is NOT executed by the calling milestone (security re-anchor).** It packages mainnet hard-gate coordination into one approval-gated planning milestone.

---

## 1. Literal operator approval line (REQUIRED, VERBATIM)

> "I approve DeOpt V2 mainnet launch readiness planning for this run."

Properties:
* Authorises drafting a multi-milestone coordination plan that maps the 9 gates to discrete approval-gated milestones.
* Authorises identifying owners + tentative sequencing.
* Does NOT authorise any actual mainnet activity (no deploys, no transactions, no firm engagements, no governance migrations).
* Does NOT authorise renaming or removing the public-beta vocabulary anywhere.
* Does NOT authorise flipping `isMainnetEnabled()` or any production flag.

---

## 2. The 9 hard gates

Per `docs/security-reanchor/MAINNET_READINESS_GAP_ANALYSIS.md`:

1. External audit complete (or documented decision not to).
2. Production signer / KMS / Safe plan complete.
3. Production monitoring + alerting complete.
4. Incident response runbook complete.
5. Pause / guardian / governance runbooks complete.
6. Deployment plan complete.
7. Liquidity / market-maker plan complete.
8. Legal / compliance / product-risk review complete.
9. Public docs adjusted from testnet to mainnet vocabulary.

A single missing closure = no mainnet. This brief structures the coordination; it does NOT close any gate.

---

## 3. Scope

### 3.1 Per-gate sub-milestones

For each of the 9 gates, draft a placeholder sub-milestone brief under `deopt-v2-backend/docs/mainnet-readiness/`:

* `01_EXTERNAL_AUDIT_CLOSURE_NEXT_TASK.md` (depends on `EXTERNAL_AUDIT_DISPATCH_PREP_NEXT_TASK.md` + an actual engagement)
* `02_PRODUCTION_SIGNER_KMS_CUTOVER_NEXT_TASK.md`
* `03_PRODUCTION_MONITORING_NEXT_TASK.md`
* `04_INCIDENT_RESPONSE_RUNBOOK_NEXT_TASK.md`
* `05_GOVERNANCE_MIGRATION_NEXT_TASK.md`
* `06_MAINNET_DEPLOYMENT_PLAN_NEXT_TASK.md`
* `07_LIQUIDITY_MARKET_MAKER_PLAN_NEXT_TASK.md`
* `08_LEGAL_COMPLIANCE_REVIEW_NEXT_TASK.md`
* `09_PUBLIC_DOCS_MAINNET_CUTOVER_NEXT_TASK.md`

Each placeholder brief should:
* Have its own literal approval line (NOT the one above).
* Cite the relevant section of `MAINNET_READINESS_GAP_ANALYSIS.md`.
* Be prominently marked: **THIS IS A PLACEHOLDER. NOT YET ACTIONABLE. Each sub-milestone requires its own operator approval and prior gates to be closed.**

### 3.2 Coordination index

Create `deopt-v2-backend/docs/mainnet-readiness/README.md`:
* Lists the 9 sub-milestones with their dependency arrows.
* States the hard rule: NO mainnet activation until ALL 9 close.
* Documents the global posture: mainnet remains disabled in the public-beta build; this directory is a coordination plan, not a runway to launch.

### 3.3 Closure tracker (template)

Create `deopt-v2-backend/docs/mainnet-readiness/CLOSURE_TRACKER.md`:
* 9 rows, one per gate.
* Columns: status (`open` / `in-progress` / `closed`), owner, last update, closure-doc link.
* All rows initially `open`.

### 3.4 Acknowledge what is NOT this brief

Spell out in the README that the following are explicitly NOT part of this milestone:
* Sending audit outreach.
* Signing any SOW.
* Deploying any mainnet contract.
* Touching mainnet RPC.
* Flipping `isMainnetEnabled()` or any production env flag.
* Removing testnet banners.
* Calling any production signer / KMS.
* Funding the executor with real ETH.

---

## 4. Out of scope

* Any chain action.
* Any external communication.
* Any vendor selection.
* Any source code change.
* Any change to the public-beta posture.

---

## 5. Hard preconditions

| # | Precondition | Verifying check |
|---|---|---|
| P1 | Approval line (§1) present verbatim | grep |
| P2 | Security re-anchor packet exists in full | `ls docs/security-reanchor/*.md` |
| P3 | Backend `.env` untouched | `stat -c '%y'` |
| P4 | Private file untouched | `stat -c '%a %y'` |
| P5 | `~/DEOPT/private/**` NOT read | trust |
| P6 | `isMainnetEnabled()` confirmed hard-coded `false` | `grep "return false" deopt-v2-frontend/src/lib/chains.ts` |
| P7 | Frontend public-beta banners + footer still wired | manual / e2e |

---

## 6. Forbidden

* Mainnet RPC.
* Mainnet deployment (any contract).
* Mainnet broadcast (any tx).
* Removing testnet banners from frontend.
* Flipping any production env flag.
* Engaging an audit firm.
* Engaging an MM.
* Engaging counsel under this brief (legal review is its own gate; outreach happens in `08_LEGAL_COMPLIANCE_REVIEW_NEXT_TASK.md` after its approval).
* Any "we're going to mainnet" external communication.

---

## 7. Acceptance criteria

* `mainnet-readiness/README.md` exists with dependency graph.
* 9 sub-milestone brief placeholders exist, each with its own literal approval line + clear PLACEHOLDER marker.
* `mainnet-readiness/CLOSURE_TRACKER.md` exists with all 9 rows initially `open`.
* `docs/security-reanchor/MAINNET_READINESS_GAP_ANALYSIS.md` cross-linked into the README.
* `git diff --check` clean.
* Sensitive-string scan zero hits.
* Positive-claim drift scan zero hits.
* `isMainnetEnabled()` in `deopt-v2-frontend/src/lib/chains.ts` still hard-coded `false`.

---

## 8. Cross-links

* `docs/security-reanchor/MAINNET_READINESS_GAP_ANALYSIS.md`
* `MAINNET_CUSTODY_POLICY.md` (project root)
* `MAINNET_CUSTODY_DECISIONS_ADDENDUM_TEMPLATE.md` (project root)
* `BACKEND_MAINNET_IMPLEMENTATION_ROADMAP.md`
* `~/DEOPT/RUN_STATE.md`

**End of mainnet launch readiness next-task brief.**
