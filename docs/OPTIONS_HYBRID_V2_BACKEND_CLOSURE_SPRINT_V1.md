# OPTIONS-HYBRID-V2-BACKEND-CLOSURE-SPRINT-V1 — Package A only

Honest partial closure. This document covers **Package A entire**
(A1–A6) which was explicitly authorized after an early conversation
about sprint scope. Packages B–J (reservations, pending settlement,
canonical settlement release, API/history/admin, repository push-down,
consolidated 80-case PG matrix, 20 properties, security, CI) remain
outstanding for a follow-up milestone. Do NOT read this doc as a
close of the full sprint.

## Scope actually delivered

- **A1** — Pre-broadcast tx identity durability. Backend now derives
  the authoritative `tx_hash` locally from the exact signed raw
  transaction bytes BEFORE calling `eth_sendRawTransaction` and
  persists it against the correlation row as the new
  `SUBMISSION_UNKNOWN` state. RPC-ack subsequently transitions to
  `SUBMITTED` after verifying the provider hash agrees byte-for-byte
  with the locally-derived value.

- **A2** — Production event surface audit. The actual production
  Options broadcast path calls the LEGACY
  `OptionMatchingEngine.executeTrade` / `executeRfqTrade`
  (`state.options_config.matching_engine_address`), NOT the V2G-O
  `OptionMatchingEngineV2.executeMatch`. The events emitted are:
  * `OptionTradeExecuted` — orderbook path, existing decoder.
  * `OptionRfqTradeExecuted` — RFQ path, decoder ADDED by this sprint.
  * `TradeExecuted` (MarginEngine) — position mirror.
  * `FeeChargedV2` × 2 (FeesManagerV2) — one per side.
  * `InternalTransfer` (CollateralVault) — premium leg.
  Full correlation key: `(tx_hash, log_index)` per canonical journal.

- **A3** — Canonical event reducer. `correlate_canonical_option_event`
  in `src/options/correlation_repository.rs` promotes correlation
  rows from AWAITING/SUBMISSION_UNKNOWN/SUBMITTED to
  `CORRELATED_CANONICAL` given decoded event evidence. RFQ decoder
  (`decode_option_rfq_trade_executed_log`) added to
  `src/options/event_indexer.rs`.

- **A4** — Multi-event / multi-execution handling. Per Solidity
  audit, one `executeTrade` / `executeRfqTrade` call produces one
  execution event; multi-execution in one outer tx is not currently
  possible. The reducer disambiguates within a tx by `log_index`
  and cross-checks event_kind against the correlation kind. Second
  event claiming the same `canonical_execution_id` → `CONFLICT`.

- **A5** — Economic consistency cross-check. Before promoting to
  `CORRELATED_CANONICAL` the reducer checks:
  1. execution_kind agreement (`Trade` vs `RfqTrade`).
  2. tx_hash agreement (against pre-persisted local hash).
  3. fill_quantity_1e8 agreement.
  4. onchain_buyer_order_id / onchain_seller_order_id agreement (if
     pre-populated by Part D fingerprint pre-population).
  Any mismatch → `CONFLICT` with terminal_reason describing the
  disagreement.

- **A6** — Reorg / conflict / restart. `reorg_orphan_canonical_correlation`
  transitions `CORRELATED_CANONICAL → ORPHANED`. Because the sparse
  UNIQUE on `canonical_execution_id` is scoped to ACTIVE states only,
  a replacement AWAITING can be inserted for the same
  `canonical_execution_id` after orphaning, allowing a successor
  branch to freshly correlate. Restart converges via PG state —
  reducer idempotent on replay (`AlreadyCorrelated` outcome).

## Migration

`0056_option_execution_correlations_submission_unknown.sql`:
- Widens `correlation_status` CHECK to include `SUBMISSION_UNKNOWN`.
- Widens sparse UNIQUE on `canonical_execution_id` to include
  `SUBMISSION_UNKNOWN` in the active set.
- Immutability trigger unchanged.

## State machine (updated)

