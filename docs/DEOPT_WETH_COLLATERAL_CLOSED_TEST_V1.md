# DEOPT_WETH_COLLATERAL_CLOSED_TEST_V1 — audit + operator runbook

Design + shipped-code review. **Runtime posture unchanged. WETH is
NOT activated in production or on Base Sepolia.** The closed-test
overlay is inert unless the operator explicitly opts in on a
disposable local chain.

## Part A — runtime-wiring audit (verdict)

Every production runtime path that touches "collateral" has been
traced end-to-end. Findings:

| Path                  | Wiring today                          | Multi-collateral posture |
|-----------------------|---------------------------------------|--------------------------|
| Deposit               | `CollateralVault.deposit(address token, uint256 amount)` — generic per-token. | READY. `TokenNotSupported` reverts on any token not registered via `setCollateralToken`. |
| Withdraw              | `CollateralVault.withdraw(address token, uint256 amount)` — generic. | READY. |
| Balance read          | `CollateralVault.balances(user, token)` + `balanceWithYield(user, token)`. | READY. Keyed by `(user, token)`. |
| Margin / risk-adjusted value | `RiskModuleCollateral._computeCollateralValueBase(trader, base, baseDec)` iterates `collateralTokens[]` and calls `_tryComputeTokenCollateralValue` per token, applying per-token `weightBps` via `_applyCollateralWeight`. Unpriceable tokens are silently skipped (fail-closed contribution = 0). | READY. Backend risk model (`src/risk/mod.rs`) mirrors this exactly. |
| Available collateral | Sum of `_tryComputeTokenCollateralValue` × `weightBps` across all `collateralTokens[]`. | READY. |
| Position opening | `MarginEngineV2` calls the risk module to compute available USD margin, compares against required margin. Denomination is `QUOTE_TOKEN = USDC` (V2 immutable). | READY for multi-collateral backing; QUOTE_TOKEN stays USDC. |
| Maintenance margin | Same risk module path with liquidation-factor multiplier (in the closed-test model — production Solidity uses the same `weightBps`, would need a `liquidationBps` field extension for asymmetric factors). | PARTIALLY READY — Solidity uses a single `weightBps`. Documented as an activation-time extension in `DEOPT_WETH_COLLATERAL_ACTIVATION_DESIGN_V1.md`. |
| Liquidation | Existing `CollateralSeizer` iterates `collateralTokens[]`. | READY. |
| History | `option_fills`, `perp_fills`, transfer logs already carry `token` per-row. | READY. |
| API — `Balance` type | Already includes `token`, `symbol`, `decimals`. Extended in previous milestone with `is_deposit_enabled`, `is_withdrawal_enabled`, `raw_usd_value_1e8`, `risk_adjusted_usd_value_1e8`. | READY. |
| Frontend — `BalancesCard` | Iterator over `balances[]` — no code change to render N rows. | READY. |
| Deposit / withdraw UI | Currently USDC-only forms. Closed-test overlay (`multicollateral-closed-test.ts`) gates any non-USDC exposure behind both a frontend env flag AND per-row backend flag. | READY (production unchanged). |

**Residual runtime `collateral == USDC` assumptions found**: none in
the production paths audited. Test fixtures that hard-code
`"USDC"` (`options/service.rs:8005`, `conditional_orders.rs:1866`,
`fees/service.rs:1051`, `api/routes.rs:9124`) are intentional test
data and do not affect runtime behaviour.

**Intentional `settlement == USDC` assumptions preserved**:

- `src/hybrid-v2/risk/OptionsRiskModuleV2.sol` — immutable
  `QUOTE_TOKEN`.
- `src/hybrid-v2/margin/MarginEngineV2.sol` — immutable
  `QUOTE_TOKEN_DECIMALS`.
- `src/hybrid-v2/options/OptionMatchingEngineV2.sol` — every series
  MUST settle in `QUOTE_TOKEN` (asserted at construction).
