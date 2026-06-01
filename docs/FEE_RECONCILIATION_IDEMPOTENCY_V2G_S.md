# V2G-S — Fee Reconciliation Idempotency

## Status

- Milestone: **V2G-S** — hardens backend V2 fee event ingestion,
  aggregation, and admin summaries against replay / overlapping
  block-range re-scans / dual-source-contract collisions /
  duplicate slice inputs.
- Date: 2026-06-01.
- Outcome:
  - **Defense-in-depth dedup at the aggregation boundary.**
    `normalize_fee_events` now drops in-slice duplicates keyed by
    `(model, tx_hash, log_index, source_contract)`. The existing
    DB `UNIQUE (chain_id, tx_hash, log_index)` index remains the
    primary dedup gate; the aggregator-side pass is an
    operational safety net for any caller that hands in a Vec
    with duplicates.
  - **New `by_product` / `by_flow` breakdowns.** `AggregatedFees`,
    `LifecycleFees`, and `OnchainFeeTxSummary` now expose
    per-product (`option` / `perp`) and per-flow (`orderbook` /
    `rfq`) splits for both positive fees AND rebates. Additive
    JSON-API change — older clients see new keys and continue
    working.
  - **11 new Rust unit tests** in
    `src/fees/onchain_summary.rs::tests` (all `v2gs_*`) covering
    every idempotency / by-product / by-flow case the task
    listed.
  - **No DB migrations.** No schema changes touched.
  - **No backend restart.** New code lands in `target/` only.
  - **Soak preserved.** PID 56199 + 4-container compose stack
    untouched.
- Hard gates respected: no broadcast, no deploy, no chain
  mutation, no backend restart, no compose touch, no
  Prometheus reset, no `.env` edit, no DB writes (incl. no
  migrations), no private-key handling, no soak interruption.

## Ingestion audit (read-only)

### Event sources

| Event | Emitter | Backend ingestion | Decode field |
|---|---|---|---|
| `TradingFeeCharged` (V1) | `MarginEngine`, `PerpEngine` (legacy compatibility) | `option_event_indexer` (engine event indexer) → DB | `appliedFee` (string) |
| `FeeChargedV2` (V2) | `FeesManagerV2` (option + perp consumers) | same indexer; same DB table | `feeAmount`, `feePpm`, `productKind`, `flowKind`, `basisAmount` |
| `FeeRebatedV2` (V2) | `FeesManagerV2` (rebate funding) | same indexer; same DB table | `rebateAmount`, `rebatePpm`, `productKind`, `flowKind`, `basisAmount` |
| `RebateBudgetSpent` | `FeesManagerV2` (audit trail only) | tracked separately in `RebateBudgetSpent` Prometheus metric; not joined into the V2 fee tx summary | n/a |

### Storage table

`option_execution_events`:
- Primary uniqueness: `UNIQUE INDEX (chain_id, tx_hash, log_index)`
  enforced at `INSERT ... ON CONFLICT (chain_id, tx_hash, log_index)
  DO NOTHING` (`src/db/repository.rs:3808`).
- That index is the **single source of truth for log dedup**.
  Re-scanning the same block range under the indexer therefore
  yields zero new rows for any log already on disk.

### Aggregation pipeline

`src/fees/onchain_summary.rs`:

```
DB rows (Vec<OptionExecutionEvent>)
   │
   ├─ normalize_fee_events  ← V2G-S: in-memory dedup pass
   │       │
   │       └─→ NormalizedFees { v1_charged, v2_charged, v2_rebated }
   │
   ├─ classify_event_model  → "v1" | "v2" | "mixed" | "none"
   │
   ├─ aggregate             → AggregatedFees
   │       │
   │       ├─ by_trader, by_recipient, by_side       (positive fees)
   │       ├─ by_product, by_flow                    (V2G-S additive)
   │       ├─ rebated_by_trader                      (rebates)
   │       └─ rebated_by_product, rebated_by_flow    (V2G-S additive)
   │
   ├─ summarize_fees_for_lifecycle      → LifecycleFees (intent view)
   └─ summarize_admin_onchain           → AdminOnchainSummary (admin view)
```

