# Tiny Option Trade Preflight Against NEW MarginEngine — V2D-U

Date: 2026-05-28
Network: Base Sepolia (chain id 84532)
Mode: Pre-broadcast preflight. **No broadcast.** No `/broadcast` endpoint
called, no `eth_sendRawTransaction`, no FeesManagerV2 enable.

## Outcome

**All preflight gates green.** A fresh tiny option execution intent
(`a6369dd5-54cd-4407-a4c5-7902bba286f7`) was created against NEW
MarginEngine via the established V1S orderbook-fill flow, both EIP-712
signatures collected, calldata generated (selector `0x031f77b3`, same
as V1S), simulation returned `simulation_ok` at block `42100183`, and
gas safety preview returned `gas_check_status = ok` with `+326_443`
gas of headroom over the 1.25× safety margin.

No `option_execution_transactions` or `execution_transactions` row was
created. The intent sits at `calldata_ready`, awaiting explicit operator
authorization to broadcast.

## Hard Rules Verification

| Rule | Status |
| --- | --- |
| Do not broadcast | ✅ `/options/execution-intents/:id/broadcast` not called |
| Do not submit transactions | ✅ no `eth_sendRawTransaction`; backend log has 0 "broadcast" hits |
| Do not call `/executor/broadcast` | ✅ not called |
| Do not deploy | ✅ no deploy script touched |
| Do not modify Solidity | ✅ `../deopt-v2-sol/` source untouched (only `MockPriceSource.setPrice` calls via existing refresh script) |
| Do not modify frontend | ✅ no frontend changes |
| Do not enable FeesManagerV2 | ✅ NEW still `useFeesManagerV2 = false`, `feesManagerV2 = 0x0` |
| Do not deploy FeesManagerV2 | ✅ no deploy |
| Do not call `setUseFeesManagerV2` | ✅ no admin write |
| Do not cleanup historical rows | ✅ no DELETE/UPDATE on evidence tables |
| Do not print private keys | ✅ no secrets in this doc, in `.env`, or echoed back |
| Do not commit real `.env` | ✅ runtime overrides remain in gitignored `.env.cutover.v2d_s.local` |

## Backend Config Summary

`/admin/config` after restart (preflight-relevant fields, no secrets):

| Field | Value |
| --- | --- |
| `chain_id` | `84532` |
| `network` | `base-sepolia` |
| `options.event_indexer.margin_engine_address` | `0x287Cef479be5889eEfCa847F9e73C860898f48Cc` (NEW) |
| `options.event_indexer.matching_engine_address` | `0xf2D1D85cD363Be3bc160d14883C80e7C2c4F420b` |
| `options.event_indexer.fees_manager_address` | `0xaef73F10224712E1312963BE11662061481aA0F0` (V1) |
| `options.event_indexer.fees_manager_v2_address` | `null` |
| `options.event_indexer.enabled` | `true` |
| `options.confirmation_worker.enabled` | `true` |
| `options.reconciliation_worker.enabled` | `true` |
| `options.execution_enabled` | `true` |
| `options.execution_simulation_enabled` | `true` |
| `options.execution_broadcast_enabled` | `false` |
| `options.execution_broadcast_gas_limit` | `1500000` |
| `options.execution_gas_safety_bps` | `12500` |
| `contracts.option_matching_engine_address` | `0xf2D1D85cD363Be3bc160d14883C80e7C2c4F420b` |
| `contracts.option_execution_eip712_verifying_contract` | `0xf2D1D85cD363Be3bc160d14883C80e7C2c4F420b` |
| `features.real_broadcast_enabled` | `false` |
| `features.execution_enabled` | `false` |
| `execution.dry_run` | `true` |
| `option_nonce_sync_enabled` | `true` (added in second restart so the intent gets correct on-chain nonces) |

Deviation from `NEXT_TASK.md`: the broadcast surfaces
(`OPTION_EXECUTION_BROADCAST_ENABLED`, `EXECUTION_ENABLED`,
`EXECUTOR_REAL_BROADCAST_ENABLED`, `EXECUTOR_DRY_RUN=false`) were
**not** enabled for this run, because the operator chose not to expose
`EXECUTOR_PRIVATE_KEY` in this subshell. The preflight goal — fresh
intent, signatures, calldata, simulation, gas safety preview — was
fully achieved without them; gas safety was computed from an external
`eth_estimateGas` (the V1R pattern) instead of the broadcast-endpoint's
internal gas check. This is a strictly *safer* configuration: the
broadcast endpoint is structurally unreachable, so the
no-broadcast guarantee holds at the config level.

