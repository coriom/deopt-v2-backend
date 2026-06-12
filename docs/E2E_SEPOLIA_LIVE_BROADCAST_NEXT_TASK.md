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

> **READY FOR OPERATOR APPROVAL (2026-06-12).** The setup-fixes
> pack executed successfully; all 4 BS rows now CLOSED on chain.
> Lens deployed at `0x496A57CF4e0d4F1BC5c00969Ed4C5204072ddA26`.
> Operator may type the §1 P3 approval line below to authorise the
> single `executeTrade` broadcast. **Critical workflow constraint:**
> the oracle mock has `maxDelay=60s` — before this milestone
> broadcasts `executeTrade`, the harness MUST push
> `MockPriceSource.setPrice` on primary `0x3eb9cdd2C2115c3f0DF5E30da53D7245F9a5f6Cc`
> AND secondary `0x2103a84C0CAB9cf7680d602C8931FaDeD7064517` within
> ~30 s of the broadcast (2 prep txs + `executeTrade` = 3 total).
> See `SEPOLIA_SETUP_FIXES_PACK_EXECUTION_RESULT.md`.

> **ATTEMPT 2026-06-12: REVERTED.** Live broadcast tx
> `0xb4e7984fdebb803afc969d6d68fc5bd0f1a6898a1a2e11e13391da882d615dfe`
> reverted `NotAuthorized()` inside MarginEngine `0x506cD65a…`
> (wiring drift; private MatchingEngine `0xf2D1D85…` no longer
> authorised at the downstream MarginEngine). Approval CONSUMED;
> no retry under the same line. Remediation paths in
> `E2E_SEPOLIA_LIVE_BROADCAST_FAILURE_NEXT_TASK.md`. The brief in
> the rest of this doc still describes the intended flow once
> BS-6 is closed via Path A (re-target to MatchingEngine
> `0x5a5EBF9A…`) or Path B (re-wire `0x506cD65a…`).

> **RETARGET CONFIRMED 2026-06-12.** Path A applied; BS-6 CLOSED.
> Private notes now target the new ME / MarginEngine pair. The
> retry milestone is `E2E-SEPOLIA-LIVE-BROADCAST-RETRY`; brief at
> `E2E_SEPOLIA_LIVE_BROADCAST_RETRY_NEXT_TASK.md`. The retry
> requires the **fresh** approval line "I approve one Base Sepolia
> test broadcast retry for this run." plus operator supply of
> `EXECUTOR_PRIVATE_KEY` for `0x295005fd…`. The rest of this
> doc (preconditions, scope, post-broadcast checks) still applies
> with these address updates:
> * `OPTION_MATCHING_ENGINE` → `0x5a5EBF9A9CCd7c012518569DE8283982982670f6`
> * `OPTION_MARGIN_ENGINE` → `0x506cD65a63C53c66ab572B9f9dd819B7BfE00D30`
> * Broadcaster → `EXECUTOR_PRIVATE_KEY` (NOT owner)
> * Trade tuple nonces → `(buyer=2, seller=2)` (current on-chain
>   values on new ME)

> **M-P5 PHASE B COMPLETE (2026-06-12).** Retry tx
> `0x748c94843cb4cbe31f56c84ceedc7e000a05dac567fa3fe7a1415a0de59b637a`
> confirmed at block `42750521`, status `1`, gas `683_044`. The
> live Sepolia option-execution lifecycle is end-to-end demonstrated.
> The rest of this doc remains as the canonical reference for the
> broadcast surface (preconditions, expected events, hard stops).
> See `E2E_SEPOLIA_LIVE_BROADCAST_RETRY_RESULT.md` for the full
> result record.

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
