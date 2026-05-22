# Option Live Broadcast Phase B Preview V1Q

Date: 2026-05-22

## Scope

V1Q runs every Phase B step up to (but excluding) the broadcast call: it
validates the backend gas safety patch, refreshes the Base Sepolia mock
oracle feeds, runs the read-only on-chain preflight, starts the backend,
seeds a brand-new orderbook fill, drives the freshly-created option
execution intent through signing, calldata, and live simulation, and then
captures the gas safety preview using a read-only `cast estimate-gas`.

No broadcast was performed, no `/executor/broadcast` call was made, no
option execution transaction row was created, no generic
`execution_transactions` row was created, no evidence row was cleaned up,
no Solidity or frontend was modified, no contract was deployed, and no
private keys were printed.

## Repo State

- Repository: `~/DEOPT/deopt-v2-backend`
- Branch: `main` (up to date with `origin/main`)
- Last commit: `f36968e 22/05` — contains the V1O gas safety patch (migration `0022_option_execution_gas_safety.sql`, `OptionExecutionGasCheckStatus`, `perform_option_broadcast_gas_safety_check`, `OPTION_EXECUTION_GAS_SAFETY_BPS_DEFAULT=12500`, tests).
- Dirty files: `NEXT_TASK.md` (Phase B prompt) and the new doc below.
- Validation:
  - `cargo fmt --all -- --check` → clean
  - `cargo clippy --all-targets --all-features -- -D warnings` → clean
  - `cargo test --all-targets --all-features` → 225+ lib tests + 13 + 37 + 67 + 76 + 12 + 43 + 8 = 0 failures across all suites
  - `cargo build --all-targets --all-features` → clean

## Oracle Refreshed In V1Q

When V1Q opened, the mock feeds were 524 s old (router `getPriceSafe(WETH,USDC) = (0, 0, false)`). `script/RefreshTestnetMockFeeds.s.sol` was re-run via `forge script … --broadcast --slow` on chain id `84532` with `TESTNET_MOCKS_ENABLED=true` and `block.chainid != 8453` guards both satisfied.

| Tx hash | To | Block | Status |
| --- | --- | ---: | --- |
| `0x136157e4f7188f8a474555f66dddea4f68189743a77d33bb2da6bd41a8a3da70` | `0x3eb9cdd2C2115c3f0DF5E30da53D7245F9a5f6Cc` (ETH/USDC primary) | 41846400 | `0x1` |
| `0x9f638191df0ae6f41b865747032ade7015d2497cd7cfd53e8bd486f5ecbf176c` | `0x8cbA01B3f4e818ffffD6c1aE1f9a18A656e918bB` (BTC/USDC primary) | 41846401 | `0x1` |
| `0x0807c887ac59a90444231a84d30e2996d743ee24cdc4b84030f20ddb029f23b3` | `0x2103a84C0CAB9cf7680d602C8931FaDeD7064517` (ETH/USDC secondary) | 41846402 | `0x1` |
| `0x505c3f216535c77c6464542c092b4334a9c1292060b6fa318848940750839dcc` | `0x7206E7c2c1C3D6e6273020163EB1f0E9339b970C` (BTC/USDC secondary) | 41846403 | `0x1` |

Manifest: `~/DEOPT/deopt-v2-sol/broadcast/RefreshTestnetMockFeeds.s.sol/84532/run-latest.json`.

## Oracle Status (after refresh)

- Block timestamp at probe: `1779461104`
- ETH/USDC primary `getLatestPrice() = (300_000_000_000, 1779461088)` (age 16 s)
- ETH/USDC secondary `getLatestPrice() = (300_000_000_000, 1779461088)` (similar)
- `OracleRouter.paused() = false`
- `OracleRouter.readPaused() = false`
- `OracleRouter.maxOracleDelay() = 600 s`
- Feed `maxDelay = 60 s`, `maxDeviationBps = 1000`, `isActive = true`
- `OracleRouter.getPriceSafe(WETH, USDC) = (300_000_000_000, 1779461088, true)` — fresh, ok=true, price > 0

