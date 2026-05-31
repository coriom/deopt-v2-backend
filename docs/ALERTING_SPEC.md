# Alerting Spec V1B

This document defines suggested monitoring alerts for DeOpt v2 backend operators. It is a specification only: V1B does not deploy Prometheus, Grafana, Slack, Telegram, PagerDuty, webhooks, or any external alert delivery.

Metrics are exposed at `/metrics` in Prometheus text format when `METRICS_ENABLED=true`. Alert rules should use only low-cardinality labels: `status`, `market_type`, `flow_type`, and `source_type`. Do not introduce wallet addresses, tx hashes, UUIDs, raw DB URLs, raw RPC URLs, private keys, admin tokens, signatures, session ids, or object ids as labels.

Operational logs should keep the same safety policy. When adding new logs around these alerts, prefer structured fields such as `service`, `subsystem`, `event`, `status`, and `error_code`; keep high-cardinality ids and secrets out of alert labels and avoid logging secret material.

## Core Alerts

### Backend Down

- Signal: `deopt_backend_up == 0` or scrape target unavailable.
- Severity: critical.
- Intent: the API process is not reachable.
- Suggested action: check process supervisor, container health, recent deploys, and host/network reachability.

### Readiness Failed

- Signal: `/ready` returns non-2xx.
- Severity: critical when sustained.
- Intent: the process is running but is not ready to serve normal traffic.
- Suggested action: inspect readiness JSON checks. If persistence is enabled, verify Postgres connectivity.

### Database Down

- Signal: `deopt_persistence_enabled == 1 and deopt_db_up == 0`.
- Severity: critical.
- Intent: persistent mode is enabled but the backend cannot ping Postgres.
- Suggested action: check Postgres service health, credentials, network route, migrations, and connection limits.

### Migration Drift

- Signal: `deopt_db_migrations_installed` lower than the expected release count.
- Severity: warning.
- Intent: database may not be at the schema level expected by the running binary.
- Suggested action: compare the deployed image with `migrations/` and migration logs.

### Real Broadcast Unexpectedly Enabled

- Signal: `deopt_real_broadcast_enabled == 1` in any environment where broadcast should be disabled.
- Severity: critical.
- Intent: the backend has crossed the default safety boundary for `eth_sendRawTransaction`.
- Suggested action: halt the deployment or remove traffic until config is verified.

## Execution Alerts

### Execution Confirmations Stuck

- Signal: `deopt_execution_transactions_total{status="submitted"}` sustained above zero while `deopt_execution_confirmed_total` does not increase.
- Severity: warning, critical if sustained beyond finality expectations.
- Intent: submitted execution transactions are not reaching confirmed state.
- Suggested action: check confirmation worker config, RPC availability, receipt errors, and reconciliation identity matches.

### Simulation Failures Rising

- Signal: `deopt_execution_simulations_total{status="simulation_failed"}` rate or delta rises above the expected baseline.
- Severity: warning.
- Intent: generated calldata or simulated market state may be failing more often than expected.
- Suggested action: inspect bounded admin recent failure summaries and decode categories without alert labels containing calldata or ids.

### Reconciliation Unmatched Or Ambiguous Rising

- Signal: future reconciliation metrics show increasing unmatched or ambiguous statuses.
- Severity: warning.
- Intent: indexed chain events are not linking cleanly to backend execution intents.
- Suggested action: inspect reconciliation summaries, indexed event windows, and intent identity fields.

## Indexer Alerts

### Stale Indexer Cursor

- Signal: future indexer cursor metric stops advancing while `deopt_indexer_enabled == 1`.
- Severity: warning.
- Intent: log indexing may be stalled.
- Suggested action: check RPC health, configured start block/ranges, and indexer logs.

## RFQ And Options Alerts

### RFQ Quote Rejection Spike

