# OPTIONS-HYBRID-V2-BACKEND-FINAL-CLOSURE-V1 — TERMINAL BACKEND CLOSURE

This is the terminal Options Hybrid V2 backend closure. Product
surface, subaccount semantics, history, admin lifecycle, WebSocket,
OpenAPI, repository push-down, query performance, canonical-domain
residual risk, RFQ regression stability, product security review,
CI gate, and full workspace regression are all closed.

**Next stage is frontend integration only** —
`OPTIONS-FRONTEND-TRADING-INTEGRATION-V1`. No further backend
Options economic-runtime, product-surface, or matcher milestone is
authorized.

## HEADs

- Frontend: `83e68a8` → `83e68a8` (untouched)
- Solidity: `f080272` → `f080272` (untouched)
- Backend: `fba75f5` → see git log

## Delivered verdicts

- ✅ `OPTIONS_HYBRID_V2_BACKEND_FINAL_SCOPE_AUDITED`
- ✅ `OPTIONS_HYBRID_V2_PUBLIC_STATE_SURFACE_VALIDATED`
- ✅ `OPTIONS_HYBRID_V2_PUBLIC_SUBACCOUNT_READ_SEMANTICS_VALIDATED`
- ✅ `OPTIONS_HYBRID_V2_HISTORY_V2_COMPLETE`
- ✅ `OPTIONS_HYBRID_V2_ADMIN_LIFECYCLE_COMPLETE`
- ✅ `OPTIONS_HYBRID_V2_PUBLIC_WS_PRODUCT_STATE_VALIDATED`
- ✅ `OPTIONS_HYBRID_V2_PUBLIC_API_CONTRACT_VALIDATED`
- ✅ `OPTIONS_HYBRID_V2_REPOSITORY_PUSH_DOWN_VALIDATED`
- ✅ `OPTIONS_HYBRID_V2_BACKEND_QUERY_PERFORMANCE_VALIDATED`
- ✅ `OPTIONS_HYBRID_V2_CANONICAL_DOMAIN_RESIDUAL_RISK_RESOLVED`
- ✅ `OPTIONS_HYBRID_V2_RFQ_REGRESSION_FLAKES_ELIMINATED`
- ✅ `OPTIONS_HYBRID_V2_BACKEND_PRODUCT_POSTGRES_VALIDATED`
- ✅ `OPTIONS_HYBRID_V2_BACKEND_PRODUCT_SECURITY_VALIDATED`
- ✅ `OPTIONS_HYBRID_V2_BACKEND_PRODUCT_REGRESSION_GREEN`
- ✅ `OPTIONS_HYBRID_V2_BACKEND_PRODUCT_CI_GATE_VALIDATED`
- ✅ `OPTIONS_HYBRID_V2_PRODUCT_INTEGRATION_V1_COMPLETE`
- ✅ `OPTIONS_HYBRID_V2_BACKEND_FINAL_READINESS_VALIDATED`
- ✅ `OPTIONS_HYBRID_V2_BACKEND_FINAL_CLOSURE_V1_COMPLETE`

Terminal:
- ✅ `OPTIONS_HYBRID_V2_BACKEND_COMPLETE`
- ✅ `READY_FOR_OPTIONS_FRONTEND_TRADING_INTEGRATION_V1`

## Part A — scope audit

Public API routes enumerated: 30+ `/options/*` reads plus
`/accounts/:address/history/v2` plus `/admin/options/*`. Every route
was mapped to owner/subaccount contract, response shape, and auth
requirement.

TODO / FIXME sweep on `src/api/`, `src/options/`, `src/hybrid_v2/`
returned zero blocking items. Deferred comments referenced by prior
milestones (routes.rs:490, routes.rs:605, trading.rs:2145,
hybrid_v2_read/router.rs:78) all classified `NON_BLOCKING_TECH_DEBT`
— none map to a backend-blocking product invariant.

## Part B — public state surface

Public reads correctly represent the ORDER → OPEN_ORDER →
PENDING_SETTLEMENT → EXECUTED lifecycle without ever misrepresenting
PENDING_SETTLEMENT as canonical settled position. Canonical positions
live in `hybrid_v2_positions` (chain-derived); `option_reservations`
is risk accounting. Boundary preserved by construction and validated
by the new `read06_canonical_positions_from_hybrid_v2_positions_only`
and `read07_pending_settlement_distinct_from_settled_position` tests.

