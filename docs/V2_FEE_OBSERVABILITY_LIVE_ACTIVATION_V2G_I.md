# V2G-I — V2 Fee Observability Live Activation & Synthetic Alert Drill

## Status

- Milestone: **V2G-I** — activation milestone. V2G-G shipped the
  artefacts, V2G-H validated them with the real Prometheus +
  Alertmanager toolchains. V2G-I now runs a **real local Alertmanager
  process** + a real webhook sink to prove end-to-end routing and
  inhibit semantics, adds rule-level unit tests for the stale and
  stalled scenarios that were deferred in V2G-G/H, and re-runs the
  backend live verification.
- Date: 2026-05-31.
- Outcome:
  - **Stale-budget rule bug found and fixed.** The V2G-G
    `FeesManagerV2RebateBudgetStale` expression used a bare vector
    `and` against two operands with disjoint label sets (`{asset=...}`
    vs `{consumer="new"}`). PromQL default vector matching returns
    the empty set in that case — the alert silently never fired.
    Fixed to `delta(budget[30m]) == 0 and on() (... rebate counters
    ...)` and pinned the contract with a new `promtool test rules`
    scenario. The label fix is recorded in
    `docs/monitoring/prometheus/v2_fee_alerts.bundle.yml` with a
    comment block referencing this milestone.
  - **`DeoptV2PerpRebateStalled` shipped as an opt-in file.**
    `docs/monitoring/prometheus/v2_fee_alerts.stalled.yml` carries
    the previously-commented-out rule as a separately-activatable
    bundle, with explicit operator instructions and a 48-hour
    `promtool test rules` scenario that exercises the 24h drought
    window + 24h `for` threshold.
  - **Local Alertmanager + webhook sink drill ran end-to-end.**
    Started `alertmanager v0.32.1` against the rendered routing
    config bound to `127.0.0.1:9093`, plus a single-file Python
    webhook sink on `127.0.0.1:9095`. Six synthetic injections fired
    via `amtool alert add`; every alert landed at the documented
    receiver, the resolve path closed cleanly, and the inhibit rule
    suppressed a downstream alert as designed.
  - **Backend live read-only verification reproduced V2G-H state
    1:1** — `/health`, `/admin/fees/v2/observability`,
    `/admin/fees/onchain` for both V2G-E txs, `/metrics` all green.
  - Operator `.env` patch remains documented but unapplied (real
    `.env` still carries the V2F-O `PERP_ENGINE_ADDRESS=OLD` line
    per the hard rule).
- Hard gates respected: no broadcast, no chain mutation, no DB
  writes, no private-key handling, no real-secret `.env` edit, no
  governance/timelock action, no remote/production service touched.

## Source-of-truth artefacts (delta over V2G-H)

| Artefact                                       | Path                                                                                       |
|------------------------------------------------|--------------------------------------------------------------------------------------------|
| Prometheus rule bundle (**bugfix**)            | `docs/monitoring/prometheus/v2_fee_alerts.bundle.yml`                                      |
| Opt-in stalled-cadence rule (**new**)          | `docs/monitoring/prometheus/v2_fee_alerts.stalled.yml`                                     |
| Prometheus unit-test spec (**extended**)       | `docs/monitoring/prometheus/v2_fee_alerts.test.yml`                                        |
| V2G-I activation doc (**new**)                 | `docs/V2_FEE_OBSERVABILITY_LIVE_ACTIVATION_V2G_I.md` (this file)                           |
| Alertmanager routing example (unchanged)       | `docs/monitoring/alertmanager/v2_fee_routing.example.yml`                                  |
| Grafana dashboard + provisioning (unchanged)   | `docs/monitoring/grafana/...`                                                              |
| Operator `.env` patch (unchanged)              | `docs/operator/v2g_g_env_patch.example.env`                                                |

## Phase 1 — Real stack target discovery

Re-ran the V2G-H discovery sweep plus a wider probe (helm / k3s /
microk8s / minikube / kind / podman / kubectl). Result is conclusive:

```
which podman kubectl helm k3s minikube kind microk8s
  → all missing
docker info
  → docker client present, daemon socket access denied (WSL host)
docker ps / docker images
  → no monitoring containers, no monitoring images
find . -maxdepth 6 -iname "docker-compose*.yml" -o -iname "Chart.yaml" \
  -o -iname "values.yaml" -o -iname "compose.yaml"
  → no matches outside vendor trees
ss -tlnp | grep -E ":9090|:9093|:3000|:9100"
  → no listeners
env | grep -iE "PROMETHEUS|ALERTMANAGER|GRAFANA"
  → empty
systemctl list-units | grep -iE "prometheus|alertmanager|grafana"
  → none
ls /etc/prometheus /etc/alertmanager /etc/grafana
  → no such directory
```

Conclusion: **the host has no operator stack** — neither local nor
remotely referenced. V2G-I therefore (a) ran the synthetic drill
against a scratch local Alertmanager process bound to localhost only,
and (b) emits remote-operator commands that the deployment operator
must execute on the target host.

## Phase 2 — Operator `.env` patch gate

Reread `docs/operator/v2g_g_env_patch.example.env` and re-confirmed
the diff against the current real `.env` (grep-only — file untouched):

```
Patch lines required (from v2g_g_env_patch.example.env):
  PERP_ENGINE_ADDRESS=0xc6C592100723Fe0C66343A16e95eC34cC0c2141c
  OLD_PERP_ENGINE_ADDRESS=0xB36395b67D0798ADA981731c9Fa5239F4362b53B
  MARGIN_ENGINE=0x287Cef479be5889eEfCa847F9e73C860898f48Cc
  OLD_MARGIN_ENGINE_ADDRESS=0x6C5665De05e7314cB63cD77F82DFa86508A5b5F8
  FEES_MANAGER_V2=0x00dA0B9876bcBf0c79CB5BcAcfEBAFb8C7Ad774f

Current `.env` state (read-only grep):
  line 50: PERP_ENGINE_ADDRESS=0xB36395b67D0798ADA981731c9Fa5239F4362b53B   # OLD
  (all four other vars missing)
```

### Exact apply command (operator-only)

```sh
# Open the gitignored real .env and apply this diff manually — the
# agent will not edit it under the V2G hard rules.
sed -i.bak 's@^PERP_ENGINE_ADDRESS=.*@PERP_ENGINE_ADDRESS=0xc6C592100723Fe0C66343A16e95eC34cC0c2141c@' \
  ~/DEOPT/deopt-v2-backend/.env

cat >> ~/DEOPT/deopt-v2-backend/.env <<'EOF'

# --- V2G-I observability classifier (operator-applied) ---
OLD_PERP_ENGINE_ADDRESS=0xB36395b67D0798ADA981731c9Fa5239F4362b53B
MARGIN_ENGINE=0x287Cef479be5889eEfCa847F9e73C860898f48Cc
OLD_MARGIN_ENGINE_ADDRESS=0x6C5665De05e7314cB63cD77F82DFa86508A5b5F8
FEES_MANAGER_V2=0x00dA0B9876bcBf0c79CB5BcAcfEBAFb8C7Ad774f
EOF
```

The `sed -i.bak` creates a `.env.bak` next to the file as a one-shot
local rollback safety net. (Real Git rollback is preferable but the
file is gitignored.)

### Rollback snippet

```sh
# Revert PERP_ENGINE_ADDRESS to the pre-patch OLD value and drop the
# four added vars. The `.env.bak` from the apply step is the
# authoritative pre-patch snapshot.
cp ~/DEOPT/deopt-v2-backend/.env.bak ~/DEOPT/deopt-v2-backend/.env
# Confirm with the verification block below; restart the backend.
```

### Post-apply verification (prints only var names + addresses, no secrets)

```sh
grep -nE "^(PERP_ENGINE_ADDRESS|OLD_PERP_ENGINE_ADDRESS|MARGIN_ENGINE|OLD_MARGIN_ENGINE_ADDRESS|FEES_MANAGER_V2)=" \
  ~/DEOPT/deopt-v2-backend/.env
# Expected output:
#   PERP_ENGINE_ADDRESS=0xc6C592100723Fe0C66343A16e95eC34cC0c2141c
#   OLD_PERP_ENGINE_ADDRESS=0xB36395b67D0798ADA981731c9Fa5239F4362b53B
#   MARGIN_ENGINE=0x287Cef479be5889eEfCa847F9e73C860898f48Cc
#   OLD_MARGIN_ENGINE_ADDRESS=0x6C5665De05e7314cB63cD77F82DFa86508A5b5F8
#   FEES_MANAGER_V2=0x00dA0B9876bcBf0c79CB5BcAcfEBAFb8C7Ad774f
```

