# DeOpt V2 Technical Handoff Report

**Date:** 2026-06-16
**Workspace root:** `~/DEOPT/`
**Purpose:** Transfer the full DeOpt V2 project context to a new ChatGPT/Claude session.
**Posture:** Public testnet beta / Base Sepolia only. NOT audited. NOT mainnet-ready. NO real-funds safety guarantees.

---

## 1. Executive summary

DeOpt V2 is a decentralized derivatives platform aimed at **options + perps** with on-chain settlement, an EIP-712 signed-intent execution layer, a programmable risk engine, a unified collateral vault, and an oracle-aware settlement path. Its product positioning is:

- **Decentralized options and perps** with **low-fee execution**, **APIs**, and **customizable trading workspaces**.
- All trades flow through signed intents → executor matching/validation → on-chain settlement → indexed event stream.
- A single trading terminal with options chain + payoff + Greeks + collateral + perps workspace + modular pixel-canvas workspace.

The current focus is **local + Base Sepolia testnet beta frontend polish**, not deployment. Backend is in safe local mode; Solidity is on `v2-product-freeze-rc1` (frozen 2026-06-10). Mainnet is permanently blocked in this build (`isMainnetEnabled() === false`).

---

## 2. Repository map

```
~/DEOPT/
├── deopt-v2-sol/             Solidity contracts (Foundry). Source of truth for protocol ABIs + addresses.
├── deopt-v2-backend/         Rust trading backend + indexer + signer adapters + docs.
├── deopt-v2-frontend/        Next.js 16 / React 19 trading terminal + landing + workspace.
├── scripts/                  Cross-repo local-dev scripts (safe-mode by construction).
│   ├── local-backend.sh
│   ├── local-seed.sh
│   ├── local-smoke.sh
│   └── local-frontend.sh
├── RUN_STATE.md              Append-only chronological execution state. Read first.
├── TESTNET_RUNBOOK.md        Public testnet beta operator runbook.
├── private/                  Operator-private inputs. Mode 700. Never read or printed.
└── (other root docs: CURRENT_STATUS, DEOPT_CONTEXT, MAINNET_CUSTODY_*, …)
```

Key sub-folders:
- `deopt-v2-sol/abis/freeze-v2-product-rc1/` — frozen ABI manifest + selectors + storage layouts.
- `deopt-v2-sol/docs/` — protocol governance, audit prep, fees, perp engine, ownership migration.
- `deopt-v2-backend/docs/` — every milestone result/runbook/next-task (≈ 200 docs).
- `deopt-v2-backend/docs/public-beta/` — public-facing testnet beta docs (Quickstart, Testing Guide, FAQ, Limitations, etc.).
- `deopt-v2-backend/docs/openapi/` — trading API OpenAPI spec.
- `deopt-v2-frontend/src/app/(trading)/` — Next.js route group with banners + nav + page-mode + terminal-mode shell.
- `deopt-v2-frontend/src/components/landing/` — homepage (`CosmicLanding`, `ParticleField`, `FaqSection`, `SectionReveal`).
- `deopt-v2-frontend/src/components/workspace/` — modular pixel-canvas workspace (`Workspace.tsx`, `WidgetFrame.tsx`, `WidgetMenuButton.tsx`, `registry.tsx`, `widgets.tsx`).
- `deopt-v2-frontend/src/content/public-beta/` — markdown mirror of the backend public-beta docs, server-rendered under `/docs/*`.
- `deopt-v2-frontend/public/greeks/` — operator-supplied Greek glyph PNGs (Delta / Gamma / Theta / Vega / Rho).

---

## 3. Current safety posture

Hard rules applied to every milestone:
- **No mainnet.** `isMainnetEnabled()` is hard-coded `false` in the backend; chain id locked to Base Sepolia (84532).
- **No audit launched yet.** Solidity is on `v2-product-freeze-rc1` with status `TESTNET_BETA_ONLY_NOT_AUDITED_NOT_MAINNET_DEPLOYED`.
- **No bug bounty yet.**
- **No KMS / Safe / production signer yet.** All sign/broadcast paths are off in local mode.
- **No secrets in docs.** Sensitive-string + private-key + RPC-URL + DATABASE_URL + admin-bearer + mainnet-RPC scans run on every milestone.
- **Testnet/beta** language only. No "audited", "mainnet-ready", "production-ready", "safe for real funds", "guaranteed", "lowest fees", "cheapest" anywhere in user-facing copy.
- **Frontend homepage body deliberately omits `testnet` / `public beta` / `no real funds` repetition** because the global `TestnetUnauditedBanner` (above the navbar) and `PublicBetaFooter` (below page-mode wrappers) already carry that copy.
- **Backend `.env` mtime preserved** (`2026-06-08 16:55:05.874571237 +0200`).
- **Private dir preserved** at mode `700`; contents never read or printed.

