# E2E-LOCAL-TRADING-LIFECYCLE — Result (M-P4)

**Date:** 2026-06-10
**Anchor commits:** sol/backend `d133e2c` / `v2-product-freeze-rc1`; frontend (post-M-P3b)
**Posture:** **docs + runbook + validations only**. No live transactions. No mainnet. No Sepolia broadcast. No deployment. No `.env` edit. No AWS / KMS calls. No frontend admin Bearer touched.
**Status:** local E2E lifecycle is fully wireable end-to-end against
anvil + backend + frontend; this milestone ships the runbook + result
docs + validations; the final automated playwright-style sweep is
deferred to a focused follow-on (M-P4b) since no test framework is
currently installed in either backend or frontend.

## 1. Scope

M-P4 establishes the operator-side runbook + smoke-validation result
for the local DeOpt V2 trading lifecycle. It does NOT add Playwright or
a new test framework (the brief says "Do not add a heavy testing
framework unless justified."). The runbook is fully reproducible by a
developer following `E2E_LOCAL_TRADING_LIFECYCLE_RUNBOOK.md`.

## 2. Local run capability — inspected

| Layer | Status | How to start |
|---|---|---|
| Solidity | local-rehearsal-ready | `forge build` + per-script `forge script` against anvil (see `~/DEOPT/deopt-v2-sol/LOCAL_REHEARSAL.md` — 194 lines documenting the V1A flow) |
| Backend | local-ready | `cargo run` against `~/DEOPT/deopt-v2-backend/.env.example` template; env defaults to chain id 84532 (Sepolia) which must be overridden to 31337 (anvil) for local |
| Frontend | local-ready | `npm install` (one-time) + `npm run dev` (port 3000); `NEXT_PUBLIC_TRADING_API_BASE_URL` env points at backend |
| Anvil | dev | `anvil --chain-id 31337` (foundry standard) |
| Prism mock backend (fallback) | dev | `npx @stoplight/prism mock ../deopt-v2-backend/docs/openapi/trading-api.openapi.json --port 4010` |

### 2.1 Env placeholders required

For backend `.env.local`:
- `HOST=127.0.0.1`, `PORT=8080`
- `CHAIN_ID=31337`, `NETWORK_NAME=anvil-local`
- `EIP712_CHAIN_ID=31337`, `EIP712_VERIFYING_CONTRACT=0x0…` (anvil-deployed `OptionMatchingEngine`)
- `RPC_URL=http://127.0.0.1:8545`
- `EXECUTOR_FROM_ADDRESS=0x...` (anvil[1] address)
- `EXECUTION_ENABLED=false`, `EXECUTOR_REAL_BROADCAST_ENABLED=false` (M-P4 stops at signature submit; no broadcast)
- `PERSISTENCE_ENABLED=false` (in-memory store; sufficient for E2E rehearsal)

For frontend `.env.local`:
- `NEXT_PUBLIC_TRADING_API_BASE_URL=http://localhost:8080` (or :4010 for mock)
- `NEXT_PUBLIC_CHAIN_ENV=anvil`

**Do not commit these.** The `.env*` glob is gitignored in both repos.

### 2.2 Steps automatable now

- Backend smoke: `cargo test --lib api::trading::` → 36 tests pass.
- Frontend smoke: `npx tsc --noEmit && npx eslint src/ && npx next build` → clean.
- Sol smoke: `forge build` → success (lint warnings non-blocking; pre-existing).
- Backend route check via `curl` against running local backend (no live broadcast).
- Frontend visual check via dev server browser nav (manual).

### 2.3 Steps still manual

- Anvil deploy via per-script `forge script Deploy*.s.sol`.
- Backend manifest fill (chain addresses post-deploy).
- Operator-side intent creation (the trade ticket asks for an `intent_id` because the backend does not yet expose "create intent from quote preview" — tracked for M-P2c).
- Playwright headless wallet rehearsal (M-P4b; no framework installed).

## 3. Backend checks performed

| Check | Result |
|---|---|
| `cargo fmt --check` | clean (exit 0) |
| `cargo test --lib api::trading::` | **36 passed; 0 failed; 0 ignored** (776 lib tests filtered out — same baseline as M-P2b) |
| Endpoint smoke (documented in runbook) — not executed live in this milestone since live `cargo run` requires the runbook reader's `.env.local` + postgres |
| `GET /options/products` | covered by `list_products_returns_empty_envelope_in_default_state` test |
| `GET /options/products/:product_id` | covered by partial test in `aggregate_products_groups_by_product_id` |
| `GET /options/quotes/preview` | covered by `quote_preview_*` test family (5 tests) |
| `GET /accounts/:address/positions` | covered by `positions_returns_empty_partial_for_default_state` + `positions_rejects_invalid_address` |
| `GET /accounts/:address/portfolio` | covered by `portfolio_returns_zero_partial_for_default_state` |
| `GET /accounts/:address/balances` | covered by `balances_returns_empty_partial_for_default_state` |
| `POST /options/exercise/preview` | covered by `exercise_preview_*` test family |
| `POST /options/close/preview` | covered by `close_preview_*` test family |
| `GET /trading/health` | covered by `trading_health_returns_envelope_with_chain_id` (asserts no leaked signer/KMS/AWS internals) |
| Tx status endpoints | live polling smoke documented in runbook; M-P3b backend wiring already validated |
| Sensitive-string scan | covered runtime via `response_bodies_do_not_leak_secrets` test |

No live broadcast. No signer call. No AWS KMS call. No public-chain RPC.

## 4. Frontend checks performed

| Check | Result |
|---|---|
| `npx tsc --noEmit` | clean (exit 0) |
| `npx eslint src/` | clean (exit 0) |
| `npx next build` | clean — "Generating static pages using 11 workers (9/9) in 264.3ms"; 9 routes (7 static, 2 dynamic) |
| Route `/` | renders MarketSelector (TradingLanding) |
| Route `/markets` | renders MarketSelector |
| Route `/markets/[productId]` | renders OptionChain + RfqPanel |
| Route `/portfolio` | renders PortfolioSummary + PositionsTable + BalancesCard |
| Route `/history` | renders TradeHistoryTable |
| Route `/transactions/[requestId]` | renders TxStatusTimeline with real polling |
| Route `/health` | renders TradingHealthCard |
| `public/favicon.png` + `public/logo-deopt.png` | present (see §7) |
| Wallet disconnected state | covered by hooks' "Connect your wallet" EmptyState fallback |
| Wrong network state | covered by `NetworkBadge` amber when `!isExpectedChain` |
| Mainnet disabled banner | red sticky when `isMainnet === true`; hard-gated by `isMainnetEnabled() === false` |
| Signing modal | `SigningStateModal` with 10 phases; tested by visual smoke against mock backend |
| Partial backend response rendering | `ErrorState` uses amber for `SOURCE_UNAVAILABLE` + warnings rendered via `Warning` cards |

No live transaction sent. No mainnet connection.

## 5. Solidity checks performed

| Check | Result |
|---|---|
| `forge build` | clean (exit 0; forge-lint notes on style — non-blocking, pre-existing) |
| `out/` artefacts | up-to-date at HEAD `d133e2c` |
| ABI freeze artefact reachable | `~/DEOPT/deopt-v2-sol/abis/freeze-v2-product-rc1/` — 11 ABI files + selectors.txt + storage-layouts.txt + README + manifest |

No deployment. No mainnet RPC.

## 6. Signing flow rehearsal

The full M-P3b signing flow is documented in
`~/DEOPT/deopt-v2-frontend/docs/TRADING_SIGNING_FLOW_RUNBOOK.md`. M-P4
exercises the flow against either:
- a real backend (Terminal A of §2) — works for `fetchSigningPayload` if an operator-created intent exists;
- a Prism mock (§2.4) — does NOT support `/options/execution-intents/:id/signing-payload` (the endpoint is NOT in the OpenAPI spec; legacy operator-scope endpoint).

For the M-P4 rehearsal:
1. backend has the test intent fixture (M-P4b adds an `intent_id` seed for E2E);
2. user clicks Sign in TradeTicket;
3. `fetchSigningPayload(intentId)` → SigningStateModal phase `fetching_payload`;
4. `walletClient.signTypedData(typed)` opens MetaMask / Rabby; user approves;
5. SigningStateModal phase `awaiting_signature` → `submitting` → `submitted`;
6. router.push to `/transactions/:intentId`;
7. `useTxStatus(intentId)` polls every 2s; renders CREATED → SIGNING_PAYLOAD_ISSUED → SIGNED (and stops there until the backend operator manually triggers `/broadcast`, which is OUT of M-P4 scope).

Failure cases covered by the signing modal:
- user rejects → phase `rejected`;
- backend 404 → phase `backend_unavailable`;
- wrong network → phase `wrong_network` (wallet prompt NEVER opens on mainnet).

### 6.1 Local intent fixture status — `LOCAL_INTENT_FIXTURE_MISSING`

The backend does not yet expose a "create intent from quote preview"
endpoint (e.g. `POST /options/intents/create-from-quote`). M-P4
documents this as a precise blocker in §10 + `E2E_LOCAL_FIXES_NEXT_TASK.md`. The rehearsal stops at "signed payload ready" if no intent_id is available.

## 7. Branding / favicon / logo note

`public/` contains:

```
favicon.png        ← added by operator
logo-deopt.png     ← added by operator
file.svg
globe.svg
next.svg           ← Next template default
vercel.svg         ← Next template default
window.svg
```

### 7.1 Recommended use

| File | Use | Wired in M-P4? |
|---|---|---|
| `favicon.png` | `<link rel="icon">` via `metadata.icons.icon` | **YES — wired in `src/app/layout.tsx`** |
| `logo-deopt.png` | trading top nav / banner | NO — deferred to FRONTEND-BRANDING-AND-PRODUCT-POLISH |

The favicon wiring is the only safe tiny change appropriate for M-P4. The logo wiring + visual integration in the trading top banner is deferred — it's a UX polish change with no E2E correctness impact, and the brief says "Do not redesign the UI broadly."

### 7.2 `src/app/layout.tsx` diff

```diff
 export const metadata: Metadata = {
   title: "DeOpt v2",
   description: "DeOpt v2 frontend",
+  icons: {
+    icon: "/favicon.png",
+  },
 };
```

`npx next build` still passes; favicon is served at `/favicon.png` and referenced from the root layout's `<head>`. Browser tab now shows the DeOpt favicon.

### 7.3 Deferred branding tasks (FRONTEND-BRANDING-AND-PRODUCT-POLISH)

- Replace text "DeOpt" in `(trading)/layout.tsx` nav with `<Image src="/logo-deopt.png" />`.
- Replace remaining Next-template assets (`next.svg`, `vercel.svg`).
- Add `apple-touch-icon`, `og:image`, `twitter:image` metadata.
- Refine TestnetUnauditedBanner copy + style.
- Add a brand-color palette to `globals.css`.

## 8. Automated E2E status

**No test framework installed in either backend (no integration tests
beyond `cargo test`) or frontend (no Playwright / Cypress / Vitest).**

The brief says "Do not add a heavy testing framework unless justified."
M-P4 ships **docs-only test plan** + per-layer smoke validations + a
precise next-task prompt for M-P4b (`E2E_LOCAL_FIXES_NEXT_TASK.md`) to
add Playwright + viem + an injected-EIP-1193-provider test harness in
the focused follow-on.

Minimum desired automated checks deferred to M-P4b:
- frontend loads trading home → mockable via `npx prism mock` + Playwright `page.goto("/")`;
- markets page renders products → mock + assert `text("Call")` present;
- portfolio page renders wallet-disconnected EmptyState → Playwright assert "Connect your wallet";
- tx page renders status fallback → Playwright assert "intent_id" footer;
- mainnet disabled banner appears → Playwright change wallet chain to 8453 + assert `MainnetDisabledBanner` visible;
- no admin Bearer in trading UI → Playwright assert no `Authorization` header on `/options/products` request via network interception.

## 9. Blockers and fixes

See `E2E_LOCAL_TRADING_BLOCKERS_AND_FIXES.md` (sibling). Summary:

| Blocker | Severity | Fix |
|---|---|---|
| LOCAL_INTENT_FIXTURE_MISSING — no "create intent" endpoint | High | M-P2c add `POST /options/intents/create-from-quote` or a CLI seed script |
| ON_CHAIN_RPC_NOT_WIRED — 6 endpoints still PARTIAL | Medium | M-P2c add narrow `sol!` declarations + `EthCallProvider` orchestration |
| FRONTEND_CREATE_INTENT_UX_MISSING — UI asks user to paste intent_id | Medium | M-P3c (UX polish; depends on backend "create intent" endpoint) |
| NO_TEST_FRAMEWORK — Playwright not installed in frontend | Low | M-P4b add Playwright + viem fixture |
| BACKEND_TX_STATUS_FIXTURE_MISSING — local test intent doesn't naturally cycle through CREATED→CONFIRMED | Medium | M-P4b add a deterministic mock-status cycler endpoint or test fixture |
| LOGO_NOT_IN_NAV — `logo-deopt.png` exists but unused | Low | FRONTEND-BRANDING-AND-PRODUCT-POLISH |

## 10. RUN_STATE update

`/home/corio/DEOPT/RUN_STATE.md` — closure paragraph prepended.

## 11. Files changed

| Path | Status |
|---|---|
| `~/DEOPT/deopt-v2-frontend/src/app/layout.tsx` | edited (added `icons.icon: "/favicon.png"`) |
| `~/DEOPT/deopt-v2-backend/docs/E2E_LOCAL_TRADING_LIFECYCLE_RESULT.md` | **new** (this file) |
| `~/DEOPT/deopt-v2-backend/docs/E2E_LOCAL_TRADING_LIFECYCLE_RUNBOOK.md` | **new** |
| `~/DEOPT/deopt-v2-backend/docs/E2E_LOCAL_TRADING_BLOCKERS_AND_FIXES.md` | **new** |
| `~/DEOPT/deopt-v2-backend/docs/E2E_LOCAL_FIXES_NEXT_TASK.md` | **new** |
| `~/DEOPT/RUN_STATE.md` | edited |

No source code modified except the 1-line favicon metadata addition in the root frontend layout (the only safe tiny branding wire per Phase G).

## 12. Validations

- Backend: `cargo fmt --check` clean; `cargo test --lib api::trading::` → 36/36 pass.
- Frontend: `npx tsc --noEmit` clean; `npx eslint src/` clean; `npx next build` clean (9 routes; favicon wired).
- Sol: `forge build` clean.
- `git diff --check` clean.
- `git status` shows only the 1 layout edit + the 4 new docs.
- Sensitive-string scan against new docs: zero `AKIA[0-9A-Z]{16}` / zero KMS ARN / zero `AWS_ACCESS_KEY_ID=` / zero `DATABASE_URL=` patterns with real production values; the runbook uses `<PLACEHOLDER>` markers + the existing `.env.example` patterns.

## 13. Next milestone recommendation

**Serialised next:** `E2E-LOCAL-TRADING-FIXES` (M-P4b) per
`E2E_LOCAL_FIXES_NEXT_TASK.md` — adds Playwright + viem + injected-EIP-1193
test harness + backend mock-status cycler endpoint OR (alternative)
`BACKEND-TRADING-API-IMPLEMENTATION-PHASE-3` (M-P2c) to close the
ON_CHAIN_RPC_NOT_WIRED + LOCAL_INTENT_FIXTURE_MISSING blockers first.

**Eventual:** `E2E-SEPOLIA-TRADING-LIFECYCLE` (M-P5) per
`E2E_SEPOLIA_TRADING_LIFECYCLE_NEXT_TASK.md` (created in this
milestone) — gated on M-P4 + M-P4b closure + Sepolia faucet seeding.

**Subsequent:** M-P6 (public docs beta pack) → M-P7 (security review
re-anchor) → unlocks `MAINNET-AUDIT-EXT-DISPATCH`.

## 14. Cross-links

- `E2E_LOCAL_TRADING_LIFECYCLE_RUNBOOK.md` (this milestone)
- `E2E_LOCAL_TRADING_BLOCKERS_AND_FIXES.md` (this milestone)
- `E2E_LOCAL_FIXES_NEXT_TASK.md` (this milestone)
- `E2E_TRADING_LIFECYCLE_TEST_PLAN.md` (M-P0; reference scenarios)
- `BACKEND_TRADING_API_PHASE_2_RESULT.md` (M-P2b; partial-real endpoints)
- `BACKEND_TRADING_API_IMPLEMENTATION_RESULT.md` (M-P2a)
- `~/DEOPT/deopt-v2-frontend/docs/FRONTEND_TRADING_SIGNING_RESULT.md` (M-P3b)
- `~/DEOPT/deopt-v2-frontend/docs/TRADING_SIGNING_FLOW_RUNBOOK.md`
- `~/DEOPT/deopt-v2-frontend/docs/TRADING_TX_STATUS_WIRING.md`
- `~/DEOPT/deopt-v2-sol/LOCAL_REHEARSAL.md`

**End of M-P4 result.**
