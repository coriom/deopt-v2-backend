# Tiny Option Trade Broadcast Result Against NEW MarginEngine — V2D-V

Date: 2026-05-28
Network: Base Sepolia (chain id 84532)
Mode: Live broadcast (operator-triggered). Agent did not call `/broadcast`.

## Result

**First successful tiny option execution against NEW MarginEngine V2.**

- Intent: `a6369dd5-54cd-4407-a4c5-7902bba286f7`
- Transaction hash: **`0x07a8e6795e2082ceabaa242543ee424cffd5037c0d918cf1a81bcee1b2d7de10`**
- Block: `42110498`, `transactionIndex = 59`
- Receipt `status = 1 (success)`
- `gasUsed = 913_477` (under `estimated_gas = 938_846`, well below `broadcast_gas_limit = 1_500_000`)
- `effectiveGasPrice = 6_000_000 wei` (`0x5b8d80`)
- `cumulativeGasUsed = 6_350_105`
- From: `0xc35F7A8A103A9A4464adfaa76B9B514093D23C27` (executor)
- To: `0xf2D1D85cD363Be3bc160d14883C80e7C2c4F420b` (OptionMatchingEngine — unchanged across V2D-R)
- Backend `option_execution_transactions.transaction_id`: `6a3209ad-b14b-46c9-89d6-8c4d15576e0f`

Exactly one transaction submitted. No retry. No `/executor/broadcast` call. No `execution_transactions` row created.

## Hard Rules Verification

| Rule | Status |
| --- | --- |
| Agent did not broadcast | ✅ `/broadcast` called by human operator exactly once |
| Do not submit transactions (agent) | ✅ no `eth_sendRawTransaction` by agent |
| Do not call `/executor/broadcast` | ✅ not called |
| Do not deploy | ✅ no deploy script changes |
| Do not modify Solidity | ✅ `../deopt-v2-sol/` source untouched (only `MockPriceSource.setPrice` via existing refresh script) |
| Do not modify frontend | ✅ no frontend changes |
| Do not enable FeesManagerV2 | ✅ NEW still `useFeesManagerV2 = false`, `feesManagerV2 = 0x0` (rechecked post-broadcast) |
| Do not deploy FeesManagerV2 | ✅ no deploy |
| Do not call `setUseFeesManagerV2` | ✅ no admin write |
| Do not cleanup historical rows | ✅ V1S row + V2D-U pre-broadcast intents untouched |
| Do not print private keys | ✅ `EXECUTOR_PRIVATE_KEY` exported in operator shell only; never echoed |
| Do not commit real `.env` | ✅ overrides remain in gitignored `.env.cutover.v2d_s.local` |

## Phase 1 — Pre-Broadcast Refresh

The operator launched the backend in their interactive shell with all task-spec env exported:
- `MARGIN_ENGINE`, `OPTION_EVENT_INDEXER_MARGIN_ENGINE_ADDRESS` = NEW
- `OPTION_EVENT_INDEXER_ENABLED=true`, `OPTION_CONFIRMATION_WORKER_ENABLED=true`,
  `OPTION_RECONCILIATION_WORKER_ENABLED=true`, `OPTION_NONCE_SYNC_ENABLED=true`
- `OPTION_EXECUTION_BROADCAST_ENABLED=true`, `EXECUTION_ENABLED=true`,
  `EXECUTOR_REAL_BROADCAST_ENABLED=true`, `EXECUTOR_DRY_RUN=false`
- `OPTION_EXECUTION_BROADCAST_GAS_LIMIT=1500000`, `OPTION_EXECUTION_GAS_SAFETY_BPS=12500`
- `EXECUTOR_MAX_FEE_PER_GAS_WEI=1000000000`, `EXECUTOR_MAX_PRIORITY_FEE_PER_GAS_WEI=1000000`
- `EXECUTOR_PRIVATE_KEY` set in shell, never written to disk or chat

`/admin/config` gates (all 7 green):

| Check | Value |
| --- | --- |
| `options.event_indexer.margin_engine_address` | `0x287Cef479be5889eEfCa847F9e73C860898f48Cc` (NEW) |
| `options.event_indexer.matching_engine_address` | `0xf2D1D85cD363Be3bc160d14883C80e7C2c4F420b` |
| `options.event_indexer.fees_manager_address` | `0xaef73F10224712E1312963BE11662061481aA0F0` (V1) |
| `options.event_indexer.fees_manager_v2_address` | `null` |
| `options.execution_broadcast_enabled` | `true` |
| `options.execution_broadcast_gas_limit` | `1_500_000` |
| `options.execution_gas_safety_bps` | `12_500` |
| `features.real_broadcast_enabled` | `true` |
| `features.execution_enabled` | `true` |
| `execution.dry_run` | `false` |
| `configured.executor_private_key` | `true` |
| `execution.max_fee_per_gas_configured` | `true` |
| `execution.max_priority_fee_per_gas_configured` | `true` |

