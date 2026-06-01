# V2G-M2 — Controlled Backend Restart + V2G-M / V2G-S / V2G-W0 Pickup

## Status

- Milestone: **V2G-M2** — single controlled backend restart after the
  V2G-L4 canonical day-1 24h gate cleared. The new release binary
  carries V2G-M (smoke-readiness endpoint), V2G-S (by_product /
  by_flow / rebated_by_product / rebated_by_flow), and V2G-W0
  (constant-time admin token compare). Monitoring stack untouched.
- Date: **2026-06-01T17:51Z**.
- Outcome:
  - Backend restarted **exactly once** (PID 56199 → PID 231297).
  - Restart duration: **1.20 s** (SIGTERM → port reusable → new
    process accepting `/health`).
  - V2G-M `/admin/fees/v2/smoke/readiness` endpoint **now live**.
  - V2G-S accounting fields (`by_product`, `by_flow`,
    `rebated_by_product`, `rebated_by_flow`) **live in
    `/admin/fees/onchain`**.
  - V2G-W0 constant-time admin token compare **live**: 403 on
    missing / wrong / same-length-wrong; 200 on valid.
  - Prometheus / Alertmanager / Grafana NOT restarted.
  - Metric baseline unchanged across the restart.
- Hard gates respected: no broadcast, no chain mutation, no
  `compose down -v`, no Prometheus reset, no DB destructive
  action, no `.env` edit, no private key / admin token printed.

---

## Phase 0 — Pre-maintenance snapshot

| Probe | Value |
|---|---|
| `date -u` | `2026-06-01T17:47:58Z` (~10 min after day-1 gate) |
| Old PID | **56199** alive 1d 00h 09m 04s |
| Cmd | `./target/release/deopt-v2-backend` |
| Release binary mtime | `2026-05-31 09:06 UTC` (pre-V2G-M era) → restart required |
| `git status --short` (backend) | clean (all V2G-M/S/W0 work was committed by the operator since the V2G-W0 doc was written) |
| `git status --short` (Solidity) | clean |
| `git status --short` (frontend) | only the V2G-V docs untracked |
| Backend `/health` | ok |
| Compose 4/4 | Up 24h |
| Prometheus / AM / Grafana liveness | 200 / 200 / 200 |

### Pre-restart metric baseline

| Metric | Value |
|---|---|
| `deopt_perp_fee_charged_v2_total{consumer="new"}` | 3 |
| `deopt_perp_fee_rebated_v2_total{consumer="new"}` | 1 |
| `deopt_option_fee_charged_v2_total{consumer="new"}` | 3 |
| `deopt_option_fee_rebated_v2_total{consumer="new"}` | 1 |
| all `consumer="old" / "unknown"` | 0 |
| `deopt_fees_manager_v2_rebate_budget_native{asset=0x6eae…412e}` | 999987 |
| `up` (3 targets) | 1 / 1 / 1 |

---

## Phase 1 — Pre-restart cargo gauntlet

| Command | Result |
|---|---|
| `cargo fmt --all -- --check` | ✅ |
| `cargo clippy --all-targets --all-features -- -D warnings` | ✅ |
| `cargo test --all-targets --all-features --no-fail-fast` | ✅ **735 / 0 / 0** |
| `cargo build --all-targets --all-features` | ✅ |
| `cargo build --release` | ✅ — rebuilt in 1m 02s |
| Release binary new mtime | `2026-06-01 17:50:35 UTC`, size 19 064 696 bytes (+39 KB vs the V2G-G binary — V2G-S buckets + V2G-W0 const-time helper) |

---

## Phase 2 — Controlled restart

**Method** (one-shot Python script `/tmp/restart_backend.py`):

1. Read the running process's env from `/proc/56199/environ`
   verbatim (130 env vars captured — DATABASE_URL, RPC_URL,
   MARGIN_ENGINE, FEES_MANAGER_V2, ADMIN_API_TOKEN,
   PERP_ENGINE / OLD_PERP_ENGINE, etc.). NO env value printed to
   the chat — only the count.
2. `os.kill(56199, SIGTERM)`.
3. Poll `os.kill(56199, 0)` every 200 ms until `ProcessLookupError`
   → old PID exited cleanly in 0.20 s.
4. `time.sleep(1.0)` — let the socket close fully.
5. `subprocess.Popen([release_binary], cwd=…, env=captured_env,
   stdout=log, stderr=STDOUT, start_new_session=True)`.
6. Log to `/tmp/deopt-backend-v2gm2.log`.

