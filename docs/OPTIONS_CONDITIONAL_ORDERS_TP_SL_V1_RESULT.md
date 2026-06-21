# OPTIONS-CONDITIONAL-ORDERS-TP-SL-V1 — Result

Persistent server-side Take-Profit / Stop-Loss conditional exit
orders for existing option positions. Direct-orderbook execution
only — paired RFQ execution-intent flow remains untouched. Perps
out of scope (no orderbook engine exists for them).

## Architecture discovered

| Topic | Finding |
|---|---|
| Oracle | `OracleRouter.getPriceSafe(baseAsset, quoteAsset)` (Solidity, reverts on stale). Already wired through `crate::api::trading_views::try_get_oracle_price_safe` and consumed by `account_positions` for `mark_price_1e8`. Reused as-is. |
| Position model | No `option_positions` table. Positions are reconstructed by aggregating `option_fills` by `(account, series_id)`. Same derivation re-used in `compute_position_signed_1e8`. |
| Auth for writes | `POST /options/orders` accepts an optional EIP-712 signature; the existing convention is "optional signature, validate shape if present". V1 conditional-orders endpoints mirror this convention. |
| Worker pattern | `spawn_option_confirmation_worker` is the template (config struct + env vars + `tokio::spawn` loop that refuses to spawn when RPC missing). |
| DB conventions | UUID primary keys; `BIGINT` timestamps in `_ms`; status as TEXT; complex enums as TEXT; composite UNIQUE for idempotency; explicit RETURNING for atomic updates. |

## Trigger source

`underlying_oracle` — the on-chain `OracleRouter.getPriceSafe` price.
The worker:
* refuses to spawn if `OPTION_ORACLE_ROUTER_ADDRESS` is unset or
  `RPC_URL` is missing,
* per-tick skips with `skipped_oracle_unavailable=true` if the
  provider returns an error or `None`,
* groups armed orders by series and reads each `(underlying,
  settlement)` pair at most once per tick.

No external HTTP source (no Binance / no CoinGecko) is introduced.
**Stale oracle never triggers an order.**

## Trigger-direction matrix

The comparator is server-derived from `(option_kind, position_side,
conditional_type)` and stored explicitly as `gte | lte`. Client-
supplied explicit comparators are validated against the derived
value; mismatches are rejected with `InvalidTriggerDirection`.

| kind | side | conditional | comparator |
|---|---|---|---|
| call | long | TP | gte |
| call | long | SL | lte |
| call | short | TP | lte |
| call | short | SL | gte |
| put  | long | TP | lte |
| put  | long | SL | gte |
| put  | short | TP | gte |
| put  | short | SL | lte |

Threshold equality is **inclusive** (`>=` / `<=`). Documented and
tested.

**Caveat (documented in UI):** underlying-price TP/SL does not
perfectly track option PnL — premium also responds to volatility
and time decay.

## Reduce-only enforcement

* At arm time:
  - position is derived from `option_fills` filtered by `(account,
    series_id)`,
  - request is rejected with `NoReduciblePosition` if signed size is
    zero,
  - request is rejected with `QuantityExceedsPosition` if
    `quantity > reducible_size`,
  - over-commit check sums non-terminal conditional orders for the
    same series (OCO siblings count once).
* At trigger time, BEFORE child submission:
  - the live reducible size is recomputed,
  - if `live_pos.side != stored.position_side` → status `Failed`
    with `failure_code=position_side_changed`,
  - if `live_reducible == 0` → status `Cancelled` with
    `failure_code=position_closed`,
  - otherwise the child quantity is capped at
    `min(stored.quantity, live_reducible)`.
* The child order is always `reduce_only = true` (mandated by
  table default + service path). A child cannot increase or reverse
  the position.

## IOC execution policy

Every triggered child order is built as:
* `side = position_side.closing_side()` (Long→Sell, Short→Buy),
* `price_1e8 = limit_price_1e8` (mandatory, never market),
* `size_1e8 = capped reducible quantity`,
* `time_in_force = IOC`,
* `post_only = false`,
* `client_order_id = "cond-<conditional_id>"` — the existing
  UNIQUE INDEX
  `(account, client_order_id) WHERE status IN (open,
  partially_filled)` protects against double-submission after
  restart. IOC orders never linger in a live status, so the worker
  re-entry on the same `conditional_id` cannot produce a second
  child.

Routing goes through the existing `submit_option_order` service —
NOT through `/options/execution-intents` (paired RFQ flow). The same
TIF matcher already proven for the direct orderbook path is reused
verbatim.

## OCO atomicity

* The two legs share an `oco_group_id` UUID assigned at create time.
* DB path: when the winner is claimed via the atomic `armed →
  triggering` UPDATE, `Repository::cancel_oco_siblings(group,
  winner_id, now_ms)` executes a single UPDATE that flips every
  remaining `armed` sibling to `cancelled` with
  `failure_code='oco_sibling_triggered'` in the same connection.
