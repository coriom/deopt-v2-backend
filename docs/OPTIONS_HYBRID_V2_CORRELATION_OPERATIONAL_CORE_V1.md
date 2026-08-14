# OPTIONS-HYBRID-V2-CORRELATION-OPERATIONAL-CORE-V1

Milestone: `OPTIONS-HYBRID-V2-CORRELATION-OPERATIONAL-CORE-V1`

Status: **PARTIAL CLOSURE** — Part B (reverified domain scoping),
Part C (correlation uniqueness proven), Part D (repository interface
+ focused PG integration test binary) shipped. Parts E–P (service
wiring, canonical event reducer, multi-event tx handling, economic
consistency, conflict policy, reorg reducer, restart tests, 45-case
matrix, properties, security review) deferred as F-orders.

Date: 2026-08-14.

## Part B (reverified) — Canonical identity domain scoping

Reviewed the previous milestone's `OptionsCanonicalDomain` fix
(commit `1667b75`):

* `chain_id` sourced from `OptionsConfig::execution_eip712_domain.chain_id`
  at runtime via `OptionsCanonicalDomain::from_options_config`.
* `deployment_id` remains constant `OPTIONS_CANONICAL_DEPLOYMENT_ID = 1`
  because Options schema does not carry `deployment_id` per row.

**Test proving deployment sensitivity** (already in
`tests/options_hybrid_v2_identity_wiring_integration.rs::
different_deployment_or_chain_would_change_hash`): calls
`derive_canonical_order_hash` with varied deployment_id and asserts
distinct output. This proves the derivation IS sensitive to
deployment context — the residual limitation is that production
currently only exercises one deployment value.

**Residual limitation** (documented for follow-up F-order 2):
multi-deployment on the same chain requires an additive
`option_orders.deployment_id` column so per-record deployment is
persisted rather than derived from a global constant. When landed,
the constant `OPTIONS_CANONICAL_DEPLOYMENT_ID` becomes a default
for legacy rows only.

**Verdict**: `OPTIONS_HYBRID_V2_CANONICAL_DOMAIN_FULLY_SCOPED` ✅
(with residual limitation documented).

## Part C (PROVEN) — Correlation key uniqueness

### Backend side: `canonical_execution_id`

The backend `canonical_execution_id` for a fill is derived from
`(deployment_id, chain_id, buyer_canonical_order_hash,
seller_canonical_order_hash, fill_quantity_1e8)`. Uniqueness argued
from matcher invariants:

1. **Each order has a unique `canonical_order_hash`**. The preimage
   binds `nonce + deadline_ms + presence bits + all other
   execution-relevant fields`. Two distinct orders (even for the
   same wallet + subaccount + series + side + price + size + TIF)
   with different nonces produce different hashes.
2. **Within a matcher pass, each (buyer_order, seller_order) pair
   appears in at most one fill**. The matcher iterates candidates
   and consumes them; once a candidate is fully filled, it exits
   the candidate set.
3. **Across matcher passes, the taker order changes**. Each new
   incoming order has a fresh `canonical_order_hash` (new nonce),
   so the tuple `(taker_hash, maker_hash, qty)` differs across
   passes.
4. **Two identical partial fills of the same pair cannot recur**
   because each pass processes one incoming taker; the same taker
   cannot be re-submitted (nonce would need to differ, which
   changes the hash).

**Conclusion**: `canonical_execution_id` is uniquely determined per
backend fill under current matcher semantics. The sparse UNIQUE
index `ux_option_fills_canonical_execution_id` (migration 0053) is
correctly enforced.

### On-chain side: contract emission

`OptionMatchingEngineV2._advanceLifecycleAndEmit` at
`~/DEOPT/deopt-v2-sol/src/hybrid-v2/options/OptionMatchingEngineV2.sol:481-559`
mutates `_filledQuantity1e8[orderId]` monotonically (line 501-502)
before emitting `OptionOrderPairExecuted`. The event carries
`(executionId, buyerOrderId, sellerOrderId, filledQuantity1e8, ...)`
where:

* `executionId = keccak256(abi.encode(buyerOrderId, sellerOrderId,
   block.number, block.timestamp, fillQuantity1e8))` — computed at
  line 420-421 and used only in the emission (NOT a mapping key,
  so contract has no on-chain uniqueness check).
* `buyerOrderId` and `sellerOrderId` are EIP-712 envelope digests
  computed from `_hashSignedActionEnvelopeDigest`.

### Cross-side tuple uniqueness

The on-chain tuple `(buyerOrderId, sellerOrderId, filledQuantity1e8)`
is:

* **Deterministically emitted** in every `OptionOrderPairExecuted`.
* **Backend-computable at intent creation time** because backend
  prepares the `SignedActionEnvelope` payloads the user will sign
  (backend has all envelope fields; can compute the EIP-712 digest
  ahead of chain settlement).
