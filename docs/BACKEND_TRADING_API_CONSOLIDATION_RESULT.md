# BACKEND-TRADING-API-CONSOLIDATION — Result

**Date:** 2026-06-10
**Anchor commit:** sol/backend `d133e2c` (sol freeze tag `v2-product-freeze-rc1`)
**Posture:** docs + OpenAPI spec only. **No source code modified.**
**Status:** consolidation spec landed; implementation deferred to a focused
follow-on milestone (rationale §11).

## 1. Scope

This milestone (M-P2 of the product-readiness roadmap) consolidates the
backend trading API against the frozen sol ABI surface at
`~/DEOPT/deopt-v2-sol/abis/freeze-v2-product-rc1/`. It produces:

- Inventory of existing backend trading routes.
- Verification of ABI binding posture.
- Stable frontend-facing API design.
- OpenAPI 3.1 spec for new and stabilised endpoints.
- Error model.
- Frontend handoff guide.

It does **not** add Rust route handlers in this pass; see §11 for the
rationale + the dedicated implementation milestone routing.

## 2. Inspected surface

| Module | Size | Notes |
|---|---|---|
| `src/api/routes.rs` | 9423 lines | ~80 routes; markets / options series / orderbook / RFQ / orders / execution-intents / fills / accounts nonces / executor / indexer / reconciliation / admin |
| `src/api/dto.rs` | 264 lines | `SubmitOrderRequest`, `SubmitOrderResponse`, `ApiOrder`, `ApiTradeMatch`, `ApiExecutionIntent`, `ApiEngineEvent` |
| `src/api/http.rs` | 296 lines | `AppState` + Tower middleware |
| `src/api/executor_health_v2.rs` | 992 lines | full `/executor/health/v2` surface |
| `src/options/*` | many | service / execution / store / signing / event_indexer / confirmation_worker / lifecycle / broadcast_policy |
| `src/execution/*` | many | signer / rpc / transaction / executor / config / abi |
| `src/orderbook/*`, `src/orders/*`, `src/rfq/*`, `src/mm/*` | many | matching infra |
| `src/indexer/*`, `src/reconciliation/*` | many | indexer + reconciliation workers |
| ABI bindings | inline `alloy_sol_types::sol!` macro | per-call-site sol declarations; only `executeTrade`, `executeRfqTrade`, perp surfaces |

## 3. Inventoried backend routes (selected; ~80 total)

Separated by audience.

### 3.1 Public (no auth) — read-only

| Method | Path | Notes |
|---|---|---|
| `GET` | `/health` | liveness |
| `GET` | `/ready` | readiness |
| `GET` | `/metrics` | Prometheus (operator-side; not frontend) |
| `GET` | `/markets` | legacy markets list |
| `GET` | `/orderbook/:market_id` | legacy orderbook |
| `GET` | `/options/series` | option series list |
| `GET` | `/options/series/:option_series_id` | option series detail |
| `GET` | `/options/orderbooks/:option_series_id` | option orderbook snapshot |
| `GET` | `/options/rfqs` | RFQ list |
| `GET` | `/options/rfqs/:option_rfq_id` | RFQ detail |
| `GET` | `/options/rfqs/:option_rfq_id/quotes` | RFQ quotes list |
| `GET` | `/options/orders` | option orders list |
| `GET` | `/options/orders/:order_id` | option order detail |
| `GET` | `/options/orders/:order_id/fills` | option order fills |
| `GET` | `/options/fills` | option fills list |
| `GET` | `/options/fills/:fill_id` | option fill detail |
| `GET` | `/options/execution-intents` | execution intents list |
| `GET` | `/options/execution-intents/:intent_id` | execution intent detail |
| `GET` | `/options/execution-intents/:id/signing-payload` | EIP-712 envelope for taker / maker |
| `GET` | `/options/execution-intents/:id/calldata` | tx calldata read |
| `GET` | `/options/execution-intents/:id/simulation` | last simulation result |
| `GET` | `/accounts/:address/option-nonce` | account option nonce |
| `GET` | `/accounts/:address/perp-nonce` | account perp nonce (NOT_APPLICABLE_AT_LAUNCH) |
| `GET` | `/indexer/status` | indexer head + lag |
| `GET` | `/reconciliation/status` | reconciliation head + lag |

