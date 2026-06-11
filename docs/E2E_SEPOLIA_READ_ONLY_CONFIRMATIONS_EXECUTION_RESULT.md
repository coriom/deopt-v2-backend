# E2E_SEPOLIA_READ_ONLY_CONFIRMATIONS_EXECUTION_RESULT (M-P5-PROV)

**Date:** 2026-06-10
**Companion to:** `E2E_SEPOLIA_OPERATOR_INPUT_PROVISIONING_RESULT.md`.
**Posture:** **No mainnet. No broadcast. No source code changes.
No `.env` edit. No state mutation.**

## 1. Execution outcome

**SKIPPED — no operator inputs in environment.**

The Sepolia RPC URL (`EXECUTION_RPC_URL` and 3 aliases) and all 5
required private operator-side values are absent from this
environment's shell + filesystem (verified presence-only; no values
printed). The brief's documented fallback path applies:

> "if RPC and required values are present, run read-only checks
>  only … else stop and produce a precise missing-input list."

## 2. Commands NOT run (and why)

The exact commands that would close BS-2 / BS-3 / BS-4 / BS-5 are
defined in `E2E_SEPOLIA_READ_ONLY_CONFIRMATION_LOG.md` §3-§6. None
were executed because each requires `$EXECUTION_RPC_URL` plus one
or more private operator-side addresses (see
`E2E_SEPOLIA_OPERATOR_INPUT_PROVISIONING_RESULT.md` §3).

### BS-2 — MarginEngineLens

* Required: `$OPTION_MARGIN_ENGINE_LENS_ADDRESS`, `$EXECUTION_RPC_URL`.
* Both MISSING → SKIPPED.

### BS-3 — Executor authorization

* Required: `$EXECUTION_RPC_URL`.
* MISSING → SKIPPED. (Executor + matching engine addresses are
  public.)

### BS-4 — Buyer / seller balances

* Required: `$EXECUTION_RPC_URL`, `$COLLATERAL_TOKEN`.
* Both MISSING → SKIPPED. (Buyer / seller addresses are public.)

### BS-5 — Active series + oracle feed

* Required: `$EXECUTION_RPC_URL`.
* MISSING → SKIPPED. (Product registry + oracle router addresses
  are public.)

## 3. Backend read-only API smoke

**SKIPPED.** Backend smoke requires (a) the backend running pointed
at Sepolia (which needs an RPC URL), AND (b) a series identified by
BS-5, AND (c) buyer/seller addresses funded per BS-4. None met.

## 4. Sanitised command summaries (no RPC URL printed)

For the operator's follow-up run, the call-set is:

```bash
# All commands assume the operator has already sourced
# ~/DEOPT/operator-private/sepolia.inputs.private.md so that
# $EXECUTION_RPC_URL and friends are in the shell env.

# BS-3
cast call "$OPTION_MATCHING_ENGINE" \
  "isExecutor(address)(bool)" \
  "$EXECUTOR_ADDRESS" \
  --rpc-url "$EXECUTION_RPC_URL"

# BS-4
cast balance "$BUYER_ADDRESS"  --rpc-url "$EXECUTION_RPC_URL"
cast balance "$SELLER_ADDRESS" --rpc-url "$EXECUTION_RPC_URL"
cast call "$COLLATERAL_TOKEN" "balanceOf(address)(uint256)" "$BUYER_ADDRESS"  --rpc-url "$EXECUTION_RPC_URL"
cast call "$COLLATERAL_TOKEN" "balanceOf(address)(uint256)" "$SELLER_ADDRESS" --rpc-url "$EXECUTION_RPC_URL"
cast call "$COLLATERAL_TOKEN" "allowance(address,address)(uint256)" \
  "$SELLER_ADDRESS" "$OPTION_COLLATERAL_VAULT" --rpc-url "$EXECUTION_RPC_URL"

# BS-5 — bounded loop only
TOTAL=$(cast call "$OPTION_PRODUCT_REGISTRY" "totalSeries()(uint256)" --rpc-url "$EXECUTION_RPC_URL")
for i in $(seq 0 $((TOTAL - 1))); do
  cast call "$OPTION_PRODUCT_REGISTRY" "seriesAt(uint256)(uint256)" "$i" --rpc-url "$EXECUTION_RPC_URL"
done
# Then for the chosen $SERIES_ID:
cast call "$OPTION_PRODUCT_REGISTRY" "getSeries(uint256)" "$SERIES_ID" --rpc-url "$EXECUTION_RPC_URL"
cast call "$OPTION_ORACLE_ROUTER_ADDRESS" "hasActiveFeed(address,address)(bool)" "$UNDERLYING" "$SETTLEMENT" --rpc-url "$EXECUTION_RPC_URL"
cast call "$OPTION_ORACLE_ROUTER_ADDRESS" "getPriceSafe(address,address)(uint256)" "$UNDERLYING" "$SETTLEMENT" --rpc-url "$EXECUTION_RPC_URL"

# BS-2 (if operator has a candidate lens address):
cast code "$OPTION_MARGIN_ENGINE_LENS_ADDRESS" --rpc-url "$EXECUTION_RPC_URL"
```

