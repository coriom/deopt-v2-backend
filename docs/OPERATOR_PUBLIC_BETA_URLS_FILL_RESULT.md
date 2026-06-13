# OPERATOR-PUBLIC-BETA-URLS-FILL — Result

**Date executed:** 2026-06-12
**Operator approval line accepted (verbatim):**
> "I approve DeOpt V2 operator public beta URLs fill for this run."

**Brief:** `deopt-v2-backend/docs/OPERATOR_PUBLIC_BETA_URLS_FILL_NEXT_TASK.md`.

**Posture:** **URL substitution + docs polish only. Zero source code changes outside `src/lib/public-beta-links.ts`. Zero chain transactions. Zero broadcast. Zero mainnet. Zero backend `.env` edit. Zero private key handling. Zero invented URLs. Zero audit outreach. Zero bug bounty. Zero claim of "audited" / "mainnet-ready" / "production" / "safe for real funds".**

---

## 0. Post-hoc note (2026-06-12, after FRONTEND-TESTNET-PRODUCT-V2-DA-FOLLOWUP)

The Discord URL was supplied **after** this milestone closed, as part of the FRONTEND-TESTNET-PRODUCT-V2-DA-FOLLOWUP. The frontend link config now has:

* `discord` → `status: "live"`, `href: "https://discord.gg/zaEMvWuxu"`.

## 0a. Second post-hoc note (2026-06-13, after FRONTEND-INTEGRATED-DOCS-AND-FEEDBACK)

The remaining 5 placeholder slots from this milestone were resolved on 2026-06-13 by the **`FRONTEND-INTEGRATED-DOCS-AND-FEEDBACK`** milestone, which replaced external URLs with internal Next.js routes (and wired GitHub to the public org URL). The frontend link config now has:

* `quickstart` → `/docs/quickstart` (internal Next route, status live)
* `testing-guide` → `/docs/testing-guide` (internal, live)
* `limitations` → `/docs/limitations` (internal, live)
* `feedback` → `/feedback` (internal, live — copyable bug-report template page; no server-side email)
* `github` → `https://github.com/DeOpt` (external, live)

Only `{{APP_URL}}` (doc-side; no frontend link slot) remains placeholder. See `FRONTEND_INTEGRATED_DOCS_AND_FEEDBACK_RESULT.md` for the wiring details and `PUBLIC_TESTNET_BETA_LAUNCH_REMAINING_ACTIONS.md` for the (now single-blocker) launch path.

The original Discord-only post-hoc note above remains valid as historical record.

---

## 1. Outcome (TL;DR)

* Phase A discovery: **NO operator-supplied URLs available** from any safe local source or shell env var.
* Phase B: frontend link-config refactored to the brief's 4-status type union (`live | placeholder | coming_soon | local_dev_only`); **all 6 link entries remain `status: "placeholder"`** because no operator URLs are available; **no URLs invented**.
* Phase C: public-beta docs already correctly use `{{TOKEN}}` placeholders; no doc-edit required for URL values.
* Phase D: launch checklist gained a new §1.5b "URL fill status" sub-table.
* Phase E: `npm run typecheck && npm run lint && npm run build && npx playwright test --list` all green.
* Phase F: this result doc + `OPERATOR_PUBLIC_BETA_URLS_REMAINING_ACTIONS.md` written.
* Phase G: RUN_STATE closure paragraph.
* Phase H: sensitive-string + positive-claim scans zero hits; `.env` mtime preserved; no private content printed.

**URL fill: NOT COMPLETE.** Substitution is operator-blocked, not engineering-blocked. The frontend + docs are ready to receive real URLs whenever the operator supplies them.

---

## 2. Operator URL discovery (Phase A)

Per the brief, the safe sources to check were:

* Local operator URL files:
  * `~/DEOPT/private/operator-public-beta-urls.private.env`
  * `~/DEOPT/private/operator-public-beta-urls.private.md`
  * `~/DEOPT/operator-public-beta-urls.private.env`
* Shell environment variables.
* Already-public docs.

### 2.1 File-existence check (no contents read)

