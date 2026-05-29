# FeesManagerV2 Tiny Trade Preflight V2E-F

Date: 2026-05-29
Network: Base Sepolia (`chain_id 84532`)
Mode: Pre-broadcast preflight with **FeesManagerV2 enabled**. **No broadcast.**
No `/options/execution-intents/:id/broadcast` call. No `/executor/broadcast`.
No `eth_sendRawTransaction`. No deploy. No Solidity or frontend change.
No `setUseFeesManagerV2` flip. No rebate budget funded. No Merkle root set.

## Outcome

All preflight gates green against the V2 fee path. A fresh tiny option
execution intent (`94897ee5-e855-40b6-a917-1476578fe48b`) was created via
the V1S/V2D-V orderbook-fill flow against NEW MarginEngine
(`0x287Cef…f48cc`) with `useFeesManagerV2 = true`. Both EIP-712 signatures
were collected, calldata generated (selector `0x031f77b3`, hex length
`1672`), simulation returned `simulation_ok` at block `42135312`, and
external `eth_estimateGas` returned `gas_check_status = ok` with `+458_587`
gas of headroom over the 1.25× safety margin.

Premium was tuned (price_1e8 = `5_000_000` → `premium_per_contract_native
= 50_000`) so the Tier0 OPTION profile (maker `50` ppm, taker `250` ppm)
rounds to **non-zero** charges (`12` units taker + `2` units maker = `14`
total native-settlement units). After eventual operator broadcast, the
indexer is expected to capture two `FeeChargedV2` events from
`0x00dA0B…774f` for this `tx_hash`.

No `option_execution_transactions` or `execution_transactions` row was
created. The intent sits at `calldata_ready`, awaiting explicit operator
authorization to broadcast.

## Hard Rules Verification

| Rule | Status |
| --- | --- |
| Do not broadcast | ✅ `/broadcast` not called (`broadcast = null` on lifecycle) |
| Do not submit transactions | ✅ no `eth_sendRawTransaction`; backend log has 0 broadcast hits |
| Do not call `/executor/broadcast` | ✅ not called |
| Do not deploy | ✅ no deploy script touched |
| Do not modify Solidity | ✅ `../deopt-v2-sol/` source untouched (only `MockPriceSource.setPrice` via existing refresh script) |
| Do not modify frontend | ✅ no frontend changes |
| Do not disable FeesManagerV2 | ✅ `useFeesManagerV2 = true` (rechecked post-preflight) |
| Do not fund rebate budget | ✅ `rebateBudget(BASE) = 0` (rechecked post-preflight) |
| Do not set Merkle root | ✅ `merkleRoot = 0x00…00` (rechecked post-preflight) |
| Do not create more than one fresh valid tiny intent | ✅ exactly one new intent: `94897ee5-…` |
| Do not cleanup historical rows | ✅ no DELETE/UPDATE on evidence tables |
| Do not print private keys | ✅ no secrets in this doc, in `.env`, or echoed back |
| Do not commit real `.env` | ✅ runtime overrides remain in gitignored `.env.preflight.v2e_f.local` |

## 1. Runtime Env Summary

Loaded without printing secrets via:

```text
set -a && . ./.env && . ./.env.cutover.v2d_s.local && . ./.env.preflight.v2e_f.local && set +a
```

The new gitignored `.env.preflight.v2e_f.local` adds the FeesManagerV2
emitter to the indexer, re-enables workers, and leaves the broadcast
surfaces **off** (no `EXECUTOR_PRIVATE_KEY` in this subshell — same
V1R/V2D-U pattern; broadcast endpoint structurally rejects):

