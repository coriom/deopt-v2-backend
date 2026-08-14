# OPTIONS-HYBRID-V2-IDENTITY-AND-CORRELATION-WIRING-V1

Milestone: `OPTIONS-HYBRID-V2-IDENTITY-AND-CORRELATION-WIRING-V1`

Status: **PARTIAL CLOSURE** — Package A (INSERT-time canonical
identity wiring) shipped; Package B (execution intent linkage +
option_execution_correlations table + event correlation) and
Package C (PG matrix + properties + CI) deferred to follow-up
milestones.

Date: 2026-08-14.

## Purpose

Turn the previously-designed Options canonical identity model into a
mandatory production invariant. Establish the identity chain:

```text
canonical_order_hash               (Package A — SHIPPED)
        ↓
option_fills.canonical_execution_id (Package A — SHIPPED)
        ↓
option_execution_intents.canonical_execution_id (Package B — DEFERRED)
        ↓
option_execution_correlations.canonical_execution_id (Package B — DEFERRED)
        ↔
onchain_execution_id / tx identity / canonical event identity  (Package B — DEFERRED)
```

## Identity glossary (frozen)

Naming discipline from prior milestone
`OPTIONS-HYBRID-V2-RISK-RESERVATION-AND-PENDING-SETTLEMENT-V1`
Part B, ratified and reaffirmed here:

* **`OptionTrade` EIP-712 digest** — user settlement authorization,
  signed by both buyer and seller, `src/options/execution.rs:213-228`.
* **`canonical_order_hash`** — backend-derived economic identity of
  a resting Options order, `src/options/canonical_identity.rs:90`.
  NEVER labeled as a user signature.
* **`canonical_execution_id`** — backend-derived economic identity
  of one matched Options fill, `src/options/canonical_identity.rs:120`
  wrapping the existing HV2 derivation at
  `src/hybrid_v2/execution/identity.rs:76`.
* **`onchain_execution_id`** — contract/event execution identity
  emitted by `OptionMatchingEngineV2` where available (existing on
  `hybrid_v2_matched_executions.execution_id`,
  `src/hybrid_v2/decoder.rs:289`).
* **`tx_hash`** — Ethereum transaction identity.
* **correlation** — deterministic mapping between backend economic
  execution (`canonical_execution_id`) and canonical on-chain
  evidence (`onchain_execution_id` or `tx_hash`).

## Package A — shipped

### Type discipline

* Added `OptionOrder.canonical_order_hash: Option<String>` and
  `OptionFill.canonical_execution_id: Option<String>` at
  `src/options/types.rs:918`, `1099`. Both `#[serde(default,
  skip_serializing_if = "Option::is_none")]` for wire compatibility
  with pre-migration readers.
* Distinct types kept where they matter: the existing signed
  `OptionTradePayload` (`src/options/execution.rs:63`) and the new
  `OptionOrderHashInputs` (`src/options/canonical_identity.rs:52`)
  do not share a name.
* No newtype wrapping introduced this milestone because the fields
  are Option<String> — the naming discipline is enforced via field
  names, doc comments, and serialization annotations. A future
  milestone may add `CanonicalOptionOrderId(pub String)` newtypes
  if a semantic-mismatch bug surfaces in production.

### INSERT-time wiring

Every canonical Options order-creation path now populates
`canonical_order_hash` BEFORE the persistence transaction:

* `src/options/service.rs:submit_option_order_inner:463-467` —
  primary REST/RPC submission path. Hash computed after building
  the `OptionOrder` struct, atomic with the subsequent
  `submit_option_order_and_match` transaction.
* `src/options/conditional_orders.rs:840-874` — TP/SL child orders
  materialized from a triggered conditional carry the canonical
  identity so downstream fills correlate.

Every canonical fill-creation path now populates
`canonical_execution_id` at fill-construction time:

* `src/db/repository.rs::option_fill_from_match:7710-7752` — DB
  matcher.
* `src/options/store.rs::option_fill_from_match:2137-2179` —
  in-memory matcher (test parity).

Both use `canonical_execution_id_for_fill(buyer_hash, seller_hash,
qty)` which returns `Some(id)` iff both orders carry a canonical
hash; returns `None` for fills involving legacy (pre-wiring)
counterparties.

### Repository SQL

* `insert_option_order_query` at
  `src/db/repository.rs:7553-7581` — additive $21 bind for
  `canonical_order_hash`.
* `insert_option_order_tx` + top-level `insert_option_order` — INSERT
  SQL updated to include the new column at `option_orders`.
* `insert_option_fill_tx` — INSERT SQL updated to include
  `canonical_execution_id` at `option_fills`.
