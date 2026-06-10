# BACKEND-EXECUTOR-TRANSACTIONS-LIST-EXTEND — result

**Posture:** SHIPPED at 2026-06-10.

## 1. Root cause

`OPTION-EXECUTION-TX-VISIBILITY-FIX` (2026-06-10) fixed the BY-INTENT
endpoint `/executor/transactions/:intent_id` to join both PERP and
OPTION tables. The LIST endpoint `GET /executor/transactions` was
left untouched: it queried only
`PgRepository::list_recent_execution_transactions` (legacy
`execution_transactions` table) and therefore omitted every OPTION
execution transaction from the operator's recent-tx scan.

The bug was reproducible against the live Sepolia RFQ smoke tx
`0x8538066c…5326`: present in
`option_execution_transactions`, invisible to the list endpoint.

## 2. Canonical unified list model

`GET /executor/transactions` now returns
`Vec<ExecutorTransactionView>` (same shape as the BY-INTENT variant)
combining rows from BOTH tables:

* **Legacy `execution_transactions` rows** map via
  `ExecutorTransactionView::from_perp` (source: `"perp"`,
  source_type: `null`).
* **`option_execution_transactions` rows** map via
  `ExecutorTransactionView::from_option` (source: `"option"`,
  source_type: `"option_orderbook_fill"` | `"option_rfq_fill"` |
  `null`).

Invariants:

* **Ordering:** latest first by `created_at_ms DESC` with
  `transaction_id DESC` tiebreak — same as the per-table SQL and the
  BY-INTENT variant.
* **Limit semantics:** the cap applies to the FINAL merged list, not
  to each side independently. A `?limit=50` request returns up to 50
  rows regardless of which table they came from. Each side is allowed
  to over-fetch (up to `limit`) so a single-side workload still
  delivers `limit` rows after merge + truncate.
* **Empty array** iff neither table has any row.
* **Read-only:** no RPC probes, no DB writes, no broadcasts.

## 3. Repository / query changes

NEW method `PgRepository::list_recent_option_execution_transactions(limit)`
at `src/db/repository.rs:1662` — returns
`Vec<(OptionExecutionTransaction, Option<String>)>` via a single
`LEFT JOIN` between `option_execution_transactions` and
`option_execution_intents`:

```sql
SELECT t.transaction_id, t.intent_id, t.onchain_intent_id, t.sender, …,
       i.source_type AS intent_source_type
FROM option_execution_transactions AS t
LEFT JOIN option_execution_intents AS i ON i.intent_id = t.intent_id
ORDER BY t.created_at_ms DESC, t.transaction_id DESC
LIMIT $1
```

This avoids an N+1 follow-up for `source_type`. Orphan rows
(missing intent — should not occur under normal write paths but
possible under data drift) carry `source_type = None` so operators
can spot the issue without a query failure.

No DB migration. No schema change. No existing query touched.

## 4. In-memory fallback changes

NEW method `OptionSeriesStore::list_recent_option_execution_transactions(limit)`
at `src/options/store.rs:1006`. Mirrors the SQL behavior:
sorts by `created_at_ms DESC` with `transaction_id DESC` tiebreak,
truncates to `limit`, then looks up each tx's parent intent's
`source_type` via the existing `get_option_execution_intent` lookup.
Used by the handler when `state.repository` is None (test / dry-run
contexts). The legacy `execution_transactions` table has no in-memory
fallback so the PERP side of the merge is empty in this mode —
mirrors the BY-INTENT variant's fallback.

## 5. Endpoint changes

`GET /executor/transactions` — same URL, same `limit` query parameter
(default 50, clamped to `[1, 500]`). Response type widened to
`Vec<ExecutorTransactionView>` (a SUPERSET of the legacy
`Vec<ExecutionTransaction>` JSON shape).

Handler logic:

1. Parse `limit` (clamped to `[1, 500]`).
2. With a repository: call both
   `list_recent_execution_transactions(limit)` +
   `list_recent_option_execution_transactions(limit)`.
3. Without a repository: in-memory fallback via the option store.
4. Set `chain_id` from `state.execution_config.executor_chain_id` if
   any row exists; else `null`.
5. Map both sides into `ExecutorTransactionView`s.
6. Sort merged list by `created_at_ms DESC` + `transaction_id DESC`.
7. Truncate to `limit`.
8. Return.

## 6. Ordering / pagination behavior

Ordering: latest first; tie-break by transaction_id DESC. Pagination:
the existing `limit` query parameter is preserved verbatim — no
breaking API change. There is no offset-based pagination today; this
milestone neither adds nor removes it.

## 7. Backward compatibility

