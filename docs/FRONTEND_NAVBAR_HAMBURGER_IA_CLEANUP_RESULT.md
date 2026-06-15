# FRONTEND-NAVBAR-HAMBURGER-IA-CLEANUP — Result

**Date:** 2026-06-15
**Workspace root:** `~/DEOPT/`
**Approval line consumed:** "I approve DeOpt V2 navbar and hamburger information architecture cleanup for this run."

## TL;DR

The trading-terminal navbar is now compact and trading-focused.
`Portfolio` and `API` are removed from the primary navbar and live
in a redesigned hamburger drawer with three sections:
**Pages** / **Docs** / **Community**. Two new placeholder routes
`/fees` and `/api` are wired up (page-mode, with footer, honest
testnet-only copy). `/portfolio` still works.

## Navigation inventory (Phase A)

Before:
- Primary navbar: `Options · Perps · Markets · Portfolio · Custom · API (placeholder) · DeOpt Académie (placeholder)`
- Hamburger: `Docs · Quickstart · Feedback · Discord · GitHub · Limitations · Changelog (coming soon)`
- Existing routes: `/trade`, `/perps`, `/markets`, `/portfolio`, `/custom`, `/docs`, `/docs/{quickstart,testing-guide,limitations,faq}`, `/feedback`, `/history`, `/health`, `/admin`, `/transactions/[id]`, `/markets/[id]`
- Missing routes: `/api`, `/fees`
- `/docs/faq` was already wired up via `SLUG_TO_FILE.faq → FAQ.md` in `src/lib/docs-loader.ts`
- Tests referencing legacy navbar IA: `tests/e2e/terminal-navbar.spec.ts` (Portfolio + API expected as primary)

## Main navbar cleanup (Phase B)

`src/app/(trading)/layout.tsx`:
- Removed `<Link navbar-link-portfolio>` and `<ComingSoonNavLink navbar-link-api>`.
- Kept `Options / Perps / Markets / Custom` as primary `<Link>`s.
- Kept `DeOpt Académie` as `<ComingSoonNavLink>`.
- Right-side controls unchanged: `NetworkBadge / WalletConnectButton / WidgetMenuButton / HamburgerMenu`.
- Banners unchanged: `TestnetUnauditedBanner / MainnetDisabledBanner / WrongNetworkBanner`.
- Black / deep-green identity preserved.

## Hamburger menu IA (Phase C)

`src/components/HamburgerMenu.tsx` rewritten:
- New three-section structure with `data-testid="hamburger-section-{pages|docs|community}"`.
- Defined locally (not via the shared `PUBLIC_BETA_LINKS` array) so the public footer remains unchanged.
- Drawer entries:

| Section | Label | Href | Internal |
|---|---|---|---|
| Pages | Portfolio | `/portfolio` | ✅ |
| Pages | Fees | `/fees` | ✅ |
| Pages | API | `/api` | ✅ |
| Pages | Feedback | `/feedback` | ✅ |
| Docs | Docs | `/docs` | ✅ |
| Docs | Quickstart | `/docs/quickstart` | ✅ |
| Docs | Known limitations | `/docs/limitations` | ✅ |
| Docs | FAQ | `/docs/faq` | ✅ |
| Community | Discord | `https://discord.gg/zaEMvWuxu` | external |
| Community | GitHub | `https://github.com/DeOpt` | external |

- Internal entries render via `<Link>`, external via `<a target="_blank" rel="noopener noreferrer">`.
- Drawer opens via the existing hamburger button, closes on outside click / Escape / link click.
- The drawer is now `overflow-y-auto` to stay usable if the section list grows.
- No admin links, no bearer tokens, no RPC URLs, no DATABASE_URL, no mainnet links.

## Fees route (Phase D)

