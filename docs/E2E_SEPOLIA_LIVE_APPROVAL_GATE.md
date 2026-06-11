# E2E Sepolia Live Approval Gate (M-P5 Phase B precondition)

**Date:** 2026-06-10
**Audience:** operator approving a single Base Sepolia test
broadcast.
**Posture:** **Approval required. Do not execute Phase B without
typing the literal approval line.** No mainnet. No Safe tx. No AWS
resource creation. No production `.env` edit.

> **CRITICAL.** This document does NOT itself authorise a broadcast.
> It defines the preconditions, the placeholders, and the literal
> approval line. The operator MUST manually sign off (in a separate
> communication or commit) before any broadcast occurs.

## 1. Preconditions

ALL of the following MUST be true before Phase B begins:

| # | Precondition | Verifying check |
|---|---|---|
| P1 | M-P5 Phase A dry-run RESULT doc green | `E2E_SEPOLIA_TRADING_LIFECYCLE_RESULT.md` Section 4 all ✓ |
| P2 | BS-1, BS-2, BS-3, BS-4, BS-5 closed | `E2E_SEPOLIA_BLOCKERS_AND_FIXES.md` severity matrix shows "closed" |
| P3 | Sepolia RPC reachable; chain id = 84532 | `eth_chainId` returns `0x14a34` |
| P4 | Backend chain id = 84532 (NEVER 8453) | startup log `chain_id=84532` |
| P5 | `EXECUTOR_REAL_BROADCAST_ENABLED=true` in `.env.sepolia` (NEVER `.env`) | `printenv` |
| P6 | `EXECUTION_ENABLED=true` in `.env.sepolia` | `printenv` |
| P7 | Executor authorised at OPTION_MATCHING_ENGINE | `cast call isExecutor(executor)` returns `true` |
| P8 | Buyer + seller pre-funded with testnet collateral | `cast call balanceOf(...)` shows > required |
| P9 | Series Active + oracle feed live | `GET /options/series/:id/details` returns `status: "ok"` with non-null oracle_mark_1e8 |
| P10 | R5 drift = 0 | reconciliation worker last-tick output `drift = 0` |
| P11 | Frontend points at Sepolia backend; MainnetDisabledBanner NOT visible | manual UI check |
| P12 | No mainnet RPC URL configured anywhere in the runtime env | `printenv \| grep -i mainnet` returns empty; `grep "8453"` returns only sol-frozen-doc references |

## 2. Placeholders (NEVER include actual values in this doc)

```
EXECUTION_RPC_URL=<operator-supplied; Sepolia HTTPS RPC>
CHAIN_ID=84532
NETWORK_NAME=base-sepolia
EXECUTION_ENABLED=true
EXECUTOR_DRY_RUN=false
EXECUTOR_REAL_BROADCAST_ENABLED=true

# Sepolia-only contract addresses (from §3.2 of RESULT + operator-supplied):
OPTION_PRODUCT_REGISTRY=<…>
OPTION_MATCHING_ENGINE=<…>
OPTION_MARGIN_ENGINE_ADDRESS=<…>
OPTION_COLLATERAL_VAULT_ADDRESS=<…>
OPTION_COLLATERAL_VAULT_VIEWS_ADDRESS=<…>
OPTION_MARGIN_ENGINE_LENS_ADDRESS=<…>
OPTION_ORACLE_ROUTER_ADDRESS=<…>

# Accounts (Sepolia public addresses from ~/DEOPT/TESTNET_RUNBOOK.md):
EXECUTOR_ADDRESS=0xc35F7A8A103A9A4464adfaa76B9B514093D23C27
TEST_BUYER=0xc0A76c2A6c6b70C0B065A05E64417886416cc976
TEST_SELLER=0xbAf0976a00a0DCc84Df5B15d927695c8b014B1c3
```

The operator's `.env.sepolia` provides the rest. Private keys
(`EXECUTOR_PRIVATE_KEY` or KMS reference) are NEVER stored in this
file and NEVER echoed in logs.

## 3. Accounts required

| Role | Address | Min testnet ETH | Min testnet collateral |
|---|---|---|---|
| Executor | `0xc35F…3C27` | 0.05 ETH | 0 |
| Test buyer | `0xc0A7…c976` | 0.01 ETH | enough to cover 1 contract premium |
| Test seller | `0xbAf0…B1c3` | 0.01 ETH | enough to cover the short collateral requirement |

