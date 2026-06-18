# FRONTEND-DEVELOPERS-CONSOLE-V1 — Result

Status: **completed** (simplified after operator review — see "Simplification iteration" below).

## Simplification iteration

The first cut of the console exposed an Identity strip (4 cells),
an Environment / status panel, Public API + Wallet Auth cards, the
embedded WebSocket sandbox, an MM Gateway section, and an
Environment Utilities section. Operator review preferred a much
sparser layout (Derive-style app-side Developers page). The
visible page was rewritten to that shape:

* **Visible `/api` page now contains, in order:**
  1. Big `Developers` title + four icon-text links — Guides, API
     Reference, GitHub, and an env-aware label (default
     `Testnet`).
  2. Wallet / Signer compact row with copy-to-clipboard buttons.
     Disconnected defaults render `Not connected` / `Not
     available`.
  3. `Mint Tokens` card with a single line of copy and a disabled
     `Mint UI planned` chip — no live faucet wired.
  4. `Session Keys` card — disabled `+ Register Session Key`
     button, honest empty table (`No session keys registered`).
  5. `Subaccounts` card — disabled `+ Create Subaccount` button,
     honest empty table (`No subaccounts configured`).
  6. Compact footer note describing MM Gateway as separate /
     operator-only / not public, with a `Read more` link to the
     docs MM Gateway page, plus a `Open the WebSocket sandbox →`
     link to `/api/sandbox`.
* **Environment / HTTP / WS / MM status panel and capability
  cards were removed** from the visible page. The trading
  shell's global testnet banner remains above the page.
* **WebSocket sandbox moved out of `/api`** to a new
  `/api/sandbox` route, served by `src/app/(trading)/api/sandbox/page.tsx`.
  The sub-route inherits the terminal layout because the
  existing `TradingShell.TERMINAL_ROUTES` matcher already
  prefix-matches `/api`.

The Identity / Environment-panel / capability-card scaffolding
described later in this document was the V1 first cut. It is
preserved here for traceability — the testids it lists are no
longer present on the page and are explicitly asserted absent by
the rewritten Playwright spec.

Transforms the trading-terminal `/api` route into a practical in-app
Developers Console: identity strip, environment status, capability
cards, planned-feature placeholders for Session Keys and Subaccounts,
environment utilities, an MM-Gateway summary, and the embedded
WebSocket sandbox at the bottom. Long-form documentation continues to
live in the separate `deopt-v2-docs` site.

## Summary

The FRONTEND-DOCS-SPLIT-V1 milestone removed the long-form API
documentation from the terminal and left behind a small `DevPanel`.
That panel was the right intermediate step but not the long-term
shape. This milestone replaces it with a real Developers Console —
practical, action-oriented, organised around what a connected
developer / trader can actually *do* inside the DeOpt app.

The page no longer repeats testnet warnings on every card; the
environment / status block carries that information once, and the
global testnet banner remains at the top of the trading shell.

## Files changed

### `~/DEOPT/deopt-v2-frontend`

**New:**

* `src/components/api/DevelopersConsole.tsx` (~751 lines, client
  component). Owns every section described below and re-uses the
  shared `WsQuickTest` component as its bottom sandbox.

**Modified:**

* `src/app/(trading)/api/page.tsx` — now a 13-line server wrapper
  that renders `<DevelopersConsole />` and sets the metadata title to
  `"Developers — DeOpt"`.
* `tests/e2e/api-v1.spec.ts` — rewritten to target the new console
  testids (16 tests).
* `tests/e2e/fees-and-api-placeholders.spec.ts` — comment + the
  hamburger → API smoke updated to look for `developers-console`
  instead of the removed `dev-panel`. `/fees` placeholder coverage
  was not touched.

**Removed:**

* `src/components/api/DevPanel.tsx` — superseded by
  `DevelopersConsole`. No other code referenced it.

### `~/DEOPT/deopt-v2-backend`

* New `docs/FRONTEND_DEVELOPERS_CONSOLE_V1_RESULT.md` (this file).
* No backend source touched.

### `~/DEOPT/deopt-v2-docs`

* Untouched.

