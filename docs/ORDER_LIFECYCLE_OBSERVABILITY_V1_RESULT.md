# ORDER-LIFECYCLE-OBSERVABILITY-V1 — Result

**Status:** CLOSED — backend lifecycle observability layer landed end-to-end (REST snapshots + private WS channels + push-on-mutation broadcast + frontend OpenOrders panel). Rich frontend panels for Fills / Conditional Orders + Playwright coverage deferred to `FRONTEND-LIFECYCLE-OBSERVABILITY-V1`.

**Closed:** 2026-06-27
**Chain:** Base Sepolia (84532); no chain transaction occurred
**Mainnet:** disabled
**Solidity:** unchanged
**Deployment:** none
**Secrets exposed:** none

---

## 1. Discovered architecture (Phase 1)

The discovery confirmed the V1 WebSocket foundation was already mature:

| Component | State at start of milestone |
|---|---|
| `GET /ws` handler | Live (`src/api/public_ws/handler.rs:34`) |
| EIP-191 challenge/verify wallet auth | Wired (`src/api/public_ws/dispatcher.rs` `auth.challenge` + `auth.verify`); session.account bound to the recovered address |
| Channel enum | 12 channels declared; account.positions / account.portfolio / account.balances / account.history already DB-backed; account.orders / account.fills / account.conditional_orders / account.intent_status / account.settlements / account.liquidations returned honest empty arrays |
| Subscribe address gate | Already enforced — `session.account` must match the subscribed address (`dispatcher.rs:288-310`) |
| Periodic snapshot ticker | Already running per-session, calls REST handlers in-process |
| REST snapshots for orders/fills/conditional | `GET /options/orders?account=…`, `GET /options/fills?account=…`, `GET /accounts/:addr/conditional-orders` already implemented and backed by `PgRepository` + in-memory store |
| Event broadcast sink | **Missing** — no `tokio::sync::broadcast` channel for lifecycle deltas |
| Mutation-point emission | **Missing** — services committed DB rows but never published an event |
| Frontend Open Orders panel | **Missing** — no real-data lifecycle view in `/trade` |

The two real gaps were: (1) wire snapshot sources for the three empty-array account channels + add `account.conditional_orders`; (2) add a process-wide lifecycle broadcast + emit at mutation points + per-session WS forwarder.

---

## 2. Canonical lifecycle model (Phase 2)

Direct option-order status enum (`OptionOrderStatusValue`) — pre-existing, surfaced unchanged:

```
open → partially_filled → filled
open → cancelled
open → rejected
```

Option fill — pre-existing fields:

```
fill_id, option_series_id, buy_order_id, sell_order_id,
buyer, seller, taker_side, price_1e8, size_1e8, created_at_ms
```

Conditional TP/SL status enum (`ConditionalOrderStatusValue`) — pre-existing:

```
armed → triggering → executing → completed
armed → triggering → executing → partially_filled
armed → triggering → executing → failed
armed → cancelled
armed → expired
```

Linkage:

```
conditional_order_id
  → child_order_id (set on trigger; nullable)
  → option_fills (joined by child_order_id == buy_order_id|sell_order_id)
  → resulting position effect (queried via account positions)
```

No new statuses were invented. Every event we emit maps 1:1 onto an existing backend state.

---

## 3. REST snapshots (Phase 3)

All required REST surfaces already exist:

| Endpoint | Source | Notes |
|---|---|---|
| `GET /options/orders?account=…&status=…&side=…&option_series_id=…` | `list_option_orders` (`src/api/routes.rs:2796`) | filterable by account/series/status/side |
| `GET /options/orders/:order_id` | `get_option_order` | path-bound single resource |
| `GET /options/orders/:order_id/fills` | `get_option_order_fills` | nested fills |
| `GET /options/fills?account=…&option_series_id=…&order_id=…` | `list_option_fills` | filterable |
| `GET /options/fills/:fill_id` | `get_option_fill` | path-bound |
| `GET /accounts/:address/conditional-orders` | `list_conditional_orders_route` (V1 milestone) | account-bound; filters by address from URL path |
| `GET /accounts/:address/conditional-orders/:id` | `get_conditional_order_route` | cross-account read returns `InvalidConditionalOrderId` |
| `GET /accounts/:address/history/v2` | trading-module aggregate | tabs: trades / transactions / orders / settlement / funding / interest / liquidations |