## Deduplication policy

### Primary: DB-level

`option_execution_events.UNIQUE (chain_id, tx_hash, log_index)` is
the canonical guarantee. The indexer's INSERT path uses
`ON CONFLICT DO NOTHING`, so:
- re-scan same block range → no-op,
- overlapping block ranges → no-op for overlap,
- restart indexer from older watermark → no-op.

### Secondary (V2G-S): aggregation-level

`normalize_fee_events` deduplicates against the key
`(FeeEventModel, tx_hash, log_index, source_contract)` on its
input Vec:

- `FeeEventModel` distinguishes V1 from V2 charge from V2 rebate.
- `tx_hash` + `log_index` are the on-chain identity of the log.
- `source_contract` (== `contract_address`) lets two distinct
  FeesManagerV2 instances (e.g. a legacy + new during a cutover)
  emit events at the same `(tx_hash, log_index)` slot without
  collapsing.

The aggregator therefore stays idempotent under:
- in-slice duplicates from a caller bug,
- doubled inputs from a paginated DB query stitched on the
  client side,
- overlapping block-range scans whose output is concatenated
  before being fed to `aggregate`.

### Tertiary: V1/V2 source-of-truth policy (pre-existing, retained)

When the same fee flow surfaces a V1 `TradingFeeCharged` log
AND a V2 `FeeChargedV2` log on the same tx, only the V2 event
contributes to totals. V1 is the *compatibility breadcrumb*
that legacy indexers still consume; the V2 event is the source
of truth. `classify_event_model` returns `"mixed"` and
`AggregatedFees.source_priority` is fixed to `"v2"`.

This is **not** a dedup — both events are surfaced in the
event list. It is a *counting policy*: V1 contributes zero to
totals when V2 is present.

## Reorg policy

In-scope for V2G-S: read-only documentation, not code.

The indexer currently does **not** delete a previously-inserted
fee event when a chain reorg replaces it. The
`(chain_id, tx_hash, log_index)` unique index is independent of
`block_hash` and `block_number`, so a reorged-out log remains in
the DB. On a shallow reorg (≤ confirmations watermark), the
indexer's confirmation worker decides which events are
"finalized" and the lifecycle/admin views read from confirmed
events only.

Carrying through to aggregation:
- `aggregate` does not distinguish confirmed vs unconfirmed
  events — that is the caller's responsibility (the caller
  filters `OptionExecutionEvent` rows by confirmation status).
- A reorg that removes a log will leave a stale row in the DB
  until ops manually prunes it; the V2G-S aggregator will
  continue to count it. **This is an open gap** to track but is
  outside V2G-S scope (no DB writes per hard gate).

Recommended follow-up (V2G-T or later): add a `confirmed_at` /
`reorged_at` column to `option_execution_events` and filter
unconfirmed/reorged rows out of the aggregator input. Requires
a non-destructive schema migration — not run here.

## Source-of-truth policy

| Event family present | `event_model` | `source_priority` | What drives totals |
|---|---|---|---|
| neither | `none` | `""` | — |
| only V1 | `v1` | `""` | V1 `appliedFee` |
| only V2 | `v2` | `""` | V2 `feeAmount` + `rebateAmount` |
| both | `mixed` | `"v2"` | V2 only; V1 zeros out |

`source_of_truth` on `LifecycleFees` is always `"onchain"` —
indexed events are canonical even when the off-chain fee ledger
disagrees. Backend ledger rows are surfaced via
`backend_ledger_status` for informational drift detection but
never override on-chain totals.

## Duplicate-handling policy

