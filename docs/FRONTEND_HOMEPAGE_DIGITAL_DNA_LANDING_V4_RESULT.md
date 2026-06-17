# FRONTEND-HOMEPAGE-DIGITAL-DNA-LANDING-V4 — Result

**Date:** 2026-06-16
**Workspace root:** `~/DEOPT/`
**Approval line consumed:** "I approve DeOpt homepage digital DNA landing V4 for this run."

## TL;DR

V4 is a direction change, not a polish:
1. **New hero slogan** — `"The execution layer for programmable derivatives."` (the V3 line is gone).
2. **New backdrop** — `<DigitalDnaBackdrop>` replaces the V3 cosmic-protocol layer. Two intertwining sinusoidal strands flow top-to-bottom through the viewport, with rungs at midpoints, glowing crossover nodes, an ambient particle field, and a scroll-driven central glow.
3. **Soft glass panels** replace the V3 `<GlassCard>` — no more visible 1px emerald borders. Content modules are blended into the DNA field via radial inset shadows.
4. **Large-screen whitespace is filled** — terminal-style hero readout strip + two vertical side-rails of protocol chips on `lg+`.
5. **Architecture diagram** keeps its 3-tier hierarchy but loses the surrounding glass panel — it sits directly over the DNA flow.
6. **New `Risk · collateral · oracle` section** brought back from V1 (folded out of the FAQ).
7. **Options + Perps split section** — side-by-side on `lg+`, each with its glass mock + chips + CTA.
8. **186 → 187 tests in 34 files**; backend Rust / Solidity / scripts: zero changes.

## Current homepage audit (Phase A)

| Element | V3 state |
|---|---|
| Hero slogan | `"Programmable derivatives. On chain."` |
| Backdrop | `CosmicBackdrop` — dot field + diagonal grid + 4 diagonal traces + 3 horizontal vol curves + vol-wave + scanlines + scroll-driven radial glows |
| Card style | `<GlassCard>` with visible `border-emerald-500/20` borders |
| Architecture | 3-tier glass cards inside a bordered `<GlassCard>` panel |
| Greeks | 13 ambient silhouettes spread across sections |
| FAQ | Derive-style accordion via `<details>` (7 questions) |
| Large-screen behavior | central `max-w-6xl` column; side margins visually empty on 2K+ monitors |

## Hero slogan update (Phase B)

`src/components/landing/CosmicLanding.tsx` → `HeroSection`:
- Headline rewritten to **`The execution layer for programmable derivatives.`** with the second clause rendered through an emerald gradient (`from-emerald-200 via-emerald-300 to-emerald-500`).
- Subhead rewritten to one short line: `"Options and perpetuals through a single intent-driven pipeline."`
- Old slogan text fully removed; `landing-hero-eyebrow` (gone in V3) stays absent.

## Digital DNA background (Phase C)

NEW `src/components/landing/DigitalDnaBackdrop.tsx`:

- **Helix geometry** built at render time. `viewBox = 200 × 1800`, 12 segments × 150 px each. `buildStrandPath(phase)` emits a single quadratic-bezier polyline that oscillates horizontally with amplitude ±80 around centre x=100. `phase=0` and `phase=1` produce two mirrored strands that intertwine.
- **Rungs** rendered as faint dashed lines (`stroke-dasharray="1.5 3"`, opacity 0.18) at segment midpoints.
- **Crossover nodes** rendered at every segment boundary as radial-gradient circles (`r=10` outer halo + `r=2.4` inner emerald core with Gaussian blur filter), giving a glowing protocol-point feel.
- **Animated dash overlays** on each strand (`deopt-dna-flow-a/b` keyframes, opposite directions, 14 s vs 18 s) — produces the "data flowing along the strand" sense.
- **Ambient particle field** at 36 px step, drifting downward at 60 s loop.
- **Scroll-driven central glow** via `--dna-progress` CSS variable updated in a rAF-throttled scroll listener. The central ellipse follows the user's vertical scroll position so the glow visually "rides" alongside the reader.
- **Helix column** spans `h-[110%]` and drifts upward by 10% over 80 s (`deopt-dna-drift` keyframe) so the helix feels organically alive without re-laying out the page.
- `prefers-reduced-motion: reduce` short-circuits the scroll listener (sets static 0.5 progress) AND every CSS animation via the media block in `globals.css`.

Old `CosmicBackdrop.tsx` deleted — single source of truth for the landing backdrop now.

## Content / background blending (Phase D)

`<SoftPanel>` replaces `<GlassCard>`:

```tsx
<div className="relative overflow-hidden rounded-2xl
                bg-gradient-to-b from-zinc-950/55 to-zinc-950/15
                p-6 backdrop-blur-md">
  <div className="pointer-events-none absolute inset-0 rounded-2xl
                  shadow-[inset_0_0_60px_-30px_rgba(16,185,129,0.35),
                          inset_0_1px_0_0_rgba(110,231,183,0.08)]" />
  <div className="relative">{children}</div>
</div>
```

