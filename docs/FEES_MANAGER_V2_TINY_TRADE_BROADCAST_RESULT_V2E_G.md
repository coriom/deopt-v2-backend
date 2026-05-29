# FeesManagerV2 Tiny Trade Broadcast Result V2E-G

Date: 2026-05-29
Network: Base Sepolia (`chain_id 84532`)
Mode: Live broadcast (operator-triggered). Agent did not call `/broadcast`.

## Result

**First successful tiny option execution against NEW MarginEngine V2 with
FeesManagerV2 enabled.**

- Intent: `94897ee5-e855-40b6-a917-1476578fe48b`
- Transaction hash: **`0xd51ea881cdbc32fe724034c0f7e25ade7359ea3d5b6cadb17b7c345effefc72c`**
- Block: `42136440`, `transactionIndex = 18`
- Receipt `status = 1 (success)`
- `gasUsed = 803_814` (under `estimated_gas = 884_184`, well below `broadcast_gas_limit = 1_500_000`)
- `effectiveGasPrice = 6_000_000 wei` (`0x5b8d80`)
- `cumulativeGasUsed = 3_645_391`
- From: `0xc35F7A8A103A9A4464adfaa76B9B514093D23C27` (executor)
- To: `0xf2D1D85cD363Be3bc160d14883C80e7C2c4F420b` (OptionMatchingEngine — unchanged)
- Backend `option_execution_transactions.transaction_id`: `06b6d1bd-ab29-4d6e-ac6a-b3cfb9be1cc1`
- **First two `FeeChargedV2` events emitted by FeesManagerV2 `0x00dA0B…774f` on Base Sepolia.**

Exactly one transaction submitted. No retry. No `/executor/broadcast` call.
No `execution_transactions` row created.

## Hard Rules Verification

| Rule | Status |
| --- | --- |
| Agent did not broadcast | ✅ `/broadcast` called by human operator exactly once |
| Do not submit transactions (agent) | ✅ no `eth_sendRawTransaction` by agent |
| Do not call `/executor/broadcast` | ✅ not called |
| Do not deploy | ✅ no deploy script changes |
| Do not modify Solidity | ✅ `../deopt-v2-sol/` source untouched (only `MockPriceSource.setPrice` via existing refresh script) |
| Do not modify frontend | ✅ no frontend changes |
| Do not disable FeesManagerV2 | ✅ `useFeesManagerV2 = true` (rechecked post-broadcast) |
| Do not fund rebate budget | ✅ `rebateBudget(mUSDC) = 0` (rechecked) |
| Do not set Merkle root | ✅ `merkleRoot = 0x00…00` (rechecked) |
| Do not create replacement intent | ✅ broadcast used the original V2E-F intent `94897ee5-…`; no replacement |
| Do not cleanup historical rows | ✅ V1S/V2D-V rows preserved |
| Do not print private keys | ✅ `EXECUTOR_PRIVATE_KEY` exported in operator shell only; never echoed |
| Do not commit real `.env` | ✅ overrides remain in gitignored `.env.cutover.v2d_s.local`, `.env.preflight.v2e_f.local`, `.env.broadcast.v2e_g.local` |

## Phase 1 — Pre-Broadcast Refresh

The operator launched the backend in their interactive shell with all
V2E-G env exported (full stack: `.env` + `.env.cutover.v2d_s.local` +
`.env.preflight.v2e_f.local` + `.env.broadcast.v2e_g.local`) plus
`EXECUTOR_PRIVATE_KEY` exported in shell. `.env.broadcast.v2e_g.local`
flips broadcast surfaces ON:

```text
OPTION_EXECUTION_BROADCAST_ENABLED=true
EXECUTION_ENABLED=true
EXECUTOR_REAL_BROADCAST_ENABLED=true
EXECUTOR_DRY_RUN=false
OPTION_EXECUTION_BROADCAST_GAS_LIMIT=1500000
OPTION_EXECUTION_GAS_SAFETY_BPS=12500
EXECUTOR_MAX_FEE_PER_GAS_WEI=1000000000
EXECUTOR_MAX_PRIORITY_FEE_PER_GAS_WEI=1000000
```

`/admin/config` gates (all green):

