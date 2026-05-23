# Option Confirmation Observability V1W

Date: 2026-05-23

## Scope

V1W is the production-observability layer on top of the V1V background
confirmation worker:

- Persists receipt-cost fields (`gas_used`, `effective_gas_price`,
  `cumulative_gas_used`, `block_hash`, `transaction_index`,
  `receipt_observed_at_ms`) on `option_execution_transactions` whenever
  `eth_getTransactionReceipt` returns them — used by both the V1V
  background worker *and* the V1T manual confirm endpoint via a shared
  persistence path.
- Adds a sanitized admin endpoint `GET /admin/options/confirmations`
  exposing worker config, current per-status counts, and the latest
  worker tick.
- Stores the latest worker tick result in memory on `AppState` so the
  admin endpoint can return it without any extra RPC call.

V1W did not broadcast any transaction, did not call
`/executor/broadcast` or the option broadcast endpoint, did not touch
`execution_transactions`, did not create new option execution intents,
did not cleanup the preserved V1L/V1S/V1T evidence rows, and did not
modify Solidity, frontend, or `.env` files.

## Receipt Fields Added

Persisted columns on `option_execution_transactions` (added in
`migrations/0024_option_execution_receipt_cost.sql`, idempotent
`ADD COLUMN IF NOT EXISTS`):

| Column | Type | Source |
| --- | --- | --- |
| `gas_used` | `BIGINT NULL` | `eth_getTransactionReceipt.gasUsed` |
| `effective_gas_price` | `TEXT NULL` (raw `0x…` hex; preserves any width) | `eth_getTransactionReceipt.effectiveGasPrice` |
| `cumulative_gas_used` | `BIGINT NULL` | `eth_getTransactionReceipt.cumulativeGasUsed` |
| `receipt_block_hash` | `TEXT NULL` | `eth_getTransactionReceipt.blockHash` |
| `receipt_transaction_index` | `BIGINT NULL` | `eth_getTransactionReceipt.transactionIndex` |
| `receipt_observed_at_ms` | `BIGINT NULL` | `now_ms()` at the moment the worker/manual confirm bridged the receipt |

The corresponding fields on `confirmation::ConfirmationReceipt` and the
JSON-RPC `EthTransactionReceipt` deserializer were extended so the values
flow `eth_getTransactionReceipt → ConfirmationReceipt → OptionExecutionReceiptCost
→ persistence` without any per-call ad-hoc parsing.

`OptionExecutionReceiptCost` (new in `options/types.rs`, re-exported from
`crate::options`) bundles the six fields. `service::receipt_cost_from_receipt`
is the single bridge function consumed by *both* paths:

- `service::confirm_option_execution_intent_with_provider` (V1T manual confirm endpoint), and
- `service::confirm_pending_option_execution_transactions` (V1V worker tick).

Both paths funnel into the same `service::persist_option_execution_confirmation`,
which delegates to either the Postgres repository or the in-memory
`OptionSeriesStore`, with identical UPSERT semantics (the SQL uses
`COALESCE($n, existing)` so a row is never zeroed back to NULL when a
later observation arrives with fewer fields populated).

## Admin Endpoint Added

`GET /admin/options/confirmations` (requires admin auth via the same
`ensure_admin_access` gate as `/admin/config` and `/admin/db`).

Response shape (sanitized; no secrets emitted):

```json
{
  "config": {
    "enabled": false,
    "poll_interval_ms": 15000,
    "finality_blocks": 3,
    "batch_size": 25,
    "require_rpc": true,
    "rpc_configured": false
  },
  "counts": {
    "pending": 0,
    "mined_success": 1,
    "mined_failed": 0,
    "receipt_missing": 0,
    "receipt_error": 0,
    "mined_reverted": 0
  },
  "latest_tick": {
    "enabled": true,
    "batch_size": 25,
    "finality_blocks": 3,
    "current_block_number": 41856964,
    "decisions": [
      {
        "transaction_id": "cae8c7e7-…",
        "tx_hash": "0x5964a7b3…",
        "outcome": "mined_success",
        "receipt_status": 1,
        "block_number": 41856962,
        "current_block_number": 41856964,
        "finality_blocks": 3,
        "error": null
      }
    ]
  }
}
```

