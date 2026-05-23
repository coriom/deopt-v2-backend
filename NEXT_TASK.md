# NEXT_TASK.md — Admin Option Execution Lifecycle Endpoint V1Z

## Context

The backend option execution pipeline is now validated end-to-end on Base Sepolia.

Validated live flow:

- V1S: first successful live option execution broadcast.
- V1T: manual confirmation/reconciliation.
- V1V: background confirmation worker.
- V1W: confirmation observability.
- V1X: option event indexer.
- V1X-B: multi-emitter event indexing.
- V1X-C: live event backfill validation.
- V1Y: reconciliation worker.
- V1Y-B: live event backfill + reconciliation retry.

V1S tx:

```text
0x5964a7b3d2c18d051baaa780413d31c44d419ce530f45263cb4c46f720881125

Intent:

e6d2941b-65f7-413a-958f-74ab22c53b08

Transaction row:

cae8c7e7-ed61-4265-aa7d-75edd94ef03c

Final live result:

V1S event rows indexed: 19.
Events indexed:
OptionTradeExecuted = 1
TradeExecuted = 1
TradingFeeCharged = 2
InternalTransfer = 3
Synced = 12
Reconciliation row status: reconciled.
No broadcast retries.
No generic execution transactions.
No forbidden mutations.

Remaining usability gap:
Admin/operator needs one endpoint that shows the full lifecycle of an option execution intent without manual SQL across many tables.

Goal

Add a read-only admin lifecycle endpoint for option execution intents.

Endpoint:

GET /admin/options/executions/:intent_id/lifecycle

It should aggregate:

intent metadata
signature status
simulation status
calldata/gas safety
broadcast transaction
receipt/confirmation
indexed events
reconciliation
fee events
internal transfer events
lifecycle health summary

This is backend-only and read-only.

Hard Rules

Do not broadcast.
Do not retry.
Do not submit transactions.
Do not call /executor/broadcast.
Do not call POST /options/execution-intents/:id/broadcast.
Do not create new option execution intents.
Do not create option_execution_transactions.
Do not create generic execution_transactions.
Do not cleanup evidence rows.
Do not modify Solidity.
Do not modify frontend.
Do not deploy contracts.
Do not print private keys.
Do not touch real .env secrets.

No DB mutation is allowed in this task except tests.

Required Endpoint

Add:

GET /admin/options/executions/:intent_id/lifecycle

Admin-authenticated.

No secrets.

Required Response Shape

Suggested response:

{
  "intent_id": "...",
  "status": "broadcast_confirmed",
  "source": {
    "source_type": "...",
    "source_id": "..."
  },
  "trade": {
    "buyer": "...",
    "seller": "...",
    "option_id": "...",
    "quantity_contracts": "...",
    "premium_per_contract_native": "...",
    "buyer_is_maker": false,
    "onchain_intent_id": "..."
  },
  "metadata": {
    "underlying": "...",
    "settlement_asset": "...",
    "expiry": 0,
    "strike": "...",
    "contract_size_1e8": "...",
    "is_call": true,
    "is_european": true
  },
  "signatures": {
    "buyer_signature_present": true,
    "seller_signature_present": true,
    "signature_mode": "strict"
  },
  "simulation": {
    "status": "simulation_ok",
    "block_number": 0,
    "simulated_at_ms": 0,
    "error": null,
    "revert_selector": null
  },
  "calldata": {
    "present": true,
    "selector": "0x031f77b3",
    "length": 0
  },
  "broadcast": {
    "transaction_id": "...",
    "tx_hash": "...",
    "status": "submitted",
    "from": "...",
    "to": "...",
    "gas_check_status": "ok",
    "estimated_gas": 0,
    "required_gas": 0,
    "broadcast_gas_limit": 0,
    "gas_safety_bps": 0
  },
  "confirmation": {
    "confirmation_status": "mined_success",
    "receipt_status": 1,
    "block_number": 0,
    "gas_used": 0,
    "effective_gas_price": 0,
    "confirmed_at_ms": 0
  },
  "events": {
    "total": 0,
    "counts_by_event_name": {},
    "counts_by_contract_address": {},
    "recent": []
  },
  "fees": {
    "trading_fee_event_count": 0,
    "events": [],
    "total_by_asset_or_raw": {}
  },
  "transfers": {
    "internal_transfer_count": 0,
    "events": []
  },
  "reconciliation": {
    "status": "reconciled",
    "event_check_status": "ok",
    "fee_check_status": "ok",
    "premium_check_status": "ok",
    "error": null,
    "checked_at_ms": 0,
    "details": {}
  },
  "health": {
    "stage": "reconciled",
    "is_terminal_success": true,
    "warnings": [],
    "errors": []
  }
}

Adapt exact field names to existing backend types.

Required Health Logic

Compute a simple lifecycle health summary.

Stages:

intent_created
signatures_ready
calldata_ready
simulation_ok
broadcast_submitted
mined_success
events_indexed
reconciled
failed

Examples:

If no tx row: stage = latest pre-broadcast stage.
If tx mined_failed: stage = failed.
If mined_success but no events: warning missing_indexed_events.
If events exist but no reconciliation: warning not_reconciled.
If reconciliation status = reconciled: terminal success.
If reconciliation status != reconciled: expose warning/error.
Required Repository / Store Methods

Add read-only methods to fetch by intent_id:

option intent.
option execution transaction row.
confirmation fields.
indexed events for tx_hash.
reconciliation row.
event counts by name/address.

Do not duplicate SQL excessively if existing methods can be reused.

Required Admin Tests

Add tests for:

lifecycle endpoint returns 404 for unknown intent.
lifecycle endpoint returns intent with no tx.
lifecycle endpoint returns mined_success tx.
lifecycle endpoint returns indexed events summary.
lifecycle endpoint returns reconciliation status.
lifecycle health shows reconciled terminal success.
lifecycle health warns when mined_success but missing events.
lifecycle health warns when events exist but no reconciliation.
no generic execution rows are created.
no broadcast path touched.

Use fixtures/mocks. Do not depend on live Base Sepolia.

Required Live Verification Against V1S

If live DB is available, call:

GET /admin/options/executions/e6d2941b-65f7-413a-958f-74ab22c53b08/lifecycle

Expected:

status = broadcast_confirmed
tx hash = 0x5964a7b3d2c18d051baaa780413d31c44d419ce530f45263cb4c46f720881125
confirmation_status = mined_success
event total = 19
reconciliation status = reconciled
health stage = reconciled
is_terminal_success = true

If live DB is not available, document that live verification was not run.

Required Docs

Create:

docs/OPTION_EXECUTION_LIFECYCLE_ENDPOINT_V1Z.md

Include:

endpoint purpose
response structure
lifecycle stages
health/warning logic
V1S expected result
relation to V1S through V1Y-B
remaining deferred work:
frontend admin UI
on-chain state cross-checks
fee-ledger reconciliation
settlement/exercise/expiry views
multichain filters
Validation

Run:

cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
cargo build --all-targets --all-features
Acceptance Criteria

Complete only if:

lifecycle endpoint exists.
endpoint is read-only.
endpoint aggregates intent, tx, receipt, events, reconciliation.
health summary works.
V1S lifecycle can be shown if live DB available.
tests pass.
docs created.
no broadcast path touched.
no transaction submitted.
Final Report

Return:

files changed
endpoint added
response shape summary
repository/store methods added
health logic summary
tests added
live V1S lifecycle result if run
docs created
validation commands run
no forbidden mutation verification
remaining blocker