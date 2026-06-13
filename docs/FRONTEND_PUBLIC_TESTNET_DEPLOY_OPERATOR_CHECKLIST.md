# Frontend Public Testnet Deploy — Operator Checklist

**Date:** 2026-06-13
**Companion to:** `FRONTEND_PUBLIC_TESTNET_DEPLOY_PREFLIGHT_RESULT.md`
**Purpose:** step-by-step procedure for standing up the DeOpt V2 frontend at a publishable HTTPS URL for the public testnet beta.

> **Posture:** Base Sepolia testnet beta only. UNAUDITED. Experimental. No real funds. **Do not point this deployment at mainnet. Do not paste an admin Bearer token. Do not configure a private RPC URL as a `NEXT_PUBLIC_*` variable.** Do not publish the public announcement under this checklist — that requires the separate publication milestone with its own approval.

---

## 0. Pre-flight gate

* ☐ `FRONTEND_PUBLIC_TESTNET_DEPLOY_PREFLIGHT_RESULT.md` verdict = READY TO DEPLOY.
* ☐ A testnet backend is reachable at a publicly callable HTTPS URL. Call it `<BACKEND_URL>`.
* ☐ `<BACKEND_URL>/trading/health` returns `chain_id: 84532`.
* ☐ Operator has access to the chosen host (Vercel / Netlify / Cloudflare Pages).

If any box is unchecked, STOP and resolve before continuing.

---

## 1. Hosting provider choice

Pick ONE:

* **(Recommended) Vercel** — fastest path, no config file needed.
* Netlify with `@netlify/plugin-nextjs`.
* Cloudflare Pages with `@cloudflare/next-on-pages`.
* Self-hosted Node container (only if already operationalised).

This checklist documents Vercel in detail; the others mirror it.

---

## 2. Vercel project setup

* ☐ Import the GitHub repo containing `deopt-v2-frontend/`.
* ☐ Project **Root Directory:** `deopt-v2-frontend`.
* ☐ Framework preset: Next.js (auto-detected).
* ☐ Install command: `npm ci`.
* ☐ Build command: `npm run build`.
* ☐ Output directory: leave as Vercel default.
* ☐ Node version: 20.x.

### 2.1 Environment variables (Production + Preview)

Add ONLY these:

| Name | Value |
|---|---|
| `NEXT_PUBLIC_TRADING_API_BASE_URL` | `<BACKEND_URL>` (the HTTPS base URL of the testnet trading backend) |
| `NEXT_PUBLIC_CHAIN_ENV` | `sepolia` |

**DO NOT add** any of:

* `NEXT_PUBLIC_BACKEND_URL` — admin route; leave unset.
* Any RPC URL with an API key.
* Any private key, mnemonic, seed phrase.
* Any admin Bearer token.
* `DATABASE_URL`.
* AWS access / secret keys.
* Anything labelled `MAINNET_*`.

---

## 3. Post-deploy smoke checklist

Open the deployed App URL `<APP_URL>` and verify each path renders:

* ☐ `<APP_URL>/` — landing page; testnet posture visible; report-feedback CTA navigates to `/feedback`.
* ☐ `<APP_URL>/trade` — options-chain terminal (Calls | Strike | Puts ladder); detail panel empty until a cell is clicked; honest "n/a testnet" copy on the Greeks tab.
* ☐ `<APP_URL>/markets` — markets list; backend-unavailable fallback card renders cleanly if `<BACKEND_URL>` is unreachable.
* ☐ `<APP_URL>/portfolio` — portfolio shell with balances / positions / trades sections.
* ☐ `<APP_URL>/docs` — public-beta docs index.
* ☐ `<APP_URL>/docs/quickstart` — quickstart page (SSG).
* ☐ `<APP_URL>/docs/testing-guide` — SSG.
* ☐ `<APP_URL>/docs/limitations` — SSG, opens with the "Testnet only. No real funds. Unaudited. Experimental. Not mainnet-ready." banner.
* ☐ `<APP_URL>/docs/faq` — SSG.
* ☐ `<APP_URL>/feedback` — bug-report template; clipboard copy works; no form submits to an external service.

### 3.1 Disclaimers + guards

* ☐ Testnet / unaudited banner visible on every trading route.
* ☐ Wrong-network banner triggers on chain id ≠ 84532 (try Anvil 31337 or mainnet 8453).
* ☐ Mainnet-blocked banner triggers on chain id 8453 with a "Switch to Base Sepolia" button.
* ☐ Footer renders on every trading route with Discord + GitHub + Quickstart + Limitations + Feedback links.
* ☐ Hamburger drawer opens from the navbar; all 6 links visible.
* ☐ Sign-failure modal surfaces a "Report this issue" link to `/feedback`.

### 3.2 Negative checks (the absences matter)

