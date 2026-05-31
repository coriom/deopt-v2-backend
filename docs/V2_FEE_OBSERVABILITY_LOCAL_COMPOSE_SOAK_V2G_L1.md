# V2G-L1 — Local-compose soak attempt + bare-binary continuation

## Status

- Milestone: **V2G-L1** — second pass at the V2G-L0 local-stack
  bootstrap. Goal was to bring up the V2G-L0 docker-compose stack and
  start a real local soak. Docker daemon access is still blocked by
  the same socket permission gap V2G-L0 documented; the agent
  **stopped at the sudo gate** per the V2G-L1 hard rule, surfaced the
  exact operator command, and continued with the bare-binary
  stand-in to keep the soak baseline current.
- Date: 2026-05-31.
- Outcome:
  - **Docker compose `up -d` NOT executed** — `docker ps` returns
    "permission denied" and `sudo` requires a password the agent
    cannot supply. The hard rule "Do not run `sudo` commands without
    stopping for operator approval" applies. Operator command
    surfaced verbatim below.
  - **Bare-binary stand-in re-run end-to-end.** Backend + Prometheus
    (scraping the backend) + Alertmanager (with V2G-J routing) +
    webhook sink all came up; the V2G-G 9-rule bundle evaluated
    against real backend metrics; the V2G-J 5-drill batch dispatched
    + resolved cleanly through the sink; the live Prometheus-side
    baseline snapshot was captured for the soak record.
  - **V2G-K soak row promoted again** — day-0 line gains a new
    "V2G-L1 bare-binary re-run" tick at `2026-05-31T17:14Z` with
    both firing and resolved dispatches recorded.
  - **Grafana not exercised** — bare-binary path doesn't ship
    Grafana. Dashboard JSON + provisioning entries remain on disk and
    validated; operator unblocks Grafana the moment `docker compose
    up -d` runs.
- Hard gates respected: no broadcast, no chain mutation, no DB
  writes, no real `.env` edit, no private-key handling, no
  governance/timelock action, no sudo command executed by the agent.

## Phase 1 — Docker access gate (operator command)

Current state:

```
id   → uid=1000(corio) gid=1000(corio) groups=1000(corio),4(adm),24(cdrom),27(sudo),30(dip),46(plugdev),100(users),989(ollama)
ls -l /var/run/docker.sock → srw-rw---- 1 root docker 0
docker ps → permission denied while trying to connect to the docker API at unix:///var/run/docker.sock
docker compose version → Docker Compose version v5.1.3
```

The user `corio` is in the `sudo` group but **not** in the `docker`
group, so the daemon's `srw-rw----` permissions reject the connection.

### Operator must run exactly one of the following

**Preferred (one-time, group-based):**

```bash
sudo usermod -aG docker "$USER"
newgrp docker
# Verify in the same shell:
docker ps
```

If the verification still says `permission denied`, log out and back
in (or reboot WSL) so the new group is picked up by the login shell.

**Alternative (passwordless sudo only for docker):**

```bash
echo "$USER ALL=(ALL) NOPASSWD: /usr/bin/docker" \
  | sudo tee /etc/sudoers.d/$USER-docker
sudo -n docker ps
```

The agent did NOT auto-execute either path because both require an
interactive sudo password.

### After Docker access works

```bash
cd ~/DEOPT/deopt-v2-backend/docs/monitoring/local-stack
docker compose up -d
docker compose ps
```

Then jump to Phase 4 / Phase 5 of this doc — the verification
commands below are identical, only the URL targets stay the same
(127.0.0.1).

## Phase 2 — Backend safe runtime

Started with the same V2G-K shell-override stack used in V2G-L0:

```sh
cd ~/DEOPT/deopt-v2-backend
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
  export METRICS_REQUIRE_ADMIN_TOKEN=false
  unset EXECUTOR_PRIVATE_KEY BUYER_PRIVATE_KEY SELLER_PRIVATE_KEY \
        DEPLOYER_PRIVATE_KEY SIGNER_PRIVATE_KEY \
        PERP_SMOKE_BUYER_PRIVATE_KEY PERP_SMOKE_SELLER_PRIVATE_KEY \
        OPTION_SMOKE_BUYER_PRIVATE_KEY OPTION_SMOKE_SELLER_PRIVATE_KEY
  exec ./target/release/deopt-v2-backend
'
```

