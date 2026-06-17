# FRONTEND-HOMEPAGE-COSMIC-PROTOCOL-LANDING — Result

**Date:** 2026-06-15
**Workspace root:** `~/DEOPT/`
**Approval line consumed:** "I approve DeOpt V2 homepage cosmic protocol landing for this run."

## TL;DR

`/` was a single emerald hero card plus the `MarketSelector`. It is
now an immersive vertical scroll story with a cosmic backdrop, eight
narrative sections, and the operator-supplied Greek-logo PNGs woven
into the hero composition. Body copy is intentionally clean of
testnet / "no real funds" repetition — those concerns are already
carried by the global `TestnetUnauditedBanner` above the navbar and
by the `PublicBetaFooter` below the page-mode wrapper.

## Homepage inventory (Phase A)

Before:
- Route: `src/app/(trading)/page.tsx` (rendered inside the `(trading)` group → banners + nav from layout, page-mode `<main max-w-screen-2xl>` + `PublicBetaFooter` from `TradingShell`).
- Hero: emerald pill, "On-chain options. Base Sepolia." heading, paragraph repeating no-real-funds / no-audit copy, three CTAs (Start testing → `/markets`, quickstart, report feedback), `<MarketSelector>` below.
- Assets in `public/`: `favicon.png`, `logo-deopt.png`, `file.svg`, `globe.svg`, `next.svg`, `vercel.svg`, `window.svg`.
- NEW in `public/greeks/` (operator-supplied): `Logo_Delta.png`, `Logo_Gamma.png`, `Logo_Rho.png`, `Logo_Theta.png`, `Logo_Vega.png`.
- Existing tests: `landing.spec.ts` (asserts banner + footer only — still valid), `landing-product-v2.spec.ts` (tightly coupled to the old hero copy — needs rewrite).

## Visual concept (Phase B)

A single component graph:

- `src/components/landing/CosmicBackdrop.tsx` — `fixed inset-0 -z-10 pointer-events-none` deep-black layer with three stacked sources: radial emerald glow (top-right + bottom-left), a 28px dot field at 6% alpha, scanlines at 4% alpha, three stylized SVG vol/data curves, and an outer vignette. Zero animation, zero deps, zero canvas.
- `src/components/landing/CosmicLanding.tsx` — composes the backdrop and eight sections (hero, protocol flow, options terminal, perps vision, architecture, risk engine, community, final CTA). All section components live in this file so the page is easy to read end-to-end.
- `src/app/(trading)/page.tsx` — now just `<CosmicLanding />`.

The landing root is `-mx-3 -my-3 lg:-mx-4 lg:-my-4` to break out of the page-mode `px-3/lg:px-4` padding and let the backdrop reach the viewport edges. The inner content is then re-centered via `max-w-6xl`.

## Hero section (Phase C)

- Chip "Programmable derivatives"
- Headline: **"Programmable derivatives infrastructure. From signed intent to settled position."** — second clause uses an emerald gradient text fill.
- Sub-headline mentions options + perpetuals + EIP-712 intents + programmable risk engine + collateral vault + oracle-aware settlement.
- Three CTAs: **Launch the terminal → `/trade`**, **Explore markets → `/markets`**, **Read the docs → `/docs`** (button styles in increasing visual weight).
- A 5-element Greek glyph constellation below using the operator-supplied PNGs (`Logo_{Delta,Gamma,Theta,Vega,Rho}.png`) — each glyph in a 48-56 px emerald-bordered circular tile with hover affordance and Greek name caption.

## Cosmic green visual layer (Phase D)

| Element | Implementation |
|---|---|
| Deep-black base | linear-gradient `#000` → `#080c0a` |
| Top-right emerald glow | radial-gradient `rgba(16,185,129,0.08)` |
| Bottom-left emerald glow | radial-gradient `rgba(16,185,129,0.06)` |
| Dot field | radial-gradient pattern, 28 px step, 6% alpha emerald-300 |
| Scanlines | repeating-linear-gradient, 4 px step, 4% alpha emerald-300 |
| Volatility smile curve | SVG path with horizontal gradient stroke |
| Data-stream curve (×2) | SVG path with horizontal gradient stroke |
| Outer vignette | radial-gradient transparent → 80% black at edges |

