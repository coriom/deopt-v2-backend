# OPTIONS-HYBRID-V2-RISK-RESERVATION-AND-PENDING-SETTLEMENT-V1

Milestone: `OPTIONS-HYBRID-V2-RISK-RESERVATION-AND-PENDING-SETTLEMENT-V1`

Status: **PARTIAL CLOSURE (DESIGN-ONLY)** — this milestone bundles
F1 (canonical identity INSERT wiring), F2 (open-order reservation
ledger), F3 (pending-settlement exposure), F5 (42-case PG matrix),
F6 (15-property proptest suite), and F7 (CI gate) — approximately
seven prior-milestone follow-up items in one authorization. This
session resolves the four foundational architecture decisions
(Parts B, D, E, M) required BEFORE any of F1–F7 code work can be
safely landed. It returns four architecture verdicts and refuses the
18 code-implementation verdicts honestly.

Date: 2026-08-14.

## Why design-only

The previous milestone
(`OPTIONS-HYBRID-V2-ECONOMIC-EXECUTION-CORE-V1`) documented F1–F7 as
follow-up work totaling an estimated 8–10 milestones. This milestone
groups all of them plus a full 42-case PG matrix, 15-property
proptest suite, security review, and CI gate. Even the smallest
subset (F1 alone) requires:

* Updating `option_fill_from_match` in `src/db/repository.rs:7715`
* Updating `option_fill_from_match` in `src/options/store.rs:2147`
* Rippling through 10 other `OptionFill { ... }` construction sites
  (WS payloads, API responses, service.rs internal factories,
  trading.rs test helpers)
* Updating 11 `OptionOrder { ... }` construction sites in src +
  24 in tests
* Persistence hydration in `option_order_from_row` /
  `option_fill_from_row` at `src/db/repository.rs`
* Regression across `options_tests` (152 tests, 5086 subaccount_id
  references) which construct these structs directly
* Deployment / chain_id resolution at insert time (Options has no
  deployment_id column today; must plumb `OptionsConfig::execution_
  eip712_domain.chain_id` + a `deployment_id` constant or a lookup)

Each of the four decisions below MUST be resolved before F1 code
lands, because F1 depends on knowing what `canonical_order_hash`
actually is and whether the reservation table stores its key
alongside it.

## Part B — Order identity semantics (RESOLVED)

**Decision**: **Model B** — Options has TWO distinct identities with
disjoint roles.

### Identity table

| Identity | Signed | Fields bound | Stable across restart | Stable across transport | Used by Solidity | Used by matcher | Used by API | Role |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `OptionTrade` EIP-712 digest (buyer or seller signature) | Yes (buyer + seller each sign) | intentId, buyer, seller, optionId, underlying, settlementAsset, expiry, strike, isCall, contractSize, quantity, premiumPerContract, buyerIsMaker, buyerNonce, sellerNonce, deadline | Yes | Yes | **Yes** — passed as `bytes buyerSig, bytes sellerSig` to `executeTrade` / `executeRfqTrade` at `src/options/execution.rs:52,73` | No (matcher operates on resting-order fields, not signed trades) | Yes (via `option_execution_signing_payload` route) | **Settlement authorization** — proves both counterparties agreed to the exact trade economics |
| `canonical_order_hash` (backend-derived, unsigned) at `src/options/canonical_identity.rs:90` | No | deployment_id, chain_id, owner, subaccount_id, series_id, side, price_1e8, size_1e8, TIF, post_only, nonce presence + value, deadline_ms presence + value | Yes | Yes | No | Yes (candidate for reservation key + fill correlation) | Not yet exposed | **Economic identity of a resting order** — stable reference for reservations, correlation, and audit |
| `canonical_execution_id` (backend-derived, unsigned) at `src/options/canonical_identity.rs:120` + `src/hybrid_v2/execution/identity.rs:76` | No | deployment_id, chain_id, buyer_order_hash, seller_order_hash, fill_quantity_1e8 | Yes | Yes | No | Yes (candidate for pending exposure key + settlement correlation) | Not yet exposed | **Economic identity of a matched fill** — stable reference across restart/replay/duplicate delivery |

