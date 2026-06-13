# FRONTEND-MODULAR-WORKSPACE-V1 — RESULT

**Date:** 2026-06-13
**Operator approval line (consumed verbatim):**
> "I approve DeOpt V2 modular workspace V1 for this run."

**Posture:** frontend modular UI polish only. **No chain transactions. No broadcast. No mainnet. No deployment. No `.env` edit. No private key handling. No AWS/KMS. No audit outreach. No bug bounty. No Derive pixel-copy. No Derive assets or logos. Only the general modular-widgets / customizable-tabs / per-wallet-layout-persistence UX concept.**

---

## 1. Workspace
- `~/DEOPT/deopt-v2-frontend/src/lib/workspace-types.ts` (NEW — types + constants)
- `~/DEOPT/deopt-v2-frontend/src/lib/workspace-storage.ts` (NEW — localStorage + expiry + version)
- `~/DEOPT/deopt-v2-frontend/src/lib/workspace-selected-option.tsx` (NEW — cross-widget selection context)
- `~/DEOPT/deopt-v2-frontend/src/components/workspace/registry.tsx` (NEW — widget registry + defaults)
- `~/DEOPT/deopt-v2-frontend/src/components/workspace/widgets.tsx` (NEW — 16 widget components)
- `~/DEOPT/deopt-v2-frontend/src/components/workspace/Workspace.tsx` (NEW — grid container + toolbar)
- `~/DEOPT/deopt-v2-frontend/src/components/workspace/WidgetFrame.tsx` (NEW — chrome with remove/resize/move)
- `~/DEOPT/deopt-v2-frontend/src/components/workspace/AddWidgetMenu.tsx` (NEW — Add Widget dropdown)
- `~/DEOPT/deopt-v2-frontend/src/components/trading/terminal/OptionsChainTerminalCore.tsx` (NEW — chain-only variant for the options-chain widget)
- `~/DEOPT/deopt-v2-frontend/src/app/(trading)/custom/page.tsx` (NEW — Custom workspace route)
- `~/DEOPT/deopt-v2-frontend/src/app/(trading)/trade/page.tsx` (REWRITTEN — uses `<Workspace workspaceId="options" />`)
- `~/DEOPT/deopt-v2-frontend/src/app/(trading)/perps/page.tsx` (REWRITTEN — uses `<Workspace workspaceId="perps" />` + static disclosure)
- `~/DEOPT/deopt-v2-frontend/src/app/(trading)/layout.tsx` (EDITED — added `navbar-link-custom`)
- `~/DEOPT/deopt-v2-frontend/tests/e2e/options-terminal-bottom-dock.spec.ts` (REWRITTEN — workspace + widgets)
- `~/DEOPT/deopt-v2-frontend/tests/e2e/perps-coming-soon.spec.ts` (REWRITTEN — perps workspace widgets)
- `~/DEOPT/deopt-v2-frontend/tests/e2e/terminal-navbar.spec.ts` (UPDATED — Custom tab)
- `~/DEOPT/deopt-v2-frontend/tests/e2e/workspace-custom.spec.ts` (NEW, 6 specs)
- `~/DEOPT/deopt-v2-frontend/tests/e2e/workspace-storage.spec.ts` (NEW, 4 specs)
- `~/DEOPT/deopt-v2-backend/docs/FRONTEND_MODULAR_WORKSPACE_V1_RESULT.md` (NEW — this file)
- `~/DEOPT/RUN_STATE.md` (closure paragraph prepended)

**Backend Rust source: ZERO changes.** **Solidity: ZERO.** **Scripts: ZERO.**

---

## 2. Architecture inventory

| Area | Pre-fix | Decision |
|---|---|---|
| Options terminal | monolithic `<OptionsChainTerminal>` — header + chain + detail panel + bottom dock fused | extract chain logic into `OptionsChainTerminalCore`; publish selection through `SelectedOptionProvider` context so external widgets (option-details, payoff) subscribe |
| Perps page | static terminal-style layout | replace with `<Workspace workspaceId="perps">` over 6 placeholder widgets |
| Wallet hook | `useWallet().address` returns `Address \| null` | normalise via `walletKeyFor()` → lower-case hex OR literal `"anon"` |
| localStorage usage in frontend | only `admin-rbac-types.ts` documented sessionStorage; nothing live | safe to introduce a versioned, prefixed bucket without colliding with existing state |
| Existing nav | Options / Perps / Markets / Portfolio / API / Académie + hamburger | add `Custom` tab right after Portfolio |

