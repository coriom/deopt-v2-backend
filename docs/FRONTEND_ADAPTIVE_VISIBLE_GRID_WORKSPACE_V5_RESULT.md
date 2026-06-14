# FRONTEND-ADAPTIVE-VISIBLE-GRID-WORKSPACE-V5 — RESULT

**Date:** 2026-06-14
**Operator approval line (consumed verbatim):**
> "I approve DeOpt V2 adaptive visible grid workspace V5 for this run."

**Posture:** frontend layout-engine refinement only. **No chain transactions. No broadcast. No mainnet. No deployment. No `.env` edit. No private key handling. No AWS/KMS. No audit outreach. No bug bounty.**

---

## 1. Workspace
- `~/DEOPT/deopt-v2-frontend/src/lib/workspace-types.ts` (REWRITTEN — adaptive `computeCols` + `computeCellWidth`; layout version 4→5; `WorkspaceLayout` gains `cols` field)
- `~/DEOPT/deopt-v2-frontend/src/lib/workspace-storage.ts` (EDITED — `saveWorkspaceLayout` now persists `cols`)
- `~/DEOPT/deopt-v2-frontend/src/components/workspace/registry.tsx` (EDITED — `defaultWidgetsFor(workspaceId, cols)` derives positions from a fractional ratio so defaults always fill the canvas at any cols)
- `~/DEOPT/deopt-v2-frontend/src/components/workspace/Workspace.tsx` (REWRITTEN — adaptive cols via `useContainerWidth` + `computeCols`; CSS variables drive the visible dotted backdrop; rescale on cols change; saved-cols persisted; new data attrs)
- `~/DEOPT/deopt-v2-frontend/tests/e2e/workspace-grid-width-v5.spec.ts` (REWRITTEN — 9 viewport-aware specs)
- `~/DEOPT/deopt-v2-frontend/tests/e2e/workspace-custom.spec.ts` (EDITED — version 5)
- `~/DEOPT/deopt-v2-frontend/tests/e2e/workspace-storage.spec.ts` (EDITED — fixture version 5)
- `~/DEOPT/deopt-v2-frontend/tests/e2e/workspace-freeform-canvas.spec.ts` (EDITED — Options-defaults arithmetic now cols-aware; version 5)
- `~/DEOPT/deopt-v2-backend/docs/FRONTEND_ADAPTIVE_VISIBLE_GRID_WORKSPACE_V5_RESULT.md` (NEW — this file)
- `~/DEOPT/RUN_STATE.md` (closure prepended)

**Backend Rust source: ZERO changes.** **Solidity: ZERO.** **Scripts: ZERO.**

---

## 2. Static-grid failure diagnosis

V4 fixed the snap units (24→48 cols) and the scrollbar reservation (`scrollbar-gutter: stable`), but still hard-coded `cols = 48` regardless of viewport. On a wide external monitor (e.g. 2560×1440 in Brave), `useContainerWidth()` returned the full container width, RGL computed cell width = `(2560 - 49·4)/48 ≈ 50.1px`, and widgets reached the right edge cleanly in absolute pixels — **but** the operator perceived a "dead zone" because each col was ~50px wide. With 48 cols, the smallest user-controllable step on a 2560px screen was 50px → the rightmost widget's right edge looked like it sat 30–50px short of the viewport (which it did, by exactly one snap step).

Root cause was therefore: **fixed col count means snap granularity scales with screen width, not the other way around**. Larger screen → coarser snap → larger visible "step gap" to the right edge.

Reproduced on 1440 / 1920 / 2560:
- 1440: 48 cols → cell ≈ 26px. Step gap acceptable.
- 1920: 48 cols → cell ≈ 36px. Step gap noticeable.
- 2560: 48 cols → cell ≈ 50px. Step gap obvious.

---

## 3. Adaptive grid implementation

`lib/workspace-types.ts`:

```ts
export const GRID_MIN_COLS = 48;
export const GRID_MAX_COLS = 120;
export const GRID_TARGET_CELL_PX = 36;

export function computeCols(containerWidth: number): number {
  if (!Number.isFinite(containerWidth) || containerWidth <= 0) return GRID_MIN_COLS;
  const raw = Math.round(containerWidth / GRID_TARGET_CELL_PX);
  return Math.max(GRID_MIN_COLS, Math.min(GRID_MAX_COLS, raw));
}

export function computeCellWidth(containerWidth: number, cols: number): number {
  if (cols <= 0) return 0;
  const usable = containerWidth - GRID_ITEM_MARGIN_PX * (cols + 1);
  return Math.max(0, usable / cols);
}
```

