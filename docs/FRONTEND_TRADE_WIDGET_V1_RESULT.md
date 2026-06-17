# FRONTEND-TRADE-WIDGET-V1 — Result

**Date:** 2026-06-16
**Milestone:** Polish the `/trade` options terminal by redesigning the
legacy "Trade · detail" widget into a professional options trading ticket
inspired by Derive-style option widgets — **without** Derive branding,
amber/yellow/orange colors, fake live-market claims, or backend changes.
Frontend-only.

## Summary

The legacy `option-details` widget (titled "Trade · detail") has been
renamed to `trade` / "Trade" and replaced by a compact options order
ticket with two modes (Book / RFQ), full Buy-to-Open / Sell-to-Open
segmented entry, dense order fields, a channel notice, an Enable
Trading action, a 5-row cost breakdown, and a 4-tab body
(Payoff / Greeks / Trades / Book). Stored layouts are migrated cleanly
via a `WORKSPACE_LAYOUT_VERSION` bump 7 → 8.

## Widget rename result

| Surface | Before | After |
| --- | --- | --- |
| `WidgetType` enum / `KNOWN_WIDGET_TYPES` set | `"option-details"` | `"trade"` |
| Widget registry key | `"option-details"` | `"trade"` |
| Registry `title` (visible in WidgetFrame chrome) | `Trade · detail` | `Trade` |
| Registry `description` | "5-tab panel: Trade / Payoff / Greeks / Details / Risk." | "Options order ticket — Buy/Sell, Limit, Post, GTC, Payoff/Greeks/Trades/Book tabs." |
| Render component | `OptionDetailsWidget` (in `widgets.tsx`) | `TradeWidget` (in `widgets.tsx`) |
| Underlying panel component | `OptionDetailPanel.tsx` (still kept; not used by the widget anymore) | `TradeTicketPanel.tsx` (new) |
| Workspace `data-testid` (computed `widget-${type}`) | `widget-option-details` | `widget-trade` |
| Default placement in `options` workspace | `option-details` at `(0.7, 0, 0.3, 0.7)` | `trade` at `(0.7, 0, 0.3, 0.7)` |
| Description text references in `payoff` + `greeks` registry entries | "also a tab inside Trade · detail" | "also a tab inside Trade" |

No duplicate `Trade` widget exists. The widget picker shows a single
`Trade` entry (workspaces: `options`, `custom-1..3`).

## Stored-layout migration

The `WORKSPACE_LAYOUT_VERSION` bumped 7 → 8 in
`src/lib/workspace-types.ts`. This is the project's established
migration pattern (V1 → V2 → … → V7 → **V8**). The behavior:

1. The loader in `src/lib/workspace-storage.ts` rejects any stored
   bucket whose `version` field is not equal to `WORKSPACE_LAYOUT_VERSION`.
2. A V7 bucket containing one or more widgets of type
   `"option-details"` is therefore dropped at load time and the
   workspace falls back to the default layout — which now seeds
   `"trade"` instead of `"option-details"`.
3. The user does NOT see a broken state. They see the new default
   Options layout: `options-chain (70%)` + `trade (30%)` on top,
   `bottom-dock (100%)` below.

The `pruneExpiredLayouts` helper carries the same wipe behavior.
`saveWorkspaceLayout` always persists the current
`WORKSPACE_LAYOUT_VERSION` (8) so any new layout written after this
milestone is V8.