| Check | Value |
| --- | --- |
| `chain_id` / `network` | `84532` / `base-sepolia` |
| `configured.executor_private_key` | `true` |
| `features.execution_enabled` | `true` |
| `features.real_broadcast_enabled` | `true` |
| `features.option_event_indexer_enabled` | `true` |
| `features.option_execution_broadcast_enabled` | `true` |
| `execution.dry_run` | `false` |
| `execution.max_fee_per_gas_configured` | `true` |
| `execution.max_priority_fee_per_gas_configured` | `true` |
| `options.execution_broadcast_gas_limit` | `1_500_000` |
| `options.execution_gas_safety_bps` | `12_500` |
| `options.event_indexer.margin_engine_address` | `0x287Cef…f48cc` (NEW) |
| `options.event_indexer.fees_manager_v2_address` | `0x00dA0B…774f` |
| `options.event_indexer.fees_manager_address` | `0xaef73F…aA0F0` (V1) |
| `options.event_indexer.matching_engine_address` | `0xf2D1D85c…420b` |
| `options.confirmation_worker.enabled` | `true` |
| `options.reconciliation_worker.enabled` | `true` |

## Phase 2 — Final Safety Checks

### On-chain V2 state (re-checked)

| Read | Expected | Observed |
| --- | --- | --- |
| `NEW.useFeesManagerV2()` | `true` | `true` |
| `NEW.feesManagerV2()` | `0x00dA0B…774f` | `0x00dA0B…774f` |
| `FMV2.isFeeConsumer(NEW)` | `true` | `true` |
| `FMV2.merkleRoot()` | `bytes32(0)` | `0x00…00` |
| `FMV2.rebateBudget(mUSDC)` | `0` | `0` |

### Oracle freshness

Pre-refresh probe: `getPriceSafe(WETH, USDC) = (0, 0, false)` — stale (~24
minutes since V2E-F refresh aged past the 60s mock cap). Refreshed via
`script/RefreshTestnetMockFeeds.s.sol --broadcast --slow`. Four
`MockPriceSource.setPrice` transactions confirmed:

| Tx | Block | Status |
| --- | ---: | --- |
| `0xd77c1ca487708b0c342ec896f930bc892f3f9306c2ea3bb5bcc67cd195ea9115` | 42136363 | 0x1 |
| `0x4718aaadfa4a1288c7209162d31999a946e2312f20f56be513d5652d30fe7175` | 42136364 | 0x1 |
| `0xeaec37e335b3376e82a79d7c9b36b514cc90a9705014cfd436fe3d87751feb6e` | 42136365 | 0x1 |
| `0xb455d48d1beb6aba08cf70650736be671f0f582b947038ed61281f540056670a` | 42136366 | 0x1 |

Post-refresh: `getPriceSafe(WETH, USDC) = (300_000_000_000, 1780041014,
true)` — **ok**, age 38 s at probe (well under 60 s mock cap).

### Nonces

| Account | On-chain `nonces(addr)` pre-broadcast | Intent value | Match |
| --- | ---: | ---: | --- |
| Buyer `0xc0A76c2A…cc976` | `2` | `2` | ✅ |
| Seller `0xbAf0976a…b1c3` | `2` | `2` | ✅ |