Notes:

- The `counts` map keys are exactly the `OptionExecutionConfirmationStatus`
  string variants plus the synthetic bucket `pending` for NULL rows.
  When no rows of a given status exist they are omitted from the map
  (the V1V worker spec says "sanitized" — empty counts are not
  fabricated).
- `latest_tick` is `null` until the worker (or the
  `confirm_pending_option_execution_transactions` service call from a
  test/admin context) has run at least once with the worker enabled.
- The endpoint reads counts from Postgres via
  `repository.summarize_option_execution_confirmations()` (single GROUP
  BY query) when persistence is enabled, otherwise from
  `OptionSeriesStore::summarize_option_execution_confirmations()`.

## Latest Tick Behavior

`AppState` gained `option_confirmation_last_tick: Arc<Mutex<Option<OptionConfirmationTickResult>>>`.

- The shared slot is updated **only when the worker tick is enabled**:
  the early-return branch in
  `confirm_pending_option_execution_transactions` for the disabled state
  exits before touching the slot, so a disabled worker can never publish
  stale-looking ticks.
- The slot stores `Option<OptionConfirmationTickResult>` — `None` means
  the worker has not run yet; `Some(…)` is the most recent completed
  tick (including ticks with zero decisions).
- The slot is overwritten every tick (no history retained; this is the
  observability layer, not an indexer).
- The mutex is poison-tolerant: the admin endpoint clones the value out
  if it can lock, otherwise emits `null` for `latest_tick` (preferring
  partial visibility over crashing the endpoint).

The worker's existing decision matrix (V1V doc) is unchanged. The only
behavior additions are the cost-bundle persistence (always attempted
when a matching receipt is present, even on `NotFinalized`) and the
`AppState` publish.

## Files Changed

- `migrations/0024_option_execution_receipt_cost.sql` (new — 6 idempotent ALTERs)
- `src/confirmation/mod.rs` — extended `ConfirmationReceipt` with the six receipt fields; test helper updated
- `src/execution/rpc.rs` — extended `EthTransactionReceipt` JSON deserialization to read all six fields from `eth_getTransactionReceipt`
- `src/options/types.rs` — new `OptionExecutionReceiptCost` struct; added the six persisted fields on `OptionExecutionTransaction`
- `src/options/mod.rs` — re-export `OptionExecutionReceiptCost`
- `src/options/store.rs` — `update_option_execution_confirmation` accepts `&OptionExecutionReceiptCost`; new `summarize_option_execution_confirmations`
- `src/db/repository.rs` — `update_option_execution_confirmation` accepts the bundle, SQL uses `COALESCE` so partial observations never zero out prior values; new `summarize_option_execution_confirmations`; row reader + every SELECT updated to include the new columns
- `src/options/service.rs` — `receipt_cost_from_receipt` bridge; worker `apply_worker_decision` + manual `confirm_option_execution_intent_with_provider` both persist via the shared cost bundle; tick result is published to `AppState`; 7 new tokio tests; `option_execution_transaction_from_request` initialises the new fields to `None`
- `src/options/confirmation_worker.rs` — unchanged (V1V worker logic remains; only the persistence path was extended)
- `src/api/http.rs` — `option_confirmation_last_tick: Arc<Mutex<Option<OptionConfirmationTickResult>>>` on `AppState`, default `None`
- `src/api/routes.rs` — `GET /admin/options/confirmations` route + `admin_option_confirmations` handler
- `docs/OPTION_CONFIRMATION_OBSERVABILITY_V1W.md` (this doc)

No frontend, no Solidity, no deploy artifacts changed. The V1L/V1S/V1T
evidence rows are untouched.

## Migration Added

**Yes**: `migrations/0024_option_execution_receipt_cost.sql` —
6 idempotent `ALTER TABLE option_execution_transactions ADD COLUMN IF
NOT EXISTS` statements. Existing rows are unaffected (all new columns
default to NULL). The migration is identical-shape to V1T's `0023`
which V1W's logic builds on.

## Tests Added

7 new `tokio::test` / unit cases in `src/options/service.rs`, on top of
the 5 `confirmation_worker::tests` and 17 existing
`service::tests` worker/manual-confirm cases from V1T/V1V. All pass.