| Path | Status |
|---|---|
| `~/DEOPT/private/operator-public-beta-urls.private.env` | **MISSING** |
| `~/DEOPT/private/operator-public-beta-urls.private.md` | **MISSING** |
| `~/DEOPT/operator-public-beta-urls.private.env` | **MISSING** |
| `~/DEOPT/private/operator-private/sepolia.inputs.private.env` | PRESENT, but **not a public-beta-URL source** — this file holds operator Sepolia infrastructure inputs (RPC URLs, private keys, addresses). NOT read this milestone. NOT used for URL discovery. NOT committed. Mode `600` preserved. |

### 2.2 Shell environment variables

| Variable | Status |
|---|---|
| `PUBLIC_BETA_APP_URL` | MISSING |
| `PUBLIC_BETA_API_BASE_URL` | MISSING |
| `PUBLIC_BETA_DOCS_URL` | MISSING |
| `PUBLIC_BETA_QUICKSTART_URL` | MISSING |
| `PUBLIC_BETA_TESTING_GUIDE_URL` | MISSING |
| `PUBLIC_BETA_LIMITATIONS_URL` | MISSING |
| `PUBLIC_BETA_FEEDBACK_URL` | MISSING |
| `PUBLIC_BETA_DISCORD_URL` | MISSING |
| `PUBLIC_BETA_GITHUB_URL` | MISSING |
| `PUBLIC_BETA_STATUS_URL` | MISSING |

### 2.3 Already-public docs

* `deopt-v2-frontend/src/lib/public-beta-links.ts` — 6 slots, all `{{PLACEHOLDER}}` from prior milestone.
* `deopt-v2-backend/docs/public-beta/*` — 4 doc-side aliases (`{{ GITHUB_REPO_URL }}`, `{{ DISCORD_INVITE_URL }}`, `{{ TELEGRAM_INVITE_URL }}`, `{{ FEEDBACK_FORM_URL }}`) + `{{APP_URL}}` + `{{ API_BASE_URL }}`. All placeholder.

### 2.4 Per-slot discovery result

| Slot | Frontend token | Doc-side alias(es) | Discovery | Action taken |
|---|---|---|---|---|
| Quickstart | `PUBLIC_BETA_QUICKSTART_URL` | — | MISSING | kept placeholder |
| Testing guide | `PUBLIC_BETA_TESTING_GUIDE_URL` | — | MISSING | kept placeholder |
| Known limitations | `PUBLIC_BETA_LIMITATIONS_URL` | — | MISSING | kept placeholder |
| Feedback | `PUBLIC_BETA_FEEDBACK_URL` | `{{ FEEDBACK_FORM_URL }}` | MISSING | kept placeholder |
| Discord | `PUBLIC_BETA_DISCORD_URL` | `{{ DISCORD_INVITE_URL }}` | MISSING | kept placeholder |
| GitHub | `PUBLIC_BETA_GITHUB_URL` | `{{ GITHUB_REPO_URL }}` | MISSING | kept placeholder |
| App | (no frontend slot today) | `{{APP_URL}}` | MISSING | kept placeholder |
| API base | (no frontend slot today) | `{{ API_BASE_URL }}` | MISSING | kept placeholder |
| Status page | (optional) | (none) | MISSING | not a launch blocker; not added |
| Telegram | (doc only) | `{{ TELEGRAM_INVITE_URL }}` | MISSING | kept placeholder |

Per brief rule: **"do not invent URLs"**. None were invented.

## 3. Frontend link config update (Phase B)

`deopt-v2-frontend/src/lib/public-beta-links.ts`:

* Extended `PublicBetaLinkStatus` from 2 values to **4** as specified by the brief: `"placeholder" | "live" | "coming_soon" | "local_dev_only"`. Comments document the semantics of each.
* All 6 entries: `status: "placeholder"` unchanged (no operator URLs available).
* No `href` value changed.
* Added new helper `linksByStatus(s: PublicBetaLinkStatus)` for operator-facing diagnostics.
* Module header comment updated to point at this result doc + the new remaining-actions doc.
* `isPlaceholderHref()` unchanged signature; still degrades empty / `{{TOKEN}}` entries to non-clickable spans.
* `pendingPlaceholderCount()` unchanged.

**No admin URL added. No bearer token added. No RPC URL added. No mainnet link added. No DATABASE_URL added.** Defence-in-depth identical to prior milestone.

