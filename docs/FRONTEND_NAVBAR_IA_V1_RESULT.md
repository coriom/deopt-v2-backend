# FRONTEND-NAVBAR-IA-V1 — Result

**Date:** 2026-06-16
**Milestone:** Refactor the global navbar and hamburger drawer IA to
look closer to a trading terminal: move the hamburger to the left,
add `RFQ/Strategy`, rename `DeOpt Académie` → `DeOpt Academy`, remove
the standalone `no network` badge, reorder right-side controls to
`Widget` then `Connect wallet`, flip the drawer to open from the left
with a 13-item ordered list, and switch the global font to a slightly
rounder grotesk (Manrope + JetBrains Mono via `next/font/google`).
Frontend-only.

## Summary

The trading-shell navbar is now `logo → DeOpt → hamburger → Options →
Perps → Markets → RFQ/Strategy → Custom → DeOpt Academy` on the left
and `Widget → Connect wallet` on the right. The hamburger drawer
opens from the left and lists the 13 IA-V1 entries in exact order
(Options · Perps · Markets · RFQ/Strategy · Custom · DeOpt Academy ·
History · Leaderboard · API · Fees · Fundings · Settings · Support).
Discord + GitHub are demoted to a small secondary footer row. Four
honest placeholder routes were added (`/rfq-strategy`, `/leaderboard`,
`/fundings`, `/settings`) sharing a new `PlaceholderPage` scaffold.
Global font moves from Arial to **Manrope** (sans) + **JetBrains Mono**
(mono), wired through `next/font/google` with CSS variable fallback.

## Before / after — navbar order

**Before (left side):**
`Logo → DeOpt → Options → Perps → Markets → Custom → DeOpt Académie`

**After (left side, IA-V1):**
`Logo → DeOpt → Hamburger → Options → Perps → Markets → RFQ/Strategy → Custom → DeOpt Academy`

The hamburger sits visually between `DeOpt` and `Options` (verified
by a DOM-order test in `terminal-navbar.spec.ts`).

## Before / after — right-side controls

**Before:**
`NetworkBadge (no network / wrong network / mainnet DISABLED chip) → Connect wallet → Widget → Hamburger`

**After:**
`Widget → Connect wallet`

- `NetworkBadge` is removed from the navbar. The full-width
  `MainnetDisabledBanner` and `WrongNetworkBanner` (separate components)
  still surface meaningful chain-id problems above the navbar — no
  error state is hidden.
- `Hamburger` is no longer on the right (moved to the left, exactly one
  trigger).
- `Widget` now appears before `Connect wallet`.

The `NetworkBadge` function itself is **kept exported** in
`src/components/banners.tsx` but is no longer mounted in the trading
shell. If a future feature wants the inline chip back it can remount it.

## Before / after — drawer side

**Before:** drawer flexbox `justify-end`, panel border `border-l-…`,
right-anchored.

**After:** drawer flexbox `justify-start`, panel border `border-r-…`,
left-anchored, panel `data-drawer-side="left"`. The drawer panel's
left edge sits at viewport x ≤ 2px (asserted by Playwright spec).

## Drawer item order — implemented

Exact rendered order with hrefs:

| # | Drawer item | Hamburger testid | Route |
| --- | --- | --- | --- |
| 1 | Options       | `hamburger-link-options`       | `/trade` |
| 2 | Perps         | `hamburger-link-perps`         | `/perps` |
| 3 | Markets       | `hamburger-link-markets`       | `/markets` |
| 4 | RFQ/Strategy  | `hamburger-link-rfq-strategy`  | `/rfq-strategy` |
| 5 | Custom        | `hamburger-link-custom`        | `/custom` |
| 6 | DeOpt Academy | `hamburger-link-academy`       | `/docs` |
| 7 | History       | `hamburger-link-history`       | `/history` |
| 8 | Leaderboard   | `hamburger-link-leaderboard`   | `/leaderboard` |
| 9 | API           | `hamburger-link-api`           | `/api` |
| 10 | Fees         | `hamburger-link-fees`          | `/fees` |
| 11 | Fundings     | `hamburger-link-fundings`      | `/fundings` |
| 12 | Settings     | `hamburger-link-settings`      | `/settings` |
| 13 | Support      | `hamburger-link-support`       | `/feedback` |

