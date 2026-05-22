# Option Oracle Refresh Preflight V1P

Date: 2026-05-22

## Scope

V1P refreshes the Base Sepolia mock oracle feeds that underpin the WETH/USDC option series so that Phase B (the first successful live option execution broadcast) can run against a non-stale oracle. The backend gas safety patch (V1O) is already in place on `main`; this phase performs no broadcast through the option execution endpoint, creates no option execution intent, signs no `OptionTrade`, simulates no option execution, and does not call `/executor/broadcast`. The only on-chain mutation is four `MockPriceSource.setPrice` calls executed by the existing `script/RefreshTestnetMockFeeds.s.sol` script.

## Backend State

- Repository: `~/DEOPT/deopt-v2-backend`
- Branch: `main` (up to date with `origin/main`)
- V1O gas safety patch commit: `f36968e` (pushed)
- Working tree dirty file: `NEXT_TASK.md` only (Phase B prompt)
- Validation:
  - `cargo fmt --all -- --check` — clean
  - `cargo clippy --all-targets --all-features -- -D warnings` — clean
  - `cargo test --all-targets --all-features` — 225+ library tests pass; integration suites pass; 0 failures
  - `cargo build --all-targets --all-features` — clean
- No new commit was created in V1P because the patch is already on `origin/main` under `f36968e` ("22/05"). A redundant commit would either be empty or require `--amend`, which is forbidden by the project's commit safety protocol. The committed change is identical in scope to the requested "Harden option broadcast gas safety" patch.

## Mock Feed Refresh

Solidity repository: `~/DEOPT/deopt-v2-sol`

Script: `script/RefreshTestnetMockFeeds.s.sol`

Safety properties of the script (read-only audit):

- Refuses Base mainnet (`block.chainid == 8453` → revert).
- Requires `TESTNET_MOCKS_ENABLED=true`.
- Requires `CHAIN_ID` to match `block.chainid` when both are present.
- Calls only `MockPriceSource.setPrice(uint256)` on already-deployed feed addresses; deploys nothing.
- Touches only ETH/USDC and BTC/USDC primary and secondary feeds — no other contracts.

### Environment values (sanitized)

| Key | Value |
| --- | --- |
| `CHAIN_ID` | `84532` |
| `TESTNET_MOCKS_ENABLED` | `true` |
| `DEPLOYER_ADDRESS` | `0xc35F7A8A103A9A4464adfaa76B9B514093D23C27` (same as executor) |
| `ORACLE_ROUTER` | `0xB416406F200B2Ef3D7a86A5D5877Ed41D9B1A581` |
| `ETH_UNDERLYING` (WETH) | `0x4DeEBc5f537F3b8ba0E3393807B4D699D72bDd02` |
| `BASE_COLLATERAL_TOKEN` (USDC) | `0x6eAe407f5640B006faC9965182e238582A3B412E` |
| `OPTION_PRODUCT_REGISTRY` | `0x3d52b033Fab00ed6104DD3bc0a715F8648344ecA` |
| `OPTION_MATCHING_ENGINE` (live) | `0xf2D1D85cD363Be3bc160d14883C80e7C2c4F420b` |
| `ETH_USDC_PRIMARY_SOURCE` | `0x3eb9cdd2C2115c3f0DF5E30da53D7245F9a5f6Cc` |
| `ETH_USDC_SECONDARY_SOURCE` | `0x2103a84C0CAB9cf7680d602C8931FaDeD7064517` |
| `BTC_USDC_PRIMARY_SOURCE` | `0x8cbA01B3f4e818ffffD6c1aE1f9a18A656e918bB` |
| `BTC_USDC_SECONDARY_SOURCE` | `0x7206E7c2c1C3D6e6273020163EB1f0E9339b970C` |

`DEPLOYER_PRIVATE_KEY` was sourced from `.env.base-sepolia` for the broadcast and never echoed. `OracleRouter.getFeed(WETH, USDC)` returns `(0x3eb9cdd2…, 0x2103a84C…, 60, 1000, true)`, confirming the feed addresses match the configured pair.

### Updater authority

`MockPriceSource.owner()` was read for each of the four feeds before broadcast:

| Feed | Owner | Matches `DEPLOYER_ADDRESS`? |
| --- | --- | --- |
| `0x3eb9cdd2…` (ETH primary) | `0xc35F7A8A…` | yes |
| `0x2103a84C…` (ETH secondary) | `0xc35F7A8A…` | yes |
| `0x8cbA01B3…` (BTC primary) | `0xc35F7A8A…` | yes |
| `0x7206E7c2…` (BTC secondary) | `0xc35F7A8A…` | yes |

Deployer balance pre-refresh: `8_125_213_134_998_515` wei (~`0.00813` ETH). Forge estimated `0.000002010382` ETH for the script — well below available balance.

### Refresh transactions

`forge script script/RefreshTestnetMockFeeds.s.sol:RefreshTestnetMockFeeds --rpc-url <base-sepolia> --broadcast --slow` was run from `~/DEOPT/deopt-v2-sol` with the loaded environment. The script logged `chainId=84532`, `updatedAt=1779460534`, `ETH_USDC_PRICE_1E8=300000000000`, `BTC_USDC_PRICE_1E8=6500000000000`.

