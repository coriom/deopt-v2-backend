# V2G-J — V2 Fee Observability Target-Stack Cutover Packet

## Status

- Milestone: **V2G-J** — operator cutover packet. Bundles everything
  the operator needs to flip the V2G-G observability surface live on
  the target Prometheus + Alertmanager + Grafana stack, plus the
  `.env` patch on the backend host. The agent performs **no** remote
  mutation in V2G-J — every packet below is operator-runnable text.
- Date: 2026-05-31.
- Outcome:
  - **Target-stack questionnaire** at
    `docs/operator/V2G_J_TARGET_STACK_QUESTIONNAIRE.md` —
    Sections A through G plus a default-policy fallback.
  - **Deployment packet** at
    `docs/operator/V2G_J_TARGET_STACK_DEPLOYMENT_PACKET.md` —
    exact commands for standalone systemd, docker/podman compose,
    and Kubernetes (vanilla + Prometheus Operator).
  - **Backend `.env` patch packet** at
    `docs/operator/V2G_J_BACKEND_ENV_PATCH_PACKET.md` — apply
    + verify + rollback for the five-var observability patch.
  - **Synthetic alert drill packet** at
    `docs/operator/V2G_J_SYNTHETIC_ALERT_DRILL_PACKET.md` —
    five drills covering the four V2G-G receivers plus the
    mainnet escalation route that V2G-I did not exercise.
  - **Stalled-rule activation policy** at
    `docs/operator/V2G_J_STALLED_RULE_ACTIVATION_POLICY.md` —
    explicit five-condition gate before
    `v2_fee_alerts.stalled.yml` lands on the target.
  - **Local pre-cutover read-only check** reproduces V2G-G/H/I
    closure 1:1.
  - All upstream validations green.
- Hard gates respected: no broadcast, no chain mutation, no DB rows
  touched, no private-key handling, no real-secret `.env` edit, no
  remote SSH, no production service reload, no governance/timelock
  action.

## Documents shipped (V2G-J delta)

| Artefact                                                            | Path                                                                  |
|---------------------------------------------------------------------|-----------------------------------------------------------------------|
| Target-stack questionnaire (**new**)                                | `docs/operator/V2G_J_TARGET_STACK_QUESTIONNAIRE.md`                   |
| Deployment packet (systemd / compose / k8s) (**new**)               | `docs/operator/V2G_J_TARGET_STACK_DEPLOYMENT_PACKET.md`               |
| Backend `.env` patch packet (**new**)                               | `docs/operator/V2G_J_BACKEND_ENV_PATCH_PACKET.md`                     |
| Synthetic alert drill packet (**new**)                              | `docs/operator/V2G_J_SYNTHETIC_ALERT_DRILL_PACKET.md`                 |
| Stalled-rule activation policy (**new**)                            | `docs/operator/V2G_J_STALLED_RULE_ACTIVATION_POLICY.md`               |
| V2G-J cutover record (**new**, this file)                           | `docs/V2_FEE_OBSERVABILITY_TARGET_CUTOVER_V2G_J.md`                   |

All upstream V2G-G/H/I artefacts are unchanged:
`docs/monitoring/prometheus/v2_fee_alerts.bundle.yml`,
`v2_fee_alerts.stalled.yml`, `v2_fee_alerts.test.yml`,
`docs/monitoring/alertmanager/v2_fee_routing.example.yml`,
`docs/monitoring/grafana/v2_fee_observability_dashboard.json`, the
provisioning sidecars, and the operator `.env` template
`docs/operator/v2g_g_env_patch.example.env`.

## Phase 1 — Target stack questionnaire

The questionnaire structures the operator's reply so the deployment
packet resolves to a concrete sequence of commands. Sections:

- **A — Stack shape**: local / remote VM / Kubernetes / docker /
  podman / managed; backend co-location; OS; container runtime.
- **B — Prometheus**: version, config path, rule directory, reload
  endpoint, `--web.enable-lifecycle` status, reload method.
