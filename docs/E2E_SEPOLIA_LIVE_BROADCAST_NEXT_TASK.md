# E2E Sepolia — Live Broadcast Next Task (M-P5 Phase B)

**Date written:** 2026-06-10
**Origin:** M-P5 Phase A (`E2E_SEPOLIA_TRADING_LIFECYCLE_RESULT.md`)
+ M-P5-FIXES (`E2E_SEPOLIA_FIXES_RESULT.md`)
+ M-P5-RO (`E2E_SEPOLIA_READ_ONLY_CONFIRMATIONS_RESULT.md`).
**Target:** **one** operator-approved Base Sepolia test broadcast.
**Posture:** **Approval-gated. NEVER auto-broadcast. NEVER mainnet.
NEVER Safe tx. NEVER AWS / KMS creation. NEVER production `.env`
edit.**

> **Current readiness (M-P5-RO2, 2026-06-10): NOT READY.**
> On-chain checks executed; 4 blockers refined to actionable
> sub-states (see
> `E2E_SEPOLIA_READONLY_CHECKS_WITH_RPC_RESULT.md` §9). Recommended
> close order: BS-5 oracle refresh → BS-3 governance grant → BS-4
> funding + approves → BS-2 lens deploy. Each step has its own
> operator-approved next-task brief.

> **CRITICAL.** This task does NOT execute. It is a brief for a
> future task that requires explicit operator approval. The
> approval gate is in `E2E_SEPOLIA_LIVE_APPROVAL_GATE.md`. The
> operator MUST type the literal approval line before any broadcast
> is attempted.

## 1. Hard preconditions (ALL must be CONFIRMED)

| # | Precondition | Source |
|---|---|---|
| 1 | `E2E_SEPOLIA_RESOLVED_VALUES_CHECKLIST.md` shows BS-1 CLOSED + BS-2/3/4/5 CONFIRMED | M-P5-FIXES |
| 2 | `E2E_SEPOLIA_LIVE_APPROVAL_GATE.md` P1 … P12 all ✓ | operator |
| 3 | Operator writes the literal approval line: "I approve one Base Sepolia test broadcast for this run." | operator |
| 4 | Backend `chain_id=84532` (NEVER 8453) | startup log |
| 5 | Backend `EXECUTION_ENABLED=true` AND `EXECUTOR_REAL_BROADCAST_ENABLED=true` in `.env.sepolia` (NEVER `.env`) | operator |
| 6 | Mainnet hard-gates intact (4 defence-in-depth layers) | code (unchanged) |

If ANY precondition is not met → **DO NOT BROADCAST**. Switch to the
`E2E_SEPOLIA_REMAINING_OPERATOR_ACTIONS.md` flow.

## 2. Scope — exactly one broadcast

The single broadcast is the `OptionMatchingEngine.executeTrade(...)`
call against the intent minted by the M-P2f
`POST /options/execution-intents` endpoint, signed by buyer + seller
via the M-P3b signing-payload flow.

* Expected tx count: **1**.
* Max gas: 1_000_000 (existing `EXECUTOR_MAX_GAS_LIMIT`).
* Expected revert: NONE.
* Expected events: `OptionTradeExecuted` + fee events.
* Expected on-chain state mutation: option position created on
  Sepolia; collateral debit on seller; premium credit to buyer's
  settlement asset.

## 3. Post-broadcast verification

After the single broadcast confirms:

1. Backend tx-status timeline transitions through SIGNED →
   BROADCAST → CONFIRMED within 60 s.
2. Event indexer observes the `OptionTradeExecuted` event within 30 s.
3. Reconciliation worker reports `match` within 60 s.
4. R5 drift = 0 (re-confirmed).
5. Frontend `/transactions/<intent_id>` shows CONFIRMED with a real
   Sepolia tx hash (NOT a synthetic `0xdeadbee5…`).
6. No `Authorization` header leaks from the frontend trading UI.
7. Mainnet hard-gates still intact (no chain id 8453 ever appears
   in startup logs or response envelopes).

If any check fails → switch to the rollback path in
`E2E_SEPOLIA_LIVE_APPROVAL_GATE.md` §5.

## 4. Deliverables

* `E2E_SEPOLIA_LIVE_BROADCAST_RESULT.md` (new — records the run).
* Updated `RUN_STATE.md` closure paragraph.
* Updated `E2E_SEPOLIA_BLOCKERS_AND_FIXES.md` with the final
  CLOSED / CONFIRMED state of all five blockers.

## 5. Forbidden

* NO mainnet (8453) anywhere.
* NO Safe tx.
* NO new AWS / KMS resource.
* NO new GitHub workflow that touches mainnet.
* NO Solidity modification.
* NO new ABI binding (frozen `v2-product-freeze-rc1` only).
* NO "mainnet-ready" / "audited" claim.
* NO storage of private-key material in any checked-in artefact.
* NO second broadcast under the same approval — each broadcast
  requires a fresh sign-off.

## 6. Hard stops

Stop immediately if:

* RPC reports chain id ≠ 84532.
* `EXECUTOR_REAL_BROADCAST_ENABLED` is observed `false` mid-run.
* Tx receipt status = 0 (revert).
* Reconciliation reports `drift != 0` post-broadcast.
* Any UI surface shows `MainnetDisabledBanner` during the run.

## 7. Cross-links

* `E2E_SEPOLIA_FIXES_RESULT.md` (M-P5-FIXES)
* `E2E_SEPOLIA_RESOLVED_VALUES_CHECKLIST.md`
* `E2E_SEPOLIA_LIVE_APPROVAL_GATE.md`
* `E2E_SEPOLIA_BLOCKERS_AND_FIXES.md`
* `E2E_SEPOLIA_REMAINING_OPERATOR_ACTIONS.md`
* `BACKEND_LIVE_BROADCAST_FLAG_FLIP_RUNBOOK_V2G_FX_Q1_C.md`
* `BACKEND_SIGNER_CUTOVER_RUNBOOK_V2G_FX_Q1.md`
* `~/DEOPT/TESTNET_RUNBOOK.md`

**End of live broadcast next-task brief.**
