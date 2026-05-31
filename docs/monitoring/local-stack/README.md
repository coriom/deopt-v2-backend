# V2G-L0 — Local non-production monitoring stack

This directory boots a single-node Prometheus + Alertmanager + Grafana
+ webhook-sink on localhost, scraping the dev DeOpt backend running
on the host. It is the **local development version** of the
production-shaped V2G-G/H/J cutover.

NOT FOR PRODUCTION. No TLS, no auth on Prometheus/Alertmanager, plain
HTTP webhook sink. Stack is bound to `127.0.0.1` so no LAN exposure.

## Layout

```
docs/monitoring/local-stack/
├── compose.yml                    -- 4-service compose file
├── prometheus/
│   ├── prometheus.yml             -- scrape config (backend + AM + self)
│   └── rules/                     -- symlinks into ../../prometheus/
│       ├── v2_fee_alerts.bundle.yml      → docs/monitoring/prometheus/v2_fee_alerts.bundle.yml
│       └── v2_fee_alerts.stalled.yml     → docs/monitoring/prometheus/v2_fee_alerts.stalled.yml
│           (loaded only when uncommented in prometheus.yml)
├── alertmanager/
│   └── alertmanager.yml           -- V2G-G routing example pointed at webhook-sink
├── grafana/
│   ├── provisioning/
│   │   ├── datasources/prometheus.yaml
│   │   └── dashboards/deopt_v2_fees.yaml
│   └── dashboards/
│       └── v2_fee_observability_dashboard.json   -- rendered with DS=Prometheus
└── webhook-sink/
    └── webhook_sink.py            -- one-file Python HTTP sink
```

The Prometheus rule files are **symlinks** into
`docs/monitoring/prometheus/` so the local stack always uses the same
rule definitions as the canonical V2G-G artefacts. If the symlinks
do not survive your filesystem (e.g. when copying the directory onto
a Windows host), replace them with literal copies and re-sync after
every rule-bundle change.

## Prerequisites

- Docker Engine ≥ 20.10 (or Docker Desktop / Podman ≥ 4.0).
- Compose v2 (`docker compose version` reports ≥ 2.0).
- Local user has access to the Docker socket. On Ubuntu/Debian:
  `sudo usermod -aG docker $USER && newgrp docker`.
  (Rootless Docker also works.)
- Ports `9090`, `9093`, `3000`, `9095` free on `127.0.0.1`.
- DeOpt backend running on the **host** (not inside compose) at
  `0.0.0.0:8080`. See "Backend startup" below.

## Backend startup (host, read-only V2G-K override stack)

Run the dev backend with broadcast disabled, no private keys, and the
V2G-G observability classifier env vars supplied via the shell so the
real `.env` stays untouched:

```sh
cd ~/DEOPT/deopt-v2-backend
bash -c '
  set -a
  . ./.env >/dev/null 2>&1
  . ./.env.cutover.v2d_s.local >/dev/null 2>&1
  . ./.env.preflight.v2e_f.local >/dev/null 2>&1
  . ./.env.observability.v2f_o.local >/dev/null 2>&1
  set +a
  # V2G-K classifier overrides (NEW everywhere; OLD = observability-only)
  export PERP_ENGINE_ADDRESS=0xc6C592100723Fe0C66343A16e95eC34cC0c2141c
  export OLD_PERP_ENGINE_ADDRESS=0xB36395b67D0798ADA981731c9Fa5239F4362b53B
  export MARGIN_ENGINE=0x287Cef479be5889eEfCa847F9e73C860898f48Cc
  export OLD_MARGIN_ENGINE_ADDRESS=0x6C5665De05e7314cB63cD77F82DFa86508A5b5F8
  export FEES_MANAGER_V2=0x00dA0B9876bcBf0c79CB5BcAcfEBAFb8C7Ad774f
  export OPTION_EVENT_INDEXER_MARGIN_ENGINE_ADDRESS=0x287Cef479be5889eEfCa847F9e73C860898f48Cc
  export OPTION_EVENT_INDEXER_FEES_MANAGER_V2_ADDRESS=0x00dA0B9876bcBf0c79CB5BcAcfEBAFb8C7Ad774f
  # hard-disable every broadcast surface
  export EXECUTION_ENABLED=false
  export EXECUTOR_DRY_RUN=true
  export EXECUTOR_REAL_BROADCAST_ENABLED=false
  export OPTION_EXECUTION_BROADCAST_ENABLED=false
  unset EXECUTOR_PRIVATE_KEY BUYER_PRIVATE_KEY SELLER_PRIVATE_KEY \
        DEPLOYER_PRIVATE_KEY SIGNER_PRIVATE_KEY \
        PERP_SMOKE_BUYER_PRIVATE_KEY PERP_SMOKE_SELLER_PRIVATE_KEY \
        OPTION_SMOKE_BUYER_PRIVATE_KEY OPTION_SMOKE_SELLER_PRIVATE_KEY
  # Bind to all interfaces so the compose-side scrape can reach in via
  # host-gateway. The backend reads HOST + PORT env vars (the
  # HTTP_BIND_ADDRESS form shipped in V2G-L0/L1 was wrong — fixed in
  # V2G-L2). If the backend is already bound to 0.0.0.0 by default
  # this is a no-op.
  export HOST=${HOST:-0.0.0.0}
  export PORT=${PORT:-8080}
  exec ./target/release/deopt-v2-backend
'
```

