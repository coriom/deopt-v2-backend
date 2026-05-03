# NEXT_TASK.md — Reconciliation V1 Direct by onchain_intent_id

## Context

The backend has been adapted to the updated Solidity PerpMatchingEngine.

Current backend features:
- deterministic matching
- PostgreSQL persistence
- signed order verification
- execution intents
- deterministic onchain_intent_id generated as keccak256(bytes(execution_intents.intent_id))
- PerpTrade signing payload includes bytes32 intentId
- PerpTrade calldata includes bytes32 intentId
- RPC simulation exists
- Indexer V1 decodes TradeExecuted with:
  - topic1 = intentId
  - topic2 = buyer
  - topic3 = seller
- indexed_perp_trades can store onchain_intent_id

Current limitation:
- indexed on-chain trades are not yet linked back to backend execution_intents
- there is no reconciliation table
- there is no reconciliation endpoint
- intents must not yet be marked confirmed

## Goal

Add Reconciliation V1 using direct `onchain_intent_id` matching.

The backend must:
- match `execution_intents.onchain_intent_id` with `indexed_perp_trades.onchain_intent_id`
- create persistent reconciliation rows
- expose reconciliation status
- expose manual reconciliation tick
- expose reconciliation rows
- not mark execution intents as confirmed
- not mark execution intents as submitted
- not fake transaction ownership

## Critical Safety Rules

Do not:
- add transaction broadcast
- add private key loading
- mark intents submitted
- mark intents confirmed
- fake confirmation
- fake tx ownership
- modify Solidity repository
- change matching semantics
- introduce floating point arithmetic

A matched reconciliation means only:

