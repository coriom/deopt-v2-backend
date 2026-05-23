# Option Execution Confirmation Worker V1V

Date: 2026-05-23

## Scope

V1V automates what V1T did by hand: pulls every option execution transaction
that has been broadcast (`status = submitted`, `tx_hash` set, not yet
finalized) and reconciles it against its on-chain receipt. The worker is
**off by default** and is intentionally minimal — no event indexer, no
position/vault snapshot, no settlement, no rebroadcast. It only:

1. Reads pending option execution transactions from persistence.
2. Calls `eth_getTransactionReceipt(tx_hash)` for each.
3. Calls `eth_blockNumber()` once per tick to gate finality.
4. Applies the finality rule
   `receipt.block_number + OPTION_CONFIRMATION_FINALITY_BLOCKS ≤ current_block`
   to decide whether to finalize.
5. Persists a terminal `confirmation_status` and transitions the linked
   intent (`broadcast_submitted` → `broadcast_confirmed` on success,
   `broadcast_submitted` → `broadcast_failed` on failed receipt) only when
   finality is reached.

V1V did not broadcast any transaction, did not call `/executor/broadcast`
or the option broadcast endpoint, did not touch generic
`execution_transactions`, did not create new option execution intents
outside tests, did not cleanup the preserved V1L/V1S/V1T evidence rows,
and did not modify Solidity, frontend, or `.env` files.

## Config

Five new env keys, all read in `src/config/env.rs` and validated by
`OptionConfirmationConfig::validate_startup`:

| Env key | Default | Purpose |
| --- | --- | --- |
| `OPTION_CONFIRMATION_WORKER_ENABLED` | `false` | Master kill-switch. When `false`, `spawn_option_confirmation_worker` is a no-op. |
| `OPTION_CONFIRMATION_POLL_INTERVAL_MS` | `15000` | Tick cadence. |
| `OPTION_CONFIRMATION_FINALITY_BLOCKS` | `3` | `current_block - receipt.block_number ≥ 3` before finalizing. |
| `OPTION_CONFIRMATION_BATCH_SIZE` | `25` | Max pending rows processed per tick. |
| `OPTION_CONFIRMATION_REQUIRE_RPC` | `true` | When `true` and the worker is enabled, startup rejects unless `RPC_URL` is set. |

Startup rejects:

- `WORKER_ENABLED=true` with `PERSISTENCE_ENABLED=false`,
- `WORKER_ENABLED=true` + `REQUIRE_RPC=true` + no `RPC_URL`,
- zero batch size or zero poll interval when enabled.

Sanitized fields are exposed in `/admin/config` under
`options.confirmation_worker`:

```json
"confirmation_worker": {
  "enabled": false,
  "poll_interval_ms": 15000,
  "finality_blocks": 3,
  "batch_size": 25,
  "require_rpc": true,
  "rpc_configured": false
}
```

No secrets are emitted — only a boolean for the RPC URL.

## Worker Behavior Summary

The worker uses the existing `TransactionReceiptProvider` trait
(`eth_blockNumber` + `eth_getTransactionReceipt`); no new RPC types
were added. Each tick:

1. If `option_confirmation_config.enabled == false`, the worker function
   returns an empty `OptionConfirmationTickResult { enabled: false, .. }`
   immediately (no RPC, no DB write).
2. Fetch `head = provider.block_number().await.ok()`.
3. Read up to `batch_size` pending option execution transactions via
   `list_pending_option_execution_transactions`. A row is *pending* when:
   - `status = submitted`
   - `tx_hash` non-null and non-empty
   - `confirmation_status ∈ { NULL, pending, receipt_missing, receipt_error }`
4. For each row, call `provider.transaction_receipt(tx_hash)` and pass the
   result to `compute_worker_decision`, which is a pure function that
   maps `(receipt, head, finality_blocks)` to an
   `OptionConfirmationDecision { outcome, receipt_status, block_number, … }`.
5. `apply_worker_decision` then writes the row + transitions the intent.

The decision matrix:

| Receipt result | head − receipt.block | Outcome | Tx row update | Intent transition |
| --- | --- | --- | --- | --- |
| `Ok(Some(receipt))` with `receipt.tx_hash == expected_tx_hash` and `status = 1` and `head − block ≥ finality_blocks` | ≥ finality | `MinedSuccess` | `confirmation_status = mined_success`, `receipt_status = 1`, `confirmed_block_number`, `confirmed_at_ms` | `broadcast_submitted → broadcast_confirmed` |
| `Ok(Some(receipt))` with `status = 0` (or non-1) and `head − block ≥ finality_blocks` | ≥ finality | `MinedFailed` | `confirmation_status = mined_failed`, `receipt_status = 0` | `broadcast_submitted → broadcast_failed` |
| `Ok(Some(receipt))` with finality not yet reached | < finality | `NotFinalized` | `confirmation_status = pending`, `receipt_status`, `confirmed_block_number = receipt.block` | unchanged |
| `Ok(Some(receipt))` whose tx hash does not match | n/a | `ReceiptMissing` | `confirmation_status = receipt_missing` with error string | unchanged |
| `Ok(Some(receipt))` missing `block_number` | n/a | `ReceiptError` | `confirmation_status = receipt_error` with error string | unchanged |
| `Ok(Some(receipt))` missing `status` field | n/a | `ReceiptError` | `confirmation_status = receipt_error` with error string | unchanged |
| `Ok(None)` (receipt not yet on-chain) | n/a | `ReceiptMissing` | `confirmation_status = receipt_missing` | unchanged |
| `Err(rpc_error)` | n/a | `ReceiptError` | `confirmation_status = receipt_error` with error string | unchanged |
| `head` unavailable (`block_number()` errored) | n/a | `NotFinalized` | `confirmation_status = pending` | unchanged |

The worker never rebroadcasts, never signs anything, never calls
`/executor/broadcast`, never inserts a `option_execution_transactions`
row (insertion is the V1Q/V1S broadcast path's job; the worker only
UPDATEs), and never touches `execution_transactions`.

## Status Transition Summary

Existing option execution intent statuses kept unchanged by V1V:
`pending`, `signatures_required`, `signatures_ready`, `calldata_ready`,
`simulation_ready`, `simulation_ok`, `simulation_failed`,
`broadcast_submitted`, `broadcast_failed`, `cancelled`, `failed`,
`broadcast_confirmed` (V1T), `broadcast_reverted` (V1T legacy).

V1V worker transitions:

- `broadcast_submitted → broadcast_confirmed` on `mined_success`
- `broadcast_submitted → broadcast_failed` on `mined_failed`

V1T's manual `/options/execution-intents/:id/confirm` endpoint still
emits `broadcast_reverted` for status-0 receipts so the V1T migration's
DB shape is preserved. New code paths emit `broadcast_failed`. Both
statuses are parse-compatible with the existing `OptionExecutionIntentStatus`
enum; the `OptionExecutionConfirmationStatus` enum gained one new variant
`MinedFailed` (the worker spec) and retains `MinedReverted` (V1T legacy)
for backward compatibility with persisted V1T rows.

## RPC Surface

V1V uses only:

- `eth_blockNumber` (already implemented in
  `HttpJsonRpcProvider::TransactionReceiptProvider::block_number`).
