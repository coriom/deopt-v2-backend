# V2G-J — Synthetic Alertmanager drill packet

Run this drill against the **target Alertmanager** (questionnaire §C4)
**after** the Prometheus rule bundle is live and the routing fragment is
merged. Each step injects a synthetic alert via `amtool alert add` and
expects it to reach a specific receiver. The drill mutates only the
Alertmanager view of the world — it does **not** touch Prometheus
metrics, the backend, or chain state.

## Safety disclaimer

- These are **synthetic** alerts. They show up in Alertmanager and
  fire the configured webhooks/PagerDuty/Slack receivers exactly as a
  real alert would. Coordinate with on-call BEFORE running.
- Every alert sent here has `start` set to `now` and is **immediately
  resolved** at the end of the drill (drill 5 + 6) so the on-call gets
  a paired firing+resolved pair.
- No Prometheus metric is mutated. The drill targets the
  Alertmanager API directly.
- Cardinality contract: every synthetic alert carries the same label
  schema the V2G-G rules emit
  (`alertname` + `severity` + `subsystem` + `milestone` + optional
  `consumer` / `asset_symbol` / `asset` / `chain`). No raw trader
  addresses, tx hashes, or PII.

## Prereqs

```sh
# Required env vars (questionnaire §C):
export AM_URL="${AM_URL:-http://127.0.0.1:9093}"   # §C4
export AMTOOL="${AMTOOL:-/tmp/alertmanager-0.32.1.linux-amd64/amtool}"

# Sanity check.
"${AMTOOL}" --alertmanager.url="${AM_URL}" alert query | head -3
# Should print column headers, no error.
```

## Drill 1 — `PerpFeeChargedFromOldEngine` (expected receiver: $C5)

```sh
NOW="$(date -u +%FT%TZ)"
"${AMTOOL}" --alertmanager.url="${AM_URL}" alert add \
  --start="${NOW}" \
  alertname=PerpFeeChargedFromOldEngine \
  subsystem=perp_v2_fees severity=high consumer=old \
  milestone=V2F-O \
  --annotation=summary="V2G-J drill — PERP OLD engine fee" \
  --annotation=runbook_url="docs/RUNBOOK_PERP_V2_FEE_ALERTS.md#perpfeechargedfromoldengine"

# Expected route resolution:
"${AMTOOL}" --alertmanager.url="${AM_URL}" config routes test \
  alertname=PerpFeeChargedFromOldEngine subsystem=perp_v2_fees severity=high consumer=old
# Expected output: deopt_oncall_critical
```

Operator confirmation: the on-call rotation associated with `$C5`
receives the synthetic page within the route's `group_wait` window.

## Drill 2 — `OptionFeeConsumerUnknown` (expected receiver: $C7)

```sh
NOW="$(date -u +%FT%TZ)"
"${AMTOOL}" --alertmanager.url="${AM_URL}" alert add \
  --start="${NOW}" \
  alertname=OptionFeeConsumerUnknown \
  subsystem=option_v2_fees severity=medium consumer=unknown \
  milestone=V2G-F \
  --annotation=summary="V2G-J drill — OPTION unknown consumer" \
  --annotation=runbook_url="docs/RUNBOOK_PERP_V2_FEE_ALERTS.md#optionfeeconsumerunknown"

"${AMTOOL}" --alertmanager.url="${AM_URL}" config routes test \
  alertname=OptionFeeConsumerUnknown subsystem=option_v2_fees severity=medium
# Expected output: deopt_anomaly_tickets
```

## Drill 3 — `FeesManagerV2RebateBudgetLow` (expected receiver: $C8)

```sh
NOW="$(date -u +%FT%TZ)"
"${AMTOOL}" --alertmanager.url="${AM_URL}" alert add \
  --start="${NOW}" \
  alertname=FeesManagerV2RebateBudgetLow \
  subsystem=fees_manager_v2 severity=medium asset_symbol=musdc \
  asset=0x6eae407f5640b006fac9965182e238582a3b412e \
  milestone=V2G-F \
  --annotation=summary="V2G-J drill — rebate budget low" \
  --annotation=runbook_url="docs/RUNBOOK_PERP_V2_FEE_ALERTS.md#feesmanagerv2rebatebudgetlow"

"${AMTOOL}" --alertmanager.url="${AM_URL}" config routes test \
  alertname=FeesManagerV2RebateBudgetLow subsystem=fees_manager_v2 severity=medium asset_symbol=musdc
# Expected output: deopt_ops_chat
```

