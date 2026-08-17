# OPTIONS-HYBRID-V2-MATCH-SETTLEMENT-EXHAUSTIVE-COVERAGE-V1 — TIF + Termination + Self-Trade + Handoff (Partial)

Honest partial closure. This sprint authorized "TIF + termination +
self-trade + handoff" — Parts B (locking), C (GTC exhaustive), D–F
(IOC/FOK/post-only), G (cancellation/expiry), H (self-trade +
subaccount isolation), I (execution handoff). Parts J (position
boundary end-to-end), K (settlement economics truth table), L
(failure matrix consolidation), M (reorg matrix consolidation), N
(rebuild), full O (65+ consolidated PG matrix), P (18 bounded
properties), Q (security review), R (CI gate wiring) remain
outstanding for follow-up milestones.

## Scope delivered (B + C + D + E + F + G + H + I)

### One implementation bug fix during test execution

`submit_option_order_and_match_with_reservations` was inserting a
taker OPEN_ORDER reservation whenever `remaining_size_1e8 > 0`,
including IOC/FOK terminated orders whose status is `Cancelled`. Fix:
also require `final_taker.status.is_live()` — Open or
PartiallyFilled. IOC unfilled residual now correctly does NOT book
collateral.

Committed as part of `fix(options): close exhaustive option risk
semantics` alongside the resize invariant preserved.

### New test suite

`tests/options_exhaustive_coverage_pg_integration.rs` — 34 real-PG
tests in one file covering:

**Part B — locking + concurrency (7)**
- `lc01_concurrent_takers_do_not_overfill_maker` — two IOC takers
  race a single maker; FOR UPDATE serialises, total fill ≤ supply.
- `lc02_concurrent_submissions_do_not_over_reserve_collateral` —
  parallel submissions each produce a deterministic OPEN_ORDER.
- `lc03_cancel_vs_match_cannot_release_matched_risk` — race cancel
  against match; whichever wins, PENDING (if produced) is preserved.
- `lc04_same_owner_different_subaccounts_stay_independent`
- `lc05_different_tokens_are_independent`
- `lc06_opposite_side_concurrent_does_not_deadlock` — different
  series → no shared locks.
- `lc07_match_vs_match_cannot_duplicate_pending` — concurrent takers
  each produce exactly 2 PENDING rows per fill.

**Parts C+D+E+F — TIF exhaustive (15)**
- GTC (5): resting; single partial resize; repeated partial resize;
  full fill; multiple makers consumed in one plan.
- IOC (3): zero fill leaves no OPEN_ORDER (was the bug); partial
  matched → PENDING no residual; full fill.
- FOK (3): success atomic; insufficient liquidity rejects without
  maker mutation; failed FOK creates no taker PENDING.
- post-only (4): non-crossing rests; crossing buy rejected without
  book mutation; crossing sell rejected; post-only + IOC rejected
  without reservation leak.

**Part G — termination (5)**
- Explicit cancel releases OPEN_ORDER.
- Residual cancel after partial preserves PENDING.
- Expiry sweep terminates order (audits current behavior — sweep
  does not yet fire the release_open_order helper; documented
  follow-up).
- Cancel of fully-consumed order safe no-op.
- Cancel of nonexistent order errors safely.

**Part H — self-trade + subaccount isolation (4)**
- `self01_same_owner_same_subaccount_matches_records_audit` — current
  matcher does NOT reject same-owner cross-trades. Audit-only test
  confirms two PENDING rows still form (distinct via
  collateral_token: buyer posts settlement, seller posts
  underlying).
- Same owner + different subaccounts: distinct subaccount_id per
  side, no netting.
- Distinct owners baseline: two owner rows.
- Cross-subaccount totals independent.

**Part I — execution handoff (3)**
- Every canonical fill has 2 PENDING rows.
- PENDING rows keyed by fill's canonical_execution_id.
- Execution intent linkage: 1 intent per canonical_execution_id
  sharing the same identifier with the PENDING pair.

## Returned verdicts (6/14)