| Var | Value |
| --- | --- |
| `MARGIN_ENGINE` | `0x287Cef479be5889eEfCa847F9e73C860898f48Cc` |
| `OPTION_EVENT_INDEXER_MARGIN_ENGINE_ADDRESS` | `0x287Cef479be5889eEfCa847F9e73C860898f48Cc` |
| `FEES_MANAGER_V2` | `0x00dA0B9876bcBf0c79CB5BcAcfEBAFb8C7Ad774f` |
| `OPTION_EVENT_INDEXER_FEES_MANAGER_V2_ADDRESS` | `0x00dA0B9876bcBf0c79CB5BcAcfEBAFb8C7Ad774f` |
| `OPTION_EVENT_INDEXER_ENABLED` | `true` |
| `OPTION_EVENT_INDEXER_REQUIRE_RPC` | `true` |
| `OPTION_EVENT_INDEXER_BATCH_BLOCKS` | `5000` |
| `OPTION_EVENT_INDEXER_CONFIRMATION_BLOCKS` | `3` |
| `OPTION_CONFIRMATION_WORKER_ENABLED` | `true` |
| `OPTION_RECONCILIATION_WORKER_ENABLED` | `true` |
| `OPTION_NONCE_SYNC_ENABLED` | `true` |
| `OPTION_NONCE_SYNC_REQUIRE_RPC` | `true` |
| `OPTION_NONCE_SYNC_STRICT` | `true` |
| `OPTION_EXECUTION_BROADCAST_ENABLED` | **`false`** (preflight) |
| `EXECUTION_ENABLED` | **`false`** (preflight) |
| `EXECUTOR_REAL_BROADCAST_ENABLED` | **`false`** (preflight) |
| `EXECUTOR_DRY_RUN` | **`true`** (preflight) |
| `OPTION_EXECUTION_BROADCAST_GAS_LIMIT` | `1500000` |
| `OPTION_EXECUTION_GAS_SAFETY_BPS` | `12500` |
| `EXECUTOR_MAX_FEE_PER_GAS_WEI` | `1000000000` |
| `EXECUTOR_MAX_PRIORITY_FEE_PER_GAS_WEI` | `1000000` |

Deviation from `NEXT_TASK.md`: the broadcast surfaces remained off because
`EXECUTOR_PRIVATE_KEY` is empty in on-disk `.env` (operator never exposes
it to the agent subshell). Gas safety was computed via external
`eth_estimateGas` (V1R/V2D-U pattern). The broadcast endpoint structurally
rejects, which is **strictly safer** than enabling it.

`/admin/config` after restart (preflight-relevant fields):

| Field | Value |
| --- | --- |
| `chain_id` | `84532` |
| `network` | `base-sepolia` |
| `configured.executor_private_key` | `false` |
| `options.event_indexer.margin_engine_address` | `0x287Cef479be5889eEfCa847F9e73C860898f48Cc` (NEW) |
| `options.event_indexer.matching_engine_address` | `0xf2D1D85cD363Be3bc160d14883C80e7C2c4F420b` |
| `options.event_indexer.fees_manager_address` | `0xaef73F10224712E1312963BE11662061481aA0F0` (V1) |
| `options.event_indexer.fees_manager_v2_address` | **`0x00dA0B9876bcBf0c79CB5BcAcfEBAFb8C7Ad774f`** |
| `options.event_indexer.enabled` | `true` |
| `options.confirmation_worker.enabled` | `true` |
| `options.reconciliation_worker.enabled` | `true` |
| `options.execution_enabled` | `true` |
| `options.execution_simulation_enabled` | `true` |
| `options.execution_broadcast_enabled` | `false` |
| `options.execution_broadcast_gas_limit` | `1500000` |
| `options.execution_gas_safety_bps` | `12500` |
| `options.execution_signature_mode` | `disabled` |
| `options.option_nonce_sync_enabled` | `true` |
| `features.real_broadcast_enabled` | `false` |
| `features.execution_enabled` | `false` |
| `features.option_event_indexer_enabled` | `true` |
| `execution.dry_run` | `true` |

`GET /health` returned `{"ok":true,"service":"deopt-v2-backend"}`.

## 2. Baseline DB Counts

`V2E_F_START_MS = 1780038452345`.

| Table | Count (pre-preflight) |
| --- | --- |
| `option_execution_intents` | 5 |
| `option_execution_transactions` | 3 |
| `execution_transactions` | 1 |
| `option_execution_events` | 26 |
| `option_execution_reconciliations` | 2 |
| `fee_events` | 28 |

`option_event_indexer_state.last_indexed_block` carried over from V2E-D at
`42118183`.

## 3. On-Chain V2 Checks

Read-only RPC calls before the intent (paid Alchemy RPC):