* All SELECT variants for `option_orders` / `option_fills` (5 sites
  total) updated via `replace_all` to include the new columns.
* Both RETURNING clauses (`cancel_option_order`,
  `expire_option_orders_due`) updated to include the new columns
  plus the previously-missing `subaccount_id` binding.
* `option_order_from_row` / `option_fill_from_row` hydrate the new
  columns defensively via `row_get::<Option<String>>(...).unwrap_or(None)`
  so pre-migration rows or partial SELECTs land at NULL rather
  than erroring.

### Database constraint audit (Part G)

Migration 0053 (previous milestone) shipped with:
* Sparse UNIQUE indexes `ux_option_orders_canonical_order_hash`
  and `ux_option_fills_canonical_execution_id` (only over NON-NULL
  values → legacy rows unaffected).
* Immutability triggers preventing UPDATE from changing an already-
  set hash.
* Lookup indexes for future admin/reconciliation surfaces.

**Audit result**: constraints are correct for the Package A wiring.
No new migration required in this milestone.

Legacy-row NULL handling is intentional: rows created before
migration 0053 remain NULL, and matcher fills whose maker OR taker
was pre-wiring produce fills with `canonical_execution_id = NULL`.
This preserves backward compatibility without silently guessing
identities we did not compute.

### Construction-site closure (Part F)

All ~15 struct-literal construction sites updated with explicit
`None` for the new fields:

Production paths (compute real hash):
* `src/options/service.rs:442` — `submit_option_order`
* `src/options/conditional_orders.rs:840` — TP/SL child

Test/fixture paths (explicit None):
* `src/options/service.rs:7533` — `orderbook_fill()` test helper
* `src/api/trading.rs:3319, 3342, 3367` — trading test fixtures
  (three OptionOrder literals)
* `src/api/trading.rs:3515, 3531` — trading test fixtures (two
  OptionFill literals)
* `tests/subaccounts_options_orders_history_tests.rs:677, 738, 781,
  975` — subaccount test fixtures

Repository hydration paths (read from row):
* `src/db/repository.rs:6633` — `option_order_from_row`
* `src/db/repository.rs:6866` — `option_fill_from_row`

Not touched intentionally:
* `src/hybrid_v2/execution/plan.rs:482` — the `OptionOrder { ... }`
  here is the **Solidity ABI `OptionOrder`** type from the `sol!`
  macro, NOT the Rust struct. Confirmed via context inspection
  (fields `seriesId`, `pricePerContract1e8`, etc. do not exist on
  the Rust struct).
* `tests/hybrid_v2_*_pg_integration.rs` (~7 files) — same:
  Solidity ABI type used as test payloads.
* `tests/hybrid_v2_broadcast_live_wiring_*.rs` (~4 files) — same.

### Test coverage

**Retained**: 11 pure-derivation unit tests in
`options::canonical_identity::tests`, all passing.

**New**: 6 in-memory integration tests in
`tests/options_hybrid_v2_identity_wiring_integration.rs`, all
passing:

1. `submit_populates_canonical_order_hash_deterministically` — order
   accepted via service layer has `canonical_order_hash` populated;
   hash matches `canonical_order_hash_for(&order)` re-derivation.
2. `different_subaccount_of_same_owner_produces_distinct_canonical_hashes`
   — same wallet, sub 1 vs sub 2, distinct hashes.
3. `matched_fill_populates_canonical_execution_id` — cross-wallet
   match produces fill with `canonical_execution_id` populated
   (0x-prefixed 32-byte); distinct from either order's hash.
4. `same_wallet_cross_subaccount_fill_execution_id_is_deterministic`
   — same wallet cross-subaccount fill; execution id matches
   re-derivation via `canonical_execution_id_for_fill(...)`.
5. `distinct_partial_fills_receive_distinct_execution_ids` — two
   makers × one aggressive taker → two fills with distinct
   execution ids.
6. `different_deployment_or_chain_would_change_hash` — direct call
   to `derive_canonical_order_hash` with varied deployment/chain
   proves the derivation IS sensitive (deployment/chain
   distinctness invariant maintained even though production
   currently uses constants).

**Regression**: `cargo test --test options_tests` passes 152/152
(6 pre-existing ignored). Full `cargo check --workspace
--all-targets` compiles with 0 errors.

## Package B — deferred

The following remain unshipped and are documented as follow-up:

### Part H — Execution intent linkage (DEFERRED)

* Migration `0054_option_execution_intents_canonical_execution_id.sql`
  adding `canonical_execution_id TEXT NULL` to
  `option_execution_intents` with sparse UNIQUE index +
  immutability trigger (mirroring 0053 posture).
