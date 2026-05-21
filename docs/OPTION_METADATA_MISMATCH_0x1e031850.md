# Option Metadata Mismatch - 0x1e031850

## Summary

`0x1e031850` is `SeriesMetadataMismatch()` from `OptionMatchingEngine`.

The deployed `OptionMatchingEngine` at `0xf2D1D85cD363Be3bc160d14883C80e7C2c4F420b`
is wired to `OptionProductRegistry` at `0x3d52b033Fab00ed6104DD3bc0a715F8648344ecA`.

Root cause: the backend accepted an `onchain_option_id` without proving that the
local option metadata hashed to the same id that `OptionProductRegistry` uses.
A stale or manually mismatched local `option_series` row could therefore produce
a signing payload and calldata whose `optionId` existed on-chain but whose
`underlying`, `settlementAsset`, `expiry`, `strike1e8`, `isCall`, or
`contractSize1e8` did not match `getSeries(optionId)`.

## Solidity Validation

`OptionMatchingEngine._validateSeriesMetadata()` checks:

- `registry.getSeriesIfExists(t.optionId)` exists.
- `series.isActive == true`.
- `series.underlying == t.underlying`.
- `series.settlementAsset == t.settlementAsset`.
- `series.expiry == t.expiry`.
- `series.strike == t.strike1e8`.
- `series.isCall == t.isCall`.
- `series.contractSize1e8 == t.contractSize1e8`.

`isEuropean` is part of the registry series and option id computation, but it is
not part of `OptionTrade` and is not checked by `OptionMatchingEngine`.

## On-Chain Registry

Read with `eth_call`:

| Field | Value |
| --- | --- |
| optionId | `24145907678156652148089862289363692212069910767044828147380657249455352740183` |
| underlying | `0x4DeEBc5f537F3b8ba0E3393807B4D699D72bDd02` |
| settlementAsset | `0x6eAe407f5640B006faC9965182e238582A3B412E` |
| expiry | `1893456000` |
| strike | `300000000000` |
| contractSize1e8 | `100000000` |
| isCall | `true` |
| isEuropean | `true` |
| exists | `true` |
| isActive | `true` |

## Backend Reproduction

Initial local database state had no `option_execution_intents` row for the known
`optionId`. After the patch, a new safe off-chain orderbook fill was created
with exact on-chain metadata.

Reproduction intent:

| Field | Value |
| --- | --- |
| intent_id | `6ac7db54-f30c-4964-a863-c8484fcf3b11` |
| onchain_intent_id | `0xb7ce8e14ca32b49bcb4b857f6e648ab4b48f9ec5a4c9650549430d3f2e6b933e` |
| source_type | `option_orderbook_fill` |
| source_id | `5ecc770c-2f22-4e7e-9e2c-b7ca1fce62d2` |
| option_series_id | `0x8b34d095ebfb300f21868dea4a0ff5e1d6f8ebd5463facaa8bcbc6075df50e6d` |
| status | `calldata_ready` |
| simulation_status | `simulation_ok` |

## Field Comparison

| Field | On-chain registry | DB intent | Signing payload | Decoded calldata | Result |
| --- | --- | --- | --- | --- | --- |
| intentId | n/a | `0xb7ce...933e` | `0xb7ce...933e` | `0xb7ce...933e` | match |
| buyer | n/a | `0xc0A76c2A6c6b70C0B065A05E64417886416cc976` | same | same | match |
| seller | n/a | `0xbAf0976a00a0DCc84Df5B15d927695c8b014B1c3` | same | same | match |
| optionId | `24145907678156652148089862289363692212069910767044828147380657249455352740183` | same | same | same | match |
| underlying | `0x4DeEBc5f537F3b8ba0E3393807B4D699D72bDd02` | same | same | same | match |
| settlementAsset | `0x6eAe407f5640B006faC9965182e238582A3B412E` | same | same | same | match |
| expiry | `1893456000` | same | same | same | match |
| strike1e8 | `300000000000` | same | same | same | match |
| isCall | `true` | same | same | same | match |
| isEuropean | `true` | not stored in intent | not in `OptionTrade` | not in `OptionTrade` | n/a |
| contractSize1e8 | `100000000` | same | same | same | match |
| quantity | n/a | `1` | `1` | `1` | separate from contract size |
| source_size_1e8 | n/a | `100000000` | n/a | n/a | converts to quantity `1` |
| premiumPerContract | n/a | `10000000` | `10000000` | `10000000` | match |
| buyerIsMaker | n/a | `false` | `false` | `false` | match |
| buyerNonce | on-chain `0` | `0` | `0` | `0` | match |
| sellerNonce | on-chain `0` | `0` | `0` | `0` | match |
| deadline | n/a | `0` | `0` | `0` | no deadline |

## Patch Summary

Backend patch was needed.

Changes:

- Added registry-compatible option id computation using the same ABI encoding as
  `OptionProductRegistry._computeOptionId()`.
- Added validation that accepts only option ids matching the local metadata for
  either `isEuropean=true` or `isEuropean=false`.
- Applied the guard both when building executable option intents and when
  converting stored intents into `OptionTradePayload`.
- Updated tests and fixtures to use registry-derived option ids instead of dummy
  ids.
- Added regressions for metadata mismatch rejection, live-series id computation,
  contract-size encoding, separate quantity encoding, and calldata decoding.

## Post-Fix Simulation

Runtime flags used:

```text
EXECUTION_ENABLED=false
EXECUTOR_REAL_BROADCAST_ENABLED=false
MM_GATEWAY_ENABLED=false
OPTION_EXECUTION_BROADCAST_ENABLED=false
```

Simulation endpoint:

```text
POST /options/execution-intents/6ac7db54-f30c-4964-a863-c8484fcf3b11/simulate
```

Result:

| Field | Value |
| --- | --- |
| simulation_status | `simulation_ok` |
| block_number | `41804194` |
| revert_selector | `null` |
| error | `null` |
| submitted | `false` |
| confirmed | `false` |

`SeriesMetadataMismatch()` is gone.
`InvalidSignature()` is gone.

## Safety Check

- No `/executor/broadcast` call was made.
- No `/options/execution-intents/:id/broadcast` call was made.
- `option_execution_transactions` row count remained `0`.
- `execution_transactions` row count remained at its pre-existing count of `1`.
- No transaction was submitted or broadcast.
- No private keys were printed.