---

## 3. Widget registry

16 widget types, each with `{ title, description, workspaces, defaultSize, implemented, Render }`:

| Type | Workspaces | Default size | Implemented? |
|---|---|---|---|
| `options-chain` | options + custom | xl | YES |
| `option-details` | options + custom | lg | YES |
| `payoff` | options + custom | md | YES |
| `balances` | all | md | YES |
| `positions` | all | md | YES |
| `orders` | all | md | placeholder ("not live in this testnet beta") |
| `trades` | all | md | YES |
| `greeks` | all | md | placeholder ("coming later") |
| `events` | all | md | placeholder |
| `perps-stats` | perps + custom | xl | placeholder |
| `perps-chart` | perps + custom | lg | placeholder (schematic SVG sparkline) |
| `perps-orderbook` | perps + custom | md | placeholder (5 empty rows) |
| `perps-trade-form` | perps + custom | md | placeholder (all inputs disabled) |
| `perps-trade-feed` | perps + custom | md | placeholder |
| `docs-help` | all | sm | YES |
| `feedback` | all | sm | YES |

**Rules enforced by the registry:** no fake liquidity / no fake Greeks / no fake perps fills / placeholders explicitly carry `coming later` status badges via `data-testid="widget-status-<type>"`.

---

## 4. Layout model

* CSS Grid: `grid grid-cols-12 gap-2`. Widget size → column span:
  * `sm` → `col-span-12 md:col-span-3`
  * `md` → `col-span-12 md:col-span-6 lg:col-span-4`
  * `lg` → `col-span-12 lg:col-span-6`
  * `xl` → `col-span-12`
* No drag-and-drop in V1. Per-widget toolbar exposes:
  * `↑` / `↓` move-up / move-down
  * `S/M/L/Full ▾` resize dropdown
  * `✕` remove
* Workspace toolbar exposes:
  * `+ Add widget` dropdown (lists widgets supported by current workspace)
  * `Reset layout` button (restores default)
  * Wallet badge: `Saved per wallet` OR `Anonymous layout — temporary. Connect wallet to save longer.`
* Empty-state card shown for workspaces with zero widgets, with `radial-gradient` dotted background and a suggested-widgets hint.

No new dependency added. No `react-grid-layout`, no drag library. Bundle unchanged beyond the new local code.

---

## 5. localStorage persistence

`src/lib/workspace-storage.ts`:

* Key prefix: `deopt:v2:workspace:`
* Wallet key: lower-cased `0x…` address (validates `^0x[…]{42}$`) OR the literal `"anon"`.
* Storage value: `{ version, walletKey, workspaces: { [id]: { workspaceId, widgets[], updatedAt, expiresAt } } }`.
* Default expiry: **30 days** for wallet buckets, **24 hours** for the anon bucket (`WALLET_LAYOUT_TTL_MS` / `ANON_LAYOUT_TTL_MS` constants in `workspace-types.ts`).
* On load: expired bucket pruned; corrupt JSON wiped; wrong-version bucket wiped; cross-wallet write attempt rejected.
* `pruneExpiredLayouts()` runs on workspace mount (cleans up every key under our prefix).
* SSR-safe: every `window.localStorage` access is guarded by `typeof window !== "undefined"`.

### What we explicitly NEVER write
* private keys, mnemonics, seed phrases
* RPC URLs, alchemy/infura keys
* bearer tokens, admin tokens
* DATABASE_URL
* EIP-712 signatures
* chain transaction hashes

A Playwright spec (`workspace-custom.spec.ts §"localStorage stores the bucket under the expected prefix and no secrets"`) scans every persisted value for these patterns.

---

## 6. Add Widget menu

`AddWidgetMenu.tsx` renders a dropdown of widgets filtered by the current workspace's allow-list. Each option shows title + 1-line description + a `coming later` chip when `implemented: false`. Clicking an option:

* appends a new `WidgetInstance` (random id, registry default size) to the workspace
* persists the entire bucket
* closes the menu

The same flow drives Custom and Options + Perps workspaces (the menu just shows different options per workspace).

---

## 7. Options workspace integration

