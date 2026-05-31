# V2G-J — Operator target-stack questionnaire

Fill this in before running the cutover packet. Every answer maps to
an exact command path in
`docs/operator/V2G_J_TARGET_STACK_DEPLOYMENT_PACKET.md` — without the
answers below, the deployment packet cannot resolve to a concrete
sequence of commands.

Send the filled-in answers as a single Markdown / text reply. Do **not**
paste credentials, tokens, or private webhook URLs in plain text;
substitute placeholders like `$RECEIVER_URL` and keep the real values
in your secret store.

## Section A — Stack shape

| # | Question | Answer | Notes |
|---|----------|--------|-------|
| A1 | Where does the monitoring stack run? | one of: `local`, `remote VM`, `Kubernetes`, `Docker compose`, `Podman compose`, `managed (Grafana Cloud / AMP)`, `other` |
| A2 | Same host as the backend? | `yes` / `no` | If `no`, list scrape target and network reachability. |
| A3 | OS / distro of the host(s) (if applicable) | e.g. `Ubuntu 22.04`, `Debian 12`, `Amazon Linux 2023` |
| A4 | Container runtime (if applicable) | `docker`, `podman`, `containerd`, `cri-o`, `n/a` |
| A5 | Kubernetes mode (if applicable) | `vanilla`, `EKS`, `GKE`, `AKS`, `Rancher`, `k3s`, `microk8s`, `n/a` |
| A6 | Are you using Prometheus Operator? | `yes` / `no` / `n/a` | Determines whether rules ship as `PrometheusRule` CRDs or plain files. |
| A7 | Managed Grafana? | `yes` / `no` / `n/a` | Determines whether dashboard ships via provisioning or API token import. |

## Section B — Prometheus

| # | Question | Answer |
|---|----------|--------|
| B1 | Prometheus binary version | `prometheus --version` output (first line) |
| B2 | Path to `prometheus.yml` | e.g. `/etc/prometheus/prometheus.yml` |
| B3 | Path to the rule files directory | e.g. `/etc/prometheus/rules.d/` |
| B4 | How does Prometheus learn about rule files? | `rule_files: …` glob, ConfigMap, sidecar, other |
| B5 | Graceful-reload endpoint URL | e.g. `http://127.0.0.1:9090/-/reload`, or `n/a (restart only)` |
| B6 | Is `--web.enable-lifecycle` on? | `yes` / `no` |
| B7 | Reload method | `systemctl reload prometheus`, `kill -HUP`, `kubectl rollout restart`, `curl -X POST .../-/reload`, other |

## Section C — Alertmanager

| # | Question | Answer |
|---|----------|--------|
| C1 | Alertmanager version | `alertmanager --version` output |
| C2 | Path to `alertmanager.yml` | e.g. `/etc/alertmanager/alertmanager.yml` |
| C3 | Reload method | `curl -X POST .../-/reload`, `systemctl reload`, `kubectl rollout`, other |
| C4 | API base URL | `http://127.0.0.1:9093`, `https://am.example.com`, etc. |
| C5 | Receiver name for **smart-contract on-call (critical)** | e.g. `deopt_oncall_critical` (must match the routing fragment) |
| C6 | Receiver name for **mainnet medium escalation** | e.g. `deopt_oncall_high` |
| C7 | Receiver name for **fee anomaly tickets** | e.g. `deopt_anomaly_tickets` |
| C8 | Receiver name for **ops chat** (budget alerts) | e.g. `deopt_ops_chat` |
| C9 | Receiver name for **backend on-call** (metrics absent) | e.g. `deopt_backend_oncall` |
| C10 | Receiver URLs (PagerDuty / Slack / webhook) | provide reference IDs only; **do not paste tokens** |
| C11 | Is the alertmanager API reachable from your laptop for the synthetic drill? | `yes` / `no` / `tunnel` |

## Section D — Grafana