The legacy `Vec<ExecutionTransaction>` JSON shape is a SUBSET of the
new `Vec<ExecutorTransactionView>` shape: every field name from the
legacy struct is preserved verbatim (`transaction_id`, `intent_id`,
`onchain_intent_id`, `target`, `calldata`, `value_wei`, `tx_hash`,
`status`, `error`, `confirmed_at_ms`, `confirmed_block_number`,
`confirmation_status`, `confirmation_error`, `created_at_ms`,
`updated_at_ms`). Existing PERP consumers reading those keys continue
to work unchanged. New additive fields (`source`, `source_type`,
`from`, `gas_limit`, `receipt_status`, `receipt_block_hash`,
`receipt_block_number`, `receipt_observed_at_ms`, `gas_used`,
`effective_gas_price`, `cumulative_gas_used`, `chain_id`) appear
alongside; JSON consumers tolerating unknown fields see no
disruption.

The smoke binary at `src/bin/mm_wt_smoke.rs:352, 472, 595, 809, 819`
compares the full JSON response between two snapshots — it works
because the JSON remains stable across consecutive calls when no tx
state has changed.

## 8. Orderbook / RFQ behavior

Both source types share the same `option_execution_transactions`
table and the same handler code path. Each row's `source_type`
field disambiguates the underlying intent
(`"option_orderbook_fill"` | `"option_rfq_fill"`). Pinned by two
integration tests:

* `executor_transactions_list_includes_option_orderbook_tx` — orderbook source.
* `executor_transactions_list_includes_option_rfq_tx_smoke_regression` —
  RFQ source; uses the EXACT live-smoke tx hash
  `0x8538066ce0a10ede63f9e4c66161be8efdcd0edf6a63d176af0967b4bde95326`.

## 9. Tests added

**6 new** integration tests in `routes::tests`:

* `executor_transactions_list_includes_option_orderbook_tx` —
  orderbook source visible with `source: "option"` +
  `source_type: "option_orderbook_fill"`.
* `executor_transactions_list_includes_option_rfq_tx_smoke_regression`
  — RFQ source visible; pinned with live-smoke tx hash
  `0x8538066c…5326`.
* `executor_transactions_list_orders_latest_first` — two rows
  with different `created_at_ms` → latest first.
* `executor_transactions_list_respects_limit_query_param` — 3
  rows + `?limit=1` → only the latest row returned.
* `executor_transactions_list_returns_empty_when_no_rows_exist` —
  empty state → `[]`.
* `executor_transactions_list_does_not_expose_secrets` — state
  carries `EXECUTOR_PRIVATE_KEY` + RPC URL with token + signer
  endpoint with secret path; response body contains none of those
  strings.

## 10. Tests run

* `cargo fmt --check` — clean.
* `cargo clippy --all-targets --all-features -- -D warnings` — clean.
* `cargo test --all-targets --all-features --no-fail-fast` — **970 /
  970 green** (+6 from prior baseline of 964).
* `git diff --check` — clean.
* `forge fmt / build / test` — not re-run (no `sol/` source touched).

## 11. Remaining gaps

* The `/executor/transactions` list endpoint no longer omits OPTION
  rows. Combined with the BY-INTENT fix from
  `OPTION-EXECUTION-TX-VISIBILITY-FIX`, the operator-facing
  transaction surface is now fully unified across both tables.
* Frontend consumers can render `source` + `source_type` for
  improved categorisation; the JSON shape change is additive so they
  are free to adopt at their own pace.

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
* No high-cardinality metric labels added.
* No fallback path that allows mainnet local-key signing.
* No bypass flag that weakens mainnet policy.
* No secrets printed. Redaction pinned by the dedicated test.

## 13. Next milestone recommendation

* **`BACKEND-OBSERVABILITY-PROMETHEUS-FOR-HEALTH-V2-SINGLETONS`** —
  render the 7 health-v2 singletons as Prometheus gauges for Grafana
  charts.
* **`FRONTEND-OPTION-TX-LIST-CONSUME-VIEW`** — frontend consumer
  update to render the new `source` + `source_type` discriminators
  in the operator UI.

Parallel operator tracks unchanged: `MAINNET-KMS-VENDOR-SELECTION`
(Q-CD-5), `MAINNET-KMS-VENDOR-ADAPTER-IMPLEMENTATION`,
`MAINNET-AUDIT-EXT-KICKOFF`, `MAINNET-TREASURY-SAFE-CREATION-PACKET`,
`MAINNET-INSURANCE-OPERATOR-POLICY-PACKET`,
`FRONTEND-V2G-W3-SSR-PROXY`.