- **C — Alertmanager**: version, config path, reload method, API URL,
  five receiver names (critical / high / tickets / ops chat / backend
  on-call), receiver URLs (reference IDs only — never paste tokens),
  network reachability of API.
- **D — Grafana**: version, provisioning directory, dashboards storage
  path, Prometheus datasource name, reload method, folder name + UID.
- **E — Backend host**: deployment shape, gitignored `.env` path,
  restart method, admin-token source, `/metrics` URL, scrape
  interval, `METRICS_ENABLED` state.
- **F — Authorised gates** (eleven explicit `yes` / `no` switches):
  patch + reload backend; drop rule bundle + reload Prometheus;
  merge routing + reload Alertmanager; drop dashboard + reload
  Grafana; fire synthetic drill; enable opt-in stalled bundle (must
  be `no` until cadence policy met); allow SSH from agent.
- **G — Out-of-band info**: mainnet chain label, settlement-asset
  addresses, known rebate cadence.

Default-policy fallback: if a section is left blank, the agent uses
the safest action — emit commands as text, never auto-execute.

## Phase 2 — Target deployment packet

Three target modes, same eleven-step sequence. The packet wraps each
mode with explicit rollback commands.

### Mode 1 — Standalone Linux / systemd

Prometheus rules → `/etc/prometheus/rules.d/v2_fee_alerts.bundle.yml`;
config reload via `curl -X POST http://127.0.0.1:9090/-/reload`.
Alertmanager: backup `alertmanager.yml`, merge the routing fragment
manually, `amtool check-config`, reload. Grafana: copy provisioning
entry to `/etc/grafana/provisioning/dashboards/`, copy rendered
dashboard JSON to `/var/lib/grafana/dashboards/deopt/`, `systemctl
reload grafana-server`.

### Mode 2 — Docker / Podman Compose

Rules / routing / dashboard ship via volume-mounted directories
(`prometheus/rules`, `alertmanager`, `grafana/dashboards`,
`grafana/provisioning`). Validate inside the running containers
with `docker compose exec prometheus promtool check rules …` and
`docker compose exec alertmanager amtool check-config …`.
Reload via the same HTTP `/-/reload` endpoints from the host.

### Mode 3 — Kubernetes

Two flavours:

- **Vanilla** — ship rules as a `ConfigMap` mounted at the rules
  directory; `kubectl diff` + `kubectl apply`; reload via port-forward
  + `curl -X POST .../-/reload`.
- **Prometheus Operator** — wrap the bundle's `groups:` into a
  `PrometheusRule` CRD with the appropriate label selectors; the
  Operator's rule-reload sidecar picks the change up automatically.

Dashboard ships as a `ConfigMap` labelled `grafana_dashboard=1`
(default for `kube-prometheus-stack`).

Each mode has a paired rollback block — the deployment is purely
additive (no destructive edits to existing configs except for
manually merged routing).

## Phase 3 — Backend `.env` patch packet

The patch flips `PERP_ENGINE_ADDRESS` from the V2F-O OLD carry-over
to NEW and appends four observability metadata vars
(`OLD_PERP_ENGINE_ADDRESS`, `MARGIN_ENGINE`, `OLD_MARGIN_ENGINE_ADDRESS`,
`FEES_MANAGER_V2`). The packet provides:

- A pre-condition check that prints only var names + values, never
  secrets.
- A timestamped `.env.bak.<UTC>` backup before any mutation.
- An idempotent `sed`-then-append apply sequence (sed is a no-op if
  the var is already NEW; the append guard skips vars already
  present).
- Post-apply verification: grep + value-check + admin probe
  (`/admin/fees/v2/observability`) + `/metrics` grep, with expected
  values baked in (PERP charged{new}=3, OPTION charged{new}=3,
  rebated{new}=1, OLD/unknown=0, mUSDC budget=999987).