| Tx hash | To | Block | Status | Gas used |
| --- | --- | ---: | --- | ---: |
| `0x0d613083d0add8535c41e613dfe9ff7be820c9c6af61a0366d3c786d9005c1d6` | `0x3eb9cdd2C2115c3f0DF5E30da53D7245F9a5f6Cc` | `41846000` | `0x1` (success) | `32_129` |
| `0x763a44deaf97c012ef34c078834f11b3cfa78e48a5e55cf1edec7315953098e8` | `0x8cbA01B3f4e818ffffD6c1aE1f9a18A656e918bB` | `41846001` | `0x1` (success) | `32_141` |
| `0x12cbd3c4102025eeeb438ecbeeb6ff8edb9f48d0a39c3c46ea0673e2fc5d396a` | `0x2103a84C0CAB9cf7680d602C8931FaDeD7064517` | `41846002` | `0x1` (success) | `32_129` |
| `0xe3d04149d561dd6bf0b7032f6179e535126d4584fb40ab4500d97749837bb21f` | `0x7206E7c2c1C3D6e6273020163EB1f0E9339b970C` | `41846003` | `0x1` (success) | `32_141` |

Full broadcast manifest: `broadcast/RefreshTestnetMockFeeds.s.sol/84532/run-latest.json`.

## Before / After Oracle Status

### Before refresh

- `block_number=41846112`, `block_timestamp=1779460512`
- ETH primary `getLatestPrice() = (300_000_000_000, 1779357928)` — last updated 102_584 s ago
- ETH secondary `(300_000_000_000, 1779357928)` — same
- BTC primary `(6_500_000_000_000, 1779357928)` — same
- BTC secondary `(6_500_000_000_000, 1779357928)` — same
- `OracleRouter.paused() = false`, `readPaused() = false`, `maxOracleDelay() = 600`
- `OracleRouter.getPriceSafe(WETH, USDC) = (0, 0, false)` — **stale**

This is the same stale-oracle condition surfaced by the `StalePrice()` selector in the V1L `cast run` trace of the failed broadcast `0xe832365b…`.

### After refresh

- `block_number=41846152`, `block_timestamp=1779460592`
- ETH primary `getLatestPrice() = (300_000_000_000, 1779460544)` — 48 s old
- ETH secondary `(300_000_000_000, 1779460548)` — 44 s old
- BTC primary `(6_500_000_000_000, 1779460546)` — 46 s old
- BTC secondary `(6_500_000_000_000, 1779460550)` — 42 s old
- `OracleRouter.paused() = false`, `readPaused() = false`, `maxOracleDelay() = 600`
- Feed config (WETH, USDC): primary `0x3eb9cdd2…`, secondary `0x2103a84C…`, `maxDelay=60`, `maxDeviationBps=1000`, `isActive=true`
- `OracleRouter.getPriceSafe(WETH, USDC) = (300_000_000_000, 1779460544, true)` — **fresh, ok=true, price>0**

All four feeds are within both the 60 s per-feed staleness cap and the 600 s router cap. The price matches the strike 1e8 used by the active series, so `MarginEngine.computeAccountRisk` should now satisfy the spot-oracle inputs that previously returned `(0, 0, false)`.

## Backend / DB

The backend was not started in V1P (no need — the action ran via forge against the Solidity repo). The V1O migration `0022_option_execution_gas_safety.sql` remains applied on the live DB from V1O preflight.

DB mutation summary since V1P start:

- `option_execution_intents` new rows: **0**
- `option_execution_transactions` new rows: **0**
- `execution_transactions` new rows: **0**
- Preserved V1L evidence row `204a3070-…` / tx `0xe832365b…`: **untouched**

## Forbidden-Action Verification

- `POST /options/execution-intents/:id/broadcast`: not called.
- `POST /options/execution-intents`: not called.
- `POST /executor/broadcast`: not called.
- No simulation requests were sent through the backend.
- No `OptionTrade` was signed.
- No backend HTTP server was running during the refresh.
- No frontend or Solidity source was modified. Only `MockPriceSource.setPrice` calls were broadcast.
- No evidence rows were cleaned up.
- No private keys were printed.

## Phase B Readiness

Phase B (first successful live option execution broadcast) can proceed:

- Backend gas safety patch present, tested, committed, and pushed.
- DB schema includes the gas-check columns.
- Oracle is fresh and `OracleRouter.getPriceSafe(WETH, USDC)` returns `(price, ts, true)`.
- Executor / buyer / seller addresses validated in V1O preflight (`isExecutor=true`, both nonces `0`).
- Active option series `24145907678156652148089862289363692212069910767044828147380657249455352740183` is `exists=true`, `isActive=true` against the canonical engine `0xf2D1D85c…`.

Remaining blockers before triggering the broadcast endpoint:

1. **Oracle freshness window.** The mock feeds last updated near block timestamp `1779460544`. The per-feed `maxDelay=60` s is tight. The whole Phase B sequence (start backend → on-chain preflight → create intent → sign locally → submit signatures → simulate → broadcast) should fit comfortably under 60 s once started, but if it stalls the simulation and/or broadcast will revert with `StalePrice()` again. Plan to either run quickly or re-run V1P immediately before Phase B if there is a delay.
2. **Fresh option fill source.** The existing intent `4075afe3-…` is `broadcast_submitted` against the failed tx hash; `6ac7db54-…` is `calldata_ready` from a stale earlier fill. Phase B must create a brand new orderbook crossing (or RFQ fill) to seed a fresh `option_execution_intents` row — neither of the existing intents can be reused.
3. **Executor balance check at broadcast time.** Phase A measured 0.00813 ETH; the refresh consumed roughly `4 × 32_000 × gas_price` (tens of microether). The broadcast itself with `gasLimit=1_300_000` and `maxFeePerGas` configured in the backend env will be far larger. Verify the executor still holds enough wei to cover `gas_limit × max_fee_per_gas_wei` immediately before the broadcast and abort if not.
4. **Explicit broadcast authorization.** Per V1O Phase A protocol, the final `POST /options/execution-intents/<id>/broadcast` requires a separate "yes, broadcast now" confirmation from the operator after the gas estimate preview is reported.