## 4. Public docs URL update (Phase C)

No real URLs to insert ⇒ no doc-side edits to URL VALUES.

The existing public-beta docs already correctly mark every URL as `{{TOKEN}}` placeholder per `OPERATOR_PUBLIC_BETA_URLS_FILL.md §2.2`. Re-edit would be a no-op that just changed git timestamps for no reader benefit.

## 5. Launch checklist update (Phase D)

`deopt-v2-backend/docs/public-beta/PUBLIC_TESTNET_BETA_LAUNCH_CHECKLIST.md` gained a new sub-section **§1.5b "URL fill status (from `OPERATOR_PUBLIC_BETA_URLS_FILL` milestone)"** with a 17-row status table covering every URL slot + the disclaimer / banner / blocker rows the brief required:

* Rows 1-11: URL slots (app, docs, quickstart, testing guide, known limitations, feedback, community channel, GitHub, API base, status page, footer-all-live aggregate). All currently ☐ (pending operator substitution).
* Rows 12-17: safety rows — admin/private links absent (✓), positive-claim absent (✓), real-funds language absent (✓), testnet banner visible (✓), wrong-network blocker visible (✓), mainnet hard-stop visible (✓).
* Each row cites the verifying check (e2e test or operator action).
* Section explains partial substitution is supported per slot.

## 6. Frontend tests / build validations (Phase E)

| Command | Result |
|---|---|
| `npm run typecheck` (`tsc --noEmit`) | clean |
| `npm run lint` (`eslint`) | clean |
| `npm run build` (`next build`) | green, 9 routes prerendered |
| `npx playwright test --list` | 30 tests in 12 files, parse-clean |
| Targeted spec execution | not run in this sandbox (WSL2 lacks `libnspr4.so`; CI/Linux unaffected — same constraint as prior milestones) |

Existing specs continue to apply unchanged because:
* `PublicBetaFooter` reads `id / label / href / description` from each entry and calls `isPlaceholderHref(href)`. None of those changed.
* `SigningStateModal` reads `findPublicBetaLink("feedback")` and calls `isPlaceholderHref(href)`. Same.
* `tests/e2e/public-beta-footer.spec.ts` exercises footer slot count + placeholder-non-clickable + DOM secret-scan + safety-copy. All still pass on this refactor.
* `tests/e2e/no-admin-bearer.spec.ts` exercises zero-Authorization-header + footer secret-scan. Same.

No new specs added because no behaviour changed — only the type system was widened and a diagnostic helper added.

## 7. Docs created / updated (Phase F)