Yields:
- 1440 → 48 cols (clamped to MIN)
- 1920 → ~53 cols
- 2560 → ~71 cols
- 3840 → 107 cols (under the 120 ceiling)

`Workspace.tsx` calls `computeCols(useContainerWidth().width)` → recomputes on every container width change (ResizeObserver-driven). Cols is passed straight to RGL's `gridConfig.cols`. Widgets reach the right edge because the snap granularity scales **inversely** with screen size.

---

## 4. Visible grid implementation

The workspace canvas now carries inline CSS variables that drive a radial-gradient backdrop:

```ts
const cellStyle = {
  "--workspace-cell-width": `${cellWidth + GRID_ITEM_MARGIN_PX}px`,
  "--workspace-row-height": `${GRID_ROW_HEIGHT_PX + GRID_ITEM_MARGIN_PX}px`,
  backgroundImage:
    "radial-gradient(circle, rgba(110, 231, 183, 0.10) 1px, transparent 1px)",
  backgroundSize:
    "var(--workspace-cell-width) var(--workspace-row-height)",
  backgroundPosition: `${GRID_ITEM_MARGIN_PX}px ${GRID_ITEM_MARGIN_PX}px`,
};
```

* `radial-gradient(circle, rgba(110, 231, 183, 0.10) 1px, transparent 1px)` — subtle 1px emerald dot.
* `background-size` = (cellWidth + margin) × (rowHeight + margin) so one dot lands per RGL cell.
* `background-position` offsets by the margin so the first dot aligns with the first cell's top-left corner.
* `cellStyle` is applied to BOTH the empty-state card (so users see the canvas IS modular even when no widgets exist) AND the grid container (so dots are visible behind / around widgets).
* Backdrop is hidden until `containerWidth > 0 && cellWidth > 0` to avoid mis-spaced first paint.

No yellow / orange / amber. Emerald at 10% opacity — subtle, not distracting.

---

## 5. Layout scaling behaviour

`WorkspaceLayout` now carries a `cols` field — the col count at the time the layout was saved.

```ts
function rescaleLayout(widgets, oldCols, newCols) {
  if (oldCols === newCols) return widgets;
  const k = newCols / oldCols;
  return widgets.map((w) => {
    const scaledX = round(w.x * k);
    const scaledW = max(minW, round(w.w * k));
    const clampedW = min(scaledW, newCols);
    const clampedX = min(scaledX, max(0, newCols - clampedW));
    return { ...w, x: clampedX, w: clampedW };
  });
}
```

On viewport change → `Workspace` recomputes `cols` → rescale effect fires → user proportions preserved (a widget that was 50% of cols stays 50% of cols at the new viewport). No compaction. No gaps removed. Heights unchanged (rowHeight is fixed at 30px).

`onLayoutChange` writes the new `cols` along with widgets so the bucket always reflects the current viewport's snap count. Reload at the same viewport → no rescale needed.

`WORKSPACE_LAYOUT_VERSION` bumped 4→5 to wipe any V4 buckets (those lack the `cols` field). Existing version-check in `workspace-storage.ts` handles the wipe.

---

## 6. Right-edge placement fix

Combined effect of adaptive cols + visible backdrop + rescale:

- Snap step ≈ 36px on every viewport → no perceived "step gap" to the right edge.
- A widget resized to its right edge always lands flush with the canvas right edge: `x + w = cols`.
- A widget placed at `(x = cols - 12, w = 12)` reaches `x + w = cols` and stays put across reload (test asserted).
- No `maxW` set on any widget → maximum width is implicitly `cols`.

---

## 7. Default layouts

`defaultWidgetsFor(workspaceId, cols)` now takes the live cols:

| Workspace | Widget | x | y | w | h |
|---|---|---|---|---|---|
| Options | `options-chain` | 0 | 0 | `round(cols·0.667)` | 18 |
| Options | `option-details` | `chain.w` | 0 | `cols − chain.w` | 18 |
| Options | `bottom-dock` | 0 | 18 | `cols` | 10 |
| Perps | `perps-stats` | 0 | 0 | `cols` | 3 |
| Perps | `perps-chart` | 0 | 3 | `round(cols·0.583)` | 14 |
| Perps | `perps-orderbook` | `chart.w` | 3 | `cols − chart.w` | 10 |
| Perps | `perps-trade-form` | `chart.w` | 13 | `cols − chart.w` | 12 |
| Perps | `perps-trade-feed` | 0 | 17 | `chart.w` | 8 |
| Perps | `bottom-dock` | 0 | 25 | `cols` | 10 |
| Custom | (empty) | — | — | — | — |