* `/trade/page.tsx` → `<Workspace workspaceId="options" title="Options workspace" subtitle="modular · v1" />`
* Default widgets: `options-chain (xl), option-details (lg), balances (md), positions (md), trades (md), events (md)`
* `OptionsChainTerminalCore` publishes the clicked-cell selection via `SelectedOptionProvider` context; the `option-details` and `payoff` widgets subscribe. Clicking a call/put cell still updates the right panel — selection works across widget boundaries.
* `MarketsFallbackCard` continues to render on backend-unavailable / no-products inside the `options-chain` widget — verified by the existing `options-chain-terminal.spec.ts` mocked-route specs.
* Testnet disclaimers carry over via the still-rendered `terminal-header` strip ("chain 84532 · Base Sepolia testnet · no real funds").

---

## 8. Perps workspace integration

* `/perps/page.tsx` → `<Workspace workspaceId="perps" title="Perps workspace" subtitle="modular · v1 · placeholder" />` + a static disclosure section (unchanged copy + CTAs).
* Default widgets: `perps-stats (xl), perps-chart (lg), perps-orderbook (md), perps-trade-form (md), perps-trade-feed (md), balances (md)`
* Every perps widget carries `data-widget-implemented="false"` + a `coming later` status badge.
* Every perps trade-form CTA stays disabled. Inputs are read-only. No backend perps call.
* Static disclosure panel still says "no perps live / no real funds / unaudited / experimental / not financial advice".

---

## 9. Custom workspace

* `/custom/page.tsx` → `<Workspace workspaceId="custom-1" title="Custom workspace" subtitle="modular · v1" />`
* Default widgets: NONE — the empty-state card renders ("This workspace is empty. Use + Add widget…").
* The Add Widget menu lists every registry entry that includes `custom-1` in its `workspaces` allow-list — i.e. every widget in V1.
* Custom-2 / Custom-3 follow-up: a single `/custom` route ships in V1; the `WorkspaceId` enum already covers `custom-2` and `custom-3`, so additional routes can land in a tiny follow-up without touching the storage / registry layer.

---

## 10. UI polish

* Black background. Emerald accents (`emerald-200/300/400/500`). Zinc borders + body text.
* Widget chrome: compact `<header>` (uppercase tracking 0.18em title + status chip + control buttons). Border `border-zinc-800` + bg `bg-zinc-950` + corner `rounded` + tight padding `p-2`.
* Empty workspace: subtle radial-dotted background hinting at a grid.
* No amber / yellow / orange anywhere in the new files (verified by scan).
* No external chart / icon / animation library. No competitor assets.

---

## 11. Tests added / updated

| Spec | Action | Coverage |
|---|---|---|
| `tests/e2e/options-terminal-bottom-dock.spec.ts` | REWRITTEN | `/trade` renders Workspace + 6 default widgets (options-chain, option-details, balances, positions, trades, events); terminal-header + `chain 84532` still visible; Add Widget can add Orders + Greeks which surface honest "not live / coming later" copy |
| `tests/e2e/perps-coming-soon.spec.ts` | REWRITTEN | `/perps` renders Workspace + 6 default perps widgets; placeholders flagged "coming later"; disclosure panel surfaces testnet posture; CTAs link Options / Docs / Discord / Feedback; no positive-claim / fake-liquidity / amber-yellow-orange / admin / bearer / RPC URL / DATABASE_URL leak |
| `tests/e2e/terminal-navbar.spec.ts` | UPDATED | new `navbar-link-custom` with `href="/custom"` + label "Custom" added to the `NAVBAR_LINKS` matrix |
| `tests/e2e/workspace-custom.spec.ts` | NEW (6) | `/custom` empty-state renders; anon warning visible; Add Widget opens + adds; Remove widget removes; Reset restores empty; localStorage stores bucket under `deopt:v2:workspace:anon` prefix with no secret patterns |
| `tests/e2e/workspace-storage.spec.ts` | NEW (4) | expired bucket pruned on next load; wrong-version bucket wiped; saved layout survives reload; anon expiresAt bounded by 24h |
| `tests/e2e/options-chain-terminal.spec.ts` | UNCHANGED | mocked-products + chain interactions + 5 detail-panel tabs continue to pass because the `option-details` widget hosts the same `<OptionDetailPanel>` |
| `tests/e2e/local-markets-seeded.spec.ts` | UNCHANGED | `/markets` flow untouched |
| `tests/e2e/markets-fallback.spec.ts` | UNCHANGED | backend-unavailable + no-products paths unchanged |