- A one-shot rollback that restores the most recent `.env.bak.*`.
- Safety warnings: never point the active address at OLD; never
  print private keys; never run against an unrelated repo's `.env`.

## Phase 4 — Synthetic alert drill packet

Six drill blocks ready to copy-paste against the target Alertmanager
once the routing fragment is live. Each drill uses `amtool alert
add` with the same label schema the V2G-G rules emit:

| # | Alert                          | Expected receiver       | Notes |
|---|--------------------------------|-------------------------|-------|
| 1 | `PerpFeeChargedFromOldEngine`  | `deopt_oncall_critical` | high+perp+old |
| 2 | `OptionFeeConsumerUnknown`     | `deopt_anomaly_tickets` | medium+option+unknown |
| 3 | `FeesManagerV2RebateBudgetLow` | `deopt_ops_chat`        | medium+fees_manager_v2+asset_symbol=musdc |
| 4 | `DeoptV2FeeMetricsAbsent`      | `deopt_backend_oncall`  | high+v2_fee_observability |
| 5 | `HighSeverityMainnetRouteDrill` | `deopt_oncall_high`    | **fills the V2G-I gap** — severity=medium+chain=base-mainnet |
| 6 | Inhibit: `DeoptV2FeeMetricsAbsent{subsystem=perp_v2_fees}` source + `PerpFeeRebatedFromOldEngine` target | source dispatched, target suppressed via `equal: [subsystem]` |

Every drill block ships with the corresponding `amtool config
routes test` invocation for offline verification before the live
inject. Cleanup block resolves every synthetic alert with `--end=$(date
+5s)`.

Receiver-outcome matrix table is included for the operator to fill
in during the live drill so the post-cutover audit log is complete.

## Phase 5 — Stalled-rule activation policy

`docs/monitoring/prometheus/v2_fee_alerts.stalled.yml` stays out of
the default cutover. The activation policy at
`docs/operator/V2G_J_STALLED_RULE_ACTIVATION_POLICY.md` codifies the
gate:

1. `increase(deopt_perp_fee_rebated_v2_total{consumer="new"}[24h])`
   ≥ N (recommended `N = 3`).
2. The same condition held for 48 consecutive hours.
3. Grafana panel confirms continuous monotonic rise (no sparse
   bursts).
4. `/admin/fees/v2/observability` reports zero anomaly events during
   the window (else fix misclassification first).
5. Operator on-call ACK.

If natural cadence is sparser (e.g. weekly), widen `[24h]→[7d]` and
`for: 24h → for: 7d` **before enabling** rather than turning the rule
on against the default window. Enable + rollback commands provided.

## Phase 6 — Local pre-cutover read-only check

Backend rebuilt + run read-only with the V2D-S + V2E-F + V2F-O env
stack and V2G-G shell-only overrides (every broadcast surface off,
every private-key env unset).

| Endpoint                                                                                       | Result |
|------------------------------------------------------------------------------------------------|--------|
| `/health`                                                                                      | `{"ok":true,"service":"deopt-v2-backend"}` ✅ |
| `/admin/fees/v2/observability`                                                                 | `milestone=V2G-G`, anomaly totals 0, four buckets `{new=3\|1, old=0, unknown=0}`, budget `999987`, all 5 contracts populated ✅ |
| `/admin/fees/onchain?tx_hash=0x5c15e923…` (PERP V2G-E)                                         | `event_model=v2`, charged=6, rebated=3, net=3, `source_of_truth=onchain` ✅ |
| `/admin/fees/onchain?tx_hash=0x9a85cbce…` (OPTION V2G-E)                                       | `event_model=mixed`, charged=25, rebated=10, net=15, `trading_fee_event_count=1`, `source_of_truth=onchain` ✅ |
| `/metrics`                                                                                     | 13 V2 fee series matching V2G-G expected values exactly ✅ |

Backend stopped (`pkill -TERM`). No DB writes, no chain RPC writes.

## Phase 7 — Docs

Created (V2G-J):

