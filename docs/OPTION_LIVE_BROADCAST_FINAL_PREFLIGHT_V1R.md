# Option Live Broadcast Final Preflight V1R

Date: 2026-05-22

## Scope

V1R is the final pre-broadcast check for the already-created Phase B intent
`e6d2941b-65f7-413a-958f-74ab22c53b08`. The configured broadcast gas cap is
raised from `1_300_000` (which V1Q proved was 85 gas short of the safety
margin) to `1_500_000`. The backend is restarted to pick up the new cap.
Oracle is refreshed immediately before the preflight, and a fresh
simulation + `eth_estimateGas` preview is collected.

V1R performed **no broadcast**, did not call `/executor/broadcast`, did not
call `POST /options/execution-intents/:id/broadcast`, did not submit any
`option_execution_transactions` row, did not write any
`execution_transactions` row, did not cleanup any evidence row, did not
mark any intent confirmed/reconciled, did not print private keys, did not
modify Solidity source, did not modify frontend, did not deploy contracts.

## Backend Config (V1R)

`/tmp/deopt-live-env.sh` updated to set `OPTION_EXECUTION_BROADCAST_GAS_LIMIT=1500000`. Backend restarted from `./target/debug/deopt-v2-backend`.

`/health` → `{ok: true}`.

`/admin/config` gate verification:

| Check | Value |
| --- | --- |
| `option_execution_broadcast_enabled` | true |
| `execution_enabled` | true |
| `executor_real_broadcast_enabled` | true |
| `executor_dry_run` | false |
| `execution_gas_safety_bps` | 12500 |
| `execution_broadcast_gas_limit` | **1500000** |
| `option_matching_engine_address` | `0xf2D1D85cD363Be3bc160d14883C80e7C2c4F420b` |

All 7 gates pass.

## Oracle Refreshed In V1R

The mock feeds had drifted ~362 s old by the time V1R opened. V1R re-ran `script/RefreshTestnetMockFeeds.s.sol` against Base Sepolia. Chain id guard (`block.chainid != 8453`) and `TESTNET_MOCKS_ENABLED=true` both satisfied. Four `MockPriceSource.setPrice` transactions broadcast successfully:

| Tx hash | To | Block | Status |
| --- | --- | ---: | --- |
| `0x2b9efeee80ec728124ebd7927bc79b4bd2ae6de896e10b557d870d4e637da6b5` | `0x3eb9cdd2C2115c3f0DF5E30da53D7245F9a5f6Cc` (ETH/USDC primary) | 41855293 | `0x1` |
| `0x53bde69bc4e398f31688df2bc6bee9b2206cfdff8bb6ea3750424a805bfebb56` | `0x8cbA01B3f4e818ffffD6c1aE1f9a18A656e918bB` (BTC/USDC primary) | 41855294 | `0x1` |
| `0xe923d71c4b5dfe82cbf357c313a221559e08c08f06b7e436468b40f0be4227ae` | `0x2103a84C0CAB9cf7680d602C8931FaDeD7064517` (ETH/USDC secondary) | 41855295 | `0x1` |
| `0x743673af4b1ec4f6a5423274acce3e20ba3d88d56e0e03563860fee2eeb6c1b2` | `0x7206E7c2c1C3D6e6273020163EB1f0E9339b970C` (BTC/USDC secondary) | 41855296 | `0x1` |

Manifest: `~/DEOPT/deopt-v2-sol/broadcast/RefreshTestnetMockFeeds.s.sol/84532/run-latest.json`.

To stay under the 60 s feed `maxDelay`, the entire downstream sequence (refresh → oracle re-probe → re-simulate → gas estimate) was packed into a single shell invocation; total elapsed time was **15 s**.

## Oracle Status

- Block timestamp at probe: `1779478878`
- ETH/USDC primary `updated_at = 1779478874` — **age 4 s**
- `OracleRouter.paused() = false`
- `OracleRouter.readPaused() = false`
- `OracleRouter.maxOracleDelay() = 600 s`; feed `maxDelay = 60 s`; `isActive = true`
- `OracleRouter.getPriceSafe(WETH, USDC) = (300_000_000_000, 1779478874, true)` — **fresh, ok=true, price > 0**

By the end of the gas preview the oracle age was still only **8 s**, well under both the 50 s task budget and the 60 s feed cap.

## Intent State

| Field | Value |
| --- | --- |
| `intent_id` | **`e6d2941b-65f7-413a-958f-74ab22c53b08`** |
| `onchain_intent_id` | `0x0a77c7c9570198c969b1fa597ea193cb6fee563e3bfae514e9a3f0c4e01705f5` |
| `status` | `calldata_ready` |
| `buyer_signature_present` | true |
| `seller_signature_present` | true |
| `calldata_ready` | true |
| `calldata_selector` | `0x031f77b3` (`executeTrade(OptionTrade,bytes,bytes)`) |
| `calldata_length_chars` | 1674 |
| Associated `option_execution_transactions` row | **none** |

Both signatures still match V1Q's local recovery against `0xc0A76c2A…` (buyer) and `0xbAf0976a…` (seller). The intent was not modified between V1Q and V1R — it is the same `calldata_ready` row.

## Simulation Result

`POST /options/execution-intents/e6d2941b-…/simulate` (fresh run, V1R):

```json
{
  "intent_id": "e6d2941b-65f7-413a-958f-74ab22c53b08",
  "simulation_status": "simulation_ok",
  "block_number": 41855296,
  "error": null,
  "revert_data": null,
  "revert_selector": null,
  "simulated_at_ms": 1779478882257,
  "submitted": false,
  "confirmed": false
}
```

