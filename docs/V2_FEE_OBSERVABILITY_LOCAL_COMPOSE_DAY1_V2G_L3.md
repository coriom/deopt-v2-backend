# V2G-L3 — local-compose soak day-1 check

## Status

- Milestone: **V2G-L3** — first in-soak health check after the V2G-L2
  compose stack came up.
- Date: 2026-05-31.
- Soak open (V2G-L2 day-0): `2026-05-31T17:38Z`.
- Day-1 check timestamp: `2026-05-31T18:03Z` (T+25min elapsed).
- Outcome: **green across all 9 daily-check steps.** Read-only check;
  no synthetic alerts replayed, no rule changes, no DB writes, no
  chain RPC writes.

**Honest caveat on the soak clock.** The day-1 checklist in
`docs/V2_FEE_OBSERVABILITY_7_DAY_SOAK_V2G_K.md` lists day-1 as
T+24h. The agent ran V2G-L3 at T+25min so the soak record is
continuously up-to-date — but this **does not count as the canonical
day-1 24h checkpoint**. A second tick is required after a real 24h
of clean operation. V2G-L3 marks the soak row as `day-1-early` so
the canonical day-1 row stays reserved for the calendar gate.

## Phase 1 — Runtime status

| Container                 | Image                      | Status                | Uptime |
|--------------------------|----------------------------|-----------------------|--------|
| deopt-v2g-l0-prometheus   | prom/prometheus:v3.12.0    | Up 33 min (healthy)   | from 17:30Z |
| deopt-v2g-l0-alertmanager | prom/alertmanager:v0.32.1  | Up 36 min             | from 17:27Z |
| deopt-v2g-l0-grafana      | grafana/grafana:11.4.0     | Up 31 min             | from 17:32Z |
| deopt-v2g-l0-webhook-sink | python:3.13-slim           | Up 31 min             | from 17:32Z |

Backend process: PID `56199`, binding `0.0.0.0:8080`, `/health` ✅
`{"ok":true,"service":"deopt-v2-backend"}`.

Listening ports: 8080 (backend), 9090 (prom), 9093 (AM), 3000
(grafana), 9095 (sink) — all `127.0.0.1` except backend on
`0.0.0.0` (so the compose Prometheus can scrape via
`host.docker.internal`).

Container resource snapshot:

```
deopt-v2g-l0-prometheus     CPU 0.23%   MEM 47.8 MiB    Net 1.96MB/187kB
deopt-v2g-l0-alertmanager   CPU 0.07%   MEM 17.8 MiB    Net 101kB/1.09MB
deopt-v2g-l0-grafana        CPU 0.07%   MEM 55.2 MiB    Net 9.18MB/113kB
deopt-v2g-l0-webhook-sink   CPU 0.01%   MEM 13.3 MiB    Net 17.3kB/7.06kB
```

All well below the soak-warning thresholds (mem < 256MB, CPU < 5%).

## Phase 2 — Prometheus

- `/-/ready` → `Prometheus Server is Ready.`
- `/api/v1/targets` → 3/3 up, no `lastError` on any target:
  - `alertmanager` last scrape `2026-05-31T18:03:58.730Z`
  - `deopt-v2-backend` last scrape `2026-05-31T18:03:56.675Z`
  - `prometheus` last scrape `2026-05-31T18:03:49.786Z`
- `/api/v1/rules` → 4 groups / 9 rules, every alert
  `state=inactive health=ok`.

PromQL day-1 baseline (identical to V2G-K / L0 / L1 / L2):

```
deopt_perp_fee_charged_v2_total      consumer=new=3, old=0, unknown=0
deopt_perp_fee_rebated_v2_total      consumer=new=1, old=0, unknown=0
deopt_option_fee_charged_v2_total    consumer=new=3, old=0, unknown=0
deopt_option_fee_rebated_v2_total    consumer=new=1, old=0, unknown=0
deopt_fees_manager_v2_rebate_budget_native{asset=0x6eae...412e} = 999987
up{job=prometheus|alertmanager|deopt-v2-backend}                = 1
deopt_db_up                                                     = 1
```

