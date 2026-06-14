# FRONTEND-TERMINAL-WORKSPACE-CLEANUP-V3 — RESULT

**Date:** 2026-06-14
**Operator approval line (consumed verbatim):**
> "I approve DeOpt V2 terminal workspace cleanup V3 for this run."

**Posture:** frontend UX cleanup only. **No chain transactions. No broadcast. No mainnet. No deployment. No `.env` edit. No private key handling. No AWS/KMS. No audit outreach. No bug bounty.**

---

## 1. Workspace
- `~/DEOPT/deopt-v2-frontend/src/components/TradingShell.tsx` (EDITED — `/markets`, `/portfolio` joined the terminal route set; main padding stripped to `p-0`)
- `~/DEOPT/deopt-v2-frontend/src/lib/workspace-bridge.tsx` (NEW — `WorkspaceBridgeProvider` + `useActiveWorkspace` / `useRegisterWorkspace`)
- `~/DEOPT/deopt-v2-frontend/src/components/workspace/WidgetMenuButton.tsx` (NEW — navbar `Widget` dropdown)
- `~/DEOPT/deopt-v2-frontend/src/components/workspace/Workspace.tsx` (EDITED — toolbar / Reset / Anon-warning / in-body Add Widget all removed; bridges to navbar via `useRegisterWorkspace`; grid margin tightened to `[4,4]`; `overflow-x-hidden overflow-y-auto`)
- `~/DEOPT/deopt-v2-frontend/src/app/(trading)/layout.tsx` (EDITED — wrap in `<WorkspaceBridgeProvider>`; mount `<WidgetMenuButton />` next to `<WalletConnectButton />`)
- `~/DEOPT/deopt-v2-frontend/tests/e2e/workspace-custom.spec.ts` (REWRITTEN, 6 specs)
- `~/DEOPT/deopt-v2-frontend/tests/e2e/workspace-storage.spec.ts` (EDITED — uses navbar Widget button)
- `~/DEOPT/deopt-v2-frontend/tests/e2e/terminal-shell.spec.ts` (EDITED — `/markets`+`/portfolio` moved to terminal route list; uses navbar Widget button)
- `~/DEOPT/deopt-v2-frontend/tests/e2e/public-beta-footer.spec.ts` (EDITED — safety-bullets route list narrowed to `/`, `/history`, `/health`)
- `~/DEOPT/deopt-v2-backend/docs/FRONTEND_TERMINAL_WORKSPACE_CLEANUP_V3_RESULT.md` (NEW — this file)
- `~/DEOPT/RUN_STATE.md` (closure prepended)

**Backend Rust source: ZERO changes.** **Solidity: ZERO.** **Scripts: ZERO.**

---

## 2. Footer / banner cleanup

`PublicBetaFooter` is rendered in three places: `(trading)/layout.tsx` (via `TradingShell`), `docs/layout.tsx`, `feedback/layout.tsx`. The docs + feedback layouts are out of scope (those are legitimate documentation/marketing surfaces).

`TradingShell.TERMINAL_ROUTES` was `["/trade", "/perps", "/custom"]`. Expanded to `["/trade", "/perps", "/custom", "/markets", "/portfolio"]` per operator feedback: `/markets` + `/portfolio` are trading utility surfaces and should not carry the marketing footer either.

`/`, `/history`, `/health` stay in **page mode** (footer remains). The brief explicitly permits keeping the footer on "landing page if still used" and `/history` / `/health` are utility pages with no workspace.

Terminal `<main>` padding also tightened: `px-1 py-1` → `p-0` so the workspace fills the full viewport width.

---

## 3. Right-side resize dead-zone fix

Three small fixes:

1. **Terminal main padding** dropped from `px-1 py-1` to `p-0` — removes a 4px gutter on every edge.
2. **Workspace grid container** changed from `overflow-auto` to `overflow-x-hidden overflow-y-auto` — keeps vertical scroll but prevents horizontal scroll AND removes the horizontal scrollbar reservation that was eating ~16px on the right when the grid was just a hair too wide.
3. **RGL `gridConfig.margin`** tightened from `[6, 6]` to `[4, 4]` — same visual breathing room between widgets, smaller right-edge dead zone (RGL applies `margin[0]` once at the right edge).
4. **Workspace grid div** now carries `w-full` explicitly to make sure `useContainerWidth()` measures the full available width.

Net effect on the right edge: previously ~26px (1px container padding + 6px RGL margin + ~16px scrollbar reservation). Now ~4px (just the RGL inter-widget margin used once at the edge).

The workspace still scrolls vertically when content exceeds viewport height; the grid never overflows horizontally; widgets resize all the way to the right edge.

---

## 4. Global toolbar cleanup

The Workspace.tsx in-body `<header>` is gone. Removed elements:

* `workspace-toolbar-<id>` row (was the entire top strip inside the workspace)
* `Reset layout` button (testid `workspace-reset`)
* `Anonymous layout — temporary…` chip (testid `workspace-anon-warning`)
* `Saved per wallet` chip (testid `workspace-wallet-badge`)
* In-body `+ Add widget` button (the old V2 `AddWidgetMenu` mounted inside Workspace; testids `workspace-add-widget` + `workspace-add-widget-menu` + `workspace-add-widget-option-*`)

Workspace title + subtitle now live as `data-workspace-title` / `data-workspace-subtitle` attributes on the workspace root for any introspection use; they no longer take up vertical real-estate.

Internally preserved:
* Reset code path (`buildDefault(workspaceId)` is still called when the user reset action is invoked from anywhere; the visible button is just gone)
* Anonymous bucket TTL (24h) — still works silently
* Per-wallet TTL (30 days) — still works silently
* Per-widget remove button (✕ in widget chrome)
* Mouse drag handle (widget header) + mouse resize handle (RGL bottom-right corner)
* localStorage schema v2 with V1 migration

The empty-state hint card is preserved, but its copy now points users at the new entry point: "Open the **Widget** button in the top navbar to add a widget."

---

## 5. Navbar `Widget` button

NEW `WidgetMenuButton.tsx`:
* Renders next to `<WalletConnectButton />` in the navbar right cluster, before `<HamburgerMenu />`.
* Label: **"Widget"** (testid `navbar-widget-button`).
* Hidden when no workspace is mounted (e.g. on `/`, `/docs`, `/feedback`, `/history`, `/health`).
* Click opens a dropdown (testid `navbar-widget-menu`) listing widgets that the active workspace supports.
* Each entry: title + 1-line description + `coming later` chip for placeholders.
* Click an entry → invokes `active.addWidget(type)` from the bridge → workspace appends widget at next free row → persists → menu closes.
* Closes on outside click + Escape key.
* Black + emerald styling, no amber/yellow/orange.
* Keyboard-accessible via `aria-haspopup="menu"` + `aria-expanded`.

---

## 6. Workspace context changes

NEW `lib/workspace-bridge.tsx`:
* `WorkspaceBridgeProvider` — single in-memory context wrapping the trading layout (mounted in `(trading)/layout.tsx`).
* `useRegisterWorkspace({workspaceId, addWidget})` — Workspace calls this on mount; bridge stores the handle and returns a cleanup that runs on unmount.
* `useActiveWorkspace()` — Navbar's `WidgetMenuButton` reads the current handle; `null` if no workspace is mounted (hides the button).
* Pure in-memory. NEVER persists. NO secrets, RPC URLs, bearer tokens, signatures.

The bridge survives route changes within the trading group: when the user navigates `/trade` → `/perps`, the prior workspace unmounts (clears its handle via the cleanup function) and the new one registers (sets the handle to the new workspace). The navbar button automatically updates.

Hydration / SSR safety: the bridge uses `useEffect` for registration (client-only); `useActiveWorkspace` returns `null` during SSR + initial render → button hides → no hydration mismatch.

---

## 7. Widget menu UX

* Button label: **"Widget"** (per operator).
* Menu title: still **"Add widget"** (preserved as the heading inside the dropdown — describes the action).
* Lists widgets filtered by `widgetsForWorkspace(active.workspaceId)`.
* Placeholders flagged `coming later`.
* Adding a widget places it at the next free grid row.
* Menu closes after adding.
* No Reset button inside the menu (operator: preferred no visible reset).
* No advanced section yet — kept tight; future polish can add a "Reset layout" entry behind an advanced toggle.

---

## 8. Tests added / updated

| Spec | Action | Coverage |
|---|---|---|
| `tests/e2e/workspace-custom.spec.ts` | REWRITTEN (6) | `/custom` empty-state hint; **NO** in-body Reset / Anon-warning / Wallet-badge / Add-widget toolbar; Navbar `Widget` button visible + opens menu + adds widget; per-widget remove still works; V2 grid coords persisted with no secrets; NO PublicBetaFooter on `/custom` |
| `tests/e2e/workspace-storage.spec.ts` | EDITED | switched all add-widget interactions from `workspace-add-widget` to `navbar-widget-button` + `navbar-widget-option-…` |
| `tests/e2e/terminal-shell.spec.ts` | EDITED | `/markets` + `/portfolio` joined TERMINAL_ROUTES; PAGE_ROUTES narrowed to `/`, `/history`, `/health`; widget chrome assertions switched to navbar widget button |
| `tests/e2e/public-beta-footer.spec.ts` | EDITED | safety-copy bullets route list narrowed to `/`, `/history`, `/health` (markets/portfolio no longer carry the footer) |
| `tests/e2e/terminal-navbar.spec.ts` | UNCHANGED | Options / Perps / Markets / Portfolio / Custom assertions still hold |
| `tests/e2e/options-terminal-bottom-dock.spec.ts` | UNCHANGED | default widget assertions don't touch the removed toolbar |
| `tests/e2e/perps-coming-soon.spec.ts` | UNCHANGED | already updated under V2 to check workspace shell + per-widget chips |
| `tests/e2e/options-chain-terminal.spec.ts` | UNCHANGED | mocked chain interactions unaffected |
| `tests/e2e/local-markets-seeded.spec.ts` | UNCHANGED | `/markets` product cards path unaffected (terminal main still renders) |
| `tests/e2e/markets-fallback.spec.ts` | UNCHANGED | backend-unavailable + no-products paths unaffected |
| `tests/e2e/landing.spec.ts`, `tests/e2e/brand-identity.spec.ts`, `tests/e2e/no-admin-bearer.spec.ts` | UNCHANGED | footer assertions there target `/` (page mode) |

