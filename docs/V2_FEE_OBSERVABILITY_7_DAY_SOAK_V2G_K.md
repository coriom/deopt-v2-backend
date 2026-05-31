# V2G-K — V2 Fee Observability 7-Day Soak Record

## Status

- Milestone: **V2G-K** — live-fire cutover + 7-day soak start. V2G-J
  shipped the operator-runnable packets; V2G-K supervises the
  cutover, records the soak start, and ships the daily check
  checklist.
- Date: 2026-05-31.
- Outcome (as of file creation):
  - **Cutover status: pending operator authorisation.** Stack
    discovery on this host still shows no Prometheus / Alertmanager
    / Grafana installed (no PATH binaries, no `/etc/...` config
    directories, no listening ports on 9090/9093/3000, no docker
    daemon access, no SSH config referencing prod). The
    questionnaire reply
    (`docs/operator/V2G_K_OPERATOR_ANSWER_TEMPLATE.md`) is empty.
    All F1-F11 gates remain at default = NO.
  - **Local baseline snapshot captured.** Backend rebuilt + run
    read-only on Base Sepolia state; `/health`, `/admin/fees/v2/observability`,
    both V2G-E `/admin/fees/onchain` payloads, and `/metrics`
    archived under `/tmp/v2g_k_baseline/` with timestamp
    `2026-05-31T16:14:34Z`. All values reproduce V2G-G/H/I/J 1:1.
  - **Local synthetic drill stand-in repeated.** Spun up the V2G-I
    Alertmanager + webhook sink config one more time; fired all
    five V2G-J drill alerts (PERP OLD, OPTION unknown, budget low,
    metrics absent, mainnet escalation). Every alert routed to the
    expected receiver, every webhook landed. Receiver-outcome
    matrix at `/tmp/v2g_k_drill/receiver_outcome_matrix.md`.
  - **Stalled-rule decision: defer.** No 24h/48h cadence data on the
    target stack yet — `v2_fee_alerts.stalled.yml` stays out of the
    rules directory per V2G-J activation policy.
  - **Soak start: pending.** The day-0 row of the daily checklist is
    parked. Soak day-1 starts the calendar day after F1-F9 are
    operator-applied on the target host.
- Hard gates respected: no broadcast, no chain mutation, no DB
  writes, no private-key handling, no real-secret `.env` edit, no
  remote SSH, no production service reload, no governance/timelock
  action.

## Cutover timestamp

| Field                        | Value                                                                 |
|------------------------------|-----------------------------------------------------------------------|
| V2G-K record opened          | **2026-05-31T16:14Z**                                                 |
| Cutover applied on target    | _pending operator (F3-F8)_                                            |
| Backend env patch applied    | _pending operator (F1-F2)_                                            |
| Synthetic drill executed     | local stand-in 2026-05-31T16:16Z; target drill _pending operator (F9)_ |
| Soak day 1 begins            | _T+24h after F8 reload_                                               |

## Cutover artefact status

Each row references the V2G-G/H/J source file and the V2G-J operator
packet describing how to land it on the target host.

