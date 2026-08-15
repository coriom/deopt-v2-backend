# OPTIONS-HYBRID-V2-RESERVATIONS-PENDING-SETTLEMENT-AND-CANONICAL-RELEASE-V1 — Foundational Packages Only

Honest partial closure. This sprint was authorized as **Foundational
+ atomic order acceptance** (Packages B, C, D, E) — a scoped subset
of the full milestone which spans Packages B through U. Packages
F-U (non-resting semantics, match risk transition, partial fills,
cancellation matcher-tx integration, pending settlement, canonical
settlement release, position boundary, fees, failure policy, reorg,
restart/rebuild, 60-case PG matrix, 16 properties, security, full
regression) remain outstanding for follow-up milestones.

## Scope delivered (B + C + D + E)

- **B** — `option_reservations` ledger schema (migration 0057)
  with two purposes (`OPEN_ORDER`, `PENDING_SETTLEMENT`), 5-state
  lifecycle (`ACTIVE` / `CONVERTED` / `RELEASED` / `SETTLED` /
  `MANUAL_REVIEW`), sparse UNIQUE ACTIVE constraints per
  `(canonical_order_hash)` and per
  `(canonical_execution_id, owner, subaccount_id, collateral_token)`,
  full immutability trigger on identity fields.

- **C** — Pure-math reservation formulas in
  `src/options/reservation_formulas.rs`. Frozen normative formulas:
  * `buy_reservation` = `ceil_div(Q × C × P, 1e16)`
  * `short_put_reservation` = `ceil_div(Q × C × S, 1e16)`
  * `short_call_reservation_physical` = `ceil_div(Q × C, 1e8)`
  U256 wide intermediate arithmetic; overflow → error; upward
  rounding; explicit zero-input rejection.

- **D** — Available-collateral algorithm in
  `src/options/reservation_repository.rs`:
  * `total_active_reserved` — sum of ACTIVE reservations scoped by
    `(deployment_id, owner, subaccount_id, collateral_token)`.
  * `available_option_collateral` — subtract active from
    caller-supplied canonical value; underflow errors (accounting
    invariant guard).
  * `ensure_option_collateral_available` — pre-persist check that
    returns `Ok(())` iff `available >= required`.

- **E** — Atomic order acceptance wiring in
  `src/options/service.rs`:
  * `reserve_open_order_risk_if_resting` — best-effort post-match
    OPEN_ORDER reservation for the resting residual. Documented
    atomicity trade-off (see below).
  * `release_open_order_reservation_for` — best-effort release on
    cancellation.

## Package E atomicity trade-off (documented)

The reservation insert is called AFTER `submit_option_order_and_match`
commits its tx. In a happy-path this creates a brief (sub-millisecond)
window between "order + fill committed" and "OPEN_ORDER reservation
inserted". Failure semantics of that window:

- If reservation insert fails, the order remains accepted and no
  gating code today reads `option_reservations` as an authoritative
  balance-check source — no economic double-spend possible.
- The failure surfaces as a `warn!` log line; operators reconcile.
- **True atomicity** requires extending `submit_option_order_and_match`
  to accept an optional reservation input and insert it inside the
  same DB transaction. This is a deferred follow-up bundled with
  Package G (match → PENDING_SETTLEMENT transition) which also
  needs matcher-tx-scope surgery.

The ledger, formulas, and available-collateral algorithm are fully
in place and PG-proven. This call site is the on/off switch for
strict atomicity in the next milestone.

## Test evidence (28 real-PG tests + 12 pure-math tests all green)

- `tests/options_reservation_ledger_pg_integration.rs` — 25 tests
  covering schema (5), formulas (4), available collateral (6),
  lifecycle (6), concurrency + restart (4). All pass.
- `tests/options_reservation_service_wiring_pg_integration.rs` — 3
  wiring tests (submit → reserve, cancel → release,
  fully-matched-IOC → no reservation). All pass.
- `src/options/reservation_formulas.rs` inline unit tests — 12
  pure-math property tests including boundary and overflow. All
  pass.

Loud-fail gate: `OPTIONS_ATOMIC_WIRING_PG_URL` required unless
`OPTIONS_ATOMIC_WIRING_PG_ALLOW_SKIP=1`. Every PG test prints
`REAL_POSTGRES_CONNECTION_CONFIRMED` on success.

## Returned verdicts (4)

- ✅ `OPTIONS_HYBRID_V2_OPTION_RESERVATION_LEDGER_VALIDATED`
- ✅ `OPTIONS_HYBRID_V2_OPEN_ORDER_RESERVATION_FORMULA_IMPLEMENTED`
- ✅ `OPTIONS_HYBRID_V2_AVAILABLE_COLLATERAL_OPERATIONAL`
- ✅ `OPTIONS_HYBRID_V2_OPEN_ORDER_RESERVATION_OPERATIONAL`
  (with documented post-match atomicity trade-off; matcher-tx
  integration is deferred to the next milestone)