Confirm:

```sh
curl -sf http://127.0.0.1:8080/health
# → {"ok":true,"service":"deopt-v2-backend"}
curl -sH "x-admin-token: $ADMIN_API_TOKEN" \
  http://127.0.0.1:8080/admin/fees/v2/observability | jq '.anomaly_totals'
# → {"old_consumer_events": 0, "unknown_consumer_events": 0}
```

## Start the stack

```sh
cd ~/DEOPT/deopt-v2-backend/docs/monitoring/local-stack
docker compose up -d
```

Wait ~30 seconds for healthchecks. Then:

| URL                                  | Service        |
|--------------------------------------|----------------|
| http://localhost:9090/               | Prometheus     |
| http://localhost:9090/targets        | check `deopt-v2-backend` job is `UP` |
| http://localhost:9090/rules          | confirm 4 deopt rule groups + 9 rules |
| http://localhost:9090/alerts         | every V2 alert `state=inactive` |
| http://localhost:9093/               | Alertmanager   |
| http://localhost:9093/#/alerts       | empty unless drill is running |
| http://localhost:3000/               | Grafana (admin / admin) |
| http://localhost:3000/d/deopt-v2g-g-v2-fees | V2 fee dashboard |

If the Prometheus target shows `DOWN`:

- Confirm backend is bound on `0.0.0.0:8080` (not `127.0.0.1:8080`).
- Confirm `host.docker.internal` resolves inside the container:
  `docker compose exec prometheus getent hosts host.docker.internal`.
- On WSL2 with Docker Desktop, `host.docker.internal` works
  out-of-the-box. On vanilla Linux Docker Engine, the
  `extra_hosts: - "host.docker.internal:host-gateway"` mapping in
  `compose.yml` handles this.

## Synthetic drill (V2G-J/K shape)

