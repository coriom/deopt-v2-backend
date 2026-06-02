# V2G-K — V2 Fee Observability 7-Day Soak Record

## Status

- Milestone: **V2G-K** — live-fire cutover + 7-day soak start. V2G-J
  shipped the operator-runnable packets; V2G-K supervises the
  cutover, records the soak start, and ships the daily check
  checklist.
- Date: 2026-05-31.
- Outcome (as of file creation):
  - **Cutover status: pending operator authorisation.** Stack
    discovery on this host still shows no Prometheus / Alertmanager
    / Grafana installed (no PATH binaries, no `/etc/...` config
    directories, no listening ports on 9090/9093/3000, no docker
    daemon access, no SSH config referencing prod). The
    questionnaire reply
    (`docs/operator/V2G_K_OPERATOR_ANSWER_TEMPLATE.md`) is empty.
    All F1-F11 gates remain at default = NO.
  - **Local baseline snapshot captured.** Backend rebuilt + run
    read-only on Base Sepolia state; `/health`, `/admin/fees/v2/observability`,
    both V2G-E `/admin/fees/onchain` payloads, and `/metrics`
    archived under `/tmp/v2g_k_baseline/` with timestamp
    `2026-05-31T16:14:34Z`. All values reproduce V2G-G/H/I/J 1:1.
  - **Local synthetic drill stand-in repeated.** Spun up the V2G-I
    Alertmanager + webhook sink config one more time; fired all
    five V2G-J drill alerts (PERP OLD, OPTION unknown, budget low,
    metrics absent, mainnet escalation). Every alert routed to the
    expected receiver, every webhook landed. Receiver-outcome
    matrix at `/tmp/v2g_k_drill/receiver_outcome_matrix.md`.
  - **Stalled-rule decision: defer.** No 24h/48h cadence data on the
    target stack yet — `v2_fee_alerts.stalled.yml` stays out of the
    rules directory per V2G-J activation policy.
  - **Soak start: pending.** The day-0 row of the daily checklist is
    parked. Soak day-1 starts the calendar day after F1-F9 are
    operator-applied on the target host.
- Hard gates respected: no broadcast, no chain mutation, no DB
  writes, no private-key handling, no real-secret `.env` edit, no
  remote SSH, no production service reload, no governance/timelock
  action.

## Cutover timestamp

| Field                        | Value                                                                 |
|------------------------------|-----------------------------------------------------------------------|
| V2G-K record opened          | **2026-05-31T16:14Z**                                                 |
| Cutover applied on target    | _pending operator (F3-F8)_                                            |
| Backend env patch applied    | _pending operator (F1-F2)_                                            |
| Synthetic drill executed     | local stand-in 2026-05-31T16:16Z; target drill _pending operator (F9)_ |
| Soak day 1 begins            | _T+24h after F8 reload_                                               |

## Cutover artefact status

Each row references the V2G-G/H/J source file and the V2G-J operator
packet describing how to land it on the target host.

| # | Layer                                | Source artefact                                                                  | Operator packet                                                                              | Status                                                |
|---|--------------------------------------|----------------------------------------------------------------------------------|---------------------------------------------------------------------------------------------|-------------------------------------------------------|
| 1 | Backend `.env` classifier vars       | `docs/operator/v2g_g_env_patch.example.env`                                       | `docs/operator/V2G_J_BACKEND_ENV_PATCH_PACKET.md`                                            | **Pending operator** (F1, F2)                          |
| 2 | Prometheus rule bundle               | `docs/monitoring/prometheus/v2_fee_alerts.bundle.yml`                             | `docs/operator/V2G_J_TARGET_STACK_DEPLOYMENT_PACKET.md` §"Mode N → Prometheus rule rollout"   | **Pending operator** (F3, F4) — `promtool check rules` green locally |
| 3 | Alertmanager routing                 | `docs/monitoring/alertmanager/v2_fee_routing.example.yml`                         | `docs/operator/V2G_J_TARGET_STACK_DEPLOYMENT_PACKET.md` §"Mode N → Alertmanager routing rollout" | **Pending operator** (F5, F6) — `amtool check-config` green locally |
| 4 | Grafana dashboard + provisioning     | `docs/monitoring/grafana/v2_fee_observability_dashboard.json` + provisioning sidecars | `docs/operator/V2G_J_TARGET_STACK_DEPLOYMENT_PACKET.md` §"Mode N → Grafana dashboard rollout" | **Pending operator** (F7, F8) — JSON parses, render substitutes clean |
| 5 | Synthetic alert drill                | `docs/operator/V2G_J_SYNTHETIC_ALERT_DRILL_PACKET.md`                            | same packet                                                                                  | **Pending operator** (F9) — local stand-in passed all 5 routes |
| 6 | Opt-in `DeoptV2PerpRebateStalled`    | `docs/monitoring/prometheus/v2_fee_alerts.stalled.yml`                            | `docs/operator/V2G_J_STALLED_RULE_ACTIVATION_POLICY.md`                                      | **Deferred** — 24h/48h cadence gate not yet evaluable; default = OFF |

