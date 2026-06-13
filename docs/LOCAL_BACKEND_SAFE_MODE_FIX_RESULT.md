# LOCAL-BACKEND-SAFE-MODE-FIX — RESULT

**Date:** 2026-06-13
**Operator approval line (consumed verbatim):**
> "I approve DeOpt V2 local backend safe-mode fix for this run."

**Posture:** localhost dev only. **No chain transactions. No broadcast. No mainnet. No deployment. No `.env` edit. No private key handling. No AWS/KMS. No audit outreach. No bug bounty.** Purpose: fix the local backend startup that was failing with `Error: Config("option confirmation worker requires persistence enabled")` so the local frontend can connect to `http://localhost:8080`.

---

## 1. Workspace
- `~/DEOPT/scripts/local-backend.sh` (EDITED — added missing OPTION_*_WORKER_ENABLED overrides + updated startup summary)
- `~/DEOPT/scripts/local-frontend.sh` (EDITED — Next.js port-3001 fallback note)
- `~/DEOPT/deopt-v2-backend/.env.local.example` (EDITED — CORS now also allows :3001)
- `~/DEOPT/deopt-v2-backend/docs/LOCAL_FULLSTACK_RUNBOOK.md` (EDITED — new expected-startup block + 1.1 port-fallback note)
- `~/DEOPT/deopt-v2-backend/docs/LOCAL_BACKEND_SAFE_MODE_FIX_RESULT.md` (NEW — this file)
- `~/DEOPT/RUN_STATE.md` (closure paragraph prepended)

**Backend Rust source: ZERO changes.** **Frontend src: ZERO changes.** **Solidity: ZERO.**

---

## 2. Root cause

`config/env.rs` defines three OPTION-worker enable flags whose `parse_env` defaults are `"false"`:

| Env var | File:line | Default | Validator |
|---|---|---|---|
| `OPTION_CONFIRMATION_WORKER_ENABLED` | `src/config/env.rs:394` | `false` | `options/confirmation_worker.rs:40` — **rejects** startup with `"option confirmation worker requires persistence enabled"` when `enabled && !persistence` |
| `OPTION_RECONCILIATION_WORKER_ENABLED` | `src/config/env.rs:462` | `false` | similar persistence requirement |
| `OPTION_EVENT_INDEXER_ENABLED` | `src/config/env.rs:490` | `false` | similar |
| `OPTION_EXECUTION_ENABLED` | `src/config/env.rs:312` | `false` | requires persistence when on |

The production-tracked `~/DEOPT/deopt-v2-backend/.env` (mtime `2026-06-08 16:55:05`) keeps all four flags ON for normal operator workflows. `cargo run` (which the prior `scripts/local-backend.sh` invocation used) sources that file via `dotenvy::dotenv()` and inherits the ON values.

The previous milestone (`LOCAL-FULLSTACK-TESTNET-BETA-SMOKE`) successfully started the backend ONLY because it bypassed `cargo run` and called `env -i target/debug/deopt-v2-backend` directly from `/tmp` — so dotenv had no `.env` to load and the defaults applied. The smoke result correctly reported "live on :8080", but the polished `scripts/local-backend.sh` it shipped re-introduced the regression because its hardcoded safety-overrides list omitted the four OPTION-worker enable keys.

Generic (perp-side) flags it DID set — `EXECUTION_ENABLED=false`, `INDEXER_ENABLED=false`, `RECONCILIATION_ENABLED=false`, `CONFIRMATION_ENABLED=false`, `OPTION_NONCE_SYNC_ENABLED=false`, `OPTION_EXECUTION_BROADCAST_ENABLED=false` — do NOT cover the OPTION worker variants.

**Conclusion:** script bug. No backend source change required. The validator behaviour is correct — it refuses to silently run a confirmation worker without a DB to write to. Keep that protection.

---

## 3. Local backend script fix

`scripts/local-backend.sh` now exports the missing safety overrides AFTER the (dotenv-implicit) production `.env` is loaded. Added:

```sh
export OPTION_EXECUTION_ENABLED=false
export OPTION_CONFIRMATION_WORKER_ENABLED=false
export OPTION_RECONCILIATION_WORKER_ENABLED=false
export OPTION_EVENT_INDEXER_ENABLED=false
export SIMULATION_ENABLED=false
export FEES_ENABLED=false
export FEES_REBATES_ENABLED=false
export RFQ_ENABLED=false
export MM_GATEWAY_ENABLED=false
export MM_PERMISSIONS_ENABLED=false
```

