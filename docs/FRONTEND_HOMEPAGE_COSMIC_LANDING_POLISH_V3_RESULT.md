# FRONTEND-HOMEPAGE-COSMIC-LANDING-POLISH-V3 — Result

**Date:** 2026-06-16
**Workspace root:** `~/DEOPT/`
**Approval line consumed:** "I approve DeOpt homepage cosmic landing polish V3 for this run."

## TL;DR

V3 strips the hero of foreground noise, roughly doubles the ambient
Greek-sigil density, layers more technical motion into the backdrop
(diagonal protocol traces + faint diagonal grid + soft vol wave +
second diagonal-drifting dot field), and rebuilds the architecture
diagram as a 3-tier glass-card flow with readable labels.

184 → 186 tests in 34 files. Backend Rust / Solidity / scripts: zero
changes.

## Homepage visual inventory (Phase A)

V2 state:
- Hero: `<Chip>Programmable derivatives</Chip>` + headline + subhead + 3 CTAs + `<div landing-hero-greeks>` with 5 visible Greek tiles.
- 4 ambient Greek silhouettes (Delta / Theta / Gamma / Vega).
- Architecture: single SVG with 7 small circle nodes + `fontSize="10"` labels — hard to read at desktop.
- Backdrop: dot field + 3 horizontal SVG curves + radial gradients + scanlines + scroll-driven `--cosmic-progress`.

## Hero cleanup (Phase B)

`src/components/landing/CosmicLanding.tsx` → `HeroSection`:
- Removed the `<Chip testid="landing-hero-eyebrow">Programmable derivatives</Chip>` badge.
- Removed the `<div data-testid="landing-hero-greeks">` constellation row (the 5 visible Greek tiles below the CTAs are gone).
- Kept the headline `"Programmable derivatives. On chain."`, the one-line subhead, and the three CTAs (Launch / Markets / Docs).
- Added two large soft ambient silhouettes (Rho top-right, Vega bottom-left) inside the hero so the eye still senses a protocol field — but as background sigils, not foreground navigation icons.

## Ambient Greek background upgrade (Phase C)

The 5 hero-tile constellation became:

| Section | Glyph | Position | Size |
|---|---|---|---|
| Hero | **Rho** | `-right-24 top-12` | 288-384 px |
| Hero | **Vega** | `-left-20 bottom-10` | 256-320 px |
| Options | **Delta** | `-left-20 top-10` | 288-384 px |
| Options | **Theta** | `right-10 bottom-10` | 160-224 px |
| Perps | **Theta** | `-right-24 top-10` | 288-384 px |
| Perps | **Gamma** | `left-10 bottom-10` | 160-224 px |
| Execution | **Rho** | `right-0 top-12` | 224-288 px |
| Execution | **Delta** | `-left-12 bottom-20` | 176-240 px |
| Architecture | **Gamma** | `right-0 top-20` | 320-448 px |
| Architecture | **Vega** | `-left-12 bottom-32` | 192-256 px |
| Final CTA | **Vega** | centered top | 256-320 px |
| Final CTA | **Rho** | `-left-12 bottom-12` | 176-224 px |
| Final CTA | **Delta** | `-right-12 bottom-12` | 176-224 px |

**13 silhouettes** spread across the page (V2 had 4 → +9). All at `opacity-[0.05] sm:opacity-[0.07]`. All `pointer-events-none`. All via `next/image` from local `/greeks/Logo_*.png`. None look clickable.

## Dynamic technical background (Phase D)

`src/components/landing/CosmicBackdrop.tsx`:

| New layer | Implementation |
|---|---|
| Diagonal dot field | Second `radial-gradient` dot pattern at 56 px step, animated diagonally by 112 px over 90 s — combines with the existing vertical 28 px pattern so the surface no longer reads as a single repeating axis |
| Diagonal grid distortion | `repeating-linear-gradient(45deg, …)` at 5% opacity — adds the "faint protocol-network" feel without painting cost |
| 4 diagonal protocol traces | 4 corner-to-corner SVG paths with gradient strokes, dashed, animated `stroke-dashoffset` at three independent speeds (22 s / 32 s / 48 s) |
| Soft vol wave | New sine path with `stroke-opacity` breathing between 0.18 ↔ 0.42 over 14 s |
| Existing layers | Vertical dot drift, horizontal vol + data-stream curves, scroll-driven radial glows, vignette — all retained |

`src/app/globals.css` namespace `deopt-cosmic-*`:
- NEW `.deopt-cosmic-dotfield-diag` + `@keyframes deopt-cosmic-drift-diag` (90 s)
- NEW `.deopt-cosmic-trace-{a,b,c}` + 3 keyframes at different speeds
- NEW `.deopt-cosmic-vol-wave` + `deopt-cosmic-vol-breathe` keyframe
- All listed in the `prefers-reduced-motion: reduce` media block

