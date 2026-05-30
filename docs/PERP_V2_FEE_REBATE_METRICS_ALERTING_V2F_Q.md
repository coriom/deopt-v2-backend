# V2F-Q — PERP V2 Fee Rebate Metric & Alert Delivery

## Status

- Milestone: **V2F-Q** (follows V2F-P).
- Date: 2026-05-30.
- Mode: backend-only; **no Solidity, no broadcast, no live chain
  mutation, no destructive DB action, no real `.env` edit, no raw
  address labels**.
- Outcome: a sibling `FeeRebatedV2` consumer metric for the V2F-P
  metric, deployable Prometheus rules under `docs/alertmanager/`,
  and a runbook covering all three PERP V2 fee alerts.

## Audit summary (Phase 1)

V2F-P shipped:

- A pure classifier `src/fees/perp_consumer.rs::classify_perp_fee_consumer`
  with the bucket constants `CONSUMER_NEW` / `CONSUMER_OLD` /
  `CONSUMER_UNKNOWN`. Pure, no allocations.
- A repository helper `PgRepository::admin_perp_fee_v2_consumer_counts`
  filtering `option_execution_events` for `event_name='FeeChargedV2'`
  AND `decoded->>'productKind'='perp'`.
- An in-memory mirror `OptionsStore::perp_fee_v2_consumer_counts`.
- A metric-rendering helper
  `monitoring.rs::append_perp_fee_v2_consumer_metric` that
  pre-seeds the three buckets and emits the gauge
  `deopt_perp_fee_charged_v2_total{consumer=...}`.
- 8 classifier unit tests + 4 endpoint metric tests pinning
  cardinality, exclusion, and the no-raw-address invariant.

The helper was specialised to a single metric name. To add a
sibling rebate metric without copy-pasting the bucket-build path,
V2F-Q widened the helper signature with `(metric_name,
metric_help)` parameters so both metrics flow through the same
classifier + pre-seeding code.

## Implementation summary (Phase 2)

1. **Repository helper.** Refactored `admin_perp_fee_v2_consumer_counts`
   to delegate to a private `admin_perp_fee_v2_consumer_counts_for_event(event_name)`
   that accepts either `"FeeChargedV2"` or `"FeeRebatedV2"` and
   keeps `decoded->>'productKind' = 'perp'`. Added the public sibling
   `admin_perp_fee_v2_rebated_consumer_counts`.
2. **In-memory store helper.** Same refactor:
   `perp_fee_v2_consumer_counts_for_event(event_name)` private
   method backing both `perp_fee_v2_consumer_counts` (charged) and
   the new `perp_fee_v2_rebated_consumer_counts` (rebated). Both
   filter on `productKind == "perp"` and exclude any decoded value
   that doesn't have a `consumer` field.
3. **Metric-rendering helper.** Refactored
   `monitoring.rs::append_perp_fee_v2_consumer_metric` to take
   `metric_name: &'static str` and `metric_help: &'static str`
   parameters. The classifier and bucket pre-seed path are
   identical between charged and rebated.
4. **Scrape wiring.** `append_fee_metrics` now fetches both raw
   counts maps (charged and rebated) from the repository or store
   and calls the metric helper twice — once with
   `deopt_perp_fee_charged_v2_total` and once with
   `deopt_perp_fee_rebated_v2_total`.

## Charged / rebated metric names

```
deopt_perp_fee_charged_v2_total{consumer="new"|"old"|"unknown"}     # V2F-P
deopt_perp_fee_rebated_v2_total{consumer="new"|"old"|"unknown"}     # V2F-Q
```

Both metrics are pull-derived gauges (the `_total` suffix is a
naming convention; the underlying source — the append-only
`option_execution_events` ledger — gives the gauge step-function
semantics, so `increase(...)[5m] > 0` is a valid rule against
either). A future milestone can promote both to a proper
monotonic counter once the metrics module grows one; the alert
rules will not need to change.

Help strings:
- charged: `"PERP FeeChargedV2 events bucketed by consumer engine (new=current, old=stranded, unknown=neither)."`
- rebated: `"PERP FeeRebatedV2 events bucketed by consumer engine (new=current, old=stranded, unknown=neither)."`

