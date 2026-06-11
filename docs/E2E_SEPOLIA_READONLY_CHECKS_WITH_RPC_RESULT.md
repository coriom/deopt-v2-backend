# E2E_SEPOLIA_READONLY_CHECKS_WITH_RPC_RESULT (M-P5-RO2)

**Date:** 2026-06-10
**Milestone:** `SEPOLIA-READONLY-CHECKS-WITH-RPC` (M-P5-PROV
follow-up).
**Posture:** **Read-only Sepolia. No broadcast. No `cast send`. No
deploy. No signer. No mainnet. No state mutation. No RPC URL or
private key printed at any point.**

## 1. Purpose

With the private operator input file now populated
(`~/DEOPT/private/operator-private/sepolia.inputs.private.env`, mode
`600`, outside all git repos), execute the BS-2 / BS-3 / BS-4 /
BS-5 read-only on-chain checks. Record outcomes as status flags
only; never expose RPC values, exact balances, or feed values.

## 2. Private input handling

Presence-only inventory (no values printed):

| Variable | Status |
|---|---|
| `BASE_SEPOLIA_RPC_URL` | PRESENT |
| `EXECUTION_RPC_URL` | PRESENT |
| `OPTION_PRODUCT_REGISTRY` | PRESENT |
| `OPTION_MATCHING_ENGINE` | PRESENT |
| `OPTION_MARGIN_ENGINE` | PRESENT |
| `OPTION_COLLATERAL_VAULT` | PRESENT |
| `OPTION_ORACLE_ROUTER` | PRESENT |
| `COLLATERAL_TOKEN` | PRESENT |
| `BUYER_ADDRESS` | PRESENT |
| `SELLER_ADDRESS` | PRESENT |
| `EXECUTOR_ADDRESS` | PRESENT |
| `OPTION_MARGIN_ENGINE_LENS_ADDRESS` | **EMPTY** |
| `ACTIVE_OPTION_SERIES_ID` | **EMPTY** |

The private file was sourced into a subshell only; its contents
were never echoed to stdout.

## 3. Chain id confirmation

`cast chain-id --rpc-url "$BASE_SEPOLIA_RPC_URL"` returned **84532**
(Base Sepolia). Mainnet hard-stop confirmed not triggered.

## 4. BS-2 — MarginEngineLens

| Check | Result |
|---|---|
| `OPTION_MARGIN_ENGINE_LENS_ADDRESS` present | **EMPTY** |
| `cast code` probe | NOT RUN (no address) |
| Status | **OPEN — DEPLOYMENT_REQUIRED** |

The lens contract is a concrete standalone deployment
(`contract MarginEngineLens is MarginEngineTypes`). No deploy script
in `deopt-v2-sol/script/`. No Sepolia address in any private
operator note that was loaded. Next-task brief:
`SEPOLIA_MARGIN_ENGINE_LENS_DEPLOY_NEXT_TASK.md` (existing,
M-P5-PROV).

## 5. BS-3 — Executor authorization

| Check | Result |
|---|---|
| `OPTION_MATCHING_ENGINE` bytecode | OK (verified M-P5-PROV) |
| `cast call isExecutor(executor)` | **false** |
| Status | **OPEN — GOVERNANCE_ACTION_REQUIRED** |

The executor address (from the populated private file) is **not
authorized** on the deployed Sepolia `OptionMatchingEngine`.
Next-task brief: `SEPOLIA_EXECUTOR_AUTH_GRANT_NEXT_TASK.md`
(existing; will need refresh).

## 6. BS-4 — Balances and allowances

| Check | Bucketed result |
|---|---|
| BUYER ETH | **LOW_BALANCE** (< 0.01 ETH threshold) |
| SELLER ETH | **LOW_BALANCE** (< 0.01 ETH threshold) |
| COLLATERAL_TOKEN symbol | `mUSDC` (verified) |
| COLLATERAL_TOKEN decimals | `6` (verified) |
| BUYER mUSDC balance | LOW_BALANCE |
| SELLER mUSDC balance | LOW_BALANCE |
| BUYER allowance → CollateralVault | LOW_ALLOWANCE |
| SELLER allowance → CollateralVault | LOW_ALLOWANCE |
| BUYER allowance → OptionMatchingEngine | LOW_ALLOWANCE |
| SELLER allowance → OptionMatchingEngine | LOW_ALLOWANCE |
| Status | **OPEN — LOW_BALANCE + LOW_ALLOWANCE (full pre-fund needed)** |

Exact balances and allowances were **not printed** — only the
bucket flags. Next-task brief:
`SEPOLIA_BUYER_SELLER_FUNDING_NEXT_TASK.md` (existing; covers
faucet → mint/transfer → approve flow).

