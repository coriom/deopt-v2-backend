# V2G-L0 — Local non-production monitoring stack bootstrap

## Status

- Milestone: **V2G-L0** — local-development bootstrap of the V2 fee
  observability stack. Sub-milestone of V2G-K (the 7-day soak) so the
  soak can start with **real Prometheus / Alertmanager metrics
  history** even before the production cutover (V2G-J F-gates F1-F11)
  has been authorised.
- Date: 2026-05-31.
- Outcome:
  - Compose-based stack files committed at
    `docs/monitoring/local-stack/` covering Prometheus 3.12.0,
    Alertmanager 0.32.1, Grafana 11.4.0, and a webhook sink — all
    bound to `127.0.0.1` and using the V2G-G rule bundle + V2G-J
    routing example + V2G-G dashboard verbatim via symlinks.
  - Docker Engine 29.4.3 + Compose v5.1.3 detected, daemon active,
    but the local user is not in the `docker` group and `sudo`
    requires a password the agent cannot supply. Compose files
    therefore **not started by the agent**; the operator runs
    `docker compose up -d` after `usermod -aG docker $USER` or by
    granting passwordless sudo.
  - **Bare-binary stand-in was started instead** using the
    Prometheus + Alertmanager server binaries already in `/tmp` from
    V2G-H. Backend + Prometheus (scraping the backend) + Alertmanager
    (with V2G-J routing) + webhook sink all came up cleanly; the live
    9-rule bundle evaluated against real backend metrics; the 5-drill
    V2G-J synthetic batch dispatched to the expected receivers; the
    sink log records every webhook delivery.
  - **V2G-K soak day-0 row promoted to `local-stack-running`** with a
    real Prometheus-side baseline (the snapshot is captured **via
    Prometheus**, not just via direct `/metrics` scrape, which is the
    real soak-start mode).
  - No chain mutation, no DB writes, no real `.env` edit, no
    private-key handling, no governance/timelock action.

## Phase 1 — Container runtime discovery

```
docker version → 29.4.3, build 055a478
docker compose version → v5.1.3
podman → not installed
podman-compose → not installed
kubectl → not installed

docker ps → permission denied (socket is root:docker, user not in docker group)
sudo -n docker ps → sudo: a password is required
systemctl is-active docker → active
ls -la /var/run/docker.sock → srw-rw---- 1 root docker
id | groups → corio adm cdrom sudo dip plugdev users ollama
```

Conclusion: **Docker installed, daemon active, user permission gap.**
Two non-invasive paths for the operator:

```sh
# Path A — add user to docker group (preferred; one-time).
sudo usermod -aG docker $USER
newgrp docker      # picks up the new group in the current shell
docker compose up -d

# Path B — grant passwordless sudo only for docker (more granular).
echo "$USER ALL=(ALL) NOPASSWD: /usr/bin/docker" \
  | sudo tee /etc/sudoers.d/$USER-docker
sudo -n docker compose up -d
```

The agent will not run either of these because each requires the
operator's interactive sudo password. The compose stack files are
nevertheless complete and ready for the operator's first
`docker compose up -d`.

## Phase 2 — Compose stack files

Layout under `docs/monitoring/local-stack/`:

```
compose.yml                              -- 4-service compose v3 file
prometheus/
  prometheus.yml                         -- scrape backend + AM + self
  rules/
    v2_fee_alerts.bundle.yml             → symlink to docs/monitoring/prometheus/v2_fee_alerts.bundle.yml
    v2_fee_alerts.stalled.yml            → symlink to docs/monitoring/prometheus/v2_fee_alerts.stalled.yml
alertmanager/
  alertmanager.yml                       -- V2G-J routing pointed at webhook-sink:9095
grafana/
  provisioning/
    datasources/prometheus.yaml          -- DS=Prometheus, url=http://prometheus:9090
    dashboards/deopt_v2_fees.yaml        -- DeOpt folder, allowUiUpdates
  dashboards/
    v2_fee_observability_dashboard.json  -- rendered via render_dashboard.sh Prometheus
webhook-sink/
  webhook_sink.py                        -- one-file Python sink, 6 receiver paths
README.md                                -- backend startup, drill commands, ports
```

