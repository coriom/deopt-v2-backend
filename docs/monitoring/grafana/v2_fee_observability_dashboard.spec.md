# V2G-G — V2 fee observability dashboard spec

This is the human-readable spec for
`docs/monitoring/grafana/v2_fee_observability_dashboard.json`. Use it
when:

- importing the dashboard into Grafana and confirming every panel maps
  to a real metric;
- building an equivalent dashboard in a different tool (DataDog,
  CloudWatch, custom HTML);
- reviewing what V2G-G actually ships so the runbook + alerting docs
  stay in sync.

## Source-of-truth

| Surface | File |
| ------- | ---- |
| Prometheus rules (deployable bundle) | `docs/monitoring/prometheus/v2_fee_alerts.bundle.yml` |
| Alertmanager routing example         | `docs/monitoring/alertmanager/v2_fee_routing.example.yml` |
| Per-product alert files (legacy)     | `docs/alertmanager/perp_v2_fee_alerts.yml`, `docs/alertmanager/option_v2_fee_alerts.yml` |
| Runbook                              | `docs/RUNBOOK_PERP_V2_FEE_ALERTS.md` |
| Alerting spec                        | `docs/ALERTING_SPEC.md` |
| Backend metric emission              | `src/monitoring.rs::append_fee_metrics`, `append_perp_fee_v2_consumer_metric`, `append_option_fee_v2_consumer_metric` |

## Datasource

Single Prometheus datasource scraping the backend `/metrics` endpoint.
Templated as `${DS_PROMETHEUS}` in the JSON.

## Template variables

| Variable | Type   | Values                | Purpose |
| -------- | ------ | --------------------- | ------- |
| `DS_PROMETHEUS` | datasource | any Prometheus | which DS to query |
| `consumer`      | custom (multi+all) | `new`, `old`, `unknown` | per-bucket filter for PERP/OPTION panels |

## Panels

| # | Title                                        | Type       | Metric expression |
|---|----------------------------------------------|------------|-------------------|
| 1 | PERP FeeChargedV2 by consumer                | timeseries | `deopt_perp_fee_charged_v2_total{consumer=~"$consumer"}` |
| 2 | PERP FeeRebatedV2 by consumer                | timeseries | `deopt_perp_fee_rebated_v2_total{consumer=~"$consumer"}` |
| 3 | OPTION FeeChargedV2 by consumer              | timeseries | `deopt_option_fee_charged_v2_total{consumer=~"$consumer"}` |
| 4 | OPTION FeeRebatedV2 by consumer              | timeseries | `deopt_option_fee_rebated_v2_total{consumer=~"$consumer"}` |
| 5 | Rebate budget by settlement asset            | timeseries | `deopt_fees_manager_v2_rebate_budget_native` |
| 6 | Base Sepolia mUSDC rebate budget (latest)    | stat       | `deopt_fees_manager_v2_rebate_budget_native{asset="0x6eae407f5640b006fac9965182e238582a3b412e"}` |
| 7 | OLD consumer events (PERP)                   | stat       | `deopt_perp_fee_charged_v2_total{consumer="old"} + deopt_perp_fee_rebated_v2_total{consumer="old"}` |
| 8 | OLD consumer events (OPTION)                 | stat       | `deopt_option_fee_charged_v2_total{consumer="old"} + deopt_option_fee_rebated_v2_total{consumer="old"}` |
| 9 | Unknown consumer events (PERP + OPTION)      | stat       | sum of four `…{consumer="unknown"}` series |
| 10| Active engine addresses (reference)          | table      | reference card; not metric-fed (see notes) |

### Color thresholds

- Time-series PERP/OPTION panels: classic palette, with `consumer="old"`
  pinned to **red** and `consumer="unknown"` pinned to **orange**.
- Rebate-budget panels: red < 1 000, orange 1 000 ≤ x < 100 000, green
  ≥ 100 000 (native units; for mUSDC at 6 dp that is 0–0.001–0.1 USDC).
- Anomaly stats: green when zero, red (PERP/OPTION OLD) or orange
  (unknown) when ≥ 1.

### Active-engine reference card (panel #10)

There is no metric for the configured engine addresses today — they are
config, not observed state. The panel renders a static reference card.
For the live runtime values, hit either:

```
GET /admin/config
GET /admin/fees/v2/observability          # V2G-G (preferred)
```

The second endpoint returns exactly the addresses the metric pipeline
classifier is using, so an operator can confirm the dashboard is
labelling the right NEW / OLD engines.

## Not yet on the dashboard (intentional)

- **Merkle root status.** V2G-F retired the "merkle root unset"
  operational notice; on-chain state already prevents `claimTier` from
  succeeding against `bytes32(0)`. Operators who want a continuous
  read-only probe can `cast call $FEES_MANAGER_V2 'merkleRoot()(bytes32)'`
  on a cron and post it to Grafana via a `node_exporter` text-collector
  — out of scope for V2G-G because no metric exists yet.
- **PERP rebate cadence ("no rebates in 24h").** Commented out in the
  Prometheus bundle; uncomment + add a Grafana annotation once the
  daily-rebate cadence is established.
- **Per-tx event audit.** The backend's `GET /admin/fees/onchain?tx_hash=…`
  endpoint covers per-tx decode; Grafana is the wrong tool for it.

## Import checklist

1. Open Grafana → Dashboards → Import → upload
   `docs/monitoring/grafana/v2_fee_observability_dashboard.json`.
2. Pick your Prometheus datasource for `${DS_PROMETHEUS}`.
3. Confirm all metric queries resolve in the variable dropdown (Grafana
   will show "No data" for a metric the backend hasn't emitted yet —
   for V2G-G that should never happen because all four PERP/OPTION
   gauges are pre-seeded at zero).
4. Save (the import flow assigns a new `id`; the `uid` stays
   `deopt-v2g-g-v2-fees`).
5. Pin to the team folder.

## Multi-network notes

The dashboard does not assume a specific chain. To run it on a
non-Base-Sepolia network:

- The `asset` label values are network-specific (each chain has its
  own canonical USDC / mUSDC address). Panel #6 hard-codes the Base
  Sepolia mUSDC address; on mainnet, clone the panel and replace the
  asset address. Panel #5 already shows every asset that has emitted
  a rebate-budget event.
- The Prometheus rule that pins the rebate budget low threshold
  (`FeesManagerV2RebateBudgetLow`) is similarly per-asset; add a second
  rule per supported asset. See the multi-asset readiness matrix in
  `docs/V2_FEE_PRODUCTION_OBSERVABILITY_V2G_G.md`.
