# Option Execution Lifecycle Endpoint V1Z

Date: 2026-05-23

## Purpose

V1Z adds a single admin-gated read endpoint that aggregates the full
persisted state for one option execution intent across every table the
V1S → V1Y-B pipeline writes: the intent itself, its signature /
simulation / calldata fields, the broadcast transaction, the
receipt / confirmation columns, the indexed `option_execution_events`,
and the `option_execution_reconciliations` row. An operator can now
answer "what is the live status of intent X?" with one HTTP call
instead of joining four tables by hand.

The endpoint is strictly read-only. It never broadcasts, retries,
submits transactions, calls `/executor/broadcast`, opens an RPC
connection, or writes anywhere. It is gated by the existing
`ensure_admin_access` middleware (`ADMIN_API_ENABLED=true` and the
optional admin token).

## Endpoint

```
GET /admin/options/executions/:intent_id/lifecycle
```

- `:intent_id` is the option execution intent UUID (path segment).
- Returns 200 with the aggregated JSON body when the intent exists.
- Returns 404 with `{"error":"invalid option execution intent id"}`
  when the intent UUID is unknown or malformed.

## Response Structure

Top-level fields:

| Field | Type | Notes |
| --- | --- | --- |
| `intent_id` | UUID | echoes the path parameter |
| `status` | enum | `OptionExecutionIntentStatus` (string) |
| `source` | object | `source_type`, `source_id` |
| `trade` | object | buyer, seller, option id, qty, premium, `buyer_is_maker`, `onchain_intent_id` |
| `metadata` | object | underlying, settlement asset, expiry, strike, contract size, `is_call` |
| `signatures` | object | `buyer_signature_present`, `seller_signature_present`, `signature_mode` |
| `simulation` | object | status, block number, simulated_at_ms, error, revert selector |
| `calldata` | object | `present`, `selector` (`0x` + first 4 bytes hex), `hex_length`, `byte_length` |
| `broadcast` | object\|null | tx row when present: transaction_id, tx_hash, status, from, to, gas fields, timestamps |
| `confirmation` | object\|null | confirmation_status, receipt_status, block_number, gas_used, effective_gas_price, cumulative_gas_used, block_hash, transaction_index, observed_at_ms |
| `events` | object | `total`, `counts_by_event_name`, `counts_by_contract_address`, `recent[]` (up to 20, newest first) |
| `fees` | object | `trading_fee_event_count`, `events[]` (id/log/block/trader/recipient/applied_fee/isMaker/option_id/settlement_asset), `total_by_recipient` |
| `transfers` | object | `internal_transfer_count`, `events[]` (id/log/block/from/to/amount/token) |
| `reconciliation` | object\|null | id, status, strict, requires_events, trade/margin event ids, fee/transfer/decoded counts, mismatch_reason, missing_required, timestamps |
| `health` | object | `stage`, `is_terminal_success`, `warnings[]`, `errors[]` |

All hex addresses returned by the lifecycle aggregator are lowercased
where they come from the event ledger; the intent/transaction fields
keep the original casing persisted by the broadcast path. No secrets
or private-key material are returned.

## Lifecycle Stages

The `health.stage` enum tracks how far the intent has progressed:

| Stage | Means |
| --- | --- |
| `intent_created` | intent row exists, signatures not both present |
| `signatures_ready` | buyer + seller signatures present |
| `calldata_ready` | calldata stored on the intent |
| `simulation_ok` | `simulation_status = simulation_ok` |
| `broadcast_submitted` | broadcast transaction row exists |
| `mined_success` | `confirmation_status = mined_success` |
| `events_indexed` | mined success **and** at least one indexed event for the tx hash |
| `reconciled` | reconciliation row exists with `status = reconciled`; this is the only terminal success |
| `failed` | intent failed/cancelled, broadcast reverted/failed, or confirmation `mined_failed` / `mined_reverted` |

`is_terminal_success` is `true` only when stage is `reconciled`.

## Health / Warning Logic

The aggregator walks the persisted state forward and records:

- **errors** when:
  - `intent.status` is `failed` or `broadcast_failed`
  - `transaction.status = failed`
  - `confirmation_status` is `mined_failed` or `mined_reverted`
  - `reconciliation.status = reconciliation_failed` (with the
    mismatch reason joined into the message)

  An error short-circuits `stage` to `failed` and leaves
  `is_terminal_success = false`.

- **warnings** when:
  - `intent.status = cancelled`
  - `simulation_status = simulation_failed`
  - `confirmation_status` is `receipt_missing` or `receipt_error`
  - `mined_success` but no indexed events → `"missing_indexed_events"`
  - indexed events exist but no reconciliation row → `"not_reconciled"`
  - reconciliation status is `partially_reconciled`,
    `missing_events`, or `skipped`

Warnings do not block forward progress; they show alongside the
current stage.

## V1S Expected Result

For the live V1S intent
`e6d2941b-65f7-413a-958f-74ab22c53b08` (transaction row
`cae8c7e7-…`, tx hash `0x5964a7b3…`):