* ☐ No "audited" claim.
* ☐ No "mainnet-ready" claim.
* ☐ No "production-ready" claim.
* ☐ No "safe for real funds" copy.
* ☐ No "guaranteed liquidity" / "institutional-grade" copy.
* ☐ Network tab: no XHR carries an `Authorization: Bearer …` header.
* ☐ Network tab: no XHR points at `mainnet.base.org` or any mainnet RPC.
* ☐ Network tab: no XHR points at an `alchemy.com/v2/<key>` or `infura.io/v3/<key>` URL.
* ☐ Page source / bundle: no `DATABASE_URL` string.

---

## 4. Footer + community link sanity

* ☐ Discord link → `https://discord.gg/zaEMvWuxu` (opens in new tab).
* ☐ GitHub link → `https://github.com/DeOpt`.
* ☐ Feedback link → `<APP_URL>/feedback` (same tab).
* ☐ Quickstart link → `<APP_URL>/docs/quickstart`.
* ☐ Limitations link → `<APP_URL>/docs/limitations`.
* ☐ Testing-guide link → `<APP_URL>/docs/testing-guide`.
* ☐ **No admin link visible anywhere on the page or in the hamburger drawer.**
* ☐ **No "Connect to mainnet" or similar CTA.**

---

## 5. (Optional) Admin route edge-block

`/admin` ships in the public bundle. Without an operator-pasted Bearer token it is harmless (no admin API call succeeds). If the operator still wants to hide it from the public deployment:

### 5.1 Vercel — `vercel.json` in `deopt-v2-frontend/`

```json
{
  "redirects": [
    { "source": "/admin", "destination": "/", "permanent": false },
    { "source": "/admin/:path*", "destination": "/", "permanent": false }
  ]
}
```

### 5.2 Netlify — `netlify.toml` in `deopt-v2-frontend/`

```toml
[[redirects]]
  from = "/admin"
  to = "/"
  status = 301

[[redirects]]
  from = "/admin/*"
  to = "/"
  status = 301
```

### 5.3 Cloudflare Pages

Use the Pages dashboard → Bulk redirects → `/admin*` → `/`.

This is OPTIONAL. The admin route requires a manually pasted Bearer token in sessionStorage; without it, no admin XHR carries authentication.

---

## 6. Post-deploy automated scans

If feasible, run these against the deployed App URL:

```bash
# Public bundle hygiene: no bearer / no RPC keys / no DB URL.
curl -sL <APP_URL> | grep -Ei 'Bearer\s+[A-Za-z0-9_.-]{16,}' && echo FAIL || echo PASS
curl -sL <APP_URL> | grep -Ei 'alchemy\.com/v2/[A-Za-z0-9_-]+'    && echo FAIL || echo PASS
curl -sL <APP_URL> | grep -Ei 'infura\.io/v3/[A-Za-z0-9_-]+'      && echo FAIL || echo PASS
curl -sL <APP_URL> | grep -Ei 'DATABASE_URL'                       && echo FAIL || echo PASS
curl -sL <APP_URL> | grep -Ei 'mainnet\.base\.org'                 && echo FAIL || echo PASS

# Positive-claim drift (must be ZERO true hits — disclaimer matches like
# "Is this audited?" or "Not mainnet-ready" are fine and expected).
curl -sL <APP_URL>/docs/limitations | grep -Ei '\b(is audited|mainnet-ready|production-ready|safe for real funds|guaranteed liquidity|institutional-grade)\b'
```

Repeat against `/trade`, `/markets`, `/portfolio`, `/docs/quickstart`.

---

## 7. After-deploy operator follow-up

Once `<APP_URL>` is live and the smoke + scans are clean:

* ☐ Re-run `OPERATOR-PUBLIC-BETA-URLS-FILL` with the new App URL — see `docs/OPERATOR_PUBLIC_BETA_URLS_FILL_NEXT_TASK.md` + the rerun task generated by this preflight (`docs/OPERATOR_PUBLIC_BETA_URLS_FILL_RERUN_NEXT_TASK.md`).
* ☐ Re-run `PUBLIC-TESTNET-BETA-LAUNCH-PREFLIGHT` — see `docs/PUBLIC_TESTNET_BETA_LAUNCH_PREFLIGHT_RERUN_NEXT_TASK.md`. Expected verdict: **READY** (or READY WITH NON-BLOCKING PLACEHOLDERS if API_BASE_URL and status page URL are still left as placeholder).

### What this checklist does NOT authorise

* Publishing the public announcement.
* Contacting an audit firm.
* Launching a bug bounty.
* Activating mainnet.
* Editing backend `.env`.
* Pasting an admin Bearer token into a public-app browser session.

Each of those is gated on a separate milestone with its own explicit approval line.

---

**End of frontend public testnet deploy operator checklist.**
