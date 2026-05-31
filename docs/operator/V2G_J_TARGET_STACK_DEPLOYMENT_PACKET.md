# V2G-J — Target-stack deployment packet (V2 fee observability)

This packet ships **operator-runnable** commands to land the V2G-G
artefacts on the real monitoring stack. The agent did NOT run any of
these commands on a remote host — they are explicitly operator-gated.

Three target modes are supported:

1. **Standalone Linux / systemd.**
2. **Docker / Podman Compose.**
3. **Kubernetes (vanilla + Prometheus Operator).**

For each mode, the sequence is the same:

1. Apply backend `.env` patch (see
   `docs/operator/V2G_J_BACKEND_ENV_PATCH_PACKET.md`).
2. Drop the Prometheus rule bundle in.
3. Validate Prometheus config.
4. Reload Prometheus.
5. Merge the Alertmanager routing fragment.
6. Validate Alertmanager config.
7. Reload Alertmanager.
8. Drop the Grafana dashboard + provisioning entry in.
9. Reload Grafana.
10. Run the synthetic alert drill (see
    `docs/operator/V2G_J_SYNTHETIC_ALERT_DRILL_PACKET.md`).
11. Wait the cadence period before enabling the opt-in stalled bundle
    (see §"Stalled-rule activation policy" in the V2G-J doc).

Prerequisites — answer the questionnaire at
`docs/operator/V2G_J_TARGET_STACK_QUESTIONNAIRE.md` first; placeholders
below mirror its section letters (`$B2` = Prometheus config path, etc.).

All paths assume the artefacts are checked out at
`~/DEOPT/deopt-v2-backend` on the **operator workstation**. Commands
that run on the target host start with `# on the target host …`.

## Common — promtool + amtool (operator workstation)

The deployment packet runs `promtool` / `amtool` locally before
shipping config to the target. The V2G-H install pattern still works:

```sh
# Operator workstation
cd /tmp
PROM_TAG="$(curl -sL https://api.github.com/repos/prometheus/prometheus/releases/latest | jq -r .tag_name)"
PROM_VER="${PROM_TAG#v}"
curl -sL -o prom.tar.gz "https://github.com/prometheus/prometheus/releases/download/${PROM_TAG}/prometheus-${PROM_VER}.linux-amd64.tar.gz"
tar -xzf prom.tar.gz "prometheus-${PROM_VER}.linux-amd64/promtool"
export PROMTOOL="/tmp/prometheus-${PROM_VER}.linux-amd64/promtool"

AM_TAG="$(curl -sL https://api.github.com/repos/prometheus/alertmanager/releases/latest | jq -r .tag_name)"
AM_VER="${AM_TAG#v}"
curl -sL -o am.tar.gz "https://github.com/prometheus/alertmanager/releases/download/${AM_TAG}/alertmanager-${AM_VER}.linux-amd64.tar.gz"
tar -xzf am.tar.gz "alertmanager-${AM_VER}.linux-amd64/amtool"
export AMTOOL="/tmp/alertmanager-${AM_VER}.linux-amd64/amtool"

# Validate the bundle and the test spec ONE MORE TIME before shipping.
cd ~/DEOPT/deopt-v2-backend
"${PROMTOOL}" check rules docs/monitoring/prometheus/v2_fee_alerts.bundle.yml
"${PROMTOOL}" check rules docs/monitoring/prometheus/v2_fee_alerts.stalled.yml
"${PROMTOOL}" test  rules docs/monitoring/prometheus/v2_fee_alerts.test.yml
"${AMTOOL}"   check-config docs/monitoring/alertmanager/v2_fee_routing.example.yml
```

Expected output: every command prints `SUCCESS` and exits 0.

---

## Mode 1 — Standalone Linux / systemd

Assumptions (from questionnaire defaults): `$B2 = /etc/prometheus/prometheus.yml`,
`$B3 = /etc/prometheus/rules.d/`, `$C2 = /etc/alertmanager/alertmanager.yml`,
`$D2 = /etc/grafana/provisioning/dashboards/`,
`$D3 = /var/lib/grafana/dashboards/`, `$D4 = Prometheus`.

### 1a. Prometheus rule rollout

