# SEPOLIA-BUYER-SELLER-FUNDING — Next Task Brief

**Date written:** 2026-06-10
**Origin:** BS-4 from M-P5 Phase A; refined by M-P5-PROV.
**Posture:** **operator-side faucet + approve task. NEVER mainnet.
NEVER Safe tx. NEVER mint without owner role. NEVER production
`.env` edit.** Requires its own operator approval before running.

> **This task is NOT executed by the calling milestone. It is a
> brief for a future operator-approved run.**

## 1. Trigger

Any of:
* `cast balance $BUYER_ADDRESS  --rpc-url $EXECUTION_RPC_URL` < 0.01 ETH.
* `cast balance $SELLER_ADDRESS --rpc-url $EXECUTION_RPC_URL` < 0.01 ETH.
* `balanceOf(seller)` < required collateral.
* `allowance(seller, OPTION_COLLATERAL_VAULT)` < required.

## 2. Goal

Bring buyer + seller testnet accounts to a state where a single
`OptionMatchingEngine.executeTrade(...)` call would succeed against
the chosen series.

## 3. Sub-actions

### 3.1 ETH faucet

```bash
# Operator visits the Base Sepolia faucet (e.g.
# https://www.alchemy.com/faucets/base-sepolia or QuickNode equivalent)
# and tops up both accounts to ≥ 0.05 ETH each.
# NO automation in this milestone.
```

### 3.2 Collateral mint or transfer

If the collateral token is the testnet mock supplied by
`script/DeployTestnetAssets.s.sol`, operator runs:

```bash
cd ~/DEOPT/deopt-v2-sol
# Operator confirms the script is the right one for the deployed
# mock token AND that it does NOT touch any mainnet path.
forge script script/PrepareSmokeV1Accounts.s.sol \
  --rpc-url "$EXECUTION_RPC_URL" \
  --private-key "$OWNER_PRIVATE_KEY" \
  --broadcast
```

If the collateral token is a non-mock ERC20, operator transfers
from an existing testnet wallet (out of scope here).

### 3.3 Approve spender

```bash
cast send "$COLLATERAL_TOKEN" \
  "approve(address,uint256)" \
  "$OPTION_COLLATERAL_VAULT" \
  $REQUIRED_AMOUNT_OR_MAX \
  --rpc-url "$EXECUTION_RPC_URL" \
  --private-key "$SELLER_PRIVATE_KEY"

# Repeat for buyer if the flow requires it.
```

## 4. Verification

```bash
cast balance "$BUYER_ADDRESS"  --rpc-url "$EXECUTION_RPC_URL"
cast balance "$SELLER_ADDRESS" --rpc-url "$EXECUTION_RPC_URL"
cast call "$COLLATERAL_TOKEN" "balanceOf(address)(uint256)" "$SELLER_ADDRESS" --rpc-url "$EXECUTION_RPC_URL"
cast call "$COLLATERAL_TOKEN" "allowance(address,address)(uint256)" \
  "$SELLER_ADDRESS" "$OPTION_COLLATERAL_VAULT" --rpc-url "$EXECUTION_RPC_URL"
```

All values must meet the thresholds defined in
`E2E_SEPOLIA_OPERATOR_INPUT_TEMPLATE.md` §4.

## 5. Forbidden

* No mainnet.
* No mint on a production token.
* No `setMinter` / `grantRole` on the test collateral token's
  production deployment.
* No production `.env` edit.
* No private key printed.
* No faucet automation calling a captcha-protected URL — operator
  visits the faucet in a browser.

## 6. Acceptance

* All 4 verification reads return values ≥ thresholds.
* `E2E_SEPOLIA_RESOLVED_VALUES_CHECKLIST.md` BS-4 → CONFIRMED.
* Operator re-runs the read-only confirmation milestone.

## 7. Cross-links

* `E2E_SEPOLIA_OPERATOR_INPUT_PROVISIONING_RESULT.md`
* `E2E_SEPOLIA_BLOCKERS_AND_FIXES.md`
* `E2E_SEPOLIA_OPERATOR_INPUT_TEMPLATE.md` §4 (threshold definitions)
* `deopt-v2-sol/script/PrepareSmokeV1Accounts.s.sol`
* `~/DEOPT/TESTNET_RUNBOOK.md`

**End of brief.**
