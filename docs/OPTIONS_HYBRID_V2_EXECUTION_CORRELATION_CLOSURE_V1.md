# OPTIONS-HYBRID-V2-EXECUTION-CORRELATION-CLOSURE-V1

Milestone: `OPTIONS-HYBRID-V2-EXECUTION-CORRELATION-CLOSURE-V1`

Status: **PARTIAL CLOSURE** — Part B (domain scoping fix), Part C
(contract/event correlation model resolved), Parts D + E (schema
migrations) shipped. Parts F–S (repository wiring, reducer, event
correlation, reorg, restart, admin/public API, 45-case PG matrix,
12-property proptest, performance, security review, CI, regression)
deferred to follow-up F-orders.

Date: 2026-08-14.

## Purpose

Complete the deterministic identity chain from `canonical_order_hash`
through canonical on-chain evidence, without changing the user-signed
Options execution architecture.

## Part B (SHIPPED) — Canonical identity domain scoping

**Fix**: replaced the process-global `OPTIONS_CANONICAL_CHAIN_ID`
constant with `OptionsCanonicalDomain` sourced at runtime from
`OptionsConfig::execution_eip712_domain.chain_id`.

`src/options/canonical_identity.rs::OptionsCanonicalDomain`:
```rust
pub struct OptionsCanonicalDomain {
    pub deployment_id: i64,   // OPTIONS_CANONICAL_DEPLOYMENT_ID (1)
    pub chain_id: u64,        // config.execution_eip712_domain.chain_id
}

impl OptionsCanonicalDomain {
    pub fn from_options_config(config: &OptionsConfig) -> Self { ... }
    pub const fn constant_test_domain() -> Self { ... }
}
```

Production paths (`submit_option_order_inner`) use
`from_options_config(&state.options_config)`. Test/local-only paths
(in-memory matcher, in-store TP/SL execution) use
`constant_test_domain()` with explicit documentation that they
are unreachable in production posture. Repository-layer
`option_fill_from_match` uses `constant_test_domain()` and produces
the same identity as the config path today (both resolve to chain_id
84532); a future multi-chain milestone must thread the domain
through the repository transaction — documented as F-order 2.

**Residual limitation**: `deployment_id` remains a compile-time
constant (`OPTIONS_CANONICAL_DEPLOYMENT_ID = 1`) because Options
schema does not yet carry `deployment_id` per row. Multi-deployment
support requires an additive schema step + wiring — documented as
F-order 2.

**Verdict**: `OPTIONS_HYBRID_V2_CANONICAL_IDENTITY_DOMAIN_SCOPING_VALIDATED` ✅

## Part C (SHIPPED) — Contract event correlation model resolved

### Event surface audit

`OptionMatchingEngineV2.executeMatch`
(`~/DEOPT/deopt-v2-sol/src/hybrid-v2/options/OptionMatchingEngineV2.sol:348-424`)
emits `OptionOrderPairExecuted`
(`~/DEOPT/deopt-v2-sol/src/hybrid-v2/interfaces/IOptionMatchingEngine.sol:138-159`):

```solidity
event OptionOrderPairExecuted(
    bytes32 indexed executionId,
    bytes32 indexed buyerOrderId,
    bytes32 indexed sellerOrderId,
    uint256 seriesId,
    bytes32 buyerSubKey,
    bytes32 sellerSubKey,
    address buyerOwner,
    address sellerOwner,
    uint32 buyerSubaccountId,
    uint32 sellerSubaccountId,
    uint128 filledQuantity1e8,
    uint128 pricePerContract1e8,
    uint256 totalPremium,
    address premiumToken,
    uint8 buyerRole,
    uint8 sellerRole,
    uint128 buyerFee,
    uint128 sellerFee,
    address actor,
    uint16 eventVersion
);
```

where:
* `buyerOrderId = _hashSignedActionEnvelopeDigest(buyerEnvelope)`
  — the EIP-712 envelope digest signed by the buyer
* `sellerOrderId = _hashSignedActionEnvelopeDigest(sellerEnvelope)`
  — same for seller
* `executionId = keccak256(abi.encode(buyerOrderId, sellerOrderId, block.number, block.timestamp, fillQuantity1e8))`