- **No `border-*` class** anywhere on the landing surface (a regression-test asserts the absence of `border-emerald-500/20`).
- Outline is replaced by **inset radial shadow + a 1-px top highlight**, so the panel feels lit from within the DNA field rather than pasted on it.
- Stronger backdrop-blur (`backdrop-blur-md`) so the helix shows through the panel's transparent regions.

## Large-screen fill (Phase E)

Two new visual fillers:

1. **`HeroReadout`** — a terminal-style strip below the hero CTAs, `hidden sm:flex`, full-width up to `max-w-3xl`. 8 cells: `Δ Γ Θ Ν Ρ · Vault · Oracle · EIP-712`, all dashes (honest placeholders, no fake values). Pill-shaped, monospace, 10 px.
2. **`SideRail`** — vertical floating chip stack pinned `left-4` or `right-4` of the section, `hidden lg:flex` only, `pointer-events-none`. Each rail carries 3-4 protocol labels (`Intent / Margin / Vault / Oracle` etc.). Wired into hero + execution-path sections.

On a 2560 × 1440 monitor the central 1152 px column is now flanked by two visible chip rails + the DNA helix runs down the center; the previous wide empty side margins are gone.

## Section redesign (Phase F)

Final 7-section flow:

| # | testid | Notes |
|---|---|---|
| 1 | `landing-hero` | New slogan + 3 CTAs + readout + side rails |
| 2 | `landing-products-section` | Splits into `landing-options-section` + `landing-perps-section` side-by-side |
| 3 | `landing-protocol-flow` | Execution path with 3 soft-panel steps + side rails |
| 4 | `landing-architecture-section` | 3-tier borderless arch cards |
| 5 | `landing-risk-section` | NEW — 3 risk modules (Margin / Vault / Oracle) |
| 6 | `landing-faq-section` | Unchanged Derive-style accordion |
| 7 | `landing-final-cta` | Launch / Markets / Feedback |

Section spacing kept at `py-32 sm:py-40`. Each section wrapped in `<SectionReveal>` (IntersectionObserver fade-in retained from V2).

## Architecture refinement (Phase G)

The V3 outer `<GlassCard>` is gone — the arch now lives in a borderless `max-w-5xl` container that sits directly over the DNA helix flow. Inner nodes use the same soft-glass `inset shadow` styling so they read as floating protocol points rather than caged boxes.

Labels preserved: Trader / Signed intent / Execution layer / Margin engine / Collateral vault / Oracle router / Settlement / Indexer (full readable words, 14 px semibold). Inter-tier downward chevrons retained.

## FAQ integration (Phase H)

`FaqSection.tsx` left **untouched** — it already uses thin `border-emerald-500/15` separators only between rows (no card frame). The soft-glass language of the rest of the V4 landing means the FAQ already blends into the DNA field without changes.

## Copy minimization and balance (Phase I)

- Hero subhead reduced to 1 line.
- Options + Perps subheads kept to 1 line each.
- Execution stages already 1-line each from V3.
- Risk module bodies = 1 line each.
- Body remains free of testnet / public-beta / no-real-funds copy.

## Tests added / updated (Phase J)

`tests/e2e/landing-product-v2.spec.ts` rewritten — **19 tests**:

| Test | Coverage |
|---|---|
| browser title `^DeOpt$` | unchanged |
| renders digital-DNA backdrop + hero | `digital-dna-backdrop` / `dna-helix-column` / `dna-strand-a` / `dna-strand-b` / `dna-particle-field` all attached |
| **V3 cosmic backdrop testid GONE** | `cosmic-backdrop` has count 0 |
| **old V3 slogan gone** | regex `Programmable derivatives\. On chain\.` not present in main |
| **new V4 hero headline** | contains "execution layer" + "programmable derivatives" |
| hero CTAs route | Launch → `/trade`, Markets → `/markets`, Docs → `/docs` |
| Options + Perps mentioned and linked | both sections attached, CTAs → /trade + /perps |
| scroll story renders all 9 testid sections | hero / products / options / perps / execution / architecture / risk / faq / final |
| **hero readout strip + side rails fill large-screen whitespace** | at 1920×1080: `landing-hero-readout` visible with Δ cell, both side rails attached |
| ambient Greek silhouettes ≥ 8 | unchanged (V3 had 13, V4 retains spread) |
| architecture diagram 3 tiers + readable labels | unchanged (full-word labels per node) |
| FAQ renders + rows expand/collapse | unchanged |
| **content panels DO NOT use `border-emerald-500/20`** | regex asserts absence — guard against accidental return of V3 bordered cards |
| body does NOT repeat testnet/public-beta/no-real-funds | unchanged |
| no positive-claim language in `<main>` | unchanged |
| no admin/bearer/RPC/DATABASE_URL/mainnet leaks | unchanged |
| no yellow/orange/amber classes | unchanged |
| final CTA routes | unchanged |
| no broken image src | unchanged |