No frame-by-frame animation, no canvas, no WebGL, no JS-driven motion. All resources are inline. Page weight delta from a fresh `npm run build` is dominated by the 5 Greek PNGs (≈ 285 KB total) that the operator authored.

## Progressive scroll storytelling (Phase E)

| Section | testid | Purpose |
|---|---|---|
| Hero | `landing-hero` | Set the tone, present the protocol, route the user |
| Protocol flow | `landing-protocol-flow` | Intent → execution → settlement in 3 glass cards |
| Options terminal | `landing-options-section` | Explain the options surface, link to `/trade`, glass mock options chain |
| Perps vision | `landing-perps-section` | Position perps as part of the stack, link to `/perps`, glass mock chart |
| Architecture | `landing-architecture-section` | SVG node-graph diagram with 7 modules + arrowed edges |
| Risk · collateral · margin | `landing-risk-section` | Three concept cards (margin / vault / oracle) |
| Community · docs · API | `landing-community-section` | Four CTAs (docs, feedback, Discord, GitHub) |
| Final CTA | `landing-final-cta` | "Pick a market. Open the chain." with two routes |

Every card has a section eyebrow + heading + body + chips. Chips use the same emerald-on-zinc pill style as the navbar status chips. No yellow / orange / amber.

## Options section (Phase F)

- Two-column layout (text + glass mock).
- Mock renders a 3-row, 7-column option chain (Calls bid/mid/ask · Strike · Puts bid/mid/ask) with emerald accents, sample WETH 30d expiry, and a footer reading `Δ · Γ · Θ · Ν · Ρ`.
- Chip strip: Options chain / Payoff / Delta / Gamma / Vega / Theta / Collateral.
- CTA: "Open the options terminal →" → `/trade`.
- No live prices implied — quote characters look like premium values but the copy never claims this is a real chain.

## Perps section (Phase F)

- Inverted two-column layout.
- Mock renders a stylized SVG sparkline with gradient fill, plus a 4-cell metrics row (Mark / Funding / OI / 24h vol) all rendered as `—` placeholders.
- Chip strip: Perpetual / Funding / Mark price / Open interest / Cross margin.
- CTA: "Open the perps workspace →" → `/perps`.
- Wording deliberately calls it the **"perps workspace"** and **"interface ships ahead of the executor"** so the page does not pretend live perps trading exists.

## Architecture section (Phase G)

`<svg>` node-graph with 7 nodes (intent / executor / margin / vault / oracle / settlement / indexer) connected by 8 dashed emerald arrows. Each node is a circle with the module's slug inside and a human label below. Below the diagram: chip strip with EIP-712 / Oracle / Vault / Margin / Settlement / Indexer / Portfolio. The diagram has `role="img"` and `aria-label="DeOpt protocol architecture diagram"` for accessibility.

## Links and routing (Phase H)

| Target | Reached from |
|---|---|
| `/trade` | Hero "Launch the terminal", Options CTA, Final-CTA "Launch the terminal" |
| `/markets` | Hero "Explore markets", Final-CTA "Explore markets" |
| `/docs` | Hero "Read the docs", Community card |
| `/perps` | Perps CTA |
| `/feedback` | Community card |
| `https://discord.gg/zaEMvWuxu` | Community card (external) |
| `https://github.com/DeOpt` | Community card (external) |

No private/admin links. No RPC URLs. No localhost references. External links are `target="_blank" rel="noopener noreferrer"`.

## Responsive behavior (Phase I)