* Wire `create_option_orderbook_execution_intents` (in
  `src/options/service.rs`, called from `submit_option_order_inner`
  post-fill-commit) to persist the corresponding fill's
  `canonical_execution_id` alongside the intent.
* Repository INSERT/SELECT for `option_execution_intents` updated
  additively.
* Validation: intent's canonical_execution_id MUST match the fill's;
  intent MUST NOT reference a fill belonging to another deployment.

Est. effort: 1 milestone.

### Part I — option_execution_correlations schema (DEFERRED)

* New migration creating `option_execution_correlations` table:
  ```sql
  CREATE TABLE option_execution_correlations (
      correlation_id UUID PRIMARY KEY,
      canonical_execution_id TEXT NOT NULL,
      onchain_execution_id TEXT NULL,
      tx_hash TEXT NULL,
      canonical_block_number BIGINT NULL,
      canonical_block_hash TEXT NULL,
      execution_kind TEXT NOT NULL CHECK (execution_kind IN ('trade','rfq_trade')),
      correlation_status TEXT NOT NULL CHECK (correlation_status IN (
          'AWAITING_CHAIN_EVIDENCE',
          'CORRELATED',
          'ORPHANED',
          'CONFLICTING_MANUAL_REVIEW'
      )),
      created_at_ms BIGINT NOT NULL,
      updated_at_ms BIGINT NOT NULL,
      terminal_reason TEXT NULL
  );
  CREATE UNIQUE INDEX ux_correlations_canonical
      ON option_execution_correlations (canonical_execution_id);
  CREATE INDEX ix_correlations_tx
      ON option_execution_correlations (tx_hash)
      WHERE tx_hash IS NOT NULL;
  CREATE INDEX ix_correlations_onchain
      ON option_execution_correlations (onchain_execution_id)
      WHERE onchain_execution_id IS NOT NULL;
  ```
* Repository methods: `insert_option_execution_correlation`,
  `attach_tx_hash_to_correlation`, `mark_correlation_settled`,
  `mark_correlation_orphaned`, `mark_correlation_conflict`.
* No canonical economic position state stored here — correlation
  metadata only.

Est. effort: 1 milestone.

### Part J — User-signed execution preparation correlation (DEFERRED)

Wire `option_execution_intents` → `option_execution_correlations`
population at broadcast persistence boundaries in
`src/options/execution.rs` and `src/options/broadcast_policy.rs`:
* When intent moves to `CALLDATA_READY`, create AWAITING correlation.
* When intent moves to `BROADCASTING`, attach `tx_hash`.

Est. effort: 1 milestone.

### Part K — Canonical event correlation (DEFERRED)

Extend `src/hybrid_v2/reducer.rs` to consume
`OptionOrderPairExecuted` events and mark the corresponding
correlation `CORRELATED` OR `CONFLICTING_MANUAL_REVIEW`. The
correlation key is `(tx_hash, log_index)` OR `onchain_execution_id`
(from event topic1) — deterministic, no amount/timestamp heuristics.

**Contract check (Part K STOP condition)**: verify that
`OptionMatchingEngineV2.executeTrade` emits an event that carries
either:
* an `executionId` derived from the same canonical preimage as
  `derive_canonical_execution_id`, OR
* a `tx_hash + intent_id` pair sufficient to bind the correlation
  deterministically.

If the contract emits neither, return
`OPTIONS_HYBRID_V2_EXECUTION_EVENT_CORRELATION_BLOCKED_BY_CONTRACT`
per the Part K instruction.

**Preliminary check (this milestone)**: the existing HV2 event
decoder at `src/hybrid_v2/decoder.rs:289-300` extracts an
`execution_id` from the `OptionOrderPairExecuted` log topic1. It is
NOT verified that this equals `canonical_execution_id` — the two
could be derived from different preimages. Full verification
requires a Solidity read + a fixture test comparing both computed
values. Deferred to Part K milestone.

Est. effort: 1 milestone (plus potential contract-blocker stop).

### Part L — Reorg correlation semantics (DEFERRED)

When a correlated event's canonical branch is reorged:
* Correlation row moves to `ORPHANED`.
* `canonical_execution_id` remains immutable.
* Replacement branch may re-correlate the same
  `canonical_execution_id` (creating a fresh AWAITING → CORRELATED
  transition).
* Historical orphan audit remains visible internally.

Est. effort: 1 milestone.

### Part M — Restart / idempotency (DEFERRED — code exists, tests missing)

The current wiring is idempotent by construction because:
* `canonical_order_hash` is deterministic from the persisted
  order's immutable fields.