No new dependencies. No WebGL. No canvas. No remote assets.

## Section transitions (Phase E)

NEW `SectionEdgeFades` component rendered inside every section:
```tsx
<div className="pointer-events-none absolute inset-x-0 top-0 h-16
                bg-gradient-to-b from-black to-transparent" />
<div className="pointer-events-none absolute inset-x-0 bottom-0 h-16
                bg-gradient-to-t from-black to-transparent" />
```

Result: section boundaries blend into the cosmic backdrop rather than hard-cutting. Combined with the existing `<SectionReveal>` IntersectionObserver fade-in, the scroll feels cinematic. Vertical padding stays `py-32 sm:py-40` from V2.

## Architecture diagram redesign (Phase F)

Old V2 SVG node-graph (7 small 24 px-radius circles with 10 pt monospace labels) replaced with a **3-tier glass-card hierarchy** wrapped in a single GlassCard panel:

```
Tier 1 (Entry):    [ Trader ]  [ Signed intent ]  [ Execution layer ]
                                       ↓
Tier 2 (Core):     [ Margin engine ]  [ Collateral vault ]  [ Oracle router ]
                                       ↓
Tier 3 (Output):   [ Settlement ]                       [ Indexer / portfolio ]
```

- Each node = `<ArchNode>` glass card: `min-h-[88px]`, emerald border, monospace eyebrow with the node `id`, **14 px semibold readable label**, optional 10 px hint line.
- The **Execution layer** and **Settlement** nodes carry `emphasis` styling: tighter emerald border + soft outer glow.
- Inter-tier arrows = `<ArchArrowDown>` — 16 × 32 px SVG with a dashed shaft and a small chevron head.
- Layout collapses to single-column stacks on `< md`.
- Testids: `landing-arch-tier-{entry,core,output}` and `landing-arch-node-{trader,intent,executor,margin,vault,oracle,settle,indexer}`.

The diagram is now legible at desktop, laptop, AND mobile.

## Copy minimization (Phase G)

| Section | Before V3 | V3 |
|---|---|---|
| Hero subhead | "Options and perpetuals through a single on-chain execution layer." | unchanged (already one line) |
| Perps body | "The perps surface lives next to the options terminal — same intent pipeline, same risk engine, same vault." | **"Same intent pipeline, same risk engine, same vault."** (dropped the lead-in) |
| Architecture title | "A connected derivatives stack." | unchanged |
| Hero eyebrow chip | "Programmable derivatives" | **removed** |

Body remains free of testnet / public-beta / no-real-funds copy.

## Responsive behavior (Phase H)

- Hero stays vertically centered with `min-h-[80vh]`; CTA row uses `flex-wrap`.
- Backdrop is `fixed inset-0 overflow-hidden`; all new SVG paths still use `preserveAspectRatio="none"` so they stretch to viewport without overflow.
- Architecture tiers collapse to `grid-cols-1` on `< md` so each card occupies its own row; the inter-tier arrows still render.
- FAQ unchanged from V2.
- Ambient Greek silhouettes are positioned with viewport-anchored offsets (e.g. `-left-20`) so they sit just off-screen on narrow viewports and never cover hero text — the `pointer-events-none` guarantees they never block interaction.
- 360 px viewport: no horizontal scrollbar verified by `overflow-hidden` on the cosmic-landing root + Tailwind's default mobile-first cascade.

## Tests added / updated (Phase I)

`tests/e2e/landing-product-v2.spec.ts` rewritten — **18 tests**:

| Test | Coverage |
|---|---|
| browser title `^DeOpt$` | unchanged |
| renders backdrop + hero | unchanged |
| **hero NO LONGER renders the 'Programmable derivatives' badge** | `landing-hero-eyebrow` has count 0; hero text does not contain the phrase |
| **hero NO LONGER renders the foreground Greek constellation** | `landing-hero-greeks` count 0; all 5 `landing-hero-greek-*` testids count 0 |
| hero CTAs route | Launch → `/trade`, Markets → `/markets`, Docs → `/docs` |
| Options + Perps mentioned and linked | unchanged |
| scroll story renders all 7 sections | unchanged |
| **ambient Greek silhouettes are dense (≥ 8) and use local /greeks/ assets** | counts ≥ 8 (V2 expected ≥ 2); every src is local + resolves to `/greeks/` |
| **cosmic backdrop carries the new dynamic layers** | `cosmic-dotfield-diagonal` attached; `cosmic-grid-distort` attached; SVG contains `deopt-cosmic-trace-a/b` + `deopt-cosmic-vol-wave` |
| **architecture diagram renders 3 tiers with glass-card nodes** | `landing-arch-tier-{entry,core,output}` attached; all 8 `landing-arch-node-*` testids present |
| **architecture node labels are full readable words** | each node contains its full label ("Signed intent", "Execution layer", "Margin engine", "Collateral vault", "Oracle router", "Settlement", "Indexer") — guards against accidental regress to 4-char abbreviations |
| FAQ rows expand/collapse | unchanged |
| landing body does NOT repeat testnet/public-beta/no-real-funds | unchanged |
| no positive-claim language in `<main>` | unchanged |
| no admin/bearer/RPC/DATABASE_URL/mainnet leaks | unchanged |
| no yellow/orange/amber classes | unchanged |
| final CTA routes | unchanged |
| no broken image src | unchanged |

