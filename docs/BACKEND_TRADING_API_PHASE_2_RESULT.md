# BACKEND-TRADING-API-IMPLEMENTATION-PHASE-2 — Result

**Date:** 2026-06-10
**Anchor commit:** sol freeze `d133e2c` / `v2-product-freeze-rc1`
**Posture:** **source code modified** (handlers rewritten for 6 endpoints +
new typed response structs); no mainnet broadcast, no `.env` change, no
signer call, no AWS / KMS call, no RPC orchestration.
**Status:** all 6 previously `SOURCE_UNAVAILABLE` endpoints now return
**partial-real** data sourced from the existing indexer / OptionFill +
OptionSeries stores, with structured `warnings` declaring the on-chain
refinement still tracked for M-P2c.

## 1. What changed

The M-P2a `src/api/trading.rs` module returned `SOURCE_UNAVAILABLE`
typed envelopes for 6 endpoints because their full responses required
RPC orchestration against `MarginEngineLens` /
`CollateralVaultViews` / `FeesManagerV2` / `OracleRouter`. M-P2b
implements **deterministic partial responses** from existing stores +
indexer data, so the frontend can render real values (and the operator
can see lifecycle state) without depending on RPC config / availability.
The structured `warnings[]` array carries the gap so the UI can render
an "approximate" badge on partial fields.

| # | Endpoint | M-P2a status | M-P2b status | Source |
|---|---|---|---|---|
| 5 | GET `/options/quotes/preview` | SOURCE_UNAVAILABLE | **PARTIAL** | series strike + deterministic ppm |
| 6 | GET `/accounts/:address/positions` | SOURCE_UNAVAILABLE | **PARTIAL** | OptionFill aggregation by series + sign |
| 7 | GET `/accounts/:address/portfolio` | SOURCE_UNAVAILABLE | **PARTIAL** | OptionFill aggregation: open_positions_count + total_notional |
| 8 | GET `/accounts/:address/balances` | SOURCE_UNAVAILABLE | **PARTIAL (empty)** | requires CV.balances RPC; M-P2c |
| 10 | POST `/options/exercise/preview` | SOURCE_UNAVAILABLE | **PARTIAL** | net position from fills + expiry check; pnl = "0" placeholder |
| 11 | POST `/options/close/preview` | SOURCE_UNAVAILABLE | **PARTIAL** | mirrors quote_preview |

`status: "partial"` (not `"ok"`) is set in every envelope where the
on-chain refinement is still pending, so the frontend can distinguish
between real-and-final vs real-and-approximate.

## 2. Design decisions

### 2.1 No new ABI bindings

`alloy_sol_types::sol!` macros were **not** added in this pass. Rationale:

- The 6 endpoints require lens + view contracts whose **addresses are not configured anywhere on `AppState`** (e.g. `MarginEngineLens` address, `OptionProductRegistry` address, `OracleRouter` address). Adding them would mean adding new config fields, which the brief forbids ("Do not add new config secrets").
- The existing `EthCallProvider` lives behind `ExecutionConfig.rpc_url` which may be `None` in many beta deployments.
- The hard-stop "stop if read-only RPC orchestration cannot be safely mocked/tested" applied to a state where no mock provider exists for new contracts.
- The brief explicitly allows partial data with structured warnings: *"If full preview cannot be computed, return QUOTE_UNSUPPORTED or SOURCE_UNAVAILABLE with structured reason, not INTERNAL_ERROR."*

### 2.2 Data sources used

- `OptionSeriesStore` (`options_store`) → series metadata: strike, expiry, settlement asset, status, contract_size.
- `OptionFill` index (`list_option_fills_service`) → per-account fills with `OptionFillFilter { account, option_series_id, .. }`.
- `BroadcastObservability` snapshot → assertion target in tests that the preview handlers do NOT touch signer counters.

### 2.3 Deterministic partial preview formula

