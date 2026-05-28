# FeesManagerV2 Backend Indexer Cutover V2E-D

Date: 2026-05-28
Network: Base Sepolia (`chain_id 84532`)
Status: `backend_indexer_cutover_ready`

## 0. Scope

V2E-D cuts backend runtime and option event indexer configuration over to
include the wired FeesManagerV2 contract while keeping the NEW MarginEngine
V2 fee path disabled.

No broadcast was performed. No transaction was submitted. No deploy occurred.
No Solidity or frontend code was modified. FeesManagerV2 was not enabled,
`setUseFeesManagerV2` was not called, rebate budget was not funded, Merkle
root was not set, no option execution intents or transactions were created,
and no option broadcast endpoints were called.

## 1. Runtime Env Summary

Backend env was loaded without printing secrets. Runtime overrides:

```text
MARGIN_ENGINE=0x287Cef479be5889eEfCa847F9e73C860898f48Cc
OPTION_EVENT_INDEXER_MARGIN_ENGINE_ADDRESS=0x287Cef479be5889eEfCa847F9e73C860898f48Cc
FEES_MANAGER_V2=0x00dA0B9876bcBf0c79CB5BcAcfEBAFb8C7Ad774f
OPTION_EVENT_INDEXER_FEES_MANAGER_V2_ADDRESS=0x00dA0B9876bcBf0c79CB5BcAcfEBAFb8C7Ad774f
OPTION_EVENT_INDEXER_MATCHING_ENGINE_ADDRESS=0xf2D1D85cD363Be3bc160d14883C80e7C2c4F420b
OPTION_EVENT_INDEXER_COLLATERAL_VAULT_ADDRESS=0x00340C360353a5AB784c5Bc5c44322A6AF0625D3
OPTION_EVENT_INDEXER_FEES_MANAGER_ADDRESS=0xaef73F10224712E1312963BE11662061481aA0F0
OPTION_EVENT_INDEXER_ENABLED=true
OPTION_EVENT_INDEXER_REQUIRE_RPC=true
OPTION_EVENT_INDEXER_BATCH_BLOCKS=5000
OPTION_EVENT_INDEXER_CONFIRMATION_BLOCKS=3
OPTION_EVENT_INDEXER_POLL_INTERVAL_MS=3600000
OPTION_CONFIRMATION_WORKER_ENABLED=false
OPTION_RECONCILIATION_WORKER_ENABLED=false
OPTION_EXECUTION_BROADCAST_ENABLED=false
EXECUTOR_REAL_BROADCAST_ENABLED=false
EXECUTOR_DRY_RUN=true
ADMIN_API_ENABLED=true
ADMIN_API_REQUIRE_TOKEN=true
ADMIN_API_TOKEN=<local runtime token, not printed>
```

Backend startup confirmed:

| Field | Observed |
| --- | --- |
| HTTP bind | `127.0.0.1:8080` |
| `chain_id` | `84532` |
| `network` | `base-sepolia` |
| `option_event_indexer_enabled` | `true` |
| `option_confirmation_worker_enabled` | `false` |
| `option_reconciliation_worker_enabled` | `false` |
| `execution_enabled` | `false` |
| `executor_dry_run` | `true` |

`GET /health` returned `{"ok":true,"service":"deopt-v2-backend"}`.

## 2. Admin Config

`GET /admin/config` returned:

| Field | Observed |
| --- | --- |
| `admin.enabled` | `true` |
| `admin.require_token` | `true` |
| `admin.token_configured` | `true` |
| `configured.rpc` | `true` |
| `configured.database` | `true` |
| `configured.executor_private_key` | `false` |
| `features.option_event_indexer_enabled` | `true` |
| `features.option_execution_broadcast_enabled` | `false` |
| `features.real_broadcast_enabled` | `false` |
| `options.execution_enabled` | `false` |
| `options.execution_broadcast_enabled` | `false` |
| `options.confirmation_worker.enabled` | `false` |
| `options.reconciliation_worker.enabled` | `false` |
| `options.event_indexer.margin_engine_address` | `0x287Cef479be5889eEfCa847F9e73C860898f48Cc` |
| `options.event_indexer.fees_manager_address` | `0xaef73F10224712E1312963BE11662061481aA0F0` |
| `options.event_indexer.fees_manager_v2_address` | `0x00dA0B9876bcBf0c79CB5BcAcfEBAFb8C7Ad774f` |
| `options.event_indexer.batch_blocks` | `5000` |
| `options.event_indexer.confirmation_blocks` | `3` |
| `options.event_indexer.poll_interval_ms` | `3600000` |

Emitter contracts reported by `/admin/config`:

| Role | Contract |
| --- | --- |
| `matching_engine` | `0xf2d1d85cd363be3bc160d14883c80e7c2c4f420b` |
| `margin_engine` | `0x287cef479be5889eefca847f9e73c860898f48cc` |
| `collateral_vault` | `0x00340c360353a5ab784c5bc5c44322a6af0625d3` |
| `fees_manager` | `0xaef73f10224712e1312963be11662061481aa0f0` |
| `fees_manager_v2` | `0x00da0b9876bcbf0c79cb5bcacfebafb8c7ad774f` |

## 3. On-Chain Read Checks

Read-only RPC calls confirmed:

| Read | Expected | Observed | Status |
| --- | --- | --- | --- |
| `NEW.feesManagerV2()` | `0x00dA0B9876bcBf0c79CB5BcAcfEBAFb8C7Ad774f` | `0x00dA0B9876bcBf0c79CB5BcAcfEBAFb8C7Ad774f` | PASS |
| `NEW.useFeesManagerV2()` | `false` | `false` | PASS |
| `FeesManagerV2.isFeeConsumer(NEW)` | `true` | `true` | PASS |
| `FeesManagerV2.merkleRoot()` | `bytes32(0)` | `0x0000000000000000000000000000000000000000000000000000000000000000` | PASS |
| `FeesManagerV2.rebateBudget(BASE_COLLATERAL_TOKEN)` | `0` | `0` | PASS |

## 4. Event Indexer Status

`GET /admin/options/events` confirmed:

| Field | Observed |
| --- | --- |
| `indexer_enabled` | `true` |
| `last_error` | `null` |
| `last_indexed_block` before manual tick | `42118157` |
| `last_indexed_block` after manual tick | `42118183` |
| `counts.FeeChargedV2` | `0` |
| `counts.FeeRebatedV2` | `0` |
| `counts.FeeConsumerSetV2` | `0` |
| `counts.MerkleRootSetV2` | `0` |
| `counts.RebateBudgetFunded` | `0` |

The startup indexer tick ran with the new emitter set and found no new logs.
The historical cursor was already past the V2E-C wire transaction blocks, so
the cutover did not backfill the wire events and did not reset historical
state.

One bounded manual tick was run through `POST /admin/options/events/tick`:

| Field | Observed |
| --- | --- |
| `enabled` | `true` |
| `chain_id` | `84532` |
| `current_block_number` | `42118186` |
| `safe_head` | `42118183` |
| `from_block` | `42118158` |
| `to_block` | `42118183` |
| `logs_found` | `0` |
| `events_decoded` | `0` |
| `events_indexed` | `0` |
| `cursor_updated` | `true` |
| `last_indexed_block` | `42118183` |

## 5. Admin Fees Result

`GET /admin/fees/onchain` responded successfully:

| Field | Observed |
| --- | --- |
| `event_model` | `v1` |
| `trading_fee_event_count` | `2` |
| `fee_charged_v2_count` | `0` |
| `fee_rebated_v2_count` | `0` |
| `observed_total` | `10` |

No V2 charged or rebated fee events are present.

## 6. No-Mutation Proof

Baseline DB counts before the cutover checks:

| Table | Count |
| --- | --- |
| `option_execution_intents` | `5` |
| `option_execution_transactions` | `3` |
| `execution_transactions` | `1` |
| `option_execution_events` | `26` |
| `option_execution_reconciliations` | `2` |
| `fee_events` | `28` |

Post-tick DB counts:

| Table | Count |
| --- | --- |
| `option_execution_intents` | `5` |
| `option_execution_transactions` | `3` |
| `execution_transactions` | `1` |
| `option_execution_events` | `26` |
| `option_execution_reconciliations` | `2` |
| `fee_events` | `28` |

Only `option_event_indexer_state.last_indexed_block` advanced to `42118183`.
No execution intent, option execution transaction, generic execution
transaction, fee event, reconciliation, or option event rows were created by
the cutover checks.

## 7. Validation

Run in `deopt-v2-backend`:

| Command | Result |
| --- | --- |
| `cargo fmt --all` | PASS |
| `cargo clippy --all-targets --all-features -- -D warnings` | PASS |
| `cargo test --all-targets --all-features` | PASS (`601` tests passed) |
| `cargo build --all-targets --all-features` | PASS |

## 8. Remaining Blocker Before Enable-Only Phase

Enable-only remains blocked on explicit human approval and a separate
enable-only preflight/broadcast flow. Required pre-enable checks must still
prove:

- backend runtime still points to NEW MarginEngine and FeesManagerV2;
- `NEW.feesManagerV2()` still equals
  `0x00dA0B9876bcBf0c79CB5BcAcfEBAFb8C7Ad774f`;
- `NEW.useFeesManagerV2()` is still `false` before enable;
- `FeesManagerV2.isFeeConsumer(NEW)` is still `true`;
- V2 rebate budget remains intentionally zero unless a separate approved
  funding phase is performed;
- `FeesManagerV2.merkleRoot()` remains `bytes32(0)` unless a separate approved
  Merkle-root phase is performed;
- the only allowed enable action is the human-run enable-only call.
