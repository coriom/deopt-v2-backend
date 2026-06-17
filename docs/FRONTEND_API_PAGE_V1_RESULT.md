# FRONTEND-API-PAGE-V1 — Result

Status: **completed**.

Replaces the 117-line `/api` placeholder with a dense, terminal-style
developer reference page that documents the real public testnet beta
API surface as it exists today (after BACKEND-PUBLIC-WS-API-V1 and
BACKEND-PUBLIC-WS-AUTH-V1).

## Summary

The new `/api` page is now an actionable developer reference, not a
placeholder. It documents in order:

1. **Public HTTP / REST API** — endpoint table, envelope shape, error
   envelope, common error codes.
2. **Public WebSocket API** (`GET /ws`) — JSON-RPC methods table, wire
   examples (request / ack / push), live public channels, deferred
   channels (no fabricated data).
3. **Wallet authentication** — full EIP-191 personal-sign flow,
   `auth.challenge` + `auth.verify` request/response examples, exact
   canonical-message bytes, address-bound session warning.
4. **Private account streams** — 4 live + 5 honest-empty channels,
   address-bound, `AUTH_ADDRESS_MISMATCH` rules.
5. **Signed intent trading flow** — explicit 6-step pseudo-flow,
   reminders that the backend never signs and that intent creation
   stays on HTTP.
6. **MM Gateway** — clearly described as **separate** from the public
   API: WebTransport over QUIC / HTTP3, default listener `:8443`,
   TLS-required, off-by-default, permission-gated, **not public**.
7. **User profiles** — three-row comparison table (public human /
   advanced trader-bot / operator-whitelisted MM) so the bot profile
   is **not collapsed** into the MM profile.
8. **Copyable code examples** — `curl`, browser WebSocket subscribe,
   browser wallet-auth pseudo-code, private subscribe.
9. **Live WebSocket sandbox** — a small, browser-only Quick Test panel
   with configurable URL, connect / disconnect / ping / subscribe
   buttons for the three live public channels, and an in-page log of
   inbound / outbound frames.

## Frontend files changed

**New:**

* `src/components/api/ApiShell.tsx` — main page shell (server
  component, ~1.2k lines of static layout + tables + examples).
* `src/components/api/CodeBlock.tsx` — small client component for
  copyable code blocks with a transient "Copied" pill.
* `src/components/api/WsQuickTest.tsx` — optional client-only live
  WebSocket sandbox panel.
* `tests/e2e/api-v1.spec.ts` — Playwright coverage for the new shell
  (sections present, code examples copyable, MM Gateway described as
  separate, no forbidden palette/marketing/secret patterns, no
  Deribit/Derive, no bottom marketing footer).

**Modified:**

* `src/app/(trading)/api/page.tsx` — now a thin server wrapper that
  renders `<ApiShell />` and sets metadata.
* `tests/e2e/fees-and-api-placeholders.spec.ts` — dropped the obsolete
  `/api placeholder` assertions; rewired the hamburger → API test to
  look for `api-shell`. `/fees` placeholder coverage is unchanged.

No other frontend files were touched. No backend source was touched.
No Solidity touched. No scripts touched. No `.env` read. No mainnet.
No broadcast. No deployment.

## Backend docs inspected

Read-only inspection of:

* `~/DEOPT/deopt-v2-backend/docs/PUBLIC_WS_API_V1.md`
* `~/DEOPT/deopt-v2-backend/docs/BACKEND_PUBLIC_WS_API_V1_RESULT.md`
  (referenced for source-availability matrix)
* `~/DEOPT/deopt-v2-backend/docs/BACKEND_PUBLIC_WS_AUTH_V1_RESULT.md`
* `~/DEOPT/deopt-v2-backend/docs/openapi/trading-api.openapi.json`
  (existence confirmed; not embedded — see OpenAPI handling decision).
* `~/DEOPT/deopt-v2-backend/src/mm/transport/webtransport.rs` (header
  only, to confirm the gateway code path remains separate and the
  default listener stays `:8443`).

No backend source was modified.

## Route implemented

* `/api` (Next.js app router, app group `(trading)`). The route was
  already in:
  * `src/components/HamburgerMenu.tsx` (drawer item).
  * `src/components/NavbarRouteIndicator.tsx` (drawer-only chip).
  * `src/components/TradingShell.tsx` `TERMINAL_ROUTES` (so the
    bottom marketing footer is suppressed and the page can use the
    full viewport height).

No navigation files were touched — wiring was already in place.

## API surfaces documented