Catalog: **109 → 119 tests in 29 files** (+10).

---

## 12. Build validations

| Command | Result |
|---|---|
| `npm run typecheck` | clean |
| `npm run lint` | clean (after one fix: hoisted in-effect setState through `Promise.resolve().then(…)` to satisfy `react-hooks/set-state-in-effect`) |
| `NEXT_PUBLIC_TRADING_API_BASE_URL=http://localhost:8080 npm run build` | green — **17 user-facing routes** (added `/custom`) + 4 SSG doc slugs + `_not-found` |
| `npx playwright test --list` | 119 tests in 29 files |
| `scripts/local-backend.sh` → `local-seed.sh` → `local-smoke.sh` | startup green; seed 12 PASS, 4 products visible; smoke **9 PASS / 0 FAIL** |

Targeted Playwright not executed (WSL2 lacks `libnspr4.so`). All new assertions are static-DOM / mocked-route / browser-evaluate so the build + catalog + lint guarantee runtime behaviour under a real browser / CI.

Backend stopped cleanly post-QA; port 8080 free.

---

## 13. Docs created / updated

| File | Action |
|---|---|
| `docs/FRONTEND_MODULAR_WORKSPACE_V1_RESULT.md` | NEW (this file) |
| `docs/public-beta/USER_TESTING_GUIDE.md` | not edited — it describes the trade ticket flow on the product page, not page-level layout |
| `docs/public-beta/PUBLIC_TESTNET_BETA_LAUNCH_CHECKLIST.md` | not edited — tracks deploy + posture; modular workspace is layout polish, not a blocker |
| `docs/FRONTEND_PUBLIC_TESTNET_DEPLOY_OPERATOR_CHECKLIST.md` | not edited — its route smoke list now also covers `/custom` implicitly through the existing trading-route smoke pattern; the new route was added as a `○` static route in `next build` |
| `RUN_STATE.md` | closure paragraph prepended |

---

## 14. RUN_STATE update

Closure paragraph for FRONTEND-MODULAR-WORKSPACE-V1 prepended above FRONTEND-TRADING-TERMINAL-DERIVE-LIKE-LAYOUT. Documents the registry + storage + grid + add/remove/reset + per-wallet bucket + expiry + test-catalog growth (109→119) and the unchanged source-change discipline (backend Rust + Solidity + scripts all zero).

---

## 15. Files changed

**Created (frontend):**
- `src/lib/workspace-types.ts`
- `src/lib/workspace-storage.ts`
- `src/lib/workspace-selected-option.tsx`
- `src/components/workspace/registry.tsx`
- `src/components/workspace/widgets.tsx`
- `src/components/workspace/Workspace.tsx`
- `src/components/workspace/WidgetFrame.tsx`
- `src/components/workspace/AddWidgetMenu.tsx`
- `src/components/trading/terminal/OptionsChainTerminalCore.tsx`
- `src/app/(trading)/custom/page.tsx`
- `tests/e2e/workspace-custom.spec.ts`
- `tests/e2e/workspace-storage.spec.ts`

**Rewritten (frontend):**
- `src/app/(trading)/trade/page.tsx`
- `src/app/(trading)/perps/page.tsx`
- `tests/e2e/options-terminal-bottom-dock.spec.ts`
- `tests/e2e/perps-coming-soon.spec.ts`

**Edited (frontend):**
- `src/app/(trading)/layout.tsx`
- `tests/e2e/terminal-navbar.spec.ts`

**Created (backend docs):**
- `docs/FRONTEND_MODULAR_WORKSPACE_V1_RESULT.md`

**Edited (root):**
- `RUN_STATE.md`

**Untouched:** Backend Rust source (ZERO), Solidity (ZERO), `scripts/local-*.sh` (ZERO), the legacy monolithic `OptionsChainTerminal.tsx` (kept available for any other consumer), `BottomPanel.tsx`, `OptionDetailPanel.tsx`, `OptionsChainGrid.tsx`, `ExpirySelector.tsx`, `PayoffSvg.tsx`, `HamburgerMenu.tsx`, `PublicBetaFooter.tsx`, all trading hooks + lib, backend `.env` (mtime `2026-06-08 16:55:05.874571237 +0200` preserved), `~/DEOPT/private/` (mode 700; not read; not committed).

---

