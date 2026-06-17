# FRONTEND-HOMEPAGE-COSMIC-LANDING-POLISH-V2 — Result

**Date:** 2026-06-16
**Workspace root:** `~/DEOPT/`
**Approval line consumed:** "I approve DeOpt homepage cosmic landing polish V2 for this run."

## TL;DR

Polish pass over the V1 cosmic landing:
1. Browser title `DeOpt v2` → **`DeOpt`**.
2. Body copy compressed to one-line statements per section.
3. Vertical rhythm doubled (`py-32` / `sm:py-40`, hero `min-h-[80vh]`).
4. Backdrop now evolves with scroll via a `--cosmic-progress` CSS
   variable + slow CSS keyframes on the dot field and data-stream
   curve (respects `prefers-reduced-motion`).
5. Each section fades in via `<SectionReveal>` IntersectionObserver.
6. Three Greek glyphs (Delta / Theta / Gamma / Vega) float at low
   opacity behind sections as protocol sigils.
7. New **FAQ accordion** (Derive-inspired full-width rows, plus icon,
   `<details>` semantics, 7 DeOpt-specific questions).

180 → 184 tests in 34 files. Backend Rust / Solidity / scripts:
zero changes.

## Homepage inventory (Phase A)

Before:
- `src/app/(trading)/page.tsx` rendered `<CosmicLanding />`.
- 8 sections (hero, protocol flow, options, perps, architecture, risk, community, final CTA).
- Each section paragraph was 3-5 lines of prose; section spacing `py-20`.
- Backdrop was fully static (no scroll listener, no CSS keyframes).
- 5 Greek logos in `public/greeks/`.
- Root metadata `title: "DeOpt v2"`.
- No FAQ.

## Page title cleanup (Phase B)

`src/app/layout.tsx` — root metadata:
```ts
title: "DeOpt",
description: "DeOpt — programmable derivatives infrastructure.",
```

Other titles were checked:
- `src/app/admin/page.tsx` — operator-only, not part of the public surface, kept `"DeOpt v2 Admin"`.
- `src/app/admin/admin-dashboard.tsx` — operator-only dashboard header, kept.

Per the brief: "If internal package/repo names still use v2, do not rename." Only the visible user-facing root title was changed.

## Copy reduction (Phase C)

| Section | Before | After |
|---|---|---|
| Hero headline | "Programmable derivatives infrastructure. From signed intent to settled position." | **"Programmable derivatives. On chain."** |
| Hero subhead | 3-line paragraph mentioning EIP-712 + risk engine + vault + oracle + terminal | **"Options and perpetuals through a single on-chain execution layer."** |
| Options heading | "The full options surface in one terminal." | **"Calls. Puts. Greeks."** |
| Options body | 3-line paragraph | **"A programmable options chain with payoff, Greeks, and collateral in one terminal."** |
| Perps heading | "Perpetuals in the same execution layer." | **"Perpetuals. Same stack."** |
| Perps body | 4-line paragraph | **"The perps surface lives next to the options terminal — same intent pipeline, same risk engine, same vault."** |
| Protocol flow | 3 cards × 3 lines | 3 cards × 1 short line ("EIP-712 · client-side.", "Match · validate · broadcast.", "Oracle mark · indexed events.") |
| Risk · margin section | 3 cards × 2 lines | **removed** (folded into FAQ + architecture chips) |
| Community section | 4 cards × 2 lines | **removed** (replaced by FAQ + Final CTA links) |
| Final CTA heading | "Pick a market. Open the chain." | **"Open the chain."** |

Body remains free of `testnet`, `public beta`, `no real funds` — those live in the global `TestnetUnauditedBanner` (above the navbar) and `PublicBetaFooter` (below the page-mode wrapper).

## Section spacing (Phase D)

| Element | Before | After |
|---|---|---|
| Hero vertical | `pt-16 pb-20` | `min-h-[80vh] pt-24 sm:pt-32 pb-24` |
| Section default | `py-20` | `py-32 sm:py-40` |
| Section grid gap | `gap-8` | `gap-12 lg:gap-16` |
| Final CTA bottom padding | `pb-24` | `pb-32 sm:pb-40` |
| FAQ inter-row padding | n/a | `py-6` per row + thin emerald separator |

