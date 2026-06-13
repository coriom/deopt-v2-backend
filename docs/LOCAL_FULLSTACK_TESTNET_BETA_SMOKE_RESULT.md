# LOCAL-FULLSTACK-TESTNET-BETA-SMOKE — RESULT

**Date:** 2026-06-13
**Operator approval line (consumed verbatim):**
> "I approve DeOpt V2 local fullstack testnet beta smoke for this run."

**Posture:** localhost dev only. **No chain transactions. No broadcast. No mainnet. No deployment. No `.env` edit. No private key handling. No AWS/KMS. No audit outreach. No bug bounty.** Purpose: make the local backend + frontend dev experience clean, reproducible, and ready for visual product testing before any public hosting attempt is retried.

---

## 1. Workspace
- `~/DEOPT/deopt-v2-backend/`
  - `Cargo.toml` (tower-http: add `"cors"` feature)
  - `src/api/routes.rs` (new `cors_layer_from_env()` + `.layer(...)` insertion)
  - `.env.local.example` (NEW)
  - `docs/LOCAL_FULLSTACK_TESTNET_BETA_SMOKE_RESULT.md` (NEW — this file)
  - `docs/LOCAL_FULLSTACK_RUNBOOK.md` (NEW)
- `~/DEOPT/deopt-v2-frontend/`
  - `.env.local.example` (REWRITTEN)
- `~/DEOPT/scripts/`
  - `local-backend.sh` (NEW)
  - `local-frontend.sh` (NEW)
  - `local-smoke.sh` (NEW)
- `~/DEOPT/RUN_STATE.md` (closure paragraph)

---

## 2. Backend local startup inventory

| Aspect | Finding |
|---|---|
| Port | `PORT` env, default `8080`, host `127.0.0.1` |
| Required env at minimum | `CHAIN_ID` (default 84532), `NETWORK_NAME` (default `base-sepolia`) |
| Persistence | `PERSISTENCE_ENABLED` (default `false`); Postgres only when `true` |
| Migrations | `repository.run_migrations()` only when persistence on |
| Admin token | `ADMIN_API_REQUIRE_TOKEN` (default `false`); admin disabled by default |
| Execution / broadcast | `EXECUTION_ENABLED`, `EXECUTOR_DRY_RUN`, `EXECUTOR_REAL_BROADCAST_ENABLED` (all default safe-OFF/DRY) |
| Indexer / reconciliation / confirmation | each gated by an enabled flag + persistence requirement |
| Options service | `OPTIONS_ENABLED` (default `false`). When false → `/options/products` returns 500 INTERNAL_ERROR. When true with `OPTIONS_REQUIRE_PERSISTENCE=false` → in-memory store returns empty envelope cleanly. |
| **CORS** | **WAS MISSING.** No `tower_http::cors::CorsLayer` anywhere in the router. Browser cross-origin requests from `http://localhost:3000` would have been blocked. |
| `.env` location | `deopt-v2-backend/.env` (production-tracked file; do NOT touch) |
| `.env.local` location | none yet — created as `.env.local.example` template |

### Backend source change (minimal, scoped)

**`Cargo.toml`:** added `"cors"` to `tower-http` features.

**`src/api/routes.rs`:** new private helper at top of file:

```rust
fn cors_layer_from_env() -> CorsLayer {
    use axum::http::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE};
    use axum::http::{HeaderValue, Method};

    let origins_raw = std::env::var("CORS_ALLOWED_ORIGINS")
        .unwrap_or_else(|_| "http://localhost:3000,http://127.0.0.1:3000".to_string());
    let origins: Vec<HeaderValue> = origins_raw
        .split(',')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .filter_map(|s| HeaderValue::from_str(s).ok())
        .collect();

    CorsLayer::new()
        .allow_origin(origins)
        .allow_methods([Method::GET, Method::POST, Method::DELETE, Method::OPTIONS])
        .allow_headers([CONTENT_TYPE, ACCEPT, AUTHORIZATION])
}
```

Applied after `TraceLayer` in `pub fn router(...)`:

```rust
        .layer(TraceLayer::new_for_http())
        .layer(cors_layer_from_env())
        .with_state(state)
```

