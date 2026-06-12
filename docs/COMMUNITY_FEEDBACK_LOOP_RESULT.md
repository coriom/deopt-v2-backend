# COMMUNITY-FEEDBACK-LOOP — Result

**Date executed:** 2026-06-12
**Operator approval line accepted (verbatim):**
> "I approve DeOpt V2 community feedback loop preparation for this run."

**Brief:** `deopt-v2-backend/docs/COMMUNITY_FEEDBACK_LOOP_NEXT_TASK.md`.

**Posture:** **Docs + frontend link-config polish only. Zero chain transactions. Zero broadcast. Zero mainnet. Zero backend `.env` edit. Zero private key handling. Zero invented URLs. Zero claims of "audited" / "mainnet-ready" / "production" / "safe for real funds".**

---

## 1. Workspace

* `~/DEOPT/deopt-v2-frontend/src/lib/public-beta-links.ts` (link-config refactor only)
* `~/DEOPT/deopt-v2-backend/docs/public-beta/` (6 new docs + 2 doc updates)
* `~/DEOPT/RUN_STATE.md` (closure paragraph)

Public-safe values only. No invented Discord / GitHub / form URLs. Placeholders deliberately kept so absence stays visible.

## 2. Feedback channel inventory (Phase A)

| Source | State before this milestone |
|---|---|
| `deopt-v2-frontend/src/lib/public-beta-links.ts` | 6 placeholder tokens: `PUBLIC_BETA_QUICKSTART_URL`, `PUBLIC_BETA_TESTING_GUIDE_URL`, `PUBLIC_BETA_LIMITATIONS_URL`, `PUBLIC_BETA_FEEDBACK_URL`, `PUBLIC_BETA_DISCORD_URL`, `PUBLIC_BETA_GITHUB_URL` |
| `docs/public-beta/FEEDBACK_AND_BUG_REPORTING.md` | 4 doc-side tokens: `{{ GITHUB_REPO_URL }}`, `{{ DISCORD_INVITE_URL }}`, `{{ TELEGRAM_INVITE_URL }}`, `{{ FEEDBACK_FORM_URL }}` |
| `docs/public-beta/FAQ.md` | Same 4 doc-side tokens in closing contact section |
| `docs/public-beta/README.md` | `{{APP_URL}}` placeholder |
| `docs/public-beta/DEVELOPER_API_GUIDE.md` | `{{ API_BASE_URL }}` placeholder |
| Operator / environment | No real URLs available |
| `~/DEOPT/private/**` | NOT read |

Conclusion: real URLs are not available. Per brief: do not invent. Keep placeholders, centralize tokens, document substitution path.

## 3. Frontend public-beta link wiring (Phase B)

Changes to `src/lib/public-beta-links.ts`:

* Added `PublicBetaLinkStatus = "placeholder" | "live"` discriminator.
* Extended `PublicBetaLink` with `status: PublicBetaLinkStatus` and `operatorFillToken: string` so the operator can find the token + the slot in one place.
* Strengthened `isPlaceholderHref()` to also treat empty / nullish hrefs as placeholders (defence-in-depth: a partial substitution that leaves an empty string still degrades to "coming soon").
* Added `pendingPlaceholderCount(): number` so operator tooling can surface "N URLs still pending" without re-parsing the module.
* Updated the module header comment to point to the new `docs/public-beta/OPERATOR_PUBLIC_BETA_URLS_FILL.md` substitution checklist.
* Hrefs themselves: **unchanged** — all six entries remain on their `{{PUBLIC_BETA_*_URL}}` tokens. No fake URLs invented.
* No admin URL added. No bearer token added. No localhost-as-public-launch URL added. No private RPC URL added.

Backward compatibility: existing consumers — `PublicBetaFooter.tsx`, `SigningStateModal.tsx` — only read `id`, `label`, `href`, `description`. They are untouched and still pass. `isPlaceholderHref()` still returns `true` for every current entry.

## 4. Bug report template (Phase C)

New: `docs/public-beta/BUG_REPORT_TEMPLATE.md`.