| Scenario | Outcome |
|---|---|
| Same physical log re-fed to `normalize_fee_events` in the same slice | Dropped at the dedup pass. First occurrence wins. |
| Same `(tx_hash, log_index)` from a different source contract | Counted separately (distinct dedup-key entries). |
| Same `(tx_hash, log_index)` across different `FeeEventModel` values (e.g. V1 + V2) | Counted separately at dedup; then the V1/V2 policy zeroes V1 out in totals. |
| Two distinct logs in same tx at different `log_index` | Counted separately. Standard case. |
| Replay same input Vec twice or thrice | `aggregate` totals identical to single-pass — pinned by `v2gs_replay_same_tx_twice_does_not_double_count_perp` and `v2gs_replay_three_times_perp_idempotent`. |
| Overlapping block-range scans whose outputs are concatenated | Dedup pass collapses overlap — pinned by `v2gs_overlapping_block_range_replay_safe`. |
| Admin `/admin/fees/onchain` re-queried with the same `tx_hash` filter | Deterministic output — pinned by `v2gs_admin_summary_per_tx_deterministic_under_replay`. |

## Tests added

### File

`src/fees/onchain_summary.rs::tests` — 11 new tests, all `v2gs_*`.

### Coverage matrix

| Test | Asserts |
|---|---|
| `v2gs_replay_same_tx_twice_does_not_double_count_perp` | PERP V2 charged + rebated; doubled input produces single-pass totals + correct by_product / by_flow / rebated_by_product. |
| `v2gs_replay_three_times_perp_idempotent` | Tripling the input is still a single-pass equivalent. |
| `v2gs_replay_mixed_option_v1_v2_does_not_double_count` | OPTION V1 + V2 + doubled — `event_model="mixed"`, `source_priority="v2"`, total = V2 only. |
| `v2gs_dup_log_index_within_same_family_is_deduped` | Two events sharing `(model, tx, log_idx, contract)` collapse to one; first occurrence wins. |
| `v2gs_distinct_log_index_same_tx_counted_separately` | Same tx, different `log_index` ⇒ both counted. |
| `v2gs_same_log_index_different_source_contracts_counted_separately` | Cutover-window safety: two FM-V2 instances emitting at colliding `(tx, log_idx)` stay separate. |
| `v2gs_by_product_and_by_flow_split_correctly` | Gross-fee breakdown: OPTION + PERP, ORDERBOOK + RFQ, by_product + by_flow keys/values match. |
| `v2gs_rebated_by_product_and_flow_split_correctly` | Rebate breakdown: same shape for rebated_by_product / rebated_by_flow. |
| `v2gs_admin_summary_per_tx_deterministic_under_replay` | `summarize_admin_onchain` is replay-safe: doubled input ⇒ same overall + same per_tx count. |
| `v2gs_overlapping_block_range_replay_safe` | Concatenating two indexer scans of overlapping ranges ⇒ single total. |
| `v2gs_lifecycle_view_exposes_by_product_and_by_flow` | `LifecycleFees.by_product` / `by_flow` populated correctly when fed an OPTION + PERP pair. |

## Files changed

### Backend (V2G-S behavioural changes)

- `src/fees/onchain_summary.rs`:
  - Added `BTreeSet` import.
  - `FeeEventModel` gained `Ord, PartialOrd` derives (required by the dedup key set).
  - `AggregatedFees` gained `by_product`, `by_flow`, `rebated_by_product`, `rebated_by_flow`.
  - `normalize_fee_events` now dedups against `(model, tx_hash, log_index, source_contract)`.
  - `accumulate_charge` / `accumulate_rebate` populate the new buckets.
  - `summarize_fees_for_lifecycle` and `OnchainFeeTxSummary` thread the new buckets through to JSON output.
  - 11 new `v2gs_*` tests + 2 new event-helper fixtures (`v2_perp_charged_event`, `v2_perp_rebated_event`).
