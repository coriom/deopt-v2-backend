# MATCHING-TIF-SEMANTICS-OPTIONS-V1 — Result

Direct-orderbook Time-In-Force and post-only semantics for the
options matching engine.

## Scope

| In-scope | Out-of-scope |
|---|---|
| `POST /options/orders` (direct orderbook submit) | `POST /options/execution-intents` (paired RFQ-style) |
| Options in-memory matcher (`OptionSeriesStore`) | Perp matching (no orderbook engine exists for perps) |
| Options DB matcher (`Repository::submit_option_order_and_match`) | Solidity contracts, chain broadcast, mainnet, deploy |
| Service-level validation (`submit_option_order`) | Cancel-on-disconnect, GTD (good-till-date) |
| HTTP request/response shape | Margin/collateral mutation paths (unchanged) |

Discovery confirmed perps have no in-memory or DB orderbook —
execution flows through signed paired intents only. No code is shared
with the options matcher.

## Execution architecture discovered

Two parallel matching paths, selected by `state.repository` presence:

- **In-memory:** `OptionSeriesStore::submit_order_and_match`
  (`src/options/store.rs`). Mutex-guarded `HashMap<OrderId, OptionOrder>`.
  Single-threaded critical section per call.
- **DB:** `Repository::submit_option_order_and_match`
  (`src/db/repository.rs`). Postgres transaction wrapping `SELECT … FOR
  UPDATE` on the opposite side of the book, followed by per-leg
  `UPDATE option_orders` + `INSERT option_fills`. Commit-or-rollback.

Both paths now share the same plan-then-execute primitives exported
from `src/options/store.rs`:

- `build_option_match_plan(incoming, sorted_candidates) -> OptionMatchPlan`
- `enforce_tif_plan(order, plan) -> Result<()>`
- `final_status_for_tif(order) -> OptionOrderStatus`

The plan walks the price-time-priority-sorted maker snapshot without
any mutation; validation runs against the plan; execution applies
each leg only after validation passes. For the DB path the snapshot
is taken inside the FOR-UPDATE-locked transaction, so concurrent
changes cannot invalidate the pre-check.

## Semantics implemented

### GTC — Good Till Cancel
- Walks compatible resting orders in price-time priority.
- Fully filled → status `Filled`.
- Partially filled → status `PartiallyFilled`, remainder rests.
- Zero fill → status `Open`, full quantity rests.

### IOC — Immediate Or Cancel
- Executes the immediate match plan (zero or more legs).
- Inserts the taker with its **final** status pre-computed:
  - Full fill → `Filled`.
  - Partial fill → `Cancelled` (remainder is recorded in
    `remaining_size_1e8` for audit but the order is terminal — it
    never appears as `Open`/`PartiallyFilled` in the book).
  - Zero fill → `Cancelled` with `remaining = size`.

### FOK — Fill Or Kill
- Builds the full match plan first.
- If `plan.total_fill_size_1e8 < order.size_1e8` →
  `BackendError::FokNotFillable`. No insert, no leg execution, no
  fee event, no maker mutation.
- If sufficient → atomically inserts the taker (status `Filled`) and
  all leg fills inside the same critical section.

### Post-only
- Only valid as a limit order (options have no market variant —
  `price_1e8 == 0` is already rejected upstream).
- Marketability is checked via the same locked plan: if
  `plan.legs.is_empty()` is false, returns
  `BackendError::PostOnlyWouldMatch`. No insert.
- Non-marketable post-only orders rest normally as makers (status
  `Open`, `post_only = true` persisted).

### Invalid combinations
Service-layer guard `validate_tif_combination(tif, post_only)` runs
before any matching:
- `post_only + IOC` → `BackendError::InvalidTimeInForceCombination("post-only is not compatible with IOC")`
- `post_only + FOK` → `BackendError::InvalidTimeInForceCombination("post-only is not compatible with FOK")`
- Malformed TIF strings are rejected at the JSON deserialization
  boundary by serde (`#[serde(rename_all = "snake_case")]`).
- `post_only` + market is N/A — options have no market orders.

## API behaviour

Endpoint: `POST /options/orders`.

Request additions (request struct
`SubmitOptionOrderRequest` in `src/api/routes.rs`):
- `time_in_force: "gtc" | "ioc" | "fok"` (existing, now honoured).
- `post_only: bool` (defaults to false via `#[serde(default)]`).

Response shape (`SubmitOptionOrderResponse`):
- Flattened `OptionOrderResponse`:
  - `status`: `open | partially_filled | filled | cancelled | rejected | expired`
  - `size_1e8`, `remaining_size_1e8` — request consumers compute
    `filled = size - remaining`.
  - `time_in_force`, `post_only` echoed.
- `fills: [OptionFillResponse]` — one entry per executed leg.