Required fields collected:
* Issue title
* Test scenario
* Severity guess (P0 / P1 / P2 / P3)
* Wallet **public** address (Base Sepolia)
* Network + chain id seen by the app
* Timestamp (UTC)
* Tx hash (if any)
* Was on Base Sepolia? — expected `yes`
* Real funds involved? — expected `no`
* Browser + version
* Wallet provider + version
* OS
* App URL / page
* Steps to reproduce
* Expected behaviour
* Actual behaviour
* Console errors (if safe — explicit redact-secrets instruction)
* Screenshots / short video

Explicit safety rules in §1:
* NEVER share private key.
* NEVER share seed phrase / mnemonic.
* NEVER share RPC URL with embedded API key.
* NEVER share admin bearer token.
* NEVER share `.env`, `secrets.json`, AWS credentials.

§4 reporting checklist enforces the safety rules before submit.

## 5. Feedback triage workflow (Phase D)

New: `docs/public-beta/FEEDBACK_TRIAGE_WORKFLOW.md`.

§1 Intake channels: GitHub, Discord, Telegram, feedback form, private security inbox. Cadences: 3 business days / continuous best-effort / weekly / 1 business day.

§2 Classification: `ux`, `frontend-bug`, `backend-bug`, `contract-issue`, `docs-issue`, `feature-request`, `market-maker / liquidity`, `security-concern`.

§2 Severity: P0 blocks all testing / P1 serious subset / P2 normal / P3 nice-to-have. P0 + security pages on-call.

§3 Reproduction requirements per category (frontend, backend, contract).

§4 Workflow per channel — GitHub issue / chat / form.

§5 Sensitive-data intake rules — what to redact, what to hide, how to escalate a credential leak.

§6 Escalation to security review — private path, internal tracking, advisory note workflow.

§7 When to pause the beta: P0 unresolved 24h, security-concern of drain-class open, `/trading/health` unhealthy > 1h, chain / indexer / frontend inconsistency without recovery path.

§8 Internal log requirements (NOT in the public repo).

## 6. Community onboarding (Phase E)

New: `docs/public-beta/COMMUNITY_ONBOARDING.md`.

* §1 What DeOpt V2 is (3-repo stack).
* §2 How to join (5 steps, no invite, no KYC, no purchase).
* §3 What testers can try (browse / quote / intent / sign / submit / watch lifecycle / portfolio); includes canonical first-trade reference `0x748c9484…` block `42750521`.
* §4 What testers should NOT do (no real funds, no mainnet wallet bypass, no private-key sharing, no production claim, no bounty gaming).
* §5 Getting testnet ETH + mUSDC (faucet recommendations; mUSDC operator-mint path + future public faucet).
* §6 How to report bugs — links to template + triage workflow + private security path.
* §7 What feedback is most valuable (reproducible reverts, wallet-specific failures, confusing UX, API ergonomics, misleading docs, stale-oracle false positives).
* §8 Disclaimers list (testnet, no real funds, unaudited, experimental, not mainnet-ready, feedback phase, community preview).
* §9 Operator promises (read every report, security ack in 1 BD, honest pause comms, never ask for keys) and non-promises (no fix guarantee, no SLA, no bounty, no address stability, no DB stability).

## 7. Launch checklist (Phase F)

New: `docs/public-beta/PUBLIC_TESTNET_BETA_LAUNCH_CHECKLIST.md`.

