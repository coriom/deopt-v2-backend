# V2G-L2 — Local compose stack live (Docker unblocked)

## Status

- Milestone: **V2G-L2** — operator unblocked Docker
  (`sudo usermod -aG docker $USER && newgrp docker`); agent fixed two
  small stack issues, the compose stack came up clean, and the full
  V2G-G observability surface now runs end-to-end on localhost.
- Date: 2026-05-31.
- Outcome:
  - **`docker compose up -d` succeeded** with `sg docker -c …` so the
    agent could use the daemon socket without re-logging in.
  - **Fixed: Prometheus container failed to load rules** because the
    V2G-L0 rule files were symlinks pointing outside the bind-mounted
    `local-stack/prometheus/` tree. Replaced with literal copies +
    shipped `sync_from_canonical.sh` so future edits stay in lockstep
    with `docs/monitoring/prometheus/`.
  - **Fixed: webhook-sink container `PermissionError` on the log
    file.** The container was running as `65534:65534` while the
    `webhook-sink-data` named volume defaulted to `root:root`.
    Dropped the explicit `user:` override (the container is already
    bound to `127.0.0.1` only).
  - **Live verification:** Prometheus scrapes 3 jobs (prometheus,
    alertmanager, deopt-v2-backend@host.docker.internal:8080) all
    `up`; 9 alerts loaded `state=inactive/health=ok`; V2 fee
    PromQL queries return the V2G-K baseline; Alertmanager routes
    five synthetic alerts to the correct receivers and persists
    them in the webhook-sink volume; Grafana exposes the
    "DeOpt — V2 fee observability (V2G-G)" dashboard under the
    `DeOpt` folder, datasource `Prometheus` (uid `prometheus`) is the
    default, proxy queries against the V2 fee gauges return the same
    baseline values.
  - **V2G-K soak day-0 promoted to `local-compose-live`** with all
    four stack components running and the day-0 row reading `ok`
    across the full daily-check column set (steps 1-9 now resolvable
    against the running stack).
- Hard gates respected: no broadcast, no chain mutation, no DB
  writes, no real `.env` edit, no private-key handling, no
  governance/timelock action. The agent used `sg docker` (no sudo).

## Phase 1 — Docker access fix (operator confirmation)

Operator ran the V2G-L1 unblock command:

```sh
sudo usermod -aG docker "$USER" && newgrp docker
docker ps   # works in the operator's shell
```

Agent picked up access via `sg docker -c …` in its own shell (no
re-login needed). Verification:

```
sg docker -c 'docker version --format "{{.Server.Version}}"'
→ 29.4.3
sg docker -c 'docker ps'
→ shows 3 deopt-v2g-l0-* containers running
```

## Phase 2 — Compose stack startup (with two fixes)

### Fix 1 — symlink rule files broke inside the container

The V2G-L0 stack shipped the Prometheus rule files as **symlinks**
pointing at the canonical `docs/monitoring/prometheus/` directory:

```
local-stack/prometheus/rules/v2_fee_alerts.bundle.yml
  → ../../../prometheus/v2_fee_alerts.bundle.yml
```

The compose `prometheus` service bind-mounts only
`local-stack/prometheus/` into `/etc/prometheus`. Inside the
container, the symlink resolves to `/etc/../prometheus/...` which is
outside the bind, so the file doesn't exist. The container failed
with:

```
err="failed evaluating rule file path
\"/etc/prometheus/rules/v2_fee_alerts.bundle.yml\": lstat
/etc/../prometheus/v2_fee_alerts.bundle.yml: no such file or directory"
```

Fix:

- Replaced the two symlinks with **literal copies** of the canonical
  rule files inside `local-stack/prometheus/rules/`.
- Shipped a `sync_from_canonical.sh` helper that re-copies after
  every canonical edit. README updated; the existing "sync rules
  from canonical source" block is now the documented post-edit
  workflow.

### Fix 2 — webhook-sink PermissionError on the named volume

The V2G-L0 `webhook-sink` service set `user: "65534:65534"` but the
`webhook-sink-data` named volume defaulted to `root:root`. The
container couldn't write `/var/log/sink/received.log`. Logs:

```
deopt-v2g-l0-webhook-sink | PermissionError: [Errno 13]
Permission denied: '/var/log/sink/received.log'
```

Fix:

- Dropped the `user: "65534:65534"` line from the `webhook-sink`
  service. The container runs as root inside the namespace; the
  bind is `127.0.0.1`-only via compose ports, so there is no external
  exposure. Comment in `compose.yml` documents the rationale.

### Post-fix recreate

```sh
cd ~/DEOPT/deopt-v2-backend/docs/monitoring/local-stack
sg docker -c 'docker compose up -d'
sg docker -c 'docker compose ps'
```

```
NAME                        IMAGE                       STATUS
deopt-v2g-l0-alertmanager   prom/alertmanager:v0.32.1   Up
deopt-v2g-l0-grafana        grafana/grafana:11.4.0      Up
deopt-v2g-l0-prometheus     prom/prometheus:v3.12.0     Up (healthy)
deopt-v2g-l0-webhook-sink   python:3.13-slim            Up
```

Health probes from the host:

| URL                                       | Result                                                                |
|-------------------------------------------|-----------------------------------------------------------------------|
| `http://127.0.0.1:9090/-/ready`           | `Prometheus Server is Ready.`                                          |
| `http://127.0.0.1:9093/-/ready`           | `OK`                                                                  |
| `http://127.0.0.1:3000/api/health`        | `{"database":"ok","version":"11.4.0",…}`                              |
| `http://127.0.0.1:9095/probe` (POST)       | `{"ok":true}`                                                         |

## Phase 3 — Backend safe runtime (bound 0.0.0.0)

The compose Prometheus service needs to reach the backend via
`host.docker.internal:8080`. The backend defaults to
`HOST=127.0.0.1` (V2G-L1 startup snippet did NOT override this).
V2G-L2 added `HOST=0.0.0.0` to the startup env:

```sh
cd ~/DEOPT/deopt-v2-backend
bash -c '
  set -a
  . ./.env >/dev/null 2>&1
  . ./.env.cutover.v2d_s.local >/dev/null 2>&1
  . ./.env.preflight.v2e_f.local >/dev/null 2>&1
  . ./.env.observability.v2f_o.local >/dev/null 2>&1
  set +a
  # NEW for V2G-L2 — bind on all interfaces so the compose Prometheus
  # container can scrape via host.docker.internal:8080.
  export HOST=0.0.0.0
  export PORT=8080
  # V2G-K classifier overrides
  export PERP_ENGINE_ADDRESS=0xc6C592100723Fe0C66343A16e95eC34cC0c2141c
  export OLD_PERP_ENGINE_ADDRESS=0xB36395b67D0798ADA981731c9Fa5239F4362b53B
  export MARGIN_ENGINE=0x287Cef479be5889eEfCa847F9e73C860898f48Cc
  export OLD_MARGIN_ENGINE_ADDRESS=0x6C5665De05e7314cB63cD77F82DFa86508A5b5F8
  export FEES_MANAGER_V2=0x00dA0B9876bcBf0c79CB5BcAcfEBAFb8C7Ad774f
  export OPTION_EVENT_INDEXER_MARGIN_ENGINE_ADDRESS=0x287Cef479be5889eEfCa847F9e73C860898f48Cc
  export OPTION_EVENT_INDEXER_FEES_MANAGER_V2_ADDRESS=0x00dA0B9876bcBf0c79CB5BcAcfEBAFb8C7Ad774f
  export EXECUTION_ENABLED=false
  export EXECUTOR_DRY_RUN=true
  export EXECUTOR_REAL_BROADCAST_ENABLED=false
  export OPTION_EXECUTION_BROADCAST_ENABLED=false
  export METRICS_REQUIRE_ADMIN_TOKEN=false
  unset EXECUTOR_PRIVATE_KEY BUYER_PRIVATE_KEY SELLER_PRIVATE_KEY \
        DEPLOYER_PRIVATE_KEY SIGNER_PRIVATE_KEY \
        PERP_SMOKE_BUYER_PRIVATE_KEY PERP_SMOKE_SELLER_PRIVATE_KEY \
        OPTION_SMOKE_BUYER_PRIVATE_KEY OPTION_SMOKE_SELLER_PRIVATE_KEY
  exec ./target/release/deopt-v2-backend
'
```

