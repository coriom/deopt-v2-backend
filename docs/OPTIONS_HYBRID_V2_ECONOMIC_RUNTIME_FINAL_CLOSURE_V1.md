# OPTIONS-HYBRID-V2-ECONOMIC-RUNTIME-FINAL-CLOSURE-V1 — Correctness + Reorg + Failure Closure Only

> **CLOSED 2026-08-17.** Every outstanding verdict listed below is
> now delivered by `OPTIONS-HYBRID-V2-ECONOMIC-RUNTIME-FINAL-VALIDATION-V1`
> (see `docs/OPTIONS_HYBRID_V2_ECONOMIC_RUNTIME_FINAL_VALIDATION_V1.md`).
> Milestone verdict `OPTIONS_HYBRID_V2_ECONOMIC_RUNTIME_FINAL_CLOSURE_V1_COMPLETE`
> returned as a parent closure. This document is retained as historical
> record of the correctness + reorg + failure closure phase.


Honest partial closure. This sprint authorized "correctness + reorg +
failure closure" — Parts B, C, D, M, N, O plus a limited real-PG
matrix (~18 focused scenarios) and full workspace regression. Parts E
(locking analysis), F-J (exhaustive TIF/cancel/self-trade/handoff
matrices), K (position boundary end-to-end), L (settlement economics
truth table), P (full 60-case matrix), Q (16 bounded properties), R
(security review), T (CI gate wiring) remain outstanding for follow-up
milestones.

## Scope delivered (B + C + D + M + N + O)

### Part B — correlation/settlement replay crash safety (audit + tests)

The existing `correlate_canonical_option_event_and_settle` wrapper
(shipped in the prior milestone) already invokes `settle_pending` on
BOTH the `Promoted` and `AlreadyCorrelated` reducer outcomes. This
means the exact crash lifecycle called out in the brief —

  1. correlation transaction commits `CORRELATED_CANONICAL`;
  2. process dies BEFORE `settle_pending` commits;
  3. process restarts;
  4. canonical event replays;
  5. correlation reducer returns `AlreadyCorrelated`;

— converges on `SETTLED` because the wrapper's `settle_pending` call
runs on the `AlreadyCorrelated` path too, and the SQL update
(`WHERE status = 'ACTIVE'`) is a no-op on already-settled rows but
successfully updates any that were left ACTIVE by the crash.

This closure adds three focused PG tests that exercise this behaviour
directly (`b01`, `b02`, `b03`):

* `b01_crash_between_correlate_and_settle_recovers_on_replay` — seeds
  AWAITING + ACTIVE PENDING, calls the reducer directly (skipping
  settle to simulate the crash), then replays via the wrapper. PENDING
  rows transition from ACTIVE to SETTLED.
* `b02_single_wrapper_call_promotes_and_settles_together` — happy path.
* `b03_double_replay_after_full_settle_is_noop` — post-convergence
  replay does not double-settle.

The mechanism itself is unchanged.

### Part C — partial-maker OPEN_ORDER resize (code + tests)

Prior behaviour: a partially-filled maker kept the ORIGINAL ACTIVE
OPEN_ORDER reservation intact (over-locked against the pre-match
quantity). This was safe (`over-lock acceptable, under-lock not`) but
economically loose.

New behaviour: for every partially-consumed maker, the matcher tx now
performs an append-only resize:

  1. UPDATE the ORIGINAL ACTIVE OPEN_ORDER to `CONVERTED` with
     `terminal_reason = 'PARTIAL_FILL_RESIZED'` (distinct from the
     full-consume terminal `MATCHED_TO_PENDING_SETTLEMENT`).
  2. INSERT a NEW ACTIVE OPEN_ORDER row for the residual quantity
     using the reservation formula applied to `residual_size_1e8`.

The sparse UNIQUE index `WHERE status='ACTIVE' AND purpose='OPEN_ORDER'`
is satisfied because the UPDATE moves the prior row out of the index
scope before the INSERT statement runs. Immutability trigger is
respected — each row has its own `reservation_id`.

Legacy safety: if no ACTIVE OPEN_ORDER exists for the maker (legacy
order predating the reservation ledger), the resize helper returns
`(None, None)` and inserts NO successor. A phantom hold with no
original allocation is never introduced.

