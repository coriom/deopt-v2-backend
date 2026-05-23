# Option Execution Reconciliation Worker V1Y

Date: 2026-05-23

## Scope

V1Y adds the option execution **reconciliation worker** and its query
surface. A `mined_success` option execution transaction is now reconciled
against the indexed event ledger (V1X-B) and the persisted intent, and
the outcome is stored in a new `option_execution_reconciliations` table.

The worker:

- Reads option execution transactions whose `confirmation_status =
  'mined_success'` and which do not yet have a reconciliation row.
- Loads the linked intent and the indexed `option_execution_events` for
  the transaction's `tx_hash`.
- Decodes `OptionTradeExecuted` and checks `onchain_intent_id`, `buyer`,
  `seller`, `option_id`, `quantity`, `premium_per_contract` against the
  persisted intent.
- Cross-checks `MarginEngine.TradeExecuted` when present.
- Records `TradingFeeCharged` and `CollateralVault.InternalTransfer`
  evidence for downstream auditing.
- Persists a terminal reconciliation outcome and the evidence JSON.

The worker never:

- Broadcasts, retries, or submits transactions.
- Calls `/executor/broadcast` or
  `POST /options/execution-intents/:id/broadcast`.
- Creates new option execution intents.
- Creates `option_execution_transactions` (except via the dedicated
  test-only `test_insert_option_execution_transaction` helper).
- Touches generic `execution_transactions`.
- Modifies Solidity, frontend, deployments, `.env`, or private-key
  material.

## Files Changed

- `migrations/0026_option_execution_reconciliations.sql` (new): adds
  `option_execution_reconciliations` plus four indexes; unique on
  `option_execution_transaction_id`.
- `src/options/types.rs`: new `OptionReconciliationStatus` enum and
  `OptionExecutionReconciliation` row struct.
- `src/options/mod.rs`: re-exports the new worker module and types.
- `src/options/reconciliation_worker.rs` (new): the
  `OptionReconciliationConfig`, decision/tick result types,
  `reconcile_option_executions` tick function, `spawn_option_reconciliation_worker`
  background loop, and 13 unit tests (5 config tests + 8 worker tests).
- `src/options/store.rs`: in-memory `OptionSeriesStore` gains
  `list_option_execution_events_by_tx_hash`,
  `list_confirmed_unreconciled_option_execution_transactions`,
  `upsert_option_execution_reconciliation`,
  `list_option_execution_reconciliations`,
  `summarize_option_execution_reconciliations`,
  `get_option_execution_reconciliation_by_transaction_id`, plus a
  test-only `test_insert_option_execution_transaction` helper.
- `src/db/repository.rs`: PgRepository gains
  `list_option_execution_events_by_tx_hash`,
  `list_confirmed_unreconciled_option_execution_transactions`,
  `upsert_option_execution_reconciliation`,
  `list_option_execution_reconciliations`,
  `summarize_option_execution_reconciliations`. New
  `option_execution_reconciliation_from_row` reader. Admin table-count
  list extended.
- `src/config/env.rs`: parses the six new env keys; wires
  `option_reconciliation` into `AppConfig` and validates on startup.
- `src/api/http.rs`: `AppState` gains `option_reconciliation_config`
  and `option_reconciliation_last_tick`.
- `src/api/routes.rs`:
  - New routes: `GET /admin/options/reconciliations` and
    `POST /admin/options/reconciliations/tick`.
  - Sanitized exposure of the reconciliation worker config under
    `options.reconciliation_worker` in `GET /admin/config`.
  - `option_reconciliation_worker_enabled` added to `GET /admin/status`.
  - Two new admin route tests; `admin_endpoints_do_not_mutate_state`
    extended to include the new endpoint.
- `src/main.rs`: validates config on startup, wires
  `option_reconciliation_config` onto `AppState`, calls
  `spawn_option_reconciliation_worker`, and logs the enabled flag.
- `docs/OPTION_RECONCILIATION_WORKER_V1Y.md` (this doc).

## Migrations Added

`migrations/0026_option_execution_reconciliations.sql` adds:

| Column | Type | Notes |
| --- | --- | --- |
| `id` | UUID PRIMARY KEY | |
| `intent_id` | UUID NOT NULL | option execution intent |
| `onchain_intent_id` | TEXT NOT NULL | |
| `option_execution_transaction_id` | TEXT NOT NULL UNIQUE | |
| `tx_hash` | TEXT NOT NULL | |
| `chain_id` | BIGINT NOT NULL | |
| `status` | TEXT NOT NULL | one of `reconciled`, `partially_reconciled`, `reconciliation_failed`, `missing_events`, `skipped` |
| `strict` | BOOLEAN NOT NULL | snapshot of `OPTION_RECONCILIATION_STRICT` at decision time |
| `requires_events` | BOOLEAN NOT NULL | snapshot of `OPTION_RECONCILIATION_REQUIRE_EVENTS` |
| `trade_executed_event_id` | UUID NULL | matching `OptionTradeExecuted` event row id |
| `margin_trade_event_id` | UUID NULL | matching `MarginEngine.TradeExecuted` event row id |
| `trading_fee_event_count` | BIGINT NOT NULL | |
| `internal_transfer_event_count` | BIGINT NOT NULL | |
| `decoded_event_count` | BIGINT NOT NULL | total persisted events for this tx |
| `mismatch_reason` | TEXT NULL | semicolon-joined per-field reasons |
| `missing_required` | TEXT NULL | name of the missing required event |
| `details` | JSONB NOT NULL | full evidence bundle |
| `reconciled_at_ms` | BIGINT NOT NULL | |
| `created_at_ms` | BIGINT NOT NULL | |
| `updated_at_ms` | BIGINT NOT NULL | |

Indexes: `intent_id`, `onchain_intent_id`, `tx_hash`, `status`.
`UNIQUE(option_execution_transaction_id)` enforces idempotency.

## Config Added

| Env key | Default | Notes |
| --- | --- | --- |
| `OPTION_RECONCILIATION_WORKER_ENABLED` | `false` | |
| `OPTION_RECONCILIATION_POLL_INTERVAL_MS` | `15000` | |
| `OPTION_RECONCILIATION_BATCH_SIZE` | `25` | max confirmed-unreconciled rows considered per tick |
| `OPTION_RECONCILIATION_REQUIRE_EVENTS` | `true` | when true, missing `OptionTradeExecuted` ⇒ `missing_events` |
| `OPTION_RECONCILIATION_REQUIRE_RPC` | `true` | requires `RPC_URL` when enabled |
| `OPTION_RECONCILIATION_STRICT` | `true` | when false, missing/incomplete events downgrade to `partially_reconciled` instead of failing |

When enabled, startup requires persistence and (if `require_rpc=true`)
`RPC_URL`. Sanitized exposure at `GET /admin/config` under
`options.reconciliation_worker`. RPC URL is never returned.

## Reconciliation Worker Behavior

Per tick (`reconcile_option_executions`):

1. List up to `batch_size` confirmed-but-unreconciled option execution
   transactions.
2. For each transaction, load:
   - the linked intent (skip if missing),
   - the indexed `option_execution_events` whose `tx_hash` matches.
3. Decide a status:
   - `Reconciled` — `OptionTradeExecuted` present and all field checks
     pass.
   - `ReconciliationFailed` — any field mismatch on
     `OptionTradeExecuted` or `TradeExecuted` (margin).
   - `MissingEvents` — required `OptionTradeExecuted` absent, either
     when `require_events=true`, or when `strict=true` and
     `require_events=true`.
   - `PartiallyReconciled` — non-strict mode where the required event
     is absent but other evidence was present.
   - `Skipped` — intent missing or transaction lacks `tx_hash`.
4. Upsert the row keyed by `option_execution_transaction_id` (re-running
   the tick is a deterministic no-op on the second pass since the
   transaction is filtered out by the join with the reconciliations
   table).

Decisions are captured in `OptionReconciliationDecision` and the
overall `OptionReconciliationTickResult` is published to
`state.option_reconciliation_last_tick` (in-memory) for admin
observability.

The background loop runs every `poll_interval_ms` and emits one tracing
log per non-empty tick (`tracing::info!`); errors are logged at `warn`.
The loop is a no-op when disabled.

## DB Schema Summary

- Idempotency: `UNIQUE(option_execution_transaction_id)` plus an
  `ON CONFLICT DO UPDATE` upsert.
- Eligibility query: `confirmation_status = 'mined_success'` AND
  `tx_hash IS NOT NULL` AND left-anti-join to the reconciliations table.
- Evidence: `details` JSONB stores `event_count_by_name`,
  `trading_fee_events`, `internal_transfer_events`, `margin_trade`,
  `intent`, `mismatch_reasons`, `missing_required` for full audit
  replay without re-querying the event ledger.

## Event Checks Implemented

`OptionTradeExecuted` (matching engine):

- `onchain_intent_id` must equal `intent.onchain_intent_id`.
- `buyer` must equal `intent.buyer` (case-insensitive address compare).
- `seller` must equal `intent.seller`.
- `option_id` must equal `intent.onchain_option_id` (string compare).
- `quantity` must parse to `intent.quantity_contracts`.
- `premium_per_contract` must parse to
  `intent.premium_per_contract_native`.

`MarginEngine.TradeExecuted` (when present):

- `buyer`, `seller`, `option_id`, `quantity`, `price` cross-checked
  against the same intent fields.

`TradingFeeCharged` and `CollateralVault.InternalTransfer`: counted and
recorded as evidence (event id, log index, block number, decoded
fields). Vault `Synced` events are counted via `event_count_by_name`
but do not block status decisions.

