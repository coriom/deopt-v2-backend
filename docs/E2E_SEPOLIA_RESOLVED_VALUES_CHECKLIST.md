# E2E Sepolia — Resolved Values Checklist (M-P5 Phase A Follow-up)

**Date:** 2026-06-10
**Posture:** **public-safe summary only.** Status flags only — no
addresses, balances, RPC URLs, or signer values.

## 1. Blocker status (read-only confirmation)

| Blocker | Status (last updated 2026-06-10 by M-P5-RO) | Confirming source |
|---|---|---|
| BS-1 OPTION_COLLATERAL_VAULT_VIEWS_ADDRESS | **CLOSED** — derived from abstract-inheritance discovery; equals OPTION_COLLATERAL_VAULT_ADDRESS | `E2E_SEPOLIA_OPERATOR_INPUT_TEMPLATE.md` §3 (M-P5-FIXES) |
| BS-2 OPTION_MARGIN_ENGINE_LENS_ADDRESS | **CONFIRMED** (2026-06-12) — lens deployed at `0x496A57CF4e0d4F1BC5c00969Ed4C5204072ddA26`; `cast code` confirms bytecode | `SEPOLIA_SETUP_FIXES_PACK_EXECUTION_RESULT.md` §3.4 |
| BS-3 Executor authorisation | **CONFIRMED** (2026-06-12) — `setExecutor(executor,true)` broadcast; `isExecutor==true` post-tx | `SEPOLIA_SETUP_FIXES_PACK_EXECUTION_RESULT.md` §3.2 |
| BS-4 Buyer / seller testnet balances | **CONFIRMED** (2026-06-12) — buyer+seller each hold 100k mUSDC (= `100_000_000_000` at 6 decimals) and have allowance `100_000_000_000` to CollateralVault; ETH practically sufficient at observed Sepolia gas; allowance → OptionMatchingEngine NOT_REQUIRED (settlement routes through vault) | `SEPOLIA_SETUP_FIXES_PACK_EXECUTION_RESULT.md` §3.3 |
| BS-5 Active series + live oracle feed | **CONFIRMED** (2026-06-12) — series #0 primary+secondary `MockPriceSource` refreshed to `300_000_000_000` (= $3000 1e8); `getPriceSafe` returns `(300_000_000_000, fresh_ts, true)`. **Workflow caveat:** feed `maxDelay=60s`; live broadcast must push `setPrice` immediately before broadcast. | `SEPOLIA_SETUP_FIXES_PACK_EXECUTION_RESULT.md` §3.1 |
| BS-6 MatchingEngine / MarginEngine wiring drift | **CLOSED — RETARGET_CONFIRMED** (2026-06-12, later in the day) — Path A applied: private notes retargeted to MatchingEngine `0x5a5EBF9A…` + MarginEngine `0x506cD65a…`; bidirectional wiring verified; `isExecutor(EXECUTOR_ADDRESS)` already `true` on new ME; EIP-712 dry-run produced clean digest + correct signature recovery. Retry brief at `E2E_SEPOLIA_LIVE_BROADCAST_RETRY_NEXT_TASK.md`; fresh broadcast approval line required before the retry. | `SEPOLIA_MATCHING_ENGINE_RETARGET_RESULT.md` |
| **M-P5 Phase B (live Sepolia option execution)** | **COMPLETE** (2026-06-12) — retry tx `0x748c94843cb4cbe31f56c84ceedc7e000a05dac567fa3fe7a1415a0de59b637a` confirmed at block `42750521`, status `1`; `OptionTradeExecuted` event captured; nonces `2 → 3`; positions ±1; vault balance reconciliation + fee accounting clean. | `E2E_SEPOLIA_LIVE_BROADCAST_RETRY_RESULT.md` |
| Backend DB-side projection of the on-chain trade | **CLOSED — RECONCILED** (2026-06-12, later) — backend rebuilt from source; started with broadcast hard-disabled; indexer caught up to block `42752194` and captured 19 events for tx `0x748c9484…` (1 OptionTradeExecuted + 1 TradeExecuted + 2 TradingFeeCharged + 3 InternalTransfer + 12 Synced); shadow `option_execution_intents` + `option_execution_transactions` rows backfilled; reconciliation worker tick produced `status=reconciled`, `decoded_event_count=19`, `trading_fee_event_count=2`, `internal_transfer_event_count=3`, no mismatch. `/admin/options/executions/.../lifecycle` returns `top.status=broadcast_confirmed`, `reconciliation.status=reconciled`, `fees.reconciliation_status=onchain_observed`. | `SEPOLIA_BACKEND_RECONCILIATION_FIX_RESULT.md` |

## 2. Decision summary

Of the five blockers identified in M-P5 Phase A:

* **1 closed** by source-tree analysis (BS-1 — no on-chain check needed).
* **4 remain OPEN** — each requires the operator to run the
  read-only confirmation commands in
  `E2E_SEPOLIA_READ_ONLY_CONFIRMATION_LOG.md` against their private
  `.env.sepolia` and operator-side notes.

**Phase B (live Sepolia broadcast) is NOT yet ready.** It cannot
proceed until BS-2, BS-3, BS-4, BS-5 all report CONFIRMED / CLOSED
in this checklist.

## 3. How an operator updates this checklist

After completing the read-only confirmation runs, the operator
replaces the "OPEN — OPERATOR_*_CHECK_REQUIRED" status with one of:

* **CLOSED** (no action needed; e.g. BS-1).
* **CONFIRMED** (operator ran the read-only check; result was the
  expected success value).
* **OPEN — OPERATOR_ACTION_REQUIRED** (operator ran the check; the
  result requires a separate governance / faucet / deploy task —
  reference the exact follow-up doc).

The operator MUST NOT write actual addresses, balances, or RPC
values into this file. The status flag is sufficient.

## 4. Public / private split

| Type of value | Lives in |
|---|---|
| Status flag (CLOSED / CONFIRMED / OPEN) | this checklist (public) |
| Sepolia address values | `~/DEOPT/operator-private/` + backend `.env.sepolia` |
| RPC URL | `.env.sepolia` (NEVER public) |
| Balances | operator-side notes (NEVER public) |
| Private keys | NEVER in any file — KMS / hardware wallet only |

## 5. Cross-links

* `E2E_SEPOLIA_FIXES_RESULT.md`
* `E2E_SEPOLIA_OPERATOR_INPUT_TEMPLATE.md`
* `E2E_SEPOLIA_READ_ONLY_CONFIRMATION_LOG.md`
* `E2E_SEPOLIA_REMAINING_OPERATOR_ACTIONS.md`
* `E2E_SEPOLIA_BLOCKERS_AND_FIXES.md`
* `E2E_SEPOLIA_LIVE_APPROVAL_GATE.md`

**End of checklist.**
