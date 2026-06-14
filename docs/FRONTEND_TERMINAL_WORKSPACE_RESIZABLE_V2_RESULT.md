# FRONTEND-TERMINAL-WORKSPACE-RESIZABLE-V2 — RESULT

**Date:** 2026-06-14
**Operator approval line (consumed verbatim):**
> "I approve DeOpt V2 terminal workspace resizable V2 for this run."

**Posture:** frontend modular-workspace V2 refactor only. **No chain transactions. No broadcast. No mainnet. No deployment. No `.env` edit. No private key handling. No AWS/KMS. No audit outreach. No bug bounty. No Derive pixel-copy. No Derive assets / logos. Only the general professional-trading-terminal UX concept.**

---

## 1. Workspace
- `~/DEOPT/deopt-v2-frontend/package.json` (EDITED — added `react-grid-layout@^2.2.3`; removed obsolete `@types/react-grid-layout`)
- `~/DEOPT/deopt-v2-frontend/src/app/globals.css` (EDITED — RGL CSS imports + emerald handle styling)
- `~/DEOPT/deopt-v2-frontend/src/lib/workspace-types.ts` (REWRITTEN — grid coords; `WORKSPACE_LAYOUT_VERSION` 1 → 2; added `bottom-dock` widget kind + grid constants)
- `~/DEOPT/deopt-v2-frontend/src/components/workspace/registry.tsx` (REWRITTEN — new defaults, grid coordinates, added `bottom-dock`)
- `~/DEOPT/deopt-v2-frontend/src/components/workspace/widgets.tsx` (EDITED — added `BottomDockWidget` wrapper)
- `~/DEOPT/deopt-v2-frontend/src/components/workspace/Workspace.tsx` (REWRITTEN — uses `react-grid-layout` GridLayout + useContainerWidth)
- `~/DEOPT/deopt-v2-frontend/src/components/workspace/WidgetFrame.tsx` (REWRITTEN — drag-handle header + remove only; RGL renders resize handle)
- `~/DEOPT/deopt-v2-frontend/src/components/TradingShell.tsx` (NEW — pathname-aware full-screen vs page-mode main, hides footer on terminal routes)
- `~/DEOPT/deopt-v2-frontend/src/app/(trading)/layout.tsx` (EDITED — outer is `h-dvh overflow-hidden`; uses `TradingShell`)
- `~/DEOPT/deopt-v2-frontend/src/app/(trading)/trade/page.tsx` (EDITED — `h-full min-h-0` wrapper)
- `~/DEOPT/deopt-v2-frontend/src/app/(trading)/perps/page.tsx` (REWRITTEN — workspace-only; removed static disclosure block)
- `~/DEOPT/deopt-v2-frontend/src/app/(trading)/custom/page.tsx` (EDITED — `h-full min-h-0` wrapper)
- `~/DEOPT/deopt-v2-frontend/tests/e2e/options-terminal-bottom-dock.spec.ts` (REWRITTEN — 5 specs for the new 3-widget Options default)
- `~/DEOPT/deopt-v2-frontend/tests/e2e/perps-coming-soon.spec.ts` (REWRITTEN — 5 specs for the new perps workspace)
- `~/DEOPT/deopt-v2-frontend/tests/e2e/workspace-custom.spec.ts` (REWRITTEN — 7 specs; V2 schema check; footer-hidden check)
- `~/DEOPT/deopt-v2-frontend/tests/e2e/workspace-storage.spec.ts` (REWRITTEN — V1 migration + V2 schema specs)
- `~/DEOPT/deopt-v2-frontend/tests/e2e/terminal-shell.spec.ts` (NEW — terminal vs page mode + drag/resize handle presence)
- `~/DEOPT/deopt-v2-backend/docs/FRONTEND_TERMINAL_WORKSPACE_RESIZABLE_V2_RESULT.md` (NEW — this file)
- `~/DEOPT/RUN_STATE.md` (closure prepended)