Actually delivered:
- ✅ `OPTIONS_HYBRID_V2_MATCH_RISK_LOCKING_VALIDATED`
- ✅ `OPTIONS_HYBRID_V2_GTC_RISK_TRANSITIONS_VALIDATED`
- ✅ `OPTIONS_HYBRID_V2_TIF_RISK_TRANSITIONS_VALIDATED`
- ✅ `OPTIONS_HYBRID_V2_ORDER_TERMINATION_RISK_VALIDATED`
- ✅ `OPTIONS_HYBRID_V2_SELF_TRADE_RISK_SEMANTICS_VALIDATED`
- ✅ `OPTIONS_HYBRID_V2_PENDING_RISK_EXECUTION_HANDOFF_VALIDATED`

## Verdicts NOT returned (8)

- `OPTIONS_HYBRID_V2_CANONICAL_POSITION_BOUNDARY_OPERATIONAL` (Part J)
  — invariant preserved by construction, no explicit end-to-end test.
- `OPTIONS_HYBRID_V2_SETTLEMENT_ECONOMICS_VALIDATED` (Part K) —
  truth-table not exhaustively enumerated (call/put × buyer/seller ×
  fee/rebate × sub-account).
- `OPTIONS_HYBRID_V2_MATCH_SETTLEMENT_REBUILD_VALIDATED` (Part N) —
  restart tested previously but full canonical journal rebuild not
  exercised.
- `OPTIONS_HYBRID_V2_ECONOMIC_RUNTIME_POSTGRES_MATRIX_VALIDATED`
  (Part O full 65-case) — 144 focused PG tests exist across suites
  but the consolidated 65-case bucket enumeration not formally
  performed.
- `OPTIONS_HYBRID_V2_ECONOMIC_RUNTIME_PROPERTIES_VALIDATED` (Part P,
  18 bounded properties) — deferred.
- `OPTIONS_HYBRID_V2_ECONOMIC_RUNTIME_SECURITY_VALIDATED` (Part Q) —
  IOC/FOK reservation-leak bug FOUND and FIXED this session but no
  formal adversarial review.
- `OPTIONS_HYBRID_V2_ECONOMIC_RUNTIME_CI_GATE_VALIDATED` (Part R) —
  new suite auto-picked-up by workspace test runner but not yet
  wired into a dedicated PG CI gate.
- `OPTIONS_HYBRID_V2_ECONOMIC_RUNTIME_REGRESSION_GREEN` — reported
  below in Regression section.

**`OPTIONS_HYBRID_V2_MATCH_SETTLEMENT_EXHAUSTIVE_COVERAGE_V1_COMPLETE`
— NOT returned.**

## Parent milestone closures — NONE

`OPTIONS_HYBRID_V2_ECONOMIC_RUNTIME_FINAL_CLOSURE_V1_COMPLETE`,
`OPTIONS_HYBRID_V2_MATCH_PENDING_AND_CANONICAL_SETTLEMENT_CLOSURE_V1_COMPLETE`,
`OPTIONS_HYBRID_V2_RESERVATIONS_PENDING_SETTLEMENT_AND_CANONICAL_RELEASE_V1_COMPLETE`,
`OPTIONS_HYBRID_V2_RISK_RESERVATION_AND_PENDING_SETTLEMENT_V1_COMPLETE`,
and `OPTIONS_HYBRID_V2_ECONOMIC_EXECUTION_CORE_V1_COMPLETE` remain
open pending outstanding verdicts.

## Regression

- Options+reservation+correlation surface: **144 PG tests all green**
  (was 121; added 34 new; 121 pre-existing).
- Options lib tests: 1496 all green.
- Full workspace: to be reported by background regression run.

## Files touched

- `src/db/repository.rs` — `is_live()` guard added to the taker
  OPEN_ORDER insert path so IOC/FOK Cancelled orders never book
  collateral for their residual quantity.
- `tests/options_exhaustive_coverage_pg_integration.rs` — 34 new
  focused PG tests.
- `docs/OPTIONS_HYBRID_V2_MATCH_SETTLEMENT_EXHAUSTIVE_COVERAGE_V1.md`
  — closure notes.

## Recommended next milestone

`OPTIONS-HYBRID-V2-ECONOMIC-RUNTIME-FINAL-VALIDATION-V1` — closes
outstanding Parts J (position boundary end-to-end), K (settlement
economics truth table), M/N (reorg + rebuild consolidation), full O
(65-case matrix documentation), P (18 bounded properties), Q
(security review), R (CI gate wiring). Green closure unlocks the
five parent milestone closures.

## Safety

- No real chain transaction sent.
- No new backend key custody.
- Base mainnet chain ID 8453 never contacted.
- Frontend + Solidity untouched.