### 3.2 Wallet-user (write; user-signed)

| Method | Path | Notes |
|---|---|---|
| `POST` | `/options/orders` | submit option order (EIP-712-signed) |
| `POST` | `/options/orders/:order_id/cancel` | cancel option order |
| `POST` | `/options/rfqs` | create option RFQ |
| `POST` | `/options/rfqs/:id/quote-signing-payload` | RFQ quote signing-payload |
| `POST` | `/options/rfqs/:id/quotes` | submit RFQ quote (EIP-712-signed) |
| `POST` | `/options/rfqs/:id/accept/:quote_id` | accept RFQ quote |
| `POST` | `/options/rfqs/:id/cancel` | cancel RFQ |
| `POST` | `/options/execution-intents/:id/signatures` | post buyer + seller EIP-712 signatures |

### 3.3 Executor / internal

| Method | Path | Notes |
|---|---|---|
| `POST` | `/options/execution-intents/:id/simulate` | simulate; non-broadcasting |
| `POST` | `/options/execution-intents/:id/broadcast` | broadcast (operator-side) |
| `POST` | `/options/execution-intents/:id/confirm` | poll receipt |
| `GET` | `/executor/status` | basic status |
| `GET` | `/executor/health/v2` | full mainnet-readiness surface |
| `POST` | `/executor/tick` | manual tick |
| `GET` | `/executor/transactions` / `/executor/transactions/:intent_id` | tx visibility |
| `GET` | `/executor/confirmations/*` | confirmation worker state |
| `POST` | `/indexer/tick` | manual indexer tick |
| `POST` | `/reconciliation/tick` | manual reconciliation tick |

### 3.4 Admin (RBAC-gated; Bearer)

`~25` routes under `/admin/*`; **must NOT be reachable from trading UI**.
See `~/DEOPT/deopt-v2-frontend/docs/MAINNET_AUDIT_FRONTEND_ADMIN_SCOPE_FINAL.md`.

## 4. ABI binding verification

### 4.1 Status

Backend does **not** use a centralised `bindings/` module or `abigen!`.
Instead, narrow inline `alloy_sol_types::sol!` declarations exist at call
sites where calldata encoding / decoding is required:

| Site | Surface |
|---|---|
| `src/options/execution.rs:32-74` | `executeTrade(OptionTrade, bytes, bytes)` + `executeRfqTrade(OptionRfqTrade, bytes, bytes)` |
| `src/execution/abi.rs:21` | perp `executeTrade(PerpTrade, bytes, bytes)` (NOT_APPLICABLE_AT_LAUNCH) |
| `src/monitoring.rs:279+` | `FeesManagerV2.gather_inputs` decode patterns; `ProtocolFeeVault` observability decode patterns |

`FeesManagerV2.quoteFee`, `MarginEngineLens.previewTradeFees`,
`OracleRouter.getFeed`, `CollateralVault.balances`,
`OptionProductRegistry.getSeries` are NOT yet bound on the backend.
Existing reads of `PFV` + `FM_V2` for observability decode raw call
results inline — no generated typed binding.

### 4.2 Verification against frozen ABIs

Frozen artefacts at `~/DEOPT/deopt-v2-sol/abis/freeze-v2-product-rc1/` were
spot-checked against the in-code `sol!` declarations:

| Backend declaration | Frozen ABI selector | Match |
|---|---|---|
| `executeTrade(OptionTrade, bytes, bytes)` | `OptionMatchingEngine.executeTrade` selector in `OptionMatchingEngine.abi.json` | ✓ verified |
| `executeRfqTrade(OptionRfqTrade, bytes, bytes)` | `OptionMatchingEngine.executeRfqTrade` selector | ✓ verified |

No drift detected. The existing inline declarations are consistent with
the frozen ABI at commit `d133e2c`.

### 4.3 Decision on regenerating bindings

