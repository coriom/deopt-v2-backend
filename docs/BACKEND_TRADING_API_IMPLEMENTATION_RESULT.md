# BACKEND-TRADING-API-IMPLEMENTATION — Result

**Date:** 2026-06-10
**Anchor commit:** sol freeze `d133e2c` / `v2-product-freeze-rc1`
**Posture:** **source code modified** (new module + 12 routes wired); no
mainnet broadcast, no `.env` change, no signer call, no AWS resource.
**Status:** the 12 frontend-facing trading endpoints are implemented +
wired + tested + clippy-clean + green.

## 1. Scope

This milestone (M-P2a) implements the 12 endpoints specified by M-P2
(`BACKEND_TRADING_API_CONSOLIDATION_RESULT.md`) and `OpenAPI`
(`docs/openapi/trading-api.openapi.json`). Endpoints derivable from
existing DB/store/indexer data return real responses. Endpoints needing
RPC orchestration against `MarginEngineLens` / `OracleRouter` /
`CollateralVaultViews` return a typed `SOURCE_UNAVAILABLE` envelope so
the frontend can wire UI surfaces against stable contracts before RPC
orchestration lands in a focused follow-on milestone.

## 2. New module

| Path | Lines | Purpose |
|---|---|---|
| `src/api/trading.rs` | ~1090 | All 12 handlers + envelope types + error model + product computation + tests |

The module is registered in `src/api/mod.rs` as `pub mod trading;` and
wired into the router in `src/api/routes.rs` before the admin middleware
(so the public trading routes pass through unauthenticated; the admin
middleware only applies to paths starting with `/admin/`).

## 3. Endpoints implemented

| # | Method | Path | Status | Data source |
|---|---|---|---|---|
| 1 | GET | `/options/products` | **REAL** | groups option-series-store rows by `(underlying, settlement_asset, is_call, expiry)`; supports `?underlying`, `?is_call`, `?include_inactive` query filters |
| 2 | GET | `/options/products/:product_id` | **REAL** | iterates option-series-store; returns product detail + member `series_ids` |
| 3 | GET | `/options/products/batch?ids=a,b,c` | **REAL** | comma-separated batch lookup; max 100 ids |
| 4 | GET | `/options/series/:series_id/details` | **PARTIAL** | series + last-fill from store; `oracle_mark_1e8` + `orderbook_top` marked as not-yet-wired warning (deferred) |
| 5 | GET | `/options/quotes/preview` | **SOURCE_UNAVAILABLE** | validates side / size / address / series-exists; returns typed `SOURCE_UNAVAILABLE` envelope |
| 6 | GET | `/accounts/:address/positions` | **SOURCE_UNAVAILABLE** | validates address; returns typed envelope |
| 7 | GET | `/accounts/:address/portfolio` | **SOURCE_UNAVAILABLE** | validates address; returns typed envelope |
| 8 | GET | `/accounts/:address/history` | **REAL** | filters `OptionFillFilter { account: Some(addr), .. }`; returns `HistoryItem[]` with side derived from buyer/seller match |
| 9 | GET | `/accounts/:address/balances` | **SOURCE_UNAVAILABLE** | validates address; returns typed envelope |
| 10 | POST | `/options/exercise/preview` | **SOURCE_UNAVAILABLE** | validates address + series-exists; returns typed envelope |
| 11 | POST | `/options/close/preview` | **SOURCE_UNAVAILABLE** | validates address + side + size + series-exists; returns typed envelope |
| 12 | GET | `/trading/health` | **REAL** | derives from `build_executor_health_v2`; strips signer/KMS/mainnet-defence internals; exposes only `overall_status`, `chain_id`, `rpc_reachable`, `indexer_lag_blocks?`, `indexed_block?` |

**Tally**: 5 REAL (products list + detail + batch + history + trading health) + 1 PARTIAL (series details: series+last-fill REAL; oracle_mark + orderbook_top deferred) + 6 SOURCE_UNAVAILABLE.