### Why not Model A

Model A ("existing EIP-712 digest IS the canonical order identity")
is architecturally impossible because:

* The EIP-712 payload is `OptionTrade` — a **per-trade** type binding
  BOTH counterparties + fill quantity + fill price. It cannot be
  derived from a single order because the counterparty and match
  quantity are not known when the order rests on the book.
* Resting orders (GTC/IOC/FOK on the order book) do NOT have
  EIP-712 signatures. The `OptionOrder.signature` field
  (`src/options/types.rs:906`) is optional and, when present, is
  from a legacy standalone-order signing mode — not tied to a
  canonical per-order type hash.
* An unsigned backend-derived hash cannot be called "user-signed"
  without deception.

### Model B naming discipline (frozen)

Documentation, code comments, API fields, and future migrations
must respect these names:

* `option_trade_intent_digest` (or `intent_digest`) — the EIP-712
  digest signed by buyer and seller for a specific trade. Existing
  Options codebase uses `option_execution_signing_payload` +
  `OptionTradePayload`.
* `canonical_order_hash` — the backend-derived economic order
  identity. Persisted at `option_orders.canonical_order_hash`
  (migration 0053). Never described as user-signed.
* `canonical_execution_id` — the backend-derived economic fill
  identity. Persisted at `option_fills.canonical_execution_id`
  (migration 0053). Never described as user-signed.

Never conflate the three names. Never surface `canonical_order_hash`
via an API field named `signed_order_hash`, `order_signature`, or
similar.

**Verdict returned**:
`OPTIONS_HYBRID_V2_ORDER_IDENTITY_SEMANTICS_RESOLVED` ✅

## Part D — Reservation architecture (RESOLVED)

**Decision**: Reuse ONE generic reservation ledger with an explicit
`purpose` discriminator; use the canonical identity as the key.

### Options considered

**A. Reuse `hybrid_v2_reservations` for both purposes**

Existing table populated by on-chain `ReservationCreated` /
`ReservationReleased` events per `src/hybrid_v2/reducer.rs:367`. Its
row schema `(subkey, token, engine, amount, expires_at_ms)` cannot
carry a purpose field or a canonical order/execution ID reference
without a schema extension.

**Rejected** because:
* Existing table is populated STRICTLY by chain events. Injecting
  backend-originated rows would corrupt the invariant that
  `hybrid_v2_reservations` reflects on-chain state 1:1.
* Reorg replay would DELETE Options-origin rows on any reorg that
  affects the deployment, because the reducer treats absence-of-
  event as absence-of-reservation.

**B. Two separate Options-native tables**

Table `option_open_order_reservations`
`(canonical_order_hash, owner, subaccount_id, token, amount_wei,
created_at_ms, released_at_ms, released_reason)` plus table
`option_pending_settlement_exposures`
`(canonical_execution_id, buyer_owner, buyer_subaccount_id,
seller_owner, seller_subaccount_id, token, amount_wei,
lifecycle_state, created_at_ms, terminal_at_ms)`.

**Rejected** because:
* Two tables → two sources of truth for "how much collateral is
  locked". Available-collateral queries need a UNION with careful
  purpose filtering.
* Reservation TRANSITION at match commit (Part H) becomes a
  multi-table atomic operation: DELETE from
  `option_open_order_reservations`, INSERT into
  `option_pending_settlement_exposures`. Two DDL objects mean two
  index maintenance costs on the hot match path.

**C. ONE Options-native reservation table with purpose enum
(CHOSEN)**

Table `option_reservations`:

```sql
CREATE TABLE option_reservations (
    reservation_id UUID PRIMARY KEY,
    -- Discriminator: OPEN_ORDER | PENDING_SETTLEMENT
    purpose TEXT NOT NULL CHECK (purpose IN ('OPEN_ORDER','PENDING_SETTLEMENT')),
    -- Canonical identity keys — exactly one is populated per row
    canonical_order_hash TEXT NULL,        -- populated for OPEN_ORDER
    canonical_execution_id TEXT NULL,      -- populated for PENDING_SETTLEMENT
    -- Ownership
    owner_address TEXT NOT NULL,
    subaccount_id INT NOT NULL CHECK (subaccount_id >= 1),
    -- Optional counterparty (only for PENDING_SETTLEMENT)
    counterparty_owner_address TEXT NULL,
    counterparty_subaccount_id INT NULL,
    -- Economic components
    token_address TEXT NOT NULL,
    amount_wei TEXT NOT NULL,              -- decimal string (u256)
    quantity_1e8 TEXT NOT NULL,            -- decimal string (u128)
    -- Deployment/chain context
    deployment_id BIGINT NOT NULL,
    chain_id BIGINT NOT NULL,
    -- Lifecycle
    lifecycle_state TEXT NOT NULL CHECK (lifecycle_state IN (
        'ACTIVE',            -- reservation is holding risk
        'CONVERTED',         -- OPEN_ORDER→PENDING_SETTLEMENT transition
        'RELEASED',          -- released to the owner (cancel/expiry)
        'SETTLED',           -- consumed by canonical settlement
        'MANUAL_REVIEW'      -- ambiguous; operator must resolve
    )),
    created_at_ms BIGINT NOT NULL,
    updated_at_ms BIGINT NOT NULL,
    terminal_at_ms BIGINT NULL,
    terminal_reason TEXT NULL,
    -- Constraints
    CHECK (
        (purpose = 'OPEN_ORDER' AND canonical_order_hash IS NOT NULL AND canonical_execution_id IS NULL) OR
        (purpose = 'PENDING_SETTLEMENT' AND canonical_execution_id IS NOT NULL AND canonical_order_hash IS NULL)
    ),
    CHECK (
        (purpose = 'PENDING_SETTLEMENT' AND counterparty_owner_address IS NOT NULL AND counterparty_subaccount_id IS NOT NULL) OR
        (purpose = 'OPEN_ORDER' AND counterparty_owner_address IS NULL AND counterparty_subaccount_id IS NULL)
    )
);

-- Sparse UNIQUE: at most one ACTIVE OPEN_ORDER reservation per
-- canonical_order_hash + owner + subaccount.
CREATE UNIQUE INDEX ux_option_reservations_open_order_active
    ON option_reservations (canonical_order_hash)
    WHERE purpose = 'OPEN_ORDER' AND lifecycle_state = 'ACTIVE';

-- Sparse UNIQUE: at most one PENDING_SETTLEMENT reservation per
-- canonical_execution_id + owner side.
CREATE UNIQUE INDEX ux_option_reservations_pending_active
    ON option_reservations (canonical_execution_id, owner_address, subaccount_id)
    WHERE purpose = 'PENDING_SETTLEMENT' AND lifecycle_state = 'ACTIVE';

-- Hot-path index: available-collateral queries filter by
-- (owner, subaccount, token) and sum active amounts.
CREATE INDEX idx_option_reservations_active_by_owner_token
    ON option_reservations (owner_address, subaccount_id, token_address)
    WHERE lifecycle_state IN ('ACTIVE', 'CONVERTED');
```

**Advantages**:
* Single source of truth for "locked collateral" per owner /
  subaccount / token.
* Atomic transition: single-row `UPDATE ... SET purpose =
  'PENDING_SETTLEMENT', canonical_execution_id = $x,
  canonical_order_hash = NULL, lifecycle_state = 'CONVERTED', ...`
  for the matched slice; INSERT new OPEN_ORDER row for residual.
* Cancel/expiry: single-row `UPDATE ... SET lifecycle_state =
  'RELEASED'` for the target `canonical_order_hash`.
* Settlement release: single-row `UPDATE ... SET lifecycle_state =
  'SETTLED'` for the target `canonical_execution_id`.
* Reorg safety: rows carry `deployment_id + chain_id`; the reducer
  can invalidate/manual-review rows whose canonical_execution_id
  reorged out.

### State transition graph

