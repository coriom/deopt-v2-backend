# DeOpt V2 — Public Testnet Beta Launch Checklist

> **Public testnet beta. Base Sepolia only. No real funds. Unaudited. Experimental.** Operator-facing checklist for going from "docs and code are ready" to "I am willing to send a public announcement".

This is NOT a mainnet launch checklist. This is a **public-testnet-beta launch** checklist. Mainnet activation gates on the security re-anchor + external audit, separately.

---

## 1. Pre-launch hard gates

Every box below must be ✓ before posting the public announcement. A single ☐ is a hold.

### 1.1 Frontend

* ☐ `npm run typecheck` clean (`tsc --noEmit`).
* ☐ `npm run lint` clean.
* ☐ `npm run build` (next build) green.
* ☐ `npx playwright test --list` parses cleanly.
* ☐ Testnet / unaudited banner visible on every trading route.
* ☐ Mainnet-disabled banner triggers on chain id `8453` and shows "Switch to Base Sepolia".
* ☐ Wrong-network banner triggers on non-expected, non-mainnet chains.
* ☐ Public-beta footer renders on every trading route.
* ☐ Footer link slots present (placeholders OR real URLs both fine; visible is what matters).
* ☐ Sign-failure modal surfaces a "Report this issue" CTA.
* ☐ No `Authorization` header in any client XHR (enforced by `tests/e2e/no-admin-bearer.spec.ts`).
* ☐ No admin-test fixture URL fetched from the browser runtime.
* ☐ No "audited / production-ready / mainnet-ready / safe for real funds" string in any user-facing UI copy.
* ☐ App is reachable at a publishable URL (operator-hosted; not localhost).

### 1.2 Backend

* ☐ `cargo test` green on the working branch.
* ☐ `cargo clippy -- -D warnings` green.
* ☐ `cargo fmt --check` green.
* ☐ `/trading/health` returns `200 ok` against the deployed backend.
* ☐ `chain_id` in `/trading/health` is `84532` (Base Sepolia). NOT `8453`.
* ☐ Indexer is caught up (`indexer_lag_blocks` ≤ a few dozen).
* ☐ Backend admin endpoints (`/admin/*`) are not reachable without bearer.
* ☐ Backend `.env` is the deployed config — NOT printed, NOT committed.
* ☐ Backend has no mainnet RPC URL anywhere in config.

### 1.3 Contracts (Solidity, Base Sepolia)

* ☐ Canonical `OptionMatchingEngine` `0x5a5EBF9A9CCd7c012518569DE8283982982670f6` deployed + verified.
* ☐ Canonical `MarginEngine` `0x506cD65a63C53c66ab572B9f9dd819B7BfE00D30` deployed + verified.
* ☐ Bidirectional wiring: ME ↔ MarginEngine both authorise the other.
* ☐ `OracleRouter` `0xb416406f200b2ef3d7a86a5d5877ed41d9b1a581` `maxDelay = 60s` configured.
* ☐ `OptionProductRegistry` `0x3d52b033fab00ed6104dd3bc0a715f8648344eca` has at least one active product + series.
* ☐ mUSDC `0x6eAe407f5640B006faC9965182e238582A3B412E` collateral registered in `CollateralVault`.
* ☐ Successful Sepolia reference trade verifiable on Basescan: `0x748c94843cb4cbe31f56c84ceedc7e000a05dac567fa3fe7a1415a0de59b637a`.
* ☐ All addresses match what `docs/public-beta/CONTRACT_ADDRESSES_BASE_SEPOLIA.md` says.
* ☐ No mainnet contract address printed anywhere except as hard-stop / negative reference.

### 1.4 Docs

* ☐ `docs/public-beta/README.md` index up to date.
* ☐ `PUBLIC_TESTNET_BETA_OVERVIEW.md` published.
* ☐ `BASE_SEPOLIA_QUICKSTART.md` published.
* ☐ `USER_TESTING_GUIDE.md` published.
* ☐ `DEVELOPER_API_GUIDE.md` published.
* ☐ `CONTRACT_ADDRESSES_BASE_SEPOLIA.md` published.
* ☐ `KNOWN_LIMITATIONS_AND_RISKS.md` published.
* ☐ `FEEDBACK_AND_BUG_REPORTING.md` published.
* ☐ `FAQ.md` published.
* ☐ `BUG_REPORT_TEMPLATE.md` published (this milestone).
* ☐ `FEEDBACK_TRIAGE_WORKFLOW.md` published (this milestone).
* ☐ `COMMUNITY_ONBOARDING.md` published (this milestone).
* ☐ `PUBLIC_TESTNET_BETA_LAUNCH_CHECKLIST.md` published (this doc).
* ☐ `PUBLIC_TESTNET_BETA_ANNOUNCEMENT_DRAFT.md` published (this milestone).
* ☐ `OPERATOR_PUBLIC_BETA_URLS_FILL.md` published (this milestone) — checklist for swapping placeholders into real URLs.