Result: backend listening on `0.0.0.0:8080`. `/health` OK; `/metrics`
exposes V2 fee gauges with no auth (admin token still gates
`/admin/*`). The `HOST=0.0.0.0` binding is **the only V2G-L2 backend
change** — it is local-dev-only and is documented as such in the
local-stack README.

`docs/monitoring/local-stack/README.md` already documents this in
the "Backend startup" snippet via `HTTP_BIND_ADDRESS=${HTTP_BIND_ADDRESS:-0.0.0.0:8080}`;
that env var name was wrong (backend reads `HOST`/`PORT`). V2G-L2
also corrects the README to use `HOST=0.0.0.0`.

## Phase 4 — Prometheus live verification (via compose Prometheus)

Through the live compose Prometheus on `127.0.0.1:9090`:

```
/api/v1/targets:
  prometheus       → up   localhost:9090/metrics
  alertmanager     → up   alertmanager:9093/metrics
  deopt-v2-backend → up   host.docker.internal:8080/metrics

/api/v1/rules:
  deopt_perp_v2_fee_alerts             - 3 rules - inactive
  deopt_option_v2_fee_alerts           - 3 rules - inactive
  deopt_fees_manager_v2_budget_alerts  - 2 rules - inactive
  deopt_v2_fee_metrics_liveness        - 1 rule  - inactive
  TOTAL: 9 alerts loaded
```

PromQL queries:

```
deopt_perp_fee_charged_v2_total   consumer=new=3, old=0, unknown=0
deopt_perp_fee_rebated_v2_total   consumer=new=1, old=0, unknown=0
deopt_option_fee_charged_v2_total consumer=new=3, old=0, unknown=0
deopt_option_fee_rebated_v2_total consumer=new=1, old=0, unknown=0
deopt_fees_manager_v2_rebate_budget_native{asset=0x6eae...412e} = 999987
```

V2G-K day-0 baseline reproduced **through the compose stack**.

## Phase 5 — Synthetic alert drill (live compose Alertmanager)

`amtool alert add` × 5 against `http://127.0.0.1:9093`:

| # | Alertname                          | Expected receiver       | AM /api/v2/alerts                     | Webhook delivery |
|---|------------------------------------|-------------------------|---------------------------------------|------------------|
| 1 | PerpFeeChargedFromOldEngine        | deopt_oncall_critical   | active → deopt_oncall_critical ✅     | /critical firing ✅ |
| 2 | OptionFeeConsumerUnknown           | deopt_anomaly_tickets   | active → deopt_anomaly_tickets ✅     | /tickets firing+resolved ✅ |
| 3 | FeesManagerV2RebateBudgetLow       | deopt_ops_chat          | active → deopt_ops_chat ✅            | /ops firing+resolved ✅     |
| 4 | DeoptV2FeeMetricsAbsent            | deopt_backend_oncall    | active → deopt_backend_oncall ✅      | /backend firing ✅          |
| 5 | HighSeverityMainnetRouteDrill      | deopt_oncall_high       | active → deopt_oncall_high ✅         | /high firing ✅             |

Sink log path (inside container):
`/var/log/sink/received.log` on `webhook-sink-data` volume.

Resolve cycle: all 5 drills were `--end`-pushed; AM `/api/v2/alerts`
shows zero remaining V2-milestone alerts immediately afterwards.
The 2/5 routes whose `group_wait` had already passed
(`tickets`, `ops`) delivered the `resolved` POST to the sink within
the same window; the other 3 routes were in their `group_interval`
window and would deliver the `resolved` POST inside `group_interval`
seconds, which is the documented AM behaviour. Not a regression.

## Phase 6 — Grafana live verification