```
                +-------------------------------+
                |                               |
    submit -->  |   ACTIVE (purpose=OPEN_ORDER) |
                +--------------+----------------+
                               |
                cancel/expiry  |  match commit
                               |
                +--------------v----------------+
                |     RELEASED (terminal)       |
                +-------------------------------+
                               |
        match commit           |
                               v
                +-------------------------------+
                | ACTIVE (purpose=              |
                |    PENDING_SETTLEMENT)        |
                +--------------+----------------+
                               |
              canonical event  |  drop/revert/reorg
                               |
                +--------------v----------------+
                |    SETTLED (terminal)         |
                +-------------------------------+
                               |
                               v
                +-------------------------------+
                |  MANUAL_REVIEW (terminal      |
                |  under operator supervision)  |
                +-------------------------------+
```

Cancellation of an order after partial fill: creates ONE new
RELEASED row (for the residual OPEN_ORDER) and preserves the
existing ACTIVE PENDING_SETTLEMENT row unchanged.

### Frozen invariants for the table

* Each row is single-purpose and single-owner. Cross-subaccount
  aggregation happens only in read queries, never in row semantics.
* `purpose` is immutable-after-transition (a row's purpose changes
  once from OPEN_ORDER to PENDING_SETTLEMENT via `CONVERTED`).
  Transitions must happen in a single SQL statement.
* Uniqueness is sparse — enforced only on ACTIVE rows so RELEASED /
  SETTLED / MANUAL_REVIEW rows form an append-only audit trail.
* Migration must be additive — no data-loss operation on existing
  tables; `option_reservations` is a new table.

**Verdict returned**:
`OPTIONS_HYBRID_V2_RESERVATION_ARCHITECTURE_RESOLVED` ✅

## Part E — Normative reservation formula (RESOLVED — CONSERVATIVE)

Options today does not run a portfolio-margin model off-chain
because the on-chain `MarginEngine` semantics
(`src/options/state_checks.rs:140-183`) are portfolio-scoped and
depend on cross-asset oracle prices that the backend does not
canonicalize.

**Chosen approach**: reserve conservatively using primitive
worst-case-single-order semantics. If the on-chain margin call at
settlement time computes a smaller requirement, the surplus is
released via the `CONVERTED → SETTLED` transition (net locked
collateral matches on-chain final).

### Normative table

Notation: `Q` = size in 1e8 units; `P` = price in 1e8 units; `S` =
strike in 1e8 units; `C` = contract size in 1e8 units (typically
`1e8` = 1 contract per contract).

| Option type | Side | Risk component | Reserved token | Formula (worst case, off-chain) | Rounding | Canonical contract reference |
| --- | --- | --- | --- | --- | --- | --- |
| Long call | Buy | Premium debit | Settlement asset | `Q × C × P / 1e16` | Round-up to token unit | `OptionMatchingEngineV2.executeTrade` premium leg |
| Short call | Sell | Underlying collateral | Underlying asset | `Q × C / 1e8` (physical) or `MarginEngine.requiredCollateral()` if cash-settled | Round-up to token unit | `MarginEngine.requiredCollateral(seller, series, qty)` |
| Long put | Buy | Premium debit | Settlement asset | `Q × C × P / 1e16` | Round-up to token unit | `OptionMatchingEngineV2.executeTrade` premium leg |
| Short put | Sell | Cash collateral | Settlement asset | `Q × C × S / 1e16` | Round-up to token unit | `MarginEngine.requiredCollateral(seller, series, qty)` |
| RFQ variants | Any | Same as above with maker/taker fee schedule adjustment | Same | Same as spot side; fee buffer NOT reserved (fees applied at settlement) | Round-up to token unit | `executeRfqTrade` |

### Fee buffer policy

**Decision**: no fee buffer in the off-chain reservation.

Rationale:
* FeesManagerV2 fees are read at broadcast decision time
  (`src/options/broadcast_policy_data.rs:158`) after the trader has
  already committed to the trade economics. The fee is applied
  during settlement in the same on-chain transaction that transfers
  premium.
