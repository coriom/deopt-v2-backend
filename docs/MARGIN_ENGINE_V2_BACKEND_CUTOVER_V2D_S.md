# Backend Cutover To NEW MarginEngine — V2D-S

Date: 2026-05-27
Network: Base Sepolia (chain id 84532)
Mode: Read-only / admin verification. No broadcast. No transaction submission.
No Solidity, frontend, or `.env` template changes. No FeesManagerV2 enable.

## Context

V2D-R completed the on-chain rewire:

- OLD_MARGIN_ENGINE          `0x6C5665De05e7314cB63cD77F82DFa86508A5b5F8`
- NEW_MARGIN_ENGINE          `0x287Cef479be5889eEfCa847F9e73C860898f48Cc`
- CUTOVER_BLOCK              `42073772`
- NEW.feesManager            `0xaef73F10224712E1312963BE11662061481aA0F0` (V1)
- NEW.feesManagerV2          `0x0000000000000000000000000000000000000000`
- NEW.useFeesManagerV2       `false`

V2D-S cuts the backend runtime over to read NEW as the current MarginEngine
while preserving historical OLD-emitter visibility for V1S and earlier
lifecycle queries.

## Hard Rules Verification

| Rule | Status |
| --- | --- |
| Do not broadcast | ✅ no broadcast invoked |
| Do not submit transactions | ✅ no `eth_sendRawTransaction` |
| Do not deploy | ✅ no deploy script touched |
| Do not modify Solidity | ✅ `../deopt-v2-sol/` untouched |
| Do not modify frontend | ✅ no frontend changes |
| Do not enable FeesManagerV2 | ✅ NEW still reports `useFeesManagerV2 = false` |
| Do not deploy FeesManagerV2 | ✅ no deploy |
| Do not call `setUseFeesManagerV2` | ✅ no admin write |
| Do not create option execution intents | ✅ counts unchanged |
| Do not create option execution transactions | ✅ counts unchanged |
| Do not create generic execution transactions | ✅ counts unchanged |
| Do not cleanup historical evidence rows | ✅ no DELETE/UPDATE |
| Do not print private keys | ✅ no secret printed |
| Do not commit real `.env` | ✅ overrides written to gitignored `.env.cutover.v2d_s.local` |

## Step 1 — Baseline

`V2D_S_START_MS = 1779916980716`

DB row counts at the start (mutation-sensitive tables):

| Table | Count |
| --- | --- |
| `option_execution_intents` | 3 |
| `option_execution_transactions` | 2 |
| `execution_transactions` | 1 |
| `option_execution_events` | 19 |
| `option_execution_reconciliations` | 1 |
| `fee_events` | 28 |

## Step 2 — Runtime env cutover

Stored in `./.env.cutover.v2d_s.local` (gitignored via `.env.*`). Loaded via
`set -a && . ./.env && . ./.env.cutover.v2d_s.local && set +a`. No secrets
appear in this file.

```
MARGIN_ENGINE=0x287Cef479be5889eEfCa847F9e73C860898f48Cc
OPTION_EVENT_INDEXER_MARGIN_ENGINE_ADDRESS=0x287Cef479be5889eEfCa847F9e73C860898f48Cc
OLD_MARGIN_ENGINE=0x6C5665De05e7314cB63cD77F82DFa86508A5b5F8
MARGIN_ENGINE_CUTOVER_BLOCK=42073772

OPTION_CONFIRMATION_WORKER_ENABLED=false
OPTION_EVENT_INDEXER_ENABLED=false
OPTION_RECONCILIATION_WORKER_ENABLED=false

ADMIN_API_ENABLED=true
ADMIN_API_REQUIRE_TOKEN=true
ADMIN_API_TOKEN=<local-only token, not committed>
```

`OLD_MARGIN_ENGINE` and `MARGIN_ENGINE_CUTOVER_BLOCK` are documentation only —
no backend code reads these keys today. `MARGIN_ENGINE` is consumed by
`src/config/env.rs:367` as the fallback for
`OPTION_EVENT_INDEXER_MARGIN_ENGINE_ADDRESS`.

## Step 3 — Backend restart with workers disabled

