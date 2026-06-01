# V2G-L4 — Canonical Day-1 Local-Compose 24 h Soak Checkpoint

## Status

- Milestone: **V2G-L4 canonical** — official 24 h day-1 close for the
  V2 fee local-compose monitoring soak that V2G-L2 started at
  `2026-05-31T17:38Z`.
- Date: **2026-06-01T17:39Z** (gate `+75 s`).
- Outcome:
  - **All acceptance criteria met.**
  - **No restart, no reset, no chain mutation, no `.env` edit, no DB write.**
  - **Canonical day-1 row added** to
    `docs/V2_FEE_OBSERVABILITY_7_DAY_SOAK_V2G_K.md` per-day table.
  - **Two residual provisioning warnings** carried forward — both
    informational, neither blocks the dashboard or the canonical
    soak close.

---

## 1. Time gate

| Check | Value |
|---|---|
| `date -u` at run | `Mon Jun  1 17:39:15 UTC 2026` |
| Required gate | `2026-06-01T17:38:00Z` |
| Delta | **+75 s** |
| Decision | **CONTINUE** |

This is the first V2G-L4 attempt at which the time gate is open.
Prior probes were logged as `1-early` (V2G-L3 at T+25m) and
`1-polish` (V2G-L4 polish at T+36m) per the per-day table.

## 2. Runtime status

```
$ docker compose ps
NAME                        SERVICE        STATUS
deopt-v2g-l0-alertmanager   alertmanager   Up 24 hours
deopt-v2g-l0-grafana        grafana        Up 23 hours
deopt-v2g-l0-prometheus     prometheus     Up 24 hours (healthy)
deopt-v2g-l0-webhook-sink   webhook-sink   Up 23 hours
```

All 4 expected containers present. Prometheus reports the
container-side healthcheck (`healthy`). No restart, no recreation.

## 3. Backend status

```
$ curl -s http://127.0.0.1:8080/health
{"ok":true,"service":"deopt-v2-backend"}
```

PID 56199 still alive; same binary as V2G-G (predates V2G-M endpoint,
V2G-S `by_product`/`by_flow`, V2G-W0 constant-time token compare —
all queued for the post-day-1 restart window).

## 4. Prometheus status

| Probe | Result |
|---|---|
| `GET /-/ready` | `Prometheus Server is Ready.` |
| `GET /-/healthy` | `Prometheus Server is Healthy.` |
| `GET /api/v1/targets?state=active` | 3/3 up: `alertmanager:9093` (alertmanager), `dev-local` (deopt-v2-backend), `localhost:9090` (prometheus) |
| `GET /api/v1/rules` | 9 alerting rules loaded, **all `inactive`** |

The 9 V2 fee alerts (V2G-G + V2G-I activation + V2G-K stalled
guard) load cleanly. No alert is firing or pending.

## 5. Metrics baseline

| Metric | Result |
|---|---|
| `deopt_perp_fee_charged_v2_total{consumer="new"}` | **3** |
| `deopt_perp_fee_charged_v2_total{consumer="old"}` | 0 |
| `deopt_perp_fee_charged_v2_total{consumer="unknown"}` | 0 |
| `deopt_perp_fee_rebated_v2_total{consumer="new"}` | **1** |
| `deopt_perp_fee_rebated_v2_total{consumer="old"}` | 0 |
| `deopt_perp_fee_rebated_v2_total{consumer="unknown"}` | 0 |
| `deopt_option_fee_charged_v2_total{consumer="new"}` | **3** |
| `deopt_option_fee_charged_v2_total{consumer="old"}` | 0 |
| `deopt_option_fee_charged_v2_total{consumer="unknown"}` | 0 |
| `deopt_option_fee_rebated_v2_total{consumer="new"}` | **1** |
| `deopt_option_fee_rebated_v2_total{consumer="old"}` | 0 |
| `deopt_option_fee_rebated_v2_total{consumer="unknown"}` | 0 |
| `deopt_fees_manager_v2_rebate_budget_native{asset=0x6eae…412e}` | **999987** |
| `up` (3 jobs) | all 1 |

