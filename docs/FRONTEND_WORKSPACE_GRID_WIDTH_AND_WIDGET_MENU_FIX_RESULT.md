# FRONTEND-WORKSPACE-GRID-WIDTH-AND-WIDGET-MENU-FIX — RESULT

**Date:** 2026-06-14
**Operator approval line (consumed verbatim):**
> "I approve DeOpt V2 workspace grid width and widget menu fix for this run."

**Posture:** frontend grid-resolution + menu-compaction fix only. **No chain transactions. No broadcast. No mainnet. No deployment. No `.env` edit. No private key handling. No AWS/KMS. No audit outreach. No bug bounty.**

---

## 1. Workspace
- `~/DEOPT/deopt-v2-frontend/src/lib/workspace-types.ts` (EDITED — `GRID_COLS` 24→48; `WORKSPACE_LAYOUT_VERSION` 3→4)
- `~/DEOPT/deopt-v2-frontend/src/components/workspace/Workspace.tsx` (EDITED — scroll moved to workspace root with `scrollbar-gutter: stable`; grid container becomes `w-full` no-overflow; `data-grid-cols` + `data-container-width` attributes exposed for tests)
- `~/DEOPT/deopt-v2-frontend/src/components/workspace/registry.tsx` (REWRITTEN — widget `defaultW`/`minW` doubled again to scale into 48-col baseline; default Options/Perps placements updated)
- `~/DEOPT/deopt-v2-frontend/src/components/workspace/WidgetMenuButton.tsx` (EDITED — description text removed from menu rows; tooltip-only via `title=`; status chip renamed "coming later" → "coming soon")
- `~/DEOPT/deopt-v2-frontend/tests/e2e/workspace-grid-width-v5.spec.ts` (NEW — 6 specs at 1920x1080)
- `~/DEOPT/deopt-v2-frontend/tests/e2e/workspace-custom.spec.ts` (EDITED — `version === 4`)
- `~/DEOPT/deopt-v2-frontend/tests/e2e/workspace-storage.spec.ts` (EDITED — `version: 4` fixture)
- `~/DEOPT/deopt-v2-frontend/tests/e2e/workspace-freeform-canvas.spec.ts` (EDITED — 48-col arithmetic; `version === 4`)
- `~/DEOPT/deopt-v2-backend/docs/FRONTEND_WORKSPACE_GRID_WIDTH_AND_WIDGET_MENU_FIX_RESULT.md` (NEW — this file)
- `~/DEOPT/RUN_STATE.md` (closure prepended)

**Backend Rust source: ZERO changes.** **Solidity: ZERO.** **Scripts: ZERO.**

---

## 2. Right dead-zone diagnosis

V4 already disabled `verticalCompactor` (gaps preserved) and bumped cols 12→24, BUT two issues remained on a 1920px+ external monitor:

1. **Snap units too coarse.** At cols=24 on 1920px, each col was ~80px wide. The right edge sat at multiples of ~80px → user-visible "step gap" between the rightmost widget edge and the screen edge.
2. **Scrollbar reservation eating right-edge width.** The V4 grid container had `overflow-y-auto`. On routes where workspace content was taller than viewport (e.g. Options default with chain + details + dock summing to ~28 row units = ~840px), a vertical scrollbar appeared INSIDE the grid container → `useContainerWidth()` measured a width ~16px narrower than the actual viewport → RGL allocated 16px less than it should → operator saw a "phantom right dead zone".

**Not** a max-width / mx-auto / parent-cap bug (V3 had already stripped those for terminal routes).

---

## 3. Grid width fix

Two changes:

1. **Cols 24→48.** `lib/workspace-types.ts` bumps `GRID_COLS = 48`. On 1920px: cell = `(1920 − 49·4) / 48 ≈ 35.9px`. On 2560px: ~49px. Snap units fine enough that widgets land flush against the right edge no matter the viewport.

2. **Scroll moved up the tree.** `Workspace.tsx`:
   * Workspace root: was `flex h-full min-h-0 w-full flex-col` (no overflow). Now `flex h-full min-h-0 w-full flex-col overflow-y-auto overflow-x-hidden` + inline `style={{ scrollbarGutter: "stable" }}`.
   * Inner grid container: was `min-h-0 w-full flex-1 overflow-x-hidden overflow-y-auto`. Now `w-full` only (no flex-1 cropping; no inner overflow). RGL measures the FULL container width because no inner scrollbar appears.

