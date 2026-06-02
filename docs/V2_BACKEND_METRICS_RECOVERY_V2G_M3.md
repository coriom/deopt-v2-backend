# V2G-M3 — Backend Metrics Recovery After WSL2 Reboot

## Status

- Milestone: **V2G-M3** — controlled backend bring-up after the
  V2G-M2 process (PID 231297) was killed by a WSL2 host reboot.
  Mirrors the V2G-M2 safe-runtime posture; no chain mutation, no
  broadcast, no DB destructive action, no Prometheus reset.
- Date: 2026-06-02.
- Outcome:
  - Backend back online — PID **18492**, `/health` 200.
  - Prometheus `deopt-v2-backend` target recovered to `up`.
  - `DeoptV2FeeMetricsAbsent` cleared (no firing alerts).
  - Metric baseline bit-equivalent to V2G-M2 close.
  - Two corrective env overrides applied vs the prior boot:
    - `HOST=0.0.0.0` (so Prom container can scrape via the
      `host.docker.internal` → 172.17.0.1 gateway).
    - `OLD_MARGIN_ENGINE_ADDRESS=0x6C5665…b5F8` (so the
      observability `margin_engine_old` field is populated as in
      V2G-M2).

---

## 1. Failure symptom

| Probe | Pre-recovery state |
|---|---|
| `curl http://127.0.0.1:8080/health` | connection refused |
| `ss -ltnp :8080` | no listener |
| Prometheus `deopt-v2-backend` target | `down` (`lastError: dial tcp 172.17.0.1:8080: connect: connection refused`) |
| `DeoptV2FeeMetricsAbsent` | firing since `2026-06-02T08:49:12Z` |
| Compose stack (alertmanager, grafana, prometheus, webhook-sink) | all `Up 59 min` (auto-restarted by docker) |

## 2. Likely / root cause

WSL2 was rebooted at ~`08:46Z` (UTC). Inferred from `uptime` =
`1h31m` taken at `10:17Z`, plus `who -b` reporting boot
`2026-06-02 10:49` in local time (UTC+2). The Prometheus alert
fired exactly 3 min after boot — matches the standard `for: 3m`
empty-scrape duration.

The V2G-M2 backend process (PID 231297) was launched
interactively in the previous WSL session with no log redirection
and no auto-restart wrapper. WSL2 reboot → process killed → no
recovery mechanism brought it back. Compose containers came back
because docker-compose has `restart: unless-stopped` policies.

No backend log file exists (foreground launch). No coredump
written. Exit cause is not recoverable from artifacts.

## 3. PIDs

| Era | PID | State |
|---|---|---|
| V2G-M | 56199 | killed by V2G-M2 SIGTERM at 2026-06-01T17:51Z |
| V2G-M2 | 231297 | killed by WSL2 reboot at ~2026-06-02T08:46Z |
| V2G-M3 first attempt | 17393 | bound to 127.0.0.1 only; Prom couldn't reach via Docker bridge → SIGTERM'd after diagnosis |
| **V2G-M3 final** | **18492** | bound to 0.0.0.0 → reachable from Prometheus container |

## 4. Restart method

Single helper script: `/tmp/restart_backend_v2gm3.sh`. Shape
(secrets redacted):

```bash
set -a
. ./.env
. ./.env.cutover.v2d_s.local
. ./.env.preflight.v2e_f.local
. ./.env.observability.v2f_o.local
set +a

# Hard gate — never use OLD as the active engine.
export PERP_ENGINE_ADDRESS=0xc6C592…2141c    # NEW
export OLD_PERP_ENGINE_ADDRESS=0xB36395…b53B  # observability only
export OLD_MARGIN_ENGINE_ADDRESS=0x6C5665…b5F8  # observability only

# Belt-and-suspenders — every broadcast path off.
export EXECUTION_ENABLED=false
export EXECUTOR_DRY_RUN=true
export EXECUTOR_REAL_BROADCAST_ENABLED=false
export OPTION_EXECUTION_BROADCAST_ENABLED=false

# Bind to all interfaces so the Prom container's
# `host.docker.internal` (172.17.0.1) gateway can reach the port.
export HOST=0.0.0.0

setsid nohup ./target/release/deopt-v2-backend \
  >>/tmp/deopt-backend-v2gm3.log 2>&1 < /dev/null &
```

