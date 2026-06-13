# FRONTEND-TESTNET-PRODUCT-V2-DA-FOLLOWUP — Result

**Date executed:** 2026-06-12
**Operator approval line accepted (verbatim):**
> "I approve DeOpt V2 frontend visual identity and UX follow-up for this run."

**Posture:** **Frontend-only visual identity + UX follow-up. Zero chain transactions. Zero broadcast. Zero mainnet. Zero backend `.env` edit. Zero private key handling. Zero audit outreach. Zero bug bounty. Zero claim of "audited" / "mainnet-ready" / "production" / "safe for real funds".**

---

## 1. Workspace

* `~/DEOPT/deopt-v2-frontend/src/` (1 new component + brand-sweep on 14 files)
* `~/DEOPT/deopt-v2-frontend/tests/e2e/` (2 new specs + 1 rewritten + 1 small update)
* `~/DEOPT/deopt-v2-backend/docs/FRONTEND_TESTNET_PRODUCT_V2_DA_FOLLOWUP_RESULT.md` (this file)
* `~/DEOPT/deopt-v2-backend/docs/public-beta/PUBLIC_TESTNET_BETA_LAUNCH_CHECKLIST.md` (status flip on community channel)
* `~/DEOPT/deopt-v2-backend/docs/OPERATOR_PUBLIC_BETA_URLS_FILL_RESULT.md` (post-hoc Discord live note)
* `~/DEOPT/RUN_STATE.md` (closure paragraph)

## 2. Visual inventory (Phase A)

* Logo assets in `public/`: `favicon.png` (138×176, DeOpt brand mark) and `logo-deopt.png` (500×500). Header was previously using `logo-deopt.png` 28×28 → didn't match the favicon shown in the browser tab.
* Amber/yellow Tailwind classes counted across 14 public-facing files (banners, footer, ui.tsx state cards, trading components, tx components, landing pill). Admin dashboard amber left untouched (operator-only, not visible to public testers).
* `MarketSelector` empty / error states fell back to a plain `EmptyState` / `ErrorState` — not a designed "trading backend is warming up" experience.
* `HowItWorks` was rendered on the landing page below the intro card.

## 3. Header logo / favicon alignment (Phase B)

* `(trading)/layout.tsx`: header `<Image src="/logo-deopt.png" width=28 height=28>` → `<Image src="/favicon.png" width=22 height=28>` to preserve the 138/176 source ratio (no distortion).
* Logo + home link both stamped with `data-testid="header-logo"` + `data-testid="header-home-link"`.
* Header bar background switched from light/zinc-200 to `bg-zinc-950 border-b border-zinc-900` to match the dark brand direction.

## 4. Landing tutorial removal (Phase C)

* `(trading)/page.tsx` rewritten: the `HowItWorks` block is no longer mounted on the landing. The component file `src/components/HowItWorks.tsx` is retained (operator may resurface it later in docs / onboarding).
* Freed vertical space replaced with a tighter hero card + a "Browse markets" section header showing the network constraint.

## 5. Markets fallback UX (Phase D)

* New `src/components/trading/MarketsFallbackCard.tsx`: a modern dark-zinc card with emerald accent border, two `kind`s (`backend-unavailable` | `no-products`), each with title + body + hint copy.
  * Retry button (`data-testid="markets-fallback-retry"`) — wired to `refetch()`.
  * Shared `ReportIssueButton` (degrades per slot).
  * Discord community link **renders as a live `<a>` to `https://discord.gg/zaEMvWuxu` whenever the link config marks it `live`**. Hidden when still a placeholder.
  * Optional `<details>` preview "What you'll see here once the backend is back" listing the testnet surfaces (option series, quote preview, oracle status, trade ticket) — so the empty state is informative even with no data.
* `MarketSelector.tsx` rewritten: wraps `useProducts` error → `MarketsFallbackCard kind="backend-unavailable"`, empty list → `MarketsFallbackCard kind="no-products"`, populated list → grouped-by-underlying card list with subtle emerald hover.

## 6. DeOpt visual identity sweep (Phase E)

Replaced amber/yellow accents with the brand palette (black background + zinc neutrals + emerald accent + red ONLY for true danger):

* `banners.tsx`:
  * `TestnetUnauditedBanner` — amber pill → zinc-950 background with emerald-500/30 border and emerald-200 strong words.
  * `MainnetDisabledBanner` — kept red (truly destructive) but toned down to `bg-red-950/80` with light red text on a 40% border.
  * `WrongNetworkBanner` — amber → zinc-950 with emerald border + emerald-200 switch button.
  * `NetworkBadge` — 4 distinct states (no-network / mainnet / wrong-network / ok) all using zinc + emerald-tinted borders; mainnet remains red.