Error responses (all `400 Bad Request`, body `{"error": <message>}`):

| Stable English message | BackendError variant |
|---|---|
| `fill-or-kill order is not fully fillable` | `FokNotFillable` |
| `post-only order would immediately match` | `PostOnlyWouldMatch` |
| `invalid time-in-force combination: post-only is not compatible with IOC` | `InvalidTimeInForceCombination(_)` |
| `invalid time-in-force combination: post-only is not compatible with FOK` | `InvalidTimeInForceCombination(_)` |
| `zero price is not allowed` | `ZeroPrice` |
| `zero size is not allowed` | `ZeroSize` |
| `deadline has expired` | `DeadlineExpired` |

The repository continues to use string-based error bodies (existing
project convention). Stability is at the message level. The variants
above are also typed in `src/error.rs` for in-process matching.

## Status model

No new variants added to `OptionOrderStatus`. The existing model
expresses every outcome unambiguously:

| Outcome | status | `remaining_size_1e8` | Fills |
|---|---|---|---|
| GTC zero fill | `Open` | size | `[]` |
| GTC partial fill | `PartiallyFilled` | size - filled | non-empty |
| GTC full fill | `Filled` | 0 | non-empty |
| IOC zero fill | `Cancelled` | size | `[]` |
| IOC partial fill | `Cancelled` | size - filled | non-empty |
| IOC full fill | `Filled` | 0 | non-empty |
| FOK full fill | `Filled` | 0 | non-empty |
| FOK rejected | n/a (error 400, no row) | n/a | n/a |
| Post-only resting | `Open` | size | `[]` |
| Post-only rejected | n/a (error 400, no row) | n/a | n/a |
| User-cancelled | `Cancelled` | unchanged | n/a |
| Expired | `Expired` | unchanged | n/a |

`cancel_reason` was intentionally **not** added — the combination
(status, fills, error response) already disambiguates every outcome
without a schema migration.

## Persistence + atomicity

**In-memory store:** the `&mut self` borrow is the critical section.
The plan and execution share the same borrow → no concurrency.

**DB matcher:** restructured to a single transaction:
1. `BEGIN`
2. `SELECT … WHERE side = opposite … FOR UPDATE`
3. Build plan from locked snapshot.
4. `enforce_tif_plan(plan)` → on error, `Result<Err>` causes the
   tx-guard drop to roll back. No `INSERT`s have happened yet.
5. `INSERT option_orders` (taker, with final status pre-computed).
6. Per leg: `UPDATE option_orders` (maker decrement) + `INSERT
   option_fills`.
7. `COMMIT`.

Concurrency: the FOR UPDATE lock on opposing-side rows blocks
another concurrent taker on the same side from racing past the plan
check. New maker inserts on the opposite side after the SELECT are
not seen by this transaction — they will only be visible to the next
taker, which is the desired snapshot semantics.

## Frontend behaviour

- `src/components/trading/TradeTicket.tsx` (options): TIF popover +
  Post checkbox kept visible (UX consistency) but **not sent** in the
  `createExecutionIntent` payload. A footnote
  (`data-testid="trade-tif-scope-note"`) documents that these
  controls apply to `POST /options/orders`, and that the paired
  execution-intent flow below always behaves as GTC and ignores
  post-only. The widget never claims TIF/Post are active for the
  intent flow.
- `src/components/trading/perps/PerpsTradeForm.tsx` (perps): TIF +
  Post UI is rendered but the whole form's submit button is
  permanently disabled (`Perps not live`). Explicit availability
  state — no claim of liveness.
- `src/components/trading/TifPopover.tsx`: reusable Derive-style
  popover (GTC/IOC/FOK descriptions + checkmark). Exports
  `TifPopover` and `PostCheckbox`.
- No direct-orderbook submit UI exists yet. When it ships, it can
  consume `TifPopover` / `PostCheckbox` directly and post to
  `/options/orders` with the new fields.

## Files changed

Backend:
- `src/error.rs` — added `FokNotFillable`, `InvalidTimeInForceCombination(String)`.
- `src/options/types.rs` — added `OptionOrder.post_only: bool` (`#[serde(default)]`).
- `src/options/service.rs` — `SubmitOptionOrderInput.post_only`,
  `validate_tif_combination()`, removed the GTC-only rejection,
  propagated `post_only` into the constructed `OptionOrder`.
- `src/options/store.rs` — new pub(crate) `OptionMatchLeg`,
  `OptionMatchPlan`, `build_option_match_plan`, `enforce_tif_plan`,
  `final_status_for_tif`. `submit_order_and_match` refactored to
  plan-then-execute.