- Root grid uses `max-w-6xl` (1152 px) so the landing centers cleanly on desktop and never exceeds the page-mode wrapper.
- Section layouts use `grid-cols-1 lg:grid-cols-[1.1fr_1fr]` patterns — single column on mobile/tablet, two columns on desktop.
- Hero uses `text-4xl sm:text-5xl lg:text-6xl` for the headline so it scales without overflowing.
- Greek constellation: `grid-cols-5 gap-3 sm:gap-6` with `h-12 w-12 sm:h-14 sm:w-14` tile sizing.
- Backdrop is `fixed inset-0` and `overflow-hidden`; the SVG uses `preserveAspectRatio="none"` so it stretches to viewport without overflow.
- No horizontal scrollbars at 360 px viewport (sections rely on `gap-3` + `flex-wrap`).
- All `<Image>` uses Next.js `next/image` so the Greeks ship as optimized variants automatically.

## Tests added / updated (Phase J)

`tests/e2e/landing-product-v2.spec.ts` — rewritten (12 tests):

| Test | Coverage |
|---|---|
| renders backdrop + hero | `cosmic-backdrop` attached, `cosmic-landing` + `landing-hero` visible, headline visible |
| hero CTAs route correctly | Launch → `/trade`, Markets → `/markets`, Docs → `/docs` |
| Options + Perps mentioned and linked | both sections visible, CTAs route to `/trade` + `/perps` |
| scroll story renders all 8 sections | each section testid is attached |
| hero uses 5 Greek PNGs | 5 `landing-hero-greek-*` containers, each img src includes `/greeks/Logo_` |
| architecture diagram has all 7 protocol node labels | SVG `<text>` contains Signed intent / Executor / Risk engine / Collateral vault / Oracle router / Settlement / Indexer |
| community links | docs/feedback/Discord/GitHub hrefs |
| no positive-claim language in `<main>` | scrub for audited / mainnet-ready / production-ready / safe for real funds / guaranteed uptime / institutional-grade |
| body does NOT repeat testnet/public beta/no real funds inside `cosmic-landing` | the global banner + footer carry that copy; the landing body must not double it |
| no admin / bearer / RPC / DATABASE_URL leak | regex sweep of landing HTML |
| no yellow/orange/amber classes in landing HTML | regex sweep |
| final CTA routes | Launch → `/trade`, Markets → `/markets` |

`tests/e2e/landing.spec.ts` — left **unchanged**. Its assertions reference only the `TestnetUnauditedBanner` (in `(trading)/layout.tsx`) and the `PublicBetaFooter` (rendered by `TradingShell` page-mode), both untouched by this milestone.

Catalog: `npx playwright test --list` → **173 → 180 tests in 34 files** (+7 net after replacing the old V2 spec).

## Build validations (Phase L)

| Command | Result |
|---|---|
| `npm run typecheck` | clean |
| `npm run lint` | clean |
| `NEXT_PUBLIC_TRADING_API_BASE_URL=http://localhost:8080 npm run build` | green — same 19 user-facing routes + 4 SSG doc slugs |
| `npx playwright test --list` | 180 tests in 34 files |
| `scripts/local-backend.sh` → `local-seed.sh` → `local-smoke.sh` | startup green; seed 12 PASS, 4 products visible; smoke **9 PASS / 0 FAIL** |
| Full e2e run | NOT FEASIBLE on this WSL host (`libnspr4.so` missing in `chromium_headless_shell`) — unchanged from V5/V6/V7/IA. Catalog enumeration passes. |

Backend stopped post-QA; port 8080 free.

## Validations

| Check | Result |
|---|---|
| `git diff --check` (frontend) | clean |
| Sensitive-string scan on edited FE files | zero hits |
| Positive-claim scan on edited FE files | zero hits |
| Amber/yellow/orange class scan on edited FE files | zero hits |
| `.env` mtime preserved | YES (`2026-06-08 16:55:05.874571237 +0200`) |
| Private dir mode preserved | YES (`700`) |
| Backend stopped post-QA | YES (port 8080 free) |
| Chain tx / broadcast / mainnet RPC / real wallet | NONE |
| `isMainnetEnabled()` still hard-coded `false` | YES (file untouched) |
| Backend Rust / Solidity / scripts changes | NONE |
| New dependency added | NONE |
| Source changes limited to frontend homepage/assets + docs/RUN_STATE | YES |
| Remote image URLs introduced | NONE (only local `/greeks/Logo_*.png`) |
| Derive / competitor assets used | NONE |

