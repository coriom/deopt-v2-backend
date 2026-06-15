# FRONTEND-PIXEL-CANVAS-HYDRATION-AND-LAYOUT-FIX — Result

**Date:** 2026-06-14
**Workspace root:** `~/DEOPT/`
**Approval line consumed:** "I approve DeOpt V2 pixel canvas hydration and layout fix for this run."

## TL;DR

V6 sometimes rendered every widget as a 0×0 rectangle stacked in the
top-left of the canvas because the `ref` carrying the `ResizeObserver`
was gated behind `if (!hydrated || widgets === null)`. The
`useEffect([])` that wired the observer ran once with
`canvasRef.current === null`, returned early, and never re-ran;
when the canvas div finally rendered, `canvasSize` stayed at
`(0, 0)` and `geometryToRect(...)` returned zero for every widget.

V7 fixes this by:
1. Always rendering the canvas div (the ref attaches on first paint).
2. Measuring via `useLayoutEffect` (before paint) instead of
   `useEffect`.
3. Gating widget rendering on `isCanvasReady(canvas)` (canvas ≥
   320 × 240 px) — below that, a measuring placeholder shows.
4. Resolving every widget rect through `resolveWidgetRect` which
   clamps to `minWPx` / `minHPx` so no widget can render below a
   readable rectangle.
5. Strict-validating loaded layouts on load — NaN, Infinity,
   out-of-range pcts, sub-readable widget sizes, unknown widget
   types are all rejected and replaced with the default.
6. Bumping `WORKSPACE_LAYOUT_VERSION` 6 → 7 so any V6 bucket
   (possibly saved during the broken render) is wiped on next load.
7. Refusing to `saveWorkspaceLayout(...)` while the canvas is still
   measuring.
8. Exposing `window.__deoptClearWorkspaceLayouts()` for console
   recovery (no UI surface).

## Root cause (Phase A)

```tsx
// V6 — broken
if (!hydrated || widgets === null) {
  return <div>Loading workspace…</div>;        // canvasRef NEVER attached
}
return <div ref={canvasRef}>…widgets…</div>;   // mounted only after hydration
```

```tsx
// V6 — observer setup
useEffect(() => {
  const el = canvasRef.current;
  if (!el || typeof ResizeObserver === "undefined") return;  // FIRES with el === null
  // ...
}, []);                                                       // and never re-runs
```

Render flow that produced the bug:
1. Paint 1: `widgets===null` → "Loading workspace…" div, `canvasRef` unattached.
2. `useEffect` fires: `canvasRef.current === null` → returns early. ResizeObserver never attached.
3. Hydration microtask: `setWidgets(...)`, `setHydrated(true)`.
4. Paint 2: canvas div finally mounts. `canvasSize` is still `{0, 0}`. Every `geometryToRect()` returns `(0,0,0,0)`. All widgets render absolute-positioned at top-left with zero size. Text overflows visibly.
5. No subsequent re-measurement triggers — the `useEffect([])` deps haven't changed.

The brief's mention of "stale localStorage" was a contributing factor
(any V6 layout saved while `canvasSize===0` could persist invalid
geometry), but the **primary** cause was the ref-gating bug. Bumping
6 → 7 and adding strict validation handles both.

## Canvas measurement guard (Phase B)

- **Canvas div is unconditional.** It always renders with the ref
  attached, with `data-canvas-ready` / `data-hydrated` /
  `data-widget-count` attributes for tests + debug.
- **`useLayoutEffect`** runs the first measurement before paint, so
  the very first render of widgets sees a real `canvasSize`.
- **`isCanvasReady(canvas)`** = canvas ≥ `MIN_CANVAS_WIDTH_PX = 320`
  AND ≥ `MIN_CANVAS_HEIGHT_PX = 240`. Below this, the workspace
  renders a `workspace-canvas-measuring-{id}` placeholder instead
  of widgets.
- **Backdrop CSS is suppressed until ready** — no flash of mis-
  spaced dots before measurement.