### Key finding

Backend `canonical_execution_id` and on-chain `executionId` use
**different preimages** (backend preimage tag `HV2_EXEC_V1` + chain
id + hashes + qty; on-chain includes `block.number + block.timestamp`
which the backend cannot know pre-mine). **They will never match by
equality.**

However, the tuple `(buyerOrderId, sellerOrderId, fillQuantity1e8)`
is deterministically emitted in the event AND is computable by the
backend at intent-creation time (backend has all envelope fields
because it prepares the envelope for the user to sign).

### Resolved correlation model

**Deterministic correlation key**:
`(onchain_buyer_order_id, onchain_seller_order_id, fill_quantity_1e8)`.

**Path**:
1. Backend prepares the two `SignedActionEnvelope` payloads for the
   user to sign.
2. Backend computes the EIP-712 envelope digest for each — this
   equals what the contract will compute as `buyerOrderId` /
   `sellerOrderId`.
3. Backend persists `(canonical_execution_id, onchain_buyer_order_id,
   onchain_seller_order_id, fill_quantity_1e8)` on the correlation
   row at intent-creation time (`AWAITING_CHAIN_EVIDENCE`).
4. When `tx_hash` is known (post-broadcast), backend attaches it
   (`SUBMITTED`).
5. When `OptionOrderPairExecuted` event is ingested, reducer looks
   up correlation by the tuple, attaches `onchain_execution_id +
   canonical_block_number + canonical_block_hash + log_index`, and
   marks `CORRELATED_CANONICAL`.

**No Solidity change required**. **No heuristic correlation**.

**Verdict**: `OPTIONS_HYBRID_V2_EXECUTION_EVENT_CORRELATION_MODEL_RESOLVED` ✅

## Part D (SHIPPED, schema only) — Execution intent canonical linkage

Migration `0054_option_execution_intents_canonical_execution_id.sql`:
* Adds nullable `option_execution_intents.canonical_execution_id`
* Sparse UNIQUE index over non-NULL values
* Immutability trigger (once set, cannot change)

**What's NOT shipped**: repository INSERT/SELECT wiring to actually
populate the new column. Deferred to F-order 3 (needs to touch
`insert_option_execution_intent` + hydration + intent-creation
service call site).

**Verdict**:
`OPTIONS_HYBRID_V2_EXECUTION_INTENT_CANONICAL_ID_LINKAGE_VALIDATED` —
**NOT RETURNED**. Schema landed but service-layer wiring is deferred.

## Part E (SHIPPED, schema only) — Correlation schema

Migration `0055_option_execution_correlations.sql`:
* New table `option_execution_correlations` with:
  * `correlation_id UUID PRIMARY KEY DEFAULT gen_random_uuid()`
  * `canonical_execution_id TEXT NOT NULL`
  * `deployment_id BIGINT NOT NULL`, `chain_id BIGINT NOT NULL`
  * `execution_kind TEXT NOT NULL CHECK (execution_kind IN ('trade', 'rfq_trade'))`
  * Optional fingerprints: `onchain_buyer_order_id`,
    `onchain_seller_order_id`, `onchain_execution_id`,
    `fill_quantity_1e8`, `tx_hash`, `canonical_block_number`,
    `canonical_block_hash`, `log_index`
  * `correlation_status TEXT NOT NULL CHECK (correlation_status IN
    ('AWAITING_CHAIN_EVIDENCE', 'SUBMITTED', 'CORRELATED_CANONICAL',
    'ORPHANED', 'CONFLICT', 'MANUAL_REVIEW'))`
  * `terminal_reason TEXT NULL`, `first_seen_at_ms`, `last_updated_at_ms`
* Sparse UNIQUE:
  * `(canonical_execution_id)` where status IN ACTIVE — at most one
    active correlation per canonical execution
  * `(tx_hash, log_index)` where status = CORRELATED_CANONICAL —
    two backend executions cannot claim the same on-chain event
* Lookup indexes on `(onchain_buyer_order_id, onchain_seller_order_id,
  fill_quantity_1e8)`, `tx_hash`, `onchain_execution_id`,
  `(deployment_id, correlation_status)`
