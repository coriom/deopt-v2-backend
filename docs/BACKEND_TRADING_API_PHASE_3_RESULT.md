# BACKEND-TRADING-API-IMPLEMENTATION-PHASE-3 — Result

**Date:** 2026-06-10
**Anchor commit:** sol freeze `d133e2c` / `v2-product-freeze-rc1`
**Posture:** **source code modified** (new module `src/api/trading_views.rs`
+ ABI bindings + read helpers + 20 tests; no handler wire-in yet); no
mainnet broadcast; no `.env` change; no signer call; no AWS / KMS call;
no broadcast.
**Status:** **M-P2c framework landed.** ABI bindings + selector
verification + helper functions + mock provider all in place. Handler
wire-in is deferred to **M-P2d** to keep this milestone surgical + safe.

## 1. What changed

M-P2c lands the **framework** for on-chain read orchestration without
yet wiring any handler. Rationale:

- New `sol!` macros and `EthCallProvider` orchestration are the safest
  to land in isolation + test thoroughly.
- The trading handler wire-in requires per-handler decoding decisions
  (how to merge on-chain decimals into the response's `Option<String>`
  fields) that benefit from a focused milestone.
- The 6 partial endpoints continue to return real data with structured
  warnings; **no regression**.

The deliverables are:

1. New module `src/api/trading_views.rs` (~550 lines).
2. New `TradingViewsConfig` struct with 4 optional address fields.
3. 8 inline `sol!` function declarations against the frozen ABI.
4. 8 selector-verification tests asserting bindings match the frozen
   `selectors.txt` (runtime assertion of `0x` first-4-bytes).
5. 3 helper async functions: `try_get_collateral_tokens` /
   `try_get_balance` / `try_preview_trade_fees`.
6. 12 behavioural tests covering happy / not-configured / RPC-failure /
   malformed-input paths.
7. `ProgrammableMockProvider` re-usable test fixture for future M-P2d
   wire-in.

## 2. Config

`TradingViewsConfig` carries 4 optional addresses:

```rust
pub struct TradingViewsConfig {
    pub margin_engine_lens_address: Option<AccountId>,
    pub collateral_vault_views_address: Option<AccountId>,
    pub collateral_vault_address: Option<AccountId>,
    pub oracle_router_address: Option<AccountId>,
}
```

**All fields default to `None`.** A `None` address routes the handler
to the M-P2b partial-data path (structured warning, no panic).

**No `.env` wiring in M-P2c.** Operator wiring lands with M-P2d once the
per-deploy address inventory is finalised. M-P2c provides the type +
behaviour only. AppState integration is deferred so this milestone's
diff stays narrow.

## 3. ABI helpers added

8 narrow inline `sol!` function declarations co-located in
`src/api/trading_views.rs`:

| sol surface | Selector (verified) | sol! signature |
|---|---|---|
| `MarginEngineLens.getAccountState` | `0xa57bd4cc` | `function getAccountState(address marginEngine, address trader) external view returns (bytes memory)` |
| `MarginEngineLens.previewTradeFees` | `0x6ffe6d79` | `function previewTradeFees(address marginEngine, uint256 optionId, uint128 quantity, uint128 price, address buyer, address seller, bool buyerIsMaker) external view returns (bytes memory)` |
| `MarginEngineLens.previewAccountSettlement` | `0xe80299c3` | `function previewAccountSettlement(address marginEngine, uint256 optionId, address trader) external view returns (bytes memory)` |
| `MarginEngineLens.previewDetailedSettlement` | `0x884ceaae` | `function previewDetailedSettlement(address marginEngine, uint256 optionId, address trader) external view returns (bytes memory)` |
| `CollateralVaultViews.getCollateralTokens` | `0xb58eb63f` | `function getCollateralTokens() external view returns (address[] memory)` |
| `CollateralVault.balances` | `0xc23f001f` | `function balances(address account, address token) external view returns (uint256)` |
| `OracleRouter.getFeed` | `0xd2edb6dd` | `function getFeed(address baseAsset, address quoteAsset) external view returns (bytes memory)` |
| `OracleRouter.hasActiveFeed` | `0x6c166bb3` | `function hasActiveFeed(address baseAsset, address quoteAsset) external view returns (bool)` |

**All selectors verified at PR time** by per-function tests asserting
the first 4 bytes of `abi_encode()` match the frozen ABI's
`methodIdentifiers`. The verification batch:

```bash
python3 -c "
import json
for c, sigs in [('MarginEngineLens', ['getAccountState(address,address)', ...]), ...]:
    d = json.load(open(f'~/DEOPT/deopt-v2-sol/abis/freeze-v2-product-rc1/{c}.abi.json'))
    mi = d.get('methodIdentifiers', {})
    for s in sigs:
        print(f'{c}.{s}: 0x{mi.get(s)}')
"
```