`src/app/(trading)/fees/page.tsx` (NEW):
- Status chip: `Public testnet beta · Base Sepolia · unaudited`.
- Honest summary: "Fee documentation is being prepared … nothing here is final, nothing here is mainnet-ready, and nothing here implies safety for real funds."
- "What you can rely on right now" disclaimer block: testnet only, mock tokens, no live fee schedule, no rebate tier.
- "What lands later" roadmap block: protocol fee schedule, settlement breakdown, rebate policy once maker side ships.
- Three follow-up cards: `/docs`, `/feedback`, Discord.
- No fake fees schedule. No mainnet readiness implication. No positive-claim language.

## API route (Phase E)

`src/app/(trading)/api/page.tsx` (NEW):
- Status chip: `Public testnet beta · Base Sepolia · unaudited`.
- Honest summary: "The public testnet API surface is still being prepared. Endpoints listed below are testnet-only, may change without notice, and are not safe to rely on for any production integration."
- "What you can rely on right now": public read-side `/health`, `/options/products`, `/markets` (chain 84532); no public write API; no admin endpoints on the public surface.
- "What lands later": OpenAPI reference, stable URL schema, rate-limit + auth guidance.
- Three follow-up cards: `/docs`, GitHub, `/feedback`.
- No fake API reference. No admin paths. No private RPC URLs. No private backend base URL.

Both pages fall into the `(trading)` route group, which makes them
inherit the same banners + navbar shell as the trading routes. Because
they are **not** listed in `TRADING_SHELL.TERMINAL_ROUTES`, they
render in page-mode (max-width content area + `PublicBetaFooter`).

## Tests added / updated

| Spec | Action | Coverage |
|---|---|---|
| `tests/e2e/terminal-navbar.spec.ts` | REWRITTEN (7 tests) | Primary navbar has Options/Perps/Markets/Custom + Académie placeholder ONLY; Portfolio + API have count 0 in the navbar; legacy "Trade" label still gone; hamburger drawer shows three sections; Pages/Docs/Community entries with correct hrefs; no admin/mainnet/bearer/RPC URL leak; Escape closes drawer; `/portfolio` route still works |
| `tests/e2e/fees-and-api-placeholders.spec.ts` | NEW (7 tests) | `/fees` renders chip + summary + disclaimers + roadmap + 3 follow-up links; `/api` same; neither contains positive-claim or sensitive leaks; hamburger → Portfolio routes to `/portfolio`; hamburger → API routes to `/api`; hamburger → Fees routes to `/fees` |

Catalog: **165 → 173 tests in 33 → 34 files** (+8).

## Build validations

| Command | Result |
|---|---|
| `npm run typecheck` | clean |
| `npm run lint` | clean |
| `NEXT_PUBLIC_TRADING_API_BASE_URL=http://localhost:8080 npm run build` | green — 19 user-facing routes + 4 SSG doc slugs (added `/fees`, `/api`) |
| `npx playwright test --list` | 173 tests in 34 files |
| `scripts/local-backend.sh` → `local-seed.sh` → `local-smoke.sh` | startup green; seed 12 PASS, 4 products visible; smoke **9 PASS / 0 FAIL** |
| Full e2e run | NOT FEASIBLE on this WSL host (`libnspr4.so` missing in `chromium_headless_shell`) — unchanged from V5/V6/V7. Catalog enumeration passes. |

Backend stopped post-QA; port 8080 free.

## Docs created / updated

- NEW `docs/FRONTEND_NAVBAR_HAMBURGER_IA_CLEANUP_RESULT.md` (this file)
- `docs/public-beta/USER_TESTING_GUIDE.md` left **unchanged** — its body references "Open the Account / Portfolio page" without naming the navbar, so the new IA (hamburger → Portfolio) still satisfies the instruction.
- `docs/public-beta/BASE_SEPOLIA_QUICKSTART.md` left **unchanged** — its "Open the Portfolio page" instruction does not specify the navbar; the hamburger entry is the new path.
- `docs/public-beta/PUBLIC_TESTNET_BETA_LAUNCH_CHECKLIST.md` left **unchanged** — no navbar-tab references inside the checklist.

If the operator wants the prose to call out the new hamburger entry explicitly, that becomes a tiny follow-up — none of the instructions are broken by the IA change, so this milestone does not edit them.

## RUN_STATE update