No drift since V2G-L2 day-0. Backend is read-only on chain (no live
trades during this window), so the baseline holding constant is the
expected outcome.

## Phase 3 — Alertmanager

- `/-/ready` → `OK`.
- `/api/v2/status`: version `0.32.1`, uptime since
  `2026-05-31T17:27:15Z`, cluster `ready`.
- `/api/v2/alerts`: **0 active alerts** (0 V2-milestone alerts, 0
  total alerts).
- `/api/v2/silences`: 0 silences in place.
- Webhook sink log:
  `docker compose exec webhook-sink wc -l /var/log/sink/received.log`
  → 11 lines (V2G-L2 drill residuals; no new dispatches since).

No synthetic drills replayed (per the V2G-L3 hard rule "do not run
synthetic alerts unless needed for diagnosis"). The V2G-L2 drill
matrix remains the authoritative routing-verified state.

## Phase 4 — Grafana

- `/api/health` → `{"database":"ok","version":"11.4.0","commit":"..."}`.
- Datasource `Prometheus` (uid `prometheus`), url
  `http://prometheus:9090`, isDefault=true.
- Dashboard `DeOpt — V2 fee observability (V2G-G)` UID
  `deopt-v2g-g-v2-fees` in folder `DeOpt`, **10 panels**,
  `version=1`, `updated=2026-05-31T17:31:46Z` (V2G-L2 first import,
  no edits since).
- Grafana proxy queries through the dashboard's datasource:

  ```
  deopt_perp_fee_charged_v2_total      consumer=new=3, old=0, unknown=0
  deopt_option_fee_rebated_v2_total    consumer=new=1, old=0, unknown=0
  deopt_fees_manager_v2_rebate_budget_native{asset=0x6eae...412e} = 999987
  ```

  Match the direct Prometheus queries — Grafana's data path is
  end-to-end healthy.

Operator dashboard URL (no change since V2G-L2):
`http://127.0.0.1:3000/d/deopt-v2g-g-v2-fees` (admin / admin).

## Container-log review (no regressions)

All ERROR/WARN findings in `docker logs --tail 100` predate the
V2G-L2 fix or are benign:

| Container    | Finding                                                                                     | Status |
|--------------|---------------------------------------------------------------------------------------------|--------|
| prometheus   | 3 ERROR lines about the missing rule file at `17:27-17:29Z`                                  | **Pre-V2G-L2** (before the symlink → literal-copy fix). No errors since `17:29Z`. ✅ |
| alertmanager | none                                                                                        | ✅      |
| grafana      | 1 early-startup warn: "can't read alerting provisioning files from directory" + 2 user-ID-namespace warnings | Benign Grafana 11 initialisation noise (no `provisioning/alerting/` dir; we don't ship Grafana-managed alert rules — Prometheus rules cover the V2G-G surface) ✅ |
| webhook-sink | `DeprecationWarning: datetime.datetime.utcnow() is deprecated`                              | Non-blocking on Python 3.13. Future cleanup: switch to `datetime.now(datetime.UTC)`. ✅ |

## Day-1 daily-check matrix (row in V2G-K)

| Step | What | Day-1 result |
|------|------|--------------|
| 1 | `/health`                                              | ok |
| 2 | `/admin/fees/v2/observability` anomaly + metrics      | ok (anomaly_totals=0, metrics=baseline) |
| 3 | `/metrics` V2 fee gauges                              | ok (13 series at baseline) |
| 4 | Prometheus `/alerts` page — 9 alerts inactive         | ok |
| 5 | Alertmanager `/api/v2/alerts` empty                   | ok |
| 6 | Grafana dashboard renders                              | ok (10 panels resolve through DS) |
| 7 | Indexer health (`deopt_db_up=1`)                       | ok |
| 8 | No `FeesManagerV2RebateBudgetStale` firing             | ok (inactive) |
| 9 | No `DeoptV2FeeMetricsAbsent` firing                    | ok (inactive) |
| 10| (Day-5 only) stalled-rule re-evaluation                | n/a (day-1) |

## Stalled-rule decision

**Defer.** TSDB history is < 1h. The V2G-J activation gate (≥3 PERP
rebates / 24h × 48h sustained) cannot be evaluated. Re-evaluation
marker stays on V2G-K soak day 5.

## V2G-J F-gates

Still pending — V2G-L3 is the local rehearsal soak, not the target
cutover. The V2G-J F1-F11 sequence remains the operator's next
external action **after** the canonical day-1 24h marker (or the
operator's accepted-rehearsal-shortcut policy).

## Follow-up suggestions (non-blocking)

1. Switch `webhook_sink.py` from `datetime.utcnow()` to
   `datetime.now(datetime.UTC)` — one-line change inside
   `docs/monitoring/local-stack/webhook-sink/webhook_sink.py`.
   Can be picked up by V2G-L3+ or carried into the soak-end note.
2. Optional: scrape `docker stats` continuously into a sidecar so the
   soak record has per-day resource trend data instead of single-point
   snapshots.
3. Optional: add an `Alerting provisioning` directory placeholder so
   the Grafana startup log stops complaining about
   `/etc/grafana/provisioning/alerting`. Zero functional impact today
   — Prometheus owns the V2G-G alert path, not Grafana.

## Files changed (V2G-L3 delta)

Created:

- `docs/V2_FEE_OBSERVABILITY_LOCAL_COMPOSE_DAY1_V2G_L3.md` (this doc).

Updated:

- `docs/V2_FEE_OBSERVABILITY_7_DAY_SOAK_V2G_K.md` — day-1-early row +
  V2G-L3 closure note.
- `docs/ALERTING_SPEC.md` — V2G-L3 day-1 pointer.
- `docs/RUNBOOK_PERP_V2_FEE_ALERTS.md` — V2G-L3 day-1 pointer.

No backend / frontend / Sol code touched.

## Validations

| Command | Result |
|---|---|
| `cargo fmt --all --check` | ✅ |
| `cargo clippy --all-targets --all-features -- -D warnings` | ✅ |
| `cargo build --all-targets --all-features` | ✅ |
| `cargo test --all-targets --all-features --no-fail-fast` | ✅ **679 / 0 / 0** |
| `promtool check rules docs/monitoring/prometheus/v2_fee_alerts.bundle.yml` | ✅ 9 rules |
| `promtool check rules docs/monitoring/prometheus/v2_fee_alerts.stalled.yml` | ✅ 1 rule |
| `promtool test rules docs/monitoring/prometheus/v2_fee_alerts.test.yml` | ✅ SUCCESS (7 groups) |
| `amtool check-config docs/monitoring/alertmanager/v2_fee_routing.example.yml` | ✅ |
| `amtool check-config docs/monitoring/local-stack/alertmanager/alertmanager.yml` | ✅ |
| `python3 -m json.tool docs/monitoring/grafana/v2_fee_observability_dashboard.json` | ✅ |
| `docker compose -f docs/monitoring/local-stack/compose.yml config -q` (via `sg docker`) | ✅ |

## Remaining blockers

1. **Canonical day-1 24h checkpoint not yet ticked.** V2G-L3 is an
   early progress probe; the true day-1 row gets ticked after T+24h
   of continuous green.
2. **`v2_fee_alerts.stalled.yml` stays disabled.** Activation gate
   still cannot be evaluated; marker on day 5.
3. **V2G-J target-host F-gates (F1-F11) still pending.** Local
   rehearsal continues.
4. **Webhook sink `datetime.utcnow()` deprecation warning.**
   Non-blocking but worth a one-line fix in a follow-up commit.

## Next recommended milestone

**V2G-L4 / V2G-K canonical day-1 — T+24h soak checkpoint.**

- Wait until at least `2026-06-01T17:38Z` (T+24h after the V2G-L2
  stack open).
- Re-run the V2G-L3 checklist; mark the canonical day-1 row in
  `docs/V2_FEE_OBSERVABILITY_7_DAY_SOAK_V2G_K.md`.
- Optionally pick up the `datetime.utcnow()` deprecation fix from the
  follow-up suggestion list.
- Continue daily checks through day-7 closure.
