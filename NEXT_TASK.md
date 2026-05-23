# NEXT_TASK.md — Option Confirmation Worker Observability V1W

## Context

V1V implemented the background option confirmation worker.

Current state:
- Worker disabled by default.
- Worker can poll pending option execution transactions.
- Worker reads `eth_blockNumber`.
- Worker reads `eth_getTransactionReceipt`.
- Worker applies finality threshold.
- Worker transitions:
  - `broadcast_submitted` → `broadcast_confirmed` on `mined_success`
  - `broadcast_submitted` → `broadcast_failed` on `mined_failed`
- No broadcast path is touched.
- No generic execution rows are created.
- Validation passed with 504 tests.

Deferred from V1V:
1. Persist `gas_used` and `effective_gas_price`.
2. Add operator/admin visibility for latest worker tick.
3. Improve observability without building a full indexer.

## Goal

Improve production observability of the option confirmation worker.

This task must:
1. persist receipt cost fields;
2. expose pending/confirmed/failed confirmation summaries in admin endpoints;
3. expose the latest worker tick result in memory;
4. document operator usage.

This is backend-only.

## Hard Rules

Do not broadcast.
Do not retry.
Do not submit transactions.
Do not call `/executor/broadcast`.
Do not call `POST /options/execution-intents/:id/broadcast`.
Do not create new option execution intents.
Do not create option_execution_transactions except in tests.
Do not create generic execution_transactions.
Do not cleanup evidence rows.
Do not modify Solidity.
Do not modify frontend.
Do not deploy contracts.
Do not print private keys.
Do not touch real `.env` secrets.

## Required Migration

Add a migration if these fields are missing from `option_execution_transactions`:

```text
gas_used
effective_gas_price
cumulative_gas_used
receipt_block_hash
receipt_transaction_index
receipt_observed_at_ms

Use nullable columns and ADD COLUMN IF NOT EXISTS.

Do not break existing V1S/V1T rows.

Required Receipt Persistence

Extend receipt parsing and worker update logic to persist, when available:

gas_used
effective_gas_price
cumulative_gas_used
block_hash
transaction_index
observed timestamp

Manual confirmation endpoint should also use the same persistence path if practical.

Do not duplicate confirmation logic unnecessarily.

Required Admin Summary Endpoint

Add or extend an admin endpoint, preferably:

GET /admin/options/confirmations

Return sanitized summary:

{
  "worker_enabled": false,
  "poll_interval_ms": 15000,
  "finality_blocks": 3,
  "batch_size": 25,
  "pending": 0,
  "pending_finality": 0,
  "receipt_missing": 0,
  "receipt_error": 0,
  "mined_success": 1,
  "mined_failed": 0,
  "latest_tick": {
    "ran_at_ms": 0,
    "head_block": null,
    "scanned": 0,
    "finalized_success": 0,
    "finalized_failed": 0,
    "pending_finality": 0,
    "receipt_missing": 0,
    "receipt_errors": 0,
    "error": null
  }
}

No secrets.

Required Latest Tick State

Store latest worker tick result in app state.

Requirements:

in-memory only is acceptable
thread-safe
readable by admin endpoint
no DB write needed for tick metadata
if worker disabled, endpoint should still return config and null/empty latest tick
Required Repository Summary

Add repository method to count option execution transactions by confirmation status.

Suggested categories:

pending/null
pending
pending_finality
receipt_missing
receipt_error
mined_success
mined_failed
mined_reverted if still backward compatible
Required Tests

Add tests for:

receipt persistence stores gas fields.
confirmation worker stores gas fields on mined_success.
admin summary endpoint returns worker config.
admin summary endpoint returns counts.
latest tick state updates after worker tick.
worker disabled still returns safe admin summary.
no generic execution rows created.
no broadcast path used.

Use mocks. Do not depend on live RPC.

Required Docs

Create:

docs/OPTION_CONFIRMATION_OBSERVABILITY_V1W.md

Include:

what V1V did
what V1W adds
migration summary
receipt cost fields
admin endpoint shape
latest tick meaning
operator checklist
remaining deferred work:
event indexer
full reconciliation worker
alerting
settlement/exercise/expiry
reorg handling beyond simple finality threshold
Validation

Run:

cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
cargo build --all-targets --all-features
Acceptance Criteria

Complete only if:

receipt gas/cost fields are persisted
admin confirmation summary exists
latest worker tick is visible
worker remains disabled by default
no broadcast path touched
tests pass
docs created
no chain transaction submitted
Final Report

Return:

files changed
migration added or not
receipt fields added
admin endpoint added
latest tick behavior
tests added
docs created
validation commands run
no forbidden mutation verification
remaining blocker