Pre-launch hard gates (§1), grouped:
* §1.1 Frontend (8 items): typecheck / lint / build / playwright catalog / banners visible / wrong-network / footer / sign-failure CTA / no `Authorization` header / no admin-test URL / no positive-claim drift / app reachable.
* §1.2 Backend (7 items): cargo test / clippy / fmt / `/trading/health` green / `chain_id=84532` / indexer caught up / admin endpoints token-gated / `.env` deployed-not-printed / no mainnet RPC in config.
* §1.3 Contracts (9 items): canonical ME + MarginEngine deployed + verified + bidirectional + OracleRouter `maxDelay=60s` + registry has active series + mUSDC registered + canonical Sepolia reference trace verifiable + all docs match + no mainnet contract printed positively.
* §1.4 Docs (15 items): every required doc present, including the 6 added in this milestone.
* §1.5 Community channels (10 items): GitHub public + issue templates + Security Advisories enabled, Discord with required channels + moderators, Telegram (optional), feedback form routing, real-URL substitution complete OR placeholders retained intentionally.
* §1.6 Operator readiness (7 items): rights, oracle-refresh runbook, testnet-reset runbook, pause plan, on-call coverage 48h, rollback path, pre-drafted comms.
* §1.7 Safety (8 items): zero bearer / RPC URL / DATABASE_URL in frontend, no private file committed, no mainnet wallet used, no mainnet contract deployed, public-beta vocabulary only.

Plus §2 post-launch within-48h actions, §3 explicit out-of-scope (audit, bounty, mainnet, Safe-tx, AWS/KMS, real liquidity), §4 hold reasons, §5 sign-off rule.

## 8. Announcement drafts (Phase G)

New: `docs/public-beta/PUBLIC_TESTNET_BETA_ANNOUNCEMENT_DRAFT.md`.

Drafts included:
* §1 Discord `#announcements` (long-form).
* §2 X / Twitter (single post, ≤ 280 chars).
* §3 X / Twitter (4-tweet thread).
* §4 LinkedIn (longer paragraph).
* §5 GitHub README banner (Markdown snippet).
* §6 Email to early testers (optional).
* §7 Pause / rollback announcement template (in case of P0).

Every draft:
* leads with "public testnet beta",
* uses "Base Sepolia (chain 84532)" + "mainnet disabled",
* states "no real funds",
* states "unaudited" / "not audited",
* asks for feedback,
* does NOT say "audited / mainnet-ready / production / safe for real funds / institutional".

§8 Honesty checklist — 10-box gate to run before posting any draft.

Bonus: new `docs/public-beta/OPERATOR_PUBLIC_BETA_URLS_FILL.md` — the substitution checklist all the drafts and the frontend module reference, with token-by-token inventory + safety rules (no bearer-in-URL, no RPC-key-in-URL, no localhost-as-public-launch, no mainnet-link, HTTPS only) + per-step verification commands.

## 9. Docs index updates (Phase H)

* `docs/public-beta/README.md`:
  * Added "Community feedback loop (2026-06-12)" section listing the 6 new docs (Community onboarding, Bug report template, Triage workflow, Launch checklist, Announcement drafts, Operator URLs-fill).
  * Updated launch-checklist item #3 to ✓ (docs ready; placeholders remain — cross-link to operator-fill).
* `docs/public-beta/FEEDBACK_AND_BUG_REPORTING.md`:
  * Channels section now cross-links to `OPERATOR_PUBLIC_BETA_URLS_FILL.md`.
  * §2 (bug template section) now points to the fuller `BUG_REPORT_TEMPLATE.md` as the "quick path".
  * New §10 "See also" linking the 6 new docs.

## 10. Frontend tests / build validations (Phase I)

| Command | Result |
|---|---|
| `npm run typecheck` (`tsc --noEmit`) | clean |
| `npm run lint` (`eslint`) | clean |
| `npm run build` (`next build`) | green, 9 routes prerendered |
| `npx playwright test --list` | 30 tests in 12 files, parse-clean |

No source changes outside `src/lib/public-beta-links.ts`. No new tests added — the existing `tests/e2e/public-beta-footer.spec.ts` + `tests/e2e/no-admin-bearer.spec.ts` already cover:
* footer renders all 6 link slots,
* placeholder hrefs are non-clickable spans,
* footer DOM contains no bearer / RPC URL / DATABASE_URL / private-key-shaped string,
* safety-copy bullets present on every route,
* no `Authorization` header from app runtime.

These specs continue to be valid against the refactored module (they exercise `isPlaceholderHref` semantics, not the new fields).