Followed by the `/admin/fees/v2/observability` + `/metrics` checks
from `docs/operator/v2g_g_env_patch.example.env` §3.

### Status

**Pending — operator gate.** The agent did not apply. The shell-only
override pattern used in V2G-G/H/I read-only verification remains a
valid temporary fallback.

## Phase 3 — Prometheus activation packet

### Rule bugfix (recorded in `v2_fee_alerts.bundle.yml`)

```diff
-      - alert: FeesManagerV2RebateBudgetStale
-        expr: |
-          (
-            increase(deopt_option_fee_rebated_v2_total{consumer="new"}[30m]) > 0
-            or
-            increase(deopt_perp_fee_rebated_v2_total{consumer="new"}[30m]) > 0
-          )
-          and
-          delta(
-            deopt_fees_manager_v2_rebate_budget_native{
-              asset="0x6eae407f5640b006fac9965182e238582a3b412e"
-            }[30m]
-          ) == 0
-        for: 15m
+      - alert: FeesManagerV2RebateBudgetStale
+        expr: |
+          delta(
+            deopt_fees_manager_v2_rebate_budget_native{
+              asset="0x6eae407f5640b006fac9965182e238582a3b412e"
+            }[30m]
+          ) == 0
+          and on() (
+            increase(deopt_option_fee_rebated_v2_total{consumer="new"}[30m]) > 0
+            or
+            increase(deopt_perp_fee_rebated_v2_total{consumer="new"}[30m]) > 0
+          )
+        for: 15m
```

Why it was broken: PromQL `vector1 and vector2` returns elements of
`vector1` that have **identical** label sets in `vector2`. The budget
gauge carries `{asset="..."}` and the rebate counters carry
`{consumer="new"}` — no overlap, so the intersection is empty and the
alert never moved out of inactive. The fix adds `on()` to ignore all
labels in the intersection and keeps the budget gauge's `asset` label
on the output (the natural per-asset alert payload).

This is a substantive rule-level regression discovered by the new
`promtool test rules` scenario. The bundle now ships the fixed
expression plus a long comment block documenting the contract and
pointing at the unit test.

### `promtool check rules`

```
$ /tmp/prometheus-3.12.0.linux-amd64/promtool check rules \
    docs/monitoring/prometheus/v2_fee_alerts.bundle.yml \
    docs/monitoring/prometheus/v2_fee_alerts.stalled.yml \
    docs/alertmanager/perp_v2_fee_alerts.yml \
    docs/alertmanager/option_v2_fee_alerts.yml
docs/monitoring/prometheus/v2_fee_alerts.bundle.yml  SUCCESS: 9 rules found
docs/monitoring/prometheus/v2_fee_alerts.stalled.yml SUCCESS: 1 rules found
docs/alertmanager/perp_v2_fee_alerts.yml             SUCCESS: 3 rules found
docs/alertmanager/option_v2_fee_alerts.yml           SUCCESS: 4 rules found
```

### `promtool test rules` (extended)

```
$ /tmp/prometheus-3.12.0.linux-amd64/promtool test rules \
    docs/monitoring/prometheus/v2_fee_alerts.test.yml
  SUCCESS
```

7 groups now: green / PERP OLD fires / OPTION unknown fires / budget
low / metrics absent / **stale budget fires (V2G-I)** / **PERP rebate
stalled fires after 24h silence (V2G-I)**.

### Integration commands (operator-facing — unchanged from V2G-H plus the stalled bundle)

```sh
# Standalone (replace deployment-specific paths as needed):
sudo install -m 0644 \
  ~/DEOPT/deopt-v2-backend/docs/monitoring/prometheus/v2_fee_alerts.bundle.yml \
  /etc/prometheus/rules.d/v2_fee_alerts.bundle.yml
# Optional opt-in (only if the network has stable rebate cadence):
sudo install -m 0644 \
  ~/DEOPT/deopt-v2-backend/docs/monitoring/prometheus/v2_fee_alerts.stalled.yml \
  /etc/prometheus/rules.d/v2_fee_alerts.stalled.yml

# Reference both from /etc/prometheus/prometheus.yml:
#   rule_files:
#     - /etc/prometheus/rules.d/v2_fee_alerts.bundle.yml
#     - /etc/prometheus/rules.d/v2_fee_alerts.stalled.yml

# Validate the merged config + graceful reload:
sudo promtool check config /etc/prometheus/prometheus.yml
curl -X POST http://127.0.0.1:9090/-/reload
```

