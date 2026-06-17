# FRONTEND-HOMEPAGE-PARTICLE-FIELD-V6 — Result

**Date:** 2026-06-16
**Workspace root:** `~/DEOPT/`
**Approval line consumed:** "I approve DeOpt homepage particle field and product messaging V6 for this run."

## TL;DR

- **Hero now says what DeOpt is**: "Decentralized options and perps with low-fee execution, APIs, and customizable trading workspaces." Subhead reinforces options chains, perps workspaces, API access, modular execution flows.
- **Particle field is denser and slightly bigger**: target density bumped ~1.7× (`floor((w × h) / 6500)`, clamped 120-320; V5 was 80-180). Radius range raised to 0.9-2.5 (V5: 0.6-1.7).
- **Soft parallax drift**: each particle's internal `y` drifts down by `(scrollY − lastScrollY) × 0.12` every frame so the field reads as suspended in depth, not pinned wallpaper.
- **Ambient Greek presence enriched**: +4 silhouettes spread across hero / products / FAQ, totalling **≥ 16** across the page.
- FAQ title remains "Some Questions".
- 186 → 188 tests in 34 files. Backend Rust / Solidity / scripts: zero changes.

## Homepage audit (Phase A)

| Element | V5 state |
|---|---|
| Hero headline | "The execution layer for programmable derivatives." |
| Hero subhead | "Options and perpetuals through a single intent-driven pipeline." |
| ParticleField particle count | 80-180 (`floor((w × h) / 11000)` clamped) |
| ParticleField particle radius | 0.6 + random × 1.1 (range 0.6-1.7) |
| Scroll-driven parallax | none — canvas fixed in place |
| Ambient Greek silhouettes | 13 |
| FAQ title | "Some Questions" |

## Product messaging update (Phase B)

`src/components/landing/CosmicLanding.tsx` → `HeroSection`:

```diff
- The execution layer for
- <span ...>programmable derivatives.</span>
+ Decentralized options and perps with
+ <span ...>low-fee execution, APIs, and customizable trading workspaces.</span>

- Options and perpetuals through a single intent-driven pipeline.
+ Trade and build on a derivatives interface designed for options chains,
+ perps workspaces, API access, and modular execution flows.
```

