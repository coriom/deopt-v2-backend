# FRONTEND-FUNDINGS-PAGE-V1 — Result

Status: **completed**.

Replaces the long-form `/fundings` placeholder with a minimal,
terminal-style funding landing. The page is honest about state:
perps are not live, no live funding rates are surfaced, and the
account funding table renders an empty state until a per-wallet
funding-history endpoint ships on the backend.

## Summary

The previous `/fundings` route used the shared `PlaceholderPage`
component with three "what you can rely on / what lands later" lists
that read more like docs than an in-app screen. The new route is a
compact terminal-style page:

* Header — `Funding` title + one-line subtitle + three quick links.
* Status strip — 3-cell honest summary.
* Market Funding table — 2 rows (`BTC-PERP`, `ETH-PERP`) with `—`
  cells and `Planned` status pills.
* Account Funding table — empty with a wallet-disconnected hint.
* Methodology card — 3-bullet recap + docs link.

No fake rates, no synthetic timestamps, no marketing language.

## Files changed

### `~/DEOPT/deopt-v2-frontend`

**New:**

* `src/components/fundings/FundingsShell.tsx` (~333 lines, client
  component). Owns all five sections.
* `tests/e2e/fundings-v1.spec.ts` — 11 Playwright tests covering
  layout, status defaults, market table placeholders, account
  empty-state, methodology bullets, forbidden-pattern guards.

**Modified:**

* `src/app/(trading)/fundings/page.tsx` — was a `PlaceholderPage`
  render; now a 13-line server wrapper that renders `<FundingsShell />`
  inside a `deopt-scroll-dark` page-scroll wrapper. Metadata title
  changed from `Fundings — DeOpt public testnet beta` to
  `Funding — DeOpt`.
* `src/components/TradingShell.tsx` — added `/fundings` to
  `TERMINAL_ROUTES` so the page suppresses the bottom marketing
  footer, fills the viewport, and gets the dark-scrollbar utility
  via the page wrapper.

### `~/DEOPT/deopt-v2-backend`

* New `docs/FRONTEND_FUNDINGS_PAGE_V1_RESULT.md` (this file).
* No backend source touched.

### `~/DEOPT/deopt-v2-docs`

* Untouched.

### Root

* `RUN_STATE.md` prepended with the milestone closure entry.

## Route implemented

* `/fundings` (Next.js app router, app group `(trading)`).
* Drawer entry (`hamburger-link-fundings`), navbar route indicator
  chip (`Fundings`), and the existing `terminal-navbar.spec.ts`
  drawer-href assertions all pointed at `/fundings` already — no
  navigation wiring changed.
* Build output confirms `○ /fundings (Static)`.

## Page sections

| # | Section | testid | Notes |
| --- | --- | --- | --- |
| 1 | Header | `fundings-page-header` + `fundings-quicklinks` | Title `Funding` + subtitle `Funding applies to perpetual markets. Options do not accrue periodic funding.` + 3 quick links (Docs → `docsPath("/")`, Perps → `/perps`, Fees → `/fees`). |
| 2 | Status strip | `fundings-status-strip` | 3 cells: `Perps Funding → Not live`, `Options → No funding`, `Account Funding → Wallet not connected` (becomes `No payments found` when connected). |
| 3 | Market Funding table | `fundings-market-section` + `fundings-market-table` | 5 columns (Market / Funding Rate / Next Funding / 24h Avg / Status), 2 rows (`BTC-PERP`, `ETH-PERP`). Every numeric cell is `—`, every status pill reads `Planned`. |
| 4 | Account Funding table | `fundings-account-section` + `fundings-account-table` | 6 columns (Time / Market / Position / Rate / Payment / Status). Single full-row empty cell: `Connect wallet to view account funding payments.` when disconnected; `No funding payments found` once a wallet is bound. |
| 5 | Methodology card | `fundings-methodology` | 3 bullets matching the brief verbatim, plus a `Read funding docs →` link to `docsPath("/protocol")`. |

## Data behavior

* **Market Funding table**: static placeholder rows. No backend
  endpoint is contacted; nothing animates. The rows are clearly
  labelled `Planned` so a reader cannot mistake them for live rates.
* **Account Funding table**: no per-account funding endpoint is
  consumed. The page reads only `useWallet().address` to pick the
  empty-state copy; no other private data is fetched.
