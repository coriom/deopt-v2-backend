# DeOpt V2 — Audit Readiness Gap Analysis

> **Snapshot date:** 2026-06-12. **Posture:** public testnet beta, Base Sepolia only, unaudited, not mainnet-ready.

Strict classification of gaps for an external audit engagement. **This document does NOT engage an auditor and does NOT claim audit-readiness.** It identifies what must be done first.

## Classification

| Tag | Meaning |
|---|---|
| **BLOCKER** | Must be closed before an audit dispatch is even considered. |
| **SHOULD-FIX** | Strongly recommended before dispatch. Auditor will flag it on day 1 if not. |
| **DOCUMENT-FOR-AUDIT** | Existing posture is fine; the gap is documentation. Write it for the handoff. |
| **POST-AUDIT** | Will be addressed after the audit report lands. Not blocking dispatch. |
| **POST-MAINNET** | Production-only concern. Not blocking either dispatch or report acceptance. |

---

## 1. BLOCKERS (close before dispatch)

### B-1 — Solidity test inventory

* **Gap:** No `forge test` files located under `deopt-v2-sol/test/`. Either the tests live elsewhere (separate directory / external repo) or they are inline. Reviewer cannot evaluate.
* **Required:** Publish a one-pager that says exactly where the Solidity test suite lives, how to run it, and approximate coverage by contract.
* **Why blocker:** The first thing an auditor asks for is the test inventory. Not having one signals immature engineering and triggers an immediate scoping cost.

### B-2 — Invariant documentation (`INVARIANTS.md`)

* **Gap:** Nonce monotonicity, vault accounting, fee accounting, position zero-sum, oracle freshness gate — all asserted by code but not enumerated in one doc.
* **Required:** `deopt-v2-sol/docs/security-review-packet/INVARIANTS.md` (sketched in `PRE_AUDIT_ACTION_PLAN.md` item 2) with one row per invariant: statement, where it's enforced, evidence (test or assertion).
* **Why blocker:** Auditors need a target list. Without it they'll write their own — at higher cost and with less of the author's context.

### B-3 — Threat-model write-up (`THREAT_MODEL.md`)

* **Gap:** Actors, trust assumptions, attack surface enumeration — implicit in the existing codebase + docs but not consolidated.
* **Required:** `deopt-v2-sol/docs/security-review-packet/THREAT_MODEL.md` per the `PRODUCT_FREEZE_AND_SECURITY_REANCHOR_NEXT_TASK.md` brief §3.
* **Why blocker:** Same as B-2; auditors need actors-and-trust-assumptions in writing.

### B-4 — `KNOWN_ISSUES.md`

* **Gap:** Legacy stale ME `0xf2D1D85…`, mock-oracle 60s maxDelay, backend-side signature-verification disabled (chain-side still enforced) — these are documented across multiple docs but not in one place.
* **Required:** Consolidated known-issues doc in the security packet. Auditor reads this first and can pre-deduct from scope estimation.
* **Why blocker:** Auditors charge for "discovery"; consolidating known issues turns discovery into confirmation.

### B-5 — `OUT_OF_SCOPE.md` (audit-facing)

* **Gap:** This packet enumerates out-of-scope items in `CROSS_REPO_SCOPE_MATRIX.md`, but an auditor-facing version needs the same content phrased as a scope letter constraint: "DO NOT review AWS KMS adapter; not deployed for production."
* **Required:** `deopt-v2-sol/docs/security-review-packet/OUT_OF_SCOPE.md` per brief §3.
* **Why blocker:** Without an explicit out-of-scope list, auditors will reasonably try to scope these in.

### B-6 — `AUDIT_REQUEST_OUTLINE.md` (placeholder, prominently marked)

* **Gap:** Brief §3 calls for this. The outline is a scope-letter sketch (target scope, timeline, deliverables) marked **NOT INITIATED** so it cannot be mistaken for an active engagement.
* **Required:** Drafted, prominently marked NOT INITIATED.
* **Why blocker:** Forces the team to articulate what an audit would cover before contacting any firm — improves the outreach later.

### B-7 — Re-confirm `freeze-manifest.json` against on-chain bytecode

* **Gap:** `MAINNET_CUSTODY_POLICY.md §1` (in the next-task brief) calls for `cast code ... | sha256sum` against the retargeted addresses to confirm bytecode matches the frozen artefact.
* **Required:** Read-only chain check + drift report. If drift detected: either re-freeze to `freeze-v2-product-rc2` OR document the drift.
* **Why blocker:** If on-chain bytecode does not match the frozen artefact, the entire packet is built on a false premise.

---

## 2. SHOULD-FIX (highly recommended before dispatch)

### S-1 — Solidity coverage delta

* **Gap:** Even with a test inventory, the auditor will ask "what's covered, what isn't." Empty cells in coverage drive scope.
* **Recommendation:** Publish a coverage table (per contract → branch / function coverage).
* **If not done:** Auditor will request it post-dispatch and the engagement clock has started.

### S-2 — Backend test inventory + coverage

* **Gap:** ~49 `mod tests` declarations across `deopt-v2-backend/src/`. No public-facing summary of what they cover.
* **Recommendation:** A one-page backend test summary, grouped by domain (api / executor / indexer / reconciliation / configuration).

