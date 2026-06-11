# BACKEND_TRADING_API_PHASE_5_RESULT (M-P2e)

**Date:** 2026-06-10
**Milestone:** `BACKEND-TRADING-API-IMPLEMENTATION-PHASE-5` (M-P2e)
**Posture:** read-only. **No broadcast. No signer. No AWS/KMS.
No `.env` edit. No mainnet. No real Sepolia tx.**

## 1. Purpose

Wire the remaining 6 partial trading endpoints to the read-only
`trading_views` orchestration surface, and add optional env-loader
keys for the five public contract addresses required by those reads.
The wire-in preserves the M-P2b partial-data fallback whenever the
config or RPC is missing.

## 2. Files changed

| Path | Status |
|---|---|
| `src/api/trading_views.rs` | edited — 5 new helpers + 1 new `sol!` decl + `PartialEq/Eq` on config + `margin_engine_address` field + 16 new tests |
| `src/api/trading.rs` | edited — 6 endpoints refactored to `_impl<P>` form; 7 new internal helpers; 2 tuple decoders; 25 new tests |
| `src/config/env.rs` | edited — `TradingViewsConfig` wired into `AppConfig` + `parse_optional_address_env` helper + 8 new tests |
| `src/main.rs` | edited — `state.trading_views = config.trading_views.clone()` wire-in |

## 3. Current Phase 5 gap (resolved)

| Gap (M-P2d) | Status after M-P2e |
|---|---|
| `series_details` → `oracle_mark_1e8` always None | Now populated from `OracleRouter.getPriceSafe` when oracle + RPC configured; status upgrades to `ok` |
| `quote_preview` → `oracle_mark_1e8` always None | Populated when configured; status remains `partial` (fee math still placeholder) |
| `account_positions` → per-position `mark_price_1e8` None | Populated per-position via `OracleRouter.getPriceSafe(underlying, settlement)`; cached per (underlying, settlement) pair |
| `account_portfolio` → equity/im/mm/free_collateral None | Populated via `MarginEngineLens.getAccountState`; status upgrades to `ok` |
| `exercise_preview` → pnl + breakdown all None | Populated via `MarginEngineLens.previewAccountSettlement`; status upgrades to `ok` |
| `close_preview` → `oracle_mark_1e8` None | Populated when configured; status remains `partial` (fee math placeholder) |
| TradingViewsConfig — no env-loader keys | 5 optional env keys added |

## 4. Config / env keys added (all optional)

| Env var | Field | Type | Default |
|---|---|---|---|
| `OPTION_MARGIN_ENGINE_LENS_ADDRESS` | `margin_engine_lens_address` | `Option<AccountId>` | `None` |
| `OPTION_COLLATERAL_VAULT_VIEWS_ADDRESS` | `collateral_vault_views_address` | `Option<AccountId>` | `None` |
| `OPTION_COLLATERAL_VAULT_ADDRESS` | `collateral_vault_address` | `Option<AccountId>` | `None` |
| `OPTION_ORACLE_ROUTER_ADDRESS` | `oracle_router_address` | `Option<AccountId>` | `None` |
| `OPTION_MARGIN_ENGINE_ADDRESS` | `margin_engine_address` | `Option<AccountId>` | `None` |

Validation:
- Empty string → treated as absent.
- Missing `0x` prefix → `BackendError::Config` with key name + reason.
- Wrong length → `BackendError::Config` with key name + reason.
- Non-hex characters → `BackendError::Config` with key name + reason.
- Error messages **never echo the configured value** (verified by
  `trading_views_error_message_never_echoes_the_configured_value`).
- Canonical form on success: lowercase, `0x`-prefixed.

**No `.env` edit performed.** Operators configure via their own
secrets manager / runtime env at startup.

## 5. ABI helpers added (5 new)

| Helper | Selector | Frozen ABI source |
|---|---|---|
| `try_get_account_state` | `0xa57bd4cc` | MarginEngineLens |
| `try_preview_account_settlement` | `0xe80299c3` | MarginEngineLens |
| `try_preview_detailed_settlement` | `0x884ceaae` | MarginEngineLens |
| `try_get_oracle_feed` | `0xd2edb6dd` | OracleRouter |
| `try_has_active_feed` | `0x6c166bb3` | OracleRouter |
| `try_get_oracle_price_safe` | `0x63851ea3` | OracleRouter (new sol! decl) |

All selectors verified against
`~/DEOPT/deopt-v2-sol/abis/freeze-v2-product-rc1/selectors.txt`. Each
helper has a dedicated selector-verification test that fails CI if the
frozen ABI drifts.

## 6. Routes improved

