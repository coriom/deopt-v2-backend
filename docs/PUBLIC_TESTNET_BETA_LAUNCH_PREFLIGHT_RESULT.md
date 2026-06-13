# PUBLIC-TESTNET-BETA-LAUNCH-PREFLIGHT — Result

**Date executed:** 2026-06-13
**Operator approval line accepted (verbatim):**
> "I approve DeOpt V2 public testnet beta launch preflight for this run."

**Posture:** **Docs + verification only. Zero chain transactions. Zero broadcast. Zero mainnet. Zero backend `.env` edit. Zero private key handling. Zero audit outreach. Zero bug bounty. Zero announcement publication. Zero claim of "audited" / "mainnet-ready" / "production" / "safe for real funds".**

---

## 1. TL;DR — Launch readiness verdict

# **NOT READY**

Reasons (per the brief's hard rules):

* **App URL is missing.** Brief: "Do not mark launch ready if app URL is missing."
* **Feedback URL AND GitHub URL are both placeholder.** Brief: "Do not mark launch ready if feedback and GitHub/issues are both missing."

Discord is live (`https://discord.gg/zaEMvWuxu`). Frontend build is green. Public docs pack is complete and address-fresh. Security re-anchor packet is in place. The non-URL parts of the launch posture are ready; only the operator-side channel URLs gate the verdict.

Remaining-actions doc with the exact operator workflow: `docs/PUBLIC_TESTNET_BETA_LAUNCH_REMAINING_ACTIONS.md`.

---

## 2. Workspace

* `~/DEOPT/deopt-v2-frontend/` (smoke-checked only — no edits)
* `~/DEOPT/deopt-v2-backend/docs/` (3 new + 2 edited)
* `~/DEOPT/RUN_STATE.md` (closure paragraph)

## 3. Launch readiness inventory (Phase A)

### 3.1 Frontend app readiness

| Item | Status |
|---|---|
| `npm run typecheck` | clean |
| `npm run lint` | clean |
| `npm run build` (`next build`) | green, 9 routes prerendered |
| `npx playwright test --list` | 63 tests in 20 files, parse-clean |
| Landing route `/` | ✓ exists |
| `/markets` + `/markets/[productId]` | ✓ exist |
| `/portfolio`, `/history`, `/health`, `/transactions/[requestId]` | ✓ exist |
| `/admin` (operator-only) | ✓ exists; not part of public flow |
| Mainnet route | NONE (correctly absent) |
| `isMainnetEnabled()` | hard-coded `false` |
| `TestnetUnauditedBanner` + `MainnetDisabledBanner` + `WrongNetworkBanner` | sticky on every trading route |
| `PublicBetaFooter` | sticky on every trading route |
| `TestnetReadinessHelper` | live on every product page |
| Brand identity (black + deep-green) | applied |
| Header logo aligned to favicon | ✓ |

### 3.2 Frontend tests / build status

All four pre-launch smoke commands pass cleanly:

```text
npm run typecheck   → tsc --noEmit, no output (success)
npm run lint        → eslint, no output (success)
npm run build       → next build, green, 9 routes prerendered
npx playwright test --list → Total: 63 tests in 20 files
```

Targeted Playwright spec execution not performed in this sandbox (WSL2 image missing `libnspr4.so`; CI/Linux unaffected — same constraint documented in every prior frontend milestone).

### 3.3 Public-beta docs readiness

15 docs in `docs/public-beta/`:

* PUBLIC_TESTNET_BETA_OVERVIEW.md ✓
* BASE_SEPOLIA_QUICKSTART.md ✓
* USER_TESTING_GUIDE.md ✓
* KNOWN_LIMITATIONS_AND_RISKS.md ✓
* FEEDBACK_AND_BUG_REPORTING.md ✓
* BUG_REPORT_TEMPLATE.md ✓
* COMMUNITY_ONBOARDING.md ✓
* PUBLIC_TESTNET_BETA_LAUNCH_CHECKLIST.md ✓
* PUBLIC_TESTNET_BETA_ANNOUNCEMENT_DRAFT.md ✓
* PUBLIC_TESTNET_BETA_ANNOUNCEMENT_FINAL_DRAFT.md ✓ (NEW in this milestone)
* OPERATOR_PUBLIC_BETA_URLS_FILL.md ✓
* README.md ✓
* FAQ.md ✓
* DEVELOPER_API_GUIDE.md ✓
* CONTRACT_ADDRESSES_BASE_SEPOLIA.md ✓
* FEEDBACK_TRIAGE_WORKFLOW.md ✓

Address freshness:
* Canonical `OptionMatchingEngine 0x5a5EBF9A…` present in 5+ docs.
* Canonical `MarginEngine 0x506cD65a…` present in 3 docs.
* Stale `OptionMatchingEngine 0xf2D1D85…` referenced in 4 docs with explicit "DO NOT USE" / "legacy" / "stale" wording — correctly historical, not presented as current.
* Stale `MarginEngine 0x287Cef47…` referenced in `CONTRACT_ADDRESSES_BASE_SEPOLIA.md:26` with "legacy margin engine … NOT the canonical pair. Do not use them for new trades." — correctly historical.
* Base Sepolia chain id `84532` referenced in 10+ docs.

Disclaimer coverage:
* `testnet only` / `unaudited` / `no real funds` / `not mainnet-ready` / `experimental` — present across all 15 docs.
* Positive-claim drift in docs: only NEGATIVE-framed mentions (lists of what NOT to claim, hold-reasons, question-answered-no). Zero true positive claims.

### 3.4 URL placeholders status (Phase B URL gate)

| Slot | Frontend token | Doc-side token | Status | Required for launch? |
|---|---|---|---|---|
| App URL | (none — the frontend IS the app) | `{{APP_URL}}` | **PLACEHOLDER** | **YES (blocker)** |
| Docs URL | (covered by per-doc tokens below) | — | varies | RECOMMENDED |
| Quickstart | `PUBLIC_BETA_QUICKSTART_URL` | — | PLACEHOLDER | RECOMMENDED |
| Testing Guide | `PUBLIC_BETA_TESTING_GUIDE_URL` | — | PLACEHOLDER | RECOMMENDED |
| Limitations | `PUBLIC_BETA_LIMITATIONS_URL` | — | PLACEHOLDER | RECOMMENDED |
| Feedback Form | `PUBLIC_BETA_FEEDBACK_URL` | `{{ FEEDBACK_FORM_URL }}` | **PLACEHOLDER** | **YES (jointly with GitHub) — blocker** |
| GitHub | `PUBLIC_BETA_GITHUB_URL` | `{{ GITHUB_REPO_URL }}` | **PLACEHOLDER** | **YES (jointly with Feedback) — blocker** |
| **Discord** | `PUBLIC_BETA_DISCORD_URL` | `{{ DISCORD_INVITE_URL }}` | **LIVE** — `https://discord.gg/zaEMvWuxu` | ✓ |
| API base URL | (frontend bundles its own via `NEXT_PUBLIC_TRADING_API_BASE_URL`) | `{{ API_BASE_URL }}` | PLACEHOLDER | NOT_REQUIRED_FOR_LAUNCH |
| Status page | (none) | — | MISSING | NOT_REQUIRED_FOR_LAUNCH |

### 3.5 Discord / community status

* **Discord LIVE** at `https://discord.gg/zaEMvWuxu` (wired in `FRONTEND-TESTNET-PRODUCT-V2-DA-FOLLOWUP` 2026-06-12).
* `tests/e2e/brand-identity.spec.ts` asserts the live anchor + safe href (no Bearer / RPC URL / DB credential).
* `tests/e2e/markets-fallback.spec.ts` asserts the live link in the markets-fallback card.
* `tests/e2e/testnet-readiness-helper.spec.ts` asserts the live link in the readiness helper.
* `FEEDBACK_TRIAGE_WORKFLOW.md §1` lists Discord as a continuous best-effort intake channel.

### 3.6 Feedback channel status

* **Placeholder.** Brief explicitly forbids launch when both feedback + GitHub are missing.

### 3.7 GitHub / issues status

* **Placeholder.** Same blocking rule.

### 3.8 App / docs / backend public hosting

* **App URL**: PLACEHOLDER (`{{APP_URL}}`). Not discoverable from the repo.
* **Docs hosting**: PLACEHOLDER. Once GitHub is live, the per-doc tokens can point at `<github>/blob/main/deopt-v2-backend/docs/public-beta/<doc>.md` — that satisfies the docs-hosting requirement without a separate docs host.
* **Backend public API**: PLACEHOLDER (`{{ API_BASE_URL }}`). Frontend bundles its own backend URL via `NEXT_PUBLIC_TRADING_API_BASE_URL` env at build time, so the public API URL is NOT required for the bundled-app announcement.

### 3.9 Known limitations + security disclaimers + mainnet-disabled + audit status

* `KNOWN_LIMITATIONS_AND_RISKS.md` published with 14 sections.
* Disclaimers (testnet / unaudited / no real funds / not mainnet-ready) present in every public doc + every public frontend banner + every announcement draft.
* `MainnetDisabledBanner` triggers on chain id 8453; `isMainnetEnabled()` hard-coded `false`.
* Audit: NOT STARTED. Documented in `docs/security-reanchor/AUDIT_READINESS_GAP_ANALYSIS.md`. 7 BLOCKERS open. NOT a launch blocker for testnet beta (it IS a mainnet blocker).

## 4. URL gate check (Phase B)

Detailed gate verdict per slot is in §3.4 above. Summary:

| Verdict tag | Slots in this state |
|---|---|
| LIVE | Discord |
| PLACEHOLDER | quickstart, testing-guide, limitations, **feedback**, **github**, app, API base |
| COMING_SOON | (none explicitly set) |
| LOCAL_DEV_ONLY | (none) |
| MISSING | status page |
| NOT_REQUIRED_FOR_LAUNCH | API base, status page |

Three blockers (app URL, feedback URL, GitHub URL) → **NOT READY**.

## 5. Frontend launch smoke (Phase D)

| Command | Result |
|---|---|
| `npm run typecheck` | clean |
| `npm run lint` | clean |
| `npm run build` | green, 9 routes prerendered |
| `npx playwright test --list` | 63 tests in 20 files |

Route + safety inspection (per brief Phase D):
* Landing page route ✓
* Trading routes ✓ (`/markets` + `/markets/[productId]`)
* Portfolio route ✓
* Wrong-network guard ✓ (`WrongNetworkBanner` wired in `(trading)/layout.tsx`)
* `isMainnetEnabled()` ✓ hard-coded false
* Public beta footer ✓
* Admin bearer scan: zero hits in `src/` + `tests/`
* RPC URL with key scan: zero hits
* DATABASE_URL scan: zero hits
* Positive-claim drift scan on `src/`: zero hits (only negative-framed and meta-references elsewhere)
* Amber/yellow class scan on public-facing src: zero hits

## 6. Public docs smoke (Phase E)

Per brief checks:
* `testnet only` ✓ (every public-beta doc)
* `unaudited` ✓
* `no real funds` ✓
* `not mainnet-ready` ✓
* `quickstart` ✓
* `user testing guide` ✓
* `known limitations` ✓
* `feedback instructions` ✓
* `bug report template` ✓
* `community onboarding` ✓
* `launch checklist` ✓

Stale address drift:
* Canonical ME `0x5a5EBF9A…` is the current address in every contract-address doc.
* Canonical MarginEngine `0x506cD65a…` is the current address.
* Stale ME `0xf2D1…` flagged in 4 docs with "legacy / stale / DO NOT USE" wording.
* Stale MarginEngine `0x287…` flagged in `CONTRACT_ADDRESSES_BASE_SEPOLIA.md` with "NOT the canonical pair. Do not use them for new trades."
* Base Sepolia chain id `84532` (NOT mainnet `8453`) present in 10+ docs.

## 7. Announcement readiness (Phase F)

Written:
* `docs/public-beta/PUBLIC_TESTNET_BETA_ANNOUNCEMENT_FINAL_DRAFT.md` (NEW) — publish-ready copy for Discord, X single + thread, LinkedIn, GitHub README banner, and pause/rollback template. Prominently marked **"DO NOT POST"** until the §0 publish-gate checklist passes. Required publication approval line documented but NOT to be consumed under this milestone:
   > "I approve DeOpt V2 public testnet beta launch publication for this run."

Honesty-checklist + sensitive-string post-check + versioning + sign-off sections included in the draft.

## 8. Launch checklist update (Phase C)

`docs/public-beta/PUBLIC_TESTNET_BETA_LAUNCH_CHECKLIST.md` gained a new top-level §0 "Current launch readiness verdict" with:
* Verdict: NOT READY.
* Blocker table (App URL / Feedback / GitHub / Discord / API base / Status page) with status + owner action + required-before-announcement + required-before-wider-beta columns.
* Path to flip the verdict (re-run `OPERATOR-PUBLIC-BETA-URLS-FILL` then re-run this preflight).
* Note that soft launch (Discord-only) does NOT require the verdict to flip.

## 9. Launch execution next-task (Phase G)

**NOT created in this milestone.** Per brief: "If launch readiness is NOT READY: Create PUBLIC_TESTNET_BETA_LAUNCH_REMAINING_ACTIONS.md."

* CREATED `docs/PUBLIC_TESTNET_BETA_LAUNCH_REMAINING_ACTIONS.md` with §1 "why we're NOT READY", §2 the 3 blocker actions, §2.4 recommended docs-hosting action, §2.5 not-blocking API base URL, §3 exact operator workflow to flip the verdict, §4-7 safety + risk-if-launched-now + partial-launch options.
* NOT created `docs/PUBLIC_TESTNET_BETA_LAUNCH_NEXT_TASK.md` — that doc will be created in a subsequent run of THIS preflight milestone, once the verdict flips to READY.

## 10. Docs created / updated (Phase H)

| Path | Action |
|---|---|
| `docs/PUBLIC_TESTNET_BETA_LAUNCH_PREFLIGHT_RESULT.md` | NEW (this doc) |
| `docs/PUBLIC_TESTNET_BETA_LAUNCH_REMAINING_ACTIONS.md` | NEW (operator workflow to flip the verdict) |
| `docs/public-beta/PUBLIC_TESTNET_BETA_ANNOUNCEMENT_FINAL_DRAFT.md` | NEW (publish-ready copy; not yet published) |
| `docs/public-beta/PUBLIC_TESTNET_BETA_LAUNCH_CHECKLIST.md` | UPDATED (new §0 "Current launch readiness verdict") |
| `~/DEOPT/RUN_STATE.md` | UPDATED (closure paragraph) |

## 11. RUN_STATE update (Phase I)

Closure paragraph prepended dated 2026-06-13. Documents: launch-readiness verdict NOT READY, frontend smoke clean, docs smoke clean, 3 URL blockers identified (App + Feedback + GitHub), Discord LIVE, announcement final draft written but NOT published.

## 12. Files changed

**Created (docs):**
* `deopt-v2-backend/docs/PUBLIC_TESTNET_BETA_LAUNCH_PREFLIGHT_RESULT.md`
* `deopt-v2-backend/docs/PUBLIC_TESTNET_BETA_LAUNCH_REMAINING_ACTIONS.md`
* `deopt-v2-backend/docs/public-beta/PUBLIC_TESTNET_BETA_ANNOUNCEMENT_FINAL_DRAFT.md`

**Edited (docs):**
* `deopt-v2-backend/docs/public-beta/PUBLIC_TESTNET_BETA_LAUNCH_CHECKLIST.md` (new §0 verdict block)
* `~/DEOPT/RUN_STATE.md` (closure paragraph)

**Source code:** ZERO frontend changes. ZERO backend Rust changes. ZERO Solidity changes.

**Untouched:**
* `deopt-v2-frontend/src/` — ZERO edits this milestone (smoke verification only).
* `deopt-v2-frontend/tests/` — ZERO edits this milestone.
* `deopt-v2-backend/.env` — UNCHANGED (mtime preserved).
* `~/DEOPT/private/**` — NOT read, NOT committed.
* `deopt-v2-sol/` — ZERO changes.

## 13. Validations (Phase J)

| Check | Result |
|---|---|
| `git diff --check` (frontend) | clean |
| `git diff --check` (backend) | clean |
| `git diff --check` (sol) | clean |
| Sensitive-string scan on changed docs | zero hits |
| Mainnet RPC pattern scan on changed docs + frontend src | zero hits |
| Positive-claim drift scan on changed docs | zero true hits (only NEGATIVE-framed checklist items and meta-references) |
| Admin bearer in frontend code/tests | NONE |
| Private RPC URL in frontend code/tests | NONE |
| DATABASE_URL printed | NO |
| `.env` mtime preserved | YES — `2026-06-08 16:55:05` |
| Private file mode 600 preserved | YES; NOT read; NOT committed |
| Chain transaction sent | NO |
| Broadcast invoked | NO |
| Mainnet RPC used | NO |
| Source changes outside docs / RUN_STATE | NONE |
| Backend Rust source changes | NONE |
| Solidity source changes | NONE |
| Frontend src changes | NONE (this milestone is verification-only) |
| Announcement published | NO (final draft written, prominently marked NOT YET PUBLISHED) |
| Audit firm contacted | NO |
| Bug bounty launched | NO |
| `isMainnetEnabled()` still hard-coded `false` | YES |

## 14. Launch readiness verdict

# **NOT READY**

Detailed table in §1 + §3.4 + the launch checklist §0.

## 15. Remaining blockers

| Blocker | Why it blocks | Owner action |
|---|---|---|
| App URL (`{{APP_URL}}`) | Brief: "Do not mark launch ready if app URL is missing." | Stand up a publishable HTTPS URL for the deployed Next.js frontend. |
| Feedback URL (`PUBLIC_BETA_FEEDBACK_URL`) | Brief: "Do not mark launch ready if feedback and GitHub/issues are both missing." | Stand up a public form OR use GitHub Issues + issue templates. |
| GitHub URL (`PUBLIC_BETA_GITHUB_URL`) | Same rule (joint with feedback). | Open the public repo, enable Security Advisories, add issue templates. |

Recommended (not blocking): docs-hosting URLs can point at the GitHub repo paths once GitHub is live.

NOT blocking: API base URL (frontend bundles its own), status page URL.

## 16. Next milestone recommendation

**Primary:** Operator completes §2.1-§2.3 of `docs/PUBLIC_TESTNET_BETA_LAUNCH_REMAINING_ACTIONS.md`, then re-runs:

1. `OPERATOR-PUBLIC-BETA-URLS-FILL` — substitutes the now-live URLs and flips `status: "live"`.
   * Approval line: "I approve DeOpt V2 operator public beta URLs fill for this run."
2. `PUBLIC-TESTNET-BETA-LAUNCH-PREFLIGHT` (re-run) — re-evaluates the verdict and (if READY) creates `PUBLIC_TESTNET_BETA_LAUNCH_NEXT_TASK.md`.
   * Approval line: "I approve DeOpt V2 public testnet beta launch preflight for this run."

**After preflight returns READY:** `PUBLIC-TESTNET-BETA-LAUNCH` (publication) with the separate explicit approval line:
> "I approve DeOpt V2 public testnet beta launch publication for this run."

**Alternative (parallel, optional):** `EXTERNAL_AUDIT_DISPATCH_PREP` — close the 7 audit-readiness BLOCKERs in `docs/security-reanchor/AUDIT_READINESS_GAP_ANALYSIS.md`. URL-fill is non-blocking for audit prep; the two arcs can advance in parallel.

**Soft-launch option (no public post):** if the operator wants to gather early signal, the live Discord URL is sufficient for a Discord-only soft launch once `{{APP_URL}}` is stood up. See `PUBLIC_TESTNET_BETA_LAUNCH_REMAINING_ACTIONS.md §7`. The verdict in this preflight does NOT need to flip for the soft launch path; it only needs to flip for the hard public-announcement launch.

**Explicitly NOT recommended now:** mainnet activation, audit firm outreach, bug bounty launch, KMS cutover, Safe migration, flipping `isMainnetEnabled()`, publishing the announcement (announcement final draft is prominently marked NOT YET PUBLISHED).

Milestone outcome: a verified verdict of **NOT READY** for the public testnet beta hard-launch announcement, driven by three URL blockers (App + Feedback + GitHub) and surfaced via a new launch-readiness §0 in the launch checklist; a publish-ready announcement final draft prominently marked NOT YET PUBLISHED; an operator-facing remaining-actions doc spelling out the exact workflow to flip the verdict; and a clean frontend smoke (typecheck / lint / build / Playwright catalog) plus a clean public-docs smoke (address freshness, disclaimer coverage, stale-ME callouts properly flagged). Discord remains live. No chain / wallet / `.env` / private activity. No announcement published.

**End of PUBLIC-TESTNET-BETA-LAUNCH-PREFLIGHT result.**