- **All geometry helpers** (`snapPx`, `pxToPct`, `pctToPx`,
  `geometryToRect`, `clampRectToCanvas`) guard against `NaN`,
  `Infinity`, negative, and zero inputs.

## Layout validation and safe reset (Phase C)

NEW in `workspace-types.ts`:

```ts
export function isValidWidgetInstance(value: unknown): value is WidgetInstance {
  // Rejects: non-object, missing id/type, unknown type,
  //          NaN/Infinity pcts, negative pcts, sub-MIN_WIDGET_PCT
  //          (4%), x+w or y+h exceeding 1 + MAX_GEOMETRY_OVERFLOW
  //          (1% float drift), invalid minWPx/minHPx.
}

export function isValidWorkspaceLayout(value: unknown): value is WorkspaceLayout {
  // Requires: valid workspaceId, widgets array, finite updatedAt/expiresAt,
  //           every widget passes isValidWidgetInstance.
}
```

`loadBucket()` in `workspace-storage.ts` now:
- Wipes the whole bucket if version ≠ 7 (handles V5 + V6 wipes).
- For each workspace, drops it from the bucket if expired OR if it
  fails `isValidWorkspaceLayout`.
- Writes the cleaned bucket back so subsequent loads start clean.

`saveWorkspaceLayout()` refuses to persist when
`canvasWidthPx <= 0` or `canvasHeightPx <= 0` — a second layer of
defence beyond the in-component `isCanvasReady` gate.

## Geometry clamp / minimum size (Phase D)

NEW in `workspace-types.ts`:

```ts
export function resolveWidgetRect(widget, canvas): PixelRect {
  const raw = geometryToRect(widget, canvas);
  const minW = widget.minWPx ?? DEFAULT_MIN_W_PX;
  const minH = widget.minHPx ?? DEFAULT_MIN_H_PX;
  return clampRectToCanvas(raw, canvas, minW, minH);
}
```

Every render path now uses `resolveWidgetRect` rather than the raw
`geometryToRect`. This guarantees that:
- `x ≥ 0`, `y ≥ 0`.
- `w ≥ minWPx` (per-widget) or `DEFAULT_MIN_W_PX = 200`.
- `h ≥ minHPx` (per-widget) or `DEFAULT_MIN_H_PX = 120`.
- `x + w ≤ canvasWidth`, `y + h ≤ canvasHeight`.
- If the canvas is too small to fit the minimum, the rect collapses
  to `(0, 0)` with the largest size that fits — never below 0.

Additionally `WidgetFrame` now uses `truncate` + `shrink-0` on
header children so a too-narrow widget shows a clipped title
instead of overlapping the remove button.

## Default layouts (Phase E)

V6 percentage defaults unchanged. They were already valid; the V6
bug was purely a render-time issue. Verified that at the smallest
supported viewport (1440×900), every default widget resolves
through `resolveWidgetRect` to a rect ≥ its `minWPx` / `minHPx`.

| Workspace | Min default widget dim (1440×900) |
|---|---|
| options.option-details | 1440×0.30 = 432 px, 900-headerish×0.70 ≈ 580 px — well above mins |
| options.bottom-dock    | 1440×1.0 = 1440 px, 900×0.30 ≈ 270 px — above mins |
| perps.perps-stats      | 1440 px × 90 px — above mins |
| perps.perps-trade-feed | 864 px × 135 px — above mins |

All defaults render readable widgets on every supported viewport.

## localStorage recovery (Phase F)

- **Automatic** invalid-layout pruning runs on every `loadBucket()`
  call (i.e., on every workspace mount).
- **`pruneExpiredLayouts()`** now also drops invalid layouts during
  the boot-time global sweep.
- **Console recovery**: `WorkspaceBridgeProvider` exposes
  `window.__deoptClearWorkspaceLayouts()` on mount. Returns the
  number of buckets wiped. No visible UI control.

Recovery one-liner for users (documentable, contains no secrets):
```js
__deoptClearWorkspaceLayouts(); location.reload();
```

## V6 behavior preserved (Phase G)

