# OPTION-EXECUTION-TX-VISIBILITY-FIX — result

**Posture:** SHIPPED at 2026-06-10.

> **Addendum (2026-06-10, follow-on `BACKEND-EXECUTOR-TRANSACTIONS-LIST-EXTEND`):**
> the §11 recommended next-milestone shipped — `GET /executor/transactions`
> (list variant) now also returns OPTION rows alongside legacy PERP rows
> under the same `ExecutorTransactionView` shape with the same `source`
> discriminator. Limit semantics preserved. The operator-facing
> transaction surface is now fully unified across both tables. See
> `docs/BACKEND_EXECUTOR_TRANSACTIONS_LIST_EXTEND_RESULT.md`.

## 1. Root cause

`GET /executor/transactions/<intent_id>` queried only the legacy
`execution_transactions` table via
`PgRepository::get_transactions_for_intent`. Option execution
transactions (both `OptionOrderbookFill` and `OptionRfqFill` source
types) are persisted in a SEPARATE table:
`option_execution_transactions`, accessed via
`PgRepository::get_option_execution_transactions_for_intent`. The
endpoint had no awareness of the option table, so any operator or
admin lookup for an option intent — even one that had already reached
`broadcast_confirmed` — got `[]` back.

The bug was reproducible against the live Sepolia RFQ smoke intent
`95516dbd-a68c-41eb-869f-e6790d9091f2` whose confirmed tx
`0x8538066c…5326` lived in the option table only.

## 2. Canonical visibility model

Given an option execution `intent_id`, the endpoint now returns ALL
related transaction records from BOTH tables, projected onto a
unified `ExecutorTransactionView` shape. Key invariants:

* **Source-of-truth provenance.** Each row carries a `source`
  discriminator (`"perp"` or `"option"`) so consumers can identify
  which underlying table backed it.
* **Same JSON keys as the legacy shape.** Field names from
  `ExecutionTransaction` are preserved verbatim (`tx_hash`, `status`,
  `confirmation_status`, `confirmed_block_number`, etc.) so existing
  PERP consumers see the same envelope.
* **Additive option-only fields.** When the row sources from the
  option table, additional optional fields populate
  (`source_type`, `from`, `gas_limit`, `receipt_status`,
  `receipt_block_hash`, `gas_used`, `effective_gas_price`,
  `cumulative_gas_used`, `receipt_observed_at_ms`). On PERP rows these
  are `None`.
* **Deterministic ordering.** Latest first by `created_at_ms DESC`
  with `transaction_id DESC` tiebreak — same as the per-table SQL.
* **No false positives.** Empty array iff neither table has a row for
  the intent_id. The endpoint never invents data.
* **Read-only.** No RPC probes, no DB writes, no broadcasts.
* **Two confirmation-status vocabularies coexist** behind the same
  `confirmation_status` key (snake_case strings):
  * PERP rows: `pending`/`confirmed`/`failed`/`not_finalized`/
    `missing_receipt`/`missing_reconciliation`/`missing_indexed_event`/
    `receipt_failed`.
  * OPTION rows: `pending`/`mined_success`/`mined_reverted`/
    `mined_failed`/`receipt_missing`/`receipt_error`.

## 3. Persistence / linking — no schema changes

No migration was needed. Both target tables already exist and already
store:

* `intent_id` (linking key)
* `tx_hash`
* `from` (option table only — `sender` column)
* `target` (option table: `to`; perp table: `target`)
* `status`
* `confirmation_status`
* `confirmed_block_number`
* `receipt_status` / `gas_used` / `effective_gas_price` /
  `cumulative_gas_used` / `receipt_block_hash` /
  `receipt_observed_at_ms` (option table only)
* `created_at_ms` / `updated_at_ms`

The chain_id is sourced from `state.execution_config.executor_chain_id`
— the BE's configured chain — surfaced when any row is returned.

## 4. Files changed