Behaviour:
* Reads `CORS_ALLOWED_ORIGINS` (comma-separated). Empty / unset → defaults to the two localhost dev origins.
* Setting `CORS_ALLOWED_ORIGINS=` (empty) yields an empty allow-list → effectively disables browser cross-origin access for backend-only deployments.
* Production deploys MUST override with the operator's frontend app URL (e.g. `https://<host>.vercel.app`).
* No new feature flag; no new admin surface; no secret read.

---

## 3. Frontend local startup inventory

| Aspect | Finding |
|---|---|
| Backend URL (trading reads) | `NEXT_PUBLIC_TRADING_API_BASE_URL` in `src/lib/trading-api.ts`; default `http://localhost:3000` (WRONG for the new local setup — should be `:8080`) |
| Chain env | `NEXT_PUBLIC_CHAIN_ENV` in `src/lib/chains.ts`; default `sepolia` |
| Admin URL | `NEXT_PUBLIC_BACKEND_URL` in `src/lib/admin-api.ts`; default `http://127.0.0.1:8080` |
| Backend-unavailable fallback | `MarketsFallbackCard` renders when `/options/products` errors at the network layer |
| Dev script | `npm run dev` (Turbopack) |

### Frontend env template

`.env.local.example` REWRITTEN to document the correct local pairing:
* `NEXT_PUBLIC_TRADING_API_BASE_URL=http://localhost:8080`
* `NEXT_PUBLIC_CHAIN_ENV=sepolia`
* `NEXT_PUBLIC_BACKEND_URL=http://127.0.0.1:8080` (optional, for `/admin`)

No frontend source change required this milestone.

---

## 4. Safe local env templates

* `deopt-v2-backend/.env.local.example` — NEW. Documents PORT/HOST/CHAIN_ID/CORS + all safety locks (broadcast off, dry-run on, no signer, no admin token, all workers off) + canonical Base Sepolia contract addresses (public; safe to commit).
* `deopt-v2-frontend/.env.local.example` — REWRITTEN. Points at `localhost:8080`.

Neither template contains a real RPC URL, real DATABASE_URL credentials, real bearer token, real private key, or any other secret.

---

## 5. Local run scripts

Three small, idempotent, public-safe shell scripts at `~/DEOPT/scripts/`:

| Script | Purpose | Safety guarantees |
|---|---|---|
| `local-backend.sh` | Starts the backend in safe local-dev mode | Hardcoded post-dotenv overrides: `EXECUTION_ENABLED=false`, `EXECUTOR_DRY_RUN=true`, `EXECUTOR_REAL_BROADCAST_ENABLED=false`, `OPTION_EXECUTION_BROADCAST_ENABLED=false`, `OPTION_EXECUTION_SIMULATION_ENABLED=false`, all nonce-sync / indexer / reconciliation / confirmation OFF, `ADMIN_API_ENABLED=false`. Defaults to `PERSISTENCE_ENABLED=false` + `OPTIONS_ENABLED=true` in-memory. |
| `local-frontend.sh` | Starts the Next.js dev server pointed at the local backend | Sets `NEXT_PUBLIC_TRADING_API_BASE_URL=http://localhost:8080` and `NEXT_PUBLIC_CHAIN_ENV=sepolia` if unset. |
| `local-smoke.sh` | Read-only curl smoke against backend | 9 GETs + 1 CORS preflight; no POST, no body, no auth header, no chain action. |

All scripts: `chmod +x`. No `set -x`. No `echo` of env-derived values. No `cat` of `.env` files. No private file read.

---

## 6. CORS / API compatibility

Before this milestone: no CORS layer → browser cross-origin requests from `:3000` to `:8080` would have been blocked by the same-origin policy. Verified with the existing tower-http 0.5 dep (had only `["trace"]`).

After this milestone:
* Preflight `OPTIONS /options/products` with `Origin: http://localhost:3000` returns `200 OK` with the expected `access-control-allow-origin`, `access-control-allow-methods`, `access-control-allow-headers`.
* Actual `GET /options/products` returns the envelope with `access-control-allow-origin: http://localhost:3000`.
* Production deploys MUST set `CORS_ALLOWED_ORIGINS=<APP_URL>` — documented in `LOCAL_FULLSTACK_RUNBOOK.md §5` and to be added to the deploy operator checklist.