---

## 4. Major completed milestones (chronological highlights)

| # | Milestone | Notes |
|---|---|---|
| 1 | **Solidity product scope freeze** | Tag `v2-product-freeze-rc1` (2026-06-10); 11 contract ABIs + selectors + storage layouts; commit `d133e2c` |
| 2 | **Backend trading API implementation (phases 2-5)** | Read-side `/options/products`, `/markets`, `/balances`, `/positions`, `/portfolio`, `/trades`; OpenAPI spec |
| 3 | **Public create-intent endpoint** | EIP-712 intent submission entrypoint |
| 4 | **Backend executor monitoring + alerts V1** | Wiring + observability |
| 5 | **Backend signer interface + AWS KMS adapter** | KMS adapters built, not yet activated for mainnet |
| 6 | **Fees Manager V2 design + deploy + integration** | Full design through hook-redeploy result |
| 7 | **Frontend trading terminal V2/V3** | Derive-like layout, options chain terminal, bottom dock, modular workspace |
| 8 | **Base Sepolia read-only checks** | `/options/products`, `/markets` live read confirmations |
| 9 | **Base Sepolia setup fixes** | Resolved values checklist, operator input provisioning |
| 10 | **First live broadcast reverted** | Wrong matching engine target; documented in `E2E_SEPOLIA_LIVE_BROADCAST_FAILURE_NEXT_TASK.md` |
| 11 | **Retarget + retry success** | Correct OME bound; broadcast succeeded |
| 12 | **First successful Base Sepolia option trade** | **Tx hash: `0x748c94843cb4cbe31f56c84ceedc7e000a05dac567fa3fe7a1415a0de59b637a`** |
| 13 | **Public beta docs pack** | Full `docs/public-beta/` set + announcement drafts |
| 14 | **Integrated docs + feedback** | `/docs/*` server-rendered from markdown; `/feedback` route |
| 15 | **Options-chain terminal V1** | Strike/expiry/underlying pills + chain ladder |
| 16 | **Modular workspace V1** | Widget system with `Widget` button + localStorage persistence |
| 17 | **Resizable terminal workspace V2/V3 cleanup** | Per-wallet/anon persistence; remove visible Reset/anon-warning |
| 18 | **Freeform workspace canvas V4** | Removed compactor; freeform widget placement |
| 19 | **Adaptive visible grid V5** | Adaptive cols based on viewport width |
| 20 | **Pixel-canvas workspace V6** | True pixel/percentage canvas; `react-grid-layout` removed |
| 21 | **Pixel-canvas hydration + layout fix** | Fixed first-paint collapsed-widgets bug; schema bumped 6→7 + strict validation + `__deoptClearWorkspaceLayouts` console helper |
| 22 | **Navbar / hamburger IA cleanup** | Portfolio + API moved out of primary nav into hamburger 3-section IA; `/fees` + `/api` placeholder routes added |
| 23 | **Homepage cosmic protocol landing V1** | Cosmic backdrop + 8 sections + Greek glyphs |
| 24 | **Homepage V2 polish** | Title "DeOpt v2" → "DeOpt"; one-line section copy; doubled vertical rhythm; scroll-driven evolving backdrop; FAQ accordion |
| 25 | **Homepage V3 polish** | Hero badge + Greek tile row removed; ambient silhouettes tripled (4 → 13); diagonal protocol traces; 3-tier glass-card architecture |
| 26 | **Homepage V4 digital-DNA landing** | Direction change: vertical helix backdrop; "Programmable derivatives. On chain." slogan; `<SoftPanel>` replaced `<GlassCard>`; HeroReadout + SideRail fill large screens |
| 27 | **Homepage V5 particle field** | DNA removed; canvas particle field with cursor attraction + click repulsion + scroll-driven mode morphing (`calm` / `wave` / `nodes` / `sparse`); FAQ title `Some Questions` |
| 28 | **Homepage V6 particle field + product messaging** ← **current** | New slogan "Decentralized options and perps with low-fee execution, APIs, and customizable trading workspaces."; particle density ~1.7×; scroll parallax drift; ambient Greek silhouettes 13 → 18 |

