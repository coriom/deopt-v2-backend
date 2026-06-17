# FRONTEND-HOMEPAGE-PARTICLE-FIELD-V5 — Result

**Date:** 2026-06-16
**Workspace root:** `~/DEOPT/`
**Approval line consumed:** "I approve DeOpt homepage particle field V5 for this run."

## TL;DR

The V4 DNA / helix backdrop is removed. The landing now ships an
interactive **canvas particle field** that reacts to the cursor
(gentle attraction), to clicks (decaying repulsion), and morphs
across four scroll-driven modes (`calm → wave → nodes → sparse`).
The FAQ title is **`Some Questions`**. The rest of the V4 landing
(hero copy, soft glass panels, Options + Perps split, soft
architecture, risk callouts, FAQ accordion) stays put.

186 → 186 tests in 34 files (re-write of the landing spec replaces
the V4 DNA assertions with V5 particle-field assertions; same gross
count).

## Current homepage audit (Phase A)

| Element | V4 state |
|---|---|
| Backdrop | `<DigitalDnaBackdrop>` — fixed-position SVG with two intertwining sinusoidal strands + rungs + glowing crossover nodes + ambient particles + scroll-driven glow |
| CSS | `globals.css` carried `deopt-dna-*` keyframes (helix drift, dual dash flows, particle drift) + reduced-motion entries |
| Hero slogan | "The execution layer for programmable derivatives." |
| FAQ title | "Frequently asked." |
| Sections | hero / products (options + perps) / execution / architecture / risk / faq / final-cta |
| Greeks | 13 ambient silhouettes spread across sections |
| Tests | asserted `digital-dna-backdrop` / `dna-helix-column` / `dna-strand-a/b` / `dna-particle-field` |

## DNA visual removal (Phase B)

- `src/components/landing/DigitalDnaBackdrop.tsx` deleted.
- `src/app/globals.css`: every `.deopt-dna-*` class + `@keyframes` removed; reduced-motion media block updated.
- `CosmicLanding.tsx`: import + render swapped to `<ParticleField>`.
- Regression test guards: `landing-product-v2.spec.ts` asserts `digital-dna-backdrop` and four other DNA testids have count 0.

## Particle field implementation (Phase C)

NEW `src/components/landing/ParticleField.tsx`:

- Pure HTML `<canvas>` rendered as a `fixed inset-0 -z-10 pointer-events-none` layer beneath the landing content.
- `useEffect` sets up:
  - `dpr`-scaled canvas resize on mount + window resize
  - 80-180 particles (target = `floor((w × h) / 11000)`, clamped); each with `(x, y, vx, vy, baseRadius)`
  - `pointermove` / `pointerleave` / `pointerdown` / `scroll` window listeners
  - `requestAnimationFrame` loop, or single static frame under reduced-motion
- Tear-down cancels rAF + removes every listener.
- Below the canvas: a subtle radial-gradient tint (`from-emerald-500/0.09` top, `from-emerald-500/0.06` bottom, linear black-to-near-black base) so the surface is never pure black; above: an edge vignette to keep readable content focused.

Performance posture:
- Max 180 particles; connection loop is O(n²) per frame but only runs when `mode ∈ [0.3, 0.85]` and only for connections within `connectionDistance` (60-100 px). 180² = 32 400 distance checks per active frame.
- Damping `vx * 0.96 / 0.94` keeps motion calm and lets the system settle quickly after a click impulse.
- Canvas is DPR-scaled but capped at `min(devicePixelRatio, 2)` so 3× and 4× retina screens don't pay for excessive resolution.

## Cursor + click interaction

- **Cursor attraction.** For each particle within ~134 px of the cursor, velocity is biased toward the cursor with falloff `(1 - d / 134) × 0.05`. Calm enough to read as "alive" not "frantic".
- **Click repulsion.** On `pointerdown`, particles within 200 px get an outward velocity impulse `(1 - d / 200) × clickStrength × 1.4`. `clickStrength` decays linearly from 1 → 0 over 600 ms so the shockwave is short-lived.
- **Pointer events.** Canvas is `pointer-events-none` so links + CTAs still click through. The window-level `pointerdown` listener fires regardless of which DOM element took the event, so a click on a CTA still triggers the repulsion impulse before the navigation happens. Spec test `particle field DOES NOT block link clicks` proves the Launch button still routes to `/trade`.

## Scroll-based particle morphing (Phase D)

Scroll progress (0..1) drives a `mode` bucket exposed on the root as `data-particle-mode`:

| Range | Mode | Radius scale | Connections | Curl | Damping |
|---|---|---|---|---|---|
| `[0.00, 0.25)` | `calm` | 1.0 | off | off | 0.965 |
| `[0.25, 0.55)` | `wave` | 1.0 → 1.3 | off | mild (right-handed) | 0.965 |
| `[0.55, 0.85)` | `nodes` | 1.3 → 1.55 | on (60-100 px) | off | 0.965 |
| `[0.85, 1.00]` | `sparse` | 1.55+ | off | off | 0.94 (settles quickly) |

The attribute updates only when the bucket actually changes (no per-frame DOM writes). Connection-line opacity scales with mode (`0.05 + mode × 0.06`), so the connections appear and brighten over the architecture mid-page region — then fade as the user reaches the FAQ.

## Greek background preservation (Phase E)

All 13 ambient `<GreekSilhouette>` instances from V4 are unchanged. They float behind sections at `opacity-[0.05] sm:opacity-[0.07]`, `pointer-events-none`, sourced from local `/greeks/Logo_*.png` only. No remote URLs, no clickable Greeks.

## Background/content blending (Phase F)

`<SoftPanel>` from V4 unchanged — content modules retain the same borderless glass styling (`rounded-2xl bg-gradient-to-b from-zinc-950/55 to-zinc-950/15 backdrop-blur-md` + inset radial shadow). The particle canvas sits at `-z-10`, the soft panels at `z-10`, so the panels are clearly above the field without re-introducing hard 1 px emerald borders.

## FAQ title / style (Phase G)

`src/components/landing/FaqSection.tsx`:
```diff
- Frequently asked.
+ Some Questions
```
Everything else in the FAQ stays: full-width rows, thin emerald separators between rows, large `text-lg sm:text-xl` question text, plus icon rotating to X via `group-open:rotate-45`, native `<details>` accordion semantics, 7 DeOpt-specific items.

Spec assertions:
- `await expect(faq).toContainText(/^Some Questions$/m)`
- `await expect(faq).not.toContainText(/Frequently asked/i)`

## Copy balance (Phase H)

Body copy unchanged from V4 — every section is already 1-line and the body is free of testnet / public-beta / no-real-funds text. The global `TestnetUnauditedBanner` + `PublicBetaFooter` still carry that copy as required.

## Responsive / performance (Phase I)

- Particle count auto-scales with viewport area (`floor(w × h / 11000)`, clamped 80-180). On a 360 × 640 mobile screen this becomes ~80 particles; on 2560 × 1440 it tops out at 180.
- Canvas is `fixed inset-0`; no horizontal overflow.
- `prefers-reduced-motion: reduce` → rAF loop is never started; a single static frame of particles is drawn instead.
- Browser's `rAF` automatically pauses when the tab is hidden, so background tabs cost zero CPU.

## Tests added / updated (Phase J)

`tests/e2e/landing-product-v2.spec.ts` rewritten — **18 tests**:

| Test | Coverage |
|---|---|
| browser title `^DeOpt$` | unchanged |
| **V4 DNA backdrop is GONE** | `digital-dna-backdrop` / `dna-helix-column` / `dna-strand-a` / `dna-strand-b` / `dna-particle-field` all have count 0 |
| **particle-field backdrop renders with canvas + reactive data attributes** | `particle-field` attached; `data-particle-field="true"`; `data-scroll-reactive-background="true"`; `data-particle-mode` matches `/calm\|wave\|nodes\|sparse/`; `particle-field-canvas` attached |
| **particle mode morphs across scroll progress** | at top → `calm`; after `scrollTo(70% of doc)` + 120 ms → mode ∈ `{wave, nodes, sparse}` and ≠ `calm` |
| **particle field DOES NOT block link clicks** | clicking the launch CTA navigates to `/trade` |
| hero retains V4 slogan | headline contains "execution layer" + "programmable derivatives" |
| Options + Perps still linked | CTA hrefs `/trade`, `/perps` |
| hero CTAs route | Launch → `/trade`, Markets → `/markets`, Docs → `/docs` |
| **FAQ title is exactly `Some Questions`** | `landing-faq-section` contains `^Some Questions$`; does NOT contain "Frequently asked" |
| FAQ items + expand/collapse | `[open]` attribute toggles correctly |
| scroll story renders all 9 testid sections | hero / products / options / perps / execution / architecture / risk / faq / final-cta |
| ambient Greek silhouettes still render local | ≥ 6 silhouettes, every src is local + resolves to `/greeks/` |
| architecture 3 tiers + readable labels | unchanged (Signed intent / Execution layer / Margin engine / …) |
| body does NOT repeat testnet/public-beta/no-real-funds | unchanged |
| no positive-claim language in `<main>` | unchanged |
| no admin/bearer/RPC/DATABASE_URL/mainnet leaks | unchanged |
| no yellow/orange/amber classes | unchanged |
| no broken image src | unchanged |

`tests/e2e/landing.spec.ts` left unchanged — asserts only banner + footer.

