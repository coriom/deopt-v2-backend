# Option First Successful Live Broadcast V1S

Date: 2026-05-22

## Result

**First successful live option execution on Base Sepolia.**

- Transaction hash: **`0x5964a7b3d2c18d051baaa780413d31c44d419ce530f45263cb4c46f720881125`**
- Block: `41856964`
- Receipt `status = 1 (success)`
- `gasUsed = 1_057_772` (within `gasLimit = 1_500_000`, slightly below `estimated_gas = 1_091_120`)
- Top-level call: `OptionMatchingEngine.executeTrade(OptionTrade,bytes,bytes)` (selector `0x031f77b3`)
- From: `0xc35F7A8A103A9A4464adfaa76B9B514093D23C27` (executor)
- To: `0xf2D1D85cD363Be3bc160d14883C80e7C2c4F420b` (OptionMatchingEngine)
- Intent: `e6d2941b-65f7-413a-958f-74ab22c53b08` → status `broadcast_submitted`

Exactly one transaction was submitted. No retry. No `/executor/broadcast` call. No `execution_transactions` row created.

## Pre-broadcast Sequence (single shell, abort-on-fail)

The whole final preflight + broadcast was packed into `/tmp/deopt-broadcast-v1s.sh` so the entire sequence stayed under the 60 s feed `maxDelay`. The script `set -u`/`pipefail` + `trap ERR abort`, runs each step in order, and unsets the trap *only* immediately before the single broadcast call (so a failure during result parsing cannot trigger a false retry).

### Step 1: `/admin/config` gates

All 7 V1R gates green (re-verified):

| Check | Value |
| --- | --- |
| `option_execution_broadcast_enabled` | true |
| `execution_enabled` | true |
| `executor_real_broadcast_enabled` | true |
| `executor_dry_run` | false |
| `execution_gas_safety_bps` | 12500 |
| `execution_broadcast_gas_limit` | 1500000 |
| `option_matching_engine_address` | `0xf2D1D85cD363Be3bc160d14883C80e7C2c4F420b` |

### Step 2: oracle refresh (just-in-time)

`forge script script/RefreshTestnetMockFeeds.s.sol:RefreshTestnetMockFeeds --rpc-url <base-sepolia> --broadcast --slow` from `~/DEOPT/deopt-v2-sol`. Chain id `84532` guard satisfied, `TESTNET_MOCKS_ENABLED=true`. Four `MockPriceSource.setPrice` transactions broadcast, all `status=0x1`:

| Feed | Tx hash | Block |
| --- | --- | ---: |
| ETH/USDC primary `0x3eb9cdd2…` | `0xec9ef66b06436e6076dd31fec3a8757df2775ddd5c2f1c4db53457ab813d588c` | 41856957 |
| BTC/USDC primary `0x8cbA01B3…` | `0xc8b753e663f72f397188beba7872533a169f80e5caf42b70b3dd0cfda19f1edf` | 41856958 |
| ETH/USDC secondary `0x2103a84C…` | `0xf7f7e3bba5f8bc5f9ff7edfad8118c31536f261f4b9fad68ec4e768c7b7fc73d` | 41856959 |
| BTC/USDC secondary `0x7206E7c2…` | `0xfa157549e509d17f3e5668a4176dcdd44e077481eeca994c6b2e73a217082334` | 41856960 |

Refresh took 16 s wall-clock.

### Step 3: oracle status (post-refresh)

- `block_ts = 1779482208`
- ETH/USDC primary `updated_at = 1779482202` → **age 6 s**
- `OracleRouter.paused() = false`
- `OracleRouter.readPaused() = false`
- `OracleRouter.maxOracleDelay() = 600 s`; feed `maxDelay = 60 s`
- `OracleRouter.getPriceSafe(WETH, USDC) = (300_000_000_000, 1779482202, true)` — **ok=true, price > 0**

Final freshness check right before the broadcast call: age **10 s** (well under both the 50 s task budget and the 60 s feed cap).

### Step 4: intent recheck

| Field | Value |
| --- | --- |
| `intent_id` | `e6d2941b-65f7-413a-958f-74ab22c53b08` |
| `status` | `calldata_ready` |
| `buyer_signature_present` | true |
| `seller_signature_present` | true |
| `calldata_ready` | true |
| Calldata selector | `0x031f77b3` |
| Calldata length (chars) | 1674 |
| Existing `option_execution_transactions` for this intent | **0** |

### Step 5: simulation (fresh, post-refresh)

```json
{
  "intent_id": "e6d2941b-65f7-413a-958f-74ab22c53b08",
  "simulation_status": "simulation_ok",
  "block_number": 41856962,
  "error": null,
  "revert_data": null,
  "revert_selector": null,
  "simulated_at_ms": ...
}
```