- `src/options/lifecycle.rs`:
  - `LifecycleFees` gained 4 additive `BTreeMap<String,String>` fields with `#[serde(default)]` so older clients see empty maps and continue working.

### Backend (incidental, no behavioural change)

- `src/options/rfq_operator_packet.rs` — `cargo fmt --all` run during V2G-S validations reformatted two line-wrappings (an `encode_option_execute_rfq_trade_calldata` call and one `assert!` macro) inside the V2G-P1 module. No logic change; surfaced here only for full file-change transparency.

### Solidity / Frontend

- **Untouched.** `deopt-v2-sol` working tree is clean; `deopt-v2-frontend` not touched. Confirmed via `git status --short`.

### Database migrations

- **None.** Schema migration would be required to track
  `confirmed_at` / `reorged_at` for proper reorg filtering — that
  is documented as a remaining blocker, NOT applied here per the
  hard gate.

## Docs updated

- **New:** `deopt-v2-backend/docs/FEE_RECONCILIATION_IDEMPOTENCY_V2G_S.md` (this file).

## Validations run

| Command | Result |
|---|---|
| `cargo fmt --all -- --check` | ✅ clean |
| `cargo clippy --all-targets --all-features -- -D warnings` | ✅ clean |
| `cargo test --all-targets --all-features --no-fail-fast` | ✅ **724 / 0 / 0** total backend tests |
| `cargo test --lib 'fees::onchain_summary::tests::v2gs_'` | ✅ **11 / 0 / 0** for the V2G-S subset |
| `cargo build --all-targets --all-features` | ✅ |
| Solidity checks | not run — `deopt-v2-sol` working tree is clean; no Solidity files were touched in V2G-S |
| Frontend checks | not run — frontend not touched |

## Monitoring soak preservation status

| Check | State at V2G-S close |
|---|---|
| Backend PID 56199 alive | ✅ (no restart) |
| `/health` | ✅ |
| Prometheus `/-/healthy` | ✅ |
| Compose containers up | ✅ |
| Day-1 24h soak gate `2026-06-01T17:38Z` | reserved |
| No `docker compose down` | ✅ |
| No Prometheus reset | ✅ |
| No backend restart | ✅ |
| No `.env` edit | ✅ |
| No DB writes (incl. no migrations) | ✅ |

## Remaining blockers

1. **Reorg-aware aggregation.** Aggregator does not filter
   reorged-out rows from its input. Requires non-destructive
   schema migration to track confirmation / reorg state.
   Queued for V2G-T (out of V2G-S scope per "no DB migrations"
   hard gate).
2. **V2G-K day-1 24h gate** still reserved for
   `2026-06-01T17:38Z`. V2G-P broadcast cannot land before it
   clears.
3. **V2G-M endpoint pickup** requires a backend restart — the
   running PID 56199 (V2G-G era binary) does not expose the new
   `by_product` / `by_flow` JSON fields. The new code is in
   `target/` only; live `/admin/fees/onchain` payloads continue
   to surface the V2G-G shape until the next maintenance window.

## Next recommended milestone

**V2G-T — reorg-aware aggregation + indexer confirmation filter.**

1. Non-destructive schema migration: add `confirmed_at TIMESTAMPTZ`
   and `reorged_at TIMESTAMPTZ` to `option_execution_events`.
2. Indexer confirmation worker writes `confirmed_at` once depth
   ≥ confirmations watermark; sets `reorged_at` when a log is
   replaced by a reorg.
3. `aggregate` (or a wrapper) filters out rows with
   `reorged_at IS NOT NULL` by default, with a `?include_unconfirmed=true`
   query flag for ops.
4. New `v2gt_*` tests covering: confirmed-only path, reorg
   removal, ops-flagged unconfirmed inclusion.
5. Update Grafana alerts to surface `reorged_at` rate.

V2G-T can run before, during, or after V2G-P; it is orthogonal
to the OPTION RFQ broadcast.
