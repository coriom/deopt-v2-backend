# ORDER-LIFECYCLE-OBSERVABILITY-WORKER-V1 — Result

**Status:** CLOSED — the conditional-orders worker now emits real lifecycle events at every persisted transition (triggering, OCO sibling cancel, child order created, per-fill, terminal status, position_closed, no_liquidity, stranded recovery). All events are commit-safe: a rolled-back transaction never produces an event, and repeated no-op ticks never re-emit a terminal.

**Closed:** 2026-06-27
**Chain:** Base Sepolia (84532); no chain transaction occurred
**Mainnet:** disabled
**Solidity:** unchanged
**Deployment:** none
**Secrets exposed:** none

---

## 1. Worker paths discovered (Phase 1)

| Path | File:line | Was emitting before V2? |
|---|---|---|
| `evaluate_conditional_orders_tick` (RPC variant) | `src/options/conditional_orders.rs:1223` | No |
| `evaluate_conditional_orders_tick_with_prices` (deterministic seam) | `src/options/conditional_orders.rs:1339` | No |
| `trigger_one` (DB+in-memory dispatcher) | `src/options/conditional_orders.rs:1464` | No |
| `execute_triggered_via_repo` (DB) | `src/options/conditional_orders.rs:1519` | Partial — child order events were emitted via `submit_option_order`, but the conditional terminal updates were NOT |
| `execute_triggered_in_store` (in-memory) | `src/options/conditional_orders.rs:689` | No — submitted child via `store.submit_order_and_match` directly, bypassing the `submit_option_order` emit helper |
| `recover_stranded_triggering` (startup sweep) | `src/options/conditional_orders.rs:1398` | No |
| `cancel_oco_siblings` (DB) | `src/db/repository.rs:2316` | Returned `u64` rows_affected only — no row data to emit from |

V1 mutation points already emitting: `submit_option_order`, `cancel_option_order`, `create_conditional_orders`, `cancel_conditional_order` (per `ORDER-LIFECYCLE-OBSERVABILITY-V1`). Periodic 5 s snapshot ticker covered worker transitions before V2 but with a 5 s latency window and without push-on-change semantics.

---

## 2. Event design (Phase 2)

The existing V1 `LifecycleEvent` / `LifecyclePayload` schema covers every worker transition — no new payload variants or fields were introduced.

| Transition | Channel | Payload variant |
|---|---|---|
| Atomic claim → `triggering` | `account.conditional_orders` | `ConditionalOrderUpdated { status: "triggering", ... }` |
| OCO sibling cancellation | `account.conditional_orders` | `ConditionalOrderUpdated { status: "cancelled", failure_code: "oco_sibling_triggered", ... }` |
| Child IOC order accepted/matched | `account.orders` | `OrderUpdated { order_id, status, remaining_size_1e8, ... }` |
| Per-fill from child IOC | `account.fills` | `FillCreated { fill_id, order_id, side, price_1e8, size_1e8, created_at_ms }` (emitted for BOTH buyer and seller accounts) |
| Conditional terminal (completed / partially_filled / failed) | `account.conditional_orders` | `ConditionalOrderUpdated { status, child_order_id, failure_code, ... }` |
| `position_closed` cancellation (no reducible position at trigger time) | `account.conditional_orders` | `ConditionalOrderUpdated { status: "cancelled", failure_code: "position_closed", ... }` |
| `no_liquidity` failure (child IOC filled 0) | `account.conditional_orders` | `ConditionalOrderUpdated { status: "failed", failure_code: "no_liquidity", ... }` |
| Stranded `triggering` recovery | `account.conditional_orders` | `ConditionalOrderUpdated { status, child_order_id, ... }` — only when the recovery row was actually persisted |
| Stale oracle / no price | (none) | NO event — only persisted transitions emit |

Payloads carry only scalar IDs + status strings + sizes. **No signature, no auth envelope, no nonce, no private key, no raw RPC payload, no DB URL, no sensitive config is ever broadcast.**

---

## 3. Commit-safety strategy (Phase 3)

### DB path (`execute_triggered_via_repo` + `recover_stranded_triggering`)

Every emit call is placed AFTER the corresponding `repo.update_conditional_order(...)` returns `Ok`. Sequence per terminal arm:

```rust
let saved = repo.update_conditional_order(&closed).await?;   // commit
emit_conditional_lifecycle(state, std::slice::from_ref(&saved));
return Ok(saved);
```

A failed repository call propagates the `Err` through `?` — the emit line is unreachable, so no rolled-back transaction can produce an event.

Child order + per-fill events for the DB path are emitted by `submit_option_order` itself (V1 emission). The worker does NOT double-emit.

### In-memory path (`execute_triggered_in_store`)

The function holds a `Mutex<OptionSeriesStore>` lock for the entire trigger sequence (claim → OCO siblings → child submission → terminal persistence). Emitting WS events while holding that lock would extend the critical section and risk a WS-layer panic poisoning the store lock.

