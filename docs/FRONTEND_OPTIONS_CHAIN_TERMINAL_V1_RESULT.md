# FRONTEND-OPTIONS-CHAIN-TERMINAL-V1 — Result

**Date executed:** 2026-06-13
**Operator approval line accepted (verbatim):**
> "I approve DeOpt V2 frontend options-chain terminal V1 for this run."

**Posture:** **Frontend-only terminal redesign. Zero chain transactions. Zero broadcast. Zero mainnet. Zero backend `.env` edit. Zero private key handling. Zero audit outreach. Zero bug bounty. Zero claim of "audited" / "mainnet-ready" / "production" / "safe for real funds".**

---

## 1. Workspace

* `~/DEOPT/deopt-v2-frontend/src/` (8 new + 4 edited)
* `~/DEOPT/deopt-v2-frontend/tests/e2e/` (2 new specs + 3 spec updates)
* `~/DEOPT/deopt-v2-backend/docs/FRONTEND_OPTIONS_CHAIN_TERMINAL_V1_RESULT.md` (this file)
* `~/DEOPT/deopt-v2-backend/docs/public-beta/PUBLIC_TESTNET_BETA_LAUNCH_CHECKLIST.md` (V1 evidence row)
* `~/DEOPT/RUN_STATE.md` (closure paragraph)

## 2. UI architecture inventory (Phase A)

Carried over from prior milestones. Gaps to address in V1:
* Navbar was Markets / Portfolio / History / Health — not the professional Trade / Markets / Portfolio / API / Académie pattern + hamburger.
* `/markets` page used the `MarketSelector` grid; no central "options chain" pattern with calls | strike | puts.
* No detail panel with Trade / Payoff / Greeks / Details / Risk tabs.
* No bottom panel with Balances / Positions / Trades / Events tabs (the portfolio page had these but not co-located with the chain).
* No reusable strike-paired chain model (the existing `useSeriesDetails` hook resolves one series at a time; needed an accumulator).
* Greeks not exposed by the backend — needed honest "n/a testnet" copy throughout.

## 3. Navbar refactor (Phase B)

* `(trading)/layout.tsx` rewritten:
  * Primary navbar: `Trade` (`/trade`, new), `Markets`, `Portfolio`, `API` (placeholder aria-disabled), `DeOpt Académie` (placeholder aria-disabled). History + Health removed from the visible navbar (still reachable by URL).
  * Each nav link stamped with `data-testid="navbar-link-{id}"`; placeholders carry `aria-disabled="true"` + `data-placeholder="true"`.
  * Right cluster: `NetworkBadge` + `WalletConnectButton` + the new `HamburgerMenu`. The compact "Report a bug" header button was retired — Feedback now lives in the hamburger drawer.
* New `src/components/HamburgerMenu.tsx`:
  * Slide-out drawer (`role="dialog" aria-modal="true"`) with Docs index, Quickstart, Feedback, Discord, GitHub, Risks/limitations, and a Changelog (coming soon) placeholder. **No admin links.**
  * Reads URLs from `public-beta-links.ts` — internal slots render as Next.js `<Link>`, external as `<a target="_blank">`, placeholder slots as non-clickable spans.
  * Escape key closes the drawer; click-outside closes it; close button stamped with `data-testid="hamburger-close-button"`.

## 4. Options chain data model (Phase C)

* New `src/lib/options-chain-model.ts`:
  * `OptionLeg` with explicit `OptionAvail` (`"live" | "n/a-testnet" | "loading"`) flags per field (bid/ask/mark/lastPremium/iv/delta/gamma/vega/theta). Default avail is `"n/a-testnet"` for everything — the backend does not expose these yet.
  * `OptionsChainRow` groups call + put at the same (strike, expiry) into a single row for the ladder.
  * `buildOptionsChain(products, seriesById)` — strict, honest assembly from a product list + per-series detail map. Series not yet loaded show as empty legs (testid `data-available="false"` in the grid).
  * `distinctExpiries()`, `distinctUnderlyings()`, `filterByExpiry()` helpers.
  * **No fake liquidity:** every cell that lacks live data renders "—". No fake greeks; no fake bid/ask; no fake IV.

## 5. Options chain grid (Phase D)