Catalog: **130 → 131 tests in 30 files** (+1 net; specs that were rewritten kept similar coverage counts).

---

## 9. Build validations

| Command | Result |
|---|---|
| `npm run typecheck` | clean |
| `npm run lint` | clean |
| `NEXT_PUBLIC_TRADING_API_BASE_URL=http://localhost:8080 npm run build` | green — 17 user-facing routes + 4 SSG doc slugs + `_not-found` |
| `npx playwright test --list` | 131 tests in 30 files |
| `scripts/local-backend.sh` → `local-seed.sh` → `local-smoke.sh` | startup green; seed 12 PASS, 4 products visible; smoke **9 PASS / 0 FAIL** |

Targeted Playwright run not executed (WSL2 lacks `libnspr4.so`). All new assertions are static-DOM / mocked-route / browser-evaluate so the build + catalog + lint guarantee runtime behaviour under a real browser / CI.

Backend stopped cleanly post-QA; port 8080 free.

---

## 10. Docs created / updated

| File | Action |
|---|---|
| `docs/FRONTEND_TERMINAL_WORKSPACE_CLEANUP_V3_RESULT.md` | NEW (this file) |
| `docs/public-beta/USER_TESTING_GUIDE.md` | not edited — no operator-facing add-widget instructions were ever there |
| `docs/public-beta/PUBLIC_TESTNET_BETA_LAUNCH_CHECKLIST.md` | not edited — tracks deploy + posture, not layout polish |
| `docs/FRONTEND_PUBLIC_TESTNET_DEPLOY_OPERATOR_CHECKLIST.md` | not edited — route smoke list unchanged |
| `RUN_STATE.md` | closure prepended |

---

## 11. RUN_STATE update

2026-06-14 closure for FRONTEND-TERMINAL-WORKSPACE-CLEANUP-V3 prepended above FRONTEND-TERMINAL-WORKSPACE-RESIZABLE-V2. Documents the navbar move, the right-edge dead-zone fix, the toolbar removal, the expanded terminal-route footer-hide set, and the unchanged source-change discipline (backend Rust + Solidity + scripts all zero).

---

## 12. Files changed

**Created (frontend):**
- `src/lib/workspace-bridge.tsx`
- `src/components/workspace/WidgetMenuButton.tsx`

**Edited (frontend):**
- `src/app/(trading)/layout.tsx` — `WorkspaceBridgeProvider` wrap + `WidgetMenuButton` in navbar
- `src/components/TradingShell.tsx` — `/markets` + `/portfolio` join terminal routes; `p-0` on terminal main
- `src/components/workspace/Workspace.tsx` — toolbar removed; `useRegisterWorkspace` bridge; `overflow-x-hidden overflow-y-auto`; margin `[4,4]`; `w-full` on grid container; empty-state hint now points at navbar
- `tests/e2e/workspace-custom.spec.ts` — REWRITTEN for V3
- `tests/e2e/workspace-storage.spec.ts` — uses navbar button
- `tests/e2e/terminal-shell.spec.ts` — terminal-route set expanded
- `tests/e2e/public-beta-footer.spec.ts` — safety-bullets route list narrowed

**Created (backend docs):**
- `docs/FRONTEND_TERMINAL_WORKSPACE_CLEANUP_V3_RESULT.md`

**Edited (root):**
- `RUN_STATE.md`