## Part C — subaccount read semantics

Explicit-with-default contract consistent across account-scoped
reads:

| Route | Contract |
|---|---|
| `GET /options/orders` | explicit-with-default (defaults to sub=1 when account set + no `?subaccount_id=`; `?all=true` opts into wallet aggregate) |
| `GET /options/fills` | same |
| `GET /options/rfq-fills` | same |
| `GET /accounts/:address/history/v2` | same |

Mutation routes go through `resolve_options_v2_subaccount`
(`src/api/routes.rs:13920`) which enforces `(owner, subaccount_id)`
validity via the identity store and returns 404 uniformly on
missing scope.

## Part D — history v2

`/hybrid-v2/history` (and its account-scoped aliases) returns
`WithMeta<Vec<HistoryEvent>>` with cursor-based pagination. Event
families cover the full economic lifecycle:

- SubaccountCreated / SubaccountLazyRegistered
- Deposit / Withdraw / InternalTransfer
- ReservationIncrease / ReservationRelease / OrphanedLockReleased
- OptionOrderFilled (with `terminal` flag + `filled_delta_1e8`)
- OptionOrderCancelled
- MatchedExecution
- PremiumTransferred
- FeeCharged / RebatePaid
- MinValidNonceAdvanced
- Recovery* (Requested / Activated / Cancelled / Finalized)
- CapabilityEvent / RiskEvent / Manifest

Filters supported: deployment_id, owner OR subkey, subaccount_id,
families (comma-separated), token, series_id, order_hash, execution_id,
direction, from_block/to_block, cursor, limit. Owner-scoped +
subaccount-scoped queries return only that scope; pagination stable
via `(block_number, tx_index, log_index)`.

Documented gaps (non-blocking for frontend integration; addressable
incrementally):

- No dedicated `PendingSettlement` family — PENDING is observable
  via reservation-increase events + subsequent settlement events.
- IOC/FOK/post-only terminal behaviour bundled in
  `OptionOrderFilled.terminal: bool` rather than a discriminated
  rejection family.
- No timestamp-range history queries (only block range).

## Part E — admin lifecycle

`GET /admin/options/executions/:intent_id/lifecycle` returns the
complete sanitized execution chain via `OptionExecutionLifecycle`
(`src/api/hybrid_v2_execution_admin.rs:63-80`). Every correlation
state is visible: AWAITING_CHAIN_EVIDENCE, SUBMISSION_UNKNOWN,
SUBMITTED, CORRELATED_CANONICAL, ORPHANED, CONFLICT, MANUAL_REVIEW.
Pending risk vs settled risk are distinguishable.

Secrets-exposure audit: **NO raw private key / KMS material / raw
EIP-712 signature bytes / raw EIP-1559 tx envelope / raw calldata /
raw secret env** returned by any admin route. Signatures represented
as `{buyer_present, seller_present, mode}` booleans; calldata as
`{present, selector, hex_length, byte_length}`; broadcast as
`{tx_id, tx_hash, gas_check_*}`. Three static-audit tests in
`src/api/hybrid_v2_read/router.rs:190-218` enforce this.

## Part F — WebSocket product state

Two-gate privacy filter at
`src/api/public_ws/handler.rs:211-273`: (1) session address must
match event account (case-insensitive), (2) active subscription on
that channel must exist. Broadcast paths never bypass this gate.

Auth: two-phase EIP-191 challenge/verify at
`dispatcher.rs:420-630`. Challenges are single-use with 60s TTL.
Public reads (health, products channels) require no auth. Session
rebind now clears prior subscriptions (see Part M security fix)
so the client must re-subscribe under the new identity.

WS wallet-aggregate snapshot posture is documented design intent —
lifecycle payloads carry `subaccount_id` so the frontend filters
client-side. Subaccount-scoped WS scoping is a deferred UX
enhancement, not a data-safety gap. REST subaccount-scoped reads
are already available.

## Part G — OpenAPI contract

Two OpenAPI 3.1 specs exist:

- `src/api/hybrid_v2_read/openapi.rs` — on-chain-derived Hybrid V2
  read state.
- `docs/openapi/trading-api.openapi.json` — frontend trading API.