`tests/e2e/landing.spec.ts` left unchanged — asserts only banner + footer.

Catalog: `npx playwright test --list` → **186 → 187 tests in 34 files** (+1).

## Build validations (Phase K)

| Command | Result |
|---|---|
| `npm run typecheck` | clean |
| `npm run lint` | clean |
| `NEXT_PUBLIC_TRADING_API_BASE_URL=http://localhost:8080 npm run build` | green — same 19 user-facing routes + 4 SSG doc slugs |
| `npx playwright test --list` | 187 tests in 34 files |
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

- NEW `docs/FRONTEND_HOMEPAGE_DIGITAL_DNA_LANDING_V4_RESULT.md` (this file)
- `docs/public-beta/PUBLIC_TESTNET_BETA_LAUNCH_CHECKLIST.md` left **unchanged** — no checklist criterion references hero slogan or backdrop style.

## RUN_STATE update

2026-06-16 closure for FRONTEND-HOMEPAGE-DIGITAL-DNA-LANDING-V4 prepended above the V3 polish entry.

## Files changed

**Created (frontend src):**
- `src/components/landing/DigitalDnaBackdrop.tsx` — helix backdrop + scroll progress + ambient particles

**Edited (frontend src):**
- `src/components/landing/CosmicLanding.tsx` — new hero copy, `<SoftPanel>` replaces `<GlassCard>`, NEW Options+Perps split section, NEW Risk section, NEW HeroReadout + SideRail, borderless architecture container, switched import from CosmicBackdrop → DigitalDnaBackdrop
- `src/app/globals.css` — V3 `deopt-cosmic-*` keyframes removed; NEW `deopt-dna-*` keyframes (helix drift, dual dash flows, particle drift); reduced-motion media block updated

**Deleted (frontend src):**
- `src/components/landing/CosmicBackdrop.tsx` — replaced by DigitalDnaBackdrop

**Rewritten (frontend tests):**
- `tests/e2e/landing-product-v2.spec.ts` — 19 tests covering V4 (DNA backdrop, slogan swap, soft panels, hero readout, side rails)

**Operator-committed assets:** the 5 PNGs in `public/greeks/` — used as imports only, never modified.

**Created (backend docs):** `docs/FRONTEND_HOMEPAGE_DIGITAL_DNA_LANDING_V4_RESULT.md`

**Edited (root):** `RUN_STATE.md`

**Untouched:** Backend Rust (ZERO), Solidity (ZERO), `scripts/local-*.sh` (ZERO), backend `.env` (mtime preserved), `~/DEOPT/private/` (mode 700), `(trading)/layout.tsx`, `TradingShell.tsx`, every other `(trading)` route, every workspace file, every market/portfolio/feedback file, `public-beta-links.ts`, `PublicBetaFooter.tsx`, `FaqSection.tsx`, `SectionReveal.tsx`, `src/app/layout.tsx`.

## Remaining homepage gaps

- The architecture tiers are arranged in a 3-row grid; per-node connections still use a single downward chevron per tier. A future V5 could route per-node SVG arrows that follow the DNA strand visually (e.g. Trader → Intent → Executor along the left strand, then Margin/Vault/Oracle along the right strand, joining at Settle).
- Greek silhouettes are still static — a slow `@keyframes` float could add organic motion later.
- The hero readout is a static dashes strip; later milestones could plug live local-only protocol data (mark, IV, OI) when those endpoints stabilise.
- FAQ rows transition instantly via native `<details>`; max-height transition could smooth it.
- No per-section background tint variation — the helix + scroll-driven glow do the heavy lifting already.

None block local QA, public-testnet-beta launch, or operator product-test.

## Next milestone recommendation

**Primary (operator action, not agent-runnable):** product-test V4 on the local frontend via `bash ~/DEOPT/scripts/local-frontend.sh`. Confirm at 1440 / 1920 / 2560:
- New hero line "The execution layer for programmable derivatives." reads strong
- Two-strand DNA helix flows visibly down the page center; nodes glow at crossovers; faint rungs visible
- Scrolling moves the central glow vertically alongside the reader
- Content panels look like floating glass — no hard 1 px emerald rectangles
- 2K monitor: side-rail chip stacks visible on either side of the central column
- Options + Perps side-by-side at lg+; stack on mobile
- Architecture nodes readable; labels are real words
- 13 ambient Greek sigils still float behind sections
- `prefers-reduced-motion` users see a calm static DNA composition

**Secondary (agent-runnable):** `BACKEND-PUBLIC-TESTNET-DEPLOY-PREFLIGHT` per existing brief.

**Strictly later (NOT NOW):** per-node arch SVG arrows that ride the DNA strand, animated Greek silhouettes, live protocol metrics in hero readout, max-height-transitioned FAQ rows, SEO metadata, i18n, analytics, real options/perps mocks, mainnet activation, audit firm outreach, bug bounty launch, KMS cutover, Safe migration, flipping `isMainnetEnabled()`.