* Immutability trigger: identity + kind + all populated on-chain
  fingerprints are immutable-once-set

**What's NOT shipped**: repository methods
(`insert_option_execution_correlation`, `attach_tx_hash_to_correlation`,
`mark_correlation_settled`, `mark_correlation_orphaned`,
`mark_correlation_conflict`, `find_correlation_by_onchain_tuple`).
Deferred to F-order 3.

**Verdict**:
`OPTIONS_HYBRID_V2_EXECUTION_CORRELATION_SCHEMA_VALIDATED` —
schema-only landed; full repository interface deferred.

## Parts F–S (DEFERRED)

Each requires substantive additional code + PG test infrastructure:

### Part F — User-signed execution preparation wiring (DEFERRED — F-order 3)

Wire correlation metadata population at three points:
* `create_option_orderbook_execution_intents` (service.rs) — insert
  correlation row with `AWAITING_CHAIN_EVIDENCE`, populate
  `onchain_buyer_order_id` + `onchain_seller_order_id` (backend
  computes envelope digests)
* Broadcast decision path (broadcast_policy.rs) — no change unless
  broadcast_policy emits pre-broadcast marker; keep as-is
* Post-broadcast tx submission — attach `tx_hash`, transition to
  `SUBMITTED`

Est. effort: 1 milestone.

### Part G — Canonical event correlation reducer (DEFERRED — F-order 4)

Extend `src/hybrid_v2/reducer.rs::apply` for `EventKind::
OptionOrderPairExecuted`:
* Decode `(buyerOrderId, sellerOrderId, filledQuantity1e8, executionId,
  block_number, block_hash, tx_hash, log_index)` from the event.
* `SELECT ... FROM option_execution_correlations WHERE
   onchain_buyer_order_id = $1 AND onchain_seller_order_id = $2 AND
   fill_quantity_1e8 = $3 AND correlation_status IN ('AWAITING_CHAIN_EVIDENCE', 'SUBMITTED')`.
* If exactly one row → mark `CORRELATED_CANONICAL` + populate
  `onchain_execution_id + canonical_block_* + log_index`.
* If zero rows → this is an execution the backend never persisted an
  intent for — persist as ORPHANED / SOURCED_OFFCHAIN state (design
  decision needed).
* If multiple rows → CONFLICT (fail-closed; operator MANUAL_REVIEW).
* Idempotent: duplicate event delivery is a no-op via correlation_id
  lookup.

Est. effort: 1 milestone.

### Part H — Multi-event transaction correlation (DEFERRED — F-order 4)

The lookup by `(onchain_buyer_order_id, onchain_seller_order_id,
fill_quantity_1e8)` is already deterministic per execution, so
multi-event transactions don't need special handling — each
`OptionOrderPairExecuted` event resolves independently. Tests
required to prove this per the milestone brief.

### Part I — Reorg semantics (DEFERRED — F-order 5)

Extend `src/hybrid_v2/reorg_recovery.rs` to invert Options
correlations when their canonical block is orphaned:
* `CORRELATED_CANONICAL` → `ORPHANED`
* If replacement canonical event lands, allow same
  `canonical_execution_id` to re-correlate (new tx_hash / log_index).

Est. effort: 1 milestone.

### Part J — Conflict/fail-closed policy (DEFERRED — F-order 5)

Repository method `mark_correlation_conflict` + admin surface
exposure + operator resolution flow. Fail-closed default: conflicting
state remains until operator resolves; execution never presents as
canonically settled.

Est. effort: 0.5 milestone.

### Part K — Restart/replay (DEFERRED — F-order 6)

Restart tests covering the 9 lifecycle checkpoints per the brief.
Requires PG integration test binary.

Est. effort: 0.5 milestone.

### Part L — Admin lifecycle exposure (DEFERRED — F-order 7)

Add correlation state to
`GET /admin/options/executions/:canonical_execution_id` or equivalent
admin route. Sanitize signatures; distinguish backend economic
identity from user-signed digest.

Est. effort: 0.5 milestone.

### Part M — Public API additive fields (DEFERRED — F-order 7)

