# OPTIONS-HYBRID-V2-CORRELATION-ATOMIC-WIRING-V1

Closure notes for the F3 milestone of the correlation programme.

Scope (narrow):

1. Create the `AWAITING_CHAIN_EVIDENCE` correlation record atomically
   with the canonical execution intent so no observable half-state
   exists.
2. Populate every pre-chain fingerprint that can be known before the
   transaction is mined.
3. Attach the authoritative `tx_hash` to the correlation row from the
   Options broadcast trust boundary.
4. Prove the lifecycle with a real-PostgreSQL matrix (26 cases) and
   bounded properties (6 cases).

Explicitly out of scope: the canonical event correlation reducer that
transitions `SUBMITTED → CORRELATED_CANONICAL`. That is
`OPTIONS-HYBRID-V2-CANONICAL-EVENT-CORRELATION-V1`.

## True transaction boundary

Audit of the Options execution intent lifecycle (Part B) identified
that `submit_option_order_and_match` (the matcher's DB transaction) is
NOT the natural place to insert the execution intent — the intent is
constructed at the service layer AFTER the match commits, from a
persisted `OptionFill` row. Moving intent construction inside the
matcher would require a large refactor.

The chosen boundary keeps intent construction outside the matcher
transaction but makes the pair `(intent INSERT, AWAITING correlation
INSERT)` atomic inside a dedicated PG transaction:

```
BEGIN;
    INSERT option_execution_intents (... canonical_execution_id ...) ON CONFLICT (source_type, source_id) DO NOTHING;
    INSERT option_execution_correlations (... AWAITING_CHAIN_EVIDENCE ...) ON CONFLICT (canonical_execution_id) DO NOTHING;
COMMIT;
```

Rollback removes both. The invariant `NEW canonical execution intent
implies pre-chain correlation` becomes a schema-enforced
transactional guarantee.

## Repository transaction support

`PgRepository::insert_option_execution_intent_with_awaiting_correlation`
opens a single tx, calls two `_tx` writers, commits. Idempotency is
served by:

- The intent INSERT: `ON CONFLICT (source_type, source_id) DO NOTHING`
  (existing behaviour).
- The correlation INSERT: new
  `upsert_awaiting_correlation_tx` respects the sparse UNIQUE index on
  `canonical_execution_id` (WHERE status IN active states) — a repeat
  call returns the existing ACTIVE row with a `(deployment_id,
  chain_id, execution_kind)` cross-check that fails closed on
  mismatch.

Retry semantics: a duplicated service invocation (client retry,
request de-duplication race) always leaves exactly one intent + one
correlation regardless of interleaving.

## Prechain fingerprint population

Fields populated at insert time (all knowable before mining):

| Field | Source |
| --- | --- |
| `canonical_execution_id` | `intent.canonical_execution_id` (from fill) |
| `deployment_id` | `OptionsCanonicalDomain::from_options_config()` |
| `chain_id` | Same |
| `execution_kind` | `source_type` mapped (`Trade` or `RfqTrade`) |
| `fill_quantity_1e8` | `intent.source_size_1e8` |

Fields left NULL (unknown pre-chain):

- `onchain_execution_id` (chain-computed with block coordinates)
- `tx_hash`, `canonical_block_number`, `canonical_block_hash`,
  `log_index` (post-mine)
- `onchain_buyer_order_id`, `onchain_seller_order_id` — the current
  production execution surface uses the legacy `OptionMatchingEngine`
  which emits `OptionTradeExecuted` (not `OptionOrderPairExecuted`);
  the legacy event topics do not include `buyerOrderId` /
  `sellerOrderId`. The reducer binds via `(tx_hash, log_index)` per
  the Part C uniqueness proof.

## Service wiring

`create_option_orderbook_execution_intent_with_nonce_provider` chooses
the atomic path when `fill.canonical_execution_id.is_some()`:

- Matcher-derived fills → atomic
  `insert_option_execution_intent_with_awaiting_correlation`.
- RFQ / user-initiated / legacy no-canonical fills → intent-only
  `insert_option_execution_intent`. The correlation table is only
  meaningful when the identity chain is complete; F-order 3 (RFQ /
  quote linkage) is out of scope.

## Options tx submission trust model

Backend relayer path (single production model):

1. Client submits an order via `POST /options/orders` (nothing to sign
   yet on the outer tx layer).
2. Frontend flow collects the buyer + seller EIP-712 signatures
   authorising the trade payload.
3. Backend calls `broadcast_option_execution_intent_*` — inside this
   function the backend signer (KMS or LocalDev) signs the OUTER
   EIP-1559 relayer transaction which carries the user signatures
   inside its calldata (`executeTrade` / `executeRfqTrade`).
4. `provider.send_raw_transaction(raw)` returns the authoritative
   `tx_hash` from the RPC provider. This is the first trustworthy
   observation of the tx identity.

Because the user's authorisation is inside the calldata and the
outer tx is signed by the backend relayer key, the tx_hash origin is
"backend observation from own broadcast" — NOT a client claim.

## Authoritative tx hash attachment

Wired at the RPC observation point in
`broadcast_option_execution_intent_with_provider_signer_and_data_provider`.
After `send_raw_transaction` returns a valid tx_hash and the local
`option_execution_transactions` row is persisted, the backend calls
`correlation_repository::attach_tx_hash(pool, canonical_execution_id,
tx_hash, now)`.

Guardrails from migration 0055 immutability + repository logic:

- Same-value re-attach on the same canonical_execution_id → succeeds
  (the WHERE clause on the UPDATE tolerates equal values).
- Different-value attempt → the WHERE clause excludes the row,
  UPDATE returns 0 rows, function fails closed.
- Unknown canonical_execution_id → UPDATE returns 0 rows, fails
  closed.
- Cross-deployment correlations with the same canonical_execution_id
  cannot exist because the correlation write in Part C cross-checks
  `deployment_id` on retry.

If attachment fails after the RPC succeeded (correlation lookup
missing / transient DB error), a `warn!` is emitted, the caller
still receives the successful broadcast outcome, and the reducer's
canonical event ingestion (F4) will bind the tx via `(tx_hash,
log_index)` on the AWAITING row.

## Submission failure semantics

The pre-existing broadcast failure paths (signer denied, invalid
tx_hash format, RPC error) never reach the attachment site — the
correlation stays in `AWAITING_CHAIN_EVIDENCE`. This is safe: no
tx_hash was ever produced. Operators can re-broadcast; the atomic
insert's idempotent upsert returns the existing correlation row.

No fabricated `SUBMITTED` state. No fabricated tx_hash. Nothing marked
canonically settled — that transition is reducer-driven (F4).

## Half-state policy

Historical rows created BEFORE this milestone:

- Intents with NULL `canonical_execution_id`: legacy pre-migration
  intents. Correctly excluded from the correlation model — no repair
  needed.
- Intents with `canonical_execution_id` but no correlation row:
  possible only if the intent was created between the Package A /
  Part E landing and this milestone. Not reachable via the new atomic
  path. If such rows exist in a production database, the reducer must
  treat them as historical (correlation absent → no cross-check
  possible; behaviour is best-effort).

New code makes the half-state impossible for every subsequent intent.

## Restart / idempotency

- Restart after a successful atomic insert: correlation stays
  `AWAITING_CHAIN_EVIDENCE`. The next service call for the same fill
  is idempotent.
- Restart after `attach_tx_hash` succeeded: correlation stays
  `SUBMITTED` with the immutable `tx_hash`. A repeat attach is
  idempotent on same-value.
- Duplicate atomic invocation after restart: same idempotency —
  exactly one intent + one correlation.

## PostgreSQL proof

`tests/options_hybrid_v2_atomic_wiring_pg_integration.rs` — 26
tests covering:

- Atomic persistence (8): C01–C08 including precondition rollback,
  duplicate retry, prior correlation reuse, cross-deployment
  fail-closed, legacy no-canonical intent path.
- Prechain fingerprint (6): C09–C14 covering populated + NULL fields,
  immutability, intent↔correlation identity match.
- Tx attachment (6): C15–C20 covering first-attach, same-value
  idempotency, conflicting-value fail-closed, unknown-id fail-closed,
  cross-deployment tolerance, intent-untouched invariant.
- Restart (3): C21–C23 covering restart-after-AWAITING,
  restart-after-SUBMITTED, duplicate-after-restart.
- Concurrency (3): C24–C26 covering simultaneous duplicate insert,
  simultaneous same-value attach, simultaneous conflicting attach.

Loud-fail gate: the test file panics if `OPTIONS_ATOMIC_WIRING_PG_URL`
is unset (unless `OPTIONS_ATOMIC_WIRING_PG_ALLOW_SKIP=1` is
explicitly set for dev-only opt-out). Every successful run prints
`REAL_POSTGRES_CONNECTION_CONFIRMED url_hash=...` so verdict
evidence is machine-scannable without leaking the URL.

Verified run against local disposable PG 16 (`postgres://deopt:deopt@127.0.0.1:5432/deopt_v2_backend`):

```
test result: ok. 26 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 2.92s
```

## Bounded properties

`tests/options_hybrid_v2_atomic_wiring_properties.rs` — 6
properties over deterministic seed sets (no `proptest` dep, per
project convention):

- P1: new canonical matcher-derived intent never commits without
  correlation (8 seeds).
- P2: precondition rollback preserves zero half-state (5 seeds).
- P3: duplicate atomic invocation is idempotent regardless of repeat
  count (3 seeds × 2–5 repeats).
- P4: same-value `attach_tx_hash` is idempotent (3 seeds × 2–6
  repeats).
- P5: different `tx_hash` cannot overwrite (4 seeds).
- P6: process restart preserves exact linkage (3 seeds).

Verified: `6 passed; 0 failed`.

## Security review

- Forged canonical_execution_id: identity is derived deterministically
  from the two orders' `canonical_order_hash` values, which are
  themselves computed at order insert time under
  `OptionsCanonicalDomain`. Orders require valid user EIP-712
  signatures. No external actor can inject a chosen id.
- Wrong fill / intent binding: `create_option_orderbook_execution_intent`
  threads exactly `fill.canonical_execution_id`. There is no path in
  the service layer to inject a different identity.
- Half-state crash window: closed by atomic transaction.
- Duplicate intent race: `ON CONFLICT (source_type, source_id) DO
  NOTHING` combined with the correlation sparse UNIQUE produces
  exactly-once semantics.
- Forged tx_hash: `attach_tx_hash` is called only from the successful
  return of `provider.send_raw_transaction` inside the broadcast
  path. No API surface accepts a client-provided tx_hash.
- Unauthenticated tx claim: N/A — client cannot claim tx_hash under
  the current trust model.
- Cross-subaccount claim: N/A — attachment is keyed by
  `canonical_execution_id` which already encodes subaccount identity
  via the canonical order hashes.
- Cross-deployment tx linkage: retry cross-check inside
  `upsert_awaiting_correlation_tx` fails closed on mismatched
  `deployment_id`.
- Overwrite of authoritative tx identity: `attach_tx_hash` UPDATE
  WHERE clause `(tx_hash IS NULL OR tx_hash = $2)` returns 0 rows on
  different values; migration 0055 immutability trigger enforces the
  same at the schema layer.

Frozen invariants preserved:

- `OPTIONS_EXECUTION_REMAINS_USER_SIGNED_EIP712`: the trade payload
  authorisation surface is unchanged. `canonical_execution_id` is a
  backend-derived identifier, not an authorisation token.
- `NEW_CANONICAL_EXECUTION_INTENT_REQUIRES_PRECHAIN_CORRELATION`:
  enforced by the atomic writer.
- `TX_HASH_ATTACHMENT_IS_IDEMPOTENT_FOR_THE_SAME_VALUE`: proven by
  C16, P4.
- `TX_HASH_REPLACEMENT_WITH_A_DIFFERENT_VALUE_FAILS_CLOSED`: proven
  by C17, C26, P5.
- `NO_BACKEND_KMS SIGNING FOR OPTIONS`: no new signer added; the
  existing signer path is unchanged.
- `NO_REAL_CHAIN WRITE`: no test in this milestone calls a real RPC;
  every submission is either mocked or does not run.

## Regression

- `cargo test --lib options ...` — 301 pass, 0 fail.
- `cargo test --test options_tests` — 152 pass, 6 ignored, 0 fail.
- `cargo test --test options_hybrid_v2_intent_linkage_integration` — 2
  pass, 0 fail.
- `cargo test --test options_hybrid_v2_identity_wiring_integration` —
  8 pass, 0 fail.
- `cargo test --test options_hybrid_v2_correlation_repository_pg_integration`
  (with real PG) — 12 pass, 0 fail.
- `cargo test --test options_hybrid_v2_subaccount_matcher_integration`
  — 8 pass, 0 fail.
- Full workspace: only pre-existing failure in
  `hybrid_v2_rebuild_operations_properties::reconciliation_drift_never_repairs_projection`
  (verified on pristine `main` HEAD `883af79` — not introduced by
  this milestone).

## Files touched

- `src/db/repository.rs` — extracted intent INSERT SQL + query bind
  helper; added `insert_option_execution_intent_tx`; added
  `PgRepository::insert_option_execution_intent_with_awaiting_correlation`.
- `src/options/correlation_repository.rs` — added
  `OptionCorrelationStatus::is_active()` +
  `upsert_awaiting_correlation_tx`.
- `src/options/service.rs` — added `OptionsCanonicalDomain` +
  `correlation_repository` imports; added
  `insert_option_execution_intent_with_awaiting_correlation` service
  wrapper + `build_awaiting_correlation_input` helper; wired the
  atomic path into `create_option_orderbook_execution_intent_with_nonce_provider`;
  wired `attach_tx_hash` into the broadcast success path.
- `tests/options_hybrid_v2_atomic_wiring_pg_integration.rs` — 26-case
  real-PG matrix.
- `tests/options_hybrid_v2_atomic_wiring_properties.rs` — 6 bounded
  properties.

## Next milestone

`OPTIONS-HYBRID-V2-CANONICAL-EVENT-CORRELATION-V1` — the reducer that
correlates on-chain `OptionTradeExecuted` / `OptionRfqTradeExecuted`
events with the AWAITING / SUBMITTED correlation rows via
`(tx_hash, log_index)` and transitions to `CORRELATED_CANONICAL`.