At every cols value, Options first-row widths sum to `cols` exactly (no right gutter). Same for Perps stats + dock + chart/orderbook columns.

---

## 8. Widget menu preservation

`WidgetMenuButton.tsx` unchanged from V5/cleanup-V3 milestone: title + (placeholder) "coming soon" chip only; description preserved as the `title=` tooltip; navbar `Widget` is the only global control; no Reset Layout visible; no Anonymous warning visible.

---

## 9. Tests added/updated

| Spec | Action | Coverage |
|---|---|---|
| `tests/e2e/workspace-grid-width-v5.spec.ts` | REWRITTEN (9) | 1440 → cols ≥ 48; 1920 → cols ≥ 48 and ≤ 120 (adaptive, not fixed); 2560 cols > 1920 cols (proves adaptation); data-cell-width within 26–46px of 36 target; visible-grid backdrop renders with `--workspace-cell-width` + `--workspace-row-height` + `radial-gradient`; Options defaults fill the adaptive cols (no right gutter at 1920); widget planted at `(cols-12, 12)` reaches the right boundary and persists; menu shows titles + "coming soon" chip but NOT description text; terminal routes hide footer at 1920x1080; layout schema is V5 with `cols` field |
| `tests/e2e/workspace-custom.spec.ts` | EDITED | `parsed.version === 5` |
| `tests/e2e/workspace-storage.spec.ts` | EDITED | wrong-version fixture uses `version: 5` |
| `tests/e2e/workspace-freeform-canvas.spec.ts` | EDITED | Options defaults arithmetic now reads `data-grid-cols` and asserts `chain.x + chain.w + details.w === cols` (was hard-coded `=== 48`); `parsed.version === 5` |
| Other specs | UNCHANGED | options-terminal-bottom-dock, perps-coming-soon, terminal-shell, terminal-navbar, options-chain-terminal, local-markets-seeded, markets-fallback, public-beta-footer, landing, brand-identity, no-admin-bearer all still pass |

Catalog: **144 → 148 tests in 32 files** (+4).

---

## 10. Build validations

| Command | Result |
|---|---|
| `npm run typecheck` | clean |
| `npm run lint` | clean (after one fix: dropped unused `WorkspaceLayout` import from Workspace.tsx) |
| `NEXT_PUBLIC_TRADING_API_BASE_URL=http://localhost:8080 npm run build` | green — 17 user-facing routes + 4 SSG doc slugs |
| `npx playwright test --list` | 148 tests in 32 files |
| `scripts/local-backend.sh` → `local-seed.sh` → `local-smoke.sh` | startup green; seed 12 PASS, 4 products visible; smoke **9 PASS / 0 FAIL** |

Backend stopped cleanly post-QA; port 8080 free.

---

## 11. Docs created/updated

| File | Action |
|---|---|
| `docs/FRONTEND_ADAPTIVE_VISIBLE_GRID_WORKSPACE_V5_RESULT.md` | NEW (this file) |
| `docs/public-beta/USER_TESTING_GUIDE.md` | not edited — no operator-facing instructions reference grid col count or visible-grid styling |
| `docs/public-beta/PUBLIC_TESTNET_BETA_LAUNCH_CHECKLIST.md` | not edited — tracks deploy + posture |
| `RUN_STATE.md` | closure prepended |

---

## 12. RUN_STATE update

2026-06-14 closure for FRONTEND-ADAPTIVE-VISIBLE-GRID-WORKSPACE-V5 prepended above FRONTEND-WORKSPACE-GRID-WIDTH-AND-WIDGET-MENU-FIX. Documents the adaptive cols formula, the visible-grid CSS variables, the saved-cols field + rescale-on-load, the layout schema bump (4→5), and zero backend/Solidity/scripts changes.

---

## 13. Files changed