Later, after running through signing/simulation, the oracle was still fresh — the simulation succeeded at block `41846512`, with feed timestamps within the 60 s window.

## Backend / Admin Config Summary

Backend started locally with the V1O env (helper `/tmp/deopt-live-env.sh`).

`/health` → `{ok: true}`.

All gates verified against `/admin/config`:

| Check | Value |
| --- | --- |
| `option_execution_enabled` | true |
| `option_execution_broadcast_enabled` | true |
| `execution_enabled` | true |
| `real_broadcast_enabled` | true |
| `executor_dry_run` | false |
| `option_matching_engine_address` | `0xf2D1D85cD363Be3bc160d14883C80e7C2c4F420b` |
| `execution_gas_safety_bps` | 12500 |
| `execution_broadcast_gas_limit` | 1300000 |
| `execution_signature_mode` | strict |
| `execution_require_simulation_ok` | true |
| `option_nonce_sync_enabled` | true |

## On-chain Preflight

Derived public addresses (from local keys, no secrets echoed):

- Buyer:    `0xc0A76c2A6c6b70C0B065A05E64417886416cc976` ✓
- Seller:   `0xbAf0976a00a0DCc84Df5B15d927695c8b014B1c3` ✓
- Executor: `0xc35F7A8A103A9A4464adfaa76B9B514093D23C27` ✓

On-chain reads against engine `0xf2D1D85c…`:

- `isExecutor(executor) = true`
- `paused() = false`
- `nonces(buyer) = 0`
- `nonces(seller) = 0`

Registry `0x3d52b033…`:

- `getSeriesIfExists(<optionId>) = (0x4DeEBc5f…, 0x6eAe407f…, 1893456000, 300000000000, 1e8, isCall=true, isEuropean=true, exists=true, isActive=true)` ✓

Oracle re-confirmed fresh just before fill creation.

Executor balance check:

- Balance: `8_123_622_380_830_706` wei (~`0.00812` ETH)
- Worst-case cost = `OPTION_EXECUTION_BROADCAST_GAS_LIMIT × EXECUTOR_MAX_FEE_PER_GAS_WEI` = `1_300_000 × 1_000_000_000` = `1.3 × 10^15` wei (`0.0013 ETH`)
- Ratio balance / worst-case = **6.25×** — comfortable
- Current base fee: `5_000_000` wei (`0.005` gwei) — `maxFeePerGas` of 1 gwei has plenty of headroom

## Fresh Fill & Intent

Two crossing GTC orders submitted via `POST /options/orders` against the active series `0x8b34d095…` (which maps to on-chain `optionId 24145907678156652148089862289363692212069910767044828147380657249455352740183`):

- Sell order: `8c5ad379-bb35-439b-b4a6-778271d84111` (account = seller, price_1e8 = 1_000_000, size_1e8 = 1e8)
- Buy order:  `16911853-0432-4032-ab97-6b4472303ca5` (account = buyer, same price/size; status `filled` immediately on submit)

The buy crossed the resting sell and produced fresh fill **`81b3e1a8-52ef-4bc7-a947-98b60df8e842`**.

This fill triggered creation of a brand-new option execution intent:

- **`intent_id = e6d2941b-65f7-413a-958f-74ab22c53b08`**
- `onchain_intent_id = 0x0a77c7c9570198c969b1fa597ea193cb6fee563e3bfae514e9a3f0c4e01705f5`
- `source_type = option_orderbook_fill`, `source_id = 81b3e1a8-…`
- `option_series_id = 0x8b34d095…`, `onchain_option_id = 24145907678156652148089862289363692212069910767044828147380657249455352740183`
- `quantity_contracts = 1`, `premium_per_contract_native = 10_000`
- `buyer_nonce = 0`, `seller_nonce = 0` (synced live from on-chain)
- `buyer_is_maker = false` (resting side was sell)
- Initial status `signatures_required`