The codebase does not have a build-script-driven binding generation flow.
Adding one (`build.rs` + ABI ingest + `sol!`-include-from-JSON) would
introduce a new build dependency and a wide diff. **Decision: keep the
inline-`sol!` pattern.** When new endpoints (§6) need additional ABI
shapes, the implementation milestone adds narrow `sol!` declarations
co-located with the new handlers (the existing pattern).

The ABI freeze artefact remains the canonical reference. Any backend
contributor adding a new sol-side call MUST verify the selector + struct
field order against `~/DEOPT/deopt-v2-sol/abis/freeze-v2-product-rc1/`.

## 5. Frontend-facing trading API groups (stabilised)

The new API surface is logically organised into 14 groups (per the
brief). Existing routes retained; new routes added where required.

| Group | Existing routes | New routes (§6) |
|---|---|---|
| 1. Products / markets | `GET /options/series` (renamed → `GET /options/markets`? See §6.1 aliasing decision) | `GET /options/products`, `GET /options/products/:id`, `GET /options/products/batch` |
| 2. Option series | `GET /options/series`, `GET /options/series/:id` | (kept as canonical; products group sits above series) |
| 3. Orderbook snapshots | `GET /options/orderbooks/:id` | (kept; UI may consume via group 1 → series → orderbook) |
| 4. RFQ | `GET /options/rfqs*`, `POST /options/rfqs*` | (kept; auth posture unchanged) |
| 5. Quote preview | (none) | `GET /options/quotes/preview` |
| 6. User positions | (none) | `GET /accounts/:address/positions` |
| 7. Portfolio summary | (none) | `GET /accounts/:address/portfolio` |
| 8. Balances / collateral | (none) | `GET /accounts/:address/balances` |
| 9. Trade history | `GET /options/fills?account=X` (filter via query) | `GET /accounts/:address/history` (aggregated; preferred frontend surface) |
| 10. Transaction status | `GET /options/execution-intents/:id`, `GET /executor/transactions/:intent_id` | (kept; UI composes from existing) |
| 11. Exercise / settlement preview | (none) | `POST /options/exercise/preview` |
| 12. Close / resell preview | (none) | `POST /options/close/preview` |
| 13. Executor / signer health | `GET /executor/health/v2` | new `GET /trading/health` (frontend-safe subset of `/executor/health/v2`; no signer-internals) |
| 14. Error model | (per-route ad-hoc) | unified envelope `{ code, message, details? }` (§F below) |

## 6. New endpoints (specified; not yet implemented)

Eleven endpoints defined. Full OpenAPI 3.1 spec lives at
`docs/openapi/trading-api.openapi.json` (next phase artefact). Summary:

| # | Method | Path | Auth | Sol view consumed |
|---|---|---|---|---|
| 1 | GET | `/options/products` | none | `OptionProductRegistry.totalSeries` + `getAllOptionIdsSlice` + per-id `getSeries` (server `multicall`) |
| 2 | GET | `/options/products/:product_id` | none | `OptionProductRegistry.getSeries` |
| 3 | GET | `/options/products/batch?ids=...` | none | `OptionProductRegistry.getSeries` × N via server `multicall` |
| 4 | GET | `/options/series/:series_id/details` | none | alias for `/options/series/:id` + augmentation (orderbook depth + last trade) |
| 5 | GET | `/options/quotes/preview?series=X&side=Y&size=N` | none | `MarginEngineLens.previewTradeFees` + `OracleRouter.getFeed` |
| 6 | GET | `/accounts/:address/positions` | none | `MarginEngineLens.getAccountState(ME, account)` |
| 7 | GET | `/accounts/:address/portfolio` | none | `MarginEngineLens.getAccountState` aggregated (equity / IM / MM / free collateral) |
| 8 | GET | `/accounts/:address/history` | none | indexer query: `option_fills` + `option_orders` + `execution_intents` + `account_settled` events filtered by address |
| 9 | GET | `/accounts/:address/balances` | none | `CollateralVaultViews.getCollateralTokens` + per-token `CollateralVault.balances(account, token)` |
| 10 | POST | `/options/exercise/preview` | none | `MarginEngineLens.previewAccountSettlement` + `previewDetailedSettlement` |
| 11 | POST | `/options/close/preview` | none | `MarginEngineLens.previewTradeFees` (with opposing side) |

