# Option Event Indexer Catch-up Resume — V2D-T2

Date: 2026-05-28
Network: Base Sepolia (chain id 84532)
Mode: Read-only / admin verification. No broadcast. No transaction submission.

## Outcome

**Catch-up complete.** With a higher-tier RPC endpoint, the option event
indexer advanced from cursor `41857113` to `42077113` in **42 manual
ticks at `BATCH_BLOCKS=5000`** (plus one background poll on startup),
crossing the target threshold `CUTOVER_BLOCK + confirmation_blocks =
42073775` with 3,338 blocks of headroom. Zero new logs were found in
the post-V1S idle range; zero mutation in
`option_execution_intents` / `option_execution_transactions` /
`execution_transactions` / `option_execution_events` /
`option_execution_reconciliations` / `fee_events`.

## RPC Provider

Provider type used: **Alchemy paid tier (PAYG/Growth)** for
`base-sepolia.g.alchemy.com`. The URL was passed into the runtime
shell by the operator and is not recorded in this repo. No URL or key
is printed in this doc, in commit history, or in any other file.

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
| Do not create option execution intents | ✅ count unchanged (3) |
| Do not create option execution transactions | ✅ count unchanged (2) |
| Do not create generic execution transactions | ✅ count unchanged (1) |
| Do not cleanup historical evidence rows | ✅ no DELETE/UPDATE |
| Do not print private keys | ✅ no secret printed |
| Do not commit real `.env` | ✅ overrides stay in gitignored `.env.cutover.v2d_s.local`; the operator-supplied RPC URL was exported into the runtime shell only and not written to any file |
| Do not paste RPC secret into docs | ✅ no URL/key present in any doc |

## Step 1 — Operator-supplied RPC

The operator exported `RPC_URL` directly into the runtime shell of the
session that launched the backend; the value never landed in
`.env`, `.env.cutover.v2d_s.local`, the conversation transcript, or
any committed artifact. `test -n "$RPC_URL" && echo "RPC_URL set"`
returned `RPC_URL set`.

## Step 2 — Baseline

`V2D_T2_START_MS = 1779963271200`

`option_event_indexer_state` (newest first):

| `last_indexed_block` | `last_error` | `updated_at_ms` |
| --- | --- | --- |
| `41857113` | `null` | `1779961946682` |

Mutation-sensitive table baselines:

| Table | Count |
| --- | --- |
| `option_execution_intents` | 3 |
| `option_execution_transactions` | 2 |
| `execution_transactions` | 1 |
| `option_execution_events` | 19 |
| `option_execution_reconciliations` | 1 |
| `fee_events` | 28 |

## Step 3 — Runtime config

Loaded `.env` + `.env.cutover.v2d_s.local` and overrode `RPC_URL` with
the operator's higher-tier endpoint. Then exported the catch-up
config:

```
OPTION_EVENT_INDEXER_ENABLED=true
OPTION_EVENT_INDEXER_REQUIRE_RPC=true
OPTION_EVENT_INDEXER_CONFIRMATION_BLOCKS=3
OPTION_EVENT_INDEXER_BATCH_BLOCKS=5000
OPTION_EVENT_INDEXER_POLL_INTERVAL_MS=600000           # 10 min, keeps background loop quiet
OPTION_EVENT_INDEXER_MARGIN_ENGINE_ADDRESS=0x287Cef479be5889eEfCa847F9e73C860898f48Cc
OPTION_EVENT_INDEXER_MATCHING_ENGINE_ADDRESS=0xf2D1D85cD363Be3bc160d14883C80e7C2c4F420b
OPTION_EVENT_INDEXER_COLLATERAL_VAULT_ADDRESS=0x00340C360353a5AB784c5Bc5c44322A6AF0625D3
OPTION_EVENT_INDEXER_FEES_MANAGER_ADDRESS=0xaef73F10224712E1312963BE11662061481aA0F0
OPTION_CONFIRMATION_WORKER_ENABLED=false
OPTION_RECONCILIATION_WORKER_ENABLED=false
```