This intent is brand-new — not `4075afe3-…` (V1L failed) and not `6ac7db54-…` (older orphan).

## Signing & Simulation

EIP-712 signing payload fetched via `GET /options/execution-intents/<id>/signing-payload`:

- `digest = 0x87dbb142bb70a08d18730fbf40b73cddb4a3bd31020775b03b6ddb53f41d3df8`
- `primaryType = OptionTrade`
- `domain.name = DeOptV2-OptionMatchingEngine`, `version = 1`, `chainId = 84532`, `verifyingContract = 0xf2D1D85c…`

Buyer and seller signatures generated locally with `cast wallet sign --no-hash <digest>`, verified locally with `cast wallet verify --no-hash`:

- Buyer signature recovers to `0xc0A76c2A…` ✓
- Seller signature recovers to `0xbAf0976a…` ✓

Strict signature submission via `POST /options/execution-intents/<id>/signatures` returned `status = calldata_ready`, `buyer_signature_present = true`, `seller_signature_present = true`, `calldata_ready = true`, `missing_signatures = false`. The calldata length is `1674` chars / `837` bytes (selector `0x031f77b3` = `executeTrade(OptionTrade,bytes,bytes)`).

`POST /options/execution-intents/<id>/simulate` returned:

```json
{
  "intent_id": "e6d2941b-65f7-413a-958f-74ab22c53b08",
  "simulation_status": "simulation_ok",
  "block_number": 41846512,
  "error": null,
  "revert_data": null,
  "revert_selector": null,
  "simulated_at_ms": 1779461311800,
  "submitted": false,
  "confirmed": false
}
```

The simulation passed against a fresh oracle. This is the first time the live `eth_call` against the OptionMatchingEngine has returned `simulation_ok` since the patch — the V1L `cast run` had reached the seller risk computation before reverting; here, with a fresh oracle, the call completes without revert.

## Gas Safety Preview

Read-only `cast estimate-gas` against the prepared transaction (`from = executor`, `to = OptionMatchingEngine`, `data = intent.calldata`, `value = 0`):

| Field | Value |
| --- | ---: |
| `estimated_gas` | **1_040_068** |
| `gas_safety_bps` | **12500** |
| `required_gas` = `estimated_gas × 12500 / 10000` | **1_300_085** |
| `broadcast_gas_limit` | **1_300_000** |
| `broadcast_gas_limit >= estimated_gas` | **true** |
| `broadcast_gas_limit >= required_gas` | **false** |
| Headroom over `required_gas` | **−85 gas** |

The configured broadcast cap is `1_300_000`. The required cap (estimate plus 25% safety margin) is `1_300_085`. The cap is **85 gas short** of `required_gas`, so the V1O gas safety check will emit `gas_check_status = below_safety_margin` and reject the broadcast before signing or sending.

This is the V1O safety gate behaving exactly as designed: even though the cap is well above the bare estimate (estimate is ~1.04 M, cap is 1.30 M, so the underlying tx itself would almost certainly succeed if blindly broadcast), the patch will refuse it because the cap does not clear the 25% safety margin. The margin exists precisely to absorb the kind of gas-cost drift that broke V1L (whose `gasUsed = 982_941` exceeded the V1L cap of `1_000_000` by `+8 _294` on a path the simulation never gas-constrained).

## DB Baseline Check

`TEST_START_MS = 1779461197288`. After all V1Q activity:

| Table | Rows since `TEST_START_MS` |
| --- | ---: |
| `option_execution_intents` | 1 (the new `e6d2941b-…`) |
| `option_execution_transactions` | **0** |
| `execution_transactions` | **0** |

The new intent is in `status = calldata_ready`, `simulation_status = simulation_ok`, `simulation_block_number = 41846512`.

The preserved V1L evidence row (`transaction_id = 204a3070-…`, `tx_hash = 0xe832365b…`, on intent `4075afe3-…`) is unchanged.

## Whether Human Can Authorize The Broadcast

