# FRONTEND-TRADING-TERMINAL-DERIVE-LIKE-LAYOUT — RESULT

**Date:** 2026-06-13
**Operator approval line (consumed verbatim):**
> "I approve DeOpt V2 trading terminal Derive-like layout polish for this run."

**Posture:** Frontend UI polish only. **No chain transactions. No broadcast. No mainnet. No deployment. No `.env` edit. No private key handling. No AWS/KMS. No audit outreach. No bug bounty. No copying of Derive assets / pixel layout / logos — only the general professional trading-terminal UX pattern (dense full-width layout, dominant central grid, sticky right detail panel, bottom dock, compact top nav).**

---

## 1. Workspace
- `~/DEOPT/deopt-v2-frontend/src/app/(trading)/layout.tsx` (EDITED — compact navbar height, wider container)
- `~/DEOPT/deopt-v2-frontend/src/app/(trading)/trade/page.tsx` (REWRITTEN — removed marketing-style hero; terminal-shell wrapper only)
- `~/DEOPT/deopt-v2-frontend/src/app/(trading)/perps/page.tsx` (REWRITTEN — terminal-style placeholder)
- `~/DEOPT/deopt-v2-frontend/src/components/trading/terminal/OptionsChainTerminal.tsx` (EDITED — denser header, sticky right panel, wider chain)
- `~/DEOPT/deopt-v2-frontend/src/components/trading/terminal/BottomPanel.tsx` (REWRITTEN — 6 tabs: + Orders + Greeks; honest placeholders)
- `~/DEOPT/deopt-v2-frontend/tests/e2e/perps-coming-soon.spec.ts` (UPDATED — terminal sections)
- `~/DEOPT/deopt-v2-frontend/tests/e2e/options-terminal-bottom-dock.spec.ts` (NEW, 3 specs)
- `~/DEOPT/deopt-v2-backend/docs/FRONTEND_TRADING_TERMINAL_DERIVE_LIKE_LAYOUT_RESULT.md` (NEW — this file)
- `~/DEOPT/RUN_STATE.md` (closure paragraph prepended)

**Backend Rust source: ZERO changes.** **Solidity: ZERO.** **Scripts: ZERO.**

---

## 2. Visual gap inventory

Pre-fix observations (described as product UX guidance, not legal/copying analysis):

| Gap | Pre-fix | Post-fix |
|---|---|---|
| Page max-width | `max-w-6xl mx-auto px-4 py-6` — content felt centered + narrow | `max-w-screen-2xl px-3 py-3 lg:px-4 lg:py-4` — viewport-width terminal |
| Navbar height | `py-3` + `text-sm` + `gap-3/4` | `py-2` + `text-[13px]` + `gap-2/3` — compact terminal strip |
| `/trade` H1 hero | giant `<h1>Options chain</h1>` + sub-line above the chain | replaced with compact `<header>` strip embedded in the terminal carrying underlying pills, expiry selector, and chain-id / testnet / no-real-funds stats |
| Chain dominance | `lg:grid-cols-[1fr_22rem]` | `lg:grid-cols-[1fr_24rem] xl:grid-cols-[1fr_26rem]` — denser, slightly larger right panel |
| Right panel stickiness | scrolled with the chain | wrapped in `lg:sticky lg:top-2 lg:self-start` — pinned beside the chain on desktop |
| Bottom dock tabs | 4 (Balances / Positions / Trades / Events) | 6 (Balances / Positions / **Orders** / Trades / **Greeks** / Events) — Orders + Greeks surface honest "not live / coming later" copy |
| `/perps` page | text-only placeholder | full terminal-style layout: stats strip + chart placeholder + orderbook placeholder + trade-form placeholder + bottom dock + disclosure + meanwhile CTAs |

---

## 3. Terminal shell layout (`(trading)/layout.tsx`)

* `<header>` reduced from `py-3` + `text-sm` to `py-2` + `text-[13px]` with `gap-2`. Single border-bottom defines the strip.
* `<main>` container expanded from `max-w-6xl` to `max-w-screen-2xl` with tighter padding (`px-3 py-3 lg:px-4 lg:py-4`).
* Navbar palette unchanged: black background, emerald hover, zinc dividers.