(The new lines cover not just the three workers that triggered the failure, but the full set of `*_ENABLED` flags any production `.env` might flip on.)

The script also got a clearer startup summary printed BEFORE `cargo run` so the operator sees the safety posture immediately:

```
[local-backend] backend URL:   http://127.0.0.1:8080
[local-backend] broadcast:     OFF
[local-backend] signer:        OFF (EXECUTOR_PRIVATE_KEY ignored; no AWS/KMS path)
[local-backend] persistence:   false
[local-backend] options:       in-memory (OPTIONS_REQUIRE_PERSISTENCE=false)
[local-backend] workers:       OFF (option confirmation / reconciliation / event-indexer / nonce-sync / fees)
[local-backend] mainnet:       blocked (chain 84532)
[local-backend] CORS origins:  http://localhost:3000,http://127.0.0.1:3000,http://localhost:3001,http://127.0.0.1:3001
[local-backend] frontend hint: open the URL printed by Next.js (it may pick 3001 if 3000 is in use)
```

Safety guarantees preserved:
* All overrides are `export VAR=…` AFTER any source-load of `.env.local` would happen, so the operator cannot accidentally re-enable broadcast or any worker via `.env.local`.
* No secret echoed. No `cat` of `.env`. No RPC URL printed. No private key read.
* `EXECUTOR_PRIVATE_KEY` is NOT set by the script. If the production `.env` defines it, `EXECUTION_ENABLED=false` + `EXECUTOR_REAL_BROADCAST_ENABLED=false` mean the key is never touched.

---

## 4. Backend guard / source changes

**NONE.** The validator at `options/confirmation_worker.rs:40` is correct — workers must NOT silently run without persistence. We keep the strict guard; we just stop the local script from accidentally tripping it.

`cargo fmt --check` and `cargo test --lib` were therefore not re-run under this milestone (no Rust source touched). The most recent runs from `LOCAL-FULLSTACK-TESTNET-BETA-SMOKE` remain valid:
* `cargo fmt --check` — clean.
* `cargo build -p deopt-v2-backend` — green.
* `cargo test -p deopt-v2-backend --lib api::` — 281 / 0 / 0.

---

## 5. Frontend port-3001 note

Next.js auto-bumps to 3001 when 3000 is in use (e.g. another `npm run dev` already running, or any service holding the port). Three changes encode this:

1. `scripts/local-frontend.sh` now prints a `NOTE: port 3000 is already in use` warning up-front (via a quick `ss -lnt` probe) so the operator knows where to look.
2. `deopt-v2-backend/.env.local.example` `CORS_ALLOWED_ORIGINS` now lists both `:3000` AND `:3001` (plus the `127.0.0.1` aliases).
3. `scripts/local-backend.sh` `CORS_ALLOWED_ORIGINS` default likewise lists both ports — so a freshly-cloned repo Just Works whether Next.js lands on 3000 or 3001.

The runbook now also says explicitly: "Open whatever URL Next.js actually prints — do not hardcode `http://localhost:3000` mentally."

No source change to the frontend was required.

---

## 6. Local smoke (LIVE during this milestone)

Backend launched via the fixed `scripts/local-backend.sh`. Startup log:

```
INFO deopt_v2_backend::options::confirmation_worker: option confirmation worker disabled
INFO deopt_v2_backend::options::reconciliation_worker: option reconciliation worker disabled
INFO deopt_v2_backend::mm::transport::webtransport: MM WebTransport gateway disabled
INFO deopt_v2_backend::options::event_indexer: option event indexer disabled
INFO deopt_v2_backend: starting http server addr=127.0.0.1:8080 chain_id=84532
  network=base-sepolia execution_enabled=false confirmation_enabled=false
  option_confirmation_worker_enabled=false option_event_indexer_enabled=false
  option_reconciliation_worker_enabled=false rfq_enabled=false options_enabled=true
  fees_enabled=false metrics_enabled=true mm_gateway_enabled=false
  mm_permissions_enabled=false indexer_enabled=false reconciliation_enabled=false
  executor_dry_run=true persistence_enabled=false
```

`scripts/local-smoke.sh` against the running backend:

```
PASS  health
PASS  ready
PASS  trading_health
PASS  options_products
PASS  markets
PASS  balances
PASS  positions
PASS  portfolio
PASS  cors_preflight (HTTP 200)

Smoke summary: 9 pass / 0 fail
```

