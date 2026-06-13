# DeOpt V2 — Local Fullstack Runbook

> Posture: localhost dev only. Public testnet beta (Base Sepolia). **No mainnet. No broadcast. No real funds.**

Three terminal windows. Three commands. No secrets.

---

## 0. Prereqs

* Rust toolchain (`cargo --version`).
* Node 20+ (`node --version`).
* `npm ci` already run in `deopt-v2-frontend/`.
* `cargo build` already run once in `deopt-v2-backend/` (optional — `scripts/local-backend.sh` will build on first run).
* Postgres NOT required for product testing; persistence is OFF by default. If you want full persistence, run a local Postgres on `127.0.0.1:5432`, set `PERSISTENCE_ENABLED=true` and `DATABASE_URL` in `deopt-v2-backend/.env.local`.

---

## 1. Terminal A — Backend on :8080

```bash
bash ~/DEOPT/scripts/local-backend.sh
```

What it does:
* `cd` into `deopt-v2-backend`.
* Applies hardcoded safety overrides (broadcast OFF, dry-run ON, no signer call, admin disabled, indexer + reconciliation + confirmation workers OFF).
* Defaults to:
  * `HOST=127.0.0.1`, `PORT=8080`
  * `CHAIN_ID=84532`, `NETWORK_NAME=base-sepolia`
  * `CORS_ALLOWED_ORIGINS=http://localhost:3000,http://127.0.0.1:3000`
  * `PERSISTENCE_ENABLED=false`
  * `OPTIONS_ENABLED=true` + `OPTIONS_REQUIRE_PERSISTENCE=false` (in-memory options store)
* `cargo run --bin deopt-v2-backend`.

Per-developer overrides go in `deopt-v2-backend/.env.local` (gitignored). The safety overrides above are exported AFTER `.env.local` is sourced by the backend's `dotenvy`, so they always win.

Expected first lines (post-LOCAL-BACKEND-SAFE-MODE-FIX):
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
INFO deopt_v2_backend::options::confirmation_worker: option confirmation worker disabled
INFO deopt_v2_backend::options::reconciliation_worker: option reconciliation worker disabled
INFO deopt_v2_backend::mm::transport::webtransport: MM WebTransport gateway disabled
INFO deopt_v2_backend::options::event_indexer: option event indexer disabled
INFO deopt_v2_backend: starting http server addr=127.0.0.1:8080 chain_id=84532 …
```

If you instead see `Error: Config("option confirmation worker requires persistence enabled")`, you are running an older `scripts/local-backend.sh` from before the safe-mode fix. Pull the latest — the fix added explicit `OPTION_CONFIRMATION_WORKER_ENABLED=false`, `OPTION_RECONCILIATION_WORKER_ENABLED=false`, `OPTION_EVENT_INDEXER_ENABLED=false`, and `OPTION_EXECUTION_ENABLED=false` overrides that win against any production `.env` value that turns them on.

---

### 1.1 Port 3000 already in use?

Next.js auto-bumps to 3001 (or 3002, etc.) when its default port is busy. `scripts/local-frontend.sh` now prints a `NOTE: port 3000 is already in use` line up-front if it detects the situation, and the backend's `CORS_ALLOWED_ORIGINS` default includes both 3000 and 3001 so either origin works without re-editing config. **Open whatever URL Next.js actually prints in the terminal — do not hardcode `http://localhost:3000` mentally.**

## 2. Terminal B — Frontend on :3000

```bash
bash ~/DEOPT/scripts/local-frontend.sh
```

What it does:
* `cd` into `deopt-v2-frontend`.
* Sets `NEXT_PUBLIC_TRADING_API_BASE_URL=http://localhost:8080` and `NEXT_PUBLIC_CHAIN_ENV=sepolia` if not already set.
* `npm run dev`.

Expected first lines:
```
[local-frontend] dev server on http://localhost:3000
[local-frontend] trading api: http://localhost:8080
…
▲ Next.js 16.1.6 (Turbopack)
…
Ready in <ms>
```

Open <http://localhost:3000/trade>. The Options chain terminal should render with the Calls | Strike | Puts ladder. The right detail panel waits for a chain cell to be clicked. The bottom panel surfaces Balances / Positions / Trades / Events. With `PERSISTENCE_ENABLED=false`, the chain is empty until you create option series via the admin API — **but the backend-unavailable fallback should NOT appear** because the backend itself is up and returning a 200 ok envelope with an empty list.

---

## 2.5 (Optional) Seed sample option series so /markets renders cards

In safe in-memory mode, `/options/products` starts empty. Run the seed
script once after the backend is up:

```bash
bash ~/DEOPT/scripts/local-seed.sh
```

POSTs 12 WETH/mUSDC series (2 expiries × 3 strikes × Call+Put) via
`POST /options/series`. Series are tagged `source: "manual"` by the
backend — they are NOT on-chain and NOT presented as real markets.
No chain transaction is sent. No signer call.