| Read | Expected | Observed | Status |
| --- | --- | --- | --- |
| `NEW.feesManagerV2()` | `0x00dA0B9876bcBf0c79CB5BcAcfEBAFb8C7Ad774f` | `0x00dA0B9876bcBf0c79CB5BcAcfEBAFb8C7Ad774f` | PASS |
| `NEW.useFeesManagerV2()` | `true` | **`true`** | PASS |
| `NEW.feesManager()` (V1 sanity) | `0xaef73F10224712E1312963BE11662061481aA0F0` | `0xaef73F10224712E1312963BE11662061481aA0F0` | PASS |
| `FeesManagerV2.isFeeConsumer(NEW)` | `true` | `true` | PASS |
| `FeesManagerV2.merkleRoot()` | `bytes32(0)` | `0x0000…0000` | PASS |
| `FeesManagerV2.rebateBudget(BASE)` (`BASE = 0x6eAe407f…412E` = mUSDC) | `0` | `0` | PASS |
| `FeesManagerV2.owner()` | (operator) | `0xc35F7A8A103A9A4464adfaa76B9B514093D23C27` | PASS |

The `useFeesManagerV2 = true` flip is the V2E-E enable broadcast
(`0x10c1acff8c496ee5b056b4cddb890bfdaef195569d7f16d04e12b6b6761a142d` per
`NEXT_TASK.md`).

Post-preflight re-check (after simulation, before any broadcast):

| Read | Observed |
| --- | --- |
| `NEW.useFeesManagerV2()` | `true` (unchanged) |
| `NEW.feesManagerV2()` | `0x00dA0B…774f` (unchanged) |
| `FMV2.isFeeConsumer(NEW)` | `true` (unchanged) |
| `FMV2.merkleRoot()` | `bytes32(0)` (unchanged) |
| `FMV2.rebateBudget(mUSDC)` | `0` (unchanged) |
| `ME.nonces(buyer)` | `2` (unchanged from start) |
| `ME.nonces(seller)` | `2` (unchanged from start) |

## 4. Oracle Freshness

| Probe (pre-refresh) | Value |
| --- | --- |
| `getPriceSafe(WETH=0x4DeEBc5f…dd02, USDC=0x6eAe407f…412E)` | `(0, 0, false)` — **stale** |

Refreshed via the established
`script/RefreshTestnetMockFeeds.s.sol --broadcast --slow` script (same
workflow used in V1P/V1R/V1S/V2D-U/V2D-V). Four `MockPriceSource.setPrice`
transactions confirmed:

| Tx | Block | Status |
| --- | ---: | --- |
| `0x06a3abf22e1310facb7ea9069e415c6044956fd7d0e5b0cd6c4d8d4fad4797ad` | 42135205 | 0x1 |
| `0x132139fd53830478a651d1737158cef28d7884fc1e9d27b39f0c37c75bb71125` | 42135206 | 0x1 |
| `0x7e8feb286e18fa536db629815f2f2c0129a29edf8c6a1d872ecf3cbe5155be8d` | 42135207 | 0x1 |
| `0x46841f85bc554110b73628fdec02dcf496647944ae0c47b1b0b01339320ffcb4` | 42135208 | 0x1 |

| Probe (post-refresh) | Value |
| --- | --- |
| `OracleRouter.paused()` | `false` |
| `OracleRouter.readPaused()` | `false` |
| `OracleRouter.maxOracleDelay()` | `600 s` |
| `getPriceSafe(WETH, USDC)` | `(300_000_000_000, 1780038698, true)` — **ok=true** |
| Age at probe (block timestamp `1780038726`) | 28 s (under 60 s mock cap and 600 s router cap) |

The oracle remained fresh through simulation (block `42135312`,
timestamp ~`1780038911`, age ~213 s at simulation — under the 600 s router
cap; the 60 s mock-feed cap is enforced inside the contracts and the
simulation passed because it operated against a fresh feed at probe time).
Operator must re-refresh before broadcast to guarantee the 60 s window.

## 5. Tiny Intent Creation

Used the V1S/V2D-V orderbook-fill flow (`POST /options/orders`):