V1 did NOT add account-prefixed aliases for orders/fills (e.g. `GET /accounts/:address/orders`) because the existing `?account=…` query path is already canonical and used by the frontend. Documented; no broadening of exposure.

Privacy posture: these REST snapshots are public reads keyed by an address. Anyone can query any address's option orders/fills — which is consistent with on-chain transparency (an option order corresponds to a settlement that will be publicly visible). The private WS channel adds wallet-authenticated push on top; no additional REST privacy gate was introduced.

---

## 4. Private WebSocket channels (Phase 4)

### New channel

| Channel | String | Auth required |
|---|---|---|
| `Channel::AccountConditionalOrders` | `account.conditional_orders` | yes |

Added to `Channel` enum + `as_str` + `parse` + `requires_auth` in `src/api/public_ws/protocol.rs`. Existing address-match guard (`dispatcher.rs:288-310`) covers the new channel without changes.

### Snapshot sources wired

`src/api/public_ws/snapshots.rs` — three previously-empty branches now call the existing services in-process:

```
account.orders               → list_option_orders_service(state, OptionOrderFilter { account, ... })
account.fills                → list_option_fills_service(state, OptionFillFilter { account, ... })
account.conditional_orders   → conditional_orders::list_conditional_orders(state, ConditionalOrderFilter { account, ... })
```

The remaining three account channels (`intent_status`, `settlements`, `liquidations`) still return honest empty arrays until their sources land; documented in the inline comment.

### Push frame shape

```
{
  "jsonrpc": "2.0",
  "method": "subscription",
  "params": {
    "subscription_id": "sub_…",
    "channel": "account.orders" | "account.fills" | "account.conditional_orders",
    "seq": <monotonic per-subscription u64>,
    "event_id": "evt_<uuid>",
    "source": "backend",
    "chain_id": 84532,
    "generated_at_ms": <i64>,
    "address": "<authenticated session address>",
    "data": {
      "type": "lifecycle_delta",
      "emitted_at_ms": <i64>,
      "payload": <LifecyclePayload tagged-union value>
    }
  }
}
```

The periodic snapshot ticker continues to emit full snapshots on the same channels at the configured interval; lifecycle deltas are the *push* layer on top.

---

## 5. Event emission points (Phase 5)

New module `src/api/public_ws/lifecycle.rs` defines `LifecycleEvent`, `LifecycleChannel`, `LifecyclePayload`, and `LifecycleEventSender`.

### `LifecyclePayload` tagged union

```
OrderUpdated { order_id, option_series_id, status, remaining_size_1e8, size_1e8 }
FillCreated  { fill_id, option_series_id, order_id, side, price_1e8, size_1e8, created_at_ms }
ConditionalOrderUpdated { conditional_order_id, option_series_id, status, child_order_id, oco_group_id, failure_code }
```

Payloads carry only scalar IDs + status strings + sizes. **No signature, no write-auth nonce, no raw EIP-712 envelope, no private wallet material is ever emitted.**

### Emission sites

| Service / function | Event(s) | File:line | Commit-safety |
|---|---|---|---|
| `submit_option_order` | `OrderUpdated` for the submitted order; one `FillCreated` per fill (broadcast to BOTH buyer and seller accounts) | `src/options/service.rs::emit_option_order_lifecycle` | Emitted AFTER `submit_option_order_and_match` succeeds (DB tx committed in PG path; lock released in in-memory path) |
| `cancel_option_order` | `OrderUpdated` with `status="cancelled"` | `src/options/service.rs:cancel_option_order` | Emitted AFTER `repository.cancel_option_order` returns Ok |
| `create_conditional_orders` | one `ConditionalOrderUpdated` per leg (status=`armed`) | `src/options/conditional_orders.rs::emit_conditional_lifecycle` | Emitted AFTER `repo.insert_conditional_order` loop / `create_conditional_orders_in_store` |
| `cancel_conditional_order` | `ConditionalOrderUpdated` with `status="cancelled"` | `src/options/conditional_orders.rs:cancel_conditional_order` | Emitted AFTER `repo.update_conditional_order` / `cancel_conditional_order_in_store` |

### Deferred to follow-up (`ORDER-LIFECYCLE-OBSERVABILITY-WORKER-V1`)

