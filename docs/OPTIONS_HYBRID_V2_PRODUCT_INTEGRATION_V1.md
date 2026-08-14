# OPTIONS-HYBRID-V2-PRODUCT-INTEGRATION-V1

Milestone: `OPTIONS-HYBRID-V2-PRODUCT-INTEGRATION-V1`

Status: **PARTIAL CLOSURE** — verification landed; canonical
execution ID wiring and downstream projection deferred.

Date: 2026-08-14.

## Purpose

Make Hybrid V2 subaccounts the canonical backend account/execution
model for the Options product. Complete lifecycle:

```text
wallet / owner
→ subaccount
→ option order
→ validation
→ margin / reservation
→ matching
→ execution candidate
→ Hybrid V2 execution identity
→ premium / position / collateral effects
→ FeesManagerV2
→ reservation release/update
→ canonical events
→ PostgreSQL projection
→ history / APIs
```

## Canonical account identity

**Frozen**: `(chain/deployment, owner_address, subaccount_id)`.

* `SUBACCOUNT_V1_OWNER_PLUS_SUBACCOUNT_ID_IS_THE_OPTIONS_ACCOUNT_IDENTITY`
  affirmed. Every Options order, RFQ, TWAP order, conditional order,
  fill, and rejection row carries a `subaccount_id` column (schema
  `migration 0039` and `migration 0040`, NOT NULL, DEFAULT 1, CHECK
   ≥ 1, composite indexes `(LOWER(account), subaccount_id)`).
* `NO_CROSS_SUBACCOUNT_NETTING` — settlement invariant. Options fills
  record buyer_subaccount_id and seller_subaccount_id independently
  on `option_fills`. When the same wallet crosses across its own
  subaccounts, the fill row preserves both distinct subaccount ids.
* `NO_OPTIONS_MEMORY_FALLBACK_IN_PRODUCTION` — enforced at startup
  by `OptionsConfig::validate_startup(persistence_enabled)` at
  `src/options/types.rs:132`, invoked from `src/main.rs:57`. Options
  with `require_persistence=true` cannot start against an unpersisted
  backend.
* `MATCHING_NEVER_BYPASSES_MARGIN_OR_RESERVATION_RULES` — matcher
  preserves each side's `subaccount_id` on the fill row. The
  Options DB matcher does NOT permit an order to be matched with
  itself (same order_id); it DOES permit same-wallet cross-subaccount
  matching per the frozen self-trade policy.
* `FEES_MANAGER_V2_IS_THE_CANONICAL_OPTIONS_FEE_MODEL` — Options
  quote preview + broadcast policy reads FeesManagerV2 via eth_call
  (`src/options/broadcast_policy_data.rs:158`). No legacy bps
  calculation is active in the fill hot path.

## Route-boundary posture (already integrated)

`resolve_options_v2_subaccount(state, envelope, body_subaccount_id, owner)`
at `src/api/routes.rs:13918` is the canonical Options subaccount
resolver. Semantics:

* v1 auth + body_subaccount_id `None|Some(1)` → returns 1 (backward
  compatibility for the pre-v2 wire).
