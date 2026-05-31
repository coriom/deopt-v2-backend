# V2G-J — `v2_fee_alerts.stalled.yml` activation policy

`docs/monitoring/prometheus/v2_fee_alerts.stalled.yml` ships the
`DeoptV2PerpRebateStalled` rule. The rule fires when
`increase(deopt_perp_fee_rebated_v2_total{consumer="new"}[24h])` is
zero for 24h. That predicate is meaningful **only when the network
has stable PERP rebate cadence** — otherwise it fires continuously
during quiet periods and trains operators to ignore it.

V2G-J holds this rule **disabled by default** and codifies the gate
that must be met before turning it on.

## Why the rule is shipped separately

Base Sepolia today has exactly one live PERP rebate event (V2G-E
`0x5c15e923…`). Activating the rule there would put the alert into
firing state continuously until the next live PERP rebate trade
lands, which trains operators to silence it and hides the real
signal it would carry on a busier network.

The rule was kept commented out in `v2_fee_alerts.bundle.yml` for
exactly this reason. V2G-I moved it to a separate opt-in file so
that:

- the prod bundle stays unambiguously deployable as a fire-and-forget
  artefact;
- enabling the cadence rule is an explicit operator decision tied to
  observed flow, not a side effect of pulling the V2G-G package.

## Activation gate (do not enable before all five conditions hold)

| # | Condition | How to verify |
|---|-----------|---------------|
| 1 | `deopt_perp_fee_rebated_v2_total{consumer="new"}` has increased at least **N times** within the last 24h on the target network. Recommended `N = 3` to start. | Prometheus query: `increase(deopt_perp_fee_rebated_v2_total{consumer="new"}[24h])` ≥ 3. |
| 2 | The same condition held for at least **48 consecutive hours** before the activation window. | Run the query above at `t=0` and `t=-24h`; both must return ≥ N. |
| 3 | The Grafana dashboard "PERP FeeRebatedV2 by consumer" panel shows a continuous, monotonically-rising `consumer="new"` line for the period. (Visual sanity check; catches sparse-but-bursty patterns that pass the count gate but would still cause spurious fires.) | Visual inspection. |
| 4 | `/admin/fees/v2/observability` returns `anomaly_totals.old_consumer_events == 0` and `anomaly_totals.unknown_consumer_events == 0` during the 48h window. (If OLD or unknown counts are non-zero, fix that first — a stalled-cadence rule on top of misclassification compounds confusion.) | Backend admin probe + Prometheus history. |
| 5 | Operator on-call is aware and ACKs the activation. | Out-of-band confirmation (Slack, ticket, etc.). |

If condition 1 cannot be met because the network's natural rebate
cadence is sparser (e.g. once per week), widen the rule's window
**before enabling** instead of disabling: change `[24h]` to `[7d]`
and `for: 24h` to `for: 7d`. Document the change in
`docs/V2_FEE_OBSERVABILITY_TARGET_CUTOVER_V2G_J.md`.

## Enable command (after gate is met)

```sh
# 1. Validate the rule one more time on the operator workstation.
"${PROMTOOL:-/tmp/prometheus-3.12.0.linux-amd64/promtool}" check rules \
  ~/DEOPT/deopt-v2-backend/docs/monitoring/prometheus/v2_fee_alerts.stalled.yml

# 2. Copy to the target rules directory.
sudo install -m 0644 \
  ~/DEOPT/deopt-v2-backend/docs/monitoring/prometheus/v2_fee_alerts.stalled.yml \
  /etc/prometheus/rules.d/v2_fee_alerts.stalled.yml

# 3. Reference it from prometheus.yml — add to the rule_files list
# alongside the bundle:
#
#   rule_files:
#     - /etc/prometheus/rules.d/v2_fee_alerts.bundle.yml
#     - /etc/prometheus/rules.d/v2_fee_alerts.stalled.yml

# 4. Validate the merged config + graceful reload.
sudo promtool check config /etc/prometheus/prometheus.yml
curl -sf -X POST http://127.0.0.1:9090/-/reload

# 5. Confirm the new rule group shows up.
curl -s http://127.0.0.1:9090/api/v1/rules \
  | jq -r '.data.groups[].name' | grep '^deopt_v2_perp_rebate_cadence$'
# Expected: deopt_v2_perp_rebate_cadence
```

## Rollback (if the rule starts firing spuriously)

```sh
# 1. Remove the rule file.
sudo rm /etc/prometheus/rules.d/v2_fee_alerts.stalled.yml

# 2. Remove the corresponding `rule_files` entry from
# prometheus.yml.

# 3. Reload.
sudo promtool check config /etc/prometheus/prometheus.yml
curl -sf -X POST http://127.0.0.1:9090/-/reload
```

The metric pipeline is unaffected — no backend change, no Grafana
change, no Alertmanager change. The cadence alert is purely a
Prometheus-rule concern.

## Re-evaluation cadence

Re-evaluate the activation gate **after every change to the network's
typical PERP rebate flow**, in particular:

- A new tier rolls out (changes maker incentives, changes rebate
  frequency).
- A maker stops participating (drops the daily count).
- A new market launches (introduces new rebate-eligible flow).

If any of those events would push the 24h count below the activation
threshold, **proactively disable** the rule before it starts firing.
Re-enable only after the 5-condition gate is re-met.
