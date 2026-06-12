# PRODUCT-FREEZE-AND-SECURITY-REANCHOR — Result

**Date executed:** 2026-06-12
**Operator approval line accepted (verbatim):**
> "I approve DeOpt V2 product freeze and security re-anchor preparation for this run."

**Brief:** `deopt-v2-backend/docs/PRODUCT_FREEZE_AND_SECURITY_REANCHOR_NEXT_TASK.md`.

**Posture:** **Docs only. Zero source code changes. Zero chain transactions. Zero broadcast. Zero mainnet. Zero `.env` edit. Zero private key handling. Zero external communication. Zero audit-firm contact. Zero bug-bounty launch. Zero claim of "audited" / "mainnet-ready" / "production" / "safe for real funds".**

---

## 1. Workspace

* Packet directory: `~/DEOPT/deopt-v2-backend/docs/security-reanchor/` (NEW)
* Next-task briefs: `~/DEOPT/deopt-v2-backend/docs/{EXTERNAL_AUDIT_DISPATCH_PREP,MAINNET_LAUNCH_READINESS,OPERATOR_PUBLIC_BETA_URLS_FILL}_NEXT_TASK.md`
* Result doc: this file
* Closure paragraph: `~/DEOPT/RUN_STATE.md`

## 2. Product freeze inventory (Phase A)

Sourced from cross-repo read of `deopt-v2-sol/`, `deopt-v2-backend/`, `deopt-v2-frontend/`, `~/DEOPT/MAINNET_CUSTODY_POLICY.md`, `~/DEOPT/RUN_STATE.md`, and the 15-file `docs/public-beta/` pack.

**Solidity:** 11 contracts frozen in `abis/freeze-v2-product-rc1/` (tag local-only). Source commit `d133e2c`. ~458 selectors. Storage layout pinned. Status declared as `TESTNET_BETA_ONLY_NOT_AUDITED_NOT_MAINNET_DEPLOYED`. Out-of-scope: `MockPriceSource`, V1 `FeesManager`, perp surface (deferred per `Q-CD-6`).

**Backend:** OpenAPI `0.1.0-mvp` with 13 public paths. Zero admin paths in the public spec. Public intent-creation endpoint live (`POST /options/execution-intents`, NO signer, NO broadcast). Execution gates default to `false`. Reconciliation worker active. ~49 `mod tests` blocks across `src/`.

**Frontend:** Mainnet hard-stop at three layers (`isMainnetEnabled()=false`, `expectedChainId()` refuses mainnet, `signTypedData()` refuses mainnet). Wrong-network blocker wired. Public-beta footer + 6-slot link config. `no-admin-bearer.spec.ts` enforces zero `Authorization` headers from runtime. 30 Playwright tests in 12 files. Build / lint / typecheck clean (re-verified earlier today).

**Public-beta docs:** 15 files (~2,481 lines). Canonical addresses verified: ME `0x5a5EBF9A…`, MarginEngine `0x506cD65a…`, mUSDC `0x6eAe407f…`. Stale legacy ME `0xf2D1D85…` flagged "DO NOT USE" in every contract-address doc. Canonical first-trade tx `0x748c9484…` referenced in 7+ docs.

**Stale doc identification:** `E2E_SEPOLIA_LIVE_BROADCAST_FAILURE_NEXT_TASK.md` + `OPTION_FIRST_BROADCAST_FAILURE_0xe832365b.md` marked as superseded by `E2E_SEPOLIA_LIVE_BROADCAST_RETRY_RESULT.md`. (Marker added in `README.md` of the packet; no edit to the historical docs themselves.)

**No source code changes performed in any repo under this milestone.**

## 3. Security re-anchor matrix (Phase B)

`docs/security-reanchor/SECURITY_REANCHOR_OVERVIEW.md` ships a 16-row matrix (one row per component / invariant) with the columns required by the brief:

1. Contract roles + ownership
2. Executor authorization model
3. Option trade signature model (EIP-712)
4. Nonce model
5. Oracle assumptions
6. Collateral / vault accounting
7. Fee accounting
8. Margin engine authorization
9. Event indexing assumptions
10. Backend shadow / manual intent projection (Sepolia reconciliation)
11. Backend broadcast gates
12. Frontend no-admin-bearer guarantee
13. Public-beta no-real-funds / unaudited messaging
14. Env / secrets handling
15. Production signer / AWS-KMS — explicit OUT OF SCOPE
16. Safe / governance production flows — explicit OUT OF SCOPE

Each row carries: status, security assumption, evidence pointer, gap, severity (I/L/M/H/C), before-audit action, before-mainnet action.

Severity ratings are explicitly operator self-assessment, not audit findings.

## 4. Risk register refresh (Phase C)

`docs/security-reanchor/UPDATED_RISK_REGISTER.md` ships 19 rows (R-1 through R-19):

* R-1 unaudited protocol
* R-2 mock-oracle / testnet-oracle assumptions
* R-3 short oracle maxDelay / stale-price refusal
* R-4 executor key risk
* R-5 owner key risk
* R-6 backend / indexer reconciliation drift
* R-7 manual / shadow intent projection misuse
* R-8 stale deployment address (legacy ME)
* R-9 frontend wrong-network
* R-10 admin bearer leakage
* R-11 API `partial` / `SOURCE_UNAVAILABLE` confusion
* R-12 liquidity / market-maker absence
* R-13 public beta confusion (mistaken-for-mainnet)
* R-14 documentation drift
* R-15 secrets hygiene
* R-16 production signer / KMS not cut over
* R-17 Safe / governance not productionized
* R-18 no bug bounty
* R-19 no external audit yet

Each row: likelihood (L1-L5), impact (I1-I5), mitigations, residual severity, status. Summary table by severity included.

## 5. Testnet evidence summary (Phase E)

`docs/security-reanchor/TESTNET_EVIDENCE_SUMMARY.md`:

* §1 canonical Sepolia trade (`0x748c9484…`, block `42750521`, status `1`, 19 indexed events) — with explicit "what this does NOT prove" subsection.
* §2 backend reconciliation (indexer + reconciliation worker, restart safety with all broadcast gates disabled).
* §3 nonce / balance / position / fee accounting (held for this one trade — explicitly NOT generalised).
* §4 frontend build + test status (clean across typecheck / lint / build; Playwright catalog passes).
* §5 public-beta docs status (15 files; canonical addresses; zero secrets).
* §6 community feedback loop status (infrastructure exists; channels still placeholder).

§7 closes with: "testnet evidence is necessary but not sufficient" for either audit-readiness or mainnet-readiness.

## 6. Audit readiness gap analysis (Phase F)

`docs/security-reanchor/AUDIT_READINESS_GAP_ANALYSIS.md` classifies gaps per the brief:

* **BLOCKER (7):** B-1 Solidity test inventory, B-2 INVARIANTS.md, B-3 THREAT_MODEL.md, B-4 KNOWN_ISSUES.md, B-5 OUT_OF_SCOPE.md, B-6 AUDIT_REQUEST_OUTLINE.md (NOT INITIATED), B-7 re-confirm freeze-manifest vs on-chain bytecode.
* **SHOULD-FIX (8):** S-1..S-8 covering coverage delta, backend test summary, Playwright execution proof, backend-side signature-verification posture doc, indexer reorg policy, storage-layout CI hook, Q-CD-* resolution, flag-flip audit-trail.
* **DOCUMENT-FOR-AUDIT (4):** D-1..D-4 covering envelope semantics rationale, public-vs-admin boundary, public-beta posture rationale, cross-link address registry.
* **POST-AUDIT (6):** implement findings, KMS cutover, Safe deployment, monitoring + IR, bug bounty, mainnet UI variant.
* **POST-MAINNET (3):** operational maturity, MM program, ongoing audit cadence.

Until all 7 BLOCKERS close, **do not contact an audit firm**.

## 7. Mainnet readiness gap analysis (Phase G)

`docs/security-reanchor/MAINNET_READINESS_GAP_ANALYSIS.md` ships the strict 9-gate hard model:

1. External audit complete (or documented decision not to).
2. Production signer / KMS / Safe plan complete.
3. Production monitoring + alerting complete.
4. Incident response runbook complete.
5. Pause / guardian / governance runbooks complete.
6. Deployment plan complete.
7. Liquidity / market-maker plan complete.
8. Legal / compliance / product-risk review complete.
9. Public docs adjusted from testnet to mainnet vocabulary.

