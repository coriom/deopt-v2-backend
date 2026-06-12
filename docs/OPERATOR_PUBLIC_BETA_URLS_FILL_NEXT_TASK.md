# OPERATOR-PUBLIC-BETA-URLS-FILL — Next Task Brief

**Date written:** 2026-06-12
**Origin:** `docs/public-beta/OPERATOR_PUBLIC_BETA_URLS_FILL.md` + `COMMUNITY_FEEDBACK_LOOP_RESULT.md`

> **Status note (2026-06-12, post-execution):** This brief was executed on 2026-06-12. Result: NO operator-supplied URLs were available at execution time, so all 6 frontend slots + 6 doc-side tokens remain placeholders. The frontend link config was refactored to a 4-status type union but no URL was invented. Full result in `OPERATOR_PUBLIC_BETA_URLS_FILL_RESULT.md`; remaining operator action list in `OPERATOR_PUBLIC_BETA_URLS_REMAINING_ACTIONS.md`. **This brief is reusable** — re-running it with the same approval line whenever the operator has real URLs in hand will perform the substitution.

**Target:** swap the `{{PLACEHOLDER}}` tokens in the frontend link config + the public-beta docs for the operator's real, publicly-accessible channel URLs (Discord, GitHub, feedback form, hosted-docs root, app URL, API base URL).
**Posture:** **Docs + frontend link-config string substitution only. NEVER mainnet. NEVER chain transactions. NEVER backend `.env` edit. NEVER private key handling. NEVER add admin / bearer / RPC-with-key URLs. NEVER claim "audited" or "mainnet-ready".**

> **This task is NOT executed by the calling milestone (security re-anchor).** It is a small, separable operator action — moved out of the security packet so it doesn't block the audit-prep arc.

---

## 1. Literal operator approval line (REQUIRED, VERBATIM)

> "I approve DeOpt V2 operator public beta URLs fill for this run."

Properties:
* Authorises substituting placeholder tokens with operator-provided real URLs.
* Authorises flipping each affected entry's `status` from `"placeholder"` to `"live"` in `deopt-v2-frontend/src/lib/public-beta-links.ts`.
* Does NOT authorise inventing URLs. The operator must supply each.
* Does NOT authorise adding RPC URLs with keys, bearer tokens, or any other secret to any link href.
* Does NOT authorise mainnet activity or any source change outside the link config + docs.

---

## 2. Scope

### 2.1 Frontend (`deopt-v2-frontend/src/lib/public-beta-links.ts`)

For each operator-supplied URL:
* Replace the `href` value (`"{{TOKEN}}"` → `"<real URL>"`).
* Change `status` to `"live"`.
* Leave other fields untouched.

For any token the operator does NOT supply, leave the placeholder in place. Partial substitution is OK and explicitly supported (`isPlaceholderHref()` degrades per slot).

### 2.2 Docs (`deopt-v2-backend/docs/public-beta/`)

In-place edit per `OPERATOR_PUBLIC_BETA_URLS_FILL.md §2.2`. Tokens to substitute (matched 1:1 with the frontend tokens):

* `{{ GITHUB_REPO_URL }}` ↔ `PUBLIC_BETA_GITHUB_URL`
* `{{ DISCORD_INVITE_URL }}` ↔ `PUBLIC_BETA_DISCORD_URL`
* `{{ TELEGRAM_INVITE_URL }}` (no frontend equivalent; doc-side only)
* `{{ FEEDBACK_FORM_URL }}` ↔ `PUBLIC_BETA_FEEDBACK_URL`
* `{{APP_URL}}` (doc-side only; the hosted app URL)
* `{{ API_BASE_URL }}` (doc-side only; the publicly callable backend URL)

### 2.3 Announcement drafts

Update `docs/public-beta/PUBLIC_TESTNET_BETA_ANNOUNCEMENT_DRAFT.md` to use the real URLs in each draft. The honesty checklist (§8) must still pass — this is positioning vocabulary, not a relaxation.

### 2.4 Verification per `OPERATOR_PUBLIC_BETA_URLS_FILL.md §4`

* Sensitive-string scan: zero hits (no bearer, no RPC URL with key, no DATABASE_URL, no 64-char hex).
* Mainnet RPC pattern scan: zero hits.
* Positive-claim drift scan: zero hits.
* Open the deployed frontend; verify every newly-live slot renders as a clickable anchor.
* Trigger a sign-failure modal (Playwright fixture or local wallet rejection) and verify "Report this issue" is now a clickable anchor.

---

## 3. Safety rules (recap; see operator-fill doc §3)

Before pasting a URL into a slot:
* No bearer / token / API key in the URL string.
* No RPC URL with a key segment (`/v2/<key>`).
* No localhost URL announced publicly.
* No mainnet-related URL (e.g. `basescan.org` instead of `sepolia.basescan.org`).
* No internal operator dashboard link.
* HTTPS only.
* No tracking / analytics tokens that expose internal naming.

---

## 4. Hard preconditions

| # | Precondition | Verifying check |
|---|---|---|
| P1 | Approval line (§1) present verbatim | grep |
| P2 | Operator has supplied at least one real URL (else: do not run this milestone) | external |
| P3 | Backend `.env` untouched | `stat -c '%y'` |
| P4 | Private file untouched | `stat -c '%a %y'` |
| P5 | `~/DEOPT/private/**` NOT read | trust |

---

## 5. Forbidden

* Inventing URLs.
* Bearer / API key / DATABASE_URL in any href.
* Mainnet anything.
* Removing testnet banners.
* Removing the public-beta posture from any doc.
* Changing `isMainnetEnabled()` in `chains.ts`.
* Source code changes outside `src/lib/public-beta-links.ts`.

---

## 6. Acceptance criteria

* Frontend `public-beta-links.ts` updated (any subset of slots).
* `status` field reflects `live` for every substituted slot.
* Doc-side placeholder tokens updated everywhere they appear.
* Announcement drafts updated.
* `npm run typecheck && npm run lint && npm run build` clean.
* `npx playwright test --list` clean.
* `git diff --check` clean.
* Sensitive-string scan zero hits.
* Positive-claim drift scan zero hits.

---

## 7. Cross-links

* `docs/public-beta/OPERATOR_PUBLIC_BETA_URLS_FILL.md`
* `docs/public-beta/PUBLIC_TESTNET_BETA_ANNOUNCEMENT_DRAFT.md`
* `docs/public-beta/PUBLIC_TESTNET_BETA_LAUNCH_CHECKLIST.md`
* `docs/COMMUNITY_FEEDBACK_LOOP_RESULT.md`
* `~/DEOPT/RUN_STATE.md`

**End of operator public beta URLs fill next-task brief.**
