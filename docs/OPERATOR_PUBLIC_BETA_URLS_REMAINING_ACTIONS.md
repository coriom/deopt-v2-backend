# DeOpt V2 — Operator Public Beta URLs — Remaining Actions

> **Public testnet beta. Base Sepolia only. No real funds. Unaudited.** Operator-facing action list for substituting the `{{PLACEHOLDER}}` tokens in the frontend link config + the public-beta docs with real, public URLs.
>
> **Why this doc exists:** the `OPERATOR_PUBLIC_BETA_URLS_FILL` milestone (2026-06-12) discovered that no operator-supplied URLs were available from any safe local source (no `private/operator-public-beta-urls.*` file, no shell env var). All 6 frontend slots + 6 doc-side tokens remain placeholders.
>
> **Status of the URL-fill milestone itself:** EXECUTED but URL substitution NOT COMPLETE — see `OPERATOR_PUBLIC_BETA_URLS_FILL_RESULT.md`. The blocker is operator-side: the channels themselves need to be created/configured before URLs can be substituted.

---

## 1. What is required, per slot

Each row below is one slot the operator must either fill or explicitly defer. Defer is fine (the UI degrades gracefully); silent skip is not (the launch checklist row stays ☐).

### 1.1 Frontend slots (`deopt-v2-frontend/src/lib/public-beta-links.ts`)

| Slot id | Token | Required to launch? | What to supply | Acceptance |
|---|---|---|---|---|
| `quickstart` | `PUBLIC_BETA_QUICKSTART_URL` | Yes (or defer-with-rationale) | Hosted URL of `docs/public-beta/BASE_SEPOLIA_QUICKSTART.md`. | Footer `public-beta-link-quickstart` renders as `<a target="_blank">`. |
| `testing-guide` | `PUBLIC_BETA_TESTING_GUIDE_URL` | Yes | Hosted URL of `docs/public-beta/USER_TESTING_GUIDE.md`. | Footer `public-beta-link-testing-guide` renders as `<a>`. |
| `limitations` | `PUBLIC_BETA_LIMITATIONS_URL` | Yes | Hosted URL of `docs/public-beta/KNOWN_LIMITATIONS_AND_RISKS.md`. | Footer `public-beta-link-limitations` renders as `<a>`. |
| `feedback` | `PUBLIC_BETA_FEEDBACK_URL` | Yes | Public bug-report form URL (Tally / Google Forms / Typeform) OR GitHub Issues URL. | Footer `public-beta-link-feedback` renders as `<a>` AND `SigningStateModal` "Report this issue" CTA becomes clickable. |
| `discord` | `PUBLIC_BETA_DISCORD_URL` | Strongly recommended | Discord invite link `https://discord.gg/<invite>`. | Footer `public-beta-link-discord` renders as `<a>`. |
| `github` | `PUBLIC_BETA_GITHUB_URL` | Yes | Public GitHub repo URL `https://github.com/<org>/<repo>`. | Footer `public-beta-link-github` renders as `<a>`. |

### 1.2 Doc-side aliases (`docs/public-beta/*`)

These are referenced by the docs but have no frontend slot today. They must be substituted in-place in the relevant doc files.

| Token | Required? | What to supply | Files containing it |
|---|---|---|---|
| `{{ GITHUB_REPO_URL }}` | Yes | Same as `PUBLIC_BETA_GITHUB_URL`. | `FEEDBACK_AND_BUG_REPORTING.md`, `FAQ.md` |
| `{{ DISCORD_INVITE_URL }}` | Recommended | Same as `PUBLIC_BETA_DISCORD_URL`. | `FEEDBACK_AND_BUG_REPORTING.md`, `FAQ.md` |
| `{{ TELEGRAM_INVITE_URL }}` | Optional | Telegram invite `https://t.me/+<invite>`. | `FEEDBACK_AND_BUG_REPORTING.md`, `FAQ.md`, `FEEDBACK_TRIAGE_WORKFLOW.md` |
| `{{ FEEDBACK_FORM_URL }}` | Recommended | Same as `PUBLIC_BETA_FEEDBACK_URL`. | `FEEDBACK_AND_BUG_REPORTING.md`, `FAQ.md`, `FEEDBACK_TRIAGE_WORKFLOW.md` |
| `{{APP_URL}}` | Yes | Hosted URL of the public-beta app. | `README.md` |
| `{{ API_BASE_URL }}` | Recommended | Publicly-callable backend API URL. | `DEVELOPER_API_GUIDE.md` |