## Docs created / updated

- NEW `docs/FRONTEND_HOMEPAGE_COSMIC_PROTOCOL_LANDING_RESULT.md` (this file)
- `docs/public-beta/PUBLIC_TESTNET_BETA_LAUNCH_CHECKLIST.md` left **unchanged** — the only relevant launch criterion ("a landing page exists for the public testnet beta") is still satisfied, and no checklist item references the old hero copy.

## RUN_STATE update

2026-06-15 closure for FRONTEND-HOMEPAGE-COSMIC-PROTOCOL-LANDING prepended above the navbar-IA entry.

## Files changed

**Edited (frontend src):**
- `src/app/(trading)/page.tsx` — now a 5-line wrapper around `<CosmicLanding />`

**Created (frontend src):**
- `src/components/landing/CosmicBackdrop.tsx`
- `src/components/landing/CosmicLanding.tsx`

**Rewritten (frontend tests):**
- `tests/e2e/landing-product-v2.spec.ts` — 12 new tests covering the cosmic landing

**Untouched assets (committed by operator):**
- `public/greeks/Logo_Delta.png`
- `public/greeks/Logo_Gamma.png`
- `public/greeks/Logo_Rho.png`
- `public/greeks/Logo_Theta.png`
- `public/greeks/Logo_Vega.png`

**Created (backend docs):**
- `docs/FRONTEND_HOMEPAGE_COSMIC_PROTOCOL_LANDING_RESULT.md` (this file)

**Edited (root):**
- `RUN_STATE.md` — closure prepended

**Untouched:** Backend Rust (ZERO), Solidity (ZERO), `scripts/local-*.sh` (ZERO), backend `.env` (mtime preserved), `~/DEOPT/private/` (mode 700), `(trading)/layout.tsx` (banners + nav untouched), `TradingShell.tsx` (page-mode + footer behaviour untouched), every other `(trading)` route page, `public-beta-links.ts`, `PublicBetaFooter.tsx`, every workspace file, every market/portfolio/feedback file.

## Remaining homepage gaps

- The cosmic backdrop is static. A future milestone could add a single subtle CSS-only `@keyframes` animation for the data-stream curve. Not in scope here — the brief asked for "subtle animated or static green flows" and "no heavy dependencies"; static was the safer choice.
- The options-chain mock uses repeated React fragment children inside a CSS grid (no keyed wrapper). Browsers render this correctly and React 19 emits a key-warning only when fragments have explicit `key` props. If a future test starts asserting against React warnings, the mock can be refactored into a typed array map.
- No SEO-specific metadata change yet (the root layout still ships `title: "DeOpt v2"`). A follow-up could land an `export const metadata` per route once the operator finalizes naming.
- No language switcher / i18n in scope.
- No analytics / event tracking added.

None block local QA, public-testnet-beta launch, or the operator's product-test.

## Next milestone recommendation

**Primary (operator action, not agent-runnable):** product-test the new landing on the local frontend via `bash ~/DEOPT/scripts/local-frontend.sh`. Confirm at 1440 / 1920 / 2560:
- Hero headline scales and the emerald gradient is visible
- Greek constellation renders crisply at desktop and mobile
- All scroll sections paint with the dot field + curve backdrop behind them
- "Launch the terminal" lands on `/trade`
- Options and Perps CTAs route correctly
- Discord and GitHub open in new tabs
- No horizontal scrollbar at 360 px viewport

**Secondary (agent-runnable):** `BACKEND-PUBLIC-TESTNET-DEPLOY-PREFLIGHT` per existing brief.

**Strictly later (NOT NOW):** subtle CSS animation pass, real options/perps mocks driven by mock backend data, SEO metadata, i18n, analytics, mainnet activation, audit firm outreach, bug bounty launch, KMS cutover, Safe migration, flipping `isMainnetEnabled()`.
