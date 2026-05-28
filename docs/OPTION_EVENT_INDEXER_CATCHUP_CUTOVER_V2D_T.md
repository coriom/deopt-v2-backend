# Option Event Indexer Catch-up To MarginEngineV2 Cutover — V2D-T

Date: 2026-05-28
Network: Base Sepolia (chain id 84532)
Mode: Read-only / admin verification. No broadcast. No transaction submission.

## Outcome

**Not reached on this attempt.** Catch-up was blocked by the Alchemy
free-tier RPC: every `eth_getLogs` request is capped at 10 blocks and
the project key is also rate-limited on compute units per second. After
60 manual ticks the cursor advanced from 41857003 to 41857113 (110
blocks). The target — ≥ CUTOVER_BLOCK + confirmation_blocks = 42073775
— was still 216,662 blocks away.

**Superseded by V2D-T2 (2026-05-28).** The follow-up run on the same
day used the operator's higher-tier RPC and finished the catch-up:
cursor reached `42077113` in 42 ticks at `BATCH_BLOCKS=5000`. See
[`OPTION_EVENT_INDEXER_CATCHUP_CUTOVER_V2D_T2.md`](OPTION_EVENT_INDEXER_CATCHUP_CUTOVER_V2D_T2.md).
The V2D-T narrative below is preserved as the failure-mode record of
the free-tier limits.

## Hard Rules Verification

| Rule | Status |
| --- | --- |
| Do not broadcast | ✅ no broadcast invoked |
| Do not submit transactions | ✅ no `eth_sendRawTransaction` |
| Do not deploy | ✅ no deploy script touched |
| Do not modify Solidity | ✅ `../deopt-v2-sol/` untouched |
| Do not modify frontend | ✅ no frontend changes |
| Do not enable FeesManagerV2 | ✅ NEW still reports `useFeesManagerV2 = false`; `fees_manager_v2_address = null` |
| Do not deploy FeesManagerV2 | ✅ no deploy |
| Do not call `setUseFeesManagerV2` | ✅ no admin write |
| Do not create option execution intents | ✅ count unchanged |
| Do not create option execution transactions | ✅ count unchanged |
| Do not create generic execution transactions | ✅ count unchanged |
| Do not cleanup historical evidence rows | ✅ no DELETE/UPDATE |
| Do not print private keys | ✅ no secret printed |
| Do not commit real `.env` | ✅ overrides remain in gitignored `.env.cutover.v2d_s.local` |

## Step 1 — Baseline

`V2D_T_START_MS = 1779961139060`

`option_event_indexer_state` (newest first):

| `last_indexed_block` | `last_error` | `updated_at_ms` |
| --- | --- | --- |
| `41857003` | `null` | `1779917329843` |

Mutation-sensitive table baselines:

| Table | Count |
| --- | --- |
| `option_execution_intents` | 3 |
| `option_execution_transactions` | 2 |
| `execution_transactions` | 1 |
| `option_execution_events` | 19 |
| `option_execution_reconciliations` | 1 |
| `fee_events` | 28 |

## Step 2 — Runtime configuration

Loaded from `.env` + gitignored `.env.cutover.v2d_s.local`, then exported
for this session:

```
OPTION_EVENT_INDEXER_ENABLED=true
OPTION_EVENT_INDEXER_REQUIRE_RPC=true
OPTION_EVENT_INDEXER_CONFIRMATION_BLOCKS=3
OPTION_EVENT_INDEXER_BATCH_BLOCKS=10                 # see note below
OPTION_EVENT_INDEXER_POLL_INTERVAL_MS=600000         # 10-min poll to avoid background interference
OPTION_EVENT_INDEXER_MARGIN_ENGINE_ADDRESS=0x287Cef479be5889eEfCa847F9e73C860898f48Cc
OPTION_EVENT_INDEXER_MATCHING_ENGINE_ADDRESS=0xf2D1D85cD363Be3bc160d14883C80e7C2c4F420b
OPTION_EVENT_INDEXER_COLLATERAL_VAULT_ADDRESS=0x00340C360353a5AB784c5Bc5c44322A6AF0625D3
OPTION_EVENT_INDEXER_FEES_MANAGER_ADDRESS=0xaef73F10224712E1312963BE11662061481aA0F0
OPTION_CONFIRMATION_WORKER_ENABLED=false
OPTION_RECONCILIATION_WORKER_ENABLED=false
```

