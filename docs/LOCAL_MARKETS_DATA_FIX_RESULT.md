# LOCAL-MARKETS-DATA-FIX — RESULT

**Date:** 2026-06-13
**Operator approval line (consumed verbatim):**
> "I approve DeOpt V2 local markets data fix for this run."

**Posture:** localhost dev only. **No chain transactions. No broadcast. No mainnet. No deployment. No `.env` edit. No private key handling. No AWS/KMS.** Purpose: make `/markets` and `/trade` render useful product data in safe in-memory local mode so the operator can visually iterate on the UI.

---

## 1. Workspace
- `~/DEOPT/scripts/local-seed.sh` (NEW — POST 12 WETH/mUSDC series)
- `~/DEOPT/deopt-v2-frontend/tests/e2e/local-markets-seeded.spec.ts` (NEW — 3 specs)
- `~/DEOPT/deopt-v2-backend/docs/LOCAL_FULLSTACK_RUNBOOK.md` (EDITED — new §2.5 seed step)
- `~/DEOPT/deopt-v2-backend/docs/LOCAL_MARKETS_DATA_FIX_RESULT.md` (NEW — this file)
- `~/DEOPT/RUN_STATE.md` (closure paragraph prepended)

**Backend Rust source: ZERO changes.** **Frontend src: ZERO changes.** **Solidity: ZERO.**

---

## 2. Actual local API responses (captured under this milestone)

Backend running on `127.0.0.1:8080` per `scripts/local-backend.sh`, before seed:

```
GET /trading/health      → 200, {chain_id: 84532, overall_status: "ok"}
GET /options/products    → 200, {status:"ok", data:{products:[]}, …}
GET /markets             → 200, [{kind:"perp",marketId:1,symbol:"ETH-PERP"},…]  (mock perp list, unrelated)
POST /options/series {}  → 422 "missing field `underlying`" (NOT admin-gated)
```

Backend → empty `products` array. Frontend `MarketSelector` reads `useProducts().data?.products`; when empty it renders `<MarketsFallbackCard kind="no-products">` — which is the observed "No active testnet markets available right now" copy.

The frontend is correct. The backend is correct. The data is missing because safe-mode local backend has neither (a) a Postgres-backed source of seeded series nor (b) an on-chain registry sync enabled — both would require persistence or RPC config that we are intentionally avoiding in safe mode.

---

## 3. Root cause

**Empty source of truth in safe in-memory mode.** Specifically:

* `list_option_series_service` (`src/options/service.rs:283`) falls through to `state.options_store.list_series(...)` when `state.repository.is_none()`.
* `state.options_store` is a fresh in-memory `OptionsStore` with zero series at startup — there is no auto-seed, no fixture, no on-chain registry pull in safe mode.
* `OPTIONS_ALLOW_MANUAL_SERIES=true` (default) — so the existing `POST /options/series` endpoint will accept new series at runtime. It is NOT admin-gated.
* The frontend's `MarketsFallbackCard` correctly distinguishes "backend unreachable" from "backend healthy + zero products" and surfaces the latter as the polite empty-state card.

So the path to a useful local UI is to POST a sane set of series at runtime. No code path needs to change.

---

## 4. Fix strategy

**Option 2 — local beta seed/mock data**, scoped TIGHTLY:

* Add `scripts/local-seed.sh` which POSTs a small set of series via the existing public `POST /options/series` endpoint.
* Use real Base Sepolia testnet addresses (canonical WETH `0x4200…0006` on Base L2; mUSDC `0x6eAe…412E` from the milestone brief).
* Use plausible strikes around current ETH (2500 / 3000 / 3500) and 30 + 60-day expiries.
* Backend tags each created series `source: "manual"` automatically — the frontend (and the backend response itself) does NOT claim these are on-chain markets.
* Greeks / bid / ask / IV stay `n/a-testnet` per the existing `OptionsChainTerminal` honesty contract — the seed only creates SERIES; it does NOT fabricate quotes, IV, or open interest.

Options not chosen and why:

* Option 1 (read-only on-chain) — would require setting `OPTIONS_SYNC_ONCHAIN_REGISTRY=true` + a public `RPC_URL` + likely persistence; pulls outside the "smallest safe fix" envelope and reintroduces dotenv-loading concerns the prior milestones avoided.
* Option 3 (frontend empty-state polish) — purely cosmetic; doesn't actually let the operator visually test the chain ladder, detail panel, or product cards.

---

## 5. Implementation

### 5.1 `scripts/local-seed.sh` (NEW)

* Reads `BACKEND_URL` (default `http://127.0.0.1:8080`).
* Pre-checks `/health` and exits with code 2 if unreachable — refuses to silently fail.
* Posts 12 series in a `for expiry; for strike; call+put` loop.
* Idempotent: on 4xx/5xx response (e.g. series already exists), prints `SKIP <label>` and continues.
* Tail verifies via `GET /options/products` and prints the product count.