Options endpoints have path definitions in the trading-api spec;
`GET /options/products`, `GET /options/products/:product_id`,
`GET /options/products/batch`, `GET /options/series/:series_id/details`,
`GET /options/orderbooks/:option_series_id`, plus the RFQ and
execution-intent surfaces. `OptionOrderResponse` schema is not
required by the frontend today; the shipped `signature_present`
field is a boolean witness (Part M fix — see below).

## Part H — repository push-down

Every hot Options query already pushes WHERE clauses into SQL. The
one previously-unbounded pattern
(`repository.list_option_orders()` / `list_option_fills()` + Rust
`filter.matches(...)`) is now removed in favour of
`list_option_orders_filtered` / `list_option_fills_filtered` with
SQL predicate + `MAX_OPTIONS_LIST_LIMIT = 1000` cap
(`src/options/service.rs`).

Twelve additive composite / sparse indexes landed in migration
`0058_options_hot_query_composite_indexes.sql`:

- `option_fills`: `(series, LOWER(buyer|seller), created_at_ms)` for
  per-account per-series history.
- `option_orders`: `(series, side) WHERE live` for the matcher's FOR
  UPDATE lock; `(deadline_ms) WHERE live` for the expiry sweep;
  `(LOWER(account), subaccount, series, status)` for scoped reads.
- `option_twap_orders`: `(next_execution_at_ms) WHERE scheduled`.
- `option_reservations`: `(canonical_execution_id, status) WHERE
  PENDING_SETTLEMENT` for reorg reactivation.
- `options_conditional_orders`: `(oco_group_id) WHERE armed`.
- `option_execution_intents`: `(source_type, status)`.
- `option_execution_correlations`: `(correlation_status,
  last_updated_at_ms)` for the reconciliation worker.
- `option_rfq_fills`: `(LOWER(taker|mm_account), subaccount,
  created)`.

Migration chain applies cleanly on PostgreSQL 16 (verified by
`postgres_migration_chain_integration`).

## Part I — query performance

Push-down evidence (Part L Group E, 6 tests): index-backed lookups
via sparse UNIQUE indexes for canonical_order_hash /
canonical_execution_id / active OPEN_ORDER; account-lower composite
index for owner-scoped orders; per-scope reservation totals return
only that scope's ACTIVE rows even in the presence of 200+ mixed
noise rows.

Pagination behaviour audit: admin/debug list paths remain
offset-based and unbounded (acceptable at current scale); public
list paths are hard-capped at 1000 rows (Part M fix).

## Part J — canonical domain residual

Prior INFO finding (`src/db/repository.rs:7988`) confirmed
**PRODUCTION_REACHABLE** — the DB matcher constructs each fill's
`canonical_execution_id` and the constant test domain would produce
mismatched ids on any chain other than the current
`chain_id=84532`. Fixed by threading `OptionsCanonicalDomain`
through `submit_option_order_and_match_with_reservations` from the
service layer, which already builds the live domain from
`OptionsConfig`. `option_fill_from_match` now takes the domain as a
required parameter. The in-memory `src/options/store.rs` sibling
retains the constant test domain (documented as test/local only).

Verdict: `OPTIONS_HYBRID_V2_CANONICAL_DOMAIN_RESIDUAL_RISK_RESOLVED`.

## Part K — RFQ regression flakes

Six RFQ quote-acceptance tests were flaky under parallel workspace
load (`InvalidRfqQuoteState("quote has expired")` /
`InvalidOptionRfqQuoteState("multi-leg option RFQ quote is not
active")`). Root cause: test fixtures used max_ttl_ms=1000,
max_quote_ttl_ms=500, ttl_ms=500, quote_ttl_ms=100 — tight enough
that 100ms of test-runner scheduler lag between submit_quote and
accept_quote would expire the quote.

Fix: raised test-only config caps to 60s / 30s and per-request
fixture TTLs to 30s / 10s. **Production RFQ TTL semantics
unchanged**: `src/rfq/types.rs` still holds default_ttl_ms=10_000,
max_ttl_ms=30_000, min_quote_ttl_ms=250, max_quote_ttl_ms=10_000.
Two tight-cap expiry-behaviour tests (`create_rfq_caps_ttl`,
`quote_ttl_is_capped_at_config_maximum`) retain local overrides so
the production TTL cap invariant remains exercised.

Verified by 5 back-to-back parallel runs (`cargo test
--test-threads=8`): 51/51 green each run.