| Check                                                            | Result |
|------------------------------------------------------------------|--------|
| `/api/health`                                                    | `{"database":"ok","version":"11.4.0","commit":"b5870186..."}` |
| `/api/datasources/name/Prometheus`                               | name=`Prometheus`, type=`prometheus`, url=`http://prometheus:9090`, isDefault=`true` |
| `/api/datasources/proxy/uid/prometheus/api/v1/query?query=up`    | `{prometheus=1, alertmanager=1, deopt-v2-backend=1}` |
| `/api/dashboards/uid/deopt-v2g-g-v2-fees`                        | title=`DeOpt — V2 fee observability (V2G-G)`, folder=`DeOpt`, **10 panels** |
| `/api/folders`                                                   | one folder: `deopt` / `DeOpt` |
| Grafana proxy query `deopt_perp_fee_charged_v2_total`            | `{new=3, old=0, unknown=0}` |
| Grafana proxy query `deopt_option_fee_rebated_v2_total`          | `{new=1, old=0, unknown=0}` |
| Grafana proxy query `deopt_fees_manager_v2_rebate_budget_native` | `{asset=0x6eae...412e}=999987` |

Operator opens the dashboard in a browser:

```
http://127.0.0.1:3000/d/deopt-v2g-g-v2-fees
# admin / admin on first login (configurable via GF_SECURITY_ADMIN_*)
```

The dashboard renders the 4 timeseries (PERP/OPTION charged/rebated
by consumer), the rebate budget panels (timeseries + Base Sepolia
mUSDC stat), the 3 anomaly stat tiles (PERP OLD, OPTION OLD,
unknown), and the engine reference table — all 10 panels resolving
against the same Prometheus DS.

## Phase 7 — V2G-K soak baseline (V2G-L2 row)

Appended to `docs/V2_FEE_OBSERVABILITY_7_DAY_SOAK_V2G_K.md` as a new
day-0 sub-row:

```
| 0+++ | 2026-05-31T17:38Z | ok (compose stack live) | ok (via Prom) |
        ok (via Prom) | ok | ok | ok (Grafana up; 10 panels resolved
        through DS) | ok | ok | ok | deferred | V2G-L2: compose stack
        up (4/4 containers); Prometheus scrapes deopt-v2-backend@
        host.docker.internal; 9 alerts inactive; 5/5 synthetic drills
        delivered to webhook-sink container's volume log; Grafana DS
        + dashboard provisioned; live PromQL through Grafana proxy
        confirms baseline. Two V2G-L2 patches landed: rules
        copy-not-symlink, webhook-sink user override removed. |
```

Daily-check steps 1-9 are now all "ok against the live local
compose stack" — V2G-K is officially in `local-compose-live`
mode.

Rollback for the compose stack:

```sh
cd ~/DEOPT/deopt-v2-backend/docs/monitoring/local-stack
sg docker -c 'docker compose down'       # stop, keep volumes
sg docker -c 'docker compose down -v'    # stop + wipe volumes
```

## Phase 8 — Validation

Backend (no Rust touched in V2G-L2):

| Command | Result |
|---|---|
| `cargo fmt --all --check` | ✅ |
| `cargo clippy --all-targets --all-features -- -D warnings` | ✅ |
| `cargo build --all-targets --all-features` | ✅ |
| `cargo test --all-targets --all-features --no-fail-fast` | ✅ **679 / 0 / 0** |

Monitoring + compose:

| Command | Result |
|---|---|
| `promtool check rules docs/monitoring/prometheus/v2_fee_alerts.bundle.yml` | ✅ 9 rules |
| `promtool check rules docs/monitoring/prometheus/v2_fee_alerts.stalled.yml` | ✅ 1 rule |
| `promtool test rules docs/monitoring/prometheus/v2_fee_alerts.test.yml` | ✅ SUCCESS |
| `amtool check-config docs/monitoring/alertmanager/v2_fee_routing.example.yml` | ✅ |
| `amtool check-config docs/monitoring/local-stack/alertmanager/alertmanager.yml` | ✅ |
| `promtool check rules docs/monitoring/local-stack/prometheus/rules/v2_fee_alerts.bundle.yml` | ✅ (now a literal file, not a symlink) |
| `promtool check rules docs/monitoring/local-stack/prometheus/rules/v2_fee_alerts.stalled.yml` | ✅ |
| `python3 -m json.tool docs/monitoring/grafana/v2_fee_observability_dashboard.json` | ✅ |
| `python3 -m json.tool docs/monitoring/local-stack/grafana/dashboards/v2_fee_observability_dashboard.json` | ✅ |
| `docker compose -f docs/monitoring/local-stack/compose.yml config -q` (via `sg docker`) | ✅ |
| `python3 -m py_compile docs/monitoring/local-stack/webhook-sink/webhook_sink.py` | ✅ |
| `docs/monitoring/local-stack/prometheus/rules/sync_from_canonical.sh` | ✅ idempotent |

Live verification (against compose stack):

| Endpoint | Result |
|---|---|
| `curl http://127.0.0.1:9090/api/v1/targets` | 3 jobs `up=1` |
| `curl http://127.0.0.1:9090/api/v1/rules`   | 9 alerts `state=inactive` |
| `curl http://127.0.0.1:9093/-/ready`        | OK |
| `curl http://127.0.0.1:3000/api/health`     | database=ok |
| `amtool alert add` × 5 + `--end` × 5        | 5 firing dispatched; resolve cycle started |

## Files changed (V2G-L2 delta)

- `docs/monitoring/local-stack/compose.yml` — dropped
  `user: "65534:65534"` from `webhook-sink` with explanatory comment.
- `docs/monitoring/local-stack/prometheus/rules/v2_fee_alerts.bundle.yml`
  — symlink → literal copy.
- `docs/monitoring/local-stack/prometheus/rules/v2_fee_alerts.stalled.yml`
  — symlink → literal copy.
- `docs/monitoring/local-stack/prometheus/rules/sync_from_canonical.sh`
  — **NEW** sync helper.
- `docs/V2_FEE_OBSERVABILITY_LOCAL_COMPOSE_LIVE_V2G_L2.md` — **NEW** (this doc).
- `docs/V2_FEE_OBSERVABILITY_7_DAY_SOAK_V2G_K.md` — V2G-L2 row + closure.
- `docs/monitoring/local-stack/README.md` — backend `HOST=0.0.0.0`
  correction + post-edit rule sync workflow.
- `docs/ALERTING_SPEC.md` / `docs/RUNBOOK_PERP_V2_FEE_ALERTS.md` — V2G-L2
  pointer + sync-helper reference.

## Remaining blockers

1. **24h soak still pending.** The V2G-K soak day-1 row starts T+24h
   after the compose stack came up at `2026-05-31T17:38Z`. Daily
   checklist steps 1-9 will be filled in row-by-row.
2. **`v2_fee_alerts.stalled.yml` stays disabled.** The compose
   Prometheus's TSDB has < 1h of history; the V2G-J activation gate
   (≥3 PERP rebates/24h × 48h sustained) cannot be evaluated yet.
   Re-evaluation marker stays on V2G-K soak day 5.
3. **V2G-J target-host cutover (F1-F11) still pending.** V2G-L2
   completes the local rehearsal; the operator can now confidently
   sign off on F1-F11 against the real target after 24h of clean
   soak.
4. **Backend `HOST=0.0.0.0` is local-dev only.** Production cutover
   should preserve `HOST=127.0.0.1` (or use mTLS-fronted reverse
   proxy) — Prometheus's compose-side scrape uses `host-gateway`
   precisely because the binding gap is by design.

## Next recommended milestone

**V2G-L3 / V2G-K day-1 — soak the compose stack for 24h.**

- Leave the compose stack and backend running.
- Re-run the V2G-K daily checklist at the same time of day for the
  next 7 days, filling each row in the soak table.
- Day 5: re-evaluate `v2_fee_alerts.stalled.yml` per the V2G-J
  activation policy.
- After 48h of clean operation, the operator can sign off on the
  V2G-J F1-F11 sequence against the real target stack.
- After 7 days, file the V2G-L3 closure note + advance to V2H
  multi-asset scope.
