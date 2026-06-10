# Frontend Trading API Handoff (M-P2 → M-P3)

**Date:** 2026-06-10
**Spec:** `docs/openapi/trading-api.openapi.json`
**Anchor commit:** sol/backend `d133e2c` (sol freeze tag `v2-product-freeze-rc1`)
**Posture:** read-only handoff. **No mainnet. No production secrets. Testnet beta only.**

## 1. Audience

Frontend team (M-P3). This document is the operational guide to consuming
the trading API for the DeOpt V2 trading MVP. Companion to:

- `BACKEND_TRADING_API_CONSOLIDATION_RESULT.md` (the design + decision doc)
- `BACKEND_TRADING_API_OPENAPI_PLAN.md` (the field-level rationale)
- `~/DEOPT/deopt-v2-sol/abis/freeze-v2-product-rc1/` (the frozen sol ABI)

## 2. Endpoint table

| Method | Path | Auth | Purpose | Hook (suggested) |
|---|---|---|---|---|
| GET | `/options/products` | none | Products list (group by underlying) | `useProducts(filter?)` |
| GET | `/options/products/:product_id` | none | Product detail + series ids | `useProductDetails(id)` |
| GET | `/options/products/batch?ids=...` | none | Batch products | `useProductsBatch(ids)` |
| GET | `/options/series/:series_id/details` | none | Series detail + orderbook top + last fill + oracle mark | `useSeriesDetails(id)` |
| GET | `/options/orderbooks/:series_id` | none | Orderbook snapshot (existing) | `useOrderbook(id)` |
| GET | `/options/quotes/preview` | none | Server-side fee + IM-impact preview | `useQuotePreview(args)` |
| GET | `/accounts/:address/positions` | none | Aggregated open positions | `usePositions(address)` |
| GET | `/accounts/:address/portfolio` | none | Portfolio summary | `usePortfolio(address)` |
| GET | `/accounts/:address/balances` | none | Per-token balances | `useBalances(address)` |
| GET | `/accounts/:address/history` | none | Trade + lifecycle history (paginated) | `useTradeHistory(address, filter)` |
| GET | `/options/execution-intents/:id` | none | Execution intent state (existing) | `useExecutionIntent(id)` |
| GET | `/executor/transactions/:intent_id` | none | Tx visibility (existing) | `useTxStatus(intent_id)` |
| GET | `/executor/confirmations/:intent_id` | none | Confirmation state (existing) | (composed inside `useTxStatus`) |
| POST | `/options/exercise/preview` | none | Exercise / settlement preview | `useExercisePreview(args)` |
| POST | `/options/close/preview` | none | Close-position preview | `useClosePreview(args)` |
| GET | `/trading/health` | none | Frontend-safe health (no signer internals) | `useTradingHealth()` |
| GET | `/options/execution-intents/:id/signing-payload` | wallet (returns EIP-712 envelope to sign client-side) | EIP-712 envelope for trade signing | wired inside trade-ticket submit flow |
| POST | `/options/execution-intents/:id/signatures` | wallet (posts buyer + seller signatures) | submit signatures | wired inside trade-ticket submit flow |

## 3. Request / response examples

### 3.1 List products

```http
GET /options/products?underlying=0x...&is_call=true&include_inactive=false&limit=50
```

```json
{
  "status": "ok",
  "data": {
    "products": [
      {
        "product_id": "0x1234...",
        "underlying": "0xabcd...",
        "underlying_symbol": "ETH",
        "settlement_asset": "0xdef0...",
        "settlement_asset_symbol": "USDC",
        "is_call": true,
        "expiry_ms": 1717000000000,
        "expiry": "2024-06-30T00:00:00Z",
        "series_count": 12,
        "is_active_any": true,
        "spot_price_1e8": "200000000000",
        "spot_price_age_ms": 1500
      }
    ]
  },
  "meta": { "source": "rpc+cache", "block_number": 12345, "chain_id": 84532, "request_id": "...", "generated_at_ms": 1717000000000 }
}
```

### 3.2 Quote preview

```http
GET /options/quotes/preview?series_id=42&side=buy&size=1&price_1e8=2000000&account=0x...
```

```json
{
  "status": "ok",
  "data": {
    "series_id": "42",
    "side": "buy",
    "size": "1",
    "price_1e8": "2000000",
    "premium": "2000000",
    "buyer_fee": { "ppm_signed": 100, "amount": "200" },
    "seller_fee": { "ppm_signed": 100, "amount": "200" },
    "settlement_asset": "0xdef0...",
    "oracle_mark_1e8": "2050000",
    "im_impact": "100000",
    "free_collateral_after": "9900000",
    "quote_expires_at_ms": 1717000020000
  }
}
```

### 3.3 Positions

```http
GET /accounts/0xabcd.../positions
```