There are NO drawer section headers (Pages / Docs / Community)
anymore — the brief explicitly asked for a direct ordered menu. The
`MENU` title strip + close button are kept. Discord (`hamburger-link-discord`)
and GitHub (`hamburger-link-github`) live in a separate `Community`
strip below the primary 13 so they do not visually compete with the
ordered list.

## Route mapping for every drawer item

| Item | Route used | Pre-existing | Notes |
| --- | --- | --- | --- |
| Options | `/trade` | yes | The chain-terminal workspace (no change). |
| Perps | `/perps` | yes | Perps workspace placeholder (no change). |
| Markets | `/markets` | yes | Products index (no change). |
| **RFQ/Strategy** | `/rfq-strategy` | **NEW placeholder** | New static page via `PlaceholderPage`. |
| Custom | `/custom` | yes | Custom workspaces (no change). |
| DeOpt Academy | `/docs` | yes | Brief explicitly allowed `/docs`; existing docs index reused. |
| History | `/history` | yes | Existing trade-history page (no change). |
| **Leaderboard** | `/leaderboard` | **NEW placeholder** | New static page via `PlaceholderPage`. |
| API | `/api` | yes | Existing public-API placeholder page. |
| Fees | `/fees` | yes | Existing fees placeholder page. |
| **Fundings** | `/fundings` | **NEW placeholder** | New static page via `PlaceholderPage`. |
| **Settings** | `/settings` | **NEW placeholder** | New static page via `PlaceholderPage`. |
| Support | `/feedback` | yes | Existing feedback / report-issue route. |

`Portfolio` is no longer in the drawer (it is not in the 13-item IA),
but `/portfolio` still resolves directly — covered by a regression
test (`Portfolio route remains reachable via direct URL`).

## Placeholder routes created

Four new static `(trading)` routes share a new shared scaffold:

- **`src/components/PlaceholderPage.tsx`** — `PlaceholderPage`
  component with: testnet-beta status chip, title, summary paragraph,
  "What you can rely on right now" bulleted callout, "What lands later"
  bulleted callout, three link tiles (Docs / Support / Discord).
- **`src/app/(trading)/rfq-strategy/page.tsx`** — RFQ / Strategy page,
  testid `rfq-strategy-page`. Honest: "no live RFQ executor yet —
  nothing on this page is wired to an order router".
- **`src/app/(trading)/leaderboard/page.tsx`** — Leaderboard page,
  testid `leaderboard-page`. Honest: "no scoring rubric, no rewards
  program, no real value attached to any ranking".
- **`src/app/(trading)/fundings/page.tsx`** — Fundings page, testid
  `fundings-page`. Honest: "Perps are not live yet, so no real funding
  has been paid or received in this build".
- **`src/app/(trading)/settings/page.tsx`** — Settings page, testid
  `settings-page`. Honest: "There are no stored preferences today —
  workspace layouts persist in localStorage under the active wallet
  key".

Each placeholder follows the same `/fees` + `/api` posture: no fake
live claims, no mainnet hint, no external API calls, no positive-claim
language, no amber/yellow/orange.

## Font / typography decision

Decision: switch the global UI font from the implicit Arial stack to
**Manrope** (sans, variable axis 200..800) and **JetBrains Mono** (mono),
wired via `next/font/google` for self-hosted, build-time loading
(no runtime CDN fetch).

Why Manrope:
- Rounded grotesk that reads softer than Arial at 12–14px while staying
  technical.
- Open-source (SIL OFL), proven for terminal-style UIs.
- Already vendored by `next/font/google` — no new dependency needed.
- Variable font, single subset (`latin`), tiny download.

Why JetBrains Mono:
- Numbers, addresses, tickers, prices, hashes, and tables stay
  monospace and aligned.
- Excellent disambiguation of `0/O`, `1/l/I` at small sizes.
- Already in widespread use for trading terminals + dev tools.