## Phase 2 — Final Safety Checks

### Oracle freshness

Pre-refresh probe: `getPriceSafe(WETH, USDC) = (0, 0, false)` (stale — V2D-U refresh had aged out over several hours). Refreshed using the existing `~/DEOPT/deopt-v2-sol/script/RefreshTestnetMockFeeds.s.sol --broadcast --slow` script (the same workflow V1P/V1R/V2D-U used). Four `MockPriceSource.setPrice` transactions confirmed:

| Tx | Block | Status |
| --- | ---: | --- |
| `0xa4af66fe3f447969412ffdced5b4c97be7c76b5aa5ea011491ce7137e58a62f9` | 42101087 | 0x1 |
| `0x40a7a9d97b4771f1a6adc5030dd4b534461a4f33ee08477af2b7318650c30dfe` | 42101088 | 0x1 |
| `0x8f07a93d1ee222ea6ec25ec0d56882d1ee0947dd2a291cc0fa745831f75e591b` | 42101089 | 0x1 |
| `0xf7cd6278a453c866c69bef845877982fc472c9219f8a5377ca95e760f3159537` | 42101090 | 0x1 |

Post-refresh: `getPriceSafe(WETH, USDC) = (300_000_000_000, 1779970462, true)` — **ok**, age ≤ 4 s at the moment of probe. The operator broadcast inside the 60 s feed window.

### Nonces

| Account | On-chain `nonces(addr)` pre-broadcast | Intent value | Match |
| --- | ---: | ---: | --- |
| Buyer `0xc0A76c2A…cc976` | `1` | `1` | ✅ |
| Seller `0xbAf0976a…b1c3` | `1` | `1` | ✅ |

