# NEXT_TASK.md — RPC Upgrade And Resume Event Indexer Catch-up V2D-T2

## Context

V2D-T attempted to catch the option event indexer up to the MarginEngineV2 cutover block.

Current state:
- NEW_MARGIN_ENGINE = 0x287Cef479be5889eEfCa847F9e73C860898f48Cc
- CUTOVER_BLOCK = 42073772
- target indexer cursor >= 42073775
- current cursor = 41857113
- remaining gap = 216,662 blocks

V2D-T failed because the current RPC provider is Alchemy free tier:
- eth_getLogs capped at 10 blocks per request
- compute-unit/sec throttling after a few ticks
- no option/intents/tx/generic tx mutations occurred
- only option_event_indexer_state cursor advanced from 41857003 to 41857113

Goal:
Swap to a paid/higher-tier RPC endpoint and resume catch-up until cursor >= 42073775.

No code change required unless a hidden blocker appears.

## Hard Rules

Do not broadcast.
Do not submit transactions.
Do not deploy.
Do not modify Solidity.
Do not modify frontend.
Do not enable FeesManagerV2.
Do not deploy FeesManagerV2.
Do not call setUseFeesManagerV2.
Do not create option execution intents.
Do not create option execution transactions.
Do not create generic execution transactions.
Do not cleanup historical rows.
Do not print private keys.
Do not commit real `.env`.
Do not paste RPC secret into docs.

Allowed:
- runtime `RPC_URL` swap.
- backend restart.
- manual bounded indexer ticks.
- DB read checks.
- admin endpoint calls.
- docs update with provider type only, not secret URL.

## Step 1 — Operator Provides RPC Endpoint Locally

The operator must set a better RPC endpoint locally.

Do not print it.

Example:

export RPC_URL="https://base-sepolia.g.alchemy.com/v2/<PAID_KEY>"

or:

export RPC_URL="<QUICKNODE_OR_BLOCKPI_OR_CHAINSTACK_BASE_SEPOLIA_RPC>"

Confirm only:

test -n "$RPC_URL" && echo "RPC_URL set"

Do not echo the value.

Step 2 — Load Backend Cutover Env

In:

cd ~/DEOPT/deopt-v2-backend

Load cutover env without printing secrets:

set -a
source .env.cutover.v2d_s.local
set +a

Then override RPC_URL in the same shell:

export RPC_URL="<operator supplied privately>"

Reapply cutover values:

export MARGIN_ENGINE=0x287Cef479be5889eEfCa847F9e73C860898f48Cc
export OPTION_EVENT_INDEXER_MARGIN_ENGINE_ADDRESS=0x287Cef479be5889eEfCa847F9e73C860898f48Cc
export OLD_MARGIN_ENGINE=0x6C5665De05e7314cB63cD77F82DFa86508A5b5F8
export MARGIN_ENGINE_CUTOVER_BLOCK=42073772
Step 3 — Baseline

Set:

V2D_T2_START_MS=$(date +%s%3N)

Record:

select last_indexed_block, last_error
from option_event_indexer_state
order by updated_at_ms desc
limit 5;

select count(*) as option_execution_intents from option_execution_intents;
select count(*) as option_execution_transactions from option_execution_transactions;
select count(*) as execution_transactions from execution_transactions;
select count(*) as option_execution_events from option_execution_events;
select count(*) as option_execution_reconciliations from option_execution_reconciliations;
select count(*) as fee_events from fee_events;

Expected:

cursor = 41857113 or later.
critical row counts unchanged from V2D-T.
Step 4 — Start Backend With Catch-up Config

Use:

export OPTION_CONFIRMATION_WORKER_ENABLED=false
export OPTION_RECONCILIATION_WORKER_ENABLED=false