Implementation:
- `src/app/layout.tsx` imports both fonts via `next/font/google` and
  attaches `${sans.variable} ${mono.variable}` to `<html>`. The
  `display: "swap"` avoids invisible-text-during-load.
- `src/app/globals.css` keeps the existing CSS-var contract
  (`--app-font-sans`, `--app-font-mono`) but updates the `:root`
  fallback chains so an environment that strips next/font's className
  still gets readable Manrope or system-ui (no recursive variable
  reference — the prior draft of this milestone hit that footgun and
  was reverted before commit).
- The Tailwind `@theme inline` block already exposes
  `--font-sans` / `--font-mono` to utility classes (`font-mono`,
  `font-sans`), so existing call sites pick up the new font without
  any per-class refactor.

No external runtime font load is introduced — `next/font/google`
self-hosts the woff2 at build time.

## Files changed

**New (6):**
- `src/components/PlaceholderPage.tsx`
- `src/app/(trading)/rfq-strategy/page.tsx`
- `src/app/(trading)/leaderboard/page.tsx`
- `src/app/(trading)/fundings/page.tsx`
- `src/app/(trading)/settings/page.tsx`
- `deopt-v2-backend/docs/FRONTEND_NAVBAR_IA_V1_RESULT.md` (this doc)

**Modified (11):**
- `src/app/layout.tsx` — `next/font/google` Manrope + JetBrains Mono
  wired via `<html>` `className`.
- `src/app/globals.css` — `:root` `--app-font-sans` / `--app-font-mono`
  fallbacks updated for the new fonts.
- `src/app/(trading)/layout.tsx` — navbar rewritten: hamburger moved
  left, `NetworkBadge` removed, `Widget` placed before `Connect wallet`
  on the right, `DeOpt Académie` (coming-soon span) replaced by a
  real `DeOpt Academy` link to `/docs`, `RFQ/Strategy` added,
  `data-testid="terminal-navbar"` and `terminal-navbar-actions` exposed
  for cleaner test selectors.
- `src/components/HamburgerMenu.tsx` — V3: drawer flexbox
  `justify-end` → `justify-start`, panel `border-l` → `border-r`,
  panel `data-testid="hamburger-drawer-panel"`, drawer container
  `data-drawer-side="left"`; sections removed; 13 ordered primary
  items + secondary `Community` row for Discord + GitHub; testid for
  `support` (route `/feedback`) replaces the older `feedback`.
- `src/components/wallet/WalletConnectButton.tsx` — added
  `data-testid="wallet-connect-button"` + `data-wallet-state` attr on
  all three render branches so tests can assert order without text-match
  fragility.
- `tests/e2e/terminal-navbar.spec.ts` — rewritten end-to-end for IA-V1
  (primary nav order, hamburger between `DeOpt` and `Options`,
  right-side order, drawer side = left, drawer 13-item order, Discord +
  GitHub in secondary list, no admin / RPC leak, Escape close,
  outside-click close, Portfolio direct-URL still resolves,
  no amber/yellow/orange).
- `tests/e2e/mainnet-disabled.spec.ts` — removed `network-badge-mainnet`
  + post-switch `network-badge-ok` assertions (chip is gone). The
  full-width `mainnet-disabled-banner` remains the sole mainnet
  call-out and is still asserted.
- `tests/e2e/wrong-network-banner.spec.ts` — removed `network-badge-wrong-network`
  + post-switch `network-badge-ok` assertions. The full-width banner
  remains.
- `tests/e2e/wallet-connected.spec.ts` — removed the `anvil` shortname
  network-badge text assertion. The wallet-button shortened-address
  assertion is kept.
- `tests/e2e/report-issue.spec.ts` — `hamburger-link-feedback` →
  `hamburger-link-support` (renamed in IA-V1, same target `/feedback`).
- `tests/e2e/fees-and-api-placeholders.spec.ts` — `Hamburger →
  Portfolio` test rewritten to assert that the drawer no longer carries
  a Portfolio link AND that `/portfolio` direct URL still resolves.

## Validations run