Targeted spec execution not performed in this sandbox — chromium needs `libnspr4.so` which is missing on the WSL2 image (same constraint as the previous milestone).

## 11. Docs created / updated

**Created (this milestone):**
* `deopt-v2-backend/docs/public-beta/BUG_REPORT_TEMPLATE.md`
* `deopt-v2-backend/docs/public-beta/FEEDBACK_TRIAGE_WORKFLOW.md`
* `deopt-v2-backend/docs/public-beta/COMMUNITY_ONBOARDING.md`
* `deopt-v2-backend/docs/public-beta/PUBLIC_TESTNET_BETA_LAUNCH_CHECKLIST.md`
* `deopt-v2-backend/docs/public-beta/PUBLIC_TESTNET_BETA_ANNOUNCEMENT_DRAFT.md`
* `deopt-v2-backend/docs/public-beta/OPERATOR_PUBLIC_BETA_URLS_FILL.md`
* `deopt-v2-backend/docs/COMMUNITY_FEEDBACK_LOOP_RESULT.md` (this doc)

**Edited (this milestone):**
* `deopt-v2-backend/docs/public-beta/README.md` (added new docs to index + checklist item ✓)
* `deopt-v2-backend/docs/public-beta/FEEDBACK_AND_BUG_REPORTING.md` (cross-link + §10 See also)
* `deopt-v2-backend/docs/PRODUCT_FREEZE_AND_SECURITY_REANCHOR_NEXT_TASK.md` (precondition section augmented — see §12)
* `~/DEOPT/RUN_STATE.md` (closure paragraph)

**Edited (frontend src):**
* `deopt-v2-frontend/src/lib/public-beta-links.ts` (status discriminator + operator-fill token + harder placeholder check + pendingPlaceholderCount helper)

**Not edited:**
* `deopt-v2-frontend/src/components/PublicBetaFooter.tsx`
* `deopt-v2-frontend/src/components/tx/SigningStateModal.tsx`
* `deopt-v2-frontend/tests/**` — existing specs still valid
* Backend Rust source — ZERO
* Solidity source — ZERO
* Backend `.env` — UNCHANGED (mtime `2026-06-08 16:55:05` preserved)
* `~/DEOPT/private/**` — NOT read, NOT committed

## 12. RUN_STATE update (Phase K)

Closure paragraph prepended to `RUN_STATE.md` documenting: 6 new public-beta docs, 2 docs edited, 1 frontend link-config refactor, zero backend Rust changes, zero Solidity changes, zero chain tx, zero `.env` edits, validations clean, placeholders intentionally retained per "do not invent" rule.

## 13. Files changed

**Added (docs):**
* `deopt-v2-backend/docs/public-beta/BUG_REPORT_TEMPLATE.md`
* `deopt-v2-backend/docs/public-beta/FEEDBACK_TRIAGE_WORKFLOW.md`
* `deopt-v2-backend/docs/public-beta/COMMUNITY_ONBOARDING.md`
* `deopt-v2-backend/docs/public-beta/PUBLIC_TESTNET_BETA_LAUNCH_CHECKLIST.md`
* `deopt-v2-backend/docs/public-beta/PUBLIC_TESTNET_BETA_ANNOUNCEMENT_DRAFT.md`
* `deopt-v2-backend/docs/public-beta/OPERATOR_PUBLIC_BETA_URLS_FILL.md`
* `deopt-v2-backend/docs/COMMUNITY_FEEDBACK_LOOP_RESULT.md`

**Edited (docs):**
* `deopt-v2-backend/docs/public-beta/README.md`
* `deopt-v2-backend/docs/public-beta/FEEDBACK_AND_BUG_REPORTING.md`
* `deopt-v2-backend/docs/PRODUCT_FREEZE_AND_SECURITY_REANCHOR_NEXT_TASK.md`
* `~/DEOPT/RUN_STATE.md`

**Edited (frontend src):**
* `deopt-v2-frontend/src/lib/public-beta-links.ts`

## 14. Validations