```sh
# On the target host. Assumes ~/DEOPT/deopt-v2-backend mirrored.
sudo install -d -m 0755 /etc/prometheus/rules.d
sudo install -m 0644 \
  ~/DEOPT/deopt-v2-backend/docs/monitoring/prometheus/v2_fee_alerts.bundle.yml \
  /etc/prometheus/rules.d/v2_fee_alerts.bundle.yml

# Edit /etc/prometheus/prometheus.yml — add to the `rule_files` list:
#
#   rule_files:
#     - /etc/prometheus/rules.d/v2_fee_alerts.bundle.yml
#
# (Do NOT add v2_fee_alerts.stalled.yml yet — see Stalled Rule
# Activation Policy.)

# Validate the merged config.
sudo promtool check config /etc/prometheus/prometheus.yml
sudo promtool check rules /etc/prometheus/rules.d/v2_fee_alerts.bundle.yml

# Graceful reload (requires --web.enable-lifecycle).
curl -sf -X POST http://127.0.0.1:9090/-/reload && echo "prometheus reloaded"

# Confirm the rules show up in /api/v1/rules.
curl -s http://127.0.0.1:9090/api/v1/rules \
  | jq -r '.data.groups[].name' | grep '^deopt_'
# Expected:
#   deopt_perp_v2_fee_alerts
#   deopt_option_v2_fee_alerts
#   deopt_fees_manager_v2_budget_alerts
#   deopt_v2_fee_metrics_liveness
```

Rollback:

```sh
sudo rm /etc/prometheus/rules.d/v2_fee_alerts.bundle.yml
# Remove the rule_files entry too.
sudo promtool check config /etc/prometheus/prometheus.yml
curl -sf -X POST http://127.0.0.1:9090/-/reload
```

### 1b. Alertmanager routing rollout

```sh
# On the target host.
sudo cp /etc/alertmanager/alertmanager.yml \
        /etc/alertmanager/alertmanager.yml.bak.$(date +%Y%m%dT%H%M%SZ)

# MANUAL: open both files and merge `route.routes`, `receivers`, and
# `inhibit_rules` from
# ~/DEOPT/deopt-v2-backend/docs/monitoring/alertmanager/v2_fee_routing.example.yml
# into /etc/alertmanager/alertmanager.yml. The example uses the same
# receiver names as Section C of the questionnaire; rename if your
# deployment uses different names.
sudo vim /etc/alertmanager/alertmanager.yml

# Validate.
sudo amtool check-config /etc/alertmanager/alertmanager.yml

# Graceful reload.
curl -sf -X POST http://127.0.0.1:9093/-/reload && echo "alertmanager reloaded"

# Confirm the route resolves to expected receivers.
sudo amtool config routes test \
  --config.file=/etc/alertmanager/alertmanager.yml \
  alertname=PerpFeeChargedFromOldEngine \
  subsystem=perp_v2_fees severity=high
# Expected: deopt_oncall_critical (or the receiver name from $C5).
```

Rollback:

```sh
sudo cp /etc/alertmanager/alertmanager.yml.bak.<timestamp> \
        /etc/alertmanager/alertmanager.yml
sudo amtool check-config /etc/alertmanager/alertmanager.yml
curl -sf -X POST http://127.0.0.1:9093/-/reload
```

### 1c. Grafana dashboard rollout

```sh
# On the operator workstation: substitute the Prometheus datasource
# name into the dashboard JSON.
~/DEOPT/deopt-v2-backend/docs/monitoring/grafana/provisioning/render_dashboard.sh "$D4" \
  > /tmp/v2_fee_observability_dashboard.rendered.json

# Copy to the target host (or run the next block on the target).
scp /tmp/v2_fee_observability_dashboard.rendered.json \
    ~/DEOPT/deopt-v2-backend/docs/monitoring/grafana/provisioning/dashboards/deopt_v2_fees.yaml \
    operator@target:/tmp/

# On the target host.
sudo install -m 0644 /tmp/deopt_v2_fees.yaml /etc/grafana/provisioning/dashboards/deopt_v2_fees.yaml
sudo install -d -m 0755 /var/lib/grafana/dashboards/deopt
sudo install -m 0644 /tmp/v2_fee_observability_dashboard.rendered.json \
  /var/lib/grafana/dashboards/deopt/v2_fee_observability_dashboard.json
sudo systemctl reload grafana-server

# Confirm in the Grafana UI: "DeOpt" folder contains
# "DeOpt — V2 fee observability (V2G-G)" with UID deopt-v2g-g-v2-fees.
```