* `canonical_execution_id` is deterministic from the two orders'
  hashes + fill quantity.
* Sparse UNIQUE indexes prevent duplicate persistence.

What's missing: dedicated restart tests exercising every lifecycle
transition (order INSERT, fill INSERT, intent creation, etc.).

Est. effort: 0.5 milestone.

### Part N — Public/admin surface exposure (DEFERRED)

Optional additive expose of `canonical_order_hash` and
`canonical_execution_id` on `GET /options/orders` /
`GET /options/fills` response DTOs. Backward-compatible. Not
required for correctness; useful for operator debugging.

Est. effort: 0.5 milestone.

## Package C — deferred

### Part O — 32+ case disposable PG matrix (DEFERRED)

Requires provisioning disposable PostgreSQL 16 + writing the
enumerated cases (order identity 7 cases, fill identity 6, intent
linkage 5, correlation 8, restart 6, isolation 3 = 35 cases).

Est. effort: 1–2 milestones.

### Part P — 12-property bounded proptest suite (DEFERRED)

Property scenarios enumerated in the brief; implementation deferred.

Est. effort: 1 milestone.

### Part Q — Performance observations (DEFERRED)

Hash generation, lookup by canonical id, correlation queries.

Est. effort: 0.5 milestone.

### Part R — Security review (DEFERRED)

Comprehensive doc + attack-surface tests for identity collision,
signed-digest confusion, correlation spoofing.

Est. effort: 0.5 milestone.

### Part S — CI gate update (DEFERRED)

PostgreSQL CI workflow extension for the new PG suites.

Est. effort: 0.5 milestone.

## Returned verdicts

Ship / affirm (Package A):

* `OPTIONS_HYBRID_V2_IDENTITY_WIRING_SURFACE_AUDITED` — audited via
  `cargo check --workspace --all-targets`; 6 lib sites + 4 test
  sites enumerated + closed.
* `OPTIONS_HYBRID_V2_IDENTITY_TYPE_DISCIPLINE_VALIDATED` — distinct
  types + serde-additive fields + naming discipline documented.
  Newtype wrapping deferred as optional hardening.
* `OPTIONS_HYBRID_V2_CANONICAL_ORDER_INSERT_WIRING_VALIDATED` —
  every production order-creation path populates hash before INSERT
  transaction commits; deterministic; subaccount/deployment/chain
  distinct.
* `OPTIONS_HYBRID_V2_CANONICAL_EXECUTION_INSERT_WIRING_VALIDATED` —
  every fill-creation path populates execution id during same
  transaction as fill; distinct sequential fills produce distinct
  ids; deterministic.
* `OPTIONS_HYBRID_V2_IDENTITY_CONSTRUCTION_SURFACE_CLOSED` — all
  ~15 struct-literal sites updated explicitly; no `..Default::
  default()` shortcuts; no silent None substitutions.
* `OPTIONS_HYBRID_V2_CANONICAL_IDENTITY_DATABASE_CONSTRAINTS_VALIDATED`
  — migration 0053 constraints correct for Package A wiring; sparse
  UNIQUE + immutability triggers; NULL for legacy rows intentional.

Explicitly NOT returned in this session:

* `OPTIONS_HYBRID_V2_EXECUTION_INTENT_CANONICAL_ID_LINKAGE_VALIDATED`
  — Package B / Part H.
* `OPTIONS_HYBRID_V2_EXECUTION_CORRELATION_SCHEMA_VALIDATED` —
  Package B / Part I.
* `OPTIONS_HYBRID_V2_USER_SIGNED_EXECUTION_CORRELATION_WIRED` —
  Package B / Part J.
* `OPTIONS_HYBRID_V2_CANONICAL_EVENT_CORRELATION_VALIDATED` —
  Package B / Part K (potentially blocked by contract; requires
  Solidity fixture check).
* `OPTIONS_HYBRID_V2_EXECUTION_CORRELATION_REORG_VALIDATED` —
  Package B / Part L.
* `OPTIONS_HYBRID_V2_IDENTITY_CORRELATION_RESTART_SAFE` — Package B
  / Part M (implicit by construction; explicit tests deferred).
* `OPTIONS_HYBRID_V2_IDENTITY_CORRELATION_API_SURFACES_VALIDATED` —
  Package B / Part N.
* `OPTIONS_HYBRID_V2_IDENTITY_CORRELATION_POSTGRES_MATRIX_VALIDATED`
  — Package C / Part O.
* `OPTIONS_HYBRID_V2_IDENTITY_CORRELATION_PROPERTIES_VALIDATED` —
  Package C / Part P.