### S-3 — Frontend test execution proof

* **Gap:** Playwright catalog passes `--list`; targeted run not executed in this sandbox.
* **Recommendation:** CI run of the full Playwright suite on a Linux box with `libnspr4` available. Archive the result HTML / JSON.

### S-4 — Backend-side signature verification posture doc

* **Gap:** Backend-side EIP-712 verification is currently disabled; chain-side verification still enforced. This is intentional but auditors will not know.
* **Recommendation:** A one-page doc explaining why, what the risk is (none — chain-side is the authoritative verifier), and when it will be enabled (separate later milestone).

### S-5 — Indexer reorg-handling policy

* **Gap:** Confirmation depth + reorg recovery contract not written down.
* **Recommendation:** A short policy doc: "indexer treats block - N as final; on a reorg deeper than N, the indexer rewinds to the divergence point and re-processes."

### S-6 — Storage-layout CI diff hook

* **Gap:** `storage-layouts.txt` snapshot exists; no CI hook diffs it on PR.
* **Recommendation:** Add a CI step that runs `forge inspect <Contract> storageLayout` and diffs against the snapshot. PR fails on drift.

### S-7 — Resolve open `Q-CD-*` decisions

* **Gap:** `Q-CD-2` (OPS multisig threshold), `Q-CD-5` (KMS vendor), `Q-CD-6` (perp scope at launch) all open per `MAINNET_CUSTODY_POLICY.md`.
* **Recommendation:** Resolve each with either a decision or an explicit deferral-with-date. Auditors will ask.

### S-8 — Operator runbook for `EXECUTOR_REAL_BROADCAST_ENABLED` flag flips

* **Gap:** Flag flip is currently a manual operator action with a manual log. Audit-friendly version requires written-down approval flow.
* **Recommendation:** Update `BACKEND_LIVE_BROADCAST_FLAG_FLIP_RUNBOOK_V2G_FX_Q1_C.md` with an audit-trail requirement.

---

## 3. DOCUMENT-FOR-AUDIT (write it down; underlying is fine)

### D-1 — Status envelope semantics rationale

* **Gap:** The `ok / partial / stale` envelope semantics are coded but not articulated in security terms.
* **Required:** A short doc explaining why `partial` is the correct response when an optional source is missing (e.g. oracle), and why `stale` is a separate code from `partial`.

### D-2 — Public vs admin API boundary

* **Gap:** Boundary is enforced but not enumerated in one place.
* **Required:** A short doc that for each route says "public" or "admin" + the gate + how the gate fails closed.

### D-3 — Public-beta posture rationale

* **Gap:** Why we're so loud about "unaudited / no real funds / not mainnet-ready". Obvious to insiders; not to auditors who joined the engagement late.
* **Required:** A one-pager (this packet's `README.md` partly covers it).

### D-4 — Cross-repo address registry

* **Gap:** Addresses appear in many docs; auditor needs one source of truth.
* **Required:** Already partially in `CONTRACT_ADDRESSES_BASE_SEPOLIA.md`. Cross-link from the security packet.

---

## 4. POST-AUDIT (after report lands; before mainnet)

### P-1 — Implement audit findings

* The audit report will likely have findings. Each one needs a written response (fix, accept, document). Tracked under POST-AUDIT.

### P-2 — Production signer / AWS-KMS cutover

* `BACKEND_SIGNER_CUTOVER_RUNBOOK_V2G_FX_Q1.md` is the runbook. Cutover happens after audit closure (so the audit can pass through the production path).

### P-3 — Safe multisig + timelock deployment

* Per `MAINNET_CUSTODY_POLICY.md §R-1..R-9`. Happens before mainnet contracts have any owner control.

### P-4 — Production monitoring + alerting + IR runbooks

* `BACKEND_EXECUTOR_MONITORING_ALERTS_V1.md` (signer integrity, executor health, flag-flip audit log) deployed.
* IR runbooks: signer compromise, oracle compromise, indexer poisoning, vault drain attempt.

### P-5 — Bug-bounty program launch

* Rules drafted (separately later). Funded. Public.

### P-6 — Mainnet UI variant

* Change the public-beta vocabulary to mainnet vocabulary. Removes the banners. Adds risk disclosures appropriate to real funds. Happens only after all of the above.

---

## 5. POST-MAINNET (not blocking audit; ongoing thereafter)

### M-1 — Operational maturity

* On-call rota. SLOs. Postmortem cadence.

### M-2 — Liquidity / market-maker program

* Not a security concern; a product concern. Tracked separately.

### M-3 — Continuous audit / engagement cadence

* If economic activity warrants it, ongoing audit retainer + recurring reviews.

---

## 6. Summary

| Tag | Open items |
|---|---|
| **BLOCKER** | 7 (B-1..B-7) |
| **SHOULD-FIX** | 8 (S-1..S-8) |
| **DOCUMENT-FOR-AUDIT** | 4 (D-1..D-4) |
| **POST-AUDIT** | 6 (P-1..P-6) |
| **POST-MAINNET** | 3 (M-1..M-3) |

Until all 7 BLOCKERS close, **do not contact an audit firm**.

The `PRE_AUDIT_ACTION_PLAN.md` doc in this packet is the actionable list to close them.

---

**End of audit readiness gap analysis.**
