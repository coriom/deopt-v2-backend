# Backend Trading API — OpenAPI 3.1 Plan

**Date:** 2026-06-10
**Posture:** specification + plan. **No source code modified.**
**Spec artefact:** `docs/openapi/trading-api.openapi.json`
**Companion:** `BACKEND_TRADING_API_CONSOLIDATION_RESULT.md`

## 1. Scope

This document is the design-level companion to the OpenAPI 3.1 spec at
`docs/openapi/trading-api.openapi.json`. It captures field-level decisions,
schema rationale, encoding policy, and the implementation backlog for
M-P2a (`BACKEND-TRADING-API-IMPLEMENTATION`).

## 2. Spec conventions

- **OpenAPI version:** 3.1.0 (JSON Schema 2020-12 compatible).
- **Auth posture:** all 11 new endpoints are public. The spec carries no
  `security` requirement on them. Existing wallet-signed and admin
  routes are documented separately in the spec under their own paths
  and security schemes.
- **chain_id** is included in every response envelope's `meta` block;
  servers configured for Sepolia return `84532`, anvil-local `31337`,
  mainnet `8453` (only post-mainnet-deployment).
- **address** fields use 0x-prefixed EIP-55 checksum strings.
- **uint256 / uint128 / int256** fields use **decimal string**
  (JSON Schema `type: "string", pattern: "^-?[0-9]+$"`).
- **timestamps** carry both `*_at_ms` (integer; unix milliseconds) and
  `*_at` (RFC3339 string).
- **enum** fields are explicit strings (no numeric enums).

## 3. Aliasing decisions

| Existing route | Status under V2 trading | Decision |
|---|---|---|
| `GET /markets` (legacy) | maintained | thin alias → `GET /options/products`; OpenAPI spec lists both with shared response schema |
| `GET /options/series` | maintained | canonical option-series list (per-strike) |
| `GET /options/series/:id` | maintained | canonical detail |
| `GET /options/series/:id/details` (NEW) | augmented | adds orderbook depth + last-trade-price for chain-UI single-round-trip |
| `GET /orderbook/:market_id` (legacy) | maintained | alias → `GET /options/orderbooks/:option_series_id` |
| `/orders/*` (legacy non-option) | unchanged | preserved for the perp surface (NOT_APPLICABLE_AT_LAUNCH); not part of M-P2 trading surface |

## 4. New endpoint shapes (summary)

Detailed JSON schemas in the OpenAPI spec. This table is the at-a-glance
plan.

### 4.1 `GET /options/products`

Returns products grouped by underlying. Product = unique
`(underlying, settlementAsset, isCall, expiry)` tuple aggregated above
series (each series = one strike under one product).