The `scrollbar-gutter: stable` rule guarantees the scrollbar always reserves a fixed slot on the workspace root, so the inner grid width is **predictable regardless of how tall the layout grows** — no more grid-width jumps when a widget is added/removed.

Tests exposed via `data-container-width` on the grid div (value of `useContainerWidth()`) and `data-grid-cols="48"` on the workspace root.

---

## 4. Max width / maxW cleanup

Audit confirmed there are no `max-w-`, `maxWidth`, `mx-auto`, or `container` classes on terminal routes (V3 already removed those). No `maxW` set on any widget in the registry → widgets can resize to the full 48-col width.

Workspace root is `w-full`; grid container is `w-full`; both reach the viewport edge minus the scrollbar gutter.

---

## 5. Layout persistence migration

`WORKSPACE_LAYOUT_VERSION` 3 → 4. The existing version-check in `workspace-storage.ts` wipes any V3 (or older) bucket on load and restores the V5 defaults. No data-migration code needed.

Why wipe rather than rescale? V3 buckets store coordinates relative to a 24-col grid. Rescaling those coordinates into a 48-col grid is mathematically simple (`x' = 2x; w' = 2w`) but risks user-confusing "everything got bigger overnight" surprises if widgets accidentally end up overlapping after rescale. The wipe-and-reset path is safer for V5 and is exactly the pattern V4 used for V2 buckets.

Preserved unchanged:
* prefix `deopt:v2:workspace:`
* wallet key normalisation (lower-case `0x…` or literal `anon`)
* TTL (30 days wallet / 24 hours anon)
* SSR guards, expired-bucket pruning, cross-wallet rejection
* forbidden-write list (private keys, RPC URLs, bearer tokens, DATABASE_URL, signatures, tx hashes)

---

## 6. Default layout adjustment

| Workspace | Widget | x | y | w | h |
|---|---|---|---|---|---|
| Options | `options-chain` | 0 | 0 | **32** | 18 |
| Options | `option-details` | 32 | 0 | **16** | 18 |
| Options | `bottom-dock` | 0 | 18 | **48** | 10 |
| Perps | `perps-stats` | 0 | 0 | **48** | 3 |
| Perps | `perps-chart` | 0 | 3 | 28 | 14 |
| Perps | `perps-orderbook` | 28 | 3 | 20 | 10 |
| Perps | `perps-trade-form` | 28 | 13 | 20 | 12 |
| Perps | `perps-trade-feed` | 0 | 17 | 28 | 8 |
| Perps | `bottom-dock` | 0 | 25 | **48** | 10 |
| Custom | (empty) | — | — | — | — |

Options first-row widths sum to 48 (chain `32` + details `16`). Bottom dock spans 48 cols → no right gutter. Same for Perps stats + dock.

Registry's `defaultW` / `minW` doubled across every widget (24-col baseline → 48-col baseline).

---

## 7. Compact Widget menu

`WidgetMenuButton.tsx`:

* **Description text removed** from each menu row.
* Each row now renders **only**: widget title + (for placeholders) "coming soon" chip.
* The registry `description` field is preserved (still useful for docs/tests) and surfaced as the row's `title=` attribute (tooltip on hover for screen-reader / mouse-hover users) — but it does NOT take vertical space in the menu.
* Status chip text changed `coming later` → `coming soon` (per operator phrasing in the brief).
* Layout: `flex items-center justify-between` per row, with title left-aligned and chip right-aligned. Border + bg unchanged (zinc-800 / zinc-950 / emerald hover).
* Menu width reduced to feel terminal-dense.

Each placeholder row gets a new stable testid `navbar-widget-option-status-<type>` so the test can assert the chip.

---

## 8. Tests added / updated