* `PublicBetaFooter.tsx` — heading amber → emerald-200 with the brand pill format (small dot + uppercase letterspacing); link anchors now `text-emerald-200`.
* `ui.tsx`:
  * `ErrorState SOURCE_UNAVAILABLE` path → zinc-950 card with emerald-500/30 border + emerald-200 text (not a real error; testnet warm-up).
  * `ErrorState` true-error path → controlled `bg-red-950/40` with light-red text on a 40% border (not a panic-red).
  * `StaleDataBadge` → zinc-900 with emerald border + emerald-200 text.
  * Retry button → emerald-500 with black text (brand primary CTA).
* Trading component "not yet wired" notices (`BalancesCard`, `PositionsTable`, `PortfolioSummary`, `QuotePreviewCard` not-ready, `TradeTicket` create-intent-pending, `PortfolioSummary` partial-warning, `QuotePreviewCard` partial-warning) → all zinc-950 cards with emerald-500/30 border + emerald-200 text.
* `TradingHealthCard` degraded status dot → emerald-700 (muted) instead of amber-500.
* `TradeTicket` `sign-blocker-reason` paragraph → emerald-300.
* `SigningStateModal` phase color amber-500 pulses → emerald-500 pulses (`creating_intent`, `intent_pending`, `fetching_payload`, `awaiting_signature`, `submitting`).
* `TxStatusTimeline` STUCK banner → zinc-950 with emerald-500/40 border + emerald-200 text (kept REVERTED red because that IS a destructive outcome).
* `RoleReadinessCard` network badge — 4 states all switched to the same border-based palette; mainnet keeps red, wrong-network is zinc + emerald border + emerald-200 text.
* `(trading)/layout.tsx` body background → `bg-black text-zinc-100`; header → `bg-zinc-950 border-b border-zinc-900` with emerald-300 hover on nav links.
* Landing pill (`landing-public-beta-pill`) → emerald-500/40 border with emerald-500/10 fill + emerald-200 letter-spaced text.
* Landing hero gained a subtle emerald-600/10 blur glow in the upper-right corner — no animation, no dep.
* Landing CTAs use the brand primary (`bg-emerald-500 text-black`) for "Start testing" and "Retry"; ghost CTAs are emerald-500/40 border + emerald-200 text.
* Updated 3 stale "amber notice" code comments (`CreateIntentButton.tsx`, `TradeTicket.tsx` docblock, `trading-api.ts` docblock) to "emerald-bordered notice" so the docstrings don't drift from the visual.

## 7. Discord link wiring (Phase F)

* `src/lib/public-beta-links.ts` Discord entry updated:
  * `href`: `"{{PUBLIC_BETA_DISCORD_URL}}"` → `"https://discord.gg/zaEMvWuxu"`.
  * `status`: `"placeholder"` → `"live"`.
  * Added comment "Operator-supplied 2026-06-12 (FRONTEND-TESTNET-PRODUCT-V2-DA-FOLLOWUP). Public Discord invite — no secret, no admin URL, no bearer."
* All other slots remain `placeholder` (no operator URLs supplied for GitHub / quickstart / testing-guide / limitations / feedback).
* Defence-in-depth unchanged: `isPlaceholderHref()` still drives per-slot degradation; `PublicBetaFooter` + `SigningStateModal` + `ReportIssueButton` + `MarketsFallbackCard` all auto-promote the Discord slot from "(coming soon)" to live `<a>` without any further code change.

## 8. Tests added / updated (Phase G)

| Spec file | Action | Coverage |
|---|---|---|
| `tests/e2e/landing-product-v2.spec.ts` | REWRITTEN (5 specs) | intro card; pill is emerald not amber; 3 CTAs visible; **HowItWorks is NOT rendered**; no positive-claim language; DeOpt heading + Base Sepolia hero copy + mainnet-permanently-disabled line visible. |
| `tests/e2e/brand-identity.spec.ts` | NEW (5 specs) | header logo src contains `/favicon.png`; main DOM on 5 trading routes contains no `amber-*` / `yellow-*` class; public-beta footer uses emerald; Discord link is a live `<a>` pointing at `https://discord.gg/zaEMvWuxu`; Discord href contains no Bearer / RPC URL / DB credential. |
| `tests/e2e/markets-fallback.spec.ts` | NEW (3 specs) | route-intercept 500 → backend-unavailable card with retry + report-issue + live Discord link + preview details; route-intercept empty list → no-products card; fallback card uses no amber/yellow, must include emerald accent. |
| `tests/e2e/markets.spec.ts` | UPDATED | replaced "No products available" text assertion with the new `markets-fallback-card` testid. |