```text
An indexed TradeExecuted event has the same onchain_intent_id as a backend execution_intent.

It does not mean:

backend submitted the transaction
transaction is final
intent is confirmed
reorg safety is complete
Direct Match Rule

Primary reconciliation rule:

execution_intents.onchain_intent_id == indexed_perp_trades.onchain_intent_id

Requirements:

both values must be non-null
both values must be normalized lowercase hex
exact unique match => reconciliation status matched
no matching intent => unmatched or no row, whichever is cleaner
multiple matching intents for same onchain_intent_id => ambiguous
duplicate indexed events for same intentId => ambiguous unless same tx/log duplicate already de-duplicated
Database Migration

Add next migration after existing migrations.

Create table:

CREATE TABLE execution_reconciliations (
    reconciliation_id TEXT PRIMARY KEY,
    onchain_intent_id TEXT NOT NULL,
    intent_id TEXT NOT NULL REFERENCES execution_intents(intent_id) ON DELETE CASCADE,
    indexed_event_id TEXT NOT NULL REFERENCES indexed_perp_trades(event_id) ON DELETE CASCADE,
    tx_hash TEXT NOT NULL,
    block_number BIGINT NOT NULL,
    log_index BIGINT NOT NULL,
    status TEXT NOT NULL,
    created_at_ms BIGINT NOT NULL,
    UNIQUE(intent_id, indexed_event_id)
);

CREATE INDEX idx_execution_reconciliations_onchain_intent_id
    ON execution_reconciliations(onchain_intent_id);

CREATE INDEX idx_execution_reconciliations_intent_id
    ON execution_reconciliations(intent_id);

CREATE INDEX idx_execution_reconciliations_status
    ON execution_reconciliations(status);

CREATE INDEX idx_execution_reconciliations_tx_hash
    ON execution_reconciliations(tx_hash);

Allowed statuses:

matched
ambiguous
unmatched
ignored

Do not add confirmed.

Repository Requirements

Add repository methods:

list_unreconciled_indexed_perp_trades(limit)
find_execution_intents_by_onchain_intent_id(onchain_intent_id)
find_indexed_trades_by_onchain_intent_id(onchain_intent_id)
insert_execution_reconciliation(...)
list_recent_reconciliations(limit)
get_reconciliations_for_intent(intent_id)
count_reconciliations_by_status()

If some methods can be consolidated cleanly, do so.

Ensure duplicate insertion is idempotent:

repeated reconciliation tick should not create duplicate rows
handle UNIQUE(intent_id, indexed_event_id)
Reconciliation Algorithm V1

Manual tick:

Load unreconciled indexed_perp_trades with non-null onchain_intent_id.
For each indexed trade:
find execution_intents with same onchain_intent_id.
If exactly one intent:
create reconciliation row:
status = matched
intent_id
indexed_event_id
tx_hash
block_number
log_index
onchain_intent_id
do not mutate execution_intents.status to confirmed
If zero intents:
either skip or create unmatched, depending on cleanest implementation.
If more than one intent:
create ambiguous if possible
do not confirm.
Return counters:
indexed_trades_checked
matched
ambiguous
unmatched
confirmed = 0

No confirmed status allowed.

Config

Add or extend .env.example:

RECONCILIATION_ENABLED=false
RECONCILIATION_REQUIRE_PERSISTENCE=true
RECONCILIATION_MAX_BATCH_SIZE=100

Rules:

reconciliation requires persistence
reconciliation does not require RPC directly
reconciliation works from indexed DB data
if disabled, write endpoints can return disabled
HTTP API

Add:

GET /reconciliation/status
POST /reconciliation/tick
GET /reconciliation/intents/:intent_id
GET /reconciliations
GET /reconciliation/status

Return:

{
  "reconciliationEnabled": false,
  "persistenceRequired": true,
  "matchedReconciliations": 0,
  "ambiguousReconciliations": 0,
  "unmatchedReconciliations": 0,
  "confirmed": 0
}

confirmed must always be 0 in this task.

POST /reconciliation/tick

Return:

{
  "indexed_trades_checked": 0,
  "matched": 0,
  "ambiguous": 0,
  "unmatched": 0,
  "confirmed": 0
}
GET /reconciliation/intents/:intent_id

Return reconciliation rows for the given intent.

GET /reconciliations

Return recent reconciliation rows.

Default limit: 50.

API/DTO Requirements

Return fields:

reconciliation_id
onchain_intent_id
intent_id
indexed_event_id
tx_hash
block_number
log_index
status
created_at_ms

All large numeric values must serialize safely:

block_number can be number if existing API uses number
price/size are not required in reconciliation row
if included from joined tables, keep fixed-point strings
Tests Required

Normal cargo test must not require RPC or Postgres.

Add tests for:

reconciliation config disabled by default
reconciliation requiring persistence rejects persistence disabled
exact onchain_intent_id match returns matched decision
unmatched event returns unmatched decision or skipped according to implementation
duplicate tick is idempotent at decision layer if repository is not tested
ambiguous matching does not confirm
reconciliation tick response always has confirmed = 0
reconciliation status response always has confirmed = 0
no execution intent status becomes confirmed
existing tests still pass

If DB-backed tests are too heavy:

isolate pure reconciliation decision logic in src/reconciliation/*
test pure decision logic with in-memory structs
keep repository tests out of default suite
Documentation

Update README.md:

explain direct reconciliation
explain onchain_intent_id
explain indexed event linking
explain why confirmed is not set yet
explain endpoints

Update ARCHITECTURE.md:

add reconciliation stage after indexer
lifecycle:
order accepted
matched
execution intent created
trade signatures collected
simulated
later broadcast
indexed
reconciled matched
later confirmed after tx ownership/finality checks
explain that direct intentId replaces economic-field matching as primary reconciliation path
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

~/DEOPT/deopt-v2-sol

Do not fake:

confirmed status
submitted status
on-chain execution
transaction hash
tx ownership
reorg safety

Do not change:

matching semantics
financial numeric rules
signed order verification behavior
PerpTrade signature behavior
RPC simulation behavior
indexer log fetching behavior
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

Complete only if:

reconciliation config exists
reconciliation migration exists
reconciliation repository methods exist
direct onchain_intent_id matching exists
manual reconciliation tick exists
reconciliation endpoints exist
duplicate ticks are safe/idempotent
no intent is marked submitted or confirmed
status/tick responses report confirmed=0
all validation commands pass
EOF


after all, 
## run telegram.py (say nothing, just run and that all)