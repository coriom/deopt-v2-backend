# FRONTEND-PUBLIC-TESTNET-DEPLOY-PREFLIGHT — RESULT

**Date:** 2026-06-13
**Operator approval line (consumed verbatim):**
> "I approve DeOpt V2 frontend public testnet deploy preflight for this run."

**Posture:** docs-only preflight. **No chain tx. No broadcast. No mainnet. No `.env` edit. No backend source change. No Solidity change. No deployment performed.** Purpose: produce exact deployment guidance + environment-variable checklist so the operator can stand up the public-beta App URL — currently the sole remaining hard blocker per `PUBLIC_TESTNET_BETA_LAUNCH_CHECKLIST.md §0`.

---

## 1. Deployment target inventory

Inspected (paths relative to `~/DEOPT/`):

| File | Finding |
|---|---|
| `deopt-v2-frontend/package.json` | Next.js 16.1.6 (Turbopack), React 19.2.3, viem 2.52.2, marked 18.0.5; scripts: `dev`, `build` (`next build`), `start`, `lint`, `typecheck`, `e2e:local` |
| `deopt-v2-frontend/next.config.ts` | Empty config — no custom output mode; default Next.js standalone-or-server behavior |
| `deopt-v2-frontend/.env.example` | Lists `NEXT_PUBLIC_TRADING_API_BASE_URL` (default `http://localhost:3000`) + `NEXT_PUBLIC_CHAIN_ENV` (default `sepolia`) |
| `deopt-v2-frontend/.env.local.example` | Lists `NEXT_PUBLIC_BACKEND_URL` (default `http://127.0.0.1:8080`) — used ONLY by the admin dashboard at `/admin` |
| `deopt-v2-frontend/.github/workflows/frontend-ci.yml` | CI runs lint + tsc + build with `NEXT_PUBLIC_BACKEND_URL=http://127.0.0.1:8080`; no secrets, no admin token |
| `deopt-v2-frontend/README.md` | Boilerplate Next.js + "Deploy on Vercel" link; no custom deploy docs |
| `deopt-v2-frontend/src/lib/public-beta-links.ts` | 6 link slots — quickstart, testing-guide, limitations, feedback (internal), discord, github (external) — all `status: "live"`; APP_URL not modelled in frontend (doc-side token only) |
| `deopt-v2-frontend/vercel.json`, `netlify.toml`, `wrangler.toml`, `Dockerfile` | **NONE present** — fresh ground for a deployment config |
| `deopt-v2-frontend/src/app/admin/` | `/admin/page.tsx` exists; requires operator-pasted Bearer token kept in sessionStorage. Without a token, route renders an empty / token-prompt dashboard. **No bearer token is bundled.** |

### 1.1 Determined facts

* **Framework:** Next.js 16.1.6 App Router (Turbopack-built).
* **Build command:** `npm run build` (= `next build`).
* **Install command:** `npm ci` (lockfile present).
* **Node version:** 20 (per CI; package engines field not set, so 20 LTS is the documented baseline).
* **Output mode:** default Next.js — mix of static (`○`), SSG (`●` with `generateStaticParams` for `/docs/[slug]`), and dynamic (`ƒ` for `/markets/[productId]`, `/transactions/[requestId]`). **Requires a Node runtime** because dynamic routes are server-rendered on demand. Cannot be fully statically exported.
* **Backend API dependency:** `NEXT_PUBLIC_TRADING_API_BASE_URL` (public reads). The build itself does NOT require a reachable backend; CI proves this by using `http://127.0.0.1:8080` and never starting one.
* **Docs/feedback self-contained:** YES — `/docs/quickstart`, `/docs/testing-guide`, `/docs/limitations`, `/docs/faq` and `/feedback` are all in-bundle. No external docs host required.
* **Deployment can be static?** **No** — `/markets/[productId]` and `/transactions/[requestId]` are dynamic. Recommend a host with a Node runtime (Vercel default, Netlify with `@netlify/plugin-nextjs`, Cloudflare Pages with `@cloudflare/next-on-pages`, or a Node container).

### 1.2 Admin-route posture (DEPLOYMENT IMPLICATION)

`/admin` ships in the public bundle. **Without an operator-set sessionStorage token, every admin XHR is unauthenticated.** No bearer token is hard-coded; no admin URL is leaked. Still:

* Operator MUST NOT paste a real admin Bearer token into a browser pointed at the public App URL.
* Operator MAY (recommended) block `/admin` at the hosting edge for the public-beta deployment via a redirect / 404 rule. Optional `vercel.json` / `netlify.toml` snippets are documented in the operator checklist below.