A new test in `tests/e2e/workspace-hydration-v7.spec.ts`
("V7 buckets carrying the legacy `option-details` type are dropped on
load") plants a V7 bucket with `option-details` and asserts that on
reload the workspace renders `widget-trade` and the resaved bucket has
`version: 8`. The previously planted bucket is gone.

## UI changes by section

### Widget chrome (no change)
Existing `WidgetFrame` continues to provide the drag handle, the title
strip, and the remove button. The title now reads `Trade`.

### Compact ticket header (new — `TradeHeader`)
- Height: 40px (within the 36–42 target).
- Left: dotted grip (`data-testid="trade-header-grip"`) + instrument
  title (`data-testid="trade-instrument-title"`). The title reflects
  the chain selection (e.g. `K = $30,000 Call · exp 2026-…`) or the
  placeholder `BTC $66000 Call Jun 19` when nothing is selected.
- Right: `Book` / `RFQ` mode selector
  (`data-testid="trade-mode-select"`). Black background, thin
  bottom border, hover-state border tint only — no SaaS gradient.

### Buy/Sell segmented selector (Book mode)
- Two equal-width buttons, 32px tall, no rounded pill.
- Active uses `bg-emerald-600/90 text-black`; inactive uses
  `bg-zinc-900 text-zinc-400`.
- Tested via `trade-side-buy` / `trade-side-sell` testids and
  `data-selected` attribute.

### Dense order fields
- `Order Type` dropdown (`Limit` / `Market`).
- `Limit Price` with `$` prefix and an `Ask: $705` micro-label hint
  (`data-testid="trade-ask-hint"`).
- `Amount` text input with `0.0` placeholder.
- Bottom row: `Post` checkbox + `GTC` time-in-force select
  (options: GTC / IOC / FOK).
- All inputs use `bg-black/40` with `border-zinc-800` and an emerald
  focus border. Keyboard accessibility preserved through native
  `<input>` / `<select>` / `<label>` semantics.

### Channel notice
Compact dark callout (`data-testid="trade-channel-notice"`) with a
small `i` icon and two stacked lines:
1. "Open a decentralized channel for gas-free and instantaneous trading."
2. "This signature is gas-free to send."

### Enable Trading action
Full-width 36px-tall button (`data-testid="trade-enable-button"`) with
a small `!` info icon. Styled as a restrained terminal control (not
the marketing emerald-on-emerald primary), reflecting the
"channel-not-yet-open" interaction state. No fake claims.

### Cost breakdown
5 rows in a labeled `<dl>` (`data-testid="trade-cost-breakdown"`):
| Label | Value | Style |
| --- | --- | --- |
| Max Cost | `$0.00` | strong (zinc-100) |
| Margin Required | `$0.00` | muted (zinc-400) |
| Buying Power | `$0.00` | muted |
| Est. Fee | `$0.00` | muted |
| Est. Rewards | `0 DRV` | muted + dotted-underline on label (`trade-cost-rewards-label`) |
All values monospace, right-aligned. No amber/yellow/orange — the
dotted underline on `Est. Rewards` uses `decoration-emerald-500/30`.

### Internal tabs
4 tabs (`Payoff` / `Greeks` / `Trades` / `Book`) on a 36px nav with a
border-bottom across the row. Active tab uses a near-white bottom
underline (no chunky pill).

#### Payoff tab
- Top metric row: `Max Loss $596` / `Break Even $66,596` /
  `Max Profit Infinity` (emerald accent on Infinity).
- Schematic `PayoffSvg` (the existing component) inside a thin-bordered
  dark frame.
- Honest disclaimer: "Schematic only. The break-even and ladder values
  above are local mock figures used to compose the ticket layout."

#### Greeks tab
- 5 greeks (Delta / Gamma / Vega / Theta / Rho) in a 2-column grid.
- Honest disclaimer
  (`data-testid="trade-greeks-mock-disclaimer"`): "Local mock values
  for layout. The pricing service has not shipped yet; the live chain
  renders '—' for greeks."

#### Trades tab
- 4-column table (Time / Instrument / Amount / Price) with 6 example
  rows. Positive amounts in emerald, negative in red.
- Honest disclaimer (`data-testid="trade-trades-mock-disclaimer"`):
  "Local mock prints used to compose the trades feed. No live tape."

#### Book tab
- 3-column ladder (Price / Size / Total): 5 ask rows in dark-red
  accent above, centered Spread row ($58, 8.91%), 4 bid rows in
  emerald accent below.
- Each row has a depth bar behind it (red for asks, emerald for bids),
  scaled by `depthPct`.
- Honest disclaimer (`data-testid="trade-book-mock-disclaimer"`):
  "Local mock book — there is no resting limit-order book in this
  testnet beta yet."

### RFQ compact mode (header dropdown → `rfq`)
A second compact body (`data-testid="trade-body-rfq"`):
- Direction segmented Buy / Sell (`trade-rfq-side-buy` /
  `trade-rfq-side-sell`).
- Instrument readout (`trade-rfq-instrument`) reflecting the same
  selection as Book mode's title.
- Ratio cell with `1` and a clear `✕` button
  (`trade-rfq-ratio` / `trade-rfq-ratio-clear`).
- Amount input (`trade-rfq-amount`).
- Filter (`trade-rfq-filter`) and expand (`trade-rfq-expand`) icon
  buttons.
- Honest "RFQ executor is not live in this testnet beta — submission
  is disabled" notice.

## Files changed

**New (2):**
- `deopt-v2-frontend/src/components/trading/terminal/TradeTicketPanel.tsx`
  (~530 lines)
- `deopt-v2-frontend/tests/e2e/trade-widget-v1.spec.ts` (~280 lines,
  18 tests)

**Modified (7):**
- `deopt-v2-frontend/src/lib/workspace-types.ts`
  - Header comment rewritten for V8.
  - `WidgetType` union: `"option-details"` → `"trade"`.
  - `KNOWN_WIDGET_TYPES` set: same swap.
  - `WORKSPACE_LAYOUT_VERSION = 7` → `WORKSPACE_LAYOUT_VERSION = 8`.
- `deopt-v2-frontend/src/lib/workspace-storage.ts`
  - Header comment refreshed for V8.
- `deopt-v2-frontend/src/components/workspace/registry.tsx`
  - Removed import of `OptionDetailsWidget`; added `TradeWidget`.
  - Replaced `"option-details"` registry entry with `"trade"` (title
    `"Trade"`, new description, `minWPx: 300`, `minHPx: 360`).
  - `defaultWidgetsFor("options")` now seeds `type: "trade"`.
  - Two other registry descriptions that referenced "Trade · detail"
    updated to "Trade".
- `deopt-v2-frontend/src/components/workspace/widgets.tsx`
  - Removed import of `OptionDetailPanel`.
  - Added import of `TradeTicketPanel`.
  - `OptionDetailsWidget` → `TradeWidget` (renders `TradeTicketPanel`).
- `deopt-v2-frontend/tests/e2e/workspace-hydration-v7.spec.ts`
  - Two `widget-option-details` → `widget-trade` swaps.
  - `version: 7` fixture → `version: 8`.
  - "Saved bucket carries the V7 version field" assertion updated to
    expect `version === 8`.
  - New test: "V7 buckets carrying the legacy `option-details` type
    are dropped on load".
- `deopt-v2-frontend/tests/e2e/workspace-pixel-canvas-v6.spec.ts`
  - Widget type filter `option-details` → `trade`.
- `deopt-v2-frontend/tests/e2e/workspace-freeform-canvas.spec.ts`
  - Widget type filter `option-details` → `trade`.
  - `widget-option-details` testid → `widget-trade`.
- `deopt-v2-frontend/tests/e2e/options-terminal-bottom-dock.spec.ts`
  - Header docstring refreshed.
  - `widget-option-details` → `widget-trade`.
  - Tab assertion rewritten from 5 tabs (trade/payoff/greeks/details/
    risk) to 4 tabs (payoff/greeks/trades/book).
  - Empty-comment refresh.
- `deopt-v2-frontend/tests/e2e/options-chain-terminal.spec.ts`
  - Header docstring refreshed.
  - "clicking a Call cell" test rewritten to assert
    `trade-instrument-title` text changes after click (instead of
    asserting `detail-panel` appears).
  - 5-tab list test rewritten to 4-tab list using `trade-tab-*` and
    `trade-panel-content-*` testids.
  - Greeks-tab test rewritten to assert `trade-greeks-mock-disclaimer`
    (replaces "coming soon in the testnet beta" copy that lived on
    the old `OptionDetailPanel`).
  - Risk-tab test removed (the Risk tab no longer exists — risk
    disclosures live on `/markets/[productId]` and `/docs/limitations`).
  - Detail-CTA test replaced by a Book/RFQ mode-selector roundtrip
    test.

`OptionDetailPanel.tsx` itself is **not deleted**. It is still imported
by the non-workspace `OptionsChainTerminal.tsx` component (a legacy
single-page variant of the chain terminal). Removing it is out of
scope for this milestone.

## Validations run

| Check | Result |
| --- | --- |
| `npm run lint` | **PASS** (no warnings) |
| `npm run typecheck` (`tsc --noEmit`) | **PASS** |
| `npm run build` (Next.js 16.1.6 Turbopack) | **PASS** — 20 routes generated, no `/trade` chunk regressions |
| `npx playwright test --list` | **PASS** — `Total: 206 tests in 35 files` (was 188 in 34 files; +1 spec, +18 trade-widget tests, +1 hydration test) |
| `git diff --check` (frontend) | **PASS** — no whitespace errors |
| Grep `Trade Detail\|trade-detail\|tradeDetail\|TradeDetails` in `src/` and `tests/` | **PASS** — 0 substantive hits (the only matches in `src/content/public-beta/USER_TESTING_GUIDE.md` predate this milestone and refer to wallet UX, not the widget) |
| Grep `option-details` in `src/` and `tests/` | **PASS** — all remaining hits are intentional migration comments + the V7→V8 wipe test |
| Grep `\bDerive\b` in the new component | **PASS** — only `deriveInstrumentTitle` (function name, not user-facing) |
| Grep `amber-\|yellow-\|orange-\|bg-amber\|bg-yellow\|bg-orange` in `TradeTicketPanel.tsx` | **PASS** — 0 hits |
| Grep `DATABASE_URL=\|PRIVATE_KEY=\|alchemy\.com/v2/\|infura\.io/v3/\|mainnet\.base\.org\|Bearer [A-Za-z0-9_.-]{16,}` in changed files | **PASS** — 0 hits |
| `.env` mtime preserved | `2026-06-08 16:55:05` (unchanged) |
| Private dir mode preserved | `700` (unchanged) |

## Skipped validations

| Validation | Reason |
| --- | --- |
| `npm run e2e:local` (full Playwright run) | WSL host lacks `libnspr4.so` for chromium_headless_shell. The catalog parses (`--list` succeeds) but the actual browser cannot launch on this machine. Tests will execute on the operator's host. |
| Visual screenshot diff | No screenshot baseline is checked in. Operator visual approval is the gate. |
| Backend integration | This milestone does not touch the backend or any backend-bound payload. |
| Solidity ABI re-pin | This milestone does not touch contracts or ABIs. |
| Production deploy preflight | Not in scope; the project has no deployed app yet. |

## Safety posture confirmation

| Statement | Confirmed |
| --- | --- |
| No secrets read, printed, or written to any file | YES |
| No private keys touched or referenced | YES |
| No RPC URLs added or referenced (no `alchemy.com/v2/…`, `infura.io/v3/…`, `mainnet.base.org`) | YES |
| No `DATABASE_URL` references added | YES |
| No admin bearer tokens added | YES |
| No `.env` files read or modified (mtime preserved) | YES |
| No chain transaction issued | YES |
| No broadcast / send / deploy / mint / approve / transfer | YES |
| No mainnet network used or referenced | YES |
| No backend logic touched (Rust workers, executor, indexer all untouched) | YES |
| No Solidity touched (no contract edits, no ABI re-pin) | YES |
| No scripts/local-*.sh edits | YES |
| Backend `private/` dir mode preserved (700) | YES |
| Frontend changes scoped to widget rename + redesign + matching test refresh | YES |
| New widget's tabs (Greeks / Trades / Book) carry honest "local mock" disclaimers | YES |
| No Derive branding in any user-facing string | YES |
| No amber / yellow / orange brand classes introduced | YES |
| No fake "audited" / "mainnet-ready" / "production-ready" / "safe for real funds" / "guaranteed" / "institutional-grade" claims | YES |

## Workspace V7→V8 schema note

The schema-version constant in `src/lib/workspace-types.ts` is now
`WORKSPACE_LAYOUT_VERSION = 8`. Storage tests at
`tests/e2e/workspace-storage.spec.ts` still reference V7 in their
docstrings but assert version-bump-wipe behavior generically and will
pass under V8 (any non-current bucket is wiped). Updating those
docstrings is intentionally out of scope to minimize churn — the
behavior they describe still holds.

## Current next recommendation

Visual sign-off pass on the redesigned `/trade` Trade widget at
1920×1080 and at a narrower 1200×900 viewport. Once approved, the
next surface in the page-by-page polish order (per the handoff report
section 11) is `/perps` perps workspace polish. If the operator wants
to defer perps and keep iterating the options terminal, the next-most
valuable touchups would be:

- **Cost breakdown live-bind**: thread the real selected option + size
  into Max Cost / Margin Required (existing backend already returns
  the underlying numbers via the quote-preview endpoint used by
  `/markets/[productId]`).
- **Enable Trading wiring**: route the button through the
  `/markets/[productId]` `TradeTicket` flow rather than acting as a
  static affordance.
- **RFQ live state**: surface a "no quotes yet" empty state instead of
  the static notice.

None of those should be initiated without an explicit milestone brief.