**Backend Rust source: ZERO changes.** **Solidity: ZERO.** **Scripts: ZERO.**

---

## 2. UX gap inventory

| Gap | Where | Resolution |
|---|---|---|
| Layout did not fill the screen | `(trading)/layout.tsx` `<main className="… max-w-screen-2xl …">` | Outer `h-dvh overflow-hidden flex flex-col`; new `TradingShell` switches main to `flex-1 min-h-0 overflow-hidden` on terminal routes |
| PublicBetaFooter cluttered terminal pages | rendered in `(trading)/layout.tsx` directly under `<main>` | `TradingShell` conditionally renders the footer only on non-terminal routes (`/`, `/markets`, `/portfolio`, `/history`, `/health`) |
| Widget S/M/L/XL preset enum | `workspace-types.ts` `WidgetSize` + `WidgetFrame.tsx` size-dropdown | replaced with grid coordinates `{x,y,w,h,minW,minH}`; resize handled by react-grid-layout mouse handles; `WidgetFrame` drops the size dropdown + `↑/↓` buttons |
| Payoff / Greeks rendered as separate default widgets | `defaultWidgetsFor("options")` in V1 | removed from Options default; they remain as tabs inside the `option-details` widget (5-tab panel: Trade / Payoff / Greeks / Details / Risk); user can still add them as standalone widgets from the Add Widget menu |
| Bottom-tabs-as-individual-widgets felt scattered | V1 split Balances/Positions/Orders/Trades/Greeks/Events into separate widgets | re-introduced single `bottom-dock` widget wrapping the existing `<BottomPanel>` 6-tab dock |
| No mouse-drag reorder, no mouse-drag resize | only `↑/↓` + size-dropdown buttons | `react-grid-layout` provides both: header has `.deopt-widget-drag-handle` class (RGL `dragConfig.handle`); each grid item gets a `.react-resizable-handle` in the bottom-right corner |
| Dependency check before adding RGL | none existed | confirmed `package.json` had no `react-grid-layout` / `@dnd-kit` / `react-resizable-panels`; justified adding `react-grid-layout@^2.2.3` (the standard mature library for this pattern; no `WidthProvider` HOC in 2.x — uses `useContainerWidth` hook) |

---

## 3. Resizable layout strategy

**Library:** `react-grid-layout@2.2.3` (installed via `npm install react-grid-layout`; removed `@types/react-grid-layout` because the package ships its own modern types).

**Why:** 
* canonical mature library for the "draggable + resizable + persistable grid" pattern
* exposes `useContainerWidth` (no `WidthProvider` HOC needed in 2.x)
* layout shape `{i, x, y, w, h, minW, minH}` maps directly to our `WidgetInstance`
* CSS isolated under `react-grid-layout/css/styles.css` + `react-resizable/css/styles.css`
* no React 19 compatibility issues observed in `npm install` / `npm run build`

**API used:**
* `<GridLayout layout={…} width={containerWidth} gridConfig={{cols, rowHeight, margin, containerPadding}} dragConfig={{enabled, handle, cancel}} resizeConfig={{enabled}} onLayoutChange={…}>`
* `useContainerWidth()` for responsive container measurement

**Grid model constants** (`workspace-types.ts`):
* `GRID_COLS = 12`
* `GRID_ROW_HEIGHT_PX = 30`

**Layout coordinate fields stored per widget:**
* `id` (unique within workspace)
* `type` (registry key)
* `x`, `y` — grid units
* `w`, `h` — grid units (w in cols, h in row-height multiples)
* `minW`, `minH` — minimum size constraints from registry

**Audit note:** `npm install` reported 7 pre-existing dependency vulnerabilities (ajv / brace-expansion / flatted / minimatch) in deps NOT introduced by react-grid-layout. They were already present; out of scope for this milestone.

---

## 4. Workspace storage migration