---

## 2. Public env matrix (testnet beta)

ONLY `NEXT_PUBLIC_*` variables may be exposed to the browser bundle. Every entry below is publicly safe.

### 2.1 Required at deploy time

| Env var | Value (testnet beta) | Used by | Notes |
|---|---|---|---|
| `NEXT_PUBLIC_TRADING_API_BASE_URL` | publicly-reachable HTTPS URL of the testnet trading backend | `src/lib/trading-api.ts` | If absent, defaults to `http://localhost:3000` (broken in prod). MUST be set. |
| `NEXT_PUBLIC_CHAIN_ENV` | `sepolia` | `src/lib/chains.ts` (`expectedChainId()`) | Anything else falls back to Base Sepolia. **Must NOT be set to `mainnet`.** |

### 2.2 Optional / soft-required

| Env var | Value | Used by | Notes |
|---|---|---|---|
| `NEXT_PUBLIC_BACKEND_URL` | leave UNSET in public-beta deploy | `src/lib/admin-api.ts` | If `/admin` is blocked at the edge, this is irrelevant. If admin is allowed (NOT recommended), defaults to `http://127.0.0.1:8080` — which simply fails harmlessly in a browser. |

### 2.3 Explicitly FORBIDDEN in the client bundle

These MUST NOT be added under any name beginning with `NEXT_PUBLIC_`:

* private RPC URL (Alchemy / Infura / QuickNode with key)
* mainnet RPC URL of any kind
* private keys / mnemonics / seed phrases
* `DATABASE_URL`
* admin Bearer token
* AWS credentials, KMS key id / arn (the frontend does not call KMS — and never will)
* production signer URL
* internal admin dashboard URL

A periodic scan over `.next/static/**` for these markers is part of the post-deploy checklist (`FRONTEND_PUBLIC_TESTNET_DEPLOY_OPERATOR_CHECKLIST.md §6`).

### 2.4 Public-facing constants (already baked in)

These are hardcoded in src; no env var required:

* Base Sepolia chain id `84532`
* Explorer `https://sepolia.basescan.org`
* Discord `https://discord.gg/zaEMvWuxu`
* GitHub `https://github.com/DeOpt`
* Internal routes `/docs/*`, `/feedback`
* `isMainnetEnabled()` returns `false` (hard-coded; ungated mainnet is impossible without a source change)

---

## 3. Deployment plan

### 3.1 Vercel (PRIMARY RECOMMENDATION)

| Step | Value |
|---|---|
| Import repo | GitHub → `DeOpt/<frontend-repo>` (the repo containing `deopt-v2-frontend/`) |
| **Root directory** | `deopt-v2-frontend` |
| Framework preset | **Next.js** (auto-detected) |
| Install command | `npm ci` |
| Build command | `npm run build` |
| Output directory | (Vercel auto-detected — do not override) |
| Node version | 20.x |
| Env vars (Production + Preview) | `NEXT_PUBLIC_TRADING_API_BASE_URL=<https URL of testnet backend>` ; `NEXT_PUBLIC_CHAIN_ENV=sepolia` |
| Custom domain | optional; if used, must be HTTPS. Until a domain is wired the operator can use the default `https://<project>.vercel.app` URL |
| Preview URL usage | every PR auto-generates a preview URL; OK to share for testing, but the announcement must point at the production URL |
| Post-deploy validation | see operator checklist §3 |

**No `vercel.json` is required.** The defaults are correct. Adding one is OPTIONAL only to block `/admin` at the edge — see operator checklist §5.

### 3.2 Netlify (FALLBACK)

| Step | Value |
|---|---|
| Connect repo | GitHub → same repo |
| **Base directory** | `deopt-v2-frontend` |
| Build command | `npm run build` |
| Publish directory | `.next` |
| Required plugin | `@netlify/plugin-nextjs` (install via `netlify.toml` or Netlify UI) |
| Env vars | identical to Vercel (§3.1) |
| Node version | 20 (`NETLIFY_NODE_VERSION` env or `netlify.toml`) |

### 3.3 Cloudflare Pages (FALLBACK)

| Step | Value |
|---|---|
| Framework | Next.js (with `@cloudflare/next-on-pages` adapter) |
| **Root directory** | `deopt-v2-frontend` |
| Build command | `npx @cloudflare/next-on-pages` |
| Output dir | `.vercel/output/static` |
| Compatibility flag | `nodejs_compat` (required for viem) |
| Env vars | identical to Vercel (§3.1) |
| Caveats | Cloudflare Pages forces edge runtime by default; verify `/markets/[productId]` and `/transactions/[requestId]` still server-render. If incompatible, prefer Vercel or Netlify. |

