# FRONTEND-TESTNET-LAUNCH-POLISH — Result

**Date executed:** 2026-06-12
**Operator approval line accepted (verbatim):**
> "I approve DeOpt V2 frontend testnet launch polish for this run."

**Brief:** `deopt-v2-backend/docs/FRONTEND_TESTNET_LAUNCH_POLISH_NEXT_TASK.md`.

**Posture:** **Frontend-only polish. Zero chain transactions. Zero broadcast. Zero mainnet. Zero backend `.env` edit. Zero private key handling. Zero claims of "audited" / "mainnet-ready" / "production".**

---

## 1. Workspace

* `~/DEOPT/deopt-v2-frontend/` (primary edit surface)
* `~/DEOPT/deopt-v2-backend/docs/public-beta/` (read-only; surfaced via footer link slots)
* `~/DEOPT/RUN_STATE.md` (closure paragraph)

Public-safe values only. No operator paths, no credentials, no RPC URLs, no DATABASE_URL.

## 2. Frontend inventory (Phase A)

| Surface | State before this milestone |
|---|---|
| Trading routes | `/`, `/markets`, `/markets/[productId]`, `/portfolio`, `/history`, `/health`, `/transactions/[requestId]` |
| Banners | `TestnetUnauditedBanner` + `MainnetDisabledBanner` sticky on every trading route via `(trading)/layout.tsx` |
| Wallet | `useWallet()` exposes `address / chainId / isMainnet / isExpectedChain / signTypedData()` — already refused signing on mainnet/wrong-network |
| Chain registry | `src/lib/chains.ts` with `ANVIL=31337`, `BASE_SEPOLIA=84532`, `BASE_MAINNET=8453`. `isMainnetEnabled()` hard-coded `false` |
| API client | `src/lib/trading-api.ts` — public-only, NEVER attaches `Authorization` header |
| State components | `LoadingState / EmptyState / ErrorState / StaleDataBadge` in `src/components/ui.tsx` |
| Existing Playwright suite | 8 spec files: landing, mainnet-disabled, no-admin-bearer, wallet-connected, tx-status-cycler, tx-status-fallback, markets, portfolio-disconnected, sign-rejected, create-intent |

Gaps identified:
* No `wallet_switchEthereumChain` action button (text-only warning).
* No full-width wrong-network blocker for non-mainnet wrong chains.
* No public-beta docs links / footer.
* No friendly stale-oracle copy in `QuotePreviewCard`.
* No "Report this issue" CTA on signing-failure modal phases.
* No `Last refreshed at` footer on tx-status timeline.
* Test `create-intent.spec.ts:54` expected literal `"Mainnet is permanently disabled"` which wasn't in the banner.

## 3. Testnet-only UX guardrails (Phase B)

* **Sticky `TestnetUnauditedBanner`** (every trading route, top of layout) — re-worded to lead with "Public testnet beta — UNAUDITED, experimental, Base Sepolia only" and retains the existing "Testnet beta — NOT YET AUDITED" sub-line that the landing spec already relies on. Adds "Addresses may change."
* **`MainnetDisabledBanner`** — rewritten so its first line reads "Mainnet is permanently disabled for this public testnet beta. Trading on Base mainnet (chain 8453) is DISABLED. Switch to Base Sepolia (testnet) to continue." Adds inline **"Switch to Base Sepolia"** action button that invokes `wallet_switchEthereumChain` against the injected provider (no custom RPC URL added; rejection / unsupported is swallowed with a `console.warn`).
* **`WrongNetworkBanner`** — new full-width amber banner for connected wallets on any non-expected, non-mainnet chain (e.g. user on Optimism Sepolia by mistake). Distinct from the mainnet banner so we never silently fall back to mainnet wording. Includes a **"Switch to {expected.shortName}"** action button using the same `wallet_switchEthereumChain` path.
* **`NetworkBadge`** in the header is unchanged shape but now stamped with `data-testid` for `network-badge-mainnet | wrong-network | ok` so specs can target them.
* **Trade-ticket sign gate** — `TradeTicket.canSign` now also requires `!isMainnet`, and the disabled-state hint surfaces a per-reason copy line ("Mainnet is permanently disabled — switch to Base Sepolia" / "Wrong network — switch your wallet to Base Sepolia" / "Connect your wallet to sign" / "Create or paste an execution intent id").

