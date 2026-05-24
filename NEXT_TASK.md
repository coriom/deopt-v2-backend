# NEXT_TASK.md — Live V1S On-chain State Cross-Check Validation V2B-LIVE

## Context

V2B implemented backend-only on-chain state cross-checks.

Implemented:
- `src/options/state_checks.rs`
- read-only `eth_call` helpers
- buyer/seller nonce checks
- buyer/seller position checks via `MarginEngine.getPositionQuantity(address,uint256)`
- observed-only open interest via `seriesShortOpenInterest(uint256)`
- observed-only vault balances via `CollateralVault.balances(address,address)`
- lifecycle endpoint now includes `state_checks`
- admin reconciliation endpoint exposes state-check config and counts

Config added:
- `OPTION_RECONCILIATION_STATE_CHECKS_ENABLED=false`
- `OPTION_RECONCILIATION_STATE_CHECKS_REQUIRE_RPC=true`
- `OPTION_RECONCILIATION_STATE_CHECKS_STRICT=false`

V2B was test-covered only. It did not run live V1S DB/RPC verification.

Known V1S:
- intent: `e6d2941b-65f7-413a-958f-74ab22c53b08`
- tx: `0x5964a7b3d2c18d051baaa780413d31c44d419ce530f45263cb4c46f720881125`
- transaction row: `cae8c7e7-ed61-4265-aa7d-75edd94ef03c`
- current reconciliation status: `reconciled`
- indexed events: 19
- buyer position expected: `+1`
- seller position expected: `-1`
- buyer/seller nonces expected: at least signed nonce + 1

## Goal

Run live V1S state-check validation with real DB and RPC.

This task should verify that V1S remains reconciled while adding state-check evidence to reconciliation details and lifecycle output.

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
Do not print private keys.
Do not touch real `.env` secrets.

Allowed mutations:
- update existing V1S `option_execution_reconciliations` row details with state-check evidence.
- update latest reconciliation tick state.

No other DB mutation is allowed.

## Step 1 — Load Env

In:

cd ~/DEOPT/deopt-v2-backend

Load env without printing secrets:

set -a
source .env
set +a

Confirm required vars exist without printing values:

test -n "$DATABASE_URL" && echo "DATABASE_URL set"
test -n "$RPC_URL" && echo "RPC_URL set"
test -n "$ADMIN_TOKEN" && echo "ADMIN_TOKEN set"
Step 2 — Apply Migrations / Verify DB

Run:

sqlx migrate run

Verify V1S records exist:

psql "$DATABASE_URL" <<'SQL'
select intent_id, status
from option_execution_intents
where intent_id = 'e6d2941b-65f7-413a-958f-74ab22c53b08';

select id, tx_hash, confirmation_status, receipt_status, confirmed_block_number
from option_execution_transactions
where id = 'cae8c7e7-ed61-4265-aa7d-75edd94ef03c';

select status, event_check_status, nonce_check_status, position_check_status, details
from option_execution_reconciliations
where option_execution_transaction_id = 'cae8c7e7-ed61-4265-aa7d-75edd94ef03c';

select event_name, count(*)
from option_execution_events
where tx_hash = '0x5964a7b3d2c18d051baaa780413d31c44d419ce530f45263cb4c46f720881125'
group by event_name
order by event_name;
SQL

Expected:

intent status = broadcast_confirmed
tx confirmation_status = mined_success
reconciliation status = reconciled
V1S events exist

Abort if V1S is not already event-reconciled.

Step 3 — Start Backend With State Checks Enabled

Use:

export OPTION_RECONCILIATION_WORKER_ENABLED=true
export OPTION_RECONCILIATION_REQUIRE_EVENTS=true
export OPTION_RECONCILIATION_REQUIRE_RPC=true
export OPTION_RECONCILIATION_STRICT=true
export OPTION_RECONCILIATION_BATCH_SIZE=25

export OPTION_RECONCILIATION_STATE_CHECKS_ENABLED=true
export OPTION_RECONCILIATION_STATE_CHECKS_REQUIRE_RPC=true
export OPTION_RECONCILIATION_STATE_CHECKS_STRICT=false

Start or restart backend.

Important:

State checks should be non-strict for this first live run.
V1S must not regress from reconciled unless there is a real code defect; in non-strict mode mismatches should be recorded as warnings/details.
Step 4 — Verify Admin Config

Run:

curl -s http://127.0.0.1:8080/admin/config \
  -H "X-Admin-Token: $ADMIN_TOKEN" | jq '.options.reconciliation_worker'

Expected:

reconciliation worker enabled = true
require_events = true
require_rpc = true
strict = true
state_checks.enabled = true
state_checks.require_rpc = true
state_checks.strict = false

Abort on mismatch.

Step 5 — Read On-chain State Directly

Before tick, perform direct read-only cast call checks.

Check buyer/seller nonces:

cast call 0xf2D1D85cD363Be3bc160d14883C80e7C2c4F420b \
  "nonces(address)(uint256)" \
  0xc0A76c2A6c6b70C0B065A05E64417886416cc976 \
  --rpc-url "$RPC_URL"

cast call 0xf2D1D85cD363Be3bc160d14883C80e7C2c4F420b \
  "nonces(address)(uint256)" \
  0xbAf0976a00a0DCc84Df5B15d927695c8b014B1c3 \
  --rpc-url "$RPC_URL"

Check positions:

cast call 0x6C5665De05e7314cB63cD77F82DFa86508A5b5F8 \
  "getPositionQuantity(address,uint256)(int128)" \
  0xc0A76c2A6c6b70C0B065A05E64417886416cc976 \
  24145907678156652148089862289363692212069910767044828147380657249455352740183 \
  --rpc-url "$RPC_URL"