Every value matches the V2G-E live smoke close exactly. No drift
across 24 h.

## 6. Alertmanager status

| Probe | Result |
|---|---|
| `GET /-/ready` | `OK` |
| `GET /api/v2/alerts` | empty list |
| `GET /api/v2/silences` | empty list |

No replays of the synthetic drill were issued. No live drill is
required to close the day-1 gate.

## 7. Grafana status

| Probe | Result |
|---|---|
| `GET /api/health` | `{"database":"ok","version":"11.4.0",...}` |
| Datasource `Prometheus` | present, uid `prometheus`, type `prometheus` |
| Folder `DeOpt` | present, uid `deopt` |
| Dashboard `deopt-v2g-g-v2-fees` | present in folder `DeOpt`, title `DeOpt — V2 fee observability (V2G-G)`, 10 content panels + 5 row containers |
| Panel queries | live (sampled — every metric referenced by the dashboard returned data in §5) |

### Panel inventory (10 content panels)

| id | title | type |
|---|---|---|
| 1 | PERP FeeChargedV2 by consumer | timeseries |
| 2 | PERP FeeRebatedV2 by consumer | timeseries |
| 3 | OPTION FeeChargedV2 by consumer | timeseries |
| 4 | OPTION FeeRebatedV2 by consumer | timeseries |
| 5 | Rebate budget (native units) by settlement asset | timeseries |
| 6 | Base Sepolia mUSDC rebate budget (latest) | stat |
| 7 | OLD consumer events (PERP) | stat |
| 8 | OLD consumer events (OPTION) | stat |
| 9 | Unknown consumer events (PERP + OPTION) | stat |
| 10 | Active engine addresses (reference) | table |

### Residual log warnings (carry-forward; non-blocking)

| Source | Warning | Status |
|---|---|---|
| `provisioning.plugins` | `Failed to read plugin provisioning files from directory path=/etc/grafana/provisioning/plugins error="no such file or directory"` | Container started at `2026-05-31T17:31Z`, before the V2G-L4-polish `.keep.yaml` stubs were written on the host. The container's view of the bind mount still does not include the file. Dashboard rendering is unaffected. Resolution: next backend / compose restart window will pick up the polish stub. |
| `provisioning.alerting` | `file has invalid suffix 'README.md' / '.gitkeep' / 'README.disabled.md', skipping` | The alerting provisioning directory now contains a few documentation breadcrumbs. Grafana warns and skips. Resolution: tidy the directory at the next maintenance window; the polish files are non-functional. |
| Acceptable image-internal noise (per task allowlist) | `xychart already registered`, `Skipping finding plugins ... plugins-bundled` | not surfaced here; suppressed at filter time. |

The provisioning warnings are **informational only** — they were
emitted at startup, never re-emitted on scrape, and have no
runtime effect on the dashboard, datasource, or panels we
exercise. Day-1 close acceptance remains green.

## 8. Soak record update

`docs/V2_FEE_OBSERVABILITY_7_DAY_SOAK_V2G_K.md` per-day table row
**Day 1** has been filled with:

- timestamp `2026-06-01T17:39Z (T+24h 01m)`
- all 9 daily-check columns set to `ok`
- the carry-forward note about the two residual provisioning
  warnings
- a pointer to this canonical doc
- stalled-rule column stays at `deferred` (Day-5 gate)

The `1-early` (V2G-L3) and `1-polish` rows are left in place as
the historical record.

## 9. Validations