## 7. BS-5 — Active series and oracle feed

| Check | Result |
|---|---|
| `OptionProductRegistry.totalSeries()` | **2** |
| Bounded scan range | `[0, 2)` |
| Active unsettled series found | **2 (both)** |
| Candidate series #0 | id `2414…0183` (call option on `0x4dee…`, settlement = mUSDC) |
| Candidate series #1 | id `6165…1572` (call option on `0x9d87…`, settlement = mUSDC) |
| Series #0 — `hasActiveFeed(under, settle)` | **true** |
| Series #0 — `getPriceSafe(under, settle)` | **0** (stale / not pushed) |
| Series #1 — `hasActiveFeed(under, settle)` | **true** |
| Series #1 — `getPriceSafe(under, settle)` | **0** (stale / not pushed) |
| Status | **OPEN — ORACLE_FEED_MISSING (price=0 on both candidates)** |

Both candidate series are unsettled and have oracle feeds
registered, but `getPriceSafe` returns `0` for both — the mock
oracle has not had a recent price pushed. Operator runs
`deopt-v2-sol/script/RefreshLocalMockFeeds.s.sol` (or equivalent)
to refresh the feeds. Next-task brief:
`SEPOLIA_ACTIVE_SERIES_ORACLE_SETUP_NEXT_TASK.md` (existing; needs
refresh to call out the price=0 sub-state).

## 8. Backend read-only API smoke

**SKIPPED.** Phase E gates on BS-3, BS-4, BS-5 all CLOSED. With 3
OPEN rows, running the smoke would produce only partial outputs
with predictable warnings; no signal worth recording. Skipped per
the brief's "only if BS-3, BS-4, and BS-5 are CLOSED or sufficiently
confirmed" clause.

## 9. Outcome summary

| Blocker | Status before M-P5-RO2 | Status after M-P5-RO2 |
|---|---|---|
| BS-1 | CLOSED (M-P5-FIXES) | CLOSED |
| BS-2 | OPEN — OPERATOR_INPUT_REQUIRED | **OPEN — DEPLOYMENT_REQUIRED** (refined) |
| BS-3 | OPEN — OPERATOR_READ_ONLY_CHECK_REQUIRED | **OPEN — GOVERNANCE_ACTION_REQUIRED** (`isExecutor==false`) |
| BS-4 | OPEN — OPERATOR_READ_ONLY_CHECK_REQUIRED | **OPEN — LOW_BALANCE + LOW_ALLOWANCE** (full pre-fund needed) |
| BS-5 | OPEN — OPERATOR_READ_ONLY_CHECK_REQUIRED | **OPEN — ORACLE_FEED_MISSING** (feeds registered, but `getPriceSafe == 0`) |

Live broadcast gate remains **NOT MET**. Each row now has a precise
operator-actionable next step.

## 10. Docs created / updated

| Path | Status |
|---|---|
| `docs/E2E_SEPOLIA_READONLY_CHECKS_WITH_RPC_RESULT.md` | new (this doc) |
| `docs/E2E_SEPOLIA_RESOLVED_VALUES_CHECKLIST.md` | edited (status flags refreshed with M-P5-RO2 results) |
| `docs/E2E_SEPOLIA_READ_ONLY_CONFIRMATION_LOG.md` | edited (cross-link banner) |
| `docs/E2E_SEPOLIA_BLOCKERS_AND_FIXES.md` | edited (cross-link banner) |
| `docs/E2E_SEPOLIA_REMAINING_OPERATOR_ACTIONS.md` | edited (cross-link banner) |
| `docs/E2E_SEPOLIA_LIVE_APPROVAL_GATE.md` | edited (status banner refreshed) |
| `docs/E2E_SEPOLIA_LIVE_BROADCAST_NEXT_TASK.md` | edited (status banner refreshed) |
| `docs/SEPOLIA-MARGIN-ENGINE-LENS-DEPLOY_NEXT_TASK.md` | unchanged (still applies) |
| `docs/SEPOLIA-EXECUTOR-AUTH-GRANT_NEXT_TASK.md` | unchanged (`isExecutor==false` confirmed; brief applies as-is) |
| `docs/SEPOLIA-BUYER-SELLER-FUNDING_NEXT_TASK.md` | unchanged (LOW_BALANCE + LOW_ALLOWANCE confirmed; brief applies as-is) |
| `docs/SEPOLIA-ACTIVE-SERIES-ORACLE-SETUP_NEXT_TASK.md` | unchanged (feed-price-zero sub-state covered by §5 / §6) |
| `~/DEOPT/RUN_STATE.md` | edited (closure paragraph) |

