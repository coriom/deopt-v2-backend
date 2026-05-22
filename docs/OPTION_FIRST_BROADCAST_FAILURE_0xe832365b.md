# First Option Live Broadcast Failure: 0xe832365b

## Scope

Diagnosis was read-only. No broadcast endpoint was called, no transaction was submitted, no retry was attempted, and the preserved evidence row was not cleaned up.

Transaction:

`0xe832365b11ead105e020b65a25570516e87ab1af2b1225b698561f090eff8b7c`

## Preserved Database Evidence

Intent:

- `intent_id`: `4075afe3-fe42-457d-a9ca-eb0907d09a74`
- `onchain_intent_id`: `0x18c8c98825599abf10ce99d0e6f12c9215fc6ecbd497784ba37aff433909493b`
- `status`: `broadcast_submitted`
- `source_type`: `option_orderbook_fill`
- `source_id`: `a8d46003-c144-43b4-b422-c922ff21135d`
- `buyer`: `0xc0A76c2A6c6b70C0B065A05E64417886416cc976`
- `seller`: `0xbAf0976a00a0DCc84Df5B15d927695c8b014B1c3`
- `buyer_nonce`: `0`
- `seller_nonce`: `0`
- `option_id`: `24145907678156652148089862289363692212069910767044828147380657249455352740183`
- `underlying`: `0x4DeEBc5f537F3b8ba0E3393807B4D699D72bDd02`
- `settlement_asset`: `0x6eAe407f5640B006faC9965182e238582A3B412E`
- `expiry`: `1893456000`
- `strike_1e8`: `300000000000`
- `is_call`: `true`
- `contract_size_1e8`: `100000000`
- `quantity_contracts`: `1`
- `premium_per_contract_native`: `1000000`
- `buyer_is_maker`: `false`
- `simulation_status`: `simulation_ok`
- `simulation_block_number`: `41838777`
- `simulated_at_ms`: `1779445842538`
- `calldata_length`: `1674`
- `calldata_selector`: `0x031f77b3`

Option transaction row:

- `transaction_id`: `204a3070-50b1-4b89-865d-ad183752b1e8`
- `sender`: `0xc35f7a8a103a9a4464adfaa76b9b514093d23c27`
- `target`: `0xf2D1D85cD363Be3bc160d14883C80e7C2c4F420b`
- `value_wei`: `0`
- `gas_limit`: `1000000`
- `tx_hash`: `0xe832365b11ead105e020b65a25570516e87ab1af2b1225b698561f090eff8b7c`
- `status`: `submitted`
- `error`: empty
- `created_at_ms`: `1779445862372`
- `updated_at_ms`: `1779445862372`
- `calldata_length`: `1674`
- `calldata_selector`: `0x031f77b3`

Counts since `TEST_START_MS=1779445529961`:

- `option_execution_transactions`: `1`
- `execution_transactions`: `0`
- rows for this tx hash in `option_execution_transactions`: `1`

## Receipt And Transaction

Receipt:

- `status`: `0 (failed)`
- `blockNumber`: `41838788`
- `blockHash`: `0x21307b4272a3fc0526e2a100c844cd037e81671c48d3f30cf939fefc57bc6b78`
- `from`: `0xc35F7A8A103A9A4464adfaa76B9B514093D23C27`
- `to`: `0xf2D1D85cD363Be3bc160d14883C80e7C2c4F420b`
- `gasUsed`: `982941`
- `effectiveGasPrice`: `6000000`
- `transactionIndex`: `15`
- `type`: `2`
- `logs`: `[]`
- `l1Fee`: `26374610050`
- `l1GasUsed`: `6024`

Transaction:

- `chainId`: `84532`
- `nonce`: `502`
- `gasLimit`: `1000000`
- `maxFeePerGas`: `1000000000`
- `maxPriorityFeePerGas`: `1000000`
- `value`: `0`
- `input_selector`: `0x031f77b3`

## Calldata Comparison

The on-chain transaction input exactly matches the preserved `option_execution_transactions.calldata`.

- database calldata file length with newline: `1675`
- transaction input file length with newline: `1675`
- `cmp`: `calldata_match=true`