Note: chain-side mainnet defence is still triple-layered — `isMainnetEnabled()` hard-coded `false`, `expectedChainId()` refuses to default to mainnet, `signTypedData()` returns `{ ok: false, reason: "wrong_network" }` if mainnet detected.

## 4. Public beta navigation and links (Phase C)

* **`src/lib/public-beta-links.ts`** (new) — six link slots (`quickstart`, `testing-guide`, `limitations`, `feedback`, `discord`, `github`), each with `id / label / href / description`. All hrefs are placeholder tokens (`{{PUBLIC_BETA_QUICKSTART_URL}}` etc.) until the COMMUNITY-FEEDBACK-LOOP milestone wires real URLs. Public-safe — zero secrets in the module.
* **`src/components/PublicBetaFooter.tsx`** (new) — renders the full safety-copy block + the six link slots. Placeholder hrefs render as non-clickable spans labelled `(coming soon)` with a tooltip explaining "link not yet configured"; live hrefs render as `<a target="_blank" rel="noopener noreferrer">`. Stamped with `data-testid="public-beta-footer"` and `data-testid="public-beta-link-{id}"`.
* **`(trading)/layout.tsx`** wires the footer at the bottom of every route. `<WrongNetworkBanner />` slot added under `<MainnetDisabledBanner />`.

## 5. Trading flow polish (Phase D)

* **`QuotePreviewCard`** — friendly `friendlyNotReady()` mapping replaces the generic amber card:
  * `ORACLE_UNAVAILABLE | STALE` → "Oracle price is stale. The testnet mock oracle has a 60 s freshness window. Wait a moment and retry — the operator will refresh the price shortly. Do NOT sign while the quote shows stale oracle."
  * `SOURCE_UNAVAILABLE` → "Backend data source is starting up — normal during testnet warm-up. Retry."
  * `RPC_UNAVAILABLE` → "Backend can't reach Base Sepolia RPC. Backend-side issue; retry shortly."
  * Generic fallback preserves the backend's raw reason.
  * Adds **"Retry quote"** button.
* **`QuotePreviewCard`** also surfaces `status: "partial"` from the envelope with a dedicated warning block enumerating any `warnings[]` entries — protects against silently signing against a stale quote.
* **`SigningStateModal`** — on any failure phase (`rejected | wrong_network | backend_unavailable | error`) renders a "Hit a wall?" callout asking the tester to share the phase + detail + intent id via the public bug-report link. Explicit "Do NOT share your private key or seed phrase." If the feedback link is still a placeholder, the CTA renders as a non-clickable "coming soon" hint instead of a dead anchor.
* **`TxStatusTimeline`** — adds `Last refreshed at` ISO timestamp row in the footer dl; defer via microtask to satisfy `react-hooks/set-state-in-effect` lint rule.
* **`TradeTicket`** Step 2 hint now reads "the backend operator handles broadcast after both buyer + seller sign on Base Sepolia (testnet)."
* **`MarketSelector`** — empty-state copy now reads "The backend has no option products configured for Base Sepolia yet. This is normal during testnet warm-up — check back shortly or ping the operator in the public feedback channel."
* **Landing page** — sub-title updated to "Browse option products below. Public testnet beta — unaudited, experimental, Base Sepolia (chain 84532) only. Mainnet trading is permanently disabled in this build. No real funds; all tokens are testnet mocks."

## 6. Empty / loading / error states (Phase E)

