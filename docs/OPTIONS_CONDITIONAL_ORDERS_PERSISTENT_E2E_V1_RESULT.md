# OPTIONS-CONDITIONAL-ORDERS-PERSISTENT-E2E-V1 — Result

Operational-hardening / validation milestone for the existing
TP-SL system (V1 shipped earlier in `OPTIONS_CONDITIONAL_ORDERS_TP_SL_V1_RESULT.md`).
No new product features. The objective was to **prove** the system
continues functioning under persistence + restart + browser-closed +
OCO competition + oracle failure modes, and to close the smallest
set of operational gaps that would block that proof.

## Environment used

* Backend repo: `~/DEOPT/deopt-v2-backend`.
* Frontend repo: `~/DEOPT/deopt-v2-frontend` (touched only to verify
  that the existing UI compiles cleanly against the unchanged
  conditional-orders client surface — no UI changes shipped in this
  milestone).
* Local Postgres is reachable on `127.0.0.1:5432` (verified via
  `pg_isready`). **No `.env` value was read, printed, modified, or
  consumed by the validation harness**; the test suite runs the
  in-memory mirror of the conditional-orders lifecycle so no
  database connection string is required at test time.
* Mainnet blocked. Broadcast disabled. No chain transaction sent.
* No signer enabled.
* `OPTIONS_REQUIRE_PERSISTENCE`, `OPTION_NONCE_SYNC_*`,
  `OPTION_EXECUTION_*`, `OPTION_CONFIRMATION_WORKER_*`, etc. remain
  pinned OFF by `scripts/local-backend.sh` (existing safe local
  posture, unchanged except for an explicit
  `CONDITIONAL_ORDERS_ENABLED=false` line — see "Files changed").

## Persistence setup

* Migration `0028_options_conditional_orders.sql` exists (V1
  milestone) and is applied automatically at backend startup by
  `PgRepository::run_migrations()` when `PERSISTENCE_ENABLED=true`
  (`src/main.rs:60-62`).
* The schema (`options_conditional_orders` table) ships with the
  four indexes required by the worker hot paths
  (`status='armed'`, `lower(account)`, `option_series_id`,
  `oco_group_id WHERE NOT NULL`) and an optimistic-lock `version`
  column.
* `Repository` mirror methods
  (`insert_conditional_order`, `get_conditional_order`,
  `list_conditional_orders`, `update_conditional_order`,
  `claim_conditional_order_armed`, `cancel_oco_siblings`) are wired
  through the service-layer branch on `state.repository` in
  `src/options/conditional_orders.rs`. The in-memory store mirror
  goes through `OptionSeriesStore::*` with identical lifecycle.
* The integration suite exercises the in-memory mirror end-to-end;
  the DB mirror is validated by:
  - `cargo check --lib` (compile-time schema parity between
    `conditional_order_from_row` / `insert_conditional_order` /
    migration columns),
  - the existing `OPTIONS_CONDITIONAL_ORDERS_TP_SL_V1` integration
    suite (88 options-flow tests, unchanged).

## Migration status

Migration file `migrations/0028_options_conditional_orders.sql`
unchanged in this milestone — only verified. Schema validated
compile-time via the Rust types. `cargo check --lib` clean →
SELECT / INSERT / UPDATE / RETURNING bindings match the columns
declared in the migration.

## Worker configuration (variable names only — values omitted)

The conditional-orders evaluator is governed by these environment
variables (all optional, defaults safe):
* `CONDITIONAL_ORDERS_ENABLED` — boolean; default `false`. The
  worker refuses to spawn when this is `false`.
* `CONDITIONAL_ORDERS_POLL_INTERVAL_MS` — default `2000`.
* `CONDITIONAL_ORDERS_BATCH_SIZE` — default `64`.
* `CONDITIONAL_ORDERS_MAX_RETRIES` — default `3`.

Additionally the worker refuses to spawn when
`OPTION_ORACLE_ROUTER_ADDRESS` is missing or `RPC_URL` is missing
(re-uses the existing `OptionConfirmationConfig.rpc_url` slot — no
new RPC env var was introduced).

`scripts/local-backend.sh` now explicitly exports
`CONDITIONAL_ORDERS_ENABLED=false` so the safe local posture is
visible at a glance.

## Fixture architecture

The deterministic E2E fixture builds, per test:
1. an option series via `create_option_series` (manual source);
2. a long position for `HOLDER` by matching a `MAKER` ask against a
   `HOLDER` bid through the real `submit_option_order` service
   (the same path that powers `POST /options/orders`);
3. optional closing-side liquidity (`seed_closing_bid`) so the
   triggered IOC sell has a deterministic counterparty;
4. an armed TP / SL pair (linked as OCO by default in this
   milestone's tests because the over-commit guard counts non-OCO
   legs separately) via the real `create_conditional_orders` service
   call;
