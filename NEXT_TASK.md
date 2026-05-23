# NEXT_TASK.md — Option Event Indexer V1X

## Context

V1S successfully executed the first live option trade on Base Sepolia.

Tx:
0x5964a7b3d2c18d051baaa780413d31c44d419ce530f45263cb4c46f720881125

Intent:
e6d2941b-65f7-413a-958f-74ab22c53b08

V1T added manual confirmation/reconciliation.

V1V added the background confirmation worker.

V1W added:
- receipt cost persistence
- admin confirmation summary
- latest worker tick observability
- `GET /admin/options/confirmations`

Remaining gap:
The backend does not yet index on-chain option events such as:
- `OptionTradeExecuted`
- `OptionPositionUpdated`
- vault/premium transfer events
- fee events if present

Without event indexing, the backend knows a transaction succeeded, but does not yet have a durable event-level ledger.

## Goal

Implement a minimal backend option event indexer.

The indexer should read logs from `OptionMatchingEngine` and related contracts, decode relevant events, persist them, and link them to known option execution intents/transactions where possible.

This is backend-only.

## Hard Rules

Do not broadcast.
Do not retry.
Do not submit transactions.
Do not call `/executor/broadcast`.
Do not call `POST /options/execution-intents/:id/broadcast`.
Do not create new option execution intents.
Do not create option_execution_transactions except in tests.
Do not create generic execution_transactions.
Do not cleanup existing evidence rows.
Do not modify Solidity.
Do not modify frontend.
Do not deploy contracts.
Do not print private keys.
Do not touch real `.env` secrets.

## Required Config

Add safe config flags:

```text
OPTION_EVENT_INDEXER_ENABLED=false
OPTION_EVENT_INDEXER_POLL_INTERVAL_MS=15000
OPTION_EVENT_INDEXER_FROM_BLOCK=0
OPTION_EVENT_INDEXER_BATCH_BLOCKS=1000
OPTION_EVENT_INDEXER_CONFIRMATION_BLOCKS=3
OPTION_EVENT_INDEXER_REQUIRE_RPC=true

Defaults:

disabled by default
no indexing if RPC missing and require_rpc=true
conservative batch size

Expose sanitized values in /admin/config.

Required DB

Add migration for indexed option events.

Suggested table:

option_execution_events

Suggested columns:

id UUID primary key
chain_id BIGINT not null
contract_address TEXT not null
tx_hash TEXT not null
log_index BIGINT not null
block_number BIGINT not null
block_hash TEXT
event_name TEXT not null
event_signature TEXT not null
intent_id UUID null
onchain_intent_id TEXT null
option_execution_transaction_id UUID null
buyer TEXT null
seller TEXT null
account TEXT null
option_id TEXT null
quantity_contracts TEXT null
premium_per_contract_native TEXT null
raw_topics JSONB not null
raw_data TEXT not null
decoded JSONB null
created_at_ms BIGINT not null
updated_at_ms BIGINT not null

Add uniqueness:

(chain_id, tx_hash, log_index)

Use ADD COLUMN IF NOT EXISTS / safe migration style where applicable.

Required Indexer Cursor

Add table or config-backed state for indexer cursor.

Suggested table:

option_event_indexer_state

Fields:

id TEXT primary key
last_indexed_block BIGINT not null
updated_at_ms BIGINT not null
last_error TEXT null

Use one row:

option_events_base_sepolia
Required Events

At minimum decode/persist:

OptionTradeExecuted
OptionPositionUpdated

Also attempt to support if ABI/source contains them:

TradingFeeCharged
InternalTransfer
CollateralTransferred
PremiumPaid

Do not invent event names. Search Solidity sources:

rg "event " ../deopt-v2-sol/src

Use actual event signatures.

Required Log Fetching

Implement or reuse RPC support for:

eth_getLogs
eth_blockNumber

Indexer behavior:

If disabled, do nothing.
If enabled:
get current head block.
safe_head = head - OPTION_EVENT_INDEXER_CONFIRMATION_BLOCKS.
read cursor.
index from cursor+1 to safe_head, bounded by batch size.
fetch logs for configured contracts/topics.
decode known events.
persist idempotently.
advance cursor only after successful persistence.
Never broadcast.
Never mutate execution statuses directly in V1X.
Do not perform full reconciliation yet.
Required Linking

When indexing OptionTradeExecuted, try to link event to backend records.

Link by one or more of:

onchain_intent_id
tx_hash
option_execution_transaction_id
intent_id

Expected for V1S:

tx hash:
0x5964a7b3d2c18d051baaa780413d31c44d419ce530f45263cb4c46f720881125
onchain intent id:
0x0a77c7c9570198c969b1fa597ea193cb6fee563e3bfae514e9a3f0c4e01705f5

If linkage fails, persist event unlinked and document why.

Required Admin Endpoint

Add:

GET /admin/options/events

Return sanitized summary:

{
  "indexer_enabled": false,
  "from_block": 0,
  "batch_blocks": 1000,
  "confirmation_blocks": 3,
  "last_indexed_block": 0,
  "latest_tick": null,
  "counts": {
    "OptionTradeExecuted": 0,
    "OptionPositionUpdated": 0
  },
  "recent": []
}

No secrets.

Required Latest Tick

Add in-memory latest tick state:

ran_at_ms
head_block
safe_head
from_block
to_block
logs_fetched
events_decoded
events_inserted
events_duplicate
events_failed
last_indexed_block
error

Expose via admin endpoint.

Required Tests

Add tests for:

indexer disabled does nothing.
cursor initializes correctly.
safe_head applies confirmation blocks.
batch block limit is respected.
eth_getLogs no logs advances cursor.
known OptionTradeExecuted log decodes and persists.
duplicate log is idempotent.
event links to transaction by tx_hash.
admin events endpoint returns config/counts/latest tick.
no broadcast path touched.
no generic execution_transactions rows created.

Use mock RPC/log provider. Do not depend on live Base Sepolia.

Required Docs

Create:

docs/OPTION_EVENT_INDEXER_V1X.md

Include:

why receipt confirmation is not enough
indexed contracts
indexed events
DB schema
cursor behavior
finality behavior
idempotence rule
admin endpoint
relation to V1S/V1T/V1V/V1W
remaining deferred work:
full reconciliation worker
event-driven position reconciliation
alerting
frontend event display
settlement/exercise/expiry flows
Validation

Run:

cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
cargo build --all-targets --all-features
Acceptance Criteria

Complete only if:

event indexer exists
disabled by default
cursor exists
eth_getLogs support exists
at least OptionTradeExecuted and OptionPositionUpdated are decoded/persisted
events are idempotent by (chain_id, tx_hash, log_index)
admin endpoint exists
tests pass
docs created
no transaction submitted
no broadcast path touched
Final Report

Return:

files changed
migrations added
config added
events decoded
DB schema summary
cursor behavior
admin endpoint added
tests added
docs created
validation commands run
no forbidden mutation verification
remaining blocker