### 3.4 App URL placeholder

Until the operator stands up the hosting, all docs use a placeholder:

```
https://<your-deopt-frontend-host>.vercel.app
```

The OPERATOR-PUBLIC-BETA-URLS-FILL rerun replaces it.

---

## 4. Pre-deploy validations (executed under this preflight)

Run from `~/DEOPT/deopt-v2-frontend/`:

| Command | Result |
|---|---|
| `npm run typecheck` | **clean** — no output |
| `npm run lint` | **clean** — no output |
| `npm run build` | **green** — 16 prerendered routes; 4 SSG doc slugs (`quickstart`, `testing-guide`, `limitations`, `faq`); 2 dynamic (`/markets/[productId]`, `/transactions/[requestId]`); Turbopack build succeeded in 3.8s |
| `npx playwright test --list` | **96 tests in 24 files** |
| Sensitive-string scan on `.next/static/**` | `Bearer <token>` zero hits; `alchemy.com/v2/<key>` / `infura.io/v3/<key>` zero hits; `DATABASE_URL` zero hits; 64-hex pattern zero hits |
| Mainnet RPC pattern scan | `mainnet.base.org`, `mainnet.alchemy`, `base-mainnet` zero hits in `.next/static/**` |
| Positive-claim drift scan | only honest-disclaimer matches (`Is this audited?` in `/docs/faq` body, `Not mainnet-ready` in `/docs/limitations`). **Zero true drift.** |
| Amber/yellow/orange class scan | zero hits in `.next/static/**` and zero hits in `src/**` |
| `.env` mtime (backend) | `2026-06-08 16:55:05` — **preserved, NOT touched** |
| Private file mode | `700` — **preserved, NOT read** |

### 4.1 Route catalog (from `next build`)

```
Route (app)
○ /
○ /_not-found
○ /admin                           ← see §1.2 about edge blocking
○ /docs
● /docs/[slug] → quickstart, testing-guide, limitations, faq
○ /feedback
○ /health
○ /history
○ /markets
ƒ /markets/[productId]
○ /portfolio
○ /trade
ƒ /transactions/[requestId]
```

15 user-facing top-level routes + 1 `_not-found`.

---

## 5. Decision

**Deploy preflight verdict:** **READY TO DEPLOY** — no source change blocking the deployment; build is reproducible; env matrix is documented; admin posture is contained.

**Deploy NOT executed under this preflight** per the brief's safety rules. Operator action is now to:

1. Pick a host (Vercel recommended).
2. Configure the env matrix in §2.
3. Apply the recommended `/admin` block (§1.2) if desired.
4. Trigger the deployment.
5. Smoke-test per `FRONTEND_PUBLIC_TESTNET_DEPLOY_OPERATOR_CHECKLIST.md §3-§4`.
6. Run `OPERATOR-PUBLIC-BETA-URLS-FILL` rerun with the new App URL.
7. Run `PUBLIC-TESTNET-BETA-LAUNCH-PREFLIGHT` rerun. Expected verdict: **READY**.

---

## 6. Remaining hard blocker

* `{{APP_URL}}` — operator must stand up the hosting. Once live, the launch verdict flips on the rerun.

---

## 7. NOT done in this preflight

* No deployment was performed.
* No `vercel.json`, `netlify.toml`, or `wrangler.toml` was added (each is OPTIONAL and operator-discretion).
* No backend `.env` edit.
* No chain transaction.
* No mainnet activity.
* No audit firm contact.
* No bug bounty launch.
* No announcement publication.
* `isMainnetEnabled()` still hard-coded `false`.

---

## 8. Cross-links

* `docs/FRONTEND_PUBLIC_TESTNET_DEPLOY_OPERATOR_CHECKLIST.md` — the per-step operator checklist generated alongside this result.
* `docs/OPERATOR_PUBLIC_BETA_URLS_FILL_NEXT_TASK.md` — to re-run once the App URL is live.
* `docs/PUBLIC_TESTNET_BETA_LAUNCH_PREFLIGHT_RERUN_NEXT_TASK.md` — to re-run after URL fill flips the verdict to READY.
* `docs/public-beta/PUBLIC_TESTNET_BETA_LAUNCH_CHECKLIST.md` — §0 verdict, §1.5c new evidence row.
* `~/DEOPT/RUN_STATE.md` — 2026-06-13 closure entry.

**End of frontend public testnet deploy preflight result.**