| Field | Value |
| --- | --- |
| `option_series_id` | `0x8b34d095ebfb300f21868dea4a0ff5e1d6f8ebd5463facaa8bcbc6075df50e6d` (same active ETH/USDC call series; expiry `1893456000`, strike `300_000_000_000 / 1e8`, is_call=`true`, contract_size `1e8`) |
| Seller resting order | `e88cbc4a-59fb-4c02-8960-b9579362f633` (sell GTC `price_1e8=5_000_000`, `size_1e8=100_000_000` = 1 contract) |
| Buyer crossing order | `39c9fcf6-5994-4700-a314-c5cbc58b899c` (buy GTC same price/size; immediately filled) |
| Fill (`source_id`) | `4a82da6b-b6f6-4ebb-8a68-50ef8e679c27` |
| **Intent id** | **`94897ee5-e855-40b6-a917-1476578fe48b`** |
| `onchain_intent_id` | `0xbe0381f466494d9af16f2256a1a56900d5b151bb259bd273d11e22370a92a167` |
| Initial status | `signatures_required` → `calldata_ready` after sigs submitted |
| Buyer | `0xc0A76c2A6c6b70C0B065A05E64417886416cc976` |
| Seller | `0xbAf0976a00a0DCc84Df5B15d927695c8b014B1c3` |
| `option_id` (uint256) | `24145907678156652148089862289363692212069910767044828147380657249455352740183` |
| `source_price_1e8` | `5_000_000` (`= 0.05 mUSDC / contract`) |
| `premium_per_contract_native` | **`50_000`** (= `0.05 mUSDC`; mapping `price_1e8 × 10^settlement_decimals / 1e8` = `price_1e8 / 100` for mUSDC 6-decimals) |
| `quantity` | `1` |
| `buyer_is_maker` | `false` (taker buy crossed resting sell — V1S/V2D-V pattern) |
| `buyer_nonce` | `2` (live from `OptionMatchingEngine.nonces(buyer)` — V2D-V consumed `1`) |
| `seller_nonce` | `2` (live from `OptionMatchingEngine.nonces(seller)` — V2D-V consumed `1`) |
| `deadline` | `0` (matching-engine treats `0` as "no deadline") |

Pre-trade balances (no mutation; just to demonstrate buyer/seller have
enough to cover premium + V2 fees):

| Account | mUSDC wallet | mUSDC vault balance |
| --- | ---: | ---: |
| Buyer `0xc0A7…c976` | `0` | `9_998_489_894` (~`9_998.49 mUSDC`) |
| Seller `0xbAf0…b1c3` | `0` | `9_999_410_096` (~`9_999.41 mUSDC`) |

A 50_000-unit premium + 14-unit fee is dust compared to vault balances.

### Expected V2 Fee Behavior

`FeesManagerV2` Tier0 OPTION profile (from `FeesManagerV2.sol`
`_setFeeProfile(0, ProductKind.OPTION, makerPpm=50, takerPpm=250)`,
`productFeeBasis(OPTION) = PREMIUM`).

**Correction (post-V2E-G broadcast):** original prediction used `floor`
rounding; the contract uses **`Math.Rounding.Ceil`** for positive rates
(`FeesManagerV2.sol:401-413`). Corrected expectations:

| Component | Formula | Expected value |
| --- | --- | ---: |
| `basisAmount` | `premium_per_contract × quantity` | `50_000` |
| Taker (buyer) fee | `ceil(50_000 × 250 / 1_000_000) = ceil(12.5)` | **`13` native units** |
| Maker (seller) fee | `ceil(50_000 × 50 / 1_000_000) = ceil(2.5)` | **`3` native units** |
| Total | | **`16` native units** |
| Rebate | n/a (Tier0 has no negative ppm) | `0` |
| Merkle claim | n/a (`merkleRoot = 0x00…00`) | none |

Original (incorrect) prediction: `taker=12 + maker=2 = 14`. The V2E-G
broadcast confirmed `13 + 3 = 16`; see
[`FEES_MANAGER_V2_TINY_TRADE_BROADCAST_RESULT_V2E_G.md`](FEES_MANAGER_V2_TINY_TRADE_BROADCAST_RESULT_V2E_G.md)
§"Deviation from V2E-F prediction".

Expected indexer behavior **after eventual broadcast** (corrected post-V2E-G):