## Step 1 — Baseline

`V2D_U_START_MS = 1779963929264`.

| Table | Count (pre-preflight) |
| --- | --- |
| `option_execution_intents` | 3 |
| `option_execution_transactions` | 2 |
| `execution_transactions` | 1 |
| `option_execution_events` | 19 |
| `option_execution_reconciliations` | 1 |
| `fee_events` | 28 |

`option_event_indexer_state.last_indexed_block = 42077113` carried over
from V2D-T2.

## Step 2 — NEW engine read-only checks

`cast call <NEW>` against Base Sepolia at preflight time:

```
feesManager()        → 0xaef73F10224712E1312963BE11662061481aA0F0  (V1)
feesManagerV2()      → 0x0000000000000000000000000000000000000000
useFeesManagerV2()   → false
```

Matches the V2D-S / V2D-T2 expectation: V2 fee path is disabled, V1
fee path active.

## Step 3 — Oracle refresh (existing testnet workflow)

Pre-refresh probe found the mock feeds stale —
`getPriceSafe(WETH, USDC) = (0, 0, false)` — same condition V1P/V1R
hit. The existing `script/RefreshTestnetMockFeeds.s.sol` from
`../deopt-v2-sol` was run via `forge script ... --broadcast --slow`.
Four `MockPriceSource.setPrice` transactions confirmed:

| Tx | Block | Status |
| --- | ---: | --- |
| `0xe2be5d151ee5a6b922468531f91438bf9e31b8d737b5448dd4ca5c0e75b56a81` | 42100005 | 0x1 |
| `0xfef1bb0e247abece84eb04c9557b6eded7bb32349f95de7867713df23dcd2683` | 42100006 | 0x1 |
| `0x28bdf30f1362ad0d08620a52b57ede32a060118f9712ac76a3853d99535dd28b` | 42100007 | 0x1 |
| `0x51745feb42509544244c67a227e658df37a2db1bd5bd61796a8ce25b012108b5` | 42100008 | 0x1 |

Post-refresh state:

| Probe | Value |
| --- | --- |
| `OracleRouter.paused()` | `false` |
| `OracleRouter.readPaused()` | `false` |
| `OracleRouter.maxOracleDelay()` | `600 s` |
| ETH/USDC primary `getLatestPrice()` | `(300_000_000_000, 1779968298)` |
| `getPriceSafe(WETH, USDC)` | `(300_000_000_000, 1779968298, true)` — **ok=true** |
| age at simulation time | ≤ 60 s (well under the 60 s feed cap and 600 s router cap) |

The refresh used the **established existing workflow only** — same
`script/RefreshTestnetMockFeeds.s.sol` V1P / V1R / V1S each used. No
new Solidity, no new script.

## Step 4 — Tiny intent creation

Used the V1S orderbook-fill flow (`POST /options/orders`):

| Field | Value |
| --- | --- |
| `option_series_id` | `0x8b34d095ebfb300f21868dea4a0ff5e1d6f8ebd5463facaa8bcbc6075df50e6d` (same series V1S used; expiry `1893456000`, strike `300_000_000_000 / 1e8`, is_call=`true`, contract_size `1e8`) |
| Seller resting order | `27604c20-2252-4798-b389-73a66ad8799a` (sell GTC `price_1e8=10000`, `size_1e8=100000000` = 1 contract) |
| Buyer crossing order | `5cbc2d09-9e94-463a-9db7-3643d68fddc4` (buy GTC same price/size; immediately filled) |
| Fill (`source_id`) | `3a88708b-3bde-4c2b-bdd5-a15a26c11a8b` |
| **Intent id** | **`a6369dd5-54cd-4407-a4c5-7902bba286f7`** |
| `onchain_intent_id` | `0xfd1e11ab5dfecdf2943f6a934752bce4ab862f3f5a7192a82ef18807cb0a9ee4` |
| Initial status | `signatures_required` → `calldata_ready` after sigs submitted |
| Buyer | `0xc0A76c2A6c6b70C0B065A05E64417886416cc976` |
| Seller | `0xbAf0976a00a0DCc84Df5B15d927695c8b014B1c3` |
| `option_id` (uint256) | `24145907678156652148089862289363692212069910767044828147380657249455352740183` |
| `premium_per_contract` | `100` (native settlement units; mapped from `price_1e8 = 10000`) |
| `quantity` | `1` |
| `buyer_is_maker` | `false` (taker buy crossed resting sell — matches V1S pattern) |
| `buyer_nonce` | `1` (from `OptionMatchingEngine.nonces(buyer)` — V1S consumed `0`) |
| `seller_nonce` | `1` (from `OptionMatchingEngine.nonces(seller)` — V1S consumed `0`) |
| `deadline` | `0` (preserved from V1S behavior; matching-engine treats `0` as "no deadline") |

