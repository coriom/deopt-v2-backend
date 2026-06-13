# PUBLIC-TESTNET-BETA-LAUNCH-PREFLIGHT — RERUN — Next Task Brief

**Date written:** 2026-06-13
**Origin:** `FRONTEND_INTEGRATED_DOCS_AND_FEEDBACK_RESULT.md` + `PUBLIC_TESTNET_BETA_LAUNCH_PREFLIGHT_RESULT.md`.
**Target:** re-evaluate the public testnet beta launch readiness verdict now that 5 of the 6 URL blockers have been replaced by internal frontend routes and GitHub is wired to `https://github.com/DeOpt`. The previous preflight (2026-06-13 earlier today) returned **NOT READY** because App / Feedback / GitHub were all placeholder; this rerun should observe the new state and flip the verdict accordingly.

**Posture:** **Docs + verification only (same as the original preflight). NEVER mainnet. NEVER chain transactions. NEVER backend `.env` edit. NEVER private key handling. NEVER publish an announcement under this brief.**

> **This task is the same milestone as `PUBLIC-TESTNET-BETA-LAUNCH-PREFLIGHT`, rerun with the same approval line.** It is documented as its own brief only because the verdict context has changed materially.

---

## 1. Literal operator approval line (REQUIRED, VERBATIM)

> "I approve DeOpt V2 public testnet beta launch preflight for this run."

(Same line as the original preflight; reusing it is intentional.)

---

## 2. Recognise the new URL state

The frontend now serves the following as **live internal routes**:

| Token | New value | Status |
|---|---|---|
| `PUBLIC_BETA_QUICKSTART_URL` | `/docs/quickstart` | LIVE (internal) |
| `PUBLIC_BETA_TESTING_GUIDE_URL` | `/docs/testing-guide` | LIVE (internal) |
| `PUBLIC_BETA_LIMITATIONS_URL` | `/docs/limitations` | LIVE (internal) |
| `PUBLIC_BETA_FEEDBACK_URL` | `/feedback` | LIVE (internal) |
| `PUBLIC_BETA_GITHUB_URL` | `https://github.com/DeOpt` | LIVE (external) |
| `PUBLIC_BETA_DISCORD_URL` | `https://discord.gg/zaEMvWuxu` | LIVE (external) — unchanged |
| `{{APP_URL}}` (doc-side) | (not yet supplied) | PLACEHOLDER — sole remaining hard blocker |
| `{{ API_BASE_URL }}` (doc-side) | (frontend bundles via build env) | NOT_REQUIRED_FOR_LAUNCH |
| Status page URL | n/a | NOT_REQUIRED_FOR_LAUNCH |

The internal routes are SSG-prerendered by Next.js (`/docs/[slug]` for the 4 doc slugs + `/feedback`). The 9 source MD files live at `deopt-v2-frontend/src/content/public-beta/` (mirrored from `deopt-v2-backend/docs/public-beta/`). The bundled app contains both code and docs — no external docs host required.

---

## 3. Scope

### 3.1 Re-evaluate verdict per the original preflight rules

* `App URL` STILL missing → **single hard blocker**.
* `Feedback URL` LIVE → no longer a blocker.
* `GitHub URL` LIVE → no longer a blocker.
* `Discord` LIVE → no change.
* `Quickstart / Testing-Guide / Limitations` LIVE → recommended blockers cleared.
* `API base URL`, `Status page URL` → NOT_REQUIRED_FOR_LAUNCH (unchanged).

### 3.2 Expected verdict path

* If `{{APP_URL}}` is supplied to the rerun (operator stood up the deployment URL): verdict → **READY** (or **READY WITH NON-BLOCKING PLACEHOLDERS** if `{{ API_BASE_URL }}` / status page URL remain placeholder). The preflight will then create `PUBLIC_TESTNET_BETA_LAUNCH_NEXT_TASK.md` with the publication approval line.
* If `{{APP_URL}}` is NOT supplied: verdict → **NOT READY** still, but with only **one** remaining blocker (down from three). The preflight will update `PUBLIC_TESTNET_BETA_LAUNCH_REMAINING_ACTIONS.md` to reflect that only the app URL remains.

### 3.3 Re-run the smoke checks

