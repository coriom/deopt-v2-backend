# NEXT_TASK.md — Execution Confirmation / Finality V1

## Context

The backend now supports:

- signed order intake
- in-memory and PostgreSQL persistence
- execution intents
- PerpTrade signing payloads
- buyer/seller PerpTrade signatures
- calldata builder for new intentId PerpMatchingEngine ABI
- RPC simulation
- guarded real broadcast
- indexer for TradeExecuted
- reconciliation by onchain_intent_id

A full Base Sepolia execution has succeeded:

- simulation_ok
- broadcast submitted=true
- real tx_hash returned
- TradeExecuted indexed
- reconciliation matched by onchain_intent_id
- confirmed remains 0 because confirmation lifecycle is not implemented yet

Known successful tx:

- tx_hash: 0x6b837e4c1be858c0acac95e228893b7be1eb06cdc9223a5c6828ef7aac778105
- intent_id: 84faf041-fb8b-445c-a549-99c95bcb5a76
- onchain_intent_id: 0x9f775e79f8ab122b1b4b4d39393e329420f4fee2b5acb10370f76ecc72f02435

## Goal

Implement Execution Confirmation / Finality V1.

The backend must be able to:

1. Read transaction receipts by tx_hash.
2. Verify receipt status == success.
3. Verify the transaction is sufficiently finalized by block confirmations.
4. Require reconciliation matched by onchain_intent_id before marking confirmed.
5. Mark execution transaction as confirmed.
6. Mark execution intent as confirmed only when all safety conditions are met.
7. Expose manual confirmation endpoints.
8. Preserve current safety rules:
   - no fake confirmation
   - no fake tx hash
   - no confirmation without indexed event
   - no confirmation without reconciliation
   - no automatic broadcast changes

## Safety Rules

Do not:

- fake transaction confirmation
- mark confirmed based only on receipt status
- mark confirmed without indexed TradeExecuted
- mark confirmed without reconciliation matched
- change matching semantics
- change PerpTrade ABI
- modify Solidity contracts
- deploy contracts
- enable real broadcast by default
- require RPC/Postgres/private keys for normal cargo test
- use floating point arithmetic

## Confirmation Definition

An execution intent is confirmed only if all conditions are true:

1. There is a submitted execution transaction row with real tx_hash.
2. RPC receipt exists for tx_hash.
3. Receipt status == 1.
4. Receipt block number is not null.
5. Current chain block >= receipt block + CONFIRMATION_REQUIRED_BLOCKS.
6. Indexer has stored a TradeExecuted event for the same tx_hash and onchain_intent_id.
7. Reconciliation exists with:
   - intent_id
   - onchain_intent_id
   - status = matched
8. The matched indexed event corresponds to the transaction hash.

If any condition fails, return a clear pending/failure reason.

## Config

Add config:

```env
CONFIRMATION_ENABLED=false
CONFIRMATION_REQUIRE_PERSISTENCE=true
CONFIRMATION_REQUIRED_BLOCKS=2
CONFIRMATION_MAX_BATCH_SIZE=50
CONFIRMATION_REQUIRE_RECONCILIATION=true

Defaults must be safe.

Normal tests must not need RPC/Postgres.

Database

Extend execution_transactions if needed with:

confirmed_at_ms BIGINT NULL
confirmed_block_number BIGINT NULL
confirmation_status TEXT NULL
confirmation_error TEXT NULL

Allowed statuses:

pending
confirmed
failed
not_finalized
missing_receipt
missing_reconciliation
missing_indexed_event
receipt_failed

Do not delete old migrations.
Add a new migration.

If execution_intents already has status values, add/use confirmed status carefully.

RPC

Extend RPC provider with:

eth_getTransactionReceipt
eth_blockNumber

Receipt parser must handle:

transactionHash
blockNumber
status
logs if needed, but prefer indexed_perp_trades + reconciliation for confirmation
missing/null receipt

Do not rely on logs from receipt alone as final confirmation if indexer/reconciliation is required.

API

Add endpoints:

GET /executor/confirmations/status
POST /executor/confirm/:intent_id
POST /executor/confirmations/tick
GET /executor/confirmations/:intent_id

Expected manual confirmation response:

{
  "intent_id": "...",
  "tx_hash": "0x...",
  "confirmation_status": "confirmed",
  "confirmed": true,
  "receipt_status": 1,
  "receipt_block_number": 41119866,
  "current_block_number": 41119870,
  "required_confirmations": 2,
  "indexed_event_found": true,
  "reconciliation_matched": true,
  "reason": null
}

If not finalized:

{
  "confirmation_status": "not_finalized",
  "confirmed": false,
  "reason": "receipt block has fewer than required confirmations"
}

If missing reconciliation:

{
  "confirmation_status": "missing_reconciliation",
  "confirmed": false,
  "reason": "matched reconciliation not found"
}
Confirmation Tick

POST /executor/confirmations/tick should:

find submitted but unconfirmed execution transactions
process at most CONFIRMATION_MAX_BATCH_SIZE
apply the full confirmation definition
update statuses idempotently
never mark confirmed unless all conditions pass
Tests

Add pure/unit tests for confirmation decision logic.

Tests must cover:

missing receipt => missing_receipt
receipt status 0 => receipt_failed
receipt status 1 but insufficient confirmations => not_finalized
receipt status 1 and enough confirmations but no indexed event => missing_indexed_event
indexed event exists but no matched reconciliation => missing_reconciliation
matched reconciliation but wrong tx_hash => missing_indexed_event or mismatch failure
all conditions true => confirmed
idempotent confirmation does not duplicate rows or corrupt state
normal cargo test does not require live RPC/Postgres
Documentation

Update README.md and ARCHITECTURE.md:

explain confirmation lifecycle
explain why receipt status alone is insufficient
explain finality block requirement
explain relationship:
broadcast -> receipt -> indexer -> reconciliation -> confirmation
document endpoints
document safe defaults
Validation

Run:

cargo fmt
cargo clippy --all-targets --all-features -- -D warnings
cargo test
cargo build
Acceptance Criteria

Complete only if:

confirmation config exists
receipt RPC exists
confirmation decision logic is tested
manual confirm endpoint exists
batch tick endpoint exists
no normal tests require RPC/Postgres/private keys
no fake confirmation is possible
confirmed requires receipt success + enough blocks + indexed event + matched reconciliation
cargo fmt/clippy/test/build pass
EOF




after all, 
## run telegram.py (say nothing, just run and that all)