This affects every `(trading)/*` route at once — including `/trade`, `/perps`, `/markets`, `/portfolio` — so the visual identity is consistent.

---

## 4. Options page layout (`OptionsChainTerminal.tsx`)

* `<div data-testid="terminal-shell">` wraps the whole terminal so tests can assert the shell.
* `<header data-testid="terminal-header">` is now a single compact strip:
  - Left: "Options · v1" chip + Underlying pills (existing `underlying-pill-…` testids preserved).
  - Center-right: `ExpirySelector` (existing `expiry-pill-{ms}` testids preserved).
  - Right: `terminal-stat-chain` cluster — "chain 84532 · Base Sepolia testnet · no real funds".
* Grid: `lg:grid-cols-[minmax(0,1fr)_24rem] xl:grid-cols-[minmax(0,1fr)_26rem]` — chain dominates, detail panel widens on larger displays.
* Right detail panel wrapped in a sticky container (`lg:sticky lg:top-2 lg:self-start`) so it remains pinned while the chain scrolls.
* `BottomPanel` follows directly under the grid as a dense dock.

`/trade/page.tsx` is reduced to a single `<div data-testid="trade-shell">` wrapper hosting `<OptionsChainTerminal />`. The previous H1/sub-title hero is gone; all surfacing happens inside the terminal header strip.

The existing `OptionsChainGrid` already renders Calls | Strike | Puts with `font-mono`, tight borders, and emerald row highlights — so it remains the dominant element. No further density changes were needed.

---

## 5. Options right panel (`OptionDetailPanel.tsx`)

Untouched in this milestone — the 5-tab structure (Trade / Payoff / Greeks / Details / Risk) introduced under FRONTEND-OPTIONS-CHAIN-TERMINAL-V1 already matches the spec, and the brief explicitly notes Greeks must surface honest "n/a testnet". Wrapping it in a sticky container on the parent gives it the terminal-style pinned-side-panel feel without changes to the panel itself.

If no option is selected, the existing `detail-panel-empty` testid continues to surface the compact "Select a call or put" state.

---

## 6. Bottom panel (`BottomPanel.tsx`)

Rewritten:

* 6 tabs in this exact order: **Balances / Positions / Orders / Trades / Greeks / Events**.
* Container padding reduced from `p-4` to `p-2`. Tab bar padding compressed.
* New `PlaceholderCard` helper renders Orders / Greeks / Events with identical structure: emerald-border card with a tracking-`[0.18em]` uppercase title + body paragraph.
* **Orders** placeholder: "Orders — not live in this testnet beta. The options trade flow is intent → sign → executor-broadcast; there is no resting limit-order book yet. Inspect a specific trade lifecycle via /transactions/<intent_id>."
* **Greeks** placeholder: "Greeks — coming later in the testnet beta. Delta / Gamma / Vega / Theta are not exposed by the current backend. The chain and detail panel already render honest dashes for greeks; this dock will surface portfolio-level greeks once the backend pricing service ships."
* **Events** placeholder: unchanged copy, now uses the shared helper.
* Balances / Positions / Trades continue to reuse `BalancesCard` / `PositionsTable` / `TradeHistoryTable`.

Each placeholder carries a stable `data-testid` (`bottom-panel-orders-placeholder`, etc.) so the new test spec can assert without DOM-text matching.

---

## 7. Perps terminal placeholder (`(trading)/perps/page.tsx`)

Complete rewrite from text-page to terminal-style placeholder, with **honest absence of every live signal**:

| Section | testid | Content |
|---|---|---|
| Header strip | `perps-header`, `perps-status-chip` | "Perps · placeholder" chip + "coming later in the public testnet beta" + chain / testnet / no-real-funds stats |
| Stats strip | `perps-stats-strip`, 6× `perps-stat-{underlying, mark, change-24h, volume-24h, funding, open-interest}` | Each cell shows `—` value + `not live` hint |
| Chart panel | `perps-chart-panel`, `perps-chart-svg`, `perps-chart-disclaimer` | Inline SVG with a `pattern`-rendered grid + a schematic emerald sparkline; clear disclaimer "Perps are not live in this testnet beta yet. The visual above is a schematic sparkline — no real price feed, no real funding rate, no real open interest." |
| Orderbook | `perps-orderbook-panel`, 5× `perps-orderbook-row-{i}` | 5 empty rows of Bid / Size / Ask, all `—`; "not live" badge |
| Trade ticket | `perps-trade-form-panel`, 5× `perps-trade-{buy, sell, size, leverage, submit}` | All inputs disabled; submit button reads "Perps trading not live" |
| Bottom dock | `perps-bottom-dock`, 5× `perps-bottom-tab-{balances, positions, orders, trades, funding}` | All tabs are aria-disabled `<span>`s — clearly inert |
| Disclosure | `perps-disclosure-panel` | Bulleted list: no perps live / no fake bid-ask-mark-IV / no real funds / unaudited / experimental / not financial advice |
| Meanwhile CTAs | `perps-meanwhile-panel` + 4× `perps-cta-{options, docs, discord, feedback}` | Try Options / Read docs / Open Discord (external) / Send feedback |

No external chart library. No backend perps call (the page does not import `useProducts` or `trading-api`). Emerald + zinc only — no amber/yellow/orange. Static SVG sparkline carries only `stroke-opacity 0.45` + `fill-opacity 0.06` so it reads as schematic not authoritative.

---

## 8. Navbar polish

Untouched in this milestone — the prior FRONTEND-NAVBAR-OPTIONS-PERPS milestone already shipped the Options / Perps / Markets / Portfolio / API / Académie + hamburger structure. The only nav change this milestone is the height/font-size/spacing compression in the layout shell (§3).

- Options active on `/trade` (route unchanged); `/options` alias still not added (intentional — label-only rename is safer).
- Perps active on `/perps` (now terminal-style).
- API + Académie remain `ComingSoonNavLink` aria-disabled placeholders.
- Hamburger drawer still carries Docs / Quickstart / Feedback / Discord / GitHub / Limitations / Changelog placeholder. No admin / mainnet links.

---

## 9. Responsive behaviour

* **Desktop ≥ `lg`**: grid is `[1fr_24rem]` (`xl: [1fr_26rem]`); detail panel sticky. Bottom dock + perps panels follow the same denser breakpoint pattern.
* **Tablet < `lg`**: grid collapses to single column — chain on top, detail panel below.
* **Mobile**: navbar wraps via `flex-wrap`; chain rows remain `font-mono text-[11px]` — horizontally scrollable via the parent if a row overflows.
* `OptionsChainGrid` itself is unchanged so its existing responsive behaviour (3-col grid with `minmax(7rem,_auto)` strike column) carries over.

---

## 10. Tests added / updated

| Spec | Action | Coverage |
|---|---|---|
| `tests/e2e/options-terminal-bottom-dock.spec.ts` | NEW (3) | all 6 bottom tabs visible; Orders + Greeks surface honest "not live / coming later" copy; `terminal-shell` + `terminal-header` + `terminal-stat-chain "chain 84532"` render |
| `tests/e2e/perps-coming-soon.spec.ts` | UPDATED | stats strip + 6 stat cells + chart panel + chart SVG + orderbook panel + trade-form panel + bottom dock + 5 dock tabs all visible; trade-form CTAs all disabled; disclosure surfaces testnet posture; meanwhile CTAs link to Options/Docs/Discord/Feedback |
| `tests/e2e/terminal-navbar.spec.ts` | UNCHANGED | Options/Perps/Markets/Portfolio + hamburger drawer assertions still hold |
| `tests/e2e/options-chain-terminal.spec.ts` | UNCHANGED | mocked products/series + chain interactions + 5 detail-panel tabs + payoff SVG + no-positive-claim scans all still hold |
| `tests/e2e/local-markets-seeded.spec.ts` | UNCHANGED | `/markets` product cards path unchanged |
| `tests/e2e/markets-fallback.spec.ts` | UNCHANGED | backend-unavailable + no-products paths unchanged |