```sh
# Operator workstation (or any host that can reach 127.0.0.1:9093).
AMTOOL=/tmp/alertmanager-0.32.1.linux-amd64/amtool   # or your install
AM_URL=http://127.0.0.1:9093
NOW=$(date -u +%FT%TZ)

# 1. PERP OLD engine fee → deopt_oncall_critical
$AMTOOL --alertmanager.url=$AM_URL alert add --start="$NOW" \
  alertname=PerpFeeChargedFromOldEngine \
  subsystem=perp_v2_fees severity=high consumer=old milestone=V2F-O

# 2. OPTION unknown consumer → deopt_anomaly_tickets
$AMTOOL --alertmanager.url=$AM_URL alert add --start="$NOW" \
  alertname=OptionFeeConsumerUnknown \
  subsystem=option_v2_fees severity=medium consumer=unknown milestone=V2G-F

# 3. rebate budget low → deopt_ops_chat
$AMTOOL --alertmanager.url=$AM_URL alert add --start="$NOW" \
  alertname=FeesManagerV2RebateBudgetLow subsystem=fees_manager_v2 \
  severity=medium asset_symbol=musdc \
  asset=0x6eae407f5640b006fac9965182e238582a3b412e milestone=V2G-F

# 4. metric pipeline absent → deopt_backend_oncall
$AMTOOL --alertmanager.url=$AM_URL alert add --start="$NOW" \
  alertname=DeoptV2FeeMetricsAbsent subsystem=v2_fee_observability \
  severity=high consumer=new milestone=V2G-G

# 5. mainnet severity escalation → deopt_oncall_high
$AMTOOL --alertmanager.url=$AM_URL alert add --start="$NOW" \
  alertname=HighSeverityMainnetRouteDrill \
  subsystem=v2_fee_observability severity=medium chain=base-mainnet \
  milestone=V2G-J

# Wait for the 1m group_wait routes, then inspect:
sleep 75
docker compose exec webhook-sink cat /var/log/sink/received.log \
  | python3 -c "
import json, sys, collections
c = collections.Counter()
for line in sys.stdin:
    e = json.loads(line)
    for a in e['alerts']:
        c[(e['receiver_path'], a['labels']['alertname'], a['status'])] += 1
for k, v in sorted(c.items()): print(k, v)
"
```

Expected receiver matrix:

| Path       | Alertname                          |
|------------|-------------------------------------|
| critical   | PerpFeeChargedFromOldEngine         |
| tickets    | OptionFeeConsumerUnknown            |
| ops        | FeesManagerV2RebateBudgetLow        |
| backend    | DeoptV2FeeMetricsAbsent             |
| high       | HighSeverityMainnetRouteDrill       |

## Stop / restart / clean

```sh
docker compose stop          # pause services, keep state
docker compose start         # resume
docker compose down          # stop + remove containers, keep volumes
docker compose down -v       # stop + remove containers + wipe all volumes (full reset)
```

## Sync rules / dashboard from canonical source

V2G-L0 originally shipped the Prometheus rule files as symlinks into
`docs/monitoring/prometheus/`. Docker bind-mounts cannot follow
symlinks whose target lives outside the bound directory, so the
Prometheus container failed to load them. **V2G-L2 replaced the
symlinks with literal copies** + shipped
`prometheus/rules/sync_from_canonical.sh` so each canonical edit is
one shell command away from being mirrored.

After editing `docs/monitoring/prometheus/v2_fee_alerts.bundle.yml`
or `.stalled.yml`:

```sh
cd ~/DEOPT/deopt-v2-backend
./docs/monitoring/local-stack/prometheus/rules/sync_from_canonical.sh

# graceful prometheus reload inside the compose stack:
docker compose -f docs/monitoring/local-stack/compose.yml exec \
  prometheus wget -qO- http://127.0.0.1:9090/-/reload >/dev/null \
  && echo prometheus reloaded
```

For the Grafana dashboard:

```sh
docs/monitoring/grafana/provisioning/render_dashboard.sh Prometheus \
  > docs/monitoring/local-stack/grafana/dashboards/v2_fee_observability_dashboard.json
docker compose -f docs/monitoring/local-stack/compose.yml restart grafana
```

## Hard refusals

- No private keys in this stack.
- No real `.env` mutation — the backend reads classifier env vars via
  the shell-only override pattern shown above.
- No chain RPC writes; backend has `EXECUTION_ENABLED=false`.
- All ports bind to `127.0.0.1` — never expose this stack on a LAN.
- The webhook-sink logs synthetic dispatches; do not point it at any
  real receiver URL.

## V2G band cross-references

- Cutover record: `docs/V2_FEE_OBSERVABILITY_TARGET_CUTOVER_V2G_J.md`.
- Soak record: `docs/V2_FEE_OBSERVABILITY_7_DAY_SOAK_V2G_K.md`.
- Bootstrap record: `docs/V2_FEE_OBSERVABILITY_LOCAL_STACK_BOOTSTRAP_V2G_L0.md`.
- Runbook: `docs/RUNBOOK_PERP_V2_FEE_ALERTS.md`.