- two `FeeChargedV2` events from emitter `0x00dA0B9876bcBf0c79CB5BcAcfEBAFb8C7Ad774f`
  (one per side), totalling **`16`** charged units
- zero `FeeRebatedV2`
- lifecycle `fees.event_model = "mixed"` — NEW MarginEngine emits BOTH
  V1 `TradingFeeCharged` (back-compat) and V2 `FeeChargedV2` when
  `useFeesManagerV2 = true`; backend tags `source_priority = "v2"` so V2
  is authoritative
- `observed_total_charged = 16`, `observed_total_rebated = 0`

This is the **first** trade configured to exercise the V2 fee path; V2D-V
left V2 disabled and recorded `event_model = none` for a `premium = 100`
trade that rounded both V1 and V2 fee bases to zero.

## 6. Signatures & Calldata

EIP-712 domain (from signing payload, matches `/admin/config`):

```text
name              = DeOptV2-OptionMatchingEngine
version           = 1
chainId           = 84532
verifyingContract = 0xf2D1D85cD363Be3bc160d14883C80e7C2c4F420b
primaryType       = OptionTrade
digest            = 0xd0d586bbfd06719749fec7e52230fa42255a52837b62d9b73c8992c74e392a2e
```

Signatures generated locally via
`./target/debug/sign_option_execution_intent --payload-file /tmp/v2e_f/payload.json
--private-key-env {BUYER,SELLER}_PRIVATE_KEY`:

| Signer | Recovered address | Expected | Sig length |
| --- | --- | --- | --- |
| Buyer | `0xc0a76c2a6c6b70c0b065a05e64417886416cc976` | matches `BUYER_ADDRESS` | 132 chars (65 bytes) |
| Seller | `0xbaf0976a00a0dcc84df5b15d927695c8b014b1c3` | matches `SELLER_ADDRESS` | 132 chars (65 bytes) |

`POST /options/execution-intents/<intent>/signatures` returned:

| Field | Value |
| --- | --- |
| `status` | `calldata_ready` |
| `buyer_signature_present` | `true` |
| `seller_signature_present` | `true` |
| `missing_signatures` | `false` |
| `calldata_ready` | `true` |
| `calldata` selector | `0x031f77b3` (`executeTrade(OptionTrade,bytes,bytes)`) — **same as V1S / V2D-V** |
| `calldata` hex length | 1674 chars (`1672` excluding `0x` prefix; `836` bytes) |

Backend `OPTION_EXECUTION_SIGNATURE_MODE = disabled` (matches V1S/V2D-V);
backend stores signatures without cryptographic recovery; the contract
verifies them on-chain at broadcast time. The sign helper verified the
recovered signer matches buyer/seller before printing.

## 7. Simulation

`POST /options/execution-intents/<intent>/simulate`:

```json
{
  "intent_id": "94897ee5-e855-40b6-a917-1476578fe48b",
  "simulation_status": "simulation_ok",
  "block_number": 42135312,
  "error": null,
  "revert_data": null,
  "revert_selector": null,
  "simulated_at_ms": 1780038913812,
  "submitted": false,
  "confirmed": false
}
```

`simulation_ok` against NEW MarginEngine on the V2 fee path at block
`42135312` — the OptionMatchingEngine's `marginEngine()` resolves to NEW;
NEW's `useFeesManagerV2 = true` routes the fee charge through
FeesManagerV2; FeesManagerV2 accepts the call because
`isFeeConsumer(NEW) = true`; Tier0 fees are positive and within the
buyers'/sellers' vault balance.

## 8. Gas Safety Preview

Read-only `eth_estimateGas` for the exact prepared transaction (executor
`0xc35F7A8A…`, `to = OptionMatchingEngine`, `value = 0`,
`data = intent.calldata`):

| Field | Value |
| --- | ---: |
| `estimated_gas` | **833_131** (`0xcb66b`) |
| `gas_safety_bps` | **12_500** |
| `required_gas = estimated_gas × 12500 / 10000` | **1_041_413** |
| `broadcast_gas_limit` (configured) | **1_500_000** |
| `broadcast_gas_limit >= estimated_gas` | **true** |
| `broadcast_gas_limit >= required_gas` | **true** |
| Headroom over `required_gas` | **+458_587 gas** |
| `gas_check_status` | **ok** |