Rollback:

```sh
sudo rm /etc/grafana/provisioning/dashboards/deopt_v2_fees.yaml \
        /var/lib/grafana/dashboards/deopt/v2_fee_observability_dashboard.json
sudo systemctl reload grafana-server
```

---

## Mode 2 — Docker / Podman Compose

Assumptions: there is a `compose.yml` (or `docker-compose.yml`) with
`prometheus`, `alertmanager`, and `grafana` services. Replace
`docker compose` with `podman compose` if you use Podman.

### 2a. Prometheus rule rollout (volume-mounted rules dir)

```sh
# In the compose directory on the host. Suppose the Prometheus service
# mounts ./prometheus/rules into /etc/prometheus/rules.d.
mkdir -p prometheus/rules
cp ~/DEOPT/deopt-v2-backend/docs/monitoring/prometheus/v2_fee_alerts.bundle.yml \
   prometheus/rules/v2_fee_alerts.bundle.yml

# Validate inside the running prometheus container.
docker compose exec prometheus promtool check rules \
  /etc/prometheus/rules.d/v2_fee_alerts.bundle.yml
docker compose exec prometheus promtool check config \
  /etc/prometheus/prometheus.yml

# Graceful reload from the host.
curl -sf -X POST http://127.0.0.1:9090/-/reload
```

If the compose file has not yet added `prometheus/rules/*.yml` to
`rule_files`, edit `prometheus/prometheus.yml` to include
`- /etc/prometheus/rules.d/v2_fee_alerts.bundle.yml` and `docker compose
up -d prometheus` to re-run the entrypoint with the new mount.

Rollback:

```sh
rm prometheus/rules/v2_fee_alerts.bundle.yml
# Revert prometheus.yml if you also edited rule_files.
curl -sf -X POST http://127.0.0.1:9090/-/reload
```

### 2b. Alertmanager routing rollout

```sh
cp alertmanager/alertmanager.yml alertmanager/alertmanager.yml.bak.$(date +%Y%m%dT%H%M%SZ)

# Manual merge as in Mode 1.
vim alertmanager/alertmanager.yml

docker compose exec alertmanager amtool check-config \
  /etc/alertmanager/alertmanager.yml
curl -sf -X POST http://127.0.0.1:9093/-/reload
```

Rollback: restore the `.bak` copy, reload.

### 2c. Grafana dashboard rollout

```sh
~/DEOPT/deopt-v2-backend/docs/monitoring/grafana/provisioning/render_dashboard.sh "$D4" \
  > grafana/dashboards/deopt/v2_fee_observability_dashboard.json
cp ~/DEOPT/deopt-v2-backend/docs/monitoring/grafana/provisioning/dashboards/deopt_v2_fees.yaml \
   grafana/provisioning/dashboards/deopt_v2_fees.yaml

docker compose restart grafana
```

The Grafana container picks up the dashboard via its provisioning
volume mount. UID stays `deopt-v2g-g-v2-fees`.

Rollback: delete the JSON + YAML, restart `grafana`.

---

## Mode 3 — Kubernetes

Two sub-modes: vanilla Prometheus + Alertmanager Helm charts (rule
files mounted via ConfigMap), or Prometheus Operator (rules ship as
`PrometheusRule` CRDs).

### 3a. Vanilla — ConfigMap-driven rules

```sh
# Generate a ConfigMap from the bundle.
kubectl create configmap deopt-v2-fee-alerts \
  --from-file=v2_fee_alerts.bundle.yml=~/DEOPT/deopt-v2-backend/docs/monitoring/prometheus/v2_fee_alerts.bundle.yml \
  --namespace=monitoring \
  --dry-run=client -o yaml > /tmp/deopt-v2-fee-alerts-cm.yaml

# Review the diff before applying.
kubectl diff -f /tmp/deopt-v2-fee-alerts-cm.yaml

# Apply.
kubectl apply -f /tmp/deopt-v2-fee-alerts-cm.yaml

# Ensure the Prometheus pod mounts that ConfigMap at the rules dir
# (e.g. /etc/prometheus/rules/). If not, edit the Helm values or
# Deployment to mount it, then `helm upgrade` / `kubectl rollout
# restart deployment/prometheus -n monitoring`.