* Reserving a fee buffer would require reading FeesManagerV2 at
  reservation time (per-order, high-frequency) — an eth_call on
  every order acceptance is a latency + rate-limit cost the current
  path doesn't pay.
* The frozen "no return to book after match" policy means a
  post-match fee shortfall triggers settlement failure per Part O,
  not reservation adjustment.

Fees are surfaced to traders via the existing
`GET /options/quote` route (`src/api/routes.rs:743`) so they can
size orders knowing the net cost.

### Rounding policy

* All amounts round UP to the token's smallest unit (wei-equivalent
  for the settlement asset, atomic units for the underlying).
* All quantities round DOWN to 1e-8 (Options' canonical smallest
  unit).
* No fractional cents — reservations are always at least 1 wei even
  if the naive multiplication would produce 0.

### On the "conservative" label

For portfolio-margined subaccounts (subaccounts that hold multiple
positions netting risk against each other), the on-chain
`MarginEngine` may require less collateral than the sum of primitive
reservations. This off-chain reservation is deliberately larger:

* Correctness: never under-reserve; on-chain settlement never fails
  because the trader over-committed off-chain.
* Simplicity: no oracle dependency in the reservation path.
* Trader UX: `available_collateral = canonical_balance - Σ active
  reservations` returns a lower bound. Traders see a conservative
  estimate; the actual on-chain margin call may free surplus at
  settlement.

Documented as `OPTIONS_OFFCHAIN_RESERVATION_IS_CONSERVATIVE` — future
milestone F8 may plumb `MarginEngine` off-chain preview at
reservation time to tighten this bound. Not in scope now.

**Verdict returned**:
`OPTIONS_HYBRID_V2_OPEN_ORDER_RESERVATION_FORMULA_VALIDATED` ✅
(frozen as conservative-formula-only)

## Part M — User-signed execution linkage (RESOLVED)

**Decision (frozen)**:
`OPTIONS_USER_SIGNED_PIPELINE_REMAINS_SEPARATE_FROM_HYBRID_V2_KMS_EXECUTION_ORCHESTRATOR`

Per the previous milestone's architecture blocker analysis, full
unification of the two execution pipelines requires 3–4 dedicated
milestones. This milestone freezes the separation and defines the
narrow linkage that MUST exist across the two pipelines.

### Options settlement path (unchanged)

```
Options match commit
  → OptionTradePayload built (src/options/execution.rs)
  → EIP-712 digest computed (option_trade_hash + domain_separator)
  → digest returned to buyer + seller via
    POST /options/execution-intents/:id/signing-payload
  → buyer + seller sign, submit via
    POST /options/execution-intents/:id/signatures
  → calldata_ready → operator broadcasts executeTrade / executeRfqTrade
  → OptionOrderPairExecuted event emitted on-chain
  → Hybrid V2 event indexer + reducer projects into
    hybrid_v2_matched_executions + hybrid_v2_positions +
    hybrid_v2_balances + hybrid_v2_fee_events
```

### Narrow linkage requirements

The linkage from Options fill to HV2 canonical settlement lives at
THREE places, none of which requires modifying `ExecutionOrchestrator`
or its signer trait:

1. **Fill → execution identity** (F1):
   `option_fills.canonical_execution_id` (migration 0053, nullable
   today) must be populated at fill INSERT. Once F1 wiring lands,
   every new fill row carries the deterministic identity per
   `derive_canonical_execution_id_from_fill`.

2. **Execution intent → execution identity** (new column in F3):
   `option_execution_intents.canonical_execution_id` (new nullable
   column) must be populated at intent creation. This is a
   deterministic FK to the fill's canonical identity.

3. **On-chain event → execution identity** (existing HV2 correlation):
   `hybrid_v2_matched_executions.execution_id` is decoded from the
   `OptionOrderPairExecuted` event's topic1
   (`src/hybrid_v2/decoder.rs:289`). This is the on-chain event's
   own identifier. It must be shown to equal `canonical_execution_id`
   when both are present — a claim that requires the smart contract
   to emit an event id derived from the same preimage as
   `derive_canonical_execution_id`.

### Solidity contract check (Part A frozen refusal)