| Command | Result |
|---|---|
| `cargo fmt --all -- --check` | ✅ clean |
| `cargo clippy --all-targets --all-features -- -D warnings` | ✅ clean |
| `cargo test --all-targets --all-features --no-fail-fast` | ✅ **735 / 0 / 0** (V2G-W0 baseline 735 — no test added in V2G-L4) |
| `cargo build --all-targets --all-features` | ✅ |
| `promtool check rules docs/monitoring/prometheus/v2_fee_alerts.bundle.yml` | ✅ SUCCESS — 9 rules |
| `promtool check rules docs/monitoring/prometheus/v2_fee_alerts.stalled.yml` | ✅ SUCCESS — 1 rule |
| `promtool test rules docs/monitoring/prometheus/v2_fee_alerts.test.yml` | ✅ SUCCESS |
| `amtool check-config docs/monitoring/alertmanager/v2_fee_routing.example.yml` | ✅ SUCCESS — 6 receivers, 1 inhibit rule, 0 templates |
| `python3 -m json.tool docs/monitoring/grafana/v2_fee_observability_dashboard.json >/dev/null` | ✅ OK |
| `docker compose config -q` | ✅ OK |

All validations executed with the canonical CLI invocation
(`--entrypoint promtool` / `--entrypoint amtool` for the docker
images, since the upstream `prom/prometheus` and
`prom/alertmanager` images ship promtool/amtool as siblings to
the default entrypoint, not as the entrypoint itself).

## 10. Hard-gate compliance

| Gate | Honoured? |
|---|---|
| Do not broadcast | ✅ |
| Do not submit transactions | ✅ |
| Do not mutate live chain | ✅ |
| Do not print private keys | ✅ |
| Do not edit real secret `.env` | ✅ (a one-line redacted grep on `local-stack/.env` for the Grafana admin var names only — no value transcription) |
| Do not delete DB rows | ✅ |
| Do not reset Prometheus data | ✅ |
| Do not run `docker compose down -v` | ✅ |
| Do not restart backend | ✅ |
| Do not restart Docker Compose | ✅ |
| Do not replay synthetic alerts | ✅ |
| Do not use OLD_PERP_ENGINE as active | ✅ |
| Do not change Merkle root | ✅ |
| Do not change rebate budget | ✅ |

## 11. Remaining blockers

1. **Provisioning carry-forward**: the running Grafana container
   still does not see the V2G-L4-polish `.keep.yaml` stub in
   `/etc/grafana/provisioning/plugins`, and the alerting
   provisioning dir contains breadcrumb files Grafana skips. Fix
   lands at the next compose-restart maintenance window.
2. **Target-host monitoring cutover (F1-F11)** still pending —
   the local L0 stack carries the soak; the canonical production
   host has not yet been provisioned.
3. **Backend restart pickup**: V2G-M `/admin/fees/v2/smoke/readiness`,
   V2G-S `by_product`/`by_flow` JSON fields, V2G-W0 constant-time
   token compare all sit in `target/` awaiting the post-day-1
   restart window.
4. **OPTION RFQ live deploy (V2G-P)** — code-ready since V2G-O /
   V2G-P0 / V2G-P1; broadcast pending operator window now that
   the day-1 gate has cleared.
5. **ProtocolFeeVault live deploy (V2G-R5)** — preceded by V2G-R3
   (FM-V2 hook ABI + CollateralVault `transferFromInternalAccount`).
6. **Day-5 stalled-rule re-evaluation gate** — not yet reached.

## 12. Next recommended milestone

**Operator-window pickup**. Now that the canonical day-1 gate has
cleared, the operator can schedule the post-day-1 backend restart
window. The restart sequence to plan:

1. Stop PID 56199 cleanly (`SIGTERM`).
2. Tidy `local-stack/grafana/provisioning/alerting/` (remove
   breadcrumb files) and confirm the `.keep.yaml` stubs are in
   both `alerting/` and `plugins/`.
3. `docker compose restart grafana` to pick up the polish stubs.
4. Restart the backend binary (V2G-W0 + V2G-S + V2G-M code path).
5. Re-run the V2G-L checks to confirm the post-restart baseline.
6. Then begin the day-2 soak row.

The V2G-P / V2G-R3 / V2G-R5 broadcast sessions are independent of
this restart but should be sequenced against the same operator
window so the entire post-day-1 cutover is a single coordinated
event.