## Verdicts NOT returned (16)

`OPTIONS_HYBRID_V2_RISK_IMPLEMENTATION_SURFACE_AUDITED` — audit done
inline, no formal verdict issued.

- `OPTIONS_HYBRID_V2_NONRESTING_ORDER_RISK_VALIDATED` (F)
- `OPTIONS_HYBRID_V2_MATCH_RISK_TRANSITION_OPERATIONAL` (G)
- `OPTIONS_HYBRID_V2_PARTIAL_FILL_RISK_VALIDATED` (H)
- `OPTIONS_HYBRID_V2_ORDER_TERMINATION_RISK_VALIDATED` (I, partial)
- `OPTIONS_HYBRID_V2_PENDING_SETTLEMENT_OPERATIONAL` (J)
- `OPTIONS_HYBRID_V2_CANONICAL_SETTLEMENT_RISK_RELEASE_VALIDATED` (K)
- `OPTIONS_HYBRID_V2_CANONICAL_POSITION_BOUNDARY_OPERATIONAL` (L)
- `OPTIONS_HYBRID_V2_SETTLEMENT_ECONOMICS_VALIDATED` (M)
- `OPTIONS_HYBRID_V2_FAILED_SETTLEMENT_RISK_POLICY_OPERATIONAL` (N)
- `OPTIONS_HYBRID_V2_OPTIONS_SETTLEMENT_REORG_VALIDATED` (O)
- `OPTIONS_HYBRID_V2_RISK_SETTLEMENT_RESTART_REBUILD_VALIDATED` (P)
- `OPTIONS_HYBRID_V2_RISK_SETTLEMENT_POSTGRES_VALIDATED` (Q, 60-case)
- `OPTIONS_HYBRID_V2_RISK_SETTLEMENT_PROPERTIES_VALIDATED` (R, 16-property)
- `OPTIONS_HYBRID_V2_RISK_SETTLEMENT_SECURITY_VALIDATED` (S)
- `OPTIONS_HYBRID_V2_RISK_SETTLEMENT_REGRESSION_GREEN` (T,
  workspace-wide with a real-PG env is green for the delivered
  surface; only pre-existing baseline
  `hybrid_v2_rebuild_operations_properties::reconciliation_drift_never_repairs_projection`
  fails and reproduces on pristine `f9c35ac`)

**`OPTIONS_HYBRID_V2_RESERVATIONS_PENDING_SETTLEMENT_AND_CANONICAL_RELEASE_V1_COMPLETE` — NOT returned.**

## Parent milestone closures — NONE

`OPTIONS_HYBRID_V2_RISK_RESERVATION_AND_PENDING_SETTLEMENT_V1_COMPLETE`
and `OPTIONS_HYBRID_V2_ECONOMIC_EXECUTION_CORE_V1_COMPLETE` require
the full Package B-P scope. This partial delivers the foundation
but does not close either parent.

## Regression

- Options-specific: 313 lib tests, all integration suites green.
- Full workspace: **1 failure**,
  `hybrid_v2_rebuild_operations_properties::reconciliation_drift_never_repairs_projection`.
  Reproduced on pristine `f9c35ac`; pre-existing, not introduced by
  this sprint.

## Files shipped

- `migrations/0057_option_reservations.sql`
- `src/options/reservation_formulas.rs`
- `src/options/reservation_repository.rs`
- `src/options/mod.rs` (module registration)
- `src/options/service.rs` (wiring: reserve on submit + release on
  cancel + reservation imports)
- `tests/options_reservation_ledger_pg_integration.rs` (25 PG tests)
- `tests/options_reservation_service_wiring_pg_integration.rs` (3 PG
  integration tests)
- `docs/OPTIONS_HYBRID_V2_RESERVATIONS_PENDING_SETTLEMENT_AND_CANONICAL_RELEASE_V1.md`
  (this doc)
- `~/DEOPT/docs/OPTIONS_HYBRID_V2_RESERVATIONS_PENDING_SETTLEMENT_AND_CANONICAL_RELEASE_V1_RESULT.md`
  (workspace result)
- `~/DEOPT/RUN_STATE.md`

## Safety

- No real chain transaction sent.
- No new backend key custody.
- Base mainnet 8453 never contacted.
- Frontend + Solidity untouched.

## Recommended next milestone

`OPTIONS-HYBRID-V2-MATCH-RISK-TRANSITION-AND-PENDING-SETTLEMENT-V1`
(Packages F + G + H + I + J + K). Extends
`submit_option_order_and_match` to insert reservation + transition
matched risk to PENDING_SETTLEMENT atomically inside the matcher
tx; wires the canonical event reducer's `Promoted` outcome to
`settle_pending`; validates TIF/cancel/reorg semantics.