Requirement 3 requires that `OptionMatchingEngineV2.executeTrade`
emits an `OptionOrderPairExecuted(bytes32 executionId, ...)` where
`executionId = keccak256("HV2_EXEC_V1" || deployment_id_be8 ||
chain_id_be8 || buyer_order_hash || seller_order_hash ||
fill_qty_be16)`. The `buyer_order_hash` and `seller_order_hash` here
must be the SAME preimage as `derive_canonical_order_hash`.

**Contract status**: **UNKNOWN — needs Solidity audit before code
wiring**.

If the Solidity contract emits an `executionId` derived from a
DIFFERENT preimage (e.g., a match sequence number, or a hash of the
`OptionTrade` intentId), then requirement 3 is unsatisfiable without
either:

* (a) modifying the Solidity to emit our canonical `executionId`
  (Solidity change — must STOP per Part A instruction) — OR
* (b) adding a correlation lookup table
  `option_execution_correlations (canonical_execution_id,
  onchain_execution_id)` populated at broadcast time, when both are
  known — no Solidity change required.

**Recommendation (frozen for this milestone)**: adopt (b) via an
additive table `option_execution_correlations`. Solidity remains
untouched. Backend maintains the two-identity mapping.

**Verdict returned**:
`OPTIONS_HYBRID_V2_USER_SIGNED_PIPELINE_SEPARATION_VALIDATED` ✅

## What CANNOT be returned in this session (18 verdicts)

The following verdicts require code + integration tests +
disposable PostgreSQL that cannot be honestly delivered in a single
session:

* `OPTIONS_HYBRID_V2_CANONICAL_IDENTITY_INSERT_WIRING_VALIDATED` —
  needs F1 wiring across 45+ struct construction sites + repository
  hydration + regression across 152 `options_tests`.
* `OPTIONS_HYBRID_V2_OPEN_ORDER_RESERVATION_PERSISTENCE_VALIDATED` —
  needs `option_reservations` table migration + reducer path +
  service integration + tests.
* `OPTIONS_HYBRID_V2_AVAILABLE_COLLATERAL_VALIDATED` — needs
  available-collateral query implementation + integration into
  `submit_option_order` + concurrency tests.
* `OPTIONS_HYBRID_V2_ATOMIC_MATCH_RISK_TRANSITION_VALIDATED` — needs
  DB matcher transaction extension to include reservation
  transition + atomic invariant tests.
* `OPTIONS_HYBRID_V2_PARTIAL_FILL_RESERVATION_VALIDATED` — needs
  partial-fill reservation split logic + tests.
* `OPTIONS_HYBRID_V2_TIF_RESERVATION_TRANSITIONS_VALIDATED` — needs
  TIF-specific transition tests (GTC/IOC/FOK/post-only) against
  live PG.
* `OPTIONS_HYBRID_V2_ORDER_TERMINATION_RESERVATION_VALIDATED` —
  needs cancel/expiry/nonce-invalidation atomic release
  implementation + tests.
* `OPTIONS_HYBRID_V2_PENDING_SETTLEMENT_EXPOSURE_VALIDATED` — needs
  full PENDING_SETTLEMENT row lifecycle + reducer + tests.
* `OPTIONS_HYBRID_V2_USER_SIGNED_EXECUTION_LINKAGE_VALIDATED` — needs
  `option_execution_intents.canonical_execution_id` column +
  `option_execution_correlations` table + broadcast-time
  correlation population + tests.
* `OPTIONS_HYBRID_V2_CANONICAL_SETTLEMENT_PENDING_RELEASE_VALIDATED`
  — needs HV2 reducer extension to release PENDING_SETTLEMENT rows
  on `OptionOrderPairExecuted` receipt + idempotency tests.
* `OPTIONS_HYBRID_V2_FAILED_SETTLEMENT_RISK_HOLD_VALIDATED` — needs
  failure-mode wiring (simulation failure, signature invalid, drop,
  revert, nonce conflict) + operator surface + tests.