```json
{
  "status": "ok",
  "data": {
    "products": [
      {
        "product_id": "<keccak256(abi.encode(underlying, settlement_asset, is_call, expiry))>",
        "underlying": "0x...",
        "underlying_symbol": "ETH",
        "settlement_asset": "0x...",
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

### 4.2 `GET /options/products/:product_id`

Single product detail, with embedded series id list (UI fetches per-series detail or batch).

### 4.3 `GET /options/products/batch?ids=a,b,c` or POST body `{ "ids": [...] }`

Batch read for `OptionProductRegistry.getSeries` via server-side
multicall (substitutes for the deferred `getSeriesSlice` lens helper).

### 4.4 `GET /options/series/:series_id/details`

Series detail + orderbook snapshot top + last fill summary:

```json
{
  "status": "ok",
  "data": {
    "series": { "series_id": "...", "product_id": "...", "underlying": "0x...", "strike_1e8": "200000000000", "expiry_ms": 1717000000000, "is_call": true, "contract_size_1e8": "100000000", "settlement_asset": "0x...", "is_active": true, "metadata": "0x...." },
    "orderbook_top": { "best_bid_price_1e8": "...", "best_bid_size": "...", "best_ask_price_1e8": "...", "best_ask_size": "..." },
    "last_fill": { "price_1e8": "...", "qty": "...", "side": "buy", "block_number": 12345, "block_timestamp_ms": 1717000000000 },
    "oracle_mark_1e8": "..."
  }
}
```

### 4.5 `GET /options/quotes/preview`

Query params: `series_id`, `side` (`buy`|`sell`), `size`, optional `price_1e8`,
optional `account` (for IM impact).

```json
{
  "status": "ok",
  "data": {
    "series_id": "...",
    "side": "buy",
    "size": "1",
    "price_1e8": "2000000",
    "premium": "...",
    "buyer_fee": { "ppm_signed": 100, "amount": "..." },
    "seller_fee": { "ppm_signed": 100, "amount": "..." },
    "settlement_asset": "0x...",
    "oracle_mark_1e8": "...",
    "im_impact": "...",
    "free_collateral_after": "...",
    "quote_expires_at_ms": 1717000020000
  },
  "warnings": []
}
```

### 4.6 `GET /accounts/:address/positions`

Returns aggregated positions from `MarginEngineLens.getAccountState`.

```json
{
  "status": "ok",
  "data": {
    "address": "0x...",
    "positions": [
      {
        "series_id": "...",
        "product_id": "...",
        "size": "1",
        "side": "long",
        "avg_entry_price_1e8": "2000000",
        "mark_price_1e8": "2050000",
        "unrealised_pnl": "...",
        "im_contribution": "...",
        "mm_contribution": "...",
        "is_exercisable": false
      }
    ]
  }
}
```

### 4.7 `GET /accounts/:address/portfolio`

```json
{
  "status": "ok",
  "data": {
    "address": "0x...",
    "equity": "...",
    "im": "...",
    "mm": "...",
    "free_collateral": "...",
    "total_notional": "...",
    "open_positions_count": 3
  }
}
```

### 4.8 `GET /accounts/:address/history`

Aggregated across fills + RFQ accepts + cancellations + exercises +
settlements; cursor paginated.

```json
{
  "status": "ok",
  "data": {
    "items": [
      {
        "event_kind": "fill" | "rfq_fill" | "order_cancel" | "rfq_cancel" | "exercise" | "settle",
        "series_id": "...",
        "side": "buy",
        "size": "1",
        "price_1e8": "2000000",
        "fee_paid": "...",
        "tx_hash": "0x...",
        "block_number": 12345,
        "block_timestamp_ms": 1717000000000,
        "intent_id": "uuid-or-hex"
      }
    ]
  },
  "meta": { "pagination": { "next_cursor": "...", "has_more": true } }
}
```

### 4.9 `GET /accounts/:address/balances`

Iterates `CollateralVaultViews.getCollateralTokens` and per-token
`CollateralVault.balances(account, token)`.

```json
{
  "status": "ok",
  "data": {
    "address": "0x...",
    "balances": [
      {
        "token": "0x...",
        "symbol": "USDC",
        "decimals": 6,
        "balance": "1000000000",
        "balance_with_yield": "1003456789",
        "strategy_assets_preview": "3456789",
        "is_collateral_active": true
      }
    ]
  }
}
```

### 4.10 `POST /options/exercise/preview`

Body: `{ series_id, account }`. Read-only: invokes
`MarginEngineLens.previewAccountSettlement` and `previewDetailedSettlement`.
**No state mutation. No signer call. No broadcast.**

```json
{
  "status": "ok",
  "data": {
    "series_id": "...",
    "account": "0x...",
    "is_already_settled": false,
    "can_settle": true,
    "pnl": "...",
    "payable_from_settlement_sink": "...",
    "insurance_preview": "...",
    "collectible_from_trader_preview": "...",
    "residual_bad_debt_preview": "0"
  }
}
```

### 4.11 `POST /options/close/preview`

Body: `{ series_id, account, side, size, price_1e8? }`. Reuses
`MarginEngineLens.previewTradeFees` with the opposing side. Same shape
as §4.5 plus `position_size_after`.

## 5. Implementation backlog (M-P2a)

For each endpoint, the implementation milestone adds:

1. Route declaration in `src/api/routes.rs`.
2. Handler function (signature pattern: `async fn handler(State(state): State<AppState>, ...path/query/body...) -> Result<Json<Envelope<TData>>, ApiError>`).
3. Service function in the appropriate `src/options/`, `src/accounts/` (new module), or `src/fees/` module.
4. ABI binding (inline `sol!` macro) for the consumed sol view (if not already declared).
5. RPC call via `EthCallProvider`.
6. Decoding + DTO mapping.
7. Cache layer (per-block; bounded TTL).
8. Tests:
   - happy path;
   - invalid address rejected;
   - unknown id;
   - empty state;
   - stale indexer state;
   - preview routes: no mutation, no broadcast, no signer call.

Estimate per endpoint: 1-2 days; 11 endpoints → 2-3 weeks for the
implementation pass.

### 5.1 ABI binding additions (M-P2a, per the existing inline `sol!` pattern)

| Endpoint | New `sol!` declaration |
|---|---|
| 1, 2, 3 | `function getSeries(uint256) returns (OptionSeries)` + struct decl in `src/options/series_views.rs` (new) |
| 5, 11 | `function previewTradeFees(...) returns (...)` in `src/options/quote_preview.rs` (new) |
| 6, 7 | `function getAccountState(address, address) returns (AccountState)` + struct decl in `src/accounts/positions.rs` (new) |
| 9 | `function getCollateralTokens() returns (address[])` + `function balances(address, address) returns (uint256)` in `src/accounts/balances.rs` (new) |
| 10 | `function previewAccountSettlement(...) returns (...)` + `function previewDetailedSettlement(...) returns (...)` in `src/options/exercise_preview.rs` (new) |
| 8 | Indexer query (no new sol binding) |
| 4 | DB query + orderbook service (no new sol binding) |

Every new `sol!` declaration MUST be verified against
`~/DEOPT/deopt-v2-sol/abis/freeze-v2-product-rc1/` selectors at PR time.

## 6. Cache / staleness guidance

| Surface | Cache TTL | Reason |
|---|---|---|
| Products list | 60 s | new products are operator-created via Timelock; low churn |
| Product detail | 60 s | as above |
| Series details | 5 s | orderbook top can move frequently |
| Quote preview | 0 s | inherent freshness requirement; backed by RPC every call |
| Positions | 0 s | inherent freshness |
| Portfolio | 0 s | inherent freshness |
| Balances | 0 s | inherent freshness |
| History | 0 s (paginated) | indexer is authoritative |
| Exercise preview | 0 s | inherent freshness |
| Close preview | 0 s | inherent freshness |

Per-block cache invalidation: if backend has access to RPC `newHeads` /
WebSocket subscriptions, invalidate cache on each block. Otherwise rely
on TTL.

## 7. Polling / SSE / WS

M-P2 / M-P2a does NOT implement SSE / WS. UI polls per
`TRADING_INTERFACE_REQUIREMENTS.md §3` (15 s for orderbook; 30 s for
portfolio; on-demand for previews).

If high-frequency UX is required later, the canonical path is
`GET /trading/sse` or `WS /trading/ws` emitting indexer events
(fill / rfq_fill / order_cancel / position update); this is a separate
follow-on milestone.

## 8. Versioning

Spec carries `info.version: "0.1.0-mvp"`. Major version bumps:

- **breaking**: removed endpoint, changed field shape, changed enum value, changed error code semantics.
- **non-breaking**: new endpoint, new optional field, new enum variant in an open enum, new error code.

Frontend client generation pinned to spec version at M-P3 closure commit.

## 9. CI gates (M-P2a + onward)

- `redocly lint docs/openapi/trading-api.openapi.json` clean.
- Spec-route presence test: every `routes.rs` route on `/options/*`,
  `/accounts/*`, `/trading/*` MUST appear in the spec; CI fails on drift.
- Response-shape contract test: a smoke test invokes each endpoint
  against an integration harness and asserts JSON-Schema validates.

## 10. Cross-links

- `~/DEOPT/deopt-v2-backend/docs/BACKEND_TRADING_API_CONSOLIDATION_RESULT.md`
- `~/DEOPT/deopt-v2-backend/docs/openapi/trading-api.openapi.json`
- `~/DEOPT/deopt-v2-backend/docs/FRONTEND_TRADING_API_HANDOFF.md`
- `~/DEOPT/deopt-v2-backend/docs/PRODUCT_GAP_ANALYSIS_SOL_BACKEND_FRONTEND.md §3`
- `~/DEOPT/deopt-v2-sol/abis/freeze-v2-product-rc1/README.md`

**End of OpenAPI 3.1 plan.**