| Event | Value |
|---|---|
| Old PID | **56199** |
| New PID | **231297** |
| Restart duration (SIGTERM → new pid spawned) | **1.20 s** |
| `/health` first 200 after restart | +1 s |
| Captured env var count | 130 (unchanged from the running config) |
| Safe-runtime invariants preserved | `EXECUTION_ENABLED=false`, `EXECUTION_REAL_BROADCAST_ENABLED=false`, `OPTION_EXECUTION_BROADCAST_ENABLED=false`, `MAKER_KEY_ENV_SET=false`, `TAKER_KEY_ENV_SET=false`, `EXECUTOR_PRIVATE_KEY_SET=false`, `EXECUTOR_DRY_RUN=true` |
| Active engines | `perp_engine_new=0xc6c592…2141c`, `margin_engine_new=0x287Cef…48Cc`, `fees_manager_v2=0x00dA0B…774f` — all unchanged |
| OLD_PERP_ENGINE | configured for **observability only**, NEVER active (engine_new ≠ engine_old verified) |
| `.env` modified | ❌ no |

---

## Phase 3 — Post-restart endpoint verification

### `/health`
`{"ok":true,"service":"deopt-v2-backend"}` — first 200 at +1 s post-restart.

### `/admin/fees/v2/observability` (V2G-G surface)

Returned full payload (auth-gated with `X-Admin-Token`):

| Field | Value |
|---|---|
| `milestone` | `V2G-G` |
| `contracts.fees_manager_v2` | `0x00dA0B…774f` |
| `contracts.margin_engine_new` | `0x287Cef…48Cc` |
| `contracts.margin_engine_old` | `0x6C5665…5b5F8` (observability only) |
| `contracts.perp_engine_new` | `0xc6C592…2141c` |
| `contracts.perp_engine_old` | `0xB36395…2b53B` (observability only) |
| `metrics.perp_fee_charged_v2_by_consumer.new` | 3 (`old=0, unknown=0`) |
| `metrics.perp_fee_rebated_v2_by_consumer.new` | 1 (`old=0, unknown=0`) |
| `metrics.option_fee_charged_v2_by_consumer.new` | 3 (`old=0, unknown=0`) |
| `metrics.option_fee_rebated_v2_by_consumer.new` | 1 (`old=0, unknown=0`) |
| `metrics.fees_manager_v2_rebate_budget_native` | `{0x6eae…412e: 999987}` |
| `anomaly_totals.old_consumer_events` | 0 |
| `anomaly_totals.unknown_consumer_events` | 0 |

### `/admin/fees/v2/smoke/readiness` (V2G-M surface — now live)

| Field | Value |
|---|---|
| `milestone` | `V2G-M` |
| `engines.{fees_manager_v2, margin_engine_new, margin_engine_old, perp_engine_new, perp_engine_old}` | live addresses, all correctly classified |
| `smoke_eoas.tier4_maker_address` | `0x290bd12c…9274` (address only) |
| `smoke_eoas.tier2_taker_address` | `0x77ca9dd6…0020` (address only) |
| `smoke_eoas.key_env_vars.{maker, taker}` | env var **NAMES** only (`PERP_SMOKE_BUYER_PRIVATE_KEY` / `PERP_SMOKE_SELLER_PRIVATE_KEY`); **never values** |
| `broadcast_gates.execution_enabled` | `false` |
| `broadcast_gates.executor_real_broadcast_enabled` | `false` |
| `broadcast_gates.option_execution_broadcast_enabled` | `false` |
| `broadcast_gates.executor_private_key_set` | `false` |
| `broadcast_gates.maker_key_env_set` / `taker_key_env_set` | `false` / `false` |
| `broadcast_gates.executor_dry_run` | `true` |
| `dry_run_packets.{option, perp}` | both present |

No private key / signing material ever surfaced.

### `/admin/fees/onchain?tx_hash=<V2G-E PERP>`

| Field | Value |
|---|---|
| `event_model` | `v2` |
| `source_priority` | `""` (V2-only) |
| `fee_charged_v2_count` | 1 |
| `fee_rebated_v2_count` | 1 |
| `observed_total_charged` | 6 |
| `observed_total_rebated` | 3 |
| `net_protocol_fee` | 3 |
| `by_product` | `{"perp": "6"}` |
| `by_flow` | `{"orderbook": "6"}` |
| `by_side` | `{"taker": "6"}` |
| `by_trader` | Tier 2 taker `0x77ca9dd6…0020` |
| `by_recipient` | Timelock `0xa67f8e…b588` |
| `rebated_by_trader` | Tier 4 maker `0x290bd12c…9274` |
| `rebated_by_product` | `{"perp": "3"}` |
| `rebated_by_flow` | `{"orderbook": "3"}` |