## Drill 4 — `DeoptV2FeeMetricsAbsent` (expected receiver: $C9)

```sh
NOW="$(date -u +%FT%TZ)"
"${AMTOOL}" --alertmanager.url="${AM_URL}" alert add \
  --start="${NOW}" \
  alertname=DeoptV2FeeMetricsAbsent \
  subsystem=v2_fee_observability severity=high consumer=new \
  milestone=V2G-G \
  --annotation=summary="V2G-J drill — metric pipeline absent" \
  --annotation=runbook_url="docs/RUNBOOK_PERP_V2_FEE_ALERTS.md"

"${AMTOOL}" --alertmanager.url="${AM_URL}" config routes test \
  alertname=DeoptV2FeeMetricsAbsent subsystem=v2_fee_observability severity=high
# Expected output: deopt_backend_oncall
```

## Drill 5 — `HighSeverityMainnetRouteDrill` (expected receiver: $C6)

V2G-I did **not** exercise the `deopt_oncall_high` receiver because the
local drill had no `chain=base-mainnet` alert. V2G-J fills that gap.

```sh
NOW="$(date -u +%FT%TZ)"
"${AMTOOL}" --alertmanager.url="${AM_URL}" alert add \
  --start="${NOW}" \
  alertname=HighSeverityMainnetRouteDrill \
  subsystem=v2_fee_observability severity=medium chain=base-mainnet \
  milestone=V2G-J \
  --annotation=summary="V2G-J drill — mainnet severity=medium escalation" \
  --annotation=runbook_url="docs/RUNBOOK_PERP_V2_FEE_ALERTS.md"

"${AMTOOL}" --alertmanager.url="${AM_URL}" config routes test \
  alertname=HighSeverityMainnetRouteDrill severity=medium chain=base-mainnet
# Expected output includes: deopt_oncall_high
#
# Reminder: the routing example uses `continue: true` on the
# severity-escalation route, so this alert may also resolve to one of
# the subsystem routes. The V2G-I drill documented the same
# `continue: true` behaviour as expected.
```

## Drill 6 — Inhibit semantics

`DeoptV2FeeMetricsAbsent{subsystem=perp_v2_fees}` should suppress
`PerpFeeRebatedFromOldEngine{subsystem=perp_v2_fees}` via the
`equal: [subsystem]` inhibit rule in the routing fragment.

```sh
NOW="$(date -u +%FT%TZ)"
# 1) Send the inhibit source (must carry subsystem=perp_v2_fees so the
# equal: [subsystem] matcher fires).
"${AMTOOL}" --alertmanager.url="${AM_URL}" alert add \
  --start="${NOW}" \
  alertname=DeoptV2FeeMetricsAbsent subsystem=perp_v2_fees \
  severity=high consumer=new milestone=V2G-G

# 2) Send the inhibit target — should be suppressed.
"${AMTOOL}" --alertmanager.url="${AM_URL}" alert add \
  --start="${NOW}" \
  alertname=PerpFeeRebatedFromOldEngine subsystem=perp_v2_fees \
  severity=high consumer=old milestone=V2F-Q

# 3) Confirm the suppression in the live alerts API.
curl -s "${AM_URL}/api/v2/alerts" | jq '
  .[] | select(.labels.alertname == "PerpFeeRebatedFromOldEngine") | {
    alert: .labels.alertname,
    state: .status.state,
    inhibitedBy: .status.inhibitedBy
  }'
# Expected: state="suppressed" and inhibitedBy is non-empty.
```

## Cleanup — resolve every synthetic alert