* `npm run typecheck` → expected clean
* `npm run lint` → expected clean
* `npm run build` → expected green at 14 routes (including the 4 new SSG doc routes + `/feedback`)
* `npx playwright test --list` → expected ≥ 82 tests across ≥ 22 files

### 3.4 Re-confirm public docs smoke (Phase E in the original brief)

* All 15 public-beta docs still present in `deopt-v2-backend/docs/public-beta/` (no docs deleted by this followup).
* Mirrored docs at `deopt-v2-frontend/src/content/public-beta/` still drift-free with the canonical source (re-mirror if any are stale).
* Canonical addresses still current. Stale ME / MarginEngine still flagged historical.

### 3.5 Update the launch checklist verdict block

`docs/public-beta/PUBLIC_TESTNET_BETA_LAUNCH_CHECKLIST.md` §0:

* Update the verdict row.
* Update the blocker table: rows 1-7 should reflect that only App URL remains as a hard blocker.

---

## 4. Out of scope

* Publishing the announcement. Even if verdict flips to READY, this brief does NOT publish — that requires the separate publication milestone with its own explicit approval line.
* Standing up the app URL itself. That's an operator-side action; this brief verifies the result.
* Any chain action.
* Any external communication.
* Mainnet anything.
* Audit firm outreach.
* Bug bounty.

---

## 5. Hard preconditions

| # | Precondition | Verifying check |
|---|---|---|
| P1 | Approval line (§1) present verbatim | grep |
| P2 | `FRONTEND_INTEGRATED_DOCS_AND_FEEDBACK_RESULT.md` exists and confirms internal-routes wiring | `ls` |
| P3 | `deopt-v2-frontend/src/lib/public-beta-links.ts` shows `discord`, `github`, `quickstart`, `testing-guide`, `limitations`, `feedback` all with `status: "live"` | grep / read |
| P4 | Backend `.env` untouched | `stat -c '%y'` |
| P5 | Private file untouched | `stat -c '%a %y'` |
| P6 | `~/DEOPT/private/**` NOT read | trust |

---

## 6. Forbidden

* Mainnet RPC.
* Mainnet contract address presented as current.
* `.env` edit.
* Bearer / RPC URL with key / DATABASE_URL in any milestone file.
* Source-code changes to `deopt-v2-sol/src/`, `deopt-v2-backend/src/`, or `deopt-v2-frontend/src/` BEYOND minor adjustments needed to thread `{{APP_URL}}` through link config IF the operator supplies it under this rerun.
* Publishing the announcement.
* Audit firm contact.
* Bug bounty launch.

---

## 7. Acceptance criteria

* New `PUBLIC_TESTNET_BETA_LAUNCH_PREFLIGHT_RESULT.md` (with re-run date suffix or appended new section) showing the updated verdict.
* `docs/public-beta/PUBLIC_TESTNET_BETA_LAUNCH_CHECKLIST.md` §0 verdict updated.
* `docs/PUBLIC_TESTNET_BETA_LAUNCH_REMAINING_ACTIONS.md` updated to reflect only the App URL remaining (or marked CLOSED if `{{APP_URL}}` is supplied and verdict → READY).
* If verdict → READY: `docs/PUBLIC_TESTNET_BETA_LAUNCH_NEXT_TASK.md` CREATED with the publication approval line clearly stated.
* `git diff --check` clean.
* Sensitive-string scan zero hits on changed files.
* Positive-claim drift scan zero true hits.

---

## 8. Cross-links

* `docs/PUBLIC_TESTNET_BETA_LAUNCH_PREFLIGHT_RESULT.md` (the previous preflight result; this rerun supersedes its verdict block)
* `docs/PUBLIC_TESTNET_BETA_LAUNCH_REMAINING_ACTIONS.md` (operator workflow — will be partially closed by this rerun)
* `docs/public-beta/PUBLIC_TESTNET_BETA_LAUNCH_CHECKLIST.md` §0
* `docs/public-beta/PUBLIC_TESTNET_BETA_ANNOUNCEMENT_FINAL_DRAFT.md` (publication template — still NOT YET PUBLISHED)
* `docs/FRONTEND_INTEGRATED_DOCS_AND_FEEDBACK_RESULT.md` (this milestone's result; the source of the new URL state)
* `~/DEOPT/RUN_STATE.md`

**End of public testnet beta launch preflight RERUN next-task brief.**
