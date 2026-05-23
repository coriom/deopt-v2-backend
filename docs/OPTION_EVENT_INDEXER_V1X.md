# Option Event Indexer V1X

Date: 2026-05-23

## Scope

V1X adds the first durable option execution event ledger on the backend.
It is disabled by default and only reads finalized logs. It never
broadcasts, signs, retries, submits transactions, creates execution
intents, creates production `option_execution_transactions`, or mutates
generic `execution_transactions`.

The indexer is intentionally narrow:

- Reads logs with `eth_blockNumber` and `eth_getLogs`.
- Uses `safe_head = head - OPTION_EVENT_INDEXER_CONFIRMATION_BLOCKS`.
- Reads from the persisted cursor and scans `cursor + 1` through the
  smaller of `safe_head` and the configured batch end.
- Persists events idempotently on `(chain_id, tx_hash, log_index)`.
- Advances the cursor only after event persistence succeeds.
- Links events to known option execution transactions/intents by `tx_hash`
  first, then by `onchain_intent_id`.
- Exposes a sanitized admin endpoint with counts, recent rows, cursor, and
  the latest in-memory tick.

## Config

New env keys in `src/config/env.rs`:

| Env key | Default |
| --- | ---: |
| `OPTION_EVENT_INDEXER_ENABLED` | `false` |
| `OPTION_EVENT_INDEXER_POLL_INTERVAL_MS` | `15000` |
| `OPTION_EVENT_INDEXER_FROM_BLOCK` | `0` |
| `OPTION_EVENT_INDEXER_BATCH_BLOCKS` | `1000` |
| `OPTION_EVENT_INDEXER_CONFIRMATION_BLOCKS` | `3` |
| `OPTION_EVENT_INDEXER_REQUIRE_RPC` | `true` |

When enabled, startup requires persistence. When `require_rpc=true`,
startup also requires `RPC_URL`. The configured contract target is the
existing `OPTION_MATCHING_ENGINE_ADDRESS`, exposed only as a sanitized
address value.

`GET /admin/config` now includes `options.event_indexer` with only safe
fields and `rpc_configured`; no RPC URL or private key material is emitted.

## Events Decoded

The current Solidity tree defines:

- `OptionTradeExecuted(bytes32,address,address,uint256,uint128,uint128,bool,uint256,uint256)`
- `TradeExecuted(address,address,uint256,uint128,uint128)` on `MarginEngine`
- `TradingFeeCharged(address,address,address,uint256,bool,uint256,uint256,uint256,uint256,uint256,bool)`
- `InternalTransfer(address,address,address,uint256)` on `CollateralVault`

V1X decodes those actual signatures. The active fetch target is the
configured `OptionMatchingEngine`, so the production V1X loop will index
`OptionTradeExecuted` first. `OptionPositionUpdated` is not declared in
`../deopt-v2-sol/src`; V1X does not invent that event name. The admin
counts include an `OptionPositionUpdated: 0` bucket for operator
compatibility with the requested shape.

## DB Schema

Migration added: `migrations/0025_option_execution_events.sql`.

`option_execution_events`:

- `id UUID PRIMARY KEY`
- `chain_id BIGINT NOT NULL`
- `contract_address TEXT NOT NULL`
- `tx_hash TEXT NOT NULL`
- `log_index BIGINT NOT NULL`
- `block_number BIGINT NOT NULL`
- `block_hash TEXT NULL`
- `event_name TEXT NOT NULL`
- `event_signature TEXT NOT NULL`
- `intent_id TEXT NULL`
- `onchain_intent_id TEXT NULL`
- `option_execution_transaction_id TEXT NULL`
- decoded common fields: `buyer`, `seller`, `account`, `option_id`,
  `quantity_contracts`, `premium_per_contract_native`
- raw evidence: `raw_topics JSONB NOT NULL`, `raw_data TEXT NOT NULL`,
  `decoded JSONB NULL`
- timestamps: `created_at_ms`, `updated_at_ms`
- unique: `(chain_id, tx_hash, log_index)`

`option_event_indexer_state`:

- `id TEXT PRIMARY KEY`
- `last_indexed_block BIGINT NOT NULL`
- `updated_at_ms BIGINT NOT NULL`
- `last_error TEXT NULL`

The V1X state row id is `option_events_base_sepolia`.

## Cursor Behavior

`OPTION_EVENT_INDEXER_FROM_BLOCK` is the initial `last_indexed_block`
fallback when no cursor row exists. Each enabled tick computes:

```text
safe_head = head - confirmation_blocks
from = last_indexed_block + 1
to = min(safe_head, from + batch_blocks - 1)
```

If `from > safe_head`, the tick does not fetch logs and does not create a
cursor row. If the range is scanned successfully, the cursor advances to
`to` even when no logs were returned.

## Admin Endpoint

Added `GET /admin/options/events` behind the existing admin auth gate.

Response includes:

- sanitized config fields
- `last_indexed_block`
- `last_error`
- `latest_tick`
- counts by event name, with `OptionTradeExecuted` and
  `OptionPositionUpdated` buckets always present
- recent indexed event rows

No secrets are returned.

## Tests Added

New coverage includes:

- disabled indexer does nothing
- cursor initializes from config
- finality safe head is respected
- batch block size is respected
- no logs advances cursor
- `OptionTradeExecuted` decodes and persists
- duplicate logs are idempotent
- event links to transaction by `tx_hash`
- latest tick is published in memory
- admin endpoint returns config/counts/latest tick
- no generic execution rows
- no broadcast path touched
- config defaults, overrides, RPC requirement, persistence requirement
- topic hash matches the Solidity `OptionTradeExecuted` signature

## Validation

Commands run:

- `cargo fmt --all`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test --lib event_indexer -- --nocapture`
- `cargo test --all-targets --all-features`
- `cargo build --all-targets --all-features`

## No-Forbidden-Mutation Verification

- No Solidity files modified.
- No frontend files modified.
- No `.env` files touched.
- No deploy commands run.
- No private keys printed.
- No broadcast endpoint called.
- No `eth_sendRawTransaction` call added or invoked by the indexer.
- No production `option_execution_transactions` insert path added.
- No generic `execution_transactions` write path added.
- No evidence cleanup path added.

## Remaining Blocker

`OptionPositionUpdated` cannot be decoded because it is not present in the
current Solidity sources. The closest currently declared position-related
trade event is `MarginEngine` `TradeExecuted(...)`; V1X includes a decoder
for that actual signature, but the production fetch target remains the
configured `OptionMatchingEngine` until related contract addresses are
made configurable.