Selector attribution:

- `0x031f77b3` = `executeTrade((bytes32,address,address,uint256,address,address,uint64,uint64,bool,uint128,uint128,uint128,bool,uint256,uint256,uint256),bytes,bytes)`

## Eth Call Results

Using the exact transaction input, `from=0xc35F7A8A103A9A4464adfaa76B9B514093D23C27`, `to=0xf2D1D85cD363Be3bc160d14883C80e7C2c4F420b`, and `value=0`:

| Block | Gas cap | Result |
| --- | ---: | --- |
| `41838777` | none | `0x`, exit `0` |
| `41838777` | `1000000` | `execution reverted`, exit `1` |
| `41838788` | none | `0x`, exit `0` |
| `41838788` | `1000000` | `execution reverted`, exit `1` |
| latest | none | `0x`, exit `0` |
| latest | `1000000` | `execution reverted`, exit `1` |
| `41838788` | `1040080` | `0x`, exit `0` |
| `41838788` | `1100000` | `0x`, exit `0` |
| `41838788` | `1200000` | `0x`, exit `0` |

`eth_estimateGas` returned `1040080` at block `41838777`, block `41838788`, and latest.

This explains why the original `simulation_ok` was possible: the simulation was run without a gas cap because `OPTION_EXECUTION_SIMULATION_GAS_LIMIT=0`, while the live transaction was constrained by `gasLimit=1000000`.

## Trace

`cast run` replayed the transaction and reported:

- `Error: Transaction failed.`
- top-level call: `OptionMatchingEngine.executeTrade(...)`
- trace gas used: `982941`
- final failure: `EvmError: OutOfGas` inside seller-side account risk computation

The trace reached all of the following before reverting:

- `OptionProductRegistry.getSeriesIfExists(...)`
- buyer signature recovery to `0xc0A76c2A6c6b70C0B065A05E64417886416cc976`
- seller signature recovery to `0xbAf0976a00a0DCc84Df5B15d927695c8b014B1c3`
- `MarginEngine.applyTrade(...)`
- premium transfer of `1000000` settlement units from buyer to seller
- buyer fee transfer of `600`
- seller fee transfer of `400`
- buyer risk computation returning successfully

The final failing segment was during seller risk computation, after seller option position state was being evaluated:

- `MarginEngine.computeAccountRisk(seller)`
- `OptionProductRegistry.getSeries(...)`
- `OutOfGas`

No top-level custom error selector was recovered. The gas-limited `eth_call` returned a generic `execution reverted`, and the replay trace showed `OutOfGas`.

A nested selector did appear during risk computation:

- `0x19abf40e` = `StalePrice()`

That selector came from a direct oracle price read during perp mark-price evaluation and was handled by surrounding safe/fallback risk logic. It was not the final top-level failure, because uncapped `eth_call` succeeds at the simulation block, tx block, and latest.

## State Checks

The following values were unchanged at block `41838777`, block `41838788`, and latest unless otherwise noted.

Option matching engine `0xf2D1D85cD363Be3bc160d14883C80e7C2c4F420b`:

- `isExecutor(0xc35F7A8A103A9A4464adfaa76B9B514093D23C27)`: `true`
- `paused`: `false`
- `nonces(buyer)`: `0`
- `nonces(seller)`: `0`

Series registry `0x3d52b033Fab00ed6104DD3bc0a715F8648344ecA`:

- `underlying`: `0x4DeEBc5f537F3b8ba0E3393807B4D699D72bDd02`
- `settlementAsset`: `0x6eAe407f5640B006faC9965182e238582A3B412E`
- `expiry`: `1893456000`
- `strike`: `300000000000`
- `contractSize1e8`: `100000000`
- `isCall`: `true`
- `isEuropean`: `true`
- `exists`: `true`
- `isActive`: `true`

Margin engine `0x6C5665De05e7314cB63cD77F82DFa86508A5b5F8`:

- `paused`: `false`
- `tradingPaused`: `false`
- buyer option position: `0`
- seller option position: `0`
- `seriesActivationState`: `0`
- `seriesEmergencyCloseOnly`: `false`
- `seriesShortOpenInterest`: `0`
- `seriesShortOpenInterestCap`: `10000000000`

