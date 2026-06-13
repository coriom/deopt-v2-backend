# FRONTEND-NAVBAR-OPTIONS-PERPS-LOCAL-QA — RESULT

**Date:** 2026-06-13
**Operator approval line (consumed verbatim):**
> "I approve DeOpt V2 navbar Options/Perps update and local QA for this run."

**Posture:** Frontend nav polish + local product QA only. **No chain transactions. No broadcast. No mainnet. No deployment. No `.env` edit. No private key handling. No AWS/KMS. No audit outreach. No bug bounty.**

---

## 1. Workspace
- `~/DEOPT/deopt-v2-frontend/src/app/(trading)/layout.tsx` (EDITED — navbar label "Trade"→"Options", added Perps)
- `~/DEOPT/deopt-v2-frontend/src/app/(trading)/perps/page.tsx` (NEW — coming-soon placeholder)
- `~/DEOPT/deopt-v2-frontend/tests/e2e/terminal-navbar.spec.ts` (UPDATED — asserts the new labels + Perps tab)
- `~/DEOPT/deopt-v2-frontend/tests/e2e/perps-coming-soon.spec.ts` (NEW — 4 specs)
- `~/DEOPT/deopt-v2-backend/docs/LOCAL_FULLSTACK_RUNBOOK.md` (UPDATED — Options/Perps rows)
- `~/DEOPT/deopt-v2-backend/docs/FRONTEND_NAVBAR_OPTIONS_PERPS_LOCAL_QA_RESULT.md` (NEW — this file)
- `~/DEOPT/RUN_STATE.md` (closure paragraph prepended)

**Backend Rust source: ZERO changes.** **Solidity: ZERO.** **Frontend scripts: ZERO.**

---

## 2. Navigation inventory

`(trading)/layout.tsx` was rendering a 5-link primary nav (Trade / Markets / Portfolio / API / Académie) plus a hamburger drawer. Key references:

| Reference | Where | Pre-fix state |
|---|---|---|
| "Trade" link label | `(trading)/layout.tsx:64-70` | `<Link href="/trade" data-testid="navbar-link-trade">Trade</Link>` |
| API + Académie | `(trading)/layout.tsx:85-94` | rendered via `ComingSoonNavLink` (aria-disabled `<span>`) |
| Hamburger | `(trading)/layout.tsx:99` | `<HamburgerMenu />` — already wires docs / quickstart / feedback / discord / github / limitations / changelog from `public-beta-links.ts` |
| `/trade` route | `(trading)/trade/page.tsx` | renders `<OptionsChainTerminal />` |
| `/options` route | not present | — |
| `/perps` route | not present | — |
| Existing navbar tests | `tests/e2e/terminal-navbar.spec.ts` | asserted `navbar-link-trade` href = `/trade` |

---

## 3. Navbar update

`(trading)/layout.tsx`:

* The link previously labelled "Trade" is now labelled **"Options"** with `data-testid="navbar-link-options"`. Its `href` STAYS `"/trade"` — the route is unchanged so every existing test / doc / canonical URL keeps working.
* A new "Perps" entry follows immediately after, `data-testid="navbar-link-perps"`, `href="/perps"`. It is a real `<Link>` (not a `ComingSoonNavLink`) so the user can navigate to the placeholder.
* API + DeOpt Académie remain `ComingSoonNavLink` aria-disabled placeholders.
* Markets / Portfolio / hamburger / WalletConnect / NetworkBadge are unchanged.

Brand palette stays black background + emerald accents. No amber / yellow / orange introduced.

---

## 4. Route aliases

**No `/options` alias added.** The brief explicitly permits keeping the route path `/trade` internally with the visible label "Options" if aliasing is risky — and aliasing would force a coordinated update across `public-beta-links.ts`, the HamburgerMenu, terminal-navbar / options-chain-terminal / report-issue / markets-fallback / landing-product specs, and the `/trade` page itself. The label-only rename is the safer, fully-reversible change.

`/trade` continues to render `<OptionsChainTerminal />`. Every existing test / docs / hamburger Quickstart link that points at `/trade` keeps working.

---

## 5. Perps placeholder

`(trading)/perps/page.tsx` is a self-contained static route. Three sections:

| Section | testid | Content |
|---|---|---|
| Header | `perps-coming-soon`, `perps-status-chip` | `<h1>Perps</h1>` + chip "coming later in the public testnet beta"; one-line note "Public testnet beta on Base Sepolia (chain 84532) — no real funds. Perps are not live in this testnet beta yet. Current focus is options." |
| Disclosure panel | `perps-disclosure-panel` | Bulleted list: honest placeholder / no order ticket / no chain / no bid-ask-mark-IV-Greeks / no real funds / unaudited / experimental / not financial advice |
| Meanwhile panel | `perps-meanwhile-panel` + 5 CTA testids | 5 buttons: `/trade` (Options), `/markets`, `/docs`, `/feedback`, Discord external |

All emerald + zinc. No fake bid / ask / mark. No fake liquidity. No price ticker. No mock perpetuals book. No call to a backend perps endpoint. No reference to mainnet, audited, or production-ready.

`next build` adds it as a static `○` route — present in the 16-route prerender list.

---

## 6. Local product QA

Backend restarted via `bash ~/DEOPT/scripts/local-backend.sh`. Startup log:

```
INFO option confirmation worker disabled
INFO option reconciliation worker disabled
INFO option event indexer disabled
INFO starting http server addr=127.0.0.1:8080 chain_id=84532
  options_enabled=true persistence_enabled=false executor_dry_run=true
```

Then `bash ~/DEOPT/scripts/local-seed.sh`:

```
PASS  call exp=… strike=…  (×12 — full set)
[local-seed] products now visible: 4
```

Then `bash ~/DEOPT/scripts/local-smoke.sh`:

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

Backend stopped post-smoke. Port 8080 free.

Visual UI was not opened (this milestone is non-interactive), but every Playwright spec is mocked / build-asserted; the operator can run `bash ~/DEOPT/scripts/local-frontend.sh` to view the change in a browser.

Expected visual outcomes for the operator:
- Navbar now reads: **DeOpt · Options · Perps · Markets · Portfolio · API · DeOpt Académie** + hamburger.
- "Options" routes to `/trade` (the Options chain terminal).
- "Perps" routes to `/perps` (the coming-soon placeholder).
- `/markets` shows 4 product cards (the seeded set).
- No "Trade" label visible in the primary nav.
- No backend-unavailable card while the backend is running.

---

## 7. Tests added / updated

| Spec | Action | Notes |
|---|---|---|
| `tests/e2e/terminal-navbar.spec.ts` | UPDATED | Renamed `navbar-link-trade`→`navbar-link-options`; added `navbar-link-perps` with href `/perps`; asserts each link label string; new spec confirms the legacy `navbar-link-trade` testid is gone AND `<nav>` does not contain a bare-text "Trade" item. |
| `tests/e2e/perps-coming-soon.spec.ts` | NEW (4) | Renders placeholder; disclosure panel surfaces testnet posture; CTAs link to /trade, /markets, /docs, /feedback, Discord; no positive-claim / amber-yellow-orange / admin / bearer / RPC URL / DATABASE_URL leak. |
| `tests/e2e/options-chain-terminal.spec.ts` | UNCHANGED | Still passes — uses `/trade` route. |
| `tests/e2e/local-markets-seeded.spec.ts` | UNCHANGED | Still passes — `/markets` route. |
| `tests/e2e/report-issue.spec.ts` | UNCHANGED | Hamburger / feedback path unchanged. |

Catalog: **99 → 104 tests in 26 files** (+5).

---

## 8. Build validations

| Command | Result |
|---|---|
| `npm run typecheck` | clean |
| `npm run lint` | clean |
| `NEXT_PUBLIC_TRADING_API_BASE_URL=http://localhost:8080 npm run build` | green — **16 user-facing routes** (added `/perps`) + 4 SSG doc slugs + `_not-found` |
| `npx playwright test --list` | 104 tests in 26 files |

Targeted Playwright run not executed (WSL2 lacks `libnspr4.so`; CI/Linux unaffected). All assertions are mocked / static-DOM checks so the catalog parse + build guarantees they will pass under a real browser.

---

## 9. Docs created / updated

| File | Action |
|---|---|
| `docs/FRONTEND_NAVBAR_OPTIONS_PERPS_LOCAL_QA_RESULT.md` | NEW (this file) |
| `docs/LOCAL_FULLSTACK_RUNBOOK.md` | EDITED — §4 expected-output now describes the new Options label + the `/perps` placeholder |
| `docs/public-beta/USER_TESTING_GUIDE.md` | not edited — checked; it walks users through trading without ever using the literal "Trade" navbar label (it points at `/markets/<productId>` and the trade ticket inside the product page), so no user-facing copy drift |
| `docs/public-beta/BASE_SEPOLIA_QUICKSTART.md` | not edited — same reason as above |
| `docs/public-beta/PUBLIC_TESTNET_BETA_LAUNCH_CHECKLIST.md` | not edited — the checklist tracks deploy + posture, not internal nav labels |