Docker compose / Kubernetes Operator commands identical to V2G-H
§Phase 3 — the new `v2_fee_alerts.stalled.yml` is just one more file
to mount.

## Phase 4 — Alertmanager activation packet

### `amtool check-config`

```
$ /tmp/alertmanager-0.32.1.linux-amd64/amtool check-config \
    docs/monitoring/alertmanager/v2_fee_routing.example.yml
SUCCESS
Found:
 - global config
 - route
 - 1 inhibit rules
 - 6 receivers
 - 0 templates
```

### Receiver mapping (validated in Phase 6 drill)

| Receiver               | Routes hit                                                                       | Live drill result            |
|------------------------|----------------------------------------------------------------------------------|------------------------------|
| `deopt_oncall_critical` | PERP/OPTION OLD/regression (high/critical), mainnet escalation                   | fired + resolved ✅          |
| `deopt_oncall_high`     | Mainnet medium-escalation                                                        | not exercised (no test alert) |
| `deopt_anomaly_tickets` | `PerpFeeConsumerUnknown` / `OptionFeeConsumerUnknown`                            | fired ✅                     |
| `deopt_ops_chat`        | `FeesManagerV2RebateBudgetLow` / `…Stale`                                        | fired ✅                     |
| `deopt_backend_oncall`  | `DeoptV2FeeMetricsAbsent`                                                        | fired + resolved + inhibit ✅ |
| `default`               | catch-all                                                                        | not exercised (no fallthrough) |

### Integration commands (unchanged from V2G-H §Phase 4)

```sh
# 1) Merge docs/monitoring/alertmanager/v2_fee_routing.example.yml
#    into the deployed /etc/alertmanager/alertmanager.yml. The example
#    is a fragment with its own route + receivers + inhibit_rules
#    subtrees — diff and merge manually.
# 2) Validate the merged config:
sudo amtool check-config /etc/alertmanager/alertmanager.yml
# 3) Graceful reload:
curl -X POST http://127.0.0.1:9093/-/reload
```

## Phase 5 — Grafana dashboard activation packet

Unchanged from V2G-H §Phase 5. Restated for completeness:

```sh
# Render the dashboard against the operator's Prometheus datasource:
docs/monitoring/grafana/provisioning/render_dashboard.sh "Prometheus" \
  > /tmp/v2_fee_observability_dashboard.rendered.json

# Provisioning (preferred for stable rollouts):
sudo install -m 0644 \
  docs/monitoring/grafana/provisioning/dashboards/deopt_v2_fees.yaml \
  /etc/grafana/provisioning/dashboards/deopt_v2_fees.yaml
sudo install -d -m 0755 /var/lib/grafana/dashboards/deopt
sudo install -m 0644 /tmp/v2_fee_observability_dashboard.rendered.json \
  /var/lib/grafana/dashboards/deopt/v2_fee_observability_dashboard.json
sudo systemctl reload grafana-server
```

Confirmed `render_dashboard.sh Prometheus | python3 -c "json.load(stdin)"`
parses cleanly with no remaining `${DS_PROMETHEUS}` placeholders.

## Phase 6 — Synthetic alert drill (local, complete)

Set up a localhost-only stand:

1. Extracted the alertmanager v0.32.1 server binary alongside the
   amtool binary already present from V2G-H
   (`/tmp/alertmanager-0.32.1.linux-amd64/alertmanager`).
2. Rendered a runnable Alertmanager config at
   `/tmp/v2g_i_alertmanager/alertmanager.yml` by combining the
   committed routing example with localhost webhook receivers
   (`http://127.0.0.1:9095/<path>`).
3. Wrote a single-file Python webhook sink
   (`/tmp/v2g_i_alertmanager/webhook_sink.py`) that listens on
   `127.0.0.1:9095` and appends every dispatch to a JSON log.