---

## 5. Current on-chain / Base Sepolia status

Public-safe Base Sepolia addresses (chain id `84532`):

| Role | Address |
|---|---|
| `OPTION_MATCHING_ENGINE` | `0x5a5EBF9A9CCd7c012518569DE8283982982670f6` |
| `OPTION_MARGIN_ENGINE` | `0x506cD65a63C53c66ab572B9f9dd819B7BfE00D30` |
| `OPTION_PRODUCT_REGISTRY` | `0x3d52b033fab00ed6104dd3bc0a715f8648344eca` |
| `OPTION_COLLATERAL_VAULT` | `0x00340C360353a5AB784c5Bc5c44322A6AF0625D3` |
| `OPTION_COLLATERAL_VAULT_VIEWS` | `0x00340C360353a5AB784c5Bc5c44322A6AF0625D3` |
| `OPTION_ORACLE_ROUTER` | `0xb416406f200b2ef3d7a86a5d5877ed41d9b1a581` |
| `OPTION_MARGIN_ENGINE_LENS_ADDRESS` | `0x496A57CF4e0d4F1BC5c00969Ed4C5204072ddA26` |
| `COLLATERAL_TOKEN` (mUSDC testnet mock) | `0x6eAe407f5640B006faC9965182e238582A3B412E` |

State:
- **First successful Sepolia trade tx:** `0x748c94843cb4cbe31f56c84ceedc7e000a05dac567fa3fe7a1415a0de59b637a`.
- **Current live broadcast status:** retry milestone completed; matching-engine retarget confirmed.
- **Backend indexer / reconciliation:** workers exist and pass local tests; in local safe mode they are forced OFF (no DB persistence required to start).
- **Mainnet:** **NOT activated.** Permanently blocked in this build.
- **No private keys, RPC URLs, or admin bearer tokens included here** — those live in operator-private files (`~/DEOPT/private/`, never read by the agent) and in the backend `.env` (mtime preserved, never edited).

---

## 6. Backend current status

- **Stack:** Rust (`cargo`), Axum-style HTTP server bound to `127.0.0.1:8080` in local mode.
- **Safe-mode locks (applied by `scripts/local-backend.sh`):**
  - `broadcast: OFF`
  - `signer: OFF` (EXECUTOR_PRIVATE_KEY ignored; no AWS/KMS path)
  - `persistence: false`
  - `options: in-memory` (`OPTIONS_REQUIRE_PERSISTENCE=false`)
  - `workers: OFF` (option confirmation / reconciliation / event-indexer / nonce-sync / fees)
  - `mainnet: blocked` (chain id pinned at `84532`)
  - CORS origins: `http://localhost:3000, http://127.0.0.1:3000, http://localhost:3001, http://127.0.0.1:3001`
- **Public API endpoints (read-side):** `/health`, `/ready`, `/trading/health`, `/options/products`, `/markets`, `/balances`, `/positions`, `/portfolio`, plus `/transactions/<intent_id>` for lifecycle inspection.
- **No public deployment yet.** Previous Railway attempt had a start-command issue (documented in backend docs); target platforms are Railway or Render.
- **Local smoke** consistently `9 / 9 PASS`.

### Local commands

```bash
cd ~/DEOPT
./scripts/local-backend.sh           # starts the backend in safe mode (foreground)
```

In a second terminal:
```bash
cd ~/DEOPT
./scripts/local-seed.sh              # seeds 12 option products (6 strikes × Call + Put × 2 expiries)
./scripts/local-smoke.sh             # runs the 9-check read-only smoke
```

Expected smoke output:
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

### Frontend dev server

```bash
cd ~/DEOPT
./scripts/local-frontend.sh          # runs `npm run dev` in the frontend repo
```

---

## 7. Frontend current status

- **Stack:** Next.js `16.1.6` + React `19.2.3` + Tailwind `v4` + TypeScript `5` + `marked@18` for docs + `viem@2.52`.
- **Tests:** `@playwright/test@1.60`. Current catalog: **188 tests in 34 files**.
- **Routes (19 user-facing + 4 SSG doc slugs):**