The 6 SOURCE_UNAVAILABLE endpoints **still validate inputs**: invalid addresses, malformed sides, non-numeric sizes, unknown series ids all return their canonical error codes (`INVALID_ADDRESS`, `INVALID_REQUEST`, `SERIES_NOT_FOUND`). Only after input validation passes does the handler return `SOURCE_UNAVAILABLE`.

## 4. Data sources

| Endpoint group | Source | Notes |
|---|---|---|
| Products / Series | `OptionSeriesStore` via `list_option_series` + `get_option_series` services | already-existing service layer; no new ABI bindings required |
| History | `OptionFillFilter { account, .. }` via `list_option_fills` service | already-existing filter supports per-account; no new ABI required |
| Trading health | `build_executor_health_v2` subset projection | existing infra; deliberately strips sensitive fields (`signer_mode`, KMS, AWS, EXECUTOR_PRIVATE_KEY, DATABASE_URL) |
| Quotes preview, positions, portfolio, balances, exercise preview, close preview | **Deferred to follow-on; SOURCE_UNAVAILABLE typed response** | will use narrow inline `alloy_sol_types::sol!` macros (existing project pattern) for `MarginEngineLens.previewTradeFees`, `MarginEngineLens.getAccountState`, `MarginEngineLens.previewAccountSettlement`, `CollateralVaultViews.getCollateralTokens` + `CollateralVault.balances` |

## 5. ABI read helpers

**None added in this pass.** The 5 REAL endpoints are pure DB/store reads; no new sol-side ABI declarations were required. The 6 deferred endpoints will add narrow inline `sol!` declarations co-located with their handlers in the follow-on milestone, matching the existing project pattern (see `src/options/execution.rs:32-74` for the `executeTrade` precedent).

Frozen ABI selectors used by the deferred endpoints (when wired) MUST be checked against `~/DEOPT/deopt-v2-sol/abis/freeze-v2-product-rc1/selectors.txt` at PR time:

- `MarginEngineLens.previewTradeFees` — selector at freeze
- `MarginEngineLens.getAccountState` — selector at freeze
- `MarginEngineLens.previewAccountSettlement` — selector at freeze
- `MarginEngineLens.previewDetailedSettlement` — selector at freeze
- `OptionProductRegistry.getSeries` — selector at freeze (already verified in M-P2 spot-check)
- `CollateralVaultViews.getCollateralTokens` — selector at freeze
- `CollateralVault.balances(address,address)` — selector at freeze
- `OracleRouter.getFeed(address,address)` — selector at freeze

## 6. Response envelope

Every successful response is shaped:

```json
{
  "status": "ok",
  "data": { ... },
  "warnings": [ ... ],
  "meta": {
    "source": "db" | "rpc" | "indexer" | "validation" | "spec" | "internal",
    "chain_id": 31337,
    "request_id": "uuid-v4",
    "generated_at_ms": 1717000000000
  }
}
```

Every error response:

```json
{
  "status": "error",
  "error": { "code": "INVALID_ADDRESS", "message": "...", "details": {} },
  "meta": { "chain_id": 31337, "request_id": "uuid-v4", "generated_at_ms": 1717000000000 }
}
```

`meta.request_id` is a fresh UUID v4 per request (for support correlation).

Decimal-string encoding is used for `strike_1e8`, `size_1e8`, `price_1e8`, `contract_size_1e8`, and other on-chain quantities (preserves precision; no JS number underflow).

## 7. Error model

17 codes wired (16 from spec + 1 new — `SOURCE_UNAVAILABLE`; OpenAPI spec updated):

