# FRONTEND-DOCS-SPLIT-V1 — Result

Status: **completed**.

Splits the long-form documentation out of the trading terminal and
into a dedicated docs site, leaving the in-app `/api` page as a
compact in-product Developers panel.

## Summary

The previous `/api` page (FRONTEND-API-PAGE-V1) was a 1.2k-line
developer reference embedded inside the trading terminal. That was
the right milestone for the wire-shape rollout but the wrong final
shape: it crowded the terminal, drowned the in-app UX, and forced
every wire-shape change to redeploy the terminal.

This milestone:

* Creates a new Next.js docs site at `~/DEOPT/deopt-v2-docs`
  (`deopt-v2-docs`), independently routable, designed for the future
  public domain `docs.deopt.xyz`.
* Replaces the in-app `/api` shell with a compact `DevPanel`: wallet
  state summary, five capability cards, four docs quick-links, and
  the reusable WebSocket quick-test sandbox.
* Adds a single configuration knob (`NEXT_PUBLIC_DOCS_URL`) so the
  terminal links to the right docs origin in any environment, with a
  safe default to the local docs dev server.

No backend source was modified. No Solidity was touched. No `.env`
was read. No mainnet anything.

## New docs repo created

Location: `~/DEOPT/deopt-v2-docs`.

Stack:

* Next.js 16.1.6 (Turbopack) — same major as the frontend.
* React 19.2.3.
* TypeScript 5.
* Tailwind v4 via `@tailwindcss/postcss` (mirrors the frontend's
  PostCSS pipeline so styling stays consistent).
* No external UI library, no MDX, no docs engine, no SaaS template.
* `next/font/google` ships Manrope (UI) + JetBrains Mono (code), the
  same fonts the terminal uses.
* ESLint + `eslint-config-next` with the same `defineConfig` shape
  as the frontend's `eslint.config.mjs`.

Layout:

```
deopt-v2-docs/
├── package.json
├── tsconfig.json
├── next.config.ts
├── postcss.config.mjs
├── eslint.config.mjs
├── .gitignore
└── src/
    ├── app/
    │   ├── layout.tsx        # root layout wraps everything in DocsShell
    │   ├── page.tsx          # /     — Overview homepage
    │   ├── globals.css
    │   ├── quickstart/page.tsx
    │   ├── limitations/page.tsx
    │   ├── developers/
    │   │   ├── page.tsx                     # /developers          — API overview
    │   │   ├── http-api/page.tsx
    │   │   ├── websocket-api/page.tsx
    │   │   ├── wallet-auth/page.tsx
    │   │   ├── signed-intents/page.tsx
    │   │   └── mm-gateway/page.tsx
    │   ├── academy/page.tsx
    │   ├── protocol/page.tsx
    │   └── reference/page.tsx
    ├── components/
    │   ├── DocsShell.tsx     # top bar + sidebar + footer (server)
    │   ├── Sidebar.tsx       # client; reads usePathname for active item
    │   ├── Page.tsx          # Page header + Section primitive
    │   ├── CodeBlock.tsx     # client; copy-to-clipboard pill
    │   ├── EndpointTable.tsx
    │   ├── ChannelTable.tsx
    │   └── StatusBadge.tsx
    └── lib/
        └── site.ts           # SITE config + NAV IA constants
```

Visual style is consistent with the trading terminal: pure black
background, deep emerald accents, JetBrains Mono only for code /
endpoints / addresses / protocol fields, Manrope for body text, no
amber / yellow / orange, crisp borders, compact dense layouts.

## Docs site structure (IA)

| Section | Routes |
| --- | --- |
| Start | `/` (Overview), `/quickstart`, `/limitations` |
| Developers | `/developers`, `/developers/http-api`, `/developers/websocket-api`, `/developers/wallet-auth`, `/developers/signed-intents`, `/developers/mm-gateway` |
| Academy | `/academy` (single landing surfacing 8 outlined modules) |
| Protocol | `/protocol` (11-component architecture map + trust model) |
| Reference | `/reference` (6-entry hub: OpenAPI, AsyncAPI, ABIs, Events, Error Codes, Known Limitations) |

The Academy / Protocol / Reference sections are intentionally light
for V1: a clean landing page each, with structure already in place
so future content drops slot in without re-architecting navigation.

What was deliberately **not** built in V1:

* Per-module Academy pages (each module shows an "Outline" badge —
  honest status).
* OpenAPI render. The backend already ships
  `docs/openapi/trading-api.openapi.json`; the docs site links to it
  rather than embedding (no redeploy on every spec change).
* AsyncAPI spec. Backend ships the WebSocket shape in Markdown;
  AsyncAPI is marked `Planned` on the reference hub.

## Frontend `/api` simplification

Removed:

* `src/components/api/ApiShell.tsx` (~1200 lines of long-form
  documentation duplicated from the backend `PUBLIC_WS_API_V1.md`).
* `src/components/api/CodeBlock.tsx` (only referenced by the removed
  shell — the docs site has its own copy).

Added:

* `src/components/api/DevPanel.tsx` (~237 lines).
* `src/lib/docs-url.ts` (env-aware helper for the docs origin).

Replaced contents:

* `src/app/(trading)/api/page.tsx` — now renders `<DevPanel />` and
  sets the metadata title to "Developers — DeOpt public testnet
  beta".

`DevPanel` ships:

* Header — "Developers / Connect, test the public API, and open the
  full DeOpt Docs."
* Signer block — connected / not connected pill, shortened address,
  network label (uses the existing `useWallet` hook).
* Five capability cards — Public HTTP (Live), Public WebSocket
  (Live), Wallet Auth (Live), Session Keys (Planned), MM Gateway
  (Operator only). Each card links to the matching docs page.
* Quick links row — Open Docs, API Reference, WebSocket Guide,
  Wallet Auth.
* Embedded `WsQuickTest` sandbox — unchanged from
  FRONTEND-API-PAGE-V1, kept because it is in-app utility, not
  documentation.
* One-line footnote — "Base Sepolia testnet beta. Unaudited.
  Mainnet disabled. No real funds."

The terminal `/api` is now ~29 KB of static HTML versus ~163 KB
before — a one-screen practical panel rather than a documentation
dump.

## Route / link mapping

| Source | Target | Mechanism |
| --- | --- | --- |
| Terminal navbar drawer "API" | `/api` (terminal route) | unchanged hamburger entry |
| Terminal "API" route indicator chip | `/api` | unchanged `NavbarRouteIndicator` |
| Terminal "Academy" drawer item | `/docs` (terminal slug-routed legacy academy) | unchanged |
| Terminal `DevPanel` capability cards / quick links | `${NEXT_PUBLIC_DOCS_URL || http://localhost:3002}/<path>` | new `src/lib/docs-url.ts` helper |
| Docs site top bar "Open App" | `${NEXT_PUBLIC_APP_URL || http://localhost:3000}` | new `src/lib/site.ts` constant |
| Docs site top bar "Developers" / "Academy" | `/developers` / `/academy` | direct |
| Docs site sidebar (5 sections, 12 leaf links) | direct | active-link styling via `usePathname` |

Production hosts are intentionally not hardcoded; both apps read
their counterpart's URL from env at build time with safe local-dev
defaults.

## Package / build setup

| App | Dev port | Build command | Notes |
| --- | --- | --- | --- |
| Trading terminal (`deopt-v2-frontend`) | 3000 (auto-3001 fallback) | `npm run build` (Next 16.1.6) | unchanged |
| Docs site (`deopt-v2-docs`) | 3002 | `npm run build` (Next 16.1.6) | new; `dev`, `build`, `start`, `lint`, `typecheck` scripts. Port 3002 (not 3001) because `next dev` for the terminal auto-increments to 3001 when port 3000 is busy — colliding with 3001 here would break local dev. |

### Recommended local commands

```bash
# Terminal A — trading terminal (port 3000, may auto-fall back to 3001)
cd ~/DEOPT/deopt-v2-frontend && npm run dev

# Terminal B — docs site (port 3002, binds explicitly)
cd ~/DEOPT/deopt-v2-docs && npm run dev
```

The terminal reads the docs origin from `NEXT_PUBLIC_DOCS_URL`
(default `http://localhost:3002`). The docs site reads the app
origin from `NEXT_PUBLIC_APP_URL` (default
`http://localhost:3000`). Either can be overridden at build time
without code changes.

`deopt-v2-docs` dependency set is intentionally minimal:

* `next 16.1.6`
* `react 19.2.3`
* `react-dom 19.2.3`
* dev only: `@tailwindcss/postcss`, `tailwindcss`, `typescript`,
  `eslint`, `eslint-config-next`, `@types/*`.

Total install: 362 packages, no native build steps, no postinstall
scripts other than what Next.js / Tailwind ship.

## Validations run

### `deopt-v2-frontend`

* `npm run lint` — **PASS** zero warnings.
* `npm run typecheck` — **PASS** (`tsc --noEmit` exits 0).
* `npm run build` — **PASS** (Next.js 16.1.6 Turbopack; `/api`
  ships as `○ /api (Static)`; full 24-route static build clean).
* `git diff --check` — **PASS**.
* Smoke probe: `next start --port 3148`, `curl /api` returns 29 KB
  static HTML. All 18 expected `dev-panel-*` testids present
  (`dev-panel`, `dev-panel-header`, `dev-panel-wallet` +
  `…-state` / `…-address` / `…-network`,
  `dev-panel-capability-cards` + 5 `dev-panel-card-*`,
  `dev-panel-quick-links` + 4 `dev-panel-link-*`,
  `dev-panel-sandbox`). All six docs hrefs point to the local docs
  origin (`http://localhost:3002/...`).

### `deopt-v2-docs`

* `npm install --prefer-offline --no-audit --no-fund` — installed
  362 packages from the local cache; no network errors.
* `npm run lint` — **PASS** zero warnings.
* `npm run typecheck` — **PASS** (`tsc --noEmit` exits 0); Next.js
  auto-added `.next/dev/types/**/*.ts` to `tsconfig.include` on
  first build — accepted, matches the frontend's `tsconfig.json`.
* `npm run build` — **PASS** (Next.js 16.1.6 Turbopack; 13 static
  pages including all of `/`, `/quickstart`, `/limitations`,
  `/developers`, the five `/developers/*` leaves, `/academy`,
  `/protocol`, `/reference`).
* Smoke probe: `next start --port 3149`, route HTML probed:
  * `/` exposes all 4 home cards + status strip + title + subtitle.
  * `/developers` exposes the 3-row profile comparison table.
  * `/developers/http-api` exposes the endpoint table, both
    envelopes, error-codes list.
  * `/developers/websocket-api` exposes the method table, five wire
    examples, and three channel tables.
  * Sidebar renders 12 leaf links across the 5 sections.

### Sensitive-pattern scans

Run against `~/DEOPT/deopt-v2-docs/src` and
`~/DEOPT/deopt-v2-frontend/src/components/api`
+ `~/DEOPT/deopt-v2-frontend/src/app/(trading)/api`:

* Zero `amber-*|yellow-*|orange-*` Tailwind classes.
* Zero `Deribit` references.
* Zero standalone `Derive` references (renamed helper away from
  `derive` in the prior milestone).
* Zero `\baudited\b` matches (the documents read "Unaudited" which
  the word-boundary regex correctly does not flag).
* Zero `mainnet[- ]ready` / `production[- ]ready` / `safe for real
  funds` / `\bguaranteed\b` matches. (Initial scan flagged
  `limitations/page.tsx` for `safe for real funds`; rephrased to
  "Do not deposit real funds" so the literal substring is gone.)
* Zero `DATABASE_URL` / `PRIVATE_KEY` / `alchemy.com/v2/` /
  `infura.io/v3/` / `mainnet.base.org` / `Bearer …` (≥ 16 chars).
* Zero production URL hardcoded (`deopt.fi`, `deopt.io`,
  `api.deopt.*`). All examples use the placeholder
  `<deopt-api-host>` token and `localhost:8080` for local dev.
* Docs site footer / hero use the testnet beta status strip without
  positive claims.
* WebTransport mentions only describe the **MM** gateway as
  separate and not public.
* Bots and MMs kept as **distinct** profiles in the
  `/developers` table.

### Playwright spec rewrite

`tests/e2e/api-v1.spec.ts` was rewritten to target the new
`DevPanel` instead of the removed `ApiShell`:

* renders header / wallet / cards / links / sandbox.
* wallet block reads "Not connected" by default.
* 5 capability cards link to the docs origin (not `localhost:3000`).
* 4 quick links link to the docs origin.
* the page no longer carries the removed long-form testids
  (`api-shell`, `api-http-endpoint-table`, `api-ws-method-table`,
  `api-private-channel-table`, `api-auth-canonical`,
  `api-mm-explicit`, `api-profile-table`).