* In-memory path: `execute_triggered_in_store` cancels the sibling
  inside the same `&mut self` borrow before the child order is
  submitted. The store is single-threaded behind a `Mutex`, so the
  critical section is the borrow itself.
* Manual cancellation of one leg leaves the sibling armed
  (documented policy). Use `cancel_oco_group_in_store` to cancel
  both legs at once (frontend "cancel group" hook hooks into this
  in a follow-up; current UI lists per-leg cancel).

**A parallel evaluator cannot trigger both legs**: only one leg can
win the `armed → triggering` transition; once it wins, the sibling
is no longer `armed` and the next pick by the evaluator sees no
match.

## Persistence model

Migration `0028_options_conditional_orders.sql`:
```sql
CREATE TABLE options_conditional_orders (
    id UUID PRIMARY KEY,
    account TEXT NOT NULL,
    option_series_id TEXT NOT NULL REFERENCES option_series(option_series_id),
    position_side TEXT NOT NULL,     -- long | short
    option_kind TEXT NOT NULL,       -- call | put
    conditional_type TEXT NOT NULL,  -- take_profit | stop_loss
    trigger_source TEXT NOT NULL,    -- underlying_oracle
    trigger_condition TEXT NOT NULL, -- gte | lte
    trigger_price_1e8 TEXT NOT NULL,
    quantity_1e8 TEXT NOT NULL,
    execution_type TEXT NOT NULL,    -- ioc_limit
    limit_price_1e8 TEXT NOT NULL,
    reduce_only BOOLEAN NOT NULL DEFAULT TRUE,
    oco_group_id UUID NULL,
    status TEXT NOT NULL,
    child_order_id TEXT NULL REFERENCES option_orders(order_id),
    failure_code TEXT NULL,
    failure_message TEXT NULL,
    expires_at_ms BIGINT NULL,
    triggered_at_ms BIGINT NULL,
    completed_at_ms BIGINT NULL,
    created_at_ms BIGINT NOT NULL,
    updated_at_ms BIGINT NOT NULL,
    version BIGINT NOT NULL DEFAULT 0
);
```
Indexes: `WHERE status = 'armed'` (hot worker path), `lower(account)`
(per-account listings), `option_series_id`, `WHERE oco_group_id IS
NOT NULL` (sibling cancel).

`version` is an optimistic-lock counter. `update_conditional_order`
requires the DB to currently hold the previous `version`, returning
`InvalidConditionalOrderId` on a stale write.

`claim_conditional_order_armed` is the atomic transition: a single
`UPDATE ... WHERE id=$1 AND status='armed' RETURNING ...`. The
worker uses it before any side-effectful work.

In-memory mirror in `OptionSeriesStore` (no-DB local mode and unit
tests) implements the same lifecycle via direct HashMap mutation
under the existing `Mutex`. Both paths share the same Rust types
and service-level entrypoints.

## API surface

Routes (mounted in `src/api/routes.rs`):
* `POST /accounts/{address}/conditional-orders`
* `GET  /accounts/{address}/conditional-orders`
* `GET  /accounts/{address}/conditional-orders/{id}`
* `DELETE /accounts/{address}/conditional-orders/{id}`

Request (single TP, single SL, or TP+SL OCO pair):
```json
{
  "option_series_id": "0x...",
  "quantity_1e8": "100000000",
  "link_as_oco": true,
  "legs": [
    { "conditional_type": "take_profit",
      "trigger_price_1e8": "80000000000",
      "limit_price_1e8":   "1500000000" },
    { "conditional_type": "stop_loss",
      "trigger_price_1e8": "60000000000",
      "limit_price_1e8":   "500000000" }
  ],
  "expires_at_ms": null
}
```

Response (per leg):
```json
{
  "id": "uuid",
  "account": "0x...",
  "option_series_id": "0x...",
  "position_side": "long",
  "option_kind": "call",
  "conditional_type": "take_profit",
  "trigger_source": "underlying_oracle",
  "trigger_condition": "gte",
  "trigger_price_1e8": "80000000000",
  "quantity_1e8": "100000000",
  "execution_type": "ioc_limit",
  "limit_price_1e8": "1500000000",
  "reduce_only": true,
  "oco_group_id": "uuid|null",
  "status": "armed",
  "child_order_id": "string|null",
  "failure_code": "string|null",
  "failure_message": "string|null",
  "triggered_at_ms": null,
  "completed_at_ms": null,
  "created_at_ms": 1782000000000,
  "updated_at_ms": 1782000000000,
  "version": 0
}
```

Error responses (all `400`, `{"error": message}` envelope, stable
English messages):
* `no reducible option position for series <id>` —
  `NoReduciblePosition`