```sh
NOW="$(date -u +%FT%TZ)"
END="$(date -u -d '+5 seconds' +%FT%TZ 2>/dev/null || date -u -v +5S +%FT%TZ)"

for ARGS in \
  'alertname=PerpFeeChargedFromOldEngine subsystem=perp_v2_fees severity=high consumer=old milestone=V2F-O' \
  'alertname=OptionFeeConsumerUnknown subsystem=option_v2_fees severity=medium consumer=unknown milestone=V2G-F' \
  'alertname=FeesManagerV2RebateBudgetLow subsystem=fees_manager_v2 severity=medium asset_symbol=musdc asset=0x6eae407f5640b006fac9965182e238582a3b412e milestone=V2G-F' \
  'alertname=DeoptV2FeeMetricsAbsent subsystem=v2_fee_observability severity=high consumer=new milestone=V2G-G' \
  'alertname=HighSeverityMainnetRouteDrill subsystem=v2_fee_observability severity=medium chain=base-mainnet milestone=V2G-J' \
  'alertname=DeoptV2FeeMetricsAbsent subsystem=perp_v2_fees severity=high consumer=new milestone=V2G-G' \
  'alertname=PerpFeeRebatedFromOldEngine subsystem=perp_v2_fees severity=high consumer=old milestone=V2F-Q' \
  ; do
  "${AMTOOL}" --alertmanager.url="${AM_URL}" alert add \
    --start="${NOW}" --end="${END}" $ARGS
done

# Verify cleanup — within ~30s every drill alert should be resolved.
sleep 30
curl -s "${AM_URL}/api/v2/alerts" \
  | jq '[ .[] | select(.labels.milestone | test("^V2(F|G)-")) ] | length'
# Expected: 0
```

## Receiver outcome matrix (for the post-drill report)

Fill this in after running the drill. The agent's V2G-I local stand-in
drill log produced exactly the same shape:

| Drill | Alert label payload                                                               | Expected receiver         | Got | Webhook delivered? |
|------:|-----------------------------------------------------------------------------------|---------------------------|-----|--------------------|
| 1     | `PerpFeeChargedFromOldEngine{subsystem=perp_v2_fees, severity=high, consumer=old}` | `deopt_oncall_critical`   |     |                    |
| 2     | `OptionFeeConsumerUnknown{subsystem=option_v2_fees, severity=medium, consumer=unknown}` | `deopt_anomaly_tickets` |     |                    |
| 3     | `FeesManagerV2RebateBudgetLow{subsystem=fees_manager_v2, severity=medium, asset_symbol=musdc, asset=0x6eae...}` | `deopt_ops_chat` |  |                    |
| 4     | `DeoptV2FeeMetricsAbsent{subsystem=v2_fee_observability, severity=high}`           | `deopt_backend_oncall`    |     |                    |
| 5     | `HighSeverityMainnetRouteDrill{severity=medium, chain=base-mainnet}`               | `deopt_oncall_high` (+ subsystem route may also dispatch) | |  |
| 6     | `PerpFeeRebatedFromOldEngine` should be suppressed by absent-source                | inhibited, not dispatched |     |                    |

## Failure / triage

If a drill alert lands at the wrong receiver:

1. Re-run `amtool config routes test` with the same label set against
   the deployed `alertmanager.yml`. If it resolves correctly there
   but not in the live Alertmanager, the deployed config is out of
   sync with the file — re-merge and reload.
2. If `amtool config routes test` also resolves wrong, the receiver
   names in the deployed config do not match the routing fragment.
   Cross-check Section C of the questionnaire.
3. Roll back the routing fragment merge per
   `docs/operator/V2G_J_TARGET_STACK_DEPLOYMENT_PACKET.md` §"Rollback"
   and re-investigate before re-running the drill.

## Out-of-scope (intentionally)

- The drill does NOT verify the **metric** path (`/metrics` → Prometheus
  → alert evaluation). The V2G-G `promtool test rules` spec covers
  that with synthetic time series; the V2G-J drill covers only the
  **routing + delivery** path.
- The drill does NOT inject a real OLD-engine fee or a real budget
  shortfall on chain. Those would require live trades / governance
  actions — out of scope for any V2G-band synthetic.
