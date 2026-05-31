# V2G-H — Wire V2 Fee Observability Into The Live Operator Stack

## Status

- Milestone: **V2G-H** — operator-infra wiring milestone. V2G-G shipped
  the production observability artifacts (Prometheus rules, Grafana
  dashboard, Alertmanager routing example, backend admin endpoint,
  frontend admin section, operator `.env` patch template, mainnet
  readiness matrix). V2G-H validates those artifacts with the real
  Prometheus / Alertmanager toolchains, prepares operator-only wiring
  commands, and re-runs the live read-only verification under the
  same hard-gate stack.
- Date: 2026-05-31.
- Outcome:
  - Local monitoring stack confirmed **absent** (no Prometheus /
    Alertmanager / Grafana installed, no docker containers, no
    systemd units). V2G-H therefore prepares the integration package
    rather than wiring it into a running stack.
  - `promtool` (Prometheus v3.12.0) extracted to
    `/tmp/prometheus-3.12.0.linux-amd64/promtool`; `amtool`
    (Alertmanager v0.32.1) extracted to
    `/tmp/alertmanager-0.32.1.linux-amd64/amtool`. Both are scratch
    installs that operators can replicate (commands embedded below).
  - `promtool check rules` validates the V2G-G rule bundle + the
    legacy per-product files; new `promtool test rules` spec at
    `docs/monitoring/prometheus/v2_fee_alerts.test.yml` exercises
    all 5 scenarios and reports SUCCESS.
  - `amtool check-config` validates the Alertmanager routing
    example; `amtool config routes test` resolves four sample alerts
    to the correct receivers (one expected `continue: true`
    double-resolution documented).
  - Grafana dashboard provisioning entry, Prometheus datasource
    example, and a small `render_dashboard.sh` substitution helper
    added under `docs/monitoring/grafana/provisioning/`.
  - Backend rebuilt + re-run in read-only mode; `/health`,
    `/admin/fees/v2/observability`, `/admin/fees/onchain` (both
    V2G-E txs), and `/metrics` all reproduce the V2G-G / V2G-F
    closure state byte-for-byte. All eight V2 alerts evaluate
    **green** against the live scrape.
  - Operator `.env` patch left documented but unapplied — real `.env`
    untouched per hard rules.
- Hard gates respected: no broadcast, no on-chain mutation, no DB
  rows touched, no private-key handling, no real-secret `.env`
  edits, no governance/timelock actions.

## Source-of-truth artefacts

| Artefact                                | Path                                                                                  |
|-----------------------------------------|---------------------------------------------------------------------------------------|
| Prometheus rule bundle                  | `docs/monitoring/prometheus/v2_fee_alerts.bundle.yml`                                 |
| Prometheus unit-test spec (V2G-H new)   | `docs/monitoring/prometheus/v2_fee_alerts.test.yml`                                   |
| Alertmanager routing example            | `docs/monitoring/alertmanager/v2_fee_routing.example.yml`                             |
| Grafana dashboard JSON                  | `docs/monitoring/grafana/v2_fee_observability_dashboard.json`                         |
| Grafana dashboard spec (md)             | `docs/monitoring/grafana/v2_fee_observability_dashboard.spec.md`                      |
| Grafana provisioning entry (V2G-H new)  | `docs/monitoring/grafana/provisioning/dashboards/deopt_v2_fees.yaml`                  |
| Grafana datasource template (V2G-H new) | `docs/monitoring/grafana/provisioning/datasources/prometheus_example.yaml`            |
| Dashboard renderer (V2G-H new)          | `docs/monitoring/grafana/provisioning/render_dashboard.sh`                            |
| Operator `.env` patch (unchanged)       | `docs/operator/v2g_g_env_patch.example.env`                                           |
| Backend admin endpoint                  | `GET /admin/fees/v2/observability` (`src/api/routes.rs`, `src/fees/v2_observability.rs`) |
| Runbook                                 | `docs/RUNBOOK_PERP_V2_FEE_ALERTS.md`                                                  |
| Alerting spec                           | `docs/ALERTING_SPEC.md`                                                               |

## Phase 1 — Monitoring stack discovery

Probed the workspace + host system from `~/DEOPT`:

```
find . -maxdepth 4 -type d \
  \( -iname "*prometheus*" -o -iname "*alertmanager*" \
     -o -iname "*grafana*" -o -iname "*monitoring*" \)
   → only docs/monitoring (V2G-G committed artifacts), no installed stack
find . -maxdepth 4 -type f \
  \( -iname "*prometheus*" -o -iname "*alertmanager*" \
     -o -iname "*grafana*" -o -iname "docker-compose*.yml" \)
   → no docker-compose, no rendered config files

which prometheus alertmanager grafana grafana-cli promtool amtool
   → all missing in PATH
docker ps      → empty (daemon present but no running containers)
docker images  → no prom/grafana/alert images
systemctl list-units --type=service | grep -iE "prom|grafana|alert"
   → no units

ls /etc/prometheus/ /etc/alertmanager/ /etc/grafana/ /var/lib/grafana/
   → all "No such file or directory"
```

Conclusion: **the monitoring stack is absent on this host.** V2G-H
therefore (a) prepares the integration package rather than rolling it
into a running stack, and (b) validates the artifacts with scratch
toolchain installs that operators can replicate.

## Phase 2 — Operator `.env` patch (manual gate)

Reread `docs/operator/v2g_g_env_patch.example.env` and diffed the
patch against the current real `.env` (read-only — file untouched):

```
Patch lines (from docs/operator/v2g_g_env_patch.example.env):

  PERP_ENGINE_ADDRESS=0xc6C592100723Fe0C66343A16e95eC34cC0c2141c
  OLD_PERP_ENGINE_ADDRESS=0xB36395b67D0798ADA981731c9Fa5239F4362b53B
  MARGIN_ENGINE=0x287Cef479be5889eEfCa847F9e73C860898f48Cc
  OLD_MARGIN_ENGINE_ADDRESS=0x6C5665De05e7314cB63cD77F82DFa86508A5b5F8
  FEES_MANAGER_V2=0x00dA0B9876bcBf0c79CB5BcAcfEBAFb8C7Ad774f

Current `.env` (V2F-O carry-over, observed via `grep -nE '^(...) ='`):

  Line 50: PERP_ENGINE_ADDRESS=0xB36395b67D0798ADA981731c9Fa5239F4362b53B   # OLD — needs flip to NEW
  (OLD_PERP_ENGINE_ADDRESS, MARGIN_ENGINE, OLD_MARGIN_ENGINE_ADDRESS,
   FEES_MANAGER_V2 all missing — need to be added.)
```

**Operator gate.** The agent does NOT edit `.env`. The exact change
the operator should apply:

```diff
-PERP_ENGINE_ADDRESS=0xB36395b67D0798ADA981731c9Fa5239F4362b53B
+PERP_ENGINE_ADDRESS=0xc6C592100723Fe0C66343A16e95eC34cC0c2141c
+OLD_PERP_ENGINE_ADDRESS=0xB36395b67D0798ADA981731c9Fa5239F4362b53B
+MARGIN_ENGINE=0x287Cef479be5889eEfCa847F9e73C860898f48Cc
+OLD_MARGIN_ENGINE_ADDRESS=0x6C5665De05e7314cB63cD77F82DFa86508A5b5F8
+FEES_MANAGER_V2=0x00dA0B9876bcBf0c79CB5BcAcfEBAFb8C7Ad774f
```

After applying, run the verification block from
`docs/operator/v2g_g_env_patch.example.env` (env grep, admin probes,
`/metrics` grep, on-chain `cast call` cross-check).

**Rollback snippet.** If the patch needs to be reverted, revert to the
state recorded above (single `PERP_ENGINE_ADDRESS=0xB36395b67D…b53B`
line, no other entries). Restart the backend; the metric pipeline
will route all PERP V2 events through the `consumer="unknown"` bucket
again (the documented V2F-O behaviour). No data loss either way —
the classifier reads env at boot, not from a persistent store.

Current shell-only override pattern remains a valid fallback (used
in this V2G-H read-only verification — see Phase 6).

## Phase 3 — Prometheus rule integration

### Toolchain install (operator-replicable, scratch-only)

`promtool` is not in this host's default toolchain. Installed locally
without changing system state:

```sh
# V2G-H scratch promtool install (does NOT touch system paths).
cd /tmp
curl -sL --max-time 60 -o prom.tar.gz \
  https://github.com/prometheus/prometheus/releases/download/v3.12.0/prometheus-3.12.0.linux-amd64.tar.gz
tar -xzf prom.tar.gz prometheus-3.12.0.linux-amd64/promtool
PROMTOOL=/tmp/prometheus-3.12.0.linux-amd64/promtool
$PROMTOOL --version
# → promtool, version 3.12.0
```

### `promtool check rules`

```
$ $PROMTOOL check rules docs/monitoring/prometheus/v2_fee_alerts.bundle.yml
Checking docs/monitoring/prometheus/v2_fee_alerts.bundle.yml
  SUCCESS: 9 rules found

$ $PROMTOOL check rules docs/alertmanager/perp_v2_fee_alerts.yml
Checking docs/alertmanager/perp_v2_fee_alerts.yml
  SUCCESS: 3 rules found

$ $PROMTOOL check rules docs/alertmanager/option_v2_fee_alerts.yml
Checking docs/alertmanager/option_v2_fee_alerts.yml
  SUCCESS: 4 rules found
```

### `promtool test rules` (V2G-H new)

New file: `docs/monitoring/prometheus/v2_fee_alerts.test.yml`. Covers
five scenarios:

1. **Green baseline** — V2G-E live state (PERP charged{new}=3,
   rebated{new}=1, OPTION charged{new}=3, rebated{new}=1, budget =
   999987, OLD/unknown=0). No alert fires.
2. **PERP OLD-engine regression** — `PerpFeeChargedFromOldEngine` +
   `PerpFeeRebatedFromOldEngine` fire with the expected
   high/perp_v2_fees labels + annotations + runbook URL. OPTION +
   budget alerts stay green.
3. **OPTION unknown-consumer regression** — `OptionFeeConsumerUnknown`
   fires with the expected medium/option_v2_fees labels + the
   `consumer="unknown"` selector label. PERP unknown stays green.
4. **Rebate budget low** — `FeesManagerV2RebateBudgetLow` fires with
   the expected medium/fees_manager_v2 labels + `asset_symbol=musdc`
   + asset selector.
5. **Metrics absent** — `DeoptV2FeeMetricsAbsent` fires after 5m
   absence of the V2 fee gauges; PERP/OPTION/budget alerts stay
   silent (they cannot evaluate while their gauges are absent).

Run:

```
$ $PROMTOOL test rules docs/monitoring/prometheus/v2_fee_alerts.test.yml
  SUCCESS
```

### Integration commands (operator-facing)

The destination directory varies per deployment. Common shapes:

```sh
# 1a) Standalone Prometheus binary on a VM (config at /etc/prometheus/):
sudo install -d -m 0755 /etc/prometheus/rules.d
sudo install -m 0644 \
  ~/DEOPT/deopt-v2-backend/docs/monitoring/prometheus/v2_fee_alerts.bundle.yml \
  /etc/prometheus/rules.d/v2_fee_alerts.bundle.yml

# Edit /etc/prometheus/prometheus.yml to reference the new rule file:
#   rule_files:
#     - /etc/prometheus/rules.d/v2_fee_alerts.bundle.yml

# Validate the merged config before reload:
sudo promtool check config /etc/prometheus/prometheus.yml

# Graceful reload (no service restart):
curl -X POST http://127.0.0.1:9090/-/reload
# or: sudo systemctl reload prometheus

# 1b) Containerised stack (docker-compose / podman-compose):
cp ~/DEOPT/deopt-v2-backend/docs/monitoring/prometheus/v2_fee_alerts.bundle.yml \
   ./prometheus/rules/v2_fee_alerts.bundle.yml
# Then re-run `docker compose up -d` (or equivalent).

# 1c) Kubernetes (Prometheus Operator):
# Wrap the rule file in a PrometheusRule CRD; the bundle's group
# names map 1:1 to PrometheusRule `groups[*].name` entries.
```

After the reload, confirm:

- `/api/v1/rules` lists the four V2G-G rule groups
  (`deopt_perp_v2_fee_alerts`, `deopt_option_v2_fee_alerts`,
  `deopt_fees_manager_v2_budget_alerts`,
  `deopt_v2_fee_metrics_liveness`).