| # | Layer                                | Source artefact                                                                  | Operator packet                                                                              | Status                                                |
|---|--------------------------------------|----------------------------------------------------------------------------------|---------------------------------------------------------------------------------------------|-------------------------------------------------------|
| 1 | Backend `.env` classifier vars       | `docs/operator/v2g_g_env_patch.example.env`                                       | `docs/operator/V2G_J_BACKEND_ENV_PATCH_PACKET.md`                                            | **Pending operator** (F1, F2)                          |
| 2 | Prometheus rule bundle               | `docs/monitoring/prometheus/v2_fee_alerts.bundle.yml`                             | `docs/operator/V2G_J_TARGET_STACK_DEPLOYMENT_PACKET.md` §"Mode N → Prometheus rule rollout"   | **Pending operator** (F3, F4) — `promtool check rules` green locally |
| 3 | Alertmanager routing                 | `docs/monitoring/alertmanager/v2_fee_routing.example.yml`                         | `docs/operator/V2G_J_TARGET_STACK_DEPLOYMENT_PACKET.md` §"Mode N → Alertmanager routing rollout" | **Pending operator** (F5, F6) — `amtool check-config` green locally |
| 4 | Grafana dashboard + provisioning     | `docs/monitoring/grafana/v2_fee_observability_dashboard.json` + provisioning sidecars | `docs/operator/V2G_J_TARGET_STACK_DEPLOYMENT_PACKET.md` §"Mode N → Grafana dashboard rollout" | **Pending operator** (F7, F8) — JSON parses, render substitutes clean |
| 5 | Synthetic alert drill                | `docs/operator/V2G_J_SYNTHETIC_ALERT_DRILL_PACKET.md`                            | same packet                                                                                  | **Pending operator** (F9) — local stand-in passed all 5 routes |
| 6 | Opt-in `DeoptV2PerpRebateStalled`    | `docs/monitoring/prometheus/v2_fee_alerts.stalled.yml`                            | `docs/operator/V2G_J_STALLED_RULE_ACTIVATION_POLICY.md`                                      | **Deferred** — 24h/48h cadence gate not yet evaluable; default = OFF |

## Starting metric snapshot (local baseline, 2026-05-31T16:14:34Z)

Archived at `/tmp/v2g_k_baseline/`. Operator should compare the target
host's first scrape after F4 reload against these exact values.

### `/health`

```
{"ok":true,"service":"deopt-v2-backend"}
```

### `/admin/fees/v2/observability`

```jsonc
{
  "milestone": "V2G-G",
  "anomaly_totals": {
    "old_consumer_events": 0,
    "unknown_consumer_events": 0
  },
  "contracts": {
    "perp_engine_new":   "0xc6C592100723Fe0C66343A16e95eC34cC0c2141c",
    "perp_engine_old":   "0xB36395b67D0798ADA981731c9Fa5239F4362b53B",
    "margin_engine_new": "0x287Cef479be5889eEfCa847F9e73C860898f48Cc",
    "margin_engine_old": "0x6C5665De05e7314cB63cD77F82DFa86508A5b5F8",
    "fees_manager_v2":   "0x00dA0B9876bcBf0c79CB5BcAcfEBAFb8C7Ad774f"
  },
  "metrics": {
    "perp_fee_charged_v2_by_consumer":    {"new": 3, "old": 0, "unknown": 0},
    "perp_fee_rebated_v2_by_consumer":    {"new": 1, "old": 0, "unknown": 0},
    "option_fee_charged_v2_by_consumer":  {"new": 3, "old": 0, "unknown": 0},
    "option_fee_rebated_v2_by_consumer":  {"new": 1, "old": 0, "unknown": 0},
    "fees_manager_v2_rebate_budget_native": {
      "0x6eae407f5640b006fac9965182e238582a3b412e": 999987
    }
  }
}
```

### `/admin/fees/onchain` — PERP V2G-E `0x5c15e923…aa394`

```
event_model:            v2
fee_charged_v2_count:   1
fee_rebated_v2_count:   1
observed_total_charged: 6
observed_total_rebated: 3
net_protocol_fee:       3
source_of_truth:        onchain
reconciliation_status:  onchain_observed
```

### `/admin/fees/onchain` — OPTION V2G-E `0x9a85cbce…3149`

```
event_model:             mixed
fee_charged_v2_count:    1
fee_rebated_v2_count:    1
observed_total_charged:  25
observed_total_rebated:  10
net_protocol_fee:        15
trading_fee_event_count: 1
source_of_truth:         onchain
```

### `/metrics` — V2 fee block