Bound binary: `target/debug/deopt-v2-backend` (existing build).
Startup log confirmed:

```
option confirmation worker disabled
option reconciliation worker disabled
option event indexer disabled
option_confirmation_worker_enabled=false
option_event_indexer_enabled=false
option_reconciliation_worker_enabled=false
```

Health:

```
GET /health -> {"ok":true,"service":"deopt-v2-backend"}
```

## Step 4 — `/admin/config`

Authenticated with `X-Admin-Token`. Relevant fields:

| Field | Value |
| --- | --- |
| `admin.enabled` | `true` |
| `admin.require_token` | `true` |
| `admin.token_configured` | `true` |
| `chain_id` | `84532` |
| `network` | `base-sepolia` |
| `options.event_indexer.margin_engine_address` | `0x287Cef479be5889eEfCa847F9e73C860898f48Cc` (NEW) |
| `options.event_indexer.fees_manager_address` | `0xaef73F10224712E1312963BE11662061481aA0F0` (V1) |
| `options.event_indexer.fees_manager_v2_address` | `null` |
| `options.event_indexer.collateral_vault_address` | `0x00340C360353a5AB784c5Bc5c44322A6AF0625D3` |
| `options.event_indexer.emitter_contracts[0]` | `{ role: margin_engine, contract_address: 0x287cef…f48cc }` (NEW) |
| `options.event_indexer.enabled` | `false` (disabled mode) |
| `options.confirmation_worker.enabled` | `false` |
| `options.reconciliation_worker.enabled` | `false` |
| `features.real_broadcast_enabled` | `false` |
| `features.option_execution_broadcast_enabled` | `false` |
| `configured.executor_private_key` | `false` |

Backend reports **NEW** as the current margin engine.
FeesManagerV2 remains absent (`null`) and disabled.
No secret values are present in the response.

## Step 5 — Read-only on-chain checks on NEW

```
cast call 0x287Cef479be5889eEfCa847F9e73C860898f48Cc "feesManager()(address)"
  -> 0xaef73F10224712E1312963BE11662061481aA0F0
cast call 0x287Cef479be5889eEfCa847F9e73C860898f48Cc "feesManagerV2()(address)"
  -> 0x0000000000000000000000000000000000000000
cast call 0x287Cef479be5889eEfCa847F9e73C860898f48Cc "useFeesManagerV2()(bool)"
  -> false
```

All three match the expected V2-disabled state.

## Step 6 — V1S lifecycle still queryable

`GET /admin/options/executions/e6d2941b-65f7-413a-958f-74ab22c53b08/lifecycle`

| Field | Value |
| --- | --- |
| `status` | `broadcast_confirmed` |
| `broadcast.tx_hash` | `0x5964a7b3d2c18d051baaa780413d31c44d419ce530f45263cb4c46f720881125` |
| `health.stage` | `reconciled` |
| `events.total` | `19` |
| `reconciliation.status` | `reconciled` |
| `state_checks` | present |
| `fees.event_model` | `v1` |
| `fees.source_priority` | `""` |
| `events.counts_by_contract_address` | `{ 0x00340c…25d3: 15, 0x6c5665de…b5f8: 3, 0xf2d1d85c…420b: 1 }` |

Historical OLD-margin-engine event rows (`0x6c5665de…b5f8`) remain visible
and contribute to the reconciled V1S view. Lifecycle does **not** break
after the cutover.

## Step 7 — Admin events / fees

`GET /admin/options/events`

- `indexer_enabled = false`
- `last_indexed_block = 41856978` (V1S-era cursor, untouched by cutover)
- `last_error = null`
- `counts_by_event_name` includes `TradingFeeCharged=2`, `Synced=12`,
  `OptionTradeExecuted=1`, `TradeExecuted=1`, `InternalTransfer=3`; all
  V2 event counters (`FeeChargedV2`, `FeeRebatedV2`,
  `RebateBudget*`, `FeeRecipientSetV2`, etc.) report 0.
- `counts_by_contract_address` still includes the OLD-emitter
  `0x6c5665de…b5f8` rows.
- `emitter_contracts` reports NEW margin engine + collateral vault +
  V1 fees manager.