- `/alerts` shows every V2 alert with `state=inactive` (Base Sepolia
  post-V2G-E expects all green).

Rollback: remove the rule file from `rule_files`, re-reload.

## Phase 4 — Alertmanager routing integration

### Toolchain install (operator-replicable)

```sh
cd /tmp
curl -sL --max-time 60 -o am.tar.gz \
  https://github.com/prometheus/alertmanager/releases/download/v0.32.1/alertmanager-0.32.1.linux-amd64.tar.gz
tar -xzf am.tar.gz alertmanager-0.32.1.linux-amd64/amtool
AMTOOL=/tmp/alertmanager-0.32.1.linux-amd64/amtool
$AMTOOL --version
# → amtool, version 0.32.1
```

### `amtool check-config`

```
$ $AMTOOL check-config docs/monitoring/alertmanager/v2_fee_routing.example.yml
Checking 'docs/monitoring/alertmanager/v2_fee_routing.example.yml'  SUCCESS
Found:
 - global config
 - route
 - 1 inhibit rules
 - 6 receivers
 - 0 templates
```

### `amtool config routes test` — sample alert resolution

```
$ $AMTOOL config routes test \
    --config.file=docs/monitoring/alertmanager/v2_fee_routing.example.yml \
    subsystem=perp_v2_fees severity=high \
    alertname=PerpFeeChargedFromOldEngine
deopt_oncall_critical

$ $AMTOOL config routes test … alertname=OptionFeeConsumerUnknown \
    subsystem=option_v2_fees severity=medium
deopt_anomaly_tickets

$ $AMTOOL config routes test … alertname=FeesManagerV2RebateBudgetLow \
    subsystem=fees_manager_v2 severity=medium
deopt_ops_chat

$ $AMTOOL config routes test … alertname=DeoptV2FeeMetricsAbsent \
    subsystem=v2_fee_observability severity=high
deopt_backend_oncall

$ $AMTOOL config routes test … alertname=PerpFeeChargedFromOldEngine \
    subsystem=perp_v2_fees severity=high chain=base-mainnet
deopt_oncall_critical,deopt_oncall_critical
WARNING: Expected receivers did not match resolved receivers.
```

Test 5 is the documented `continue: true` behavior: the mainnet
severity-escalation route and the subsystem route both resolve to
`deopt_oncall_critical`. In a production deployment the two routes
should point at **distinct** receivers (e.g. a dedicated mainnet
critical channel that the contract on-call subscribes to). The
example config keeps them the same for documentation simplicity.

### Integration commands (operator-facing)

The example file is a **fragment**, not a full Alertmanager config.
Merge it into the deployed `alertmanager.yml` with care:

```sh
# 1) Patch the deployed Alertmanager config (manual diff/merge — the
#    fragment has its own route + receivers + inhibit_rules subtrees).
# 2) Validate the merged config:
sudo amtool check-config /etc/alertmanager/alertmanager.yml

# 3) Graceful reload:
curl -X POST http://127.0.0.1:9093/-/reload
# or: sudo systemctl reload alertmanager
```

After the reload, send a synthetic alert through `amtool alert add`
to confirm each receiver receives traffic:

```sh
amtool alert add --alertmanager.url http://127.0.0.1:9093 \
  --start "$(date -u +%FT%TZ)" \
  alertname=PerpFeeChargedFromOldEngine \
  subsystem=perp_v2_fees severity=high consumer=old
```

Rollback: revert the merge in `alertmanager.yml`, re-reload.

## Phase 5 — Grafana dashboard integration

### Artefacts

- `docs/monitoring/grafana/v2_fee_observability_dashboard.json` (V2G-G).
- `docs/monitoring/grafana/v2_fee_observability_dashboard.spec.md`
  (V2G-G — human-readable spec).
- **V2G-H new**:
  `docs/monitoring/grafana/provisioning/dashboards/deopt_v2_fees.yaml`
  (Grafana dashboard provider entry, `apiVersion: 1`).
- **V2G-H new**:
  `docs/monitoring/grafana/provisioning/datasources/prometheus_example.yaml`
  (optional Prometheus datasource entry, only needed when no
  Prometheus datasource exists yet).