Additive `canonical_execution_id` + `correlation_status` on
`GET /options/fills` response DTOs. Backward compatible.

Est. effort: 0.5 milestone.

### Part N — 45-case disposable PG matrix (DEFERRED — F-order 8)

Enumerated in the milestone brief. Requires provisioning
`HYBRID_V2_PG_TEST_DATABASE_URL` + writing per-case test file.

Est. effort: 1–2 milestones.

### Part O — 12 bounded proptest (DEFERRED — F-order 8)

Property scenarios enumerated in the brief.

Est. effort: 0.5 milestone.

### Part P — Performance observations (DEFERRED — F-order 9)

Hot-path lookup benchmarks.

Est. effort: 0.5 milestone.

### Part Q — Security review (DEFERRED — F-order 9)

Comprehensive doc + attack-surface tests.

Est. effort: 0.5 milestone.

### Part R — CI gate (DEFERRED — F-order 9)

PostgreSQL CI workflow extension.

Est. effort: 0.5 milestone.

### Part S — Regression (PARTIAL — this session verified existing behavior)

* `cargo test --test options_tests`: 152/152 pass
* `cargo test --test options_hybrid_v2_identity_wiring_integration`: 6/6 pass
* `cargo check --workspace --all-targets`: 0 errors
* No behavior regression from Part B refactor.

## Returned verdicts

Ship / affirm (Parts B + C + partial D + partial E):

* `OPTIONS_HYBRID_V2_CANONICAL_IDENTITY_DOMAIN_SCOPING_VALIDATED` ✅
* `OPTIONS_HYBRID_V2_EXECUTION_EVENT_CORRELATION_MODEL_RESOLVED` ✅

Explicitly NOT returned in this session:

* `OPTIONS_HYBRID_V2_EXECUTION_INTENT_CANONICAL_ID_LINKAGE_VALIDATED`
  — schema landed (migration 0054); service-layer wiring deferred
  to F-order 3.
* `OPTIONS_HYBRID_V2_EXECUTION_CORRELATION_SCHEMA_VALIDATED` —
  schema landed (migration 0055); repository interface + tests
  deferred to F-order 3.
* `OPTIONS_HYBRID_V2_USER_SIGNED_EXECUTION_CORRELATION_WIRED` —
  F-order 3.
* `OPTIONS_HYBRID_V2_CANONICAL_EVENT_CORRELATION_VALIDATED` —
  F-order 4.
* `OPTIONS_HYBRID_V2_MULTI_EVENT_TRANSACTION_CORRELATION_VALIDATED`
  — F-order 4.
* `OPTIONS_HYBRID_V2_EXECUTION_CORRELATION_REORG_VALIDATED` —
  F-order 5.
* `OPTIONS_HYBRID_V2_EXECUTION_CORRELATION_CONFLICT_POLICY_VALIDATED`
  — F-order 5.
* `OPTIONS_HYBRID_V2_IDENTITY_CORRELATION_RESTART_SAFE` — F-order 6.
* `OPTIONS_HYBRID_V2_IDENTITY_CORRELATION_ADMIN_SURFACE_VALIDATED`
  — F-order 7.
* `OPTIONS_HYBRID_V2_IDENTITY_CORRELATION_PUBLIC_SURFACES_VALIDATED`
  — F-order 7.
* `OPTIONS_HYBRID_V2_IDENTITY_CORRELATION_POSTGRES_MATRIX_VALIDATED`
  — F-order 8.
* `OPTIONS_HYBRID_V2_IDENTITY_CORRELATION_PROPERTIES_VALIDATED` —
  F-order 8.
* `OPTIONS_HYBRID_V2_IDENTITY_CORRELATION_PERFORMANCE_BOUNDED` —
  F-order 9.
* `OPTIONS_HYBRID_V2_IDENTITY_CORRELATION_SECURITY_VALIDATED` —
  F-order 9.
* `OPTIONS_HYBRID_V2_IDENTITY_CORRELATION_CI_GATE_VALIDATED` —
  F-order 9.