```
premium = size * price_1e8       // u128 saturating multiplication
fee     = premium * 100 / 1_000_000  // 100 ppm == 1 bps
                                     // PARTIAL_PREVIEW_TAKER_PPM constant
```

The 100 ppm assumption is conservative (below the chain-side `MAX_TAKER_FEE_PPM = 1000` cap). The structured `PARTIAL_PREVIEW` warning declares the assumption + `assumed_taker_ppm` field in `details`.

### 2.4 Position aggregation

```
For every fill of account A in series S:
  signed_delta = +size_1e8 if buyer == A
                 -size_1e8 if seller == A
  net[S] += signed_delta

For every S where net != 0:
  side = "long" if net > 0 else "short"
  abs_size = |net|
  avg_entry_price_1e8 = sum(size_1e8 * price_1e8) / sum(size_1e8)
```

No on-chain refinement (mark price / unrealised pnl / IM / MM contribution / is_exercisable) — these stay `None` with a structured warning. Operator deposits / withdrawals that don't go through fills are NOT yet reflected.

### 2.5 Exercise preview

`pnl = "0"` placeholder; `can_settle = is_past_expiry && net_position != 0`. The settlement breakdown (insurance / collectible / residual) requires `MarginEngineLens.previewAccountSettlement` + an oracle read; both tracked for M-P2c.

### 2.6 Balances

Returns an empty `balances: []` array with `SOURCE_UNAVAILABLE_FIELD` warning. Per-token balance reads require `CollateralVaultViews.getCollateralTokens` + `CollateralVault.balances(account, token)` RPC, neither of which is reachable without lens-address config. Tracked for M-P2c.

## 3. New typed response payloads

Added to `src/api/trading.rs`:

| Type | Used by |
|---|---|
| `FeeBreakdown { ppm_signed: i64, amount: String }` | quote / close preview |
| `QuotePreviewData` | quote / close preview |
| `Position` | positions list element |
| `PositionsData { address, positions: Vec<Position> }` | positions |
| `PortfolioData { address, equity?, im?, mm?, free_collateral?, total_notional?, open_positions_count? }` | portfolio |
| `BalanceRow` | balances list element |
| `BalancesData { address, balances: Vec<BalanceRow> }` | balances |
| `ExercisePreviewData { series_id, account, is_already_settled, can_settle, pnl, payable_from_settlement_sink?, … }` | exercise preview |

All optional fields use `Option<String>` so the JSON payload is `null` when the value is not yet wired. The `warnings: []` array carries the structured reason for any null.

## 4. Error model

Two new structured warning codes used (in addition to the existing 17 error codes):

| Warning code | Meaning | Used in |
|---|---|---|
| `PARTIAL_PREVIEW` | preview data is a deterministic approximation; `details.assumed_taker_ppm` carries the constant | quote / close / exercise preview |
| `SOURCE_UNAVAILABLE_FIELD` | a specific field is not yet wired; the message names the missing source | positions / portfolio / balances / preview endpoints |

The new endpoint-level error codes (per brief Phase F):

| Code | When returned |
|---|---|
| `SERIES_NOT_FOUND` 404 | unknown `series_id` |
| `INVALID_ADDRESS` 400 | malformed wallet address |
| `INVALID_REQUEST` 400 | bad side / size / price |
| `QUOTE_UNSUPPORTED` 422 | series is paused / settled / expired |

`SOURCE_UNAVAILABLE` as an HTTP-level error is no longer emitted by these 6 endpoints (each now returns success + warnings). The error code remains valid for transient backend issues.

## 5. Tests

### 5.1 Tests updated

- `positions_returns_source_unavailable_for_valid_address` → `positions_returns_empty_partial_for_default_state` (asserts `status: "partial"` + empty positions + non-empty warnings).
- `portfolio_returns_source_unavailable_for_valid_address` → `portfolio_returns_zero_partial_for_default_state` (asserts `status: "partial"` + `open_positions_count = 0` + `total_notional = "0"`).
- `balances_returns_source_unavailable_for_valid_address` → `balances_returns_empty_partial_for_default_state` (asserts `status: "partial"` + empty balances + `SOURCE_UNAVAILABLE_FIELD` warning).