- `src/db/repository.rs` — `submit_option_order_and_match`
  refactored to plan-then-execute inside the existing transaction,
  with the taker insert deferred until after `enforce_tif_plan`.
  Insert SQL extended with `post_only` column; six SELECTs extended;
  `option_order_from_row` reads the column; binder writes it.
- `src/api/routes.rs` — `SubmitOptionOrderRequest.post_only`
  (`#[serde(default)]`), `OptionOrderResponse.post_only`, handler
  propagation, mapping of the two new error variants to
  `400 Bad Request`.
- `migrations/0027_option_orders_post_only.sql` — `ALTER TABLE …
  ADD COLUMN post_only BOOLEAN NOT NULL DEFAULT FALSE`.
- `tests/options_tests.rs` — updated the IOC test that asserted
  rejection (now asserts cancellation) and added 11 new tests
  covering GTC partial / IOC partial / IOC multi-level / FOK
  multi-level / FOK price-limit / post-only non-crossing rest /
  post-only crossing buy + sell rejection / post-only combo
  rejection (IOC, FOK) / FOK insufficient-liquidity rejection / IOC
  zero-fill cancellation / quantity invariant.

Frontend:
- `src/components/trading/TifPopover.tsx` — new shared component.
- `src/components/trading/TradeTicket.tsx` — TIF/Post row + scope
  footnote.
- `src/components/trading/perps/PerpsTradeForm.tsx` — TIF/Post row.

## Validations executed

Backend (`~/DEOPT/deopt-v2-backend`):
- `cargo check --lib` → clean.
- `cargo test --lib` → 1005 passed, 0 failed.
- `cargo test --test options_tests` → 88 passed, 0 failed (13 new
  tests, all green).

Frontend (`~/DEOPT/deopt-v2-frontend`):
- `npm run lint` (eslint) → clean.
- `npx tsc --noEmit` → clean.

Safety posture:
- No secrets read or printed.
- No `.env` values surfaced.
- No private keys, RPC URLs, DB URLs, or admin bearer tokens read
  or echoed.
- No chain transaction signed or broadcast.
- No deployment.
- No mainnet enabling.
- No Solidity touched.

## Known limitations

- The `cancel_reason` field is not persisted. Consumers infer the
  reason from `status` + the originating error response (e.g.
  `Cancelled` with `remaining > 0` and `time_in_force == ioc` ⇒
  IOC-remainder cancel). If a future caller needs explicit reasons
  on the order row, add a `cancel_reason text NULL` column in a
  follow-up migration.
- No lifecycle events are emitted for IOC remainder cancellation or
  post-only rejection (no event bus exists for option orders yet).
  Outcomes are observable via the immediate API response and via
  subsequent `GET /options/orders`.
- The direct-orderbook UI does not yet exist on the frontend. The
  new TIF/Post controls in `TradeTicket.tsx` flow into local state
  only; they are accompanied by an in-UI note clarifying scope.
- Perps remain orderbook-less. IOC/FOK/post-only do not apply to
  perp execution and are not implemented there.
- Self-trade exclusion is enforced upstream (validation) but is not
  re-checked inside the matcher critical section. Out of scope for
  this milestone; tracked separately.

## Next recommendation

Build a direct-orderbook submit UI (or wire the existing
`TradeTicket.tsx` TIF/Post controls into a new `POST /options/orders`
client) so the operator can exercise IOC/FOK/post-only end-to-end
through the UI. Until then, the new semantics are only reachable via
direct HTTP / `cargo test`.

---

## Addendum — direct-orderbook frontend wiring (same milestone)

Following the milestone close, the operator requested an honest
end-to-end browser path. This addendum documents precisely what was
**already present** before this work and what was **added now**, so
the audit trail stays unambiguous.

### Already present (no changes)

- `src/components/trading/TifPopover.tsx` — reusable popover +
  `PostCheckbox`. Drives only local React state in `TradeTicket.tsx`
  and `PerpsTradeForm.tsx`. Untouched.
- `src/components/trading/TradeTicket.tsx` — paired RFQ-style trade
  ticket. Carries the TIF/Post controls in local state plus the
  scope footnote that explicitly states the controls apply to
  `POST /options/orders` and that the paired execution-intent flow
  below always behaves as GTC and ignores post-only. Untouched.
- `src/components/trading/perps/PerpsTradeForm.tsx` — perps form
  with permanently-disabled submit (`Perps not live`). Untouched.
- `src/lib/trading-api.ts` — the `rawRequest` helper + the
  `TradingApiError` envelope. Existing endpoints: GET/POST against
  `/options/products`, `/options/execution-intents`, etc. **No
  client function for `POST /options/orders` existed.**
- `src/lib/trading-types.ts` — types for products, series,
  execution intents, signatures. **No `OptionOrderResponse`,
  `SubmitOptionOrderRequest`, or related types existed.**