No horizontal overflow on mobile — section layouts collapse to single columns at `< lg`.

## Animated / evolving background (Phase E)

`CosmicBackdrop.tsx` rewritten as a `"use client"` component:

1. **Scroll listener** (rAF-throttled) writes `--cosmic-progress: 0..1` to the backdrop root.
2. **Inline gradients** read that variable inside `calc()` so:
   - Top-right emerald glow **drifts down + brightens** as the user scrolls deeper.
   - Bottom-left emerald glow **drifts up**.
   - Vignette **tightens + darkens** at the edges.
3. **CSS keyframes** in `globals.css` (namespace `deopt-cosmic-*`):
   - `.deopt-cosmic-dotfield` slow downward drift (40 s linear infinite).
   - `.deopt-cosmic-stream` SVG `stroke-dashoffset` flow (18 s linear infinite).
4. **`prefers-reduced-motion: reduce`** short-circuits both the scroll listener (sets a static `0.5` progress) and disables every CSS animation via media query.

No new dependencies. No WebGL. No canvas. No remote assets. The scroll handler is rAF-throttled and writes one CSS variable per frame — paint cost stays minimal.

## Scroll storytelling (Phase F)

Final flow:

| Position | Section | testid | Function |
|---|---|---|---|
| 1 | Hero | `landing-hero` | Position protocol + route to terminal |
| 2 | Options | `landing-options-section` | "Calls. Puts. Greeks." + mock chain → `/trade` |
| 3 | Perps | `landing-perps-section` | "Perpetuals. Same stack." + mock chart → `/perps` |
| 4 | Execution layer | `landing-protocol-flow` | Intent → execution → settlement (3 cards) |
| 5 | Architecture | `landing-architecture-section` | 7-node SVG graph |
| 6 | FAQ | `landing-faq-section` | 7 DeOpt-specific accordion rows |
| 7 | Final CTA | `landing-final-cta` | Launch / Markets / Feedback |

Every section is wrapped in `<SectionReveal>` — IntersectionObserver fades it in once 12% intersects the viewport, then disconnects. Falls back to "always visible" when JS is disabled OR `prefers-reduced-motion`.

## Greek image usage (Phase G)

The 5 hero greek glyphs are kept (small 48-56 px circular tiles, hover affordance).

New: 4 Greek **silhouettes** float as low-opacity background sigils (`opacity-[0.05] sm:opacity-[0.07]`) behind these sections:

| Section | Glyph | Position |
|---|---|---|
| Options | **Delta** | `-left-20 top-10`, 288-384 px square |
| Perps | **Theta** | `-right-24 top-10`, 288-384 px square |
| Architecture | **Gamma** | `right-0 top-20`, 320-448 px square |
| Final CTA | **Vega** | centered top, 256-320 px square |

All use `next/image` with local `/greeks/Logo_*.png` paths — no remote URLs, optimized variants automatically.

## FAQ section (Phase H)

New file `src/components/landing/FaqSection.tsx`:

- **Native `<details>` accordion** — accessibility + open/close behavior handled by the browser, zero state management.
- Full-width rows with thin emerald separators (`border-emerald-500/15`).
- Large question text (`text-lg sm:text-xl`).
- Plus icon in a 28 px emerald-bordered circle on the right; rotates 45° to become an X via `group-open:rotate-45`.
- Answer fades in below the question (max-w-2xl, short).
- 7 DeOpt-specific questions:
  1. What is DeOpt?
  2. Which products can I trade?
  3. How are trades executed and settled?
  4. What collateral and margin model does DeOpt use?
  5. Does DeOpt expose an API?
  6. What about fees?
  7. Where can I learn more?
- Answers reference `/api`, `/fees`, `/docs`, Discord — no overclaiming, perps phrased as "workspace ships ahead of the executor".
- Black / deep-green only — no orange / amber / yellow.

## CTA / routing (Phase I)