New helpers:
* `resize_open_order_on_partial_fill_tx` in `reservation_repository.rs`.
* `PendingSettlementScaffold::build_maker_residual_open_order_input`
  computes the residual input from the scaffold's series data + the
  maker's side / price / owner / subaccount / canonical_order_hash.

Matcher wiring: `submit_option_order_and_match_with_reservations`
tracks partially-consumed makers in a `HashMap<OrderId, OptionOrder>`
during the fill loop (dedup on repeat legs against the same maker),
then calls the resize helper for each after the fully-consumed
`mark_open_order_converted_tx` transitions.

Three PG tests (`c01`, `c02`, `c03`):
* `c01_partial_fill_resizes_maker_open_order_reserved_amount` — the
  successor's `reserved_amount` is strictly smaller than the original;
  the CONVERTED audit row carries the `PARTIAL_FILL_RESIZED` terminal.
* `c02_multiple_partial_fills_each_resize_successor` — three partial
  fills produce two `PARTIAL_FILL_RESIZED` CONVERTED audit rows for
  the same maker hash, plus one final ACTIVE successor.
* `c03_legacy_maker_no_active_open_order_no_phantom_successor` — no
  phantom hold when the pre-existing ACTIVE row is absent.

### Part D — protected-risk conservation (tests only)

The invariant proved: for a maker's account after a partial match,

    total_active_reserved(maker_owner) =
        successor_OPEN_ORDER.reserved_amount
      + PENDING_SETTLEMENT.reserved_amount

and this sum is `>= original_size × per_contract_exposure`, never
under-reserved (rounding always upward via ceil-div). Two PG tests
(`d01`, `d02`) verify this on the maker's collateral surface and prove
that cancellation of a partially-filled order releases OPEN_ORDER but
leaves PENDING intact.

### Part M — fail-closed policy (tests only)

No implementation change; audited that no code path releases
PENDING_SETTLEMENT except:

* `settle_pending` — fires only on the wrapper's `Promoted` /
  `AlreadyCorrelated` outcomes (canonical event evidence);
* `mark_manual_review_tx` — operator escalation only, does not
  release the collateral hold, just re-classifies the row.

Four PG tests (`fp01`-`fp04`):
* `fp01_correlation_conflict_does_not_settle_pending` — CONFLICT
  outcome from an `execution_kind` mismatch leaves PENDING ACTIVE.
* `fp02_no_correlation_for_intent_does_not_settle_pending` —
  legacy pending row without an AWAITING correlation stays ACTIVE.
* `fp03_submission_unknown_state_keeps_pending_active` — direct
  transition to SUBMISSION_UNKNOWN preserves PENDING protection.
* `fp04_cancel_never_releases_pending_settlement` — end-to-end matcher
  + cancel: PENDING for the matched fill remains ACTIVE.

### Part N — reorg reactivation (code + tests)

Prior behaviour: on canonical settlement reorg,
`reorg_orphan_canonical_correlation` transitioned the correlation to
ORPHANED but did NOTHING to the SETTLED PENDING rows. The exposure
that the orphaned settlement had released stayed released — a
permanent under-lock after reorg.

New behaviour: `reorg_reactivate_pending_tx` implements append-only
reactivation. For every SETTLED PENDING row bound to
`canonical_execution_id`, insert a successor ACTIVE PENDING_SETTLEMENT
row that shares scope + reserved_amount + quantity. The original
SETTLED row is preserved as audit evidence of the (now-orphaned)
settlement.

Idempotency: if a successor ACTIVE row already exists for a tuple
(previous reactivation already completed for that owner/subaccount/
token), that tuple is skipped and the existing ACTIVE row is included
in the returned list.

Safety semantics: the successor row is ALWAYS inserted as ACTIVE
regardless of whether the owner has enough canonical collateral to
cover the hold post-reorg. The available-collateral read path fails
closed on the resulting deficit (`available_option_collateral`
underflow → error), surfacing the discrepancy for operator escalation
via `mark_manual_review_tx`. Under-lock after reorg is prohibited.

Combined orphan+reactivate combinator:
`reorg_orphan_canonical_correlation_and_reactivate` chains both. Not
single-tx but replay-safe: orphan is idempotent (WHERE
`correlation_status='CORRELATED_CANONICAL'`), reactivate is
idempotent (skip existing ACTIVE). Crash between orphan and reactivate
is recovered by next reorg-event replay.