- `src/app/(trading)/api/sandbox/page.tsx` — existing WebSocket
  sandbox. Untouched (template for the new orderbook sandbox).

### Added in this addendum

Frontend:

| File | Status | Purpose |
|---|---|---|
| `src/lib/trading-types.ts` | extended | Added `OptionOrderTif`, `OptionOrderStatusValue`, `OptionOrderSide`, `SubmitOptionOrderRequest`, `OptionFillResponse`, `OptionOrderResponse`, `SubmitOptionOrderResponse`. Snake-case wire format mirrors the backend serde shape. |
| `src/lib/trading-api.ts` | extended | Added `submitOptionOrder(body, signal?)` — wraps `rawRequest("POST", "/options/orders", body)` and rethrows backend 400 messages through `TradingApiError`. |
| `src/components/trading/DirectOrderbookForm.tsx` | new | Dedicated direct-orderbook submit form. Fields: series id, account, side, price_1e8, size_1e8, TIF popover, post-only checkbox. Renders final order status / remaining / fill legs on success, and the stable backend error message on 400. Carries an inline scope notice that distinguishes it from `TradeTicket.tsx`. |
| `src/app/(trading)/api/orderbook-sandbox/page.tsx` | new | `/api/orderbook-sandbox` sandbox page hosting the form. Header explains GTC/IOC/FOK/post-only and links back to the regular `/options` page for the paired RFQ flow. |
| `src/components/api/DevelopersConsole.tsx` | extended | Added `developers-console-orderbook-sandbox-link` so the new sandbox is reachable from `/api`. |
| `tests/e2e/orderbook-sandbox.spec.ts` | new | Playwright spec mocking `POST /options/orders` via `page.route`. Five tests: GTC resting (`open`), IOC remainder cancelled (`cancelled` + fills), FOK rejected (400 + stable message), post-only crossing rejected (400 + stable message), sandbox link visible from `/api`. |
| `tests/node/orderbook-client.contract.mjs` | new | Pure-Node validation (no Playwright, no TS runner). Spins up an in-process `node:http` mock and exercises the wire contract for GTC / IOC / FOK / post-only / invalid TIF combo. Runs via `npm run test:node`. |
| `package.json` | extended | New `"test:node"` script: `node --test tests/node/*.mjs`. |

Backend: **no further changes** in this addendum. The matching
engine, error variants, migration, and API request/response shape
shipped earlier in the milestone are unchanged.

### Validations executed (addendum)

- `npm run lint` → clean.
- `npx tsc --noEmit` → clean.
- `npm run test:node` → **5/5 passed** in ~150 ms.
  - GTC resting: request carries `time_in_force: "gtc"` +
    `post_only: false`; response `status: "open"`; no fills.
  - IOC partial fill: request carries `time_in_force: "ioc"`;
    response `status: "cancelled"` with `remaining > 0` and a
    non-empty fills array; quantity invariant
    `filled + remaining == requested` holds.
  - FOK rejection: 400 body
    `{"error": "fill-or-kill order is not fully fillable"}`
    surfaces unchanged via `TradingApiError.message`.
  - Post-only marketable rejection: 400 body
    `{"error": "post-only order would immediately match"}`
    surfaces unchanged.
  - Invalid TIF combo: 400 body
    `{"error": "invalid time-in-force combination: …"}` surfaces
    unchanged.
- `tests/e2e/orderbook-sandbox.spec.ts` is committed but **not
  executed in this environment** — the local sandbox is missing
  the system library `libnspr4.so` required by Chromium, and
  `npx playwright install --with-deps chromium` requires `sudo`.
  The spec is syntactically valid (lint + typecheck clean), follows
  the existing project patterns (`page.route` mocking, no real
  network, no real wallet), and is intended to run in any
  environment where `npm run e2e:install` succeeds.

### Posture (unchanged)

- No Solidity touched.
- No chain transaction signed or broadcast.
- No deployment.
- No mainnet enabling.
- No `.env`, RPC URLs, DB URLs, private keys, or admin bearer
  tokens read or echoed.

### How to run

```sh
# Backend matching semantics (88 tests)
cd ~/DEOPT/deopt-v2-backend
cargo test --test options_tests

# Frontend wire-contract validation (5 tests, no browser deps)
cd ~/DEOPT/deopt-v2-frontend
npm run test:node

# Browser-level validation (requires Playwright deps)
npm run e2e:install      # one-time chromium + system deps install
npm run e2e:local -- tests/e2e/orderbook-sandbox.spec.ts
```

### Operator path

Visit `/api/orderbook-sandbox` (linked from `/api`), enter a real
series id from `/options/series`, pick GTC/IOC/FOK + optional
post-only, submit. The form displays the final order state and any
backend rejection message. Distinct from the `/options` trade
ticket (which still drives the paired RFQ execution-intent flow).