* `OPTIONS_HYBRID_V2_IDENTITY_CORRELATION_EXISTING_BEHAVIOUR_PRESERVED`
  — this session's Part S check confirmed no regression, but the
  brief requires broader regression across all Options surfaces
  which needs the deferred implementation to be present first.
* `OPTIONS_HYBRID_V2_EXECUTION_CORRELATION_CLOSURE_V1_COMPLETE` —
  **NOT RETURNED**.
* `OPTIONS_HYBRID_V2_IDENTITY_AND_CORRELATION_WIRING_V1_COMPLETE`
  (parent milestone) — **NOT RETURNED**.
* `READY_FOR_OPTIONS_HYBRID_V2_OPEN_ORDER_RESERVATION_V1` — **NOT
  RETURNED**.

## Safety statements

* NO real public-chain transaction sent this session.
* Exact `eth_sendRawTransaction` real-chain calls: 0.
* No `/tmp/deopt_*` created. No PG containers provisioned.
* No new backend private-key custody.
* Base mainnet 8453 never contacted.
* Frontend repo untouched (HEAD `83e68a8`).
* Solidity repo untouched (HEAD `f080272`).

## Follow-up F-orders (blocking full closure)

| Order | Milestone | Est. effort |
| --- | --- | --- |
| 1 | Repository interface for option_execution_intents.canonical_execution_id + option_execution_correlations | 1 milestone |
| 2 | Multi-chain / multi-deployment schema step (per-record deployment_id + chain_id) | 1 milestone |
| 3 | Service-layer wiring: populate correlation at intent creation + attach tx_hash post-broadcast (Parts F, D wiring) | 1 milestone |
| 4 | Canonical event correlation reducer + multi-event tx tests (Parts G, H) | 1 milestone |
| 5 | Reorg semantics + conflict policy (Parts I, J) | 1 milestone |
| 6 | Restart tests (Part K) | 0.5 milestone |
| 7 | Admin + public API exposure (Parts L, M) | 0.5 milestone |
| 8 | 45-case PG matrix + 12 property tests (Parts N, O) | 1–2 milestones |
| 9 | Performance + security review + CI gate (Parts P, Q, R) | 1 milestone |

Total: 6–8 milestones on top of this session's B/C/D-schema/E-schema
closure.

## Files

Backend HEAD before this session: `2f914a9`.
Backend HEAD after this session: (see git log).

Files added:
* `deopt-v2-backend/migrations/0054_option_execution_intents_canonical_execution_id.sql`
* `deopt-v2-backend/migrations/0055_option_execution_correlations.sql`
* `deopt-v2-backend/docs/OPTIONS_HYBRID_V2_EXECUTION_CORRELATION_CLOSURE_V1.md` (this file)
* `docs/OPTIONS_HYBRID_V2_EXECUTION_CORRELATION_CLOSURE_V1_RESULT.md` (workspace result)

Files modified:
* `deopt-v2-backend/src/options/canonical_identity.rs` — added
  `OptionsCanonicalDomain` struct; refactored `canonical_order_hash_for`
  and `canonical_execution_id_for_fill` to accept domain.
* `deopt-v2-backend/src/options/service.rs` — service uses
  `from_options_config(&state.options_config)`.
* `deopt-v2-backend/src/options/conditional_orders.rs` — in-store
  TP/SL uses `constant_test_domain()` (documented as test-only path).
* `deopt-v2-backend/src/options/store.rs` — in-memory matcher uses
  `constant_test_domain()` (documented).
* `deopt-v2-backend/src/db/repository.rs` — DB matcher uses
  `constant_test_domain()` (documented; future F-order 2 threads
  through).
* `deopt-v2-backend/tests/options_hybrid_v2_identity_wiring_integration.rs`
  — test call sites updated with domain.

## Next stage

Recommended: `OPTIONS-HYBRID-V2-CORRELATION-REPOSITORY-AND-WIRING-V1`
(F-orders 1 + 3 combined). This lands the Rust repository interface
for the two new schemas + populates them at intent-creation and
post-broadcast time, closing three of the deferred verdicts.

The originally-scoped `OPTIONS-HYBRID-V2-OPEN-ORDER-RESERVATION-V1`
should NOT proceed until F-orders 1–5 land minimum.