Notes:

- The .env files are sourced (`set -a … set +a`) verbatim; the
  files themselves are **never edited**.
- `ADMIN_API_TOKEN` is supplied by sourcing `.env.cutover.v2d_s.local`;
  the script never echoes it. The token was read from
  `/proc/<pid>/environ` once into a shell variable to drive the
  admin endpoint probes; only its length (23) was printed.
- No private key is exported — `BUYER_PRIVATE_KEY`,
  `SELLER_PRIVATE_KEY`, `EXECUTOR_PRIVATE_KEY` are present in the
  `.env` keyspace but the safety gates explicitly mark them not
  used at runtime (`executor_dry_run=true`,
  `executor_private_key_set=false`, etc.).
- `start_new_session=true` (via `setsid`) detaches the process
  from this shell so a Claude Code session exit won't kill it.

## 5. Backend endpoint recovery

All four targeted endpoints return 200 with the expected payload:

| Endpoint | HTTP | Notable fields |
|---|---|---|
| `/health` | 200 | `{"ok":true,"service":"deopt-v2-backend"}` |
| `/admin/fees/v2/observability` | 200 | `milestone=V2G-G`; `fees_manager_v2=0x00dA0B…774f`; `perp_engine_new=0xc6C592…2141c`; `perp_engine_old=0xB36395…b53B`; `margin_engine_new=0x287Cef…48Cc`; `margin_engine_old=0x6C5665…b5F8`; `anomaly_totals.old_consumer_events=0`; `unknown_consumer_events=0` |
| `/admin/fees/v2/smoke/readiness` | 200 | `milestone=V2G-M`; all `broadcast_gates.*=false`; `executor_dry_run=true`; `smoke_eoas.*` addresses only (no keys); `key_env_vars` carries env-var NAMES only |
| `/admin/fees/onchain?tx_hash=…PERP` | 200 | `event_model=v2`; charged=6 / rebated=3 / net=3; `transactions[0].by_product={"perp":"6"}`; `by_flow={"orderbook":"6"}`; `rebated_by_product={"perp":"3"}`; `rebated_by_flow={"orderbook":"3"}` |
| `/admin/fees/onchain?tx_hash=…OPTION` | 200 | `event_model=mixed`; `source_priority=v2`; charged=25 / rebated=10 / net=15; `transactions[0].by_product={"option":"25"}`; `by_flow={"orderbook":"25"}`; `rebated_by_product={"option":"10"}`; `rebated_by_flow={"orderbook":"10"}` |

The V2G-S `by_product` / `by_flow` / `rebated_by_product` /
`rebated_by_flow` keys live inside `transactions[]` (per-tx
context), matching the V2G-M2 layout.

## 6. Metrics baseline

`curl http://127.0.0.1:8080/metrics` returns:

```text
deopt_perp_fee_charged_v2_total{consumer="new"}     3
deopt_perp_fee_charged_v2_total{consumer="old"}     0
deopt_perp_fee_charged_v2_total{consumer="unknown"} 0
deopt_perp_fee_rebated_v2_total{consumer="new"}     1
deopt_perp_fee_rebated_v2_total{consumer="old"}     0
deopt_perp_fee_rebated_v2_total{consumer="unknown"} 0
deopt_option_fee_charged_v2_total{consumer="new"}     3
deopt_option_fee_charged_v2_total{consumer="old"}     0
deopt_option_fee_charged_v2_total{consumer="unknown"} 0
deopt_option_fee_rebated_v2_total{consumer="new"}     1
deopt_option_fee_rebated_v2_total{consumer="old"}     0
deopt_option_fee_rebated_v2_total{consumer="unknown"} 0
deopt_fees_manager_v2_rebate_budget_native{asset="0x6eae…412e"} 999987
```

Bit-equivalent to the V2G-M2 post-restart baseline. No drift, no
data loss — V2 fee events live in Postgres + indexed from chain.

## 7. Prometheus / Alertmanager / Grafana recovery