### 5.2 Tests added (10)

| Test | Asserts |
|---|---|
| `quote_preview_returns_partial_for_unknown_series` | unknown series → `SERIES_NOT_FOUND` 404 |
| `quote_preview_rejects_bad_price` | non-numeric `price_1e8` → `INVALID_REQUEST` 400 |
| `quote_preview_returns_partial_for_active_series` | seeded active series → `status: "partial"` + correct fields + `PARTIAL_PREVIEW` warning |
| `quote_preview_refuses_inactive_series` | Expired/Inactive series → `QUOTE_UNSUPPORTED` 422 |
| `close_preview_returns_partial_for_active_series` | seeded active series → `status: "partial"` + correct side propagation |
| `close_preview_refuses_inactive_series` | Inactive series → `QUOTE_UNSUPPORTED` 422 |
| `exercise_preview_returns_partial_for_active_series` | seeded active series → `status: "partial"` + `can_settle = false` (no fills) + `PARTIAL_PREVIEW` warning |
| `exercise_preview_unknown_series_yields_series_not_found` | unknown series → `SERIES_NOT_FOUND` 404 |
| `preview_endpoints_do_not_call_signer_or_broadcast` | `signer_attempted/success/denied` counters unchanged across quote + close + exercise calls |
| `response_bodies_do_not_leak_secrets` | serialized success bodies do NOT contain `EXECUTOR_PRIVATE_KEY` / `DATABASE_URL` / `AWS_` / `aws_kms` / `signer_mode` |

### 5.3 Test posture

Trading-module tests: **36 in module + 776 elsewhere in lib = 812 lib tests** (+10 vs M-P2a baseline 802).

Test suite total (8 binaries): **1068 passed; 0 failed; 0 ignored** (`cargo test --all-targets --no-fail-fast`).

## 6. Full validations

| Check | Result |
|---|---|
| `cargo fmt --check` | clean |
| `cargo clippy --all-targets --all-features -- -D warnings` | clean (after fixing 1 `manual_checked_ops` warning on division — refactored to `checked_div().map(...)`) |
| `cargo build --lib` | clean |
| `cargo test --all-targets --no-fail-fast` | 1068 passed; 0 failed |
| OpenAPI spec validates via `python3 json.load` | clean |
| `git diff --check` | clean |
| Sensitive-string scan against modified `src/api/trading.rs` | zero `AKIA[0-9A-Z]{16}` / zero KMS ARN / zero `AWS_ACCESS_KEY_ID=` matches; enforced at test time by `response_bodies_do_not_leak_secrets` |

## 7. OpenAPI / handoff updates

No structural changes to the OpenAPI spec are required: the endpoint
paths + schemas already accept `null` for optional fields. The
`status: "partial"` value is documented in the existing
`EnvelopeStatus` enum (`"ok" | "stale" | "partial"`). The new warning
codes (`PARTIAL_PREVIEW`, `SOURCE_UNAVAILABLE_FIELD`) are documented
here + in the frontend handoff doc; they are free-form per the
`Warning.code: { type: string }` schema field.

## 8. Files changed

| Path | Status |
|---|---|
| `src/api/trading.rs` | edited — 6 handler bodies rewritten; new typed response structs `Position` / `PositionsData` / `PortfolioData` / `BalanceRow` / `BalancesData` / `QuotePreviewData` / `FeeBreakdown` / `ExercisePreviewData`; 10 new tests; 3 tests updated |
| `docs/BACKEND_TRADING_API_PHASE_2_RESULT.md` | **new** (this file) |
| `docs/BACKEND_TRADING_API_IMPLEMENTATION_RESULT.md` | (preserved; M-P2a result anchor) |
| `docs/openapi/trading-api.openapi.json` | (preserved; no spec drift) |
| `RUN_STATE.md` | edited (closure paragraph prepended) |

## 9. What is NOT in this milestone