### 1.5 Community channels

* ☐ GitHub repo (or public mirror) is publicly accessible.
* ☐ GitHub issue templates for `bug-report` and `feature-request` exist.
* ☐ GitHub Security Advisories are ENABLED on the repo.
* ☐ Discord server created with at minimum: `#announcements`, `#general`, `#bug-reports`, `#dev-integrations`, `#oracle-status`. Channel descriptions set. Moderators appointed.
* ☐ Telegram channel (optional) created and mirrored to `#announcements`.
* ☐ Feedback form (Tally / Google Forms / Typeform) configured with bug-report fields. Output routed to a monitored inbox.
* ☐ Security disclosure path documented in `FEEDBACK_AND_BUG_REPORTING.md §7`.
* ☐ Real URLs swapped into `deopt-v2-frontend/src/lib/public-beta-links.ts` (or placeholder slots intentionally retained — see `OPERATOR_PUBLIC_BETA_URLS_FILL.md`).
* ☐ Footer placeholders match the doc placeholders (consistent token names).
* ☐ Initial welcome message posted in each channel.

### 1.6 Operator readiness

* ☐ Operator has the rights to create / configure all the channels above.
* ☐ Operator has a tested oracle-refresh runbook (see internal `SEPOLIA_SETUP_FIXES_PACK_*` docs + `~/DEOPT/TESTNET_RUNBOOK.md`).
* ☐ Operator has a tested testnet-reset runbook (database, indexer, contract redeploy).
* ☐ Operator has a pause-and-communicate plan (see `FEEDBACK_TRIAGE_WORKFLOW.md §7`).
* ☐ Operator has on-call coverage for the first 48 hours after announcement.
* ☐ Operator has a rollback path: revert frontend deploy, banner-level "down for maintenance" notice, public Discord announcement.
* ☐ Pause / rollback comm draft prepared in advance (see `PUBLIC_TESTNET_BETA_ANNOUNCEMENT_DRAFT.md`).

### 1.7 Safety

* ☐ Zero admin bearer tokens in frontend bundle.
* ☐ Zero private RPC URL in frontend bundle.
* ☐ Zero DATABASE_URL anywhere in frontend.
* ☐ No private file (`~/DEOPT/private/**`) committed.
* ☐ No mainnet wallet ever used in any test or integration.
* ☐ No mainnet contract deployed under the DeOpt branding.
* ☐ All public copy uses only public-beta vocabulary — testnet, unaudited, experimental, community preview, feedback phase. NOT production, NOT audited, NOT mainnet-ready, NOT safe for real funds, NOT institutional.

---

## 2. Post-launch (within 48h)

* ☐ Daily triage of GitHub issues, Discord `#bug-reports`, and feedback-form inbox.
* ☐ First public "beta status update" post in `#announcements` (+ Telegram mirror) within 7 days.
* ☐ Acknowledge the first 10 reporters individually (encourages a feedback culture).
* ☐ Track P0 / P1 incident count + median time-to-acknowledge.
* ☐ Audit announcement copy in third-party repostings — correct any drift toward "audited / mainnet-ready / production / safe for real funds".

---

## 3. What is intentionally NOT on this checklist

These are explicitly out of scope for the public testnet beta launch:

* External audit kickoff. Comes after `PRODUCT_FREEZE_AND_SECURITY_REANCHOR`.
* Bug bounty program. Comes later, separately.
* Mainnet deployment. Gates on external audit + post-M-P7 closure.
* Safe-tx multisig wiring. Operator-side; not a tester-facing concern.
* AWS / KMS / production signer integration. Same.
* Real liquidity / market-maker partnerships. Not a beta concern.

If any of these creep into the launch announcement language, pull the announcement and redraft.

---

## 4. Hold reasons

A launch MUST be held (not just delayed) if any of these are true at announcement time:

* Backend `/trading/health` is not `ok`.
* No verifiable Sepolia reference trade exists.
* Frontend banner copy implies the protocol is audited or mainnet-ready.
* Any private credential (private key / bearer / RPC URL with key / DATABASE_URL) has leaked into the public repos or docs.
* No moderation coverage in the chat channels.
* No security disclosure path is documented + open.

Holding the launch is the responsible choice. The public testnet beta is more valuable launched honestly than launched fast.

---

## 5. Sign-off

When all of §1 boxes are ✓, the operator signs off in writing (an internal note or a closed GitHub issue) and proceeds to post the announcement using one of the drafts in `PUBLIC_TESTNET_BETA_ANNOUNCEMENT_DRAFT.md`.

If a box is ☐: do not announce.

---

**End of public testnet beta launch checklist.**