* `src/api/routes.rs`
  * NEW `ExecutorTransactionView` struct + `from_perp` / `from_option`
    mappers.
  * Rewrote `executor_transactions_for_intent` to:
    1. Parse the path UUID.
    2. With a repository: query both tables + the option intent (for
       `source_type`) in 3 sequential calls.
    3. Without a repository: fall back to the in-memory
       `options_store` (mirrors the lifecycle endpoint's pattern). The
       legacy `execution_transactions` table has no in-memory fallback
       (empty is correct in that mode).
    4. Combine, sort latest-first, return.
  * Added `TimestampMs` to the existing `use crate::types::{...}` line.
  * 5 new integration tests under `routes::tests`.
* `docs/OPTION_EXECUTION_TX_VISIBILITY_FIX_RESULT.md` — NEW (this
  document).
* `docs/BACKEND_MAINNET_IMPLEMENTATION_ROADMAP.md` — note that §1.5
  (`OPTION-EXECUTION-TX-VISIBILITY-FIX`) is closed.
* `RUN_STATE.md` — closure paragraph appended.

No `.env` edited. No `sol/` source touched. No DB migration. No
existing endpoint URL changed (only response payload widened
additively).

## 5. Endpoint behavior

`GET /executor/transactions/:intent_id` — same path; response is now
`Vec<ExecutorTransactionView>` (a superset of the legacy
`Vec<ExecutionTransaction>` JSON shape). HTTP 200 always, empty array
for genuinely no-row intents.

Response shape per row:

```jsonc
{
  "transaction_id":            "<string>",
  "intent_id":                 "<uuid>",
  "onchain_intent_id":         "<hex|null>",
  "chain_id":                  <u64|null>,
  "source":                    "perp" | "option",
  "source_type":               "option_orderbook_fill" | "option_rfq_fill" | null,
  "from":                      "<hex|null>",
  "target":                    "<hex>",
  "calldata":                  "<hex>",
  "value_wei":                 "<decimal>",
  "gas_limit":                 <u64|null>,
  "tx_hash":                   "<hex|null>",
  "status":                    "prepared|rejected|submitted|failed",
  "error":                     "<string|null>",
  "confirmation_status":       "<snake_case|null>",
  "confirmed_at_ms":           <i64|null>,
  "confirmed_block_number":    <u64|null>,
  "receipt_status":            <u64|null>,
  "receipt_block_hash":        "<hex|null>",
  "receipt_block_number":      <u64|null>,
  "receipt_observed_at_ms":    <i64|null>,
  "gas_used":                  <u64|null>,
  "effective_gas_price":       "<decimal|null>",
  "cumulative_gas_used":       <u64|null>,
  "confirmation_error":        "<string|null>",
  "created_at_ms":             <i64>,
  "updated_at_ms":             <i64>
}
```

## 6. Orderbook / RFQ behavior

The endpoint is source-type-agnostic: orderbook + RFQ intents share
the same `option_execution_transactions` table; only the `source_type`
discriminator differs. Both source types surface their txs identically
— the row reports `source_type: "option_orderbook_fill"` or
`source_type: "option_rfq_fill"` based on the intent.

Pinned by two integration tests:

* `executor_transactions_for_intent_returns_orderbook_option_tx` —
  orderbook source path; tx confirmed with full receipt fields.
* `executor_transactions_for_intent_returns_rfq_option_tx_rfq_smoke_regression`
  — RFQ source path; uses the EXACT live-smoke tx hash
  `0x8538066c…5326` to demonstrate the bug no longer reproduces.

## 7. Confirmation worker linkage

The confirmation worker (`src/options/confirmation_worker.rs`) updates
`option_execution_transactions.confirmation_status` /
`.confirmed_at_ms` / `.confirmed_block_number` / `.receipt_status` /
`.gas_used` / `.effective_gas_price` / `.cumulative_gas_used` /
`.receipt_block_hash` / `.receipt_observed_at_ms` rows in place via
`PgRepository::update_option_execution_transaction_confirmation`. The
new endpoint reads those exact columns, so a worker run that flips
`pending` → `mined_success` becomes immediately visible to the operator
in the next API call. No new wiring needed.

## 8. Event / reconciliation linkage

The existing admin lifecycle endpoint
`/admin/options/executions/:intent_id/lifecycle` already aggregates
the full traversal `intent → tx → events → reconciliation` via
`crate::options::lifecycle::get_option_execution_lifecycle`. It
queries:

* `option_execution_intents` (intent metadata).
* `option_execution_transactions` (broadcast row + tx_hash).
* `option_execution_events` (events by tx_hash via
  `list_option_execution_events_by_tx_hash`).
* `option_execution_reconciliations`
  (`get_option_execution_reconciliation_by_transaction_id`).

That endpoint already worked correctly for both orderbook + RFQ
intents — the bugfix was solely in the operator-focused
`/executor/transactions/:intent_id` surface.

For consumers wanting the rich aggregated view (events list, fees,
transfers, reconciliation), the lifecycle endpoint is the canonical
surface; for consumers wanting the bare transaction rows, the
`/executor/transactions/:intent_id` surface is now correct.

## 9. Tests added

5 integration tests in `routes::tests`:

* `executor_transactions_for_intent_returns_orderbook_option_tx` —
  orderbook intent + confirmed tx → 1 row with `source: "option"` +
  `source_type: "option_orderbook_fill"` + full receipt fields.
* `executor_transactions_for_intent_returns_rfq_option_tx_rfq_smoke_regression`
  — RFQ intent + the EXACT live-smoke tx hash → 1 row with
  `source_type: "option_rfq_fill"`. Pin against the production
  regression scenario.
* `executor_transactions_for_intent_returns_empty_when_no_tx_exists`
  — intent present + no tx → `[]`. Pins no-false-positive contract.
* `executor_transactions_for_intent_returns_attempts_latest_first` —
  failed attempt + successful re-attempt → 2 rows, latest-first by
  `created_at_ms`. Pins ordering + multi-attempt support.
* `executor_transactions_for_intent_does_not_expose_secrets` — state
  carries `EXECUTOR_PRIVATE_KEY` + RPC URL with token + signer
  endpoint with secret path; response body contains none of those
  strings. Pins redaction.

## 10. Tests run

* `cargo fmt --check` — clean.
* `cargo clippy --all-targets --all-features -- -D warnings` — clean.
* `cargo test --all-targets --all-features --no-fail-fast` — **964 /
  964 green** (+5 from prior baseline of 959).
* `git diff --check` — clean.
* `forge fmt / build / test` — not re-run (no `sol/` source touched).

## 11. Remaining gaps

None for the operator-facing endpoint. Possible polish follow-ons (not
launch blockers):

* `BACKEND-EXECUTOR-TRANSACTIONS-LIST-EXTEND` — extend the
  `/executor/transactions` (list, no intent_id) endpoint to also
  surface OPTION rows. Today the list endpoint shows only legacy PERP
  rows. The bug was specifically in the by-intent variant; the list
  variant is less critical (operators typically filter by intent_id).
* `FRONTEND-OPTION-TX-LIST-CONSUME-VIEW` — frontend consumer update
  to render the new `source` + `source_type` discriminators.

## 12. Forbidden-list compliance

* No mainnet tx attempted. No Sepolia live broadcast.
* No Safe tx. No governance / Timelock / ownership / guardian
  mutation.
* No rebate reserve allocation. No PFV withdrawal. No fund movement.
* No RFQ / order smoke.
* No `.env` edit.
* No DB migration (additive or otherwise — no schema change was
  required).
* No private key / admin token / RPC secret / `DATABASE_URL` / API key
  in source or output.
* No real KMS key creation. No provider account creation.
* No guessed KMS provider credentials. No guessed mainnet executor
  address. No guessed PFV mainnet address.
* No webhook secret creation.
* No high-cardinality metric labels added (this fix changed no
  metrics).
* No fallback path that allows mainnet local-key signing.
* No bypass flag that weakens mainnet policy.
* No secrets printed. Redaction pinned by the dedicated test.

## 13. Next milestone recommendation

This was the last bug noted on the auditor-anchored backend remediation
list. Suggested next:

* **`BACKEND-EXECUTOR-TRANSACTIONS-LIST-EXTEND`** — small follow-on
  for the list-variant `/executor/transactions` endpoint.
* **`BACKEND-OBSERVABILITY-PROMETHEUS-FOR-HEALTH-V2-SINGLETONS`** —
  optional Grafana-friendly render of the 7 health-v2 singletons.

Parallel operator tracks unchanged: `MAINNET-KMS-VENDOR-SELECTION`
(Q-CD-5), `MAINNET-KMS-VENDOR-ADAPTER-IMPLEMENTATION`,
`MAINNET-AUDIT-EXT-KICKOFF`, `MAINNET-TREASURY-SAFE-CREATION-PACKET`,
`MAINNET-INSURANCE-OPERATOR-POLICY-PACKET`,
`FRONTEND-V2G-W3-SSR-PROXY`.