All of the following are explicitly enumerated on the page:

* HTTP endpoints (15 rows): `/trading/health`, the seven `/options/...`
  routes, `/options/execution-intents` (marked as **wallet signature
  flow**), the five `/accounts/{address}/...` routes, `/leaderboard`.
* Public WebSocket methods (7 client → server + 1 server → client).
* Public WebSocket channels (3 live + 5 honest deferred — no fake
  orderbook / trades / ticker / mark / oracle claims).
* Private account channels (4 live + 5 reserved-empty), with
  address-bound session rules and `AUTH_ADDRESS_MISMATCH` semantics.
* Common error codes (8): `INVALID_ADDRESS`, `INVALID_REQUEST`,
  `AUTH_REQUIRED`, `AUTH_EXPIRED`, `AUTH_INVALID_SIGNATURE`,
  `AUTH_ADDRESS_MISMATCH`, `SOURCE_UNAVAILABLE`, `INTERNAL_ERROR`.
* Canonical signing message (exact byte-for-byte template).
* MM Gateway — described as separate WebTransport / QUIC / HTTP3 on
  listener `:8443`, TLS required, off by default, permission-gated,
  not exposed publicly.

## Code examples included

All copyable via the `CodeBlock` component:

* `curl https://<deopt-api-host>/trading/health`
* Browser `new WebSocket(...)` + subscribe.
* Browser `window.ethereum.personal_sign` wallet-auth pseudo-code.
* Private account subscribe after auth.
* Response envelope (200 OK) + error envelope.
* WebSocket client request, server ack, server push.
* `auth.challenge` request + response, `auth.verify` request +
  response.
* Trading-flow pseudo-code (6 steps).
* Canonical EIP-191 message template.

No production URL is hardcoded. Examples uniformly use the
placeholder `<deopt-api-host>`; local dev `http://localhost:8080` is
mentioned once as the default backend listener.

## Optional live demo status

**Shipped** as a small client-side `WsQuickTest` panel:

* Configurable WebSocket URL input. Default is derived in this order:
  `NEXT_PUBLIC_PUBLIC_WS_URL` → `NEXT_PUBLIC_TRADING_API_BASE_URL`
  (with `http` → `ws` scheme swap, `/ws` path appended) →
  `ws://localhost:8080/ws`.
* Buttons: **Connect**, **Disconnect**, **ping**, three **sub** buttons
  for `trading.health`, `options.products`, `leaderboard`, **Clear**.
* Connection status pill ( `idle` · `connecting` · `open` · `closing`
  · `closed` · `error` ).
* Inbound / outbound frame log (capped at 200 lines to keep the DOM
  cheap; outbound frames prefixed with `→`, inbound with `←`,
  informational with `·`).
* No wallet signing live flow. The panel never reads or exposes
  secrets, never calls any external API, never writes to localStorage.
* Degrades gracefully when the backend is offline: the connection
  attempt reports `error`, the panel stays usable, and the log shows
  the failure.

## OpenAPI handling decision

The backend already ships
`docs/openapi/trading-api.openapi.json`. This milestone deliberately
**does not** embed or render the spec inside the frontend because:

1. The page is meant to be a high-signal terminal-style summary, not
   a Swagger UI clone.
2. Embedding the JSON would bloat the static page asset and force a
   re-deploy on every spec change.
3. The endpoint table on the page already covers the same surface in
   a more readable form.

A follow-on milestone could add a "View OpenAPI" link that downloads
the JSON from a backend-served path (`/openapi.json` is **not**
currently exposed by the backend — out of scope here, no backend
work in this milestone).

## Known limitations

* The WS Quick Test panel only exercises **public** channels. Wiring
  the wallet-auth flow into the live panel would re-use the existing
  `WalletProvider`, but doing so safely (no leaked addresses, no
  unintended `personal_sign` prompts) is a follow-on milestone.
* The `account.orders`, `account.fills`, `account.intent_status`,
  `account.settlements`, `account.liquidations` channels are
  documented as **reserved-empty**; once the backend ships real
  sources, the table will need a one-line status update.
* No OpenAPI render — see decision above.
* The page intentionally does not list admin endpoints. They are
  explicitly called out as **not part of the public API** and the
  frontend never holds an admin bearer.
* No service-worker / no caching layer added — the page is statically
  prerendered (Next.js marks it `○ /api (Static)`).
* No backend WS URL discovery endpoint is called — the default URL is
  derived from `NEXT_PUBLIC_*` env at build time, and the user can
  edit it freely in the input.