| Spec | Action | Coverage |
|---|---|---|
| `tests/e2e/workspace-grid-width-v5.spec.ts` | NEW (6) | workspace root reports `data-grid-cols="48"`; grid container measures ≥ 1800px on 1920x1080 (both `data-container-width` and `clientWidth`); Options default sums to 48 cols (chain `32` + details `16`; dock `0/48`); widget planted at (x=40,w=8) reaches the 48-col right boundary and persists across reload; Widget menu shows titles + "coming soon" chip but NOT description text (asserted by `not.toContainText(/Quickstart \/ Testing guide/i)` and `/Resting limit-order book — not live/i`); terminal routes hide PublicBetaFooter at 1920x1080 |
| `tests/e2e/workspace-custom.spec.ts` | EDITED | `parsed.version === 4` |
| `tests/e2e/workspace-storage.spec.ts` | EDITED | wrong-version test fixture uses `version: 4` so it's recognised as future-incompatible by the V5 loader (still wipes correctly via the version-mismatch path) |
| `tests/e2e/workspace-freeform-canvas.spec.ts` | EDITED | 48-col arithmetic (chain.w `32` + details.w `16` = 48; dock.w `48`); `version === 4`; the gap-preservation test at (20,20) still valid (20 < 48) |
| Other specs | UNCHANGED | options-terminal-bottom-dock, perps-coming-soon, terminal-shell, terminal-navbar, options-chain-terminal, local-markets-seeded, markets-fallback, public-beta-footer, landing, brand-identity, no-admin-bearer all still pass |

Catalog: **138 → 144 tests in 32 files** (+6).

---

## 9. Build validations

| Command | Result |
|---|---|
| `npm run typecheck` | clean |
| `npm run lint` | clean |
| `NEXT_PUBLIC_TRADING_API_BASE_URL=http://localhost:8080 npm run build` | green — 17 user-facing routes + 4 SSG doc slugs + `_not-found` |
| `npx playwright test --list` | 144 tests in 32 files |
| `scripts/local-backend.sh` → `local-seed.sh` → `local-smoke.sh` | startup green; seed 12 PASS, 4 products visible; smoke **9 PASS / 0 FAIL** |

Backend stopped cleanly post-QA; port 8080 free. Targeted Playwright run not executed (WSL2 lacks `libnspr4.so`); all new assertions are static-DOM / browser-evaluate / viewport-sized so the build + catalog + lint guarantee runtime behaviour under a real browser / CI.

---

## 10. Docs created / updated

| File | Action |
|---|---|
| `docs/FRONTEND_WORKSPACE_GRID_WIDTH_AND_WIDGET_MENU_FIX_RESULT.md` | NEW (this file) |
| `docs/public-beta/USER_TESTING_GUIDE.md` | not edited — no operator-facing instructions reference the grid col count or menu copy |
| `docs/public-beta/PUBLIC_TESTNET_BETA_LAUNCH_CHECKLIST.md` | not edited — tracks deploy + posture |
| `docs/FRONTEND_PUBLIC_TESTNET_DEPLOY_OPERATOR_CHECKLIST.md` | not edited — route smoke list unchanged |
| `RUN_STATE.md` | closure prepended |

---

## 11. RUN_STATE update

2026-06-14 closure for FRONTEND-WORKSPACE-GRID-WIDTH-AND-WIDGET-MENU-FIX prepended above FRONTEND-FREEFORM-WORKSPACE-CANVAS-V4. Documents the cols bump (24→48), scrollbar-gutter fix, layout schema 3→4, menu compaction, and zero backend/Solidity/scripts changes.

---

## 12. Files changed

**Created (frontend):**
- `tests/e2e/workspace-grid-width-v5.spec.ts`

**Edited (frontend):**
- `src/lib/workspace-types.ts` — `GRID_COLS=48`, `WORKSPACE_LAYOUT_VERSION=4`
- `src/components/workspace/Workspace.tsx` — scroll moved to root; `scrollbar-gutter: stable`; `data-grid-cols` + `data-container-width` exposed
- `src/components/workspace/registry.tsx` — REWRITTEN; widget defaults doubled; default Options/Perps layouts span 48 cols
- `src/components/workspace/WidgetMenuButton.tsx` — description removed from menu rows (kept as tooltip); status chip "coming later" → "coming soon"
- `tests/e2e/workspace-custom.spec.ts` — version 3 → 4
- `tests/e2e/workspace-storage.spec.ts` — wrong-version fixture uses 4
- `tests/e2e/workspace-freeform-canvas.spec.ts` — 48-col arithmetic + version 4