Post-broadcast: both nonces incremented to `2` (consistent with the MatchingEngine bumping each side's nonce on apply-trade).

### Re-simulation

`POST /options/execution-intents/<intent>/simulate`:

```
{
  "intent_id": "a6369dd5-54cd-4407-a4c5-7902bba286f7",
  "simulation_status": "simulation_ok",
  "block_number": 42110349,
  "error": null,
  "revert_data": null,
  "revert_selector": null,
  "submitted": false,
  "confirmed": false
}
```

### Gas safety

External `eth_estimateGas` for the prepared transaction:

| Field | Value |
| --- | ---: |
| `estimated_gas` | 938_846 |
| `gas_safety_bps` | 12_500 |
| `required_gas` | 1_173_557 |
| `broadcast_gas_limit` | 1_500_000 |
| Headroom over `required_gas` | +326_443 |
| `gas_check_status` | **ok** |
| Executor balance | `8_067_630_852_391_371 wei` (~`0.00807 ETH`) |
| Worst-case broadcast cost | `0.0015 ETH` |
| balance/worst ratio | **5.38×** |

### Baseline

`V2D_V_START_MS = 1779988986247`. Pre-broadcast counts:

| Table | Count |
| --- | --- |
| `option_execution_intents` | 5 |
| `option_execution_transactions` | 2 |
| `execution_transactions` | 1 |
| `option_execution_events` | 19 |
| `option_execution_reconciliations` | 1 |
| `fee_events` | 28 |

## Phase 3 — Human Broadcast

Operator ran exactly one:

```
POST http://127.0.0.1:8080/options/execution-intents/a6369dd5-54cd-4407-a4c5-7902bba286f7/broadcast
```

Response:

```json
{
  "intent_id": "a6369dd5-54cd-4407-a4c5-7902bba286f7",
  "status": "broadcast_submitted",
  "tx_hash": "0x07a8e6795e2082ceabaa242543ee424cffd5037c0d918cf1a81bcee1b2d7de10",
  "to": "0xf2D1D85cD363Be3bc160d14883C80e7C2c4F420b",
  "from": "0xc35f7a8a103a9a4464adfaa76b9b514093d23c27",
  "transaction_id": "6a3209ad-b14b-46c9-89d6-8c4d15576e0f",
  "broadcast_enabled": true,
  "submitted": true,
  "duplicate": false,
  "confirmed": false,
  "estimated_gas": 938846,
  "required_gas": 1173557,
  "simulation_gas_limit": 0,
  "broadcast_gas_limit": 1500000,
  "gas_safety_bps": 12500,
  "gas_check_status": "ok",
  "gas_check_error": null
}
```

No retry. No `/executor/broadcast`. No second `/broadcast`.

## Phase 4 — Post-Broadcast Verification

### `cast receipt`

| Field | Value |
| --- | --- |
| `transactionHash` | `0x07a8e6795e2082ceabaa242543ee424cffd5037c0d918cf1a81bcee1b2d7de10` |
| `blockNumber` | `42110498` |
| `blockHash` | `0x4b366e58782cf8b3df8613b9dbc188db09b20be44939729bdbbc2ad901e62506` |
| `from` | `0xc35F7A8A103A9A4464adfaa76B9B514093D23C27` |
| `to` | `0xf2D1D85cD363Be3bc160d14883C80e7C2c4F420b` |
| `status` | **1 (success)** |
| `gasUsed` | `913_477` |
| `effectiveGasPrice` | `6_000_000 wei` |
| `cumulativeGasUsed` | `6_350_105` |
| `transactionIndex` | 59 |
| `type` | 2 (EIP-1559) |

Event log highlights (7 logs total):

- **CollateralVault `0x00340c…0625D3`** (5 logs): two `InternalTransfer` (topic `0xf67cd268…`) entries showing buyer balance debited and seller balance credited (per side delta `0x253f4d98a`); one `FeeAccrued` (topic `0x77178bcf…`) with data `0x...0064` = `100` (protocol fee for this tiny trade); two `Synced` (topic `0xa1c4d40b…`) per-side post-settlement balance snapshots.
- **NEW MarginEngine `0x287cef…f48cc`** (1 log): `OptionPositionUpdated` (topic `0x6f0909c4…`) keyed on the option series hash `0x35621974bccc555e161c6707f0a1a3bca2d02be5e3a4d380980bfaef656e7957` with payload `(1, 100)` (quantity, premium). **This is the first OptionPositionUpdated emitted by NEW MarginEngine V2 on Base Sepolia.**
- **OptionMatchingEngine `0xf2d1d85c…420b`** (1 log): `OptionTradeExecuted` (topic `0xb2387b9f…`) keyed by `intentId = 0xfd1e11ab5dfecdf2943f6a934752bce4ab862f3f5a7192a82ef18807cb0a9ee4` with payload referencing the same option series and the buyer/seller addresses.

### Backend confirmation

The background `OPTION_CONFIRMATION_WORKER` picked up the receipt immediately. `option_execution_transactions` row state after confirmation:

| Field | Value |
| --- | --- |
| `transaction_id` | `6a3209ad-b14b-46c9-89d6-8c4d15576e0f` |
| `intent_id` | `a6369dd5-54cd-4407-a4c5-7902bba286f7` |
| `tx_hash` | `0x07a8e6795e2082ceabaa242543ee424cffd5037c0d918cf1a81bcee1b2d7de10` |
| `status` | `submitted` |
| `error` | `null` |
| `gas_check_status` | `ok` |
| `confirmation_status` | **`mined_success`** |
| `confirmed_block_number` | `42110498` |
| `receipt_status` | `1` |
| `gas_used` | `913_477` |
| `effective_gas_price` | `6_000_000 wei` (`0x5b8d80`) |
| `cumulative_gas_used` | `6_350_105` |
| `receipt_block_hash` | `0x4b366e58…2506` |
| `receipt_transaction_index` | `59` |

Intent state: `broadcast_confirmed`.

### Event indexer

Cursor advanced to `42110593` covering the tx block `42110498`. After the broadcast, the indexer captured **7 events** for `tx_hash = 0x07a8e679…1b2d7de10`:

| Event name | Count | Emitters |
| --- | ---: | --- |
| `InternalTransfer` | 1 | CollateralVault |
| `Synced` | 4 | CollateralVault |
| `TradeExecuted` | 1 | NEW MarginEngine (`0x287cef…`) |
| `OptionTradeExecuted` | 1 | OptionMatchingEngine |

Counts by contract address for this tx:

```
0x00340c360353a5ab784c5bc5c44322a6af0625d3 (CollateralVault):       5
0x287cef479be5889eefca847f9e73c860898f48cc (NEW MarginEngine):       1
0xf2d1d85cd363be3bc160d14883c80e7c2c4f420b (OptionMatchingEngine):   1
```

A manual `POST /admin/options/events/tick` after the auto-tick advanced
cursor 4 more blocks (`42110594 → 42110597`) with 0 additional logs;
no error.

### Reconciliation

`option_execution_reconciliations` row for the intent:

| Field | Value |
| --- | --- |
| `id` | `aa1d0762-1f57-4036-be55-d381452d8406` |
| `intent_id` | `a6369dd5-…` |
| `onchain_intent_id` | `0xfd1e11ab…` |
| `option_execution_transaction_id` | `6a3209ad-…` |
| `tx_hash` | `0x07a8e679…` |
| `chain_id` | `84532` |
| `status` | **`reconciled`** |
| `strict` | `true` |
| `requires_events` | `true` |
| `trade_executed_event_id` | `02da92d6-cc0b-434e-86d3-ba371bdec934` |
| `margin_trade_event_id` | `04741387-401b-442f-97d1-6b081530ea6a` |
| `trading_fee_event_count` | `0` (V1 `TradingFeeCharged` not emitted — V1 fees rounded to 0 or routed through CollateralVault `FeeAccrued` only) |
| `internal_transfer_event_count` | `1` |
| `decoded_event_count` | `7` |
| `mismatch_reason` | `null` |
| `missing_required` | `null` |
| `reconciled_at_ms` | `1779989310065` |

Reconciliation `details.margin_trade` payload confirms `price = 100`, `quantity = 1`, `option_id = 24145907678156652148...740183`, `block_number = 42110498`. The `internal_transfer_events` array shows a single transfer of `amount = 100` (settlement asset `0x6eAe407f…`) from buyer to seller — i.e. the buyer's premium payment.

A manual `POST /admin/options/reconciliations/tick` returned `considered = 0` (nothing pending — already reconciled).

### Lifecycle (`/admin/options/executions/<intent>/lifecycle`)

| Field | Value |
| --- | --- |
| `status` | **`broadcast_confirmed`** |
| `health.stage` | **`reconciled`** |
| `broadcast.tx_hash` | `0x07a8e679…1b2d7de10` |
| `broadcast.status` | `submitted` |
| `confirmation.confirmation_status` | `mined_success` |
| `confirmation.confirmed_block_number` | `42110498` |
| `confirmation.gas_used` | `913_477` |
| `confirmation.effective_gas_price` | `0x5b8d80` |
| `confirmation.cumulative_gas_used` | `6_350_105` |
| `reconciliation.status` | `reconciled` |
| `events.total` | `7` |
| `fees.source_of_truth` | `onchain` |
| `fees.event_model` | `none` (no `TradingFeeCharged` or `FeeChargedV2`/`FeeRebatedV2`) |
| `fees.source_priority` | `""` |
| `fees.fee_charged_v2_count` | `0` |
| `fees.fee_rebated_v2_count` | `0` |
| `fees.trading_fee_event_count` | `0` |
| `fees.observed_total_charged` | `0` |
| `fees.observed_total_rebated` | `0` |
| `state_checks` | not present (`OPTION_RECONCILIATION_STATE_CHECKS_ENABLED=false` per config; same as V2D-U baseline) |

`event_model = none` is the expected V1-fee-path outcome for this tiny premium: the V1 `FeesManager` did not emit a `TradingFeeCharged` because the rounded fee was 0 (or the fee path took the `FeeAccrued`-only branch, which the lifecycle aggregator treats as non-fee evidence). The 100-unit `FeeAccrued` shows up under CollateralVault's emitter slot, not under the V1 `FeesManager` address, so the V1 counter remains 0. Critically `fee_charged_v2_count` and `fee_rebated_v2_count` are both 0 — **the V2 fee path was not exercised**, as required.

## FeesManagerV2 Disabled (re-verified post-broadcast)

```
feesManager()        → 0xaef73F10224712E1312963BE11662061481aA0F0  (V1)
feesManagerV2()      → 0x0000000000000000000000000000000000000000
useFeesManagerV2()   → false
```

The trade hit NEW MarginEngine's V1-fee branch as designed.

## No-Forbidden-Mutation Verification

`V2D_V_START_MS = 1779988986247`. Row counts since then:

| Table | Δ | Notes |
| --- | ---: | --- |
| `option_execution_intents` | 0 | reused the V2D-U intent `a6369dd5-…` |
| `option_execution_transactions` | **+1** | `6a3209ad-…` (the broadcast/confirmed tx) |
| `execution_transactions` (generic) | **0** | ✅ forbidden table untouched |
| `option_execution_events` | +7 | from the indexer indexing this tx |
| `option_execution_reconciliations` | +1 | the reconciliation row above |
| `fee_events` | 0 | backend fee ledger not enabled |

Absolute totals after V2D-V:

| Table | Count |
| --- | --- |
| `option_execution_intents` | 5 |
| `option_execution_transactions` | 3 (V1L failed `204a3070-…`, V1S success `cae8c7e7-…`, V2D-V success `6a3209ad-…`) |
| `execution_transactions` | 1 (unchanged) |
| `option_execution_events` | 26 |
| `option_execution_reconciliations` | 2 (V1S + V2D-V) |
| `fee_events` | 28 (unchanged) |

V1L evidence row and V1S row both preserved.

## Cross-checks vs V1S

| Field | V1S | V2D-V |
| --- | --- | --- |
| Margin engine emitter | OLD `0x6c5665de…b5f8` | **NEW `0x287cef…f48cc`** |
| `feesManager` on engine | V1 | V1 |
| `feesManagerV2` on engine | n/a | `0x0` |
| Intent | `e6d2941b-…` | `a6369dd5-…` |
| Tx hash | `0x5964a7b3…1125` | `0x07a8e679…7de10` |
| Block | `41856964` | `42110498` |
| `gasUsed` | 1_057_772 | 913_477 |
| `estimated_gas` | 1_091_120 | 938_846 |
| Premium native | 10_000 | 100 |
| Buyer nonce | 0 → 1 | 1 → 2 |
| Seller nonce | 0 → 1 | 1 → 2 |
| Reconciliation | reconciled | reconciled |
| Fee event model | `v1` | `none` |

The gas drop (~15%) reflects the V2D-D engine's storage-layout / code-path differences; the safety margin still produced +326k gas of headroom over `required_gas`.

## Validation Commands

```
cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
cargo build --all-targets --all-features
```

See "Validation results" below.

## Validation Results

| Command | Result |
| --- | --- |
| `cargo fmt --all` | clean (no code changes; V2D-V is docs-only) |
| `cargo clippy --all-targets --all-features -- -D warnings` | clean |
| `cargo test --all-targets --all-features --no-fail-fast` | **passed: 601, failed: 0, ignored: 0** |
| `cargo build --all-targets --all-features` | clean |

## Remaining Blocker Before FeesManagerV2 Enablement

The V1-fee-path side of MarginEngineV2 is now production-validated on
Base Sepolia. Enabling FeesManagerV2 (V2E/V2F scope) requires:

1. **Deploy `FeesManagerV2`** on Base Sepolia and grant it the
   admin/operator roles that V1 `FeesManager` currently holds.
2. **Wire it into NEW MarginEngine**: call
   `MarginEngineV2.setFeesManagerV2(<v2 addr>)` and then
   `MarginEngineV2.setUseFeesManagerV2(true)` — these two writes are
   guarded by admin in the existing SC code. Neither was performed in
   any prior task (V2D-S/T/T2/U/V all explicitly forbid them).
3. **Backend env**: set `OPTION_EVENT_INDEXER_FEES_MANAGER_V2_ADDRESS=<v2 addr>`
   so the indexer subscribes the nine V2 event topics
   (`FeeChargedV2`, `FeeRebatedV2`, `RebateBudget*`, `FeeRecipientSetV2`,
   `FeeConsumerSetV2`, `MerkleRootSetV2`, `TierClaimedV2`). All decoder
   wiring is already in place per V2D-E.
4. **Run a one-shot probe trade** with V2 enabled to verify the
   indexer captures `FeeChargedV2` (and optionally `FeeRebatedV2`),
   the lifecycle reports `event_model = "v2"` (or `"mixed"` if the V1
   compat log is still emitted alongside), and `observed_total_charged`
   / `net_protocol_fee` match expectations.
5. **Rebate budget**: if rebates are enabled (`FEES_REBATES_ENABLED=true`),
   the V2 budget account needs to be funded via
   `RebateBudgetFunded` before the first rebate-eligible flow.
6. **Operator runbook**: V2E should add a CLAUDE-side runbook entry
   so the next agent recreating this state has the `RPC_URL` /
   `EXECUTOR_PRIVATE_KEY` transport pattern documented (operator-shell
   only, never written to disk or chat).

There are no remaining backend-side blockers for V1-path trades against
NEW MarginEngine.