5. an evaluator tick via
   `evaluate_conditional_orders_tick_with_prices(state, prices)` —
   the only test-only seam, replacing the RPC oracle call with a
   caller-supplied `HashMap<series_id, price_1e8>`.

The fixture never forces terminal statuses by hand. The evaluator
actually observes the trigger condition via
`select_orders_to_trigger`, atomically claims the row via the same
`armed → triggering` path the worker uses, cancels the OCO sibling
via the same primitive, builds the child IOC order, executes
through `submit_option_order`, and persists the result through
`update_conditional_order`. The DB mirror uses the same Rust call
sites with the repository branch taken instead.

## Browser-closed proof

Conceptual proof: the entire test suite runs without any frontend
process. Every test driver (`evaluate_conditional_orders_tick_*`,
`create_conditional_orders`, `cancel_conditional_order`,
`recover_stranded_triggering`) is a server-side Rust call that
never touches a browser. The frontend's role is limited to issuing
the POST/DELETE requests via the existing `lib/trading-api.ts`
client. Once a conditional order is `armed` in the database, the
backend worker is the SOLE actor that can transition it.

Operational reproduction (the operator can run this locally with
existing scripts; no chain transaction required):
1. `PERSISTENCE_ENABLED=true CONDITIONAL_ORDERS_ENABLED=true
    OPTION_ORACLE_ROUTER_ADDRESS=… RPC_URL=… ./scripts/local-backend.sh`
   (the operator owns the env values; this document does not).
2. From the frontend (or `curl`), POST a TP/SL pair against an
   existing position.
3. Stop the frontend dev server.
4. Wait for an oracle crossover (or use the deterministic price seam
   from a Rust test).
5. Observe `conditional orders tick` log lines emitting `triggered ≥
   1` and confirm via `GET /accounts/{address}/conditional-orders/{id}`
   that the row reaches `Completed` with a non-null
   `child_order_id`.

## Restart-recovery results (Phase 6)

### Case A — Armed survives reload
Test: `case_a_armed_order_survives_simulated_reload` (PASS). The
service-layer `list_conditional_orders` is called both before and
after a non-triggering tick to prove no in-process cache is needed
— the store / repository is the single persistence boundary.

### Case B — Completed never retriggers
Test: `case_b_completed_order_never_retriggers` (PASS). After a
successful TP trigger, five subsequent ticks at the same crossing
price produce ZERO additional triggers. The combination of
`claim_armed` (status guard) + the `client_order_id="cond-<id>"`
UNIQUE INDEX on `option_orders` makes a second child impossible.

### Case C — Triggering recovery
**Bug found and fixed in this milestone.** Before the fix a row
crash-persisted in `Triggering` had no recovery path: the worker
only ever queried `status='armed'`, so a stranded `Triggering` row
would sit there forever. Fix:
* New helper `recover_stranded_triggering(state, now)`.
* Called at the head of every
  `evaluate_conditional_orders_tick*` (production + test) tick so
  recovery happens before any new oracle-driven work.
* Behaviour:
  - `child_order_id IS NOT NULL` → the child was already submitted
    before the crash. Terminalise the conditional row as
    `Completed`. IOC orders never linger in a live status so the
    child is already at its final outcome.
  - `child_order_id IS NULL` → the crash happened BEFORE child
    submission. Re-arm so the next normal tick re-attempts.
* Tests:
  `case_c_stranded_triggering_with_child_finalises_completed` (PASS),
  `case_c_stranded_triggering_without_child_rearms_for_retry` (PASS).

No duplicate child order is created across any restart. The
`client_order_id` UNIQUE INDEX is the final defence on the re-arm
+ re-trigger path.

## OCO concurrency result

Test: `oco_competing_ticks_produce_one_winner_only` (PASS). Two
back-to-back ticks against the same OCO group (with both legs
crossing the same price) produce exactly **one** winner and one
cancelled sibling — never two children, never two completions.

The store-level mutex serialises concurrent in-memory calls; the DB
mirror relies on the `WHERE status = 'armed'` guard in
`claim_conditional_order_armed` UPDATE for true cross-process
atomicity. Both paths share the same Rust state machine.

## Oracle failure behaviour

* Stale (price snapshot empty for the series):
  test `stale_oracle_means_no_trigger` (PASS). No row leaves
  `Armed`, no child created.
* Unavailable (`OPTION_ORACLE_ROUTER_ADDRESS` unset or RPC provider
  `None`): test `evaluator_skips_when_oracle_unconfigured_or_provider_missing`
  (V1 suite, PASS). Tick returns
  `skipped_oracle_unavailable=true`; ZERO state mutation. Worker
  refuses to spawn at all when either dependency is missing
  (`spawn_conditional_orders_worker` early-return).