## Classification behavior

Identical for both metrics — they share
`src/fees/perp_consumer.rs::classify_perp_fee_consumer`.

| Input `consumer` | NEW configured | OLD configured | Bucket |
| --- | --- | --- | --- |
| Matches NEW (case-insensitive) | yes | any | `new` |
| Matches OLD (case-insensitive) | yes | yes | `old` |
| Doesn't match either | yes | yes | `unknown` |
| Anything | unset | unset | `unknown` |
| Zero address (consumer or configured) | — | — | does not match |
| Empty/whitespace consumer | — | — | `unknown` |

OPTION-flavoured rebates and `FeeChargedV2` events are excluded at
the SQL/store layer and never reach the classifier — verified by
the new tests below.

## Tests added (Phase 3)

End-to-end `/metrics` rendering tests in `src/api/routes.rs::tests`
(4 tests, all green; in addition to the 4 V2F-P charged tests that
still pass):

- `perp_fee_rebated_v2_metric_emits_three_buckets_at_zero` —
  empty backend; `# HELP` / `# TYPE` present and all three
  `consumer=...{0}` series exposed.
- `perp_fee_rebated_v2_metric_classifies_new_and_excludes_option_and_charged` —
  seeds a PERP rebate from NEW + an OPTION rebate + a PERP
  `FeeChargedV2`. Asserts:
  - rebate metric: `new=1, old=0, unknown=0`.
  - charged metric: `new=1` (the PERP `FeeChargedV2` is still
    counted on its own metric).
  - rendered body contains none of the configured NEW/OLD/trader
    addresses (low-cardinality invariant).
- `perp_fee_rebated_v2_metric_classifies_old_consumer` — seeds a
  PERP rebate with `consumer = OLD`; asserts `old=1` and the OLD
  address does not leak.
- `perp_fee_rebated_v2_metric_classifies_unknown_consumer` — seeds
  a PERP rebate with a stray consumer; asserts `unknown=1` and no
  address leaks.

The pre-existing classifier unit tests (8) and V2F-P endpoint tests
(4) still pass — the metric path is symmetric across charged and
rebated.

## Alert rules added (Phase 4)

Deployable Prometheus rules now live at
`docs/alertmanager/perp_v2_fee_alerts.yml`. Three alerts:

- **`PerpFeeChargedFromOldEngine`** (V2F-O / V2F-P, re-stated for
  completeness):
  `increase(deopt_perp_fee_charged_v2_total{consumer="old"}[5m]) > 0`.
- **`PerpFeeRebatedFromOldEngine`** (V2F-Q, new):
  `increase(deopt_perp_fee_rebated_v2_total{consumer="old"}[5m]) > 0`.
- **`PerpFeeConsumerUnknown`** (V2F-Q, new):
  `increase(deopt_perp_fee_charged_v2_total{consumer="unknown"}[5m]) + increase(deopt_perp_fee_rebated_v2_total{consumer="unknown"}[5m]) > 0`.

All three rules default to `severity: high` (charged/rebated) or
`severity: medium` (unknown). Override at the Alertmanager rule
layer for mainnet:
- charged/rebated → `severity: critical`;
- unknown → `severity: high`.

Each rule carries a `runbook_url` annotation pointing at the
relevant anchor in `docs/RUNBOOK_PERP_V2_FEE_ALERTS.md`.

## Runbook path

`docs/RUNBOOK_PERP_V2_FEE_ALERTS.md` (new). Covers, for each of
the three alerts:

- Meaning and expected current value.
- Severity matrix (Base Sepolia vs mainnet).
- First-response forensics checks shared across all three alerts
  (`/metrics` shape, DB lookup, `/admin/fees/onchain?tx_hash=…`,
  on-chain `cast call` snapshots of `FeesManagerV2.feeConsumers()`,
  manifest cross-check).
- Per-alert remediation in priority order, including the
  rebate-specific `rebateBudget(asset)` snapshot.