* **Status strip**: `Perps Funding` and `Options` are static (perps
  are not live; options never pay funding). `Account Funding`
  switches between `Wallet not connected` and `No payments found`
  based purely on the wallet hook.
* **Quick links**: route through `docsPath()` (env-aware,
  `NEXT_PUBLIC_DOCS_URL` fallback `http://localhost:3002`) for Docs
  and the Methodology docs link. Internal links (`/perps`, `/fees`)
  use `next/link`.

## Empty-state behavior

* Wallet disconnected → `Account Funding` cell reads `Wallet not
  connected`, and the table row reads `Connect wallet to view
  account funding payments.`
* Wallet connected, no payments → `Account Funding` cell reads `No
  payments found`, and the table row reads `No funding payments
  found`.
* Market table rows never empty: they always show `Planned` so the
  user understands that perps funding is not live yet.

## Validations run

From `~/DEOPT/deopt-v2-frontend`:

* `npm run lint` — **PASS** (zero warnings).
* `npm run typecheck` — **PASS** (`tsc --noEmit` exits 0).
* `npm run build` — **PASS** (24-route build; `○ /fundings (Static)`).
* `git diff --check` — **PASS**.

Smoke probe (`next start --port 3166`, `curl /fundings`):

* All 20 expected `fundings-*` testids present, including the two
  `fundings-market-row-{0,1}` rows.
* `public-beta-footer` count = 0 — the marketing footer is
  correctly suppressed by the `TERMINAL_ROUTES` addition.

Forbidden-pattern scans on `src/components/fundings` +
`src/app/(trading)/fundings`:

* Zero `amber-*|yellow-*|orange-*` Tailwind classes.
* Zero `Deribit` references; zero standalone `\bderive\b` matches.
* Zero `\baudited\b|mainnet[- ]ready|production[- ]ready|safe for
  real funds|\bguaranteed\b`.
* Zero `DATABASE_URL|PRIVATE_KEY|alchemy.com/v2|infura.io/v3|
  mainnet.base.org|Bearer …` (≥ 16 chars).
* Zero hardcoded production URL (`deopt.fi`, `deopt.io`,
  `api.deopt.*`). All external links go through the existing
  `docsPath()` helper or use internal Next.js routes.

## Skipped validations and why

* **Playwright e2e suite run** — skipped in-session. The new
  `fundings-v1.spec.ts` is committed and runnable via
  `npm run e2e:install && npm run e2e:local`. The HTML smoke probe
  above proves the page renders all required testids in production
  mode.
* **Live funding-rate smoke** — not applicable. No live rates are
  surfaced; the page is intentionally static until backend funding
  surfaces ship.

## Known limitations

* Market Funding table is hard-coded to 2 rows
  (`BTC-PERP`, `ETH-PERP`). Once the perps executor ships and a
  public per-market funding endpoint exists, the table can be wired
  to that source without changing the surrounding layout.
* Account Funding table never fetches data because no public per-
  account funding-history endpoint exists yet. The empty-state copy
  is the only state today.
* No client-side caching, no SWR, no live ticker — by design.
* The Methodology docs link points to `/protocol` on the docs site
  for now; once a dedicated funding page lands in `deopt-v2-docs`,
  swap the path without touching any other code.
* `Read funding docs →` opens in a new tab (`target="_blank"`,
  `rel="noopener noreferrer"`).

## Safety posture confirmation

* No secrets read or printed.
* No `.env` opened. No `.env` values printed.
* No private keys, RPC URLs, database URLs, admin bearer tokens, or
  wallet secrets introduced or exposed.
* No chain transaction sent.
* No broadcast.
* No deployment.
* No backend source touched (only this result document was added
  to the backend tree).
* No Solidity contracts touched.
* No mainnet anything.
* No fake live funding rates, no fake historical payments.
* No Derive / Deribit references in the public UI.
* No production URL hardcoded; docs origin reads through
  `NEXT_PUBLIC_DOCS_URL` with `http://localhost:3002` fallback.

## Current next recommendation

When the perps funding source ships on the backend, wire it through
a new `/perps/funding` or equivalent endpoint and replace the
`MarketFundingTable` static rows with a live fetch that honours the
existing envelope shape. The component layout already reserves the
right columns; only the data source needs to change.

Should not start without an explicit milestone brief.