* `conditional order quantity exceeds the reducible position size`
  — `QuantityExceedsPosition`
* `conditional trigger direction is inconsistent with the position`
  — `InvalidTriggerDirection`
* `conditional order trigger price must be > 0` —
  `InvalidTriggerPrice`
* `conditional order limit price must be > 0` —
  `InvalidConditionalLimitPrice`
* `invalid conditional order id` — `InvalidConditionalOrderId` (also
  emitted on cross-wallet cancel)
* `conditional order is already terminal` —
  `ConditionalOrderAlreadyTerminal`
* `underlying oracle price is unavailable or stale` —
  `OracleUnavailable` (reserved; the evaluator skip path returns
  `skipped_oracle_unavailable=true` without surfacing it as an HTTP
  error)
* `oco sibling already triggered or terminal` — `OcoSiblingTerminal`

Cross-wallet cancellation is mapped to `InvalidConditionalOrderId`
rather than a separate error, mirroring the existing
"don't disclose record existence to non-owners" posture.

**No admin endpoints, no internals leakage.**

## Frontend integration

* `src/lib/trading-types.ts` — `ConditionalType`, `TriggerSource`,
  `TriggerCondition`, `ConditionalExecutionType`, `PositionSide`,
  `OptionKind`, `ConditionalOrderStatusValue`,
  `ConditionalLegRequest`, `CreateConditionalOrderRequest`,
  `ConditionalOrderResponse`.
* `src/lib/trading-api.ts` — `createConditionalOrders`,
  `listConditionalOrders`, `cancelConditionalOrder` (and `DELETE`
  added to the `rawRequest` method union).
* `src/components/trading/TpSlManager.tsx` — new component:
  - section title "TP / SL",
  - per-leg `Take Profit` / `Stop Loss` toggles with `Trigger (1e8)`
    + `IOC limit (1e8)` fields,
  - `Close quantity (1e8)` field,
  - `Link as OCO` checkbox (only enabled when both legs are
    selected),
  - reduce-only safety note + the required `Triggers use the
    underlying oracle price. Execution uses a price-protected IOC
    order and may fill partially or not fill.` copy,
  - `Create TP / SL` action that posts to the backend,
  - active table (`Type · Trigger · Qty · Limit · Status · OCO ·
    Cancel`) with refresh + per-row cancel,
  - honest disabled state when wallet disconnected or no series
    selected. The disabled state is exposed as
    `data-tp-sl-disabled` for tests.
* `src/components/trading/terminal/TradeTicketPanel.tsx` — renders
  `<TpSlManager address={...} seriesId={leg?.seriesId}/>` under
  `<DirectOrderbookForm/>` in Orderbook mode only. RFQ mode does
  NOT receive TP/SL controls (paired RFQ does not honour them).
* No perps changes. The Perps trade form keeps its existing
  `Perps not live` disabled posture.

## Tests + validations

Backend unit tests (`src/options/conditional_orders.rs::tests`):
* trigger-direction matrix for long/short × call/put × TP/SL
* `TriggerCondition.matches` boundary inclusivity
* `classify_reducible` sign mapping
* `closing_side` inversion (Long→Sell, Short→Buy)
* evaluator skip path when oracle missing

Backend integration tests (`tests/conditional_orders_tests.rs`):
* `create_long_call_tp_is_armed_with_gte_comparator`
* `create_long_call_sl_is_armed_with_lte_comparator`
* `missing_position_rejected_with_no_reducible_position`
* `excessive_quantity_rejected`
* `explicit_inconsistent_comparator_rejected`
* `oco_pair_shares_group_and_assigns_both_legs`
* `oco_winner_completes_and_cancels_sibling_via_in_store_execute`
  (full trigger → child IOC fill → sibling cancel)
* `cancel_armed_order_transitions_to_cancelled`
* `cross_wallet_cancel_rejected_as_invalid_id`
* `list_returns_only_owner_account_rows`
* `get_by_unknown_id_returns_none`
* `evaluator_skips_when_oracle_unconfigured_or_provider_missing`

Validation results:
* `cargo fmt --check` clean.
* `cargo check --lib` clean.
* `cargo test --lib` → **1013 passed**, 0 failed (8 new unit tests
  for conditional orders).
* `cargo test --test conditional_orders_tests` → **12 passed**.
* `cargo test --test options_tests` → **88 passed** (no regression).
* `npm run lint` clean (frontend).
* `npx tsc --noEmit` clean (frontend).

## Files changed

Backend:
* `migrations/0028_options_conditional_orders.sql` — new.
* `src/error.rs` — 10 new `BackendError` variants
  (`InvalidConditionalOrderId`, `InvalidConditionalOrderState`,
  `ConditionalOrderAlreadyTerminal`, `NoReduciblePosition`,
  `QuantityExceedsPosition`, `InvalidTriggerDirection`,
  `InvalidTriggerPrice`, `InvalidConditionalLimitPrice`,
  `OracleUnavailable`, `DuplicateConditionalOrderKey`,
  `OcoSiblingTerminal`).