Post-broadcast: both nonces incremented to `3` (matching engine bumped
each side's nonce on apply-trade).

### Re-simulation

`POST /options/execution-intents/<intent>/simulate`:

```json
{
  "intent_id": "94897ee5-e855-40b6-a917-1476578fe48b",
  "simulation_status": "simulation_ok",
  "block_number": 42136392,
  "error": null,
  "revert_data": null,
  "revert_selector": null,
  "simulated_at_ms": 1780041073928,
  "submitted": false,
  "confirmed": false
}
```

### Gas safety

External `eth_estimateGas` for the prepared transaction at re-simulation
time:

| Field | Value |
| --- | ---: |
| `estimated_gas` | 884_184 |
| `gas_safety_bps` | 12_500 |
| `required_gas` | 1_105_230 |
| `broadcast_gas_limit` | 1_500_000 |
| Headroom over `required_gas` | +394_770 |
| `gas_check_status` | **ok** |
| Executor balance | `8_047_712_505_802_828 wei` (~`0.00805 ETH`) |
| Worst-case broadcast cost | `0.00150 ETH` |
| balance/worst ratio | **5.37×** |

### Baseline

`V2E_G_START_MS = 1780039519899`. Pre-broadcast counts:

| Table | Count |
| --- | --- |
| `option_execution_intents` | 6 |
| `option_execution_transactions` | 3 |
| `execution_transactions` | 1 |
| `option_execution_events` | 26 |
| `option_execution_reconciliations` | 2 |
| `fee_events` | 28 |

## Phase 3 — Human Broadcast

Operator ran exactly one:

```text
POST http://127.0.0.1:8080/options/execution-intents/94897ee5-e855-40b6-a917-1476578fe48b/broadcast
```

Response:

```json
{
  "intent_id": "94897ee5-e855-40b6-a917-1476578fe48b",
  "status": "broadcast_submitted",
  "tx_hash": "0xd51ea881cdbc32fe724034c0f7e25ade7359ea3d5b6cadb17b7c345effefc72c",
  "to": "0xf2D1D85cD363Be3bc160d14883C80e7C2c4F420b",
  "from": "0xc35f7a8a103a9a4464adfaa76b9b514093d23c27",
  "transaction_id": "06b6d1bd-ab29-4d6e-ac6a-b3cfb9be1cc1",
  "broadcast_enabled": true,
  "submitted": true,
  "duplicate": false,
  "confirmed": false,
  "estimated_gas": 833131,
  "required_gas": 1041413,
  "simulation_gas_limit": 0,
  "broadcast_gas_limit": 1500000,
  "gas_safety_bps": 12500,
  "gas_check_status": "ok",
  "gas_check_error": null
}
```

Note: backend's internal `estimated_gas = 833_131` differs slightly from
the external re-simulation `884_184` (different block / storage warmth);
both fit comfortably under the cap. No retry. No `/executor/broadcast`.
No second `/broadcast`.

## Phase 4 — Post-Broadcast Verification

### `cast receipt`

| Field | Value |
| --- | --- |
| `transactionHash` | `0xd51ea881cdbc32fe724034c0f7e25ade7359ea3d5b6cadb17b7c345effefc72c` |
| `blockNumber` | `42136440` |
| `blockHash` | `0x4e1b2de839f4e2990245bece037caac8f2e76181ac14559ac3ed95a95f29e988` |
| `from` | `0xc35F7A8A103A9A4464adfaa76B9B514093D23C27` |
| `to` | `0xf2D1D85cD363Be3bc160d14883C80e7C2c4F420b` |
| `status` | **1 (success)** |
| `gasUsed` | `803_814` |
| `effectiveGasPrice` | `6_000_000 wei` |
| `cumulativeGasUsed` | `3_645_391` |
| `transactionIndex` | 18 |
| `type` | 2 (EIP-1559) |

### Indexer state and per-tx events

Background poll loop captured all 21 logs for this tx. One bounded manual
`POST /admin/options/events/tick` after auto-tick advanced cursor 4 more
blocks (`42136481 → 42136485`) with 0 additional logs. Final
`option_event_indexer_state.last_indexed_block = 42136495`.

Per-tx counts (from `option_execution_events.tx_hash = <TX>`):

| Event name | Count | Emitters |
| --- | ---: | --- |
| `Synced` | 12 | CollateralVault |
| `InternalTransfer` | 3 | CollateralVault |
| **`FeeChargedV2`** | **2** | **FeesManagerV2 `0x00dA0B…774f`** |
| `TradingFeeCharged` | 2 | NEW MarginEngine `0x287cef…f48cc` (V1 compat path) |
| `TradeExecuted` | 1 | NEW MarginEngine `0x287cef…f48cc` |
| `OptionTradeExecuted` | 1 | OptionMatchingEngine `0xf2d1d85c…420b` |
| **Total** | **21** | |

Counts by contract address:

```text
0x00340c360353a5ab784c5bc5c44322a6af0625d3 (CollateralVault):       15
0x287cef479be5889eefca847f9e73c860898f48cc (NEW MarginEngine):        3
0x00da0b9876bcbf0c79cb5bcacfebafb8c7ad774f (FeesManagerV2):           2
0xf2d1d85cd363be3bc160d14883c80e7c2c4f420b (OptionMatchingEngine):    1
```

### V2 fee accounting (from decoded `FeeChargedV2` payloads)

| log_index | side | trader | recipient | basisAmount | feePpm | feeAmount |
| ---: | --- | --- | --- | ---: | ---: | ---: |
| 111 | taker | `0xc0a7…c976` (buyer) | `0xa67f…b588` | `50_000` | `250` | **`13`** |
| 118 | maker | `0xbaf0…b1c3` (seller) | `0xa67f…b588` | `50_000` | `50` | **`3`** |

Both events carry `productKind = "option"`, `flowKind = "orderbook"`,
`consumer = 0x287cef…f48cc` (NEW MarginEngine), `settlementAsset =
0x6eAe407f…412E` (mUSDC). `FeeRebatedV2` count = `0` (Tier0 has no
negative ppm; no rebate path exercised).

### Deviation from V2E-F prediction

The V2E-F preflight doc predicted `taker = 12 + maker = 2 = 14`. Actual:
**`taker = 13 + maker = 3 = 16`**, +2 over prediction. Root cause: V2E-F
used `floor` rounding; the contract uses **`Math.Rounding.Ceil`** for
positive rates (see `FeesManagerV2.sol:401-413`,
`Math.mulDiv(basisAmount, positiveRate, PPM_DENOMINATOR,
Math.Rounding.Ceil)`):

```solidity
function _amountFromRate(uint256 basisAmount, int32 ratePpm)
    internal pure returns (uint256)
{
    if (basisAmount == 0 || ratePpm == 0) return 0;
    if (ratePpm > 0) {
        uint256 positiveRate = _positiveRateToUint256(ratePpm);
        return Math.mulDiv(basisAmount, positiveRate, PPM_DENOMINATOR,
                           Math.Rounding.Ceil);
    }
    uint256 rebateRate = _rebateRateToUint256(ratePpm);
    return Math.mulDiv(basisAmount, rebateRate, PPM_DENOMINATOR,
                       Math.Rounding.Floor);
}
```

Hence `ceil(50_000 × 250 / 1_000_000) = ceil(12.5) = 13` and
`ceil(50_000 × 50 / 1_000_000) = ceil(2.5) = 3`. The V2E-F doc has been
amended to reflect this. (Rebates round `Floor` — fee-favorable to the
protocol both directions.)

### Backend confirmation

The background `OPTION_CONFIRMATION_WORKER` picked up the receipt
immediately. `option_execution_transactions` row state after confirmation:

| Field | Value |
| --- | --- |
| `transaction_id` | `06b6d1bd-ab29-4d6e-ac6a-b3cfb9be1cc1` |
| `intent_id` | `94897ee5-e855-40b6-a917-1476578fe48b` |
| `tx_hash` | `0xd51ea881cdbc32fe724034c0f7e25ade7359ea3d5b6cadb17b7c345effefc72c` |
| `status` | `submitted` |
| `confirmation_status` | **`mined_success`** |
| `confirmed_block_number` | `42136440` |
| `receipt_status` | `1` |
| `gas_used` | `803_814` |

### Reconciliation

`option_execution_reconciliations` row for the intent:

| Field | Value |
| --- | --- |
| `id` | `0115a4b2-910d-4274-bd45-681e228be842` |
| `intent_id` | `94897ee5-…` |
| `status` | **`reconciled`** |
| `strict` | `true` |
| `trade_executed_event_id` | (present) |
| `margin_trade_event_id` | (present) |
| `trading_fee_event_count` | `2` (V1 compat `TradingFeeCharged` on NEW) |
| `internal_transfer_event_count` | `3` |
| `decoded_event_count` | `21` |
| `mismatch_reason` | `null` |
| `missing_required` | `null` |

### Lifecycle (`/admin/options/executions/<intent>/lifecycle`)

| Field | Value |
| --- | --- |
| `status` | **`broadcast_confirmed`** |
| `health.stage` | **`reconciled`** |
| `health.is_terminal_success` | `true` |
| `broadcast.tx_hash` | `0xd51ea881…fc72c` |
| `broadcast.status` | `submitted` |
| `confirmation.confirmation_status` | `mined_success` |
| `confirmation.confirmed_block_number` | `42136440` |
| `confirmation.gas_used` | `803_814` |
| `reconciliation.status` | `reconciled` |
| `events.total` | `21` |
| `fees.source_of_truth` | `onchain` |
| `fees.event_model` | **`mixed`** (both V1 `TradingFeeCharged` and V2 `FeeChargedV2`) |
| `fees.fee_charged_v2_count` | `2` |
| `fees.fee_rebated_v2_count` | `0` |
| `fees.trading_fee_event_count` | `2` |
| `fees.observed_total_charged` | `16` |
| `fees.observed_total_rebated` | `0` |
| `fees.net_protocol_fee` | `16` |
| `fees.by_trader` | `{ buyer: "13", seller: "3" }` |
| `fees.by_side` | `{ taker: "13", maker: "3" }` |
| `fees.by_recipient` | `{ "0xa67f…b588": "16" }` |

Note: `event_model = "mixed"` (not `"v2"` as anticipated). NEW MarginEngine
emits **both** V1 `TradingFeeCharged` (back-compat) and V2 `FeeChargedV2`
when `useFeesManagerV2 = true`. Both event streams carry the same per-side
amounts (`13` taker + `3` maker = `16`); the backend tags this `mixed` and
sets `source_priority = "v2"` so the V2 stream is treated as authoritative
for fee accounting (which is why `observed_total_charged = 16`, not `32`).

### `/admin/fees/onchain?tx_hash=<TX>`

```json
{
  "event_model": "mixed",
  "source_of_truth": "onchain",
  "source_priority": "v2",
  "fee_charged_v2_count": 2,
  "fee_rebated_v2_count": 0,
  "trading_fee_event_count": 2,
  "observed_total_charged": "16",
  "observed_total_rebated": "0",
  "net_protocol_fee": "16",
  "by_trader": { "0xc0a7…c976": "13", "0xbaf0…b1c3": "3" },
  "by_side":    { "taker": "13", "maker": "3" },
  "by_recipient": { "0xa67f…b588": "16" },
  "transactions": [ { "tx_hash": "0xd51ea881…fc72c",
                      "event_model": "mixed",
                      "source_priority": "v2",
                      "fee_charged_v2_count": 2,
                      "observed_total_charged": "16", ... } ]
}
```

Endpoint returns 4 fee events for the tx: 2 V1 `TradingFeeCharged`
(emitted by NEW MarginEngine for back-compat) and 2 V2 `FeeChargedV2`
(emitted by FeesManagerV2). Both streams agree on per-side amounts.

## FeesManagerV2 State Re-verified (post-broadcast)

```text
NEW.useFeesManagerV2()    → true
NEW.feesManagerV2()       → 0x00dA0B9876bcBf0c79CB5BcAcfEBAFb8C7Ad774f
FMV2.isFeeConsumer(NEW)   → true
FMV2.merkleRoot()         → bytes32(0)
FMV2.rebateBudget(mUSDC)  → 0
ME.nonces(buyer)          → 3 (was 2)
ME.nonces(seller)         → 3 (was 2)
```

## No-Forbidden-Mutation Verification

`V2E_G_START_MS = 1780039519899`. Row deltas since then:

| Table | Δ | Notes |
| --- | ---: | --- |
| `option_execution_intents` | 0 | reused V2E-F intent `94897ee5-…` |
| `option_execution_transactions` | **+1** | `06b6d1bd-…` (the broadcast/confirmed tx) |
| `execution_transactions` (generic) | **0** | ✅ forbidden table untouched |
| `option_execution_events` (for this tx) | **+21** | from indexer indexing this tx |
| `option_execution_reconciliations` | **+1** | reconciliation row `0115a4b2-…` |
| `fee_events` | 0 | backend fee ledger disabled |

Absolute totals after V2E-G:

| Table | Count |
| --- | --- |
| `option_execution_intents` | 6 |
| `option_execution_transactions` | 4 (V1L failed `204a3070-…`, V1S success `cae8c7e7-…`, V2D-V success `6a3209ad-…`, V2E-G success `06b6d1bd-…`) |
| `execution_transactions` | 1 (unchanged) |
| `option_execution_events` | 47 (26 + 21) |
| `option_execution_reconciliations` | 3 (V1S + V2D-V + V2E-G) |
| `fee_events` | 28 (unchanged) |

## Cross-checks vs V2D-V (V2 disabled) and V1S (V1 only)

| Field | V1S | V2D-V | V2E-G |
| --- | --- | --- | --- |
| Margin engine | OLD `0x6c5665…b5f8` | NEW `0x287cef…f48cc` | NEW `0x287cef…f48cc` |
| `useFeesManagerV2` | n/a | `false` | **`true`** |
| `feesManagerV2` | n/a | `0x0` | `0x00dA0B…774f` |
| Intent | `e6d2941b-…` | `a6369dd5-…` | `94897ee5-…` |
| Tx hash | `0x5964a7b3…1125` | `0x07a8e679…7de10` | `0xd51ea881…fc72c` |
| Block | `41856964` | `42110498` | `42136440` |
| `gasUsed` | 1_057_772 | 913_477 | **803_814** |
| `estimated_gas` | 1_091_120 | 938_846 | 833_131 (backend at broadcast) / 884_184 (external re-sim) |
| Premium native | 10_000 | 100 | **50_000** |
| Buyer nonce | 0 → 1 | 1 → 2 | **2 → 3** |
| Seller nonce | 0 → 1 | 1 → 2 | **2 → 3** |
| Reconciliation | `reconciled` | `reconciled` | **`reconciled`** |
| Fee event model | `v1` | `none` (premium too small) | **`mixed`** |
| Total fee charged | 10 | 0 | **16** (13 taker + 3 maker) |
| Fee recipient | `0x009f38…7500` | n/a | **`0xa67f8e…b588`** (V2 recipient) |
| `FeeChargedV2` count | 0 | 0 | **2** |

## Validation Commands

```text
cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
cargo build --all-targets --all-features
```

See "Validation results" below.

## Validation Results

| Command | Result |
| --- | --- |
| `cargo fmt --all` | clean (no code changes; V2E-G is docs-only) |
| `cargo clippy --all-targets --all-features -- -D warnings` | clean |
| `cargo test --all-targets --all-features --no-fail-fast` | **passed: 601, failed: 0, ignored: 0** |
| `cargo build --all-targets --all-features` | clean |

## Remaining Blocker Before Perps/Frontend V2 Fee Observability

The V2-fee branch of MarginEngineV2 + FeesManagerV2 is now
production-validated on Base Sepolia (option product, Tier0, no rebate,
no Merkle claim). Remaining work spans two adjacent surfaces:

1. **Frontend fee observability** — the `frontend/` does not yet render
   the V2 `FeeChargedV2` stream. Likely the order/positions/PnL UIs read
   `observed_total_charged` and `net_protocol_fee` from
   `/admin/fees/onchain` (or a public mirror endpoint) but should also
   surface `event_model = "mixed"` vs `"v2"` and the per-side `feePpm`
   for an explainable fee breakdown. V2E-G's
   `source_priority = "v2"` already gives the frontend an authoritative
   source. No frontend code was changed in this task.
2. **Rebate path validation (deferred)** — Tier0 has no negative makerPpm,
   so the rebate code path (`FeeRebatedV2`, `RebateBudgetSpent`,
   `claimTier`, Merkle leaves) is untested live. Requires Tier1+ claim
   (Merkle root set + leaf-claim tx) and a funded `rebateBudget(mUSDC)`.
   Both are explicitly out of V2E-G scope.
3. **Perp product V2 fee adoption** — `FeesManagerV2` profiles
   `ProductKind.PERP` are already set (`Tier0: maker=50 ppm, taker=300
   ppm`), `productFeeBasis(PERP) = NOTIONAL`. Wiring `PerpEngine` to
   `FeesManagerV2` (analogous to `MarginEngineV2.setFeesManagerV2 +
   setUseFeesManagerV2(true)`) is the next launch milestone. Backend
   indexer already decodes the V2 events regardless of emitting product.
4. **Tier claim / Merkle root validation** — Tier0 only; tier-claim flow
   (`claimTier(account, tier, volume28d, …, proof[])`,
   `setMerkleRoot(root, validFrom, validUntil)`) is on-chain but
   exercised only in unit tests. Live validation requires a real
   `(MerkleRootSetV2, TierClaimedV2)` pair.
5. **Fee recipient address publication** — V2 fee recipient
   `0xa67f8e8e673ce4bb2fb563b0e6e9fa8f70e3b588` is distinct from V1's
   `0x009f38440f058d095b61e0e2ee7fabdf05be7500`. Operator should
   confirm this is the intended treasury/operator address and document
   it in CLAUDE-side runbook.

There are no remaining backend-side blockers for V2 option tiny-trade
broadcast on Tier0.