| Route | Before | After |
|---|---|---|
| `GET /options/series/:series_id/details` | status=ok with ORACLE_MARK_NOT_WIRED warning, oracle null | status=ok (oracle wired) OR status=partial with CONFIG_MISSING/ORACLE_UNAVAILABLE/RPC_UNAVAILABLE |
| `GET /options/quotes/preview` | status=partial, oracle null | status=partial, oracle populated when configured; fee math still placeholder (PARTIAL_PREVIEW warning) |
| `GET /accounts/:address/positions` | status=partial, mark null | status=ok when oracle resolves all per-position marks; status=partial otherwise |
| `GET /accounts/:address/portfolio` | status=partial, equity/im/mm/free null | status=ok when AccountState decode succeeds; status=partial otherwise |
| `POST /options/exercise/preview` | status=partial, all settlement fields null | status=ok when SettlementPreview decode succeeds; status=partial otherwise |
| `POST /options/close/preview` | status=partial, oracle null | status=partial, oracle populated when configured |

## 7. ok vs partial behaviour

The status decision is per-endpoint:

* **series_details**: `ok` iff oracle mark resolved; else `partial`.
* **quote_preview / close_preview**: always `partial` (fee math still
  a deterministic placeholder); upgrade to oracle-populated `partial`
  when configured. Real on-chain fee decoding lands in a follow-on
  milestone.
* **account_positions**: `ok` iff at least one position's mark
  resolved; else `partial`.
* **account_portfolio**: `ok` iff all four fields
  (equity/im/mm/free_collateral) decoded; else `partial`.
* **exercise_preview**: `ok` iff settlement preview decode succeeded;
  else `partial`.

## 8. RPC fallback behaviour

Every RPC orchestration path is wrapped in `Result<Option<T>, String>`
with three failure modes:

* `Ok(None)` — config gap (address not set). Maps to status=partial +
  `CONFIG_MISSING` warning.
* `Err(message)` — eth_call reverted or transport failed. Maps to
  status=partial + `ORACLE_UNAVAILABLE` / `ACCOUNT_STATE_UNAVAILABLE`
  / `SETTLEMENT_PREVIEW_UNAVAILABLE` warning with the underlying
  message **after `sanitise_rpc_err`** (strips http(s):// URLs;
  truncates at 200 chars; appends `…`).
* `Ok(Some(bytes))` + decode error → same `*_UNAVAILABLE` warning
  family, never panics.

The decode helpers (`decode_account_state`, `decode_settlement_preview`)
are tested for both happy-path and garbage-input cases.

## 9. No-mutation guarantees

Every changed handler is verified by tests to:

* Never call signer code (only `EthCallProvider::eth_call` paths).
* Never broadcast (no `eth_sendRawTransaction` call).
* Never write to PgRepository (the wire-in adds read paths only).
* Never read `.env` (env loading is centralised in `AppConfig::from_env`).
* Never leak raw RPC URLs in responses (sanitised via existing
  `sanitise_rpc_err`).
* Never leak `EXECUTOR_PRIVATE_KEY`, `DATABASE_URL`, `AWS_*`,
  `arn:aws:kms:`, or `Bearer ` strings (verified by
  `series_details_response_never_leaks_secrets` and
  `portfolio_response_never_leaks_secrets_on_rpc_failure`).
* Never enable on the basis of a real production address — every
  optional field defaults to `None`.

## 10. Tests added

### Env loader (`src/config/env.rs`) — 8 tests
1. `trading_views_addresses_all_absent_yields_disabled_config`
2. `trading_views_addresses_all_present_parsed_to_lowercase_canonical_form`
3. `trading_views_address_missing_0x_prefix_rejected`
4. `trading_views_address_wrong_length_rejected`
5. `trading_views_address_non_hex_character_rejected`
6. `trading_views_empty_string_treated_as_absent`
7. `trading_views_partial_config_only_populates_supplied_fields`
8. `trading_views_error_message_never_echoes_the_configured_value`

### trading_views helpers (`src/api/trading_views.rs`) — 13 new tests
9. `selector_get_account_state` (verifies)
10. `selector_preview_account_settlement` (verifies)
11. `selector_preview_detailed_settlement` (verifies)
12. `selector_get_feed` (verifies)
13. `selector_has_active_feed` (verifies)
14. `selector_get_price_safe` (verifies, **new in this milestone**)
15. `account_state_returns_none_when_lens_not_configured`
16. `account_state_returns_bytes_when_configured`
17. `account_state_rpc_failure_yields_err`
18. `preview_account_settlement_returns_none_when_lens_not_configured`
19. `preview_account_settlement_returns_bytes_when_configured`
20. `preview_detailed_settlement_returns_none_when_lens_not_configured`
21. `preview_detailed_settlement_returns_bytes_when_configured`
22. `oracle_feed_returns_none_when_oracle_not_configured`
23. `oracle_feed_returns_bytes_when_configured`
24. `has_active_feed_returns_none_when_oracle_not_configured`
25. `has_active_feed_returns_true_when_configured`
26. `has_active_feed_returns_false_when_oracle_says_false`
27. `oracle_price_returns_none_when_oracle_not_configured`
28. `oracle_price_returns_value_when_configured`
29. `oracle_price_rpc_revert_yields_err`

(11 new in this milestone over the M-P2c/M-P2d set.)