```
AWAITING_CHAIN_EVIDENCE  (Part E: atomic with intent INSERT)
    │
    │  attach_local_tx_identity (A1: pre-send)
    ▼
SUBMISSION_UNKNOWN       (tx identity persisted; RPC outcome unknown)
    │
    │  attach_tx_hash (post-RPC-ack with hash agreement)
    ▼
SUBMITTED                (RPC ack observed; tx_hash confirmed)
    │
    │  correlate_canonical_option_event (A3-A5)
    ▼
CORRELATED_CANONICAL     (canonical event ingested + cross-checked)
    │
    │  reorg_orphan_canonical_correlation (A6)
    ▼
ORPHANED                 (may re-correlate on replacement branch)

Alternate terminals: CONFLICT | MANUAL_REVIEW
```

Additional edges (Part A1 crash-window robustness):
- `SUBMISSION_UNKNOWN → CORRELATED_CANONICAL` (canonical evidence
  arrives before RPC ack completes; tx_hash is durably bound).
- `AWAITING_CHAIN_EVIDENCE → CORRELATED_CANONICAL` (legacy path
  where local tx identity wasn't captured — reducer directly
  promotes with fingerprint).

## Tx identity durability (A1)

Ethereum EIP-1559 transaction hash is deterministic: `keccak256` of
the exact signed raw bytes (type-byte `0x02` + RLP body). The
new `derive_signed_transaction_hash` in `src/execution/transaction.rs`
computes it in Rust without I/O — no signer, no provider round-trip.

New broadcast lifecycle:

```
1. Sign raw EIP-1559 tx via existing signer (KMS or LocalDev).
2. Derive local tx_hash from raw bytes.
3. If correlation row exists: attach_local_tx_identity → SUBMISSION_UNKNOWN.
   Fail closed pre-broadcast if the correlation row exists but attach
   fails (e.g. conflicting hash from a prior signer attempt).
4. Call provider.send_raw_transaction(raw).
5. Verify provider-returned hash equals local hash. Disagreement → fail
   closed with the local hash as the authoritative identity.
6. attach_tx_hash → SUBMITTED.
7. Update intent status BroadcastSubmitted.
```

Crash-window guarantee: if the process dies anywhere between step 3
and step 7, the correlation row still carries the authoritative
tx_hash and the reducer can bind canonical evidence on restart via
`(tx_hash, log_index)` — no evidence is ever lost.

## Reducer API

`correlate_canonical_option_event(pool, CanonicalExecutionEventInput)`
returns `CorrelationReducerOutcome`:
- `Promoted(row)` — new promotion to CORRELATED_CANONICAL.
- `AlreadyCorrelated(row)` — idempotent replay (same tx_hash+log_index).
- `Conflict(row)` — economic consistency check failed; row escalated
  to CONFLICT with terminal_reason.
- `NoCorrelationForIntent` — legacy intent without pre-existing
  correlation row; event still persisted to
  `option_execution_events` but no correlation update.

## Tests

- `tests/options_hybrid_v2_prebroadcast_tx_identity_pg_integration.rs`
  — 7 tests (1 pure unit + 6 PG). Covers derivation, attach_local
  transitions, idempotency, immutability, submission_unknown →
  submitted, crash-window promotion, migration-0056 sparse UNIQUE
  behavior. **7 pass**.
- `tests/options_hybrid_v2_canonical_event_reducer_pg_integration.rs`
  — 10 tests. Covers promotion, submission_unknown promotion,
  no-correlation path, idempotent replay, kind mismatch conflict,
  tx_hash mismatch conflict, fill_quantity mismatch conflict, second
  event conflict, reorg orphan + replacement, restart preservation.
  **10 pass**.

Combined with the atomic-wiring milestone tests: **49 tests all
green** against real Postgres 16.

Loud-fail gate preserved: `OPTIONS_ATOMIC_WIRING_PG_URL` required,
`REAL_POSTGRES_CONNECTION_CONFIRMED` printed per test.

## Regression

