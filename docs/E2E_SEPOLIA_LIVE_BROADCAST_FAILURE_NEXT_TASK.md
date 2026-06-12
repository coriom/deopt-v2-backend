# E2E-SEPOLIA-LIVE-BROADCAST-FAILURE — Next Task Brief

> **STATUS UPDATE (2026-06-12): Path A APPLIED + CONFIRMED.** BS-6
> is now CLOSED. Private notes retargeted to MatchingEngine
> `0x5a5EBF9A…` + MarginEngine `0x506cD65a…`; bidirectional wiring
> verified; `isExecutor(0x295005fd…)` already `true`; EIP-712
> dry-run produced a clean digest with both buyer/seller
> signatures recovering correctly via ecrecover. Retry brief at
> `E2E_SEPOLIA_LIVE_BROADCAST_RETRY_NEXT_TASK.md` (fresh approval
> line required: "I approve one Base Sepolia test broadcast retry
> for this run."). See `SEPOLIA_MATCHING_ENGINE_RETARGET_RESULT.md`
> for the full no-broadcast remediation record.


**Date written:** 2026-06-12
**Origin:** `E2E_SEPOLIA_LIVE_BROADCAST_RESULT.md` (tx `0xb4e7984f…` reverted `NotAuthorized()` inside the wired MarginEngine).
**Posture:** **State-mutating remediation candidate. NEVER mainnet. NEVER Safe tx. NEVER AWS / KMS creation. NEVER production `.env` edit. Requires its own operator approval before any retry.**

> **This task is NOT executed by the calling milestone. It is a brief for a future operator-approved run.**

---

## 1. Trigger

* Tx `0xb4e7984fdebb803afc969d6d68fc5bd0f1a6898a1a2e11e13391da882d615dfe` on Base Sepolia confirmed at block `42749084` with status `0` (revert).
* Revert selector `0xea8e4eb5` = `NotAuthorized()` raised by **MarginEngine `0x506cD65a63C53c66ab572B9f9dd819B7BfE00D30`** when MatchingEngine `0xf2D1D85cD363Be3bc160d14883C80e7C2c4F420b` called its `applyTrade` selector.

---

## 2. Root cause

Three-way wiring drift between the operator's private notes and Sepolia state:

```
private notes:          0xf2D1D85… (ME) ↔ 0x287Cef47… (MarginEngine)   ← bidirectionally consistent, but ME no longer points at this MarginEngine

on-chain reality:       0xf2D1D85… (ME) → 0x506cD65a… (MarginEngine)   ← one-way: ME points here, MarginEngine doesn't accept
                        0x5a5EBF9A… (ME) ↔ 0x506cD65a… (MarginEngine)  ← bidirectionally consistent — the CORRECT pair
```

`0x506cD65a…` owner = `0xa67f8E8E673ce4bb2Fb563B0e6E9FA8F70E3b588` (not our `0xc35F…`).

`0x5a5EBF9A9CCd7c012518569DE8283982982670f6` is already wired correctly:
* `marginEngine()` = `0x506cD65a…`
* `optionRegistry()` = `0x3d52b033…` (same as our private file)
* `owner()` = `0xa67f8E8E…`
* `isExecutor(0x295005fd…)` = **`true`** (the public testnet executor is already authorised)

---

## 3. Two remediation paths

### Path A — re-target to the correct pair (PREFERRED)

**No state mutation on chain.** Update operator notes + backend env to use the already-correctly-wired MatchingEngine.

| Step | Action | Owner |
|---|---|---|
| A1 | Update `~/DEOPT/private/operator-private/sepolia.inputs.private.env`: set `OPTION_MATCHING_ENGINE=0x5a5EBF9A9CCd7c012518569DE8283982982670f6`. Optionally set `OPTION_MARGIN_ENGINE=0x506cD65a63C53c66ab572B9f9dd819B7BfE00D30`. **Edit via `$EDITOR`; never `cat`; `chmod 600` after.** | operator |
| A2 | Operator obtains the executor private key for `0x295005fd4F311e6691F008D57d32FCFEde844518` (already `isExecutor==true` on the correct ME). Sets `EXECUTOR_PRIVATE_KEY` in the private file. | operator |
| A3 | The previously-computed buyer / seller EIP-712 signatures from the failed broadcast are **invalid** for the new MatchingEngine because the EIP-712 domain separator embeds the verifying-contract address. The harness re-derives the digest via `previewTradeDigest` against `0x5a5EBF9A…` and re-signs with `BUYER_PRIVATE_KEY` and `SELLER_PRIVATE_KEY`. | harness |
| A4 | Confirm `previewTradeValidity` returns 5/5 `true` against the new ME. | harness |
| A5 | Operator types a **new** approval line: "I approve one Base Sepolia test broadcast for this run." | operator |
| A6 | Re-run `E2E-SEPOLIA-LIVE-BROADCAST` pointing at `0x5a5EBF9A…`. Oracle refresh + executeTrade with the new signatures. Broadcaster: `EXECUTOR_PRIVATE_KEY` (= `0x295005fd…`). | harness |

**No governance mutation. No second operator. No Safe tx.**

### Path B — re-wire the legacy MatchingEngine (heavier)

Requires the **other operator** (the one controlling `0xa67f8E8E673ce4bb2Fb563B0e6E9FA8F70E3b588`) to call governance on `0x506cD65a…`:

```bash
cast send 0x506cD65a63C53c66ab572B9f9dd819B7BfE00D30 \
  "setMatchingEngine(address)" 0xf2D1D85cD363Be3bc160d14883C80e7C2c4F420b \
  --rpc-url "$BASE_SEPOLIA_RPC_URL" \
  --private-key "$OTHER_OPERATOR_OWNER_PRIVATE_KEY"
```

After that, the existing setup (stale MatchingEngine `0xf2D1D85…`, EXECUTOR `0x295005fd…` already authorised) would work.

Path B is heavier (cross-operator coordination + ABI verification of the `setMatchingEngine` selector / argument shape on the actual deployed `0x506cD65a…` bytecode — its bytecode prefix `0x6102608060` differs from the frozen `MarginEngine.abi.json` so the selector may differ).

---

## 4. Hard preconditions for either remediation

| # | Precondition |
|---|---|
| P1 | New approval line (verbatim) from the operator. The prior approval was consumed by tx `0xb4e7984f…`. |
| P2 | Chain id `84532` re-confirmed. |
| P3 | Required key(s) PRESENT in subshell only; values never printed. |
| P4 | Path-A: `previewTradeValidity` returns 5/5 `true` against the new ME. |
| P5 | `getPriceSafe` returns nonzero + `ok=true` immediately before the new executeTrade. |
| P6 | Tx-count budget ≤ 5 (2 oracle refresh + 1 executeTrade + ≤2 buffer for refresh recovery). |

---

## 5. Forbidden in the remediation

* No mainnet.
* No Safe tx (Sepolia rehearsal).
* No AWS / KMS creation.
* No production `.env` edit.
* No second executeTrade under the same approval.
* No public posting of any private key.

---

## 6. Hard stops

* `OptionMatchingEngine.marginEngine()` and `MarginEngine.matchingEngine()` must form a closed bidirectional pair before any executeTrade is attempted.
* If Path A's `EXECUTOR_PRIVATE_KEY` derived address does not match `0x295005fd…`, STOP — wrong key.
* If `previewTradeValidity` returns any `false` flag, STOP.
* If `getPriceSafe` is stale at the moment of broadcast, STOP.

---

## 7. Acceptance criteria

After successful remediation:
* New executeTrade tx confirms with status `1`.
* `OptionTradeExecuted` event present in the receipt logs.
* `nonces(BUYER_ADDRESS)` and `nonces(SELLER_ADDRESS)` both increment to `1` on the chosen MatchingEngine.
* `E2E_SEPOLIA_LIVE_BROADCAST_RESULT.md` re-issued (RUN_v2) with the new tx hash.
* BS-6 (wiring drift) marked CLOSED in `E2E_SEPOLIA_RESOLVED_VALUES_CHECKLIST.md`.

---

## 8. Cross-links

* `docs/E2E_SEPOLIA_LIVE_BROADCAST_RESULT.md`
* `docs/E2E_SEPOLIA_LIVE_APPROVAL_GATE.md`
* `docs/SEPOLIA_SETUP_FIXES_PACK_EXECUTION_RESULT.md`
* `~/DEOPT/RUN_STATE.md`
* `~/DEOPT/TESTNET_RUNBOOK.md`

**End of failure-next-task brief.**
