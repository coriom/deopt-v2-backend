# OPTIONS-HYBRID-V2-MATCH-PENDING-AND-CANONICAL-SETTLEMENT-CLOSURE-V1 — Core Economic Closure Only

Honest partial closure. This sprint authorized "Core economic
closure" — Parts B + D + E + K plus focused PG tests. Parts C
(concurrency/locking analysis), F–J (TIF/cancel/self-trade
validation matrices), L–P (position boundary, fees, failure, reorg,
restart), and the full Q/R/S/T/U sweep (55-case matrix, 14 properties,
security review, workspace-wide regression sweep, docs updates) remain
outstanding for follow-up milestones.

## Scope delivered (B + D + E + K)

- **B (atomicity refactor)** — `submit_option_order_and_match` now
  accepts an optional `MatcherReservationPlan`. When present, the
  entire order + fills + reservation state changes commit in ONE
  PostgreSQL transaction. The pre-existing post-match reservation
  window is REMOVED — no downstream code path can observe an accepted
  order without its OPEN_ORDER reservation (or a fill without its
  PENDING_SETTLEMENT rows). Legacy in-memory / no-repository callers
  pass `None` and preserve the old no-reservation behaviour.

- **D (match risk transition inside matcher)** — for each fill leg
  the matcher tx:
  1. Inserts the taker's OPEN_ORDER reservation IF residual > 0 (no
     residual → no OPEN_ORDER, fully matched).
  2. Transitions every fully-consumed maker's ACTIVE OPEN_ORDER to
     CONVERTED (via `mark_open_order_converted_tx`).
  3. Inserts two PENDING_SETTLEMENT rows per fill — one for the buyer
     (settlement asset, ceil-div premium), one for the seller
     (underlying for physical calls, settlement asset for puts).
  Partial maker fills LEAVE the maker's OPEN_ORDER ACTIVE (immutable
  reserved_amount) — conservative temporary over-lock, safe per the
  brief's "over-lock acceptable, under-lock not" invariant.

- **E (partial-fill rounding conservation)** — the policy is per-fill
  independent ceil-div: each PENDING_SETTLEMENT row computes its
  reserved amount as `ceil_div(fill.size × contract_size × price,
  1e16)` for buyers, and the corresponding short-call physical or
  short-put formula for sellers. Sum across partial fills of the same
  original order may exceed a naive single-fill computation by up to
  N raw units (one per fill); this is safe because ceil-div always
  rounds UP, never under-reserving. Proven in PG tests r01, r02.

- **K (canonical settlement release wired to reservation ledger)** —
  new `correlate_canonical_option_event_and_settle` combines the
  existing correlation reducer with `settle_pending`. When the event
  Promotes a correlation, all ACTIVE PENDING_SETTLEMENT rows for that
  canonical_execution_id transition to SETTLED atomically enough:
  each function is idempotent (Promoted vs AlreadyCorrelated; SETTLED
  UPDATE WHERE `status = 'ACTIVE'` is a no-op on replay). Crash
  between promote and settle is recovered by replay.

## Removed post-commit reservation window

Before this milestone: `submit_option_order_inner` called the matcher
first, then invoked `reserve_open_order_risk_if_resting` as a
best-effort post-commit insert. That helper is now DELETED. The
reservation write happens inside the matcher tx or not at all.

Old helper `derive_reservation_inputs` (which was subtly wrong —
computed from `remaining_size_1e8` instead of the original `size_1e8`
that the reservation must cover) also deleted; replaced by
`derive_taker_reservation_inputs` in the plan-builder that uses the
correct `size_1e8` field.

## PG test evidence (12 tests, all green)

`tests/options_match_pending_settlement_pg_integration.rs` — 9 tests:
- Atomicity (4): resting-order atomic commit, fully-matched taker
  behaviour, partial-fill maker OPEN_ORDER retention, cancel isolates
  OPEN_ORDER from PENDING.
- Canonical settlement release (3): correlate+settle promotes and
  settles buyer + seller; duplicate event idempotent; no-correlation
  returns no settlement.
- Rounding conservation (2): per-fill ceil-div; two-fill sum equals
  full-quantity buy_reservation for exact inputs.

`tests/options_reservation_service_wiring_pg_integration.rs` — 3
tests updated to use unique per-run client_order_ids: submit creates
OPEN_ORDER (via new atomic path), cancel releases, fully-matched IOC
leaves no taker OPEN_ORDER (correctness preserved through matcher
refactor).

Full reservation + correlation regression: **89 tests all green**
(atomic-wiring 26, atomic-wiring properties 6, canonical event
reducer 10, pre-broadcast tx identity 7, correlation repository 12,
reservation ledger 25, reservation wiring 3).

## Returned verdicts (5/19)

- ✅ `OPTIONS_HYBRID_V2_ORDER_RESERVATION_MATCH_ATOMICITY_VALIDATED`
- ✅ `OPTIONS_HYBRID_V2_PENDING_SETTLEMENT_TRANSITION_VALIDATED`
- ✅ `OPTIONS_HYBRID_V2_PARTIAL_FILL_ROUNDING_CONSERVATION_VALIDATED`
- ✅ `OPTIONS_HYBRID_V2_CANONICAL_SETTLEMENT_RISK_RELEASE_VALIDATED`
- ✅ `OPTIONS_HYBRID_V2_MATCH_PENDING_SETTLEMENT_REGRESSION_GREEN`
  (Options + reservation + correlation surface clean; only
  pre-existing baseline `hybrid_v2_rebuild_operations_properties::reconciliation_drift_never_repairs_projection`
  fails and reproduces on pristine `1f422c0`)

## Verdicts NOT returned (14)

