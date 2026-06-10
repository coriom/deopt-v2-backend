# E2E Local Trading — Blockers & Fixes

**Date:** 2026-06-10
**Posture:** docs-only inventory of what blocks a fully-automated local E2E pass + concrete remediation plan.

## Blockers

### B1 — `LOCAL_INTENT_FIXTURE_MISSING` (High)

**Symptom:** the trading UI asks the user to paste an `intent_id`
because the backend exposes signing-payload + signature-submit + tx-status
endpoints but does NOT expose an endpoint to *create* an execution
intent from a quote preview.

**Impact:** orderbook + RFQ scenarios from
`E2E_TRADING_LIFECYCLE_TEST_PLAN.md §1` cannot run end-to-end without
manual operator setup.

**Fix:** M-P2c — add `POST /options/intents/create-from-quote`:
- Input: `{ series_id, side, size, price_1e8, account }`.
- Output: `{ intent_id, status: "CREATED" }`.
- Internally calls the existing intent creation pipeline at
  `src/options/service.rs::create_execution_intent` (which currently
  lives behind operator scope).
- Validates series + size + price + account; returns
  `INVALID_REQUEST` for bad inputs.
- Does NOT auto-broadcast; the resulting intent waits for buyer +
  seller signatures.

Alternative (cheaper): add a CLI seed script
`scripts/seed-intent.sh` that calls the existing operator endpoint to
create a deterministic intent for a known account + series. M-P4b
can call this from Playwright.

### B2 — `ON_CHAIN_RPC_NOT_WIRED` (Medium)

**Symptom:** 6 trading endpoints (positions / portfolio / balances /
quote-preview / exercise-preview / close-preview) return `status:
"partial"` with `SOURCE_UNAVAILABLE_FIELD` warnings for on-chain
fields (mark price, unrealised PnL, IM/MM/free collateral, oracle
mark, settlement breakdown).

**Impact:** UI renders amber "approximate" badges instead of
authoritative values; failure-case sweep (insufficient collateral,
etc.) can't be exercised against real chain state.

**Fix:** M-P2c — add 7 narrow inline `alloy_sol_types::sol!`
declarations against the frozen ABI at
`~/DEOPT/deopt-v2-sol/abis/freeze-v2-product-rc1/`:

| sol surface | Used by |
|---|---|
| `MarginEngineLens.getAccountState(address, address)` | positions / portfolio |
| `MarginEngineLens.previewTradeFees(address, uint256, uint128, uint128, address, address, bool)` | quote / close preview |
| `MarginEngineLens.previewAccountSettlement(address, uint256, address)` | exercise preview |
| `MarginEngineLens.previewDetailedSettlement(address, uint256, address)` | exercise preview (detailed) |
| `CollateralVaultViews.getCollateralTokens()` | balances |
| `CollateralVault.balances(address, address)` | balances |
| `OracleRouter.getFeed(address, address)` + `hasActiveFeed(address, address)` | quote / exercise preview |

Plus:
- New config fields `margin_engine_lens_address`, `oracle_router_address` on `AppState` (or under a new `TradingViewsConfig`).
- Uses existing `EthCallProvider` infrastructure; graceful `RPC_UNAVAILABLE` fallback if `rpc_url == None`.
- Per-endpoint integration tests against a mock `EthCallProvider`.
- Selectors verified at PR time against `selectors.txt`.

### B3 — `FRONTEND_CREATE_INTENT_UX_MISSING` (Medium)

**Symptom:** Phase B1 above produces a backend endpoint, but the UI
currently has a text input for "Execution intent id" that the user
manually fills.

**Impact:** Usability degrades the perception of "complete trading UI."

**Fix:** M-P3c (UX polish) — in `TradeTicket.tsx`, replace the manual
intent_id input with a "Create intent" button that:
1. POSTs `/options/intents/create-from-quote` with the current
   side/size/price/account.
2. Reads `intent_id` from the response.
3. Sets internal state.
4. Continues to the signing flow.

Depends on B1.

### B4 — `NO_TEST_FRAMEWORK` (Low)

**Symptom:** No Playwright / Cypress / Vitest installed in the
frontend repo. No integration test crate in the backend repo
(beyond `cargo test`).

**Impact:** No automated E2E sweep; M-P4 ships docs-only test plan.

**Fix:** M-P4b — add Playwright + viem + an injected-EIP-1193-provider
fixture:
1. `npm install --save-dev @playwright/test`.
2. `npx playwright install chromium`.
3. Add `tests/e2e/wallet-fixture.ts` — wraps a viem `localAccount`
   into an EIP-1193 provider injected into the browser context.