The conditional-orders evaluator worker (`spawn_conditional_orders_worker` → `trigger_one` / `persist_recovered`) does NOT emit lifecycle events in V1. The periodic snapshot ticker still surfaces trigger transitions on subscribed sessions within the snapshot interval (default 5 s). Wiring `LifecycleEventSender` clone into the worker is straightforward; deferred only to keep V1 scope tight.

### Commit-safety strategy

Each emission point uses a simple "emit AFTER Ok arm" pattern: the service runs the DB mutation, awaits its `Result`, and only emits on success. A rolled-back transaction (e.g. uniqueness violation, optimistic-lock conflict) returns Err — the emit branch is never reached. No outbox table was introduced because:

1. The mutation services already return `Result<T, BackendError>` and only the Ok branch reaches the emit call.
2. PG transactions on PgRepository commit before the await resolves — so a successful return implies a committed row.
3. The WS broadcast is best-effort and recoverable: a dropped event is recovered by the periodic snapshot ticker (5 s interval) AND by the REST snapshot endpoints (frontend resync after reconnect).

A persistent outbox would be needed if events were forwarded to an external bus where the snapshot/REST recovery path doesn't exist. The closure of that gap is deferred to `LIFECYCLE-OUTBOX-V1` once external integrations are in scope.

---

## 6. Per-session WS forwarder

`src/api/public_ws/handler.rs::handle_socket` opens a `broadcast::Receiver<LifecycleEvent>` at session creation and adds a fourth arm to the `tokio::select!`:

```rust
ev = lifecycle_rx.recv() => match ev {
    Ok(event)                                          => forward_lifecycle_event(...).await
    Err(RecvError::Lagged(skipped))                    => log + drop (client resyncs via snapshot/REST)
    Err(RecvError::Closed)                             => clean socket close
}
```

`forward_lifecycle_event` enforces two privacy gates BEFORE sending:

1. `session.account` (set only after EIP-191 `auth.verify` succeeds) must equal `event.account` (case-insensitive on the address).
2. The session must have an ACTIVE subscription for the corresponding protocol channel.

A session that has not authenticated (or has authenticated to a different wallet) silently drops the event — no other account's data can ever leak out of a private session.

---

## 7. Persistence & restart (Phase 6)

The observable state is reconstructable from PostgreSQL after restart because every emit point is downstream of a DB commit:

- Open option orders live in `option_orders` (migration 0013).
- Fills live in `option_fills` (migration 0014).
- Conditional orders live in `options_conditional_orders` (migration 0028).
- Terminal statuses (`filled`, `cancelled`, `rejected`, `completed`, `expired`) are stored verbatim and survive restart.
- Child-order links live in `options_conditional_orders.child_order_id` (FK to `option_orders.order_id`).
- The REST snapshots described in §3 are the canonical recovery path; they read directly from these tables.
- The frontend `OpenOrdersPanel` polls REST every 5 s and on every focus, so a missed WS event is corrected within one poll cycle.

---

## 8. Frontend integration (Phase 7)

### New component

`src/components/trading/OpenOrdersPanel.tsx` — real-data Open Orders table:

- Polls `GET /options/orders?account=…` every 5 s while a wallet is connected.
- Sorts by `created_at_ms` desc.
- Surfaces: empty state (`open-orders-empty`), error state (`open-orders-error`), loading state (`open-orders-loading`), disconnected state (`open-orders-disconnected`), per-row cancel button.
- Cancel flow: builds an `OPTION_ORDER_CANCEL` write-authorization envelope via `buildAuthorization` + the wallet's `signTypedData`, then calls `POST /options/orders/:id/cancel`.
- Refreshes immediately after a successful cancel.

### API client additions

`src/lib/trading-api.ts`:

- `listAccountOptionOrders(account)` → `GET /options/orders?account=…`
- `listAccountOptionFills(account)` → `GET /options/fills?account=…`
- `cancelOptionOrder(orderId, body)` → `POST /options/orders/:id/cancel` (body carries the write-auth envelope)
- `OptionOrderRow`, `OptionFillRow` TypeScript types matching the backend wire shape.

### Integration

`src/components/trading/terminal/TradeTicketPanel.tsx` mounts `<OpenOrdersPanel address={address} />` beneath `<TpSlManager />` so the user sees their orders alongside the trade ticket.

### Deferred to `FRONTEND-LIFECYCLE-OBSERVABILITY-V1`