### 1.3 Optional slot (NOT yet wired into the frontend)

| Slot | Token candidate | Notes |
|---|---|---|
| Status / uptime page | `PUBLIC_BETA_STATUS_URL` | Operator-side status page (Uptime / BetterStack / similar). NOT a launch blocker. If added, also extend `PUBLIC_BETA_LINKS` in `src/lib/public-beta-links.ts` with a new `status` entry. |

---

## 2. Substitution procedure (recap; see `docs/public-beta/OPERATOR_PUBLIC_BETA_URLS_FILL.md` for full detail)

### 2.1 Frontend module (`src/lib/public-beta-links.ts`)

For each operator-supplied URL:
1. Replace the `href` value from `"{{TOKEN}}"` to the real URL.
2. Change `status` from `"placeholder"` to `"live"`.
3. Leave other fields untouched.

For any slot the operator has decided to defer with a public-facing acknowledgement (e.g. "Telegram coming soon"):
* Optionally change `status` to `"coming_soon"` while keeping the `{{TOKEN}}` href. The footer still renders "(coming soon)" — the type-system change is for operator diagnostics only.

For a slot that will only ever be a dev URL (e.g. `localhost:3000` for self-hosted backend):
* `status: "local_dev_only"` + a real localhost URL. The footer can be extended later to suppress local-dev rows in the production bundle (`NEXT_PUBLIC_ENV === "production"`).

### 2.2 Docs

In-place edit per `docs/public-beta/OPERATOR_PUBLIC_BETA_URLS_FILL.md §2.2`.

### 2.3 Announcement drafts

After substitution, `docs/public-beta/PUBLIC_TESTNET_BETA_ANNOUNCEMENT_DRAFT.md` placeholder tokens become real URLs. Re-run the §8 honesty checklist before posting any draft.

---

## 3. Safety rules (recap; see `docs/public-beta/OPERATOR_PUBLIC_BETA_URLS_FILL.md §3` for full detail)

Before pasting a URL into a slot:
* No bearer / API key / token in the URL string.
* No RPC URL with a key segment (`/v2/<key>`).
* No localhost URL announced as public (use `local_dev_only` status for dev-only slots).
* No mainnet-related URL (e.g. `basescan.org` instead of `sepolia.basescan.org`).
* No internal operator dashboard link.
* HTTPS only.
* No tracking / analytics tokens that expose internal naming.

---

## 4. After substitution: re-run validations

Per the URL-fill milestone:

```bash
cd /home/corio/DEOPT/deopt-v2-frontend
npm run typecheck
npm run lint
npm run build
npx playwright test --list
```

All four must pass before posting any public announcement.

Then run the sensitive-string scan + positive-claim drift scan over the changed files per `OPERATOR_PUBLIC_BETA_URLS_FILL_RESULT.md §10`.

---

## 5. Re-running this milestone

This milestone's approval line is reusable. Once the operator has real URLs in hand, re-run:

> "I approve DeOpt V2 operator public beta URLs fill for this run."

The milestone will re-do Phase A discovery (looking for the same candidate paths + env vars), and if the URLs are now PRESENT, substitute them per Phase B. Partial substitution is supported — `isPlaceholderHref()` degrades per slot.

---

## 6. Launch impact

A public announcement CAN technically go out with all 6 frontend slots showing "(coming soon)" — the UI degrades cleanly. Whether that announcement is worth sending is an operator call. The launch checklist (`docs/public-beta/PUBLIC_TESTNET_BETA_LAUNCH_CHECKLIST.md §1.5b`) leaves rows 1-11 as ☐ until the operator either fills the URLs or explicitly documents in this doc which slots will ship as placeholders for the initial announcement.

---

**End of operator public beta URLs remaining actions.**