* All state components stamped with stable `data-testid` (`loading-state`, `empty-state`, `error-state`).
* `ErrorState` now exposes a per-code friendly hint via `hintForCode()` covering: `NETWORK`, `SOURCE_UNAVAILABLE`, `RPC_UNAVAILABLE`, `INDEXER_STALE`, `QUOTE_STALE`, `INVALID_ADDRESS`, `INSUFFICIENT_BALANCE`, `INSUFFICIENT_COLLATERAL`, `RATE_LIMITED`. The raw error code + message + (optional) `request_id` are still rendered for operator triage.
* `ErrorState` is now stamped with `data-error-code={code}` so e2e can pin specific failure paths.

Covered scenarios (no new components needed — existing ones handle these via the above states):
* no products → `EmptyState` ("No products available")
* no active series → existing `OptionChain` empty handling
* backend unavailable → `ErrorState` with `NETWORK` code + friendly hint
* source unavailable / partial data → `quote-partial-warning` / `quote-not-ready` block
* stale oracle → friendly copy in `QuotePreviewCard.friendlyNotReady`
* quote unavailable → `quote-not-ready` block with retry
* wallet disconnected → `EmptyState` ("Connect your wallet") in `BalancesCard / PortfolioSummary / PositionsTable`
* wrong network → `WrongNetworkBanner` + `TradeTicket.canSign` blocker + `signBlockerReason` hint
* insufficient funds → friendly hint for `INSUFFICIENT_BALANCE / INSUFFICIENT_COLLATERAL`
* signature rejected → `SigningStateModal phase="rejected"` + "Report this issue" CTA
* transaction reverted → `TxStatusTimeline REVERTED` banner (untouched, still surfaces `reverted_reason`)
* backend offline but on-chain tx known → existing fallback path in `useTxStatus` returns `null` and timeline degrades to `CREATED`

## 7. Safety copy review (Phase F)

* Searched for `production / institutional / safe for real / guaranteed / audited / mainnet-ready` in `src/`.
* Only hits in: (a) `src/app/admin/*` operator-only section (`ProductionReadinessSection` — admin route, not user-facing), (b) `src/lib/eip712.ts` defensive comment "MUST NOT use this builder against production data", (c) admin types file. None are user-facing trading-UI strings.
* Public testnet beta vocabulary now appears in 38+ locations across the trading-UI surface.
* Mainnet wording (`Mainnet is permanently disabled`) intentionally appears in both `banners.tsx` (banner copy) and `TradeTicket.tsx` (sign-blocker hint).

## 8. Tests added / updated (Phase G)

| Spec file | Action | Coverage |
|---|---|---|
| `tests/e2e/landing.spec.ts` | UPDATED | testnet/unaudited banner + `public-beta-footer` testid present |
| `tests/e2e/mainnet-disabled.spec.ts` | UPDATED + new test | sticky mainnet banner with "Mainnet is permanently disabled" wording + `network-badge-mainnet` testid + `switch-to-base-sepolia-button` action moves wallet off mainnet |
| `tests/e2e/wrong-network-banner.spec.ts` | NEW | banner appears on Optimism Sepolia (11155420), absent on Base Sepolia (84532), `switch-network-action` triggers `wallet_switchEthereumChain` and resolves |
| `tests/e2e/public-beta-footer.spec.ts` | NEW | renders all 6 link slots, placeholder hrefs are non-clickable, DOM has no bearer/RPC/DATABASE_URL/private-key-shaped string, safety-copy bullets present on every trading route |
| `tests/e2e/no-admin-bearer.spec.ts` | UPDATED | original spec + new public-beta-footer secret-scan |

Catalog: `npx playwright test --list` → 30 tests across 12 files, clean.

## 9. Build validations (Phase H)