| Check | Result |
|---|---|
| `git diff --check` (frontend) | clean |
| `git diff --check` (backend) | clean |
| Sensitive-string scan on changed files | zero hits (no bearer / RPC URL / private key / DATABASE_URL) |
| Positive-claim scan ("is audited / production-ready / mainnet-ready / safe for real funds / guaranteed") | zero hits (only negative-framed uses, intentional) |
| `.env` mtime | preserved `2026-06-08 16:55:05` |
| Private file (`~/DEOPT/private/**`) mode | 600 preserved; NOT read; NOT committed |
| Admin bearer token in frontend code/tests | NONE (footer + sign-failure-CTA tests still enforce) |
| Mainnet RPC referenced | NO |
| Mainnet chain id 8453 used positively | NO — only as hard-stop marker |
| Chain transaction sent | NO |
| Broadcast invoked | NO |
| Real wallet used | NO |
| Source changes outside `deopt-v2-frontend/src/lib/public-beta-links.ts` and docs / RUN_STATE | NO |
| Backend Rust source changes | NONE |
| Solidity source changes | NONE |
| Invented URLs added to placeholders | NO — all 6 frontend slots retain `{{PUBLIC_BETA_*_URL}}` tokens |

## 15. Remaining placeholders

| Token | Where | Why kept |
|---|---|---|
| `PUBLIC_BETA_QUICKSTART_URL` | `src/lib/public-beta-links.ts` | hosted docs URL not yet known |
| `PUBLIC_BETA_TESTING_GUIDE_URL` | same | same |
| `PUBLIC_BETA_LIMITATIONS_URL` | same | same |
| `PUBLIC_BETA_FEEDBACK_URL` | same | feedback form not yet stood up |
| `PUBLIC_BETA_DISCORD_URL` | same | Discord server not yet created |
| `PUBLIC_BETA_GITHUB_URL` | same | public GitHub mirror URL not provided |
| `{{ GITHUB_REPO_URL }}` | `FEEDBACK_AND_BUG_REPORTING.md`, `FAQ.md` | same as above (doc-side alias) |
| `{{ DISCORD_INVITE_URL }}` | same | same |
| `{{ TELEGRAM_INVITE_URL }}` | same | Telegram channel not configured |
| `{{ FEEDBACK_FORM_URL }}` | same | feedback form not yet stood up |
| `{{APP_URL}}` | `README.md` | hosted app URL not yet known |
| `{{ API_BASE_URL }}` | `DEVELOPER_API_GUIDE.md` | publicly-callable API URL not yet known |

All placeholders survive `isPlaceholderHref()` check; the frontend footer + sign-failure CTA degrade gracefully to "coming soon" hints.

## 16. Next milestone recommendation

**Primary:** `PRODUCT_FREEZE_AND_SECURITY_REANCHOR` — re-anchor the frozen ABI manifest + draft the public-facing security-review packet (6 docs). The brief explicitly says do not block this on Discord/Form URLs being pending, and the placeholders here are well-contained.

**Optional first:** `OPERATOR_PUBLIC_BETA_URLS_FILL` — if the operator has the live channels ready and wants to flip placeholders to live URLs before any further milestones. Quick win (one frontend file + a docs sweep) but not technically required to proceed.

Out-of-scope explicitly retained: mainnet activation, external-audit kickoff, bug-bounty launch, Safe-tx multisig flows, AWS / KMS / production signer cutover.

Milestone outcome: 6 new public-beta docs (bug-report template, triage workflow, community onboarding, launch checklist, announcement drafts, operator URLs-fill checklist) plus a hardened frontend link config — all without inventing fake URLs, all without exposing private values, all positioned as community-preview testnet beta. The frontend public-beta footer + sign-failure-modal "Report this issue" CTA will auto-promote placeholder slots into live anchors as the operator fills tokens via `OPERATOR_PUBLIC_BETA_URLS_FILL.md`. The launch checklist + announcement drafts are ready for operator sign-off once the channels exist.

**End of COMMUNITY-FEEDBACK-LOOP result.**