| Test | Coverage |
| --- | --- |
| `receipt_cost_persists_through_manual_confirm` | Manual confirm endpoint writes all six cost fields when the receipt provides them. |
| `worker_stores_gas_fields_on_mined_success` | Worker tick writes the same cost fields on a finalized `mined_success` outcome. |
| `worker_does_not_store_cost_when_receipt_absent` | Receipt provider returning `Ok(None)` does *not* write any cost fields; row stays clean. |
| `worker_publishes_latest_tick_after_tick` | `AppState.option_confirmation_last_tick` is `None` initially, then `Some(…)` after a worker tick with `enabled=true`. |
| `worker_disabled_does_not_publish_latest_tick` | Disabled worker run leaves `latest_tick = None`. |
| `summary_counts_bucket_pending_correctly` | In-memory `summarize_option_execution_confirmations` returns the `("pending", n)` bucket for NULL `confirmation_status` rows, and transitions to `("mined_success", n)` after a finalized tick. |
| `store_update_receipt_cost_persists_independently_of_status_transition` | Direct unit test on `OptionSeriesStore::update_option_execution_confirmation` — proves the cost bundle is persisted even when `confirmation_status = Pending` (non-terminal), and that the per-field semantics are "set when the source has a value" (mirroring the SQL `COALESCE`). |

Lib totals: **255 passing tests** (`+7` since V1V close). All
integration suites unchanged: HTTP/options-HTTP/RFQ/orderbook/etc. still
green at 13 + 37 + 67 + 76 + 12 + 8 + 43 = 256 across `tests/*.rs`.

Sanity-check assertions present in every new test:

- `assert_no_generic_execution_rows(&state)` — no rows touched on the
  generic perp `execution_transactions` table.
- `MockBroadcastProvider.send_count() == 0` (worker tests; carried over
  from V1V `worker_does_not_use_broadcast_provider`).

## Validation Commands

- `cargo fmt --all` → clean
- `cargo clippy --all-targets --all-features -- -D warnings` → clean
  (three `update_option_execution_confirmation` functions ship
  `#[allow(clippy::too_many_arguments)]` because the cost bundle pushes
  arity from 7 → 8; the alternative is collapsing into a struct, which
  would just reshape the call sites without changing arity meaningfully)
- `cargo test --all-targets --all-features` → all suites green
  (255 lib + 13 + 37 + 67 + 76 + 12 + 8 + 43 = **511 passing**, 0
  failures)
- `cargo build --all-targets --all-features` → clean

## No-Forbidden-Mutation Verification

- `POST /options/execution-intents/:id/broadcast`: not called.
- `/executor/broadcast`: not called.
- `eth_sendRawTransaction`: not called (only the existing
  `eth_blockNumber` + `eth_getTransactionReceipt`).
- `option_execution_transactions`: zero new INSERTs outside
  `#[cfg(test)]` blocks; V1W only UPDATEs the existing rows.
- `execution_transactions` (generic table): zero new rows.
- Preserved V1L/V1S/V1T evidence rows untouched.
- `.env` / Solidity / frontend / deployment artifacts unchanged.
- No private keys printed.

## Remaining Blocker

None for the V1W scope. Out-of-scope follow-ups, deferred:

1. **Time-bounded `latest_tick` history** — V1W stores only the most
   recent tick; an admin "show last N ticks" view would need either a
   bounded ring buffer in `AppState` or a (separate) ops-only table.
2. **Per-status time-series** — counts are point-in-time only. Wiring
   them into the existing Prometheus metrics surface would let
   dashboards graph them; out of scope here.
3. **Native numeric `effective_gas_price`** — stored as the hex string
   from `eth_getTransactionReceipt` (preserves arbitrary width).
   Operators that want it in decimal can decode it client-side; a
   `NUMERIC` column would let SQL aggregate it directly but isn't
   needed for the current observability surface.
4. **Event indexer** for `OptionTradeExecuted` / `TradingFeeCharged` /
   `InternalTransfer` — same out-of-scope note as V1V.
5. **Settlement / exercise / expiry pipelines** for the open option
   position — same out-of-scope note as V1S/V1T/V1V.