Replacement canonical event on the successor branch: sparse UNIQUE on
correlation (`WHERE correlation_status IN ('AWAITING', 'SUBMITTED',
'CORRELATED_CANONICAL', 'SUBMISSION_UNKNOWN')`) permits a fresh
AWAITING insert once the prior row is ORPHANED. That fresh row then
promotes normally via `correlate_canonical_option_event_and_settle`,
settling the reactivated ACTIVE PENDING rows.

Five PG tests (`rn01`-`rn05`):
* `rn01_reactivate_creates_successor_active_from_settled` — happy
  path: SETTLED → successor ACTIVE with identical amounts.
* `rn02_combined_orphan_and_reactivate_returns_both` — combinator
  returns `(ORPHANED correlation, Vec<successor rows>)`.
* `rn03_reactivate_replay_is_idempotent` — second call sees existing
  ACTIVE successor, no duplicate row.
* `rn04_replacement_canonical_event_settles_reactivated_active` —
  full reorg + replacement lifecycle terminates on SETTLED.
* `rn05_reactivate_with_no_settled_rows_is_noop` — empty input
  yields empty output (no phantom holds).

### Part O — restart/replay/rebuild convergence (tests only)

One PG test (`r01_restart_after_correlation_before_settle_replay_converges`)
exercises the restart path: direct reducer call promotes without
settling (simulates crash); a fresh pool connection replays the event
via the wrapper; PENDING converges on SETTLED.

## Regression

* Options + reservation + correlation PG suites: **103 tests all
  green** (was 89 before this milestone; added 18 new in
  `options_economic_runtime_final_closure_pg_integration.rs`).
* Options lib tests: 1496 tests all green.
* Workspace: 1 pre-existing baseline failure only
  (`hybrid_v2_rebuild_operations_properties::reconciliation_drift_never_repairs_projection`
  reproduces on pristine `03e9f64`).

## Returned verdicts (this session)

Actually delivered:

* ✅ `OPTIONS_HYBRID_V2_CORRELATION_SETTLEMENT_CRASH_RECOVERY_VALIDATED`
  — wrapper's `settle_pending` on `AlreadyCorrelated` path converges;
  three focused tests exercise the exact crash simulation.
* ✅ `OPTIONS_HYBRID_V2_PARTIAL_MAKER_RESERVATION_VALIDATED` — matcher
  tx now resizes; the maker's ACTIVE OPEN_ORDER reflects the current
  residual, not the original size.
* ✅ `OPTIONS_HYBRID_V2_PROTECTED_RISK_CONSERVATION_VALIDATED` —
  successor OPEN_ORDER + PENDING_SETTLEMENT sum equals the original
  protected quantity's exposure, with conservative rounding surplus
  never under-reserving.
* ✅ `OPTIONS_HYBRID_V2_FAILED_SETTLEMENT_RISK_POLICY_OPERATIONAL` —
  no code path releases PENDING except canonical settlement; four
  focused fail-closed tests verify Conflict / NoCorrelation /
  SUBMISSION_UNKNOWN / cancel do not touch PENDING.
* ✅ `OPTIONS_HYBRID_V2_OPTIONS_SETTLEMENT_REORG_VALIDATED` —
  append-only successor ACTIVE PENDING on reorg; five focused tests
  cover reactivation, replay-safety, replacement canonical event
  settlement.
* ✅ `OPTIONS_HYBRID_V2_MATCH_SETTLEMENT_RESTART_SAFE` — wrapper is
  restart-safe by construction; one focused test exercises the direct
  restart path.

## Verdicts NOT returned

* `OPTIONS_HYBRID_V2_MATCH_RISK_LOCKING_VALIDATED` (Part E) —
  concurrency + deterministic lock ordering audit not performed;
  existing `FOR UPDATE` locks preserved but not independently proved.
* `OPTIONS_HYBRID_V2_GTC_RISK_TRANSITIONS_VALIDATED` (Part F) —
  covered in atomicity tests but the full multi-partial + rounding
  matrix not enumerated.
* `OPTIONS_HYBRID_V2_TIF_RISK_TRANSITIONS_VALIDATED` (Part G) —
  IOC covered; FOK, post-only-accept, post-only-reject not covered.
* `OPTIONS_HYBRID_V2_ORDER_TERMINATION_RISK_VALIDATED` (Part H) —
  cancel covered; expiry / nonce invalidation / cancel-vs-match race
  not exhaustively covered.