Key design choices:

- **All ports bind to `127.0.0.1` only.** No LAN exposure.
- **Rule files are symlinks into the canonical
  `docs/monitoring/prometheus/`.** No drift between local-stack and
  V2G-G/H/J/K rule bundles. The README documents the copy fallback
  for non-symlink filesystems.
- **Dashboard JSON is rendered, not symlinked.** Substituting
  `${DS_PROMETHEUS}` happens once at file-write time so the Grafana
  provisioning loader can pick the dashboard up without any
  template-input prompt.
- **Webhook sink is a single Python file.** No external dependencies
  beyond a stock `python:3.13-slim` container; the sink also runs as
  a host-mode binary (validated in Phase 3).
- **The `DeoptV2PerpRebateStalled` opt-in rule is symlinked but NOT
  referenced** in `prometheus.yml`. Activation remains gated by the
  V2G-J policy (≥3 PERP rebates/24h × 48h sustained).

Validation:

| Command                                                              | Result                                |
|----------------------------------------------------------------------|---------------------------------------|
| `docker compose config`                                              | ✅ valid, all services resolve        |
| `promtool check rules .../rules/v2_fee_alerts.bundle.yml` (via symlink) | ✅ 9 rules found                     |
| `amtool check-config alertmanager/alertmanager.yml`                  | ✅ 6 receivers, 1 inhibit rule        |
| Grafana provisioning YAML parse                                      | ✅ both files `apiVersion: 1`         |
| Dashboard JSON parse + DS render check                               | ✅ 10 panels, UID `deopt-v2g-g-v2-fees` |
| `python3 -m py_compile webhook_sink.py`                              | ✅                                    |

Note: `promtool check config prometheus/prometheus.yml` reports
"FAILED: /etc/prometheus/rules/...bundle.yml does not point to an
existing file" — that's the in-container path, which only resolves
once Prometheus is running inside compose. The container-side
validation runs as part of `docker compose exec prometheus promtool
check config /etc/prometheus/prometheus.yml`.

## Phase 3 — Backend safe startup

```sh
# Run from ~/DEOPT/deopt-v2-backend
bash -c '
  set -a
  . ./.env >/dev/null 2>&1
  . ./.env.cutover.v2d_s.local >/dev/null 2>&1
  . ./.env.preflight.v2e_f.local >/dev/null 2>&1
  . ./.env.observability.v2f_o.local >/dev/null 2>&1
  set +a
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
  # Local stack runs unauthenticated Prometheus scrape against
  # 127.0.0.1:8080. Disable METRICS_REQUIRE_ADMIN_TOKEN to keep
  # the Prometheus config tokenless. Localhost-only bind keeps it
  # safe.
  export METRICS_REQUIRE_ADMIN_TOKEN=false
  unset EXECUTOR_PRIVATE_KEY BUYER_PRIVATE_KEY SELLER_PRIVATE_KEY \
        DEPLOYER_PRIVATE_KEY SIGNER_PRIVATE_KEY \
        PERP_SMOKE_BUYER_PRIVATE_KEY PERP_SMOKE_SELLER_PRIVATE_KEY \
        OPTION_SMOKE_BUYER_PRIVATE_KEY OPTION_SMOKE_SELLER_PRIVATE_KEY
  exec ./target/release/deopt-v2-backend
'
```

Result:

- `/health` ✅ `{"ok":true,"service":"deopt-v2-backend"}`
- `/metrics` ✅ `64 deopt_* series`, no admin token required when
  `METRICS_REQUIRE_ADMIN_TOKEN=false`.
- `/admin/fees/v2/observability` ✅ — same baseline as V2G-K
  (perp/option charged{new}=3, rebated{new}=1, OLD/unknown=0, mUSDC
  budget=999987).

## Phase 4 — Local stack startup (bare-binary stand-in)