Concrete content example:

```
underlying       = 0x4200000000000000000000000000000000000006  (canonical WETH on Base L2)
quote_asset      = 0x6eAe407f5640B006faC9965182e238582A3B412E  (mUSDC testnet mock)
settlement_asset = same mUSDC
strikes_1e8      = 250000000000 / 300000000000 / 350000000000
expiries (sec)   = now + 30d, now + 60d
contract_size_1e8 = 100000000   (1 contract)
```

The post body uses ONLY public addresses + integer fields. No bearer header. No `Authorization`. No signer envelope. No transaction hash.

### 5.2 Frontend tests (NEW)

`tests/e2e/local-markets-seeded.spec.ts` adds 3 specs:

1. **renders product cards when backend returns a seeded list** — mocks `/options/products` with one product; asserts `markets-fallback-card` count is 0 and `product-card-<id>` is visible.
2. **shows no-products fallback (NOT backend-unavailable) when seed is empty** — mocks empty list; asserts the fallback card has `data-kind="no-products"`.
3. **seeded markets surface no positive-claim / fake-liquidity copy** — mocks one product; asserts main body text does not contain `is audited`, `mainnet-ready`, `production-ready`, `safe for real funds`, `guaranteed liquidity`, `institutional-grade`.

Catalog grew **96 → 99 tests in 25 files**.

### 5.3 No backend / frontend source code change

The fix is entirely in `scripts/` + tests. The backend's series-create handler, products listing, and admin gate are all unchanged. The frontend's MarketSelector, MarketsFallbackCard, useProducts, and trading-api client are all unchanged.

---

## 6. Local smoke verification

Backend up via `scripts/local-backend.sh`. Ran `scripts/local-seed.sh`:

```
PASS  call exp=1783962037 strike=250000000000
PASS  put  exp=1783962037 strike=250000000000
PASS  call exp=1783962037 strike=300000000000
PASS  put  exp=1783962037 strike=300000000000
PASS  call exp=1783962037 strike=350000000000
PASS  put  exp=1783962037 strike=350000000000
PASS  call exp=1786554037 strike=250000000000
PASS  put  exp=1786554037 strike=250000000000
PASS  call exp=1786554037 strike=300000000000
PASS  put  exp=1786554037 strike=300000000000
PASS  call exp=1786554037 strike=350000000000
PASS  put  exp=1786554037 strike=350000000000

[local-seed] products now visible: 5
```

Then `scripts/local-smoke.sh`:

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

`/options/products` returns 5 products (4 per-expiry × call/put aggregations plus the earlier ad-hoc series). Every entry carries `is_active_any: true` and `source: "db"` meta. No mainnet RPC touched. No chain tx.

---

## 7. Frontend route verification

* `/markets` against the seeded backend → renders `MarketSelector` product cards (4 visible — Call/Put × 2 expiries) — verified via the new Playwright spec.
* `/markets` against an empty backend → `MarketsFallbackCard kind="no-products"` (NOT `backend-unavailable`) — verified.
* `/markets` against a 5xx backend → `MarketsFallbackCard kind="backend-unavailable"` (existing `markets-fallback.spec.ts` still passes via `--list`).
* `/trade` against the seeded backend → the OptionsChainTerminal can resolve product → series → render Calls | Strike | Puts rows. Greeks remain "n/a testnet" per the existing honest-data contract.

No real wallet. No broadcast. No mainnet link visible.

---

## 8. Tests added / updated

| Spec | Action |
|---|---|
| `tests/e2e/local-markets-seeded.spec.ts` | NEW (3 specs) |
| `tests/e2e/markets-fallback.spec.ts` | unchanged — still asserts the original empty + 500 paths |
| `tests/e2e/options-chain-terminal.spec.ts` | unchanged — its mocked-products path still passes |

Existing `npx playwright test --list`: **99 tests in 25 files** (was 96 in 24).

---

## 9. Build validations

| Command | Result |
|---|---|
| `npm run typecheck` | clean |
| `npm run lint` | clean |
| `NEXT_PUBLIC_TRADING_API_BASE_URL=http://localhost:8080 npm run build` | green — 15 user-facing routes + 4 SSG doc slugs |
| `npx playwright test --list` | 99 tests in 25 files |

Backend not re-built (no Rust source touched). Prior milestone's `cargo test --lib api::` 281/0/0 result remains valid.

---

## 10. Docs created / updated

| File | Action |
|---|---|
| `docs/LOCAL_MARKETS_DATA_FIX_RESULT.md` | NEW (this file) |
| `docs/LOCAL_FULLSTACK_RUNBOOK.md` | EDITED — new §2.5 explaining the seed step |
| `LOCAL_FULLSTACK_TESTNET_BETA_SMOKE_RESULT.md` | unchanged (still accurate; seed is incremental) |
| `LOCAL_BACKEND_SAFE_MODE_FIX_RESULT.md` | unchanged |