→ all 8 selectors match. Runtime CI gate via:

```
test selector_get_account_state ... ok
test selector_preview_trade_fees ... ok
test selector_preview_account_settlement ... ok
test selector_preview_detailed_settlement ... ok
test selector_get_collateral_tokens ... ok
test selector_balances ... ok
test selector_get_feed ... ok
test selector_has_active_feed ... ok
```

**Write functions: NONE added.** Read-only surface only.

## 4. Read helpers

Three async helpers exposed for handler wire-in (M-P2d):

| Function | Returns | Behaviour |
|---|---|---|
| `try_get_collateral_tokens(cfg, from, provider)` | `Result<Option<Vec<Address>>, String>` | `Ok(None)` if `collateral_vault_views_address` not configured; `Ok(Some(...))` on RPC success; `Err` on RPC failure |
| `try_get_balance(cfg, from, account, token, provider)` | `Result<Option<U256>, String>` | `Ok(None)` if `collateral_vault_address` not configured; `Err` if `account` is malformed hex; otherwise mirrors |
| `try_preview_trade_fees(cfg, from, margin_engine, option_id, qty, price, buyer, seller, buyerIsMaker, provider)` | `Result<Option<Vec<u8>>, String>` | `Ok(None)` if lens not configured; returns raw bytes for caller to decode into engine struct |

Each helper:
- Uses the existing `EthCallProvider` trait — no new trait, no new
  transport.
- Builds `EthCallRequest` with `value: 0`, `gas_limit: None`.
- Calls `abi_encode` from the inline `sol!` macro.
- Decodes return data via `abi_decode_returns` where applicable.
- Surfaces failure as `Err(String)` — caller maps to typed warning +
  envelope.

Future helpers (`try_get_account_state` / `try_preview_account_settlement`
/ `try_preview_detailed_settlement` / `try_get_feed` /
`try_has_active_feed`) follow the same shape; M-P2d adds them when
wiring is needed.

## 5. Endpoint wire-in posture

**No handler wire-in in M-P2c.** The 6 partial endpoints from M-P2b
continue to behave identically: `status: "partial"` + structured
warnings. This is intentional — see §1 rationale.

M-P2d will:

1. Add `pub trading_views: TradingViewsConfig` to `AppState` (single
   constructor edit).
2. Construct a per-request `HttpJsonRpcProvider` from
   `execution_config.rpc_url` (if `Some`).
3. Call the appropriate helper from §4 in each handler.
4. Merge the on-chain value into the response payload where the
   helper returned `Some`.
5. Keep the structured warning when the helper returned `None` (config
   missing) or `Err` (RPC failure).
6. Upgrade `status: "partial"` → `status: "ok"` when all configured
   reads succeed.

## 6. RPC fallback behaviour (spec for M-P2d)

| Scenario | Helper return | Handler response |
|---|---|---|
| Address not configured | `Ok(None)` | `status: "partial"` + existing M-P2b warning |
| Address configured, RPC unreachable | `Err(...)` | `status: "partial"` + new `RPC_UNAVAILABLE` warning |
| Address configured, RPC reachable, decode fail | `Err(...)` | `status: "partial"` + new `PREVIEW_REVERTED` warning + safe summary in `details` |
| Address configured, success | `Ok(Some(...))` | merge into payload; `status: "ok"` if all reads succeed |

No raw RPC URL in any warning. No raw provider error with sensitive
details — `Err` messages get a sanitised wrap before reaching the
response envelope.

## 7. Tests added

**20 new tests** in `src/api/trading_views::tests`:

| Group | Count | Notes |
|---|---|---|
| Selector verification | 8 | one per declared sol function; asserts first 4 bytes of `abi_encode()` match frozen ABI |
| `try_get_collateral_tokens` | 3 | not configured → `Ok(None)`; configured → list returned + decoded; RPC failure → `Err` |
| `try_get_balance` | 4 | not configured / configured success / malformed account hex → `Err` / RPC failure |
| `try_preview_trade_fees` | 3 | not configured / configured raw-bytes / RPC failure |
| `account_to_address` | 2 | valid hex / rejects garbage + short hex |

`ProgrammableMockProvider`: re-usable test fixture (matches the
`ProgrammableEthProvider` pattern from `broadcast_policy_data.rs`).
Selector-rule based; supports both success outputs and failure
injection. Available for M-P2d wire-in tests.

## 8. Validations