Closing reminder: **none alone is sufficient; all 9 must close**.

## 8. Docs created (Phase D + I)

| Path | Action |
|---|---|
| `docs/security-reanchor/README.md` | NEW (entry point + ordered reading) |
| `docs/security-reanchor/PRODUCT_FREEZE_SUMMARY.md` | NEW (§§1–6 — Solidity / backend / frontend / public-beta freeze) |
| `docs/security-reanchor/SECURITY_REANCHOR_OVERVIEW.md` | NEW (16-row review matrix) |
| `docs/security-reanchor/CROSS_REPO_SCOPE_MATRIX.md` | NEW (in-scope vs out-of-scope per repo + §5 auditor scope letter sketch) |
| `docs/security-reanchor/UPDATED_RISK_REGISTER.md` | NEW (R-1..R-19) |
| `docs/security-reanchor/TESTNET_EVIDENCE_SUMMARY.md` | NEW (§§1–7 with "not an audit" disclaimer) |
| `docs/security-reanchor/AUDIT_READINESS_GAP_ANALYSIS.md` | NEW (BLOCKER / SHOULD-FIX / DOCUMENT / POST-AUDIT / POST-MAINNET) |
| `docs/security-reanchor/PRE_AUDIT_ACTION_PLAN.md` | NEW (17 actionable items) |
| `docs/security-reanchor/MAINNET_READINESS_GAP_ANALYSIS.md` | NEW (9 hard gates) |
| `docs/EXTERNAL_AUDIT_DISPATCH_PREP_NEXT_TASK.md` | NEW (next-task brief; literal approval line; DOES NOT engage firm) |
| `docs/MAINNET_LAUNCH_READINESS_NEXT_TASK.md` | NEW (next-task brief; coordination plan; DOES NOT activate mainnet) |
| `docs/OPERATOR_PUBLIC_BETA_URLS_FILL_NEXT_TASK.md` | NEW (next-task brief; URLs substitution; DOES NOT invent URLs) |
| `docs/PRODUCT_FREEZE_AND_SECURITY_REANCHOR_RESULT.md` | NEW (this file) |
| `~/DEOPT/RUN_STATE.md` | EDITED (closure paragraph) |

**Source code:** ZERO changes.
**`.env`:** UNCHANGED (mtime `2026-06-08 16:55:05` preserved).
**Private file:** NOT read, NOT committed.

## 9. RUN_STATE update (Phase J)

Closure paragraph prepended dated 2026-06-12 documenting: 9 new packet docs, 3 new next-task briefs, 1 result doc, 1 RUN_STATE edit, zero source changes, zero chain tx, zero `.env` edits, validations clean, audit dispatch NOT initiated, mainnet NOT activated.

## 10. Validations (Phase K)

| Check | Result |
|---|---|
| `git diff --check` (backend) | clean |
| `git diff --check` (frontend) | clean (frontend untouched this milestone) |
| `git diff --check` (sol) | clean (sol untouched this milestone) |
| Sensitive-string scan (milestone files) | zero hits |
| Mainnet RPC pattern scan | zero hits |
| Positive-claim drift scan ("is audited / production-ready / mainnet-ready / safe for real funds / guaranteed") | zero hits (only negative-framed warnings; intentional) |
| `.env` mtime preserved | YES — `2026-06-08 16:55:05` |
| Private file mode 600 preserved | YES; NOT read; NOT committed |
| Admin bearer in any milestone file | NONE |
| Chain transaction sent | NO |
| Broadcast invoked | NO |
| Mainnet RPC used | NO |
| Real wallet used | NO |
| Source changes (any of 3 repos) | NONE |
| External communication | NONE |
| Audit firm contacted | NO |
| Bug bounty launched | NO |
| KMS / Safe / production-signer cutover initiated | NO |

## 11. Files changed