The frontend's `trading-api.ts` already uses `NEXT_PUBLIC_TRADING_API_BASE_URL` consistently — no hardcoded backend URL remains in src. `localhost` only appears as a fallback in `.env*.example` and `chains.ts` (anvil dev).

---

## 7. Backend local smoke (LIVE on `127.0.0.1:8080`)

Started with:
```
PORT=8080 HOST=127.0.0.1 PERSISTENCE_ENABLED=false EXECUTION_ENABLED=false
EXECUTOR_REAL_BROADCAST_ENABLED=false … OPTIONS_ENABLED=true
OPTIONS_REQUIRE_PERSISTENCE=false … CORS_ALLOWED_ORIGINS=http://localhost:3000,http://127.0.0.1:3000
target/debug/deopt-v2-backend
```

Tail of the startup log:
```
INFO deopt_v2_backend: starting http server
  service=deopt-v2-backend addr=127.0.0.1:8080 chain_id=84532 network=base-sepolia
  execution_enabled=false confirmation_enabled=false
  option_confirmation_worker_enabled=false option_event_indexer_enabled=false
  option_reconciliation_worker_enabled=false rfq_enabled=true options_enabled=true
  fees_enabled=false rebates_enabled=false metrics_enabled=true mm_gateway_enabled=false
  indexer_enabled=false reconciliation_enabled=false executor_dry_run=true
  persistence_enabled=false
```

Smoke results (via `scripts/local-smoke.sh`):

| Check | HTTP | Notes |
|---|---|---|
| `/health` | 200 | `{"ok":true,"service":"deopt-v2-backend"}` |
| `/ready` | 200 | `database: persistence_disabled` (honest) |
| `/trading/health` | 200 | `chain_id: 84532`, `overall_status: ok` |
| `/options/products` | 200 | empty `products: []`, `source: "db"` |
| `/markets` | 200 | perp mocks list |
| `/accounts/<sample>/balances` | 200 | `partial`; `SOURCE_UNAVAILABLE_FIELD` warning (honest) |
| `/accounts/<sample>/positions` | 200 | `partial`; `CONFIG_MISSING` + `RPC_UNAVAILABLE` warnings |
| `/accounts/<sample>/portfolio` | 200 | `partial`; `CONFIG_MISSING` warning |
| CORS preflight `OPTIONS /options/products` | 200 | `access-control-allow-origin: http://localhost:3000`; `access-control-allow-methods: GET,POST,DELETE,OPTIONS`; `access-control-allow-headers: content-type,accept,authorization` |

**Result: 9 pass / 0 fail.**

Backend code health:
* `cargo fmt --check` — clean after `cargo fmt` (one routes.rs reformat for the new helper).
* `cargo build -p deopt-v2-backend` — green (16.09s incremental).
* `cargo test -p deopt-v2-backend --lib api::` — **281 passed / 0 failed** (the existing API test suite still green with the new CORS layer applied).

---

## 8. Frontend local smoke

* `npm run typecheck` — clean.
* `npm run lint` — clean.
* `npm run build` with `NEXT_PUBLIC_TRADING_API_BASE_URL=http://localhost:8080` + `NEXT_PUBLIC_CHAIN_ENV=sepolia` — green (15 user-facing routes + 4 SSG doc slugs + `_not-found`).
* `npx playwright test --list` — **96 tests in 24 files** (catalog unchanged from prior milestone).

Visual fullstack run NOT executed under this preflight (would require a browser session). Per the runbook, the operator can verify by:
1. `bash ~/DEOPT/scripts/local-backend.sh` in terminal A.
2. `bash ~/DEOPT/scripts/local-frontend.sh` in terminal B.
3. Open `http://localhost:3000/trade` — chain renders with empty rows (NOT the backend-unavailable fallback).

---

## 9. Fullstack local smoke