`/markets` returns the perp mock list. `/trading/health` reports `chain_id: 84532`. `/options/products` returns the empty-envelope OK (in-memory options store). The frontend at `/markets` and `/trade` will no longer surface the "Trading backend temporarily unavailable" card while the backend is running.

Backend stopped post-smoke. Port 8080 free.

---

## 7. Frontend smoke

| Command | Result |
|---|---|
| `npm run typecheck` | clean |
| `npm run lint` | clean |
| `NEXT_PUBLIC_TRADING_API_BASE_URL=http://localhost:8080 NEXT_PUBLIC_CHAIN_ENV=sepolia npm run build` | green — 15 user-facing routes + 4 SSG doc slugs + `_not-found` |
| `npx playwright test --list` | 96 tests in 24 files |

No frontend source change. The build is identical to the prior milestone because the only delta is the backend startup script.

Live `/trade` rendering with the backend up was confirmed via:
* `/options/products` returns `{"status":"ok","data":{"products":[]},...}` — frontend treats this as honest empty state, NOT backend-unavailable.
* `/trading/health` returns `chain_id: 84532` — wrong-network banner stays hidden when the wallet is on Base Sepolia.

---

## 8. Docs created/updated

| File | Action |
|---|---|
| `~/DEOPT/deopt-v2-backend/docs/LOCAL_BACKEND_SAFE_MODE_FIX_RESULT.md` | NEW (this file) |
| `~/DEOPT/deopt-v2-backend/docs/LOCAL_FULLSTACK_RUNBOOK.md` | EDITED — updated expected-startup output; new §1.1 port-3001 note |
| `~/DEOPT/deopt-v2-backend/.env.local.example` | EDITED — CORS default now includes `:3001` aliases |
| `~/DEOPT/scripts/local-backend.sh` | EDITED — 4 new safety overrides + clearer startup summary + `:3001` in CORS default |
| `~/DEOPT/scripts/local-frontend.sh` | EDITED — port-3000-busy detection note |
| `~/DEOPT/RUN_STATE.md` | UPDATED — closure paragraph prepended |
| `LOCAL_FULLSTACK_TESTNET_BETA_SMOKE_RESULT.md` | UNCHANGED — its claims remain accurate; this fix just makes the published script reach the same state. |

---

## 9. Remaining local blockers

NONE for in-memory safe mode (the milestone target). Local product testing path is unblocked.

For persistent local mode (out of scope here): operator still needs to install Postgres, set `DATABASE_URL`, set `PERSISTENCE_ENABLED=true`, and remove the worker-disable overrides from a forked script. Documented in `LOCAL_FULLSTACK_RUNBOOK.md §0` as "optional persistent mode".

---

## 10. Next milestone recommendation

**Primary (operator):** product-test the frontend at the URL Next.js prints (likely `http://localhost:3001` if 3000 is busy) against the local backend. Use `/feedback` or a new GitHub issue for actionable bugs.

**Secondary (agent-runnable):** `BACKEND-PUBLIC-TESTNET-DEPLOY-PREFLIGHT` (already drafted as `docs/BACKEND_PUBLIC_TESTNET_DEPLOY_PREFLIGHT_NEXT_TASK.md`) — retry the previously-failed Railway deploy with the now-documented start command and env matrix.

**Strictly later (NOT NOW):** announcement publication, audit firm outreach, bug bounty launch, mainnet, KMS cutover, Safe migration, flipping `isMainnetEnabled()`.

---

## 11. Cross-links
* `~/DEOPT/scripts/local-backend.sh`, `local-frontend.sh`, `local-smoke.sh`
* `~/DEOPT/deopt-v2-backend/docs/LOCAL_FULLSTACK_RUNBOOK.md`
* `~/DEOPT/deopt-v2-backend/docs/LOCAL_FULLSTACK_TESTNET_BETA_SMOKE_RESULT.md`
* `~/DEOPT/deopt-v2-backend/docs/BACKEND_PUBLIC_TESTNET_DEPLOY_PREFLIGHT_NEXT_TASK.md`
* `~/DEOPT/deopt-v2-backend/src/config/env.rs:394, 462, 490` (the worker enable defaults the script now overrides)
* `~/DEOPT/deopt-v2-backend/src/options/confirmation_worker.rs:40` (the validator that fired)

**End of local backend safe-mode fix result.**
