# SEPOLIA-MARGIN-ENGINE-LENS-DEPLOY — Next Task Brief

**Date written:** 2026-06-10
**Origin:** BS-2 from M-P5 Phase A; refined by M-P5-PROV.
**Posture:** **state-mutating Sepolia deploy task. NEVER mainnet.
NEVER Safe tx. NEVER AWS / KMS creation. NEVER production `.env`
edit.** Requires its own operator approval before running.

> **This task is NOT executed by the calling milestone. It is a
> brief for a future operator-approved run.**

## 1. Trigger

* BS-2 is OPEN AND
* Operator has no Sepolia deployment address for `MarginEngineLens`,
  OR `cast code $LENS` returns empty (`0x`).

## 2. Goal

Deploy `MarginEngineLens` (one-time, stateless) on Base Sepolia
(chain 84532) so the backend's M-P2e trading-views helpers can
issue lens reads against it.

## 3. Scope

* Single contract deploy to Base Sepolia.
* Sepolia chain id 84532 only — mainnet hard-stop.
* Output: a `0x40-hex` Sepolia address.
* Result recorded in operator-private notes + backend
  `.env.sepolia` (NOT `.env`).

## 4. Tooling

```bash
cd ~/DEOPT/deopt-v2-sol
forge create src/lens/MarginEngineLens.sol:MarginEngineLens \
  --rpc-url "$EXECUTION_RPC_URL" \
  --private-key "$DEPLOYER_PRIVATE_KEY"
# OR via an existing script if one is added.
```

`MarginEngineLens` has no constructor args; the call is one-shot.

## 5. Verification (post-deploy)

```bash
cast code "$NEW_LENS_ADDR" --rpc-url "$EXECUTION_RPC_URL" | head -c 12
# Expect: non-"0x"-only

# Smoke a harmless read (revert on no state is acceptable):
cast call "$NEW_LENS_ADDR" \
  "getAccountState(address,address)" \
  "$OPTION_MARGIN_ENGINE" \
  "0x0000000000000000000000000000000000000001" \
  --rpc-url "$EXECUTION_RPC_URL"
```

## 6. Forbidden

* No mainnet.
* No Safe tx.
* No AWS / KMS creation.
* No production `.env` edit.
* No private key printed.
* No constructor argument that hard-codes a mainnet address.

## 7. Acceptance

* `cast code` returns non-empty.
* Operator writes the new lens address into private
  `.env.sepolia` as `OPTION_MARGIN_ENGINE_LENS_ADDRESS`.
* `E2E_SEPOLIA_RESOLVED_VALUES_CHECKLIST.md` BS-2 → CONFIRMED.
* Operator re-runs `SEPOLIA-OPERATOR-INPUT-PROVISIONING-AND-READONLY-CHECKS`
  to re-confirm.

## 8. Cross-links

* `E2E_SEPOLIA_OPERATOR_INPUT_PROVISIONING_RESULT.md`
* `E2E_SEPOLIA_BLOCKERS_AND_FIXES.md`
* `deopt-v2-sol/src/lens/MarginEngineLens.sol`
* `~/DEOPT/TESTNET_RUNBOOK.md`

**End of brief.**