| # | Question | Answer |
|---|----------|--------|
| D1 | Grafana version | `/api/health` `version` field |
| D2 | Provisioning directory for dashboards | e.g. `/etc/grafana/provisioning/dashboards/` |
| D3 | Dashboards storage path referenced by provisioning | e.g. `/var/lib/grafana/dashboards/` |
| D4 | Prometheus datasource name in Grafana | e.g. `Prometheus`, `DS_PROMETHEUS`, `prom-prod` |
| D5 | Grafana reload method | `systemctl reload grafana-server`, `kubectl rollout`, UI-only, API-only |
| D6 | If UI-only import is preferred | `yes` — operator will upload the rendered JSON manually |
| D7 | Folder name + UID for the DeOpt dashboard folder | e.g. `DeOpt` / `deopt` |

## Section E — Backend host

| # | Question | Answer |
|---|----------|--------|
| E1 | Where does the backend run? | host / container / pod |
| E2 | Path to the gitignored `.env` on that host | e.g. `/srv/deopt/deopt-v2-backend/.env` |
| E3 | How is the backend restarted? | `systemctl restart deopt-backend`, `docker compose restart`, `kubectl rollout`, other |
| E4 | Admin token storage | env var / secret manager / vault path (reference only) |
| E5 | `/metrics` scrape URL exposed to Prometheus | e.g. `http://127.0.0.1:8080/metrics` |
| E6 | Scrape interval already configured? | `yes` / `no` (default Prom 15s is fine) |
| E7 | Is `METRICS_ENABLED=true` in the running env? | yes / no — V2G-G alerts assume yes |

## Section F — Gates the operator authorises

Tick each gate the operator authorises for this cutover window. The
agent will only execute the gates that are explicitly authorised in
writing.

| # | Action | Authorised? |
|---|--------|-------------|
| F1 | Apply the `.env` patch on the backend host now | `yes` / `no` / `defer` |
| F2 | Reload the backend now (post-patch) | `yes` / `no` / `defer` |
| F3 | Drop the V2G-G rule bundle into the Prometheus rules dir now | `yes` / `no` |
| F4 | Reload Prometheus now | `yes` / `no` |
| F5 | Merge the routing fragment into Alertmanager now | `yes` / `no` |
| F6 | Reload Alertmanager now | `yes` / `no` |
| F7 | Drop the Grafana provisioning entry + JSON now | `yes` / `no` |
| F8 | Reload Grafana now | `yes` / `no` |
| F9 | Fire the synthetic alert drill against the target Alertmanager now | `yes` / `no` |
| F10 | Enable the opt-in `v2_fee_alerts.stalled.yml` bundle now | **must be `no`** until the cadence gate in §V2G-J Stalled Rule Activation Policy is met |
| F11 | Allow SSH from agent to target host (if operator authorises) | `yes` / `no` + hostnames |

## Section G — Out-of-band info

- **Mainnet chain label**, if different from `base-mainnet` (used by
  the mainnet escalation route): \_\_\_\_\_\_\_\_\_\_
- **Settlement asset addresses on the target network** (one per asset
  that gets a `FeesManagerV2RebateBudgetLow_<symbol>` rule clone): \_\_\_\_\_\_\_\_\_\_
- **Known rebate cadence** (used to size `DeoptV2PerpRebateStalled`'s
  `[24h]` window or relax it): \_\_\_\_\_\_\_\_\_\_

## Default policy if a question is left blank

| Section | Default behaviour |
|---------|-------------------|
| A — stack shape | Treat as `standalone Linux/systemd` (V2G-J packet §1). |
| B — Prometheus | Use `/etc/prometheus/` + `curl -X POST .../-/reload`. |
| C — Alertmanager | Use the routing example's receiver names verbatim; operator must wire URLs. |
| D — Grafana | Use provisioning under `/etc/grafana/provisioning/` + `/var/lib/grafana/dashboards/`. |
| E — backend | Assume `.env` lives at `~/DEOPT/deopt-v2-backend/.env` (the local dev path). |
| F — gates | All `no` until explicitly authorised. Synthetic drill is skipped; the agent emits commands only. |
| G — out-of-band | Use Base Sepolia values from V2G-G/H. |

When in doubt, the agent will prefer the safest action (no remote
side effect, no service reload) and emit commands as text.