- `eth_getTransactionReceipt(tx_hash)` (already implemented; returns
  `Option<ConfirmationReceipt>` whose status field is parsed from the
  receipt's `status` and `block_number` from `block_number`).

No new RPC methods, request types, or trait methods were added.

## Tests Added

13 new tests passing (`248` total lib tests, up from `235` at V1T close):

In `src/options/confirmation_worker.rs`:

- `disabled_default_passes_validate`
- `enabled_requires_persistence`
- `enabled_with_require_rpc_requires_rpc_url`
- `enabled_rejects_zero_batch_size`
- `enabled_rejects_zero_poll_interval`

In `src/options/service.rs`:

- `worker_disabled_returns_disabled_and_does_nothing` — proves the
  worker tick is a pure no-op when `enabled = false`: no RPC call, no DB
  update.
- `worker_missing_receipt_leaves_pending` — `Ok(None)` from the receipt
  provider does not transition the intent.
- `worker_receipt_without_finality_does_not_finalize` — `mined_success`
  receipt at block 100, head at 101, finality_blocks = 3, expect
  `NotFinalized`, intent unchanged.
- `worker_successful_receipt_with_finality_finalizes_mined_success` —
  receipt at 100, head at 105, finality 3, expect `MinedSuccess`,
  intent transitions to `broadcast_confirmed`, tx row records
  receipt_status=1 + block 100.
- `worker_failed_receipt_with_finality_finalizes_mined_failed` —
  receipt status=0 → intent → `broadcast_failed`, tx row records
  `mined_failed`.
- `worker_does_not_use_broadcast_provider` — explicit assertion that
  `MockBroadcastProvider.send_count() == 0` after a worker tick that
  finalizes a row.
- `worker_never_creates_generic_execution_rows` — three repeated ticks,
  `assert_no_generic_execution_rows(&state)` passes, only the option
  store row is updated in place.
- `worker_outcome_is_finalized_only_for_mined_states` — unit-level
  check on `OptionConfirmationOutcome::is_finalized`.

In `src/config/env.rs`:

- `option_confirmation_worker_uses_safe_defaults`
- `option_confirmation_worker_parses_overrides`
- `option_confirmation_worker_rejects_when_persistence_disabled`
- `option_confirmation_worker_rejects_when_rpc_required_but_missing`

## Files Changed

- `src/options/confirmation_worker.rs` (new — config struct, decision/tick
  types, `spawn_option_confirmation_worker`, 5 unit tests)
- `src/options/mod.rs` — `pub mod confirmation_worker;` + re-exports
- `src/options/types.rs` — added `OptionExecutionConfirmationStatus::MinedFailed`
- `src/options/store.rs` — `list_pending_option_execution_transactions`
- `src/options/service.rs` — `confirm_pending_option_execution_transactions`,
  `compute_worker_decision`, `apply_worker_decision`,
  `list_pending_option_execution_transactions` (state-aware wrapper);
  extended `MockReceiptProvider` with `with_head`; 8 new tokio tests
- `src/config/env.rs` — `OPTION_CONFIRMATION_*` env parsing,
  validate_startup wiring; 4 new tests
- `src/api/http.rs` — `option_confirmation_config: OptionConfirmationConfig`
  on `AppState`, default `disabled()` in the legacy constructor
- `src/api/routes.rs` — `/admin/config` exposes `options.confirmation_worker`
- `src/main.rs` — validate at boot, plumb into `AppState`, call
  `spawn_option_confirmation_worker`, emit `option_confirmation_worker_enabled`
  in the startup log
- `docs/OPTION_CONFIRMATION_WORKER_V1V.md` (this doc)

## Migration Added

**None.** V1T's migration `0023_option_execution_confirmation.sql` already
added every column the worker needs (`confirmation_status`,
`confirmed_at_ms`, `confirmed_block_number`, `receipt_status`,
`confirmation_error`). The new `mined_failed` value goes through the same
`TEXT` column.

## No-Forbidden-Mutation Verification

- `POST /options/execution-intents/:id/broadcast`: not called.
- `/executor/broadcast`: not called.
- `eth_sendRawTransaction`: not called.
- `option_execution_transactions`: zero new INSERTs outside tests
  (worker only UPDATEs the existing row).
- `execution_transactions` (generic table): zero new rows.
- Preserved V1L/V1S/V1T evidence rows untouched.
- No `.env` files modified; no real secrets touched.
- Solidity/frontend/deploy: unchanged.
- No private keys printed.

## Validation Commands

- `cargo fmt --all` → clean
- `cargo clippy --all-targets --all-features -- -D warnings` → clean
- `cargo test --all-targets --all-features` → all suites green
  - `248` lib tests pass (`+13` since V1T close)
  - `13 + 37 + 67 + 76 + 12 + 8 + 43 + 0` passing across HTTP / options-HTTP /
    RFQ / orderbook / etc. integration suites — same totals as V1T plus the
    13 new worker/config cases inside `lib`
- `cargo build --all-targets --all-features` → clean

## Remaining Blocker

None for the V1V scope (background confirmation worker with finality).
Out-of-scope follow-ups (intentionally deferred, consistent with
"no full indexer"):

1. **Persist `gas_used` and `effective_gas_price`** in
   `option_execution_transactions` so the worker writes a complete cost
   record. Requires a small `ConfirmationReceipt` extension and a 1-column
   migration; deferred until needed for analytics.
2. **Cap the worker's per-tick visible window** to a `WHERE created_at_ms
   > now - lookback_ms` clause so a long backlog of pending rows can't
   starve recent broadcasts. Not needed today (typical pending count is
   low) but trivial to add when scale demands it.
3. **Settlement / exercise / expiry pipelines** for the position now
   held by buyer (long 1 call) + seller (short 1 call) — out of scope.
4. **Event indexer** for `OptionTradeExecuted` / `TradingFeeCharged` /
   `InternalTransfer` etc. so logs become queryable — out of scope.
5. **Operator endpoint** that returns the worker's last tick result
   (`OptionConfirmationTickResult`) for observability — would be a small
   addition over the existing struct, but out of V1V scope.