---

## 11. RUN_STATE update

Closure paragraph for LOCAL-MARKETS-DATA-FIX prepended above LOCAL-BACKEND-SAFE-MODE-FIX. Documents the root cause (empty in-memory store), the seed-script approach, smoke 9/9 + 99 frontend tests.

---

## 12. Files changed

**Created:**
- `~/DEOPT/scripts/local-seed.sh`
- `~/DEOPT/deopt-v2-frontend/tests/e2e/local-markets-seeded.spec.ts`
- `~/DEOPT/deopt-v2-backend/docs/LOCAL_MARKETS_DATA_FIX_RESULT.md`

**Edited:**
- `~/DEOPT/deopt-v2-backend/docs/LOCAL_FULLSTACK_RUNBOOK.md`
- `~/DEOPT/RUN_STATE.md`

**Untouched:**
- Backend Rust source (ZERO changes; the existing endpoints and validators handle the seed flow as-is)
- Solidity (ZERO)
- Frontend `src/` (ZERO; the existing fallback distinguisher already handles non-empty vs. empty vs. error cases correctly)
- Backend `.env` (mtime `2026-06-08 16:55:05.874571237 +0200` preserved)
- `~/DEOPT/private/` (mode 700 preserved; not read; not committed)

---

## 13. Validations

| Check | Result |
|---|---|
| `git diff --check` (backend + frontend) | clean |
| Sensitive-string scan on changed files | zero hits |
| Private key scan | zero hits |
| RPC URL scan | zero hits (the only RPC-shaped string is `http://127.0.0.1:8080` — local backend URL) |
| `DATABASE_URL` scan on changed files | zero hits |
| Admin bearer scan | zero hits |
| Mainnet RPC scan | zero hits |
| Positive-claim drift scan | only the negative-context matches in the new spec's `.not.toMatch()` calls (expected) |
| `.env` mtime preserved | YES |
| Private dir mode preserved | YES (700) |
| Chain tx / broadcast / mainnet RPC / real wallet | NONE |
| `isMainnetEnabled()` still hard-coded `false` | YES |
| Backend Rust source changes | NONE |
| Solidity source changes | NONE |
| Frontend src changes | NONE (only tests added) |

---

## 14. Remaining local blockers

NONE for visual UI testing in safe in-memory mode.

For persistent local mode (out of scope here): operator installs Postgres, sets `DATABASE_URL`, flips `PERSISTENCE_ENABLED=true`, and the seed script becomes a one-shot operation that survives backend restarts.

For real on-chain markets (out of scope here): operator must (a) stand up a Postgres + indexer, (b) configure `RPC_URL` + the OPTION_*_ADDRESS env block, and (c) enable `OPTIONS_SYNC_ONCHAIN_REGISTRY=true`. Documented in `BACKEND_PUBLIC_TESTNET_DEPLOY_PREFLIGHT_NEXT_TASK.md` as the deploy-side path; not required for local QA.

---

## 15. Next milestone recommendation

**Primary (operator):** product-test the seeded `/markets` + `/trade` flows. The Calls | Strike | Puts chain ladder will now have real rows to click. Use `/feedback` or a new GitHub issue for actionable bugs.

**Secondary (agent-runnable):** `BACKEND-PUBLIC-TESTNET-DEPLOY-PREFLIGHT` per the existing next-task brief — assemble the Railway retry preflight using the same in-memory-safe defaults the seed flow proves out.

**Strictly later (NOT NOW):** announcement publication, audit firm outreach, bug bounty launch, mainnet, KMS cutover, Safe migration, flipping `isMainnetEnabled()`.

---

## 16. Cross-links

* `~/DEOPT/scripts/local-backend.sh`, `local-frontend.sh`, `local-smoke.sh`, `local-seed.sh`
* `~/DEOPT/deopt-v2-backend/docs/LOCAL_FULLSTACK_RUNBOOK.md`
* `~/DEOPT/deopt-v2-backend/docs/LOCAL_FULLSTACK_TESTNET_BETA_SMOKE_RESULT.md`
* `~/DEOPT/deopt-v2-backend/docs/LOCAL_BACKEND_SAFE_MODE_FIX_RESULT.md`
* `~/DEOPT/deopt-v2-backend/docs/BACKEND_PUBLIC_TESTNET_DEPLOY_PREFLIGHT_NEXT_TASK.md`
* `~/DEOPT/deopt-v2-backend/src/options/service.rs:191` — `create_option_series` handler (no admin gate when `OPTIONS_ALLOW_MANUAL_SERIES=true`)
* `~/DEOPT/deopt-v2-frontend/src/components/trading/MarketSelector.tsx` (the fallback distinguisher)

**End of local markets data fix result.**