- Signal: `deopt_rfq_quotes_total{status="rejected"}` or `deopt_option_rfq_quotes_total{status="rejected"}` increases faster than normal.
- Severity: warning.
- Intent: quotes are being rejected or invalidated at an unusual rate.
- Suggested action: compare quote signature mode, RFQ expiry windows, MM connectivity, and permission rules.

### Option RFQ Fill Drop

- Signal: `deopt_option_rfqs_total{status="accepted"}` grows while `deopt_option_fills_total` does not move as expected.
- Severity: warning.
- Intent: option RFQ acceptance or fill recording may be impaired.
- Suggested action: inspect option RFQ summary and fee ledger summaries.

## Market Maker Alerts

### MM Session Drop

- Signal: `deopt_mm_gateway_enabled == 1` and `deopt_mm_sessions_total` or `deopt_mm_sessions_authenticated_total` drops below an expected floor.
- Severity: warning.
- Intent: connected market maker capacity dropped.
- Suggested action: check WebTransport listener health, certificates, auth challenge flow, and MM client logs.

## Fees Alerts

### Fee Ledger Write Failures

- Signal: application logs show fee ledger persistence errors or fee event counts stop increasing while fills/trades are expected.
- Severity: warning.
- Intent: accounting rows may not be recorded.
- Suggested action: check Postgres health, fee feature flags, and recent fee admin summaries.

### Rebate Accrual Stalled

- Signal: `deopt_rebate_accruals_total{status="accrued"}` does not move while rebate-eligible volume is expected.
- Severity: warning.
- Intent: rebate accounting may be disabled or not recording eligible flow.
- Suggested action: verify `FEES_REBATES_ENABLED`, MM permissions, and fee configuration.

### PERP FeeChargedV2 From OLD PerpEngine (V2F-O / V2F-P)

- Name: `perp_fee_charged_from_old_engine`
- Metric (V2F-P): `deopt_perp_fee_charged_v2_total{consumer="new"|"old"|"unknown"}`
- Prometheus rule (deployable):

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

