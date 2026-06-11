# SEPOLIA-ACTIVE-SERIES-ORACLE-SETUP — Next Task Brief

**Date written:** 2026-06-10
**Origin:** BS-5 from M-P5 Phase A; refined by M-P5-PROV.
**Posture:** **state-mutating governance task. NEVER mainnet.
NEVER Safe tx. NEVER AWS / KMS creation. NEVER production `.env`
edit.** Requires its own operator approval before running.

> **This task is NOT executed by the calling milestone. It is a
> brief for a future operator-approved run.**

## 1. Trigger

* BS-5 is OPEN AND
* `OptionProductRegistry.totalSeries()` is 0 OR no series returned
  by the bounded walk has `isActive == true` AND
  `OracleRouter.hasActiveFeed(underlying, settlement) == true`.

## 2. Goal

Reach a state where at least one option series is Active with a
live oracle feed on Base Sepolia.

## 3. Sub-action A — Activate an existing series

If `seriesAt(i)` returns a series that is `Inactive` or `Disabled`,
operator activates it:

```bash
cast send "$OPTION_PRODUCT_REGISTRY" \
  "setSeriesActive(uint256,bool)" "$SERIES_ID" true \
  --rpc-url "$EXECUTION_RPC_URL" \
  --private-key "$OWNER_PRIVATE_KEY"
```

## 4. Sub-action B — Create a series

If no series exists, operator creates one via:

```bash
cast send "$OPTION_PRODUCT_REGISTRY" \
  "createSeries(address,address,uint64,uint64,uint128,bool,bool)" \
  "$UNDERLYING" "$SETTLEMENT" \
  "$EXPIRY_TIMESTAMP" "$STRIKE_1E8" \
  "$CONTRACT_SIZE_1E8" \
  true \  # is_call
  true \  # is_european (or false for american)
  --rpc-url "$EXECUTION_RPC_URL" \
  --private-key "$SERIES_CREATOR_PRIVATE_KEY"
```

## 5. Sub-action C — Register or refresh oracle feed

If `hasActiveFeed(underlying, settlement) == false`, operator
registers the feed via the existing OracleRouter ops flow:

```bash
# The exact entry point depends on the OracleRouter ABI — operator
# confirms before running.
cast send "$OPTION_ORACLE_ROUTER_ADDRESS" \
  "setFeed(address,address,address,uint256)" \
  "$UNDERLYING" "$SETTLEMENT" "$FEED_ADDRESS" "$MAX_AGE_SECS" \
  --rpc-url "$EXECUTION_RPC_URL" \
  --private-key "$OWNER_PRIVATE_KEY"
```

If the feed is registered but stale (`getPriceSafe` reverts),
operator refreshes via the feed's update entry point (chain-link
push, push oracle adapter — depends on the feed type).

## 6. Verification

```bash
# Active series:
cast call "$OPTION_PRODUCT_REGISTRY" "getSeries(uint256)" "$SERIES_ID" --rpc-url "$EXECUTION_RPC_URL"

# Oracle:
cast call "$OPTION_ORACLE_ROUTER_ADDRESS" "hasActiveFeed(address,address)(bool)" "$UNDERLYING" "$SETTLEMENT" --rpc-url "$EXECUTION_RPC_URL"
cast call "$OPTION_ORACLE_ROUTER_ADDRESS" "getPriceSafe(address,address)(uint256)" "$UNDERLYING" "$SETTLEMENT" --rpc-url "$EXECUTION_RPC_URL"
```

Both reads must succeed; `hasActiveFeed` returns `true`;
`getPriceSafe` returns a nonzero uint256.

## 7. Forbidden

* No mainnet.
* No Safe tx (Sepolia rehearsal uses owner key, not the production
  governance flow).
* No AWS / KMS creation.
* No production `.env` edit.
* No private key printed.
* No mass series creation — single target series only.

## 8. Acceptance

* Verification reads succeed with expected values.
* `E2E_SEPOLIA_RESOLVED_VALUES_CHECKLIST.md` BS-5 → CONFIRMED.
* Operator re-runs the read-only confirmation milestone.

## 9. Cross-links

* `E2E_SEPOLIA_OPERATOR_INPUT_PROVISIONING_RESULT.md`
* `E2E_SEPOLIA_BLOCKERS_AND_FIXES.md`
* `~/DEOPT/deopt-v2-sol/abis/freeze-v2-product-rc1/OptionProductRegistry.abi.json`
* `~/DEOPT/deopt-v2-sol/abis/freeze-v2-product-rc1/OracleRouter.abi.json`
* `~/DEOPT/TESTNET_RUNBOOK.md`

**End of brief.**