| Route | Mode | Notes |
|---|---|---|
| `/` | page-mode | Cosmic / particle-field landing |
| `/trade` | terminal-mode | Options modular workspace |
| `/perps` | terminal-mode | Perps modular workspace (placeholder widgets) |
| `/markets` | terminal-mode | Markets browser |
| `/markets/[productId]` | dynamic | Per-product detail |
| `/portfolio` | terminal-mode | Portfolio summary + positions + balances |
| `/custom` | terminal-mode | Empty modular workspace |
| `/fees` | page-mode | Placeholder (honest testnet copy) |
| `/api` | page-mode | Placeholder (testnet read-side + roadmap) |
| `/docs` | page-mode | Docs index |
| `/docs/quickstart` | SSG | From `BASE_SEPOLIA_QUICKSTART.md` |
| `/docs/testing-guide` | SSG | From `USER_TESTING_GUIDE.md` |
| `/docs/limitations` | SSG | From `KNOWN_LIMITATIONS_AND_RISKS.md` |
| `/docs/faq` | SSG | From `FAQ.md` |
| `/feedback` | page-mode | Public-safe bug report template |
| `/history` | page-mode | Trade history |
| `/health` | page-mode | Health surface |
| `/transactions/[requestId]` | dynamic | Intent lifecycle inspector |
| `/admin` | (operator-only) | Read-only operations dashboard (not part of public surface) |

### Navbar state

- **Primary navbar:** `DeOpt` logo · `Options` · `Perps` · `Markets` · `Custom` · `DeOpt Académie (coming soon)` · right side `NetworkBadge` + `WalletConnectButton` + `WidgetMenuButton` + `HamburgerMenu`.
- **Portfolio + API are removed from primary nav** — they live in the hamburger drawer (Pages section).
- **Hamburger drawer** has three sections: **Pages** (Portfolio, Fees, API, Feedback), **Docs** (Docs, Quickstart, Known limitations, FAQ), **Community** (Discord, GitHub).
- **Discord:** `https://discord.gg/zaEMvWuxu`
- **GitHub:** `https://github.com/DeOpt`

### Local dev

```bash
cd ~/DEOPT
./scripts/local-frontend.sh
```

Backend must be running on `http://localhost:8080` (default) before testing trade flows. The frontend uses `NEXT_PUBLIC_TRADING_API_BASE_URL` to point at it.

---

## 8. Workspace system current status

- **Component graph:** `Workspace.tsx` + `WidgetFrame.tsx` + `WidgetMenuButton.tsx` + `registry.tsx` + `widgets.tsx` in `src/components/workspace/`; types/storage in `src/lib/workspace-types.ts` and `src/lib/workspace-storage.ts`; bridge context in `src/lib/workspace-bridge.tsx`.
- **Engine:** **Pure pixel/percentage canvas** (no `react-grid-layout` — uninstalled in V6). Each widget stores `xPct / yPct / wPct / hPct` in `[0, 1]` plus per-type `minWPx` / `minHPx`.
- **Snap:** `CANVAS_SNAP_PX = 24`. Visible dotted backdrop uses the same constant; drag/resize round to it.
- **Layout schema version:** `WORKSPACE_LAYOUT_VERSION = 7`. V5 column buckets and V6 buckets get safe-reset on load via the version-mismatch path.
- **Strict load validation:** `isValidWorkspaceLayout()` rejects NaN/Infinity geometry, sub-readable widget sizes (`MIN_WIDGET_PCT = 0.04`), unknown widget types, missing fields. Invalid layouts are pruned at boot via `pruneExpiredLayouts()`.
- **Canvas measurement guard:** `isCanvasReady(canvas)` requires ≥ `MIN_CANVAS_WIDTH_PX = 320` AND ≥ `MIN_CANVAS_HEIGHT_PX = 240`. Below that, a `workspace-canvas-measuring-{id}` placeholder shows instead of widgets. `useLayoutEffect` runs the first measurement pre-paint.
- **Persistence:** `localStorage` under `deopt:v2:workspace:<wallet|anon>`. TTL: **30 days** for connected wallets, **24 hours** for anon. No secrets / private keys / RPC URLs / bearer tokens / DATABASE_URL / signatures ever written.
- **Drag/resize:** pointer events on the header (drag) and bottom-right handle (resize). Canvas owns the `pointermove` / `pointerup` / `pointercancel` listeners so gestures that leave the widget bounding rect are not lost. Pointer capture is set on gesture start.
- **Right dead-zone bug:** fixed at the source via the V6 pixel canvas + V7 hydration guards. Regression-tested.
- **Hydration / collapsed-widgets bug:** fixed in V7 — canvas div is unconditionally rendered, `useLayoutEffect` measures pre-paint, every widget rect is resolved through `resolveWidgetRect()` (clamped to `minWPx`/`minHPx`).
- **UI:** `Widget` button in navbar opens the menu; no visible "Reset Layout" button; no visible "Anonymous layout is temporary" message; `__deoptClearWorkspaceLayouts()` is exposed on `window` as a console-only recovery helper.