**No — broadcast must NOT be authorized at the current configuration.** With `estimated_gas = 1_040_068`, the V1O safety gate requires `broadcast_gas_limit >= 1_300_085`, and the current cap is `1_300_000`. The backend will return `BroadcastRejected("broadcast gas limit 1300000 is below required_gas 1300085 (estimated_gas 1040068 * safety 12500 bps)")`, persist a `failed` `option_execution_transactions` row with `gas_check_status = below_safety_margin`, and move the intent to `broadcast_failed`.

Two paths forward (operator picks):

1. **Recommended:** raise `OPTION_EXECUTION_BROADCAST_GAS_LIMIT` to a value comfortably above `required_gas`. Suggested: `1_500_000` or `1_600_000` (more than the next-block-level retry would need, and well below current basefee × balance constraints). After restart, re-run V1Q steps 7–11; expect `gas_check_status = ok`.
2. Alternative: tighten `OPTION_EXECUTION_GAS_SAFETY_BPS` to `12498` (still ≥ floor `10000`) so the existing cap clears the margin. This defeats some of the V1O intent and is not recommended.

Either way, after re-running with a new config, the operator should issue a separate explicit "yes, broadcast now" confirmation before Phase B fires the broadcast endpoint.

## Backend Process

The backend was started under `nohup` for V1Q (PID logged to `/tmp/deopt-backend.log`). It can stay running while the operator decides the next config. Killing it before the next attempt is fine and will not lose the intent (intent + signatures + calldata + simulation result are all persisted in Postgres).

## No-Forbidden-Mutation Verification

- `POST /options/execution-intents/<id>/broadcast`: **not called**
- `/executor/broadcast`: **not called**
- No `OptionTrade` transaction signed for `eth_sendRawTransaction`; the only signatures produced were the two EIP-712 user signatures consumed by the backend's strict signature submission
- No `option_execution_transactions` row created since `TEST_START_MS`
- No `execution_transactions` row created since `TEST_START_MS`
- Preserved V1L evidence rows untouched
- No Solidity or frontend source modified
- No contracts deployed; only `MockPriceSource.setPrice` calls broadcast in V1Q to refresh oracle feeds
- No private keys printed
- No `confirmed` / `reconciled` rows written for any intent

## Files Changed

- `docs/OPTION_LIVE_BROADCAST_PHASE_B_PREVIEW_V1Q.md` (new, in backend repo; not committed)
- `~/DEOPT/deopt-v2-sol/broadcast/RefreshTestnetMockFeeds.s.sol/84532/run-latest.json` (forge artifact; not a tracked source change)
- `~/DEOPT/deopt-v2-sol/cache/RefreshTestnetMockFeeds.s.sol/84532/run-latest.json` (forge cache)
- `/tmp/deopt-live-env.sh` (helper; not tracked; unchanged since V1O Phase A)
- `/tmp/deopt-phase-b-test-start.txt` (records `TEST_START_MS`)

## Remaining Blockers Before The Live Broadcast

1. **Raise `OPTION_EXECUTION_BROADCAST_GAS_LIMIT` above `required_gas = 1_300_085`.** Suggested: `1_500_000` for conservative headroom that also handles future basefee/cost movement. After the change, restart the backend and re-run the gas safety preview to confirm `broadcast_gas_limit >= required_gas`.
2. **Oracle freshness window.** Plan for the broadcast call to land within ~50 s of the next mock feed refresh. If V1Q halt-to-resume takes longer than that, re-run `script/RefreshTestnetMockFeeds.s.sol` immediately before the broadcast.
3. **Re-verify executor balance** at attempt time. Right now it is ~0.00812 ETH vs worst-case 0.0013 ETH, but the upcoming cap raise increases the worst-case cost proportionally (e.g. cap = 1.5 M → worst-case 0.0015 ETH). Comfortable headroom either way, but worth re-checking.
4. **Explicit human authorization** ("yes, broadcast now") is required before the broadcast endpoint is called.