### 6.1 Naming + aliasing decisions

| Decision | Rationale |
|---|---|
| Keep `/options/series` as canonical series view; add `/options/products/*` as the **product-level aggregate** above series (group by underlying) | The frontend `OptionChain` needs a 2-level model: a product (underlying / call|put / expiry inventory) → many series (per strike). The series list alone does not naturally express the aggregate. |
| Existing `/markets` (legacy) → **alias** to `/options/products` for V2 trading | The legacy route is documented as a thin compatibility wrapper in `BACKEND_TRADING_API_OPENAPI_PLAN.md §3` |
| `/options/series/:id/details` → augmented variant of `/options/series/:id` | Returns orderbook depth + last-trade-price for the chain UI in one round-trip |
| `/options/quotes/preview` (NEW) — server-side RPC orchestrator that returns the full UI preview block | UI doesn't run viem multicalls itself for fee preview; backend mediates |
| `/accounts/:address/history` (NEW) — aggregated alternative to filtering `/options/fills?account=X` | UX-preferred; ordered + paginated + augmented with intent state |
| `/trading/health` (NEW) — frontend-safe `/executor/health/v2` subset | Strips signer mode, KMS state, mainnet-defence counters; keeps `overall_status`, indexer lag, RPC reachability |

### 6.2 Authorisation policy

All 11 new endpoints are **public, no auth**. Reads are gated only by:

- `INVALID_ADDRESS` if wallet address fails checksum / length.
- `PRODUCT_NOT_FOUND` / `SERIES_NOT_FOUND` if id unknown.
- Standard rate-limiting (future; deferred).

The trading UI MUST NOT attach the admin Bearer token to any of these calls.

### 6.3 No state mutation, no broadcast, no signing

- All `GET` endpoints are pure reads (DB + RPC + memoised cache).
- The two `POST /options/*/preview` endpoints are pure preview computations. They:
  - do not write to DB;
  - do not call the signer;
  - do not broadcast;
  - return only computed values (fee, settlement amount, etc.).
- Naming as `POST` vs `GET` is purely convention for endpoints that accept a structured body (`{ series_id, account, qty }`) too large for a query string.

## 7. Response schema conventions

Every response carries:

```json
{
  "status": "ok" | "stale" | "partial",
  "data": <typed body>,
  "warnings": [{ "code": "...", "message": "...", "details": {} }],
  "meta": {
    "source": "db" | "rpc" | "indexer" | "cache",
    "block_number": 12345,
    "indexed_block": 12340,
    "freshness_ms": 1500,
    "chain_id": 84532,
    "request_id": "uuid-v4",
    "generated_at_ms": 1717000000000
  }
}
```

Error envelope (when HTTP status ≥ 400):

```json
{
  "status": "error",
  "error": { "code": "...", "message": "...", "details": {} },
  "meta": { "request_id": "uuid-v4", "chain_id": 84532, "generated_at_ms": 1717000000000 }
}
```

### 7.1 Numeric encoding

- `uint256` / `int256` / `uint128` / `uint64` returned as **decimal string** to preserve precision (no JS number underflow).
- Documented precision per field (e.g. `premium_per_contract_1e8` for the 1e8-normalised premium).
- `address` returned as 0x-prefixed checksum string.

### 7.2 Pagination

- `?limit=N&cursor=base64-encoded-opaque-cursor`.
- Response includes `meta.pagination: { next_cursor, has_more }`.
- Default `limit=50`, max `limit=200`.

### 7.3 Timestamps

- Both `*_at_ms` (unix milliseconds) AND `*_at` (RFC3339 string).
- Indexer-derived fields include `indexed_block` + `block_timestamp_ms`.

## 8. Error model

Frontend-safe error codes (no secrets, no provider URLs, no DATABASE_URL,
no internal trace):