* New `src/components/trading/terminal/OptionsChainGrid.tsx`:
  * 3-column layout: Calls | Strike center | Puts. Each leg cell shows Bid / Ask / Mark / IV (all "—" by default per the data model).
  * Buttons stamped with `data-testid="chain-call-{strike}-{expiry}"` / `chain-put-{strike}-{expiry}` + `data-selected` + `data-available`. Cells with `seriesId === null` are disabled with `cursor-not-allowed`.
  * Strike center column shows the strike + expiry.
  * Honest footer caption: "Bid / Ask / Mark / IV are not exposed by the testnet beta backend yet — every cell renders '—' honestly."
* New `src/components/trading/terminal/ExpirySelector.tsx`:
  * Tab-style pills with `"All"` + one pill per distinct expiry; stamped with `data-testid="expiry-pill-{ms}"`.
* `OptionsChainTerminal` orchestrator:
  * Drives the chain off `useProducts` + `useProductDetails` + `useSeriesDetails`. Accumulates series details via a small `useSeriesById` accumulator that fetches one unseen series id at a time (avoids parallel fetches).
  * Underlying pill row + expiry pill row in the header.
  * Backend-unavailable → `MarketsFallbackCard kind="backend-unavailable"` + Retry + Discord CTA.
  * No-products → `MarketsFallbackCard kind="no-products"`.

## 6. Selected option detail panel (Phase E)

* New `src/components/trading/terminal/OptionDetailPanel.tsx`:
  * Empty-state when no row selected (`data-testid="detail-panel-empty"`).
  * Header shows CALL / PUT badge, K = strike, expiry, truncated series id.
  * 5 tabs (`role="tablist"`) — `Trade / Payoff / Greeks / Details / Risk`. Each tab stamped with `data-testid="detail-tab-{id}"` + `data-selected`. Active panel stamped with `data-testid="detail-panel-content-{id}"`.
  * **Trade tab:** Buy (long) / Sell (short) toggle, quantity input, quote-preview grid with Premium / Buyer fee / Seller fee / Collateral required — every field a `—` + `n/a testnet` chip since live preview isn't wired in the chain context. Includes a clearly-labeled link to `/markets/[productId]` for the full sign + submit flow (the V3 trade ticket still owns the create-intent + sign path).
  * **Payoff tab:** new `PayoffSvg` component — small inline SVG showing the archetypal payoff for long/short × call/put. No chart lib. Schematic-only — explicitly disclaimed.
  * **Greeks tab:** honest "Greeks — coming soon in the testnet beta" card with grid of dashes + `n/a testnet` chips. Explains "not exposed by the current backend" + "will add as the testnet matures" + "every Greek cell renders '—' rather than inventing values".
  * **Details tab:** series id, product id, option type, strike (1e8), expiry, **canonical retargeted Matching Engine / Margin Engine addresses** (truncated), settlement = mUSDC testnet mock, oracle status, network = Base Sepolia chain 84532.
  * **Risk tab:** controlled red panel listing testnet-only / unaudited / no real funds / mock oracle / operator-controlled / no financial advice. Links to `/docs/limitations`.

## 7. Bottom balances/positions panel (Phase F)

* New `src/components/trading/terminal/BottomPanel.tsx`:
  * 4 tabs: Balances / Positions / Trades / Events.
  * Reuses existing `BalancesCard`, `PositionsTable`, `TradeHistoryTable` (no duplicated logic).
  * Events tab is a placeholder card explaining the per-wallet event feed is in a follow-up milestone and pointing testers at `/transactions/<intent_id>` for the per-trade lifecycle.
  * Each tab stamped with `data-testid="bottom-tab-{id}"` + active panel `data-testid="bottom-panel-content-{id}"`.

## 8. Responsive behavior (Phase G)

* Terminal layout uses `grid lg:grid-cols-[minmax(0,1fr)_22rem]` — desktop shows chain + side panel; below `lg` they stack vertically.
* Chain grid columns use `minmax(7rem, auto)` for the strike center, preventing overflow.
* Underlying + expiry pills use `flex-wrap` so they re-flow on narrow widths.
* Navbar uses `flex-wrap items-center justify-between gap-y-2` so the right cluster wraps below the nav on mobile.
* Hamburger drawer is `w-72 max-w-[90vw]` — capped to viewport width.
* No new icon / chart / animation library added.