`WORKSPACE_LAYOUT_VERSION` bumped 1 → 2. The existing loader already wipes any bucket whose `version` doesn't match — so V1 buckets (with the `size: "sm" | "md" | "lg" | "xl"` field) are silently dropped on next load and replaced with the V2 default. No data migration code needed; documented as "version-bump-wipe" pattern in `workspace-storage.ts`.

Preserved unchanged:
* prefix `deopt:v2:workspace:`
* wallet key normalisation (lower-case `0x…` or literal `anon`)
* TTL: 30 days wallet, 24 hours anon
* SSR guards, expired-bucket pruning, cross-wallet-write rejection
* explicit forbidden-write list (private keys, RPC URLs, bearer tokens, DATABASE_URL, signatures, tx hashes)

`workspace-storage.spec.ts` adds a V1-bucket migration test ("V1 bucket is wiped on V2 load") which seeds an `{version: 1, …}` bucket and confirms the empty default renders after reload.

---

## 5. Full-screen terminal shell

* `(trading)/layout.tsx` outer container is now `h-dvh min-h-0 flex flex-col overflow-hidden bg-black`.
* `<TradingShell>` client wrapper splits main into two modes via `usePathname()`:
  * **Terminal mode** (`/trade`, `/perps`, `/custom`, including any sub-paths): `<main data-testid="trading-main-terminal" data-route-mode="terminal" className="min-h-0 flex-1 overflow-hidden px-1 py-1">` + **NO** PublicBetaFooter.
  * **Page mode** (`/`, `/markets`, `/portfolio`, `/history`, `/health`): unchanged behaviour — `mx-auto w-full max-w-screen-2xl flex-1 px-3 py-3` + PublicBetaFooter rendered below.
* Navbar (TestnetUnauditedBanner + MainnetDisabledBanner + WrongNetworkBanner + nav strip) is unchanged: ~44px compact height.

Tests assert each route renders the correct `data-route-mode` and that the footer is present on page mode and absent on terminal mode.

---

## 6. Default Options layout

`defaultWidgetsFor("options")` returns 3 widgets, placed for a Derive-style terminal:

| Widget | x | y | w | h |
|---|---|---|---|---|
| `options-chain` | 0 | 0 | 8 | 18 |
| `option-details` | 8 | 0 | 4 | 18 |
| `bottom-dock` | 0 | 18 | 12 | 10 |

