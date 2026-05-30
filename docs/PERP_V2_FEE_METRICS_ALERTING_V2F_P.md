# V2F-P — PERP V2 Fee Metrics And OLD-Engine Alert Instrumentation

## Status

- Milestone: **V2F-P** (follows V2F-O live observability verification).
- Date: 2026-05-30.
- Mode: backend-only; **no Solidity, no broadcast, no live chain
  mutation, no destructive DB action, no real `.env` edit, no raw
  address labels**.
- Outcome: V2F-O's spec-only `perp_fee_charged_from_old_engine` alert
  becomes an **instrumented**, deployable Prometheus rule keyed off a
  new low-cardinality counter.

## Metric

Name: `deopt_perp_fee_charged_v2_total`

Help (Prometheus `# HELP`): "PERP FeeChargedV2 events bucketed by
consumer engine (new=current, old=stranded, unknown=neither)."

Type: gauge (always-emitted, derived from the persisted event
ledger at scrape time; the V2F-O verification proved the event
ledger is the source of truth, and the underlying
`(chain_id, tx_hash, log_index)` uniqueness constraint in
`option_execution_events` makes re-indexing idempotent).

Labels (exhaustive, low-cardinality):

| Label | Allowed values | Notes |
| --- | --- | --- |
| `consumer` | `new` \| `old` \| `unknown` | The classifier `src/fees/perp_consumer.rs::classify_perp_fee_consumer` never emits any other value. |

Raw addresses (trader, recipient, contract, OLD engine, NEW engine),
tx hashes, log indices, settlement assets, intent IDs, option IDs,
and any other high-cardinality value are **explicitly not promoted to
labels**. The classifier returns one of three `&'static str` constants
(`CONSUMER_NEW`, `CONSUMER_OLD`, `CONSUMER_UNKNOWN`); no other return
path exists.

## Classification rules

Implemented by `src/fees/perp_consumer.rs::classify_perp_fee_consumer`
and exercised at scrape time by `src/monitoring.rs::append_perp_fee_v2_consumer_metric`.

1. **Only PERP `FeeChargedV2` events contribute.** The SQL filter
   (`PgRepository::admin_perp_fee_v2_consumer_counts`) and the
   in-memory mirror
   (`OptionsStore::perp_fee_v2_consumer_counts`) require
   `event_name = 'FeeChargedV2'` AND `decoded->>'productKind' = 'perp'`.
   `FeeRebatedV2` rows and OPTION-flavoured `FeeChargedV2` rows are
   excluded by construction — they never reach the classifier.
2. **Match `decoded.consumer` against configured NEW / OLD addresses
   case-insensitively.** Whitespace is trimmed; the zero address is
   treated as "not configured" on both sides.
3. **Outputs:**
   - `consumer == NEW_PERP_ENGINE_ADDRESS` (from `PERP_ENGINE_ADDRESS`
     env var) → `"new"`.
   - `consumer == OLD_PERP_ENGINE_ADDRESS` env var → `"old"`.
   - Neither configured, or no match → `"unknown"`.
4. **Always emit all three label values** (with zero counts when
   absent) so the Prometheus rule
   `increase(deopt_perp_fee_charged_v2_total{consumer="old"}[5m]) > 0`
   has a stable time series to alert on from the first scrape.

## Tests

Pure-classifier tests in `src/fees/perp_consumer.rs::tests`:

- `matches_new_consumer_case_insensitively`
- `matches_old_consumer_case_insensitively`
- `returns_unknown_when_no_addresses_configured`
- `returns_unknown_when_old_unset_and_consumer_is_not_new`
- `returns_unknown_when_addresses_are_zero`
- `returns_unknown_for_unrelated_consumer`
- `empty_consumer_resolves_to_unknown`
- `classifier_never_emits_raw_address` (asserts the classifier output
  is always one of `"new"`, `"old"`, `"unknown"` for every input
  shape).

End-to-end `/metrics` rendering tests in `src/api/routes.rs::tests`:

- `perp_fee_charged_v2_metric_emits_three_buckets_at_zero` — confirms
  all three series are exposed even on an empty backend, with
  matching `# HELP` / `# TYPE` lines.
- `perp_fee_charged_v2_metric_classifies_new_and_excludes_option_and_rebate` —
  seeds a PERP V2 from NEW + an OPTION V2 + a V2 rebate, asserts
  `consumer="new"` is `1`, the other two stay at `0`, and asserts the
  raw NEW/OLD addresses + the trader address + the rebate trader
  address **do not appear anywhere** in the rendered body.
- `perp_fee_charged_v2_metric_classifies_old_consumer` — seeds a PERP
  V2 with `consumer = OLD`, asserts `consumer="old"` is `1` and the
  raw OLD address does not appear in the body.
- `perp_fee_charged_v2_metric_classifies_unknown_consumer` — seeds a
  PERP V2 with a stray consumer address, asserts `consumer="unknown"`
  is `1` and neither configured address nor the stray address
  appears in the body.

The pre-existing
`metrics_do_not_expose_secrets_or_high_cardinality_values` test
(unchanged) continues to guard the broader cardinality contract.

## Prometheus alert rule

Deployable YAML (also recorded in `docs/ALERTING_SPEC.md`):

```yaml
- alert: PerpFeeChargedFromOldEngine
  expr: increase(deopt_perp_fee_charged_v2_total{consumer="old"}[5m]) > 0
  for: 0m
  labels:
    severity: high
  annotations:
    summary: "PERP FeeChargedV2 emitted from OLD stranded PerpEngine"
    description: "OLD PerpEngine should not emit new PERP V2 fee events after A3 fallback. Snapshot FeesManagerV2.feeConsumers() and verify engine wiring against the V2F-LM acceptance state."
```

Severity override guidance: on mainnet, the rule should be deployed
with `severity: critical` (override the `severity: high` label).
Alert delivery wiring (Alertmanager → PagerDuty / Slack) is
environment-specific and is intentionally out of scope of this repo.

## Env example changes

`/.env.example` updated (real `.env` untouched):

- `PERP_MATCHING_ENGINE_ADDRESS` switched from the zero address to the
  Base Sepolia V2F value `0x774d96…F165`.
- `PERP_ENGINE_ADDRESS` switched from the zero address to **NEW**
  (`0xc6C592…141c`). A comment block warns operators never to point
  this at the OLD stranded address because
  `FeesManagerV2.isFeeConsumer(OLD) = false` after V2F-LM and any
  routed trade would revert.
- Added `OLD_PERP_ENGINE_ADDRESS=0xB36395…b53B` with a comment
  explaining it is observability-only, used solely by the
  `deopt_perp_fee_charged_v2_total{consumer="old"}` metric and the
  `PerpFeeChargedFromOldEngine` alert. Leave unset to bucket
  everything non-NEW as `unknown`.

Real `.env` (which still references OLD in `PERP_ENGINE_ADDRESS`) is
intentionally **not** modified by this milestone — that flip belongs
to operator-driven runtime config rotation, not to the source tree.

## No-chain-mutation proof

- The metric is derived from already-persisted
  `option_execution_events` rows. No new SQL writes, no new RPC calls,
  no new transaction signing surface.
- `OLD_PERP_ENGINE_ADDRESS` is read only by the metric path
  (`monitoring.rs::append_perp_fee_v2_consumer_metric`). It is **not**
  used by `executor`, `transaction`, `simulator`, or `perp_nonce_sync`
  — grep-verified.
