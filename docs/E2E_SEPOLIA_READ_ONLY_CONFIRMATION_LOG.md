# E2E Sepolia — Read-Only Confirmation Log (M-P5 Phase A Follow-up)

**Date:** 2026-06-10
**Audience:** operator running the read-only on-chain confirmations
for BS-2 / BS-3 / BS-4 / BS-5.
**Posture:** **read-only only. NO transaction. NO signer call. NO
broadcast. NO role grant. NO token transfer. NO mint. NO approve.**

> **M-P5-PROV update (2026-06-10):** the operator-private template
> at `E2E_SEPOLIA_OPERATOR_INPUT_PRIVATE_TEMPLATE.md` lists every
> private value the commands below reference. The companion
> `E2E_SEPOLIA_READ_ONLY_CONFIRMATIONS_EXECUTION_RESULT.md`
> documents the decision tree for each outcome (CONFIRMED vs.
> branch-to-fix-brief).

> **M-P5-RO2 update (2026-06-10):** the commands below were
> EXECUTED against the populated private file. Outcomes recorded
> as bucket flags in
> `E2E_SEPOLIA_READONLY_CHECKS_WITH_RPC_RESULT.md`. The 4 BS rows
> all remain OPEN with refined sub-states.

> All commands in this log are `cast call` (eth_call) or
> `cast code` (eth_getCode) or `cast balance` (eth_getBalance) —
> never `cast send` or any state-mutating verb. If a command attempts
> a write path, stop and flag the deviation.

## 1. Preconditions

* `EXECUTION_RPC_URL` set in the operator's shell (NEVER printed
  back to logs).
* `cast` (foundry) installed.
* The operator's private fill-in of `E2E_SEPOLIA_OPERATOR_INPUT_TEMPLATE.md`
  is available (NEVER checked in).

```bash
# Sanity: chain id must be Base Sepolia (84532 = 0x14a34).
cast chain-id --rpc-url "$EXECUTION_RPC_URL"
# Expect: 84532
```

If the result is anything other than `84532` — **STOP**. The
remainder of this log assumes Sepolia.

## 2. BS-1 — CollateralVaultViews

**Status: CLOSED via abstract-inheritance discovery
(no on-chain check required).**

See `E2E_SEPOLIA_OPERATOR_INPUT_TEMPLATE.md` §3. The views surface
is inherited into the concrete `CollateralVault` contract; the
backend env key `OPTION_COLLATERAL_VAULT_VIEWS_ADDRESS` is set to
the same value as `OPTION_COLLATERAL_VAULT_ADDRESS`. Optional
sanity check:

```bash
# Optional: confirm the views selectors live at the CollateralVault
# address (this MUST already be true; surfaces any drift from the
# frozen ABI).
cast call "$OPTION_COLLATERAL_VAULT" \
  "getCollateralTokens()(address[])" \
  --rpc-url "$EXECUTION_RPC_URL"
# Expect: array (possibly empty). No revert.
```

## 3. BS-2 — MarginEngineLens address

**Status: OPEN — operator-supplied address required.**

`MarginEngineLens` is a concrete standalone contract
(`contract MarginEngineLens is MarginEngineTypes`, source at
`deopt-v2-sol/src/lens/MarginEngineLens.sol`). It takes `marginEngine`
as a function parameter, so a single deployed instance serves all
margin engines.

No deploy script for it exists in `deopt-v2-sol/script/`. The
operator must supply a Sepolia address — either from previous
deployment notes OR via a fresh deploy in a SEPARATE governance
task.

If the operator has a known address `$LENS_ADDR`:

```bash
# Confirm bytecode exists.
cast code "$LENS_ADDR" --rpc-url "$EXECUTION_RPC_URL" | head -c 10
# Expect: a non-"0x" prefix (e.g. "0x6080604052…"). "0x" alone means
# no contract deployed at the address.

# Confirm at least one frozen-ABI selector responds without revert.
# getAccountState(marginEngine, trader) — selector 0xa57bd4cc.
cast call "$LENS_ADDR" \
  "getAccountState(address,address)" \
  "$OPTION_MARGIN_ENGINE" \
  "0x0000000000000000000000000000000000000001" \
  --rpc-url "$EXECUTION_RPC_URL" | head -c 20
# Expect: encoded bytes. Revert is acceptable here if the trader has
# no state — what matters is that the function selector exists.
```

Once confirmed, set `OPTION_MARGIN_ENGINE_LENS_ADDRESS=$LENS_ADDR`
in `.env.sepolia` (NEVER `.env`).

## 4. BS-3 — Executor authorisation at OPTION_MATCHING_ENGINE

**Status: OPEN — operator-side read-only confirmation required.**

```bash
# Known public Sepolia executor (from ~/DEOPT/TESTNET_RUNBOOK.md):
EXECUTOR=0xc35F7A8A103A9A4464adfaa76B9B514093D23C27
OPTION_MATCHING_ENGINE=0xf2D1D85cD363Be3bc160d14883C80e7C2c4F420b

cast call "$OPTION_MATCHING_ENGINE" \
  "isExecutor(address)(bool)" \
  "$EXECUTOR" \
  --rpc-url "$EXECUTION_RPC_URL"
# Expect: true
```

* If `true` → BS-3 CONFIRMED.
* If `false` → operator runs the existing
  `script/SetMatchingEngineExecutor.s.sol` in a SEPARATE governance
  task (NOT part of this milestone — that requires a state-mutating
  broadcast).
* If revert / unknown selector → the ABI is unexpected. Stop and
  flag.

Record only the boolean outcome in the public-safe checklist; do not
quote the RPC URL or any signer detail.

## 5. BS-4 — Buyer / seller testnet balances