Result:

| Endpoint | Outcome |
|----------|---------|
| `/health`                        | ✅ `{"ok":true,"service":"deopt-v2-backend"}` |
| `/metrics`                       | ✅ 13 V2 fee gauge series; no admin token required |
| `/admin/fees/v2/observability`   | ✅ `milestone=V2G-G`, anomaly totals 0, 4 buckets `{new=3\|1, old=0, unknown=0}`, mUSDC budget 999987, all 5 contracts populated |

## Phase 3 — Compose stack startup

**Not executed** — Docker access gate. See Phase 1 for the operator
command. The compose files are validated and ready:

| Validation | Result |
|------------|--------|
| `docker compose -f docs/monitoring/local-stack/compose.yml config -q` | ✅ exit 0 |
| `promtool check rules docs/monitoring/local-stack/prometheus/rules/v2_fee_alerts.bundle.yml` (via symlink) | ✅ 9 rules |
| `amtool check-config docs/monitoring/local-stack/alertmanager/alertmanager.yml` | ✅ |
| `python3 -m json.tool docs/monitoring/local-stack/grafana/dashboards/v2_fee_observability_dashboard.json` | ✅ |
| `python3 -m py_compile docs/monitoring/local-stack/webhook-sink/webhook_sink.py` | ✅ |

### Bare-binary stand-in (V2G-L1 continuation)

To keep the V2G-K soak baseline current despite the Docker gate, the
agent re-ran the V2G-L0 bare-binary stand-in:

```sh
# Prometheus + Alertmanager binaries already extracted to /tmp in V2G-H.
/tmp/alertmanager-0.32.1.linux-amd64/alertmanager \
  --config.file=/tmp/v2g_l1_stack/alertmanager.yml \
  --storage.path=/tmp/v2g_l1_stack/alertmanager-data \
  --web.listen-address=127.0.0.1:9093 \
  --cluster.listen-address= --log.level=warn

/tmp/prometheus-3.12.0.linux-amd64/prometheus \
  --config.file=/tmp/v2g_l1_stack/prometheus.yml \
  --storage.tsdb.path=/tmp/v2g_l1_stack/prometheus-data \
  --web.listen-address=127.0.0.1:9090 --log.level=warn

SINK_LOG=/tmp/v2g_l1_stack/sink/received.log \
  python3 /tmp/v2g_l1_stack/webhook_sink.py
```

Verification — health and readiness:

```
http://127.0.0.1:9090/-/ready  → Prometheus Server is Ready.
http://127.0.0.1:9093/-/ready  → OK
http://127.0.0.1:9095/probe    → {"ok":true}
```

## Phase 4 — Prometheus live verification

Stand-in Prometheus (15s evaluation_interval, 5s scrape_interval):

```
/api/v1/targets:
  alertmanager      → up   http://127.0.0.1:9093/metrics
  deopt-v2-backend  → up   http://127.0.0.1:8080/metrics
  prometheus        → up   http://127.0.0.1:9090/metrics

/api/v1/rules:
  deopt_fees_manager_v2_budget_alerts   - 2 rules - all inactive
  deopt_option_v2_fee_alerts            - 3 rules - all inactive
  deopt_perp_v2_fee_alerts              - 3 rules - all inactive
  deopt_v2_fee_metrics_liveness         - 1 rule  - inactive
  TOTAL alerts loaded: 9
```

PromQL query baseline (via `/api/v1/query`):

```
deopt_perp_fee_charged_v2_total        consumer=new=3, old=0, unknown=0
deopt_perp_fee_rebated_v2_total        consumer=new=1, old=0, unknown=0
deopt_option_fee_charged_v2_total      consumer=new=3, old=0, unknown=0
deopt_option_fee_rebated_v2_total      consumer=new=1, old=0, unknown=0
deopt_fees_manager_v2_rebate_budget_native  asset=0x6eae...412e = 999987
up{job="deopt-v2-backend"}             = 1
up{job="prometheus"}                   = 1
up{job="alertmanager"}                 = 1
```