| Check | Result |
|---|---|
| `cargo fmt --check` | clean |
| `cargo clippy --all-targets --all-features -- -D warnings` | clean (after fixing 3 trivial warnings: `field_reassign_with_default` → struct-literal init; `type_complexity` → `MockRule` type alias; `too_many_arguments` → `#[allow]` on `try_preview_trade_fees`) |
| `cargo build --lib` | clean |
| `cargo test --all-targets --no-fail-fast` | **1088 passed; 0 failed; 0 ignored** (lib 832 / engine_tests 8 / orderbook_eip712 12 / orderbook 43 / option_execution 67 / option_lifecycle 76 / option_rfq 13 / rfq 37); +20 vs M-P2b baseline 1068 |
| `git diff --check` | clean |
| OpenAPI spec | preserved; no spec drift |
| Sensitive-string scan | zero AKIA / KMS ARN / `AWS_ACCESS_KEY_ID=` / production-EVM patterns in `src/api/trading_views.rs` |

## 9. Files changed

| Path | Status |
|---|---|
| `src/api/trading_views.rs` | **new** (~550 lines: TradingViewsConfig + 8 sol! decls + 3 helpers + 8 selector tests + 12 behaviour tests + `ProgrammableMockProvider` fixture) |
| `src/api/mod.rs` | edited (`pub mod trading_views;`) |
| `docs/BACKEND_TRADING_API_PHASE_3_RESULT.md` | **new** (this doc) |
| `RUN_STATE.md` | edited |

**Production source modified intentionally** — new module + 1-line
re-export. No existing handler / service / type modified. No
`RemoteSignerClient::new` change. No `.env` change. No AWS / KMS /
signer wiring change.

## 10. What is NOT in M-P2c

```text
- AppState wiring (TradingViewsConfig is constructed but not held on AppState yet)
- Handler wire-in (the 6 partial endpoints behave identically to M-P2b)
- HttpJsonRpcProvider per-request construction inside handlers
- `.env` / `.env.example` edits (no operator env wiring)
- Write function bindings (read-only surface only)
- AWS / KMS calls (zero, by design)
- Mainnet anything
- Sepolia broadcasts
```

## 11. M-P2d follow-on scope

The handler wire-in milestone (estimated 3-5 days):

1. Add `pub trading_views: TradingViewsConfig` field to `AppState` (1 constructor edit at `src/api/http.rs`).
2. Add 4-line env wiring in `src/config/env.rs`:
   - `MARGIN_ENGINE_LENS_ADDRESS`
   - `COLLATERAL_VAULT_VIEWS_ADDRESS`
   - `COLLATERAL_VAULT_ADDRESS`
   - `ORACLE_ROUTER_ADDRESS`
   - (all optional; format-validated as 0x-prefixed 20-byte hex)
3. Update `.env.example` with placeholders (no real values).
4. Wire `account_balances` first (simplest): `try_get_collateral_tokens` → loop `try_get_balance` per token.
5. Wire `quote_preview` / `close_preview` next: `try_preview_trade_fees` for fee fields.
6. Wire `account_positions` / `account_portfolio` next: `try_get_account_state` (new helper) returning the lens's account-state struct.
7. Wire `exercise_preview`: `try_preview_account_settlement` + `try_preview_detailed_settlement`.
8. Update OpenAPI examples if `status: "ok"` cases land.
9. Update `FRONTEND_TRADING_API_HANDOFF.md` with new examples.
10. Add 25-35 handler-side integration tests reusing `ProgrammableMockProvider`.

## 12. Blockers

None inside M-P2c. M-P2d depends on operator-side decision about per-deploy
addresses (anvil-local + Sepolia-rehearsal). Decision can be made
asynchronously.

## 13. Next milestone recommendation

**Serialised next:** `BACKEND-TRADING-API-IMPLEMENTATION-PHASE-4` (M-P2d)
— handler wire-in using the M-P2c framework. 3-5 days. Per-deploy
address inventory operator-side prerequisite.

**Alternative:** `E2E-LOCAL-TRADING-FIXES` (M-P4b) per
`E2E_LOCAL_FIXES_NEXT_TASK.md` — Playwright + cycler. Independent of
M-P2d; can run in parallel.

Recommended order: M-P2d → M-P4b → M-P5 (E2E Sepolia).

## 14. Cross-links

- `~/DEOPT/deopt-v2-sol/abis/freeze-v2-product-rc1/selectors.txt`
- `~/DEOPT/deopt-v2-sol/abis/freeze-v2-product-rc1/{MarginEngineLens,CollateralVault,CollateralVaultViews,OracleRouter}.abi.json`
- `~/DEOPT/deopt-v2-sol/docs/SOL_BACKEND_FRONTEND_ABI_HANDOFF.md`
- `docs/BACKEND_TRADING_API_PHASE_2_RESULT.md` (M-P2b: partial-real responses)
- `docs/BACKEND_TRADING_API_IMPLEMENTATION_RESULT.md` (M-P2a: 12-endpoint baseline)
- `docs/openapi/trading-api.openapi.json`
- `docs/E2E_LOCAL_TRADING_LIFECYCLE_RESULT.md` (M-P4)

**End of M-P2c result.**