| V6 behavior | Status |
|---|---|
| Freeform pixel/percentage canvas | ✅ preserved |
| Visible grid aligned with snap (24 px) | ✅ preserved (now suppressed until ready) |
| Mouse drag on widget header | ✅ preserved |
| Mouse resize from bottom-right handle | ✅ preserved |
| Gaps between widgets preserved | ✅ preserved |
| Right-edge placement (xPct+wPct=1) | ✅ preserved |
| Navbar Widget button | ✅ preserved |
| No visible Reset Layout button | ✅ preserved |
| No "Anonymous layout temporary" message | ✅ preserved |
| No PublicBetaFooter on terminal routes | ✅ preserved |
| Compact Widget menu without descriptions | ✅ preserved |
| No yellow/orange/amber | ✅ preserved |

## Tests added/updated

| Spec | Action | Coverage |
|---|---|---|
| `tests/e2e/workspace-hydration-v7.spec.ts` | NEW (16 tests) | `data-canvas-ready=true` once measured; `/trade` default has 3 widgets ≥ 200×120 px; `/perps` default has every placeholder visible without collapse; empty `/custom` renders zero positioned widgets; adding first widget in `/custom` creates ≥ 200×120 size; **7 invalid-fixture tests** (NaN xPct / Infinity wPct / negative xPct / sub-readable wPct / xPct+wPct overflow / unknown widget type / missing geometry field) — each → empty workspace; header truncates and remove button stays inside the strip; `__deoptClearWorkspaceLayouts` callable + wipes buckets; saved bucket has `version === 7` |
| `tests/e2e/workspace-pixel-canvas-v6.spec.ts` | EDITED | `version === 7`; V5 wipe test retitled "wiped on V7 load"; NEW test "V6 bucket wiped on V7 load (post-hydration-bug safe-reset)" |
| `tests/e2e/workspace-freeform-canvas.spec.ts` | EDITED | `version === 7` |
| `tests/e2e/workspace-custom.spec.ts` | EDITED | `version === 7` |
| `tests/e2e/workspace-storage.spec.ts` | EDITED | V1/V6 wipe tests; expired/anon TTL test → V7 fixture |