| Code | HTTP | Used by |
|---|---|---|
| `INVALID_ADDRESS` | 400 | every address-bearing endpoint |
| `INVALID_REQUEST` | 400 | quote/close preview side/size validation; batch ids validation |
| `PRODUCT_NOT_FOUND` | 404 | product detail |
| `SERIES_NOT_FOUND` | 404 | series details, quote preview, exercise preview, close preview |
| `ORDERBOOK_UNAVAILABLE` | 503 | (defined; not yet triggered) |
| `QUOTE_STALE` | 409 | (defined; not yet triggered) |
| `QUOTE_UNSUPPORTED` | 422 | (defined; not yet triggered) |
| `INSUFFICIENT_BALANCE` | 422 | (defined; not yet triggered) |
| `INSUFFICIENT_COLLATERAL` | 422 | (defined; not yet triggered) |
| `PREVIEW_REVERTED` | 422 | (defined; not yet triggered) |
| `INDEXER_STALE` | 503 | (defined; not yet triggered) |
| `RPC_UNAVAILABLE` | 503 | (defined; not yet triggered) |
| `SIGNER_UNAVAILABLE` | 503 | (defined; not yet triggered) |
| `EXECUTOR_UNAVAILABLE` | 503 | (defined; not yet triggered) |
| `SOURCE_UNAVAILABLE` | 503 | quote preview, positions, portfolio, balances, exercise preview, close preview |
| `RATE_LIMITED` | 429 | (defined; not yet triggered) |
| `INTERNAL_ERROR` | 500 | last-resort; never leaks internal trace |

**Sensitive-string discipline (enforced by test):**
The `trading_health_returns_envelope_with_chain_id` test serialises the
full response body and asserts the absence of `signer_mode`, `KMS`,
`aws`, `kms`, `EXECUTOR_PRIVATE_KEY`, `DATABASE_URL`, and `AWS_`
substrings.

## 8. Public / admin boundary

- The 12 trading routes are registered **before** the admin
  `from_fn_with_state(gate_state, admin_route_gate)` layer, but
  `admin_route_gate` short-circuits when `!path.starts_with("/admin/")`,
  so trading routes pass through unauthenticated.
- The trading routes do NOT consume any admin headers and do NOT branch
  on RBAC. They are public.
- Admin routes remain protected exactly as before; no change.
- The `trading_health` handler is a frontend-safe SUBSET projection of
  `executor_health_v2` — `signer_mode`, `remote_signer_configured`,
  `signer_address`, KMS provider name, and mainnet-defence counters are
  deliberately omitted. This is asserted by test.

## 9. Tests added

**26 new tests** in `src/api/trading::tests`:

| Group | Count | Tests |
|---|---|---|
| Product id computation | 3 | deterministic, call vs put, case-insensitive |
| Error code enum | 2 | screaming-snake encoding, HTTP status mapping |
| Address validation | 3 | rejects garbage, rejects short hex, accepts valid |
| Aggregate products | 1 | groups by `(underlying, settlement, cp, expiry)`; `series_count` + `is_active_any` correct |
| MetaBlock | 1 | request_id is UUID-shape; chain_id propagated |
| Quote preview | 3 | side validation, size validation, account validation |
| Positions / Portfolio / Balances | 4 | invalid address rejected; SOURCE_UNAVAILABLE for valid |
| History | 2 | invalid address rejected; empty list for unknown account |
| Exercise preview | 1 | invalid address rejected |
| Close preview | 2 | invalid side rejected, non-numeric size rejected |
| List products / batch | 3 | empty envelope, batch empty rejected, batch oversize rejected |
| Trading health | 1 | envelope shape + no leaked secrets (sensitive-string substring assertions) |

Run:

```bash
cargo test --lib api::trading
```

→ 26 passed; 0 failed.

## 10. Full validations

| Check | Result |
|---|---|
| `cargo fmt --check` | clean |
| `cargo clippy --all-targets --all-features -- -D warnings` | clean |
| `cargo build --lib` | clean |
| `cargo test --all-targets --no-fail-fast` | **1058 passed; 0 failed; 0 ignored** (lib 802 / engine_tests 8 / orderbook_options_eip712 12 / orderbook_tests 43 / option_execution_tests 67 / option_lifecycle 76 / option_rfq_tests 13 / rfq_tests 37). Trading additions land in the lib suite (+26 over the prior 776 baseline). |
| OpenAPI spec validation | `python3 json.load` clean; 12 paths, 29 schemas; added `SOURCE_UNAVAILABLE` to the error enum |
| Sensitive-string scan against the new module | zero `AKIA[0-9A-Z]{16}` matches; zero real `arn:aws:kms:<region>:<12-digit>:key/<uuid>` matches; zero `AWS_ACCESS_KEY_ID=` / `AWS_SECRET_ACCESS_KEY=` env-assignment matches; zero real KMS key UUIDs; zero production-EVM-shaped strings; the test `trading_health_returns_envelope_with_chain_id` enforces this at runtime |