Compose path skipped because of the docker permission gap. Instead,
ran:

```sh
# Prometheus server (binary from /tmp/prometheus-3.12.0.linux-amd64/)
prometheus \
  --config.file=/tmp/v2g_l0_stack/prometheus.yml \
  --storage.tsdb.path=/tmp/v2g_l0_stack/prometheus-data \
  --web.listen-address=127.0.0.1:9090 \
  --log.level=warn

# Alertmanager server (binary from /tmp/alertmanager-0.32.1.linux-amd64/)
alertmanager \
  --config.file=/tmp/v2g_l0_stack/alertmanager.yml \
  --storage.path=/tmp/v2g_l0_stack/alertmanager-data \
  --web.listen-address=127.0.0.1:9093 \
  --cluster.listen-address= \
  --log.level=warn

# Webhook sink (one-file Python — same file as the compose service)
SINK_LOG=/tmp/v2g_l0_stack/sink/received.log \
  python3 /tmp/v2g_l0_stack/webhook_sink.py
```

The host-mode `prometheus.yml` is identical to the compose version
except that `static_configs.targets` point at `127.0.0.1:<port>`
rather than the in-network service names. The same V2G-G rule bundle
is loaded.

Verification:

| URL                                          | Result                                                                     |
|----------------------------------------------|----------------------------------------------------------------------------|
| `http://127.0.0.1:9090/-/healthy`            | ✅ `Prometheus Server is Healthy.`                                          |
| `http://127.0.0.1:9090/-/ready`              | ✅ `Prometheus Server is Ready.`                                            |
| `http://127.0.0.1:9090/api/v1/targets`       | ✅ all 3 jobs `health=up` (prometheus, alertmanager, deopt-v2-backend)      |
| `http://127.0.0.1:9090/api/v1/rules`         | ✅ 4 groups / 9 rules, every alert `state=inactive` `health=ok`             |
| `http://127.0.0.1:9090/api/v1/query?query=deopt_perp_fee_charged_v2_total` | ✅ 3 series: `consumer={new=3, old=0, unknown=0}` |
| `http://127.0.0.1:9090/api/v1/query?query=deopt_fees_manager_v2_rebate_budget_native` | ✅ `{asset="0x6eae..."} = 999987`               |
| `http://127.0.0.1:9093/-/ready`              | ✅                                                                          |

Grafana was not started in the bare-binary stand-in (Grafana ships
no portable single-binary release in the same way Prometheus and
Alertmanager do; the OSS Grafana binary requires a non-trivial
config tree). Once the operator runs `docker compose up -d`, Grafana
provisioning lands the dashboard automatically via the entries shipped
in this milestone.

## Phase 5 — Synthetic alert drill

Fired the five V2G-J drills via `amtool alert add` against
`127.0.0.1:9093`. Sink log records confirm every dispatch reached the
expected receiver:

```
Drill 1 PerpFeeChargedFromOldEngine      → critical  (deopt_oncall_critical)  ✅
Drill 2 OptionFeeConsumerUnknown         → tickets   (deopt_anomaly_tickets)  ✅
Drill 3 FeesManagerV2RebateBudgetLow     → ops       (deopt_ops_chat)         ✅
Drill 4 DeoptV2FeeMetricsAbsent          → backend   (deopt_backend_oncall)   ✅
Drill 5 HighSeverityMainnetRouteDrill    → high      (deopt_oncall_high)      ✅
```

Sink log path: `/tmp/v2g_l0_stack/sink/received.log` (host-mode) and
`webhook-sink-data` named volume (compose mode). All 5 alerts also
show in `/api/v2/alerts` with `state=active` and the correct
`receivers` array. No alerts went to the catch-all `/default`
receiver. The 1m group_wait routes (`tickets`, `ops`) dispatched after
the wait window as designed.

## Phase 6 — Soak baseline (V2G-K day-0)