`simulation_ok` against the freshly-refreshed oracle at block `41855296`.

## Gas Safety Preview

Read-only `cast estimate-gas` for the exact prepared transaction (from = executor `0xc35F7A8A…`, to = `0xf2D1D85c…`, data = `intent.calldata`, value = 0):

| Field | Value |
| --- | ---: |
| `estimated_gas` | **1_091_120** |
| `gas_safety_bps` | **12_500** |
| `required_gas = estimated_gas × 12500 / 10000` | **1_363_900** |
| `broadcast_gas_limit` | **1_500_000** |
| `broadcast_gas_limit >= estimated_gas` | **true** |
| `broadcast_gas_limit >= required_gas` | **true** |
| Headroom over `required_gas` | **+136_100 gas** |
| `gas_check_status` | **ok** |

The 1.5 M cap clears both the bare estimate (`1.09 M`) and the 25%-margin required gas (`1.36 M`), with `+136_100` of headroom (~10% of `required_gas`).

Note: `estimated_gas` changed from 1_040_068 (V1Q) to 1_091_120 (V1R) — a +5% drift between calls, likely due to oracle freshness state differences and/or sequence cost touching different storage warmths. The V1O safety margin (25%) absorbs this kind of drift comfortably; this is the exact failure mode the patch was designed to guard against.

## DB Mutation Check

`V1R_START_MS = 1779477534728`. After the V1R sequence:

| Table | Rows since `V1R_START_MS` |
| --- | ---: |
| `option_execution_intents` | **0** |
| `option_execution_transactions` | **0** |
| `execution_transactions` | **0** |

Preserved intents:

| `intent_id` | `status` |
| --- | --- |
| `4075afe3-…` (V1L failed) | `broadcast_submitted` (unchanged) |
| `e6d2941b-…` (current) | `calldata_ready` |

Preserved transaction rows:

| `transaction_id` | `intent_id` | `status` | `tx_hash` |
| --- | --- | --- | --- |
| `204a3070-…` | `4075afe3-…` | `submitted` | `0xe832365b…` (V1L failed, untouched) |

No row exists for intent `e6d2941b-…`.

## Files Changed

- `docs/OPTION_LIVE_BROADCAST_FINAL_PREFLIGHT_V1R.md` (new, backend repo; not committed)
- `/tmp/deopt-live-env.sh` (helper; not tracked; updated `OPTION_EXECUTION_BROADCAST_GAS_LIMIT` from 1_300_000 to 1_500_000)
- `/tmp/deopt-phase-b-v1r-start.txt` (records `V1R_START_MS`)
- `~/DEOPT/deopt-v2-sol/broadcast/RefreshTestnetMockFeeds.s.sol/84532/run-latest.json` (forge artifact; not tracked source)
- `~/DEOPT/deopt-v2-sol/cache/RefreshTestnetMockFeeds.s.sol/84532/run-latest.json` (forge cache)

No git-tracked source changes in either repo.

## No-Forbidden-Mutation Verification

- `POST /options/execution-intents/:id/broadcast`: **not called**
- `/executor/broadcast`: **not called**
- No option execution transaction signed for `eth_sendRawTransaction`
- `option_execution_transactions` rows since `V1R_START_MS`: **0**
- `execution_transactions` rows since `V1R_START_MS`: **0**
- Preserved V1L row `204a3070-…` / tx `0xe832365b…` unchanged
- No `confirmed` / `reconciled` rows written
- No Solidity / frontend / deployment changes; only `MockPriceSource.setPrice` calls broadcast in V1R for the oracle refresh
- No private keys printed

## Whether Human Can Authorize The Broadcast

**Yes.** Every gate is green:

1. Backend running with V1O gas safety patch (`f36968e`), gas safety BPS = 12500, broadcast cap = 1_500_000, all option-execution feature flags on, strict signature mode, executor key configured, RPC configured, persistence on.
2. On-chain preflight (re-verified before V1Q): `isExecutor(0xc35F…) = true`, both nonces still `0`, active option series intact.
3. Oracle fresh: `getPriceSafe(WETH, USDC) = (price, ts, true)`, age ≤ 8 s at preview end.
4. Intent `e6d2941b-…` has both EIP-712 signatures stored, calldata stored (selector `0x031f77b3`), and a fresh `simulation_ok` at block `41855296`.
5. Gas safety preview: `estimated_gas=1_091_120`, `required_gas=1_363_900`, cap `1_500_000`, **`gas_check_status=ok`** with `+136_100` headroom.
6. Executor balance ≈ `0.00812 ETH`; worst-case broadcast cost ≈ `1_500_000 × 1 gwei = 0.0015 ETH`; ratio ~5.4× — comfortable.

Operator should issue an explicit "yes, broadcast now" so the harness can call `POST /options/execution-intents/e6d2941b-65f7-413a-958f-74ab22c53b08/broadcast` exactly once, no retry, no `/executor/broadcast`. The window between oracle refresh and broadcast should be kept under ~50 s; if the operator authorization arrives later, re-refresh the oracle and re-run V1R steps 4–10 before broadcasting.

## Remaining Blockers

1. **Operator authorization** — explicit "yes, broadcast now" required.
2. **Oracle freshness window** — broadcast should happen within ~50 s of the last refresh; if there is delay between V1R and the authorize step, re-run the refresh and re-collect the gas preview.
3. **Executor balance** — currently ~0.00812 ETH against worst-case 0.0015 ETH (5.4×), safe; re-check at attempt time only as a sanity step.