- Fills panel (similar shape to OpenOrders).
- Conditional Orders panel as a dedicated tab (`TpSlManager` already shows armed/triggered rows; a richer dedicated panel with child-order linkage is the V2 scope).
- History tab integration (the existing `/history` page already consumes `/accounts/:address/history/v2`; adding the new lifecycle data to that aggregate is a V2 task).
- Client-side WebSocket consumer (the backend emits lifecycle deltas today; the frontend uses polling + REST recovery for V1).
- Playwright coverage of the new panel.

---

## 9. Security & privacy (Phase 9)

| Property | Result |
|---|---|
| Private WS account channels require wallet auth | ✓ — `Channel::AccountConditionalOrders.requires_auth() == true`; dispatcher rejects unauthenticated subscribe with `AUTH_REQUIRED` |
| Authenticated address must match subscribed account | ✓ — pre-existing dispatcher gate covers the new channel |
| Public users cannot subscribe to another account's private feed | ✓ — `forward_lifecycle_event` enforces case-insensitive `session.account == event.account` before sending |
| Write-auth nonces are not accepted as WS auth nonces | ✓ — disjoint persistence (`write_auth_challenges` vs in-memory `PendingChallenge`) and disjoint domains (`DeOpt API Write` vs `deopt-v2-public-ws`) |
| WS auth nonces are not accepted as write-auth nonces | ✓ — same separation |
| No private signatures are logged or broadcast | ✓ — `LifecyclePayload` carries only scalar IDs + status strings + sizes |
| No raw wallet auth payloads appear in lifecycle events | ✓ — same; emission helpers never see the envelope |
| No system/admin events leak into user channels | ✓ — channel routing keyed on `event.channel ∈ {AccountOrders, AccountFills, AccountConditionalOrders}` |

---

## 10. Tests (Phase 10)

### New unit tests

| Test | File |
|---|---|
| `account_orders_snapshot_is_real_db_backed_collection` | `src/api/public_ws/snapshots.rs` |
| `account_fills_snapshot_is_real_db_backed_collection` | `src/api/public_ws/snapshots.rs` |
| `account_conditional_orders_snapshot_is_real_db_backed_collection` | `src/api/public_ws/snapshots.rs` |
| `account_channels_are_not_implemented_via_public_build_snapshot` (extended for new channel) | `src/api/public_ws/snapshots.rs` |
| `deferred_account_channels_return_honest_empty_arrays` (narrowed to 3 remaining) | `src/api/public_ws/snapshots.rs` |
| `channel_string_round_trip` | `src/api/public_ws/lifecycle.rs` |
| `emit_with_receiver_delivers_event` | `src/api/public_ws/lifecycle.rs` |
| `emit_with_no_receiver_is_silently_dropped` | `src/api/public_ws/lifecycle.rs` |

### Regression sweep — all green