| Field | Live Value |
| --- | --- |
| `status` | `broadcast_confirmed` |
| `trade.onchain_intent_id` | `0x0a77c7c9…` |
| `calldata.selector` | `0x031f77b3` |
| `calldata.byte_length` | `836` |
| `broadcast.tx_hash` | `0x5964a7b3d2c18d051baaa780413d31c44d419ce530f45263cb4c46f720881125` |
| `broadcast.from` | `0xc35f7a8a…` (executor) |
| `broadcast.to` | `0xf2D1D85c…` (matching engine) |
| `broadcast.gas_check_status` | `ok` |
| `broadcast.broadcast_gas_limit` | `1500000` |
| `confirmation.confirmation_status` | `mined_success` |
| `confirmation.receipt_status` | `1` |
| `confirmation.confirmed_block_number` | `41856964` |
| `events.total` | `19` |
| `events.counts_by_event_name` | `OptionTradeExecuted=1, TradeExecuted=1, TradingFeeCharged=2, InternalTransfer=3, Synced=12` |
| `events.counts_by_contract_address` | matching engine 1, margin engine 3, collateral vault 15 |
| `fees.trading_fee_event_count` | `2` |
| `fees.total_by_recipient[0x009f38…]` | `"10"` (buyer fee 6 + seller fee 4) |
| `transfers.internal_transfer_count` | `3` |
| `reconciliation.status` | `reconciled` |
| `reconciliation.trade_executed_event_id` | `0c06450f-…` |
| `reconciliation.margin_trade_event_id` | `8a50ea52-…` |
| `health.stage` | `reconciled` |
| `health.is_terminal_success` | `true` |
| `health.warnings` | `[]` |
| `health.errors` | `[]` |

Live verification was performed on `2026-05-23` against the local
backend bound to `127.0.0.1:8080` with `ADMIN_API_ENABLED=true`. The
endpoint produced the values above and did not introduce any new rows
in `option_execution_intents`, `option_execution_transactions`,
`option_execution_events`, `option_execution_reconciliations`, or
`execution_transactions`.

## Relation to V1S — V1Y-B

The lifecycle response intentionally mirrors the persisted artefacts
produced by each prior phase:

| Phase | Persisted artefact surfaced by V1Z |
| --- | --- |
| V1S | `intent`, `signatures`, `calldata`, `broadcast` (tx hash, status) |
| V1T | `confirmation_status`, `receipt_status`, `confirmed_block_number`, `confirmed_at_ms` |
| V1V/W | gas / cost columns when the worker observes the receipt |
| V1X / V1X-B / V1X-C | `events.total`, `counts_by_event_name`, `counts_by_contract_address`, `recent[]` |
| V1Y | `reconciliation` row + `mismatch_reason` / `missing_required` |
| V1Y-B | non-terminal reconciliation transitions reflected in `health.stage` and warnings |

This keeps V1Z purely additive — no V1Y-era data model changes — and
makes the endpoint a single source of truth for the cross-table
state.

## Tests Added

Aggregator unit tests (`src/options/lifecycle.rs::tests`, 10):

- `unknown_intent_returns_invalid_intent_id_error`
- `intent_with_no_tx_reports_pre_broadcast_stage`
- `mined_success_without_events_warns_missing_events`
- `indexed_events_without_reconciliation_warns_not_reconciled`
- `reconciled_terminal_success`
- `reconciliation_missing_events_surfaces_warning`
- `reconciliation_failed_surfaces_error`
- `mined_failed_marks_health_failed`
- `events_summary_buckets_by_name_and_address`
- `lifecycle_aggregator_does_not_touch_broadcast_or_generic_paths`

Admin route tests (`src/api/routes.rs::tests`, 2):

- `admin_option_execution_lifecycle_returns_404_for_unknown_intent`
- `admin_option_execution_lifecycle_returns_aggregated_view`

Total: **12** new tests. Each fixture-backed test asserts that
`state.repository.is_none()` and `state.trade_signatures.lock().unwrap().is_empty()`
to prove the aggregator never touches the generic execution path.

## Files Changed

- `src/options/lifecycle.rs` (new): the aggregator,
  `OptionExecutionLifecycle` response struct, lifecycle stages, health
  computation, and 10 unit tests.
- `src/options/mod.rs`: re-exports the new module and public types.
- `src/db/repository.rs`: new
  `get_option_execution_reconciliation_by_transaction_id` to read a
  single reconciliation row by transaction id (previously only the
  in-memory store had this method).
- `src/api/routes.rs`: new route
  `GET /admin/options/executions/:intent_id/lifecycle` + 2 admin
  route tests.
- `docs/OPTION_EXECUTION_LIFECYCLE_ENDPOINT_V1Z.md` (this doc).

No migration, no Solidity, no frontend, no deployment, no env, no
worker config, no schema changes.

## Validation Commands

- `cargo fmt --all`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test --all-targets --all-features`
- `cargo build --all-targets --all-features`

## No-Forbidden-Mutation Verification

- The new endpoint runs SELECT statements only:
  - `option_execution_intents`
  - `option_execution_transactions`
  - `option_execution_events`
  - `option_execution_reconciliations`
- No `eth_sendRawTransaction`, no broadcast endpoint, no executor
  call.
- No `/options/execution-intents/:id/broadcast` invocation.
- No new `option_execution_intents`,
  `option_execution_transactions`, or `execution_transactions` rows
  during live verification (row counts unchanged before/after).
- No Solidity / frontend / deployment / `.env` changes.
- No private keys printed.

## Remaining Blocker

- **Frontend admin UI**: V1Z provides the API; building the operator
  console view that consumes it is deferred.
- **On-chain state cross-checks** (buyer/seller nonces, position
  deltas, vault balances): still deferred per V1Y "Remaining
  Blocker" #1. The lifecycle aggregator can adopt those readings once
  the reconciliation worker grows them.
- **Fee-ledger reconciliation**: V1Z surfaces `appliedFee` per event
  and the per-recipient totals, but cross-checking against the backend
  fee ledger is still future work.
- **Settlement / exercise / expiry views**: deferred to a future
  lifecycle extension once those flows ship.
- **Multichain filters**: V1Z queries the single `chain_id` stored on
  each event row; multi-chain operator views would need a wrapper
  endpoint and/or chain-scoped reconciliation tables.