After seeding, `/markets` shows 4 product cards (per expiry × call/put)
and `/trade` renders the Calls | Strike | Puts ladder against the
seeded series. Greeks / bid / ask / IV stay "n/a testnet" — the seed
only creates the SERIES; it does NOT fabricate quotes or open
interest.

To re-seed (e.g. after restarting the backend with persistence still
off), re-run the script — it is idempotent; already-existing series
return non-200 and are SKIPped.

## 3. Terminal C — Smoke check (read-only)

```bash
bash ~/DEOPT/scripts/local-smoke.sh
```

Runs 9 read-only curls against `http://127.0.0.1:8080`:
* `/health`, `/ready`, `/trading/health`
* `/options/products`, `/markets`
* `/accounts/<sample 0x>/balances`, `/positions`, `/portfolio`
* CORS preflight from `Origin: http://localhost:3000`.

Expected: `9 pass / 0 fail`.

To use a custom backend URL:

```bash
BACKEND_URL=http://127.0.0.1:8081 bash ~/DEOPT/scripts/local-smoke.sh
```

---

## 4. What the frontend actually shows

| Route | Expected with backend up + empty options store |
|---|---|
| `/` | Landing; testnet posture; report-feedback CTA to `/feedback` |
| `/trade` | Options chain (empty rows); Calls/Strike/Puts headers; detail panel empty until first click; honest "n/a testnet" greeks copy. Navbar label is **Options** (route stays `/trade`). |
| `/perps` | Coming-soon placeholder with disclosure panel + "in the meantime" CTAs to Options / Markets / Docs / Feedback / Discord. No fake liquidity. |
| `/markets` | "No active testnet markets" empty card (NOT the backend-unavailable card) |
| `/portfolio` | Empty positions; balances all "—"; status `partial` from honest backend warnings |
| `/docs` + sub-routes | All SSG-prerendered MD docs |
| `/feedback` | Bug-report template |

If `/trade` instead shows "Trading backend temporarily unavailable", check:
1. Backend terminal still running?
2. Backend log shows `starting http server`?
3. `curl http://127.0.0.1:8080/health` returns `{"ok":true,…}` ?
4. CORS check: `bash scripts/local-smoke.sh` says PASS for `cors_preflight`?
5. Frontend `.env.local` (or shell) sets `NEXT_PUBLIC_TRADING_API_BASE_URL=http://localhost:8080`?

---

## 5. CORS configuration (introduced this milestone)

The backend now applies a tower-http CORS layer. Configurable via `CORS_ALLOWED_ORIGINS` env (comma-separated). Defaults to `http://localhost:3000,http://127.0.0.1:3000` so a fresh clone works without any setup.

For a deployed backend, the operator must set:
```
CORS_ALLOWED_ORIGINS=https://<your-frontend-host>.vercel.app
```
(Or whatever the operator's published `<APP_URL>` is.)

To explicitly disable browser cross-origin access (e.g., for an internal-only backend), set:
```
CORS_ALLOWED_ORIGINS=
```
(Empty string → empty allow-list.)

Allowed methods: `GET`, `POST`, `DELETE`, `OPTIONS`. Allowed headers: `content-type`, `accept`, `authorization`.

---

## 6. Safety guarantees

* The script-set safety overrides cannot be flipped via `.env.local` — they are exported AFTER dotenvy loads. The only way to enable broadcast is to bypass the script and run `cargo run` directly with an explicit overriding env.
* No private key is read by these scripts. `EXECUTOR_PRIVATE_KEY` stays empty.
* No mainnet RPC. `CHAIN_ID` is forced to `84532` (Base Sepolia).
* No AWS / KMS call. The backend does not contain any production-signer path that would be reached with these flags.
* The frontend does NOT send `Authorization` headers from `trading-api.ts`; the admin dashboard (`/admin`) prompts the user to paste a token into sessionStorage — don't.

---

## 7. Stopping everything

* Backend: `Ctrl-C` in Terminal A. Or `pkill -f target/debug/deopt-v2-backend`.
* Frontend: `Ctrl-C` in Terminal B.
* Postgres (if used): `docker stop deopt-pg` or whatever container runtime you chose.

---

## 8. Cross-links

* `~/DEOPT/scripts/local-backend.sh`, `local-frontend.sh`, `local-smoke.sh`
* `~/DEOPT/deopt-v2-backend/.env.local.example` — backend template
* `~/DEOPT/deopt-v2-frontend/.env.local.example` — frontend template
* `~/DEOPT/deopt-v2-backend/docs/LOCAL_FULLSTACK_TESTNET_BETA_SMOKE_RESULT.md` — milestone closeout result for this runbook
* `~/DEOPT/deopt-v2-backend/docs/FRONTEND_PUBLIC_TESTNET_DEPLOY_OPERATOR_CHECKLIST.md` — for when you stop testing locally and deploy
* `~/DEOPT/TESTNET_RUNBOOK.md` — broader testnet operator runbook

**End of local fullstack runbook.**