```json
{
  "status": "ok",
  "data": {
    "address": "0xabcd...",
    "positions": [
      {
        "series_id": "42",
        "product_id": "0x1234...",
        "size": "1",
        "side": "long",
        "avg_entry_price_1e8": "2000000",
        "mark_price_1e8": "2050000",
        "unrealised_pnl": "50000",
        "im_contribution": "100000",
        "mm_contribution": "50000",
        "is_exercisable": false
      }
    ]
  }
}
```

### 3.4 Exercise preview

```http
POST /options/exercise/preview
Content-Type: application/json

{ "series_id": "42", "account": "0xabcd..." }
```

```json
{
  "status": "ok",
  "data": {
    "series_id": "42",
    "account": "0xabcd...",
    "is_already_settled": false,
    "can_settle": true,
    "pnl": "150000",
    "payable_from_settlement_sink": "150000",
    "insurance_preview": "0",
    "collectible_from_trader_preview": "0",
    "residual_bad_debt_preview": "0"
  }
}
```

### 3.5 Error envelope

```json
{
  "status": "error",
  "error": {
    "code": "INVALID_ADDRESS",
    "message": "Wallet address malformed",
    "details": { "expected": "EIP-55-checksummed 0x-prefixed 20-byte address" }
  },
  "meta": { "chain_id": 84532, "request_id": "...", "generated_at_ms": 1717000000000 }
}
```

## 4. Wallet / network assumptions

- Frontend uses `viem@^2` (recommended) or `wagmi@^2` if a wallet-modal connector is desired.
- All EIP-712 signing uses the backend-issued signing-payload (existing endpoints).
- Backend issues nonce; UI never computes nonce client-side.
- Network id MUST match `ENV_CHAIN_ID`; UI displays a banner + disables Sign if mismatched.
- `connect-src 'self'` Strict CSP target landed by M-P3 admin-scope closure (V2G-W3 SSR proxy).

### 4.1 Chain id mapping

| Environment | chain_id | Backend URL convention |
|---|---|---|
| Anvil local | 31337 | `http://localhost:3000` |
| Base Sepolia | 84532 | operator-managed; ENV-driven |
| Base mainnet | 8453 | NOT YET — gated on M-P7 + audit completion |

## 5. Boundary policy

| Boundary | Rule |
|---|---|
| Trading reads | Public; no Bearer token; no admin scope |
| Trading writes (orders / RFQ / signatures) | Wallet-signed EIP-712 envelopes; backend verifies; no Bearer |
| Admin reads / writes | SSR proxy gate + OIDC/MFA + Bearer; **MUST NOT** be invoked from trading routes |
| Executor / internal | Operator-side; not exposed in trading UI |

**The trading UI MUST NEVER attach a Bearer token to a trading
endpoint.** This is a CI guard at M-P3 closure.

## 6. Caching / staleness guidance

| Hook | Cache strategy | Re-fetch on |
|---|---|---|
| `useProducts` | SWR / react-query, `staleTime: 60_000`, `refetchInterval: 60_000` | route enter, manual refresh |
| `useProductDetails` | `staleTime: 60_000` | route enter |
| `useProductsBatch` | request-coalesced | series-list change |
| `useOrderbook` | `refetchInterval: 5_000` (5 s) | route enter, focus |
| `useSeriesDetails` | `refetchInterval: 5_000` | route enter |
| `useQuotePreview` | `staleTime: 0`, request-on-input-change | form field change (debounced 250 ms) |
| `usePositions` | `refetchInterval: 30_000` | wallet connect, route enter, post-fill toast |
| `usePortfolio` | `refetchInterval: 30_000` | as above |
| `useBalances` | `refetchInterval: 30_000` | wallet connect, deposit / withdraw event |
| `useTradeHistory` | infinite-scroll paginated | next-cursor |
| `useExecutionIntent` | `refetchInterval: 2_000` while pending; stop when terminal | intent submit |
| `useTxStatus` | `refetchInterval: 2_000` while pending; stop when terminal | intent submit |
| `useExercisePreview` | `staleTime: 0` | form change |
| `useClosePreview` | `staleTime: 0` | form change |
| `useTradingHealth` | `refetchInterval: 60_000` | route enter |

## 7. Polling vs SSE / WS

M-P2 / M-P2a do NOT ship SSE / WS. Polling intervals in §6 are
adequate for the testnet beta.

Future option: `GET /trading/sse` or `WS /trading/ws` emitting
indexer events (`fill`, `rfq_fill`, `order_cancel`, `position_update`,
`exercise`, `settle`). Deferred to a focused follow-on milestone
post-product-MVP launch.

## 8. TypeScript / Zod type generation

Recommended path:

```bash
# from frontend root
npx openapi-typescript ../deopt-v2-backend/docs/openapi/trading-api.openapi.json -o src/lib/trading-api.types.ts
```