## 11. RUN_STATE update

M-P5-RO2 closure paragraph prepended above M-P5-PROV.

## 12. Files changed

**Source code: ZERO.** Only docs + the existing private file
(unchanged in this milestone — it was populated by the prior
SEPOLIA-RPC-ENV-DISCOVERY-AND-PRIVATE-INPUT-FILL milestone).

## 13. Validations

| Check | Result |
|---|---|
| `git diff --check` | clean |
| `git status --short` | only intended docs |
| Sensitive-string scan on new doc | zero hits |
| `.env` modified? | NO (timestamp preserved) |
| Private input file modified? | NO (read-only this milestone) |
| Private input file tracked? | NO (verified in M-P5-PROV; unchanged) |
| RPC URL printed? | **NO** (subshell-scoped; bucket flags only) |
| Private key handled? | **NO** |
| Mainnet RPC used? | **NO** (chain id 84532 confirmed) |
| State-mutating command run? | **NO** |
| `cast send` invocation? | **NO** |
| `forge script --broadcast` invocation? | **NO** |

## 14. Remaining blockers

| Blocker | Status | Close path |
|---|---|---|
| BS-2 OPTION_MARGIN_ENGINE_LENS unknown | OPEN — DEPLOYMENT_REQUIRED | `SEPOLIA_MARGIN_ENGINE_LENS_DEPLOY_NEXT_TASK.md` |
| BS-3 Executor authorisation `false` | OPEN — GOVERNANCE_ACTION_REQUIRED | `SEPOLIA_EXECUTOR_AUTH_GRANT_NEXT_TASK.md` |
| BS-4 LOW_BALANCE + LOW_ALLOWANCE | OPEN | `SEPOLIA_BUYER_SELLER_FUNDING_NEXT_TASK.md` |
| BS-5 ORACLE_FEED_MISSING (price=0) | OPEN | `SEPOLIA_ACTIVE_SERIES_ORACLE_SETUP_NEXT_TASK.md` |

## 15. Next milestone recommendation

Operator picks ONE of the 4 state-mutating fix tasks (each requires
its own operator approval line), preferring the order:

1. **`SEPOLIA_ACTIVE_SERIES_ORACLE_SETUP_NEXT_TASK.md`** — refresh
   the mock oracle feed so `getPriceSafe` returns nonzero. Cheapest
   action; required for any quote preview / signing payload that
   relies on the oracle.
2. **`SEPOLIA_EXECUTOR_AUTH_GRANT_NEXT_TASK.md`** — owner / governance
   call to `setExecutor(executor, true)`. Required for the live
   broadcast itself.
3. **`SEPOLIA_BUYER_SELLER_FUNDING_NEXT_TASK.md`** — faucet ETH +
   mint mUSDC + approve CollateralVault + approve OptionMatchingEngine.
4. **`SEPOLIA_MARGIN_ENGINE_LENS_DEPLOY_NEXT_TASK.md`** — one-shot
   `forge create` for the lens (or operator supplies the address if
   a prior Sepolia deployment exists in private notes).

After each fix, operator re-runs `SEPOLIA-READONLY-CHECKS-WITH-RPC`
(this milestone) to advance the corresponding BS row to CONFIRMED.
Once all 4 BS rows CONFIRMED, the live approval gate flips to READY
and the operator can proceed to `E2E-SEPOLIA-LIVE-BROADCAST` with
the literal approval line.

## 16. Cross-links

* `E2E_SEPOLIA_OPERATOR_INPUT_PROVISIONING_RESULT.md` (M-P5-PROV)
* `E2E_SEPOLIA_READ_ONLY_CONFIRMATIONS_RESULT.md` (M-P5-RO)
* `E2E_SEPOLIA_FIXES_RESULT.md` (M-P5-FIXES)
* `E2E_SEPOLIA_RESOLVED_VALUES_CHECKLIST.md`
* `E2E_SEPOLIA_BLOCKERS_AND_FIXES.md`
* `E2E_SEPOLIA_LIVE_APPROVAL_GATE.md`
* `E2E_SEPOLIA_LIVE_BROADCAST_NEXT_TASK.md`
* `SEPOLIA-MARGIN-ENGINE-LENS-DEPLOY_NEXT_TASK.md`
* `SEPOLIA-EXECUTOR-AUTH-GRANT_NEXT_TASK.md`
* `SEPOLIA-BUYER-SELLER-FUNDING_NEXT_TASK.md`
* `SEPOLIA-ACTIVE-SERIES-ORACLE-SETUP_NEXT_TASK.md`
* `~/DEOPT/TESTNET_RUNBOOK.md`

**End of M-P5-RO2 result.**