Saved to `/tmp/v2g_l1_stack/baseline/prometheus_baseline.txt`.

## Phase 5 — Alertmanager synthetic drill (live)

Fired the V2G-J 5-drill batch via `amtool alert add` against the live
local Alertmanager. Each alert dispatched to the expected receiver,
delivered to the webhook sink as a `firing` POST, then transitioned
to `resolved` after the `--end` push:

| # | Alertname                       | Expected receiver       | Sink path | Firing | Resolved |
|---|---------------------------------|-------------------------|-----------|--------|----------|
| 1 | PerpFeeChargedFromOldEngine     | deopt_oncall_critical   | /critical | ✅     | ✅       |
| 2 | OptionFeeConsumerUnknown        | deopt_anomaly_tickets   | /tickets  | ✅     | ✅       |
| 3 | FeesManagerV2RebateBudgetLow    | deopt_ops_chat          | /ops      | ✅     | ✅       |
| 4 | DeoptV2FeeMetricsAbsent         | deopt_backend_oncall    | /backend  | ✅     | ✅       |
| 5 | HighSeverityMainnetRouteDrill   | deopt_oncall_high       | /high     | ✅     | ✅       |

Sink log path (bare-binary): `/tmp/v2g_l1_stack/sink/received.log`.
Total dispatches at the end of the drill: **10** (5 firing + 5
resolved). No fallthrough to `/default`. Inhibit rule was not
exercised because the four target alerts carried distinct subsystem
labels (the V2G-J drill packet has a dedicated inhibit drill block
that V2G-I exercised separately).

## Phase 6 — Grafana verification

**Gated on Docker access.** No process listening on `127.0.0.1:3000`
during V2G-L1 because Grafana ships only as a container in the
`compose.yml`. The dashboard JSON, provisioning entries, and
datasource template all validate; once the operator runs `docker
compose up -d` after fixing the docker group, Grafana picks them up
on first start.

Expected after compose up:

| URL                                            | Outcome                                                       |
|------------------------------------------------|---------------------------------------------------------------|
| http://127.0.0.1:3000/api/health               | `{"database": "ok", "version": "11.4.0", ...}`                |
| http://127.0.0.1:3000/login                    | Grafana login page (admin / admin)                            |
| Sidebar → Dashboards → "DeOpt" folder          | Folder exists with `folderUid: deopt`                         |
| `DeOpt → DeOpt — V2 fee observability (V2G-G)` | 10 panels + 5 rows render against the Prometheus datasource   |
| Panel 6 (Base Sepolia mUSDC budget stat)       | Reads `999987` (above the 1000 alert threshold)               |
| Panel 7 (OLD consumer events PERP stat)        | `0`, green                                                    |
| Panel 8 (OLD consumer events OPTION stat)      | `0`, green                                                    |
| Panel 9 (Unknown consumer events stat)         | `0`, green                                                    |

The expected pattern is documented in
`docs/monitoring/grafana/v2_fee_observability_dashboard.spec.md`.

## Phase 7 — Local soak baseline (V2G-L1 row)

Appended to `docs/V2_FEE_OBSERVABILITY_7_DAY_SOAK_V2G_K.md` as a new
day-0 sub-row:

```
| 0++ | 2026-05-31T17:14Z | ok (bare-binary stack) | ok (via Prom) |
      ok (via Prom) | ok | ok | gated (no Grafana in bare-binary; docker
      compose blocked) | ok | ok | ok | deferred | V2G-L1: bare-binary
      Prometheus + Alertmanager + sink running on 127.0.0.1; backend
      scraped via 127.0.0.1:8080. 5/5 drills firing + 5/5 resolved.
      Docker `up -d` gated on `usermod -aG docker $USER`. |
```

Rollback:

```sh
# Bare-binary stand-in:
pkill -TERM -f '/tmp/prometheus-3.12.0.linux-amd64/prometheus'
pkill -TERM -f '/tmp/alertmanager-0.32.1.linux-amd64/alertmanager'
pkill -TERM -f '/tmp/v2g_l1_stack/webhook_sink.py'
pkill -TERM -f 'target/release/deopt-v2-backend'

# Compose stack (after operator unblocks Docker):
cd ~/DEOPT/deopt-v2-backend/docs/monitoring/local-stack
docker compose down            # keep volumes
docker compose down -v         # wipe state
```

