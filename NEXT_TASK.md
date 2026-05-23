# NEXT_TASK.md — Option Execution Reconciliation Worker V1Y

## Context

V1S successfully executed the first live option trade on Base Sepolia.

Tx:
0x5964a7b3d2c18d051baaa780413d31c44d419ce530f45263cb4c46f720881125

Intent:
e6d2941b-65f7-413a-958f-74ab22c53b08

Transaction row:
cae8c7e7-ed61-4265-aa7d-75edd94ef03c

V1T manually confirmed and reconciled the trade:
- receipt status = 1
- intent status = broadcast_confirmed
- tx confirmation_status = mined_success
- buyer nonce advanced
- seller nonce advanced
- buyer option position +1
- seller option position -1
- buyer fee = 6
- seller fee = 4
- premium/vault movements reconciled manually

V1V added background confirmation worker.

V1W added confirmation observability and receipt cost persistence.

V1X added option event indexer.

V1X-B added multi-emitter event coverage:
- OptionMatchingEngine
- MarginEngine
- CollateralVault
- optional FeesManager

V1X-C validated live backfill for V1S:
- OptionTradeExecuted indexed
- TradeExecuted indexed
- two TradingFeeCharged events indexed
- InternalTransfer events indexed
- Synced events indexed
- events linked to tx row and intent by tx_hash
- idempotency verified

Remaining gap:
The backend has confirmation and event indexing, but no automatic reconciliation worker.

## Goal

Implement an option execution reconciliation worker.

The worker should verify that confirmed option executions match expected backend intent data and observed on-chain events/state.

This is backend-only.

## Hard Rules

Do not broadcast.
Do not retry.
Do not submit transactions.
Do not call `/executor/broadcast`.
Do not call `POST /options/execution-intents/:id/broadcast`.
Do not create new option execution intents.
Do not create new option_execution_transactions except in tests.
Do not create generic execution_transactions.
Do not cleanup existing evidence rows.
Do not modify Solidity.
Do not modify frontend.
Do not deploy contracts.
Do not print private keys.
Do not touch real `.env` secrets.

Allowed mutations:
- migrations for reconciliation fields/tables
- reconciliation status updates for already confirmed option executions
- tests fixtures only

## Required Config

Add safe config:

```text
OPTION_RECONCILIATION_WORKER_ENABLED=false
OPTION_RECONCILIATION_POLL_INTERVAL_MS=15000
OPTION_RECONCILIATION_BATCH_SIZE=25
OPTION_RECONCILIATION_REQUIRE_EVENTS=true
OPTION_RECONCILIATION_REQUIRE_RPC=true
OPTION_RECONCILIATION_STRICT=true

Defaults:

disabled by default
strict by default
no reconciliation if required events are missing
no mutation if RPC unavailable and require_rpc=true

Expose sanitized values in /admin/config.

Required DB

Add reconciliation fields or table.

Preferred table:

option_execution_reconciliations

Suggested columns:

id UUID primary key
intent_id UUID not null
option_execution_transaction_id UUID not null
tx_hash TEXT not null
chain_id BIGINT not null
status TEXT not null
checked_at_ms BIGINT not null
receipt_status TEXT null
event_check_status TEXT null
state_check_status TEXT null
fee_check_status TEXT null
premium_check_status TEXT null
nonce_check_status TEXT null
position_check_status TEXT null
error TEXT null
details JSONB null
created_at_ms BIGINT not null
updated_at_ms BIGINT not null

Statuses:

pending
reconciled
reconciliation_failed
missing_events
state_mismatch
fee_mismatch
premium_mismatch
rpc_error

Add uniqueness:

(chain_id, tx_hash)

Do not break existing V1S/V1T rows.

Required Repository Methods

Add methods to:

list confirmed but unreconciled option executions:
transaction confirmation_status = mined_success
intent status = broadcast_confirmed
no reconciliation row with status = reconciled
batch limited
fetch expected intent data:
buyer
seller
option_id
quantity
premium
buyer/seller nonces
onchain_intent_id
calldata/selector if useful
fetch indexed events for tx_hash:
OptionTradeExecuted
TradeExecuted
TradingFeeCharged
InternalTransfer
other relevant indexed events
insert/update reconciliation result idempotently.
Required Worker Behavior

Worker tick:

If disabled, do nothing.
Find confirmed unreconciled option executions.
For each:
load intent
load tx row
load indexed events
validate required events
compare event data to expected intent
optionally read on-chain state for positions/nonces if RPC enabled
write reconciliation result

Never rebroadcast.
Never mark failed txs as reconciled.
Never create generic execution rows.

Required Event Checks

For the tx being reconciled, verify:

OptionTradeExecuted

Must match expected:

onchain_intent_id
buyer
seller
option_id
quantity
premium
buyer_is_maker if available
TradeExecuted / Margin event

Verify if present:

buyer/seller or accounts
option_id
quantity direction
premium
TradingFeeCharged

Verify:

two fee events expected if both buyer and seller charged
fee payer addresses
fee recipient
fee amounts present
fee asset if available

Do not hardcode fee amounts globally. For V1S, expected observed amounts are buyer fee 6 and seller fee 4, but generic reconciliation should derive from indexed events / intent / configured fee logic if available.

InternalTransfer / Vault Events

Verify:

premium movement buyer -> seller
fee movement payer -> recipient
asset address if available
amounts if available

If exact event shape is insufficient, mark as partial and document.

Required On-chain State Checks

If RPC enabled, read:

buyer nonce
seller nonce
buyer option position
seller option position

Use existing contract views.

If exact position view names are uncertain, search Solidity sources:

rg "function .*position|positions|getPosition|openInterest|nonces" ../deopt-v2-sol/src

For V1S expected:

buyer nonce >= intent buyer nonce + 1
seller nonce >= intent seller nonce + 1
buyer position includes +1
seller position includes -1

Do not invent state if view unavailable. If view is missing, document and make event-only reconciliation possible under non-strict mode only.

Required Admin Endpoint

Add:

GET /admin/options/reconciliations

Return sanitized summary:

{
  "worker_enabled": false,
  "strict": true,
  "require_events": true,
  "require_rpc": true,
  "pending": 0,
  "reconciled": 1,
  "reconciliation_failed": 0,
  "latest_tick": {
    "ran_at_ms": 0,
    "scanned": 0,
    "reconciled": 0,
    "failed": 0,
    "missing_events": 0,
    "rpc_errors": 0,
    "error": null
  },
  "recent": []
}

No secrets.

Required Latest Tick State

Store latest worker tick in memory:

ran_at_ms
scanned
reconciled
failed
missing_events
state_mismatches
fee_mismatches
premium_mismatches
rpc_errors
error

Expose via admin endpoint.

Required Tests

Add tests for:

worker disabled does nothing.
confirmed tx with missing events becomes missing_events or remains pending depending config.
confirmed tx with matching OptionTradeExecuted reconciles.
mismatched buyer/seller fails.
mismatched option_id fails.
mismatched quantity fails.
fee events are included in reconciliation details.
same tx reconciliation is idempotent.
admin endpoint returns counts/latest tick.
worker never calls broadcast.
worker never creates generic execution rows.

Use mocks. Do not depend on live Base Sepolia.

Required Optional Validation Against V1S

Add a safe one-shot reconciliation path or admin-safe tick if practical.

If live DB contains V1S indexed events, run one reconciliation tick for V1S only.

Allowed:

insert/update reconciliation row for V1S.

Forbidden:

any broadcast
any new option tx
any generic execution tx

If live one-shot is not run, document manual command/path.

Required Docs

Create:

docs/OPTION_RECONCILIATION_WORKER_V1Y.md

Include:

why confirmation is not enough
reconciliation inputs
DB schema
worker lifecycle
event checks
state checks
fee/premium checks
strict vs non-strict behavior
admin endpoint
relation to V1S/V1T/V1V/V1W/V1X
remaining deferred work:
frontend lifecycle display
advanced fee reconciliation
settlement/exercise/expiry indexing
reorg handling
alerting
Validation

Run:

cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
cargo build --all-targets --all-features
Acceptance Criteria

Complete only if:

reconciliation worker exists
disabled by default
confirmed mined_success option txs can be reconciled
indexed events are used
mismatches fail loudly
reconciliation result is persisted
admin endpoint exists
tests pass
docs created
no broadcast path touched
no transaction submitted
Final Report

Return:

files changed
migrations added
config added
reconciliation worker behavior
DB schema summary
event checks implemented
state checks implemented
admin endpoint added
tests added
docs created
validation commands run
no forbidden mutation verification
whether V1S was reconciled by the worker or only test-covered
remaining blocker