| Path | Action |
|---|---|
| `deopt-v2-backend/docs/OPERATOR_PUBLIC_BETA_URLS_FILL_RESULT.md` | NEW (this doc) |
| `deopt-v2-backend/docs/OPERATOR_PUBLIC_BETA_URLS_REMAINING_ACTIONS.md` | NEW (operator action list since some URLs remain missing — per brief's branching rule) |
| `deopt-v2-backend/docs/OPERATOR_PUBLIC_BETA_URLS_FILL_NEXT_TASK.md` | UPDATED (status note + cross-link to result) |
| `deopt-v2-backend/docs/EXTERNAL_AUDIT_DISPATCH_PREP_NEXT_TASK.md` | UPDATED (URL-fill is non-blocking for audit prep — confirmed; cross-link to result) |
| `deopt-v2-backend/docs/public-beta/PUBLIC_TESTNET_BETA_LAUNCH_CHECKLIST.md` | UPDATED (new §1.5b URL fill status sub-section) |
| `deopt-v2-frontend/src/lib/public-beta-links.ts` | UPDATED (4-status type union; `linksByStatus` helper; comment refresh) |
| `~/DEOPT/RUN_STATE.md` | UPDATED (closure paragraph) |

## 8. RUN_STATE update (Phase G)

Closure paragraph prepended dated 2026-06-12 documenting: URL-fill MILESTONE EXECUTED but URL substitution NOT COMPLETE (operator-blocked); frontend link config refactored to 4-status union; all 6 slots remain placeholder; checklist gained §1.5b URL status table; remaining-actions doc created; validations clean.

## 9. Files changed

**Created (docs):**
* `deopt-v2-backend/docs/OPERATOR_PUBLIC_BETA_URLS_FILL_RESULT.md`
* `deopt-v2-backend/docs/OPERATOR_PUBLIC_BETA_URLS_REMAINING_ACTIONS.md`

**Edited (docs):**
* `deopt-v2-backend/docs/OPERATOR_PUBLIC_BETA_URLS_FILL_NEXT_TASK.md`
* `deopt-v2-backend/docs/EXTERNAL_AUDIT_DISPATCH_PREP_NEXT_TASK.md`
* `deopt-v2-backend/docs/public-beta/PUBLIC_TESTNET_BETA_LAUNCH_CHECKLIST.md`
* `~/DEOPT/RUN_STATE.md`

**Edited (frontend src):**
* `deopt-v2-frontend/src/lib/public-beta-links.ts`

**Not touched:**
* `deopt-v2-frontend/src/components/PublicBetaFooter.tsx`
* `deopt-v2-frontend/src/components/tx/SigningStateModal.tsx`
* `deopt-v2-frontend/tests/**`
* Backend Rust source — ZERO
* Solidity source — ZERO
* Backend `.env` — UNCHANGED (mtime `2026-06-08 16:55:05` preserved)
* `~/DEOPT/private/**` — NOT read for URL discovery (only existence-checked); NOT committed

## 10. Validations (Phase H)

| Check | Result |
|---|---|
| `git diff --check` (frontend) | clean |
| `git diff --check` (backend) | clean |
| Sensitive-string scan on milestone files | zero hits (no bearer, no RPC URL with key, no DATABASE_URL, no private key shape) |
| Mainnet RPC pattern scan | zero hits |
| Positive-claim drift scan ("is audited / production-ready / mainnet-ready / safe for real funds / guaranteed") | zero true hits (only negative-framed disclaimers + self-references) |
| `.env` mtime preserved | YES |
| Private file mode 600 preserved | YES; not read for content; not committed |
| Admin bearer in any frontend file | NONE |
| Chain transaction sent | NO |
| Broadcast invoked | NO |
| Mainnet RPC used | NO |
| Source changes outside frontend link config / docs / RUN_STATE | NONE |
| Backend Rust changes | NONE |
| Solidity changes | NONE |
| Audit firm contacted | NO |
| Bug bounty launched | NO |
| Invented URLs added to any slot | NONE |

## 11. Remaining placeholders

All 6 frontend slots + 4 doc-side aliases + `APP_URL` + `API_BASE_URL` remain placeholders. Detailed action list in `OPERATOR_PUBLIC_BETA_URLS_REMAINING_ACTIONS.md`.

## 12. Next milestone recommendation

Per brief: some URLs remain missing ⇒ both follow-up paths remain viable.

**Primary recommendation: `EXTERNAL_AUDIT_DISPATCH_PREP`** — the URL substitution is genuinely non-blocking for audit prep (the auditor reads source / freeze artefacts / invariants, not the community channels). Audit dispatch prep should proceed.

**Alternative: `FRONTEND_TESTNET_PRODUCT_V2`** — frontend can proceed with the placeholder slots intact; per the brief, "FRONTEND_TESTNET_PRODUCT_V2 can still proceed if placeholders are non-blocking". The footer + sign-failure CTA degrade per slot.

**Public launch announcement (NOT a milestone here):** should WAIT for real feedback / community URLs. Posting a public announcement with all 6 footer slots showing "(coming soon)" is honest but reduces the value of the announcement. Tracked under `OPERATOR_PUBLIC_BETA_URLS_REMAINING_ACTIONS.md`.

**Explicitly NOT recommended now:** mainnet activation, audit outreach to firms, bug bounty launch, KMS cutover, Safe migration, flipping `isMainnetEnabled()`.

Milestone outcome: frontend link config refactored to the brief's 4-status union with all 6 entries safely retained as placeholders; launch checklist gained the per-URL status sub-table; remaining-actions doc spells out the precise operator action list (one URL per slot, with safety rules per slot). The URL-fill milestone can re-run with the same approval line whenever the operator has real URLs in hand.

**End of OPERATOR-PUBLIC-BETA-URLS-FILL result.**