Catalog: `npx playwright test --list` → **186 tests in 34 files** (same gross count; replaced V4 DNA-only assertions with V5 particle-field assertions).

## Build validations (Phase K)

| Command | Result |
|---|---|
| `npm run typecheck` | clean |
| `npm run lint` | clean |
| `NEXT_PUBLIC_TRADING_API_BASE_URL=http://localhost:8080 npm run build` | green — same 19 user-facing routes + 4 SSG doc slugs |
| `npx playwright test --list` | 186 tests in 34 files |
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
| Source changes limited to frontend homepage/components + docs/RUN_STATE | YES |
| Remote image URLs introduced | NONE |

## Docs created / updated

- NEW `docs/FRONTEND_HOMEPAGE_PARTICLE_FIELD_V5_RESULT.md` (this file)
- `docs/public-beta/PUBLIC_TESTNET_BETA_LAUNCH_CHECKLIST.md` left **unchanged** — no checklist criterion references the backdrop kind or the FAQ title.

## RUN_STATE update

2026-06-16 closure for FRONTEND-HOMEPAGE-PARTICLE-FIELD-V5 prepended above the V4 DNA-landing entry.

## Files changed

**Created (frontend src):**
- `src/components/landing/ParticleField.tsx`

**Edited (frontend src):**
- `src/components/landing/CosmicLanding.tsx` — header comment + import + render swapped DigitalDnaBackdrop → ParticleField
- `src/components/landing/FaqSection.tsx` — title text "Frequently asked." → "Some Questions"
- `src/app/globals.css` — all `deopt-dna-*` keyframes removed; reduced-motion media block trimmed to keep only `deopt-section-reveal`

**Deleted (frontend src):**
- `src/components/landing/DigitalDnaBackdrop.tsx` (V4 file; never committed to main by operator — removed within this session)
- `src/components/landing/CosmicBackdrop.tsx` was already deleted in V4 (still pending in operator's working tree)

**Rewritten (frontend tests):**
- `tests/e2e/landing-product-v2.spec.ts` — 18 tests covering V5 (particle field, DNA gone, "Some Questions" title)

**Operator-committed assets:** the 5 PNGs in `public/greeks/` — used as imports only, never modified.

**Created (backend docs):** `docs/FRONTEND_HOMEPAGE_PARTICLE_FIELD_V5_RESULT.md`

**Edited (root):** `RUN_STATE.md`

**Untouched:** Backend Rust (ZERO), Solidity (ZERO), `scripts/local-*.sh` (ZERO), backend `.env` (mtime preserved), `~/DEOPT/private/` (mode 700), `(trading)/layout.tsx`, `TradingShell.tsx`, every other `(trading)` route, every workspace file, every market/portfolio/feedback file, `public-beta-links.ts`, `PublicBetaFooter.tsx`, `SectionReveal.tsx`, `src/app/layout.tsx`.

## Remaining homepage gaps

- Particle field uses a single-color (`rgba(110, 231, 183, …)`) emerald palette. Future passes could introduce a tertiary deep-blue-green tint for the `nodes` mode to differentiate visually.
- Click impulse decay is fixed at 600 ms; could become a CSS variable for future tuning.
- Reduced-motion users see a single static frame — they don't get the cursor / click reactions. This is intentional and accessibility-correct.
- The connection-line `O(n²)` cost grows quadratically with particle count; if the operator ever wants 500+ particles, a spatial-hash grid would be the right next step.
- Greek silhouettes remain static (no rotation/float).

None block local QA, public-testnet-beta launch, or operator product-test.

## Next milestone recommendation

**Primary (operator action, not agent-runnable):** product-test V5 via `bash ~/DEOPT/scripts/local-frontend.sh`. Confirm at 1440 / 1920 / 2560:
- DNA helix is fully gone; the page now reads as a subtle particle field
- Moving the cursor over the page gently nudges nearby particles toward it
- Clicking anywhere triggers a quick outward ripple from the click point
- Scrolling through the page: hero is calm dots → mid-page connection-line clusters appear → bottom is sparse
- FAQ heading reads exactly `Some Questions`
- Greek sigils float behind sections without dominating
- Links still navigate normally (particle layer never blocks input)
- `prefers-reduced-motion` shows a calm static field

**Secondary (agent-runnable):** `BACKEND-PUBLIC-TESTNET-DEPLOY-PREFLIGHT` per existing brief.

**Strictly later (NOT NOW):** per-section particle palette tints, animated Greek silhouettes, hero live readout, max-height-transitioned FAQ rows, SEO metadata, i18n, analytics, real options/perps mocks, mainnet activation, audit firm outreach, bug bounty launch, KMS cutover, Safe migration, flipping `isMainnetEnabled()`.