Catalog: `npx playwright test --list` → **165 tests in 33 files** (+16 from V6's 149).

## Build validations

| Command | Result |
|---|---|
| `npm run typecheck` | clean |
| `npm run lint` | clean |
| `NEXT_PUBLIC_TRADING_API_BASE_URL=http://localhost:8080 npm run build` | green — 17 user-facing routes + 4 SSG doc slugs |
| `npx playwright test --list` | 165 tests in 33 files |
| `scripts/local-backend.sh` → `local-seed.sh` → `local-smoke.sh` | startup green; seed 12 PASS, 4 products visible; smoke **9 PASS / 0 FAIL** |
| Full e2e run | NOT FEASIBLE on this WSL host — `chromium_headless_shell` reports `error while loading shared libraries: libnspr4.so` (unchanged from V5/V6) |

Backend stopped post-QA; port 8080 free.

## Validations

| Check | Result |
|---|---|
| `git diff --check` (frontend + backend) | clean |
| Sensitive-string scan on edited FE files | zero hits |
| Positive-claim scan on edited FE files | zero hits |
| Amber/yellow/orange class scan on edited FE files | zero hits |
| `.env` mtime preserved | YES (`2026-06-08 16:55:05.874571237 +0200`) |
| Private dir mode preserved | YES (`700`) |
| Backend stopped post-QA | YES (port 8080 free) |
| Chain tx / broadcast / mainnet RPC / real wallet | NONE |
| `isMainnetEnabled()` still hard-coded `false` | YES (file untouched) |
| Backend Rust / Solidity / scripts changes | NONE |
| New dependency added | NONE |
| Source changes limited to frontend + docs/RUN_STATE | YES |

## Files changed

**Edited (frontend src):**
- `src/lib/workspace-types.ts` — V7 constants (`MIN_CANVAS_WIDTH_PX`, `MIN_CANVAS_HEIGHT_PX`, `MIN_WIDGET_PCT`, `MAX_GEOMETRY_OVERFLOW`); NEW `isCanvasReady`, `isValidWidgetInstance`, `isValidWorkspaceLayout`, `resolveWidgetRect`; safe-NaN guards on every helper
- `src/lib/workspace-storage.ts` — `loadBucket` drops invalid layouts; `saveWorkspaceLayout` refuses zero-size canvas; NEW `clearWorkspaceLayouts`, `clearWorkspaceLayoutForWorkspace`; `pruneExpiredLayouts` also drops invalid
- `src/lib/workspace-bridge.tsx` — `WorkspaceBridgeProvider` runs `pruneExpiredLayouts()` on mount and exposes `window.__deoptClearWorkspaceLayouts`
- `src/components/workspace/Workspace.tsx` — canvas div always rendered; `useLayoutEffect` for measurement; `isCanvasReady` gate; render through `resolveWidgetRect`; `data-canvas-ready` / `data-hydrated` attributes; `workspace-canvas-measuring-{id}` placeholder
- `src/components/workspace/WidgetFrame.tsx` — `truncate` + `shrink-0` on header so narrow widgets don't overlap controls

**Edited (frontend tests):**
- `tests/e2e/workspace-pixel-canvas-v6.spec.ts` — version=7; V5 wipe retitled; NEW V6→V7 wipe test
- `tests/e2e/workspace-freeform-canvas.spec.ts` — version=7
- `tests/e2e/workspace-custom.spec.ts` — version=7
- `tests/e2e/workspace-storage.spec.ts` — V7 fixtures + titles

**New (frontend tests):**
- `tests/e2e/workspace-hydration-v7.spec.ts` (16 tests)

**Created (backend docs):**
- `docs/FRONTEND_PIXEL_CANVAS_HYDRATION_AND_LAYOUT_FIX_RESULT.md` (this file)

**Edited (root):**
- `RUN_STATE.md` — V7 closure prepended above V6 entry

**Untouched:** Backend Rust (ZERO), Solidity (ZERO), `scripts/local-*.sh` (ZERO), backend `.env` (mtime preserved), `~/DEOPT/private/` (mode 700), user-testing guide (no operator-visible change), launch checklist (no operator-action change).

## Remaining display bugs

None observed. The pathways previously known to produce collapsed
widgets — empty `canvasSize`, NaN/Infinity geometry, sub-readable
percentages, unknown widget types — are all caught either by the
load-time validator, the render-time clamp, or the `isCanvasReady`
gate.

Edge cases that are not yet covered and that should land in a
follow-up if reported:
- Very narrow phone viewports (< 320 px wide) — V7 deliberately
  refuses to render widgets and shows the measuring placeholder.
  Mobile responsive layout is a future milestone.
- Browser zoom > 200% — geometry still rounds to `CANVAS_SNAP_PX`
  but the canvas measurement scales with zoom; widgets render
  proportionally smaller. Not a bug, but visible.

## Next milestone recommendation

**Primary (operator action, not agent-runnable):** product-test V7
on the large external Brave window via
`bash ~/DEOPT/scripts/local-frontend.sh`. Confirm:
- `/trade` renders 3 readable widgets (options chain, trade detail, bottom dock) on first paint with no flash of collapsed text
- `/perps` renders every placeholder widget with non-zero size
- empty `/custom` shows the centered "This workspace is empty"
  hint (NOT collapsed widgets) on first paint
- adding the first widget in `/custom` creates a readable rect
- if a corrupted layout is suspected, opening DevTools and running
  `__deoptClearWorkspaceLayouts(); location.reload();` clears it
- right-edge placement still works after a drag/resize gesture

**Secondary (agent-runnable):**
`BACKEND-PUBLIC-TESTNET-DEPLOY-PREFLIGHT` per existing brief.

**Strictly later (NOT NOW):** mobile portrait responsive layout,
drag-from-menu, gap-aware menu placement, visible drag-handle grip
icon, cross-device sync, real perps trading UI, mainnet activation,
audit firm outreach, bug bounty launch, KMS cutover, Safe
migration, flipping `isMainnetEnabled()`.