4. Add 6 smoke tests:
   - frontend loads `/` and shows MarketSelector;
   - markets page renders products from mock backend;
   - portfolio page renders wallet-disconnected EmptyState;
   - `/transactions/<id>` renders status fallback;
   - mainnet-disabled banner appears when wallet reports chain 8453;
   - no `Authorization` header on trading-route XHRs (network
     interception).
5. Add `npm run test:e2e` script + GitHub Actions workflow stub.

Run against Prism mock backend (no real backend required for the
basic frontend smoke).

### B5 — `BACKEND_TX_STATUS_FIXTURE_MISSING` (Medium) — **CLOSED in M-P4c (2026-06-10)**

**Symptom:** A local intent created by the user lingers in
`SIGNING_PAYLOAD_ISSUED` or `SIGNED` state because no operator
manually broadcasts. The `useTxStatus` polling shows a stuck
spinner; no terminal state reached.

**Impact:** Cannot exercise CONFIRMED / REVERTED / STUCK UI paths
without manual operator intervention or production wiring.

**Fix shipped (M-P4c, 2026-06-10):** local/test-only fixture surface
added at `crate::api::local_test_fixtures` and wired into
`/admin/test/*` + `/trading/test/tx-status/:intent_id`:
- `POST /admin/test/execution-intents` — create synthetic intent.
- `GET  /admin/test/intent/:intent_id` — read synthetic intent.
- `POST /admin/test/intent/:intent_id/transition` — drive synthetic
  status `created → pending → confirmed | failed | reverted | stuck`.
- `GET  /trading/test/tx-status/:intent_id` — frontend-facing read.
- Disabled by default. 4 independent gates refuse mainnet
  (`chain_id == 8453`): factory refusal + per-request runtime check +
  production-startup default + admin Bearer gate. All endpoints return
  HTTP 404 unless the fixture is explicitly installed at runtime AND
  `chain_id != 8453`.
- 40 backend tests added (26 unit + 14 HTTP). Full lib suite 880 green.
- See `E2E_LOCAL_TX_STATUS_CYCLER_RESULT.md` +
  `E2E_LOCAL_TX_STATUS_CYCLER_RUNBOOK.md`.

### B6 — `LOGO_NOT_IN_NAV` (Low)

**Symptom:** `public/logo-deopt.png` exists but is not referenced in
the trading nav.

**Impact:** Branding incomplete; pure visual polish.

**Fix:** FRONTEND-BRANDING-AND-PRODUCT-POLISH milestone — replace
text "DeOpt" in `src/app/(trading)/layout.tsx` nav with
`<Image src="/logo-deopt.png" alt="DeOpt" width={120} height={32} />`.
Also remove the leftover Next-template assets (`next.svg`,
`vercel.svg`).

## Severity matrix

| Blocker | Severity | Blocks M-P4 acceptance? | Blocks M-P5? |
|---|---|---|---|
| B1 LOCAL_INTENT_FIXTURE_MISSING | High | YES (full lifecycle requires it) | YES |
| B2 ON_CHAIN_RPC_NOT_WIRED | Medium | NO (partial-real is acceptable for E2E rehearsal) | NO (Sepolia state IS the on-chain state) |
| B3 FRONTEND_CREATE_INTENT_UX_MISSING | Medium | NO (manual paste works for rehearsal) | NO |
| B4 NO_TEST_FRAMEWORK | Low | NO (manual run accepted) | YES (Playwright needed for repeatable Sepolia sweep) |
| B5 BACKEND_TX_STATUS_FIXTURE_MISSING | Medium | ~~NO~~ **CLOSED** (M-P4c) | ~~NO~~ **CLOSED** (M-P4c) |
| B6 LOGO_NOT_IN_NAV | Low | NO | NO |

## Order-of-operations recommendation

1. **M-P4b** — Playwright + mock-status cycler (closes B4 + B5).
2. **M-P2c** — on-chain RPC orchestration (closes B2 + B1).
3. **M-P3c** — create-intent UX (closes B3, depends on M-P2c).
4. **FRONTEND-BRANDING-AND-PRODUCT-POLISH** — logo + Next-template cleanup (closes B6).
5. **M-P5** — Sepolia E2E.

Alternative: skip M-P4b + go straight to M-P2c + M-P5 with manual
Playwright steps at Sepolia. This is faster but less repeatable.

## Cross-links

- `E2E_LOCAL_TRADING_LIFECYCLE_RESULT.md`
- `E2E_LOCAL_TRADING_LIFECYCLE_RUNBOOK.md`
- `E2E_LOCAL_FIXES_NEXT_TASK.md`
- `BACKEND_TRADING_API_PHASE_2_RESULT.md`

**End of blockers + fixes.**