### Root

* `RUN_STATE.md` prepended with the milestone closure entry.

## Before / after — terminal `/api` role

| Aspect | Before (FRONTEND-DOCS-SPLIT-V1) | After (this milestone) |
| --- | --- | --- |
| Role | Compact placeholder linking to the docs site | Practical in-app Developers Console |
| Title | `Developers` | `Developers` (subtitle now "Manage developer access, signer status, API connectivity, and automation tools.") |
| Sections | Header + wallet block + 5 cards + 4 quick links + sandbox | Header + identity strip + environment panel + Public API card + Wallet Auth card + Session Keys + Subaccounts + Environment Utilities + MM Gateway + sandbox |
| Disclaimer density | One footer line + per-card status pills | One global testnet banner (trading shell) + one environment-state badge inside the console |
| Backend probes | None | Single `/trading/health` probe at mount for HTTP reachability |
| Long-form documentation | Removed in V1; not present | Still removed; not added back |

## Final page sections

| # | Section | testid | Notes |
| --- | --- | --- | --- |
| 1 | Header | `developers-console-header` + `developers-console-quicklinks` | Title + subtitle + 4 quick links (Guides → `/quickstart`, API Reference → `/developers`, GitHub → `github.com/DeOpt`, Environment → in-page anchor `#developers-console-env-panel`). |
| 2 | Identity strip | `developers-console-identity` | 4 cells: Wallet, Signer, Network, Executor. Disconnected defaults: `Not connected`, `Not available`, `Not connected`, `Unknown (Status source unavailable)`. |
| 3 | Environment / status panel | `developers-console-environment` | Environment badge (Local / Testnet / Production / Unknown, read from `NEXT_PUBLIC_DEOPT_ENV`), Chain (from connected wallet → `findChain`, else `expectedChain`), Public HTTP (Live / Offline / Unknown from `/trading/health` probe), Public WebSocket (Unknown — no intrusive probe), MM Gateway (Operator only / Off by default). |
| 4 | Public API card | `card-public-api` | Badges (HTTP / WebSocket / Wallet Auth) + 3 actions (Open Docs / API Reference / WebSocket Guide). |
| 5 | Wallet Auth card | `card-wallet-auth` | Status Live + 1 action (Open Wallet Auth). |
| 6 | Session Keys | `developers-console-session-keys` | Planned badge, disabled `Register Session Key` button, table with 6 columns, empty-state row "No session keys registered". |
| 7 | Subaccounts | `developers-console-subaccounts` | Planned badge, disabled `Create Subaccount` button, table with 5 columns, empty-state row "No subaccounts configured". |
| 8 | Environment Utilities | `developers-console-utilities` | Conditional copy depending on environment. Shows `Open Quickstart` + `View Limitations` always; shows a `Mint UI planned` chip in Local / Testnet / Unknown environments (hidden in Production). |
| 9 | MM Gateway | `developers-console-mm-gateway` | Two status badges (Operator only / Off by default), capability list (Bulk submit/cancel · RFQ quoting · Quote replace · Cancel-on-disconnect), action `Read MM Gateway Docs`. |
| 10 | WebSocket Quick Test | `developers-console-sandbox` + `api-ws-quick-test` | Existing client-only sandbox. URL input default derives from `NEXT_PUBLIC_PUBLIC_WS_URL` → `NEXT_PUBLIC_TRADING_API_BASE_URL` → `ws://localhost:8080/ws`. Never auto-connects. |

## Links to docs

Every external link on the console targets the docs site through the
existing `docsPath()` helper, which reads `NEXT_PUBLIC_DOCS_URL` and
falls back to `http://localhost:3002` for local dev. No production
URL is hardcoded.

| Action | Docs path |
| --- | --- |
| Guides | `/quickstart` |
| API Reference | `/developers` |
| Public API card → Open Docs | `/` |
| Public API card → API Reference | `/developers/http-api` |
| Public API card → WebSocket Guide | `/developers/websocket-api` |
| Wallet Auth card → Open Wallet Auth | `/developers/wallet-auth` |
| Environment Utilities → Open Quickstart | `/quickstart` |
| Environment Utilities → View Limitations | `/limitations` |
| MM Gateway → Read MM Gateway Docs | `/developers/mm-gateway` |
| GitHub | `https://github.com/DeOpt` (kept as-is — same target the hamburger drawer already advertises). |

