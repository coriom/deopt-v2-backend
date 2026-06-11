# SEPOLIA-EXECUTOR-AUTH-GRANT — Next Task Brief

**Date written:** 2026-06-10
**Origin:** BS-3 from M-P5 Phase A; refined by M-P5-PROV.
**Posture:** **state-mutating governance task. NEVER mainnet.
NEVER Safe tx. NEVER AWS / KMS creation. NEVER production `.env`
edit.** Requires its own operator approval before running.

> **This task is NOT executed by the calling milestone. It is a
> brief for a future operator-approved run.**

## 1. Trigger

* BS-3 is OPEN AND
* `cast call OPTION_MATCHING_ENGINE "isExecutor(address)(bool)"
  EXECUTOR_ADDRESS --rpc-url $EXECUTION_RPC_URL` returns `false`.

## 2. Goal

Authorise the executor address on `OPTION_MATCHING_ENGINE` so it
can call the trade-execution entry point on Base Sepolia.

## 3. Scope

* Single state-mutating call on Sepolia.
* Sepolia chain id 84532 only — mainnet hard-stop.
* The authoriser is the matching engine's owner (likely
  OPS_MULTISIG or DEPLOYER, depending on the deployment phase).
* Result: `isExecutor(executor) == true`.

## 4. Tooling

The exact entry point depends on the OptionMatchingEngine ABI —
typical patterns:

```solidity
function setExecutor(address account, bool authorized) external onlyOwner;
// OR
function grantRole(bytes32 role, address account) external onlyAdmin;
```

Operator confirms the exact signature from the frozen ABI at
`~/DEOPT/deopt-v2-sol/abis/freeze-v2-product-rc1/OptionMatchingEngine.abi.json`
before running.

```bash
cast send "$OPTION_MATCHING_ENGINE" \
  "setExecutor(address,bool)" "$EXECUTOR_ADDRESS" true \
  --rpc-url "$EXECUTION_RPC_URL" \
  --private-key "$OWNER_PRIVATE_KEY"
```

## 5. Verification

```bash
cast call "$OPTION_MATCHING_ENGINE" \
  "isExecutor(address)(bool)" \
  "$EXECUTOR_ADDRESS" \
  --rpc-url "$EXECUTION_RPC_URL"
# Expect: true
```

## 6. Forbidden

* No mainnet.
* No Safe tx (Sepolia rehearsal uses owner key, not the production
  multisig flow).
* No AWS / KMS creation.
* No production `.env` edit.
* No private key printed.
* No "grant on all engines" — single-engine target only.

## 7. Acceptance

* Verification call returns `true`.
* `E2E_SEPOLIA_RESOLVED_VALUES_CHECKLIST.md` BS-3 → CONFIRMED.
* Operator re-runs the read-only confirmation milestone.

## 8. Cross-links

* `E2E_SEPOLIA_OPERATOR_INPUT_PROVISIONING_RESULT.md`
* `E2E_SEPOLIA_BLOCKERS_AND_FIXES.md`
* `~/DEOPT/deopt-v2-sol/abis/freeze-v2-product-rc1/OptionMatchingEngine.abi.json`
* `~/DEOPT/TESTNET_RUNBOOK.md`

**End of brief.**