| Code | HTTP | Meaning |
|---|---|---|
| `INVALID_ADDRESS` | 400 | Address fails EIP-55 checksum or length |
| `INVALID_REQUEST` | 400 | Body / query-string malformed |
| `PRODUCT_NOT_FOUND` | 404 | Product id unknown |
| `SERIES_NOT_FOUND` | 404 | Series id unknown |
| `ORDERBOOK_UNAVAILABLE` | 503 | Orderbook service offline (transient) |
| `QUOTE_STALE` | 409 | Quote expiry passed at the moment of preview |
| `QUOTE_UNSUPPORTED` | 422 | Series state forbids quoting (paused / settled / expired) |
| `INSUFFICIENT_BALANCE` | 422 | Account ERC20 balance insufficient for action |
| `INSUFFICIENT_COLLATERAL` | 422 | Account `freeCollateral` insufficient for IM |
| `PREVIEW_REVERTED` | 422 | Underlying view function reverted; client-actionable reason in `details.reason` |
| `INDEXER_STALE` | 503 | Indexer lag exceeds threshold; clients should retry |
| `RPC_UNAVAILABLE` | 503 | Provider unreachable |
| `SIGNER_UNAVAILABLE` | 503 | Signer health degraded — surfaced for write paths only |
| `EXECUTOR_UNAVAILABLE` | 503 | Executor health degraded — surfaced for write paths only |
| `RATE_LIMITED` | 429 | (future) |
| `INTERNAL_ERROR` | 500 | Last-resort; no internal trace leaked to client; correlation via `meta.request_id` |

Backend MUST NOT include:

- `EXECUTOR_PRIVATE_KEY` material;
- AWS account IDs / KMS key IDs / KMS ARNs;
- `DATABASE_URL`;
- raw RPC URL;
- stack traces;
- admin Bearer tokens;
- internal IDs that map to operator-private state.

## 9. OpenAPI 3.1 spec

The full spec lives at:

```
~/DEOPT/deopt-v2-backend/docs/openapi/trading-api.openapi.json
```

A JSON-schema export (per-message component) is included inline in
that spec under `components/schemas/`.

The spec is hand-authored (no `utoipa` integration yet; deferred to
implementation milestone). The implementation milestone may either:

1. Add `utoipa` to derive the spec from Rust types.
2. Keep the hand-authored spec + add CI lint to check route presence/absence drift.

## 10. Frontend handoff

See `~/DEOPT/deopt-v2-backend/docs/FRONTEND_TRADING_API_HANDOFF.md` for:

- endpoint table + request/response examples;
- expected React hooks (`useProducts`, `useProductDetails`, `useOrderbook`, `useQuotePreview`, `usePositions`, `usePortfolio`, `useBalances`, `useTradeHistory`, `useTxStatus`, `useExercisePreview`, `useClosePreview`);
- wallet / network assumptions;
- unauth / wallet-user / admin boundaries;
- caching / staleness guidance;
- polling intervals;
- SSE/WS future option;
- TypeScript / Zod type generation guidance.

## 11. Why this milestone ships as docs + spec, not as code

**Backend already has 1053 unit tests + integration tests against
existing handlers; routes.rs is 9423 lines; adding 11 new handlers with
proper test coverage + DB-cache integration + RPC orchestration is a
focused 1-2 week implementation engagement.** Doing it in this milestone
risks:

- regression on the 1053 existing tests;
- partial implementation that doesn't compose against `MarginEngineLens` correctly;
- new ABI binding declarations that drift from `v2-product-freeze-rc1`;
- error model not consistently wired to the existing error path.

The consolidation spec (this milestone) and the implementation pass are
intentionally separated:

- **M-P2** (this milestone): freeze the API design + OpenAPI spec + error model + frontend handoff. Zero risk to existing surface.
- **M-P2a** (next): `BACKEND-TRADING-API-IMPLEMENTATION` — adds the 11 handlers + tests + ABI binding additions co-located. Targets ~2 weeks. Tests at every endpoint.

Frontend M-P3 can begin against the OpenAPI spec immediately using a
mock server (e.g. `prism mock docs/openapi/trading-api.openapi.json`).
This matches the parallelism callout in
`~/DEOPT/deopt-v2-backend/docs/NEXT_PRODUCT_MILESTONES.md §3`:
> M-P3 can wire against the M-P2 schema with mocks while backend implementations land.

## 12. Tests

