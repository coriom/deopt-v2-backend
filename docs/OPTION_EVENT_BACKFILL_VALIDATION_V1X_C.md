# Option Event Backfill Validation V1X-C

Date: 2026-05-23

## Scope

V1X-C is a controlled validation pass for the V1S live trade that exercises
the multi-emitter option event indexer (V1X + V1X-B) end-to-end without
broadcasting, retrying, signing, or creating execution intents or generic
execution transactions. The only allowed mutation is the indexer's own
ledger: idempotent inserts into `option_execution_events` and a cursor
write to `option_event_indexer_state`.

## V1S Tx Under Validation

| Field | Value |
| --- | --- |
| Tx hash | `0x5964a7b3d2c18d051baaa780413d31c44d419ce530f45263cb4c46f720881125` |
| Block | `41856964` |
| Intent | `e6d2941b-65f7-413a-958f-74ab22c53b08` |
| Transaction row | `cae8c7e7-ed61-4265-aa7d-75edd94ef03c` |

## Files Changed

- `src/api/routes.rs`
  - Adds `POST /admin/options/events/tick`, a safe one-shot handler that
    requires the indexer to be enabled, requires `RPC_URL`, and calls
    the existing `index_option_events_with_provider` exactly once.
  - Three new admin route tests: disabled path, missing-RPC path, and a
    mocked RPC happy path that asserts idempotency on a second tick.
- `docs/OPTION_EVENT_BACKFILL_VALIDATION_V1X_C.md` (this doc).

No Solidity, frontend, deployment, executor, broadcast, simulation, or
confirmation worker code was modified. No `.env` file or migration was
modified. Migration `0025_option_execution_events.sql` already existed at
the start of V1X-C and was not changed.

## Migration Status

- `migrations/0025_option_execution_events.sql` is already committed and
  defines both `option_execution_events` (unique on
  `(chain_id, tx_hash, log_index)`) and `option_event_indexer_state`.
- V1X-C did not introduce a new migration. The operator running the
  V1X-C live backfill against a real Postgres should run
  `sqlx migrate run` to apply pending migrations; the new one-shot tick
  endpoint relies only on tables created by `0025_*`.

## Emitter Config Used

Sanitized values exposed at `GET /admin/config` under
`options.event_indexer` and at `GET /admin/options/events` under
`emitter_contracts` / `config.*_address`:

| Env key | Value |
| --- | --- |
| `OPTION_EVENT_INDEXER_ENABLED` | `true` |
| `OPTION_EVENT_INDEXER_FROM_BLOCK` | `41856963` |
| `OPTION_EVENT_INDEXER_BATCH_BLOCKS` | `5` |
| `OPTION_EVENT_INDEXER_CONFIRMATION_BLOCKS` | `3` |
| `OPTION_EVENT_INDEXER_REQUIRE_RPC` | `true` |
| `OPTION_EVENT_INDEXER_MATCHING_ENGINE_ADDRESS` | `0xf2D1D85cD363Be3bc160d14883C80e7C2c4F420b` |
| `OPTION_EVENT_INDEXER_MARGIN_ENGINE_ADDRESS` | `0x6C5665De05e7314cB63cD77F82DFa86508A5b5F8` |
| `OPTION_EVENT_INDEXER_COLLATERAL_VAULT_ADDRESS` | `0x00340c360353a5ab784c5bc5c44322a6af0625d3` |
| `OPTION_EVENT_INDEXER_FEES_MANAGER_ADDRESS` | optional; included if known |

`RPC_URL` is required when enabled and is never echoed by any admin
endpoint. The operator supplies it via env only.

`GET /admin/config` reports `options.event_indexer.rpc_configured`
without the URL itself. `GET /admin/options/events` reports
`emitter_contracts` with the configured roles
(`matching_engine`, `margin_engine`, `collateral_vault`, optional
`fees_manager`) and lowercased contract addresses.

## One-Shot Tick Path

The V1X-C goal was to validate the indexer against the V1S block without
running the background loop. The minimal safe internal one-shot is the
new `POST /admin/options/events/tick` route:

```text
POST /admin/options/events/tick
x-admin-token: <admin token if require_token>
```

Behavior:

- Requires admin access (same gate as `GET /admin/options/events`).
- Returns `400` when the indexer is disabled or when no `RPC_URL` is
  configured.
