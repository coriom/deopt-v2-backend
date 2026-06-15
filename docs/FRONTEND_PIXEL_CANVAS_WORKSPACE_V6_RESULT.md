# FRONTEND-PIXEL-CANVAS-WORKSPACE-V6 — Result

**Date:** 2026-06-14
**Workspace root:** `~/DEOPT/`
**Approval line consumed:** "I approve DeOpt V2 pixel canvas workspace V6 for this run."

## TL;DR

V6 replaces the V5 `react-grid-layout` column model with a true
pixel/percentage freeform canvas. Widgets store `(xPct, yPct, wPct,
hPct)` in `[0, 1]`. The canvas is measured live by `ResizeObserver`,
pointer-driven drag/resize updates pixel rects then converts back to
percentages for persistence, and the visible dotted backdrop is sized
by the same `CANVAS_SNAP_PX = 24` constant that drives drag/resize
snapping. The `react-grid-layout` dependency is removed entirely.

There is no column count, no compactor, no per-viewport rescaling,
and no scrollbar-gutter slack. The right edge of the canvas equals
the right edge of the rendered terminal `<main>`, and a widget
resized to `xPct + wPct = 1.0` sits flush against it on every
monitor.

## Root-cause verification (Phase A)

V5 used `useContainerWidth` from RGL, which only attached its
measurement ref to the inner `workspace-grid-{id}` div. That div
only renders when `widgets.length > 0`, so on empty workspaces (and
during first hydration) `containerWidth` stayed `0`, `computeCols`
returned `GRID_MIN_COLS = 48`, and the first user-added widget was
placed under those minimum-column assumptions before the real
measurement landed.

Even once RGL measured, the column-grid model forced widget right
edges to `cols * cellStride + margin`. RGL rounds the per-cell width
internally, so the last cell typically ends 1-4 px short of the
container right edge — visible as a "step gap dead zone" on large
monitors.

Conclusion: any column-grid engine will conflate "where the widget
snaps" with "where the widget can be placed". The cleanest fix is
to decouple them — snap is a pixel constant, placement is
percentage-of-canvas.

## Chosen architecture (Phase B)

- `react-grid-layout` removed from `package.json` and `globals.css`.
- New geometry type:
  ```ts
  interface WidgetInstance {
    id: string;
    type: WidgetType;
    xPct: number;  // 0..1
    yPct: number;  // 0..1
    wPct: number;  // 0..1
    hPct: number;  // 0..1
    minWPx?: number;
    minHPx?: number;
  }
  ```
- `Workspace.tsx` mounts a single `<div ref={canvasRef}>` whose
  `getBoundingClientRect()` is observed via `ResizeObserver` on
  every layout change. Widgets are absolute-positioned children of
  that canvas.
- Pixel rect for each widget = `geometryToRect(widget, canvasSize)`
  and is recomputed on every render.

## Pixel/percentage canvas (Phase C)

Snap unit: `CANVAS_SNAP_PX = 24` (`src/lib/workspace-types.ts`).
Drag and resize both round to this step before clamping to canvas
bounds.

Persistence is canvas-independent: a layout saved at 1920×980 reads
back unchanged at 2560×1320 because percentages are absolute. The
saved bucket still carries `canvasWidthPx` / `canvasHeightPx` for
debug telemetry but the runtime never needs them to render.

## Visible grid alignment (Phase C)

```css
backgroundImage:    radial-gradient(circle, rgba(110, 231, 183, 0.10) 1px, transparent 1px);
backgroundSize:     ${CANVAS_SNAP_PX}px ${CANVAS_SNAP_PX}px;
backgroundPosition: 0 0;
```

The backdrop is attached to the **same** `canvasRef` element that
hosts widgets, so origin alignment is guaranteed: dots and widget
top-lefts share `(0, 0)`. No second container, no offset, no margin
slack. Subtle emerald (10% alpha) — no yellow / orange / amber.

## Pointer-driven drag/resize (Phase D)

- `WidgetFrame` exposes `onDragStart(e)` on the header and
  `onResizeStart(e)` on a bottom-right corner handle
  (`data-testid="widget-resize-handle-{id}"`).
- `Workspace` owns the `pointermove` / `pointerup` / `pointercancel`
  listeners on the canvas element, so a drag that leaves the
  widget's bounding rect is not lost.
- Pointer capture is set on the widget element where the gesture
  started; release happens implicitly on `pointerup`.
- On every `pointermove`: compute new pixel rect → `snapPx()` →
  `clampRectToCanvas(rect, canvas, minWPx, minHPx)` →
  `rectToPctGeometry()` → setState. Persistence runs on
  `pointerup` to avoid `localStorage` churn during the gesture.