```
deopt_perp_fee_charged_v2_total{consumer="new"} 3
deopt_perp_fee_charged_v2_total{consumer="old"} 0
deopt_perp_fee_charged_v2_total{consumer="unknown"} 0
deopt_perp_fee_rebated_v2_total{consumer="new"} 1
deopt_perp_fee_rebated_v2_total{consumer="old"} 0
deopt_perp_fee_rebated_v2_total{consumer="unknown"} 0
deopt_option_fee_charged_v2_total{consumer="new"} 3
deopt_option_fee_charged_v2_total{consumer="old"} 0
deopt_option_fee_charged_v2_total{consumer="unknown"} 0
deopt_option_fee_rebated_v2_total{consumer="new"} 1
deopt_option_fee_rebated_v2_total{consumer="old"} 0
deopt_option_fee_rebated_v2_total{consumer="unknown"} 0
deopt_fees_manager_v2_rebate_budget_native{asset="0x6eae407f5640b006fac9965182e238582a3b412e"} 999987
```

### Expected green state on the target host post-cutover

The values above are the floor. The soak passes day-1 only if the
target host's first post-cutover scrape:

- Returns `/health` `{"ok":true,...}`.
- Returns `anomaly_totals.old_consumer_events == 0`,
  `anomaly_totals.unknown_consumer_events == 0`.
- Returns `metrics.perp_fee_charged_v2_by_consumer.new >= 3` and
  `metrics.option_fee_charged_v2_by_consumer.new >= 3` (≥ because
  fresh trades may have increased the count).
- Returns `metrics.fees_manager_v2_rebate_budget_native["0x6eae...412e"]
  > 1000` (above the `FeesManagerV2RebateBudgetLow` threshold).
- Returns the same five contracts populated with the same addresses.

## Synthetic drill — local stand-in transcript (reference)

The agent ran the V2G-J drill packet against the same local
Alertmanager stand-in used in V2G-I, against the V2G-J routing
example. All five drills landed at the expected receiver; the webhook
sink confirms HTTP delivery.

```
Drill 1 PerpFeeChargedFromOldEngine     → deopt_oncall_critical  (sink: /critical)  ✅
Drill 2 OptionFeeConsumerUnknown        → deopt_anomaly_tickets  (sink: /tickets)   ✅
Drill 3 FeesManagerV2RebateBudgetLow    → deopt_ops_chat         (sink: /ops)       ✅
Drill 4 DeoptV2FeeMetricsAbsent         → deopt_backend_oncall   (sink: /backend)   ✅
Drill 5 HighSeverityMainnetRouteDrill   → deopt_oncall_high      (sink: /high)      ✅  (new in V2G-J, missing from V2G-I)
```

Full matrix: `/tmp/v2g_k_drill/receiver_outcome_matrix.md` (filesystem
only; not committed because it is operator-environment-specific).

The operator's target-host drill will produce the same shape if the
routing fragment was merged verbatim. If not, follow the failure /
triage block in
`docs/operator/V2G_J_SYNTHETIC_ALERT_DRILL_PACKET.md` §"Failure /
triage".

## Stalled-rule decision

`v2_fee_alerts.stalled.yml` stays **OFF** during this soak window.

Reason: the activation policy at
`docs/operator/V2G_J_STALLED_RULE_ACTIVATION_POLICY.md` requires:

1. `increase(perp_rebated_v2_total{consumer="new"}[24h]) >= 3`,
2. condition held for 48 consecutive hours,
3. Grafana panel confirms monotonic rise,
4. anomaly totals zero during window,
5. on-call ACK.

The target host has zero history at soak start (Prometheus only just
started scraping). The earliest the gate **could** be re-evaluated is
soak day 4 (T+48h after the rule pipeline has been in place for 24h
to start the 24h window, plus 24h to confirm sustained behaviour).
Re-evaluate on day 5 of the soak; decision row is in the daily
checklist below.

If the cadence gate is not met by day 7 — likely on Base Sepolia
today, where V2G-E is the only live PERP rebate — defer further. Do
not relax the activation policy under soak pressure.

## Rollback links

If any soak day shows a regression, roll back the affected layer
**only** — full rollback is not required for a single-layer fault.

