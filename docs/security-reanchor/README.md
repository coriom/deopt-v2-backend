# DeOpt V2 — Security Re-Anchor Packet

> **Public testnet beta only. Base Sepolia (chain 84532). No real funds. Unaudited. Not mainnet-ready.**
>
> This is a **security-review preparation** packet. It is NOT itself an audit. It is the bundle that defines what an external auditor would see if one were engaged.

**Date written:** 2026-06-12
**Source milestone:** `PRODUCT_FREEZE_AND_SECURITY_REANCHOR_NEXT_TASK.md`
**Approval line consumed (verbatim):** "I approve DeOpt V2 product freeze and security re-anchor preparation for this run."

---

## What this packet is

A consolidated, cross-repo snapshot of DeOpt V2 at the public testnet beta moment, written so an external auditor (or an internal reviewer) can pick it up cold and have a defensible mental model of:

* What is **frozen** (ABI, public API, frontend gates, public-beta posture).
* What is **in scope** for any future review.
* What is **out of scope** (mainnet, AWS/KMS production signer cutover, Safe-tx multisig flows, bug bounty, perp surface).
* What **changed** since the earlier `MAINNET_AUDIT_*` and `MAINNET_CUSTODY_*` working docs.
* What **risks remain** at this snapshot.
* What must be done **before** an external audit dispatch.
* What must be done **before** any mainnet activation.

It deliberately does NOT:

* Claim DeOpt V2 is audited.
* Claim DeOpt V2 is mainnet-ready.
* Claim safety for real funds.
* Initiate any external audit engagement.
* Initiate any bug-bounty program.
* Modify any source code.
* Send any chain transaction.
* Touch mainnet RPC or `.env`.

---

## Documents in this packet

Read in this order:

| # | Doc | Purpose |
|---|---|---|
| 1 | [PRODUCT_FREEZE_SUMMARY.md](./PRODUCT_FREEZE_SUMMARY.md) | What is frozen (Solidity ABI, backend OpenAPI, frontend public-beta links, public-beta docs pack). |
| 2 | [SECURITY_REANCHOR_OVERVIEW.md](./SECURITY_REANCHOR_OVERVIEW.md) | Cross-repo security review matrix — component-by-component status, assumption, evidence, gap, severity, before-audit action, before-mainnet action. |
| 3 | [CROSS_REPO_SCOPE_MATRIX.md](./CROSS_REPO_SCOPE_MATRIX.md) | What is in scope vs out of scope for a future audit, broken down per repo (sol / backend / frontend / project root). |
| 4 | [UPDATED_RISK_REGISTER.md](./UPDATED_RISK_REGISTER.md) | Refreshed risk register at the testnet-beta moment (19 risks with severity, likelihood, mitigation, residual). |
| 5 | [TESTNET_EVIDENCE_SUMMARY.md](./TESTNET_EVIDENCE_SUMMARY.md) | What we can prove from the testnet (canonical Sepolia trade, reconciliation, build / lint / typecheck, public-beta docs). Explicit "not an audit" disclaimer. |
| 6 | [AUDIT_READINESS_GAP_ANALYSIS.md](./AUDIT_READINESS_GAP_ANALYSIS.md) | Gaps classified as BLOCKER / SHOULD-FIX / DOCUMENT-FOR-AUDIT / POST-AUDIT / POST-MAINNET. |
| 7 | [PRE_AUDIT_ACTION_PLAN.md](./PRE_AUDIT_ACTION_PLAN.md) | Concrete action list before an external audit could be dispatched (NOT an outreach plan; that is a separate later milestone). |
| 8 | [MAINNET_READINESS_GAP_ANALYSIS.md](./MAINNET_READINESS_GAP_ANALYSIS.md) | Strict list of mainnet-blocking gaps (audit + custody + monitoring + IR + governance + legal). |

---

## What this packet supersedes

This packet **does not delete** prior docs. It re-anchors them at 2026-06-12. The earlier audit-prep working docs (`MAINNET_AUDIT_*`, `MAINNET_CUSTODY_POLICY.md`, `MAINNET_CUSTODY_DECISIONS_ADDENDUM_TEMPLATE.md`) are reference material; the per-doc relationships are spelled out in `CROSS_REPO_SCOPE_MATRIX.md` and `MAINNET_READINESS_GAP_ANALYSIS.md`.

This packet **does** mark the following docs as superseded for **product-freeze** purposes (they remain valid as historical record):

* `E2E_SEPOLIA_LIVE_BROADCAST_FAILURE_NEXT_TASK.md` — superseded by `E2E_SEPOLIA_LIVE_BROADCAST_RETRY_RESULT.md` (retry succeeded; tx `0x748c9484…`).
* `OPTION_FIRST_BROADCAST_FAILURE_0xe832365b.md` — superseded by the canonical successful trade.

Stale Solidity addresses called out throughout:

* Legacy `OptionMatchingEngine 0xf2D1D85cD363Be3bc160d14883C80e7C2c4F420b` — **DO NOT USE**. Canonical is `0x5a5EBF9A9CCd7c012518569DE8283982982670f6`.
* (Note: there is no separate `0x287…` MarginEngine referenced in the current freeze state; the canonical MarginEngine is `0x506cD65a63C53c66ab572B9f9dd819B7BfE00D30`. If a `0x287…` address appears in older docs it predates the M-P5 retarget and is historical.)

---

## Audience

* **Internal reviewers** (operator team, near-term).
* **External auditors** (future engagement, NOT yet contracted).
* **Community / public** — selected docs in this packet are public-safe and can be linked from `docs/public-beta/`. All docs explicitly forbid private values (`.env`, RPC URL with key, admin bearer, private keys).

---

## Out of scope for this packet (explicit)

* Mainnet activation.
* External audit engagement (firm selection, scope letter, signed SOW).
* Bug bounty launch.
* Safe-tx multisig production flow.
* AWS / KMS production signer cutover.
* Perp surface (deferred per `Q-CD-6` in `MAINNET_CUSTODY_POLICY.md`).
* Production monitoring / alerting / incident response.
* Legal / compliance / regulatory review.

Each of the above is acknowledged with its own follow-up brief:

* `deopt-v2-backend/docs/EXTERNAL_AUDIT_DISPATCH_PREP_NEXT_TASK.md` — preparation only; does NOT engage a firm.
* `deopt-v2-backend/docs/MAINNET_LAUNCH_READINESS_NEXT_TASK.md` — strict gates for any future mainnet path.

---

**End of security re-anchor packet README.**