# Confirm.
kubectl exec -n monitoring deploy/prometheus -- \
  promtool check rules /etc/prometheus/rules/v2_fee_alerts.bundle.yml

# Graceful reload (Prometheus pods generally support /-/reload):
kubectl port-forward -n monitoring svc/prometheus 9090:9090 &
PF_PID=$!
curl -sf -X POST http://127.0.0.1:9090/-/reload
kill "${PF_PID}"
```

Rollback:

```sh
kubectl delete configmap deopt-v2-fee-alerts -n monitoring
kubectl rollout restart deployment/prometheus -n monitoring
```

### 3b. Prometheus Operator — `PrometheusRule` CRD

```yaml
# /tmp/deopt-v2-fee-alerts-prometheusrule.yaml
# Wrap the V2G-G rule bundle into a PrometheusRule CRD. Drop in any
# matching label selectors the Prometheus CR expects (commonly
# `prometheus: <name>` or `release: <helmrelease>`).
apiVersion: monitoring.coreos.com/v1
kind: PrometheusRule
metadata:
  name: deopt-v2-fee-alerts
  namespace: monitoring
  labels:
    prometheus: deopt
    role: alert-rules
spec:
  # Paste the contents of v2_fee_alerts.bundle.yml's `groups:` here
  # verbatim. Names and labels are CRD-compatible.
  groups:
    # … (paste from docs/monitoring/prometheus/v2_fee_alerts.bundle.yml)
```

```sh
kubectl diff -f /tmp/deopt-v2-fee-alerts-prometheusrule.yaml
kubectl apply -f /tmp/deopt-v2-fee-alerts-prometheusrule.yaml
kubectl rollout status -n monitoring statefulset/prometheus
# The Prometheus pod picks the rule up automatically via the
# Operator's rule-reload sidecar — no manual reload needed.
```

Rollback:

```sh
kubectl delete -f /tmp/deopt-v2-fee-alerts-prometheusrule.yaml
```

### 3c. Alertmanager routing (Kubernetes)

If the Operator manages Alertmanager via the `AlertmanagerConfig` CRD,
wrap the routing fragment per
[the upstream docs](https://prometheus-operator.dev/docs/api-reference/api/#monitoring.coreos.com/v1alpha1.AlertmanagerConfig).
Otherwise, edit the Helm `values.yaml`'s `alertmanager.config` block
and `helm upgrade`.

### 3d. Grafana dashboard (Kubernetes)

```sh
# Use the standard Grafana dashboards ConfigMap pattern.
~/DEOPT/deopt-v2-backend/docs/monitoring/grafana/provisioning/render_dashboard.sh "$D4" \
  > /tmp/v2_fee_observability_dashboard.rendered.json

kubectl create configmap deopt-v2-fee-dashboard \
  --from-file=v2_fee_observability_dashboard.json=/tmp/v2_fee_observability_dashboard.rendered.json \
  --namespace=monitoring \
  --dry-run=client -o yaml \
  | kubectl label --local --dry-run=client -f - grafana_dashboard=1 -o yaml > /tmp/dashboard-cm.yaml

kubectl diff -f /tmp/dashboard-cm.yaml
kubectl apply -f /tmp/dashboard-cm.yaml

# If the Grafana chart's sidecar uses label-based dashboard discovery
# (`grafana_dashboard=1` is the default for kube-prometheus-stack),
# the dashboard appears in the UI within ~30s.
```

Rollback:

```sh
kubectl delete configmap deopt-v2-fee-dashboard -n monitoring
```

---

## Final gate

After Mode-specific steps complete, run the synthetic alert drill packet
(`docs/operator/V2G_J_SYNTHETIC_ALERT_DRILL_PACKET.md`) before
considering the cutover green. If a synthetic injection does not reach
the expected receiver, **roll the corresponding Mode block back** and
re-investigate before retrying.

The `v2_fee_alerts.stalled.yml` opt-in bundle stays out of every Mode
until the cadence policy in
`docs/V2_FEE_OBSERVABILITY_TARGET_CUTOVER_V2G_J.md` §"Stalled-rule
activation policy" is met.