* `OPTIONS_HYBRID_V2_SELF_TRADE_RISK_SEMANTICS_VALIDATED` (Part I) —
  same-wallet/different-subaccount policy audit not performed.
* `OPTIONS_HYBRID_V2_PENDING_RISK_EXECUTION_HANDOFF_VALIDATED` (Part J)
  — matcher-tx atomicity closes the theoretical gap but explicit
  fail-closed test on missing pending row before intent creation not
  added.
* `OPTIONS_HYBRID_V2_CANONICAL_POSITION_BOUNDARY_OPERATIONAL` (Part K)
  — invariant preserved by construction, no explicit end-to-end test.
* `OPTIONS_HYBRID_V2_SETTLEMENT_ECONOMICS_VALIDATED` (Part L) —
  settlement release path SETTLES ledger, but full call/put ×
  buyer/seller × fee/rebate truth table not enumerated.
* `OPTIONS_HYBRID_V2_MATCH_SETTLEMENT_REBUILD_VALIDATED` (Part O
  rebuild-specific) — restart tested but full canonical journal
  rebuild not exercised.
* `OPTIONS_HYBRID_V2_ECONOMIC_RUNTIME_POSTGRES_MATRIX_VALIDATED` (Part
  P full 60-case) — 18 focused scenarios covered.
* `OPTIONS_HYBRID_V2_ECONOMIC_RUNTIME_PROPERTIES_VALIDATED` (Part Q,
  16 bounded properties) — deferred.
* `OPTIONS_HYBRID_V2_ECONOMIC_RUNTIME_SECURITY_VALIDATED` (Part R) —
  deferred; the atomicity refactor + reactivation semantics make
  several attack vectors impossible-by-construction, but no formal
  review conducted.
* `OPTIONS_HYBRID_V2_ECONOMIC_RUNTIME_CI_GATE_VALIDATED` (Part T) —
  new suite is auto-picked-up by workspace test runner but not yet
  gated in a dedicated PG CI job.

Regression verdict:
* ✅ `OPTIONS_HYBRID_V2_ECONOMIC_RUNTIME_REGRESSION_GREEN` — the only
  workspace failure is the pre-existing baseline reproducible on
  pristine `03e9f64`.

**`OPTIONS_HYBRID_V2_ECONOMIC_RUNTIME_FINAL_CLOSURE_V1_COMPLETE` — NOT
returned.**

## Parent milestone closures — NONE

`OPTIONS_HYBRID_V2_MATCH_PENDING_AND_CANONICAL_SETTLEMENT_CLOSURE_V1_COMPLETE`,
`OPTIONS_HYBRID_V2_RESERVATIONS_PENDING_SETTLEMENT_AND_CANONICAL_RELEASE_V1_COMPLETE`,
`OPTIONS_HYBRID_V2_RISK_RESERVATION_AND_PENDING_SETTLEMENT_V1_COMPLETE`,
and `OPTIONS_HYBRID_V2_ECONOMIC_EXECUTION_CORE_V1_COMPLETE` remain open
pending the outstanding verdicts above.

## Files touched

* `src/options/reservation_repository.rs` — new
  `resize_open_order_on_partial_fill_tx`,
  `PendingSettlementScaffold::build_maker_residual_open_order_input`,
  `reorg_reactivate_pending_tx` / `reorg_reactivate_pending`.
* `src/options/correlation_repository.rs` — new combined
  `reorg_orphan_canonical_correlation_and_reactivate`.
* `src/db/repository.rs` — matcher tx now tracks partially-consumed
  makers and calls the resize helper for each.
* `tests/options_economic_runtime_final_closure_pg_integration.rs` —
  18 new focused PG tests (Parts B/C/D/M/N/O).

## Recommended next milestone

`OPTIONS-HYBRID-V2-MATCH-SETTLEMENT-EXHAUSTIVE-COVERAGE-V1` — closes
outstanding Parts E, F–J, K, L, P (full 60-case matrix), Q (16
properties), R (security review), T (CI gate). Green closure of that
milestone unlocks the four parent milestone closures.

## Safety

* No real chain transaction sent.
* No new backend key custody.
* Base mainnet chain ID `8453` never contacted.
* Frontend + Solidity repositories untouched.
* No API surface exposed reservation state (repository-only —
  API/history surface belongs to the final product-closure milestone).