**Created (docs only):**
* `docs/security-reanchor/README.md`
* `docs/security-reanchor/PRODUCT_FREEZE_SUMMARY.md`
* `docs/security-reanchor/SECURITY_REANCHOR_OVERVIEW.md`
* `docs/security-reanchor/CROSS_REPO_SCOPE_MATRIX.md`
* `docs/security-reanchor/UPDATED_RISK_REGISTER.md`
* `docs/security-reanchor/TESTNET_EVIDENCE_SUMMARY.md`
* `docs/security-reanchor/AUDIT_READINESS_GAP_ANALYSIS.md`
* `docs/security-reanchor/PRE_AUDIT_ACTION_PLAN.md`
* `docs/security-reanchor/MAINNET_READINESS_GAP_ANALYSIS.md`
* `docs/EXTERNAL_AUDIT_DISPATCH_PREP_NEXT_TASK.md`
* `docs/MAINNET_LAUNCH_READINESS_NEXT_TASK.md`
* `docs/OPERATOR_PUBLIC_BETA_URLS_FILL_NEXT_TASK.md`
* `docs/PRODUCT_FREEZE_AND_SECURITY_REANCHOR_RESULT.md`

**Edited:**
* `~/DEOPT/RUN_STATE.md`

**Untouched:**
* Backend Rust source (zero changes).
* Solidity source (zero changes).
* Frontend source (zero changes).
* Backend `.env` (mtime preserved).
* `~/DEOPT/private/**` (NOT read, NOT committed).
* Any chain.
* Any external party.

## 12. Remaining audit blockers

Per `docs/security-reanchor/AUDIT_READINESS_GAP_ANALYSIS.md`:

* **7 BLOCKERS** open: B-1 (Solidity test inventory), B-2 (INVARIANTS.md), B-3 (THREAT_MODEL.md), B-4 (KNOWN_ISSUES.md), B-5 (OUT_OF_SCOPE.md), B-6 (AUDIT_REQUEST_OUTLINE.md), B-7 (re-confirm freeze-manifest vs on-chain bytecode).
* **8 SHOULD-FIX** open: S-1..S-8.
* **4 DOCUMENT-FOR-AUDIT** open: D-1..D-4.
* Until all 7 BLOCKERS close, **do not contact any audit firm**.

## 13. Remaining mainnet blockers

Per `docs/security-reanchor/MAINNET_READINESS_GAP_ANALYSIS.md`:

* **All 9 gates open.**
* Mainnet activation is deliberately disabled and remains so. `isMainnetEnabled()` in `chains.ts` is hard-coded `false`.
* `MAINNET_LAUNCH_READINESS_NEXT_TASK.md` is the coordination brief that structures the closure of these 9 gates as a multi-milestone arc. It does NOT activate mainnet.

## 14. Next milestone recommendation

**Primary:** `EXTERNAL_AUDIT_DISPATCH_PREP` — close the 7 BLOCKERS + 8 SHOULD-FIXes + 4 DOCUMENT items, draft the outreach copy as **DRAFT / NOT SENT**, identify a private firm shortlist. Approval line: "I approve DeOpt V2 external audit dispatch preparation for this run."

**Alternative if operator has community channel URLs ready:** `OPERATOR_PUBLIC_BETA_URLS_FILL` — quick win (≤ 1 hour for the operator). Does NOT block the audit-prep arc. Approval line: "I approve DeOpt V2 operator public beta URLs fill for this run."

**Strictly later (NOT NOW):** `MAINNET_LAUNCH_READINESS` (the coordination brief itself), and only after the 7 BLOCKERS close + audit dispatch + audit closure happen. Approval line: "I approve DeOpt V2 mainnet launch readiness planning for this run."

**Explicitly NOT recommended now:**
* Engaging an audit firm.
* Launching a bug-bounty program.
* Touching mainnet RPC.
* Cutting over the production signer / KMS.
* Migrating governance to Safe.
* Removing testnet banners from the frontend.
* Flipping `isMainnetEnabled()`.

Milestone outcome: a 9-doc security-review preparation packet under `docs/security-reanchor/` plus 3 follow-up briefs — all docs-only, zero source / chain / `.env` / private activity, audit dispatch deliberately deferred, mainnet deliberately deferred.

**End of PRODUCT-FREEZE-AND-SECURITY-REANCHOR result.**