A first attempt earlier in the session (intent
`563d5884-31b4-4142-991e-a416d6e9a934`) produced `buyer_nonce=0` and
`seller_nonce=0` because `OPTION_NONCE_SYNC_ENABLED` was off in the
first backend restart. After enabling
`OPTION_NONCE_SYNC_ENABLED=true` /
`OPTION_NONCE_SYNC_REQUIRE_RPC=true` /
`OPTION_NONCE_SYNC_STRICT=true` and submitting a fresh order pair,
the second intent (`a6369dd5-…`) carries the live nonces. Intent
`563d5884-…` is left as `signatures_required` (no broadcast attempt
made; no `option_execution_transactions` row created) so no historical
evidence is mutated.

## Step 5 — Signatures + calldata

EIP-712 domain (from signing-payload, matches `/admin/config`):

```
name              = DeOptV2-OptionMatchingEngine
version           = 1
chainId           = 84532
verifyingContract = 0xf2D1D85cD363Be3bc160d14883C80e7C2c4F420b
primaryType       = OptionTrade
digest            = 0xd8b78996ca1ef3ed4a23af8cb994a730929523f274fb44baded5b71f621ec3c1
```

Signatures generated locally via
`./target/debug/sign_option_execution_intent --payload-file
/tmp/v2d_u/payload.json --private-key-env {BUYER,SELLER}_PRIVATE_KEY`:

| Signer | Recovered address | Sig length |
| --- | --- | --- |
| Buyer | `0xc0a76c2a6c6b70c0b065a05e64417886416cc976` | 132 chars (65 bytes) |
| Seller | `0xbaf0976a00a0dcc84df5b15d927695c8b014b1c3` | 132 chars (65 bytes) |

`POST /options/execution-intents/<intent>/signatures` returned:

| Field | Value |
| --- | --- |
| `status` | `calldata_ready` |
| `buyer_signature_present` | `true` |
| `seller_signature_present` | `true` |
| `missing_signatures` | `false` |
| `calldata_ready` | `true` |
| `calldata` selector | `0x031f77b3` (`executeTrade(OptionTrade,bytes,bytes)`) — **same as V1S** |
| `calldata` length | 1674 chars (836 bytes; lifecycle reports `hex_length=1672` excluding the `0x` prefix) |

Backend `OPTION_EXECUTION_SIGNATURE_MODE = disabled` (matches V1S);
backend stores the signatures without cryptographic recovery, but the
contract will verify them on-chain at broadcast time. The sign helper
verified the recovered signer matches buyer/seller before printing.

## Step 6 — Simulation against NEW

`POST /options/execution-intents/<intent>/simulate`:

```
{
  "intent_id": "a6369dd5-54cd-4407-a4c5-7902bba286f7",
  "simulation_status": "simulation_ok",
  "block_number": 42100183,
  "error": null,
  "revert_data": null,
  "revert_selector": null,
  "simulated_at_ms": 1779968655510,
  "submitted": false,
  "confirmed": false
}
```

`simulation_ok` against NEW MarginEngine at block `42100183` — the
OptionMatchingEngine's `marginEngine()` resolves to NEW per V2D-R's
rewire, and the on-chain V2 path returns success for V1-fee flows.

## Step 7 — Gas safety preview

Read-only `eth_estimateGas` for the exact prepared transaction
(executor `0xc35F7A8A…`, `to = OptionMatchingEngine`, `value = 0`,
`data = intent.calldata`):

| Field | Value |
| --- | ---: |
| `estimated_gas` | **938_846** |
| `gas_safety_bps` | **12_500** |
| `required_gas = estimated_gas × 12500 / 10000` | **1_173_557** |
| `broadcast_gas_limit` (configured) | **1_500_000** |
| `broadcast_gas_limit >= estimated_gas` | **true** |
| `broadcast_gas_limit >= required_gas` | **true** |
| Headroom over `required_gas` | **+326_443 gas** |
| `gas_check_status` | **ok** |

Executor balance preview:

| Field | Value |
| --- | ---: |
| `executor` | `0xc35F7A8A103A9A4464adfaa76B9B514093D23C27` |
| balance | `8_068_673_269_919_280 wei` (~`0.00807 ETH`) |
| worst-case broadcast cost (1_500_000 × 1 gwei) | `0.00150 ETH` |
| balance / worst ratio | **5.38×** (comfortable; V1R recorded ~5.4× and broadcast succeeded) |