cast call 0x6C5665De05e7314cB63cD77F82DFa86508A5b5F8 \
  "getPositionQuantity(address,uint256)(int128)" \
  0xbAf0976a00a0DCc84Df5B15d927695c8b014B1c3 \
  24145907678156652148089862289363692212069910767044828147380657249455352740183 \
  --rpc-url "$RPC_URL"

Expected:

buyer nonce >= 1
seller nonce >= 1
buyer position includes +1
seller position includes -1

Also optionally read open interest:

cast call 0x6C5665De05e7314cB63cD77F82DFa86508A5b5F8 \
  "seriesShortOpenInterest(uint256)(uint128)" \
  24145907678156652148089862289363692212069910767044828147380657249455352740183 \
  --rpc-url "$RPC_URL"

Record results.

Step 6 — Set Baseline

Set:

V2B_LIVE_START_MS=$(date +%s%3N)

Record counts:

psql "$DATABASE_URL" <<SQL
select count(*) as option_txs_since_start
from option_execution_transactions
where created_at_ms >= ${V2B_LIVE_START_MS};

select count(*) as generic_txs_since_start
from execution_transactions
where created_at_ms >= ${V2B_LIVE_START_MS};

select count(*) as intents_since_start
from option_execution_intents
where created_at_ms >= ${V2B_LIVE_START_MS};
SQL

Expected all 0.

Step 7 — Run One Reconciliation Tick

Call exactly once:

curl -s -X POST http://127.0.0.1:8080/admin/options/reconciliations/tick \
  -H "X-Admin-Token: $ADMIN_TOKEN" | jq

Expected:

V1S is considered if the worker rechecks already reconciled rows when state checks are newly enabled, or otherwise a documented skip.
If skipped because reconciled is terminal and not eligible for state-check enrichment, patch eligibility minimally to allow rechecking reconciled rows only when state-check evidence is missing.
Do not manually delete reconciliation row.
Do not create duplicate reconciliation row.
Step 8 — Verify Reconciliation Details

Query:

psql "$DATABASE_URL" <<'SQL'
select
  id,
  status,
  event_check_status,
  nonce_check_status,
  position_check_status,
  error,
  checked_at_ms,
  details
from option_execution_reconciliations
where option_execution_transaction_id = 'cae8c7e7-ed61-4265-aa7d-75edd94ef03c';
SQL

Expected:

status remains reconciled
details include state_checks
buyer nonce status ok
seller nonce status ok
buyer position status ok
seller position status ok
open interest observed if available
vault balances observed or skipped with reason

If state checks are missing because eligibility skipped the row, patch as described.

Step 9 — Verify Lifecycle Endpoint

Call:

curl -s \
  -H "X-Admin-Token: $ADMIN_TOKEN" \
  http://127.0.0.1:8080/admin/options/executions/e6d2941b-65f7-413a-958f-74ab22c53b08/lifecycle | jq '.state_checks, .health'

Expected:

lifecycle includes state_checks
health.stage remains reconciled
health.is_terminal_success = true
warnings empty or only documented non-strict skips
no errors
Step 10 — Verify Admin Reconciliation Endpoint

Call:

curl -s http://127.0.0.1:8080/admin/options/reconciliations \
  -H "X-Admin-Token: $ADMIN_TOKEN" | jq

Expected:

state-check config visible
check_counts includes nonce/position statuses
recent V1S row includes state-check evidence
Step 11 — Idempotency Check

Call tick again only if implementation safely skips/enriches idempotently:

curl -s -X POST http://127.0.0.1:8080/admin/options/reconciliations/tick \
  -H "X-Admin-Token: $ADMIN_TOKEN" | jq

Expected:

no duplicate reconciliation row
V1S remains reconciled
state check details stable or updated idempotently

Verify:

psql "$DATABASE_URL" -c "
select count(*)
from option_execution_reconciliations
where option_execution_transaction_id = 'cae8c7e7-ed61-4265-aa7d-75edd94ef03c';
"

Expected:

count = 1
Step 12 — No Forbidden Mutation Check

Run:

psql "$DATABASE_URL" <<SQL
select count(*) as option_txs_since_start
from option_execution_transactions
where created_at_ms >= ${V2B_LIVE_START_MS};

select count(*) as generic_txs_since_start
from execution_transactions
where created_at_ms >= ${V2B_LIVE_START_MS};

select count(*) as intents_since_start
from option_execution_intents
where created_at_ms >= ${V2B_LIVE_START_MS};
SQL

Expected:

all 0
Required Doc Update

Update:

docs/OPTION_ONCHAIN_STATE_CROSS_CHECKS_V2B.md

Add:

## Live V1S State-Check Result

Include:

direct cast nonce reads
direct cast position reads
open interest read if done
tick response
reconciliation details state_checks
lifecycle state_checks result
admin reconciliation summary
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

live nonce reads performed.
live position reads performed.
reconciliation details include state_checks or exact blocker documented.
lifecycle endpoint exposes state_checks.
V1S remains reconciled.
no broadcast.
no new option tx.
no generic tx.
no new intent.
docs updated.
validations pass.
Final Report

Return:

direct on-chain nonce results
direct on-chain position results
open interest/vault observations if available
reconciliation tick response
final reconciliation state-check details
lifecycle state-check result
admin reconciliation summary
idempotency result
no forbidden mutation verification
validation commands run
docs updated
remaining blocker