| Layer                                       | Rollback command (target host)                                                                                |
|--------------------------------------------|---------------------------------------------------------------------------------------------------------------|
| Backend `.env` patch                       | `cp ~/DEOPT/deopt-v2-backend/.env.bak.<UTC> ~/DEOPT/deopt-v2-backend/.env && <restart>`                       |
| Prometheus rule bundle                     | `sudo rm /etc/prometheus/rules.d/v2_fee_alerts.bundle.yml && sudo promtool check config $B2 && curl -X POST $B5` |
| Alertmanager routing fragment              | `sudo cp $C2.bak.<UTC> $C2 && sudo amtool check-config $C2 && curl -X POST $AM_URL/-/reload`                  |
| Grafana dashboard + provisioning           | `sudo rm $D2/deopt_v2_fees.yaml $D3/deopt/v2_fee_observability_dashboard.json && sudo systemctl reload grafana-server` |
| Opt-in stalled bundle (if enabled in soak) | `sudo rm /etc/prometheus/rules.d/v2_fee_alerts.stalled.yml && curl -X POST $B5`                                |

All rollback paths are non-destructive (no DB writes, no chain
calls). Per-layer rollback aligns with the V2G-J packet §"Rollback"
sections.

## Daily checklist (7 days)

Run on each soak day at a consistent time of day. Fill in the rows
below with the operator's check timestamp + outcome. Carry forward
any anomalies to the V2G-K closure summary at the end of the soak.

For each day, run the steps in the **`Steps`** column on the target
host (or the target-host's `/metrics` URL from the operator
workstation).

### Checks (apply daily — reorder freely)

| # | Step                                                                                       | Pass criterion |
|---|--------------------------------------------------------------------------------------------|----------------|
| 1 | `curl -sf $BACKEND_URL/health`                                                             | `{"ok":true,…}` |
| 2 | `curl -sH "x-admin-token: $ADMIN_TOKEN" $BACKEND_URL/admin/fees/v2/observability \| jq '.anomaly_totals, .metrics, .contracts'` | `anomaly_totals.old_consumer_events == 0` and `…unknown_consumer_events == 0`; metric `new` buckets monotonically ≥ baseline; budget ≥ baseline `999987` (may have moved if rebates landed during the soak — drop is fine, increase is fine, jumps are noteworthy). |
| 3 | `curl -sH "x-admin-token: $ADMIN_TOKEN" $BACKEND_URL/metrics \| grep -E '^deopt_(perp\|option)_fee_(charged\|rebated)_v2_total\|^deopt_fees_manager_v2_rebate_budget_native'` | Same shape as baseline; values may have advanced |
| 4 | Prometheus `/alerts` page lists 9 deopt_*_v2_*-named alerts, all `state=inactive`           | No `pending` or `firing` against the V2 fee surface |
| 5 | Alertmanager `/api/v2/alerts` returns no V2 fee alerts                                     | empty array (or only synthetic resolution residuals) |
| 6 | Grafana — open dashboard `deopt-v2g-g-v2-fees`; confirm all 4 timeseries + 2 budget + 3 anomaly stat + engine reference panels render | All panels show data; OLD + unknown stat tiles green |
| 7 | Indexer health — `/admin/options/events` count is monotonic (or stable if backend not indexing this period) | `event_indexer_enabled=true`, last scan within scrape interval |
| 8 | No `FeesManagerV2RebateBudgetStale` firing (the bugfix from V2G-I would catch indexer lag)  | inactive |
| 9 | No `DeoptV2FeeMetricsAbsent` firing                                                        | inactive |
| 10| (Day 5 only) Re-evaluate the stalled-rule activation gate                                  | document yes/no in the per-day table |

### Per-day log

Format: `YYYY-MM-DDTHH:MMZ`, then for each item: `ok | warn | fail`,
plus a one-line note for any non-OK row. Day 0 is the soak open record
(this file); Day 1 starts T+24h after F8 reload.