`estimated_gas` is ~14% lower than V1S's `1_091_120`; this is the
expected drift between two different prepared traces (different
storage warmth, different nonce slot, freshly refreshed oracle). The
12_500-bps (25%) safety margin absorbs this drift comfortably.

## Step 8 — No-broadcast verification

| Source of truth | Result |
| --- | --- |
| `option_execution_transactions` rows since `V2D_U_START_MS` | **0** |
| `execution_transactions` rows since `V2D_U_START_MS` | **0** |
| Backend log lines mentioning `broadcast` or `sendRawTransaction` | **0** |
| Final intent `a6369dd5-…` lifecycle `broadcast` field | `null` |
| Final intent `a6369dd5-…` lifecycle `confirmation` field | `null` |
| Backend feature flag `options.execution_broadcast_enabled` | `false` (broadcast endpoint structurally rejects) |
| Backend feature flag `features.real_broadcast_enabled` | `false` |
| `EXECUTOR_PRIVATE_KEY` configured | **no** (never entered the runtime subshell) |

The new intents added to the DB (2) are `signatures_required` /
`calldata_ready` — both pre-broadcast states. The previously-created
intent `563d5884-…` from the nonce-sync-off attempt is also
pre-broadcast.

Absolute table totals at end of preflight:

| Table | Count |
| --- | --- |
| `option_execution_intents` | 5 (+2: `563d5884-…`, `a6369dd5-…`; both pre-broadcast) |
| `option_execution_transactions` | 2 (unchanged) |
| `execution_transactions` | 1 (unchanged) |
| `option_execution_events` | 19 (unchanged) |
| `option_execution_reconciliations` | 1 (unchanged) |
| `fee_events` | 28 (unchanged) |

## Step 9 — Indexer cursor

Background poll loop ran throughout the preflight. Final cursor advanced
to `~42100206` (past target `42073775` + cutover safety, comfortably
past simulation block `42100183`). Zero logs found in the tick range —
expected, since no broadcast happened.

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
| `cargo fmt --all` | clean (no code changes) |
| `cargo clippy --all-targets --all-features -- -D warnings` | clean |
| `cargo test --all-targets --all-features --no-fail-fast` | **passed: 601, failed: 0, ignored: 0** |
| `cargo build --all-targets --all-features` | clean |

V2D-U introduces no code changes; only docs and an appended block in
the gitignored `.env.cutover.v2d_s.local`.

## Remaining Blocker Before Human Tiny Broadcast

The preflight is complete and every gate is green for the next human
broadcast attempt. The remaining gates are operator actions, not
infrastructure blockers:

1. **Explicit operator authorization** — a separate "yes, broadcast
   now" confirmation, then exactly one
   `POST /options/execution-intents/a6369dd5-54cd-4407-a4c5-7902bba286f7/broadcast`.
   No retry, no `/executor/broadcast`.
2. **Expose `EXECUTOR_PRIVATE_KEY` in the broadcast shell** and flip
   `OPTION_EXECUTION_BROADCAST_ENABLED=true`, `EXECUTION_ENABLED=true`,
   `EXECUTOR_REAL_BROADCAST_ENABLED=true`, `EXECUTOR_DRY_RUN=false`,
   `OPTION_EXECUTION_BROADCAST_GAS_LIMIT=1500000`,
   `OPTION_EXECUTION_GAS_SAFETY_BPS=12500`. (All other flags already
   match the V1R/V1S/V2D-U baseline.)
3. **Re-refresh the oracle right before the broadcast** if more than
   ~50 s have elapsed since the V2D-U refresh — the mock feed
   `maxDelay` is 60 s and the simulation re-runs the same staleness
   check the SC does. Use the same
   `script/RefreshTestnetMockFeeds.s.sol --broadcast --slow` script.
4. **Re-pull live nonces** at broadcast time (`cast call <ME>
   "nonces(address)" <buyer/seller>`) and confirm they still equal
   `1`. If any other broadcast bumped them in between, regenerate the
   intent (signatures will need to be re-issued with the new nonces).
5. **FeesManagerV2 stays disabled** intentionally — this tiny trade
   charges V1 fees only and exercises NEW MarginEngine on the V1 fee
   path. V2E/V2F own the FeesManagerV2 enablement.
6. **Paid `RPC_URL`** must stay exported in the broadcast shell —
   simulation, gas estimate, broadcast, and confirmation all share
   the same RPC budget.