- **V2G-H new**:
  `docs/monitoring/grafana/provisioning/render_dashboard.sh`
  (substitutes `${DS_PROMETHEUS}` with the actual datasource name
  for provisioned dashboards — Grafana's provisioning loader does
  NOT substitute template inputs).

### Validation

```sh
# Render the dashboard against a "Prometheus" datasource name and
# parse the result as JSON to confirm the substitution is clean:
$ docs/monitoring/grafana/provisioning/render_dashboard.sh Prometheus \
  | python3 -c "
import json, sys
d = json.load(sys.stdin)
print('panels:', len([p for p in d['panels'] if p['type'] != 'row']))
print('uid:', d['uid'])
print('targets datasource UIDs:', {
    t['datasource']['uid']
    for p in d['panels'] if p['type'] != 'row'
    for t in p.get('targets', [])
})
"
panels: 10
uid: deopt-v2g-g-v2-fees
targets datasource UIDs: {'Prometheus'}
```

Provisioning YAMLs structurally validated (both load as
`apiVersion: 1`).

### Integration commands (operator-facing)

```sh
# 1) Render the dashboard for the deployed Prometheus datasource name.
#    Common values: "Prometheus", "DS_PROMETHEUS", "default".
docs/monitoring/grafana/provisioning/render_dashboard.sh "Prometheus" \
  > /tmp/v2_fee_observability_dashboard.rendered.json

# 2a) UI import (simplest):
#     Grafana → Dashboards → Import →
#       upload /tmp/v2_fee_observability_dashboard.rendered.json
#     Pick the Prometheus datasource when prompted.

# 2b) Provisioning (preferred for stable rollouts):
sudo install -m 0644 \
  docs/monitoring/grafana/provisioning/dashboards/deopt_v2_fees.yaml \
  /etc/grafana/provisioning/dashboards/deopt_v2_fees.yaml
sudo install -d -m 0755 /var/lib/grafana/dashboards/deopt
sudo install -m 0644 /tmp/v2_fee_observability_dashboard.rendered.json \
  /var/lib/grafana/dashboards/deopt/v2_fee_observability_dashboard.json
# Reload Grafana — the dashboard provisioner re-reads the path every
# updateIntervalSeconds (30s in our entry); systemd reload also works:
sudo systemctl reload grafana-server
```

After provisioning, confirm in the UI: the "DeOpt" folder contains
**DeOpt — V2 fee observability (V2G-G)** with UID `deopt-v2g-g-v2-fees`.

Rollback: delete the dashboard provisioning entry and the dashboard
JSON, reload Grafana.

## Phase 6 — Backend read-only live verification

Backend rebuilt (`cargo build --release` — the V2G-G touch already
covered all V2G-G + V2G-H sources) and run with the V2D-S + V2E-F +
V2F-O env stack + V2G-G shell-only overrides. Every broadcast surface
disabled; every signing-key env unset. No DB destructive operations.

### `/health`

```
{"ok":true,"service":"deopt-v2-backend"}
```

### `/admin/fees/v2/observability`

```jsonc
{
  "milestone": "V2G-G",
  "contracts": {
    "perp_engine_new":    "0xc6C592100723Fe0C66343A16e95eC34cC0c2141c",
    "perp_engine_old":    "0xB36395b67D0798ADA981731c9Fa5239F4362b53B",
    "margin_engine_new":  "0x287Cef479be5889eEfCa847F9e73C860898f48Cc",
    "margin_engine_old":  "0x6C5665De05e7314cB63cD77F82DFa86508A5b5F8",
    "fees_manager_v2":    "0x00dA0B9876bcBf0c79CB5BcAcfEBAFb8C7Ad774f"
  },
  "anomaly_totals": {
    "old_consumer_events":     0,
    "unknown_consumer_events": 0
  },
  "metrics": {
    "perp_fee_charged_v2_by_consumer":   {"new": 3, "old": 0, "unknown": 0},
    "perp_fee_rebated_v2_by_consumer":   {"new": 1, "old": 0, "unknown": 0},
    "option_fee_charged_v2_by_consumer": {"new": 3, "old": 0, "unknown": 0},
    "option_fee_rebated_v2_by_consumer": {"new": 1, "old": 0, "unknown": 0},
    "fees_manager_v2_rebate_budget_native": {
      "0x6eae407f5640b006fac9965182e238582a3b412e": 999987
    }
  },
  "features": {
    "metrics_enabled": true,
    "option_event_indexer_enabled": true,
    "fees_enabled": false,
    "rebates_enabled": false,
    "persistence_enabled": true
  }
}
```