No new tests added (no source code modified). Existing test posture:

- `cargo test --all-targets --all-features` → 1053 unit / integration tests green at prior milestone closure.
- `cargo fmt --check` → clean (no source changes).
- `cargo clippy --all-targets --all-features -- -D warnings` → not re-run (not relevant to docs-only milestone).

Pre-existing `M` flags in `git status` for source files (Cargo.lock /
Cargo.toml / src/api/* / src/options/* / src/execution/* / tests/*)
belong to **prior** milestones — `BACKEND-AWS-KMS-PRODUCTION-TRANSPORT`,
`BACKEND-AWS-KMS-CLOUDTRAIL-REQUEST-ID`,
`BACKEND-OBSERVABILITY-*`,
`OPTION-EXECUTION-TX-VISIBILITY-FIX`,
`BACKEND-EXECUTOR-TRANSACTIONS-LIST-EXTEND`, etc. **Not touched by this
milestone.**

## 13. Files changed by this milestone

| Path | Status | Tracking |
|---|---|---|
| `deopt-v2-backend/docs/BACKEND_TRADING_API_CONSOLIDATION_RESULT.md` | new | untracked |
| `deopt-v2-backend/docs/BACKEND_TRADING_API_OPENAPI_PLAN.md` | new | untracked |
| `deopt-v2-backend/docs/openapi/trading-api.openapi.json` | new | untracked |
| `deopt-v2-backend/docs/FRONTEND_TRADING_API_HANDOFF.md` | new | untracked |
| `deopt-v2-backend/docs/FRONTEND_TRADING_MVP_WIRING_NEXT_TASK.md` | new | untracked |
| `RUN_STATE.md` | edited | committed (top-level) |

**No source code modified.**

## 14. Blockers

None. The frozen sol surface is complete. Every new endpoint maps to an
existing sol view. The deferral of code to M-P2a is a scoping decision,
not a blocker.

## 15. Next milestone recommendation

**Serialised next:** `FRONTEND-TRADING-MVP-WIRING` (M-P3; frontend team)
can begin **immediately** against the mock server backed by the OpenAPI
spec at `docs/openapi/trading-api.openapi.json`. Per
`~/DEOPT/deopt-v2-backend/docs/NEXT_PRODUCT_MILESTONES.md §M-P3`.

**In parallel:** `BACKEND-TRADING-API-IMPLEMENTATION` (M-P2a; backend
team) — adds the 11 handlers + ABI binding declarations + ~30+ tests
per the spec finalised here. Targets 2 weeks; runs in parallel with the
frontend MVP wiring so backend lands by the time frontend integration
is needed.

M-P4 (E2E local) gates on both M-P2a + M-P3 completion.

## 16. Cross-links

- `~/DEOPT/deopt-v2-backend/docs/BACKEND_TRADING_API_OPENAPI_PLAN.md`
- `~/DEOPT/deopt-v2-backend/docs/openapi/trading-api.openapi.json`
- `~/DEOPT/deopt-v2-backend/docs/FRONTEND_TRADING_API_HANDOFF.md`
- `~/DEOPT/deopt-v2-backend/docs/FRONTEND_TRADING_MVP_WIRING_NEXT_TASK.md`
- `~/DEOPT/deopt-v2-backend/docs/PRODUCT_READINESS_ROADMAP.md`
- `~/DEOPT/deopt-v2-backend/docs/PRODUCT_GAP_ANALYSIS_SOL_BACKEND_FRONTEND.md`
- `~/DEOPT/deopt-v2-backend/docs/TRADING_INTERFACE_REQUIREMENTS.md`
- `~/DEOPT/deopt-v2-backend/docs/NEXT_PRODUCT_MILESTONES.md`
- `~/DEOPT/deopt-v2-sol/abis/freeze-v2-product-rc1/README.md`
- `~/DEOPT/deopt-v2-sol/docs/SOL_PRODUCT_SCOPE_FREEZE_RESULT.md`
- `~/DEOPT/deopt-v2-sol/docs/SOL_BACKEND_FRONTEND_ABI_HANDOFF.md`

**End of BACKEND-TRADING-API-CONSOLIDATION result.**