* `OPTIONS_HYBRID_V2_IDENTITY_CORRELATION_PERFORMANCE_BOUNDED` —
  Package C / Part Q.
* `OPTIONS_HYBRID_V2_IDENTITY_CORRELATION_SECURITY_VALIDATED` —
  Package C / Part R.
* `OPTIONS_HYBRID_V2_IDENTITY_CORRELATION_CI_GATE_VALIDATED` —
  Package C / Part S.
* `OPTIONS_HYBRID_V2_IDENTITY_AND_CORRELATION_WIRING_V1_COMPLETE` —
  **NOT RETURNED**.
* `READY_FOR_OPTIONS_HYBRID_V2_OPEN_ORDER_RESERVATION_V1` — **NOT
  RETURNED**.

## Safety statements

* NO real public-chain transaction sent this session.
* Exact `eth_sendRawTransaction` real-chain calls: 0.
* No `/tmp/deopt_*` created. No PG containers provisioned.
* Base mainnet 8453 never contacted.
* Frontend repo untouched (HEAD `83e68a8`).
* Solidity repo untouched (HEAD `f080272`).

## Follow-up work (blocking full closure)

| Order | Milestone | Est. effort |
| --- | --- | --- |
| 1 | `OPTIONS-HYBRID-V2-EXECUTION-INTENT-LINKAGE-V1` (Part H) | 1 milestone |
| 2 | `OPTIONS-HYBRID-V2-EXECUTION-CORRELATION-TABLE-V1` (Parts I, J) | 1 milestone |
| 3 | `OPTIONS-HYBRID-V2-EVENT-CORRELATION-V1` (Part K, may block on contract) | 1 milestone |
| 4 | `OPTIONS-HYBRID-V2-REORG-RESTART-CORRELATION-V1` (Parts L, M) | 1 milestone |
| 5 | `OPTIONS-HYBRID-V2-IDENTITY-CORRELATION-API-SURFACES-V1` (Part N) | 0.5 milestone |
| 6 | `OPTIONS-HYBRID-V2-IDENTITY-CORRELATION-PG-MATRIX-V1` (Parts O, P) | 1–2 milestones |
| 7 | `OPTIONS-HYBRID-V2-IDENTITY-CORRELATION-PERF-SEC-CI-V1` (Parts Q, R, S) | 1 milestone |

Total: 6–8 milestones on top of this session's Package A closure.

## Files

Backend HEAD before this session: `081d636`.
Backend HEAD after this session: (see git log; commits pushed).

Files added:
* `deopt-v2-backend/tests/options_hybrid_v2_identity_wiring_integration.rs` (6 tests, all passing)
* `deopt-v2-backend/docs/OPTIONS_HYBRID_V2_IDENTITY_AND_CORRELATION_WIRING_V1.md` (this file)
* `docs/OPTIONS_HYBRID_V2_IDENTITY_AND_CORRELATION_WIRING_V1_RESULT.md` (workspace result)

Files modified:
* `deopt-v2-backend/src/options/canonical_identity.rs` — added
  `canonical_order_hash_for`, `canonical_execution_id_for_fill`,
  `OPTIONS_CANONICAL_DEPLOYMENT_ID`, `OPTIONS_CANONICAL_CHAIN_ID`.
* `deopt-v2-backend/src/options/types.rs` — added
  `canonical_order_hash` and `canonical_execution_id` fields.
* `deopt-v2-backend/src/options/service.rs` — wired hash into
  `submit_option_order_inner`.
* `deopt-v2-backend/src/options/conditional_orders.rs` — wired hash
  into TP/SL child order creation.
* `deopt-v2-backend/src/options/store.rs` — in-memory
  `option_fill_from_match` derives execution id.
* `deopt-v2-backend/src/db/repository.rs` — DB matcher
  `option_fill_from_match` derives execution id; INSERT/SELECT/
  RETURNING SQL extended; hydration reads new columns.
* `deopt-v2-backend/src/api/trading.rs` — 5 test fixtures updated
  with explicit `None` for new fields.
* `deopt-v2-backend/tests/subaccounts_options_orders_history_tests.rs`
  — 4 test fixtures updated.

## Next stage

Recommended next milestone:
`OPTIONS-HYBRID-V2-EXECUTION-INTENT-LINKAGE-V1` (F-order 1 above).

The originally-scoped `OPTIONS-HYBRID-V2-OPEN-ORDER-RESERVATION-V1`
requires at least F-orders 1–4 to land first, so that reservation
transitions can reference `canonical_execution_id` and canonical
event correlation is available for settlement release.