- An explicit do-not list: do not delete DB rows, do not broadcast
  from the backend host, do not mutate `.env` during the incident,
  do not silently widen classifier buckets.
- All-clear verification (Prometheus `[5m]` window + DB count
  query).

## No-chain-mutation proof

- The rebate metric is derived from already-persisted
  `option_execution_events` rows. No new SQL writes, no new RPC
  calls, no transaction signing path is touched.
- `OLD_PERP_ENGINE_ADDRESS` env var continues to be read only by
  the metric pipeline
  (`monitoring.rs::append_perp_fee_v2_consumer_metric`). It is not
  used by `executor`, `transaction`, `simulator`, or
  `perp_nonce_sync`.
- No `.env` real edit. No DB row delete. No Solidity change.
- All four `cargo` validations passed locally (see "Validation
  commands run").

## Remaining gaps

- Both metrics remain pull-derived gauges with `_total` names. A
  follow-up may promote them to a monotonic counter primitive once
  the metrics module grows one; the alert YAML will not need to
  change (the same `increase(...)[5m]` query works against a
  counter).
- `PerpFeeConsumerUnknown` currently bundles the charged-unknown
  and rebated-unknown buckets into a single rule. If routing teams
  want per-family routing, split into two sibling rules — the
  metric labels already support it.
- The Alertmanager YAML is rules-only; the receivers/routes
  (PagerDuty / Slack / e-mail) remain environment-specific and
  belong in the operator deployment repo.
- A live PERP `FeeRebatedV2` has not been observed on Base Sepolia
  yet (V2F-LM emitted zero rebates by design). The first live
  rebate event will exercise the metric path on real data; the
  regression tests substitute for that until then.

## Acceptance checklist

- [x] `deopt_perp_fee_rebated_v2_total{consumer=...}` exists.
- [x] All three buckets pre-seeded.
- [x] PERP rebates counted; OPTION rebates excluded; `FeeChargedV2`
  excluded from the rebate metric.
- [x] Raw addresses never emitted as labels (verified by
  `classifier_never_emits_raw_address` and the per-test body
  assertions).
- [x] `PerpFeeRebatedFromOldEngine` rule recorded in
  `docs/ALERTING_SPEC.md` and as deployable YAML in
  `docs/alertmanager/perp_v2_fee_alerts.yml`.
- [x] `PerpFeeConsumerUnknown` rule recorded in both files.
- [x] Runbook `docs/RUNBOOK_PERP_V2_FEE_ALERTS.md` created.
- [x] V2F-P doc updated (this milestone referenced).
- [x] ALERTING_SPEC updated (rebated + unknown alerts).
- [x] All four `cargo` validations pass.

## Validation commands run

```
cargo fmt --all                                          ✅ no diff
cargo clippy --all-targets --all-features -- -D warnings ✅ clean
cargo test --all-targets --all-features --no-fail-fast   ✅ all suites pass (4 new rebated tests + previous 8 classifier + 4 charged tests)
cargo build --all-targets --all-features                 ✅ Finished
```

## Next recommended milestone

**V2F-R — Promote PERP V2 Fee Counters & Live Rebate Smoke**
(backend + optional Solidity-coordinated smoke, no broadcast on its own):

1. Promote the metrics module to support a monotonic counter
   primitive and switch both
   `deopt_perp_fee_charged_v2_total` and
   `deopt_perp_fee_rebated_v2_total` from gauges to true counters,
   preserving the existing `{consumer="new"|"old"|"unknown"}`
   cardinality contract. The alert YAML stays valid as-is.
2. Coordinate with the Solidity team to land a tiny V2 maker-rebate
   tier on Base Sepolia so the first PERP `FeeRebatedV2` event is
   emitted and the new metric path is exercised on live data
   (V2F-Q's tests are regression-only until then).
3. Promote the cardinality cap discussion in V2F-P into code: refuse
   to render the metric if the classifier sees more than N distinct
   consumer addresses in a single scrape (defense-in-depth against
   a contract regression).
4. Add an Alertmanager smoke check
   (`promtool check rules docs/alertmanager/perp_v2_fee_alerts.yml`)
   to CI so the YAML cannot rot.