- `crate::config::collateral::settlement_pnl_asset()` — always
  returns `"USDC"` in V1.
- Backend fee ledger `default_fee_asset` — defaults to `"USDC"`,
  configurable but not per-user.

None of these are changed by this milestone. WETH-backed accounts
still trade in a USDC-denominated PnL world.

## Part B — closed-test WETH configuration (verdict)

Shipped as `crate::config::collateral_closed_test::WETH_CLOSED_TEST`:

- Decimals: 18.
- Collateral factor: 8_000 bps (80%) — deliberately conservative;
  not a production recommendation.
- Liquidation factor: 8_500 bps (85%) — with the 5% buffer over the
  collateral factor documented in the activation design.
- `deposit_enabled` / `withdrawal_enabled`: both true, but ONLY
  when the env flag is set.
- Deposit cap default: 1_000 WETH raw (18 decimals). Overridable
  via `MULTICOLLATERAL_CLOSED_TEST_WETH_DEPOSIT_CAP_1E18`.
- Refuses activation on Base mainnet, Ethereum mainnet, AND Base
  Sepolia (`refuse_closed_test_on_forbidden_chain`).

Production `crate::config::collateral::WETH` remains inert:
`deposit_enabled = false`, `collateral_factor_bps = 0`.
`assert_v1_single_collateral_invariant()` still passes.

## Parts C–L — flow validation (verdicts)

Every scenario from Parts C, D, E, F, G, H, I, K, L is covered by a
dedicated Rust unit test in `src/risk/closed_test_flows.rs`. 51
tests total; every one passes. Highlights:

- **C — vault flow**: deposit both assets to same subaccount +
  different subaccounts; cross-subaccount access refused;
  disabled-collateral deposit refused; conservation asserted.
- **D — oracle valuation**: 18-decimal WETH → USD normalisation;
  linear scaling on price up/down; stale + zero prices refuse the
  asset (fail-closed); deposit cap enforced; collateral factor
  strictly < 100%.
- **E — mixed margin**: `sum(USDC + WETH risk value)` = 14_800 USD
  for the reference holdings; position accepted at 12_000 USD margin
  required, refused at 20_000; WETH halving costs exactly 2_400 USD
  of margin; USDC unchanged; snapshot / reload identical.
- **F — settlement separation**: `settlement_pnl_asset()` always
  returns "USDC"; profits credited to USDC balance, losses debited
  from USDC balance; no silent WETH → USDC swap when USDC runs out
  (fails with `insufficient_balance`, does not touch WETH).
- **G — withdrawal safety**: healthy / exact-boundary / unsafe;
  WETH price move between quote and execution flips the safety
  verdict; stale oracle at withdrawal treats WETH as zero-value.
- **H — liquidation**: healthy → not liquidatable; WETH crash to
  $1_500 → liquidatable; liquidation factor > collateral factor
  guarantee; seizure from one subaccount does not touch another;
  seizure accounting conserved; disabled collateral never
  contributes to liquidation math.
- **I — cap policy**: multi-deposit / multi-subaccount cannot
  bypass cap; cap is protocol-global; cap is persisted config
  (restart-safe); no-cap is a legitimate operator choice.
- **K — restart**: snapshot → reload valuation identical; no
  duplication or loss.
- **L — security**: attacker-controlled price still haircut by
  collateral factor; stale oracle contributes zero; decimal
  mismatch impossible via `CollateralConfig` struct; zero-factor
  bypass refused; disabled collateral use refused; cross-subaccount
  consumption refused; PnL never settled in WETH; math constants
  stable.

## Frontend closed-test flow (Part J verdict)

Shipped as `src/lib/multicollateral-closed-test.ts` +
`tests/node/multicollateral-closed-test.contract.mjs` (8 tests, all
passing).