2026-06-15 closure for FRONTEND-NAVBAR-HAMBURGER-IA-CLEANUP prepended above the V7 entry. Documents the navbar trimming, hamburger IA, two new placeholder routes, and the 173-test catalog.

## Files changed

**Edited (frontend src):**
- `src/app/(trading)/layout.tsx` — removed Portfolio + API primary nav items
- `src/components/HamburgerMenu.tsx` — three-section IA (Pages / Docs / Community); local entry definitions

**Created (frontend src):**
- `src/app/(trading)/fees/page.tsx` — `/fees` placeholder
- `src/app/(trading)/api/page.tsx` — `/api` placeholder

**Rewritten (frontend tests):**
- `tests/e2e/terminal-navbar.spec.ts` — new IA expectations (7 tests)

**Created (frontend tests):**
- `tests/e2e/fees-and-api-placeholders.spec.ts` — placeholder routes + hamburger routing (7 tests)

**Created (backend docs):**
- `docs/FRONTEND_NAVBAR_HAMBURGER_IA_CLEANUP_RESULT.md` (this file)

**Edited (root):**
- `RUN_STATE.md` — V7+1 closure prepended

**Untouched:** Backend Rust (ZERO), Solidity (ZERO), `scripts/local-*.sh` (ZERO), backend `.env` (mtime preserved), `~/DEOPT/private/` (mode 700), `src/lib/public-beta-links.ts` (footer + CTA logic unchanged), `src/components/PublicBetaFooter.tsx` (still iterates PUBLIC_BETA_LINKS unchanged), every other `(trading)` route page.

## Validations

| Check | Result |
|---|---|
| `git diff --check` (frontend + backend) | clean |
| Sensitive-string scan on changed FE files | zero hits |
| Positive-claim scan on changed FE files | zero positive hits (only negations in copy + test assertions) |
| Amber/yellow/orange class scan on changed FE files | zero hits |
| `.env` mtime preserved | YES (`2026-06-08 16:55:05.874571237 +0200`) |
| Private dir mode preserved | YES (`700`) |
| Backend stopped post-QA | YES (port 8080 free) |
| Chain tx / broadcast / mainnet RPC / real wallet | NONE |
| `isMainnetEnabled()` still hard-coded `false` | YES (file untouched) |
| Backend Rust / Solidity / scripts changes | NONE |
| New dependency added | NONE |
| Source changes limited to frontend + docs/RUN_STATE | YES |

## Remaining navigation gaps

- `DeOpt Académie` is still a coming-soon placeholder in the navbar — operator intent.
- `/fees` and `/api` are honest placeholders; real schedules / reference land in later milestones.
- `Changelog` was dropped from the new hamburger because (a) the brief did not require it and (b) it was a non-clickable placeholder with no body. Easy to re-introduce as a new docs slug + drawer entry if operator wants it back.
- No keyboard shortcut to open the hamburger (only the click + Escape-to-close). Not required by the brief.

None block local QA, public-testnet-beta launch, or the operator's product-test.

## Next milestone recommendation

**Primary (operator action, not agent-runnable):** product-test the new IA on the local terminal via `bash ~/DEOPT/scripts/local-frontend.sh`. Confirm:
- Primary navbar shows only Options / Perps / Markets / Custom / Académie
- Hamburger opens → three sections visible (Pages / Docs / Community)
- Clicking Portfolio in the hamburger lands on the existing `/portfolio` page
- Clicking Fees in the hamburger lands on the new placeholder
- Clicking API in the hamburger lands on the new placeholder
- Discord and GitHub open in new tabs
- No visible Reset Layout, no "Anonymous layout temporary" message, no bottom footer on terminal routes

**Secondary (agent-runnable):** `BACKEND-PUBLIC-TESTNET-DEPLOY-PREFLIGHT` per existing brief.

**Strictly later (NOT NOW):** real fee schedule, OpenAPI reference, mobile portrait responsive layout, drag-from-menu, mainnet activation, audit firm outreach, bug bounty launch, KMS cutover, Safe migration, flipping `isMainnetEnabled()`.