Full workspace: **1 failure**, `hybrid_v2_rebuild_operations_properties
::reconciliation_drift_never_repairs_projection`. Reproduced on
pristine `d1a3f1f` — pre-existing, NOT introduced by this sprint.

Options-specific: 301/301 lib tests, 152/152 integration tests, all
correlation tests green.

## NOT delivered (Packages B–J)

- **B** Open-order reservations (`option_reservations` ledger,
  formulas, atomic order acceptance).
- **C** Match → PENDING_SETTLEMENT transition (matcher refactor).
- **D** Canonical settlement risk release + failure policy + reorg
  economic state.
- **E** API/history/admin surfaces for full lifecycle exposure.
- **F** Repository push-down optimizations.
- **G** Consolidated 80-case real-PG closure matrix.
- **H** 20 bounded properties covering full lifecycle.
- **I** Full-lifecycle security review.
- **J** CI gate updates.

Realistic estimate: 6-10 focused milestones on top of this sprint's
Package A closure.

## Parent milestone closure status

Package A completes the correlation post-chain half. **Do NOT close**
the following parent milestones on the strength of this sprint —
their outstanding requirements span Packages B-J:

- `OPTIONS_HYBRID_V2_CORRELATION_OPERATIONAL_CORE_V1_COMPLETE` —
  requires public/admin API exposure (E).
- `OPTIONS_HYBRID_V2_EXECUTION_CORRELATION_CLOSURE_V1_COMPLETE` —
  requires settlement release (D).
- `OPTIONS_HYBRID_V2_IDENTITY_AND_CORRELATION_WIRING_V1_COMPLETE` —
  ready to close pending Package E API surface work.
- `OPTIONS_HYBRID_V2_RISK_RESERVATION_AND_PENDING_SETTLEMENT_V1_COMPLETE`
  — requires ALL of Package B + C.
- `OPTIONS_HYBRID_V2_ECONOMIC_EXECUTION_CORE_V1_COMPLETE` — requires
  reservation semantics wired into the broadcast preflight.
- `OPTIONS_HYBRID_V2_PRODUCT_INTEGRATION_V1_COMPLETE` — requires
  full lifecycle end-to-end.

## Files touched

- `src/execution/transaction.rs` — `derive_signed_transaction_hash` +
  hex decode helper.
- `src/execution/mod.rs` — re-export.
- `src/options/correlation_repository.rs` — SubmissionUnknown variant,
  `attach_local_tx_identity`, `correlate_canonical_option_event`,
  `reorg_orphan_canonical_correlation`, reducer outcome/input types.
  Updated `attach_tx_hash` and `mark_correlated_canonical` to accept
  SUBMISSION_UNKNOWN as source state. Updated `mark_correlated_canonical`
  to NULLIF empty-string bindings to preserve NULL under immutability
  trigger.
- `src/options/service.rs` — pre-broadcast local tx identity
  persistence, provider hash agreement check, updated attach_tx_hash
  invocation. MockBroadcastProvider updated to derive hash from raw
  bytes matching production semantics.
- `src/options/event_indexer.rs` — RFQ event signature + topic0 +
  decoder + topic list wiring.
- `migrations/0056_option_execution_correlations_submission_unknown.sql`
  — CHECK + sparse UNIQUE widening.
- `tests/options_hybrid_v2_prebroadcast_tx_identity_pg_integration.rs`
  — 7 tests.
- `tests/options_hybrid_v2_canonical_event_reducer_pg_integration.rs`
  — 10 tests.

## Safety statements

- No real public-chain transaction sent.
- `eth_sendRawTransaction` real-chain calls: 0.
- No new backend private key custody.
- Base mainnet 8453 never contacted.
- Frontend + Solidity untouched.
- No API surface exposed raw signatures or raw signed tx bytes.

## Next step

`OPTIONS-HYBRID-V2-RESERVATIONS-AND-PENDING-SETTLEMENT-V1` —
covering Packages B + C (reservation ledger + match risk transition).
The correlation subsystem is now operational end-to-end for the
matcher-derived intent path; reservations are the last missing
building block before full lifecycle can be validated E2E.