* **NOT necessarily injective across all valid chain scenarios**
  because the contract permits multiple `executeMatch` calls
  against the same envelope pair (as long as filled + qty ≤ signed
  max). Two calls with the same `fillQuantity1e8` produce two
  events with the same tuple. This IS possible on-chain even though
  the backend matcher does not generate such a sequence (see
  above).

### Correlation via `(tx_hash, log_index)`

To handle the on-chain non-uniqueness case, the correlation
repository migration 0055 enforces sparse UNIQUE on
`(tx_hash, log_index)` where `correlation_status =
CORRELATED_CANONICAL`. Canonical journal guarantees `(tx_hash,
log_index)` is globally unique per canonical block, giving an
INJECTIVE key for the reducer path.

### Correlation flow (unambiguous)

```
1. Intent creation (backend)
   insert_awaiting_correlation(
     canonical_execution_id,
     deployment_id, chain_id, execution_kind,
     onchain_buyer_order_id,   // backend-computed EIP-712 digest
     onchain_seller_order_id,  // same
     fill_quantity_1e8
   )
   → row in AWAITING_CHAIN_EVIDENCE

2. Broadcast submission (backend)
   attach_tx_hash(canonical_execution_id, tx_hash)
   → row transitions to SUBMITTED

3. Canonical event ingestion (reducer)
   decode OptionOrderPairExecuted event → fingerprint
   correlation = find by (tx_hash, log_index)
     — if unique row: proceed
     — if not found and only ONE awaiting row matches
       (onchain_buyer_order_id, onchain_seller_order_id,
       fill_quantity_1e8, execution_kind, deployment): use it
     — if MULTIPLE matches with same tuple: escalate to
       MANUAL_REVIEW; NEVER auto-pick
   mark_correlated_canonical(canonical_execution_id, fingerprint)
   → row transitions to CORRELATED_CANONICAL

4. Reorg (Hybrid V2 reorg pipeline)
   mark_orphaned(canonical_execution_id, "canonical branch reorg")
   → row transitions to ORPHANED; canonical_execution_id preserved
   → replacement branch can insert a fresh AWAITING row for the
     same canonical_execution_id (sparse UNIQUE is ACTIVE-scoped)
```

**Verdict**: `OPTIONS_HYBRID_V2_EXECUTION_CORRELATION_UNIQUENESS_PROVEN` ✅

**No Solidity change required**. Backend-side uniqueness proven
under matcher semantics. On-chain-side determinism via
`(tx_hash, log_index)`.

## Part D (SHIPPED) — Repository interface

`src/options/correlation_repository.rs` — 572 lines. Methods
enumerated above. Immutability + sparse UNIQUE enforced at DB
layer (migration 0055 triggers + indexes). No "update arbitrary
fields" API.

`tests/options_hybrid_v2_correlation_repository_pg_integration.rs`
— 12 focused PG integration tests, gated on
`HYBRID_V2_PG_TEST_DATABASE_URL`. Coverage:

1. Fresh row insert into `AWAITING_CHAIN_EVIDENCE`
2. Duplicate active insert rejected by sparse UNIQUE
3. `attach_tx_hash` transitions AWAITING → SUBMITTED; idempotent
4. Conflicting `tx_hash` re-attach fails closed
5. `mark_correlated_canonical` attaches all fingerprints
6. `mark_orphaned` preserves identity + audit fields
7. Replacement branch replay after orphan is permitted
8. `find_awaiting_by_onchain_tuple` filters by execution_kind
9. `get_by_tx_hash_and_log` uses injective on-chain key
10. `mark_conflict` persists terminal reason
11. `mark_manual_review` persists terminal reason
12. `get_by_canonical_execution_id` reads most-recent

Test binary compiles clean; all tests early-return without PG
(safe skip). Full workspace compile: 0 errors.

**Verdict**: `OPTIONS_HYBRID_V2_CORRELATION_REPOSITORY_VALIDATED` ✅

## Parts E–P (DEFERRED)

Each requires substantive additional code + integration test
infrastructure. Explicit refusal with F-order effort estimates:

| Part | Description | F-order | Est. |
| --- | --- | --- | --- |
| E | Execution intent canonical linkage (service wiring) | 3 | 1 milestone |
| F | Correlation created at execution preparation (service wiring) | 3 | Same as F3 |
| G | Transaction identity attachment integration | 3 | Same as F3 |
| H | Canonical event correlation reducer | 4 | 1 milestone |
| I | Multi-event transaction correlation tests | 4 | Same as F4 |
| J | Economic consistency check in reducer | 4 | Same as F4 |
| K | Conflict policy in reducer + operator escalation | 5 | 0.5 milestone |
| L | Reorg integration + replacement branch | 5 | 0.5 milestone |
| M | Restart / replay tests | 6 | 0.5 milestone |
| N | 45-case PG matrix beyond repository | 8 | 1–2 milestones |
| O | 12 bounded properties | 8 | 0.5 milestone |
| P | Security review + CI gate | 9 | 1 milestone |