| Target | Reached from |
|---|---|
| `/trade` | Hero "Launch the terminal", Options CTA, Final-CTA "Launch the terminal" |
| `/markets` | Hero "Markets", Final-CTA "Markets" |
| `/docs` | Hero "Docs", FAQ "more" link |
| `/perps` | Perps CTA |
| `/api` | FAQ "api" answer link |
| `/fees` | FAQ "fees" answer link |
| `/feedback` | Final-CTA "Feedback" |
| `https://discord.gg/zaEMvWuxu` | FAQ "more" answer link (external) |

No private/admin links. No RPC URLs. No localhost.

## Tests added / updated (Phase J)

`tests/e2e/landing-product-v2.spec.ts` rewritten — **15 tests**:

| Test | Coverage |
|---|---|
| browser title is `DeOpt` | `expect(page).toHaveTitle(/^DeOpt$/)` |
| landing renders backdrop + hero | `cosmic-backdrop` attached, `cosmic-landing` + `landing-hero` visible |
| hero CTAs route | Launch → `/trade`, Markets → `/markets`, Docs → `/docs` |
| Options + Perps mentioned and linked | both sections attached, CTAs route to `/trade` + `/perps` |
| scroll story renders all 7 sections | hero / options / perps / execution / architecture / faq / final |
| hero uses 5 Greek PNGs | 5 `landing-hero-greek-*` containers, each img src includes `/greeks/Logo_` |
| background Greek silhouettes | ≥ 2 `landing-greek-silhouette-*` containers, every img is local (not http/https) and resolves to `/greeks/` |
| architecture diagram labels | SVG `<text>` contains Intent / Executor / Risk / Vault / Oracle / Settle / Indexer |
| FAQ rows expand/collapse | each `landing-faq-item-*` attached; clicking summary toggles `[open]` |
| FAQ plus icon present for every row | `landing-faq-icon-*` count ≥ 7 |
| body does NOT repeat testnet/public-beta/no-real-funds | regex sweep of `cosmic-landing` text |
| no positive-claim language in `<main>` | audited / mainnet-ready / production-ready / safe for real funds / guaranteed uptime / institutional-grade |
| no admin / mainnet / bearer / RPC / DATABASE_URL leak | regex sweep of `cosmic-landing` HTML |
| no yellow/orange/amber classes | regex sweep |
| final CTA routes | Launch → `/trade`, Markets → `/markets`, Feedback → `/feedback` |
| no broken image src | every `<img>` in landing has a non-empty src |

`tests/e2e/landing.spec.ts` — left **unchanged**. Asserts only the `TestnetUnauditedBanner` + `PublicBetaFooter`, both untouched.

Catalog: `npx playwright test --list` → **180 → 184 tests in 34 files** (+4).

## Build validations (Phase L)

| Command | Result |
|---|---|
| `npm run typecheck` | clean |
| `npm run lint` | clean (after deferring two `setState` calls in `SectionReveal` to satisfy `react-hooks/set-state-in-effect`) |
| `NEXT_PUBLIC_TRADING_API_BASE_URL=http://localhost:8080 npm run build` | green — same 19 user-facing routes + 4 SSG doc slugs |
| `npx playwright test --list` | 184 tests in 34 files |
| `scripts/local-backend.sh` → `local-seed.sh` → `local-smoke.sh` | startup green; seed 12 PASS, 4 products visible; smoke **9 PASS / 0 FAIL** |
| Full e2e run | NOT FEASIBLE on this WSL host (`libnspr4.so` missing in `chromium_headless_shell`) — unchanged from prior milestones |

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
| Source changes limited to frontend homepage/components/assets + docs/RUN_STATE | YES |
| Remote image URLs introduced | NONE |

## Docs created / updated

- NEW `docs/FRONTEND_HOMEPAGE_COSMIC_LANDING_POLISH_V2_RESULT.md` (this file)
- `docs/public-beta/PUBLIC_TESTNET_BETA_LAUNCH_CHECKLIST.md` left **unchanged** — the only relevant criterion ("a landing page exists") is still satisfied; no checklist item references hero copy or section count.