* `src/options/conditional_orders.rs` — new module: types, enums,
  trigger-direction matrix, reduce-only helpers, OCO atomic
  primitives, in-store create/cancel/execute, service-layer
  branching (`create_conditional_orders`,
  `list_conditional_orders`, `get_conditional_order`,
  `cancel_conditional_order`), worker
  (`spawn_conditional_orders_worker`,
  `evaluate_conditional_orders_tick`), `ConditionalOrdersConfig`
  with `CONDITIONAL_ORDERS_ENABLED|POLL_INTERVAL_MS|BATCH_SIZE|
  MAX_RETRIES` env vars (defaults disabled, 2000 ms, 64, 3).
* `src/options/mod.rs` — register the module.
* `src/options/store.rs` — `OptionSeriesStore.conditional_orders`
  map + `insert/get/list/update_conditional_order`,
  `list_open_match_candidates`, `sort_match_candidates_view`
  re-export.
* `src/db/repository.rs` — `insert_conditional_order`,
  `get_conditional_order`, `list_conditional_orders`,
  `update_conditional_order` (optimistic lock),
  `claim_conditional_order_armed` (atomic transition),
  `cancel_oco_siblings`, `list_option_fills_for_account_and_series`,
  `conditional_order_from_row` + `CONDITIONAL_ORDER_SELECT`.
* `src/api/http.rs` — `AppState.conditional_orders_config` (default
  `ConditionalOrdersConfig::default()`).
* `src/api/routes.rs` — 4 routes + 4 handlers + DTOs + error
  mapping (10 new BAD_REQUEST variants). `address_from_account_str`
  promoted to `pub(crate)` so the worker can call it.
* `tests/conditional_orders_tests.rs` — new (12 tests).

Frontend:
* `src/lib/trading-types.ts` — conditional-orders type block.
* `src/lib/trading-api.ts` — `createConditionalOrders`,
  `listConditionalOrders`, `cancelConditionalOrder`. `rawRequest`
  method union extended with `"DELETE"`.
* `src/components/trading/TpSlManager.tsx` — new component.
* `src/components/trading/terminal/TradeTicketPanel.tsx` —
  `<TpSlManager />` rendered under `<DirectOrderbookForm />` in
  Orderbook mode only.

## Known limitations / deferred

* Option-mark / option-PnL TP-SL triggers — V1 uses
  underlying-price only. A documented caveat appears in the UI.
* Trailing stop — not in V1.
* Attached-on-entry TP/SL brackets — V1 requires an existing
  position before arming.
* Perps TP/SL — perps have no orderbook engine; out of scope.
* Dedicated WebSocket private channel
  (`account.conditional_orders`) — deferred. The UI uses REST list
  + per-action refresh, which is sufficient given that the worker
  poll interval already bounds end-to-end latency.
* Signed/EIP-712 wallet proof on POST/DELETE — V1 mirrors the
  existing `POST /options/orders` convention (signature optional;
  cross-wallet cancel is still rejected via account match). A
  follow-up should make signatures mandatory when the rest of the
  trading surface does.
* Idempotency-key UNIQUE constraint — not added in V1
  (`(account, series_id)` is composite-keyed via the over-commit
  guard rather than a DB UNIQUE).
* History v2 dedicated tab — the conditional-orders REST listing
  is the read surface in V1.

## Safety posture

* No Solidity touched. The on-chain `OracleRouter.getPriceSafe`
  ABI is referenced as-is via the existing helper.
* No chain transaction signed or broadcast.
* No deployment.
* No mainnet enabling.
* No `.env` values, RPC URLs, DB URLs, private keys, admin bearer
  tokens, wallet secrets, or local secret paths read, printed,
  modified, or surfaced.
* The worker is opt-in (`CONDITIONAL_ORDERS_ENABLED=true`); the
  default-off posture means no automatic background work happens
  without explicit operator configuration.
* When oracle/RPC is missing the worker logs a warning and refuses
  to spawn; ticks otherwise skip with `skipped_oracle_unavailable=
  true` and zero state mutation.

## Next recommendation

When persistence is enabled in production, do **one** rollout of:
1. `cargo sqlx prepare` (or apply migration 0028 in CI),
2. set `CONDITIONAL_ORDERS_ENABLED=true` plus the existing
   `OPTION_ORACLE_ROUTER_ADDRESS` + `RPC_URL` env vars,
3. observe `conditional orders tick` log lines for `evaluated > 0`,
4. measure end-to-end latency from a synthetic price crossover to
   `completed` status on a test account before opening the surface
   to all wallets.