Headline now uses `text-4xl sm:text-5xl lg:text-6xl` (one step down from V5's `text-5xl/6xl/7xl`) and `max-w-5xl` so the longer product line fits cleanly. Subhead uses `max-w-2xl` for the same reason. No false safety claims, no "lowest fees" / "cheapest" — "low-fee" is used as positioning only. English-only.

## Particle density and size (Phase C)

`src/components/landing/ParticleField.tsx`:

| Knob | V5 | V6 | Effect |
|---|---|---|---|
| Target density formula | `floor((w × h) / 11000)` | `floor((w × h) / 6500)` | ~1.7× density |
| Min particles | 80 | 120 | mobile keeps a denser baseline without flooding |
| Max particles | 180 | 320 | 2K monitors get noticeably more depth |
| Base radius | 0.6 + r × 1.1 | 0.9 + r × 1.6 | slightly larger, still subtle |
| Mode radius scale | 1.0 → 1.55 | same | unchanged |
| Connection cap | 60-100 px in `nodes` mode | same | unchanged |
| Damping | 0.94/0.965 | same | unchanged |

`data-particle-density` attribute is updated on every `resize()` so the runtime value is visible in DevTools + the spec asserts it is ≥ 120.

## Scroll parallax / downward drift (Phase D)

```ts
const PARALLAX_FACTOR = 0.12;
let lastScrollY = window.scrollY;

function frame(now) {
  const sy = window.scrollY;
  const parallaxDrift = (sy - lastScrollY) * PARALLAX_FACTOR;
  lastScrollY = sy;
  for (const p of particles) {
    p.x += p.vx;
    p.y += p.vy + parallaxDrift;   // ← parallax baked into the per-frame y update
    // …cursor/click forces, damping, wrap-around as before
    if (p.y < -10) p.y = height + 10;
    else if (p.y > height + 10) p.y = -10;
  }
}
```

Why this implementation:
- Apply the parallax to **internal `p.y`** every rAF tick, not as a CSS transform. This keeps the canvas geometry pinned (no CSS reflow, no wrapper translate, no visible canvas-edge clipping on long pages).
- The existing **wrap-around** keeps the field visually continuous: a particle that drifts past `height + 10` reappears at `-10`. From the user's perspective, the field flows downward without ever "running out" at the bottom of a long page.
- Cursor + click forces still operate in viewport coordinates because `cursor.x/y` come from `clientX/Y` and particles are drawn at their internal coordinates which equal screen coordinates (canvas is viewport-sized and pinned).
- Scrolling UP gives a negative `parallaxDrift` → particles drift up by the same factor, which feels natural and symmetric.
- The constant `PARALLAX_FACTOR = 0.12` sits inside the brief's recommended 0.08-0.18 range.

`data-scroll-parallax="true"` is exposed on the particle field root so DevTools + tests can detect the feature.

## Particle morphing refinement (Phase E)

V5's `pickMode(progress)` retained unchanged:

| Range | Mode | Behavior |
|---|---|---|
| `[0.00, 0.25)` | `calm` | small + drifting, no connections |
| `[0.25, 0.55)` | `wave` | mild curl, particles grow slightly, no connections |
| `[0.55, 0.85)` | `nodes` | connection lines appear (60-100 px), particles brighter |
| `[0.85, 1.00]` | `sparse` | connections off, damping bumped (`0.94`) so motion settles fast |

The denser particle population means the `nodes` mode now produces a noticeably richer constellation field. Connection-line `O(n²)` cost grows quadratically — at the new 320 cap that's 51,200 distance checks per active frame, still comfortably within 60 fps on modern devices.

## Greek background enrichment (Phase F)

+4 ambient silhouettes:

| Section | Glyph | Position | Size |
|---|---|---|---|
| Hero | **Gamma** | centered (50%, 50%) | 160-224 px |
| Products | **Rho** | `right-1/3 top-20` | 144-192 px |
| Products | **Vega** | `left-1/4 bottom-20` | 144-192 px |
| FAQ | **Theta** | `-left-20 top-16` | 192-288 px |
| FAQ | **Rho** | `-right-16 bottom-20` | 160-224 px |

Wait — the FAQ adds **two** silhouettes for the first time (FAQ had none in V5). Total ambient silhouettes:
- Hero: 3 (Rho + Vega + Gamma)
- Products: 4 (Delta + Theta + Rho + Vega)
- Execution: 2
- Architecture: 2
- Risk: 2
- FAQ: 2 (NEW)
- Final CTA: 3
- **Total: 18 silhouettes** (V5: 13). Spec asserts ≥ 16.

All `pointer-events-none`, `opacity-[0.05] sm:opacity-[0.07]`, local `/greeks/Logo_*.png` only.

## Large-screen visual balance (Phase G)

The denser particles + the +5 new Greek silhouettes already fill the space — no extra labels added (the V4 `<HeroReadout>` and `<SideRail>` stay). Section spacing kept `py-32 sm:py-40` so the depth feels cinematic without crowding text.

## Tests added / updated (Phase H)

`tests/e2e/landing-product-v2.spec.ts` rewritten — **20 tests**:

| Test | Coverage |
|---|---|
| browser title `^DeOpt$` | unchanged |
| **old `Programmable derivatives` slogan fully gone** | regex sweep of main |
| **hero mentions options + perps + low-fee + APIs + customizable workspaces** | hero text contains decentralized / options / perps / `low[- ]fee|fee[- ]efficient` / `apis?` / customizable / `workspaces?|interface` |
| **hero subhead reinforces product surface** | subhead contains options / perps / `apis?` / `workspaces?` |
| hero CTAs route | Launch → `/trade`, Markets → `/markets`, Docs → `/docs` |
| **V4 DNA backdrop testids still GONE** | guards against regression |
| **particle field exposes scroll-parallax + density attrs** | `data-scroll-parallax="true"` + `data-particle-density` (number) ≥ 120 |
| particle mode morphs across scroll | top=calm; 70% scroll → not calm |
| particle field DOES NOT block link clicks | launch CTA navigates to /trade |
| **FAQ title is exactly `Some Questions`** | unchanged |
| FAQ items + expand/collapse | unchanged |
| **ambient Greek background ≥ 16 silhouettes** | local-only srcs |
| hero does NOT render a foreground Greek-tile row | regression guard |
| scroll story renders all 9 sections | unchanged |
| architecture diagram retains 3 tiers + readable labels | unchanged |
| body free of testnet/public-beta/no-real-funds | unchanged |
| **no positive-claim language anywhere** | + `\blowest fees\b` and `\bcheapest\b` guards added |
| no admin/bearer/RPC/DATABASE_URL/mainnet leaks | unchanged |
| no yellow/orange/amber classes | unchanged |
| no broken image src | unchanged |

Catalog: `npx playwright test --list` → **186 → 188 tests in 34 files** (+2).

## Build validations (Phase I)

| Command | Result |
|---|---|
| `npm run typecheck` | clean |
| `npm run lint` | clean |
| `NEXT_PUBLIC_TRADING_API_BASE_URL=http://localhost:8080 npm run build` | green — same 19 user-facing routes + 4 SSG doc slugs |
| `npx playwright test --list` | 188 tests in 34 files |
| `scripts/local-backend.sh` → `local-seed.sh` → `local-smoke.sh` | startup green; seed 12 PASS, 4 products visible; smoke **9 PASS / 0 FAIL** |
| Full e2e run | NOT FEASIBLE on this WSL host (`libnspr4.so` missing) — unchanged |

Backend stopped post-QA; port 8080 free.

## Validations

| Check | Result |
|---|---|
| `git diff --check` (frontend) | clean |
| Sensitive-string scan on edited FE files | zero hits |
| Positive-claim scan on edited FE files (incl. `lowest fees`, `cheapest`) | zero hits |
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

- NEW `docs/FRONTEND_HOMEPAGE_PARTICLE_FIELD_V6_RESULT.md` (this file)
- `docs/public-beta/PUBLIC_TESTNET_BETA_LAUNCH_CHECKLIST.md` left **unchanged** — no checklist criterion references hero copy or particle density.

## RUN_STATE update

2026-06-16 closure for FRONTEND-HOMEPAGE-PARTICLE-FIELD-V6 prepended above the V5 particle-field entry.

## Files changed

**Edited (frontend src):**
- `src/components/landing/CosmicLanding.tsx` — new hero headline + subhead; +3 ambient Greek silhouettes in hero / products
- `src/components/landing/ParticleField.tsx` — density formula, radius range, scroll parallax drift, new data attributes (`data-scroll-parallax`, `data-particle-density`)
- `src/components/landing/FaqSection.tsx` — +2 ambient Greek silhouettes (Theta + Rho); `next/image` import added

**Edited (frontend tests):**
- `tests/e2e/landing-product-v2.spec.ts` — 20 tests covering V6 (new product copy, parallax + density attrs, ≥ 16 Greek silhouettes, `lowest fees`/`cheapest` guards)

**Operator-committed assets:** the 5 PNGs in `public/greeks/` — used as imports only, never modified.

**Created (backend docs):** `docs/FRONTEND_HOMEPAGE_PARTICLE_FIELD_V6_RESULT.md`

**Edited (root):** `RUN_STATE.md`

**Untouched:** Backend Rust (ZERO), Solidity (ZERO), `scripts/local-*.sh` (ZERO), backend `.env` (mtime preserved), `~/DEOPT/private/` (mode 700), `(trading)/layout.tsx`, `TradingShell.tsx`, every other `(trading)` route, every workspace file, every market/portfolio/feedback file, `public-beta-links.ts`, `PublicBetaFooter.tsx`, `SectionReveal.tsx`, `src/app/layout.tsx`, `src/app/globals.css`.

## Remaining homepage gaps

- Parallax factor is hard-coded at `0.12`. Could become a CSS variable for per-page tuning.
- Particle palette is single-color. Per-section tint could land later for visual differentiation.
- Connection loop stays `O(n²)`. With 320 particles max, 51 200 checks per active frame — fine on modern CPUs but a spatial-hash grid would scale further.
- Greek silhouettes are still static (no rotate/float keyframe).
- Hero `<HeroReadout>` is still dashes only — live local-only protocol stats would be a nice-to-have.

None block local QA, public-testnet-beta launch, or operator product-test.

## Next milestone recommendation

**Primary (operator action, not agent-runnable):** product-test V6 via `bash ~/DEOPT/scripts/local-frontend.sh`. Confirm at 1440 / 1920 / 2560:
- Hero clearly mentions options + perps + low-fee execution + APIs + customizable workspaces
- Particle field is noticeably denser and the particles read slightly larger
- Scrolling slowly downward drifts the particle field down at ~12 % of scroll speed — particles never fully "stick" to viewport
- Cursor still gently nudges nearby particles; click still triggers a quick outward ripple
- Mid-page connection-line constellations appear; bottom section is sparse again
- Ambient Greek sigils are denser without ever competing with text
- FAQ heading still reads `Some Questions`
- `prefers-reduced-motion` users see a calm static field

**Secondary (agent-runnable):** `BACKEND-PUBLIC-TESTNET-DEPLOY-PREFLIGHT` per existing brief.

**Strictly later (NOT NOW):** per-section particle palette tints, CSS-variable parallax factor, animated Greek silhouettes, live hero readout values, max-height-transitioned FAQ rows, SEO metadata, i18n, analytics, real options/perps mocks, mainnet activation, audit firm outreach, bug bounty launch, KMS cutover, Safe migration, flipping `isMainnetEnabled()`.