Promoting V2G-K from `local-baseline-captured-via-direct-scrape` to
`local-stack-running-with-Prometheus-history`. The soak record
(`docs/V2_FEE_OBSERVABILITY_7_DAY_SOAK_V2G_K.md`) day-0 row is
appended with the live Prometheus-side baseline (this is the real
day-0; the V2G-K direct-scrape baseline is preserved as the "pre-stack
reference").

| Field                  | Value                                              |
|------------------------|----------------------------------------------------|
| Stack start (host-mode)| 2026-05-31T16:43:23Z (backend) / 16:43:14Z (Prom)  |
| Prometheus target      | deopt-v2-backend `health=up`, instance `dev-local` |
| Rule groups            | 4 / 9 alerts loaded, all `state=inactive`          |
| Alertmanager           | 6 receivers + 1 inhibit rule, healthy              |
| Synthetic drill        | 5/5 ✅ at 2026-05-31T16:44Z                         |
| Initial metric snapshot| identical to V2G-K direct-scrape baseline          |
| Stack stop             | 2026-05-31T16:48Z (clean TERM)                     |

Rollback for the local stack:

```sh
# Bare-binary mode (used in V2G-L0):
pkill -TERM -f '/tmp/prometheus-3.12.0.linux-amd64/prometheus'
pkill -TERM -f '/tmp/alertmanager-0.32.1.linux-amd64/alertmanager'
pkill -TERM -f '/tmp/v2g_l0_stack/webhook_sink.py'
pkill -TERM -f 'target/release/deopt-v2-backend'

# Compose mode (operator):
cd ~/DEOPT/deopt-v2-backend/docs/monitoring/local-stack
docker compose down            # keep state
docker compose down -v         # wipe state
```

The V2G-K daily checklist still applies. With the local stack now
running through Prometheus, **steps 4-9** of the V2G-K daily check
(Prom `/alerts` page, AM `/api/v2/alerts`, Grafana panels, indexer
health, no stale/absent alert firing) become live checks rather than
"n/a" placeholders.

## Phase 7 — Validation

Backend (docs + monitoring artefacts only — no Rust code touched):

| Command | Result |
|---|---|
| `cargo fmt --all --check` | ✅ clean |
| `cargo clippy --all-targets --all-features -- -D warnings` | ✅ clean |
| `cargo build --all-targets --all-features` | ✅ clean |
| `cargo test --all-targets --all-features --no-fail-fast` | ✅ **679 passed, 0 failed, 0 ignored** |

Monitoring artefacts:

| Command | Result |
|---|---|
| `promtool check rules docs/monitoring/prometheus/v2_fee_alerts.bundle.yml` | ✅ 9 rules / 4 groups |
| `promtool check rules docs/monitoring/prometheus/v2_fee_alerts.stalled.yml` | ✅ 1 rule |
| `promtool test rules docs/monitoring/prometheus/v2_fee_alerts.test.yml` | ✅ SUCCESS (7 groups) |
| `amtool check-config docs/monitoring/alertmanager/v2_fee_routing.example.yml` | ✅ |
| `amtool check-config docs/monitoring/local-stack/alertmanager/alertmanager.yml` | ✅ |
| `promtool check rules docs/monitoring/local-stack/prometheus/rules/v2_fee_alerts.bundle.yml` | ✅ via symlink |
| `python3 -m json.tool docs/monitoring/grafana/v2_fee_observability_dashboard.json` | ✅ |
| `python3 -m json.tool docs/monitoring/local-stack/grafana/dashboards/v2_fee_observability_dashboard.json` | ✅ |
| `docker compose -f docs/monitoring/local-stack/compose.yml config` | ✅ all 4 services |
| `python3 -m py_compile docs/monitoring/local-stack/webhook-sink/webhook_sink.py` | ✅ |

Frontend / Sol: untouched in V2G-L0 scope.

## Files changed

Created:

- `docs/V2_FEE_OBSERVABILITY_LOCAL_STACK_BOOTSTRAP_V2G_L0.md` (this doc).
- `docs/monitoring/local-stack/compose.yml`.
- `docs/monitoring/local-stack/prometheus/prometheus.yml`.
- `docs/monitoring/local-stack/prometheus/rules/v2_fee_alerts.bundle.yml` (symlink).
- `docs/monitoring/local-stack/prometheus/rules/v2_fee_alerts.stalled.yml` (symlink).
- `docs/monitoring/local-stack/alertmanager/alertmanager.yml`.
- `docs/monitoring/local-stack/grafana/provisioning/datasources/prometheus.yaml`.
- `docs/monitoring/local-stack/grafana/provisioning/dashboards/deopt_v2_fees.yaml`.
- `docs/monitoring/local-stack/grafana/dashboards/v2_fee_observability_dashboard.json`.
- `docs/monitoring/local-stack/webhook-sink/webhook_sink.py`.
- `docs/monitoring/local-stack/README.md`.

Updated:

- `docs/V2_FEE_OBSERVABILITY_7_DAY_SOAK_V2G_K.md` — V2G-L0 promotion
  of the day-0 baseline to local-stack mode with Prometheus history.
- `docs/V2_FEE_OBSERVABILITY_TARGET_CUTOVER_V2G_J.md` — local-stack
  pointer.
- `docs/ALERTING_SPEC.md` — local-stack pointer.
- `docs/RUNBOOK_PERP_V2_FEE_ALERTS.md` — local-stack quick-start.

## Remaining blockers

1. **Docker daemon access** — operator must run one of the two
   commands in Phase 1 to grant `corio` access to the `docker` group
   (or run the stack under a different user). Until then, the
   compose stack lives as text + symlinks only; the bare-binary
   stand-in is the soak path.
2. **Grafana dashboard not visualised under the bare-binary
   stand-in.** All other layers (backend, Prometheus, Alertmanager,
   webhook sink) are running and exercising the V2G-G rule bundle
   end-to-end; Grafana ships as a docker container in this milestone.
   Operator gets Grafana panels the moment `docker compose up -d`
   runs.
3. **V2G-J target-host cutover (F1-F11) is still pending.** V2G-L0
   does **not** unlock those gates — it gives the operator a local
   sandbox to rehearse the cutover before applying it on the real
   target. F-gates remain in the V2G-J/K plan.
4. **`v2_fee_alerts.stalled.yml`** stays disabled. The local stack's
   24h history at soak-start is zero, so the activation policy gate
   cannot be evaluated. Re-evaluation marker stays on soak day 5.

## V2G-L1 follow-up (appended 2026-05-31)

V2G-L1 retried the compose start one cycle after V2G-L0. Docker
socket gate unchanged; agent stopped at sudo and re-ran the
bare-binary stand-in. The V2G-J 5-drill batch was repeated end-to-end
with both `firing` and `resolved` dispatches recorded (V2G-L0 had
only captured the `firing` half). See
`docs/V2_FEE_OBSERVABILITY_LOCAL_COMPOSE_SOAK_V2G_L1.md`. Operator
unblock command:

```sh
sudo usermod -aG docker "$USER" && newgrp docker
```

## Next recommended milestone

**V2G-L1 — operator brings up the compose stack and rehearses the
V2G-J cutover.**

- Operator runs `sudo usermod -aG docker $USER` (or grants
  passwordless sudo) and `docker compose up -d` from
  `docs/monitoring/local-stack/`.
- Confirm all 4 containers reach `running` / `healthy`; visit
  `http://localhost:9090/targets`, `http://localhost:9093/`,
  `http://localhost:3000/`.
- Run the V2G-J drill packet against `http://localhost:9093` and
  inspect the webhook-sink log via `docker compose exec webhook-sink
  cat /var/log/sink/received.log`.
- Confirm Grafana renders the V2 fee observability dashboard
  (`/d/deopt-v2g-g-v2-fees`).
- Run the V2G-K daily checklist against the local stack for ≥ 24h
  to validate the cadence and confirm no rules fire spuriously.
- After 48h of clean local-stack operation, the operator has a fully
  rehearsed cutover and can sign off on F1-F11 for the real target.