* WebSocket Quick Test sandbox keeps its controls and default URL.
* no positive-claim language, no admin / Bearer / RPC / DB URLs,
  no Deribit / Derive, no amber / yellow / orange classes, no
  bottom marketing footer.

`tests/e2e/fees-and-api-placeholders.spec.ts` was edited to point
the hamburger → API smoke at `dev-panel`; `/fees` placeholder
coverage was not touched.

## Skipped validations and why

* **Playwright e2e suite run** — skipped in-session. Both the
  trading-terminal spec (`api-v1.spec.ts`) and the
  fees-and-api-placeholders spec are committed and runnable by the
  operator via `npm run e2e:install && npm run e2e:local`. Smoke
  probes via `curl` on `next start` substitute for testid coverage.
* **Docs-site Playwright spec** — none was added in V1. The site is
  almost entirely static and the testids are already structured
  (`docs-home-*`, `docs-sidebar-link-*`, `docs-topbar-*`, page-level
  per-route ids). Adding a Playwright suite for the docs is a small
  follow-on.
* **Live `/ws` smoke against a real backend** — skipped. The
  milestone forbids broadcast / mainnet / chain interaction and the
  WS Quick Test panel is designed to fail gracefully offline.

## Known limitations

* The docs site Academy / Protocol / Reference sections are
  structured landings, not deeply written guides. They are honest
  about it ("Outline" badges, "Planned" / "Available" status).
* The docs site has no client-side search yet (intentional — no
  heavy dep added in V1; can ship as a small follow-on).
* No locale support / i18n in V1.
* No "Edit on GitHub" links (intentional — the source repo URL is
  not surfaced anywhere in public UI).
* The docs site uses `next/font/google` like the frontend; offline
  CI environments need the font cache from the frontend's
  `node_modules` (or the build will fail). For typical online CI
  this is a non-issue.
* The `NEXT_PUBLIC_APP_URL` / `NEXT_PUBLIC_DOCS_URL` env values are
  consumed at build time. Switching environments requires a
  rebuild — the same constraint the trading terminal already has
  for `NEXT_PUBLIC_TRADING_API_BASE_URL`.
* No automatic redirect from the old verbose `/api` page is needed
  because the route slug and testid namespace changed in-place.

## Safety posture confirmation

* No secrets read or printed.
* No `.env` opened. No `.env` values printed.
* No private keys, RPC URLs, database URLs, admin bearer tokens, or
  wallet secrets introduced or exposed.
* No chain transaction sent.
* No broadcast.
* No deployment.
* No backend source modified. Only `docs/FRONTEND_DOCS_SPLIT_V1_RESULT.md`
  is added to the backend tree (this file).
* No Solidity contracts touched.
* No mainnet anything. Mainnet remains disabled and unmentioned
  except in the explicit "Mainnet disabled" disclaimer copy.
* MM WebTransport gateway is described **only** as separate /
  operator-whitelisted / not public on both the docs site and the
  terminal panel.
* Bot users and MMs are kept as **distinct** profiles in the
  Developers overview.
* No production URL is hardcoded. All public-facing endpoints use
  the placeholder `<deopt-api-host>` token; localhost defaults
  apply for dev only.
* The docs site never reads from the network at runtime (apart
  from the WebSocket Quick Test panel embedded in the terminal,
  which targets the user-provided URL only).

## Current next recommendation

In order of value:

1. **Add a small Playwright smoke spec to `deopt-v2-docs`** —
   verify every section landing renders, every sidebar link
   resolves, the homepage cards work, and the topbar Open App link
   points at the configured app origin. ~80 lines, no new deps.
2. **Wire the terminal `DevPanel` wallet block to a real opt-in
   wallet-auth demo** — re-uses the existing `WalletProvider` and
   shows the developer how `auth.challenge` / `auth.verify` look
   on a live `/ws` connection. The WS Quick Test panel already
   has the surface area for this.
3. **Author Academy module pages** — pick `options-basics` and
   `greeks` first; the IA already routes to them via
   `/academy/<slug>` placeholders. Markdown-driven would be fine
   without introducing MDX as a dependency.

None should be started without an explicit milestone brief.