## 9. Tests added/updated (Phase H)

| Spec | Action | Coverage |
|---|---|---|
| `tests/e2e/terminal-navbar.spec.ts` | NEW (5 specs) | Trade/Markets/Portfolio visible with correct hrefs; API+Académie are `aria-disabled` placeholders; hamburger drawer opens + contains docs/quickstart/feedback/limitations/Discord/GitHub links with correct hrefs; Changelog is a placeholder; drawer has no admin/mainnet/bearer/RPC leak; Escape closes the drawer |
| `tests/e2e/options-chain-terminal.spec.ts` | NEW (9 specs) | chain structure (Calls/Strike/Puts) visible; clicking a Call cell updates the detail panel; 5 tabs render; Greeks tab honestly says "coming soon" + "not exposed by the current backend"; Payoff tab renders the SVG; Risk tab surfaces testnet/unaudited/no-real-funds; backend-unavailable renders `MarketsFallbackCard`; `/trade` DOM contains no positive-claim / no fake-liquidity / no amber-yellow-orange / no admin / no mainnet / no bearer / no RPC URL / no DATABASE_URL; detail panel CTA links to `/markets/[productId]` (internal) |
| `tests/e2e/report-issue.spec.ts` | REWRITTEN | header report-button retired → Feedback link on every route now reached via the hamburger drawer; landing Report-feedback CTA navigates to internal `/feedback`; hamburger Feedback link points at `/feedback` |
| `tests/e2e/landing-product-v2.spec.ts` | UPDATED | Report feedback CTA now resolved via `report-issue-link` OR `report-issue-button` (handles live + placeholder degradation) |
| `tests/e2e/markets-fallback.spec.ts` | UPDATED | same dual-testid resolution for the Report-issue CTA inside the markets fallback card |

Catalog: `npx playwright test --list` — **96 tests in 24 files** (was 82 in 22; +14 tests).

## 10. Build validations (Phase J)

| Command | Result |
|---|---|
| `npm run typecheck` (`tsc --noEmit`) | clean |
| `npm run lint` (`eslint`) | clean (after 4 fixes: hoisted `useMemo` for `allProducts`, microtask-deferred 3 in-effect `setState` calls per `react-hooks/set-state-in-effect`) |
| `npm run build` (`next build`) | green, **15 routes** prerendered (added `/trade`) |
| `npx playwright test --list` | 96 tests in 24 files, parse-clean |
| Targeted spec run | not executed (WSL2 sandbox missing `libnspr4.so`; CI/Linux unaffected — same constraint as prior milestones) |

## 11. Docs created / updated (Phase I)

* NEW `deopt-v2-backend/docs/FRONTEND_OPTIONS_CHAIN_TERMINAL_V1_RESULT.md` (this doc).
* UPDATED `deopt-v2-backend/docs/public-beta/PUBLIC_TESTNET_BETA_LAUNCH_CHECKLIST.md` (V1 evidence row added to §1.5b).
* UPDATED `~/DEOPT/RUN_STATE.md` (closure paragraph).
* No followup brief created — V1 covers the brief scope; V2 (Greeks wiring, real bid/ask, live mark, indicator overlays) is a future iteration.

## 12. RUN_STATE update

Closure paragraph prepended dated 2026-06-13.

## 13. Files changed

**Created (frontend src):**
* `src/components/HamburgerMenu.tsx`
* `src/lib/options-chain-model.ts`
* `src/components/trading/terminal/ExpirySelector.tsx`
* `src/components/trading/terminal/OptionsChainGrid.tsx`
* `src/components/trading/terminal/OptionDetailPanel.tsx`
* `src/components/trading/terminal/PayoffSvg.tsx`
* `src/components/trading/terminal/BottomPanel.tsx`
* `src/components/trading/terminal/OptionsChainTerminal.tsx`
* `src/app/(trading)/trade/page.tsx`

**Edited (frontend src):**
* `src/app/(trading)/layout.tsx` (navbar refactor + hamburger + retire compact report-bug button)

**Created (tests):**
* `tests/e2e/terminal-navbar.spec.ts`
* `tests/e2e/options-chain-terminal.spec.ts`

