# DeOpt V2 — Trading Interface Requirements

**Date:** 2026-06-10
**Posture:** docs-only requirements. **No source code modified.** Companion
to `PRODUCT_GAP_ANALYSIS_SOL_BACKEND_FRONTEND.md §4`.

## 1. Scope

The MVP trading interface for the DeOpt V2 testnet beta. Read + write
surface; user-anchored EIP-712 signature flow; no admin-token reuse.

## 2. Functional requirements

### 2.1 Connect wallet

- Detect injected provider (EIP-1193 standard).
- Allow user to connect via injected wallet (MetaMask, Rabby, etc.).
- Expose `address` + `chainId` to React context.
- Refuse interaction (Trade button disabled) when `chainId != ENV_CHAIN_ID`.
- Show network banner persistently while mismatched.
- "Switch network" CTA invokes `wallet_switchEthereumChain`.
- On disconnect: clear context; re-display landing.

### 2.2 Market selector

- Fetch enumerable product list from `GET /options/products` (M-P2 endpoint).
- Group by underlying (e.g. ETH, BTC).
- Per underlying: expose call/put toggle + expiry selector + strike grid.
- Each cell links to the trade ticket route.

### 2.3 Option chain

- For a given `(underlying, expiry, call|put)`: render strike rows × side columns (bid / ask / mark / IV / open interest / volume).
- Mark price = oracle-derived theoretical (delegated to backend `GET /quotes/preview`).
- Live refresh via polling (15s default; SWR / react-query refetchInterval acceptable for MVP).
- Click row → trade ticket route for that series.

### 2.4 Trade ticket

- Side toggle (buy / sell).
- Size input (number; min/max enforced by backend `/quotes/preview` response).
- Price input (limit price; optional for RFQ where backend returns a quote).
- Slippage tolerance input (advanced expander).
- Preview block: fee, oracle mark, IM impact, free-collateral impact, total cost.
- "Sign" button → fetches `/options/execution-intents/:id/signing-payload`; submits via wallet.
- "Broadcast" button → calls `/options/execution-intents/:id/broadcast`.
- Pending-tx state → links to `/tx/:intent_id`.

### 2.5 Positions table

- Fetch from `GET /accounts/:address/positions` (M-P2 endpoint).
- Columns: series id, side, size, mark price, unrealised PnL, IM, MM, last-updated.
- Row action: "Exercise" (if at-or-past-expiry) or "Close" (intent-mediated).

### 2.6 Portfolio summary

- Fetch from `GET /accounts/:address/portfolio-summary` (M-P2 endpoint).
- Display: total equity, free collateral, IM, MM, total notional.
- Refresh on every fill or every 30s polling.

### 2.7 History tab

- Fetch from `GET /accounts/:address/option-history` (M-P2 endpoint).
- Filterable by: side, series, status (filled/cancelled/exercised/settled).
- Paginated.

### 2.8 Account / deposit / withdraw

- Per-token balance from `GET /accounts/:address/balances` (M-P2 endpoint).
- "Deposit" CTA → opens deposit widget; user approves `IERC20.approve(CV)`; user calls `CV.deposit(asset, amount)`.
- "Withdraw" CTA → opens withdraw widget; user calls `CV.withdraw(asset, amount)`.

### 2.9 Transaction status

- `/tx/:intent_id` page renders the lifecycle (signing-payload-issued → signed → broadcast-sent → confirmed | reverted | stuck).
- Reads from `GET /options/execution-intents/:id` + `GET /executor/transactions/:intent_id` + `GET /executor/confirmations/:intent_id`.

### 2.10 Error / toast handling

Every failure produces a typed toast with: code, human message, retry CTA where applicable, "details" expander showing the backend-returned `details` field.

## 3. Component contract

Components consume backend responses via a small set of `hooks/` that wrap
SWR / react-query. Every hook returns `{ data, error, loading, refetch }`.

Required hooks (M-P3 implements):

- `useMarkets()` → `GET /options/products`
- `useOptionSeries(productId)` → `GET /options/series`
- `useOrderbook(seriesId)` → `GET /options/orderbooks/:id`
- `useQuotePreview(seriesId, side, size, price?)` → `GET /quotes/preview`
- `usePositions(address)` → `GET /accounts/:address/positions`
- `usePortfolioSummary(address)` → `GET /accounts/:address/portfolio-summary`
- `useOptionHistory(address, filter)` → `GET /accounts/:address/option-history`
- `useExecutionIntent(intentId)` → `GET /options/execution-intents/:id`
- `useTxStatus(intentId)` → composite over execution-intent + transactions + confirmations
- `useExecutorHealth()` → `GET /executor/health/v2` (banner-state only)

## 4. Non-functional requirements