Backend OpenAPI spec is the canonical source. M-P3 generates TS types
once at the freeze commit + re-runs on every M-P2a spec update.

For runtime validation: `zod` schemas can be generated via
`openapi-zod-client` or hand-written from the OpenAPI shape (small
surface, ~12 endpoints).

## 9. Mock server (during M-P3 wiring)

Before M-P2a lands the real handlers, the frontend can run a mock
server backed by the OpenAPI spec:

```bash
npx @stoplight/prism mock docs/openapi/trading-api.openapi.json --port 3000
```

Mock server returns example fixtures from the spec; frontend MVP can
wire all components against the mock first, then re-point at the real
backend on M-P2a closure.

## 10. Unauthenticated vs wallet-user vs admin boundaries

| Surface | Auth | Examples |
|---|---|---|
| Public (read-only) | none | products, series, orderbook, positions, portfolio, balances, history, quote/exercise/close preview, trading health |
| Wallet-user (write; user-signed) | EIP-712 client signature on backend-issued payload | submit order, submit RFQ quote, accept RFQ, post execution-intent signatures |
| Admin | SSR proxy + OIDC/MFA + Bearer | every `/admin/*` route; **not accessible from trading UI** |

## 11. Error handling pattern

Every hook returns `{ data, error, isLoading, refetch }`. Error code
mapping for toast/banner rendering:

| Code | UX |
|---|---|
| `INVALID_ADDRESS` | inline form error + reconnect CTA |
| `PRODUCT_NOT_FOUND` / `SERIES_NOT_FOUND` | 404 page or empty state |
| `ORDERBOOK_UNAVAILABLE` | banner: "Orderbook temporarily unavailable" + retry |
| `QUOTE_STALE` | banner: "Quote expired — refresh" |
| `QUOTE_UNSUPPORTED` | inline error: "Cannot quote this series (paused / settled / expired)" |
| `INSUFFICIENT_BALANCE` | inline error + deposit CTA |
| `INSUFFICIENT_COLLATERAL` | inline error + deposit CTA |
| `PREVIEW_REVERTED` | inline error with details.reason |
| `INDEXER_STALE` | yellow banner: "Some data may be stale (indexer catching up)" |
| `RPC_UNAVAILABLE` | red banner: "RPC offline — retry later" |
| `SIGNER_UNAVAILABLE` | red banner: "Trading offline" (write paths disabled) |
| `EXECUTOR_UNAVAILABLE` | red banner: "Trading offline" |
| `RATE_LIMITED` | toast: "Slow down" (future) |
| `INTERNAL_ERROR` | red toast with `meta.request_id` for support |

## 12. Banners that MUST persist

Per `~/DEOPT/deopt-v2-backend/docs/PRODUCT_READINESS_ROADMAP.md §6`:

- **"Unaudited testnet beta"** — sticky top banner; cannot be dismissed.
- **Network badge** — testnet badge visible until M-P7 + external audit completes.
- **Backend unhealthy banner** — when `/trading/health.overall_status != "ok"`.

## 13. Versioning + freeze hooks

- Spec version `0.1.0-mvp` at M-P2 close.
- Frontend M-P3 pins types to this spec version.
- M-P2a closure may bump to `0.1.0` (no `-mvp` suffix) when handlers land green.
- Breaking changes (any field shape change, any error code semantic change) trigger frontend re-pin.

## 14. Known limitations

- Perp surface is OUT of scope.
- Rebates DEFERRED at launch.
- Liquidation surface visible via `useTxStatus` only (no dedicated liquidation UI in MVP).
- No SSE / WS (polling only).
- No mobile-optimised UI (responsive baseline only).
- No portfolio analytics (PnL history graphs / Greeks).

## 15. Cross-links

- `~/DEOPT/deopt-v2-backend/docs/BACKEND_TRADING_API_CONSOLIDATION_RESULT.md`
- `~/DEOPT/deopt-v2-backend/docs/BACKEND_TRADING_API_OPENAPI_PLAN.md`
- `~/DEOPT/deopt-v2-backend/docs/openapi/trading-api.openapi.json`
- `~/DEOPT/deopt-v2-backend/docs/TRADING_INTERFACE_REQUIREMENTS.md`
- `~/DEOPT/deopt-v2-backend/docs/E2E_TRADING_LIFECYCLE_TEST_PLAN.md`
- `~/DEOPT/deopt-v2-backend/docs/NEXT_PRODUCT_MILESTONES.md`
- `~/DEOPT/deopt-v2-sol/abis/freeze-v2-product-rc1/README.md`
- `~/DEOPT/deopt-v2-sol/docs/SOL_BACKEND_FRONTEND_ABI_HANDOFF.md`

**End of frontend trading API handoff.**
