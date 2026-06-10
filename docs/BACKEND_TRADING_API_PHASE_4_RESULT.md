# BACKEND-TRADING-API-IMPLEMENTATION-PHASE-4 — Result

**Date:** 2026-06-10
**Anchor commit:** sol freeze `d133e2c` / `v2-product-freeze-rc1`
**Posture:** source code modified (AppState wire-in + 1 handler enriched with
read-orchestration; new sanitiser helper; 8 new tests). **No mainnet
broadcast. No `.env` change. No signer call. No AWS / KMS call. No
broadcast.**
**Status:** **AppState wire-in complete**. `account_balances` handler
fully wired with read-orchestration via the M-P2c framework, including
graceful partial-fallback paths + URL-sanitised error envelopes. The
remaining 6 partial endpoints (`quote_preview`, `account_positions`,
`account_portfolio`, `exercise_preview`, `close_preview`,
`series/:id/details`) follow the same pattern documented in §10 and
are tracked for **M-P2e** to keep this milestone surgically scoped.

## 1. What changed

| Layer | Change |
|---|---|
| `src/api/http.rs` | Added `pub trading_views: TradingViewsConfig` field on `AppState`; constructor initialises with `TradingViewsConfig::disabled()` |
| `src/api/trading_views.rs` | Made `tests` mod `pub` so the `ProgrammableMockProvider` fixture is reachable from `src/api/trading.rs` tests |
| `src/api/trading.rs` | Replaced the M-P2b stub body of `account_balances` with a thin wrapper that builds an `Option<HttpJsonRpcProvider>` from `state.execution_config.rpc_url` and dispatches to a new `account_balances_impl<P: EthCallProvider>(&AppState, &str, Option<&P>)` |
| `src/api/trading.rs` | Added `sanitise_rpc_err` helper — strips http(s) URLs + length-caps error text before it reaches the response envelope |
| `src/api/trading.rs` | Added 8 new tests covering the wired behaviour |

## 2. AppState wire-in

```rust
// src/api/http.rs
pub struct AppState {
    // … existing fields …
    pub options_config: OptionsConfig,
    pub fees_config: FeesConfig,
    /// M-P2d — Optional addresses for the trading-views read surface.
    /// All fields default to `None`; trading handlers fall back to the
    /// M-P2b partial-data path when any field is absent.
    pub trading_views: crate::api::trading_views::TradingViewsConfig,
    // …
}
```

The constructor adds one line:

```rust
trading_views: crate::api::trading_views::TradingViewsConfig::disabled(),
```

**`TradingViewsConfig::disabled()`** returns the all-`None` default,
so every existing call site that constructs `AppState` via the
canonical builder retains its prior behaviour. Test helpers that
override fields (`test_state`, `seeded_state`, `base_state`,
`programmable_state`, …) inherit the disabled default and may opt in to
configured addresses on a per-test basis.