`MARGIN_ENGINE`, `OLD_MARGIN_ENGINE`, and
`MARGIN_ENGINE_CUTOVER_BLOCK` come from `.env.cutover.v2d_s.local`
(unchanged from V2D-S).

`/admin/config` after restart:

| Field | Value |
| --- | --- |
| `options.event_indexer.enabled` | `true` |
| `options.event_indexer.batch_blocks` | `5000` |
| `options.event_indexer.confirmation_blocks` | `3` |
| `options.event_indexer.require_rpc` | `true` |
| `options.event_indexer.rpc_configured` | `true` |
| `options.event_indexer.margin_engine_address` | `0x287Cef479be5889eEfCa847F9e73C860898f48Cc` (NEW) |
| `options.event_indexer.matching_engine_address` | `0xf2D1D85cD363Be3bc160d14883C80e7C2c4F420b` |
| `options.event_indexer.collateral_vault_address` | `0x00340C360353a5AB784c5Bc5c44322A6AF0625D3` |
| `options.event_indexer.fees_manager_address` | `0xaef73F10224712E1312963BE11662061481aA0F0` (V1) |
| `options.event_indexer.fees_manager_v2_address` | `null` |
| `options.confirmation_worker.enabled` | `false` |
| `options.reconciliation_worker.enabled` | `false` |

No secrets, no URL appear in the admin response.

## Step 4 — Tier probe

The first manual tick used the task-specified `BATCH_BLOCKS=5000`. It
succeeded immediately:

```
{"from_block":41862114,"to_block":41867113,"batch_blocks":5000,
 "logs_found":0,"events_decoded":0,"events_indexed":0,
 "cursor_updated":true,"last_indexed_block":41867113}
```

The new tier accepts the 5000-block `eth_getLogs` window cleanly —
no compute-unit/sec throttling for this batch size.

(Note: the indexer's background poll fired once on startup before the
first admin tick, advancing the cursor from `41857113` to
`41862113`. With `OPTION_EVENT_INDEXER_POLL_INTERVAL_MS=600000` the
loop then waited ~10 min between further ticks, leaving the manual
admin path as the primary driver.)

## Step 5 — Catch-up loop

`POST /admin/options/events/tick` × N until `last_indexed_block ≥ 42073775`.

| | Value |
| --- | --- |
| Tick count (admin/tick endpoint) | **42** |
| Successful ticks | 42 / 42 |
| Failed ticks | 0 |
| Batch size | 5000 blocks/tick |
| Logs found across all ticks | **0** |
| Events decoded | 0 |
| Events indexed | 0 |
| Baseline cursor | 41857113 |
| Final cursor (admin + DB) | **42077113** |
| Target cursor (≥) | 42073775 |
| Headroom past target | +3338 blocks |
| `last_error` (persisted state) | `null` |

Sample early tick:

```
tick 1:  from=41867114 to=41872113  cursor=41872113  logs=0
…
tick 41: from=42067114 to=42072113  cursor=42072113  logs=0
tick 42: from=42072114 to=42077113  cursor=42077113  logs=0
REACHED target=42073775 cursor=42077113 after 42 ticks
```

## Step 6 — Verified event state

`/admin/options/events` after the loop:

- `last_indexed_block = 42077113`
- `last_error = null`
- `emitter_contracts[1] = {role: margin_engine, contract_address: 0x287cef…f48cc}` (NEW)
- `counts_by_event_name`: `TradingFeeCharged=2`, `Synced=12`,
  `OptionTradeExecuted=1`, `TradeExecuted=1`, `InternalTransfer=3`;
  all V2 counters (`FeeChargedV2`, `FeeRebatedV2`, `RebateBudget*`,
  `*V2`) remain `0`.