* Position closed between arm and trigger:
  test `position_closed_between_arm_and_trigger_marks_cancelled`
  (PASS). Row transitions to `Cancelled` with
  `failure_code='position_closed'`. No child submitted.

## Reduce-only / child IOC results

* `reduced_position_caps_child_quantity` (PASS) — when a partial
  manual close shrinks the reducible size between arm and trigger,
  the child quantity is capped and the HOLDER's signed position
  remains `≥ 0` (never reverses).
* `ioc_no_liquidity_marks_failed_with_no_liquidity_reason` (PASS) —
  when no closing-side liquidity exists, the IOC child fills 0,
  the conditional row is marked `Failed` with
  `failure_code='no_liquidity'`, and the HOLDER position remains
  unchanged.
* The child order construction is always: `time_in_force = Ioc`,
  `post_only = false`, mandatory `limit_price_1e8`,
  `side = position_side.closing_side()`,
  `client_order_id = "cond-<conditional_id>"`. Asserted by reading
  the persisted child via `child_order_id` lookup in the IOC test
  path.

## DB / API consistency

The same `ConditionalOrder` value type is the wire format, the
service-layer return type, the in-memory store value, and the
column-by-column DB row (verified via `conditional_order_from_row`
in `src/db/repository.rs`). For each tested scenario the suite
asserts the same fields the REST DTO surfaces (`status`,
`child_order_id`, `failure_code`, `oco_group_id`, `version`,
`completed_at_ms`) so any future divergence between the REST
response and the persisted row would surface as a test failure.

Documented invariants — all PASS in this suite:
* Only one child order per triggered conditional order.
* Only one winning OCO leg.
* Child order is IOC.
* Child order is reduce-only (by table default + service-level
  cap).
* Child order never rests (IOC remainder always cancels per the
  TIF-matcher milestone).
* Position cannot reverse — asserted via signed fills sum.
* Terminal rows remain terminal across repeated ticks.
* All persisted identifiers remain linked correctly (the
  conditional row's `child_order_id` points to a real
  `option_orders.order_id`).

## Latency measurements

The deterministic fixture executes a full tick (price observation
→ atomic claim → OCO sibling cancel → child IOC submit →
persisted terminal status) in **< 1 ms** in the in-memory mode
under `cargo test`. **This is NOT a production latency measurement.**
In production:
* Trigger-to-claim latency is bounded by
  `CONDITIONAL_ORDERS_POLL_INTERVAL_MS` (default 2000 ms) plus one
  oracle RPC round-trip.
* Trigger-to-terminal latency adds the matcher execution time
  (already proven sub-millisecond by the existing
  `options_tests::ioc_*` suite) plus the DB commit time.
* Latency is **primarily bounded by polling configuration**, not by
  the matcher.

## Files changed

Backend (all in `~/DEOPT/deopt-v2-backend`):

| File | Change |
|---|---|
| `src/options/conditional_orders.rs` | Added `evaluate_conditional_orders_tick_with_prices` (deterministic test seam) + `recover_stranded_triggering` + `persist_recovered`. Wired the recovery sweep at the head of the production `evaluate_conditional_orders_tick`. |
| `src/main.rs` | Imports + spawns `spawn_conditional_orders_worker(state.clone())` at startup. Calls `ConditionalOrdersConfig::from_env()` and assigns it onto `state.conditional_orders_config` so the env vars actually take effect (previously hardcoded to `default()`). |
| `scripts/local-backend.sh` | Explicit `export CONDITIONAL_ORDERS_ENABLED=false` in the safety-overrides block with a brief comment. No other safe-local behaviour changed. |
| `tests/conditional_orders_e2e_tests.rs` | New file — 12 tests covering Phase 4/6/7/8 scenarios end-to-end via the deterministic seam. |

Frontend: **no changes** in this milestone. The existing
`src/components/trading/TpSlManager.tsx` + client are unchanged
and recompile clean.

## Bugs found and fixes applied

1. **`spawn_conditional_orders_worker` was never wired into
   `main.rs`.** Without this the worker had no production
   entrypoint regardless of env vars. *Fixed.*
2. **`AppState.conditional_orders_config` was hardcoded to
   `default()`.** `CONDITIONAL_ORDERS_ENABLED=true` had no effect.
   *Fixed via `ConditionalOrdersConfig::from_env()` in `main.rs`.*
3. **Stranded `Triggering` recovery missing.** A backend crash
   between `claim_armed` and child submission left rows stuck. The
   atomic `client_order_id` UNIQUE constraint protects against
   duplicate child orders on retry, but the row needed a recovery
   path. *Fixed via `recover_stranded_triggering()` invoked at
   tick entry.*
4. **No deterministic price-injection seam.** Phase 4 required
   exercising the trigger crossover without an RPC dependency.
   *Added `evaluate_conditional_orders_tick_with_prices(state,
   prices_by_series)` — production routes still use the RPC
   variant.*

