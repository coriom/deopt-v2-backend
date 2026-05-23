# NEXT_TASK.md — V1S Live Reconciliation Worker Validation V1Y-B

## Context

V1Y implemented the option execution reconciliation worker.

Implemented:
- `option_execution_reconciliations` table.
- `OPTION_RECONCILIATION_WORKER_ENABLED=false` by default.
- `GET /admin/options/reconciliations`.
- `POST /admin/options/reconciliations/tick`.
- Event-based reconciliation checks.
- Idempotent upsert by `option_execution_transaction_id`.

V1Y was test-covered only. It did not run a live reconciliation tick against V1S.

V1S successful tx:

```text
0x5964a7b3d2c18d051baaa780413d31c44d419ce530f45263cb4c46f720881125

Intent:

e6d2941b-65f7-413a-958f-74ab22c53b08

Transaction row:

cae8c7e7-ed61-4265-aa7d-75edd94ef03c

Known state:

V1T manually confirmed V1S.
V1X-C indexed V1S events.
V1Y implemented the reconciliation worker, but did not run it live.
Goal

Run one controlled live reconciliation tick for the already confirmed and already indexed V1S option execution.

This task may write only an idempotent reconciliation row for the existing V1S transaction.

Hard Rules

Do not broadcast.
Do not retry.
Do not submit transactions.
Do not call /executor/broadcast.
Do not call POST /options/execution-intents/:id/broadcast.
Do not create new option execution intents.
Do not create new option_execution_transactions.
Do not create generic execution_transactions.
Do not cleanup evidence rows.
Do not modify Solidity.
Do not modify frontend.
Do not deploy contracts.
Do not print private keys.
Do not touch real .env secrets.

Allowed mutations:

apply pending backend migrations.
insert/update option_execution_reconciliations for the existing V1S transaction.
update in-memory latest tick state.

No other DB mutation is allowed.

Step 1 — Repo And Migration Check

Work in:

~/DEOPT/deopt-v2-backend

Run:

git status -sb
git log -1 --oneline

Verify V1Y code is present.

Verify migration exists:

migrations/0026_option_execution_reconciliations.sql

Apply migrations if needed:

sqlx migrate run

Do not print DATABASE_URL.

Verify table exists:

select to_regclass('option_execution_reconciliations');
Step 2 — Env / Config Check

Reload env without printing secrets.

Set for this controlled tick:

OPTION_RECONCILIATION_WORKER_ENABLED=true
OPTION_RECONCILIATION_POLL_INTERVAL_MS=15000
OPTION_RECONCILIATION_BATCH_SIZE=25
OPTION_RECONCILIATION_REQUIRE_EVENTS=true
OPTION_RECONCILIATION_REQUIRE_RPC=true
OPTION_RECONCILIATION_STRICT=true

Required supporting config:

PERSISTENCE_ENABLED=true
RPC_URL configured
event indexer tables already populated for V1S
option tx confirmation fields already present from V1T/V1W

Verify sanitized /admin/config exposes:

reconciliation worker enabled
require_events true
require_rpc true
strict true
batch_size 25

Abort on mismatch.

Step 3 — DB Baseline

Set:

V1Y_B_START_MS=$(date +%s%3N)

Record:

select intent_id, status
from option_execution_intents
where intent_id = 'e6d2941b-65f7-413a-958f-74ab22c53b08';

select id, tx_hash, confirmation_status, receipt_status, confirmed_block_number
from option_execution_transactions
where id = 'cae8c7e7-ed61-4265-aa7d-75edd94ef03c';

select event_name, count(*)
from option_execution_events
where tx_hash = '0x5964a7b3d2c18d051baaa780413d31c44d419ce530f45263cb4c46f720881125'
group by event_name
order by event_name;

select *
from option_execution_reconciliations
where option_execution_transaction_id = 'cae8c7e7-ed61-4265-aa7d-75edd94ef03c';

select count(*) from option_execution_transactions
where created_at_ms >= :V1Y_B_START_MS;

select count(*) from execution_transactions
where created_at_ms >= :V1Y_B_START_MS;

Expected:

intent status = broadcast_confirmed
tx confirmation_status = mined_success
V1S events exist
no generic execution tx mutations

If V1S already has reconciled, stop and document idempotency instead of forcing a new mutation.

Step 4 — One-shot Reconciliation Tick

Call exactly once:

POST /admin/options/reconciliations/tick

Do not run the unbounded background loop if avoidable.

Expected:

scanned >= 1, or skipped if already reconciled.
V1S reconciled if unreconciled.
no broadcast.
no new option execution transaction.
no generic execution transaction.
Step 5 — Verify Reconciliation Result

Query:

select *
from option_execution_reconciliations
where option_execution_transaction_id = 'cae8c7e7-ed61-4265-aa7d-75edd94ef03c';

Expected:

status = reconciled or justified partially_reconciled.
strict = true.
requires_events = true.
event counts include:
OptionTradeExecuted
TradeExecuted
TradingFeeCharged
InternalTransfer
details JSON contains:
expected intent fields
observed trade event
fee events
transfer events
event_count_by_name
no mismatch_reason

If result is not reconciled:

do not patch immediately.
inspect mismatch reason.
document root cause.
no retry.
Step 6 — Admin Endpoint Check

Call:

GET /admin/options/reconciliations

Verify:

counts show reconciled >= 1
latest_tick is populated
recent includes V1S or the new reconciliation row
no secrets exposed
Step 7 — Idempotency Check

Call the tick a second time only if the implementation is explicitly designed to skip already reconciled rows.

Expected:

V1S not duplicated.
same reconciliation row remains unique.
scanned may be 0 or skipped.
counts do not double.

If unsure, do not call a second tick; verify uniqueness via DB instead.

Step 8 — No Forbidden Mutation Check

Verify since V1Y_B_START_MS:

select count(*) from option_execution_transactions
where created_at_ms >= :V1Y_B_START_MS;

select count(*) from execution_transactions
where created_at_ms >= :V1Y_B_START_MS;

Expected:

option_execution_transactions = 0
execution_transactions = 0

No broadcast endpoint calls.

Required Docs

Create:

docs/OPTION_LIVE_RECONCILIATION_VALIDATION_V1Y_B.md

Include:

V1S tx hash
intent id
transaction id
migration status
config used
DB baseline
events available before tick
tick response
reconciliation row
details summary
admin endpoint result
idempotency result
no forbidden mutation verification
remaining blocker
Validation

Run:

cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
cargo build --all-targets --all-features
Acceptance Criteria

Complete only if:

migration 0026 applied
V1S confirmed tx found
V1S indexed events found
one-shot reconciliation tick run or safely skipped if already reconciled
V1S reconciliation row exists
admin endpoint reflects reconciliation
no broadcast
no new option execution transaction
no generic execution transaction
docs created
validations pass
Final Report

Return:

files changed
migration applied or not
config used
DB baseline summary
tick response
reconciliation status
reconciliation details summary
admin endpoint summary
idempotency result
validation commands run
no forbidden mutation verification
remaining blocker