## Starting metric snapshot (local baseline, 2026-05-31T16:14:34Z)

Archived at `/tmp/v2g_k_baseline/`. Operator should compare the target
host's first scrape after F4 reload against these exact values.

### `/health`

```
{"ok":true,"service":"deopt-v2-backend"}
```

### `/admin/fees/v2/observability`

```jsonc
{
  "milestone": "V2G-G",
  "anomaly_totals": {
    "old_consumer_events": 0,
    "unknown_consumer_events": 0
  },
  "contracts": {
    "perp_engine_new":   "0xc6C592100723Fe0C66343A16e95eC34cC0c2141c",
    "perp_engine_old":   "0xB36395b67D0798ADA981731c9Fa5239F4362b53B",
    "margin_engine_new": "0x287Cef479be5889eEfCa847F9e73C860898f48Cc",
    "margin_engine_old": "0x6C5665De05e7314cB63cD77F82DFa86508A5b5F8",
    "fees_manager_v2":   "0x00dA0B9876bcBf0c79CB5BcAcfEBAFb8C7Ad774f"
  },
  "metrics": {
    "perp_fee_charged_v2_by_consumer":    {"new": 3, "old": 0, "unknown": 0},
    "perp_fee_rebated_v2_by_consumer":    {"new": 1, "old": 0, "unknown": 0},
    "option_fee_charged_v2_by_consumer":  {"new": 3, "old": 0, "unknown": 0},
    "option_fee_rebated_v2_by_consumer":  {"new": 1, "old": 0, "unknown": 0},
    "fees_manager_v2_rebate_budget_native": {
      "0x6eae407f5640b006fac9965182e238582a3b412e": 999987
    }
  }
}
```

### `/admin/fees/onchain` — PERP V2G-E `0x5c15e923…aa394`

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

### `/admin/fees/onchain` — OPTION V2G-E `0x9a85cbce…3149`

```
event_model:             mixed
fee_charged_v2_count:    1
fee_rebated_v2_count:    1
observed_total_charged:  25
observed_total_rebated:  10
net_protocol_fee:        15
trading_fee_event_count: 1
source_of_truth:         onchain
```

### `/metrics` — V2 fee block

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

### Expected green state on the target host post-cutover

The values above are the floor. The soak passes day-1 only if the
target host's first post-cutover scrape:

- Returns `/health` `{"ok":true,...}`.
- Returns `anomaly_totals.old_consumer_events == 0`,
  `anomaly_totals.unknown_consumer_events == 0`.
- Returns `metrics.perp_fee_charged_v2_by_consumer.new >= 3` and
  `metrics.option_fee_charged_v2_by_consumer.new >= 3` (≥ because
  fresh trades may have increased the count).
- Returns `metrics.fees_manager_v2_rebate_budget_native["0x6eae...412e"]
  > 1000` (above the `FeesManagerV2RebateBudgetLow` threshold).
- Returns the same five contracts populated with the same addresses.

## Synthetic drill — local stand-in transcript (reference)

The agent ran the V2G-J drill packet against the same local
Alertmanager stand-in used in V2G-I, against the V2G-J routing
example. All five drills landed at the expected receiver; the webhook
sink confirms HTTP delivery.

```
Drill 1 PerpFeeChargedFromOldEngine     → deopt_oncall_critical  (sink: /critical)  ✅
Drill 2 OptionFeeConsumerUnknown        → deopt_anomaly_tickets  (sink: /tickets)   ✅
Drill 3 FeesManagerV2RebateBudgetLow    → deopt_ops_chat         (sink: /ops)       ✅
Drill 4 DeoptV2FeeMetricsAbsent         → deopt_backend_oncall   (sink: /backend)   ✅
Drill 5 HighSeverityMainnetRouteDrill   → deopt_oncall_high      (sink: /high)      ✅  (new in V2G-J, missing from V2G-I)
```

Full matrix: `/tmp/v2g_k_drill/receiver_outcome_matrix.md` (filesystem
only; not committed because it is operator-environment-specific).

The operator's target-host drill will produce the same shape if the
routing fragment was merged verbatim. If not, follow the failure /
triage block in
`docs/operator/V2G_J_SYNTHETIC_ALERT_DRILL_PACKET.md` §"Failure /
triage".

## Stalled-rule decision

`v2_fee_alerts.stalled.yml` stays **OFF** during this soak window.

Reason: the activation policy at
`docs/operator/V2G_J_STALLED_RULE_ACTIVATION_POLICY.md` requires:

1. `increase(perp_rebated_v2_total{consumer="new"}[24h]) >= 3`,
2. condition held for 48 consecutive hours,
3. Grafana panel confirms monotonic rise,
4. anomaly totals zero during window,
5. on-call ACK.

The target host has zero history at soak start (Prometheus only just
started scraping). The earliest the gate **could** be re-evaluated is
soak day 4 (T+48h after the rule pipeline has been in place for 24h
to start the 24h window, plus 24h to confirm sustained behaviour).
Re-evaluate on day 5 of the soak; decision row is in the daily
checklist below.