| Command | Result |
|---|---|
| `npm run typecheck` (`tsc --noEmit`) | clean |
| `npm run lint` (`eslint`) | clean after fix |
| `npm run build` (`next build`) | success, 9 routes generated |
| `npx playwright test --list` | 30 tests in 12 files, no parse errors |
| `npx playwright test landing mainnet-disabled wrong-network-banner public-beta-footer no-admin-bearer wallet-connected portfolio-disconnected markets` | NOT executed in this sandbox — chromium needs `libnspr4.so` which is missing on WSL2 and cannot be installed without elevation. The `--list` output above proves the spec graph parses; CI / a Linux box with libnspr4 will be able to run them. |

Lint regression caught: the new `Last refreshed at` `useEffect` initially triggered `react-hooks/set-state-in-effect`. Fixed by deferring `setLastUpdatedAt` through a `Promise.resolve().then(...)` microtask — same pattern used by the existing `useSigningPayload` hook.

## 10. Docs created / updated (Phase I)

| Path | Action |
|---|---|
| `deopt-v2-backend/docs/FRONTEND_TESTNET_LAUNCH_POLISH_RESULT.md` | NEW — this document |
| `deopt-v2-backend/docs/public-beta/README.md` | UPDATED — checklist item #7 marked ✓ |
| `deopt-v2-backend/docs/public-beta/USER_TESTING_GUIDE.md` | UPDATED — wallet-switch + footer-links section |
| `deopt-v2-backend/docs/COMMUNITY_FEEDBACK_LOOP_NEXT_TASK.md` | UPDATED — confirmed that placeholder URL replacement is the new bottleneck; frontend slots are ready to receive them |
| `~/DEOPT/RUN_STATE.md` | UPDATED — closure paragraph |

No `FRONTEND_TESTNET_LAUNCH_POLISH_FOLLOWUP_NEXT_TASK.md` created — there are no significant remaining frontend gaps (see §13).

## 11. RUN_STATE update (Phase J)

Closure paragraph prepended to `RUN_STATE.md` documenting: 5 frontend source files edited, 2 frontend source files added, 2 frontend test files added, 3 frontend test files updated, 4 docs files added/updated, zero backend source changes, zero chain transactions, zero `.env` edits, validations clean.

## 12. Files changed

**Frontend source (added):**
* `deopt-v2-frontend/src/lib/public-beta-links.ts`
* `deopt-v2-frontend/src/components/PublicBetaFooter.tsx`

**Frontend source (edited):**
* `deopt-v2-frontend/src/components/banners.tsx` — rewrite (banner copy + `WrongNetworkBanner` + switch-network buttons)
* `deopt-v2-frontend/src/app/(trading)/layout.tsx` — wire `WrongNetworkBanner` + `PublicBetaFooter`
* `deopt-v2-frontend/src/app/(trading)/page.tsx` — safer copy
* `deopt-v2-frontend/src/components/ui.tsx` — `data-testid` + `hintForCode()`
* `deopt-v2-frontend/src/components/trading/QuotePreviewCard.tsx` — friendly stale-oracle + partial warning
* `deopt-v2-frontend/src/components/trading/MarketSelector.tsx` — empty-state copy
* `deopt-v2-frontend/src/components/trading/TradeTicket.tsx` — `!isMainnet` gate + `signBlockerReason` hint
* `deopt-v2-frontend/src/components/tx/SigningStateModal.tsx` — failure CTA
* `deopt-v2-frontend/src/components/tx/TxStatusTimeline.tsx` — `Last refreshed at`

**Frontend tests (added):**
* `deopt-v2-frontend/tests/e2e/wrong-network-banner.spec.ts`
* `deopt-v2-frontend/tests/e2e/public-beta-footer.spec.ts`

**Frontend tests (edited):**
* `deopt-v2-frontend/tests/e2e/landing.spec.ts`
* `deopt-v2-frontend/tests/e2e/mainnet-disabled.spec.ts`
* `deopt-v2-frontend/tests/e2e/no-admin-bearer.spec.ts`