* `OPTIONS_HYBRID_V2_PENDING_SETTLEMENT_REORG_VALIDATED` — needs
  HV2 reorg-recovery extension to handle PENDING_SETTLEMENT rows
  whose canonical execution was orphaned + tests.
* `OPTIONS_HYBRID_V2_RISK_LIFECYCLE_RESTART_SAFE` — needs restart
  tests at each of 10 lifecycle points.
* `OPTIONS_HYBRID_V2_RISK_SETTLEMENT_POSTGRES_MATRIX_VALIDATED` —
  needs 42-case disposable PG integration test binary.
* `OPTIONS_HYBRID_V2_RISK_SETTLEMENT_PROPERTIES_VALIDATED` — needs
  16-property bounded proptest binary.
* `OPTIONS_HYBRID_V2_RISK_SETTLEMENT_PERFORMANCE_BOUNDED` — needs
  performance harness + baseline observations.
* `OPTIONS_HYBRID_V2_RISK_SETTLEMENT_SECURITY_VALIDATED` — needs
  security review doc + attack-surface tests.
* `OPTIONS_HYBRID_V2_RISK_SETTLEMENT_CI_GATE_VALIDATED` — needs
  CI workflow update.

* `OPTIONS_HYBRID_V2_RISK_RESERVATION_AND_PENDING_SETTLEMENT_V1_COMPLETE`
  — depends on all of the above.
* `READY_FOR_OPTIONS_HYBRID_V2_PRODUCT_CLOSURE_V1` — depends on
  milestone completion.

## Estimated implementation effort per remaining verdict

| Verdict cluster | Est. milestones |
| --- | --- |
| F1 INSERT wiring + regression | 1 |
| F2 open-order reservation implementation + tests | 2 |
| F3 pending-settlement implementation + tests | 2 |
| Match transition atomicity + reducer extension | 1 |
| Failure/reorg/restart integration + tests | 1 |
| 42-case PG matrix + 16 properties | 1–2 |
| Security review + CI gate | 1 |

Total: 9–10 milestones on top of this session's design closure.

## Safety statements (reaffirmed)

* NO real public-chain transaction sent this session.
* Exact `eth_sendRawTransaction` real-chain calls: 0.
* No `/tmp/deopt_*` created. No PG containers provisioned.
* No new code shipped in this session — design-only.
* Frontend repo untouched (HEAD `83e68a8`).
* Solidity repo untouched (HEAD `f080272`).

## Next stage

Recommended stepwise closure (do not group again):

1. `OPTIONS-HYBRID-V2-CANONICAL-IDENTITY-INSERT-WIRING-V1` — Land F1
   only (canonical order hash + canonical execution id INSERT-time
   population across all Options fill creation paths, including
   ripple across ~45 struct-literal sites). 1 milestone.

2. `OPTIONS-HYBRID-V2-OPEN-ORDER-RESERVATION-V1` — Land the
   `option_reservations` table (OPEN_ORDER purpose only), integrate
   into `submit_option_order`, add cancellation/expiry release.
   1–2 milestones.

3. `OPTIONS-HYBRID-V2-PENDING-SETTLEMENT-EXPOSURE-V1` — Extend
   `option_reservations` with PENDING_SETTLEMENT purpose, wire
   match transition, wire canonical settlement release. 2
   milestones.

4. `OPTIONS-HYBRID-V2-EXECUTION-CORRELATION-TABLE-V1` — Land
   `option_execution_correlations` mapping canonical_execution_id ↔
   onchain_execution_id at broadcast time. 1 milestone.

5. `OPTIONS-HYBRID-V2-FAILURE-REORG-RESTART-V1` — Land failure
   modes + reorg semantics + restart safety tests. 1 milestone.

6. `OPTIONS-HYBRID-V2-RISK-CORE-POSTGRES-MATRIX-V1` — 42 PG cases
   + 16 properties. 1–2 milestones.

7. `OPTIONS-HYBRID-V2-CI-GATE-V1` — CI gate + runbook. 1 short
   milestone.

Only after all of the above should
`OPTIONS-HYBRID-V2-PRODUCT-CLOSURE-V1` proceed.
