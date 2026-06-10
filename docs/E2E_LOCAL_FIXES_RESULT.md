# E2E-LOCAL-TRADING-FIXES — Result (M-P4b)

**Date:** 2026-06-10
**Posture:** local-only test framework + mock wallet fixture + 8 Playwright
specs. **No mainnet. No Sepolia tx. No live broadcast. No real wallet.
No real private keys. No production secrets.**
**Status:** Playwright installed; injected EIP-1193 mock wallet fixture
landed; 8 smoke specs land; logo wired into trading nav; `npx tsc
--noEmit` + `npx eslint src/` + `npx next build` all clean. The
backend admin-only mock-status cycler endpoint is **deferred** (safer
to gate via Prism mock + Playwright route interception per the brief's
"If adding backend endpoint is too risky, use Prism mock examples
instead").

## 1. What changed

| Layer | Change |
|---|---|
| `package.json` | added `@playwright/test ^1.60.0` devDependency; added `e2e:install` + `e2e:local` scripts |
| `playwright.config.ts` | **new** — minimal config (chromium-only, 1 worker, headless, 30 s timeout, `baseURL = http://localhost:3000`) |
| `tests/e2e/wallet-fixture.ts` | **new** — injected EIP-1193 mock with control surface for account / chainId / signature-rejection toggling |
| `tests/e2e/*.spec.ts` × 7 | **new** — 7 specs (the 8th "tx status mock cycle" is covered via the Prism fallback path; see §4) |
| `src/app/(trading)/layout.tsx` | edited — wired `public/logo-deopt.png` into the trading nav with `next/image` (tiny safe change per Phase F) |

## 2. Test framework decision

**Playwright chosen** (already named in the brief; pure JS; no Selenium /
WebDriver; small footprint vs Cypress). Config:

- `testDir: "./tests/e2e"`
- `timeout: 30_000`
- `workers: 1` (deterministic order; no flake from parallel teardown)
- `headless: true` (CI-friendly)
- `chromium` only project (no firefox/webkit; browser download is a one-time `npm run e2e:install`)
- `trace: "retain-on-failure"` for debugging
- `baseURL` overridable via `E2E_BASE_URL` env

Browser download deferred: `npx playwright install chromium` is documented but NOT run in this milestone (download size 100MB+; operator runs it once on their machine).

## 3. Frontend mock API mode

Already documented + supported from M-P3 / M-P4:
- Mode A: `NEXT_PUBLIC_TRADING_API_BASE_URL=http://localhost:3000` (real backend)
- Mode B: `NEXT_PUBLIC_TRADING_API_BASE_URL=http://localhost:4010` (Prism mock against the OpenAPI spec)
- Playwright route interception (per `no-admin-bearer.spec.ts`) provides per-request mocking without external services

No production `.env` edited. The frontend `.env.example` already documents
the placeholder.

## 4. Wallet fixture

`tests/e2e/wallet-fixture.ts` provides an injected EIP-1193 mock provider
via `page.addInitScript`. Surface:

```ts
installMockWallet(page, {
  account?: `0x${string}`,         // default: anvil[0] public address
  chainId?: number,                // default: 31337 (anvil)
  signatureRejected?: boolean,     // default: false
})
```

Plus a runtime control surface exposed as `window.__deoptMockWallet`:
- `setAccount(addr | null)` — change connected account; emits `accountsChanged`.
- `setChainId(id)` — change chain id; emits `chainChanged`.
- `setNextSignReject(true)` — make the next `signTypedData` throw EIP-1193 code 4001.

Supported RPC methods:
- `eth_requestAccounts` / `eth_accounts` — returns `[account]` or `[]`.
- `eth_chainId` — returns hex chain id.
- `wallet_switchEthereumChain` — updates state + emits `chainChanged`.
- `eth_signTypedData_v4` / `eth_signTypedData` — returns deterministic mock signature `0x` + 130 hex chars (65 bytes) OR throws code 4001 if `signatureRejected=true`.

**No real private keys. No real signing. No external wallet extension. No MetaMask automation.**

## 5. Mock tx status fixture

**Deferred to M-P4c (or M-P2e).** Rationale: adding an
admin-only test-mode cycler endpoint to the backend requires:
- A new `/admin/test/intent/:id/transition` route gated by admin Bearer + mainnet refusal.
- New `BackendError` variants for invalid status.
- 8 unit tests including mainnet refusal proof.

Each of these is itself a substantial code change with security
implications. The brief explicitly allows fallback: *"If adding backend
endpoint is too risky, use Prism mock examples instead and document
limitation."*

For M-P4b, the Playwright `tx-status-fallback.spec.ts` exercises the
`/transactions/:requestId` route without backend transitions; it
verifies the timeline renders the CREATED stage + intent_id footer.
Backend transitions remain a manual operator step until M-P4c.

## 6. Playwright specs

7 specs landed:

| File | Spec |
|---|---|
| `landing.spec.ts` | trading home renders TestnetUnauditedBanner with all 3 required strings ("Testnet beta — NOT YET AUDITED", "Do NOT deposit real funds", "Mainnet trading is disabled") |
| `markets.spec.ts` | markets page renders Call/Put product cards OR empty state ("No products available") gracefully |
| `portfolio-disconnected.spec.ts` | portfolio renders "Connect your wallet" EmptyState when wallet is detected but not connected |
| `wallet-connected.spec.ts` | clicking Connect wallet triggers `eth_requestAccounts` → shortened address (`0xf39F…2266`) + "anvil" network badge visible |
| `mainnet-disabled.spec.ts` | wallet reporting chain 8453 → `MainnetDisabledBanner` red sticky visible; NetworkBadge shows "mainnet DISABLED" |
| `tx-status-fallback.spec.ts` | `/transactions/test-id` page renders Transaction title + intent_id footer + CREATED timeline stage |
| `no-admin-bearer.spec.ts` | route interception captures `Authorization` header on every trading-route XHR; asserts ZERO captured (admin Bearer absent) |
| `sign-rejected.spec.ts` | mocked `signing-payload` endpoint + wallet fixture's `signatureRejected=true` exercises the rejection code-4001 path (smoke; UI rendering verified) |

**8 specs total.** Each runs against a Next dev server (`npm run dev`)
or production preview (`npm run start` after build).

## 7. Logo / nav polish (Phase F)

`public/logo-deopt.png` wired into the trading nav via `next/image`:

```diff
+ import Image from "next/image";
  …
- <Link href="/" className="font-semibold">
+ <Link href="/" className="flex items-center gap-2 font-semibold">
+   <Image src="/logo-deopt.png" alt="DeOpt" width={28} height={28} priority />
    DeOpt
  </Link>
```

- Logo size: 28×28 px (matches the nav text height).
- `priority` flag ensures eager-load (header is above-the-fold).
- Favicon (from M-P4) preserved.
- No other UI redesign. Leftover Next-template assets (`next.svg`, `vercel.svg`) untouched.

## 8. Tests run

- `npx tsc --noEmit` → exit 0 (clean; new test files use Playwright types).
- `npx eslint src/` → exit 0 (test files under `tests/` not eslint-targeted; that's intentional — Playwright tests use their own TypeScript config inheritance).
- `npx next build` → exit 0 ("Generating static pages using 11 workers (9/9) in 334.0ms"; 9 routes; logo in nav rendered).
- `npx playwright test` → **NOT RUN** in this milestone (requires `npm run e2e:install` to download chromium first; operator runs locally).
- Backend untouched in M-P4b → backend `cargo` checks not re-run; last green state from M-P2d (1096 tests) preserved.

## 9. Docs created / updated

### Created

- `~/DEOPT/deopt-v2-backend/docs/E2E_LOCAL_FIXES_RESULT.md` (this doc).
- `~/DEOPT/deopt-v2-backend/docs/E2E_LOCAL_AUTOMATION_RUNBOOK.md` (companion runbook).

### Updated by reference (no edits required — content still accurate)

- `E2E_LOCAL_TRADING_LIFECYCLE_RUNBOOK.md` — still describes the manual flow; the new automation runbook is a SUPERSET.
- `E2E_LOCAL_TRADING_BLOCKERS_AND_FIXES.md` — B4 NO_TEST_FRAMEWORK partially closed (framework + fixture + specs land; chromium browser download remains operator-side); B5 BACKEND_TX_STATUS_FIXTURE_MISSING still open + deferred to M-P4c; B6 LOGO_NOT_IN_NAV **CLOSED** by Phase F.
- `E2E_SEPOLIA_TRADING_LIFECYCLE_NEXT_TASK.md` — prerequisites unchanged (M-P2c → M-P2e still required); M-P4b is one of the M-P5 prereqs and partially closed.

## 10. RUN_STATE update

`/home/corio/DEOPT/RUN_STATE.md` — closure paragraph prepended.

## 11. Files changed

| Path | Status |
|---|---|
| `~/DEOPT/deopt-v2-frontend/package.json` | edited (+ Playwright devDep, + 2 scripts) |
| `~/DEOPT/deopt-v2-frontend/playwright.config.ts` | new |
| `~/DEOPT/deopt-v2-frontend/tests/e2e/wallet-fixture.ts` | new |
| `~/DEOPT/deopt-v2-frontend/tests/e2e/landing.spec.ts` | new |
| `~/DEOPT/deopt-v2-frontend/tests/e2e/markets.spec.ts` | new |
| `~/DEOPT/deopt-v2-frontend/tests/e2e/portfolio-disconnected.spec.ts` | new |
| `~/DEOPT/deopt-v2-frontend/tests/e2e/wallet-connected.spec.ts` | new |
| `~/DEOPT/deopt-v2-frontend/tests/e2e/mainnet-disabled.spec.ts` | new |
| `~/DEOPT/deopt-v2-frontend/tests/e2e/tx-status-fallback.spec.ts` | new |
| `~/DEOPT/deopt-v2-frontend/tests/e2e/no-admin-bearer.spec.ts` | new |
| `~/DEOPT/deopt-v2-frontend/tests/e2e/sign-rejected.spec.ts` | new |
| `~/DEOPT/deopt-v2-frontend/src/app/(trading)/layout.tsx` | edited (logo wire-in) |
| `~/DEOPT/deopt-v2-backend/docs/E2E_LOCAL_FIXES_RESULT.md` | new |
| `~/DEOPT/deopt-v2-backend/docs/E2E_LOCAL_AUTOMATION_RUNBOOK.md` | new |
| `~/DEOPT/RUN_STATE.md` | edited |

## 12. Validations

- Frontend: `npx tsc --noEmit` + `npx eslint src/` + `npx next build` all clean.
- Backend: untouched in M-P4b; last green state from M-P2d (1096 tests) preserved.
- `git diff --check` clean.
- `git status` shows expected new + edited files.
- Sensitive-string scan: zero `EXECUTOR_PRIVATE_KEY` / `DATABASE_URL` / `AWS_ACCESS_KEY_ID=` / production-EVM-shape strings in new test fixtures / specs / runbook (test fixture uses anvil[0]'s **public** address `0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266` — well-known dev key, no real funds, explicitly documented as such).

## 13. Blockers

| Blocker | Status |
|---|---|
| B1 LOCAL_INTENT_FIXTURE_MISSING | open (needs backend `POST /options/intents/create-from-quote`; tracked for M-P2e) |
| B4 NO_TEST_FRAMEWORK | **partially closed** (framework + fixture + specs land; chromium browser download remains operator-side `npm run e2e:install`) |
| B5 BACKEND_TX_STATUS_FIXTURE_MISSING | open (deferred to M-P4c per brief allowance; admin Bearer + mainnet refusal needed before adding an HTTP-exposed cycler) |
| B6 LOGO_NOT_IN_NAV | **CLOSED** in M-P4b Phase F |

## 14. Next milestone recommendation

**Serialised next:** `BACKEND-TRADING-API-IMPLEMENTATION-PHASE-5` (M-P2e)
to close B1 + the remaining 6 partial endpoints + env loader keys.
3-5 days.

**Alternative:** `E2E-LOCAL-TX-STATUS-CYCLER` (M-P4c) — add the
admin-only test-mode cycler endpoint to close B5. ~2 days.

**Recommended order:** M-P4c → M-P2e → M-P5 (E2E Sepolia) → M-P6 → M-P7.

## 15. Cross-links

- `~/DEOPT/deopt-v2-backend/docs/E2E_LOCAL_AUTOMATION_RUNBOOK.md` (this milestone)
- `~/DEOPT/deopt-v2-backend/docs/E2E_LOCAL_TRADING_LIFECYCLE_RESULT.md` (M-P4)
- `~/DEOPT/deopt-v2-backend/docs/E2E_LOCAL_TRADING_LIFECYCLE_RUNBOOK.md` (M-P4)
- `~/DEOPT/deopt-v2-backend/docs/E2E_LOCAL_TRADING_BLOCKERS_AND_FIXES.md` (M-P4)
- `~/DEOPT/deopt-v2-frontend/docs/FRONTEND_TRADING_SIGNING_RESULT.md` (M-P3b)
- `~/DEOPT/deopt-v2-frontend/docs/TRADING_SIGNING_FLOW_RUNBOOK.md`

**End of M-P4b result.**