## Phase 8 — Validation

Backend (no Rust code touched — docs/monitoring only):

| Command | Result |
|---|---|
| `cargo fmt --all --check` | ✅ |
| `cargo clippy --all-targets --all-features -- -D warnings` | ✅ |
| `cargo build --all-targets --all-features` | ✅ |
| `cargo test --all-targets --all-features --no-fail-fast` | ✅ **679 / 0 / 0** |

Monitoring:

| Command | Result |
|---|---|
| `promtool check rules docs/monitoring/prometheus/v2_fee_alerts.bundle.yml` | ✅ 9 rules / 4 groups |
| `promtool check rules docs/monitoring/prometheus/v2_fee_alerts.stalled.yml` | ✅ 1 rule |
| `promtool test rules docs/monitoring/prometheus/v2_fee_alerts.test.yml` | ✅ SUCCESS (7 groups) |
| `amtool check-config docs/monitoring/alertmanager/v2_fee_routing.example.yml` | ✅ |
| `amtool check-config docs/monitoring/local-stack/alertmanager/alertmanager.yml` | ✅ |
| `python3 -m json.tool docs/monitoring/grafana/v2_fee_observability_dashboard.json` | ✅ |
| `python3 -m json.tool docs/monitoring/local-stack/grafana/dashboards/v2_fee_observability_dashboard.json` | ✅ |
| `docker compose -f docs/monitoring/local-stack/compose.yml config -q` | ✅ exit 0 |
| `python3 -m py_compile docs/monitoring/local-stack/webhook-sink/webhook_sink.py` | ✅ |

Live verification:

| Command | Result |
|---|---|
| `curl http://127.0.0.1:8080/health`                             | ✅ |
| `curl http://127.0.0.1:9090/-/ready`                            | ✅ |
| `curl http://127.0.0.1:9093/-/ready`                            | ✅ |
| `curl http://127.0.0.1:9090/api/v1/targets`                     | 3 targets `up=1` |
| `curl http://127.0.0.1:9090/api/v1/rules`                       | 9 alerts `state=inactive` |
| `amtool alert add` × 5 + `--end` × 5                            | 5 firing + 5 resolved dispatches |

## Remaining blockers

1. **Docker daemon access.** Operator must run `sudo usermod -aG
   docker "$USER" && newgrp docker` (or grant passwordless sudo for
   `/usr/bin/docker`). Until then, `docker compose up -d` cannot
   start, and the Grafana UI cannot be exercised.
2. **Grafana not visualised under bare-binary stand-in.** Backend +
   Prometheus + Alertmanager + sink wire is proven end-to-end; the
   Grafana piece unlocks the moment Docker access is fixed.
3. **`v2_fee_alerts.stalled.yml`** stays disabled per the V2G-J
   activation policy. Local Prometheus history is still less than
   the 24h+48h gate window. Re-evaluation marker remains on V2G-K
   soak day 5.

## Next recommended milestone

**V2G-L2 — Docker access fixed; compose stack live; Grafana verified.**

- Operator runs `sudo usermod -aG docker "$USER" && newgrp docker`.
- Verify `docker ps` works.
- Run `cd ~/DEOPT/deopt-v2-backend/docs/monitoring/local-stack && docker compose up -d`.
- Confirm all 4 containers reach `running`/`healthy`.
- Re-run the V2G-J 5-drill batch against `http://127.0.0.1:9093` and
  the webhook-sink container's log
  (`docker compose exec webhook-sink cat /var/log/sink/received.log`).
- Open Grafana at `http://127.0.0.1:3000/d/deopt-v2g-g-v2-fees`,
  confirm all 10 panels render against the Prometheus datasource
  with the expected baseline values.
- Append the V2G-L2 row to the V2G-K soak record; the V2G-J cutover
  rehearsal is then ready for sign-off (24h of clean local
  operation).