## Validations run

From `~/DEOPT/deopt-v2-frontend`:

* `npm run lint` — **PASS** (zero errors, zero warnings).
* `npm run typecheck` — **PASS** (`tsc --noEmit` exits 0).
* `npm run build` — **PASS** (Next.js 16.1.6 Turbopack; `/api` ships
  as `○ /api (Static)`; 24 static pages built clean).
* `git diff --check` — **PASS** (no trailing whitespace or conflict
  markers).

From `~/DEOPT/deopt-v2-backend`:

* No backend changes; no tests run.

Smoke validation (server start + HTML probe, no Playwright run):

* Started `next start --port 3147` after `npm run build`.
* `curl http://localhost:3147/api` returned a 163 KB static HTML
  document.
* All required `data-testid` markers present:
  * `api-shell`, `api-hero`, `api-hero-chips`, `api-architecture`
    (4 arch cards), `api-http` (endpoint table, both envelopes,
    error codes list), `api-ws` (method table, three wire examples,
    both channel tables), `api-auth` (flow, four wire examples,
    canonical message, warning), `api-private-channels` (table with
    9 rows), `api-intents` (flow code block), `api-mm-gateway`
    (explicit block), `api-profiles` (3 profile rows),
    `api-examples` (4 copyable examples), `api-quick-test`
    (connect / disconnect / ping / sub-trading.health /
    sub-options.products / sub-leaderboard / clear / status pill /
    log / URL input).

Forbidden-pattern scans (against `src/components/api/` and
`src/app/(trading)/api/`):

* No `Deribit` or `Derive` references in source.
* No `\baudited\b` (only `Unaudited` in the safety microcopy, which
  the negative-claim test correctly does not match).
* No `mainnet[- ]ready`, `production[- ]ready`,
  `safe for real funds`, `\bguaranteed\b`.
* No `Bearer\s+[A-Za-z0-9_.-]{16,}`, `alchemy.com/v2/`,
  `infura.io/v3/`, `DATABASE_URL`, `/admin/`, `mainnet.base.org`.
* No `amber-*`, `yellow-*`, `orange-*` Tailwind classes.
* WebTransport mentions only describe the **MM** gateway as separate
  and not public.
* No production URL hardcoded — examples use `<deopt-api-host>` and
  local dev `localhost:8080`.

## Skipped validations and why

* **Playwright e2e run** — skipped in this session because no dev
  server was pre-started and a full `next start` boot + suite run was
  out of scope for a doc-heavy frontend milestone. The new
  `tests/e2e/api-v1.spec.ts` is committed and runnable by the
  operator via `npm run e2e:install && npm run e2e:local`. The HTML
  smoke probe above proves the page renders all required `testid`
  markers in production mode.
* **Smoke against a real backend `/ws`** — skipped because the
  milestone forbids broadcasting / mainnet / chain interaction and
  there is no committed local backend boot procedure in this
  session's repo state. The WS Quick Test panel is designed to fail
  gracefully against an offline backend.

## Safety posture confirmation

* No secrets read or printed.
* No `.env` contents read or printed.
* No private keys, RPC URLs, database URLs, admin bearer tokens, or
  wallet secrets introduced or exposed.
* No chain transaction sent.
* No broadcast.
* No deployment.
* No backend source modified.
* No Solidity contracts touched.
* No mainnet anything.
* MM WebTransport gateway remains **separate** from the public API
  and is **not** described as a public WebSocket. The page explicitly
  states that the public API does not expose WebTransport and that
  the MM Gateway is not a public WebSocket API.
* Bot users and MMs are kept as **distinct** profiles in the User
  Profiles table — they are not collapsed.

## Current next recommendation

In order of value:

1. **Wire the WS Quick Test panel to the wallet** — add an opt-in
   "Sign challenge with connected wallet" button so the public
   developer page becomes a copyable wallet-auth demo. Re-use the
   existing `WalletProvider`. Keep it explicitly opt-in so the page
   stays safe to share publicly.
2. **Upgrade `account.orders` from honest-empty to a real source**
   once the backend ships the underlying table — the wire shape on
   the WS server is already reserved, so the `/api` table will just
   need its status pill flipped from **Reserved** to **Live**.
3. **Surface the OpenAPI spec** — once the backend serves
   `/openapi.json` over HTTP, add a `View OpenAPI` link on the HTTP
   API section and fetch it client-side from the configured base
   URL. No frontend bundle bloat.

None of the above should start without an explicit milestone brief.