export OPTION_EVENT_INDEXER_ENABLED=true
export OPTION_EVENT_INDEXER_REQUIRE_RPC=true
export OPTION_EVENT_INDEXER_CONFIRMATION_BLOCKS=3
export OPTION_EVENT_INDEXER_BATCH_BLOCKS=5000
export OPTION_EVENT_INDEXER_MARGIN_ENGINE_ADDRESS=0x287Cef479be5889eEfCa847F9e73C860898f48Cc

Start/restart backend.

Verify:

curl -s http://127.0.0.1:8080/health | jq

curl -s http://127.0.0.1:8080/admin/config \
  -H "X-Admin-Token: $ADMIN_TOKEN" | jq '.options.event_indexer'

Expected:

health ok.
batch_blocks = 5000.
margin_engine emitter = NEW.
no secrets.
Step 5 — Resume Catch-up Loop

Run bounded ticks until:

last_indexed_block >= 42073775

Use max 60 ticks:

for i in $(seq 1 60); do
  echo "tick $i"
  curl -s -X POST http://127.0.0.1:8080/admin/options/events/tick \
    -H "X-Admin-Token: $ADMIN_TOKEN" \
    | jq '{from_block,to_block,logs_found,events_indexed,last_indexed_block,last_error}'

  LAST=$(curl -s http://127.0.0.1:8080/admin/options/events \
    -H "X-Admin-Token: $ADMIN_TOKEN" | jq -r '.last_indexed_block')

  echo "last=$LAST"

  if [ "$LAST" -ge 42073775 ]; then
    break
  fi
done

Expected:

roughly 44 ticks at 5,000-block batches.
cursor reaches >= 42073775.
last_error = null.
logs may be zero or some legitimate logs.
no critical tx/intents/generic tx rows.

If provider still caps range:

lower batch to 1000.
if still throttled, use another RPC provider.
Step 6 — Verify Catch-up

Call:

curl -s http://127.0.0.1:8080/admin/options/events \
  -H "X-Admin-Token: $ADMIN_TOKEN" | jq

Expected:

last_indexed_block >= 42073775.
last_error = null.
margin_engine emitter = NEW.
V1S historical data still visible.

DB:

select last_indexed_block, last_error
from option_event_indexer_state
order by updated_at_ms desc
limit 5;

select event_name, count(*)
from option_execution_events
group by event_name
order by event_name;
Step 7 — No Forbidden Mutation Check

Run:

select count(*) as option_execution_intents from option_execution_intents;
select count(*) as option_execution_transactions from option_execution_transactions;
select count(*) as execution_transactions from execution_transactions;
select count(*) as option_execution_events from option_execution_events;
select count(*) as option_execution_reconciliations from option_execution_reconciliations;
select count(*) as fee_events from fee_events;

Expected:

no new option_execution_intents.
no new option_execution_transactions.
no new execution_transactions.
option_execution_events may increase only from legitimate indexed logs.
fee_events unchanged.
Step 8 — Return Backend To Safe Mode

Disable automatic indexer after catch-up unless operator wants it running:

export OPTION_EVENT_INDEXER_ENABLED=false

Restart backend or document stopped/safe mode.

Required Docs

Create:

docs/OPTION_EVENT_INDEXER_CATCHUP_CUTOVER_V2D_T2.md

Update:

docs/OPTION_EVENT_INDEXER_CATCHUP_CUTOVER_V2D_T.md
docs/MARGIN_ENGINE_V2_BACKEND_CUTOVER_V2D_S.md

Include:

provider type used, not secret URL.
baseline cursor.
final cursor.
tick count.
batch size.
logs/events indexed.
no-mutation proof.
remaining blocker before tiny test trade.
Validation

Run:

cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
cargo build --all-targets --all-features
Acceptance Criteria

Complete only if:

cursor reaches >= 42073775.
last_error = null.
backend still reports NEW emitter.
no option intents/txs/generic txs created.
docs created/updated.
validations pass.
Final Report

Return:

RPC provider type, without URL.
baseline cursor.
final cursor.
tick count.
events indexed.
admin events summary.
no-mutation verification.
docs updated.
validation commands run.
remaining blocker before tiny test trade.