`tests/e2e/landing.spec.ts` left unchanged — asserts only TestnetUnauditedBanner + PublicBetaFooter.

Catalog: **184 → 186 tests in 34 files** (+2 net).

## Build validations (Phase J)

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

- NEW `docs/FRONTEND_HOMEPAGE_COSMIC_LANDING_POLISH_V3_RESULT.md` (this file)
- `docs/public-beta/PUBLIC_TESTNET_BETA_LAUNCH_CHECKLIST.md` left **unchanged** — no checklist item references hero badge presence or architecture-diagram styling.

## RUN_STATE update

2026-06-16 closure for FRONTEND-HOMEPAGE-COSMIC-LANDING-POLISH-V3 prepended above the V2 polish entry.

## Files changed

**Edited (frontend src):**
- `src/components/landing/CosmicLanding.tsx` — hero cleanup, +9 ambient silhouettes, SectionEdgeFades, new 3-tier architecture diagram with ArchNode + ArchArrowDown helpers
- `src/components/landing/CosmicBackdrop.tsx` — second diagonal dot field, diagonal grid distortion, 4 diagonal SVG protocol traces + 3 animation classes, soft vol wave with stroke-opacity breathing
- `src/app/globals.css` — keyframes for the new diagonal drift, three trace speeds, vol-wave breathing; all listed in the `prefers-reduced-motion: reduce` media block

**Rewritten (frontend tests):**
- `tests/e2e/landing-product-v2.spec.ts` — 18 tests covering V3 polish (hero badge gone, foreground Greeks gone, ambient ≥ 8, new arch tiers + node labels)

**Operator-committed assets:** the 5 PNGs in `public/greeks/` — used as imports only, never modified.

**Created (backend docs):** `docs/FRONTEND_HOMEPAGE_COSMIC_LANDING_POLISH_V3_RESULT.md`

**Edited (root):** `RUN_STATE.md`

**Untouched:** Backend Rust (ZERO), Solidity (ZERO), `scripts/local-*.sh` (ZERO), backend `.env` (mtime preserved), `~/DEOPT/private/` (mode 700), `(trading)/layout.tsx`, `TradingShell.tsx`, every other `(trading)` route, every workspace file, every market/portfolio/feedback file, `public-beta-links.ts`, `PublicBetaFooter.tsx`, `FaqSection.tsx`, `SectionReveal.tsx`, `src/app/layout.tsx`.

## Remaining homepage gaps

- The backdrop is much busier visually; on very low-power devices the animation budget could be optimised by lowering trace counts or extending durations. The `prefers-reduced-motion` fallback stays in place.
- Inter-tier connections in the architecture diagram use one downward arrow per tier. A richer version could route per-node arrows (Trader → Intent → Executor; Executor → Margin / Vault; Oracle → Margin + Settle; …) using SVG underlay — defer to V4 if operator wants the deeper graph.
- Greek silhouettes are still static (no rotation/float). A future pass could give them a slow keyframe.
- No per-section background tint variation yet (the global radial glow + diagonal traces cover the "evolving zone" spirit).

None block local QA, public-testnet-beta launch, or operator product-test.

## Next milestone recommendation

**Primary (operator action, not agent-runnable):** product-test V3 polish via `bash ~/DEOPT/scripts/local-frontend.sh`. Confirm at 1440 / 1920 / 2560:
- Browser tab still shows just `DeOpt`
- Hero has only headline + subhead + 3 CTAs (no badge, no Greek tile row)
- Background reads more dynamic: diagonal traces visible, dot field has depth, soft vol wave breathes, glow drifts with scroll
- Architecture diagram is readable at 1080p without leaning in; labels are real words; arrows flow downward through three tiers
- 13 ambient Greek sigils float behind sections without ever competing with content
- Section boundaries blend rather than hard-cut
- `prefers-reduced-motion` users still see a calm static composition

**Secondary (agent-runnable):** `BACKEND-PUBLIC-TESTNET-DEPLOY-PREFLIGHT` per existing brief.

**Strictly later (NOT NOW):** per-node arch arrows, animated Greek silhouettes, per-section background tints, max-height-transitioned FAQ rows, SEO metadata, i18n, analytics, real options/perps mocks, mainnet activation, audit firm outreach, bug bounty launch, KMS cutover, Safe migration, flipping `isMainnetEnabled()`.