**Created (frontend):** none (test spec REWRITTEN, not added new)
**Rewritten (frontend):** `src/lib/workspace-types.ts`, `src/components/workspace/Workspace.tsx`, `tests/e2e/workspace-grid-width-v5.spec.ts`
**Edited (frontend):** `src/lib/workspace-storage.ts` (`cols` param), `src/components/workspace/registry.tsx` (`defaultWidgetsFor` takes `cols`), `tests/e2e/workspace-custom.spec.ts`, `tests/e2e/workspace-storage.spec.ts`, `tests/e2e/workspace-freeform-canvas.spec.ts`
**Created (backend docs):** `docs/FRONTEND_ADAPTIVE_VISIBLE_GRID_WORKSPACE_V5_RESULT.md`
**Edited (root):** `RUN_STATE.md`
**Untouched:** Backend Rust source (ZERO), Solidity (ZERO), `scripts/local-*.sh` (ZERO), `BottomPanel.tsx`, `OptionDetailPanel.tsx`, `OptionsChainGrid.tsx`, `OptionsChainTerminalCore.tsx`, `ExpirySelector.tsx`, `PayoffSvg.tsx`, `HamburgerMenu.tsx`, `PublicBetaFooter.tsx`, `lib/workspace-selected-option.tsx`, `lib/workspace-bridge.tsx`, `components/workspace/WidgetFrame.tsx`, `components/workspace/WidgetMenuButton.tsx`, `components/workspace/widgets.tsx`, `components/TradingShell.tsx`, all `(trading)/page.tsx` route files, backend `.env` (mtime `2026-06-08 16:55:05.874571237 +0200` preserved), `~/DEOPT/private/` (mode 700; not read; not committed).

---

## 14. Validations

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
| New dependency added | NONE (react-grid-layout 2.2.3 unchanged) |
| Source changes limited to frontend + docs/RUN_STATE | YES |

---

## 15. Remaining workspace gaps

* **Drag-to-add from menu** via RGL's `droppingItem` — V5 still uses click-to-add + `placeAtBottom`.
* **Smarter Add Widget gap-aware placement** — currently appends at `max(y+h)`.
* **Visible drag-handle grip icon** — header is the drag region but lacks a `⋮⋮` icon.
* **Mobile portrait responsive breakpoint** — at very narrow viewports (< 800px) the 48-col minimum may still feel cramped on phones; a future polish could swap to a portrait-stack mode.
* **Cross-device localStorage sync** — out of scope.
* **Visible grid intensity preference** — current 10% emerald opacity is fixed; could expose a user preference for visibility tuning.

None block local QA, public-testnet-beta launch, or the operator's product-test pass.

---

## 16. Next milestone recommendation

**Primary (operator):** product-test V5 on the large external monitor via `bash ~/DEOPT/scripts/local-frontend.sh`. Confirm:
* Options chain on `/trade` reaches the right edge at 1440 / 1920 / 2560 with no visible step gap
* Subtle dotted backdrop renders behind widgets and changes spacing when you resize the browser window
* `data-grid-cols` increases when you move the window to a larger monitor
* A widget dragged to the right edge stays flush after reload
* Empty `/custom` workspace shows the dotted canvas (terminal feel)
* Menu still shows titles + "coming soon" chip only

**Secondary (agent-runnable):** `BACKEND-PUBLIC-TESTNET-DEPLOY-PREFLIGHT` per existing brief.

**Strictly later (NOT NOW):** drag-from-menu, gap-aware placement, mobile-portrait breakpoint, cross-device sync, real perps trading UI, mainnet activation, audit firm outreach, bug bounty launch, KMS cutover, Safe migration, flipping `isMainnetEnabled()`, publishing the announcement.

---

## 17. Cross-links
* `~/DEOPT/deopt-v2-frontend/src/lib/workspace-types.ts`
* `~/DEOPT/deopt-v2-frontend/src/components/workspace/Workspace.tsx`
* `~/DEOPT/deopt-v2-frontend/src/components/workspace/registry.tsx`
* `~/DEOPT/deopt-v2-frontend/tests/e2e/workspace-grid-width-v5.spec.ts`
* `~/DEOPT/deopt-v2-backend/docs/FRONTEND_WORKSPACE_GRID_WIDTH_AND_WIDGET_MENU_FIX_RESULT.md`
* `~/DEOPT/deopt-v2-backend/docs/BACKEND_PUBLIC_TESTNET_DEPLOY_PREFLIGHT_NEXT_TASK.md`

**End of frontend adaptive visible grid workspace V5 result.**