### `/admin/fees/onchain?tx_hash=<V2G-E OPTION>`

| Field | Value |
|---|---|
| `event_model` | `mixed` |
| `source_priority` | `v2` |
| `trading_fee_event_count` (V1 compatibility log) | 1 |
| `fee_charged_v2_count` | 1 |
| `fee_rebated_v2_count` | 1 |
| `observed_total_charged` | 25 |
| `observed_total_rebated` | 10 |
| `net_protocol_fee` | 15 |
| `by_product` | `{"option": "25"}` |
| `by_flow` | `{"orderbook": "25"}` |
| `by_side` | `{"taker": "25"}` |
| `rebated_by_product` | `{"option": "10"}` |
| `rebated_by_flow` | `{"orderbook": "10"}` |

**No double-counting:** V1 trading_fee_event_count = 1 but V1 contributes **0** to `observed_total_charged` per the V2G-S source-priority policy (`mixed` ⇒ `source_priority = v2`).

---

## Phase 4 — V2G-S reconciliation fields confirmed live

All V2G-S buckets present on both endpoints:

| Bucket | Per-tx admin payload | Aggregate (over all events) |
|---|---|---|
| `by_product` | ✅ | ✅ |
| `by_flow` | ✅ | ✅ |
| `rebated_by_product` | ✅ | ✅ |
| `rebated_by_flow` | ✅ | ✅ |
| `observed_total_charged` (gross) | ✅ | ✅ |
| `observed_total_rebated` (rebates paid) | ✅ | ✅ |
| `net_protocol_fee` | ✅ | ✅ |

Replay-safety is structurally guaranteed by the V2G-S
`(model, tx_hash, log_index, source_contract)` dedup key — pinned
offline by the 11 `v2gs_*` tests; not re-tested here.

---

## Phase 5 — V2G-W0 security pickup confirmed live

`X-Admin-Token` enforcement is now ON (`ADMIN_API_REQUIRE_TOKEN=true`).

| Probe | Expected | Got |
|---|---|---|
| Missing header | 403 | **403** ✅ |
| Wrong token | 403 | **403** ✅ |
| Same-length wrong token | 403 | **403** ✅ |
| Valid token | 200 | **200** ✅ |

The constant-time helper sits at `src/admin.rs::constant_time_eq`.
This probe verifies behavioural parity; the timing property is a
code-review claim and remains a code-review claim (not a runtime
benchmark assertion).

ADMIN_API_TOKEN was **never** printed to chat output. The token
value was read from `/proc/231297/environ` into a shell variable
and passed to `curl -H` via `$ADMIN_TOKEN`; the chat shows only
the redacted prefix `v2d_s_lo...REDACTED` from the env-grep
diagnostic.

---

## Phase 6 — Prometheus / Alertmanager / Grafana post-restart

| Probe | Result |
|---|---|
| Prometheus targets | 3/3 up (alertmanager, deopt-v2-backend, prometheus) — `deopt-v2-backend dev-local` recovered after ~10 s scrape cycle |
| Prometheus rules | 9 V2 fee alerts loaded, all `inactive` |
| Alertmanager `/-/ready` | OK |
| Alertmanager `/api/v2/alerts` | 0 |
| Alertmanager `/api/v2/silences` | 0 |
| Grafana `/api/health` | `{"database":"ok","version":"11.4.0",...}` |

### Post-restart metric baseline

Bit-equivalent to pre-restart:

| Metric | Value |
|---|---|
| `deopt_perp_fee_charged_v2_total{consumer="new"}` | 3 |
| `deopt_perp_fee_rebated_v2_total{consumer="new"}` | 1 |
| `deopt_option_fee_charged_v2_total{consumer="new"}` | 3 |
| `deopt_option_fee_rebated_v2_total{consumer="new"}` | 1 |
| all `consumer="old"`/`"unknown"` | 0 |
| `deopt_fees_manager_v2_rebate_budget_native{asset=0x6eae…412e}` | 999987 |
| `up` | 1 / 1 / 1 |

No reset, no drift.

---

## Phase 7 — Frontend admin read-only check

`/admin` UI builds against the new endpoint surface. No code
changes:

| Command | Result |
|---|---|
| `npm run lint` | ✅ clean |
| `npx tsc --noEmit` | ✅ clean |
| `npm run build` | ✅ static prerender of `/`, `/admin`, `/_not-found` |