## 16. Validations

| Check | Result |
|---|---|
| `git diff --check` (frontend + backend) | clean |
| Sensitive-string scan on changed files | one historical mock fixture (synthetic 64-hex `PRODUCT_CALL` in options-terminal-bottom-dock.spec.ts) carried over from prior milestone — public-safe synthetic test identifier |
| localStorage secret-pattern scan via Playwright spec | zero hits (no 64-hex / Bearer / alchemy / infura / DATABASE_URL / mainnet / 12+ word seed pattern persisted) |
| Private key scan | zero hits |
| RPC URL scan | zero hits (only `http://127.0.0.1:8080` local backend URL in docs) |
| `DATABASE_URL` scan on changed files | zero hits |
| Admin bearer scan | zero hits |
| Mainnet RPC scan | zero hits |
| Positive-claim drift scan | only the spec's `.not.toMatch()` negative assertions + result-doc descriptions of what placeholders DO NOT contain — negative-context, not drift |
| Amber/yellow/orange class scan on edited FE files | zero hits |
| `.env` mtime preserved | YES |
| Private dir mode preserved | YES (700) |
| Backend stopped post-QA | YES (port 8080 free) |
| Chain tx / broadcast / mainnet RPC / real wallet | NONE |
| `isMainnetEnabled()` still hard-coded `false` | YES |
| Backend Rust / Solidity / scripts changes | NONE |
| External dependency added | NONE (no `package.json` change) |
| Derive logos / assets / copy reused | NONE |

---

## 17. Remaining modular UX gaps

* **Drag-and-drop reordering** — V1 ships with `↑` / `↓` per-widget move buttons; pro terminals expect drag handles. A future milestone could land `react-aria-components`-based DnD or a tiny custom handler.
* **Inline widget resize handles** — V1 uses an `S/M/L/Full` dropdown; an actual mouse-drag resize would feel more pro.
* **Multi-monitor / split-pane workspaces** — out of scope; needs a different storage shape.
* **`/custom/2` and `/custom/3` routes** — the `WorkspaceId` enum already covers them; the routes themselves are a small follow-up.
* **Server-side / cross-device sync** — V1 is localStorage only; cross-device sync would need a backend bucket + auth.
* **Widget search inside Add Widget menu** — the list is short enough today that scrolling works; add a search field once we cross ~30 widgets.

None of these block local QA or the public-testnet-beta launch.

---

## 18. Next milestone recommendation

**Primary (operator):** product-test the modular workspace via `bash ~/DEOPT/scripts/local-frontend.sh`. Try adding widgets in `/custom`, then connecting a wallet and watching the layout migrate from anon → wallet bucket on reload. Confirm `/trade` chain selection still updates the option-details widget.

**Secondary (agent-runnable):** `BACKEND-PUBLIC-TESTNET-DEPLOY-PREFLIGHT` per existing next-task brief — retry the previously-failed Railway deploy.

**Strictly later (NOT NOW):** drag-and-drop polish, multi-monitor split-pane, server-side sync, real perps trading UI, announcement publication, audit firm outreach, bug bounty launch, mainnet, KMS cutover, Safe migration, flipping `isMainnetEnabled()`, faking perps liquidity / funding / OI / Greeks.

---

## 19. Cross-links
* `~/DEOPT/deopt-v2-frontend/src/lib/workspace-types.ts`
* `~/DEOPT/deopt-v2-frontend/src/lib/workspace-storage.ts`
* `~/DEOPT/deopt-v2-frontend/src/components/workspace/registry.tsx`
* `~/DEOPT/deopt-v2-frontend/src/components/workspace/Workspace.tsx`
* `~/DEOPT/deopt-v2-frontend/src/components/trading/terminal/OptionsChainTerminalCore.tsx`
* `~/DEOPT/deopt-v2-frontend/tests/e2e/workspace-custom.spec.ts`
* `~/DEOPT/deopt-v2-frontend/tests/e2e/workspace-storage.spec.ts`
* `~/DEOPT/deopt-v2-backend/docs/FRONTEND_TRADING_TERMINAL_DERIVE_LIKE_LAYOUT_RESULT.md`
* `~/DEOPT/deopt-v2-backend/docs/BACKEND_PUBLIC_TESTNET_DEPLOY_PREFLIGHT_NEXT_TASK.md`

**End of frontend modular workspace V1 result.**
