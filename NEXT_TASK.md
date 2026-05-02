# NEXT_TASK.md — Indexer V1 for PerpMatchingEngine Events

## Context

The Rust backend now has:
- deterministic in-memory matching
- strict EIP-712 order verification
- PostgreSQL persistence
- execution intents
- matched PerpTrade signature collection
- real PerpMatchingEngine calldata builder
- RPC simulation via eth_call
- simulation result persistence

Current status:
- no real transaction broadcast exists
- no private key loading exists
- no submitted/confirmed lifecycle exists
- no indexer exists yet

Current limitation:
- the backend cannot read on-chain events
- the backend cannot reconcile contract state/events with backend state
- future broadcast would have no safe confirmation path

## Goal

Add Indexer V1.

The indexer must:
- read Base Sepolia logs from RPC
- decode `PerpMatchingEngine.TradeExecuted`
- persist indexed events
- persist block cursor
- expose indexer status
- expose manual indexer tick
- not mark backend intents confirmed yet unless there is a safe deterministic link

This task must not add transaction broadcast.

## Critical Safety Rule

Do not fake confirmation.

An indexed `TradeExecuted` event proves that a trade was executed on-chain, but mapping it to a backend `ExecutionIntent` must be explicit and reliable.

If there is no deterministic intent id in the event, do not mark an intent as confirmed.

For this task:
- index the event
- persist it
- expose it
- prepare reconciliation
- do not fake confirmed lifecycle

## Target Solidity Event

From `PerpMatchingEngine`:

```solidity
event TradeExecuted(
    address indexed buyer,
    address indexed seller,
    uint256 indexed marketId,
    uint128 sizeDelta1e8,
    uint128 executionPrice1e8,
    bool buyerIsMaker,
    uint256 buyerNonce,
    uint256 sellerNonce
);

Contract:

PerpMatchingEngine
Address from env:
PERP_MATCHING_ENGINE_ADDRESS
Config

Add or extend .env.example:

INDEXER_ENABLED=false
INDEXER_START_BLOCK=0
INDEXER_POLL_INTERVAL_MS=3000
INDEXER_MAX_BLOCK_RANGE=500
INDEXER_REQUIRE_PERSISTENCE=true

Rules:

INDEXER_ENABLED=false: indexer does not start.
INDEXER_ENABLED=true: requires RPC_URL.
if INDEXER_REQUIRE_PERSISTENCE=true, requires PERSISTENCE_ENABLED=true.
no private key required.
no transaction sending.
Required Modules

Add modules such as:

src/indexer/mod.rs
src/indexer/config.rs
src/indexer/events.rs
src/indexer/decoder.rs
src/indexer/runner.rs
src/indexer/status.rs

Adapt names if cleaner.

Database Requirements

Add migration:

migrations/0004_indexer.sql

Tables:

indexer_cursors
CREATE TABLE indexer_cursors (
    name TEXT PRIMARY KEY,
    last_indexed_block BIGINT NOT NULL,
    updated_at_ms BIGINT NOT NULL
);
indexed_perp_trades
CREATE TABLE indexed_perp_trades (
    event_id TEXT PRIMARY KEY,
    tx_hash TEXT NOT NULL,
    log_index BIGINT NOT NULL,
    block_number BIGINT NOT NULL,
    block_hash TEXT,
    buyer TEXT NOT NULL,
    seller TEXT NOT NULL,
    market_id TEXT NOT NULL,
    size_delta_1e8 TEXT NOT NULL,
    execution_price_1e8 TEXT NOT NULL,
    buyer_is_maker BOOLEAN NOT NULL,
    buyer_nonce TEXT NOT NULL,
    seller_nonce TEXT NOT NULL,
    created_at_ms BIGINT NOT NULL,
    UNIQUE(tx_hash, log_index)
);

Indexes:

block_number
buyer
seller
market_id
tx_hash

Optional generic table:

indexed_events
CREATE TABLE indexed_events (
    event_id TEXT PRIMARY KEY,
    contract_address TEXT NOT NULL,
    event_name TEXT NOT NULL,
    tx_hash TEXT NOT NULL,
    log_index BIGINT NOT NULL,
    block_number BIGINT NOT NULL,
    payload_json TEXT NOT NULL,
    created_at_ms BIGINT NOT NULL,
    UNIQUE(tx_hash, log_index)
);

If too much scope, implement indexed_perp_trades only.

RPC Behavior

Implement log fetching using eth_getLogs.

For each tick:

Read current chain block number.
Read cursor for perp_matching_engine.
Determine range:
from = last_indexed_block + 1
to = min(current_block, from + INDEXER_MAX_BLOCK_RANGE - 1)
Fetch logs for:
address = PERP_MATCHING_ENGINE_ADDRESS
topic0 = keccak256 of TradeExecuted(address,address,uint256,uint128,uint128,bool,uint256,uint256)
Decode logs.
Persist decoded trades.
Advance cursor only after successful persistence.

If no cursor exists:

initialize from INDEXER_START_BLOCK.
Reorg Handling

Minimal V1:

store block_hash if available
do not implement deep reorg rollback yet
document limitation

Do not pretend full reorg safety.

HTTP API

Add:

GET /indexer/status
POST /indexer/tick
GET /indexed/perp-trades
GET /indexer/status

Return:

{
  "indexerEnabled": false,
  "rpcConfigured": true,
  "persistenceRequired": true,
  "lastIndexedBlock": 0,
  "targetContract": "0x..."
}
POST /indexer/tick

Runs one manual indexing tick.

Return:

from_block
to_block
logs_found
events_indexed
cursor_updated
GET /indexed/perp-trades

Return recent indexed TradeExecuted events.

Limit can default to 50.

Tests Required

Normal cargo test must not require RPC or Postgres.

Add tests for:

indexer config disabled
indexer enabled requires RPC_URL
indexer requiring persistence rejects persistence disabled
TradeExecuted topic0 is correct
decoder decodes a known synthetic log
cursor range calculation
indexer status endpoint returns fields
no confirmation status is written
existing tests still pass

If full log decoding test is too heavy:

test topic0 and pure decode helpers
do not require live RPC
Documentation

Update README.md:

explain indexer role
explain env vars
explain manual tick
explain indexed_perp_trades
explain no fake confirmation

Update ARCHITECTURE.md:

add indexer stage
explain backend lifecycle:
order accepted
matched
intent created
signed
simulated
later submitted
indexed
reconciled
explain that confirmation requires deterministic reconciliation
Constraints

Do not add:

transaction broadcast
private key loading
sendTransaction
production executor
frontend
TypeScript
Python
Node.js
Solidity changes

Do not modify:

~/DEOPT/deoptv2

Do not fake:

confirmed status
submitted status
on-chain execution
transaction hash
reorg safety

Do not change:

matching semantics
financial numeric rules
EIP-712 order verification
PerpTrade signature flow
RPC simulation behavior
Validation

Run:

cargo fmt
cargo clippy --all-targets --all-features -- -D warnings
cargo test
cargo build

Normal tests must not require:

RPC
Postgres
private keys
Base Sepolia
Acceptance Criteria

The task is complete only if:

indexer config exists
DB migration exists
TradeExecuted topic/decoder exists
indexer cursor exists
manual tick endpoint exists
indexed perp trades can be persisted
no fake confirmed status is introduced
all tests pass
EOF

after all, 
## run telegram.py (say nothing, just run and that all)