| Requirement | Target | Notes |
|---|---|---|
| Time-to-interactive | < 3 s on Base Sepolia RPC | static-render landing + lazy-load trading routes |
| First-paint without wallet | always | Trade button is disabled until wallet connects |
| Stale-quote refusal | refuse if `quote_expires_at_ms < now` | backend enforces; UI mirrors |
| Network mismatch refusal | refuse if `wallet.chainId != ENV_CHAIN_ID` | persistent banner |
| Backend unavailable refusal | refuse if `/executor/health/v2.overall_status == "unhealthy"` | persistent banner |
| Cardinality-safe metrics | no per-tx-hash, no per-user-address in admin-side metrics surface | same as backend cardinality policy |
| Strict CSP | `default-src 'self'` minimum at M-P3 closure | from `ADMIN_FRONTEND_AUTH_PROXY_V2G_W2.md` |
| Unaudited warning banner | persistent until M-P7 closure + external audit completes | sticky banner; cannot be dismissed |

## 5. Wallet / network handling specifics

- Provider abstraction: `viem` is the minimum dep; optional `wagmi` for connector ergonomics; do NOT use `@web3modal/*` (CI guard).
- EIP-712 signing: backend hands the typed-data envelope; UI signs via wallet's `signTypedData_v4` equivalent; signature posted back.
- Nonce: backend provides via `GET /accounts/:address/option-nonce`; UI never computes nonce client-side.
- Gas estimation: backend simulates; UI displays as informational only.
- Chain id banner: testnet badge always visible until mainnet activation (post-M-P7 closure).

## 6. UX state matrix

(Companion to gap-analysis §4.5; here as a per-flow table for M-P3.)

### 6.1 Connect-wallet flow

| State | Visual | Source |
|---|---|---|
| Disconnected | "Connect wallet" CTA + landing copy | no provider in context |
| Connecting | "Approving in wallet…" spinner | `awaitingConnect` |
| Connected, wrong network | top banner + Switch CTA | `chainId != ENV_CHAIN_ID` |
| Connected, correct network | wallet badge in nav | both ok |

### 6.2 Trade-ticket flow

| State | Visual | Source |
|---|---|---|
| Ready | preview block + enabled Sign button | quote valid + form valid |
| Stale quote | banner: "Quote expired — refresh" | `quote_expires_at_ms < now` |
| Insufficient collateral | inline error + deposit CTA | `free_collateral < required` |
| Signature requested | "Approve typed data in your wallet" modal | `awaitingSignature` |
| Signed | "Submitting…" spinner | signed but not yet broadcast |
| Broadcast | progress timeline + link to `/tx/:intent_id` | broadcast intent posted |
| Confirmed | green toast | confirmation worker reports |
| Reverted | red toast + reason | revert reason from receipt |
| Rejected | warn toast | wallet returned reject |
| Backend unavailable | top banner + Sign disabled | `/executor/health/v2.overall_status != "ok"` |

### 6.3 Positions flow

| State | Visual | Source |
|---|---|---|
| No positions | empty state with "Open a position" CTA | empty array |
| Loading | skeleton rows | initial fetch |
| Open positions | rows with mark / unrealised PnL | `usePositions` |
| Exercise eligible | row action button | `now >= expiry` for that series |
| Exercise pending | "Exercising…" spinner | execution-intent submitted |
| Exercised | row moves to history | settle event ingested |

## 7. RBAC / auth surface

- Trading UI is unauthenticated (user wallet is the identity).
- Admin UI is gated by SSR proxy + OIDC/MFA + Bearer (V2G-W3 closure; M-P3 prereq).
- Trading UI MUST NOT use the admin Bearer token under any circumstance.

## 8. Out of scope for MVP

- Order book depth chart (Q3 nice-to-have).
- Mobile-optimised UI (responsive baseline only).
- L2 sequencer outage UI (banner reused from backend health endpoint, no extra UX).
- Perp trading (NOT_APPLICABLE_AT_LAUNCH).
- Portfolio analytics (PnL history graphs, Greeks visualisations).
- Push notifications.
- Multi-wallet portfolio view.

## 9. Cross-links

- `PRODUCT_READINESS_ROADMAP.md`
- `PRODUCT_GAP_ANALYSIS_SOL_BACKEND_FRONTEND.md`
- `E2E_TRADING_LIFECYCLE_TEST_PLAN.md`
- `NEXT_PRODUCT_MILESTONES.md`
- `~/DEOPT/deopt-v2-frontend/docs/ADMIN_FRONTEND_AUTH_PROXY_V2G_W2.md`
- `~/DEOPT/deopt-v2-frontend/docs/MAINNET_AUDIT_FRONTEND_ADMIN_SCOPE_FINAL.md`

**End of trading interface requirements.**