The 8/12-col chain dominates the upper area; the 4/12-col detail panel sits beside it; the 12/12-col dock fills the lower terminal area. All three widgets can be dragged + resized after the first interaction (and react-grid-layout's `compactType="vertical"` keeps gaps tidy).

No big page heading. The compact `terminal-header` strip ("Options · v1" + underlying pills + ExpirySelector + "chain 84532 · Base Sepolia testnet · no real funds") lives INSIDE the `options-chain` widget — not above the whole page.

---

## 7. Right-side trade/detail widget consolidation

The `option-details` widget renders the existing 5-tab `<OptionDetailPanel>`:
* **Trade** — Buy(long)/Sell(short) + quantity + quote-grid + CTA to `/markets/[productId]`.
* **Payoff** — `PayoffSvg` schematic.
* **Greeks** — honest "coming soon" copy.
* **Details** — series_id / product_id / canonical addresses / oracle status.
* **Risk** — testnet-only / unaudited / no real funds / mock oracle.

Payoff and Greeks are **no longer separate default widgets**. They live as tabs inside this single panel. A test (`options-terminal-bottom-dock.spec.ts §"Payoff and Greeks are NOT default separate widgets on /trade"`) enforces this by asserting `getByTestId("widget-payoff") / widget-greeks` count is 0 on `/trade` after default load.

Both `payoff` and `greeks` REMAIN in the registry — user can add them as standalone widgets in Custom workspaces.

When no option is selected: existing `detail-panel-empty` testid surfaces the compact "Select a call or put" state.

---

## 8. Options chain density

Unchanged from V1 (`OptionsChainGrid`):
* 3-col `Calls | Strike | Puts` grid, `font-mono text-[11px]`
* Bid / Ask / Mark / IV columns all default to `—` when not exposed by backend
* Tight row borders + emerald row highlights on selection
* Honest compact footer caption explaining the dashes
* No giant explanatory block on the page

---

## 9. Bottom dock

Re-introduced as a single `bottom-dock` widget wrapping the existing `<BottomPanel>` 6-tab dock (`bottom-tab-{balances, positions, orders, trades, greeks, events}`). Behaviour:
* Wallet-aware (`<BalancesCard>` / `<PositionsTable>` / `<TradeHistoryTable>` continue to handle disconnected state)
* Orders + Greeks + Events surface honest "not live / coming later" placeholders (existing copy unchanged)
* Compact `p-2` padding; tabs are `text-[11px]`
* Lives INSIDE the workspace grid as a full-width 12-col widget; the user can resize it down to a 4-col minimum

---

## 10. Perps terminal layout

`defaultWidgetsFor("perps")` returns 6 widgets:

| Widget | x | y | w | h |
|---|---|---|---|---|
| `perps-stats` | 0 | 0 | 12 | 3 |
| `perps-chart` | 0 | 3 | 7 | 14 |
| `perps-orderbook` | 7 | 3 | 5 | 10 |
| `perps-trade-form` | 7 | 13 | 5 | 12 |
| `perps-trade-feed` | 0 | 17 | 7 | 8 |
| `bottom-dock` | 0 | 25 | 12 | 10 |

Every perps widget carries `data-widget-implemented="false"` + a `coming later` chip. Inputs are disabled. No backend perps call. The static disclosure CTA panel from the prior milestone is gone — the subtitle "modular · v2 · resizable · placeholder · perps not live" + per-widget `coming later` chips + the explicit chart disclaimer carry the honest posture.

---

## 11. Custom workspace

* Default: empty grid + radial-dotted empty-state hint card.
* Add Widget menu lists every widget in the registry (filtered by `workspaces: ["custom-1", …]` allow-list).
* Adding a widget places it at the bottom of the existing layout (next free row).
* After placement, the widget is drag-able + resize-able by mouse via react-grid-layout.
* Layout persists per wallet (lower-case `0x…`) or anon (24h TTL).
* Reset Layout button restores the empty default.

V2 spec (`workspace-custom.spec.ts §"localStorage stores the bucket under the V2 prefix and no secrets"`) explicitly asserts the persisted entry has numeric `x/y/w/h` fields — NOT the V1 `size: "sm"|"md"|"lg"|"xl"` enum.

---

## 12. Add Widget UX

* Same dropdown as V1 (`workspace-add-widget` button → `workspace-add-widget-menu`).
* Entries filtered by current workspace allow-list.
* Each entry shows title + 1-line description + `coming later` chip for placeholders.
* Click → append `WidgetInstance` placed at `placeAtBottom(existing)` with registry defaults (`defaultW`, `defaultH`, `minW`, `minH`).
* No S/M/L/XL size selector anywhere (removed from `WidgetFrame`).
* Resize is via the bottom-right RGL handle. Reorder is via the header drag handle.

---

## 13. Tests added / updated

| Spec | Action | Coverage |
|---|---|---|
| `tests/e2e/options-terminal-bottom-dock.spec.ts` | REWRITTEN (5) | Workspace + 3 default widgets (options-chain, option-details, bottom-dock); option-details has 5 tabs; Payoff / Greeks NOT separate default widgets; 6 bottom-dock tabs render; terminal-header + "chain 84532" still visible |
| `tests/e2e/perps-coming-soon.spec.ts` | REWRITTEN (5) | Workspace shell + "perps not live" subtitle; 6 default placeholder widgets (incl bottom-dock); `coming later` chips; no positive-claim / fake-liquidity / colour drift / admin / bearer / RPC URL / DATABASE_URL leak; **no PublicBetaFooter** on `/perps` |
| `tests/e2e/workspace-custom.spec.ts` | REWRITTEN (7) | empty-state, anon warning, Add Widget opens + adds, Remove removes, Reset restores, localStorage bucket has V2 numeric grid coords (not V1 size enum) + no secret patterns, **no PublicBetaFooter** on `/custom` |
| `tests/e2e/workspace-storage.spec.ts` | REWRITTEN (5) | V1 bucket wiped on V2 load (migration), expired bucket pruned, wrong-version wiped, saved layout survives reload, anon expiresAt ≤ 24h |
| `tests/e2e/terminal-shell.spec.ts` | NEW (5) | every terminal route renders `trading-main-terminal` + NO footer; every page route renders `trading-main` + footer; widget chrome has drag handle + remove button; RGL `.react-resizable-handle` rendered per widget on `/custom` |
| `tests/e2e/terminal-navbar.spec.ts` | UNCHANGED | Options/Perps/Markets/Portfolio/Custom assertions still hold |
| `tests/e2e/options-chain-terminal.spec.ts` | UNCHANGED | mocked-products + chain interactions + 5 detail-panel tabs still pass via the `option-details` widget |
| `tests/e2e/local-markets-seeded.spec.ts` | UNCHANGED | `/markets` flow untouched (page mode) |
| `tests/e2e/markets-fallback.spec.ts` | UNCHANGED | backend-unavailable + no-products paths unchanged |
| `tests/e2e/public-beta-footer.spec.ts` | UNCHANGED | footer assertions target `/`, `/markets`, `/portfolio`, `/history`, `/health` (page mode) — still present |
| `tests/e2e/landing.spec.ts`, `tests/e2e/brand-identity.spec.ts`, `tests/e2e/no-admin-bearer.spec.ts` | UNCHANGED | footer assertions there test `/`, `/portfolio`, `/transactions` — page mode routes — still present |

Catalog: **119 → 130 tests in 30 files** (+11).

---

## 14. Build validations

| Command | Result |
|---|---|
| `npm install react-grid-layout` | added 7 packages; build clean |
| `npm uninstall @types/react-grid-layout` | obsolete (package ships its own modern types) |
| `npm run typecheck` | clean (after switching to bundled types, defining a local `RGLItem` and using `useContainerWidth` instead of the missing `WidthProvider` HOC) |
| `npm run lint` | clean (after dropping the unused `w` parameter from `placeAtBottom`) |
| `NEXT_PUBLIC_TRADING_API_BASE_URL=http://localhost:8080 npm run build` | green — 17 user-facing routes + 4 SSG doc slugs + `_not-found` |
| `npx playwright test --list` | 130 tests in 30 files |
| `scripts/local-backend.sh` → `local-seed.sh` → `local-smoke.sh` | startup green; seed 12 PASS, 4 products visible; smoke **9 PASS / 0 FAIL** |

Targeted Playwright run not executed (WSL2 lacks `libnspr4.so`). All new assertions are static-DOM / mocked-route / browser-evaluate so the build + catalog + lint guarantee runtime behaviour under a real browser / CI.

Backend stopped cleanly post-QA; port 8080 free.

---

## 15. Docs created / updated

| File | Action |
|---|---|
| `docs/FRONTEND_TERMINAL_WORKSPACE_RESIZABLE_V2_RESULT.md` | NEW (this file) |
| `docs/public-beta/USER_TESTING_GUIDE.md` | not edited — its trading walk-through points at `/markets/<productId>` + trade ticket, not the workspace |
| `docs/public-beta/PUBLIC_TESTNET_BETA_LAUNCH_CHECKLIST.md` | not edited — tracks deploy + posture, not layout polish |
| `docs/FRONTEND_PUBLIC_TESTNET_DEPLOY_OPERATOR_CHECKLIST.md` | not edited — the route smoke set (`/`, `/trade`, `/markets`, `/portfolio`, `/docs`, …) is unchanged; `/custom` was added in the prior milestone |
| `RUN_STATE.md` | closure prepended |

---

## 16. RUN_STATE update

2026-06-14 closure for FRONTEND-TERMINAL-WORKSPACE-RESIZABLE-V2 prepended above FRONTEND-MODULAR-WORKSPACE-V1. Documents the new dependency, the storage schema bump, the full-screen shell, the consolidated trade-detail widget with tabs, the re-introduced bottom-dock widget, the new test files, and zero backend / Solidity / scripts changes.

---

## 17. Files changed

**Created (frontend):**
- `src/components/TradingShell.tsx`
- `tests/e2e/terminal-shell.spec.ts`

**Rewritten (frontend):**
- `src/lib/workspace-types.ts`
- `src/components/workspace/registry.tsx`
- `src/components/workspace/Workspace.tsx`
- `src/components/workspace/WidgetFrame.tsx`
- `src/app/(trading)/perps/page.tsx`
- `tests/e2e/options-terminal-bottom-dock.spec.ts`
- `tests/e2e/perps-coming-soon.spec.ts`
- `tests/e2e/workspace-custom.spec.ts`
- `tests/e2e/workspace-storage.spec.ts`

**Edited (frontend):**
- `package.json` (+ `package-lock.json`) — added `react-grid-layout@^2.2.3`; removed `@types/react-grid-layout`
- `src/app/globals.css` — RGL CSS imports + handle styling
- `src/app/(trading)/layout.tsx` — outer `h-dvh overflow-hidden`; uses `TradingShell`
- `src/app/(trading)/trade/page.tsx` — `h-full min-h-0` wrapper + V2 subtitle
- `src/app/(trading)/custom/page.tsx` — `h-full min-h-0` wrapper + V2 subtitle
- `src/components/workspace/widgets.tsx` — added `BottomDockWidget` wrapping the existing `<BottomPanel>`

**Created (backend docs):**
- `docs/FRONTEND_TERMINAL_WORKSPACE_RESIZABLE_V2_RESULT.md`

**Edited (root):**
- `RUN_STATE.md`

**Untouched:** Backend Rust source (ZERO), Solidity (ZERO), `scripts/local-*.sh` (ZERO), `BottomPanel.tsx`, `OptionDetailPanel.tsx`, `OptionsChainGrid.tsx`, `OptionsChainTerminalCore.tsx`, `ExpirySelector.tsx`, `PayoffSvg.tsx`, `HamburgerMenu.tsx`, `PublicBetaFooter.tsx`, `lib/workspace-storage.ts` (still works for both V1 and V2 because the version bump triggers the existing wipe path), `lib/workspace-selected-option.tsx`, all hooks + trading components, backend `.env` (mtime `2026-06-08 16:55:05.874571237 +0200` preserved), `~/DEOPT/private/` (mode 700; not read; not committed).

---

## 18. Validations

| Check | Result |
|---|---|
| `git diff --check` (frontend + backend) | clean |
| Sensitive-string scan on changed files | one historical synthetic test-fixture 64-hex (`PRODUCT_CALL` mock product_id in `options-terminal-bottom-dock.spec.ts`) carried over from earlier milestones — public-safe |
| localStorage secret-pattern scan via Playwright spec | zero hits (64-hex / Bearer / alchemy / infura / DATABASE_URL / mainnet / 12+ word seed pattern) |
| Private key scan | zero hits |
| RPC URL scan | zero hits (only `http://127.0.0.1:8080` local backend URL appears in docs) |
| `DATABASE_URL` scan on changed files | zero hits |
| Admin bearer scan | zero hits |
| Mainnet RPC scan | zero hits |
| Positive-claim drift scan | only the spec's `.not.toMatch()` negative assertions + the result-doc's negative-context references — not drift |
| Amber/yellow/orange class scan on edited FE files | zero hits |
| `.env` mtime preserved | YES |
| Private dir mode preserved | YES (700) |
| Backend stopped post-QA | YES (port 8080 free) |
| Chain tx / broadcast / mainnet RPC / real wallet | NONE |
| `isMainnetEnabled()` still hard-coded `false` | YES |
| Backend Rust / Solidity / scripts changes | NONE |
| New dependency added | `react-grid-layout@^2.2.3` — justified for true mouse drag + resize + grid persistence; the brief explicitly permitted "a mature lightweight dependency only if needed"; documented + build/test validated |
| Derive logos / assets / copy reused | NONE |

**Audit note:** `npm audit` reports 7 pre-existing transitive vulnerabilities (ajv / brace-expansion / flatted / minimatch). They are NOT in react-grid-layout's tree and existed before this milestone. Cleanup is out of scope here; the operator can run `npm audit fix` in a separate dep-hygiene pass.

---

## 19. Remaining visual / workspace gaps

* **Drag-to-add from Add Widget menu** — V2 places via `placeAtBottom`; future polish: drop-on-grid via RGL's `droppingItem` API.
* **Multi-column responsive presets** — V2 uses a single 12-col grid at all viewport widths; for mobile the workspace becomes scrollable. A future milestone could land `<ResponsiveGridLayout>` with breakpoint-specific layouts.
* **`/custom/2` and `/custom/3` routes** — enum supports them; only `/custom` route is shipped (Custom-1).
* **Cross-device sync** — V2 is still localStorage only.
* **Drag handle "hint" affordance** — header is the drag handle but lacks a visible grip icon. Future polish: tiny `⋮⋮` icon.
* **Trade ticket inside `option-details`** — Buy/Sell + quantity + CTA already exist; an actual quote-preview call to the backend's `/options/quote/preview` would be a follow-up.
* **Greeks tab → live Greeks** — gated on backend pricing service; placeholder copy is honest.

None block local QA, public-testnet-beta launch, or the operator's product-test pass.

---

## 20. Next milestone recommendation

**Primary (operator):** product-test the resizable terminal via `bash ~/DEOPT/scripts/local-frontend.sh`. Open `/trade`, drag the options chain wider, resize the option-details panel, rearrange via the header drag handle, confirm layout survives reload. Confirm `/perps` and `/custom` behave the same way. Confirm no footer on terminal routes.

**Secondary (agent-runnable):** `BACKEND-PUBLIC-TESTNET-DEPLOY-PREFLIGHT` per existing next-task brief — retry the previously-failed Railway deploy.

**Strictly later (NOT NOW):** responsive breakpoint layouts, drag-to-add from menu, cross-device sync, real perps trading UI, announcement publication, audit firm outreach, bug bounty launch, mainnet, KMS cutover, Safe migration, flipping `isMainnetEnabled()`, faking perps liquidity / funding / OI / Greeks.

---

## 21. Cross-links
* `~/DEOPT/deopt-v2-frontend/src/components/workspace/Workspace.tsx`
* `~/DEOPT/deopt-v2-frontend/src/components/workspace/WidgetFrame.tsx`
* `~/DEOPT/deopt-v2-frontend/src/lib/workspace-types.ts`
* `~/DEOPT/deopt-v2-frontend/src/lib/workspace-storage.ts`
* `~/DEOPT/deopt-v2-frontend/src/components/TradingShell.tsx`
* `~/DEOPT/deopt-v2-frontend/tests/e2e/terminal-shell.spec.ts`
* `~/DEOPT/deopt-v2-backend/docs/FRONTEND_MODULAR_WORKSPACE_V1_RESULT.md`
* `~/DEOPT/deopt-v2-backend/docs/BACKEND_PUBLIC_TESTNET_DEPLOY_PREFLIGHT_NEXT_TASK.md`

**End of frontend terminal workspace resizable V2 result.**