- `config.margin_engine_address` = NEW.
- `config.fees_manager_v2_address` = `null`.
- `recent` returns 19 historical events.

`GET /admin/fees/onchain` (unfiltered):

| Field | Value |
| --- | --- |
| `source_of_truth` | `onchain` |
| `event_model` | `v1` |
| `source_priority` | `""` |
| `trading_fee_event_count` | `2` |
| `fee_charged_v2_count` | `0` |
| `fee_rebated_v2_count` | `0` |
| `observed_total` | `10` |
| `observed_total_charged` | `10` |
| `observed_total_rebated` | `0` |
| `net_protocol_fee` | `10` |
| `reconciliation_status` | `onchain_observed` |
| `backend_ledger_enabled` | `false` |
| `backend_ledger_status` | `disabled` |
| `transactions[]` | 1 entry (V1S) |
| `events[]` | 2 entries |

Both admin endpoints respond cleanly and surface the historical V1S
state without crash, despite the emitter address change.

## Step 8 — Optional bounded indexer tick

Backend restarted with:

```
OPTION_EVENT_INDEXER_ENABLED=true
OPTION_EVENT_INDEXER_FROM_BLOCK=42073772
OPTION_EVENT_INDEXER_BATCH_BLOCKS=5
OPTION_EVENT_INDEXER_CONFIRMATION_BLOCKS=3
OPTION_EVENT_INDEXER_REQUIRE_RPC=true
OPTION_EVENT_INDEXER_MATCHING_ENGINE_ADDRESS=0xf2D1D85cD363Be3bc160d14883C80e7C2c4F420b
```

(The matching-engine env was added because the runtime requires it when
the indexer is enabled. The option matching engine address itself is
unchanged from V1S — only the MarginEngine emitter was swapped.)

`POST /admin/options/events/tick` →

```
{
  "enabled": true,
  "chain_id": 84532,
  "current_block_number": 42074509,
  "safe_head": 42074506,
  "from_block": 41856989,
  "to_block": 41856993,
  "batch_blocks": 5,
  "confirmation_blocks": 3,
  "logs_found": 0,
  "events_decoded": 0,
  "events_indexed": 0,
  "cursor_updated": true,
  "last_indexed_block": 41856993
}
```

The tick reused the existing indexer cursor (last_indexed_block was
41856978; the tick advanced it by 5 to 41856993). `OPTION_EVENT_INDEXER_FROM_BLOCK`
only applies when there is no cursor state; with the V1S cursor already
present, the indexer correctly continued from there. `logs_found = 0`
in this small window. No errors. No new option_execution_*
or execution_transactions rows were created (Step 9 below).

A full catch-up from cursor 41856993 to CUTOVER_BLOCK 42073772 spans
~216,779 blocks — not attempted in this read-only step; that is a
separate operator decision (either a tuned-batch worker run or a
one-shot bounded loop) and is out of scope for V2D-S.

## Step 9 — No-mutation proof

After Step 8, DB row counts re-snapshot:

| Table | Baseline | After tick | Δ |
| --- | --- | --- | --- |
| `option_execution_intents` | 3 | 3 | 0 |
| `option_execution_transactions` | 2 | 2 | 0 |
| `execution_transactions` | 1 | 1 | 0 |
| `option_execution_events` | 19 | 19 | 0 |
| `option_execution_reconciliations` | 1 | 1 | 0 |
| `fee_events` | 28 | 28 | 0 |

All counts unchanged. No new option execution intent, transaction, or
generic execution transaction created. No fee event written. The
indexer cursor advanced (this is acceptable per task spec — the
`option_event_indexer_state` row is the only state touched and the
tick run was operator-authorized).

## V2D-T Catch-up Update (2026-05-28)

The follow-up catch-up run is documented in
[`OPTION_EVENT_INDEXER_CATCHUP_CUTOVER_V2D_T.md`](OPTION_EVENT_INDEXER_CATCHUP_CUTOVER_V2D_T.md).
Headline: catch-up **did not reach** `CUTOVER_BLOCK + confirmations =
42073775`. After 60 manual ticks the cursor advanced from `41857003`
to `41857113` (110 blocks); the remaining 216,662 blocks were blocked
by the Alchemy free tier (10-block `eth_getLogs` cap +
compute-unit/sec rate limit). No mutation in
`option_execution_intents`, `option_execution_transactions`, or
`execution_transactions`.