```text
- Solidity changes
- Frontend changes
- new ABI bindings (no inline `sol!` macros added)
- new config fields (no env / `.env` edits)
- AWS / KMS calls
- signer calls
- broadcasts of any kind
- mainnet anything
- Sepolia anything
- production EVM addresses in tracked code
- DATABASE_URL / RPC URL / Bearer / API key in tracked code
- `SOURCE_UNAVAILABLE` as an error code for these 6 endpoints (moved to success-with-warning)
```

## 10. M-P2c follow-on scope

The on-chain refinement (full RPC orchestration) lands in M-P2c. It adds:

- A new config field `MarginEngineLens` address (or reuses the existing `margin_engine_address` if the deploy keeps lens deployment side-by-side).
- A new config field `OracleRouter` address.
- Narrow inline `alloy_sol_types::sol!` declarations in a new file `src/options/trading_views.rs`:
  - `MarginEngineLens.getAccountState(address, address)`
  - `MarginEngineLens.previewTradeFees(address, uint256, uint128, uint128, address, address, bool)`
  - `MarginEngineLens.previewAccountSettlement(address, uint256, address)`
  - `MarginEngineLens.previewDetailedSettlement(address, uint256, address)`
  - `CollateralVaultViews.getCollateralTokens()`
  - `CollateralVault.balances(address, address)`
  - `OracleRouter.getFeed(address, address)` + `hasActiveFeed(address, address)`
- Selector verification against `~/DEOPT/deopt-v2-sol/abis/freeze-v2-product-rc1/selectors.txt` at PR time.
- Per-endpoint RPC orchestration using the existing `EthCallProvider` infrastructure with graceful `RPC_UNAVAILABLE` fallback if `rpc_url` is `None`.
- Updates to the same 6 handlers to merge on-chain refined fields into the existing partial envelopes; `status: "partial"` becomes `status: "ok"` once all fields land.

## 11. Blockers

None. M-P2c is a focused follow-on (1-1.5 weeks). M-P3 (frontend) already handles `partial` envelope status + structured warnings gracefully (the `SOURCE_UNAVAILABLE` amber card pattern adapts to per-field "approximate" badges with minor frontend changes).

## 12. Next milestone recommendation

**Serialised next:** `FRONTEND-TRADING-SIGNING` (M-P3b) — wires viem +
EIP-712 signing flow + intent submission + live tx status against the
backend's existing `/options/execution-intents/:id/signing-payload` +
`POST /options/execution-intents/:id/signatures` + `POST
/options/execution-intents/:id/broadcast` + confirmation worker
endpoints. Critical for M-P4 (E2E local).

**In parallel:** `BACKEND-TRADING-API-IMPLEMENTATION-PHASE-3` (M-P2c) —
on-chain RPC orchestration that elevates `status: "partial"` to
`status: "ok"` for positions / portfolio / balances / preview
endpoints once lens + oracle addresses are configured.

M-P4 (E2E local) gates on M-P3b + M-P2c (or alternatively on the
status quo if E2E acceptance criteria allow `partial`).

## 13. Cross-links

- `~/DEOPT/deopt-v2-backend/docs/BACKEND_TRADING_API_IMPLEMENTATION_RESULT.md` (M-P2a result)
- `~/DEOPT/deopt-v2-backend/docs/BACKEND_TRADING_API_CONSOLIDATION_RESULT.md` (M-P2 spec)
- `~/DEOPT/deopt-v2-backend/docs/openapi/trading-api.openapi.json`
- `~/DEOPT/deopt-v2-backend/docs/FRONTEND_TRADING_API_HANDOFF.md`
- `~/DEOPT/deopt-v2-backend/docs/PRODUCT_READINESS_ROADMAP.md`
- `~/DEOPT/deopt-v2-backend/docs/NEXT_PRODUCT_MILESTONES.md`
- `~/DEOPT/deopt-v2-sol/abis/freeze-v2-product-rc1/`

**End of M-P2b result.**