## 4. Expected on-chain footprint per broadcast

| Item | Expected |
|---|---|
| Tx count | exactly **1** (the `OptionMatchingEngine.executeTrade(...)` call) |
| Max gas | 1_000_000 (existing `EXECUTOR_MAX_GAS_LIMIT` default) |
| Gas-price policy | EIP-1559; `EXECUTOR_MAX_FEE_PER_GAS_WEI` per operator config |
| Events emitted | `OptionTradeExecuted` + downstream fee events |
| Storage written | `option_execution_transactions` row + backend-side intent status update |
| Mainnet touched | NO — chain id MUST be 84532 |

## 5. Rollback / stop criteria

Stop immediately and switch back to dry-run mode if any of:

* RPC reports chain id ≠ 84532.
* Tx hash receipt returns `status: 0` (revert) — capture the revert
  reason via `cast tx --rpc-url $EXECUTION_RPC_URL $TX_HASH`
* Gas usage > 2× expected.
* Any event reveals fee transfer to an unexpected address.
* Reconciliation worker reports `drift != 0` post-broadcast.
* Frontend shows MainnetDisabledBanner at any point.

If a stop criterion fires:
1. Set `EXECUTOR_REAL_BROADCAST_ENABLED=false`.
2. Set `EXECUTION_ENABLED=false`.
3. Restart backend; verify the logs show dry-run mode.
4. Document the failure in `E2E_SEPOLIA_LIVE_BROADCAST_STOP_RESULT.md`.

## 6. R5 drift check

**Before** broadcast:
```
curl http://localhost:8080/admin/recent | jq '.r5_drift'
```
Expect `0`.

**After** broadcast:
```
curl http://localhost:8080/admin/recent | jq '.r5_drift'
```
Expect `0`. Any non-zero post-broadcast is a stop criterion.

## 7. Event / indexer / reconciliation checks

* `spawn_option_event_indexer` must observe the `OptionTradeExecuted`
  event within 30 seconds of confirmation.
* `spawn_option_reconciliation_worker` must report `match` for the
  intent within 60 seconds.
* `spawn_option_confirmation_worker` must transition the intent to
  `BroadcastConfirmed` within 60 seconds.

If any worker misses, switch to dry-run mode per §5.

## 8. Frontend status checks

After broadcast confirmation, the frontend's
`/transactions/<intent_id>` page must show:

* CONFIRMED stage highlighted.
* `tx_hash` populated with a real Sepolia hash (NOT a synthetic
  `0xdeadbee5...` from the M-P4c cycler).
* No `Authorization` header attached to any XHR (asserted by the
  existing `no-admin-bearer.spec.ts` pattern).

## 9. Explicit operator approval line

The operator MUST type the following literal line in the approval
channel (commit message, sign-off doc, or chat):

> I approve one Base Sepolia test broadcast for this run.

This line:

* Authorises **exactly one** broadcast.
* Expires when the broadcast confirms OR after 4 hours, whichever
  comes first.
* Does NOT authorise any subsequent broadcast; each new broadcast
  requires a fresh approval.

The approval line is recorded verbatim in
`E2E_SEPOLIA_LIVE_BROADCAST_RESULT.md` after the run.

## 10. Forbidden in this gate

* No private keys appear in this doc.
* No real Sepolia RPC URL appears in this doc.
* No AWS / KMS account / key IDs appear in this doc.
* No mainnet (8453) configuration is ever permitted.
* No Safe transaction is constructed.
* No new AWS resource is created.
* No production `.env` edit is required.

## 11. Cross-links

* `E2E_SEPOLIA_TRADING_LIFECYCLE_RESULT.md` (M-P5 Phase A)
* `E2E_SEPOLIA_READ_ONLY_PREFLIGHT_RUNBOOK.md`
* `E2E_SEPOLIA_BLOCKERS_AND_FIXES.md`
* `E2E_SEPOLIA_FIXES_NEXT_TASK.md`
* `BACKEND_LIVE_BROADCAST_FLAG_FLIP_RUNBOOK_V2G_FX_Q1_C.md` (existing
  live broadcast safety)
* `BACKEND_SIGNER_CUTOVER_RUNBOOK_V2G_FX_Q1.md` (existing signer
  config)
* `~/DEOPT/TESTNET_RUNBOOK.md`

**End of live approval gate.**