Collateral vault `0x00340C360353a5AB784c5Bc5c44322A6AF0625D3`:

- buyer settlement balance: `9998500000`
- seller settlement balance: `9999400000`
- `internalTransfersPaused`: `false`
- `isEngineAuthorized(marginEngine)`: `true`

Perp state included in account risk:

- buyer market count: `1`
- seller market count: `1`
- buyer market `1` position: `100000000`
- seller market `1` position: `-100000000`

Oracle router `0xB416406F200B2Ef3D7a86A5D5877Ed41D9B1A581`:

- `paused`: `false`
- `readPaused`: `false`
- `maxOracleDelay`: `600`
- `hasActiveFeed(underlying, settlement)`: `true`
- `getPriceSafe(underlying, settlement)`: `(0, 0, false)`
- feed config: primary `0x3eb9cdd2C2115c3f0DF5E30da53D7245F9a5f6Cc`, secondary `0x2103a84C0CAB9cf7680d602C8931FaDeD7064517`, feed max delay `60`, max deviation `1000`, active `true`
- primary latest price: `300000000000`, updated at `1779357928`
- secondary latest price: `300000000000`, updated at `1779357928`

Block timestamps:

- block `41838777`: `1779445842`
- block `41838788`: `1779445864`

The oracle feed was stale during the attempt, but the exact calldata still succeeds when the gas cap is removed or raised. The stale oracle state contributes to the risk-computation path observed in the trace, but the mined transaction failed because the live gas cap was too low.

## Root Cause

The live transaction failed out-of-gas.

The backend broadcast used `gasLimit=1000000`, inherited from `EXECUTOR_MAX_GAS_LIMIT` because `OPTION_EXECUTION_BROADCAST_GAS_LIMIT=0`. The pre-broadcast simulation used no gas cap because `OPTION_EXECUTION_SIMULATION_GAS_LIMIT=0`.

Read-only reproduction:

- same calldata with no gas cap succeeds
- same calldata with `gasLimit=1000000` reverts
- `eth_estimateGas` returns `1040080`
- same calldata with `gasLimit=1040080` succeeds
- trace shows `OutOfGas`

This is a gas configuration and preflight mismatch, not a calldata, signature, nonce, executor, balance, or inactive-series failure.

## Backend Patch Assessment

No backend code patch was applied for this diagnosis.

The backend behaved according to the documented configuration:

- option simulation omits the gas field when `OPTION_EXECUTION_SIMULATION_GAS_LIMIT=0`
- option broadcast uses `OPTION_EXECUTION_BROADCAST_GAS_LIMIT` when nonzero, otherwise `EXECUTOR_MAX_GAS_LIMIT`

The failure indicates the live broadcast runbook/configuration should require a gas-capped simulation that matches or exceeds the intended broadcast gas cap, or should estimate gas and apply a safety margin before enabling a real broadcast.

## Recommended Next Action

Do not retry this submitted transaction automatically.

Before any next live option execution attempt:

1. Refresh or otherwise validate testnet oracle feeds so `getPriceSafe(underlying, settlement)` is true.
2. Estimate gas for the exact prepared calldata immediately before broadcast.
3. Set `OPTION_EXECUTION_BROADCAST_GAS_LIMIT` above the estimate with margin, for example at least `1250000` for this observed path.
4. Set `OPTION_EXECUTION_SIMULATION_GAS_LIMIT` to the same cap, or higher, so `simulation_ok` proves the intended gas-constrained call.
5. Create a fresh intent/signature flow, because the preserved intent is already marked `broadcast_submitted` with this failed tx hash.

## Validation Commands

Read-only evidence commands were run with `psql`, `cast receipt`, `cast tx`, `cast call`, `cast estimate`, `cast run`, `cast sig`, `cast block`, and `rg`.

No forbidden mutation was observed after evidence collection:

- `option_execution_transactions` since `TEST_START_MS`: `1`
- `execution_transactions` since `TEST_START_MS`: `0`
- rows for tx hash in `option_execution_transactions`: `1`