Catalog: `npx playwright test --list` — **51 tests in 17 files** (was 42 in 15).

## 9. Build validations

| Command | Result |
|---|---|
| `npm run typecheck` (`tsc --noEmit`) | clean |
| `npm run lint` (`eslint`) | clean |
| `npm run build` (`next build`) | green, 9 routes prerendered |
| `npx playwright test --list` | 51 tests in 17 files, parse-clean |
| Targeted spec run | not executed (WSL2 sandbox missing `libnspr4.so`; CI/Linux unaffected — same constraint as prior milestones) |

## 10. Docs created / updated (Phase H)

* NEW `deopt-v2-backend/docs/FRONTEND_TESTNET_PRODUCT_V2_DA_FOLLOWUP_RESULT.md` (this doc).
* UPDATED `deopt-v2-backend/docs/public-beta/PUBLIC_TESTNET_BETA_LAUNCH_CHECKLIST.md` (§1.5b row 7 "Community channel (Discord/Telegram) configured" → ✓ Discord).
* UPDATED `deopt-v2-backend/docs/OPERATOR_PUBLIC_BETA_URLS_FILL_RESULT.md` (post-hoc note: Discord became live via this followup, not the URL-fill milestone).
* UPDATED `~/DEOPT/RUN_STATE.md` (closure paragraph).
* No `FRONTEND_TESTNET_PRODUCT_V2_DA_FOLLOWUP_NEXT_TASK.md` created — no remaining UX/design gap blocking testers.

## 11. RUN_STATE update

Closure paragraph prepended dated 2026-06-12. Documents: header logo switched to favicon, HowItWorks removed from landing, MarketsFallbackCard added, brand palette swept across 14 files, Discord URL live, 2 new spec files + 1 rewritten, build/lint/typecheck/catalog clean, zero source changes outside frontend / docs / RUN_STATE.

## 12. Files changed

**Created (frontend src):**
* `src/components/trading/MarketsFallbackCard.tsx`

**Edited (frontend src):**
* `src/app/(trading)/page.tsx` (landing rewrite — emerald pill, hero, Browse markets section header)
* `src/app/(trading)/layout.tsx` (header logo → favicon; black body; zinc-950 header; emerald nav hover)
* `src/app/(trading)/portfolio/page.tsx` (banner copy + emerald-tinted card via underlying components)
* `src/components/banners.tsx` (full rewrite of TestnetUnauditedBanner / MainnetDisabledBanner / WrongNetworkBanner / NetworkBadge palette)
* `src/components/PublicBetaFooter.tsx` (emerald heading + emerald link anchors + zinc-500 placeholder spans + brand-pill format)
* `src/components/ui.tsx` (ErrorState SOURCE_UNAVAILABLE → zinc/emerald; ErrorState true-error → controlled red; StaleDataBadge → zinc/emerald; Retry button → emerald)
* `src/components/trading/MarketSelector.tsx` (rewrap into MarketsFallbackCard; populated cards use zinc-950 + emerald hover)
* `src/components/trading/BalancesCard.tsx` (not-ready notice → zinc/emerald)
* `src/components/trading/PositionsTable.tsx` (same)
* `src/components/trading/PortfolioSummary.tsx` (not-ready + partial-warning → zinc/emerald)
* `src/components/trading/QuotePreviewCard.tsx` (not-ready + retry button + partial-warning → zinc/emerald)
* `src/components/trading/TradingHealthCard.tsx` (degraded dot → emerald-700)
* `src/components/trading/TradeTicket.tsx` (create-intent-pending notice + sign-blocker reason → emerald; docstring "amber" → "emerald-bordered")
* `src/components/trading/CreateIntentButton.tsx` (docstring "amber" → "emerald-bordered")
* `src/components/trading/RoleReadinessCard.tsx` (4-state network badge → border-based palette; mainnet red, others emerald-tinted)
* `src/components/tx/SigningStateModal.tsx` (5 phase-color amber pulses → emerald)
* `src/components/tx/TxStatusTimeline.tsx` (STUCK banner → zinc/emerald)
* `src/lib/public-beta-links.ts` (Discord href + status flip to live)
* `src/lib/trading-api.ts` (docstring "amber" → "emerald-bordered")

