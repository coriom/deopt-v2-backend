# NEXT_TASK.md — Backend FeesManagerV2 Event Decoding And Lifecycle Support V2D-E

## Context

V2D-D integrated FeesManagerV2 into the option MarginEngine path.

Solidity now supports:
- V1/V2 coexistence
- V1 default behavior unchanged
- V2 option fees when enabled
- option premium basis
- positive fees trader -> feeRecipient
- maker rebates rebateFundingAccount -> trader
- V2 events emitted by FeesManagerV2
- V1-compatible `TradingFeeCharged` event still emitted for positive fees

Remaining gap:
The backend currently indexes V1-style `TradingFeeCharged` events and lifecycle fees, but does not yet decode or summarize the new FeesManagerV2 events.

## Goal

Add backend support for FeesManagerV2 events.

This is backend-only.

Do not deploy.
Do not broadcast.
Do not modify Solidity.
Do not modify frontend.

## Hard Rules

Do not submit transactions.
Do not broadcast.
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
Do not touch `.env`.
Do not print secrets.

Allowed:
- backend event decoder updates
- backend admin/lifecycle read model updates
- tests
- docs

## Required Solidity Event Audit

Read current Solidity events from:

```text
../deopt-v2-sol/src/fees/IFeesManagerV2.sol
../deopt-v2-sol/src/fees/FeesManagerV2.sol

Extract exact event signatures for:

FeeChargedV2
FeeRebatedV2
RebateBudgetFunded
RebateBudgetWithdrawn
RebateBudgetSpent
FeeRecipientSet
FeeConsumerSet
MerkleRootSet
TierClaimed

Do not invent signatures.

Required Backend Event Indexer Updates

Update option event indexer to decode V2 events if emitted by configured fees_manager.

Existing indexer already supports optional FeesManager.

Add decoded support for:

FeeChargedV2
FeeRebatedV2
RebateBudgetFunded
RebateBudgetWithdrawn
RebateBudgetSpent
FeeRecipientSet
FeeConsumerSet
MerkleRootSet
TierClaimed

Persist in option_execution_events with:

event_name
event_signature
contract_address
tx_hash
log_index
decoded JSON
account/trader if available
recipient if available
fee/rebate amount if available
settlement asset if available
product kind
flow kind
isMaker
feePpm / rebatePpm
basisAmount

Do not break V1 decoding.

Required Fee Read Model Updates

Update lifecycle fee section to support both:

V1: TradingFeeCharged
V2: FeeChargedV2 + FeeRebatedV2

Lifecycle fees should expose:

{
  "source_of_truth": "onchain",
  "event_model": "v1" | "v2" | "mixed",
  "observed_total_charged": "...",
  "observed_total_rebated": "...",
  "net_protocol_fee": "...",
  "trading_fee_event_count": 0,
  "fee_charged_v2_count": 0,
  "fee_rebated_v2_count": 0,
  "by_trader": {},
  "by_recipient": {},
  "by_side": {},
  "backend_ledger_status": "...",
  "reconciliation_status": "..."
}

Compatibility:

keep existing fields if frontend currently uses them.
do not remove observed_total.
for V1-only rows, behavior remains unchanged.
Required Admin Endpoint Updates

Update:

GET /admin/fees/onchain

It must support:

V1 TradingFeeCharged
V2 FeeChargedV2
V2 FeeRebatedV2

Return:

total charged
total rebated
net fee
counts by event model
by trader
by recipient
by side
recent fee/rebate events

Do not break existing response fields.

Required Reconciliation Updates

Extend fee reconciliation details to include V2 events.

If a tx has:

only V1 events -> existing behavior.
only V2 events -> use V2 events.
both V1 compatibility and V2 events -> avoid double-counting.

Important:
If MarginEngine emits V1-compatible TradingFeeCharged and FeesManagerV2 emits FeeChargedV2, backend must not count both as separate protocol fee payments unless design explicitly requires it.

Recommended:

prefer V2 events when present.
include V1 events as compatibility evidence.
set event_model = "mixed" and source_priority = "v2".
Required Tests

Add tests for:

decode FeeChargedV2.
decode FeeRebatedV2.
decode rebate budget events.
lifecycle V1-only fees unchanged.
lifecycle V2-only positive fee summary.
lifecycle V2 rebate summary.
mixed V1+V2 avoids double-counting.
/admin/fees/onchain returns V2 charged/rebated totals.
no generic execution rows created.
no broadcast path touched.

Use fixtures/mocks. Do not depend on live deployment.

Required Docs

Create:

docs/FEES_MANAGER_V2_BACKEND_EVENT_SUPPORT_V2D_E.md

Include:

V2 events decoded
source-of-truth rules
V1/V2 compatibility
double-count prevention
lifecycle changes
admin endpoint changes
remaining deferred work:
live Base Sepolia deployment/wiring
V2 fee drift checks
perps integration
RFQ flow support
frontend fee dashboard update
Validation

Run:

cd ~/DEOPT/deopt-v2-backend
cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
cargo build --all-targets --all-features
Acceptance Criteria

Complete only if:

backend decodes FeesManagerV2 events.
lifecycle supports V1 and V2 fee event models.
admin fee endpoint supports V2 fees and rebates.
mixed V1/V2 events avoid double-counting.
tests pass.
docs created.
no Solidity/frontend changes.
no transaction submitted.
Final Report

Return:

files changed
V2 events decoded
lifecycle fee model update
admin fee endpoint update
double-counting policy
tests added
docs created
validation commands run
no forbidden mutation verification
remaining blocker
