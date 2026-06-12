# COMMUNITY-FEEDBACK-LOOP — Next Task Brief

**Date written:** 2026-06-12
**Origin:** `PUBLIC_BETA_DOCS_PACK_NEXT_TASK.md` + `docs/public-beta/README.md` launch checklist item #3.
**Target:** wire up the placeholder community feedback channels (`{{ GITHUB_REPO_URL }}`, `{{ DISCORD_INVITE_URL }}`, `{{ TELEGRAM_INVITE_URL }}`, `{{ FEEDBACK_FORM_URL }}`) in the public-beta docs and frontend, and establish a lightweight triage cadence.
**Posture:** **Operator-only setup + docs replacement. NEVER mainnet. NEVER chain transactions. NEVER backend `.env` edit. NEVER private key handling. NEVER source code changes that reach the contract surface.**

> **This task is NOT executed by the calling milestone. It captures community-channel setup as one approval-gated milestone.**

---

## 1. Literal operator approval line (REQUIRED, VERBATIM)

> "I approve DeOpt V2 community feedback loop setup for this run."

Properties:
* Authorises external account / repo / server creation by the operator.
* Authorises docs + frontend URL replacement.
* Does NOT authorise contract changes, chain transactions, or anything that costs gas.

---

## 2. Scope — channel setup (operator-side, out of harness)

* **GitHub repository.** Ensure the public repo (or its public-facing mirror) is ready for external issues. Issue templates for "bug report" and "feature request" added.
* **Discord server.** Create / configure: `#announcements`, `#general`, `#bug-reports`, `#dev-integrations`, `#oracle-status`. Set channel descriptions; appoint moderators.
* **Telegram channel.** Mirror of `#announcements`. Optional `@DeOptBetaBugs` bot for forwarding bug-report submissions.
* **Feedback form.** Set up a Google Form / Tally / Typeform variant matching the bug-report template in `docs/public-beta/FEEDBACK_AND_BUG_REPORTING.md` §2. Output forwarded to a triage email or a private Discord channel.
* **Security disclosure inbox.** Set up `security@…` email forward AND enable GitHub Security Advisories on the repo.

## 3. Scope — placeholder URL replacement (in-tree)

* Update `docs/public-beta/FEEDBACK_AND_BUG_REPORTING.md` placeholders with real URLs.
* Update `docs/public-beta/README.md` launch checklist item #3 to mark "feedback channel configured" ✓.
* Update `docs/public-beta/FAQ.md` final-section placeholders.
* Update **`deopt-v2-frontend/src/lib/public-beta-links.ts`** — six `{{PUBLIC_BETA_*_URL}}` placeholders are already wired through to the footer + the signing-failure-modal "Report this issue" CTA. As of `FRONTEND_TESTNET_LAUNCH_POLISH_RESULT.md` (2026-06-12), substituting the real URLs is a one-file edit; the footer auto-promotes placeholder spans into clickable anchors when `isPlaceholderHref()` returns `false`.
* Update `docs/public-beta/BASE_SEPOLIA_QUICKSTART.md` §6 if it references a specific channel.

## 4. Scope — triage cadence

* **Daily.** Operator scans `#bug-reports`, GitHub issues, and feedback-form inbox.
* **Weekly.** Operator posts a "beta status update" in `#announcements` (and Telegram mirror) covering: known issues, what's been fixed, what's in flight, next milestones.
* **High-severity bugs.** Operator triages within 1 business day. If a security-impacting bug is reported publicly, ask the reporter to switch to the private security inbox and delete the public message.

---

## 5. Out of scope

* No paid moderation contract.
* No live chat support staffing (best-effort only).
* No commercial CRM integration.
* No automated bot that auto-replies — humans only for the public beta.

---

## 6. Hard preconditions

| # | Precondition | Verifying check |
|---|---|---|
| P1 | Approval line (§1) present verbatim | grep |
| P2 | `docs/public-beta/FEEDBACK_AND_BUG_REPORTING.md` exists with placeholder URLs | read |
| P3 | Operator has the rights to create / configure the external channels | external |
| P4 | `.env` (`deopt-v2-backend/.env`) untouched | `stat -c '%y'` |
| P5 | Private file untouched | `stat -c '%a %y'` |

---

## 7. Forbidden

* No mainnet (chain id `8453`) wording in any channel description.
* No claim "audited" or "mainnet-ready" in any channel description.
* No private key / RPC URL / DB credential in any channel post.
* No solicitation of personal data beyond what the bug-report template needs.

---

## 8. Acceptance criteria

* All four URL placeholders in `docs/public-beta/` replaced with real URLs OR documented as "still being set up — see #announcements" if some are delayed.
* Operator posts an initial welcome message in each channel.
* GitHub issue templates are visible when a new issue is opened.
* Feedback form responses route to a monitored inbox.
* `docs/public-beta/README.md` launch checklist item #3 marked ✓.
* `git diff --check` clean.

---

## 9. Cross-links

* `docs/public-beta/FEEDBACK_AND_BUG_REPORTING.md`
* `docs/public-beta/README.md`
* `docs/public-beta/FAQ.md`
* `FRONTEND_TESTNET_LAUNCH_POLISH_NEXT_TASK.md`
* `~/DEOPT/RUN_STATE.md`

**End of community feedback loop next-task brief.**