Every command uses `--rpc-url "$EXECUTION_RPC_URL"` rather than a
literal URL. No `cast send`, no `forge script --broadcast`, no
state-mutating verb. The RPC URL is never printed.

## 5. Decision tree (for the operator's follow-up run)

| BS-3 result | Action |
|---|---|
| `isExecutor == true` | Mark BS-3 CONFIRMED in checklist. |
| `isExecutor == false` | Branch to `SEPOLIA-EXECUTOR-AUTH-GRANT_NEXT_TASK.md`. |
| Call reverts / RPC fails | Mark BS-3 READ_FAILED; investigate. |

| BS-4 result | Action |
|---|---|
| All balances ≥ recommended thresholds + allowance ≥ requirement | Mark BS-4 CONFIRMED. |
| Any ETH < recommended | Branch to `SEPOLIA-BUYER-SELLER-FUNDING_NEXT_TASK.md`. |
| Allowance < requirement | Branch to `SEPOLIA-BUYER-SELLER-FUNDING_NEXT_TASK.md` §3. |

| BS-5 result | Action |
|---|---|
| Active series + `hasActiveFeed == true` + `getPriceSafe` returns nonzero | Mark BS-5 CONFIRMED. |
| No Active series | Branch to `SEPOLIA-ACTIVE-SERIES-ORACLE-SETUP_NEXT_TASK.md` §3. |
| `hasActiveFeed == false` | Branch to `SEPOLIA-ACTIVE-SERIES-ORACLE-SETUP_NEXT_TASK.md` §4. |

| BS-2 result | Action |
|---|---|
| `cast code $LENS` returns non-`0x` | Mark BS-2 CONFIRMED. |
| `cast code $LENS` returns `0x` (no bytecode) | Branch to `SEPOLIA-MARGIN-ENGINE-LENS-DEPLOY_NEXT_TASK.md`. |
| Operator has no lens candidate address | Branch to `SEPOLIA-MARGIN-ENGINE-LENS-DEPLOY_NEXT_TASK.md`. |

## 6. Status outcomes for each blocker

| Blocker | Outcome of this run |
|---|---|
| BS-2 | OPEN — SKIPPED (no operator input in env) |
| BS-3 | OPEN — SKIPPED (no RPC in env) |
| BS-4 | OPEN — SKIPPED (no RPC + no COLLATERAL_TOKEN in env) |
| BS-5 | OPEN — SKIPPED (no RPC in env) |

## 7. Forbidden — what this milestone NEVER did

* No `cast send` invocation.
* No `forge script --broadcast` invocation.
* No state-mutating call.
* No RPC URL printed to stdout / docs.
* No private-file write outside `.gitignore`-protected paths.
* No `.env` edit.
* No backend / frontend rebuild.

## 8. Cross-links

* `E2E_SEPOLIA_OPERATOR_INPUT_PROVISIONING_RESULT.md` (M-P5-PROV main)
* `E2E_SEPOLIA_OPERATOR_INPUT_PRIVATE_TEMPLATE.md`
* `E2E_SEPOLIA_READ_ONLY_CONFIRMATION_LOG.md` (cast-call playbook)
* `E2E_SEPOLIA_READ_ONLY_CONFIRMATIONS_RESULT.md` (M-P5-RO)
* `E2E_SEPOLIA_RESOLVED_VALUES_CHECKLIST.md`
* `E2E_SEPOLIA_BLOCKERS_AND_FIXES.md`
* `E2E_SEPOLIA_LIVE_APPROVAL_GATE.md`
* `SEPOLIA-MARGIN-ENGINE-LENS-DEPLOY_NEXT_TASK.md`
* `SEPOLIA-EXECUTOR-AUTH-GRANT_NEXT_TASK.md`
* `SEPOLIA-BUYER-SELLER-FUNDING_NEXT_TASK.md`
* `SEPOLIA-ACTIVE-SERIES-ORACLE-SETUP_NEXT_TASK.md`

**End of M-P5-PROV execution result.**