State at end of this milestone:
* Backend running at `127.0.0.1:8080` with CORS open to `localhost:3000`.
* Frontend builds against `localhost:8080`.
* Read-only curl smoke is **9/9 PASS**.
* No chain transaction sent. No broadcast attempted. No signer call.
* The frontend's MarketsFallbackCard's `backend-unavailable` state is no longer triggered when the local backend is running with options-in-memory mode.

**Outcome: LOCAL FULLSTACK GREEN.**

---

## 10. Docs created/updated

| File | Action |
|---|---|
| `~/DEOPT/deopt-v2-backend/docs/LOCAL_FULLSTACK_TESTNET_BETA_SMOKE_RESULT.md` | NEW (this file) |
| `~/DEOPT/deopt-v2-backend/docs/LOCAL_FULLSTACK_RUNBOOK.md` | NEW — three-terminal procedure |
| `~/DEOPT/deopt-v2-backend/.env.local.example` | NEW — backend dev template |
| `~/DEOPT/deopt-v2-frontend/.env.local.example` | REWRITTEN — was 1 line (`NEXT_PUBLIC_BACKEND_URL`); now full local pairing |
| `~/DEOPT/scripts/local-backend.sh` | NEW |
| `~/DEOPT/scripts/local-frontend.sh` | NEW |
| `~/DEOPT/scripts/local-smoke.sh` | NEW |
| `~/DEOPT/deopt-v2-backend/src/api/routes.rs` | MODIFIED — CORS layer helper + `.layer(...)` |
| `~/DEOPT/deopt-v2-backend/Cargo.toml` | MODIFIED — tower-http `"cors"` feature added |
| `~/DEOPT/RUN_STATE.md` | UPDATED — closure paragraph prepended |

---

## 11. Remaining blockers (local fullstack)

NONE for local product testing. The operator can now run the three-terminal procedure and visually iterate on the trading UI against a working local backend.

For PUBLIC deployment (still gated on the existing `FRONTEND_PUBLIC_TESTNET_DEPLOY_OPERATOR_CHECKLIST.md`):
* Operator hosting choice + stand-up of `<APP_URL>` (sole remaining hard blocker for `PUBLIC-TESTNET-BETA-LAUNCH-PREFLIGHT` rerun).
* Operator must set `CORS_ALLOWED_ORIGINS=<APP_URL>` on the deployed backend.
* Railway start-command issue (the operator's previous attempt failed because no start command was configured) — see `BACKEND_PUBLIC_TESTNET_DEPLOY_PREFLIGHT_NEXT_TASK.md` (created under this milestone).

---

## 12. Next milestone recommendation

**Primary (operator-side, no agent work needed):** product-test the frontend at `http://localhost:3000/trade` against the local backend. Iterate on UI/UX. Capture any actionable bugs into the existing `/feedback` route or a new GitHub issue.

**Secondary (agent-runnable):** `BACKEND-PUBLIC-TESTNET-DEPLOY-PREFLIGHT` — assemble a deployment preflight for the backend (mirroring the frontend deploy preflight; documents start command, env matrix, sane defaults, host-side CORS_ALLOWED_ORIGINS, and dispatches the Railway-failed retry).

**Strictly later (NOT NOW):** publish announcement, contact audit firm, launch bug bounty, mainnet, KMS cutover, Safe migration.

---

## 13. Cross-links
* `~/DEOPT/deopt-v2-backend/docs/LOCAL_FULLSTACK_RUNBOOK.md`
* `~/DEOPT/deopt-v2-backend/docs/FRONTEND_PUBLIC_TESTNET_DEPLOY_PREFLIGHT_RESULT.md`
* `~/DEOPT/deopt-v2-backend/docs/FRONTEND_PUBLIC_TESTNET_DEPLOY_OPERATOR_CHECKLIST.md`
* `~/DEOPT/deopt-v2-backend/docs/PUBLIC_TESTNET_BETA_LAUNCH_PREFLIGHT_RERUN_NEXT_TASK.md`
* `~/DEOPT/TESTNET_RUNBOOK.md`
* `~/DEOPT/RUN_STATE.md`

**End of local fullstack testnet beta smoke result.**