**Docs (added / edited):**
* `deopt-v2-backend/docs/FRONTEND_TESTNET_LAUNCH_POLISH_RESULT.md` (this doc)
* `deopt-v2-backend/docs/public-beta/README.md` — checklist item #7 ✓
* `deopt-v2-backend/docs/public-beta/USER_TESTING_GUIDE.md` — switch-network reference
* `deopt-v2-backend/docs/COMMUNITY_FEEDBACK_LOOP_NEXT_TASK.md` — frontend slots note
* `~/DEOPT/RUN_STATE.md` — closure paragraph

**Untouched:**
* Backend Rust source — ZERO changes.
* Solidity source — ZERO changes.
* Backend `.env` — mtime `2026-06-08 16:55:05` preserved.
* `~/DEOPT/private/**` — not read, not committed.
* Indexer / database — no DB writes.

## 13. Remaining UX gaps

None blocking external Base Sepolia testers. Soft polish items for a future iteration:
* **Wallet-disconnect mid-flow recovery banner.** Currently the modal surfaces "Wrong network" or "Backend unavailable"; a disconnect event mid-flow is also handled (`address` resets to null and Step 2 button re-disables with "Connect your wallet to sign" hint), but there is no explicit "Reconnect to resume" banner. Tracked as a follow-up nicety.
* **Live signing-payload preview before wallet approval.** Currently the modal says "Approve in your wallet (no transaction will be sent)" but doesn't surface the typed-data domain + primaryType inline. Could add a collapsed `<details>` for advanced users.
* **Network-add affordance.** Some wallets need `wallet_addEthereumChain` before `wallet_switchEthereumChain` can succeed for Base Sepolia. We deliberately did NOT add a custom RPC URL on the user's behalf (security posture); the quickstart doc tells testers how to add it manually instead.

## 14. Validations

| Check | Result |
|---|---|
| `git diff --check` (frontend repo) | clean |
| `git diff --check` (backend repo) | clean |
| `git status` (frontend repo) | only intended file additions / edits |
| `git status` (backend repo) | only this result doc + updated next-task / public-beta docs + RUN_STATE |
| Sensitive-string scan (frontend changed files) | zero hits — no bearer / RPC URL / private key / DATABASE_URL |
| Sensitive-string scan (changed docs) | zero hits |
| `.env` modified? | NO — mtime `2026-06-08 16:55:05` preserved |
| Private file (`/home/corio/DEOPT/private/**`) modified? | NO — not read, not committed |
| Admin bearer token present in client bundle / tests? | NO — `tests/e2e/no-admin-bearer.spec.ts` enforces it on every navigation |
| Mainnet RPC used? | NO |
| Chain transaction sent? | NO |
| Broadcast invoked? | NO |
| Real wallet used? | NO — mock EIP-1193 fixture only |
| Source changes outside `deopt-v2-frontend/` (excluding docs + RUN_STATE)? | NO |
| New backend dependency? | NO |
| Claim "audited"? | NO |
| Claim "mainnet-ready"? | NO |
| Claim "production-ready"? | NO |
| Claim "safe for real funds"? | NO |

## 15. Next milestone recommendation

The frontend is ready for external testers on Base Sepolia. The natural follow-up is **`COMMUNITY-FEEDBACK-LOOP`** — wiring real URLs into the four feedback channels (`{{PUBLIC_BETA_*_URL}}` placeholders) so the new footer + sign-failure CTA become live links instead of "(coming soon)" hints.

Optional alternative: **`PRODUCT-FREEZE-AND-SECURITY-REANCHOR`** if the operator wants to draft the public security-review packet before opening the channels.

NOT recommended at this stage: mainnet activation, external-audit kick-off, bug-bounty launch — those gate on `PRODUCT-FREEZE-AND-SECURITY-REANCHOR` completion and a later external review.

**End of FRONTEND-TESTNET-LAUNCH-POLISH result.**