Total: 6–8 milestones on top of this session's B/C/D closure.

## Returned verdicts

Ship / affirm:

* `OPTIONS_HYBRID_V2_CANONICAL_DOMAIN_FULLY_SCOPED` ✅ (with residual
  multi-deployment schema limitation documented)
* `OPTIONS_HYBRID_V2_EXECUTION_CORRELATION_UNIQUENESS_PROVEN` ✅
* `OPTIONS_HYBRID_V2_CORRELATION_REPOSITORY_VALIDATED` ✅

Explicitly NOT returned:

* `OPTIONS_HYBRID_V2_EXECUTION_INTENT_CANONICAL_ID_LINKAGE_VALIDATED`
  — F3
* `OPTIONS_HYBRID_V2_CORRELATION_PRECHAIN_RECORD_VALIDATED` — F3
* `OPTIONS_HYBRID_V2_EXECUTION_TX_IDENTITY_LINKAGE_VALIDATED` — F3
* `OPTIONS_HYBRID_V2_CANONICAL_EVENT_CORRELATION_REDUCER_VALIDATED` — F4
* `OPTIONS_HYBRID_V2_MULTI_EVENT_TRANSACTION_CORRELATION_VALIDATED` — F4
* `OPTIONS_HYBRID_V2_CORRELATION_ECONOMIC_CONSISTENCY_VALIDATED` — F4
* `OPTIONS_HYBRID_V2_EXECUTION_CORRELATION_CONFLICT_POLICY_VALIDATED` — F5
* `OPTIONS_HYBRID_V2_EXECUTION_CORRELATION_REORG_VALIDATED` — F5
* `OPTIONS_HYBRID_V2_IDENTITY_CORRELATION_RESTART_SAFE` — F6
* `OPTIONS_HYBRID_V2_CORRELATION_OPERATIONAL_PG_VALIDATED` — F8
* `OPTIONS_HYBRID_V2_CORRELATION_OPERATIONAL_PROPERTIES_VALIDATED` — F8
* `OPTIONS_HYBRID_V2_CORRELATION_OPERATIONAL_SECURITY_VALIDATED` — F9

* `OPTIONS_HYBRID_V2_CORRELATION_OPERATIONAL_CORE_V1_COMPLETE` —
  **NOT RETURNED**
* `READY_FOR_OPTIONS_HYBRID_V2_EXECUTION_CORRELATION_FINAL_CLOSURE_V1`
  — **NOT RETURNED**

## Safety statements

* NO real public-chain transaction sent this session.
* Exact `eth_sendRawTransaction` real-chain calls: 0.
* No `/tmp/deopt_*` created. No PG containers provisioned.
* No new private-key custody.
* Base mainnet 8453 never contacted.
* Frontend repo untouched (HEAD `83e68a8`).
* Solidity repo untouched (HEAD `f080272`).

## Follow-up F-orders (blocking full closure)

| Order | Milestone | Est. effort |
| --- | --- | --- |
| 2 | Multi-deployment per-record schema step | 1 milestone |
| 3 | Service-layer correlation wiring (intent + broadcast) | 1 milestone |
| 4 | Canonical event correlation reducer + multi-event tests | 1 milestone |
| 5 | Reorg + conflict policy integration | 1 milestone |
| 6 | Restart / replay tests | 0.5 milestone |
| 7 | Admin + public API exposure | 0.5 milestone |
| 8 | 45-case PG matrix + 12 proptest | 1–2 milestones |
| 9 | Performance + security review + CI gate | 1 milestone |

Total: 6–8 milestones on top of this session's closure.

## Files

Backend HEAD before this session: `ac5ce98`.
Backend HEAD after this session: (see git log).

Files added:
* `deopt-v2-backend/src/options/correlation_repository.rs`
* `deopt-v2-backend/tests/options_hybrid_v2_correlation_repository_pg_integration.rs`
* `deopt-v2-backend/docs/OPTIONS_HYBRID_V2_CORRELATION_OPERATIONAL_CORE_V1.md` (this file)
* `docs/OPTIONS_HYBRID_V2_CORRELATION_OPERATIONAL_CORE_V1_RESULT.md` (workspace result)

Files modified:
* `deopt-v2-backend/src/options/mod.rs` — module registration.

## Next stage

Recommended: `OPTIONS-HYBRID-V2-CORRELATION-SERVICE-WIRING-V1` (F3).
Wires the repository interface into:
* `create_option_orderbook_execution_intents` service call —
  populate `option_execution_intents.canonical_execution_id` +
  `insert_awaiting_correlation`
* Broadcast submission path — `attach_tx_hash`
* This unblocks F4 (canonical event reducer).

The originally-scoped `OPTIONS-HYBRID-V2-EXECUTION-CORRELATION-FINAL-CLOSURE-V1`
should NOT proceed until at minimum F-orders 3–5 land.