---

## 9. Homepage current state and design direction

⚠️ **We are currently iterating on `/`.** Do not move to other pages until the operator visually approves the homepage.

### Current state (post-V6)

- **Slogan:** `Decentralized options and perps with low-fee execution, APIs, and customizable trading workspaces.` (second clause renders through an emerald gradient).
- **Subhead:** `Trade and build on a derivatives interface designed for options chains, perps workspaces, API access, and modular execution flows.`
- **Backdrop:** `ParticleField` canvas — 120-320 emerald particles (density `floor((w × h) / 6500)` clamped), radius `0.9-2.5`, soft parallax drift (`Δscroll × 0.12`), cursor attraction (mild), click repulsion (decays over 600 ms), scroll-driven mode morphing (`calm` → `wave` → `nodes` → `sparse`).
- **Sections:** Hero / Products (Options + Perps split side-by-side) / Execution path / Architecture (3-tier glass cards) / Risk (margin · vault · oracle) / **FAQ titled `Some Questions`** / Final CTA.
- **Cards:** `<SoftPanel>` — `rounded-2xl bg-gradient-to-b from-zinc-950/55 to-zinc-950/15 backdrop-blur-md` + inset radial shadow + thin top highlight. **No hard 1 px emerald borders.**
- **Ambient Greek silhouettes:** 18 total (Hero 3 / Products 4 / Execution 2 / Architecture 2 / Risk 2 / FAQ 2 / Final CTA 3). All `pointer-events-none`, `opacity-[0.05] sm:opacity-[0.07]`, local `/greeks/Logo_*.png`.
- **Large-screen fill:** `<HeroReadout>` terminal-style strip + `<SideRail>` vertical chip stacks (visible at `lg+`).
- **Reduced motion:** `prefers-reduced-motion: reduce` short-circuits the rAF loop and disables all keyframe animations.

### Visual identity (HOLD)

- black / deep green / emerald · premium · technical · futuristic.
- particle field · subtle Greeks · soft glass · scroll-driven evolution.
- **No DNA / helix** (V4 direction abandoned).
- **No yellow / orange / amber.**
- **No generic SaaS card grid.**
- **No cartoon crypto look.**
- **No huge empty areas.**
- **No strong bordered cards.**
- **No foreground Greek icon row in hero.**

### Copy rules (HOLD)

- Body **must not** repeat `testnet` / `public beta` / `no real funds` — global banners cover that.
- **No** `audited`, `mainnet-ready`, `production-ready`, `safe for real funds`, `guaranteed`, `lowest fees`, `cheapest`.
- "low-fee" / "fee-efficient" allowed as product positioning.
- English only.

### Slogan history (do not regress)

- ❌ `Programmable derivatives. On chain.` (V1-V3)
- ❌ `The execution layer for programmable derivatives.` (V4-V5)
- ✅ `Decentralized options and perps with low-fee execution, APIs, and customizable trading workspaces.` (V6 — current)

---

## 10. Current deployment status

- **Frontend:** not publicly deployed. Target: **Vercel**.
- **Backend:** not publicly deployed. Target: **Railway** or **Render**. Previous Railway attempt had a start-command issue (`docs/`-side runbook); to be revisited.
- **Public launch blockers:** primarily `APP_URL` (frontend public URL) and the backend public URL — both needed by `docs/public-beta/OPERATOR_PUBLIC_BETA_URLS_FILL.md`.
- **Visual freeze must happen first.** Operator wants every page visually approved before public deploy.

### Planned deployment sequence (DO NOT EXECUTE YET)

1. `BACKEND-PUBLIC-TESTNET-DEPLOY-PREFLIGHT` (next-task doc exists).
2. Backend public deploy (Railway or Render).
3. Frontend public deploy (Vercel).
4. `OPERATOR-PUBLIC-BETA-URLS-FILL` (replace `{{APP_URL}}` etc. placeholders).
5. `PUBLIC-TESTNET-BETA-LAUNCH-PREFLIGHT`.
6. `PUBLIC-TESTNET-BETA-LAUNCH`.