| Check | Result |
| --- | --- |
| `npm run lint` | **PASS** (no warnings) |
| `npm run typecheck` (`tsc --noEmit`) | **PASS** |
| `npm run build` (Next.js 16.1.6 Turbopack) | **PASS** — 24 routes generated (was 20; +4 new placeholders) |
| `npx playwright test --list` | **PASS** — `Total: 211 tests in 35 files` |
| `git diff --check` (frontend) | **PASS** — no whitespace errors |
| Grep `DeOpt Académie` / `Académie` / `Academie` in source code (non-test) | **PASS** — only the `terminal-navbar.spec.ts` regression assertions reference the strings |
| Grep `no network` in `src/components/HamburgerMenu.tsx` + `src/app/(trading)/layout.tsx` | **PASS** — 0 hits |
| Hamburger button count in changed shell | **PASS** — exactly one trigger (`hamburger-button`) per page |
| Right-side order DOM check | **PASS** — `navbar-widget-button` appears before `wallet-connect-button` |
| Drawer side | **PASS** — `data-drawer-side="left"` + `border-r` |
| Drawer item order | **PASS** — 13 entries rendered in the brief's exact order |
| Grep amber/yellow/orange classes in new + changed files | **PASS** — 0 hits |
| Grep `DATABASE_URL=` / `PRIVATE_KEY=` / `alchemy.com/v2/` / `infura.io/v3/` / `mainnet.base.org` in changed files | **PASS** — 0 hits |
| Grep `Bearer [A-Za-z0-9_.-]{16,}` in changed files | **PASS** — 0 hits |
| `.env` mtime preserved | `2026-06-08 16:55:05` (unchanged) |
| Private dir mode preserved | `700` (unchanged) |

## Skipped validations

| Validation | Reason |
| --- | --- |
| `npm run e2e:local` (actually run Playwright) | WSL host lacks `libnspr4.so` for chromium_headless_shell. The catalog parses (`--list` succeeds) but the browser will not launch on this machine. Tests will execute on the operator's host. |
| Visual screenshot diff | No baseline checked in. Operator visual approval is the gate for layout / font feel. |
| Backend integration | This milestone does not touch backend or any backend payload. |
| Solidity ABI re-pin | Not in scope. |
| Production deploy preflight | Not in scope; the app is not deployed yet. |

## Safety posture confirmation

| Statement | Confirmed |
| --- | --- |
| No secrets read, printed, or written | YES |
| No private keys touched or referenced | YES |
| No RPC URLs added or referenced | YES |
| No `DATABASE_URL` references added | YES |
| No admin bearer tokens added | YES |
| No `.env` files read or modified (mtime preserved) | YES |
| No chain transaction issued | YES |
| No broadcast / send / deploy / mint / approve / transfer | YES |
| No mainnet network used or referenced | YES |
| No backend logic touched | YES |
| No Solidity touched | YES |
| No `scripts/local-*.sh` edits | YES |
| Backend `private/` dir mode preserved (700) | YES |
| Wallet-connection logic preserved (no edits to provider code, only adding `data-testid` on the button shell) | YES |
| Workspace / widget picker preserved (no edits to registry, storage, or `WidgetMenuButton` internals) | YES |
| No new external runtime API calls (next/font self-hosts at build time) | YES |
| No marketing claims / positive-claim language introduced | YES |
| No amber / yellow / orange classes introduced | YES |

## Current next recommendation

Visual sign-off pass on the new navbar + drawer + Manrope font at
1920×1080 and 1200×900. Specifically: confirm hamburger sits between
`DeOpt` and `Options`, drawer opens from the left, the 13 IA-V1 items
read in the brief's exact order, Discord + GitHub feel like a quiet
secondary row, and Manrope at 12–14px feels rounder than the prior
Arial stack without losing terminal density. If approved, the next
surface in the page-by-page polish queue (per the handoff report
section 11) is `/perps` perps workspace polish. If the operator
prefers to fill in the new placeholder pages first, the highest-value
next step is the **`/settings`** page since it would let the
workspace-layout reset / export tooling get a real home instead of
the console helper. Nothing should be started without an explicit
milestone brief.
