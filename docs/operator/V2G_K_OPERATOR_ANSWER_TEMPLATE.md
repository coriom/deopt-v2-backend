# V2G-K — Operator answer template

Fill this in and reply to the agent. The agent will not perform any
F-gate action until the matching authorisation row reads **YES** and
the target paths/URLs are populated. Anything left blank is treated as
"defer — agent emits commands only".

## A — Stack shape

| Question | Answer |
|----------|--------|
| `A1` Where does the monitoring stack run? (`systemd`, `docker-compose`, `podman-compose`, `kubernetes`, `managed`, `absent`) | |
| `A2` Backend co-located with the stack? (`yes`/`no`) | |
| `A3` Host OS + version | |
| `A4` Container runtime (if any) | |
| `A5` Kubernetes flavour (if any) | |
| `A6` Prometheus Operator? (`yes`/`no`/`n/a`) | |
| `A7` Managed Grafana? (`yes`/`no`/`n/a`) | |

## B — Prometheus

| Question | Answer |
|----------|--------|
| `B1` Prometheus version | |
| `B2` `prometheus.yml` path | |
| `B3` Rule files directory | |
| `B5` Reload endpoint URL | |
| `B6` `--web.enable-lifecycle` enabled? | |
| `B7` Reload method | |

## C — Alertmanager

| Question | Answer |
|----------|--------|
| `C1` Alertmanager version | |
| `C2` `alertmanager.yml` path | |
| `C3` Reload method | |
| `C4` API base URL | |
| `C5` `deopt_oncall_critical` receiver | |
| `C6` `deopt_oncall_high` receiver | |
| `C7` `deopt_anomaly_tickets` receiver | |
| `C8` `deopt_ops_chat` receiver | |
| `C9` `deopt_backend_oncall` receiver | |
| `C11` API reachable from operator workstation? (`yes`/`no`/`tunnel`) | |

## D — Grafana

| Question | Answer |
|----------|--------|
| `D1` Grafana version | |
| `D2` Provisioning dashboards dir | |
| `D3` Dashboards storage path | |
| `D4` Prometheus datasource name | |
| `D5` Reload method | |
| `D6` UI-only import preferred? | |
| `D7` Folder name + UID | |

## E — Backend host

| Question | Answer |
|----------|--------|
| `E1` Backend deployment shape | |
| `E2` `.env` path | |
| `E3` Backend restart command | |
| `E5` `/metrics` scrape URL exposed to Prometheus | |
| `E7` `METRICS_ENABLED=true` currently? | |

## F — Authorisation gates (must each say **YES** to execute that step)

| Gate | Authorised? |
|------|-------------|
| F1 — apply `.env` patch on target backend host | |
| F2 — restart backend post-patch | |
| F3 — drop V2G-G rule bundle into target Prometheus rules dir | |
| F4 — reload Prometheus | |
| F5 — merge routing fragment in target Alertmanager | |
| F6 — reload Alertmanager | |
| F7 — drop Grafana provisioning + dashboard JSON on target | |
| F8 — reload Grafana | |
| F9 — fire synthetic alert drill against target Alertmanager | |
| F10 — enable opt-in `v2_fee_alerts.stalled.yml` (default **NO**) | |
| F11 — allow agent SSH to target host (provide `user@host` + auth method) | |

## G — Out-of-band info

- Mainnet chain label (if not `base-mainnet`): \_\_\_\_\_\_
- Settlement-asset addresses on target network (1 per asset): \_\_\_\_\_\_
- Known PERP rebate cadence (events/24h, if known): \_\_\_\_\_\_

## How the agent reads this

- Any row with a blank Answer/Authorised cell: agent emits commands as
  text and stops at the gate.
- For any F-gate set to YES, the agent emits the exact commands it
  would run plus a one-line confirmation block; it still does not
  auto-execute until the operator pastes "EXEC" against that gate.
- For F11 the agent additionally requires the operator to paste the
  SSH host fingerprint they expect (so the agent can sanity-check
  before issuing any remote command).

Until this template is filled in, V2G-K stays at "soak start
pending — operator has the packets, gates remain open".