### `/admin/fees/onchain?tx_hash=…` (PERP V2G-E)

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

### `/admin/fees/onchain?tx_hash=…` (OPTION V2G-E)

```
event_model:            mixed
fee_charged_v2_count:   1
fee_rebated_v2_count:   1
observed_total_charged: 25
observed_total_rebated: 10
net_protocol_fee:       15
trading_fee_event_count: 1   (V1-compat for the taker leg)
source_of_truth:        onchain
```

### `/metrics`

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

Backend stopped (`pkill -f 'target/release/deopt-v2-backend'`). No DB
mutation; no chain RPC writes (indexer poller disabled to 24h via the
V2F-O env stack — V2G-F already caught the indexer past block
`42206003`).

### `promtool check metrics` — pre-existing convention warning

`promtool check metrics < /metrics` flags every DeOpt
`*_total`-suffixed gauge as "non-counter metrics should not have
`_total` suffix" (19 metrics total, including the four V2 fee
consumer gauges). This is a **pre-existing** style convention in the
DeOpt metric pipeline — none of the V2 fee gauges are emitted as
Prometheus counters, but the names retain `_total` for backwards
compatibility with the V1B alert spec. The bundle uses
`increase(...)[5m]` which works correctly against gauges (Prometheus
treats the per-scrape delta the same way), so the lint warning does
NOT affect alert evaluation. Fixing the lint would require flipping
every `_total` gauge to a counter, which is a much larger refactor
beyond V2G-H scope.

## Phase 7 — Alert expression dry evaluation

### Logical evaluation against the live scrape

Same evaluator script V2G-G used. Every V2 alert is **green**:

```
PerpFeeChargedFromOldEngine             would_fire=False  (raw 0)
PerpFeeRebatedFromOldEngine             would_fire=False  (raw 0)
PerpFeeConsumerUnknown                  would_fire=False  (raw 0)
OptionFeeChargedFromOldMarginEngine     would_fire=False  (raw 0)
OptionFeeRebatedFromOldMarginEngine     would_fire=False  (raw 0)
OptionFeeConsumerUnknown                would_fire=False  (raw 0)
FeesManagerV2RebateBudgetLow            would_fire=False  (raw 999987)
DeoptV2FeeMetricsAbsent                 would_fire=False
```

`FeesManagerV2RebateBudgetStale` (V2G-G — V2 rebate firing while
budget unchanged for 30m) cannot be evaluated against a single scrape
— it requires time-window data. The promtool unit test spec does NOT
yet exercise it because the scenario is multi-window (budget steady +
rebates rising), but the rule's `delta(...)[30m] == 0 and increase(...
)[30m] > 0` is well-formed and lints clean. Operators with a live
Prometheus stack can confirm it by querying the two component
expressions directly.

### `promtool test rules` (synthetic scenarios)

```
$ $PROMTOOL test rules docs/monitoring/prometheus/v2_fee_alerts.test.yml
  SUCCESS