**Edited (tests):**
* `tests/e2e/report-issue.spec.ts` (rewritten for hamburger-Feedback path)
* `tests/e2e/landing-product-v2.spec.ts` (dual-testid for Report-feedback CTA)
* `tests/e2e/markets-fallback.spec.ts` (same)

**Not touched:**
* Backend Rust source — ZERO
* Solidity source — ZERO
* Backend `.env` — UNCHANGED (mtime preserved)
* `~/DEOPT/private/**` — NOT read, NOT committed
* `src/lib/public-beta-links.ts` — unchanged from FRONTEND-INTEGRATED-DOCS-AND-FEEDBACK
* All existing trading components (`TradeTicket`, `QuotePreviewCard`, `RoleReadinessCard`, `TestnetReadinessHelper`, `BalancesCard`, `PositionsTable`, `TradeHistoryTable`, `PortfolioSummary`, etc.) — unchanged; reused as-is in the bottom panel and via the V3 trade ticket on `/markets/[productId]`

## 14. Validations

| Check | Result |
|---|---|
| `git diff --check` (frontend) | clean |
| `git diff --check` (backend) | clean |
| Sensitive-string scan (milestone files) | zero hits |
| Mainnet RPC pattern scan | zero hits |
| Positive-claim drift scan | zero true hits (only the new chain spec's `.not.toMatch()` negative assertions) |
| Amber/yellow/orange class scan on public-facing src | zero hits |
| Admin bearer scan | zero hits |
| Private RPC URL scan | zero hits |
| DATABASE_URL scan | zero hits |
| `.env` mtime preserved | YES — `2026-06-08 16:55:05` |
| Private file mode 600 preserved | YES; NOT read; NOT committed |
| Chain transaction sent | NO |
| Broadcast invoked | NO |
| Mainnet RPC used | NO |
| Real wallet used | NO |
| Source changes outside frontend / docs / RUN_STATE | NONE |
| Backend Rust source changes | NONE |
| Solidity source changes | NONE |
| Audit firm contacted | NO |
| Bug bounty launched | NO |
| Announcement published | NO |
| `isMainnetEnabled()` still hard-coded `false` | YES |

## 15. Remaining UX gaps

None blocking external testers. V2 candidates (out of scope here):
* Wire **live mark / bid / ask** when the backend exposes them.
* Wire **Greeks** (IV, Δ, Γ, ν, Θ) when the backend exposes a Greeks endpoint.
* Add per-row last-fill indicator from the existing `last_fill` payload on series details.
* Indicator overlays (oracle pin, expiry countdown, spot vs strike highlight) — currently the chain shows expiry per row only.
* Filter by ITM/ATM/OTM.
* Bottom panel: live event feed when the per-wallet events endpoint ships.

## 16. Next milestone recommendation

**Primary:** operator stands up `{{APP_URL}}` (publishable HTTPS URL hosting the deployed Next.js frontend) → re-run `PUBLIC-TESTNET-BETA-LAUNCH-PREFLIGHT` per `PUBLIC_TESTNET_BETA_LAUNCH_PREFLIGHT_RERUN_NEXT_TASK.md`. The terminal is independent of hosting; the launch verdict is still gated only on app URL.

**Alternative parallel:** `EXTERNAL_AUDIT_DISPATCH_PREP` — terminal change doesn't affect audit prep.

**Strictly later (NOT NOW):** `PUBLIC-TESTNET-BETA-LAUNCH` (publication) with separate explicit approval line.

**Explicitly NOT recommended now:** mainnet activation, audit firm outreach, bug bounty launch, KMS cutover, Safe migration, flipping `isMainnetEnabled()`, publishing the announcement, faking Greeks / bid / ask data.

Milestone outcome: a self-contained options-chain terminal at `/trade` with the professional Calls | Strike | Puts ladder pattern, a 5-tab detail panel (Trade / Payoff / Greeks / Details / Risk), a 4-tab bottom panel (Balances / Positions / Trades / Events), a hamburger drawer carrying docs/feedback/community/limitations/changelog, refactored top navbar with Trade / Markets / Portfolio / API / Académie, all driven by an honest data model that surfaces "n/a testnet" rather than inventing live data. 96 tests across 24 spec files. Zero source changes outside frontend / docs / RUN_STATE.

**End of FRONTEND-OPTIONS-CHAIN-TERMINAL-V1 result.**
