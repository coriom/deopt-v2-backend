# DeOpt V2 — Public Testnet Beta Launch — Remaining Actions

> **Public testnet beta. Base Sepolia only. No real funds. Unaudited.** Operator-facing remaining-actions list produced by the `PUBLIC_TESTNET_BETA_LAUNCH_PREFLIGHT` milestone (2026-06-13).
>
> **Updated 2026-06-13 (post-`FRONTEND_INTEGRATED_DOCS_AND_FEEDBACK`):** 5 of the original 3 hard blockers + 3 recommended placeholders are now CLOSED via internal frontend routes. **Only `{{APP_URL}}` remains as a hard blocker.** Once the operator stands up the app hosting URL, re-run `PUBLIC-TESTNET-BETA-LAUNCH-PREFLIGHT` per `PUBLIC_TESTNET_BETA_LAUNCH_PREFLIGHT_RERUN_NEXT_TASK.md` and the verdict should flip to READY.
>
> **Verdict from preflight: NOT READY.** The frontend + docs + community-feedback infrastructure are ready, but the operator-side URLs that the public announcement depends on are not yet live.

---

## 1. Why we're NOT READY (in one sentence each)

* **App URL is missing.** The announcement copy points at `{{APP_URL}}`; without a publishable, operator-hosted URL, the public has nowhere to go.
* ~~**Feedback channel AND GitHub URLs are both still placeholder.**~~ **CLOSED 2026-06-13** via `FRONTEND_INTEGRATED_DOCS_AND_FEEDBACK`: feedback now resolves to the internal `/feedback` route (copy-to-clipboard bug-report template + Discord + GitHub CTAs); GitHub now resolves to `https://github.com/DeOpt`.

Everything else either (a) is already live (Discord, `/docs/quickstart`, `/docs/testing-guide`, `/docs/limitations`, `/docs/faq`, `/feedback`, GitHub), or (b) is NOT_REQUIRED_FOR_LAUNCH (API base URL, status page).

---

## 2. The 3 blocker actions

### 2.1 Stand up the app at a publishable URL

| Field | Value |
|---|---|
| Token to fill | `{{APP_URL}}` (doc-side; no frontend slot for this slot since the frontend IS the app) |
| What's needed | A publicly resolvable HTTPS URL the operator controls, pointing at the deployed Next.js build of `deopt-v2-frontend`. |
| Acceptance | The URL loads the landing page; `Public testnet beta` banner is visible; sticky public-beta footer renders; `/markets` route is reachable; `/portfolio` is reachable. |
| Safety rules | HTTPS only. No bearer / API key / RPC URL in the URL. No localhost. No mainnet wording. The app must be the latest tagged build from the `deopt-v2-frontend` repo at the moment of preflight re-run. |
| NOT this milestone | Deploying contracts. Sending transactions. Touching mainnet. Production signer cutover. |

Suggested hosts: any Vercel / Netlify / Cloudflare Pages / S3+CloudFront / operator-owned static host. The build is `next build` + `next start` (Node-runtime) per `package.json`. The operator decides; the announcement just needs a URL.

### 2.2 Stand up a feedback channel URL

| Field | Value |
|---|---|
| Token to fill | `PUBLIC_BETA_FEEDBACK_URL` (frontend slot) + `{{ FEEDBACK_FORM_URL }}` (doc-side alias) |
| What's needed | EITHER (a) a public bug-report form (Tally / Google Forms / Typeform) routed to a monitored inbox, OR (b) the public GitHub Issues URL of the public mirror repo with `bug-report` + `feature-request` issue templates configured. |
| Acceptance | The URL is publicly reachable, returns 200, accepts submissions (form) or accepts new issues (GitHub), the destination inbox / channel is being monitored at least weekly per `FEEDBACK_TRIAGE_WORKFLOW.md §1`. |
| Safety rules | Form submissions must not collect secrets. Do not embed an admin bearer in the URL. Do not redirect to a private operator dashboard. |
| NOT this milestone | Building a custom form. Setting up Linear / Jira / etc. The lightest path that satisfies acceptance is fine. |

