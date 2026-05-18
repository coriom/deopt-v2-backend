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

## Deferred Alert Families

- Oracle stale alerts are deferred until oracle metrics exist.
- Liquidation and bad-debt alerts are deferred until risk/liquidation subsystems exist.
- Latency histograms, request error-rate metrics, and OpenTelemetry traces are deferred beyond V1B.
- WebTransport CI metrics and frontend monitoring UI are deferred.