No wallet / write affordance added. The V2G-U
`<ProductionReadinessSection>` will now flip:

- `/admin/fees/v2/smoke/readiness` row → from "pending backend restart" to "live" on the next operator click of "Load V2 Fee Smoke Readiness".
- `/admin/fees/onchain` V2G-S panels → surface `by_product` / `by_flow` keys for the V2G-E tx queries.

---

## Phase 8 — Documents updated

- **New:** `docs/V2_BACKEND_RESTART_PICKUP_V2G_M2.md` (this file).
- **Updated:** `docs/V2_FEE_OBSERVABILITY_7_DAY_SOAK_V2G_K.md` —
  new maintenance row between Day-1 (canonical) and Day-2.
- **Updated:** `docs/V2_FEE_BACKEND_EXECUTOR_READINESS_V2G_M.md` —
  V2G-M2 pickup note appended.
- **Updated:** `docs/FEE_RECONCILIATION_IDEMPOTENCY_V2G_S.md` —
  V2G-M2 pickup note appended.
- **Updated:** `docs/ADMIN_TOKEN_CONSTANT_TIME_HARDENING_V2G_W0.md`
  — V2G-M2 pickup note appended.

---

## Phase 9 — Final validations

| Command | Result |
|---|---|
| `cargo fmt --all -- --check` | ✅ |
| `cargo clippy --all-targets --all-features -- -D warnings` | ✅ |
| `cargo test --all-targets --all-features --no-fail-fast` | ✅ **735 / 0 / 0** |
| `cargo build --all-targets --all-features` | ✅ |
| `cargo build --release` | ✅ (rebuilt before restart) |
| `promtool check rules v2_fee_alerts.bundle.yml` | ✅ 9 rules |
| `promtool check rules v2_fee_alerts.stalled.yml` | ✅ 1 rule |
| `promtool test rules v2_fee_alerts.test.yml` | ✅ |
| `amtool check-config v2_fee_routing.example.yml` | ✅ 6 receivers |
| `python3 -m json.tool grafana/v2_fee_observability_dashboard.json` | ✅ |
| `docker compose config -q` | ✅ |
| `npm run lint` (frontend) | ✅ |
| `npx tsc --noEmit` (frontend) | ✅ |
| `npm run build` (frontend) | ✅ |

(promtool/amtool re-confirmed during V2G-L4 canonical close 8 min before this restart — not re-run here per task §9.)

---

## Hard-gate compliance

| Gate | Honoured |
|---|---|
| No broadcast | ✅ |
| No tx submission | ✅ |
| No chain mutation | ✅ |
| No DB row deletion | ✅ |
| No `.env` edit | ✅ (env captured from `/proc/56199/environ`, not from file) |
| No private key printed | ✅ |
| No ADMIN_API_TOKEN printed | ✅ (only the 8-char redacted prefix in a diagnostic env-grep) |
| No Prometheus reset | ✅ |
| No `compose down -v` | ✅ |
| No Prometheus / AM / Grafana restart | ✅ |
| No Merkle root / rebate budget change | ✅ |
| No deploy / rewire | ✅ |
| OLD_PERP_ENGINE not used as active | ✅ (verified — `perp_engine_new=0xc6c592…` is the active engine) |

---

## Remaining blockers

1. **Provisioning carry-forward** (Grafana): `/etc/grafana/provisioning/plugins` missing in the running container, alerting dir has invalid-suffix breadcrumbs. Grafana NOT restarted in V2G-M2 — these resolve at the next compose-restart maintenance window.
2. **OPTION RFQ live deploy (V2G-P)** — V2G-O bytecode ready; broadcast pending.
3. **ProtocolFeeVault live deploy (V2G-R5)** — preceded by V2G-R3.
4. **V2G-W1 per-identity JWT** — design pinned in V2G-V; implementation pending.
5. **V2G-W2 audit log** — pending.
6. **V2G-W3 Next.js SSR proxy + drop sessionStorage token** — pending.
7. **Target-host monitoring cutover (F1-F11)** — pending.
8. **Day-5 stalled-rule re-evaluation gate** — not yet reached.

---

## Next recommended milestone

**Day-2 soak row** at `2026-06-02T17:38Z`. Same V2G-L4 checklist
with one new acceptance row: **`/admin/fees/v2/smoke/readiness`
returns `milestone=V2G-M`** (now true post-V2G-M2). The
V2G-M2-introduced auth gate (X-Admin-Token required) carries
forward; operators must include the token when probing
`/admin/*` endpoints during Day-2.