Catalog: **104 → 109 tests in 27 files** (+5).

---

## 11. Build validations

| Command | Result |
|---|---|
| `npm run typecheck` | clean |
| `npm run lint` | clean |
| `NEXT_PUBLIC_TRADING_API_BASE_URL=http://localhost:8080 npm run build` | green — 16 user-facing routes + 4 SSG doc slugs + `_not-found` |
| `npx playwright test --list` | 109 tests in 27 files |
| `bash ~/DEOPT/scripts/local-backend.sh` → `local-seed.sh` → `local-smoke.sh` | startup green; seed 12 PASS, 0 SKIP, 4 products visible; smoke **9 PASS / 0 FAIL** |

Targeted Playwright run not executed (WSL2 lacks `libnspr4.so`); all new assertions are static-DOM / mocked-route so the build + catalog parse + lint guarantee runtime behaviour under a real browser / CI.

Backend stopped cleanly post-QA; port 8080 free.

---

## 12. Docs created / updated

| File | Action |
|---|---|
| `docs/FRONTEND_TRADING_TERMINAL_DERIVE_LIKE_LAYOUT_RESULT.md` | NEW (this file) |
| `docs/public-beta/USER_TESTING_GUIDE.md` | not edited — its trading walk-through points at `/markets/<productId>` and the trade ticket inside the product page, not the page-level terminal layout |
| `docs/public-beta/PUBLIC_TESTNET_BETA_LAUNCH_CHECKLIST.md` | not edited — tracks deploy + posture, not layout polish |
| `docs/FRONTEND_PUBLIC_TESTNET_DEPLOY_OPERATOR_CHECKLIST.md` | not edited — its route smoke list (`/`, `/trade`, `/markets`, `/portfolio`, `/docs`, …) is unchanged |
| `RUN_STATE.md` | closure paragraph prepended |

---

## 13. RUN_STATE update

Closure paragraph for FRONTEND-TRADING-TERMINAL-DERIVE-LIKE-LAYOUT prepended above FRONTEND-NAVBAR-OPTIONS-PERPS-LOCAL-QA. Documents the layout density changes, the new BottomPanel tabs, the `/perps` rewrite, and the unchanged source-change discipline (backend Rust + Solidity + scripts all zero).

---

## 14. Files changed

**Created (frontend):**
- `tests/e2e/options-terminal-bottom-dock.spec.ts`

**Rewritten (frontend):**
- `src/app/(trading)/trade/page.tsx`
- `src/app/(trading)/perps/page.tsx`
- `src/components/trading/terminal/BottomPanel.tsx`

**Edited (frontend):**
- `src/app/(trading)/layout.tsx`
- `src/components/trading/terminal/OptionsChainTerminal.tsx`
- `tests/e2e/perps-coming-soon.spec.ts`

**Created (backend docs):**
- `docs/FRONTEND_TRADING_TERMINAL_DERIVE_LIKE_LAYOUT_RESULT.md`

**Edited (root):**
- `RUN_STATE.md`

**Untouched:** Backend Rust source (ZERO), Solidity (ZERO), `scripts/local-*.sh` (ZERO), `OptionDetailPanel.tsx`, `OptionsChainGrid.tsx`, `ExpirySelector.tsx`, `PayoffSvg.tsx`, `HamburgerMenu.tsx`, `PublicBetaFooter.tsx`, backend `.env` (mtime `2026-06-08 16:55:05.874571237 +0200` preserved), `~/DEOPT/private/` (mode 700; not read; not committed).

---

## 15. Validations