- `OPTIONS_HYBRID_V2_MATCH_RISK_LOCKING_VALIDATED` — Part C
  concurrency + deterministic lock ordering analysis not performed
  this session (existing FOR UPDATE locks preserved but not
  independently audited).
- `OPTIONS_HYBRID_V2_GTC_RISK_TRANSITIONS_VALIDATED` — GTC covered
  in atomicity tests; full multi-partial + rounding-conservation
  matrix not exhaustively enumerated.
- `OPTIONS_HYBRID_V2_TIF_RISK_TRANSITIONS_VALIDATED` — IOC covered
  by one test; FOK, post-only-accept, post-only-reject not covered.
- `OPTIONS_HYBRID_V2_ORDER_TERMINATION_RISK_VALIDATED` — cancel
  covered; expiry / nonce invalidation / cancel-vs-match race NOT
  exhaustively covered.
- `OPTIONS_HYBRID_V2_SELF_TRADE_RISK_SEMANTICS_VALIDATED` — not
  audited this session.
- `OPTIONS_HYBRID_V2_PENDING_RISK_EXECUTION_HANDOFF_VALIDATED` —
  matcher-tx atomicity closes the theoretical gap but explicit
  fail-closed test on missing pending row before intent creation
  not added.
- `OPTIONS_HYBRID_V2_CANONICAL_POSITION_BOUNDARY_OPERATIONAL` — the
  invariant is preserved (off-chain match writes nothing to
  hybrid_v2_positions) but no explicit end-to-end test enumerates it.
- `OPTIONS_HYBRID_V2_SETTLEMENT_ECONOMICS_VALIDATED` — canonical
  settlement release SETTLES the reservation ledger but the
  premium/fee/transfer canonical reducer wiring is unchanged (out of
  scope this session).
- `OPTIONS_HYBRID_V2_FAILED_SETTLEMENT_RISK_POLICY_OPERATIONAL` —
  PENDING_SETTLEMENT correctly persists through submission failures
  by construction (no release path exists that fires on
  submission-failure signals) but no explicit failure-policy test
  matrix.
- `OPTIONS_HYBRID_V2_OPTIONS_SETTLEMENT_REORG_VALIDATED` — canonical
  reorg reducer exists but reservation-side reactivation policy not
  designed / implemented.
- `OPTIONS_HYBRID_V2_MATCH_SETTLEMENT_RESTART_SAFE` — restart
  covered for the correlation subsystem; reservation-side restart
  matrix not enumerated.
- `OPTIONS_HYBRID_V2_MATCH_PENDING_SETTLEMENT_POSTGRES_VALIDATED`
  (55-case) — 12 focused cases covered; the full 55-case scenario
  matrix (concurrent takers, cross-token isolation, FOK, post-only
  reject, insufficient-collateral rejection, etc.) not enumerated.
- `OPTIONS_HYBRID_V2_MATCH_PENDING_SETTLEMENT_PROPERTIES_VALIDATED`
  (14 properties) — deferred.
- `OPTIONS_HYBRID_V2_MATCH_PENDING_SETTLEMENT_SECURITY_VALIDATED` —
  deferred; the atomicity refactor makes several attack vectors from
  the security checklist impossible-by-construction (post-commit
  reservation gap, match-without-risk, cancel-releases-pending), but
  no formal review conducted.

**`OPTIONS_HYBRID_V2_MATCH_PENDING_AND_CANONICAL_SETTLEMENT_CLOSURE_V1_COMPLETE` — NOT returned.**

## Parent milestone closures — NONE

`OPTIONS_HYBRID_V2_RESERVATIONS_PENDING_SETTLEMENT_AND_CANONICAL_RELEASE_V1_COMPLETE`,
`OPTIONS_HYBRID_V2_RISK_RESERVATION_AND_PENDING_SETTLEMENT_V1_COMPLETE`,
and `OPTIONS_HYBRID_V2_ECONOMIC_EXECUTION_CORE_V1_COMPLETE` remain
open pending the outstanding verdicts above.

## Files touched

- `src/db/repository.rs` — extended `submit_option_order_and_match`
  with atomic reservation writes + full-match maker CONVERTED
  transition + PENDING_SETTLEMENT per-fill inserts.
- `src/options/reservation_repository.rs` — new
  `MatcherReservationPlan` + `PendingSettlementScaffold` types + the
  `build_pending_pair` helper that applies formulas per fill.
- `src/options/correlation_repository.rs` — new
  `correlate_canonical_option_event_and_settle` combining reducer
  with pending settlement release.
- `src/options/service.rs` — deleted old post-match helpers
  (`reserve_open_order_risk_if_resting`, `derive_reservation_inputs`).
  Replaced with `build_matcher_reservation_plan` +
  `derive_taker_reservation_inputs`. Matcher call updated to pass
  reservation plan.
- `tests/options_match_pending_settlement_pg_integration.rs` — 9 new
  PG tests covering atomicity, canonical settlement release, rounding
  conservation.
- `tests/options_reservation_service_wiring_pg_integration.rs` —
  updated to unique-per-run client_order_ids so tests re-run cleanly
  against the shared PG DB.

## Recommended next milestone

`OPTIONS-HYBRID-V2-MATCH-SETTLEMENT-EXHAUSTIVE-COVERAGE-V1` — covers
outstanding Parts C, F–J, L–P (concurrency + TIF exhaustive coverage +
self-trade + position/fee/failure/reorg/restart) plus the full 55-case
PG matrix and 14 bounded properties. That milestone closes the three
parent milestones once green.

## Safety

- No real chain transaction sent.
- No new backend key custody.
- Base mainnet 8453 never contacted.
- Frontend + Solidity untouched.
- No API surface exposed reservation state (still repository-only —
  Package E API surface work belongs to the final closure milestone).