## 11. Files changed

| Path | Status | Notes |
|---|---|---|
| `src/api/trading.rs` | **new** | ~1090 lines (handlers + types + tests) |
| `src/api/mod.rs` | edited | added `pub mod trading;` |
| `src/api/routes.rs` | edited | wired 12 new routes before the admin middleware layer |
| `docs/openapi/trading-api.openapi.json` | edited | added `SOURCE_UNAVAILABLE` to error enum |
| `docs/BACKEND_TRADING_API_IMPLEMENTATION_RESULT.md` | **new** | this doc |
| `docs/FRONTEND_TRADING_MVP_WIRING_NEXT_TASK.md` | (preserved; M-P2 produced; no semantic change needed at this stage) | M-P3 frontend team can continue with the existing prompt; backend now partially implemented |
| `RUN_STATE.md` | edited | closure paragraph prepended |

**Production source modified** intentionally (`src/api/trading.rs` is
new module added; `src/api/mod.rs` + `src/api/routes.rs` are extended
with the new route declarations only). No existing handler / service /
type is modified. No `RemoteSignerClient::new` change. No `.env` change.
No AWS / KMS / signer wiring change.

## 12. Frontend handoff status

The frontend MVP wiring milestone (M-P3) per
`FRONTEND_TRADING_MVP_WIRING_NEXT_TASK.md` remains valid and can
proceed. Two posture updates:

- The mock-server posture (prism against the OpenAPI spec) remains
  optional but no longer mandatory: 5 endpoints + 1 partial endpoint
  now return real data when run against a local backend.
- The 6 SOURCE_UNAVAILABLE endpoints return a typed envelope; the
  frontend can render a "not yet wired" placeholder in those panels
  without breaking the UX. The error-handling UX matrix in
  `FRONTEND_TRADING_API_HANDOFF.md §11` covers `SOURCE_UNAVAILABLE`
  → yellow banner.

## 13. Blockers

None. The remaining 6 endpoints need narrow inline `sol!` declarations
+ `EthCallProvider` orchestration; tracked in a M-P2b follow-on
milestone but not blocking M-P3 frontend wiring.

## 14. Next milestone recommendation

**Serialised next:** `FRONTEND-TRADING-MVP-WIRING` (M-P3; frontend team)
— begins against the new backend; uses `SOURCE_UNAVAILABLE` warning
banner in 6 panels until M-P2b wires the deferred endpoints.

**In parallel:** `BACKEND-TRADING-API-IMPLEMENTATION-PHASE-2` (M-P2b)
— wires the 6 `SOURCE_UNAVAILABLE` endpoints with narrow inline `sol!`
ABI declarations + `EthCallProvider` orchestration + per-endpoint
integration tests; ~1-2 weeks; runs in parallel with M-P3 so the full
trading UI is data-backed by M-P4 (E2E local).

## 15. Cross-links

- `~/DEOPT/deopt-v2-backend/docs/BACKEND_TRADING_API_CONSOLIDATION_RESULT.md`
- `~/DEOPT/deopt-v2-backend/docs/BACKEND_TRADING_API_OPENAPI_PLAN.md`
- `~/DEOPT/deopt-v2-backend/docs/openapi/trading-api.openapi.json`
- `~/DEOPT/deopt-v2-backend/docs/FRONTEND_TRADING_API_HANDOFF.md`
- `~/DEOPT/deopt-v2-backend/docs/FRONTEND_TRADING_MVP_WIRING_NEXT_TASK.md`
- `~/DEOPT/deopt-v2-backend/docs/PRODUCT_READINESS_ROADMAP.md`
- `~/DEOPT/deopt-v2-backend/docs/PRODUCT_GAP_ANALYSIS_SOL_BACKEND_FRONTEND.md`
- `~/DEOPT/deopt-v2-backend/docs/NEXT_PRODUCT_MILESTONES.md`
- `~/DEOPT/deopt-v2-sol/abis/freeze-v2-product-rc1/`

**End of M-P2a result.**