- `docs/V2_FEE_OBSERVABILITY_TARGET_CUTOVER_V2G_J.md` (this file).
- `docs/operator/V2G_J_TARGET_STACK_QUESTIONNAIRE.md`.
- `docs/operator/V2G_J_TARGET_STACK_DEPLOYMENT_PACKET.md`.
- `docs/operator/V2G_J_BACKEND_ENV_PATCH_PACKET.md`.
- `docs/operator/V2G_J_SYNTHETIC_ALERT_DRILL_PACKET.md`.
- `docs/operator/V2G_J_STALLED_RULE_ACTIVATION_POLICY.md`.

Updated (V2G-J):

- `docs/V2_FEE_OBSERVABILITY_LIVE_ACTIVATION_V2G_I.md` — V2G-J
  cutover-packet pointer appended.
- `docs/V2_FEE_PRODUCTION_OBSERVABILITY_V2G_G.md` — V2G-J cutover
  pointer.
- `docs/ALERTING_SPEC.md` — operator-packet pointers.
- `docs/RUNBOOK_PERP_V2_FEE_ALERTS.md` — V2G-J cutover-day
  workflow.

## Phase 8 — Validation

Backend (no Rust code touched in V2G-J — docs only):

| Command                                                       | Result |
|---------------------------------------------------------------|--------|
| `cargo fmt --all --check`                                     | ✅ clean |
| `cargo clippy --all-targets --all-features -- -D warnings`    | ✅ clean |
| `cargo build --all-targets --all-features`                    | ✅ clean |
| `cargo test --all-targets --all-features --no-fail-fast`      | ✅ **679 passed, 0 failed, 0 ignored** (unchanged from V2G-G/H/I) |

Monitoring:

| Command                                                                  | Result |
|--------------------------------------------------------------------------|--------|
| `promtool check rules docs/monitoring/prometheus/v2_fee_alerts.bundle.yml`  | ✅ 9 rules / 4 groups |
| `promtool check rules docs/monitoring/prometheus/v2_fee_alerts.stalled.yml` | ✅ 1 rule / 1 group |
| `promtool test rules docs/monitoring/prometheus/v2_fee_alerts.test.yml`     | ✅ SUCCESS (7 groups) |
| `amtool check-config docs/monitoring/alertmanager/v2_fee_routing.example.yml` | ✅ |
| `python3 -m json.tool docs/monitoring/grafana/v2_fee_observability_dashboard.json` | ✅ |

Frontend / Sol: untouched in V2G-J scope.

## Stalled-rule activation policy (summary)

`v2_fee_alerts.stalled.yml` is **OFF** until:

1. `increase(perp_rebated_v2_total{consumer="new"}[24h])` ≥ 3,
2. condition held for 48 consecutive hours,
3. dashboard panel shows continuous monotonic rise,
4. `/admin/fees/v2/observability` anomaly totals stay zero,
5. on-call ACK.

Re-evaluate the gate after every change to network-wide rebate flow
(new tiers, dropped makers, new markets).

## Rollback plan (cutover layer)

Cutover is a four-layer flip — each layer rolls back independently:

| Layer                       | Rollback                                                                                       |
|-----------------------------|------------------------------------------------------------------------------------------------|
| Backend `.env`              | `cp .env.bak.<UTC> .env` → restart backend per §E3                                             |
| Prometheus rule bundle      | remove rule file from `rule_files`, `promtool check config`, `curl -X POST /-/reload`           |
| Alertmanager routing merge  | restore `.bak` of `alertmanager.yml`, `amtool check-config`, `curl -X POST /-/reload`           |
| Grafana dashboard           | remove provisioning entry + dashboard JSON, `systemctl reload grafana-server`                  |
| Opt-in stalled bundle (if enabled) | same as Prometheus bundle rollback                                                       |

The stale-rule **bugfix** that landed in V2G-I is part of the
`v2_fee_alerts.bundle.yml`; if it ever needs to be reverted, that's a
single-line git revert in the doc repo, not a target-stack operation.

