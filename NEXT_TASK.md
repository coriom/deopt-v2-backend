# NEXT_TASK.md — Live V1S On-chain Fee Verification V2C-LIVE

## Context

V2C implemented backend on-chain fee event reconciliation.

Implemented:
- lifecycle fee section now exposes:
  - source_of_truth = onchain
  - observed_total
  - by_trader
  - by_recipient
  - by_side
  - backend_ledger_status
  - reconciliation_status
- new admin endpoint:
  GET /admin/fees/onchain
- docs:
  docs/FEE_MODEL_TARGET_GAP_ANALYSIS_V2C.md

V2C was test-covered only. Live V1S fee verification was not run.

Known V1S:
- intent: `e6d2941b-65f7-413a-958f-74ab22c53b08`
- tx: `0x5964a7b3d2c18d051baaa780413d31c44d419ce530f45263cb4c46f720881125`
- indexed events: 19
- TradingFeeCharged count: 2
- expected buyer fee: 6
- expected seller fee: 4
- expected total observed fee: 10
- current lifecycle should expose on-chain fee source of truth.

## Goal

Run live verification of V2C fee reconciliation against the operator DB/backend.

This task must verify that V1S on-chain `TradingFeeCharged` events are visible through:
1. lifecycle endpoint;
2. `/admin/fees/onchain`;
3. fee aggregation by trader, recipient, and side.

## Hard Rules

Do not broadcast.
Do not retry transactions.
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
Do not change live fee rates.
Do not print private keys.
Do not touch real `.env` secrets.

No DB mutation is allowed except normal read-only endpoint access.

## Step 1 — Load Env

In:

cd ~/DEOPT/deopt-v2-backend

Load env without printing secrets:

set -a
source .env
set +a

Confirm required vars exist without printing values:

test -n "$DATABASE_URL" && echo "DATABASE_URL set"
test -n "$ADMIN_TOKEN" && echo "ADMIN_TOKEN set"
Step 2 — Start Backend

Start or restart backend.

Verify health:

curl -s http://127.0.0.1:8080/health | jq

Expected:

{"ok":true}
Step 3 — Verify V1S Indexed Fee Events In DB

Run:

psql "$DATABASE_URL" <<'SQL'
select
  event_name,
  count(*)
from option_execution_events
where tx_hash = '0x5964a7b3d2c18d051baaa780413d31c44d419ce530f45263cb4c46f720881125'
group by event_name
order by event_name;

select
  event_name,
  log_index,
  contract_address,
  account,
  decoded
from option_execution_events
where tx_hash = '0x5964a7b3d2c18d051baaa780413d31c44d419ce530f45263cb4c46f720881125'
  and event_name = 'TradingFeeCharged'
order by log_index;
SQL

Expected:

TradingFeeCharged = 2
decoded fee data includes buyer/seller fee shape.
total applied fee should sum to 10.
Step 4 — Verify Lifecycle Fee Section

Call:

curl -s \
  -H "X-Admin-Token: $ADMIN_TOKEN" \
  http://127.0.0.1:8080/admin/options/executions/e6d2941b-65f7-413a-958f-74ab22c53b08/lifecycle \
  | jq '.fees'

Expected:

source_of_truth = "onchain"
trading_fee_event_count = 2
observed_total = "10"
by_side.taker = "6" or equivalent
by_side.maker = "4" or equivalent
backend_ledger_status explicit:
disabled
missing_or_disabled
or present
reconciliation_status explicit.

If fields are missing, document exact mismatch.

Step 5 — Verify Admin On-chain Fees Endpoint

Call by tx hash:

curl -s \
  -H "X-Admin-Token: $ADMIN_TOKEN" \
  "http://127.0.0.1:8080/admin/fees/onchain?tx_hash=0x5964a7b3d2c18d051baaa780413d31c44d419ce530f45263cb4c46f720881125" \
  | jq

Expected:

tx appears.
observed_total = "10".
TradingFeeCharged count = 2.
grouped by trader, recipient, side.

Call recent endpoint:

curl -s \
  -H "X-Admin-Token: $ADMIN_TOKEN" \
  "http://127.0.0.1:8080/admin/fees/onchain?limit=10" \
  | jq

Expected:

recent on-chain fee events include V1S or return a sane list.
no secrets.
Step 6 — Verify No Mutation

Set baseline before endpoint calls if not already done, or compare stable counts after.

Run:

psql "$DATABASE_URL" <<'SQL'
select count(*) as option_execution_intents from option_execution_intents;
select count(*) as option_execution_transactions from option_execution_transactions;
select count(*) as execution_transactions from execution_transactions;
select count(*) as option_execution_events from option_execution_events;
select count(*) as option_execution_reconciliations from option_execution_reconciliations;
select count(*) as fee_events from fee_events;
SQL

Call lifecycle and /admin/fees/onchain again.

Run the same counts.

Expected:

all counts unchanged.
no write mutation.
Step 7 — Safety Search

Run:

rg "sendRawTransaction|eth_sendRawTransaction|/executor/broadcast|execution-intents/.*/broadcast|POST" src/fees src/options src/api/routes.rs

Expected:

no new broadcast/send path in fee endpoint.
POST matches only unrelated existing routes or admin tick routes.
Step 8 — Docs Update

Update:

docs/FEE_MODEL_TARGET_GAP_ANALYSIS_V2C.md

Add:

## Live V1S Fee Verification Result

Include:

lifecycle fee section result
/admin/fees/onchain?tx_hash=... result
observed total
by trader/recipient/side
backend ledger status
no-mutation verification
remaining blocker
Validation

Run:

cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
cargo build --all-targets --all-features
Acceptance Criteria

Complete only if:

live V1S lifecycle fee section verified.
/admin/fees/onchain verified.
observed total = 10.
backend ledger status explicit.
no DB mutation from read-only endpoints.
docs updated.
validations pass.
Final Report

Return:

DB fee event baseline
lifecycle fee result
admin onchain fee endpoint result
backend ledger status
no-mutation verification
safety search result
validation commands run
docs updated
remaining blocker