### Step 6: gas safety preview

| Field | Value |
| --- | ---: |
| `estimated_gas` | 1_091_120 |
| `required_gas = estimated_gas × 12500 / 10000` | 1_363_900 |
| `broadcast_gas_limit` | 1_500_000 |
| `gas_safety_bps` | 12_500 |
| `broadcast_gas_limit >= estimated_gas` | true |
| `broadcast_gas_limit >= required_gas` | true |
| Headroom over `required_gas` | +136_100 gas |
| `gas_check_status` | **ok** |

### Step 7: `BROADCAST_START_MS`

`BROADCAST_START_MS = 1779482214252`.

### Step 8: broadcast (exactly once)

Call: `POST http://127.0.0.1:8080/options/execution-intents/e6d2941b-65f7-413a-958f-74ab22c53b08/broadcast`

Response:

```json
{
  "intent_id": "e6d2941b-65f7-413a-958f-74ab22c53b08",
  "status": "broadcast_submitted",
  "tx_hash": "0x5964a7b3d2c18d051baaa780413d31c44d419ce530f45263cb4c46f720881125",
  "to": "0xf2D1D85cD363Be3bc160d14883C80e7C2c4F420b",
  "from": "0xc35f7a8a103a9a4464adfaa76b9b514093d23c27",
  "transaction_id": "cae8c7e7-ed61-4265-aa7d-75edd94ef03c",
  "broadcast_enabled": true,
  "submitted": true,
  "duplicate": false,
  "confirmed": false,
  "estimated_gas": 1091120,
  "required_gas": 1363900,
  "simulation_gas_limit": 0,
  "broadcast_gas_limit": 1500000,
  "gas_safety_bps": 12500,
  "gas_check_status": "ok",
  "gas_check_error": null
}
```

No retry was attempted. The script printed `=== DONE — DO NOT RETRY ===` and exited 0.

## On-chain Verification

### `cast receipt`

| Field | Value |
| --- | --- |
| `transactionHash` | `0x5964a7b3d2c18d051baaa780413d31c44d419ce530f45263cb4c46f720881125` |
| `blockNumber` | 41856964 |
| `blockHash` | `0x53d62c21ecbe462e2868e216b4655474de0d2b7b832f15ab6e72b216fb1f3853` |
| `from` | `0xc35F7A8A103A9A4464adfaa76B9B514093D23C27` |
| `to` | `0xf2D1D85cD363Be3bc160d14883C80e7C2c4F420b` |
| `status` | **1 (success)** |
| `gasUsed` | 1_057_772 |
| `effectiveGasPrice` | 6_000_000 |
| `cumulativeGasUsed` | 1_672_948 |
| `transactionIndex` | 5 |
| `type` | 2 |

Event log highlights (all under the same `transactionHash`):

- Premium transfer chain on CollateralVault `0x00340c…0625D3` (settlement asset `0x6eAe407f…`): buyer balance debited, seller balance credited, fee splits emitted with topic `0xf67cd268…` (`InternalTransfer`) and `0x77178bcf…` (`FeeAccrued`).
- `MarginEngine 0x6C5665De…` emitted two `0x12cf6338…` events (apply-trade per side) carrying the `OptionTrade` payload reference + the option series hash `0x35621974bccc555e161c6707f0a1a3bca2d02be5e3a4d380980bfaef656e7957`.
- `MarginEngine` also emitted `0x6f0909c4…` (`OptionPositionUpdated`) for buyer (long, `0x1`) and seller (short, `0x1`).
- `OptionMatchingEngine 0xf2D1D85c…` emitted `0xb2387b9f0e4823ecef9a16ea4aaba6598c0703fb5e9d8dba37ef303add4cb808` (`OptionTradeExecuted`) keyed by `intentId = 0x0a77c7c9570198c969b1fa597ea193cb6fee563e3bfae514e9a3f0c4e01705f5`.

### `cast tx`

| Field | Value |
| --- | --- |
| `from` | `0xc35F7A8A103A9A4464adfaa76B9B514093D23C27` |
| `to` | `0xf2D1D85cD363Be3bc160d14883C80e7C2c4F420b` |
| `value` | 0 |
| `chainId` | 84532 |
| `nonce` | 523 |
| `gasLimit` | 1_500_000 |
| `maxFeePerGas` | 1_000_000_000 |
| `maxPriorityFeePerGas` | 1_000_000 |
| `type` | 2 (EIP-1559) |
| input selector | `0x031f77b3` |
| input encodes `intentId` | `0x0a77c7c9570198c969b1fa597ea193cb6fee563e3bfae514e9a3f0c4e01705f5` ✓ (matches stored intent) |
| input encodes `optionId` | `0x35621974bccc555e161c6707f0a1a3bca2d02be5e3a4d380980bfaef656e7957` (low-32 bytes of `24145907678156652148089862289363692212069910767044828147380657249455352740183`) ✓ |

