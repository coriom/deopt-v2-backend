# NEXT_TASK.md — Tiny Option Trade Preflight Against NEW MarginEngine V2D-U

## Context

MarginEngineV2 deploy, rewire, backend cutover, and indexer catch-up are complete.

Current live state:
- OLD_MARGIN_ENGINE = 0x6C5665De05e7314cB63cD77F82DFa86508A5b5F8
- NEW_MARGIN_ENGINE = 0x287Cef479be5889eEfCa847F9e73C860898f48Cc
- backend MARGIN_ENGINE = NEW
- OPTION_EVENT_INDEXER_MARGIN_ENGINE_ADDRESS = NEW
- event indexer cursor = 42077113
- cutover block = 42073772
- FeesManagerV2 disabled:
  - NEW.useFeesManagerV2 = false
  - NEW.feesManagerV2 = address(0)
- V1 FeesManager remains active:
  - NEW.feesManager = 0xaef73F10224712E1312963BE11662061481aA0F0

V2D-T2 confirmed:
- catch-up complete
- no option intents created
- no option txs created
- no generic txs created
- no new option events
- backend stopped in safe mode

Goal:
Prepare a tiny option execution preflight against NEW_MARGIN_ENGINE.

This task must:
1. start backend with required workers/surfaces configured;
2. verify NEW margin engine live state;
3. refresh oracle if needed;
4. create a fresh tiny option execution intent;
5. collect signatures;
6. build calldata;
7. simulate execution against NEW;
8. verify gas safety;
9. stop before broadcast.

No live broadcast in this task.

## Hard Rules

Do not broadcast.
Do not submit transactions.
Do not call `/options/execution-intents/:id/broadcast`.
Do not call `/executor/broadcast`.
Do not call `eth_sendRawTransaction`.
Do not deploy.
Do not modify Solidity.
Do not modify frontend.
Do not enable FeesManagerV2.
Do not deploy FeesManagerV2.
Do not call setUseFeesManagerV2.
Do not cleanup historical rows.
Do not print private keys.
Do not commit real `.env`.

Allowed:
- backend runtime env update.
- backend restart.
- oracle refresh only if existing testnet mock-feed refresh script is already part of the established workflow.
- create one fresh tiny option execution intent.
- sign buyer/seller payloads locally if existing flow requires it.
- simulate.
- gas estimate / gas safety preview.
- docs.

## Required Runtime Env

Work in:

```text
~/DEOPT/deopt-v2-backend

Load env without printing secrets:

set -a
source .env.cutover.v2d_s.local 2>/dev/null || source .env
set +a

Then export runtime values:

export RPC_URL="<paid Base Sepolia RPC already set in shell>"

export MARGIN_ENGINE=0x287Cef479be5889eEfCa847F9e73C860898f48Cc
export OPTION_EVENT_INDEXER_MARGIN_ENGINE_ADDRESS=0x287Cef479be5889eEfCa847F9e73C860898f48Cc
export OLD_MARGIN_ENGINE=0x6C5665De05e7314cB63cD77F82DFa86508A5b5F8
export MARGIN_ENGINE_CUTOVER_BLOCK=42073772

export OPTION_EVENT_INDEXER_ENABLED=true
export OPTION_EVENT_INDEXER_REQUIRE_RPC=true
export OPTION_EVENT_INDEXER_CONFIRMATION_BLOCKS=3
export OPTION_EVENT_INDEXER_BATCH_BLOCKS=5000

export OPTION_CONFIRMATION_WORKER_ENABLED=true
export OPTION_RECONCILIATION_WORKER_ENABLED=true

export OPTION_EXECUTION_BROADCAST_ENABLED=true
export EXECUTION_ENABLED=true
export EXECUTOR_REAL_BROADCAST_ENABLED=true
export EXECUTOR_DRY_RUN=false

export OPTION_EXECUTION_BROADCAST_GAS_LIMIT=1500000
export EXECUTION_GAS_SAFETY_BPS=12500

Important:

These flags allow the backend to prepare/simulate/broadcast surfaces.
This task still must not call the broadcast endpoint.
Step 1 — Baseline

Set:

V2D_U_START_MS=$(date +%s%3N)

Record DB counts:

select count(*) as option_execution_intents from option_execution_intents;
select count(*) as option_execution_transactions from option_execution_transactions;
select count(*) as execution_transactions from execution_transactions;
select count(*) as option_execution_events from option_execution_events;
select count(*) as option_execution_reconciliations from option_execution_reconciliations;
select count(*) as fee_events from fee_events;

Also record latest event indexer cursor.

Step 2 — Start Backend

Start backend.

Verify:

curl -s http://127.0.0.1:8080/health | jq
curl -s http://127.0.0.1:8080/admin/config -H "X-Admin-Token: $ADMIN_TOKEN" | jq

Expected:

backend healthy.
margin engine = NEW.
event indexer margin engine = NEW.
broadcast surfaces configured but not called.
FeesManagerV2 absent/disabled.
Step 3 — On-chain NEW Checks

Run read-only checks:

cast call 0x287Cef479be5889eEfCa847F9e73C860898f48Cc "feesManager()(address)" --rpc-url "$RPC_URL"
cast call 0x287Cef479be5889eEfCa847F9e73C860898f48Cc "feesManagerV2()(address)" --rpc-url "$RPC_URL"
cast call 0x287Cef479be5889eEfCa847F9e73C860898f48Cc "useFeesManagerV2()(bool)" --rpc-url "$RPC_URL"

Expected:

feesManager = 0xaef73F10224712E1312963BE11662061481aA0F0
feesManagerV2 = 0x0000000000000000000000000000000000000000
useFeesManagerV2 = false
Step 4 — Oracle Freshness

Check WETH/USDC oracle freshness with existing read-only/preflight method.

If stale, refresh testnet mock feeds using the existing established script only.

Expected:

getPriceSafe(WETH,USDC) returns ok=true.
age under feed max delay.
no option trade attempted if oracle stale.
Step 5 — Create Fresh Tiny Intent

Create a fresh tiny option trade intent using the same established option endpoint flow as V1S.

Use conservative tiny size:

quantity = 1
premium small
existing listed/valid option series
buyer/seller accounts already funded from prior V1S flow if still valid.

Record:

intent_id
source_id
buyer
seller
option_id
premium
quantity
buyer/seller nonces
Step 6 — Signatures / Calldata / Simulation

Use existing flow:

fetch EIP-712 payload.
sign buyer.
sign seller.
submit strict signatures.
fetch calldata.
simulate.

Expected:

calldata selector expected.
simulation_status = simulation_ok.
no revert.
simulated block recorded.
no broadcast.
Step 7 — Gas Safety Preview

Estimate gas and compute:

required_gas = estimated_gas * EXECUTION_GAS_SAFETY_BPS / 10000

Expected:

OPTION_EXECUTION_BROADCAST_GAS_LIMIT >= required_gas
gas safety status = ok
executor balance enough
Step 8 — No Forbidden Mutation Check

Allowed new rows:

one new option_execution_intent.
signature/calldata/simulation records associated with the new intent.

Forbidden:

no option_execution_transactions.
no execution_transactions.
no broadcast tx hash.
no confirmed/reconciled row for the new intent.

Query:

select count(*) from option_execution_transactions where created_at_ms >= ${V2D_U_START_MS};
select count(*) from execution_transactions where created_at_ms >= ${V2D_U_START_MS};

Expected:

both 0.
Required Docs

Create:

docs/MARGIN_ENGINE_V2_TINY_TRADE_PREFLIGHT_V2D_U.md

Include:

runtime config summary.
baseline DB counts.
NEW engine checks.
oracle freshness.
new intent id.
signature/calldata status.
simulation result.
gas safety preview.
no-broadcast proof.
remaining blocker before human broadcast.
Validation

Run:

cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
cargo build --all-targets --all-features
Acceptance Criteria

Complete only if:

backend reports NEW.
NEW has V2 disabled.
oracle fresh.
fresh tiny intent created.
signatures accepted.
calldata ready.
simulation_ok.
gas safety ok.
no broadcast endpoint called.
no option_execution_transactions created.
no generic execution_transactions created.
docs created.
validations pass.
Final Report

Return:

backend config summary.
NEW engine checks.
oracle status.
tiny intent id.
signature/calldata status.
simulation result.
gas safety preview.
no-broadcast verification.
docs updated.
validation commands run.
remaining blocker before human tiny broadcast.