The in-page `Environment` quick link uses an anchor
(`#developers-console-env-panel`) and does not leave the page.

## Session Keys / Subaccounts status

Both sections are surfaced **as Planned**:

* `Register Session Key` and `Create Subaccount` buttons render
  visibly disabled (`disabled` + `aria-disabled="true"` + a muted
  cursor-not-allowed style). They carry a `title` attribute that
  explains the gate.
* Both tables render their full header row and an honest empty-state
  row (`No session keys registered` / `No subaccounts configured`).
* The "Planned" status badge is restated next to each section title.
* No fake rows are ever produced. There is no fake-data factory; the
  empty state is the only state.
* The copy explicitly distinguishes session keys ("automation keys
  for bots and advanced traders, never able to withdraw funds")
  from MM permissions, which live exclusively on the operator-only
  MM Gateway.

## Executor / status handling

* No safe public endpoint surfaces the executor mode; the console
  shows `Unknown` with the muted sub-line "Status source
  unavailable" rather than synthesising a value.
* The HTTP reachability check uses the existing `fetchTradingHealth`
  helper against the configured `NEXT_PUBLIC_TRADING_API_BASE_URL`
  base, with an `AbortController` for unmount and a `requestId`
  guard so a late response can never overwrite a newer one.
* WebSocket reachability stays `Unknown` because the sandbox at the
  bottom is the only safe live probe (intrusive auto-connects were
  explicitly out of scope).
* The MM Gateway status pills (`Operator only`, `Off by default`)
  are surfaced from static knowledge published in
  `docs/PUBLIC_WS_API_V1.md`; the page never claims live MM status.

## WebSocket sandbox status

* Unchanged from FRONTEND-API-PAGE-V1 / FRONTEND-DOCS-SPLIT-V1
  (`src/components/api/WsQuickTest.tsx`).
* Now visually subordinate: under a small heading, at the bottom of
  the page, followed by a one-line footnote.
* Default URL still derives from
  `NEXT_PUBLIC_PUBLIC_WS_URL` → `NEXT_PUBLIC_TRADING_API_BASE_URL`
  (with `http` → `ws` scheme swap, `/ws` appended) →
  `ws://localhost:8080/ws`. Never auto-connects.

## Validations run

From `~/DEOPT/deopt-v2-frontend`:

* `npm run lint` — **PASS** (zero warnings; an early
  `react-hooks/use-memo` error from a non-inline `useMemo` first
  argument was fixed by wrapping in an arrow function).
* `npm run typecheck` — **PASS** (`tsc --noEmit` exits 0).
* `npm run build` — **PASS** (24 static + dynamic routes; `/api`
  still ships as `○ /api (Static)` — the console reads its data on
  the client at mount, so the page itself can be statically
  prerendered).
* `git diff --check` — **PASS**.

Smoke probe (`next start --port 3157`, `curl /api`):

* The whole testid surface is present:
  `developers-console`, `developers-console-header`,
  `developers-console-quicklinks` + the four
  `developers-quicklink-*` chips, the four `identity-*` cells, the
  three `environment-*` cells + `environment-current`, the two
  capability cards + their action testids, the two planned-section
  testids + their `*-status`, `*-register` / `*-create`, `*-table`
  and `*-table-empty` rows, `developers-console-utilities` with
  `utilities-quickstart` / `utilities-limitations` /
  `utilities-mint-planned`, `developers-console-mm-gateway` with
  `mm-gateway-status-operator` / `mm-gateway-status-default` /
  `mm-gateway-capabilities` / `mm-gateway-action-docs`,
  `developers-console-sandbox` + the existing
  `api-ws-quick-test-*` testids.
* Every external link target is `http://localhost:3002/...`
  (`NEXT_PUBLIC_DOCS_URL` fallback), proving the docs helper is in
  use and no production URL leaked.
* The substring "testnet beta" appears **0 times** inside the
  `developers-console` container body. The two occurrences elsewhere
  on the rendered page come from the global trading shell's testnet
  banner and the wallet network badge, both of which are outside the
  console.

Forbidden-pattern scans on `src/components/api` +
`src/app/(trading)/api`:

* Zero `amber-*|yellow-*|orange-*` Tailwind classes.
* Zero `Deribit` references; zero standalone `\bderive\b` matches.
* Zero `\baudited\b|mainnet[- ]ready|production[- ]ready|safe for real funds|\bguaranteed\b` matches.
* Zero `DATABASE_URL`, `PRIVATE_KEY`, `alchemy.com/v2`, `infura.io/v3`, `mainnet.base.org`, `Bearer …`.
* Zero hardcoded production URL (`deopt.fi`, `deopt.io`,
  `api.deopt.*`, `https://api.…`).

## Skipped validations and why

* **Playwright e2e run** — skipped in-session. The 16 rewritten
  tests in `tests/e2e/api-v1.spec.ts` are committed and runnable by
  the operator via `npm run e2e:install && npm run e2e:local`. The
  HTML smoke probe above proves the page renders all required
  testids in production mode.
* **Live `/ws` smoke against a real backend** — skipped. Milestone
  forbids broadcast / mainnet / chain interaction; the WS sandbox
  is the only place the page would intentionally touch a live
  WebSocket, and only on explicit user action.
* **End-to-end wallet-connect flow** — not exercised in-session.
  The wallet hook surface is untouched; the console reads
  `useWallet()` the same way the existing trading routes already
  do.

## Safety posture confirmation

* No secrets read or printed.
* No `.env` opened. No `.env` values printed.
* No private keys, RPC URLs, database URLs, admin bearer tokens, or
  wallet secrets introduced or exposed.
* No chain transaction sent.
* No broadcast.
* No deployment.
* No backend source touched (the only file added to the backend
  tree is this result document).
* No Solidity contracts touched.
* No mainnet anything. Mainnet remains gated by `expectedChainId()`
  and the existing `MainnetDisabledBanner`.
* MM Gateway is described **only** as operator-only / separate / off
  by default. Bots and MMs are kept as distinct roles in the page
  copy.
* No fake session keys, no fake subaccounts, no fake mint
  endpoint, no fake "live" MM data.

## Known limitations

* The executor mode is shown as `Unknown` because no safe public
  endpoint exposes it today. Once a public executor status surface
  ships, the identity-strip cell can be wired without re-shaping
  the layout.
* The public-WebSocket reachability cell stays `Unknown` for the
  same reason — the bottom sandbox is the explicit user-driven
  probe.
* `Register Session Key` and `Create Subaccount` buttons are
  disabled placeholders. They will need real handlers wired in the
  follow-on milestone that ships the backend surfaces.
* The environment badge is read from `NEXT_PUBLIC_DEOPT_ENV`. Until
  the operator sets that variable, it shows `Unknown`. The page
  treats `Unknown` like a non-Production environment for the
  Environment Utilities chip.
* No Mint UI is wired. The `Mint UI planned` chip is intentionally
  passive — the brief explicitly forbids fake mint claims.
* Header `GitHub` link still points at the existing public org
  `github.com/DeOpt`, the same URL already published in the
  hamburger drawer. No new repo info exposed.

## Current next recommendation

In order of value:

1. **Wire a wallet-auth demo into the WS sandbox** — the existing
   `WalletProvider` already supports `signTypedData`; adding an
   opt-in "Sign challenge" button under the sandbox would let any
   wallet-connected developer test `auth.challenge` /
   `auth.verify` end-to-end from the console without leaving the
   app.
2. **Surface a public executor / WS reachability status endpoint
   on the backend** — once that exists, the `Executor` cell and the
   `Public WebSocket` cell can move off `Unknown` without changing
   the console layout.
3. **Author the Session Keys + Subaccounts backend surfaces** —
   the console UI already reserves visible space for these tables;
   the only frontend follow-up is wiring real CRUD handlers behind
   the currently-disabled buttons.

None should start without an explicit milestone brief.