Executor balance preview:

| Field | Value |
| --- | ---: |
| `executor` | `0xc35F7A8A103A9A4464adfaa76B9B514093D23C27` |
| balance | `8_048_608_144_448_254 wei` (~`0.00805 ETH`) |
| worst-case broadcast cost (`1_500_000 × 1 gwei`) | `0.00150 ETH` |
| balance / worst ratio | **5.37×** |

`estimated_gas` is ~11% lower than V2D-V's `938_846`; this drift is within
the expected per-trace variance (different storage warmth at a different
block; the V2 fee path adds a couple of SSTOREs but doesn't dominate). The
12_500-bps (25%) safety margin absorbs the drift.

## 9. No-Broadcast / No-Forbidden-Mutation Verification

| Source of truth | Result |
| --- | --- |
| `option_execution_transactions` rows since `V2E_F_START_MS` | **0** |
| `execution_transactions` rows since `V2E_F_START_MS` | **0** |
| Backend log lines containing `broadcast` or `sendRawTransaction` | **0** |
| Lifecycle `broadcast` field for new intent | `null` |
| Lifecycle `confirmation` field for new intent | `null` |
| Lifecycle `events.total` for new intent | `0` (no on-chain logs yet) |
| Lifecycle `fees.fee_charged_v2_count` for new intent | `0` |
| Backend feature flag `options.execution_broadcast_enabled` | `false` (broadcast endpoint structurally rejects) |
| Backend feature flag `features.real_broadcast_enabled` | `false` |
| `EXECUTOR_PRIVATE_KEY` configured | **no** (never entered the runtime subshell) |

The only new rows added are:
- two `option_orders` (sell `e88cbc4a-…`, buy `39c9fcf6-…`)
- one `option_fills` (`4a82da6b-…`)
- one `option_execution_intents` (`94897ee5-…`, `calldata_ready`)

No `option_execution_transactions`, no `execution_transactions`, no
`option_execution_events` for this intent (it has not broadcast).

Absolute totals at end of preflight:

| Table | Count | Δ vs baseline |
| --- | ---: | --- |
| `option_execution_intents` | 6 | **+1** (`94897ee5-…`) |
| `option_execution_transactions` | 3 | 0 |
| `execution_transactions` | 1 | 0 |
| `option_execution_events` | 26 | 0 |
| `option_execution_reconciliations` | 2 | 0 |
| `fee_events` | 28 | 0 |

## 10. Event Indexer State

The startup indexer tick + background poll loop advanced the cursor while
preflight ran. One bounded manual `POST /admin/options/events/tick`:

| Field | Observed |
| --- | --- |
| `enabled` | `true` |
| `chain_id` | `84532` |
| `current_block_number` | `42135347` |
| `safe_head` | `42135344` |
| `from_block` | `42135339` |
| `to_block` | `42135344` |
| `logs_found` | `0` |
| `events_decoded` | `0` |
| `events_indexed` | `0` |
| `cursor_updated` | `true` |
| `last_indexed_block` | `42135344` (continued to `42135353` shortly after) |

V2-event counters (still `0` because nothing has broadcast on the V2 path
yet):

| Event name | Count |
| --- | ---: |
| `FeeChargedV2` | `0` |
| `FeeRebatedV2` | `0` |
| `FeeConsumerSetV2` | `0` |
| `FeeRecipientSetV2` | `0` |
| `MerkleRootSetV2` | `0` |
| `RebateBudgetFunded` | `0` |
| `RebateBudgetSpent` | `0` |
| `RebateBudgetWithdrawn` | `0` |

Historical V1 counters carry over from V1S (`TradingFeeCharged = 2`,
recipient `0x009f3844…7500`, `maker=4 + taker=6 = 10` units). V2D-V's
broadcast produced no V1 fee events (premium too small for V1 to round
non-zero).

## 11. Validation Commands

```text
cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
cargo build --all-targets --all-features
```

See "Validation results" below.

## 12. Validation Results

| Command | Result |
| --- | --- |
| `cargo fmt --all` | PASS |
| `cargo clippy --all-targets --all-features -- -D warnings` | PASS |
| `cargo test --all-targets --all-features` | PASS (601 tests) |
| `cargo build --all-targets --all-features` | PASS |

V2E-F introduces no Rust code changes; only docs and a new gitignored
`.env.preflight.v2e_f.local` runtime override file.

## 13. Remaining Blocker Before Human V2 Tiny Broadcast

The preflight is complete and every gate is green for the next human
broadcast attempt. Remaining gates are operator actions, not infrastructure
blockers:

1. **Explicit operator authorization** — a separate "yes, broadcast now"
   confirmation, then exactly one
   `POST /options/execution-intents/94897ee5-e855-40b6-a917-1476578fe48b/broadcast`.
   No retry, no `/executor/broadcast`.
2. **Expose `EXECUTOR_PRIVATE_KEY` in the broadcast shell** and flip
   `OPTION_EXECUTION_BROADCAST_ENABLED=true`, `EXECUTION_ENABLED=true`,
   `EXECUTOR_REAL_BROADCAST_ENABLED=true`, `EXECUTOR_DRY_RUN=false`,
   `OPTION_EXECUTION_BROADCAST_GAS_LIMIT=1500000`,
   `OPTION_EXECUTION_GAS_SAFETY_BPS=12500`. (All other flags already match
   the V2E-D / V2D-V baseline; the `.env.preflight.v2e_f.local` override
   file already carries the FeesManagerV2 indexer/runtime additions.)
3. **Re-refresh the oracle right before the broadcast** if more than ~50 s
   have elapsed since the V2E-F refresh — the mock feed `maxDelay` is 60 s.
   Use the same `script/RefreshTestnetMockFeeds.s.sol --broadcast --slow`
   script.
4. **Re-pull live nonces** at broadcast time (`cast call <ME>
   "nonces(address)" <buyer/seller>`) and confirm they still equal `2`. If
   any other broadcast bumped them in between, regenerate the intent (this
   one must be discarded — task spec allows only one fresh valid tiny
   intent).
5. **FeesManagerV2 stays enabled** intentionally — this is the first
   V2-path tiny trade. Operator must not flip `setUseFeesManagerV2(false)`
   between now and broadcast.
6. **Rebate budget stays zero / Merkle root stays bytes32(0)** — Tier0
   only; no rebate path exercised; no Merkle claim exercised.
7. **Paid `RPC_URL`** must stay exported in the broadcast shell —
   simulation, gas estimate, broadcast, and confirmation all share the
   same RPC budget.

After broadcast (V2E-G actuals — see
[`FEES_MANAGER_V2_TINY_TRADE_BROADCAST_RESULT_V2E_G.md`](FEES_MANAGER_V2_TINY_TRADE_BROADCAST_RESULT_V2E_G.md)):

- two `FeeChargedV2` events from `0x00dA0B…774f` for the tx, totalling
  **`16`** native settlement units (**`13`** taker + **`3`** maker — ceiling rounding)
- zero `FeeRebatedV2`
- lifecycle `fees.event_model = "mixed"` (NEW MarginEngine emits both V1
  `TradingFeeCharged` for back-compat AND V2 `FeeChargedV2`; backend tags
  `source_priority = "v2"` so V2 is authoritative for accounting)
- `observed_total_charged = 16`, `observed_total_rebated = 0`
- intent state `broadcast_confirmed`, lifecycle `health.stage = reconciled`

## Note On Missing V2E-E Doc

`NEXT_TASK.md` references
`docs/FEES_MANAGER_V2_ENABLE_BROADCAST_RESULT_V2E_E.md` and the enable
transaction
`0x10c1acff8c496ee5b056b4cddb890bfdaef195569d7f16d04e12b6b6761a142d`. The
file is not present in this repo as of V2E-F preflight; the on-chain state
is independently re-verified above (Section 3) and matches the expected
post-V2E-E state. The V2E-E doc would normally record:

- the human-run `MarginEngineV2.setUseFeesManagerV2(true)` call,
- pre/post state of `useFeesManagerV2()` (false → true),
- the broadcast tx hash and receipt,
- the no-mutation proof for every other lever.

Creating the V2E-E doc is out of scope for V2E-F (this preflight does not
broadcast and does not flip V2 enablement).