**Status: OPEN — operator-side read-only confirmation required.**

```bash
BUYER=0xc0A76c2A6c6b70C0B065A05E64417886416cc976
SELLER=0xbAf0976a00a0DCc84Df5B15d927695c8b014B1c3
COLLATERAL_TOKEN=<operator-supplied>

# Test ETH balance for gas.
cast balance "$BUYER"  --rpc-url "$EXECUTION_RPC_URL"
cast balance "$SELLER" --rpc-url "$EXECUTION_RPC_URL"

# Collateral balance (ERC20).
cast call "$COLLATERAL_TOKEN" \
  "balanceOf(address)(uint256)" \
  "$SELLER" \
  --rpc-url "$EXECUTION_RPC_URL"

cast call "$COLLATERAL_TOKEN" \
  "balanceOf(address)(uint256)" \
  "$BUYER" \
  --rpc-url "$EXECUTION_RPC_URL"

# Allowance to OPTION_COLLATERAL_VAULT (required for deposit flow).
cast call "$COLLATERAL_TOKEN" \
  "allowance(address,address)(uint256)" \
  "$SELLER" "$OPTION_COLLATERAL_VAULT" \
  --rpc-url "$EXECUTION_RPC_URL"
```

Map each result to one of:

| Result | Status |
|---|---|
| Buyer ETH ≥ 0.01 + Seller ETH ≥ 0.01 + collateral ≥ requirement + allowance ≥ requirement | **OK** |
| Any ETH < 0.01 | **LOW_BALANCE** (operator tops up via testnet faucet) |
| Any collateral < requirement | **LOW_BALANCE** (operator runs `script/PrepareSmokeV1Accounts.s.sol` — separate task) |
| Allowance < requirement | **LOW_ALLOWANCE** (operator-side approve — separate task) |
| Any call reverts | **UNKNOWN** (ABI mismatch; stop) |

Record only the status string in the checklist. Do NOT record actual
balances in public docs.

## 6. BS-5 — Active series + live oracle feed

**Status: OPEN — operator-side read-only confirmation required.**

### 6.1 List active series

```bash
OPTION_PRODUCT_REGISTRY=0x3d52b033fab00ed6104dd3bc0a715f8648344eca

cast call "$OPTION_PRODUCT_REGISTRY" \
  "totalSeries()(uint256)" \
  --rpc-url "$EXECUTION_RPC_URL"
# Expect: > 0
```

Walk the index range:

```bash
N=$(cast call "$OPTION_PRODUCT_REGISTRY" "totalSeries()(uint256)" --rpc-url "$EXECUTION_RPC_URL")
for i in $(seq 0 $((N - 1))); do
  cast call "$OPTION_PRODUCT_REGISTRY" \
    "seriesAt(uint256)(uint256)" "$i" \
    --rpc-url "$EXECUTION_RPC_URL"
done
```

For each returned `series_id`, check active + expiry:

```bash
cast call "$OPTION_PRODUCT_REGISTRY" \
  "getSeries(uint256)" "$SERIES_ID" \
  --rpc-url "$EXECUTION_RPC_URL"
```

Pick a series whose:
* `isActive` flag is true.
* expiry is at least 1 hour in the future.

### 6.2 Oracle feed

```bash
OPTION_ORACLE_ROUTER=0xb416406f200b2ef3d7a86a5d5877ed41d9b1a581

# Confirm a feed exists for the (underlying, settlement) pair.
cast call "$OPTION_ORACLE_ROUTER" \
  "hasActiveFeed(address,address)(bool)" \
  "$UNDERLYING" "$SETTLEMENT" \
  --rpc-url "$EXECUTION_RPC_URL"
# Expect: true

# Try the fail-fast price read (will revert on stale feed).
cast call "$OPTION_ORACLE_ROUTER" \
  "getPriceSafe(address,address)(uint256)" \
  "$UNDERLYING" "$SETTLEMENT" \
  --rpc-url "$EXECUTION_RPC_URL"
# Expect: a uint256 > 0
```

* If both reads succeed → BS-5 CONFIRMED.
* If `hasActiveFeed` is false → operator-side feed registration is
  required (governance action; separate task).
* If `getPriceSafe` reverts but `hasActiveFeed` is true → the feed
  is stale. Operator-side feed refresh is required (separate task).

Record only the status flag in the checklist.

## 7. Aggregated decision

After §3–§6 complete, the operator updates
`E2E_SEPOLIA_RESOLVED_VALUES_CHECKLIST.md` with the
read-only-confirmed status of each blocker. If all are CLOSED /
CONFIRMED, the next milestone is the live approval gate; otherwise,
`E2E_SEPOLIA_REMAINING_OPERATOR_ACTIONS.md` carries the precise
operator-side fixes.

## 8. Forbidden

* No `cast send` anywhere in this log.
* No `cast tx` that signs and broadcasts.
* No private-key handling.
* No mainnet RPC URL.
* No real Sepolia RPC URL echoed back into a checked-in artefact.
* No real balance numbers in public docs.

## 9. Cross-links

* `E2E_SEPOLIA_OPERATOR_INPUT_TEMPLATE.md`
* `E2E_SEPOLIA_RESOLVED_VALUES_CHECKLIST.md`
* `E2E_SEPOLIA_BLOCKERS_AND_FIXES.md`
* `E2E_SEPOLIA_FIXES_RESULT.md`
* `E2E_SEPOLIA_READ_ONLY_PREFLIGHT_RUNBOOK.md`
* `E2E_SEPOLIA_LIVE_APPROVAL_GATE.md`
* `~/DEOPT/TESTNET_RUNBOOK.md`

**End of read-only confirmation log.**