### 2.3 Stand up the GitHub repo URL

| Field | Value |
|---|---|
| Token to fill | `PUBLIC_BETA_GITHUB_URL` (frontend slot) + `{{ GITHUB_REPO_URL }}` (doc-side alias) |
| What's needed | The public GitHub URL of the DeOpt V2 monorepo OR a public mirror of it. The README must carry the GitHub README banner from `docs/public-beta/PUBLIC_TESTNET_BETA_ANNOUNCEMENT_FINAL_DRAFT.md §5`. GitHub Security Advisories must be ENABLED. |
| Acceptance | The URL is publicly reachable. `bug-report` and `feature-request` issue templates exist. Security Advisories is on. Public README has the testnet-beta banner. |
| Safety rules | No `.env` committed. No private file committed. No bearer / RPC URL with key / DATABASE_URL anywhere in the repo. The pre-existing sensitive-string discipline from every milestone applies. |
| NOT this milestone | Public-relations launch, write-up post, contributor program. |

### 2.4 (Recommended; nice-to-have) Stand up hosted docs

* Tokens: `PUBLIC_BETA_QUICKSTART_URL`, `PUBLIC_BETA_TESTING_GUIDE_URL`, `PUBLIC_BETA_LIMITATIONS_URL`, `{{APP_URL}}` (covers app), `{{ API_BASE_URL }}` (optional — only if API-only integrators are part of the audience).
* If the GitHub repo URL (§2.3) is live, the public-beta docs can be linked directly to the repo paths (`<github-url>/blob/main/deopt-v2-backend/docs/public-beta/BASE_SEPOLIA_QUICKSTART.md`). That's a valid substitution and unblocks the announcement without a separate docs host.
* A dedicated docs host (mkdocs / Docusaurus / GitBook / Notion) is nicer but NOT REQUIRED for the initial public testnet beta launch.

### 2.5 (Not blocking) API base URL

* If the announcement audience includes API integrators, the operator should also supply `{{ API_BASE_URL }}` pointing at the publicly-callable backend's `/trading/health` root. The frontend bundles its own backend URL at build time (via `NEXT_PUBLIC_TRADING_API_BASE_URL`), so the app does NOT need this token to function.
* Mark NOT_REQUIRED_FOR_LAUNCH unless an API-integrator section is added to the announcement.

---

## 3. Exact operator workflow to flip the verdict to READY

1. Stand up the app at `{{APP_URL}}` per §2.1.
2. Stand up the feedback channel per §2.2.
3. Stand up the GitHub repo URL per §2.3.
4. (Recommended) Substitute the docs tokens per §2.4 — pointing them at the GitHub repo paths is valid.
5. Re-run **`OPERATOR-PUBLIC-BETA-URLS-FILL`** with its literal approval line:
   > "I approve DeOpt V2 operator public beta URLs fill for this run."
   * The milestone re-checks `public-beta-links.ts` + the doc-side tokens, substitutes the now-supplied URLs, flips `status: "live"`, re-runs the build + lint + typecheck + Playwright catalog, and updates `OPERATOR_PUBLIC_BETA_URLS_FILL_RESULT.md`.
6. Re-run **`PUBLIC-TESTNET-BETA-LAUNCH-PREFLIGHT`** (this milestone) with its literal approval line:
   > "I approve DeOpt V2 public testnet beta launch preflight for this run."
   * The milestone re-evaluates the verdict. With the §2.1-§2.3 actions complete, the verdict should flip to **READY** or **READY WITH NON-BLOCKING PLACEHOLDERS** (the latter if §2.4 docs hosting is deferred).
   * On READY, the milestone creates `PUBLIC_TESTNET_BETA_LAUNCH_NEXT_TASK.md` with the publication approval line.

Then and only then run **`PUBLIC-TESTNET-BETA-LAUNCH`** with its own separate, explicit approval line:
> "I approve DeOpt V2 public testnet beta launch publication for this run."

