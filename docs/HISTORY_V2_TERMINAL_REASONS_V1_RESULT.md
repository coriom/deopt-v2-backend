# HISTORY-V2-TERMINAL-REASONS-V1 — result

**Status: CLOSED.**

Persists real terminal reasons on option orders so `/history` can
distinguish a user cancel from an IOC remainder cancel without
relying on TIF + post-only inference. Pre-persistence rejections
(post-only would cross, FOK not fillable, matching rejections) are
still unrecoverable — they error synchronously at submit time and no
order row is ever inserted, so there is nothing in history to attach
a reason to. That gap is honestly documented, not faked.

This milestone supersedes the narrower
`HISTORY-V2-CANCEL-REASON-COLUMN-V1` deferred from
`HISTORY-V2-FAILURE-REASONS-V1`.

---

## Schema

New migration:
**`migrations/0030_option_orders_terminal_reason.sql`**

```sql
ALTER TABLE option_orders
    ADD COLUMN IF NOT EXISTS terminal_reason_code TEXT NULL,
    ADD COLUMN IF NOT EXISTS terminal_reason_message TEXT NULL,
    ADD COLUMN IF NOT EXISTS terminal_reason_source TEXT NULL;
```

Field roles:

* **`terminal_reason_code`** — stable snake_case enum-like token.
  Today: `user_cancelled`, `ioc_remainder_cancelled`.
* **`terminal_reason_message`** — optional free-text detail (NULL for
  the two automated cases today; reserved for future codes that carry
  a worker-supplied message).
* **`terminal_reason_source`** — who/what authored the transition:
  `user` | `tif_policy` | `system` (extensible).

Choice notes:

* `terminal_reason_*` (not `failure_code`/`failure_message`) because
  a user cancel is not a failure — generic naming fits both cases.
* Three nullable columns, no backfill, no new indexes (read alongside
  the row in existing per-account / per-order queries; we don't
  filter by reason).

---

## Reason codes & sources (today)

| code | source | when stamped | severity |
|---|---|---|---|
| `user_cancelled` | `user` | `cancel_option_order` repository `UPDATE` | info |
| `ioc_remainder_cancelled` | `tif_policy` | staged taker insert in `submit_option_order_and_match` when IOC status collapses to `cancelled` with `remaining > 0` | info |

Pre-persistence outcomes (NOT stamped because no row exists):

| brief code | why not stamped |
|---|---|
| `post_only_would_cross` | `enforce_tif_plan` errors before INSERT |
| `fok_not_fully_fillable` | `enforce_tif_plan` errors before INSERT |
| `matching_rejected` | matching rejection paths error pre-INSERT |
| `expired` | option order expiry sweep is not implemented (no worker exists today) |
| `system_cancelled` / `risk_rejected` / `insufficient_*` | no existing transition produces them on `option_orders` |

The frontend continues to render TIF-inferred labels for the
unrecoverable cases — pre-persistence rejections that DO somehow
land in the DB (manual seeding, future code paths) will still render
their inferred label.

---

## Population points

Two existing terminal transitions, both already had the bare status
write but no reason metadata:

1. **User cancel**
   * File: `src/db/repository.rs`, `PgRepository::cancel_option_order`
     (and in-memory mirror in `src/options/store.rs::cancel_order`).
   * Change: the `UPDATE option_orders ... SET status = 'cancelled', ...`
     now also `SET terminal_reason_code = 'user_cancelled',
     terminal_reason_source = 'user', terminal_reason_message = NULL`
     and the `RETURNING` clause surfaces the new columns.

2. **IOC remainder cancel at insert**
   * File: `src/db/repository.rs::submit_option_order_and_match` (the
     staged-taker assembly inside the locked tx) + the in-memory
     mirror in `src/options/store.rs::submit_order_and_match`.
   * Change: a small helper `stamp_insert_terminal_reason(&mut order)`
     in `src/options/store.rs` inspects the staged order and stamps
     `ioc_remainder_cancelled` / `tif_policy` if and only if the
     status is `Cancelled`, TIF is `Ioc`, and the order has unfilled
     remainder. Called on both code paths so the in-memory store and
     the Postgres repository stay in lock-step.

No matching semantics change. No TIF semantics change. No write-auth
semantics change. No fill generation change. The reason is purely
metadata attached to outcomes that already happen.

---

## API exposure

Read-only field additions:

* **`/accounts/:address/history/v2?tab=orders`** — `HistoryV2Item`
  now carries `terminal_reason_code` / `terminal_reason_message` /
  `terminal_reason_source` (all `skip_serializing_if = Option::is_none`,
  so empty rows stay compact).
* **`/options/orders`, `/options/orders/:id`, `/options/orders/cancel`** —
  `OptionOrderResponse` mirrors the same three fields.

Lifecycle WS payload (`OrderUpdated`) is intentionally **unchanged**.
Adding optional fields would have been a non-breaking serde change,
but the frontend already updates the visible reason via the refresh
banner / REST resync path proven by `HISTORY-LIFECYCLE-V2` — so we
took the lower-risk option per the brief.

---

## Frontend (companion doc lives in the frontend repo)

`deopt-v2-frontend/docs/HISTORY_V2_TERMINAL_REASONS_V1_RESULT.md`
covers:

* `HistoryV2Item` TS extension;
* `deriveOrderReason` priority: **persisted backend reason wins**,
  TIF inference is the fallback;
* `HistoryReason.source` carried through and surfaced on the DOM as
  `data-reason-source`;
* unchanged Trades/Fills tab (fills are always successful);
* unchanged TP/SL tab (conditional rows keep their `failure_code` /
  `failure_message` path);