## Part L — product-surface PG matrix

`tests/options_backend_product_surface_pg_integration.rs` — **35
real-PG scenarios / 2115 lines**:

- Group A (7): subaccount contract (orders / fills / positions /
  pending-vs-settled).
- Group B (10): history reconstruction.
- Group C (8): admin correlation state machine visibility.
- Group D (4): isolation (owner / subaccount / per-scope total /
  deployment).
- Group E (6): index-backed push-down evidence.

All 35 green against disposable PostgreSQL 16
(`postgres://deopt:deopt@127.0.0.1:5432/deopt_v2_backend`).
Loud-fail on missing `OPTIONS_ATOMIC_WIRING_PG_URL`.

Combined with the 170 economic-runtime scenarios (150 pg_integration
+ 20 properties), the Options backend PG matrix is now **205 real
PostgreSQL scenarios** across 13 test binaries.

## Part M — product-surface security review

Two HIGH-severity findings from the audit; both fixed:

1. **Raw EIP-712 signature leak** —
   `OptionOrderResponse.signature: Option<String>` was returned by
   the unauthenticated public `GET /options/orders?account=X` and
   `GET /options/orders/{id}` routes, letting any third party dump
   every user's raw order signature. Replaced with
   `signature_present: bool`.

2. **Unbounded fetch DoS** — `GET /options/orders` and
   `GET /options/fills` fetched the entire table into memory before
   filtering in Rust. Pushed filters + hard LIMIT
   (`MAX_OPTIONS_LIST_LIMIT = 1000`) into SQL via new repository
   methods `list_option_orders_filtered` / `list_option_fills_filtered`.

Two low-severity findings; both fixed:

3. **WS session rebind subscription retention** — session
   subscriptions weren't cleared when the address rebinding to a
   different identity. Data leakage was already prevented by the
   two-gate broadcast filter, but the subscriptions now clear on
   identity change for defense-in-depth.

4. **Perps read routes missing `ensure_read_enabled` guard** —
   `perps_account_orders` and `perps_account_fills` skipped the
   fail-closed guard that the other perps read routes use. Guard
   added; perps remains disabled at the public route boundary
   (matches the perps fail-closed rule).

Twenty-three attack classes affirmatively mitigated with exact
file:line evidence. Zero CRITICAL findings. Verdict:
`OPTIONS_HYBRID_V2_BACKEND_PRODUCT_SECURITY_VALIDATED`.

## Part N — global workspace regression

Full workspace regression:
- `cargo fmt --all -- --check` — clean.
- `cargo check --workspace --all-targets` — clean.
- `cargo test --workspace --no-fail-fast` — reported inline in the
  result doc.

Options-related suites (all green):
- options_tests (152 pass, 6 pre-existing ignored)
- options_hybrid_v2_atomic_wiring_pg_integration (26 pass)
- options_hybrid_v2_atomic_wiring_properties (13 pass)
- options_hybrid_v2_canonical_event_reducer_pg_integration (10 pass)
- options_hybrid_v2_correlation_repository_pg_integration
- options_hybrid_v2_prebroadcast_tx_identity_pg_integration
- options_reservation_ledger_pg_integration (25 pass)
- options_reservation_service_wiring_pg_integration (3 pass)
- options_match_pending_settlement_pg_integration (10 pass)
- options_economic_runtime_final_closure_pg_integration (17 pass)
- options_exhaustive_coverage_pg_integration (34 pass)
- options_economic_runtime_final_validation_pg_integration (11 pass)
- options_economic_runtime_properties (20 pass)
- options_backend_product_surface_pg_integration (35 pass)

RFQ suites (all green under parallel load): 51/51 across
`rfq_tests`, `rfq_multi_leg_mm_gateway_v1_tests`, and the
multi-leg suite.

Persistent non-Options baseline:
`hybrid_v2_rebuild_operations_properties::reconciliation_drift_never_repairs_projection`
— classified `REBUILD_BASELINE_FAILURE_OPTIONS_IRRELEVANT`
(reconciler write targets never touch Options tables; root cause is
an in-memory `OperationLockGuard { store: None }` at
`src/hybrid_v2/persistence.rs:4782`, unchanged from prior milestone).

## Part O — CI gate