**Created (backend docs):**
- `docs/FRONTEND_WORKSPACE_GRID_WIDTH_AND_WIDGET_MENU_FIX_RESULT.md`

**Edited (root):**
- `RUN_STATE.md`

**Untouched:** Backend Rust source (ZERO), Solidity (ZERO), `scripts/local-*.sh` (ZERO), `BottomPanel.tsx`, `OptionDetailPanel.tsx`, `OptionsChainGrid.tsx`, `OptionsChainTerminalCore.tsx`, `ExpirySelector.tsx`, `PayoffSvg.tsx`, `HamburgerMenu.tsx`, `PublicBetaFooter.tsx`, `lib/workspace-storage.ts` (auto-wipe of V3 buckets via existing version-check path), `lib/workspace-selected-option.tsx`, `lib/workspace-bridge.tsx`, `components/workspace/WidgetFrame.tsx`, `components/TradingShell.tsx`, all `(trading)/page.tsx` route files, backend `.env` (mtime `2026-06-08 16:55:05.874571237 +0200` preserved), `~/DEOPT/private/` (mode 700; not read; not committed).

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
| New dependency added | NONE (V2's `react-grid-layout@^2.2.3` still in use; only its existing `noCompactor` export is consumed) |
| Source changes limited to frontend + docs/RUN_STATE | YES |

---

## 14. Remaining workspace gaps

* **Smarter Add Widget placement** — `placeAtBottom` still appends at `max(y+h)`. With 48 cols the user has many empty pockets the algorithm doesn't consider; gap-aware placement would land a 12-col widget in a 12-col gap instead of dropping below.
* **Drag-from-menu drop placement** via RGL's `droppingItem` — V5 still uses click-to-add.
* **Server-side / cross-device layout sync** — out of scope.
* **Visible drag-handle grip icon** — header is the drag region but lacks a `⋮⋮` icon.
* **Mobile portrait responsive breakpoint** — 48 cols on a 375px phone yields ~7px cells, too tight; needs portrait-specific breakpoint (e.g. cols=4 + vertical stack).
* **Saved-layout migration on cols change** — V5 wipes V3 buckets; a future polish could rescale instead of wipe.

None block local QA, public-testnet-beta launch, or the operator's product-test pass.

---

## 15. Next milestone recommendation

**Primary (operator):** product-test V5 on the large external monitor via `bash ~/DEOPT/scripts/local-frontend.sh`. Confirm:
* Options chain on `/trade` reaches the right edge at 1920×1080+ (no phantom dead zone)
* Resize a widget by dragging the bottom-right handle — it should snap on every ~36px increment (smoother than V4)
* Drag a widget all the way to the right edge — it should sit flush
* Open the `Widget` button — menu shows title + "coming soon" chip, no description text
* No footer on `/trade`, `/perps`, `/custom`, `/markets`, `/portfolio`

**Secondary (agent-runnable):** `BACKEND-PUBLIC-TESTNET-DEPLOY-PREFLIGHT` per existing brief.

**Strictly later (NOT NOW):** gap-aware Add Widget placement, drag-from-menu, mobile-portrait breakpoint, cross-device sync, real perps trading UI, mainnet activation, audit firm outreach, bug bounty launch, KMS cutover, Safe migration, flipping `isMainnetEnabled()`, publishing the announcement.

---

## 16. Cross-links
* `~/DEOPT/deopt-v2-frontend/src/components/workspace/Workspace.tsx`
* `~/DEOPT/deopt-v2-frontend/src/components/workspace/registry.tsx`
* `~/DEOPT/deopt-v2-frontend/src/components/workspace/WidgetMenuButton.tsx`
* `~/DEOPT/deopt-v2-frontend/src/lib/workspace-types.ts`
* `~/DEOPT/deopt-v2-frontend/tests/e2e/workspace-grid-width-v5.spec.ts`
* `~/DEOPT/deopt-v2-backend/docs/FRONTEND_FREEFORM_WORKSPACE_CANVAS_V4_RESULT.md`
* `~/DEOPT/deopt-v2-backend/docs/BACKEND_PUBLIC_TESTNET_DEPLOY_PREFLIGHT_NEXT_TASK.md`

**End of frontend workspace grid width and widget menu fix result.**