---

## 11. Current immediate next recommended work

Most likely next milestone:

**Either** (A) one more homepage polish iteration if the V6 particle field still needs tuning, **or** (B) operator visual approval of the homepage and a move to the next surface.

If (B), the page-by-page polish order is:

1. **Homepage `/`** ← current focus
2. **Options terminal `/trade`** — make the chain ladder + detail panel + bottom dock feel as premium as the homepage; verify the pixel-canvas defaults look correct at 1440 / 1920 / 2560.
3. **Perps `/perps`** — placeholder polish; honest "workspace ships ahead of the executor" framing.
4. **Markets `/markets`** — premium markets browser feel.
5. **Portfolio `/portfolio`** — visual parity pass.
6. **Docs / Feedback** — minor consistency pass with the new landing identity.
7. **Deployment prep** — backend public testnet deploy preflight, then platform decision.

---

## 12. Working style / prompt style for future sessions

A new assistant continuing this work should:

- **Answer in French.** Operator is French-speaking. (Code/docs stay English.)
- Provide a **concise strategic explanation first**, then **a full Claude Code prompt when asked**.
- Prompts must include, in this order:
  1. Workspace root (`~/DEOPT`).
  2. **Operator approval line** (verbatim, milestone-specific).
  3. Hard rules block: **no transactions, no broadcast, no mainnet, no mainnet RPC, no `.env` edits, no private keys printed, no audited/mainnet-ready claims**.
  4. Current state context.
  5. Files to inspect.
  6. **Phased work plan** (A, B, C, …).
  7. Tests to add/update.
  8. Validations to run.
  9. Docs to create/update.
  10. RUN_STATE update requirement.
  11. Final report format.
- The operator's frequent phrase is **"Top, passe-moi le prochain prompt"** — meaning: deliver the next actionable Claude Code prompt directly.
- Operator wants **iterative product polish**, not generic advice. Be specific about file paths, testids, regex assertions, before/after diffs.
- Keep responses in chat **short and result-oriented**. Long prose goes into the milestone result doc.

---

## 13. Standard validation expectations

Every milestone runs (from `deopt-v2-frontend/` unless noted):

```bash
npm run typecheck                                              # tsc --noEmit
npm run lint                                                   # eslint
NEXT_PUBLIC_TRADING_API_BASE_URL=http://localhost:8080 \
  npm run build                                                # next build
npx playwright test --list                                     # spec catalog count
```

If the backend was touched, also:
```bash
cd deopt-v2-backend && cargo check   # then cargo test if reasonable scope
```

Local end-to-end if the backend is implicated:
```bash
~/DEOPT/scripts/local-backend.sh     # background
~/DEOPT/scripts/local-seed.sh
~/DEOPT/scripts/local-smoke.sh       # expect 9/9 PASS
# kill backend cleanly afterward
```

Sensitive / posture scans on every changed file:
- `git diff --check` (no whitespace errors)
- Sensitive-string scan: `0x[a-fA-F0-9]{64}`, `Bearer\s+[A-Za-z0-9_.-]{16,}`, `alchemy\.com/v2/`, `infura\.io/v3/`, `PRIVATE_KEY=`, `ANTHROPIC_API_KEY=`, `mainnet\.base\.org`, `DATABASE_URL=`
- Private-key scan / RPC URL scan / DATABASE_URL scan / admin-bearer scan / mainnet-RPC scan
- Positive-claim scan: `\bis audited\b`, `\bmainnet[- ]ready\b`, `\bproduction[- ]ready\b`, `\bsafe for real funds\b`, `\bguaranteed\b`, `\blowest fees\b`, `\bcheapest\b`
- Amber/yellow/orange class scan: `\b(amber|yellow|orange)-[0-9]{2,3}\b`, `bg-(amber|yellow|orange)`

Posture confirmations:
- backend `.env` mtime preserved (`2026-06-08 16:55:05.874571237 +0200`)
- private dir mode preserved (`700`)
- no private input files printed / tracked / committed
- no chain transaction happened
- no broadcast / send / deploy / mint / approve / transfer happened
- no mainnet use
- source changes limited to declared scope