`BATCH_BLOCKS` was originally set to the task-specified `5000` but the
first tick failed immediately with:

> `Under the Free tier plan, you can make eth_getLogs requests with up
> to a 10 block range. Based on your parameters, this block range
> should work: [0x27eafec, 0x27eaff5]. Upgrade to PAYG for expanded
> block range.`

`BATCH_BLOCKS=10` was used for the rest of the run because that is the
RPC's hard cap for the current key.

`/admin/config` after restart confirmed:

| Field | Value |
| --- | --- |
| `options.event_indexer.enabled` | `true` |
| `options.event_indexer.batch_blocks` | `10` |
| `options.event_indexer.confirmation_blocks` | `3` |
| `options.event_indexer.require_rpc` | `true` |
| `options.event_indexer.rpc_configured` | `true` |
| `options.event_indexer.margin_engine_address` | `0x287Cef479be5889eEfCa847F9e73C860898f48Cc` (NEW) |
| `options.event_indexer.matching_engine_address` | `0xf2D1D85cD363Be3bc160d14883C80e7C2c4F420b` (unchanged) |
| `options.event_indexer.collateral_vault_address` | `0x00340C360353a5AB784c5Bc5c44322A6AF0625D3` (unchanged) |
| `options.event_indexer.fees_manager_address` | `0xaef73F10224712E1312963BE11662061481aA0F0` (V1) |
| `options.event_indexer.fees_manager_v2_address` | `null` |
| `options.event_indexer.emitter_contracts[]` | matching_engine + margin_engine (NEW) + collateral_vault + fees_manager |
| `options.confirmation_worker.enabled` | `false` |
| `options.reconciliation_worker.enabled` | `false` |

No secrets present in the response.

## Step 3 — Catch-up loop (60 ticks)

`POST /admin/options/events/tick` × 60.

| Tick range | Result |
| --- | --- |
| 1–8 | success, cursor advances by 10 blocks each (41857013 → 41857023 → … → 41857113); `logs_found = 0`, `events_indexed = 0` on every tick. |
| 9–60 | failed; alternating `indexer failed: Your app has exceeded its compute units per second capacity. If you have retries enabled, you can safely ignore this message` and `simulation failed: error decoding response body` (Alchemy CU/s rate-limit + transient JSON-RPC payload errors under throttling). |

Sample successful tick (tick 1):

```
{"enabled":true,"chain_id":84532,
 "current_block_number":42096661,"safe_head":42096658,
 "from_block":41857034,"to_block":41857043,
 "batch_blocks":10,"confirmation_blocks":3,
 "logs_found":0,"events_decoded":0,"events_indexed":0,
 "cursor_updated":true,"last_indexed_block":41857043}
```

Aggregate:

| | Value |
| --- | --- |
| Tick count | 60 |
| Successful ticks | 8 (ticks 1–8) |
| Failed ticks | 52 (ticks 9–60, RPC throttling) |
| Logs found | 0 |
| Events decoded | 0 |
| Events indexed | 0 |
| Baseline cursor | 41857003 |
| Final cursor (admin + DB) | **41857113** |
| Target cursor | 42073775 |
| Blocks remaining | 216,662 |
| `last_error` (persisted state) | `null` (only successful ticks update the state row; failed ticks log but do not overwrite) |

## Step 4 — Verified event state

`/admin/options/events` after the loop:

- `last_indexed_block` = `41857113`
- `last_error` = `null`
- `emitter_contracts[1]` = `{role: margin_engine, contract_address: 0x287cef…f48cc}` (NEW)
- `counts_by_event_name`: `TradingFeeCharged=2`, `Synced=12`, `OptionTradeExecuted=1`, `TradeExecuted=1`, `InternalTransfer=3`; all V2 counters (`FeeChargedV2`, `FeeRebatedV2`, `RebateBudget*`, `*V2`) report 0. Historical V1S events still visible.
- `counts_by_contract_address`: OLD margin engine `0x6c5665de…b5f8` rows still queryable (3 events), CollateralVault 15, OptionMatchingEngine 1.

DB:

```
select last_indexed_block, last_error, updated_at_ms
from option_event_indexer_state
order by updated_at_ms desc limit 5;
-- 41857113 | (null) | 1779961946682
```

```
select event_name, count(*) from option_execution_events group by event_name;
-- InternalTransfer 3
-- OptionTradeExecuted 1
-- Synced 12
-- TradeExecuted 1
-- TradingFeeCharged 2
```

## Step 5 — No-mutation verification

| Table | Baseline | After loop | Δ |
| --- | --- | --- | --- |
| `option_execution_intents` | 3 | 3 | 0 |
| `option_execution_transactions` | 2 | 2 | 0 |
| `execution_transactions` | 1 | 1 | 0 |
| `option_execution_events` | 19 | 19 | 0 |
| `option_execution_reconciliations` | 1 | 1 | 0 |
| `fee_events` | 28 | 28 | 0 |

`option_event_indexer_state` is the only mutation: the cursor advanced
from 41857003 to 41857113 across 8 successful ticks. Per the task spec
("`option_execution_events` may increase only from legitimate indexed
logs; cursor advance is acceptable"), this is allowed.

## Step 6 — Indexer disabled, backend stopped

The session backend was killed. The next operator-run should start the
backend with `OPTION_EVENT_INDEXER_ENABLED=false` (default in
`.env.cutover.v2d_s.local`) until a usable RPC is configured. No new
runtime processes left behind.

## Remaining Blocker Before Tiny Test Trade

1. **RPC tier is the hard blocker.** The current Alchemy key for
   `base-sepolia.g.alchemy.com` is on free tier:
   - `eth_getLogs` capped at a **10-block range** per call.
   - Compute units / second rate-limit; the indexer's own batching
     plus the simulator's other `eth_*` calls (e.g. `eth_call`,
     `eth_blockNumber`) collide on this limit so even at
     `BATCH_BLOCKS=10` we got throttled after ~8 calls.

   To catch up ~216,662 blocks we need either:
   - **Upgrade Alchemy to PAYG / Growth** (recommended; lifts both
     limits in one step), or
   - **Switch `RPC_URL`** to a provider with a larger
     `eth_getLogs` window (QuickNode, BlockPi, self-hosted Geth, etc.).

   No new code is needed — `OPTION_EVENT_INDEXER_BATCH_BLOCKS` is a
   single env var.
2. **Cursor state is persisted**, so the catch-up is resumable. After
   the RPC upgrade, the next operator can simply restart with
   `OPTION_EVENT_INDEXER_ENABLED=true` + a larger `BATCH_BLOCKS`
   (e.g. `5000`) and either wait for the background poll loop or hit
   `/admin/options/events/tick` until `last_indexed_block ≥ 42073775`.
3. **Other V2D-S deferred items remain unchanged**: broadcast,
   simulation-preflight against NEW, mutation workers re-enable, key
   provisioning, and FeesManagerV2 enablement are all still
   deferred. See `MARGIN_ENGINE_V2_BACKEND_CUTOVER_V2D_S.md`
   §"Remaining Blocker".

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
| `cargo fmt --all` | clean (no code changes; V2D-T touches docs only) |
| `cargo clippy --all-targets --all-features -- -D warnings` | clean |
| `cargo test --all-targets --all-features --no-fail-fast` | **passed: 601, failed: 0, ignored: 0** |
| `cargo build --all-targets --all-features` | clean |

V2D-T introduces no code changes; only docs and the
`.env.cutover.v2d_s.local` runtime overrides file (gitignored). The
clean test+clippy+build run is a regression check after the env-file
edit and confirms the existing backend continues to compile and pass.