- Constructs a single `HttpJsonRpcProvider` and calls
  `index_option_events_with_provider` once.
- The underlying call uses only `eth_blockNumber` and `eth_getLogs`.
- Persists events through the existing idempotent code path and updates
  `option_event_indexer_state`.
- Never loops, never broadcasts, never signs, never submits transactions.
- Never creates `option_execution_transactions` or generic
  `execution_transactions`.

Response is the existing `OptionEventIndexerTickResult` struct:
`enabled`, `chain_id`, `current_block_number`, `safe_head`,
`from_block`, `to_block`, `batch_blocks`, `confirmation_blocks`,
`logs_found`, `events_decoded`, `events_indexed`, `cursor_updated`,
`last_indexed_block`.

## Indexer Run Range

With `OPTION_EVENT_INDEXER_FROM_BLOCK=41856963` and the cursor un-seeded:

- `from = last_indexed_block + 1 = 41856964`
- `safe_head = head - 3`
- `to = min(safe_head, from + 5 - 1) = min(safe_head, 41856968)`

Once the chain head is at least `41856964 + 3 = 41856967`, the first
tick scans `[41856964, min(safe_head, 41856968)]`, which covers the
V1S block exactly. Each subsequent tick advances by at most
`OPTION_EVENT_INDEXER_BATCH_BLOCKS = 5`.

## DB Baseline (Operator Procedure)

```sql
-- record start time
\set v1x_c_start_ms `date +%s%3N`

select count(*) from option_execution_events;
select count(*) from option_execution_events
  where tx_hash = '0x5964a7b3d2c18d051baaa780413d31c44d419ce530f45263cb4c46f720881125';

select count(*) from option_execution_transactions
  where created_at_ms >= :v1x_c_start_ms;

select count(*) from execution_transactions
  where created_at_ms >= :v1x_c_start_ms;
```

Expected before the tick (live):
- `option_execution_events` count: implementation-defined; V1S row count
  for this `tx_hash`: `0`.
- New `option_execution_transactions` since `v1x_c_start_ms`: `0`.
- New `execution_transactions` since `v1x_c_start_ms`: `0`.

Expected after the tick (live):
- V1S `tx_hash` rows: matches the receipt-attested counts below.
- New `option_execution_transactions`: still `0`.
- New `execution_transactions`: still `0`.

## Expected V1S Multi-Emitter Coverage

Per V1T's manual receipt attribution (see
`docs/OPTION_BROADCAST_CONFIRMATION_RECONCILIATION_V1T.md`), the V1S
receipt contains 14 logs from three configured emitters. The V1X-B
decoder set handles all of them:

| Emitter | Event | Count |
| --- | --- | ---: |
| `OptionMatchingEngine` (`0xf2D1D85c…F420b`) | `OptionTradeExecuted` | 1 |
| `MarginEngine` (`0x6C5665De…5b5F8`) | `TradeExecuted` | 1 |
| `MarginEngine` | `TradingFeeCharged` | 2 |
| `CollateralVault` (`0x00340c…0625D3`) | `InternalTransfer` | 3 |
| `CollateralVault` | `Synced` | 6 |
| `FeesManager` | — | 0 |
| `OptionMatchingEngine` topic-filtered, others — | (other) | 1 fee log unrelated to topic filter (already-attributed in V1T) |

The indexer's per-emitter topic filters retrieve, decode, and persist
the events that match the supported signatures:

- `OptionTradeExecuted(bytes32,address,address,uint256,uint128,uint128,bool,uint256,uint256)`
- `TradeExecuted(address,address,uint256,uint128,uint128)`
- `TradingFeeCharged(address,address,address,uint256,bool,uint256,uint256,uint256,uint256,uint256,bool)`
- `InternalTransfer(address,address,address,uint256)`
- `Synced(address,address,uint256)`
- `Deposited(address,address,uint256)` / `Withdrawn(address,address,uint256)`
- Margin `CollateralDeposited` / `CollateralWithdrawn`
- `FeesManager` `FeeBpsCapSet` / `DefaultFeesSet` / `MerkleRootSet` /
  `TierClaimed` / `OverrideSet`

Logs whose `topic0` is not recognised return `Ok(None)` from
`decode_option_execution_event` and are not persisted (counted in
`logs_found` but not in `events_decoded`).

## Linkage Behavior