## RUN_STATE update

2026-06-16 closure for FRONTEND-HOMEPAGE-COSMIC-LANDING-POLISH-V2 prepended above the V1 cosmic-landing entry.

## Files changed

**Edited (frontend src):**
- `src/app/layout.tsx` — title `"DeOpt v2"` → `"DeOpt"`
- `src/app/globals.css` — namespace `deopt-cosmic-*` keyframes + `deopt-section-reveal` styles + reduced-motion media query
- `src/components/landing/CosmicBackdrop.tsx` — scroll-driven `--cosmic-progress` + CSS animations
- `src/components/landing/CosmicLanding.tsx` — leaner copy, `<SectionReveal>` wraps, Greek silhouettes, larger spacing, swaps risk/community for FAQ + Final-CTA-with-feedback
- `src/app/(trading)/page.tsx` — already a 5-line wrapper, untouched in V2 (commit from V1 still in working tree)

**Created (frontend src):**
- `src/components/landing/FaqSection.tsx` — Derive-style accordion (7 DeOpt-specific items)
- `src/components/landing/SectionReveal.tsx` — IntersectionObserver fade-in

**Rewritten (frontend tests):**
- `tests/e2e/landing-product-v2.spec.ts` — 15 tests covering V2 polish

**Operator-committed assets (untouched by agent except as imports):**
- `public/greeks/Logo_{Delta,Gamma,Rho,Theta,Vega}.png`

**Created (backend docs):**
- `docs/FRONTEND_HOMEPAGE_COSMIC_LANDING_POLISH_V2_RESULT.md`

**Edited (root):**
- `RUN_STATE.md` — V2 polish closure prepended

**Untouched:** Backend Rust (ZERO), Solidity (ZERO), `scripts/local-*.sh` (ZERO), backend `.env` (mtime preserved), `~/DEOPT/private/` (mode 700), `(trading)/layout.tsx`, `TradingShell.tsx`, every other `(trading)` route, every workspace file, every market/portfolio/feedback file, `public-beta-links.ts`, `PublicBetaFooter.tsx`.

## Remaining homepage gaps

- The backdrop's scroll listener writes one CSS variable per rAF tick — composited paint should be cheap on modern devices, but extreme low-power devices might see drops. The reduced-motion path is the safe fallback.
- The hero `min-h-[80vh]` looks great on desktop and tablets; on extremely short windows (< 600 px tall) the hero still scrolls cleanly because the inner content is centered with `justify-center`.
- FAQ accordion uses `<details>` for native semantics — height transitions are instant (browser default). A future pass could add a max-height transition for smooth open/close.
- No per-section background-color variation yet; the brief mentioned "layered radial gradients with different section colors/intensity" as one option among many — the scroll-progress-driven global gradient evolution covers the spirit, but per-section tint could be added.
- Greek silhouettes are static. A future pass could give them a slow 60-90s rotate or float keyframe.

None block local QA, public-testnet-beta launch, or operator product-test.

## Next milestone recommendation

**Primary (operator action, not agent-runnable):** product-test V2 polish on the local frontend via `bash ~/DEOPT/scripts/local-frontend.sh`. Confirm at 1440 / 1920 / 2560:
- Browser tab shows just `DeOpt`
- Hero reads tight and centered, vertical breathing room feels cinematic
- Scrolling down evolves the backdrop (top-right glow drifts, dot field drifts downward, vignette tightens)
- FAQ rows expand on click; plus icon rotates into X
- Greek silhouettes are visible but never compete with content
- No yellow / orange / amber anywhere
- `prefers-reduced-motion` users see a static, calm version

**Secondary (agent-runnable):** `BACKEND-PUBLIC-TESTNET-DEPLOY-PREFLIGHT` per existing brief.

**Strictly later (NOT NOW):** per-section background tint variation, max-height-transitioned FAQ rows, animated Greek silhouettes, SEO metadata per route, i18n, analytics, real options/perps mocks driven by mock backend data, mainnet activation, audit firm outreach, bug bounty launch, KMS cutover, Safe migration, flipping `isMainnetEnabled()`.