4. Started both processes bound only to localhost (no external
   network exposure):

   ```
   alertmanager --config.file=… --web.listen-address=127.0.0.1:9093 \
                --cluster.listen-address= --log.level=warn
   webhook_sink.py
   ```

5. Fired six synthetic alerts via `amtool alert add`:

   | # | alertname                       | extra labels                                                           | expected receiver           | result |
   |---|---------------------------------|------------------------------------------------------------------------|-----------------------------|--------|
   | 1 | `PerpFeeChargedFromOldEngine`   | subsystem=perp_v2_fees severity=high consumer=old                      | `deopt_oncall_critical`     | ✅     |
   | 2 | `OptionFeeConsumerUnknown`      | subsystem=option_v2_fees severity=medium consumer=unknown              | `deopt_anomaly_tickets`     | ✅     |
   | 3 | `FeesManagerV2RebateBudgetLow`  | subsystem=fees_manager_v2 severity=medium asset_symbol=musdc asset=... | `deopt_ops_chat`            | ✅     |
   | 4 | `DeoptV2FeeMetricsAbsent`       | subsystem=v2_fee_observability severity=high consumer=new              | `deopt_backend_oncall`      | ✅     |
   | 5 | `PerpFeeChargedFromOldEngine` resolve (`--end=now`) | (same labels as #1)                                  | resolved → `deopt_oncall_critical` | ✅     |
   | 6 | inhibit: `DeoptV2FeeMetricsAbsent` source + `PerpFeeRebatedFromOldEngine` target (both `subsystem=perp_v2_fees`) | source dispatched, target **suppressed by inhibit** | ✅     |

   Tail of the webhook sink log shows exactly the expected dispatches:

   ```
   dispatches by (receiver_path, alertname, status):
     backend   PerpFeeChargedFromOldEngine    n/a (received via critical only)
     backend   DeoptV2FeeMetricsAbsent        firing, resolved
     critical  PerpFeeChargedFromOldEngine    firing, resolved
     critical  DeoptV2FeeMetricsAbsent        firing (inhibit source — subsystem=perp_v2_fees label set for drill 6)
     ops       FeesManagerV2RebateBudgetLow   firing
     tickets   OptionFeeConsumerUnknown       firing
   ```

   `amtool` issued a single informational warning per call: "moving
   to a new parser… input is incompatible." The classic parser was
   used as fallback and every alert was accepted — the warning is
   about CLI annotation-string quoting and does not affect routing.

6. Inhibit verification — `GET /api/v2/alerts` after drill 6 shows
   `PerpFeeRebatedFromOldEngine{subsystem=perp_v2_fees}` with
   `state=suppressed` and `inhibitedBy=['0b9ba37d2238f003']` (the
   fingerprint of the `DeoptV2FeeMetricsAbsent` source). The
   inhibit rule's `equal: [subsystem]` matched because both
   carried `subsystem=perp_v2_fees`.

7. Cleanup: `pkill -f alertmanager` + `pkill -f webhook_sink`. No
   open listeners on 9093 / 9095 after shutdown.

### Receivers exercised, not exercised

- **Exercised:** `deopt_oncall_critical`, `deopt_anomaly_tickets`,
  `deopt_ops_chat`, `deopt_backend_oncall`, plus the inhibit rule and
  the resolve path.
- **Not exercised in the drill:** `deopt_oncall_high` (only used for
  mainnet `severity=medium`+`chain=base-mainnet` matchers), `default`
  (catch-all — never reached because every test alert matched a
  concrete sub-route).

For mainnet activation, the operator should fire one
`severity=medium,chain=base-mainnet` synthetic alert per route to
confirm the mainnet escalation arm dispatches to
`deopt_oncall_high`.

## Phase 7 — Stale + stalled alert tests

### `FeesManagerV2RebateBudgetStale` — new test (60m horizon, 1m cadence)

Simulates a 60-minute window where the rebate-budget gauge is frozen
at 999_987 while the OPTION rebated counter ticks up by +1 every 5
minutes (steady maker rebate flow + stalled indexer). Pinned eval
points:

- `t=4m`: alert silent (no OPTION rebate tick yet).
- `t=30m`: alert firing (condition was true at t=5m, `for: 15m`
  threshold crossed at t=20m).
- `t=60m`: alert still firing.

Label payload pinned: `{alertname, severity=medium,
subsystem=fees_manager_v2, milestone=V2G-G, asset_symbol=musdc,
asset=0x6eae407f...}`.

This test caught and pinned the rule bug described in §Phase 3.

### `DeoptV2PerpRebateStalled` — new opt-in rule + new test (48h horizon, 5m cadence)

The rule lives in the new `v2_fee_alerts.stalled.yml` because it
should not fire on Base Sepolia today (V2G-E is the only live PERP
rebate so far — no cadence baseline). The test injects 576 samples
(48h at 5m sampling) where every V2 fee gauge holds its V2G-E value
constant:

- `t=24h`: condition becomes true (no PERP rebate increment in last
  24h), alert state moves to pending.
- `t=48h`: alert has been pending for 24h, transitions to firing.

Label payload pinned: `{alertname, severity=medium,
subsystem=perp_v2_fees, milestone=V2G-I, consumer="new"}`.

The 5-minute sampling cadence keeps the synthetic dataset small
(~7500 sample points across all 13 series) while still producing
clean window arithmetic.

### Combined run

```
$ /tmp/prometheus-3.12.0.linux-amd64/promtool test rules \
    docs/monitoring/prometheus/v2_fee_alerts.test.yml
  SUCCESS
```

Seven test groups, 50+ alert expectations, 0 failures.

## Phase 8 — Live backend green verification

Backend rebuilt + run read-only with the V2D-S + V2E-F + V2F-O env
stack and V2G-G shell-only overrides; every broadcast surface off,
every signing-key env unset.

### `/health`

```
{"ok":true,"service":"deopt-v2-backend"}
```

### `/admin/fees/v2/observability`

```jsonc
{
  "milestone": "V2G-G",
  "anomaly_totals": {"old_consumer_events": 0, "unknown_consumer_events": 0},
  "metrics": {
    "perp_fee_charged_v2_by_consumer":   {"new":3,"old":0,"unknown":0},
    "perp_fee_rebated_v2_by_consumer":   {"new":1,"old":0,"unknown":0},
    "option_fee_charged_v2_by_consumer": {"new":3,"old":0,"unknown":0},
    "option_fee_rebated_v2_by_consumer": {"new":1,"old":0,"unknown":0},
    "fees_manager_v2_rebate_budget_native": {
      "0x6eae407f5640b006fac9965182e238582a3b412e": 999987
    }
  },
  "contracts": { /* all 5 populated */ }
}
```

### `/admin/fees/onchain` for both V2G-E txs

```
PERP   0x5c15e923…   event_model=v2     charged=6  rebated=3   net=3  source_of_truth=onchain
OPTION 0x9a85cbce…   event_model=mixed  charged=25 rebated=10  net=15 trading_fee_event_count=1  source_of_truth=onchain
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
mutation; no chain RPC writes.

## Phase 9 — Docs

Created:

- `docs/V2_FEE_OBSERVABILITY_LIVE_ACTIVATION_V2G_I.md` (this doc).
- `docs/monitoring/prometheus/v2_fee_alerts.stalled.yml` (opt-in
  cadence rule).

Updated:

- `docs/monitoring/prometheus/v2_fee_alerts.bundle.yml` —
  `FeesManagerV2RebateBudgetStale` PromQL bugfix + reference to the
  V2G-I unit test.
- `docs/monitoring/prometheus/v2_fee_alerts.test.yml` — two new
  groups: `rebate_budget_stale_fires`,
  `perp_rebate_stalled_fires_after_24h_silence`. `rule_files` now
  includes both production bundles.
- `docs/V2_FEE_OBSERVABILITY_LIVE_STACK_WIRING_V2G_H.md` — V2G-I
  closure note appended.
- `docs/ALERTING_SPEC.md` — V2G-I additions (stale rule bugfix,
  opt-in stalled rule).
- `docs/RUNBOOK_PERP_V2_FEE_ALERTS.md` — V2G-I additions
  (`DeoptV2PerpRebateStalled` response procedure, stalled-rule
  activation guidance).

## Phase 10 — Validation

Backend (no Rust code touched in V2G-I scope):

| Command | Result |
|---|---|
| `cargo fmt --all --check`                                  | ✅ clean |
| `cargo clippy --all-targets --all-features -- -D warnings` | ✅ clean |
| `cargo build --all-targets --all-features`                 | ✅ clean |
| `cargo test --all-targets --all-features --no-fail-fast`   | ✅ **679 passed, 0 failed, 0 ignored** (unchanged from V2G-G/H) |

Monitoring:

| Command | Result |
|---|---|
| `promtool check rules v2_fee_alerts.bundle.yml`             | ✅ 9 rules / 4 groups |
| `promtool check rules v2_fee_alerts.stalled.yml`            | ✅ 1 rule / 1 group |
| `promtool check rules docs/alertmanager/perp_v2_fee_alerts.yml`   | ✅ 3 rules |
| `promtool check rules docs/alertmanager/option_v2_fee_alerts.yml` | ✅ 4 rules |
| `promtool test rules v2_fee_alerts.test.yml`                | ✅ SUCCESS (7 groups) |
| `amtool check-config v2_fee_routing.example.yml`            | ✅ |
| `amtool check-config alertmanager.yml` (drill config)       | ✅ |
| Grafana dashboard JSON parse + DS substitution              | ✅ (10 panels, UID `deopt-v2g-g-v2-fees`) |
| Provisioning YAML structural validation                     | ✅ |

Frontend / Sol: untouched in V2G-I scope.

## Rollback plan

| Layer | Revert command |
|-------|----------------|
| Grafana dashboard | `sudo rm /etc/grafana/provisioning/dashboards/deopt_v2_fees.yaml /var/lib/grafana/dashboards/deopt/v2_fee_observability_dashboard.json` → `sudo systemctl reload grafana-server` |
| Alertmanager routing | revert merge in `alertmanager.yml` → `amtool check-config` → `curl -X POST .../-/reload` |
| Prometheus rule bundle | `sudo rm /etc/prometheus/rules.d/v2_fee_alerts.bundle.yml` → drop the `rule_files` entry → `promtool check config` → `curl -X POST .../-/reload` |
| Stalled opt-in rule | same as above for `v2_fee_alerts.stalled.yml` |
| Stale-rule bugfix in bundle | git revert the V2G-I commit (the diff is small + isolated) |
| Operator `.env` patch | `cp ~/DEOPT/deopt-v2-backend/.env.bak ~/DEOPT/deopt-v2-backend/.env` → restart backend |

## Remaining blockers

1. **Real `.env` patch is operator-only.** V2G-I ships exact diff +
   apply + rollback + verification commands; cannot be auto-applied.
2. **Live monitoring stack still absent on this host.** V2G-I ran
   the synthetic drill against a scratch local Alertmanager bound to
   127.0.0.1. Operator must replicate the integration on the target
   host using the V2G-H §3-5 commands.
3. **Mainnet escalation routes not drill-exercised.** The
   `deopt_oncall_high` receiver was never hit in the V2G-I drill
   because no test alert carried `chain=base-mainnet`. Operator
   should fire a `severity=medium,chain=base-mainnet` synthetic on
   the target stack as part of the cutover gate.
4. **`continue: true` double-resolution.** Still documented as the
   intentional V2G-G shape; production should use distinct
   receivers for mainnet escalation vs subsystem routes if double
   paging is undesirable.
5. **`_total`-on-gauge naming convention.** Pre-existing across 19
   DeOpt metrics; non-blocking for alert evaluation (`increase()` on
   gauges works the same way). Renaming would be a wider refactor.

## Next recommended milestone

**V2G-J — operator cutover on the target stack.**

- Apply the operator `.env` patch from §Phase 2 to the real
  gitignored `.env`; confirm `/admin/fees/v2/observability` returns
  the green snapshot from the prod host.
- Run the V2G-H §3-5 integration commands on the target Prometheus
  / Alertmanager / Grafana deployment.
- Re-run the V2G-I drill against the target Alertmanager (with the
  production receiver URLs, not the localhost sink).
- Fire the missing mainnet-escalation synthetic (`severity=medium,
  chain=base-mainnet`) to confirm `deopt_oncall_high` routing.
- Toggle `OLD_MARGIN_ENGINE_ADDRESS` to a bogus address in a
  staging shell, confirm `OptionFeeConsumerUnknown` fires within
  a minute and clears within 5 after revert.
- After 24h of steady V2G-band trade flow, enable
  `v2_fee_alerts.stalled.yml` in the production rules directory and
  monitor for a week to confirm the cadence threshold is
  appropriately sized.