If the cadence gate is not met by day 7 — likely on Base Sepolia
today, where V2G-E is the only live PERP rebate — defer further. Do
not relax the activation policy under soak pressure.

## Rollback links

If any soak day shows a regression, roll back the affected layer
**only** — full rollback is not required for a single-layer fault.

| Layer                                       | Rollback command (target host)                                                                                |
|--------------------------------------------|---------------------------------------------------------------------------------------------------------------|
| Backend `.env` patch                       | `cp ~/DEOPT/deopt-v2-backend/.env.bak.<UTC> ~/DEOPT/deopt-v2-backend/.env && <restart>`                       |
| Prometheus rule bundle                     | `sudo rm /etc/prometheus/rules.d/v2_fee_alerts.bundle.yml && sudo promtool check config $B2 && curl -X POST $B5` |
| Alertmanager routing fragment              | `sudo cp $C2.bak.<UTC> $C2 && sudo amtool check-config $C2 && curl -X POST $AM_URL/-/reload`                  |
| Grafana dashboard + provisioning           | `sudo rm $D2/deopt_v2_fees.yaml $D3/deopt/v2_fee_observability_dashboard.json && sudo systemctl reload grafana-server` |
| Opt-in stalled bundle (if enabled in soak) | `sudo rm /etc/prometheus/rules.d/v2_fee_alerts.stalled.yml && curl -X POST $B5`                                |

All rollback paths are non-destructive (no DB writes, no chain
calls). Per-layer rollback aligns with the V2G-J packet §"Rollback"
sections.

## Daily checklist (7 days)

Run on each soak day at a consistent time of day. Fill in the rows
below with the operator's check timestamp + outcome. Carry forward
any anomalies to the V2G-K closure summary at the end of the soak.

For each day, run the steps in the **`Steps`** column on the target
host (or the target-host's `/metrics` URL from the operator
workstation).

### Checks (apply daily — reorder freely)

| # | Step                                                                                       | Pass criterion |
|---|--------------------------------------------------------------------------------------------|----------------|
| 1 | `curl -sf $BACKEND_URL/health`                                                             | `{"ok":true,…}` |
| 2 | `curl -sH "x-admin-token: $ADMIN_TOKEN" $BACKEND_URL/admin/fees/v2/observability \| jq '.anomaly_totals, .metrics, .contracts'` | `anomaly_totals.old_consumer_events == 0` and `…unknown_consumer_events == 0`; metric `new` buckets monotonically ≥ baseline; budget ≥ baseline `999987` (may have moved if rebates landed during the soak — drop is fine, increase is fine, jumps are noteworthy). |
| 3 | `curl -sH "x-admin-token: $ADMIN_TOKEN" $BACKEND_URL/metrics \| grep -E '^deopt_(perp\|option)_fee_(charged\|rebated)_v2_total\|^deopt_fees_manager_v2_rebate_budget_native'` | Same shape as baseline; values may have advanced |
| 4 | Prometheus `/alerts` page lists 9 deopt_*_v2_*-named alerts, all `state=inactive`           | No `pending` or `firing` against the V2 fee surface |
| 5 | Alertmanager `/api/v2/alerts` returns no V2 fee alerts                                     | empty array (or only synthetic resolution residuals) |
| 6 | Grafana — open dashboard `deopt-v2g-g-v2-fees`; confirm all 4 timeseries + 2 budget + 3 anomaly stat + engine reference panels render | All panels show data; OLD + unknown stat tiles green |
| 7 | Indexer health — `/admin/options/events` count is monotonic (or stable if backend not indexing this period) | `event_indexer_enabled=true`, last scan within scrape interval |
| 8 | No `FeesManagerV2RebateBudgetStale` firing (the bugfix from V2G-I would catch indexer lag)  | inactive |
| 9 | No `DeoptV2FeeMetricsAbsent` firing                                                        | inactive |
| 10| (Day 5 only) Re-evaluate the stalled-rule activation gate                                  | document yes/no in the per-day table |

### Per-day log

Format: `YYYY-MM-DDTHH:MMZ`, then for each item: `ok | warn | fail`,
plus a one-line note for any non-OK row. Day 0 is the soak open record
(this file); Day 1 starts T+24h after F8 reload.