```

5 tests / 35 alert expectations / 0 failures. Covers green baseline,
PERP OLD-engine, OPTION unknown, budget low, metric pipeline absent.

## Phase 8 — Runbook and rollback

This file is the V2G-H runbook. Concise rollback checklist:

| Step | What to revert | How |
|------|----------------|-----|
| 1 | Grafana dashboard provisioning | `sudo rm /etc/grafana/provisioning/dashboards/deopt_v2_fees.yaml /var/lib/grafana/dashboards/deopt/v2_fee_observability_dashboard.json` → `sudo systemctl reload grafana-server` |
| 2 | Alertmanager routing fragment | revert merge in `alertmanager.yml` → `amtool check-config` → reload |
| 3 | Prometheus rule bundle | remove from `rule_files` → `promtool check config` → reload |
| 4 | Operator `.env` patch | restore the previous `PERP_ENGINE_ADDRESS=0xB363…b53B` line + remove the four added vars → restart backend |
| 5 | Backend admin endpoint | not removable — it is built into the binary. Disable by setting `ADMIN_API_ENABLED=false` (also disables every other admin route). |
| 6 | Frontend admin panel | not removable independently — it is built into the dashboard. Disable by hiding the admin route in deployment. |

### Operator checklist (final go/no-go)

1. ☐ Real `.env` patched per Phase 2; verification commands all pass.
2. ☐ Prometheus rule bundle live; `/alerts` lists 9 inactive alerts in 4 groups.
3. ☐ Alertmanager routing live; `amtool config routes test` returns expected receivers for each of the four sample alerts.
4. ☐ Grafana dashboard provisioned; "DeOpt" folder contains "DeOpt — V2 fee observability (V2G-G)".
5. ☐ Backend `/admin/fees/v2/observability` returns the documented green snapshot.
6. ☐ Frontend admin section loads the same snapshot with anomaly tiles at 0.
7. ☐ Synthetic alert send (`amtool alert add`) reaches each receiver in step 3.
8. ☐ Backlog handed off (Mainnet manifest, `FeesManagerV2RebateBudgetStale` cadence tuning, `PerpRebateStalled` activation).

## Files changed (V2G-H)

Backend:

- `docs/V2_FEE_OBSERVABILITY_LIVE_STACK_WIRING_V2G_H.md` — **NEW** (this doc).
- `docs/monitoring/prometheus/v2_fee_alerts.test.yml` — **NEW** (promtool unit tests).
- `docs/monitoring/grafana/provisioning/dashboards/deopt_v2_fees.yaml` — **NEW**.
- `docs/monitoring/grafana/provisioning/datasources/prometheus_example.yaml` — **NEW**.
- `docs/monitoring/grafana/provisioning/render_dashboard.sh` — **NEW** (executable).
- `docs/V2_FEE_PRODUCTION_OBSERVABILITY_V2G_G.md` — V2G-H closure note appended.
- `docs/ALERTING_SPEC.md` — V2G-H additions (test spec + provisioning).
- `docs/RUNBOOK_PERP_V2_FEE_ALERTS.md` — V2G-H operator commands section.

Frontend / Sol:

- Untouched.

## Remaining blockers

1. **Real `.env` patch is operator-only.** V2G-H ships exact diff +
   rollback; agent still cannot edit `.env` per hard rules.
2. **Live monitoring stack absent on this host.** All V2G-H validation
   ran against the scratch promtool / amtool installs at `/tmp/...`.
   Operator must replicate the install + integration commands on the
   target host(s).
3. **`continue: true` double-resolution in the routing example.** The
   mainnet severity-escalation route and the subsystem route both
   point at the same `deopt_oncall_critical` receiver — fine for
   documentation, but in production they should point at distinct
   receivers to avoid double-paging.
4. **`promtool check metrics` style warning.** Pre-existing
   `_total`-on-gauge naming convention; documented as a non-blocking
   lint, not a V2G-H fix.
5. **`FeesManagerV2RebateBudgetStale` unit test deferred.** Multi-window
   scenario; promtool's input series syntax supports it but the test
   spec is left at 5 scenarios for clarity. Add when the live cadence
   is known.

## Next recommended milestone

**V2G-I — flip V2 fee observability to live-firing in the operator's
real Prometheus + Alertmanager + Grafana stack.**

- Apply the operator `.env` patch from Phase 2.
- Run the integration commands from Phases 3-5 on the target host.
- Send a synthetic OLD-engine alert via `amtool alert add` and
  confirm it reaches the contract-on-call receiver.
- Toggle `OLD_MARGIN_ENGINE_ADDRESS` to a bogus address in a staging
  shell, confirm `OptionFeeConsumerUnknown` fires within a minute and
  clears within 5 after revert.
- Cross-check Grafana panels render with non-zero data after the
  next live trade.
- Tune `FeesManagerV2RebateBudgetStale` window once the V2G-band live
  cadence is established (uncomment `DeoptV2PerpRebateStalled` at
  the same time).