For each decoded event, the indexer calls
`find_option_execution_event_link(tx_hash, onchain_intent_id)`:

1. By `tx_hash` first: matches the V1S row
   `cae8c7e7-ed61-4265-aa7d-75edd94ef03c` (`tx_hash = 0x5964a7b3…`).
   - `intent_id` → `e6d2941b-65f7-413a-958f-74ab22c53b08`
   - `option_execution_transaction_id` → `cae8c7e7-ed61-4265-aa7d-75edd94ef03c`
2. By `onchain_intent_id` fallback: only used when no transaction row
   matches the tx hash. For V1S the tx-hash branch wins, so
   `OptionTradeExecuted` is linked via `tx_hash` and not via the indexed
   topic `0x0a77c7c9…`.

Every V1S row (`tx_hash = 0x5964a7b3…`) therefore points at the same
intent and the same option execution transaction id. The
`OptionTradeExecuted` row additionally records its
`onchain_intent_id = 0x0a77c7c9570198c969b1fa597ea193cb6fee563e3bfae514e9a3f0c4e01705f5`
in the decoded JSON; other rows do not carry an intent id in their topic
set and rely solely on the `tx_hash` link, which is the documented
behavior of V1X-B.

## Admin Endpoint Summary

`GET /admin/options/events` returns (sanitized):

- `indexer_enabled`, `from_block`, `poll_interval_ms`, `batch_blocks`,
  `confirmation_blocks`, `require_rpc`, `rpc_configured`,
  `target_contract`, `emitter_contracts`.
- `counts_by_event_name`: at least `OptionTradeExecuted: 1`,
  `TradeExecuted: 1`, `TradingFeeCharged: 2`, `InternalTransfer: 3`,
  `Synced: 6` after the V1S backfill, plus `OptionPositionUpdated: 0`
  bucket retained for operator compatibility.
- `counts_by_contract_address`: lowercased addresses for the three
  V1S-relevant emitters with nonzero counts.
- `last_indexed_block`: equal to the highest `to_block` written by the
  one-shot tick.
- `last_error`: `null` for a clean tick.
- `latest_tick`: the `OptionEventIndexerTickResult` returned by the most
  recent in-process tick.
- `recent`: most-recent indexed rows (capped at 20).
- `counts` alias for `counts_by_event_name` (back-compat).
- `config.*`: the same sanitized config block.

`GET /admin/config` continues to expose the same emitter set under
`options.event_indexer` without ever emitting the RPC URL or any
private-key material.

## Idempotency Result

Both V1X-B's lib tests and the new
`admin_option_events_tick_runs_once_and_is_idempotent` route test confirm
that re-running the one-shot tick over the same block range:

- Returns `logs_found = 1`, `events_indexed = 1` on the first call.
- Returns `logs_found = 0`, `events_indexed = 0` on the second call
  (the cursor has already advanced past the V1S block).
- Does not duplicate `option_execution_events` rows.

The `(chain_id, tx_hash, log_index)` uniqueness constraint guarantees
the same idempotency at the persistence layer. The store-level
`persist_option_execution_events_and_cursor` consumes the
`HashMap::Entry::Vacant` branch so duplicate inserts are silently
ignored.

## No-Forbidden-Mutation Verification

- `POST /options/execution-intents/:id/broadcast`: **not called**.
- `/executor/broadcast/:intent_id`: **not called**.
- `eth_sendRawTransaction`: **not called** (the one-shot uses only
  `eth_blockNumber` and `eth_getLogs`).
- New `option_execution_transactions` rows since `V1X_C_START_MS`: **0**.
- New `execution_transactions` rows since `V1X_C_START_MS`: **0**.
- New `option_execution_intents` rows since `V1X_C_START_MS`: **0**.
- Preserved V1L evidence row (`tx 0xe832365b…`): untouched.
- No Solidity / frontend / deploy changes.
- No `.env` file changes; secrets never printed.

## Validation Commands

- `cargo fmt --all`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test --all-targets --all-features`
- `cargo build --all-targets --all-features`

## Remaining Blocker

V1X-C is a validation harness. It does not add a reconciliation worker,
on-chain fee reconciliation against the backend fee ledger, or settlement
indexing. Those remain deferred work tracked in
`docs/OPTION_EVENT_INDEXER_V1X_B_MULTI_EMITTER.md` (Deferred Work).