## DB Mutation Summary

`BROADCAST_START_MS = 1779482214252`. After the broadcast:

| Table | Rows since `BROADCAST_START_MS` |
| --- | ---: |
| `option_execution_transactions` | **1** |
| `execution_transactions` | **0** |

Persisted `option_execution_transactions` row:

| Field | Value |
| --- | --- |
| `transaction_id` | `cae8c7e7-ed61-4265-aa7d-75edd94ef03c` |
| `intent_id` | `e6d2941b-65f7-413a-958f-74ab22c53b08` |
| `sender` | `0xc35f7a8a103a9a4464adfaa76b9b514093d23c27` |
| `target` | `0xf2D1D85cD363Be3bc160d14883C80e7C2c4F420b` |
| `value_wei` | `0` |
| `gas_limit` | `1_500_000` |
| `tx_hash` | `0x5964a7b3d2c18d051baaa780413d31c44d419ce530f45263cb4c46f720881125` |
| `status` | `submitted` |
| `error` | `null` |
| `estimated_gas` | `1_091_120` |
| `required_gas` | `1_363_900` |
| `simulation_gas_limit` | `0` |
| `broadcast_gas_limit` | `1_500_000` |
| `gas_safety_bps` | `12_500` |
| `gas_check_status` | `ok` |
| `gas_check_error` | `null` |

Persisted intent state:

| Field | Value |
| --- | --- |
| `intent_id` | `e6d2941b-65f7-413a-958f-74ab22c53b08` |
| `status` | `broadcast_submitted` |
| `error` | `null` |
| `simulation_status` | `simulation_ok` |

Preserved V1L evidence row (`transaction_id=204a3070-…`, tx `0xe832365b…`, intent `4075afe3-…`) was not touched. V1S is the second `option_execution_transactions` row in the database (one historical failure, one historical success — neither was cleaned up).

## No-Forbidden-Endpoint Verification

- `POST /options/execution-intents/<id>/broadcast` was called exactly **once**.
- `/executor/broadcast` was **not** called.
- No generic executor manual tick.
- `execution_transactions` rows since `BROADCAST_START_MS` = **0**.

## No-Retry Verification

- Single broadcast attempt; `provider.send_count = 1` (mirrored by the single `option_execution_transactions` row).
- Backend script intentionally drops the abort-on-error trap immediately before the broadcast call so a downstream parsing failure cannot be misread as needing a retry.
- No `confirmed` / `reconciled` flags were written; the doc explicitly excludes them.

## Files Changed

- `docs/OPTION_FIRST_SUCCESSFUL_LIVE_BROADCAST_V1S.md` (new; backend repo; not committed)
- `/tmp/deopt-broadcast-v1s.sh` (helper for this run; not tracked)
- `/tmp/deopt-v1s-broadcast-start.txt`, `/tmp/deopt-v1s-broadcast-response.json` (run artifacts; not tracked)
- `~/DEOPT/deopt-v2-sol/broadcast/RefreshTestnetMockFeeds.s.sol/84532/run-latest.json` (forge artifact; not tracked source)
- No git-tracked source changes.

## Remaining Blocker

None for the V1S goal (first successful live option broadcast). Follow-up items, deferred:

1. **Index + reconcile + confirm** the V1S transaction. The intent currently sits at `broadcast_submitted` and the on-chain tx is in block `41856964` with `status=1`, but the backend does not auto-mark `confirmed` / `reconciled` per V1I scope. A future V1T can wire the option indexer / reconciliation / confirmation pipeline against this same `(intent_id, tx_hash)` pair so it transitions to `broadcast_confirmed`.
2. **Index the `OptionTradeExecuted` event** (`0xb2387b9f…`) into a dedicated table so the matching engine emits can be queried out of the box.
3. **Permanently lift `OPTION_EXECUTION_BROADCAST_GAS_LIMIT`** from V1Q's 1_300_000 (which V1Q showed was 85 gas short) to ≥ 1_500_000 in the canonical backend `.env` / runtime config — currently the live cap only exists in the `/tmp/deopt-live-env.sh` helper.
4. **Refresh script automation**: Base Sepolia mock feeds expire fast (60 s). Either schedule a cron in the Solidity repo to keep them fresh, or fold a "refresh-if-stale" step into the option broadcast preflight so operators don't need to remember.
5. **Backfill** the V1S row with `gas_used`, `effective_gas_price`, `block_number`, and `confirmation_status` once the option confirmation pipeline lands.