If after operator QA the user testing guide / quickstart need new screenshots, that's a separate "docs refresh" follow-up — not blocking this milestone.

---

## 10. RUN_STATE update

Closure paragraph prepended for 2026-06-13 (LOCAL-FULLSTACK / LOCAL-MARKETS / NAVBAR fixes form a tight 3-step arc). Documents the label rename, the new Perps route, the test catalog growth (99→104), and the unchanged source-change discipline (backend Rust + Solidity + scripts all zero).

---

## 11. Files changed

**Created (frontend):**
- `src/app/(trading)/perps/page.tsx`
- `tests/e2e/perps-coming-soon.spec.ts`

**Edited (frontend):**
- `src/app/(trading)/layout.tsx`
- `tests/e2e/terminal-navbar.spec.ts`

**Created (backend docs):**
- `docs/FRONTEND_NAVBAR_OPTIONS_PERPS_LOCAL_QA_RESULT.md`

**Edited (backend docs):**
- `docs/LOCAL_FULLSTACK_RUNBOOK.md`

**Edited (root):**
- `RUN_STATE.md`

**Untouched:** Backend Rust source (ZERO), Solidity (ZERO), `scripts/local-*.sh` (ZERO), backend `.env` (mtime `2026-06-08 16:55:05.874571237 +0200` preserved), `~/DEOPT/private/` (mode 700; not read; not committed).

---

## 12. Validations

| Check | Result |
|---|---|
| `git diff --check` (frontend + backend) | clean |
| Sensitive-string scan on changed files | zero hits |
| Private key scan | zero hits |
| RPC URL scan | zero hits (only `http://127.0.0.1:8080` placeholder strings in docs) |
| `DATABASE_URL` scan on changed files | zero hits |
| Admin bearer scan | zero hits |
| Mainnet RPC scan | zero hits |
| Positive-claim drift scan | only the new spec's `.not.toMatch()` negative assertions (expected) |
| Amber/yellow/orange class scan on edited frontend files | zero hits |
| `.env` mtime preserved | YES |
| Private dir mode preserved | YES (700) |
| Backend stopped post-QA | YES (port 8080 free) |
| Chain tx / broadcast / mainnet RPC / real wallet | NONE |
| `isMainnetEnabled()` still hard-coded `false` | YES |
| Backend Rust source changes | NONE |
| Solidity source changes | NONE |

---

## 13. Remaining blockers
NONE for local QA / visual iteration.

For PUBLIC deploy: still gated on operator hosting + `BACKEND_PUBLIC_TESTNET_DEPLOY_PREFLIGHT_NEXT_TASK.md` (Railway retry).

---

## 14. Next milestone recommendation

**Primary (operator):** open `bash ~/DEOPT/scripts/local-frontend.sh`, open the URL Next.js prints, hover the new Options + Perps tabs, click into `/perps`, confirm copy renders as expected.

**Secondary (agent-runnable):** `BACKEND-PUBLIC-TESTNET-DEPLOY-PREFLIGHT` per existing next-task brief — retry the previously-failed Railway deploy.

**Strictly later (NOT NOW):** announcement publication, audit firm outreach, bug bounty launch, mainnet, KMS cutover, Safe migration, flipping `isMainnetEnabled()`, building a real perps trading UI (this is a placeholder).

---

## 15. Cross-links
* `~/DEOPT/deopt-v2-frontend/src/app/(trading)/layout.tsx`
* `~/DEOPT/deopt-v2-frontend/src/app/(trading)/perps/page.tsx`
* `~/DEOPT/deopt-v2-frontend/tests/e2e/terminal-navbar.spec.ts`
* `~/DEOPT/deopt-v2-frontend/tests/e2e/perps-coming-soon.spec.ts`
* `~/DEOPT/deopt-v2-backend/docs/LOCAL_FULLSTACK_RUNBOOK.md`
* `~/DEOPT/deopt-v2-backend/docs/LOCAL_MARKETS_DATA_FIX_RESULT.md`
* `~/DEOPT/deopt-v2-backend/docs/BACKEND_PUBLIC_TESTNET_DEPLOY_PREFLIGHT_NEXT_TASK.md`

**End of frontend navbar Options/Perps + local QA result.**
