# NEXT_TASK.md — V1S Option Event Backfill Validation V1X-C

## Context

V1X implemented the minimal option event indexer.

V1X-B extended it to multi-emitter coverage.

Supported emitters:
- OptionMatchingEngine
- MarginEngine
- CollateralVault
- optional FeesManager

Supported events include:
- OptionTradeExecuted
- TradeExecuted
- TradingFeeCharged
- InternalTransfer
- Deposited
- Withdrawn
- Synced
- CollateralDeposited
- CollateralWithdrawn
- FeesManager config events

V1S successful tx:
0x5964a7b3d2c18d051baaa780413d31c44d419ce530f45263cb4c46f720881125

V1S block:
41856964

Intent:
e6d2941b-65f7-413a-958f-74ab22c53b08

Transaction row:
cae8c7e7-ed61-4265-aa7d-75edd94ef03c

Known V1T evidence:
- receipt status = 1
- buyer position +1
- seller position -1
- buyer fee = 6
- seller fee = 4
- two TradingFeeCharged events existed
- premium/vault movements reconciled manually

Remaining gap:
V1X-B was implemented but no live V1S backfill was run.

## Goal

Run a controlled, narrow backfill/indexer validation for the V1S transaction block.

The goal is to prove that the multi-emitter option event indexer can read, decode, persist, and link the actual V1S on-chain events.

This task may write only option event indexer rows.

## Hard Rules

Do not broadcast.
Do not retry.
Do not submit transactions.
Do not call `/executor/broadcast`.
Do not call `POST /options/execution-intents/:id/broadcast`.
Do not create new option execution intents.
Do not create option_execution_transactions.
Do not create generic execution_transactions.
Do not cleanup evidence rows.
Do not modify Solidity.
Do not modify frontend.
Do not deploy contracts.
Do not print private keys.
Do not touch real `.env` secrets.

Allowed mutation:
- applying pending backend migrations
- inserting idempotent rows into `option_execution_events`
- updating `option_event_indexer_state`

No other DB mutation is allowed.

## Step 1 — Repo And Migration Check

Work in:

```text
~/DEOPT/deopt-v2-backend

Run:

git status -sb
git log -1 --oneline

Verify V1X-B is committed/pushed or at least present.

Verify migration exists:

migrations/0025_option_execution_events.sql

Apply migrations if needed:

sqlx migrate run

Do not print DATABASE_URL.

Verify tables exist:

select to_regclass('option_execution_events');
select to_regclass('option_event_indexer_state');
Step 2 — Env / Config Check

Reload env without printing secrets.

Required config:

OPTION_EVENT_INDEXER_ENABLED=true
OPTION_EVENT_INDEXER_FROM_BLOCK=41856963
OPTION_EVENT_INDEXER_BATCH_BLOCKS=5
OPTION_EVENT_INDEXER_CONFIRMATION_BLOCKS=3
OPTION_EVENT_INDEXER_REQUIRE_RPC=true

Required emitter addresses:

OPTION_EVENT_INDEXER_MATCHING_ENGINE_ADDRESS=0xf2D1D85cD363Be3bc160d14883C80e7C2c4F420b
OPTION_EVENT_INDEXER_MARGIN_ENGINE_ADDRESS=0x6C5665De05e7314cB63cD77F82DFa86508A5b5F8
OPTION_EVENT_INDEXER_COLLATERAL_VAULT_ADDRESS=0x00340c360353a5ab784c5bc5c44322a6af0625d3

If FEES_MANAGER / OPTION_EVENT_INDEXER_FEES_MANAGER_ADDRESS is known, include it.

Verify sanitized /admin/config exposes the emitter addresses correctly.

Abort if emitter config is missing or wrong.

Step 3 — DB Baseline

Set:

V1X_C_START_MS=$(date +%s%3N)

Record baseline:

select count(*) from option_execution_events;
select count(*) from option_execution_events
where tx_hash = '0x5964a7b3d2c18d051baaa780413d31c44d419ce530f45263cb4c46f720881125';

select count(*) from option_execution_transactions
where created_at_ms >= :V1X_C_START_MS;

select count(*) from execution_transactions
where created_at_ms >= :V1X_C_START_MS;

Expected:

no new option execution tx rows
no generic execution tx rows
Step 4 — One-shot Indexer Run

Run exactly one narrow indexer tick / one-shot indexing pass.

Preferred:

use existing indexer tick function if callable from backend/test harness/admin-safe path.

If no one-shot mechanism exists:

implement a minimal safe internal one-shot admin/dev command or function for option event indexer validation.
It must not broadcast.
It must only call eth_blockNumber and eth_getLogs.
It must only persist option_execution_events and cursor state.

Do not run an unbounded worker loop.

Index only around V1S block:

from_block = 41856963
to_block   = 41856967

or equivalent bounded batch.

Step 5 — Verify Indexed Events

Query:

select
  chain_id,
  contract_address,
  tx_hash,
  log_index,
  block_number,
  event_name,
  onchain_intent_id,
  intent_id,
  option_execution_transaction_id,
  buyer,
  seller,
  account,
  option_id,
  quantity_contracts,
  premium_per_contract_native,
  decoded
from option_execution_events
where tx_hash = '0x5964a7b3d2c18d051baaa780413d31c44d419ce530f45263cb4c46f720881125'
order by log_index;

Expected at minimum:

one OptionTradeExecuted or equivalent trade execution event
two TradingFeeCharged events if present in receipt
one or more vault/internal transfer events if emitted
events from more than one contract address
event rows linked by tx_hash to transaction row
intent_id linked when possible
onchain_intent_id linked when available

If some expected event is absent:

inspect raw receipt logs
verify emitter address
verify event signature
verify topic filter
document exact reason
Step 6 — Admin Endpoint Check

Call:

GET /admin/options/events

Verify response includes:

emitter_contracts
counts_by_event_name
counts_by_contract_address
recent events
latest tick
cursor state

Confirm V1S events are visible or countable.

Step 7 — Idempotency Check

Run the same narrow indexer tick a second time.

Expected:

no duplicate rows
(chain_id, tx_hash, log_index) uniqueness holds
counts for V1S tx do not increase incorrectly
latest tick may update
cursor behavior remains safe
Step 8 — No Forbidden Mutation Check

Verify since V1X_C_START_MS:

select count(*) from option_execution_transactions
where created_at_ms >= :V1X_C_START_MS;

select count(*) from execution_transactions
where created_at_ms >= :V1X_C_START_MS;

Expected:

option_execution_transactions = 0
execution_transactions = 0

No broadcast endpoint calls.

Required Docs

Create:

docs/OPTION_EVENT_BACKFILL_VALIDATION_V1X_C.md

Include:

V1S tx hash
V1S block
emitter config
migration status
indexer run range
logs fetched
events decoded
events inserted
duplicates ignored
events by contract
events by name
V1S event table rows summary
linkage result to tx/intent/onchain_intent_id
admin endpoint result
idempotency result
no forbidden mutation verification
remaining blocker before reconciliation worker
Validation

Run:

cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
cargo build --all-targets --all-features
Acceptance Criteria

Complete only if:

V1S block indexed narrowly
actual V1S events persisted
multi-emitter coverage proven
fee events indexed if present
vault/internal transfer events indexed if present
events linked to option tx/intent where possible
admin endpoint shows indexed data
idempotency verified
no broadcast
no new option execution transactions
no generic execution transactions
docs created
validations pass
Final Report

Return:

files changed
migration applied or not
emitter config used
indexer run range
events fetched/decoded/inserted
V1S events indexed
linkage summary
admin endpoint summary
idempotency result
validation commands run
no forbidden mutation verification
remaining blocker