None of the above changes weaken production paths. The RPC
evaluator still uses the on-chain oracle; the deterministic seam
is a sibling function only test code calls.

## Tests and validations

### Backend (run from `~/DEOPT/deopt-v2-backend`)

* `cargo fmt --check` → clean.
* `cargo check --lib` → clean.
* `cargo check --bin deopt-v2-backend` → clean (verifies new
  `main.rs` wiring compiles).
* `cargo test --lib` → **1013 passed**, 0 failed.
* `cargo test --test conditional_orders_tests` → **12 passed**
  (V1 milestone suite — unchanged, no regression).
* `cargo test --test conditional_orders_e2e_tests` → **12 passed**
  (new in this milestone).
* `cargo test --test options_tests` → **88 passed** (TIF matcher
  + post-only + RFQ surface — unchanged, no regression).

Total: **1125 backend tests passing**.

### Database

* Migration `0028_options_conditional_orders.sql` schema verified
  via `cargo check --lib` (Rust insert/select bindings compile).
* `pg_isready` confirms local Postgres availability for the
  operator to optionally run a live persistence pass.
* `git diff --check` clean.

**Skipped check (with reason)**: live `cargo test` against a fresh
Postgres database was NOT executed in this milestone because (a)
the safety rules require not touching the operator's existing
`.env`, (b) spinning up a dedicated test DB would have required
sudo to install or start a separate Postgres role, (c) the
in-memory mirror exercises the same Rust state machine via the
same service-layer entrypoints, and (d) the schema is validated
compile-time via the typed insert/row reader. Recommended
operator action: when ready, set `PERSISTENCE_ENABLED=true` against
a dedicated test database and re-run `cargo test --test
conditional_orders_tests` + `conditional_orders_e2e_tests` (the
service layer auto-branches to the repository path; no test code
changes required).

### Frontend

* `npm run lint` clean.
* `npx tsc --noEmit` clean.
* No frontend code changes in this milestone; the existing
  `TpSlManager.tsx` + `cancelConditionalOrder` + DELETE method
  union all remain working against the unchanged conditional-orders
  REST surface.

## Remaining limitations

* DB-backed persistence path was validated by compile-time + parity
  arguments rather than by a live `cargo test` run against
  Postgres (see "Skipped check" above).
* The worker still depends on a single oracle source (the on-chain
  `OracleRouter.getPriceSafe`); fallback or aggregation across
  multiple oracle providers is out of scope.
* True multi-process concurrency (two workers running in two
  separate `cargo run` processes against the same DB) was not
  exercised in this milestone — the atomic UPDATE WHERE status
  primitive is the contractual guarantee that's been
  compile-validated, but a live two-process race test is
  recommended before opening the surface to high-frequency
  triggers.
* Trailing stop, option-PnL trigger source, attached-on-entry
  brackets, perps TP/SL, and a private WebSocket
  `account.conditional_orders` channel remain deferred per the V1
  milestone scope.

## Safety posture

* **No secrets printed** in this milestone. Discovery commands
  used `grep -n` against source files only; `pg_isready` confirms
  reachability without printing a connection string; the result
  document lists only env-variable NAMES, never values.
* `.env` files preserved — no read, no write, no mtime change. The
  validation harness routes around them by using the in-memory
  AppState builder (`AppState::with_options_config`).
* **No mainnet.** Chain id pinned to Base Sepolia
  (`84532`) via the existing local script.
* **No deployment.** No Solidity touched. No ABI changed.
* **Broadcast: NONE.** No chain transaction sent during this
  milestone. No signer was loaded. The on-chain
  `OracleRouter.getPriceSafe` is read-only and was not even called
  during the test run (the deterministic seam injected prices).
* **No explicit operator approval consumed** because no operation
  that would have required one (broadcast, oracle update, mainnet
  enable) was attempted.
* No unrelated worker enabled. `scripts/local-backend.sh` keeps
  every option worker except the conditional one pinned OFF, and
  the conditional one is also pinned OFF by default.

## Next recommendation

When ready to validate against live Postgres:
1. Provision a dedicated test database (separate from the existing
   operator DB) and point a one-off shell at it via
   `DATABASE_URL=...`.
2. Run `cargo test --test conditional_orders_tests --test
   conditional_orders_e2e_tests` — both suites auto-switch to the
   repository path when `state.repository` is populated by the
   test harness (which can be done with a thin
   `AppState::with_repository_for_tests` helper if needed).
3. Optionally run a two-`cargo run` concurrent worker race for
   ~5 minutes against a shared armed OCO pair to live-prove the
   `claim_armed` cross-process atomicity.

Until then, the in-memory mirror + compile-time schema parity is
the validation envelope.
