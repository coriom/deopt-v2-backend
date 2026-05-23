# NEXT_TASK.md — Option Broadcast Confirmation Worker V1V

## Context

V1S successfully submitted the first live option execution broadcast.

Tx:
0x5964a7b3d2c18d051baaa780413d31c44d419ce530f45263cb4c46f720881125

Intent:
e6d2941b-65f7-413a-958f-74ab22c53b08

V1T confirmed and reconciled it manually:
- intent status: broadcast_confirmed
- option transaction confirmation_status: mined_success
- buyer position +1
- seller position -1
- vault premium/fee movements reconciled

The backend now has a confirmation path, but confirmation is still manual.

## Goal

Implement a background option execution confirmation worker.

The worker should automatically detect submitted option execution transactions, read receipts, apply a configurable finality threshold, and update transaction/intent statuses safely.

This task must not broadcast or submit any transaction.

## Hard Rules

Do not broadcast.
Do not retry.
Do not submit transactions.
Do not call `/executor/broadcast`.
Do not call `POST /options/execution-intents/:id/broadcast`.
Do not create new option execution intents.
Do not create new option_execution_transactions except via tests.
Do not create generic execution_transactions.
Do not cleanup existing evidence rows.
Do not modify Solidity.
Do not modify frontend.
Do not deploy contracts.
Do not print private keys.
Do not touch real `.env` secrets.

## Required Config

Add safe config flags:

```text
OPTION_CONFIRMATION_WORKER_ENABLED=false
OPTION_CONFIRMATION_POLL_INTERVAL_MS=15000
OPTION_CONFIRMATION_FINALITY_BLOCKS=3
OPTION_CONFIRMATION_BATCH_SIZE=25
OPTION_CONFIRMATION_REQUIRE_RPC=true

Defaults must be safe:

worker disabled by default
RPC required if enabled
no mutation if RPC unavailable

Expose sanitized values in /admin/config.

Required DB/Repository Behavior

Add repository methods to:

list pending option execution transactions:
status submitted OR confirmation_status NULL/pending
has tx_hash
limit by batch size
update receipt observation:
receipt_status
block_number
block_hash
gas_used
effective_gas_price if available
observed_at_ms
update final confirmation:
confirmation_status = mined_success / mined_failed
confirmed_at_ms
confirmation_error if failed
update related intent:
broadcast_submitted → broadcast_confirmed on success
broadcast_submitted → broadcast_failed on mined failed tx
do not overwrite terminal states blindly

If needed, add migration only for missing fields.

Required Worker Behavior

Worker loop:

If disabled, do nothing.
If enabled:
list pending option execution txs.
for each tx_hash:
call eth_getTransactionReceipt.
if receipt missing:
leave pending.
if receipt exists:
persist receipt fields.
compare current chain head block.
if head_block - receipt_block < OPTION_CONFIRMATION_FINALITY_BLOCKS:
leave as observed/pending_finality.
if enough finality:
if receipt status = 1:
mark tx mined_success.
mark intent broadcast_confirmed.
if receipt status = 0:
mark tx mined_failed.
mark intent broadcast_failed.
Never rebroadcast.
Never create generic execution transaction rows.
Never retry failed txs.
Required RPC Support

Implement or reuse provider calls for:

eth_getTransactionReceipt
eth_blockNumber

Avoid overbuilding a full indexer.

Required Tests

Add tests for:

worker disabled does nothing.
submitted tx with no receipt remains pending.
receipt exists but finality not reached remains pending_finality/observed.
receipt status 1 + finality marks mined_success and intent broadcast_confirmed.
receipt status 0 + finality marks mined_failed and intent broadcast_failed.
worker never calls broadcast provider.
worker never creates generic execution_transactions rows.
config parsing defaults and overrides.

Use mock RPC provider.

Required Docs

Create:

docs/OPTION_CONFIRMATION_WORKER_V1V.md

Include:

why manual confirmation is insufficient
worker lifecycle
config variables
finality behavior
DB status transitions
failure behavior
no retry policy
how this relates to V1S/V1T
remaining work:
event indexer
full reconciliation worker
alerting
reorg handling beyond simple finality threshold
Validation

Run:

cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
cargo build --all-targets --all-features
Acceptance Criteria

Complete only if:

confirmation worker exists
worker disabled by default
finality threshold implemented
receipt polling implemented
submitted txs can become mined_success/mined_failed
related intent status updates safely
no broadcast path touched
tests pass
docs created
no chain transaction submitted
Final Report

Return:

files changed
migrations added or not
config added
worker behavior summary
status transition summary
tests added
docs created
validation commands run
no forbidden mutation verification
remaining blocker