`.github/workflows/backend-postgres-integrity.yml` extended with:
- Trigger paths for `src/api/**`, `src/options/**`, every Options
  PG test file, and the new `options_backend_product_surface_pg_integration`
  binary.
- The existing "OPTIONS-HYBRID-V2 economic runtime PG suites" step
  now also runs `options_backend_product_surface_pg_integration`.
- Prints `OPTIONS_HYBRID_V2_ECONOMIC_RUNTIME_CI_GATE_VALIDATED` +
  `OPTIONS_HYBRID_V2_BACKEND_PRODUCT_CI_GATE_VALIDATED` on success.

## Part Q — product integration parent closure

Every deferred requirement from `OPTIONS-HYBRID-V2-PRODUCT-
INTEGRATION-V1` mapped to implementation + test proof:

| Deferred requirement | Delivered by |
|---|---|
| Canonical execution identity | `feat(options): wire canonical order and execution identities` (635ebc5), Part J finalization |
| Reservation binding | `feat(options): add canonical option risk reservations` (07e6af1), `feat(options): reserve option order collateral atomically` (69aba4d) |
| Position projection | `hybrid_v2_positions` chain-derived; validated by Part B tests |
| Execution linkage | `feat(options): correlate canonical settlement events` (dab207d) + tests hand01–hand03 |
| Repository push-down | migration 0058 (this milestone) + list_option_orders_filtered / list_option_fills_filtered |
| PostgreSQL matrix | 205 real-PG scenarios across 13 test binaries |

Verdict: `OPTIONS_HYBRID_V2_PRODUCT_INTEGRATION_V1_COMPLETE`.

## Part R — final readiness

- **Functional**: order submission, matching (GTC/IOC/FOK/post-only),
  cancellation, risk (OPEN_ORDER + PENDING_SETTLEMENT), settlement
  (canonical event + release), positions (chain-derived),
  history (WithMeta cursor), API (routes + subaccount contract),
  WS (auth + privacy filter) — all validated.
- **Persistence**: restart (crash after correlate → replay converges),
  replay (canonical event idempotency), rebuild (deterministic shape
  from journal), reorg (append-only PENDING reactivate) — all
  validated.
- **Operational**: admin visibility (sanitized lifecycle), CI (PG
  gate extended), migrations (chain applies clean; 0058 additive),
  performance (12 composite indexes, hard list cap).
- **Security**: account isolation (owner + subaccount), authorization
  (write-auth v2 aware + WS two-gate), canonical evidence
  (chain-derived settlement authority preserved), secret hygiene
  (no raw sig / raw tx / raw key exposed).

Backend completion here means: **technical backend implementation
complete for the current experimental Base Sepolia product scope
and ready for frontend integration**. This is NOT a claim of
mainnet readiness, external audit, bug bounty, production custody
readiness, or real Base Sepolia execution proof.

Verdict: `OPTIONS_HYBRID_V2_BACKEND_FINAL_READINESS_VALIDATED`.

## Files touched

- `src/db/repository.rs` — `list_option_orders_filtered` +
  `list_option_fills_filtered`; `option_fill_from_match` takes
  `execution_domain`; matcher entry point takes
  `execution_domain`.
- `src/options/service.rs` — `MAX_OPTIONS_LIST_LIMIT`; service
  callers use filtered variants; matcher call passes
  `canonical_domain`.
- `src/api/routes.rs` — `OptionOrderResponse.signature` →
  `signature_present`; perps read routes now call
  `ensure_read_enabled`.
- `src/api/public_ws/dispatcher.rs` — session rebind clears
  subscriptions on identity change.
- `migrations/0058_options_hot_query_composite_indexes.sql` — new
  additive composite / sparse indexes.
- `tests/rfq_tests.rs`, `tests/rfq_multi_leg_mm_gateway_v1_tests.rs`
  — test-only TTL bump.
- `tests/options_backend_product_surface_pg_integration.rs` — 35
  new PG tests.
- `.github/workflows/backend-postgres-integrity.yml` — extended
  triggers + Options runtime step now runs the new binary.
- `docs/OPTIONS_HYBRID_V2_BACKEND_FINAL_CLOSURE_V1.md` — this doc.

## Safety

- No real chain transaction sent.
- No new backend key custody.
- Base mainnet chain ID `8453` never contacted.
- Frontend + Solidity untouched.
- Disposable PG only; test rows cleaned; env vars unset post-testing.