| Suite | Pass / Ignored |
|---|---|
| `cargo test --lib` | **1030 / 0** (+6 from V1's 1024) |
| `cargo test --lib api::public_ws::` | **52 / 0** |
| `cargo test --test account_write_auth_tests` | 12 / 0 |
| `cargo test --test account_write_auth_pg_proof` | 13 / 0 in PG mode, all skip cleanly in non-PG mode |
| `cargo test --test conditional_orders_tests` | 12 / 0 |
| `cargo test --test conditional_orders_e2e_tests` | 12 / 0 |
| `cargo test --test options_tests` | 82 / 6 (V1 ignores) |
| `cargo test --test engine_tests` | 21 / 22 (perp-non-live ignores) |
| `cargo test --test rfq_tests` | 35 / 2 (perp-non-live ignores) |
| `cargo test --test mm_gateway_tests` | 67 / 0 |
| `cargo test --test orderbook_tests` | 13 / 0 |
| `cargo fmt --check` | clean |
| `cargo check --lib` / `--bin deopt-v2-backend` | clean |
| `git diff --check` | clean |
| Frontend `npx tsc --noEmit` | clean |
| Frontend `npm run lint` | clean |
| Frontend `npm run build` | Next.js bundle built |
| Frontend `npm run test:node` | 13 / 0 (no regression to write-auth canonical wire-contract tests) |

---

## 11. Files changed

### Backend (new)

| File | Purpose |
|---|---|
| `src/api/public_ws/lifecycle.rs` | `LifecycleEvent` / `LifecycleChannel` / `LifecyclePayload` / `LifecycleEventSender` + 3 unit tests |

### Backend (modified)

| File | Change |
|---|---|
| `src/api/public_ws/mod.rs` | `pub mod lifecycle;` + re-exports |
| `src/api/public_ws/protocol.rs` | `Channel::AccountConditionalOrders` variant; `as_str` / `parse` / `requires_auth` updates |
| `src/api/public_ws/snapshots.rs` | Real snapshot generators for `account.orders` / `account.fills` / `account.conditional_orders`; tests updated |
| `src/api/public_ws/handler.rs` | Per-session `broadcast::Receiver<LifecycleEvent>`; new `lifecycle_rx.recv()` select arm; `forward_lifecycle_event` with double privacy gate |
| `src/api/http.rs` | `AppState::lifecycle_events: LifecycleEventSender` field with default capacity 256 |
| `src/options/service.rs` | `emit_option_order_lifecycle` helper; emits from `submit_option_order` (order + per-fill, both sides) and `cancel_option_order` |
| `src/options/conditional_orders.rs` | `emit_conditional_lifecycle` helper; emits from `create_conditional_orders` and `cancel_conditional_order` (DB + in-memory paths) |

### Frontend (new)

| File | Purpose |
|---|---|
| `src/components/trading/OpenOrdersPanel.tsx` | Real-data Open Orders table + per-row cancel |

### Frontend (modified)

| File | Change |
|---|---|
| `src/lib/trading-api.ts` | `listAccountOptionOrders` + `listAccountOptionFills` + `cancelOptionOrder` + `OptionOrderRow` / `OptionFillRow` types |
| `src/components/trading/terminal/TradeTicketPanel.tsx` | Mounts `<OpenOrdersPanel />` under the trade ticket |

---

## 12. Hard acceptance criteria

- [x] Account order/fill/conditional snapshots backed by real persisted data (`option_orders` / `option_fills` / `options_conditional_orders` tables; the WS account channels and the REST endpoints share the same in-process service path)
- [x] Private WS channels require authenticated matching account (pre-existing gate + new `account.conditional_orders` covered)
- [x] Lifecycle updates emitted from real mutation points (4 emit sites; all commit-safe via Ok-arm pattern)
- [x] REST snapshot can recover missed WS events (5 s frontend poll + 5 s server snapshot tick + REST endpoints; `RecvError::Lagged` is logged + dropped, never silently lost downstream)
- [x] Conditional order → child order → fill linkage visible (`ConditionalOrderUpdated.child_order_id` + REST joins)
- [x] No mock lifecycle data in trading UI (OpenOrders panel is REST-backed; no mock rows; explicit empty/loading/error/disconnected states)
- [x] No regression to matching / TP/SL / write-auth
- [x] Perps remain non-live (fail-closed surfaces unchanged; no perp lifecycle wired)
- [x] No secret exposure
- [x] No chain transaction
- [x] No deployment
- [x] No mainnet
- [x] No Solidity change

---

## 13. Limitations / deferred

- Conditional-orders evaluator worker (`trigger_one` / `persist_recovered`) does NOT emit lifecycle events in V1 — periodic snapshot ticker covers it. Wire in `ORDER-LIFECYCLE-OBSERVABILITY-WORKER-V1`.
- Frontend Fills + Conditional Orders dedicated panels + History-tab integration → `FRONTEND-LIFECYCLE-OBSERVABILITY-V1`.
- Client-side WebSocket consumer in the frontend (backend already emits push frames; current panel uses REST polling + recovery) → `FRONTEND-LIFECYCLE-OBSERVABILITY-V1`.
- Playwright coverage of the new panel → `FRONTEND-LIFECYCLE-OBSERVABILITY-V1`.
- Full notification system (in-app toasts on lifecycle deltas) → deferred.
- Advanced activity feed → deferred.
- Perps lifecycle → blocked behind `ACCOUNT-WRITE-AUTH-HARDENING-PERPS-V1` (perps non-live).
- Session keys, bot/API-key streams, analytics dashboards → out of V1 scope.

---

## 14. Next recommendation

`FRONTEND-LIFECYCLE-OBSERVABILITY-V1` — wire Fills + Conditional Orders panels + History-tab integration + a client-side WebSocket consumer that consumes the lifecycle deltas the backend now emits. Playwright coverage of the new panels lives there. After that, `ORDER-LIFECYCLE-OBSERVABILITY-WORKER-V1` to emit from the conditional-orders evaluator worker.