- Minimum sizes are per-widget (registry's `minWPx` / `minHPx`)
  with fallbacks `DEFAULT_MIN_W_PX = 200` and `DEFAULT_MIN_H_PX =
  120`.
- The Remove button (`✕`) calls `e.stopPropagation()` on
  `pointerdown` so clicking it never starts a drag.
- `cursor: move` on the header, `cursor: se-resize` on the handle.

## Storage migration (Phase E)

- `WORKSPACE_LAYOUT_VERSION` bumped 5 → 6.
- V6 saves: `{ workspaceId, widgets, canvasWidthPx, canvasHeightPx, updatedAt, expiresAt }`.
- Any V5 (or older) bucket is detected by `loadBucket()`'s
  version-mismatch path and wiped — the V5 column geometry is
  structurally incompatible with V6 percentages and a conversion
  would be unreliable (no canvas size was stored). Safe-reset is
  the right policy.
- Same secret-avoidance posture: no private keys, no RPC URLs, no
  bearer tokens, no `DATABASE_URL`, no signatures.
- TTLs unchanged: 30 days for connected wallets, 24 hours for `anon`.

## Default layouts (Phase F)

Defaults are pure percentages so they fill the canvas edge-to-edge on
every monitor:

| Workspace | Widget            | xPct | yPct | wPct | hPct |
|-----------|-------------------|------|------|------|------|
| options   | options-chain     | 0.00 | 0.00 | 0.70 | 0.70 |
| options   | option-details    | 0.70 | 0.00 | 0.30 | 0.70 |
| options   | bottom-dock       | 0.00 | 0.70 | 1.00 | 0.30 |
| perps     | perps-stats       | 0.00 | 0.00 | 1.00 | 0.10 |
| perps     | perps-chart       | 0.00 | 0.10 | 0.60 | 0.45 |
| perps     | perps-orderbook   | 0.60 | 0.10 | 0.40 | 0.30 |
| perps     | perps-trade-form  | 0.60 | 0.40 | 0.40 | 0.30 |
| perps     | perps-trade-feed  | 0.00 | 0.55 | 0.60 | 0.15 |
| perps     | bottom-dock       | 0.00 | 0.70 | 1.00 | 0.30 |
| custom-*  | (empty)           | —    | —    | —    | —    |

`chain.wPct + details.wPct = 1.0` and `bottom-dock.wPct = 1.0` — no
right gutter at any viewport.

## Widget menu (Phase G)

Unchanged from V5: navbar `Widget` button only, titles + "coming
soon" chip only, description text in `title=` tooltip only. No
visible Reset Layout, no Anonymous-warning, no body-toolbar Add
Widget. The pre-V3 `AddWidgetMenu.tsx` component (already dead) was
deleted.

## Terminal shell (Phase H)

`TradingShell` was already correct (V3); no changes needed. `/trade`,
`/perps`, `/custom`, `/markets`, `/portfolio` render
`trading-main-terminal` and hide `PublicBetaFooter`. The compact
top status strip (`TestnetUnauditedBanner`, `MainnetDisabledBanner`,
`WrongNetworkBanner`) is preserved.

## Tests added / updated (Phase I)

| Spec | Action | Coverage |
|---|---|---|
| `tests/e2e/workspace-pixel-canvas-v6.spec.ts` | NEW (rename of v5 spec, full rewrite — 10 tests) | canvas width ≥ 1400/1880 at 1440/1920, grows beyond 1920 at 2560; `data-canvas-snap-px` matches backdrop step; Options defaults sum to 1.0 horizontally; widget planted at `xPct+wPct=1` has its rendered right edge within ±2 px of canvas right edge; menu shows titles + chip only, no description; terminal routes hide footer; schema is V6 with `xPct/yPct/wPct/hPct` and no `x/y/w/h/cols`; saved layout's percentages survive viewport resize; V5 column bucket is wiped on V6 load |
| `tests/e2e/workspace-freeform-canvas.spec.ts` | EDITED | Uses `workspace-canvas-{id}` instead of `workspace-grid-{id}`; asserts percentage-geometry persistence; gap-preservation now plants `(xPct=0.4, yPct=0.4)`; schema version = 6 |
| `tests/e2e/workspace-custom.spec.ts` | EDITED | Storage assertion now reads `xPct/yPct/wPct/hPct` and `version === 6` |
| `tests/e2e/workspace-storage.spec.ts` | EDITED | Wrong-version + expired bucket fixtures use V6 shape; comments + test titles say V6 |
| `tests/e2e/terminal-shell.spec.ts` | EDITED | Resize handle selector now `[data-testid^='widget-resize-handle-']` (was `.react-resizable-handle`); header comment says V6 |

Catalog: `npx playwright test --list` → **149 tests in 32 files**.

Mocks/route interception unchanged. No real wallet, no live backend
required.

## Build validations (Phase J)

| Command | Result |
|---|---|
| `npm run typecheck` | clean |
| `npm run lint` | clean |
| `NEXT_PUBLIC_TRADING_API_BASE_URL=http://localhost:8080 npm run build` | green — 17 user-facing routes + 4 SSG doc slugs |
| `npx playwright test --list` | 149 tests in 32 files |
| `scripts/local-backend.sh` → `local-seed.sh` → `local-smoke.sh` | startup green; seed 12 PASS, 4 products visible; smoke **9 PASS / 0 FAIL** |
| `npx playwright test --grep "workspace\|terminal-shell"` | NOT FEASIBLE on this WSL host — `chromium_headless_shell` reports `error while loading shared libraries: libnspr4.so: cannot open shared object file`. Same outcome as V5; runs on operator's host with system deps installed. Catalog enumeration passes. |

Backend stopped post-QA; port 8080 free.

## Validations

| Check | Result |
|---|---|
| `git diff --check` (frontend) | clean |
| Sensitive-string scan on edited FE files | zero substantive hits (only NEGATIVE assertions inside test/regex strings) |
| Positive-claim scan on edited FE files | zero hits |
| Amber/yellow/orange class scan on edited FE files | zero hits |
| `.env` mtime preserved | YES (`2026-06-08 16:55:05.874571237 +0200`) |
| Private dir mode preserved | YES (`700`) |
| Backend stopped post-QA | YES (port 8080 free) |
| Chain tx / broadcast / mainnet RPC / real wallet | NONE |
| `isMainnetEnabled()` still hard-coded `false` | YES (file untouched) |
| Backend Rust / Solidity / scripts changes | NONE |
| New dependency added | NONE (one **removed**: `react-grid-layout`) |
| Source changes limited to frontend + docs/RUN_STATE | YES |

## Files changed

**Rewritten (frontend):**
- `src/lib/workspace-types.ts` — pct geometry + snap helpers (no `cols`)
- `src/lib/workspace-storage.ts` — `saveWorkspaceLayout` takes canvas px size
- `src/components/workspace/Workspace.tsx` — pixel-canvas + pointer drag/resize
- `src/components/workspace/registry.tsx` — `defaultWPct` / `defaultHPct` / `minWPx` / `minHPx`; pct defaults
- `src/components/workspace/WidgetFrame.tsx` — `onDragStart` / `onResizeStart` props; bottom-right handle

**Edited (frontend):**
- `src/app/globals.css` — RGL stylesheet imports removed; dead RGL CSS rules removed
- `package.json` / `package-lock.json` — `npm uninstall react-grid-layout` (6 packages removed)

**Deleted (frontend):**
- `src/components/workspace/AddWidgetMenu.tsx` — pre-V3 dead component

**Renamed + rewritten (tests):**
- `tests/e2e/workspace-grid-width-v5.spec.ts` → `workspace-pixel-canvas-v6.spec.ts`

**Edited (tests):**
- `tests/e2e/workspace-freeform-canvas.spec.ts`
- `tests/e2e/workspace-custom.spec.ts`
- `tests/e2e/workspace-storage.spec.ts`
- `tests/e2e/terminal-shell.spec.ts`

**Created (docs):**
- `docs/FRONTEND_PIXEL_CANVAS_WORKSPACE_V6_RESULT.md` (this file)

**Edited (root):**
- `RUN_STATE.md` — V6 closure prepended above V5 entry

**Untouched:** Backend Rust (ZERO), Solidity (ZERO), `scripts/local-*.sh` (ZERO), backend `.env` (mtime preserved), `~/DEOPT/private/` (mode 700, not read, not committed), all non-workspace FE files, user-testing guide (V6 surface change is invisible at the prose level — same Widget button, same menu, same widgets), launch checklist (no operator-action change).

## Remaining workspace gaps

- Drag-from-menu (drop a widget onto a specific canvas position)
- Visible drag-handle grip icon (`⋮⋮`) on the header
- Mobile portrait responsive breakpoint (V6 is sized for desktop terminal)
- Cross-device localStorage sync
- User-tunable visible-grid intensity (currently fixed 10% emerald)
- Keyboard accessibility for drag/resize (V6 is pointer-only by design)

None block local QA, public-testnet-beta launch, or the operator's
product-test pass on the large external monitor.

## Next milestone recommendation

**Primary (operator action, not agent-runnable):** product-test V6 on
the large external Brave window via
`bash ~/DEOPT/scripts/local-frontend.sh`. Confirm:
- options chain on `/trade` reaches the right edge at 1440 / 1920 / 2560 with no visible step gap
- subtle dotted backdrop renders behind widgets and dot spacing stays at 24 px even when the browser window is resized
- `data-canvas-width` matches the visible terminal width
- a widget dragged to the right edge sits flush against the canvas edge after reload
- a widget resize gesture snaps in 24 px steps and stops exactly at the canvas edge
- empty `/custom` workspace shows the dotted canvas (terminal feel) immediately
- menu still shows titles + "coming soon" chip only

**Secondary (agent-runnable):** `BACKEND-PUBLIC-TESTNET-DEPLOY-PREFLIGHT` per existing brief.

**Strictly later (NOT NOW):** drag-from-menu, gap-aware placement,
mobile-portrait breakpoint, cross-device sync, real perps trading UI,
mainnet activation, audit firm outreach, bug bounty launch, KMS
cutover, Safe migration, flipping `isMainnetEnabled()`.