- Severity: high on Base Sepolia, critical on mainnet (override the
  rule's `severity: high` label in production).
- Signal (under the metric): any indexed `FeeChargedV2` event where
  - `decoded.productKind == "perp"` (or `productKindRaw == 1`), **and**
  - `decoded.consumer` matches the configured `OLD_PERP_ENGINE_ADDRESS`
    (currently `0xB36395b67D0798ADA981731c9Fa5239F4362b53B` on Base
    Sepolia — confirm per environment before deploying the rule).

  The backend computes this in
  `src/monitoring.rs::append_perp_fee_v2_consumer_metric` using the
  pure classifier
  `src/fees/perp_consumer.rs::classify_perp_fee_consumer`. Raw
  addresses are **never** promoted to a metric label.
- Intent: after the V2F cutover, `NEW_PERP_ENGINE`
  (`0xc6C592100723Fe0C66343A16e95eC34cC0c2141c`) is the only PERP fee
  consumer registered with FeesManagerV2
  (`FeesManagerV2.isFeeConsumer(NEW) == true`). The OLD engine is
  stranded under the A3 Base Sepolia fallback and is **expected to
  emit zero new PERP fees**. Any nonzero occurrence indicates one of:
  (a) FeesManagerV2 was re-pointed at OLD; (b) OLD was re-allowed as a
  fee consumer; (c) operator runbooks routed an executor at OLD; or
  (d) a contract-level regression.
- Expected current value: zero (verified live for tx
  `0x400acedf36381034ae37c983cc50e80d11a81587ca8065fbaef40293ff63a79a`
  at block `42188599`, where `consumer == NEW_PERP_ENGINE`).
- Companion checks (forensics, after the alert fires):
  - `GET /admin/fees/onchain?tx_hash=…` then look at
    `events[*].source_contract == FEES_MANAGER_V2` and the decoded
    `consumer` topic on the raw row
    (`option_execution_events.decoded->>'consumer'`).
  - Snapshot `FeesManagerV2.feeConsumers()` and `useFeesManagerV2()`
    on every engine; compare to the V2F-LM acceptance state.
- Suggested action: halt PerpEngine writes, snapshot
  `FeesManagerV2.feeConsumers()` and `useFeesManagerV2()` on every
  engine, and verify wiring against the V2F-LM acceptance state.
- Cardinality guarantees: the metric emits exactly three series
  (`consumer="new"|"old"|"unknown"`) per scrape. Raw addresses,
  trader addresses, tx hashes, log indices, intent IDs, and option
  series IDs are **not** promoted to labels. Tests
  `perp_fee_charged_v2_metric_classifies_*` and
  `classifier_never_emits_raw_address` pin this.
- Status: **instrumented** (V2F-P). The metric is always emitted (all
  three labels surface at zero by default so the
  `increase(...)[5m]` query has a stable time series from the first
  scrape). Alert delivery wiring (Alertmanager / PagerDuty / Slack)
  is still environment-specific and out of scope of this repo.

### PERP FeeRebatedV2 From OLD PerpEngine (V2F-Q)

- Name: `perp_fee_rebated_from_old_engine`
- Metric (V2F-Q): `deopt_perp_fee_rebated_v2_total{consumer="new"|"old"|"unknown"}`
- Prometheus rule (deployable; also in
  `docs/alertmanager/perp_v2_fee_alerts.yml`):

  ```yaml
  - alert: PerpFeeRebatedFromOldEngine
    expr: increase(deopt_perp_fee_rebated_v2_total{consumer="old"}[5m]) > 0
    for: 0m
    labels:
      severity: high
    annotations:
      summary: "PERP FeeRebatedV2 emitted from OLD stranded PerpEngine"
      description: "OLD PerpEngine should not emit new PERP V2 rebate events after A3 fallback. Snapshot FeesManagerV2 wiring and compare to V2F-LM acceptance state."
  ```

- Severity: high on Base Sepolia, critical on mainnet.
- Signal: any indexed `FeeRebatedV2` event where
  `decoded.productKind == "perp"` and `decoded.consumer ==
  OLD_PERP_ENGINE_ADDRESS`. Backend filters at SQL/store layer; the
  same `classify_perp_fee_consumer` classifier as the charged alert
  promotes the result to a low-cardinality `consumer` bucket.
- Intent: same A3-stranded reasoning as the charged-event alert —
  OLD should not emit new PERP V2 rebate logs. A nonzero result
  indicates a contract-level regression (consumer re-allowlisting,
  FeesManagerV2 pointed back at OLD) or a configuration error.
- Expected current value: zero (no PERP rebate event has been live on
  Base Sepolia yet; V2F-LM PERP smoke emitted 2× FeeChargedV2 and 0×
  FeeRebatedV2).
- Cardinality guarantees: exactly three series per scrape
  (`consumer="new"|"old"|"unknown"`). Same `classifier_never_emits_raw_address`
  invariant as the V2F-P metric.
- Tests: `perp_fee_rebated_v2_metric_*` in `src/api/routes.rs::tests`.
- Runbook: `docs/RUNBOOK_PERP_V2_FEE_ALERTS.md#perpfeerebatedfromoldengine`.
- Status: **instrumented** (V2F-Q).

### PERP V2 Fee Event From Unknown Consumer (V2F-Q)

- Name: `perp_fee_consumer_unknown`
- Metric (V2F-Q): combined increase across the V2F-P + V2F-Q
  `unknown` buckets:
  `deopt_perp_fee_charged_v2_total{consumer="unknown"}` and
  `deopt_perp_fee_rebated_v2_total{consumer="unknown"}`.
- Prometheus rule (deployable; also in
  `docs/alertmanager/perp_v2_fee_alerts.yml`):

  ```yaml
  - alert: PerpFeeConsumerUnknown
    expr: |
      increase(deopt_perp_fee_charged_v2_total{consumer="unknown"}[5m])
      + increase(deopt_perp_fee_rebated_v2_total{consumer="unknown"}[5m])
      > 0
    for: 0m
    labels:
      severity: medium
    annotations:
      summary: "PERP V2 fee event emitted from unknown consumer"
      description: "FeesManagerV2 fired a PERP fee or rebate whose consumer matches neither the configured NEW (PERP_ENGINE_ADDRESS) nor OLD (OLD_PERP_ENGINE_ADDRESS) engine. Reconcile env vars with the contract state before escalating."
  ```

- Severity: medium (anomaly, not a hard regression).
- Intent: the metric pipeline classifies any PERP `FeeChargedV2` /
  `FeeRebatedV2` whose `decoded.consumer` does not match the
  configured NEW or OLD as `consumer = "unknown"`. Possible causes:
  - env-var drift (`PERP_ENGINE_ADDRESS` / `OLD_PERP_ENGINE_ADDRESS`
    have not been updated after a redeploy);
  - a third engine was registered as a FeesManagerV2 fee consumer;
  - a stray contract address ended up in the log topic (decoder
    regression).
- Suggested first action: diff the env vars against
  `FeesManagerV2.feeConsumers()` on-chain. If the env is stale, roll
  the config; if a new consumer is legitimate, file a milestone to
  add it to the classifier vocabulary (e.g.
  `consumer="next_gen"`) — never widen the existing buckets to
  swallow it silently.
- Runbook: `docs/RUNBOOK_PERP_V2_FEE_ALERTS.md#perpfeeconsumerunknown`.
- Status: **instrumented** (V2F-Q).

### OPTION V2 Fee + Rebate From OLD MarginEngine (V2G-F)

- Names: `option_fee_charged_from_old_margin_engine`,
  `option_fee_rebated_from_old_margin_engine`.
- Metrics (V2G-F):
  `deopt_option_fee_charged_v2_total{consumer="new"|"old"|"unknown"}`
  and
  `deopt_option_fee_rebated_v2_total{consumer="new"|"old"|"unknown"}`.
- Prometheus rules: see
  `docs/alertmanager/option_v2_fee_alerts.yml`.
- Severity: high on Base Sepolia, critical on mainnet (mirrors the
  PERP severity matrix).
- Signal: any indexed `FeeChargedV2` / `FeeRebatedV2` event where
  `decoded.productKind == "option"` and `decoded.consumer ==
  OLD_MARGIN_ENGINE_ADDRESS`. Classification is shared with the PERP
  path via the thin
  `src/fees/option_consumer.rs::classify_option_fee_consumer`
  wrapper around the PERP classifier — same case-insensitive match,
  same three-bucket vocabulary, same raw-address suppression.
- Intent: the NEW MarginEngine
  (`0x287Cef479be5889eEfCa847F9e73C860898f48Cc`, V2-fees enabled
  since V2E-E) is the only OPTION fee consumer registered with
  FeesManagerV2 today. The legacy MarginEngine
  (`0x6C5665De05e7314cB63cD77F82DFa86508A5b5F8`, V2D-R) is
  superseded and is expected to emit zero new OPTION V2 fees. Any
  nonzero occurrence indicates the same class of regression as the
  PERP-side alert.
- Expected current value: zero. V2G-E is the only OPTION rebate
  event on chain so far (tx
  `0x9a85cbced2216bf3c18049111cce68883cb0b035e194b3dcbaaf4fe7d5293149`,
  block `42206003`), emitted from NEW.
- Cardinality guarantees: exactly three series per metric per scrape
  (`consumer="new"|"old"|"unknown"`). Tests
  `option_fee_charged_v2_metric_*` /
  `option_fee_rebated_v2_metric_*` in `src/api/routes.rs::tests`
  enforce both the bucket count and the no-raw-address invariant.
- Runbook entries:
  `docs/RUNBOOK_PERP_V2_FEE_ALERTS.md#optionfeechargedfromoldmarginengine`,
  `docs/RUNBOOK_PERP_V2_FEE_ALERTS.md#optionfeerebatedfromoldmarginengine`.
- Status: **instrumented** (V2G-F).

### OPTION V2 Fee Event From Unknown Consumer (V2G-F)

- Name: `option_fee_consumer_unknown`.
- Metric (V2G-F): combined increase across the OPTION `unknown`
  buckets — same shape as `perp_fee_consumer_unknown`.
- Prometheus rule: see
  `docs/alertmanager/option_v2_fee_alerts.yml`.
- Severity: medium (anomaly, not a hard regression).
- Cause taxonomy: env-var drift on `MARGIN_ENGINE` /
  `OPTION_EVENT_INDEXER_MARGIN_ENGINE_ADDRESS`,
  `OLD_MARGIN_ENGINE_ADDRESS` unset, or a third MarginEngine
  registered with FeesManagerV2.
- Runbook:
  `docs/RUNBOOK_PERP_V2_FEE_ALERTS.md#optionfeeconsumerunknown`.
- Status: **instrumented** (V2G-F).

### FeesManagerV2 Rebate Budget Low (V2G-F)

- Name: `fees_manager_v2_rebate_budget_low`.
- Metric (V2G-F):
  `deopt_fees_manager_v2_rebate_budget_native{asset="<lowercased address>"}`.
  Derived from indexed events as `SUM(RebateBudgetFunded.amount) −
  SUM(RebateBudgetSpent.amount) − SUM(RebateBudgetWithdrawn.amount)`,
  per `decoded.settlementAsset`, clamped at zero (floors at 0 if
  the indexer is briefly behind a withdraw rather than
  wrap-underflowing). Backed by
  `OptionSeriesStore::fees_manager_v2_rebate_budget_by_asset` and
  `PgRepository::admin_fees_manager_v2_rebate_budget_by_asset`.
- Prometheus rule (deployable; also in
  `docs/alertmanager/option_v2_fee_alerts.yml`):

  ```yaml
  - alert: FeesManagerV2RebateBudgetLow
    expr: |
      deopt_fees_manager_v2_rebate_budget_native{
        asset="0x6eae407f5640b006fac9965182e238582a3b412e"
      } < 1000
    for: 0m
    labels:
      severity: medium
    annotations:
      summary: "FeesManagerV2 rebate budget is low"
      description: "Derived rebate budget for the canonical settlement asset (mUSDC on Base Sepolia) has fallen below 1 000 native units. Top up via FeesManagerV2.fundRebateBudget. Also read the on-chain rebateBudget(token) as ground truth."
  ```

- Severity: medium on Base Sepolia, high on mainnet.
- Cardinality contract: the `asset` label is the lowercased
  settlement-asset address (never a symbol). On Base Sepolia we use
  only mUSDC, so the alert is keyed on a single canonical address;
  multi-asset environments need one rule per supported asset.
- Ground truth: the on-chain
  `FeesManagerV2.rebateBudget(token)(uint256)` view; the derived
  metric will match it once the indexer has caught up.
- Runbook:
  `docs/RUNBOOK_PERP_V2_FEE_ALERTS.md#feesmanagerv2rebatebudgetlow`.
- Status: **instrumented** (V2G-F).

### V2G-G — production observability bundle

V2G-G consolidates the V2F-Q + V2G-F alert rules into a single
deployable Prometheus bundle and adds two new alerts. Source-of-truth
file:

```
docs/monitoring/prometheus/v2_fee_alerts.bundle.yml
```

The bundle contains four rule groups:

1. `deopt_perp_v2_fee_alerts` — `PerpFeeChargedFromOldEngine`,
   `PerpFeeRebatedFromOldEngine`, `PerpFeeConsumerUnknown` (unchanged
   from V2F-Q; mirrored from `docs/alertmanager/perp_v2_fee_alerts.yml`).
2. `deopt_option_v2_fee_alerts` — `OptionFeeChargedFromOldMarginEngine`,
   `OptionFeeRebatedFromOldMarginEngine`, `OptionFeeConsumerUnknown`
   (unchanged from V2G-F; mirrored from
   `docs/alertmanager/option_v2_fee_alerts.yml`).
3. `deopt_fees_manager_v2_budget_alerts` —
   `FeesManagerV2RebateBudgetLow` (V2G-F, mUSDC threshold `< 1000`),
   plus **new in V2G-G** `FeesManagerV2RebateBudgetStale` which fires
   if the rebate-budget gauge is unchanged for 30 m while
   `FeeRebatedV2{consumer="new"}` is still incrementing (indexer-lag /
   stalled-event-pipeline detector). Severity `medium` on Base
   Sepolia, `high` on mainnet.
4. `deopt_v2_fee_metrics_liveness` — **new in V2G-G**
   `DeoptV2FeeMetricsAbsent` fires if any of the four V2 fee
   consumer-bucket gauges is absent from `/metrics` for 5 m. The
   metric pipeline pre-seeds every bucket at zero on boot, so absence
   means either the scrape target is down or `METRICS_ENABLED=false`.
   Severity `high` on Base Sepolia, escalates per the routing table.

The bundle is meant to be the single file the operator points
Prometheus at; the per-product `docs/alertmanager/*.yml` files are
retained for backwards-link stability.

Companion artefacts:

- `docs/monitoring/alertmanager/v2_fee_routing.example.yml` — example
  routing tree (PERP/OPTION OLD → contract on-call; unknown → ticket
  queue; budget → ops chat; metrics absent → backend on-call). Ships
  an inhibit rule so a dead metric pipeline does not cascade into
  noise.
- `docs/monitoring/grafana/v2_fee_observability_dashboard.json` +
  `…spec.md` — Grafana dashboard rendering every V2 fee gauge plus
  anomaly stat cards.

#### Multi-asset extension

The `FeesManagerV2RebateBudgetLow` rule is keyed on a specific
lowercased settlement-asset address (Base Sepolia mUSDC today). On a
multi-asset deployment, add one rule per asset (template at the
bottom of the bundle file). The metric pipeline already supports
multiple `asset=...` series — see
`fees_manager_v2_rebate_budget_metric_reflects_funded_minus_spent_and_withdrawn`
in `src/api/routes.rs::tests` and the readiness matrix in
`docs/V2_FEE_PRODUCTION_OBSERVABILITY_V2G_G.md`.

#### Backend admin probe (V2G-G)

For a one-shot operator check without leaving the admin token's blast
radius, hit:

```
GET /admin/fees/v2/observability
```

Returns the same data the four V2 fee gauges + budget gauge surface,
plus the configured NEW / OLD engine addresses the classifier is
using. Implementation:
`src/fees/v2_observability.rs::admin_v2_observability`.

The frontend admin page also renders this as a dedicated "V2 Fee
Observability (V2G-G)" section.

#### V2G-H — toolchain validation + provisioning

V2G-H validates the V2G-G rule bundle + routing example with the real
Prometheus / Alertmanager toolchains, and adds Grafana provisioning
plumbing:

- `docs/monitoring/prometheus/v2_fee_alerts.test.yml` — `promtool test
  rules` spec covering green baseline, PERP OLD-engine firing, OPTION
  unknown-consumer firing, rebate budget low, and metric pipeline
  absent. Runs clean against the V3.12.0 promtool. Use this as
  CI-time regression gate for any future rule changes.
- `docs/monitoring/grafana/provisioning/dashboards/deopt_v2_fees.yaml`
  — Grafana dashboard provisioning provider entry (`apiVersion: 1`,
  `folder: DeOpt`, `allowUiUpdates: true`).
- `docs/monitoring/grafana/provisioning/datasources/prometheus_example.yaml`
  — example Prometheus datasource entry (only needed if no DS exists
  yet).
- `docs/monitoring/grafana/provisioning/render_dashboard.sh` —
  substitutes `${DS_PROMETHEUS}` with the datasource name for
  provisioned dashboards (Grafana's provisioning loader does NOT
  resolve template inputs).

See `docs/V2_FEE_OBSERVABILITY_LIVE_STACK_WIRING_V2G_H.md` for the
exact integration commands per stack shape (standalone, containerised,
Kubernetes Operator).

#### V2G-I — drill + stale rule bugfix + opt-in stalled rule

V2G-I activated the V2G-H artefacts against a real local Alertmanager
process and discovered a substantive bug in the V2G-G stale-budget
rule:

- **`FeesManagerV2RebateBudgetStale` PromQL bugfix.** The original
  expression used `vector and vector` with **disjoint** label sets
  (`{asset=...}` on the budget gauge vs `{consumer="new"}` on the
  rebate counters). PromQL default vector matching returns the empty
  set under those conditions, so the alert silently never fired. The
  fixed expression uses `delta(budget[30m]) == 0 and on() (rebate
  counters)` and keeps the budget gauge's `asset` label on the output
  for a natural per-asset alert payload. The fix is recorded inline
  in `docs/monitoring/prometheus/v2_fee_alerts.bundle.yml` with a
  comment block pointing at the V2G-I test.
- **`DeoptV2PerpRebateStalled` shipped as a separate opt-in bundle.**
  `docs/monitoring/prometheus/v2_fee_alerts.stalled.yml` contains
  just this one rule. Activation is operator-controlled because the
  rule's 24h `for` window only makes sense once the network has
  steady rebate cadence — Base Sepolia today has too few rebate
  events (V2G-E only).
- **Synthetic drill artefacts.** V2G-I documents how to spin up a
  scratch Alertmanager + webhook sink on localhost to replay the
  drill on any host. See `docs/V2_FEE_OBSERVABILITY_LIVE_ACTIVATION_V2G_I.md`
  §Phase 6.

#### V2G-J — operator cutover packets

V2G-J turns the V2G-G artefacts into operator-runnable cutover
packets without performing any remote mutation. Five sub-packets
under `docs/operator/`:

- `V2G_J_TARGET_STACK_QUESTIONNAIRE.md` — pre-cutover intake.
- `V2G_J_TARGET_STACK_DEPLOYMENT_PACKET.md` — exact commands for
  standalone Linux/systemd, docker/podman compose, and Kubernetes
  (vanilla + Prometheus Operator).
- `V2G_J_BACKEND_ENV_PATCH_PACKET.md` — idempotent `.env` apply +
  verify + rollback for the five-var observability classifier patch.
- `V2G_J_SYNTHETIC_ALERT_DRILL_PACKET.md` — six drills covering all
  five receivers, including the mainnet escalation route
  (`severity=medium, chain=base-mainnet`) that V2G-I left unexercised.
- `V2G_J_STALLED_RULE_ACTIVATION_POLICY.md` — explicit
  five-condition gate before `v2_fee_alerts.stalled.yml` lands on
  the target stack.

See `docs/V2_FEE_OBSERVABILITY_TARGET_CUTOVER_V2G_J.md` for the
top-level cutover record.

#### V2G-L0 — local non-production stack

V2G-L0 ships a complete localhost docker-compose stack at
`docs/monitoring/local-stack/` (Prometheus 3.12.0 + Alertmanager
0.32.1 + Grafana 11.4.0 + webhook sink). Rule files are symlinked
from the canonical `docs/monitoring/prometheus/` so the local stack
always loads the same V2G-G bundle the production cutover ships. The
Alertmanager routing example is wired to a localhost webhook sink so
the V2G-J synthetic drill can be replayed locally end-to-end.

The agent ran a bare-binary fallback of the same stack
(Prometheus + Alertmanager binaries from `/tmp`) when the docker
daemon was inaccessible; 5/5 synthetic drills landed at the expected
receivers under the bare-binary run. Full record at
`docs/V2_FEE_OBSERVABILITY_LOCAL_STACK_BOOTSTRAP_V2G_L0.md`.

V2G-L1 retried the compose start, hit the same docker socket gate,
stopped at sudo per hard rule, and re-ran the bare-binary stand-in to
keep the V2G-K soak warm (5/5 firing + 5/5 resolved through the sink
this time). Full record at
`docs/V2_FEE_OBSERVABILITY_LOCAL_COMPOSE_SOAK_V2G_L1.md`. Operator
unblock command in the same file's Phase 1.

#### V2G-L2 — compose stack live

Operator ran `sudo usermod -aG docker $USER && newgrp docker` and
the agent brought the V2G-L0 compose stack up after two small fixes:
the V2G-L0 Prometheus rule symlinks pointed outside the bind-mounted
tree (replaced with literal copies + ship
`prometheus/rules/sync_from_canonical.sh`), and the webhook-sink
container's `user: "65534:65534"` override prevented writes to the
named-volume log path (dropped). All 4 containers come up healthy;
Prometheus scrapes the dev backend through `host.docker.internal`,
9 alerts load `inactive`, 5/5 synthetic drills route to the expected
receivers, sink volume captures every dispatch, Grafana exposes the
"DeOpt — V2 fee observability (V2G-G)" dashboard with the
`Prometheus` datasource resolving against the same backend.

Full record at
`docs/V2_FEE_OBSERVABILITY_LOCAL_COMPOSE_LIVE_V2G_L2.md`.

#### V2G-L3 — day-1 early progress probe

V2G-L3 ran the V2G-K daily checklist at T+25min after the V2G-L2
stack came up. All 9 daily-check steps green; container resource
use < 0.25% CPU / < 60MB mem each; no synthetic drill replayed; no
log regressions since the V2G-L2 fix. The canonical day-1 24h
checkpoint is reserved for T+24h. Full record at
`docs/V2_FEE_OBSERVABILITY_LOCAL_COMPOSE_DAY1_V2G_L3.md`.

#### V2G-K — 7-day soak record

V2G-K opens the 7-day soak record at
`docs/V2_FEE_OBSERVABILITY_7_DAY_SOAK_V2G_K.md`. The soak record
ships a starting metric snapshot (the baseline the target host's
first post-cutover scrape is compared against), a 7-day daily
checklist, the day-5 stalled-rule re-evaluation marker, and a
per-layer rollback table. The opt-in `v2_fee_alerts.stalled.yml`
stays disabled until the activation policy gate is satisfied.

## Retired / Downgraded Operational Notices

### Merkle Root Unset (retired 2026-05-31, V2G-F)

The "FeesManagerV2 merkle root unset" operational notice that the
V2G-A plan flagged is now **retired**. The root has been live since
V2G-C and was rotated under V2G-D2 to
`0xd8a627d7a9b600370e6f490fdd789150d7f9c4ea2f09752c88121d1f758fc2df`
with window `1780099200 → 1781913600` (2026-05-30 → 2026-06-20 UTC).
V2G-D3 claims for the Tier 4 maker / Tier 2 taker, and V2G-E's
first live `FeeRebatedV2` events, both depend on the root being
non-zero; if a future incident sees `merkleRoot() == bytes32(0)`
it is a fresh contract-level regression, not an unfinished
deployment.

Operators who want a continuous safety check can keep a
read-only `cast call $FEES_MANAGER_V2 'merkleRoot()(bytes32)'`
probe in a Grafana dashboard; no Prometheus alert is shipped for it
because the on-chain state already prevents `claimTier` from
succeeding against `bytes32(0)`.

## Deferred Alert Families

- Oracle stale alerts are deferred until oracle metrics exist.
- Liquidation and bad-debt alerts are deferred until risk/liquidation subsystems exist.
- Latency histograms, request error-rate metrics, and OpenTelemetry traces are deferred beyond V1B.
- WebTransport CI metrics and frontend monitoring UI are deferred.