| Check | Result |
|---|---|
| `git diff --check` (frontend + backend) | clean |
| Sensitive-string scan on changed files | zero hits |
| Private key scan | zero hits |
| RPC URL scan | zero hits (only `http://127.0.0.1:8080` local backend URL appears in docs) |
| `DATABASE_URL` scan on changed files | zero hits |
| Admin bearer scan | zero hits |
| Mainnet RPC scan | zero hits |
| Positive-claim drift scan | only the spec's `.not.toMatch()` negative assertions + the result-doc descriptions of what `/perps` does NOT contain — negative-context, not drift |
| Amber/yellow/orange class scan on edited FE files | zero hits |
| `.env` mtime preserved | YES |
| Private dir mode preserved | YES (700) |
| Backend stopped post-QA | YES (port 8080 free) |
| Chain tx / broadcast / mainnet RPC / real wallet | NONE |
| `isMainnetEnabled()` still hard-coded `false` | YES |
| Backend Rust source changes | NONE |
| Solidity source changes | NONE |
| External chart / icon / animation lib added | NONE (raw SVG only) |
| Derive logos / assets / copy reused | NONE (general professional terminal UX only) |

---

## 16. Remaining visual gaps

* **Terminal-grade compact-density font-stack** — using system `font-mono`; a dedicated tabular-figures monospace would improve column alignment further. Out of scope; future polish.
* **Trade tab in `OptionDetailPanel`** — the existing 5-tab structure is solid but could grow a compact wallet-state strip ("Connected · 0x… · Sepolia") between header and Buy/Sell. Out of scope here.
* **Bottom dock horizontal resize / collapse handle** — pro terminals let the user drag dock height. Not implemented; would require client-state + a non-trivial drag handler. Out of scope.
* **Real Greeks / bid / ask / IV / Funding / OI wiring** — gated on backend pricing service; the placeholders explicitly surface this. Out of scope.
* **Mobile chain layout** — currently stacks; a future polish could surface a "Calls" / "Puts" tab toggle below `sm` so users only see one side at a time. Out of scope.

None of these block local visual QA or the public-testnet-beta launch.

---

## 17. Next milestone recommendation

**Primary (operator):** product-test the new terminal feel via `bash ~/DEOPT/scripts/local-frontend.sh`. Compare side-by-side to the old `/trade` and `/perps`. The Calls | Strike | Puts ladder + sticky detail panel + 6-tab dock now render full-width; `/perps` shows a real terminal shell with explicit "not live" placeholders.

**Secondary (agent-runnable):** `BACKEND-PUBLIC-TESTNET-DEPLOY-PREFLIGHT` per the existing next-task brief — retry the previously-failed Railway deploy.

**Strictly later (NOT NOW):** building real perps trading UI, announcement publication, audit firm outreach, bug bounty launch, mainnet, KMS cutover, Safe migration, flipping `isMainnetEnabled()`.

---

## 18. Cross-links
* `~/DEOPT/deopt-v2-frontend/src/app/(trading)/layout.tsx`
* `~/DEOPT/deopt-v2-frontend/src/app/(trading)/trade/page.tsx`
* `~/DEOPT/deopt-v2-frontend/src/app/(trading)/perps/page.tsx`
* `~/DEOPT/deopt-v2-frontend/src/components/trading/terminal/OptionsChainTerminal.tsx`
* `~/DEOPT/deopt-v2-frontend/src/components/trading/terminal/BottomPanel.tsx`
* `~/DEOPT/deopt-v2-frontend/tests/e2e/options-terminal-bottom-dock.spec.ts`
* `~/DEOPT/deopt-v2-frontend/tests/e2e/perps-coming-soon.spec.ts`
* `~/DEOPT/deopt-v2-backend/docs/FRONTEND_NAVBAR_OPTIONS_PERPS_LOCAL_QA_RESULT.md`
* `~/DEOPT/deopt-v2-backend/docs/FRONTEND_OPTIONS_CHAIN_TERMINAL_V1_RESULT.md`
* `~/DEOPT/deopt-v2-backend/docs/BACKEND_PUBLIC_TESTNET_DEPLOY_PREFLIGHT_NEXT_TASK.md`

**End of frontend trading terminal Derive-like layout result.**