**No env-loader keys** were added in M-P2d (env wiring is deferred to
M-P2e so this milestone's diff stays narrow and reviewable). The
operator constructs the addresses via the existing AppState builder
path; M-P2e adds `OPTION_MARGIN_ENGINE_LENS_ADDRESS`,
`OPTION_COLLATERAL_VAULT_VIEWS_ADDRESS`,
`OPTION_COLLATERAL_VAULT_ADDRESS`, and `OPTION_ORACLE_ROUTER_ADDRESS`
in `src/config/env.rs` + `.env.example`.

## 3. ABI helpers added

**None new in M-P2d.** The 3 helpers added in M-P2c
(`try_get_collateral_tokens`, `try_get_balance`, `try_preview_trade_fees`)
are sufficient for the `account_balances` wire-in. The remaining 5
helpers documented for M-P2c (`try_get_account_state`,
`try_preview_account_settlement`, `try_preview_detailed_settlement`,
`try_get_feed`, `try_has_active_feed`) are still
**pending** and land alongside their respective handler wire-ups in
**M-P2e**.

## 4. Selector verification

All 8 selectors landed in M-P2c remain pinned by the runtime
selector-verification tests. M-P2d adds no new ABI surface so no
additional selector tests were required.

## 5. Routes improved

**1 of 7 partial endpoints elevated to optional-OK in M-P2d:**

| # | Endpoint | M-P2c | M-P2d |
|---|---|---|---|
| 5 | `GET /options/quotes/preview` | partial | partial (M-P2e wires `try_preview_trade_fees` into fee fields) |
| 6 | `GET /accounts/:address/positions` | partial | partial (M-P2e adds `try_get_account_state`) |
| 7 | `GET /accounts/:address/portfolio` | partial | partial (M-P2e adds `try_get_account_state`) |
| 8 | `GET /accounts/:address/balances` | partial (empty) | **partial → "ok" when fully configured + RPC reachable** |
| 10 | `POST /options/exercise/preview` | partial | partial (M-P2e adds `try_preview_account_settlement` + `try_preview_detailed_settlement`) |
| 11 | `POST /options/close/preview` | partial | partial (M-P2e mirrors `quote_preview`) |
| 4 | `GET /options/series/:series_id/details` | partial (oracle pending) | partial (M-P2e adds `try_get_feed` + `try_has_active_feed`) |

## 6. `account_balances` — ok vs partial behavior

| Scenario | Status | Warnings |
|---|---|---|
| Trading-views fully unconfigured + no rpc_url | `partial` | `SOURCE_UNAVAILABLE_FIELD` listing each missing dependency by name (`CollateralVaultViews address`, `CollateralVault address`, `RPC provider`) |
| Only one of the 3 deps configured | `partial` | `SOURCE_UNAVAILABLE_FIELD` naming the missing pieces |
| All 3 deps configured, RPC reachable, success | **`ok`** | empty `warnings` array |
| All 3 deps configured, token-list RPC failure | `partial` | `RPC_UNAVAILABLE` (URL-sanitised message) |
| All 3 deps configured, token-list ok, per-token balance failure | `partial` | `RPC_UNAVAILABLE` (URL-sanitised; iteration short-circuits on first failure to bound work) |
| Invalid address | `400 INVALID_ADDRESS` (error, not partial) | n/a |

## 7. RPC fallback behavior

The implementation guarantees:

- **No panic** on any RPC failure path.
- **`warnings[]` carries a `RPC_UNAVAILABLE` code** when an RPC call returns `Err`.
- **`meta.source`** flips from `"rpc"` (fully OK) to `"db"` (fallback / partial).
- **Iteration short-circuits** on first per-token balance failure so a flaky token doesn't multiply the failure log.
- **`status` stays `"partial"` when any warning is present** even if some balances did succeed (conservative posture; UI shows amber badge).

## 8. URL / secret sanitisation

`sanitise_rpc_err(s)` strips `http://` and `https://` URLs from any
RPC-side error message before it reaches the response envelope. The
implementation:

1. Walks the string character by character.
2. When it encounters `h` followed by `ttp://` or `ttps://`, swallows
   the URL up to the next whitespace and emits `<url-redacted>` in its
   place.
3. Caps total length at 200 chars; truncates with an ellipsis.

Three dedicated tests cover http/https/length:

- `sanitise_rpc_err_strips_urls` — asserts `<url-redacted>` replaces an http URL.
- `sanitise_rpc_err_strips_https_too` — same for https.
- `sanitise_rpc_err_caps_length` — caps to 200 + ellipsis bytes.

**No raw RPC URL** appears in any handler response body.

## 9. No-mutation guarantees

The `account_balances_impl` function:

- **NEVER** calls a signer.
- **NEVER** calls broadcast.
- **NEVER** calls AWS / KMS.
- **NEVER** mutates DB / store / engine state.
- **NEVER** requires admin Bearer.
- **NEVER** requires mainnet values.
- Uses `EthCallProvider.eth_call` only — which is the read-only RPC method.

The `preview_endpoints_do_not_call_signer_or_broadcast` test from
M-P2b (asserts `signer_attempted/success/denied` counters unchanged
across quote + close + exercise calls) is unchanged and still passes;
M-P2d's wire-in is consistent with that contract.

## 10. Pattern for M-P2e

Each remaining partial endpoint follows the same pattern as
`account_balances`:

```rust
pub async fn handler_name(
    State(state): State<AppState>,
    // … extractors …
) -> Result<Json<Envelope<X>>, TradingApiError> {
    let provider = state
        .execution_config
        .rpc_url
        .as_ref()
        .map(|url| HttpJsonRpcProvider::new(url.clone()));
    handler_name_impl(&state, …, provider.as_ref()).await
}

pub(crate) async fn handler_name_impl<P: EthCallProvider>(
    state: &AppState,
    …,
    provider: Option<&P>,
) -> Result<Json<Envelope<X>>, TradingApiError> {
    // 1. Validate inputs (return error envelope on failure).
    // 2. Compute partial fields from existing store / indexer.
    // 3. If trading_views address + provider configured: call helper.
    // 4. On Ok(Some): merge into partial.
    // 5. On Ok(None): add SOURCE_UNAVAILABLE_FIELD warning.
    // 6. On Err: add RPC_UNAVAILABLE warning (sanitised).
    // 7. status = "ok" iff all configured reads succeeded and no warnings.
}
```

Tests follow the M-P2c pattern: per-handler 5-8 tests covering
unconfigured / configured-no-provider / configured-with-mock-success /
configured-with-mock-failure / invalid-input paths.

Estimated M-P2e effort: 3-5 days for the remaining 6 endpoints + their
~30 tests.

## 11. Tests added

**8 new tests** in `src/api/trading::tests`:

| Test | Asserts |
|---|---|
| `balances_impl_fully_unconfigured_falls_back_to_partial` | status partial; empty list; SOURCE_UNAVAILABLE_FIELD warning |
| `balances_impl_invalid_address_rejected` | INVALID_ADDRESS 400 |
| `balances_impl_configured_addresses_no_provider_still_partial` | partial; warning names `RPC provider` |
| `balances_impl_configured_returns_ok_when_provider_yields_tokens` | **status: "ok"**; 1 balance row; no warnings |
| `balances_impl_rpc_failure_falls_back_to_partial_with_sanitised_warning` | partial; RPC_UNAVAILABLE warning; **no `http://` or `https://` in message** |
| `sanitise_rpc_err_strips_urls` | `<url-redacted>` replaces http URL |
| `sanitise_rpc_err_strips_https_too` | same for https |
| `sanitise_rpc_err_caps_length` | length ≤ 203 (200 + ellipsis) |

Plus the existing 36 M-P2b/M-P2c tests continue to pass without
modification — backwards compatibility preserved.

## 12. Validations

| Check | Result |
|---|---|
| `cargo fmt --check` | clean (after `cargo fmt` auto-format) |
| `cargo clippy --all-targets --all-features -- -D warnings` | clean (after fixing 1 `single_char_add_str` warning → `push_str("…")` → `push('…')`) |
| `cargo build --lib` | clean |
| `cargo test --all-targets --no-fail-fast` | **1096 passed; 0 failed; 0 ignored** (lib 840 + 256 integration); +8 vs M-P2c baseline 1088 |
| `git diff --check` | clean |
| OpenAPI spec | preserved; no spec drift (env loader keys deferred to M-P2e; response shapes unchanged) |
| Sensitive-string scan | zero `AKIA[0-9A-Z]{16}` / KMS ARN / `AWS_ACCESS_KEY_ID=` patterns; URL sanitiser runtime-enforces no `http(s)://` in error envelopes |

## 13. Files changed

| Path | Status |
|---|---|
| `src/api/http.rs` | edited (1 new field on `AppState` + 1 line in constructor) |
| `src/api/trading_views.rs` | edited (`mod tests` → `pub mod tests` so the `ProgrammableMockProvider` fixture is reachable from `trading.rs` tests) |
| `src/api/trading.rs` | edited (rewrote `account_balances` body; added `account_balances_impl<P>` + `sanitise_rpc_err` + 8 tests) |
| `docs/BACKEND_TRADING_API_PHASE_4_RESULT.md` | **new** (this doc) |
| `RUN_STATE.md` | edited |

No other source modified. No new module added. No `.env` change. No
existing handler / service / type / signer / executor / KMS / RemoteSignerClient
touched.

## 14. What is NOT in M-P2d

```text
- env-loader keys (deferred to M-P2e; operator constructs addresses via AppState builder)
- `.env.example` edits
- the remaining 6 partial endpoints (quote_preview / positions / portfolio / exercise_preview / close_preview / series_details)
- 5 additional trading_views helpers (try_get_account_state / try_preview_account_settlement / try_preview_detailed_settlement / try_get_feed / try_has_active_feed)
- AWS / KMS / signer / broadcast / mainnet / sol / frontend touches
- write function ABI bindings (M-P2c constraint maintained)
```

## 15. Blockers

None inside M-P2d. M-P2e's per-handler wire-in is a focused 3-5 day
follow-on; depends on operator-side decision about per-deploy
addresses (anvil-local + Sepolia rehearsal).

## 16. Next milestone recommendation

**Serialised next:** `BACKEND-TRADING-API-IMPLEMENTATION-PHASE-5` (M-P2e)
— wire the remaining 6 partial endpoints using the `account_balances`
pattern + add the 5 missing trading_views helpers. 3-5 days.

**Alternative (parallel):** `E2E-LOCAL-TRADING-FIXES` (M-P4b) per
`E2E_LOCAL_FIXES_NEXT_TASK.md` — Playwright + cycler. Independent of
M-P2e.

**Recommended order:** M-P4b → M-P2e → M-P5 (E2E Sepolia) → M-P6 → M-P7
→ MAINNET-AUDIT-EXT-DISPATCH.

## 17. Cross-links

- `docs/BACKEND_TRADING_API_PHASE_3_RESULT.md` (M-P2c framework)
- `docs/BACKEND_TRADING_API_PHASE_2_RESULT.md` (M-P2b partial responses)
- `docs/BACKEND_TRADING_API_IMPLEMENTATION_RESULT.md` (M-P2a baseline)
- `docs/FRONTEND_TRADING_API_HANDOFF.md` (frontend handoff)
- `docs/openapi/trading-api.openapi.json`
- `~/DEOPT/deopt-v2-sol/abis/freeze-v2-product-rc1/selectors.txt`

**End of M-P2d result.**