The publication milestone uses `docs/public-beta/PUBLIC_TESTNET_BETA_ANNOUNCEMENT_FINAL_DRAFT.md` after substituting the live URLs into the tokens.

---

## 4. What this remaining-actions doc does NOT do

This document is a checklist; it does **NOT** perform any of the following:

* Send transactions.
* Touch mainnet.
* Deploy contracts.
* Create Safe transactions.
* Call AWS / KMS.
* Call the production signer.
* Edit backend `.env`.
* Publish any announcement.
* Engage an audit firm.
* Launch a bug bounty.

If at any point in completing §2 the operator catches themselves about to do any of the above: **stop**, drop back to the security-reanchor packet, and re-scope.

---

## 5. Safety rules reminder (per slot)

Before pasting any URL into any token:

* **No bearer / API key / token in the URL string.**
* **No RPC URL with a key segment** (`/v2/<key>`).
* **No localhost URL announced as public.** Use `local_dev_only` status (in the link config) if the slot is genuinely dev-only.
* **No mainnet-related URL.** All Basescan links must point at `sepolia.basescan.org`.
* **No internal operator dashboard link.** `/admin/*` routes are not for public announcement.
* **HTTPS only.**
* **No tracking / analytics tokens** that expose internal naming.

The pre-existing `docs/public-beta/OPERATOR_PUBLIC_BETA_URLS_FILL.md` has the full safety + verification commands. This remaining-actions doc references those rules; it does not replace them.

---

## 6. Risk if launch is attempted now

If the operator chose to publish the announcement at this exact moment (before §2.1-§2.3 close), the consequences would be:

* The `{{APP_URL}}` placeholder would be visible in the post → reputational damage and a hard signal to readers that the team isn't ready.
* No working bug-report channel beyond Discord chat → first wave of bugs becomes chat-history-only with no triage trail (see `FEEDBACK_TRIAGE_WORKFLOW.md §4.2` on the value of GitHub issues as the system of record).
* No GitHub repo URL → integrators can't read the source, can't open issues, can't file security advisories privately.
* Discord would still work, but the operator would be effectively running the entire feedback loop through one channel, against the brief's intent.

These are NOT showstoppers for the protocol — chain state remains testnet-only and safe — but they ARE showstoppers for a credible public launch.

---

## 7. When to consider partial launch

Partial / soft launch is an option if the operator wants to start gathering signal earlier:

* **Soft launch (Discord-only, no public post):** open Discord to a small, hand-picked set of testers using the live Discord URL. No external post. The frontend can be reached via a Discord-only-shared `{{APP_URL}}` once stood up. The remaining placeholders stay placeholder.
* **Hard launch (the announcement copy in `PUBLIC_TESTNET_BETA_ANNOUNCEMENT_FINAL_DRAFT.md`):** requires §2.1-§2.3 complete.

A soft launch does NOT require this preflight milestone to flip to READY; it requires only §2.1 (app URL) and operator judgment. A hard launch requires the full flip.

---

## 8. Cross-links

* `docs/PUBLIC_TESTNET_BETA_LAUNCH_PREFLIGHT_RESULT.md` (this milestone's full report).
* `docs/public-beta/PUBLIC_TESTNET_BETA_LAUNCH_CHECKLIST.md` (operator pre-launch checklist with the V3 evidence row + this preflight's verdict update).
* `docs/public-beta/PUBLIC_TESTNET_BETA_ANNOUNCEMENT_FINAL_DRAFT.md` (publish-ready copy, NOT YET PUBLISHED).
* `docs/public-beta/OPERATOR_PUBLIC_BETA_URLS_FILL.md` (URL-fill procedure + safety rules).
* `docs/OPERATOR_PUBLIC_BETA_URLS_FILL_RESULT.md` (last URL-fill milestone result — Discord live, others placeholder).
* `docs/OPERATOR_PUBLIC_BETA_URLS_REMAINING_ACTIONS.md` (the per-slot operator action list).
* `~/DEOPT/RUN_STATE.md` (macro execution state).

**End of public testnet beta launch — remaining actions.**