- No `.env` real edit. No DB row delete. No Solidity change.
- All seven `cargo` validations passed locally (see "Validation
  commands run" below).

## V2F-Q follow-up (2026-05-30)

Sibling rebate metric and alert delivery shipped in the V2F-Q
milestone:

- `deopt_perp_fee_rebated_v2_total{consumer="new"|"old"|"unknown"}`
  emitted from the same scrape path; reuses the V2F-P classifier and
  pre-seed pattern after a small refactor that lifted
  `append_perp_fee_v2_consumer_metric` into a parameterised helper.
- New repository/store helpers
  `admin_perp_fee_v2_rebated_consumer_counts` and
  `perp_fee_v2_rebated_consumer_counts` mirror the V2F-P helpers,
  filtering on `event_name = 'FeeRebatedV2'` and `productKind =
  'perp'`.
- Two new Prometheus rules — `PerpFeeRebatedFromOldEngine` and
  `PerpFeeConsumerUnknown` — recorded both in
  `docs/ALERTING_SPEC.md` and as deployable YAML at
  `docs/alertmanager/perp_v2_fee_alerts.yml`.
- Runbook `docs/RUNBOOK_PERP_V2_FEE_ALERTS.md` covers all three
  alerts (charged-from-OLD, rebated-from-OLD, consumer-unknown)
  with shared forensics, severity matrix, and remediation order.

See `docs/PERP_V2_FEE_REBATE_METRICS_ALERTING_V2F_Q.md` for the
full V2F-Q record.

## Remaining gaps

- The metric currently emits a gauge of cumulative event counts. A
  follow-up could promote it to a proper counter (monotonic
  `_total`) once the metrics module grows a counter primitive, so
  `rate()` queries align with Prometheus naming conventions. The
  current `increase(...)[5m] > 0` rule works against the gauge as a
  step function because the underlying event ledger is append-only.
- Alert delivery wiring (Alertmanager rules file, Slack/PagerDuty
  routes) is still environment-specific and not in this repo.
- `FeeRebatedV2` PERP variant is not yet observed on Base Sepolia
  (V2F-LM emitted zero rebates by design); the first PERP rebate will
  be the natural follow-up to extend the metric or add a sibling
  counter.
- A future milestone could add a `consumer_count` cap in monitoring
  (refuse to emit if `raw_counts` returns >1024 distinct consumer
  addresses) as a defense-in-depth against accidental cardinality
  growth from a contract regression.

## Acceptance checklist

- [x] Metric `deopt_perp_fee_charged_v2_total{consumer=...}` exists
  and always emits `new`/`old`/`unknown` series.
- [x] Classification function exists and is pure.
- [x] Tests cover `new`/`old`/`unknown`, OPTION exclusion, rebate
  exclusion, and no-raw-address-label invariants.
- [x] Prometheus alert rule recorded as deployable YAML in
  `docs/ALERTING_SPEC.md`.
- [x] `.env.example` updated; real `.env` left alone.
- [x] V2F-O and ALERTING_SPEC docs updated to point at this milestone.
- [x] All four `cargo` validations pass.

## Validation commands run

```
cargo fmt --all                                          ✅ no diff
cargo clippy --all-targets --all-features -- -D warnings ✅ clean
cargo test --all-targets --all-features --no-fail-fast   ✅ all suites pass (8 classifier tests + 4 endpoint tests added)
cargo build --all-targets --all-features                 ✅ Finished
```

## Next recommended milestone

**V2F-Q — Alert Delivery Wiring & Counter Hardening** (backend-only,
no Solidity, no broadcast):

1. Migrate `deopt_perp_fee_charged_v2_total` from gauge to monotonic
   counter primitive once the metrics module grows one; convert the
   alert rule to `rate(...)` while preserving the
   `consumer="new"|"old"|"unknown"` cardinality contract.
2. Add a sibling
   `deopt_perp_fee_rebated_v2_total{consumer="new"|"old"|"unknown"}`
   ready for the first PERP rebate event.
3. Add a cardinality-cap guard in
   `append_perp_fee_v2_consumer_metric` that refuses to render if the
   classifier somehow sees more than N distinct consumer addresses
   (defense-in-depth against contract regression).
4. Provide an Alertmanager rules file under `deploy/` (or
   `docs/alertmanager/`) with both alerts pre-wired and an example
   PagerDuty / Slack receiver route.
5. Add a runbook entry that walks an oncall through the
   forensics sequence (snapshot `FeesManagerV2.feeConsumers()`,
   `useFeesManagerV2()` per engine, last block before the alert).