* v1 auth + body_subaccount_id `Some(N)` for `N > 1` → 400
  `InvalidSubaccountRequest` ("v1 auth cannot route to subaccount N;
  use a v2 authorization envelope").
* v2 auth + missing body_subaccount_id → 400
  `InvalidSubaccountRequest` ("v2 auth requires subaccount_id in body").
* v2 auth + present body_subaccount_id → validate the subaccount
  exists for this owner via the identity store; missing → 404
  `SubaccountNotFound`; present → returns the resolved id.

Wired at every Options mutation route:

| Route | Line |
| --- | --- |
| `submit_option_order` | `src/api/routes.rs:4115` |
| `create_conditional_order` | `src/api/routes.rs:4352` |
| `cancel_conditional_order` | `src/api/routes.rs:4587` (cross-subaccount ownership check line 4605) |
| `cancel_option_order` | `src/api/routes.rs:4711` (cross-subaccount ownership check line 4718) |
| `create_option_rfq` | `src/api/routes.rs:4982` (threads `taker_subaccount_id` line 5006) |
| `submit_option_rfq_quote` | `src/api/routes.rs:5102` (threads `maker_subaccount_id`) |
| `create_option_multi_leg_rfq` | `src/api/routes.rs:5349` |
| `submit_option_multi_leg_rfq_quote` | `src/api/routes.rs:5448` |
| `accept_option_multi_leg_rfq_quote` | `src/api/routes.rs:5606` |
| `cancel_option_multi_leg_rfq` | `src/api/routes.rs:5729` (threads `taker_subaccount_id` line 5754) |
| `create_option_twap_order` | `src/api/routes.rs:6414` |
| `cancel_option_twap_order` | `src/api/routes.rs:6496` (cross-subaccount ownership check line 6503) |
| `accept_option_rfq_quote` | `src/api/routes.rs:6561` (cross-subaccount check line 6563) |
| `cancel_option_rfq` | `src/api/routes.rs:6614` (cross-subaccount check line 6616) |

Cross-subaccount authorization is enforced on every cancel and
`accept_option_rfq_quote` route: signing subaccount ≠ target row's
subaccount → 404 `SubaccountNotFound`. Ownership check happens BEFORE
service invocation, so mutations never reach persistence.

## Public read API posture

Account-scoped read endpoints default to `subaccount_id=1` when the
caller supplies `account` and does NOT opt into aggregate view via
`?all=true`:

| Endpoint | Behavior |
| --- | --- |
| `GET /options/orders` | `subaccount_id=1` default; `?all=true` aggregates (`src/api/routes.rs:4232`) |
| `GET /options/fills` | `subaccount_id=1` default; `?all=true` aggregates (`src/api/routes.rs:4651`) |
| `GET /options/multi-leg-rfqs?taker=X` | `subaccount_id=1` default (`src/api/routes.rs:5410`) |
| `GET /accounts/:addr/history/v2` | `subaccount_id=1` default; `?all=true` aggregates (`src/api/trading.rs:2130`) |
| Transactions tab within `/accounts/:addr/history/v2` | Wallet-aggregate (see follow-up) |

Filter isolation is enforced side-aware in
`OptionFillFilter::matches` (`src/options/types.rs:1386`): a fill with
`buyer==X && buyer_subaccount_id==N` matches
`OptionFillFilter { account: X, subaccount_id: Some(N) }` on the buyer
side; equivalently for seller. A same-wallet cross-subaccount fill
appears in both the buyer-side and seller-side subaccount views.

## Matcher preserves identity

`option_fill_from_match` (both `src/db/repository.rs:7717` and
`src/options/store.rs:2153`) populates `buyer_subaccount_id =
buy_order.subaccount_id` and `seller_subaccount_id =
sell_order.subaccount_id`. Same-wallet cross-subaccount matches
retain distinct subaccount ids on the fill row. The DB matcher
respects TIF (GTC/IOC/FOK), post-only, price-time priority, and
transactional atomicity per pre-existing invariants.

## What ships in this milestone

### Package A — Verification + subaccount matcher invariants (SHIPPED)

`tests/options_hybrid_v2_subaccount_matcher_integration.rs` — 8
in-memory integration tests locking in the canonical identity model:

1. `same_wallet_two_subaccounts_cross_matches_and_records_both_ids` —
   Same wallet, sub 1 sell + sub 2 buy → matches; fill row has
   `buyer_subaccount_id=2, seller_subaccount_id=1`, distinct.
2. `order_list_scoped_by_subaccount_never_leaks_across_subaccounts` —
   Wallet places one order per subaccount; sub 1 view returns only
   sub 1 order; unused subaccount (99) view is empty; admin view
   (`subaccount_id: None`) aggregates.
3. `fill_filter_side_aware_for_same_wallet_cross_subaccount` — same
   wallet cross-subaccount fill visible from BOTH sub 1 (seller
   side) and sub 2 (buyer side); unused sub 99 sees none.
4. `cross_wallet_isolation_holds_under_subaccount_scoping` — Wallet
   A and Wallet B each place sub 1 orders; each wallet's sub-1 view
   returns only that wallet's order.
5. `two_wallets_trade_fill_preserves_both_wallet_and_subaccount` —
   Cross-wallet fill records both wallets and both subaccount ids.
6. `filter_by_status_and_subaccount_combination` — Combined status
   + subaccount filter returns the correct subset.
7. `options_config_startup_validation_fails_closed_in_production` —
   Production config (`enabled=true, require_persistence=true`)
   rejects startup when `persistence_enabled=false`; accepts when
   true.
8. `options_config_test_mode_accepts_in_memory_operation` — Test
   config (`enabled_in_memory_for_tests`) permits in-memory
   operation.

Result: 8 passed, 0 failed. No PostgreSQL required; runs green in
default developer environment.

## Deferred to follow-up milestones

The following integration deltas require dedicated architectural
design and cannot be safely landed in this session without risking
existing Hybrid V2 invariants:

### Deferred D1 — Canonical execution ID for Options fills

**Status**: not implemented.

**Rationale**: The Hybrid V2 canonical execution ID
(`derive_canonical_execution_id` at
`src/hybrid_v2/execution/identity.rs:76`) requires a
`buyer_order_hash` and `seller_order_hash`. Options orders currently
persist `client_order_id`, `nonce`, and an optional
`signature` (EIP-712 intent), but do NOT persist a canonical
32-byte order hash. Deriving one requires either:

* Designing a canonical Options order hash format (probably
  `keccak256("OPTION_ORDER_V1" || owner || subaccount_id || nonce ||
  series_id || side || price_1e8 || size_1e8 || tif || post_only ||
  deadline_ms)`) and persisting it on `option_orders`, OR
* Using the EIP-712 intent digest when present (would require making
  the signed intent a hard prerequisite; today it is optional).

Each option is a schema addition + Rust struct field + service
plumbing + migration + backfill + read API integration. Estimated
effort: 1 dedicated milestone.

### Deferred D2 — Options → Hybrid V2 reservation binding

**Status**: not implemented.

**Rationale**: `hybrid_v2_reservations` is populated by the canonical
on-chain events `ReservationCreated` / `ReservationReleased` via
the Hybrid V2 event indexer + reducer (`src/hybrid_v2/reducer.rs:367`,
`src/hybrid_v2/persistence.rs:3258`). The backend does not
synthesize reservation rows.

For Options open-order risk to appear in Hybrid V2's canonical
reservation ledger, one of the following is required:

* Options order acceptance triggers an on-chain reservation
  transaction (violates the "no automatic broadcast" invariant
  frozen in Hybrid V2 v1).
* A parallel Options-scoped reservation ledger (e.g.
  `options_reservations`) is designed with its own reducer and
  correlation to `hybrid_v2_reservations` post-execution.
* On-chain contract adds a "reserve for order" flow that is called
  from Options `submit_option_order` via the existing execution
  orchestrator.

Each option is a multi-week engineering + product decision. Estimated
effort: 1–2 dedicated milestones.

### Deferred D3 — Options fills → Hybrid V2 position projection

**Status**: partial (on-chain flow existing; off-chain projection
absent).

**Rationale**: `hybrid_v2_positions` is populated by
`OptionOrderPairExecuted` on-chain events via the Hybrid V2 reducer
(`src/hybrid_v2/reducer.rs:523`, `src/hybrid_v2/persistence.rs:3468`).
When an Options fill actually settles on-chain, the resulting event
does update Hybrid V2 positions correctly. What is NOT wired is a
pre-broadcast projection of the Options fill into
`hybrid_v2_positions` — this is by design: Hybrid V2 positions are
economic on-chain state, not backend accepted-order state.

The gap is that portfolio views (e.g. `/hybrid_v2/deployments/:id/
subaccounts/:owner/subaccount/:sid/positions`) will show 0 option
exposure between "backend accepted the fill" and "chain confirmed
the execution". This is honest — the position is not real until
the chain settles.

If a "pending option position" projection is desired, it belongs in
a separate off-chain projection table (e.g. `option_pending_positions`)
with a documented staleness contract, NOT in `hybrid_v2_positions`.
Estimated effort: 1 dedicated milestone.

### Deferred D4 — Options → Hybrid V2 ExecutionOrchestrator wiring

**Status**: not implemented.

**Rationale**: `ExecutionOrchestrator::prepare` at
`src/hybrid_v2/execution/orchestrator.rs:42` accepts an
`ExecutionIntent` derived from an on-chain matched execution. Options
today runs its own pre-broadcast pipeline
(`src/options/execution.rs`, `src/options/signing.rs`,
`src/options/broadcast_policy.rs`) which is signature-relay based
(both buyer and seller sign the EIP-712 payload; backend broadcasts
without holding a signer). This pipeline predates Hybrid V2's
signer-holding orchestrator and coexists with it.

Routing Options fills through `ExecutionOrchestrator` would require:

* Deciding whether Options moves from user-signed to backend-signed
  execution (major product change).
* Wiring canonical execution ID (D1 prerequisite).
* Deprecating or unifying the two execution pipelines.

Estimated effort: 1–2 dedicated milestones + product review.

### Deferred D5 — Repository-layer subaccount filter push-down

**Status**: correct-but-slow. Filtering happens in
`OptionOrderFilter::matches` / `OptionFillFilter::matches` (Rust
side) after `repository.list_option_orders()` /
`list_option_fills()` fetches ALL rows.

**Rationale**: Correctness is intact (side-aware filter matches
canonical semantics), but at scale this is O(all rows) per request.
The fix is a new set of repository methods that push
`subaccount_id` into the SQL WHERE clause. Additive; no schema
change; can land as a straight refactor.

**Not blocking**: current Options table row counts do not warrant
urgency. Estimated effort: 1 short milestone (repository refactor +
migration verification test).

### Deferred D6 — E2E PG matrix

**Status**: covered opportunistically. The 75-case matrix specified
in Part Q of the milestone brief was not implemented as a dedicated
new test binary. Existing coverage:

* `tests/subaccounts_options_orders_history_tests.rs` (63
  subaccount_id references) exercises: v1 default routing, v1
  reject-if-subaccount>1, v2 posture, order submit → history →
  cancel paths.
* `tests/subaccounts_options_ws_payload_tests.rs` exercises: WS
  lifecycle payload includes subaccount_id.
* `tests/options_tests.rs` (5086 subaccount_id refs) exercises: 8 of
  the TIF/matcher/RFQ scenarios listed in Part Q using
  `enabled_in_memory_for_tests`.
* `tests/options_hybrid_v2_subaccount_matcher_integration.rs` (new,
  this milestone) — 8 targeted invariants.

The full "disposable PG × 75 case matrix" as specified requires
provisioning a disposable PostgreSQL for the test run (via the
existing `HYBRID_V2_PG_TEST_DATABASE_URL` env-var gate pattern) plus
authoring the 75 individual cases. Estimated effort: 1–2 dedicated
milestones.

## Returned verdicts

Ship / affirm:

* `OPTIONS_HYBRID_V2_INTEGRATION_MODEL_RESOLVED` — canonical
  account identity + FeesManagerV2 + route resolver documented and
  wired.
* `OPTIONS_HYBRID_V2_CANONICAL_ACCOUNT_IDENTITY_VALIDATED` —
  matcher + filter isolation invariants locked in via 8-test suite.
* `OPTIONS_HYBRID_V2_ORDER_MODEL_VALIDATED` — schema (`option_orders`,
  `option_fills`, `option_twap_orders`, `options_conditional_orders`)
  all carry `subaccount_id` per migration 0039; Rust structs
  populate it; matcher preserves it on both sides of fills.
* `OPTIONS_HYBRID_V2_PRE_MATCH_VALIDATION_VALIDATED` — TIF
  combination, deadline, price/size validation, series active +
  subaccount ownership all enforced in `submit_option_order`
  (existing coverage).
* `OPTIONS_HYBRID_V2_MATCHING_INTEGRATION_VALIDATED` — matcher
  preserves both subaccount ids; same-wallet cross-subaccount
  matching permitted and covered by test; cross-wallet isolation
  verified; TIF semantics unchanged.
* `OPTIONS_HYBRID_V2_FEES_MANAGER_V2_INTEGRATION_VALIDATED` —
  FeesManagerV2 canonical; no legacy bps in fill hot path.
* `OPTIONS_HYBRID_V2_PUBLIC_API_INTEGRATION_VALIDATED` — every
  Options mutation route resolves subaccount via
  `resolve_options_v2_subaccount`; read endpoints default to
  subaccount 1 with explicit `?all=true` opt-out.
* `OPTIONS_HYBRID_V2_ADMIN_LIFECYCLE_INTEGRATION_VALIDATED` — Options
  execution admin lifecycle continues via `hybrid_v2_execution_admin.rs`
  routes; no regression.
* `OPTIONS_HYBRID_V2_EXISTING_BEHAVIOUR_PRESERVED` — no existing
  Options public route removed; no TIF semantic regression; no RFQ
  regression; no Perps regression; no frontend changes; no
  Solidity changes.
* `OPTIONS_HYBRID_V2_SECURITY_VALIDATED` — cross-subaccount cancel
  rejection enforced at route boundary; subaccount ownership check
  before mutation; `NO_OPTIONS_MEMORY_FALLBACK_IN_PRODUCTION` guard
  active.

Explicitly NOT returned in this session:

* `OPTIONS_HYBRID_V2_MARGIN_AND_RESERVATION_INTEGRATION_VALIDATED` —
  see deferred D2.
* `OPTIONS_HYBRID_V2_EXECUTION_CANDIDATE_VALIDATED` — see deferred D1.
* `OPTIONS_HYBRID_V2_SETTLEMENT_MODEL_RESOLVED` — settlement truth
  table not authored in this session; existing on-chain semantics
  govern.
* `OPTIONS_HYBRID_V2_EXECUTION_PIPELINE_INTEGRATED` — see deferred D4.
* `OPTIONS_HYBRID_V2_PROJECTION_AND_INDEXER_INTEGRATION_VALIDATED` —
  see deferred D3.
* `OPTIONS_HYBRID_V2_HISTORY_INTEGRATION_VALIDATED` — existing
  history endpoints subaccount-scoped; no NEW canonical history
  integration in this session.
* `OPTIONS_HYBRID_V2_RESTART_REORG_REBUILD_VALIDATED` — no new
  Options-specific restart/reorg/rebuild test binary in this session.
* `OPTIONS_HYBRID_V2_POSTGRES_E2E_MATRIX_VALIDATED` — see deferred D6.
* `OPTIONS_HYBRID_V2_PROPERTIES_VALIDATED` — no new proptest suite
  in this session.
* `OPTIONS_HYBRID_V2_PERFORMANCE_BOUNDED` — see deferred D5.
* `OPTIONS_HYBRID_V2_CI_GATE_VALIDATED` — no new CI gates authored
  in this session; existing PG integrity workflow continues to gate
  the previous milestone's suites.
* `OPTIONS_HYBRID_V2_PRODUCT_INTEGRATION_V1_COMPLETE` — **NOT
  RETURNED**. Six deferred items require dedicated milestones.
* `READY_FOR_OPTIONS_FRONTEND_TRADING_INTEGRATION_V1` — **NOT
  RETURNED**. Frontend integration requires D1 and D3 minimum.

## Safety statements (reaffirmed)

* NO real public-chain transaction sent this session.
* Exact `eth_sendRawTransaction` real-chain calls: 0.
* `BASE_MAINNET_8453_IS_FORBIDDEN` continues to hold.
* Frontend repo untouched. Solidity repo untouched.
* No secret material leaked; no `/tmp/deopt_*` files created; no
  containers provisioned.

## Files

Backend HEAD before this session: `4a4e616`.
Backend HEAD after this session: (see git log).

Files added:

* `tests/options_hybrid_v2_subaccount_matcher_integration.rs`
  (8 in-memory integration tests, 0 broadcast, 0 PG).
* `docs/OPTIONS_HYBRID_V2_PRODUCT_INTEGRATION_V1.md` (this file).

Files unchanged (no source-level code modification in this session):

* All Options service / repository / lifecycle files.
* All Hybrid V2 execution / reducer / persistence files.
* All API route files.
* All existing test files.

## Next stage

Per the milestone brief, next stage after full green is
`OPTIONS-FRONTEND-TRADING-INTEGRATION-V1`. Because this milestone
ships PARTIAL, the honest next stage is one of:

1. **`OPTIONS-HYBRID-V2-CANONICAL-EXECUTION-ID-V1`** — Land D1
   (canonical Options order hash + `option_fills.
   canonical_execution_id` column).
2. **`OPTIONS-HYBRID-V2-RESERVATION-DESIGN-V1`** — Product
   decision on D2 (whether Options open-order risk uses Hybrid V2
   canonical reservations, an Options-scoped ledger, or remains
   implicit).
3. **`OPTIONS-HYBRID-V2-REPOSITORY-PUSH-DOWN-V1`** — Land D5
   (repository-layer subaccount filter push-down).
4. **`OPTIONS-HYBRID-V2-PG-E2E-MATRIX-V1`** — Land D6 (75-case
   disposable PG matrix as specified in Part Q).

Only after D1, D2, D3, D4 are landed should
`OPTIONS-FRONTEND-TRADING-INTEGRATION-V1` proceed.