- `counts_by_contract_address`: OLD margin engine `0x6c5665de…b5f8`
  rows still queryable (3 events), CollateralVault 15,
  OptionMatchingEngine 1.

DB:

```
select last_indexed_block, last_error from option_event_indexer_state
order by updated_at_ms desc limit 5;
-- 42077113 | (null)
```

V1S lifecycle still reconciled:

| Field | Value |
| --- | --- |
| `status` | `broadcast_confirmed` |
| `broadcast.tx_hash` | `0x5964a7b3…1125` |
| `reconciliation.status` | `reconciled` |
| `events.total` | `19` |
| `counts_by_contract_address` | `{0x00340c…25d3: 15, 0x6c5665de…b5f8: 3, 0xf2d1d85c…420b: 1}` |

The cutover did not invalidate any historical V1S data.

## Step 7 — No-mutation verification

| Table | Baseline | After | Δ |
| --- | --- | --- | --- |
| `option_execution_intents` | 3 | 3 | 0 |
| `option_execution_transactions` | 2 | 2 | 0 |
| `execution_transactions` | 1 | 1 | 0 |
| `option_execution_events` | 19 | 19 | 0 |
| `option_execution_reconciliations` | 1 | 1 | 0 |
| `fee_events` | 28 | 28 | 0 |

`option_event_indexer_state.last_indexed_block` advanced from
`41857113` → `42077113` (allowed; task-permitted cursor mutation).

The +220,000-block range covered by the loop contained **no logs from
any subscribed emitter** (matching engine, NEW margin engine, OLD
margin engine, collateral vault, V1 fees manager). The chain was
quiet for option/perp/option-fee activity through the cutover window
on Base Sepolia — expected since no trader on V1 was racing the
cutover, and FeesManagerV2 is intentionally still disabled.

## Step 8 — Backend stopped (safe mode)

The session backend was killed. Port 8080 is free; no indexer
processes remain. The next operator should restart with
`OPTION_EVENT_INDEXER_ENABLED=false` (the default in
`.env.cutover.v2d_s.local`) until a planned trade or live observation
window opens — at which point the indexer can be re-enabled with the
same paid `RPC_URL` and any `BATCH_BLOCKS` setting up to the tier's
limit.

## Remaining Blocker Before Tiny Test Trade

The indexer-side prerequisite for the tiny test trade is now
satisfied: cursor is past `CUTOVER_BLOCK + 3`. The remaining items
(all unchanged from V2D-S §"Remaining Blocker") are deliberate
operator decisions, not infrastructure blockers:

1. **Re-enable mutation workers** for the trade run:
   `OPTION_EVENT_INDEXER_ENABLED=true`,
   `OPTION_CONFIRMATION_WORKER_ENABLED=true`,
   `OPTION_RECONCILIATION_WORKER_ENABLED=true`.
2. **Flip broadcast surfaces**:
   `EXECUTOR_REAL_BROADCAST_ENABLED=true`,
   `OPTION_EXECUTION_BROADCAST_ENABLED=true`, and provision
   `EXECUTOR_PRIVATE_KEY` (off today by design).
3. **Run simulation preflight against NEW** via
   `POST /admin/options/executions/<id>/simulate` — OptionMatchingEngine
   address is unchanged but its `marginEngine` now resolves to NEW.
4. **FeesManagerV2 stays disabled** intentionally (V2E/V2F scope);
   the tiny test trade charges V1 fees only.
5. **Keep the paid `RPC_URL`** for the test-trade session — the
   broadcast preflight and confirmation worker also rely on
   simulation calls under the same compute-unit budget.

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
| `cargo fmt --all` | clean (no code changes; V2D-T2 touches docs only) |
| `cargo clippy --all-targets --all-features -- -D warnings` | clean |
| `cargo test --all-targets --all-features --no-fail-fast` | **passed: 601, failed: 0, ignored: 0** |
| `cargo build --all-targets --all-features` | clean |