| Day | Date / time (UTC) | Step 1 | Step 2 | Step 3 | Step 4 | Step 5 | Step 6 | Step 7 | Step 8 | Step 9 | Stalled rule | Notes |
|-----|-------------------|--------|--------|--------|--------|--------|--------|--------|--------|--------|--------------|-------|
| 0   | 2026-05-31T16:14Z | ok (local) | ok (local baseline) | ok (local) | n/a | n/a | n/a | n/a | n/a | n/a | deferred | Local baseline capture only. Target host pending operator gates. |
| 1   |                   |        |        |        |        |        |        |        |        |        | deferred     |       |
| 2   |                   |        |        |        |        |        |        |        |        |        | deferred     |       |
| 3   |                   |        |        |        |        |        |        |        |        |        | deferred     |       |
| 4   |                   |        |        |        |        |        |        |        |        |        | re-evaluate? |       |
| 5   |                   |        |        |        |        |        |        |        |        |        | decide       |       |
| 6   |                   |        |        |        |        |        |        |        |        |        | follow day-5 |       |
| 7   |                   |        |        |        |        |        |        |        |        |        | follow day-5 |       |

### What "warn" vs "fail" means

- **warn** — soak continues; document the anomaly. Examples: indexer
  lagged briefly but caught up within one scrape; budget gauge dipped
  but stayed above threshold; one alert pending but didn't fire.
- **fail** — soak pauses. Roll back the affected layer per the
  rollback table, root-cause, then resume.

## Soak completion criteria (after Day 7)

The soak passes if:

- Every daily row is `ok` or `warn` (no `fail`).
- Every layer rollback (if invoked) was scoped to one layer.
- `old_consumer_events == 0` for all 7 days.
- `unknown_consumer_events == 0` for all 7 days **or** ≥ 1 with a
  documented root-cause that did not require classifier widening.
- `FeesManagerV2RebateBudgetStale` did not fire (indexer healthy).
- `DeoptV2FeeMetricsAbsent` did not fire (metric pipeline healthy).
- Stalled-rule gate evaluated on day 5; decision documented.

On pass, file a V2G-L closure note that records:

- Daily checklist outcomes,
- Mean time to ack for any real alert that fired,
- Any tuning recommendations (window widths, threshold adjustments),
- Stalled-rule final decision (on / off / window changed),
- Mainnet readiness gap analysis using the V2G-G multi-asset matrix.

On fail or partial-pass: do not enable the stalled rule, file an
incident note, and re-evaluate at the next opportunity (typically
24h after root cause).

## V2G-K → V2G-L handoff

When the soak completes, V2G-L (next milestone) covers:

- Closing the soak record + producing a 7-day report.
- Wiring the multi-asset rule template (V2G-G matrix) for any new
  settlement asset that came online during the soak.
- Updating the stalled-rule window if observed cadence differs from
  the policy default.

## Remaining operator gates

| Gate | What it unlocks |
|------|------------------|
| F1   | apply `.env` patch on target backend host |
| F2   | restart backend post-patch                |
| F3   | drop V2G-G rule bundle into target Prometheus rules dir |
| F4   | reload Prometheus                          |
| F5   | merge routing fragment in target Alertmanager |
| F6   | reload Alertmanager                         |
| F7   | drop Grafana provisioning + dashboard JSON  |
| F8   | reload Grafana                              |
| F9   | fire synthetic drill against target Alertmanager |
| F10  | enable opt-in stalled bundle (default NO — see policy) |
| F11  | allow agent SSH to target host (user@host + auth) |

Until each gate has an explicit `YES` from the operator (plus the
target paths from sections A-E of the questionnaire), the V2G-K
soak day-1 row stays blank.

## Next recommended milestone

**V2G-L — 7-day soak closure + multi-asset rollout prep.**

- Close the soak record above with the daily outcomes.
- Author the V2G-L closure note covering real-alert MTTR, tuning
  recommendations, and stalled-rule final decision.
- Pull the V2G-G multi-asset matrix into a concrete rule-clone
  proposal for any new settlement assets that came online (or remain
  pending for mainnet).
- Begin V2H scope on FeesManagerV2 multi-asset support: the metric
  pipeline already emits one `asset=...` series per indexed
  settlement asset; multi-asset is primarily an operator
  rollout exercise.