* new node + Playwright tests.

---

## Validations (backend)

| | result |
|---|---|
| `cargo fmt --check` | clean |
| `cargo check --lib` | clean |
| `cargo check --tests` | clean |
| `cargo test --lib api::trading` | 136/0/0 (was 135; +1 history-v2 orders-tab wire test) |
| `cargo test --test options_tests` | 86/0/6 (was 82; +4 terminal-reason tests) |
| `cargo test --test orderbook_tests` | 13/0/0 |
| `cargo test --test account_write_auth_tests` | 12/0/0 |
| `cargo test --test conditional_orders_tests` | 12/0/0 |
| `git diff --check` (backend) | clean |

New backend tests (`tests/options_tests.rs`):

* `user_cancel_persists_terminal_reason_user_cancelled`
* `ioc_remainder_persists_terminal_reason_ioc_remainder_cancelled`
* `ioc_full_fill_does_not_persist_terminal_reason`
* `post_only_rejection_creates_no_order_row_and_no_terminal_reason`

New lib test (`src/api/trading.rs`):

* `history_v2_orders_tab_surfaces_persisted_terminal_reason`

---

## Files changed (backend)

### New

* `migrations/0030_option_orders_terminal_reason.sql`
* `docs/HISTORY_V2_TERMINAL_REASONS_V1_RESULT.md`

### Modified

* `src/options/types.rs` — added `terminal_reason_code/message/source`
  to `OptionOrder` (`Option<String>`, serde-skip-if-none); added
  `pub mod terminal_reason` with `USER_CANCELLED`,
  `IOC_REMAINDER_CANCELLED`, `SOURCE_USER`, `SOURCE_TIF_POLICY`.
* `src/options/mod.rs` — re-export `terminal_reason`.
* `src/options/store.rs` — added `stamp_insert_terminal_reason`;
  `submit_order_and_match` calls it; `cancel_order` stamps
  `user_cancelled` / `user`.
* `src/options/service.rs` — initialise `terminal_reason_*` to `None`
  on the freshly-built `OptionOrder` (compile-time exhaustiveness).
* `src/options/conditional_orders.rs` — same `None` initialisation on
  the conditional child order constructor.
* `src/db/repository.rs` — wired the three new columns through
  `insert_option_order_query`, `insert_option_order_tx`,
  `option_order_from_row`, and every option_orders SELECT;
  `cancel_option_order` `UPDATE` writes the user-cancel reason and
  the staged-taker assembly calls `stamp_insert_terminal_reason`.
* `src/api/trading.rs` — `HistoryV2Item` gains three optional
  reason fields; `orders_rows_for` populates them; new lib test
  pins the wire shape.
* `src/api/routes.rs` — `OptionOrderResponse` gains the same three
  optional reason fields (orders / orders-by-id / cancel responses).

---

## Safety

* No mainnet. No deployment. No Solidity. No transaction. No broadcast.
* No matching semantics, TP-SL semantics or write-auth semantics
  change.
* No real keys; no secret in logs / traces / artifacts.
* No perps lifecycle masquerading as live.
* No fabricated reasons — the two persisted codes are stamped only
  at the exact existing transition points whose cause is unambiguous.
* No backfill — pre-migration rows keep NULL reason fields and the
  frontend's TIF-derived fallback continues to render them.

---

## Hard acceptance criteria

| criterion | state |
|---|---|
| terminal reason fields are persisted for real existing option order terminal transitions where an order row exists | yes (user cancel + IOC remainder) |
| no fake order rows are created for pre-persistence rejections | yes (test pins it) |
| history and order APIs expose terminal reason fields | yes |
| `/history` Orders tab prefers persisted reason over inference | yes (node + Playwright proof) |
| active/open and successful filled orders do not show false terminal reasons | yes |
| Trades/Fills tab remains honest | yes |
| matching semantics unchanged | yes |
| TP/SL semantics unchanged | yes |
| write-auth semantics unchanged | yes |
| lifecycle frontend/backend tests remain green | yes |
| full Playwright suite remains green (modulo documented landing-particle skip) | yes — 305/0/1 |
| no secret exposure | yes |
| no chain transaction / no deployment / no mainnet / no Solidity change | yes |

---

## Deferred

* `HISTORY-V2-REJECTED-ATTEMPTS-FEED-V1` — surface pre-persistence
  rejections (post-only would cross, FOK not fillable, matching
  rejection) on a separate activity feed. Requires a new
  `option_order_rejections` table (or similar) so we don't fake
  order rows. Out of scope for this milestone.
* `OPTION-ORDER-EXPIRY-SWEEP-V1` — no expiry worker exists today;
  the `OptionOrderStatus::Expired` variant is unreachable. Future
  work.
* `LANDING-PARTICLE-SCROLL-LISTENER-V1` — last skipped Playwright
  test.
* `OPTIONS-ROUTE-INTERNAL-RENAME-V1`, `HISTORY-V2-FILTERS-V1`,
  `ORDER-LIFECYCLE-OBSERVABILITY-WORKER-PG-PROOF-V1`,
  `ACCOUNT-WRITE-AUTH-HARDENING-PERPS-V1` — carried forward.
* `TESTNET-READINESS-REHEARSAL-V1` — full launch rehearsal.

---

## Recommendation

`LANDING-PARTICLE-SCROLL-LISTENER-V1` to clear the last Playwright
skip and reach a fully-green CI landscape, **or**
`TESTNET-READINESS-REHEARSAL-V1` to start the launch posture work
now that the trading lifecycle observability story is end-to-end
honest.