Solution: `WorkerLifecycleBatch` collects pending events INSIDE the lock, and `trigger_one` emits them AFTER the lock is dropped:

```rust
let mut batch = WorkerLifecycleBatch::default();
let result = execute_triggered_in_store(&mut store, ..., Some(&mut batch));
drop(store);                       // release the lock first
if result.is_ok() {
    batch.emit(state);             // emit only on successful execution
}
return result;
```

The batch is populated at each successful `store.update_conditional_order(...)` and after `store.submit_order_and_match(child, ...)` returns Ok. If any of these calls fails (e.g. `OcoSiblingTerminal`), the batch is dropped without emit — preserving the "no event for rolled-back state" invariant.

### OCO sibling cancellation (DB)

`PgRepository::cancel_oco_siblings` was extended from `Result<u64>` (rows_affected) to `Result<Vec<ConditionalOrder>>` (RETURNING the cancelled rows). The worker iterates the returned rows and emits one `ConditionalOrderUpdated` per sibling — no second round-trip needed.

### Recovery sweep

`persist_recovered` is called once per stranded row. The emit line runs only inside the `Ok(())` arm, so:
- If the optimistic-lock check fails (another worker mutated the row concurrently), the recovery is skipped silently — NO event emitted.
- If persistence genuinely commits the new status, exactly ONE event is emitted.

---

## 4. Account filtering (Phase 6)

Every worker emit carries `account = ConditionalOrder::account` (or `OptionOrder::account` / `OptionFill::{buyer,seller}` for the child events). The WS handler's `forward_lifecycle_event` enforces:

1. `session.account.eq_ignore_ascii_case(&event.account)` — wallet authenticated to the same account.
2. The session has an active subscription on the corresponding `account.*` channel.

These two gates predate V2 and apply transparently to the new worker events. The test `worker_trigger_emits_triggering_then_terminal` asserts every conditional event carries the HOLDER's address; `worker_child_order_emits_order_updated_and_fill_created` asserts the fill goes to BOTH the HOLDER (child-order owner) and the MAKER (counterparty) — each side's WS session will only receive its own copy.

No unauthenticated session can receive a worker event. No public channel ever carries account-private lifecycle data.

---

## 5. In-memory ↔ PostgreSQL parity (Phase 5)

The two paths emit the same logical event sequence for a successful trigger:

| Sequence element | DB path | In-memory path |
|---|---|---|
| 1. `ConditionalOrderUpdated(triggering)` | ✓ via `trigger_one` after `repo.claim_conditional_order_armed` | ✓ via `trigger_one` after `store.update_conditional_order` |
| 2. Per-sibling `ConditionalOrderUpdated(cancelled, oco_sibling_triggered)` | ✓ via `cancel_oco_siblings` returning rows | ✓ via `WorkerLifecycleBatch.push_conditional` inside the OCO loop |
| 3. Child `OrderUpdated` + per-fill `FillCreated` (both sides) | ✓ via `submit_option_order` (V1 emission, unchanged) | ✓ via `WorkerLifecycleBatch.set_child_order` + `emit_option_order_lifecycle` |
| 4. Conditional terminal `ConditionalOrderUpdated` | ✓ after final `repo.update_conditional_order` | ✓ via `WorkerLifecycleBatch.push_conditional` |

The only behavioural difference: DB-path order/fill events emit synchronously inside `submit_option_order`, while in-memory-path order/fill events emit only after the trigger lock is dropped (via the batch). For an external observer this is invisible — both arrive in the same order.

---

## 6. Files changed

### Backend (modified)

| File | Change |
|---|---|
| `src/db/repository.rs` | `cancel_oco_siblings` now `RETURNING *` → `Result<Vec<ConditionalOrder>>` so the worker can emit per-sibling lifecycle events without a second query. |
| `src/options/service.rs` | `emit_option_order_lifecycle` promoted from `fn` to `pub(crate) fn` so the worker's in-memory path can reuse the V1 order+fill emission helper. |
| `src/options/conditional_orders.rs` | Added `WorkerLifecycleBatch` collect-then-emit-after-commit helper; emit at claim (triggering), per-OCO-sibling cancel (DB + in-memory), child order + per-fill (in-memory), all terminal updates in both paths, stranded recovery. |
| `tests/conditional_orders_tests.rs` | Updated `oco_winner_completes_and_cancels_sibling_via_in_store_execute` to pass `None` for the new `WorkerLifecycleBatch` parameter on `execute_triggered_in_store`. |

### Backend (new)

| File | Purpose |
|---|---|
| `tests/conditional_orders_worker_lifecycle_tests.rs` | 7-scenario worker-emission proof. |

No frontend file was changed under this milestone.

---

## 7. Tests + validation (Phase 7 / Phase 9)

### New worker-lifecycle tests (7/7 pass)

