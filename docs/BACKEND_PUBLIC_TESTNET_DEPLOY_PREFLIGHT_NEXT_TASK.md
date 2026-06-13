# BACKEND-PUBLIC-TESTNET-DEPLOY-PREFLIGHT — Next Task Brief

**Date written:** 2026-06-13
**Origin:** `LOCAL_FULLSTACK_TESTNET_BETA_SMOKE_RESULT.md` + the failed Railway attempt that prompted the local-fullstack-first directive.

**Target:** assemble a deployment preflight for the DeOpt v2 backend, mirroring the shape of `FRONTEND_PUBLIC_TESTNET_DEPLOY_PREFLIGHT_RESULT.md`. Documents start command, env matrix (public-safe subset), sane safety defaults, host-side `CORS_ALLOWED_ORIGINS`, and the retry path for the previously-failed Railway deploy.

**Posture:** docs-only preflight (same as the frontend equivalent). **No chain transactions. No broadcast. No mainnet. No `.env` edit. No backend source change beyond the small CORS feature already landed in `LOCAL-FULLSTACK-TESTNET-BETA-SMOKE`. No private key handling. No AWS/KMS call. No deployment performed under the preflight.**

---

## 1. Literal operator approval line (REQUIRED, VERBATIM)

> "I approve DeOpt V2 backend public testnet deploy preflight for this run."

---

## 2. Scope

### 2.1 Start command

Railway's previous failure was that no start command was configured. The preflight should produce a concrete value:

```
cargo run --release --bin deopt-v2-backend
```

or, if the host prefers prebuilt binaries:

```
./target/release/deopt-v2-backend
```

The preflight should also document the build command:

```
cargo build --release --bin deopt-v2-backend
```

### 2.2 Public env matrix

Mirror `LOCAL_FULLSTACK_RUNBOOK.md §5`, but tuned for a hosted backend:

| Env var | Value |
|---|---|
| `HOST` | `0.0.0.0` (so the host can route to it; NOT `127.0.0.1`) |
| `PORT` | host-supplied (Railway: `$PORT`; Fly.io: `$PORT`) |
| `CHAIN_ID` | `84532` |
| `NETWORK_NAME` | `base-sepolia` |
| `CORS_ALLOWED_ORIGINS` | `<APP_URL>` (the operator's deployed frontend URL) |
| `RUST_LOG` | `info` |
| `EXECUTION_ENABLED` | `false` |
| `EXECUTOR_DRY_RUN` | `true` |
| `EXECUTOR_REAL_BROADCAST_ENABLED` | `false` |
| `OPTION_EXECUTION_BROADCAST_ENABLED` | `false` |
| `OPTIONS_ENABLED` | `true` |
| `OPTION_RFQ_ENABLED` | `true` |
| `OPTIONS_REQUIRE_PERSISTENCE` | `false` UNTIL the operator stands up Postgres + sets `DATABASE_URL`; then `true`. |
| `OPTION_RFQ_REQUIRE_PERSISTENCE` | same |
| `PERSISTENCE_ENABLED` | `false` initially; `true` once Postgres is wired. |
| `DATABASE_URL` | host-private value if persistence enabled |
| `ADMIN_API_ENABLED` | `false` for public testnet beta |
| `ADMIN_API_REQUIRE_TOKEN` | `false` (admin disabled, so token gate doesn't run) |
| `METRICS_ENABLED` | `true` (for operator scraping) |

Public contract addresses (safe to set as plaintext host env):
* `OPTION_MATCHING_ENGINE_ADDRESS=0x5a5EBF9A9CCd7c012518569DE8283982982670f6`
* `OPTION_MARGIN_ENGINE_ADDRESS=0x506cD65a63C53c66ab572B9f9dd819B7BfE00D30`
* `OPTION_PRODUCT_REGISTRY_ADDRESS=0x3d52b033fab00ed6104dd3bc0a715f8648344eca`
* `OPTION_COLLATERAL_VAULT_ADDRESS=0x00340C360353a5AB784c5Bc5c44322A6AF0625D3`
* `OPTION_COLLATERAL_VAULT_VIEWS_ADDRESS=0x00340C360353a5AB784c5Bc5c44322A6AF0625D3`
* `OPTION_ORACLE_ROUTER_ADDRESS=0xb416406f200b2ef3d7a86a5d5877ed41d9b1a581`
* `OPTION_MARGIN_ENGINE_LENS_ADDRESS=0x496A57CF4e0d4F1BC5c00969Ed4C5204072ddA26`
* `COLLATERAL_TOKEN=0x6eAe407f5640B006faC9965182e238582A3B412E`

**Explicitly forbidden on the public host:**
* `EXECUTOR_PRIVATE_KEY` — must remain empty.
* mainnet RPC URL.
* private RPC URL with API key — if a private RPC is needed, the operator may set `RPC_URL` via the host's secret-management UI, but NEVER paste it into the host's plaintext env vars or into a committed file.
* `DATABASE_URL` if it contains credentials, must use host secrets management.
* `ADMIN_API_TOKEN` real value (admin should stay disabled for public beta).

### 2.3 Deployment target

| Target | Start command | Notes |
|---|---|---|
| **Railway** (primary; retry the previously-failed attempt) | `cargo run --release --bin deopt-v2-backend` | Set `start command` explicitly in the Railway service settings to avoid the previous "no start command configured" failure. |
| **Fly.io** (fallback) | `./target/release/deopt-v2-backend` after a Docker build | Requires a Dockerfile. |
| **Render** (fallback) | `cargo run --release --bin deopt-v2-backend` | Free tier sleeps; not ideal for a beta. |

### 2.4 Smoke checklist (post-deploy)

| URL | Expected |
|---|---|
| `<BACKEND_URL>/health` | `{"ok":true,…}` |
| `<BACKEND_URL>/ready` | `database: persistence_disabled` OR `database: ok` |
| `<BACKEND_URL>/trading/health` | `chain_id: 84532` |
| `<BACKEND_URL>/options/products` | `200 OK`, empty envelope (acceptable) |
| `<BACKEND_URL>/markets` | mock perp list |
| CORS preflight from `Origin: <APP_URL>` | `200 OK` with `access-control-allow-origin: <APP_URL>` |
| `<BACKEND_URL>/admin/status` | `403 Forbidden` (admin disabled by env) |

### 2.5 Acceptance criteria

* Backend `<BACKEND_URL>` reachable over HTTPS.
* Smoke set above passes.
* The frontend deployed at `<APP_URL>` with `NEXT_PUBLIC_TRADING_API_BASE_URL=<BACKEND_URL>` shows the options-chain terminal with empty rows (NOT the backend-unavailable fallback).
* `OPERATOR_PUBLIC_BETA_URLS_FILL_RERUN_NEXT_TASK.md` can then be re-run with both `<APP_URL>` and (optionally) the backend URL.

---

## 3. Hard preconditions

| # | Precondition | Verifying check |
|---|---|---|
| P1 | Approval line (§1) present verbatim | grep |
| P2 | `LOCAL_FULLSTACK_TESTNET_BETA_SMOKE_RESULT.md` exists; local smoke 9/9 PASS | grep |
| P3 | Backend `.env` untouched | `stat -c '%y'` |
| P4 | Private file mode preserved | `stat -c '%a %y'` |
| P5 | `~/DEOPT/private/**` NOT read | trust |

---

## 4. Forbidden

* Mainnet RPC.
* Mainnet contract address presented as current.
* `.env` edit on production tracked file.
* Bearer / RPC-with-key / DATABASE_URL pasted into any committed file.
* Source code changes to backend beyond the existing CORS layer.
* Publishing the announcement.
* Audit firm contact.
* Bug bounty launch.

---

## 5. Acceptance artifact

* `docs/BACKEND_PUBLIC_TESTNET_DEPLOY_PREFLIGHT_RESULT.md` (NEW)
* `docs/BACKEND_PUBLIC_TESTNET_DEPLOY_OPERATOR_CHECKLIST.md` (NEW)
* `docs/PUBLIC_TESTNET_BETA_LAUNCH_PREFLIGHT_RERUN_NEXT_TASK.md` (UPDATED — adds backend deploy as a precondition)
* `RUN_STATE.md` update.

---

## 6. Cross-links

* `docs/LOCAL_FULLSTACK_TESTNET_BETA_SMOKE_RESULT.md`
* `docs/LOCAL_FULLSTACK_RUNBOOK.md`
* `docs/FRONTEND_PUBLIC_TESTNET_DEPLOY_PREFLIGHT_RESULT.md`
* `docs/FRONTEND_PUBLIC_TESTNET_DEPLOY_OPERATOR_CHECKLIST.md`
* `~/DEOPT/RUN_STATE.md`

**End of backend public testnet deploy preflight next-task brief.**