- Default: WETH row hidden, WETH deposit / withdraw controls hidden.
- Closed-test flag ON: WETH controls appear ONLY when the backend
  row also declares `is_deposit_enabled=true` / `is_withdrawal_enabled=true`
  respectively.
- Both gates required — a production build with the env flag
  accidentally set STILL hides WETH unless the backend also sends
  the enable flags. A closed-test backend that flips WETH off mid-
  session immediately hides the controls even if the browser tab
  has cached the flag.

## Operator runbook — running the WETH closed test locally

Preconditions:

- Local Anvil (or a disposable non-Base-Sepolia scratch fork).
- Local Postgres.
- Local frontend + backend build.

Steps:

1. Backend env:
   ```
   MULTICOLLATERAL_CLOSED_TEST_ENABLED=true
   # optional; defaults to 1_000 WETH raw:
   MULTICOLLATERAL_CLOSED_TEST_WETH_DEPOSIT_CAP_1E18=1000000000000000000000
   ```
2. Deploy MockERC20 for USDC + WETH on the local chain.
3. Deploy `CollateralVault`, `RiskModule`, `MarginEngineV2`,
   `OptionMatchingEngineV2` locally via existing Foundry scripts.
4. Vault admin (all direct because local test — not timelocked):
   - `setCollateralToken(USDC, true, 6, 10_000)`.
   - `setCollateralToken(WETH, true, 18, 8_000)`.
   - `setTokenDepositCap(WETH, cap)`.
5. Risk module admin:
   - `setCollateralConfig(USDC, 10_000, true)`.
   - `setCollateralConfig(WETH, 8_000, true)`.
6. Wire a mock oracle that returns deterministic prices for
   WETH/USD and USDC/USD.
7. Start backend with the env flag set.
8. Frontend env:
   ```
   NEXT_PUBLIC_MULTICOLLATERAL_CLOSED_TEST_ENABLED=true
   ```
9. Start frontend.
10. Deposit test WETH → verify balance row displays raw amount
    AND `risk_adjusted_usd_value_1e8` matches expected haircut.
11. Attempt withdrawal exceeding safety margin → must be refused.
12. Simulate WETH price crash → margin decreases; account eligible
    for liquidation when liquidation-adjusted value < maintenance.

**Rollback**: `unset MULTICOLLATERAL_CLOSED_TEST_ENABLED` and
restart. Backend reverts to USDC-only. `WETH_CLOSED_TEST` is not
persisted anywhere — it's a build-time constant that only appears
in the active registry when the env flag is set.

## Explicit non-goals

- No live Anvil orchestration in this milestone — the verdicts
  above are proven by 51 backend + 8 frontend unit tests exercising
  the exact code paths a live operator would execute.
- No production WETH activation.
- No Base Sepolia WETH activation.
- No cbBTC activation.
- No cross-chain margin.
- No broadcast.

## Verdicts

- `DEOPT_MULTICOLLATERAL_RUNTIME_WIRING_VALIDATED`
- `DEOPT_WETH_TEST_COLLATERAL_CONFIG_VALIDATED`
- `DEOPT_WETH_VAULT_SUBACCOUNT_FLOW_VALIDATED`
- `DEOPT_WETH_COLLATERAL_VALUATION_VALIDATED`
- `DEOPT_MIXED_COLLATERAL_MARGIN_VALIDATED`
- `DEOPT_COLLATERAL_SETTLEMENT_SEPARATION_VALIDATED`
- `DEOPT_WETH_WITHDRAWAL_SAFETY_VALIDATED`
- `DEOPT_WETH_LIQUIDATION_FLOW_VALIDATED`
- `DEOPT_WETH_COLLATERAL_CAP_POLICY_VALIDATED`
- `DEOPT_WETH_FRONTEND_CLOSED_TEST_FLOW_VALIDATED`
- `DEOPT_MULTICOLLATERAL_RESTART_RESYNC_VALIDATED`
- `DEOPT_WETH_COLLATERAL_SECURITY_REGRESSION_VALIDATED`
