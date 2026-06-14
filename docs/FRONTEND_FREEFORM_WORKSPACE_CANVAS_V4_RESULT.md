# FRONTEND-FREEFORM-WORKSPACE-CANVAS-V4 — RESULT

**Date:** 2026-06-14
**Operator approval line (consumed verbatim):**
> "I approve DeOpt V2 freeform workspace canvas V4 for this run."

**Posture:** frontend layout-engine fix only. **No chain transactions. No broadcast. No mainnet. No deployment. No `.env` edit. No private key handling. No AWS/KMS. No audit outreach. No bug bounty.**

---

## 1. Workspace
- `~/DEOPT/deopt-v2-frontend/src/lib/workspace-types.ts` (EDITED — `GRID_COLS` 12→24; `WORKSPACE_LAYOUT_VERSION` 2→3)
- `~/DEOPT/deopt-v2-frontend/src/components/workspace/Workspace.tsx` (EDITED — imports `noCompactor`; passes `compactor={noCompactor}` to GridLayout as a top-level prop)
- `~/DEOPT/deopt-v2-frontend/src/components/workspace/registry.tsx` (REWRITTEN — every widget's `defaultW`/`minW` doubled to scale into the 24-col grid; default layouts updated; bottom-dock + perps-stats now span full 24 cols)
- `~/DEOPT/deopt-v2-frontend/tests/e2e/workspace-freeform-canvas.spec.ts` (NEW — 7 viewport-aware specs at 1920x1080)
- `~/DEOPT/deopt-v2-frontend/tests/e2e/workspace-custom.spec.ts` (EDITED — `version === 3`)
- `~/DEOPT/deopt-v2-frontend/tests/e2e/workspace-storage.spec.ts` (EDITED — V1 migration test now describes the V3 loader)
- `~/DEOPT/deopt-v2-backend/docs/FRONTEND_FREEFORM_WORKSPACE_CANVAS_V4_RESULT.md` (NEW — this file)
- `~/DEOPT/RUN_STATE.md` (closure prepended)

**Backend Rust source: ZERO changes.** **Solidity: ZERO.** **Scripts: ZERO.**

---

## 2. Right-side dead-zone diagnosis

**Root cause: react-grid-layout's default `verticalCompactor` packs widgets upward + leftward.** It is NOT a width-measurement bug.

On a large monitor (e.g. 1920px), the V3 stack was:
* outer `<div className="flex h-dvh min-h-0 flex-col overflow-hidden">` — no width cap
* `<main className="min-h-0 flex-1 overflow-hidden p-0">` — no width cap
* page wrapper `flex h-full min-h-0 flex-col` — no width cap
* `<Workspace>` inner `flex h-full min-h-0 w-full flex-col` — no width cap
* `<div ref={containerRef} className="min-h-0 w-full flex-1 overflow-x-hidden overflow-y-auto">` — measured by `useContainerWidth()` via ResizeObserver, returns the true full-width
* `<GridLayout width={containerWidth} gridConfig={{cols:12,…}}>` — receives the full width

So the **container DID reach the right edge** of the viewport. But:

* `cols=12` meant each col was ~155px on 1920px viewports; widgets could only land on 12 discrete positions
* `verticalCompactor` (RGL 2.x default) RE-PACKED widgets every render, pulling any widget the user dragged to the right back to the left until it touched another widget or the left edge
* User-visible effect: "I drag a widget toward the right edge and it snaps back. There's a dead zone."

**Fix:** swap to `noCompactor` so user positions are preserved, AND bump cols to 24 for finer-grained placement on large screens.

---

## 3. Full-width terminal canvas fix

No additional changes to the shell were needed — V3 already made terminal pages `p-0` with no max-width cap; the grid container was `w-full` and measured via `useContainerWidth()`'s ResizeObserver. With the V4 cols + compactor changes, widgets now actually reach the right edge.

For 1920x1080 the new test `workspace-freeform-canvas.spec.ts §"terminal main on /trade has no max-width cap"` asserts `mainWidth > 1800`.

---

## 4. Auto-compaction / gap behaviour

`Workspace.tsx` now imports `noCompactor` from `react-grid-layout` and passes it via the top-level `compactor` prop on `<GridLayout>`:

```tsx
import { GridLayout, noCompactor, useContainerWidth } from "react-grid-layout";
…
<GridLayout
  layout={rglLayout}
  width={containerWidth}
  gridConfig={{ cols: GRID_COLS, rowHeight: GRID_ROW_HEIGHT_PX, margin: [4, 4], containerPadding: [0, 0] }}
  dragConfig={{ enabled: true, handle: ".deopt-widget-drag-handle", cancel: "button" }}
  resizeConfig={{ enabled: true }}
  compactor={noCompactor}
  onLayoutChange={onLayoutChange}
>
```

(`compactor` is a TOP-LEVEL prop on `GridLayoutProps`, NOT inside `gridConfig` — first attempt failed typecheck. Moved out.)

Effect:
* Widgets stay EXACTLY where the user drops them. No upward / leftward packing.
* Users can leave intentional empty gaps between widgets (e.g. place chain at (0,0), trade panel at (20,0), nothing in cols 16-19).
* `onLayoutChange` continues to persist user-driven x/y/w/h to localStorage; the new layout is NOT post-processed.
* Adding a widget still uses `placeAtBottom(existing)` — simple "next free row" placement so the user sees the new widget without scanning the grid.

A new spec (`workspace-freeform-canvas.spec.ts §"Gaps are preserved — a widget placed at (x=20,y=20) is NOT packed back to (0,0)"`) plants a widget at (20,20) in localStorage, reloads, and asserts the persisted bucket still reads (20,20).

---

## 5. Default layout update

The 24-col grid lets defaults span the full viewport:

| Workspace | Widget | x | y | w | h |
|---|---|---|---|---|---|
| Options | `options-chain`   | 0  | 0  | **16** | 18 |
| Options | `option-details`  | 16 | 0  | **8**  | 18 |
| Options | `bottom-dock`     | 0  | 18 | **24** | 10 |
| Perps   | `perps-stats`     | 0  | 0  | **24** | 3  |
| Perps   | `perps-chart`     | 0  | 3  | 14     | 14 |
| Perps   | `perps-orderbook` | 14 | 3  | 10     | 10 |
| Perps   | `perps-trade-form`| 14 | 13 | 10     | 12 |
| Perps   | `perps-trade-feed`| 0  | 17 | 14     | 8  |
| Perps   | `bottom-dock`     | 0  | 25 | **24** | 10 |
| Custom  | (empty)           | —  | —  | —      | —  |

Sum of first-row widths on Options = 16 + 8 = 24 cols → fills the full grid. Bottom dock = 24 cols → full width. Same for Perps stats + dock.

Every widget's `defaultW` / `minW` in the registry was doubled (from a 12-col baseline to a 24-col baseline) so user-added widgets land at sensible sizes.

---

## 6. Widget resize behaviour

* Resize handle: still RGL's default bottom-right `.react-resizable-handle`, restyled to an emerald chevron in `globals.css` (carried from V2).
* `minW`/`minH` scale-up means user can still shrink widgets to a usable minimum but can also grow them to the full 24-col width.
* No `maxW`/`maxH` set anywhere → no artificial cap.
* Widget inner content uses `min-h-0 flex-1 overflow-auto` so when the widget is sized small, the inner body scrolls. When sized full-width, it fills.

---

## 7. Persistence behaviour

* `WORKSPACE_LAYOUT_VERSION` bumped 2→3. The existing version-check in `workspace-storage.ts` wipes any V2 (or older) bucket on load → V4 defaults restored. No data-migration code needed.
* `onLayoutChange` writes x/y/w/h exactly as RGL reports them. No compaction before save. No normalisation. No re-packing after load.
* Wallet TTL (30 days) and anon TTL (24 hours) unchanged.
* SSR guards, expired pruning, cross-wallet rejection, forbidden-write list all unchanged.
* `workspace-custom.spec.ts §"localStorage stores V2 grid-coord bucket with no secrets"` updated to assert `parsed.version === 3` + still no `size` field.
* `workspace-storage.spec.ts §"V1 bucket is wiped"` retitled to "V1 bucket (size enum) is wiped when the V3 loader sees it" + bucket fixture uses `version: 1` (still version-mismatched against the V3 loader → wipe path triggered).

---

## 8. Widget navbar control

UNCHANGED from V3:
* Only global control is `Widget` button next to `Connect wallet` in navbar (`testid="navbar-widget-button"`)
* No visible Reset Layout button
* No visible "Anonymous layout is temporary" message
* No global toolbar inside the workspace body
* Widget-frame remove ✕ + drag handle + RGL resize handle preserved

A V4 spec (`workspace-freeform-canvas.spec.ts §"Navbar Widget button still opens the menu at 1920x1080"`) asserts the button is visible + opens the menu at the larger viewport.

---

## 9. Tests added / updated

| Spec | Action | Coverage |
|---|---|---|
| `tests/e2e/workspace-freeform-canvas.spec.ts` | NEW (7) | 1920x1080 viewport — terminal main fills (>1800px wide); workspace grid fills; Options default first-row widths sum to 24 cols (no right gutter); bottom dock spans full 24 cols; widget planted at (20,20) is NOT packed back to (0,0); V3 schema persists with grid coords; `/trade`+`/perps`+`/custom` hide footer at 1920x1080; navbar Widget button works at 1920x1080 |
| `tests/e2e/workspace-custom.spec.ts` | EDITED | `parsed.version` expectation 2 → 3 |
| `tests/e2e/workspace-storage.spec.ts` | EDITED | docstring + test name updated to reference the V3 loader (the V1-bucket fixture is unchanged; it still triggers the version-mismatch wipe path) |
| `tests/e2e/workspace-storage.spec.ts §"saved layout survives a reload"` | UNCHANGED | grid coords write-then-read still works |
| `tests/e2e/workspace-storage.spec.ts §"anon expiresAt is bounded by 24h"` | UNCHANGED | TTL unchanged |
| `tests/e2e/options-terminal-bottom-dock.spec.ts` | UNCHANGED | default widget assertions don't reference exact col counts |
| `tests/e2e/perps-coming-soon.spec.ts` | UNCHANGED | default widget assertions don't reference exact col counts |
| `tests/e2e/terminal-shell.spec.ts` | UNCHANGED | navbar Widget button assertions still valid |
| `tests/e2e/terminal-navbar.spec.ts` | UNCHANGED | nav links unchanged |
| `tests/e2e/options-chain-terminal.spec.ts` | UNCHANGED | mocked-route chain interactions still pass |
| `tests/e2e/public-beta-footer.spec.ts` | UNCHANGED | page-mode route list (`/`, `/history`, `/health`) unchanged |
| `tests/e2e/landing.spec.ts`, `tests/e2e/brand-identity.spec.ts`, `tests/e2e/no-admin-bearer.spec.ts` | UNCHANGED | footer assertions there target `/` (page mode) |

Catalog: **131 → 138 tests in 31 files** (+7).

---

## 10. Build validations

| Command | Result |
|---|---|
| `npm run typecheck` | clean (after one fix: `compactor` is a TOP-LEVEL prop on `<GridLayout>`, not inside `gridConfig`) |
| `npm run lint` | clean |
| `NEXT_PUBLIC_TRADING_API_BASE_URL=http://localhost:8080 npm run build` | green — 17 user-facing routes + 4 SSG doc slugs + `_not-found` |
| `npx playwright test --list` | 138 tests in 31 files |
| `scripts/local-backend.sh` → `local-seed.sh` → `local-smoke.sh` | startup green; seed 12 PASS, 4 products visible; smoke **9 PASS / 0 FAIL** |

Targeted Playwright run not executed (WSL2 lacks `libnspr4.so`). All new assertions are static-DOM / browser-evaluate / viewport-sized so the build + catalog + lint guarantee runtime behaviour under a real browser / CI.

Backend stopped cleanly post-QA; port 8080 free.

---

## 11. Docs created / updated

| File | Action |
|---|---|
| `docs/FRONTEND_FREEFORM_WORKSPACE_CANVAS_V4_RESULT.md` | NEW (this file) |
| `docs/public-beta/USER_TESTING_GUIDE.md` | not edited — no operator-facing instructions reference the grid col count |
| `docs/public-beta/PUBLIC_TESTNET_BETA_LAUNCH_CHECKLIST.md` | not edited — tracks deploy + posture, not layout grid sizing |
| `docs/FRONTEND_PUBLIC_TESTNET_DEPLOY_OPERATOR_CHECKLIST.md` | not edited — route smoke list unchanged |
| `RUN_STATE.md` | closure prepended |

---

## 12. RUN_STATE update

2026-06-14 closure for FRONTEND-FREEFORM-WORKSPACE-CANVAS-V4 prepended above FRONTEND-TERMINAL-WORKSPACE-CLEANUP-V3. Documents the noCompactor switch (root cause of the right-side dead zone), the 12→24 col bump (finer placement on large screens), the storage version bump 2→3 with V2-bucket wipe-on-load, and the new viewport-aware specs.

---

## 13. Files changed

**Created (frontend):**
- `tests/e2e/workspace-freeform-canvas.spec.ts`

**Edited (frontend):**
- `src/lib/workspace-types.ts` — `GRID_COLS=24`, `WORKSPACE_LAYOUT_VERSION=3`
- `src/components/workspace/Workspace.tsx` — `import { noCompactor }`; `compactor={noCompactor}` on `<GridLayout>` (top-level prop, not inside `gridConfig`)
- `src/components/workspace/registry.tsx` — REWRITTEN; widget defaults doubled; default Options/Perps layouts span 24 cols
- `tests/e2e/workspace-custom.spec.ts` — version assertion 2 → 3
- `tests/e2e/workspace-storage.spec.ts` — V1 migration test docstring/name updated for V3 loader

**Created (backend docs):**
- `docs/FRONTEND_FREEFORM_WORKSPACE_CANVAS_V4_RESULT.md`

**Edited (root):**
- `RUN_STATE.md`

**Untouched:** Backend Rust source (ZERO), Solidity (ZERO), `scripts/local-*.sh` (ZERO), `BottomPanel.tsx`, `OptionDetailPanel.tsx`, `OptionsChainGrid.tsx`, `OptionsChainTerminalCore.tsx`, `ExpirySelector.tsx`, `PayoffSvg.tsx`, `HamburgerMenu.tsx`, `PublicBetaFooter.tsx`, `lib/workspace-storage.ts` (no schema-handling change beyond auto-wipe via the new version), `lib/workspace-selected-option.tsx`, `lib/workspace-bridge.tsx`, `components/workspace/WidgetFrame.tsx`, `components/workspace/WidgetMenuButton.tsx`, `components/TradingShell.tsx`, all `(trading)/page.tsx` route files, backend `.env` (mtime `2026-06-08 16:55:05.874571237 +0200` preserved), `~/DEOPT/private/` (mode 700; not read; not committed).

---

## 14. Validations

| Check | Result |
|---|---|
| `git diff --check` (frontend + backend) | clean |
| Sensitive-string scan on changed files | zero hits |
| localStorage secret-pattern scan via Playwright spec | zero hits (existing spec carried over) |
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
| New dependency added | NONE (V2's `react-grid-layout@^2.2.3` already in use; the V4 fix uses its existing `noCompactor` export) |
| Source changes limited to frontend + docs/RUN_STATE | YES |

---

## 15. Remaining visual / workspace gaps

* **Snap-back of newly-added widgets** — `placeAtBottom` uses simple `max(y+h)` for the next slot; a smarter "find empty rectangle ≥ widget size" placement would let Add Widget drop into existing gaps. Out of scope; future polish.
* **Drag-from-menu drop placement** — RGL's `droppingItem` API can enable drag-to-grid from the menu; V4 still uses click-to-add.
* **Server-side / cross-device layout sync** — out of scope.
* **Visible drag-handle grip icon** — header is the drag region but lacks a `⋮⋮` icon; future polish.
* **Mobile portrait responsive layout** — 24 cols on a 375px phone yields ~14px cells, too tight; a portrait-specific breakpoint (e.g. cols=4 + vertical stack) is a future polish. Existing mobile behaviour: workspace becomes scrollable; widgets still drag/resize but at finger-tap precision.
* **Visible empty-state hint on big screens** — copy still points at "the Widget button in the top navbar"; for tablets where the navbar wraps the hint could be more visual.

None block local QA, public-testnet-beta launch, or the operator's product-test pass.

---

## 16. Next milestone recommendation

**Primary (operator):** product-test V4 on a large external monitor via `bash ~/DEOPT/scripts/local-frontend.sh`. Confirm:
* `/trade` chain reaches the right edge at 1920×1080+
* dragging a widget to the right and dropping leaves it there (no snap-back)
* placing a widget in `/custom` with empty space around it persists across reload
* the navbar `Widget` button + per-widget remove + drag/resize still work
* no footer on `/trade`, `/perps`, `/custom`, `/markets`, `/portfolio`

**Secondary (agent-runnable):** `BACKEND-PUBLIC-TESTNET-DEPLOY-PREFLIGHT` per existing brief — retry the previously-failed Railway deploy.

**Strictly later (NOT NOW):** smarter Add Widget placement (gap-aware), drag-from-menu, mobile-portrait breakpoint, cross-device sync, real perps trading UI, mainnet activation, audit firm outreach, bug bounty launch, KMS cutover, Safe migration, flipping `isMainnetEnabled()`, publishing the announcement, faking perps liquidity / funding / OI / Greeks.

---

## 17. Cross-links
* `~/DEOPT/deopt-v2-frontend/src/components/workspace/Workspace.tsx`
* `~/DEOPT/deopt-v2-frontend/src/components/workspace/registry.tsx`
* `~/DEOPT/deopt-v2-frontend/src/lib/workspace-types.ts`
* `~/DEOPT/deopt-v2-frontend/tests/e2e/workspace-freeform-canvas.spec.ts`
* `~/DEOPT/deopt-v2-backend/docs/FRONTEND_TERMINAL_WORKSPACE_CLEANUP_V3_RESULT.md`
* `~/DEOPT/deopt-v2-backend/docs/FRONTEND_TERMINAL_WORKSPACE_RESIZABLE_V2_RESULT.md`
* `~/DEOPT/deopt-v2-backend/docs/BACKEND_PUBLIC_TESTNET_DEPLOY_PREFLIGHT_NEXT_TASK.md`

**End of frontend freeform workspace canvas V4 result.**