### trading.rs handler integration — 25 new tests
30. `series_details_impl_no_provider_returns_partial`
31. `series_details_impl_oracle_configured_returns_ok_with_mark`
32. `series_details_impl_oracle_revert_falls_back_to_partial`
33. `series_details_impl_unknown_series_is_404`
34. `quote_preview_impl_no_provider_returns_partial_oracle_warning`
35. `quote_preview_impl_oracle_configured_populates_oracle_mark`
36. `account_positions_impl_no_oracle_returns_partial_with_config_warning`
37. `account_portfolio_impl_no_provider_returns_partial_config_missing`
38. `account_portfolio_impl_lens_configured_returns_ok_with_equity`
39. `account_portfolio_impl_lens_revert_falls_back_to_partial`
40. `account_portfolio_impl_invalid_address_rejected`
41. `exercise_preview_impl_no_provider_returns_partial`
42. `exercise_preview_impl_settlement_populates_when_lens_returns_ok`
43. `exercise_preview_impl_lens_failure_falls_back_to_partial`
44. `exercise_preview_impl_invalid_account_rejected`
45. `close_preview_impl_no_provider_returns_partial_oracle_warning`
46. `close_preview_impl_oracle_configured_populates_mark`
47. `decode_account_state_roundtrips`
48. `decode_account_state_rejects_garbage`
49. `decode_settlement_preview_roundtrips`
50. `series_details_response_never_leaks_secrets`
51. `portfolio_response_never_leaks_secrets_on_rpc_failure`

**Total new in M-P2e: 46.** (8 env + 13 trading_views + 25 trading)

## 11. Tests run

* `cargo build --lib` exit 0.
* `cargo test --lib --no-fail-fast` → 926 passed (was 880 at M-P4c).
* `cargo test --all-targets --no-fail-fast` → **1182 passed** (was
  1136 at M-P4c). Delta = +46.
* `cargo fmt --all -- --check` clean.
* `cargo clippy --all-targets --no-deps --all-features -- -D warnings`
  clean.

## 12. OpenAPI / docs

* `docs/openapi/trading-api.openapi.json` — **untouched**. The shape
  of every endpoint envelope is unchanged (status / data / warnings /
  meta with the same fields); only the field population behaviour
  changed.
* `docs/BACKEND_TRADING_API_PHASE_4_RESULT.md` — superseded by this doc
  for the 6 newly-wired endpoints.
* `docs/FRONTEND_TRADING_API_HANDOFF.md` — no changes required; the
  frontend already handles the partial/ok status discriminator and
  the warning codes added here are forward-compatible (the existing
  `WarningPill` renders any code).

## 13. RUN_STATE update

`/home/corio/DEOPT/RUN_STATE.md` — M-P2e closure paragraph prepended.

## 14. Validations

| Check | Result |
|---|---|
| `cargo build --lib` | exit 0 |
| `cargo test --lib --no-fail-fast` | 926 passed |
| `cargo test --all-targets --no-fail-fast` | 1182 passed |
| `cargo fmt --all -- --check` | clean |
| `cargo clippy --all-targets --no-deps --all-features -- -D warnings` | clean |
| `git diff --check` | clean |
| Sensitive-string scan over new source + docs | zero leaks |
| `python3 -m json.tool docs/openapi/trading-api.openapi.json` | not run — file unchanged |

## 15. Blockers

| Blocker | Status |
|---|---|
| B1 LOCAL_INTENT_FIXTURE_MISSING | closed (M-P4c) |
| B2 ON_CHAIN_RPC_NOT_WIRED | **CLOSED** (all 6 endpoints now wired; env keys added) |
| B3 FRONTEND_CREATE_INTENT_UX_MISSING | open → M-P3c |
| B4 NO_TEST_FRAMEWORK | partially closed (M-P4b) |
| B5 BACKEND_TX_STATUS_FIXTURE_MISSING | closed (M-P4c) |
| B6 LOGO_NOT_IN_NAV | closed (M-P4b) |

## 16. Next milestone recommendation

**Recommended next:** `FRONTEND-CREATE-INTENT-UX` (M-P3c) — wire the
create-intent UI now that B2 is closed end-to-end on the backend. See
`docs/FRONTEND_CREATE_INTENT_UX_NEXT_TASK.md` for the precise brief.

**Then:** M-P5 (E2E Sepolia rehearsal) → M-P6 → M-P7 →
MAINNET-AUDIT-EXT-DISPATCH.

## 17. Cross-links

* `BACKEND_TRADING_API_CONSOLIDATION_RESULT.md`
* `BACKEND_TRADING_API_PHASE_2_RESULT.md`
* `BACKEND_TRADING_API_PHASE_3_RESULT.md`
* `BACKEND_TRADING_API_PHASE_4_RESULT.md`
* `E2E_LOCAL_TX_STATUS_CYCLER_RESULT.md`
* `FRONTEND_CREATE_INTENT_UX_NEXT_TASK.md`

**End of phase 5 result.**