**Created (tests):**
* `tests/e2e/brand-identity.spec.ts`
* `tests/e2e/markets-fallback.spec.ts`

**Edited (tests):**
* `tests/e2e/landing-product-v2.spec.ts` (rewritten — HowItWorks asserted absent)
* `tests/e2e/markets.spec.ts` (testid switch)

**Not touched:**
* `src/components/HowItWorks.tsx` — kept on disk for later use (no longer mounted in the landing).
* Backend Rust source — ZERO
* Solidity source — ZERO
* Backend `.env` — UNCHANGED (mtime preserved)
* `~/DEOPT/private/**` — NOT read, NOT committed

## 13. Validations (Phase I)

| Check | Result |
|---|---|
| `git diff --check` (frontend) | clean |
| `git diff --check` (backend) | clean |
| Sensitive-string scan on milestone files | zero hits (no bearer, no RPC URL with key, no DATABASE_URL, no private key shape) |
| Mainnet RPC pattern scan | zero hits |
| Positive-claim drift scan | zero true hits (only landing-spec `.not.toMatch()` negative assertions + this result doc's self-references) |
| Amber/yellow class scan on public-facing files | zero hits (only stale `admin/*` operator dashboard remains; admin is not visible to testers) |
| `.env` mtime preserved | YES — `2026-06-08 16:55:05` |
| Private file mode 600 preserved | YES; NOT read; NOT committed |
| Admin bearer in any frontend file | NONE (`no-admin-bearer.spec.ts` continues to enforce) |
| Chain transaction sent | NO |
| Broadcast invoked | NO |
| Mainnet RPC used | NO |
| Real wallet used | NO |
| Source changes outside frontend / docs / RUN_STATE | NONE |
| Backend Rust source changes | NONE |
| Solidity source changes | NONE |
| Audit firm contacted | NO |
| Bug bounty launched | NO |
| `isMainnetEnabled()` still hard-coded `false` | YES |

## 14. Remaining UX / design gaps

None blocking. Optional polish later:

* Re-mount `HowItWorks` inside a dedicated `/about` or `/onboarding` route once docs are linked from a public URL — currently the component file sits unused.
* Tighten typography on the markets-fallback `<details>` preview if testers report it competes with the main fallback message.
* Light-mode variant: this milestone commits to the dark brand direction (black + emerald). If a light variant is ever wanted, the brand palette is centralised in the class strings (would need a CSS-variable refactor — out of scope here).

## 15. Remaining placeholders

5 of the 6 frontend slots remain `placeholder` (no operator URLs supplied). Discord is now `live`.

| Slot | Status |
|---|---|
| quickstart | placeholder |
| testing-guide | placeholder |
| limitations | placeholder |
| feedback | placeholder |
| **discord** | **live** — `https://discord.gg/zaEMvWuxu` |
| github | placeholder |

`OPERATOR_PUBLIC_BETA_URLS_FILL_RESULT.md` updated to note the post-hoc Discord wiring.

## 16. Next milestone recommendation

**Primary:** `EXTERNAL_AUDIT_DISPATCH_PREP` — close the 7 audit-readiness BLOCKERs + 8 SHOULD-FIXes. This brand polish does not change the audit-prep path; placeholders for the other URL slots remain non-blocking for audit prep.

**Alternative (operator-side):** re-run `OPERATOR_PUBLIC_BETA_URLS_FILL` if the operator has additional URLs in hand (GitHub repo, feedback form, hosted docs root).

**Strictly later (NOT NOW):** `PUBLIC_TESTNET_BETA_LAUNCH` (announcement). Discord is now live; the announcement drafts still have placeholder tokens for GitHub + feedback form. Posting today is technically possible but the announcement value is higher once at least GitHub is live too.

**Explicitly NOT recommended now:** mainnet activation, audit outreach to firms, bug bounty launch, KMS cutover, Safe migration, flipping `isMainnetEnabled()`.

Milestone outcome: 1 new frontend src (`MarketsFallbackCard`), 14 edited frontend src (brand palette + header logo + landing rewrite + Discord live), 2 new + 2 updated specs (51 tests across 17 files), zero changes outside frontend / docs / RUN_STATE. The frontend now reads as a coherent black + deep-green DeOpt product without amber/yellow drift, the markets section degrades gracefully into a dark/green fallback with retry + report + Discord, and the header logo is consistent with the favicon shown in the browser tab.

**End of FRONTEND-TESTNET-PRODUCT-V2-DA-FOLLOWUP result.**
