# NEXT_TASK.md — Human Tiny Option Broadcast Against NEW MarginEngine V2D-V

## Context

V2D-U completed tiny option trade preflight against NEW MarginEngine.

NEW_MARGIN_ENGINE:
0x287Cef479be5889eEfCa847F9e73C860898f48Cc

OptionMatchingEngine:
0xf2D1D85cD363Be3bc160d14883C80e7C2c4F420b

FeesManagerV2 remains disabled:
- NEW.useFeesManagerV2 = false
- NEW.feesManagerV2 = address(0)
- NEW.feesManager = V1 FeesManager

Valid tiny intent:
a6369dd5-54cd-4407-a4c5-7902bba286f7

Invalid/stale intent to ignore:
563d5884-... with buyer/seller nonces 0/0

Preflight result:
- signatures accepted
- calldata ready
- simulation_ok at block 42100183
- estimated_gas = 938,846
- required_gas = 1,173,557
- broadcast_gas_limit = 1,500,000
- gas_check_status = ok
- executor balance = 0.00807 ETH
- no broadcast called

Important:
This task involves a real on-chain broadcast. The agent must not perform the broadcast. The human operator performs it manually only after final checks.

## Goal

Run final pre-broadcast checks, stop for human broadcast, then verify and document the tiny trade execution against NEW MarginEngine.

## Hard Rules For Agent

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
- backend runtime env setup.
- read-only preflight checks.
- oracle refresh only if stale, using existing established testnet mock-feed refresh flow.
- simulation refresh.
- gas estimate refresh.
- stop for human broadcast.
- parse human broadcast output.
- post-broadcast verification.
- docs.

## Phase 1 — Pre-Broadcast Refresh

Work in:

cd ~/DEOPT/deopt-v2-backend

Load env without printing secrets:

set -a
source .env.cutover.v2d_s.local 2>/dev/null || source .env
set +a

Export runtime values:

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

The human must provide EXECUTOR_PRIVATE_KEY in the local terminal only:

read -s EXECUTOR_PRIVATE_KEY
export EXECUTOR_PRIVATE_KEY
test -n "$EXECUTOR_PRIVATE_KEY" && echo "EXECUTOR_PRIVATE_KEY set"

Never paste this key into chat.

Start backend.

Verify /admin/config:

margin engine = NEW
broadcast enabled true
real broadcast enabled true
dry_run false
gas limit = 1,500,000
gas safety = 12,500
FeesManagerV2 disabled
Phase 2 — Final Safety Checks

Before broadcast, verify:

1. Correct intent
intent_id = a6369dd5-54cd-4407-a4c5-7902bba286f7

Reject stale intent:

563d5884-...
2. Oracle freshness

Check WETH/USDC oracle freshness.

If stale:

refresh mock feeds using existing established script.
then re-check.

Abort if oracle cannot be made fresh.

3. Nonces

Read on-chain nonces for buyer/seller.

Expected:

buyer nonce = 1
seller nonce = 1

If either nonce differs:

abort.
do not broadcast.
regenerate fresh intent/signatures in a separate task.
4. Re-simulate

Call:

curl -s -X POST \
  http://127.0.0.1:8080/admin/options/executions/a6369dd5-54cd-4407-a4c5-7902bba286f7/simulate \
  -H "X-Admin-Token: $ADMIN_TOKEN" | jq

Expected:

simulation_status = simulation_ok
error = null
revert_selector = null
5. Re-estimate gas

Expected:

estimated gas around 938,846
required gas <= 1,500,000
gas_check_status = ok
6. Baseline counts

Set:

V2D_V_START_MS=$(date +%s%3N)

Record:

option_execution_transactions since start = 0
execution_transactions since start = 0
Phase 3 — Stop For Human Broadcast

Agent stops and tells human operator to run exactly one command:

curl -s -X POST \
  http://127.0.0.1:8080/options/execution-intents/a6369dd5-54cd-4407-a4c5-7902bba286f7/broadcast \
  -H "X-Admin-Token: $ADMIN_TOKEN" | jq

Rules:

run once only.
no retry.
do not call /executor/broadcast.
paste full output back.
Phase 4 — Post-Broadcast Verification

After operator pastes output:

Extract:

tx hash
option_execution_transaction id
broadcast status
gas used if available.

Verify chain receipt:

cast receipt <TX_HASH> --rpc-url "$RPC_URL"

Expected:

status = 1
to = OptionMatchingEngine
block number recorded

Verify backend transaction row:

confirmation_status
receipt_status
tx_hash
gas_used
effective_gas_price

Run confirmation tick if needed.

Run event indexer tick from tx block if needed.

Run reconciliation tick.

Expected final:

intent status = broadcast_confirmed
tx = mined_success
event indexer captures NEW MarginEngine events.
reconciliation status = reconciled.
lifecycle shows:
health.stage = reconciled
confirmation mined_success
events total > 0
fees source = onchain
state_checks ok or explicit non-strict result
FeesManagerV2 still disabled
V1 fee events only or V1-compatible events
Required Docs

Create:

docs/MARGIN_ENGINE_V2_TINY_TRADE_BROADCAST_RESULT_V2D_V.md

Update:

docs/MARGIN_ENGINE_V2_TINY_TRADE_PREFLIGHT_V2D_U.md

Include:

pre-broadcast refresh.
human broadcast output.
tx hash.
receipt.
confirmation.
events indexed.
reconciliation.
lifecycle result.
no forbidden mutation proof.
remaining blocker before FeesManagerV2 enablement.
Validation

Run:

cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
cargo build --all-targets --all-features
Acceptance Criteria

Complete only if:

final simulation_ok.
final gas safety ok.
human broadcast called exactly once.
tx mined success.
backend confirms tx.
events indexed.
reconciliation done.
lifecycle reconciled.
FeesManagerV2 remains disabled.
docs created.
validations pass.
Final Report

Return:

pre-broadcast checks.
broadcast output summary.
tx hash.
receipt summary.
backend transaction status.
indexed events.
reconciliation result.
lifecycle result.
docs updated.
validation commands run.
remaining blocker before FeesManagerV2 enablement.