## V2D-T2 Catch-up Resume (2026-05-28, same day)

The operator supplied a higher-tier Alchemy RPC endpoint (paid
PAYG/Growth). The catch-up was resumed and completed; full record in
[`OPTION_EVENT_INDEXER_CATCHUP_CUTOVER_V2D_T2.md`](OPTION_EVENT_INDEXER_CATCHUP_CUTOVER_V2D_T2.md).
Headline: cursor advanced from `41857113` to `42077113` in **42 manual
ticks at `BATCH_BLOCKS=5000`** (plus one background poll on startup),
finishing 3,338 blocks past the target `42073775`. Zero logs found in
the post-V1S idle range, zero mutation in
`option_execution_intents` / `option_execution_transactions` /
`execution_transactions`. RPC URL stayed runtime-only — no URL or key
is recorded in this repo. The indexer-side prerequisite for the tiny
test trade is now satisfied; the remaining items below (broadcast
flips, signing key, simulation preflight, mutation worker re-enable)
are unchanged.

## Remaining Blocker Before Tiny Test Trade

The backend is cleanly cut over to NEW for read-only and admin
purposes. Before broadcasting a tiny on-chain test trade, the
following still need a deliberate, separate decision/action:

1. **Confirm OptionMatchingEngine wiring**: V1S used
   OptionMatchingEngine `0xf2D1D85cD363Be3bc160d14883C80e7C2c4F420b`,
   which now points at NEW MarginEngine per V2D-R. A simulation-only
   preflight against NEW should be run before broadcast (the existing
   broadcast preflight path returns `simulation_ok`/`simulation_revert`
   from `/admin/options/executions/<id>/simulate`).
2. **Enable broadcast**: `EXECUTOR_REAL_BROADCAST_ENABLED=false` and
   `OPTION_EXECUTION_BROADCAST_ENABLED=false` today. Both are off and
   no `EXECUTOR_PRIVATE_KEY` is configured. The tiny trade requires
   both flips + a signing key. This is intentionally a separate task.
3. **Re-enable mutation workers**: `OPTION_CONFIRMATION_WORKER_ENABLED`,
   `OPTION_EVENT_INDEXER_ENABLED`, `OPTION_RECONCILIATION_WORKER_ENABLED`
   are all `false` in the V2D-S overrides. They should be re-enabled
   in the run that will host the test trade, and the event indexer
   should be allowed to catch up from `last_indexed_block` to a
   block ≥ `CUTOVER_BLOCK + confirmations` before the trade is created.
4. **FeesManagerV2 remains disabled**: trades after cutover charge V1
   fees only (`useFeesManagerV2=false`, `feesManagerV2=0`). This is
   the intended V2D-S state. FeesManagerV2 deploy + enable is a
   later V2E/V2F task, explicitly out of scope here.
5. **`MARGIN_ENGINE_CUTOVER_BLOCK` is purely documentation**: no
   backend code currently consumes it. If the event indexer needs a
   strict "OLD emitter is dead after this block" check, that is a
   small follow-up (likely a per-emitter `until_block` field on
   `OptionEventIndexerConfig`); not blocking V2D-S sign-off.

## Validation Commands

```
cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
cargo build --all-targets --all-features
```

(See "Validation results" below.)

## Validation Results

| Command | Result |
| --- | --- |
| `cargo fmt --all` | clean (no diff) |
| `cargo clippy --all-targets --all-features -- -D warnings` | clean — `Finished dev profile`, no warnings |
| `cargo test --all-targets --all-features --no-fail-fast` | **passed: 601, failed: 0, ignored: 0** across 12 test binaries (lib + main + 5 sign-bin unittests + engine_tests + mm_gateway_tests + options_tests + orderbook_tests + rfq_tests) |
| `cargo build --all-targets --all-features` | clean — `Finished dev profile`, no warnings |

No code changes were made in V2D-S; only docs and a gitignored runtime
env overrides file were added. The clean test+clippy+build run confirms
the existing backend continues to compile and pass after the cutover
verification work.
