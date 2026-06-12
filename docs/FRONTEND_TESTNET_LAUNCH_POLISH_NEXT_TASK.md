# FRONTEND-TESTNET-LAUNCH-POLISH — Next Task Brief

**Date written:** 2026-06-12
**Origin:** `PUBLIC_BETA_DOCS_PACK_NEXT_TASK.md` + `docs/public-beta/README.md` launch checklist item #7.
**Target:** prepare the DeOpt V2 frontend for a small-community public testnet beta on Base Sepolia.
**Posture:** **Frontend-only. NEVER mainnet. NEVER chain transactions outside the existing wallet-initiated flow. NEVER private signer / AWS / KMS. NEVER `.env` edit on the backend. NEVER source code changes that reach the contract surface.**

> **This task is NOT executed by the calling milestone. It packages frontend polish into one approval-gated milestone.**

---

## 1. Literal operator approval line (REQUIRED, VERBATIM)

> "I approve DeOpt V2 frontend testnet launch polish for this run."

Properties:
* Authorises frontend code changes (TypeScript, React components, copy, banners, tests).
* Does NOT authorise backend `.env` edits, contract changes, or chain transactions outside the standard wallet flow.

---

## 2. Scope

* **Testnet banner.** Persistent "Testnet — Unaudited — Experimental" banner on every page, dismissible only for the current session.
* **Wrong-network detection.** Detect when the connected wallet is not on Base Sepolia (chain id `84532`) and show a non-dismissible blocker prompting "Switch to Base Sepolia".
* **Mainnet-no-go guard.** If the connected wallet is on chain id `8453` (Base mainnet), the app shows a hard-blocking modal explaining "DeOpt V2 is not deployed on mainnet" and refuses to proceed.
* **Copy review.** Pass through all UI strings; replace any wording that could imply "production" / "audited" / "safe for real funds" with the public-beta vocabulary in `docs/public-beta/`.
* **Lifecycle UX.** Confirm the trade-status page polls the lifecycle endpoint and updates in real time. Add a "Last refreshed at" footer for clarity.
* **Stale-oracle handling.** When the quote-preview endpoint returns `partial` with `ORACLE_UNAVAILABLE`, the UI shows a friendly "Oracle is stale — wait for next refresh" message instead of a generic error.
* **Disabled-state polish.** "Submit Trade" button stays disabled until both signatures are present.
* **Wallet-disconnect resilience.** If the wallet disconnects mid-flow, the UI surfaces a clear "Reconnect wallet" prompt instead of silently breaking.
* **No-broadcast guard.** Verify the frontend never sends an `executeTrade` calldata from the connected wallet (the executor does that off the trader's wallet).
* **Playwright update.** Update existing Playwright specs (`tx-status-cycler`, `no-admin-bearer`) and add at least two new specs covering the testnet-banner and the wrong-network blocker.

## 3. Out of scope

* No wallet-broadcast flow changes that bypass the executor.
* No mainnet-specific branches.
* No backend changes.
* No new contracts.
* No production styling overhaul (icon redesign, typography theme, etc. — out of scope; polish only).

---

## 4. Hard preconditions

| # | Precondition | Verifying check |
|---|---|---|
| P1 | Approval line (§1) present verbatim | grep |
| P2 | Backend reachable at the configured `NEXT_PUBLIC_API_BASE_URL` | `curl /trading/health` |
| P3 | `docs/public-beta/CONTRACT_ADDRESSES_BASE_SEPOLIA.md` exists and lists the retargeted ME | read |
| P4 | `.env` (`deopt-v2-backend/.env`) untouched | `stat -c '%y'` |
| P5 | Private file untouched | `stat -c '%a %y'` |

---

## 5. Forbidden

* No mainnet (chain id `8453`) appearing in any non-block-state code path.
* No backend `.env` edit.
* No private key in source.
* No claim "audited" / "mainnet-ready" / "production" / "safe for real funds" in any UI string.

---

## 6. Acceptance criteria

* Banner visible on every page.
* Wrong-network blocker triggers correctly when wallet switches to anything other than `84532`.
* Mainnet blocker triggers when wallet switches to `8453`.
* Lifecycle page updates in real time without manual refresh.
* `npm run typecheck`, `npm run lint`, `npm run build`, `npm run playwright` (chromium-only, headless) all green.
* `git diff --check` clean.

---

## 7. Cross-links

* `docs/public-beta/README.md` (launch checklist)
* `docs/public-beta/USER_TESTING_GUIDE.md`
* `docs/public-beta/KNOWN_LIMITATIONS_AND_RISKS.md`
* `~/DEOPT/RUN_STATE.md`

**End of frontend testnet launch polish next-task brief.**