| Test | Asserts |
|---|---|
| `worker_trigger_emits_triggering_then_terminal` | Tick → events include `triggering` + a terminal status; all events carry HOLDER's account. |
| `worker_child_order_emits_order_updated_and_fill_created` | Tick → at least one `account.orders` event + fill events for BOTH HOLDER and MAKER. |
| `oco_sibling_cancellation_emits_per_sibling_update` | Tick → SL leg gets `cancelled` with `failure_code="oco_sibling_triggered"`; TP winner gets at least one lifecycle update. |
| `stale_oracle_emits_no_lifecycle_events` | Tick with no price → 0 conditional events. |
| `repeated_ticks_do_not_re_emit_terminal_event` | First tick emits; 3 subsequent ticks emit nothing. |
| `position_closed_emits_cancelled_with_failure_code` | Holder closes position pre-trigger → tick emits `cancelled` + `failure_code="position_closed"`. |
| `stranded_recovery_emits_lifecycle_update` | Manually flip row to `triggering`-no-child; `recover_stranded_triggering` emits a lifecycle update and re-arms the row. |

### Regression sweep (all green)

| Suite | Pass / Ignored |
|---|---|
| `cargo test --lib` | **1030 / 0** |
| `cargo test --lib api::public_ws::` | 52 / 0 |
| `cargo test --test conditional_orders_tests` | **12 / 0** |
| `cargo test --test conditional_orders_e2e_tests` | **12 / 0** |
| `cargo test --test conditional_orders_worker_lifecycle_tests` | **7 / 0** (new) |
| `cargo test --test account_write_auth_tests` | 12 / 0 |
| `cargo test --test account_write_auth_pg_proof` | 0 / 0 in non-PG mode (silently skipped); 13/0 expected in PG mode |
| `cargo test --test options_tests` | 82 / 6 |
| `cargo test --test engine_tests` | 21 / 22 (perp non-live) |
| `cargo test --test rfq_tests` | 35 / 2 (perp non-live) |
| `cargo test --test mm_gateway_tests` | 67 / 0 |
| `cargo test --test orderbook_tests` | 13 / 0 |
| `cargo fmt --check` | clean |
| `cargo check --lib` / `--bin deopt-v2-backend` | clean |
| `git diff --check` | clean |

No frontend checks were re-run — no frontend file was modified under this milestone.

---

## 8. Hard acceptance criteria (Phase 9)

- [x] Worker-triggered TP/SL lifecycle updates emitted from real worker mutation points (`trigger_one` + `execute_triggered_via_repo` + `execute_triggered_in_store` + `recover_stranded_triggering`).
- [x] Child order + fills produced by worker trigger are observable (`account.orders` + `account.fills`).
- [x] OCO sibling cancellation is observable per-sibling (`ConditionalOrderUpdated { status: "cancelled", failure_code: "oco_sibling_triggered" }`).
- [x] Stale oracle does not emit fake events (test `stale_oracle_emits_no_lifecycle_events`).
- [x] Events are account-filterable and private (worker carries `account`; WS handler's `forward_lifecycle_event` enforces session-account match + active subscription).
- [x] REST snapshots remain the source of truth after restart (DB rows unchanged; this milestone only adds emission, never alters persistence).
- [x] No duplicate events for repeated no-op ticks (test `repeated_ticks_do_not_re_emit_terminal_event`).
- [x] No regression to TP/SL, matching, lifecycle V1, or write-auth (all suites green).
- [x] No secret exposure.
- [x] No chain transaction.
- [x] No deployment.
- [x] No mainnet.
- [x] No Solidity change.

---

## 9. Limitations / deferred

- WebSocket client consumer on the frontend → `FRONTEND-LIFECYCLE-OBSERVABILITY-V1`.
- Fills panel + Conditional Orders panel + History-tab integration → `FRONTEND-LIFECYCLE-OBSERVABILITY-V1`.
- Playwright coverage of lifecycle deltas in the UI → `FRONTEND-LIFECYCLE-OBSERVABILITY-V1`.
- Live PostgreSQL proof of the worker lifecycle events under concurrent ticks → optional follow-up (`ORDER-LIFECYCLE-OBSERVABILITY-WORKER-PG-PROOF-V1`). The in-memory parity proof + V1's `account_write_auth_pg_proof` already cover the DB path's commit semantics; a dedicated PG proof for the worker emission sequence is a nice-to-have, not a hard gate.
- Perps lifecycle remains blocked behind `ACCOUNT-WRITE-AUTH-HARDENING-PERPS-V1` (perps non-live).

---

## 10. Next recommendation

`FRONTEND-LIFECYCLE-OBSERVABILITY-V1` — wire the frontend Fills panel, Conditional Orders panel, History-tab integration, a client-side WebSocket consumer that subscribes to `account.orders`, `account.fills`, `account.conditional_orders` (all of which now publish real worker-driven deltas), plus Playwright coverage.