| Probe | State |
|---|---|
| Prometheus `/-/ready` | OK |
| Prometheus targets `deopt-v2-backend` | `up` (recovered after the 0.0.0.0 rebind + ~10 s scrape cycle) |
| Prometheus targets `alertmanager` | `up` |
| Prometheus targets `prometheus` | `up` |
| Prometheus `/api/v1/rules` alerting groups | 9 alerting rules; **9 inactive / 0 pending / 0 firing** |
| `DeoptV2FeeMetricsAbsent` | resolved (alert no longer present in `/api/v1/alerts`) |
| Alertmanager `/-/ready` | OK |
| Alertmanager `/api/v2/alerts` | 0 active |
| Alertmanager `/api/v2/silences` | 0 active, 0 total |
| Grafana `/api/health` | `{"database":"ok","version":"11.4.0",…}` |
| Compose containers | alertmanager / grafana / prometheus / webhook-sink — all Up ~1h, no restart triggered by this milestone |

## 8. No chain mutation / no DB destructive action / no Prometheus reset

| Hard gate | Honoured |
|---|---|
| No broadcast | ✅ — `EXECUTION_ENABLED=false`, `EXECUTOR_REAL_BROADCAST_ENABLED=false`, `OPTION_EXECUTION_BROADCAST_ENABLED=false` |
| No tx submission | ✅ |
| No chain mutation | ✅ |
| No deploy / rewire | ✅ |
| No Merkle root change | ✅ |
| No rebate budget change | ✅ (still 999987) |
| No feeRecipient / rebateFundingAccount change | ✅ |
| No `.env` edit | ✅ (only shell-level `export` overrides in `/tmp/restart_backend_v2gm3.sh`) |
| No private key printed | ✅ |
| No `ADMIN_API_TOKEN` printed | ✅ (only its character length, never the value) |
| No DB row deletion | ✅ (`sqlx::postgres::notice: relation "_sqlx_migrations" already exists, skipping` on startup is benign idempotent migration check) |
| No Prometheus reset / restart | ✅ (the 3 monitoring containers were already up from yesterday — uptime ~1h, no compose action this milestone) |
| No Alertmanager / Grafana restart | ✅ |
| `compose down -v` | ❌ never invoked |
| OLD_PERP_ENGINE not active | ✅ — `perp_engine_new=0xc6C592…2141c` is the active engine; OLD is observability-only |

## 9. Remaining risks

1. **`HOST=127.0.0.1` mismatch in `.env`** — the live config now
   has `HOST=0.0.0.0` as a shell override, but `.env` still says
   `HOST=127.0.0.1`. Any future operator launching without
   re-applying the override will reproduce the same Prom
   scrape-down failure. Recommended follow-up: V2G-M3.1 small PR
   that either (a) flips `.env.example` to document the
   override or (b) adds a `.env.local.host-bind` overlay file.
2. **No process supervisor** — the backend remains a foreground
   binary started via `setsid nohup`. A second WSL2 reboot will
   reproduce the failure. Recommended follow-up: systemd unit or
   a docker-compose service.
3. **Foreground log path** — `/tmp/deopt-backend-v2gm3.log` is a
   tmpfs file (lost on WSL reboot). For long-term operation a
   persistent log path (`/var/log/deopt/` or systemd journal)
   would help post-mortem the next exit.
4. **OLD_MARGIN_ENGINE name mismatch** — `.env.cutover.v2d_s.local`
   sets `OLD_MARGIN_ENGINE` but the config loader reads
   `OLD_MARGIN_ENGINE_ADDRESS`. V2G-M3 worked around this with a
   shell override. Same V2G-M3.1 PR should fix the env file.

None of the above blocks any pending live gate; they are
operator-ergonomic hygiene.

## 10. Cross-links

- V2G-M2 prior restart: `V2G_BACKEND_RESTART_PICKUP_V2G_M2.md`.
- V2G-RX.1 (most recent offline milestone): `../docs/PROTOCOL_FEE_VAULT_GUARDIAN_HARDENING_V2G_RX1.md` (in deopt-v2-sol).
- 7-day soak log: `V2_FEE_OBSERVABILITY_7_DAY_SOAK_V2G_K.md` (this milestone appends a maintenance row).
- V2G-AUDIT0 gate decision: `~/DEOPT/AUDIT_GATE_DECISION_V2G_AUDIT0.md`.
- Operator helper: `/tmp/restart_backend_v2gm3.sh` (tmpfs — not committed; reproduce via this doc's §4 if needed).