## State Checks Implemented

Optional on-chain state checks (buyer/seller nonces, position deltas)
are **not** included in V1Y because the existing repository surface
does not yet expose those views. The `OPTION_RECONCILIATION_REQUIRE_RPC`
gate exists today so that when the worker grows to call
`MarginEngine.getPositionQuantity` and `OptionMatchingEngine.nonces`,
the RPC dependency is already explicit. Until that lands, the worker
runs entirely off persisted data and is RPC-free in non-strict
deployments.

## Admin Endpoints Added

- `GET /admin/options/reconciliations` (admin-gated):
  - `config.enabled / poll_interval_ms / batch_size / require_events /
    require_rpc / strict / rpc_configured`.
  - `counts` map across all five statuses (defaulting to 0).
  - `latest_tick` = the last `OptionReconciliationTickResult`.
  - `recent` = the most recent 20 reconciliation rows.
- `POST /admin/options/reconciliations/tick` (admin-gated):
  - Returns 400 when the worker is disabled.
  - Otherwise runs `reconcile_option_executions` exactly once and
    returns the tick result. Does not broadcast, retry, or submit
    transactions.

`GET /admin/config` exposes
`options.reconciliation_worker.{enabled, poll_interval_ms, batch_size,
require_events, require_rpc, strict, rpc_configured}`.

`GET /admin/status` exposes `option_reconciliation_worker_enabled`.

## Tests Added

Worker unit tests (`src/options/reconciliation_worker.rs::tests`):

- `disabled_default_passes_validate`
- `enabled_requires_persistence`
- `enabled_with_require_rpc_requires_rpc_url`
- `enabled_rejects_zero_batch_size`
- `enabled_rejects_zero_poll_interval`
- `worker_disabled_does_nothing`
- `missing_event_marks_missing_events_in_strict_mode`
- `matching_option_trade_executed_reconciles`
- `mismatched_buyer_or_seller_marks_reconciliation_failed`
- `mismatched_option_id_marks_reconciliation_failed`
- `mismatched_quantity_marks_reconciliation_failed`
- `idempotent_reruns_overwrite_same_row`
- `no_broadcast_or_generic_execution_rows_touched`

Admin route tests (`src/api/routes.rs`):

- `admin_option_reconciliations_returns_counts_and_latest_tick`
- `admin_option_reconciliations_tick_rejects_when_disabled`
- existing `admin_endpoints_do_not_mutate_state` extended with
  `/admin/options/reconciliations`.

## Docs Created

- `docs/OPTION_RECONCILIATION_WORKER_V1Y.md` (this doc).

## Validation Commands

- `cargo fmt --all`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test --all-targets --all-features`
- `cargo build --all-targets --all-features`

## No-Forbidden-Mutation Verification

- `POST /options/execution-intents/:id/broadcast`: **not called**.
- `/executor/broadcast/:intent_id`: **not called**.
- `eth_sendRawTransaction`: **not called** anywhere on the new code
  path; the worker reads `option_execution_transactions` and
  `option_execution_events` only.
- No new `option_execution_intents` rows.
- No new `option_execution_transactions` rows (the test helper
  `test_insert_option_execution_transaction` is `#[cfg(test)]`).
- No `execution_transactions` mutations.
- No Solidity / frontend / deployment changes.
- No `.env` file changes; secrets never printed.

## Was V1S Reconciled By The Worker?

V1Y ships only the worker and admin surface; no migration was run and
no worker tick was fired against the live DB during this task. V1S
reconciliation in production therefore happens the next time an
operator:

1. Runs `sqlx migrate run` to apply `0026_option_execution_reconciliations.sql`.
2. Sets `OPTION_RECONCILIATION_WORKER_ENABLED=true` (with `RPC_URL`).
3. Either waits for the background worker tick or calls
   `POST /admin/options/reconciliations/tick`.

Coverage of V1S in code is provided by the unit tests, which use the
exact V1S `tx_hash`, `onchain_intent_id`, buyer/seller, `option_id=7`,
quantity, and premium values from the V1T evidence.

## Remaining Blocker

1. Live `on_chain_state_checks` (buyer/seller nonces, position deltas
   off `MarginEngine.getPositionQuantity`, vault balance sanity off
   `CollateralVault.balances`) — out of scope for V1Y. The
   `OPTION_RECONCILIATION_REQUIRE_RPC` flag is wired so the future
   work can plug straight in.
2. Fee-ledger cross-check between `TradingFeeCharged` totals and the
   backend's fee ledger entries — deferred. The V1Y row already records
   per-event `appliedFee` + `isMaker` so the comparison surface is
   ready.
3. Settlement / exercise / expiry reconciliation — deferred (separate
   on-chain flows).
4. Operator UI surface for the reconciliation rows — out of scope.