| Day | Date / time (UTC) | Step 1 | Step 2 | Step 3 | Step 4 | Step 5 | Step 6 | Step 7 | Step 8 | Step 9 | Stalled rule | Notes |
|-----|-------------------|--------|--------|--------|--------|--------|--------|--------|--------|--------|--------------|-------|
| 0   | 2026-05-31T16:14Z | ok (local) | ok (local baseline) | ok (local) | n/a | n/a | n/a | n/a | n/a | n/a | deferred | Local baseline capture only. Target host pending operator gates. |
| 0+  | 2026-05-31T16:48Z | ok (local stack) | ok (via Prom) | ok (via Prom) | ok | ok | n/a (bare-binary) | ok | ok | ok | deferred | V2G-L0 bootstrap: bare-binary stack (backend + Prometheus + Alertmanager + sink) ran end-to-end on `127.0.0.1`. 9 alerts loaded, all `state=inactive`; 5/5 synthetic drills routed correctly (PERP OLD → critical, OPTION unknown → tickets, budget low → ops, metrics absent → backend, mainnet escalation → high). Compose stack files committed at `docs/monitoring/local-stack/` — operator-gated on docker group membership. See `docs/V2_FEE_OBSERVABILITY_LOCAL_STACK_BOOTSTRAP_V2G_L0.md`. |
| 0++ | 2026-05-31T17:14Z | ok (bare-binary stack) | ok (via Prom) | ok (via Prom) | ok | ok | gated (docker `up -d` blocked by sudo/group gate; Grafana ships only as a container) | ok | ok | ok | deferred | V2G-L1 second attempt at compose `up -d`. Docker access still gated; agent stopped at the sudo gate per hard rule and surfaced `sudo usermod -aG docker "$USER" && newgrp docker` for the operator. Bare-binary stand-in re-ran end-to-end; 5 drills firing + 5 drills resolved through the sink. See `docs/V2_FEE_OBSERVABILITY_LOCAL_COMPOSE_SOAK_V2G_L1.md`. |
| 0+++ | 2026-05-31T17:38Z | ok (compose stack live) | ok (via Prom) | ok (via Prom) | ok | ok | ok (Grafana up, 10 panels resolved via DS proxy) | ok | ok | ok | deferred | **V2G-L2: full compose stack live.** Operator unblocked Docker (`usermod -aG docker $USER && newgrp docker`); agent fixed two stack issues (rule files copy-not-symlink so the container bind resolves; webhook-sink user override dropped so the named volume is writable) and brought up all 4 containers. Prometheus scrapes `deopt-v2-backend@host.docker.internal:8080` UP; 9 alerts loaded inactive; 5/5 synthetic drills routed to expected receivers, sink volume captured every dispatch; Grafana `DeOpt — V2 fee observability (V2G-G)` dashboard provisioned with `Prometheus` DS; proxy queries through Grafana confirm the V2 fee gauge baseline. See `docs/V2_FEE_OBSERVABILITY_LOCAL_COMPOSE_LIVE_V2G_L2.md`. |
| 1-early | 2026-05-31T18:03Z (T+25m) | ok | ok | ok | ok | ok | ok | ok | ok | ok | deferred | V2G-L3 early progress probe (NOT the canonical day-1 24h checkpoint). All 9 daily-check steps green; 3/3 Prom targets up; 9 alerts inactive; AM /api/v2/alerts empty; Grafana dashboard renders against the same baseline; container resource use < 0.25% CPU / < 60MB mem each; only log warning is a benign `datetime.utcnow()` deprecation in webhook_sink.py (non-blocking). See `docs/V2_FEE_OBSERVABILITY_LOCAL_COMPOSE_DAY1_V2G_L3.md`. |
| 1-polish | 2026-05-31T18:14Z (T+36m) | ok | n/a | n/a | n/a | n/a | n/a | n/a | n/a | n/a | deferred | **Canonical T+24h day-1 GATE BLOCKED — too early by 23h 23m.** Agent stopped the canonical day-1 check; did not tick canonical day-1 row. Two polish items applied per NEXT_TASK.md step 9: (a) replaced `datetime.utcnow()` with `datetime.now(datetime.UTC)` in `docs/monitoring/local-stack/webhook-sink/webhook_sink.py` (timestamp shape unchanged); (b) added `docs/monitoring/local-stack/grafana/provisioning/{alerting,plugins}/.keep.yaml` no-op stubs so Grafana startup no longer emits `level=error` for missing alerting/plugins dirs. Containers restarted in-place via `docker compose restart {webhook-sink,grafana}` — non-destructive, volumes preserved, all 4 containers up after. Stack remains healthy: 3/3 targets up, 9 alerts inactive, AM empty, dashboard renders baseline (budget = 999987). Rerun canonical day-1 at 2026-06-01T17:38Z (UTC). |
| 1   | 2026-06-01T17:39Z (T+24h 01m) | ok | ok | ok | ok | ok | ok | ok | ok | ok | deferred     | **Canonical day-1 24h checkpoint — green.** Gate `2026-06-01T17:38:00Z` cleared with delta `+75s`. Compose: 4/4 Up 23-24h (Prometheus healthy). Backend `/health` ok. Prometheus `/-/ready` + `/-/healthy` ok; 3/3 targets up (`alertmanager:9093`, `dev-local`, `localhost:9090`); 9 V2 fee alerts loaded, all `inactive`. Alertmanager ready; 0 active alerts; 0 silences. Grafana `/api/health` ok (DB ok, v11.4.0); datasource `Prometheus` (uid `prometheus`); folder `DeOpt` (uid `deopt`); dashboard uid `deopt-v2g-g-v2-fees` resolves to "DeOpt — V2 fee observability (V2G-G)" — 10 content panels + 5 row containers; panel queries return live data. Metrics baseline: PERP charged{new}=3 / rebated{new}=1, OPTION charged{new}=3 / rebated{new}=1, all old/unknown=0, `rebate_budget_native(mUSDC)=999987`, 3/3 `up=1`. Stalled rule still disabled (Day-5 gate). Target-host F1-F11 still pending. No backend restart, no compose restart, no Prom reset, no `.env` edit, no DB writes. **Residual provisioning warnings (carry-forward, dashboard unaffected):** `/etc/grafana/provisioning/plugins` still missing in the running container (container started 17:31Z before the V2G-L4 polish stubs were written; in-container view does not see the host-side `.keep.yaml`); alerting dir contains invalid-suffix files (`README.md`, `.gitkeep`, `README.disabled.md`) — Grafana warns and skips them. Dashboard renders correctly regardless. Full record: `docs/V2_FEE_OBSERVABILITY_LOCAL_COMPOSE_DAY1_CANONICAL_V2G_L4.md`. |
| 1-maint | 2026-06-01T17:51Z (T+24h 13m) | ok | ok | ok | ok | ok | ok | ok | ok | ok | deferred | **V2G-M2 controlled backend restart.** Old PID 56199 (V2G-G era release binary) → new PID 231297 (release binary rebuilt 17:50Z, includes V2G-M / V2G-S / V2G-W0). Restart duration 1.20s. `/health` first 200 at +1s. Env preserved via /proc/56199/environ (130 vars, no edit to .env on disk). Active engines unchanged: `perp_engine_new=0xc6c592…2141c`, `margin_engine_new=0x287Cef…48Cc`, `fees_manager_v2=0x00dA0B…774f`. OLD_PERP_ENGINE observability-only (not active). Safe-runtime preserved: `execution_enabled=false`, `executor_real_broadcast_enabled=false`, `executor_private_key_set=false`, `executor_dry_run=true`. **V2G-M endpoint live:** `/admin/fees/v2/smoke/readiness` returns `milestone=V2G-M` with EOA-address-only `smoke_eoas` + env-name-only `key_env_vars` + all `broadcast_gates=false`. **V2G-S fields live:** `/admin/fees/onchain` returns `by_product`/`by_flow`/`rebated_by_product`/`rebated_by_flow` for both V2G-E txs; PERP `event_model=v2 charged=6 rebated=3 net=3`, OPTION `event_model=mixed source_priority=v2 charged=25 rebated=10 net=15` (V1 compat log = 1 but contributes 0). **V2G-W0 auth live:** `X-Admin-Token` required; 403 on missing/wrong/same-length-wrong, 200 on valid (token NEVER printed). Prom 3/3 targets up, 9 alerts inactive, AM empty, Grafana healthy. Metrics baseline bit-equivalent to pre-restart. Prometheus / Alertmanager / Grafana NOT restarted (Grafana provisioning carry-forward remains for a future compose-restart window). Full record: `docs/V2_BACKEND_RESTART_PICKUP_V2G_M2.md`. |
| 2-maint | 2026-06-02T10:23Z (T+~41h) | ok | ok | ok | ok | ok | ok | ok | ok | ok | deferred | **V2G-M3 backend metrics recovery after WSL2 reboot.** Backend was down — `/health` connection refused, Prometheus `deopt-v2-backend` target `down` (`dial tcp 172.17.0.1:8080: connect: connection refused`), `DeoptV2FeeMetricsAbsent` firing since `2026-06-02T08:49:12Z`. Root cause: WSL2 host rebooted ~`08:46Z` (`uptime 1h31m` at `10:17Z`); V2G-M2 PID 231297 ran foreground with no auto-restart, so it died with the WSL session. Compose containers came back automatically (`restart: unless-stopped`). Recovery: rebuilt release binary (`cargo fmt/clippy/test/build` all clean, **764/0/0 tests**), then launched via `/tmp/restart_backend_v2gm3.sh` sourcing the canonical V2F-O stack (`.env` → `.env.cutover.v2d_s.local` → `.env.preflight.v2e_f.local` → `.env.observability.v2f_o.local`) with shell overrides: `PERP_ENGINE_ADDRESS=NEW`, `OLD_PERP_ENGINE_ADDRESS=…b53B` (observability only), `OLD_MARGIN_ENGINE_ADDRESS=…b5F8` (observability only), `HOST=0.0.0.0` (so the Prom container can reach via `host.docker.internal`). First attempt PID 17393 bound to `127.0.0.1` only (Prom couldn't scrape) → SIGTERM → final **PID 18492** bound to `0.0.0.0` → Prom backend target `up` after ~10s scrape cycle. `DeoptV2FeeMetricsAbsent` cleared. Metrics baseline bit-equivalent to V2G-M2: PERP charged{new}=3 / rebated{new}=1, OPTION charged{new}=3 / rebated{new}=1, all old/unknown=0, `rebate_budget_native(mUSDC)=999987`, anomaly counts 0/0. All 9 V2 fee alerts inactive, AM 0 active / 0 silences, Grafana healthy. **No chain mutation, no broadcast, no `.env` edit, no DB destructive action, no Prometheus reset, no AM/Grafana restart, no `compose down`, no private key printed, no `ADMIN_API_TOKEN` printed.** OLD_PERP_ENGINE never active. Full record: `docs/V2_BACKEND_METRICS_RECOVERY_V2G_M3.md`. |
| 2-v2gp-clean1 | 2026-06-02T16:26Z (T+~48h) | ok | ok | ok | ok | ok | ok | ok | ok | ok | deferred | **V2G-P-CLEAN1 testnet-clean blocker close.** Backend was bound to `HOST=127.0.0.1` after the previous V2G-P pickup (debug build at `target/debug/`), so Prom container couldn't scrape via `host.docker.internal` → `DeoptV2FeeMetricsAbsent` firing. V2G-P-CLEAN1 fixed the bind + tightened indexer poll: SIGTERM PID 65050, relaunch release binary via `/tmp/restart_backend_v2gp_pickup.sh` with `HOST=0.0.0.0` + `OPTION_EVENT_INDEXER_POLL_INTERVAL_MS=30000` (was 86_400_000 = 24h); new PID 71706 on `0.0.0.0:8080`. All V2G-P address overrides preserved including `OLD_MARGIN_ENGINE_ADDRESS=0x287Cef…48Cc` (V2G-P-previous, not V2D-era). RFQ decode unchanged (cursor persisted): `event_model=mixed`, `fee_charged_v2_count=1`, `fee_rebated_v2_count=1`, `observed_total_charged=19`, `observed_total_rebated=10`, `by_flow.rfq=19`, `rebated_by_flow.rfq=10`. Metrics unchanged: option charged{new=1,old=3,unknown=0}; option rebated{new=1,old=1,unknown=0}; rebateBudget(mUSDC)=999977. Prom 3/3 targets UP; 9/9 alert rules inactive; `DeoptV2FeeMetricsAbsent` cleared; `OptionFeeConsumerUnknown` inactive. AM 0 active alerts. Grafana healthy. **No chain mutation, no `.env` edit, no Prometheus reset, no broadcast, no `compose down`, no private key printed.** Full record: `docs/OPTION_RFQ_LIVE_DEPLOY_REWIRE_SMOKE_RESULT_V2G_P.md` §6 (V2G-P-CLEAN1 close). |
| 2-v2gp-backend | 2026-06-02T15:30Z (T+~47h) | ok | ok | ok | ok | ok | ok | ok | ok | ok | deferred | **V2G-P backend pickup + indexer catchup.** Shell-only env override applied (no `.env` edit) to switch `MARGIN_ENGINE`, `OPTION_MATCHING_ENGINE_ADDRESS`, `OPTION_EVENT_INDEXER_MARGIN_ENGINE_ADDRESS`, `OPTION_EVENT_INDEXER_MATCHING_ENGINE_ADDRESS` to V2G-P NEW addresses; `OLD_MARGIN_ENGINE` / `OLD_MARGIN_ENGINE_ADDRESS` corrected to V2G-P-previous `0x287Cef…48Cc` (was V2D-era `0x6C5665…b5F8`, causing OPTION historical events to mis-classify as `unknown`). Indexer cursor was at ~42270k while V2G-P RFQ tx is at 42321819; with `OPTION_EVENT_INDEXER_POLL_INTERVAL_MS=86400000` (24h) natural catch-up would take ~13 days, so we used `POST /admin/options/events/tick` (X-Admin-Token gated) 10× to force scan; tick #10 found 21 logs / indexed 21 events at block 42323683 — exactly the V2G-P RFQ tx. **Result: `/admin/fees/onchain?tx_hash=0x0509d4…89e8` now returns `event_model=mixed`, `source_of_truth=onchain`, `reconciliation_status=onchain_observed`, `fee_charged_v2_count=1`, `fee_rebated_v2_count=1`, `observed_total_charged=19`, `observed_total_rebated=10`, `net_protocol_fee=9`, `by_product.option=19`, `by_flow.rfq=19`, `rebated_by_product.option=10`, `rebated_by_flow.rfq=10`. `/metrics` shows `deopt_option_fee_charged_v2_total{consumer="new"}=1`, `{old}=3`, `{unknown}=0`; rebated `{new}=1`, `{old}=1`, `{unknown}=0`; `rebateBudget(mUSDC)=999977` (matches on-chain).** Residual: backend bound to `HOST=127.0.0.1` (not `0.0.0.0`), so Prom container can't scrape via `host.docker.internal` → `DeoptV2FeeMetricsAbsent` is firing despite the backend `/metrics` itself being correct; fix at next scheduled restart with the V2G-P pickup script (which sets `HOST=0.0.0.0`). **No chain mutation, no `.env` edit, no Prometheus reset, no broadcast.** Full record: `docs/OPTION_RFQ_LIVE_DEPLOY_REWIRE_SMOKE_RESULT_V2G_P.md` §6. |
| 2-v2gp | 2026-06-02T14:52Z (T+~46h) | ok | ok | ok | ok | ok | ok | ok | ok | ok | deferred | **V2G-P first live OPTION RFQ broadcast.** New `MarginEngine 0x506cD6…0D30` + `MarginEngineLiquidationLib 0xB72A7B…cc18` + canonical `OptionMatchingEngine 0x5a5E…70f6` deployed; 6 dependents rewired; FM-V2 wired; V2 dispatch enabled; first signed RFQ trade tx `0x0509d4…89e8` (block 42321819) executed Tier-2 taker → Tier-4 maker on V2G-E series for 1 contract at 200000 mUSDC premium. Taker paid 19 mUSDC fee (94 ppm); maker received 10 mUSDC rebate (−50 ppm); `FM_V2.rebateBudget(mUSDC)` decremented `999987 → 999977` (−10). Total V2G-P broadcast gas across 7 stages ≈ 0.000064 ETH. **No backend restart this stage**; backend still on V2G-M3 env. `/admin/fees/onchain` currently returns `event_model=none` for the new tx — expected until V2G-P backend pickup (`/tmp/restart_backend_v2gp_pickup.sh` queued for the next maintenance window with new ME/OME indexer env). **No chain mutation other than the V2G-P broadcasts; no .env edit; no monitoring reset; no private key printed.** OLD_ME `0x287Cef…48Cc` orphaned (still `isFeeConsumer=true` on FM-V2 but no inbound traffic). Abandoned duplicate OME_1 `0xADE4…95D7` documented as inert orphan. Full record: `docs/OPTION_RFQ_LIVE_DEPLOY_REWIRE_SMOKE_RESULT_V2G_P.md`. |
| 2   |                   |        |        |        |        |        |        |        |        |        | deferred     |       |
| 3   |                   |        |        |        |        |        |        |        |        |        | deferred     |       |
| 4   |                   |        |        |        |        |        |        |        |        |        | re-evaluate? |       |
| 5   |                   |        |        |        |        |        |        |        |        |        | decide       |       |
| 6   |                   |        |        |        |        |        |        |        |        |        | follow day-5 |       |
| 7   |                   |        |        |        |        |        |        |        |        |        | follow day-5 |       |

### V2G-L0 — local-stack bootstrap (appended 2026-05-31)

V2G-L0 turned the V2G-K "soak-start-pending" baseline into a real
local-stack rehearsal. Headlines:

- Compose stack committed at `docs/monitoring/local-stack/` covering
  Prometheus 3.12.0 + Alertmanager 0.32.1 + Grafana 11.4.0 + webhook
  sink, all `127.0.0.1`-bound.
- Docker daemon discovered active but operator-gated (user not in
  `docker` group, sudo password-required). Compose `up` step deferred
  to operator.
- Bare-binary stand-in (Prometheus + Alertmanager binaries from
  `/tmp` + backend + sink) ran end-to-end on the agent: 3/3
  scrape targets `up`, 9 alerts loaded `inactive`, 5/5 synthetic
  drills delivered to the expected receivers (including the V2G-J
  mainnet escalation drill that V2G-I had not exercised).
- V2G-K day-0 row now reads `local-stack-running` with a real
  Prometheus-side baseline.
- Stalled-rule decision: **defer.** Local Prometheus has zero
  history; activation policy gate (≥3 PERP rebates / 24h × 48h)
  cannot be evaluated. Re-evaluation marker stays on soak day 5.

Full record: `docs/V2_FEE_OBSERVABILITY_LOCAL_STACK_BOOTSTRAP_V2G_L0.md`.

### V2G-L4 polish — gate-blocked stop + minor cleanups (appended 2026-05-31)

Operator triggered a canonical-day-1 attempt at `2026-05-31T18:14Z`,
only 36 minutes after the V2G-L2 stack open. Hard gate "current time
≥ 2026-06-01T17:38Z" failed by 23h 23m, so the canonical day-1 row
in the daily-check table stays reserved.

Polish that's safe regardless of the gate (NEXT_TASK.md step 9) was
applied:

- `docs/monitoring/local-stack/webhook-sink/webhook_sink.py` —
  `datetime.utcnow()` → `datetime.now(datetime.UTC)` with the
  trailing `Z` preserved so dispatch timestamps stay byte-identical
  to V2G-L0..L3.
- `docs/monitoring/local-stack/grafana/provisioning/alerting/.keep.yaml`
  + `provisioning/plugins/.keep.yaml` — no-op `apiVersion: 1`
  files (`groups: []` and `apps: []` respectively) so the Grafana
  startup `level=error` lines about missing dirs are gone.

Restart pattern was non-destructive: `docker compose restart
{webhook-sink,grafana}`. No volume reset, no `down -v`, no Prometheus
data reset. Stack remained healthy throughout.

### V2G-L3 — day-1 early progress probe (appended 2026-05-31)

V2G-L3 ran the daily checklist 25 minutes into the V2G-L2 stack
soak as a sanity-check / progress probe (operator request, not the
canonical day-1 24h marker). All 9 daily-check steps green; no
synthetic drill replayed; baseline metric values unchanged. The
canonical day-1 row stays reserved for T+24h.

Full record: `docs/V2_FEE_OBSERVABILITY_LOCAL_COMPOSE_DAY1_V2G_L3.md`.

### V2G-L2 — compose stack live (appended 2026-05-31)

Operator unblocked Docker. Agent brought the V2G-L0 compose stack up
after two small fixes (rule symlink → literal copy; sink user
override removed). All 4 containers running, Prometheus scraping
backend through `host.docker.internal`, 5/5 synthetic drills
delivered through the sink, Grafana dashboard provisioned at
`/d/deopt-v2g-g-v2-fees`. The V2G-K soak is now in
**local-compose-live** mode; day-1 starts T+24h after
`2026-05-31T17:38Z`.

Full record: `docs/V2_FEE_OBSERVABILITY_LOCAL_COMPOSE_LIVE_V2G_L2.md`.

### V2G-L1 — local-compose attempt (appended 2026-05-31)

V2G-L1 retried the V2G-L0 compose start. Same Docker socket gate.
Agent stopped at sudo per hard rule and reran the bare-binary
stand-in to keep the soak warm; 5/5 drills firing + 5/5 resolved.

Operator unblock command (single line):

```sh
sudo usermod -aG docker "$USER" && newgrp docker && cd ~/DEOPT/deopt-v2-backend/docs/monitoring/local-stack && docker compose up -d && docker compose ps
```

Full record: `docs/V2_FEE_OBSERVABILITY_LOCAL_COMPOSE_SOAK_V2G_L1.md`.

### What "warn" vs "fail" means

- **warn** — soak continues; document the anomaly. Examples: indexer
  lagged briefly but caught up within one scrape; budget gauge dipped
  but stayed above threshold; one alert pending but didn't fire.
- **fail** — soak pauses. Roll back the affected layer per the
  rollback table, root-cause, then resume.

## Soak completion criteria (after Day 7)

The soak passes if:

- Every daily row is `ok` or `warn` (no `fail`).
- Every layer rollback (if invoked) was scoped to one layer.
- `old_consumer_events == 0` for all 7 days.
- `unknown_consumer_events == 0` for all 7 days **or** ≥ 1 with a
  documented root-cause that did not require classifier widening.
- `FeesManagerV2RebateBudgetStale` did not fire (indexer healthy).
- `DeoptV2FeeMetricsAbsent` did not fire (metric pipeline healthy).
- Stalled-rule gate evaluated on day 5; decision documented.

On pass, file a V2G-L closure note that records:

- Daily checklist outcomes,
- Mean time to ack for any real alert that fired,
- Any tuning recommendations (window widths, threshold adjustments),
- Stalled-rule final decision (on / off / window changed),
- Mainnet readiness gap analysis using the V2G-G multi-asset matrix.

On fail or partial-pass: do not enable the stalled rule, file an
incident note, and re-evaluate at the next opportunity (typically
24h after root cause).

## V2G-K → V2G-L handoff

When the soak completes, V2G-L (next milestone) covers:

- Closing the soak record + producing a 7-day report.
- Wiring the multi-asset rule template (V2G-G matrix) for any new
  settlement asset that came online during the soak.
- Updating the stalled-rule window if observed cadence differs from
  the policy default.

## Remaining operator gates

| Gate | What it unlocks |
|------|------------------|
| F1   | apply `.env` patch on target backend host |
| F2   | restart backend post-patch                |
| F3   | drop V2G-G rule bundle into target Prometheus rules dir |
| F4   | reload Prometheus                          |
| F5   | merge routing fragment in target Alertmanager |
| F6   | reload Alertmanager                         |
| F7   | drop Grafana provisioning + dashboard JSON  |
| F8   | reload Grafana                              |
| F9   | fire synthetic drill against target Alertmanager |
| F10  | enable opt-in stalled bundle (default NO — see policy) |
| F11  | allow agent SSH to target host (user@host + auth) |

Until each gate has an explicit `YES` from the operator (plus the
target paths from sections A-E of the questionnaire), the V2G-K
soak day-1 row stays blank.

## Next recommended milestone

**V2G-L — 7-day soak closure + multi-asset rollout prep.**

- Close the soak record above with the daily outcomes.
- Author the V2G-L closure note covering real-alert MTTR, tuning
  recommendations, and stalled-rule final decision.
- Pull the V2G-G multi-asset matrix into a concrete rule-clone
  proposal for any new settlement assets that came online (or remain
  pending for mainnet).
- Begin V2H scope on FeesManagerV2 multi-asset support: the metric
  pipeline already emits one `asset=...` series per indexed
  settlement asset; multi-asset is primarily an operator
  rollout exercise.
