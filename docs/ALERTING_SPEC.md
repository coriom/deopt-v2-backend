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

## Deferred Alert Families

- Oracle stale alerts are deferred until oracle metrics exist.
- Liquidation and bad-debt alerts are deferred until risk/liquidation subsystems exist.
- Latency histograms, request error-rate metrics, and OpenTelemetry traces are deferred beyond V1B.
- WebTransport CI metrics and frontend monitoring UI are deferred.