## Remaining operator gates

| Gate | Owner | Authorisation needed |
|------|-------|----------------------|
| `.env` patch applied on target backend host | operator | F1 |
| Backend reloaded post-patch                 | operator | F2 |
| Rule bundle dropped on Prometheus host      | operator | F3 |
| Prometheus reloaded                         | operator | F4 |
| Routing fragment merged in Alertmanager     | operator | F5 |
| Alertmanager reloaded                       | operator | F6 |
| Grafana provisioning entry + JSON dropped   | operator | F7 |
| Grafana reloaded                            | operator | F8 |
| Synthetic drill against target Alertmanager | operator | F9 |
| Stalled bundle enabled                      | operator + policy gate | F10 (must remain `no` until policy met) |
| Agent SSH to target host                    | operator | F11 (must include host + auth method) |

The agent will not execute any of the above without explicit
written authorisation per gate.

## V2G-L0 local-stack pointer (appended 2026-05-31)

For operators who want to rehearse the V2G-J cutover end-to-end on
a single host before touching the real target stack, V2G-L0 ships
a complete localhost docker-compose stack at
`docs/monitoring/local-stack/`. Layout:

- `compose.yml` — Prometheus 3.12.0 + Alertmanager 0.32.1 + Grafana
  11.4.0 + webhook sink, all 127.0.0.1-bound.
- `prometheus/rules/` — symlinks into the canonical
  `docs/monitoring/prometheus/v2_fee_alerts.bundle.yml`. No drift.
- `alertmanager/alertmanager.yml` — the V2G-J routing example pointed
  at the local webhook sink.
- `grafana/...` — provisioning entries + a dashboard JSON rendered
  with DS=`Prometheus`.
- `webhook-sink/webhook_sink.py` — single-file Python sink that
  receives every dispatch on six receiver paths.

Bring up with `docker compose up -d` after granting docker socket
access. See
`docs/V2_FEE_OBSERVABILITY_LOCAL_STACK_BOOTSTRAP_V2G_L0.md` for the
full rehearsal record, including the bare-binary fallback the agent
used when docker socket access was blocked.

## V2G-K soak record pointer (appended 2026-05-31)

V2G-K opened the 7-day soak record at
`docs/V2_FEE_OBSERVABILITY_7_DAY_SOAK_V2G_K.md`. The soak record
includes:

- Baseline metric snapshot captured against the local backend (will
  be compared against the target host's first post-cutover scrape).
- Local synthetic drill stand-in transcript: all 5 V2G-J drills,
  including the mainnet escalation route that V2G-I did not exercise,
  routed cleanly to the expected receivers.
- Per-layer rollback table.
- 7-day daily checklist with the day-5 stalled-rule re-evaluation
  marker.
- Stalled-rule decision: **defer** until the activation policy gate
  (≥3 PERP rebates per 24h × 48h sustained) can be evaluated.

Cutover status: **pending operator authorisation.** F1-F11 all
default = NO; the operator answer template is at
`docs/operator/V2G_K_OPERATOR_ANSWER_TEMPLATE.md`.

## Next recommended milestone

**V2G-K — production live-fire and 7-day soak.**

- Operator runs the V2G-J packets on the target stack (Modes 1/2/3 per
  Section A).
- Operator runs the synthetic drill packet; agent records the
  receiver-outcome matrix in a follow-up addendum.
- After 48h of clean operation under the V2G-J cutover, evaluate the
  stalled-rule activation gate; enable if conditions hold.
- Soak the surface for a calendar week, then file the V2G-K closure
  doc with: number of real alerts received, mean time to ack,
  receiver fan-out anomalies (if any), and either keep or relax the
  stalled-rule window per observed cadence.
- Begin V2H planning — multi-asset settlement (the multi-asset matrix
  in V2G-G already pre-stages the per-asset rule cloning pattern).