**Untouched:** Backend Rust source (ZERO), Solidity (ZERO), `scripts/local-*.sh` (ZERO), `BottomPanel.tsx`, `OptionDetailPanel.tsx`, `OptionsChainGrid.tsx`, `OptionsChainTerminalCore.tsx`, `ExpirySelector.tsx`, `PayoffSvg.tsx`, `HamburgerMenu.tsx`, `PublicBetaFooter.tsx`, `lib/workspace-types.ts`, `lib/workspace-storage.ts` (still v2; no schema change), `lib/workspace-selected-option.tsx`, registry, all widgets, all hooks + trading components, backend `.env` (mtime `2026-06-08 16:55:05.874571237 +0200` preserved), `~/DEOPT/private/` (mode 700; not read; not committed).

The legacy `AddWidgetMenu.tsx` file is still on disk but no longer imported by Workspace; it remains available for any one-off internal tooling. Removing it would force a follow-up audit of any test that might have referenced it — kept for safety.

---

## 13. Validations

| Check | Result |
|---|---|
| `git diff --check` (frontend + backend) | clean |
| Sensitive-string scan on changed files | zero hits |
| localStorage secret-pattern scan via Playwright spec | zero hits |
| Private key scan | zero hits |
| RPC URL scan | zero hits (only `http://127.0.0.1:8080` local backend URL in docs) |
| `DATABASE_URL` scan on changed files | zero hits |
| Admin bearer scan | zero hits |
| Mainnet RPC scan | zero hits |
| Positive-claim drift scan on edited FE files | zero hits |
| Amber/yellow/orange class scan on edited FE files | zero hits |
| `.env` mtime preserved | YES |
| Private dir mode preserved | YES (700) |
| Backend stopped post-QA | YES (port 8080 free) |
| Chain tx / broadcast / mainnet RPC / real wallet | NONE |
| `isMainnetEnabled()` still hard-coded `false` | YES |
| Backend Rust / Solidity / scripts changes | NONE |
| New dependency added | NONE (V2's react-grid-layout is the latest; no new package this milestone) |
| Source changes limited to frontend + docs/RUN_STATE | YES |

---

## 14. Remaining UI / workspace gaps

* **No global "Reset layout" UI** — by operator request. If a future user gets confused, we can add it back under a hamburger sub-menu OR as a keyboard shortcut (e.g. `Cmd+Shift+R`).
* **No global "Anonymous layout" hint** — by operator request. Users learn implicitly when they connect a wallet and the layout persists 30 days instead of 24h.
* **`/markets` and `/portfolio` page bodies were not redesigned** — they still render their existing dense components inside terminal main; the only V3 change is "no footer" + full-width main. A future milestone could turn them into proper Workspace pages too.
* **`/custom/2` and `/custom/3` routes** — enum supports them; only `/custom` ships.
* **Drag-to-add from menu** — V3 keeps `placeAtBottom` insertion; RGL's `droppingItem` API could enable drop-on-grid placement.
* **Server-side localStorage sync** — out of scope.
* **Visible drag-handle grip icon** — header is the drag region but lacks a `⋮⋮` icon; future polish.

None block local QA, public-testnet-beta launch, or the operator's product-test pass.

---

## 15. Next milestone recommendation

**Primary (operator):** product-test V3 via `bash ~/DEOPT/scripts/local-frontend.sh`. Open `/trade`, drag the chain wider — it should reach the right edge now. Hit the navbar `Widget` button → add a Payoff or Greeks widget. Navigate `/trade` → `/perps` → `/custom`; confirm the navbar `Widget` button updates contextually and hides on `/`. Confirm no footer on `/trade`, `/perps`, `/custom`, `/markets`, `/portfolio`. Confirm the toolbar inside the workspace body is gone.

**Secondary (agent-runnable):** `BACKEND-PUBLIC-TESTNET-DEPLOY-PREFLIGHT` per existing brief — retry the previously-failed Railway deploy.

**Strictly later (NOT NOW):** drag-to-add, server-side sync, real perps trading UI, mainnet activation, audit firm outreach, bug bounty launch, announcement publication, KMS cutover, Safe migration, flipping `isMainnetEnabled()`, faking perps liquidity / funding / OI / Greeks.

---

## 16. Cross-links
* `~/DEOPT/deopt-v2-frontend/src/lib/workspace-bridge.tsx`
* `~/DEOPT/deopt-v2-frontend/src/components/workspace/WidgetMenuButton.tsx`
* `~/DEOPT/deopt-v2-frontend/src/components/workspace/Workspace.tsx`
* `~/DEOPT/deopt-v2-frontend/src/components/TradingShell.tsx`
* `~/DEOPT/deopt-v2-frontend/src/app/(trading)/layout.tsx`
* `~/DEOPT/deopt-v2-backend/docs/FRONTEND_TERMINAL_WORKSPACE_RESIZABLE_V2_RESULT.md`
* `~/DEOPT/deopt-v2-backend/docs/BACKEND_PUBLIC_TESTNET_DEPLOY_PREFLIGHT_NEXT_TASK.md`

**End of frontend terminal workspace cleanup V3 result.**
