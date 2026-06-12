# EXTERNAL-AUDIT-DISPATCH-PREP — Next Task Brief

**Date written:** 2026-06-12
**Origin:** `PRODUCT_FREEZE_AND_SECURITY_REANCHOR_NEXT_TASK.md` + `docs/security-reanchor/AUDIT_READINESS_GAP_ANALYSIS.md`
**Target:** prepare the materials and decisions required before an external audit engagement could be **dispatched** (i.e., outreach sent to firms). This brief does NOT engage a firm. It does NOT send outreach copy. It is the final preparation milestone before any contact with external auditors.
**Posture:** **Docs + plan only. NEVER mainnet. NEVER chain transactions. NEVER backend `.env` edit. NEVER private key handling. NEVER any communication with an external firm under this brief. NEVER claim "audited" or "mainnet-ready".**

> **This task is NOT executed by the calling milestone (security re-anchor).** It packages audit-dispatch preparation into one approval-gated milestone.

---

## 1. Literal operator approval line (REQUIRED, VERBATIM)

> "I approve DeOpt V2 external audit dispatch preparation for this run."

Properties:
* Authorises drafting audit-outreach copy as a **draft only**, prominently marked.
* Authorises filling in any remaining BLOCKERS or SHOULD-FIXes from `docs/security-reanchor/AUDIT_READINESS_GAP_ANALYSIS.md`.
* Authorises selecting a target shortlist of audit firms (NAMES ONLY in the operator's private notes; no contact, no outreach sent).
* Does NOT authorise contacting any firm.
* Does NOT authorise signing an SOW.
* Does NOT authorise paying a retainer.

---

## 2. Scope

### 2.1 Close remaining BLOCKERS

Per `docs/security-reanchor/AUDIT_READINESS_GAP_ANALYSIS.md`:

* B-1: Solidity test inventory (`deopt-v2-sol/docs/TEST_INVENTORY.md`).
* B-2: `INVARIANTS.md` in the security-review-packet.
* B-3: `THREAT_MODEL.md` in the security-review-packet.
* B-4: `KNOWN_ISSUES.md` in the security-review-packet.
* B-5: `OUT_OF_SCOPE.md` in the security-review-packet.
* B-6: `AUDIT_REQUEST_OUTLINE.md` (prominently marked NOT INITIATED until this brief approves dispatch).
* B-7: Re-confirm `freeze-manifest.json` against on-chain bytecode (read-only `cast code | sha256sum`).

### 2.2 Close SHOULD-FIXes

S-1 through S-8 per the gap analysis. Each gets either a closure or an explicit accepted-rationale.

### 2.3 Write DOCUMENT-FOR-AUDIT items

D-1 through D-4.

### 2.4 Draft outreach copy (DRAFT ONLY)

Create `docs/security-reanchor/AUDIT_OUTREACH_DRAFT.md`:

* Salutation template.
* Brief project description (public-beta posture, frozen surface).
* Scope-letter sketch (pointer to `OUT_OF_SCOPE.md` + `CROSS_REPO_SCOPE_MATRIX.md`).
* Timeline expectations (placeholder).
* Deliverables expected.
* Budget range (operator-private; not in the published doc).

Prominently marked: **DRAFT — NOT SENT — REQUIRES SEPARATE EXPLICIT APPROVAL BEFORE TRANSMISSION**.

### 2.5 Audit firm shortlist (operator-private)

* Identify a shortlist of audit firms in the operator's private notes.
* Do NOT publish the shortlist.
* Do NOT contact any firm under this milestone's approval.

### 2.6 Pre-dispatch validation pass

Run the per-row checks in `AUDIT_READINESS_GAP_ANALYSIS.md`:
* All BLOCKERS closed.
* All SHOULD-FIXes closed or accepted.
* All DOCUMENT-FOR-AUDIT items written.

Sign-off: operator + Sol maintainer + backend maintainer + frontend maintainer.

---

## 3. Out of scope

* Sending the outreach copy. (Strictly separate later milestone.)
* Negotiating SOW.
* Selecting a firm publicly.
* Paying any retainer.
* Initiating any chain action.
* Mainnet activation.
* Bug-bounty launch.
* Production signer / KMS cutover.

---

## 4. Hard preconditions

| # | Precondition | Verifying check |
|---|---|---|
| P1 | Approval line (§1) present verbatim | grep |
| P2 | Security re-anchor packet exists in full | `ls docs/security-reanchor/{README,PRODUCT_FREEZE_SUMMARY,SECURITY_REANCHOR_OVERVIEW,CROSS_REPO_SCOPE_MATRIX,UPDATED_RISK_REGISTER,TESTNET_EVIDENCE_SUMMARY,AUDIT_READINESS_GAP_ANALYSIS,PRE_AUDIT_ACTION_PLAN,MAINNET_READINESS_GAP_ANALYSIS}.md` |
| P3 | Backend `.env` untouched | `stat -c '%y'` |
| P4 | Private file untouched | `stat -c '%a %y'` |
| P5 | `~/DEOPT/private/**` NOT read (out of scope) | trust |
| P6 | Chain id `84532` (read-only checks only) | `cast chain-id` |
| P7 | NO mainnet RPC URL used | scan |

---

## 5. Forbidden

* No outreach sent. To anyone. Under any pretext.
* No SOW signed.
* No firm name published.
* No mainnet RPC.
* No `.env` edit.
* No source code change to `deopt-v2-sol/src/`, `deopt-v2-backend/src/`, or `deopt-v2-frontend/src/` (docs + ABI verification only).
* No claim "audited" / "mainnet-ready" / "production" / "safe for real funds".

---

## 6. Acceptance criteria

* All 7 BLOCKERS closed (B-1 through B-7).
* All 8 SHOULD-FIXes closed OR documented as accepted.
* All 4 DOCUMENT-FOR-AUDIT items written.
* `AUDIT_OUTREACH_DRAFT.md` exists, prominently marked DRAFT / NOT SENT.
* Operator + maintainer sign-off recorded (a single results doc citing the names).
* `git diff --check` clean across affected repos.
* Sensitive-string scan zero hits on new/edited docs.
* Positive-claim drift scan zero hits.

---

## 7. Cross-links

* `docs/security-reanchor/AUDIT_READINESS_GAP_ANALYSIS.md`
* `docs/security-reanchor/PRE_AUDIT_ACTION_PLAN.md`
* `docs/security-reanchor/CROSS_REPO_SCOPE_MATRIX.md`
* `~/DEOPT/RUN_STATE.md`

**End of external audit dispatch prep next-task brief.**