**Full Playwright run is NOT FEASIBLE on the operator's WSL host** — `chromium_headless_shell` reports `error while loading shared libraries: libnspr4.so: cannot open shared object file: No such file or directory`. Use `--list` for catalog enumeration; actual e2e execution happens on a host with the system deps installed.

---

## 14. Open questions / pending decisions

| Decision | State |
|---|---|
| Final homepage visual approval | **pending operator** |
| Backend public hosting (Railway vs Render) | undecided; Railway attempt previously blocked on start-command |
| Frontend public hosting | **Vercel** (most likely) |
| Public API docs route finalization | `/api` placeholder ships honest read-side endpoints; full reference deferred |
| Fees page finalization | `/fees` is an honest placeholder; real schedule pending Fees Manager V2 ops decisions |
| DeOpt Académie | placeholder; later in beta cycle |
| External audit firm engagement | **after** product-complete freeze + mainnet roadmap clarified |
| Bug bounty launch | **after** audit |
| KMS / Safe / mainnet activation | sequenced after audit; documented in `MAINNET_CUSTODY_POLICY.md` + `BACKEND_SIGNER_CUTOVER_RUNBOOK_V2G_FX_Q1.md` |
| Discord / GitHub / Vercel domain | live (Discord + GitHub URLs above); Vercel domain TBD |

---

## Appendix A — Where to look first when resuming

1. **`~/DEOPT/RUN_STATE.md`** — most recent closure paragraphs are at the top. Read the latest 2-3.
2. **`~/DEOPT/CURRENT_STATUS.md`** — operator's lightweight current-state file.
3. **`~/DEOPT/NEXT_TASK.md`** — pending next task if pre-loaded.
4. **`deopt-v2-backend/docs/FRONTEND_HOMEPAGE_PARTICLE_FIELD_V6_RESULT.md`** — latest homepage milestone result.
5. **`deopt-v2-frontend/src/components/landing/`** — current homepage component graph.
6. **`deopt-v2-sol/abis/freeze-v2-product-rc1/freeze-manifest.json`** — frozen ABI manifest.

## Appendix B — Frequently used file paths

```
~/DEOPT/RUN_STATE.md
~/DEOPT/scripts/local-{backend,seed,smoke,frontend}.sh
~/DEOPT/deopt-v2-backend/docs/public-beta/
~/DEOPT/deopt-v2-frontend/src/app/layout.tsx                    # root metadata "DeOpt"
~/DEOPT/deopt-v2-frontend/src/app/(trading)/layout.tsx          # banners + nav shell
~/DEOPT/deopt-v2-frontend/src/app/(trading)/page.tsx            # landing entry
~/DEOPT/deopt-v2-frontend/src/components/TradingShell.tsx       # page-mode vs terminal-mode
~/DEOPT/deopt-v2-frontend/src/components/HamburgerMenu.tsx
~/DEOPT/deopt-v2-frontend/src/components/PublicBetaFooter.tsx
~/DEOPT/deopt-v2-frontend/src/components/landing/
  CosmicLanding.tsx
  ParticleField.tsx
  FaqSection.tsx
  SectionReveal.tsx
~/DEOPT/deopt-v2-frontend/src/components/workspace/
  Workspace.tsx
  WidgetFrame.tsx
  WidgetMenuButton.tsx
  registry.tsx
  widgets.tsx
~/DEOPT/deopt-v2-frontend/src/lib/
  workspace-types.ts           # V7 schema + validation + clamp
  workspace-storage.ts         # localStorage + clearWorkspaceLayouts
  workspace-bridge.tsx         # navbar ↔ workspace context
  workspace-selected-option.tsx
  public-beta-links.ts
~/DEOPT/deopt-v2-frontend/tests/e2e/landing-product-v2.spec.ts
~/DEOPT/deopt-v2-frontend/tests/e2e/workspace-*.spec.ts
~/DEOPT/deopt-v2-frontend/public/greeks/Logo_{Delta,Gamma,Theta,Vega,Rho}.png
```

## Appendix C — Identifying the active milestone from a chat

The operator's milestone briefs always:
- start with **"Workspace root is `~/DEOPT`."**
- contain **"Execute <MILESTONE-NAME> only."**
- carry an explicit **"I approve <…> for this run."** line that the assistant must accept verbatim and CONSUME.
- end with a list of **return-final-report** groups.

If the chat lacks any of those, do **not** start a milestone — ask the operator for the brief.
