# OPTIONS-HYBRID-V2-ECONOMIC-EXECUTION-CORE-V1

Milestone: `OPTIONS-HYBRID-V2-ECONOMIC-EXECUTION-CORE-V1`

Status: **PARTIAL CLOSURE** — canonical identity foundation shipped;
economic-lifecycle documentation + settlement truth table shipped;
Part M (D4 orchestrator wiring) is a genuine architecture blocker
returned as a scoped follow-up.

Date: 2026-08-14.

## Purpose

Complete the canonical economic bridge between the existing Options
matching engine and the existing Hybrid V2 on-chain execution
infrastructure. This milestone grouped:

* D1 — canonical Options execution identity
* D2 — Options ↔ Hybrid V2 reservation binding
* D3 — pending settlement exposure semantics
* D4 — Options → Hybrid V2 `ExecutionOrchestrator` wiring
* Settlement truth table + failure policy

## Architecture blocker (Part M / D4)

**Verdict**: `OPTIONS_HYBRID_V2_EXECUTION_ORCHESTRATOR_INTEGRATED`
**cannot be returned in this session** because full unification of
the two execution pipelines is a genuine architecture blocker.

### Blocker summary

| Aspect | Options today | Hybrid V2 ExecutionOrchestrator |
| --- | --- | --- |
| Signature source | User-supplied (buyer + seller EIP-712) | Backend-generated (executor EIP-1559 via KMS/mTLS) |
| Signature count | 2 (buyer + seller) | 1 (executor) |
| Digest structure | EIP-712 over `OptionTrade` struct | EIP-1559 preimage over full raw tx |
| Signer trait | None (signatures passed through) | `ExecutionSigner::sign_execution(SigningRequest)` — MUST produce a fresh signature |
| Calldata derivation | Pre-built + pre-signed by users | Backend-derived by `ExecutionPlanBuilder`, then signed |
| Target selector | `executeTrade`, `executeRfqTrade` | `executeMatch` (only allowlisted selector on `OptionMatchingEngineV2`) |
| Broadcast responsibility | Backend submits pre-signed tx | Backend submits self-signed tx via `broadcast_outbox` |

**Concrete blocker evidence**:

* `src/hybrid_v2/execution/signer.rs:177-189` — `ExecutionSigner`
  trait has one async method (`sign_execution`) that returns a
  freshly-generated `SignedTx`. No "pre-signed pass-through" variant
  exists. The trait cannot accept an externally-signed payload.
* `src/hybrid_v2/execution/plan.rs` — `ExecutionPlanBuilder::
  build_from_request` derives calldata deterministically from
  manifest + runtime state, then the signer is asked to sign it.
  Options' pre-signed calldata cannot be re-derived because the
  buyer/seller signatures are inputs, not outputs.
* `src/hybrid_v2/execution/target_policy.rs:83-108` — only
  `executeMatch::SELECTOR` is enrolled on `OptionMatchingEngineV2`.
  Options uses `executeTrade` and `executeRfqTrade`, different ABIs
  requiring a separate target-policy audit + selector allowlisting.
* Options' EIP-712 digest structure at `src/options/execution.rs:
  213-228` is `keccak256("\x19\x01" || domain_separator ||
  option_trade_hash)`; HV2's digest at `src/hybrid_v2/execution/
  signer.rs:89-93` is `keccak256(EIP1559_preimage)`. Different
  preimage shape.

### Two paths forward

**Option A — separate pipelines (recommended)**: Keep Options
user-signed, keep HV2 backend-signed. Bind the two at the identity
layer (canonical execution ID, this milestone) and at the settlement
layer (Hybrid V2 reducer consumes `OptionOrderPairExecuted` events
regardless of who submitted the transaction). No orchestrator
wiring. Frontier visibility comes from Hybrid V2's canonical read
store, not from orchestrator UI.

Estimated cost: 0 additional milestones. Requires the D1
integration wiring (populate `canonical_order_hash` + `canonical_
execution_id` at insert time in the fill path — see Follow-up F1
below).

**Option B — full unification (expensive)**: Redesign
`ExecutionSigner` to accept externally-signed payloads, generalize
`ExecutionPlanBuilder` to accept pre-signed calldata as an input,
extend `target_policy.rs` to enumerate Options' selectors, ensure
firewall recovery logic accepts EIP-712 preimages alongside EIP-1559.
Estimated cost: 3–4 dedicated milestones, requires product decision
on whether Options should move from user-signed to backend-signed
(major posture change).

**Recommendation**: proceed with Option A. Options remains
user-signed; Hybrid V2's read/history stack correlates via the
canonical execution ID landed in this milestone.

## What ships in this milestone

### Package A — canonical identity foundation (SHIPPED)

* `src/options/canonical_identity.rs` (new module):
  * `derive_canonical_order_hash(inputs)` — deterministic keccak256
    over deployment + chain + owner + subaccount + series + side +
    price + size + TIF + post_only + nonce presence/value + deadline
    presence/value.
  * `derive_canonical_execution_id_from_fill(...)` wraps the
    existing HV2 identity derivation.
  * 11 unit tests, all passing.
* `migrations/0053_options_canonical_execution_identity.sql`:
  * Additive nullable `canonical_order_hash` on `option_orders`.
  * Additive nullable `canonical_execution_id` on `option_fills`.
  * Sparse uniqueness indexes (only non-NULL values).
  * Immutability triggers matching HV2 posture.

### Package B — economic lifecycle documentation (SHIPPED)

See sections below:

* Economic lifecycle map
* Canonical position policy
* Settlement truth table
* Failure policy

### Package C — orchestrator wiring (BLOCKED — see Part M above)

### Package D — PG matrix + properties + CI (PARTIAL)

* No new PG integration test binary in this session (D6-scale work
  requires provisioning + writing 42+ cases).
* Existing regression: all 152 `options_tests`, 21 `subaccounts_
  options_orders_history_tests`, 9 `subaccounts_options_ws_payload_
  tests`, and the 8-test `options_hybrid_v2_subaccount_matcher_
  integration` binary from the previous milestone continue to pass.

## Economic lifecycle map (Part B)

The Options economic lifecycle, unambiguous, source-of-truth per
state:

| State | Source of truth | Risk locked | Book qty | Pending exec qty | Canonical position effect | Cancel allowed | Failure transition | Canonical event required |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| Order accepted | `option_orders` row + `submit_option_order` outcome | Off-chain implicit (see F2 below) | 0 | 0 | none | yes | rejected → not persisted | none |
| Resting order | `option_orders` row | Off-chain implicit | remaining_size_1e8 | 0 | none | yes | expired/invalidated → no economic mutation | none |
| Partial match | `option_fills` insert + `option_orders` UPDATE | Off-chain implicit | remaining after fill | fill qty | none until settlement | resting residual only | see failure policy below | `OptionOrderPairExecuted` (post-settlement) |
| Full match | atomic same as partial | Off-chain implicit | 0 | fill qty | none until settlement | not applicable | see failure policy below | `OptionOrderPairExecuted` (post-settlement) |
| Pending execution | Options `option_execution_intents` row (existing table) | Off-chain implicit | 0 | fill qty | none | no (backend cannot unilaterally abort user-signed intent) | see failure policy below | `OptionOrderPairExecuted` on success; none on failure |
| Simulation failure | `option_execution_intents.status = failed_simulation` | Off-chain implicit | 0 | 0 (marked failed) | none | not applicable | intent terminal; user may re-submit new intent | none |
| Signing failure | not applicable (Options is user-signed; no backend signing step) | — | — | — | — | — | — | — |
| Prepared execution | `option_execution_intents.status = calldata_ready` | Off-chain implicit | 0 | fill qty | none | no | broadcast decision comes next | none |
| Broadcast pending | `option_execution_transactions` row + `option_execution_intents.status = broadcasted` | Off-chain implicit | 0 | fill qty | none | no | see failure policy below | `OptionOrderPairExecuted` on success |
| Reverted execution | tx receipt reverted | Off-chain implicit | 0 | fill qty until manual intervention | none | no | manual intervention required per failure policy | none (no OptionOrderPairExecuted) |
| Dropped/ambiguous execution | tx never mined | Off-chain implicit | 0 | fill qty | none | no | manual intervention per failure policy | none |
| Canonical successful execution | on-chain `OptionOrderPairExecuted` in `hybrid_v2_matched_executions` | Chain (canonical) | 0 | 0 (released) | position delta applied | no | not applicable | `OptionOrderPairExecuted` (already required) |
| Chain reorg of execution | Hybrid V2 reorg recovery journal | Chain (canonical) | 0 | fill qty (returns to pending until replacement branch resolves) | reverted | no | replacement branch OR manual intervention | reorg detection event |

### Existing representation → canonical migration

| Existing representation | Can reuse | Missing field/table | Economic meaning | Required migration/change |
| --- | --- | --- | --- | --- |
| `option_orders` schema | Yes | `canonical_order_hash` (added migration 0053) | Cryptographic order identity | 0053 (this milestone) |
| `option_fills` schema | Yes | `canonical_execution_id` (added migration 0053) | Cryptographic fill identity | 0053 (this milestone) |
| `option_execution_intents` | Yes | `canonical_execution_id` FK (deferred F3) | Correlation from Options intent to HV2 execution row | future migration |
| `hybrid_v2_matched_executions` | Yes | No change | Canonical settled fill row | none |
| `hybrid_v2_positions` | Yes | No change | Canonical position derived from settled events | none |
| `hybrid_v2_reservations` | Yes for on-chain reservations | Not populated by off-chain Options today | Off-chain open-order risk not currently modeled at HV2 layer | future — see F2 |

## Canonical position policy (Part I)

**Frozen**: `OFFCHAIN_MATCH_DOES_NOT_DIRECTLY_CREATE_CANONICAL_
POSITION`.

`hybrid_v2_positions` is populated only by
`OptionOrderPairExecuted` on-chain events via
`src/hybrid_v2/reducer.rs:523` and
`src/hybrid_v2/persistence.rs:3468`. There is no code path in
`src/options/` that writes `hybrid_v2_positions`, and this
milestone freezes that boundary.

Between the point where an Options match is committed off-chain and
the point where the chain event lands in the HV2 reducer, the
position at `hybrid_v2_positions` remains unchanged. This is
economically correct: the match is not settled.

If UI needs pre-settlement visibility, the correct surface is
Options' own `option_fills` table (queries scoped by subaccount per
the previous milestone's routing). That table represents
"matched but not-yet-canonical-settled" state, is honestly labeled
as such, and does not mutate `hybrid_v2_positions`.

Off-chain reservation modeling (F2) is documented as a follow-up.
Today Options relies on eth_call simulation at broadcast time
(`src/options/broadcast_policy_data.rs`) to prevent overcommit; no
Rust-side reservation ledger exists. Adding one would require a
separate reservation-model milestone with product decisions on:
* whether reservations block subsequent order acceptance
* whether reservations are consumed at settlement or at broadcast
* whether reorged executions restore reservation state
* whether reservations are exposed via a new API surface

## Reservation model boundary (Part E, honest scope)

At the milestone brief's Part E level of detail, Options today
does NOT have a dedicated reservation ledger. The existing pattern:

* Order acceptance → no explicit reservation row (relies on
  simulation at broadcast time to reject over-committed trades)
* Match → no explicit reservation transition (see settlement policy
  below)
* Cancel → no explicit reservation release (no reservation existed)

For this milestone the frozen posture is:

* `OPTIONS_OPEN_ORDER_RISK_MODEL` = "eth_call simulation at broadcast
  time is the only pre-execution gate; no off-chain reservation
  ledger exists"
* Adding an off-chain reservation ledger is a design decision that
  belongs in a dedicated milestone (see F2 below).

## Pending settlement exposure (Part F, honest scope)

Pending settlement exposure is currently represented by the
existence of an `option_execution_intents` row in a non-terminal
status:

* `pending_signatures` — both signatures not yet collected
* `calldata_ready` — signatures collected, ready to broadcast
* `broadcasting` — tx submitted
* `broadcasted` — receipt not yet seen

While the intent is in any of these states, the corresponding
Options fill is "matched-but-not-canonical-settled". Position
accounting at `hybrid_v2_positions` does NOT yet apply.

Adding an explicit `option_pending_settlement_exposure` table with
its own reducer path would require the same product-level decisions
as the reservation ledger and is scoped as F3 below.

## Settlement truth table (Part J)

Actor conventions:
* `Buyer(A, s1)` = wallet A subaccount 1 as buyer
* `Seller(B, s2)` = wallet B subaccount 2 as seller
* All amounts denominated in the canonical settlement asset

### T1 — Call buyer, complete quantity, positive fee, different wallets

| Actor | Pre-settlement | Premium | Collateral | Position | Fee | Post-settlement `hybrid_v2_reservations` | Canonical event |
| --- | --- | --- | --- | --- | --- | --- | --- |
| Buyer(A, s1) | matched intent pending | −premium | none | +long_call qty | −fee_buyer | unchanged | `OptionOrderPairExecuted` |
| Seller(B, s1) | matched intent pending | +premium | −collateral_locked | +short_call qty | −fee_seller | unchanged (collateral now bound to position) | `OptionOrderPairExecuted` |

### T2 — Call seller, complete quantity, positive fee (same T1 view from seller side)

Same as T1 — `OptionOrderPairExecuted` is one event, effects on both
sides derive from it via HV2 reducer.

### T3 — Put buyer, complete quantity, rebate

Same shape as T1; `fee_buyer` becomes negative (rebate credit).

### T4 — Same wallet, two subaccounts, complete quantity

| Actor | Pre-settlement | Premium | Collateral | Position | Fee | Reservations | Canonical event |
| --- | --- | --- | --- | --- | --- | --- | --- |
| Buyer(A, s2) | matched intent pending | −premium | none | +long qty on `(A, s2)` | −fee | unchanged | one `OptionOrderPairExecuted` |
| Seller(A, s1) | matched intent pending | +premium | −collateral | +short qty on `(A, s1)` | −fee | unchanged | same event |

Per `NO_CROSS_SUBACCOUNT_NETTING`, wallet A's subaccount 1 and
subaccount 2 remain economically isolated. The same on-chain event
updates two distinct rows in `hybrid_v2_positions` and (potentially)
`hybrid_v2_balances`.

### T5 — Partial fill

Same shape as T1 with `quantity` replaced by `fill_quantity`.
Remaining order size stays in `option_orders.remaining_size_1e8`
and can be filled again (GTC) or terminates (IOC residual, FOK
never partial).

### T6 — Zero-fee execution

Same shape as T1 with `fee = 0` on both sides.

### T7 — Rebate execution

Same shape as T1 with `fee_buyer < 0` or `fee_seller < 0`; net
protocol fee = `fee_buyer + fee_seller`; if net < 0, the rebate
must not exceed the pre-funded rebate budget (checked by
`FeesManagerV2`).

### Off-chain vs on-chain vs projection

| Effect | Where recorded |
| --- | --- |
| Order accepted | off-chain (`option_orders`) |
| Match committed | off-chain (`option_fills`) |
| Pending intent | off-chain (`option_execution_intents`) |
| Broadcast tx submitted | off-chain (`option_execution_transactions`) |
| Tx receipt | off-chain (`option_execution_transactions.receipt_json`) |
| `OptionOrderPairExecuted` event | on-chain, indexed → HV2 raw logs |
| Position delta | HV2 reducer → `hybrid_v2_positions` (canonical) |
| Balance delta | HV2 reducer → `hybrid_v2_balances` (canonical) |
| Fee event | HV2 reducer → `hybrid_v2_fee_events` (canonical) |
| Normalized history | HV2 reducer → `hybrid_v2_history` (canonical) |
| Options-native lifecycle projection | `option_execution_reconciliations` (existing) |

## Failure policy (Part H)

For each failure mode, the safety posture:

| Failure mode | Trigger | Automatic action | Manual intervention | Restores order to book? |
| --- | --- | --- | --- | --- |
| Deterministic simulation revert | eth_call revert | Intent → `failed_simulation` terminal | Operator inspects revert reason; may create replacement intent | No |
| Signer unavailable | not applicable (user-signed) | — | — | — |
| Signing rejected | user does not sign | Intent stays in `pending_signatures` up to TTL | User resubmits signature | No |
| Broadcast disabled | config flag | Intent → `calldata_ready` terminal | Operator enables broadcast + retries | No |
| Transaction submission unknown | RPC transient | Reconciliation worker polls; intent stays `broadcasting` | Operator inspects if unresolved | No |
| Transaction dropped | mempool eviction | Reconciliation worker detects; intent → `broadcast_dropped` terminal | Operator may create replacement intent | No |
| Transaction reverted | on-chain revert | Intent → `broadcast_reverted` terminal; no `OptionOrderPairExecuted` event | Operator inspects; may create replacement | No |
| Nonce conflict | pre-broadcast policy check | Broadcast decision rejects; intent → `nonce_conflict` terminal | Operator resolves nonce state and retries | No |
| Chain reorg after successful settlement | HV2 reorg recovery | HV2 reducer inverts orphaned position/premium/fee | Depends on whether same tx canonical on replacement branch | No |

**Frozen default**: on any failure, the corresponding fill quantity
does NOT return to the order book. The match is committed; only the
settlement is in question. Order remaining_size stays as-is. If the
user wants to re-attempt, a new order must be submitted.

**Rationale**: silently restoring matched quantity to the book
would create a hidden race condition — a counterparty might have
already treated the fill as settled off-chain and adjusted its
hedging. Manual intervention is safer.

## Execution candidate boundary (Part L)

A canonical execution candidate for Options is the tuple
`(canonical_execution_id, buyer_order_hash, seller_order_hash,
buyer_owner, buyer_subaccount_id, seller_owner, seller_subaccount_id,
series_id, fill_quantity_1e8, execution_premium_1e8, fee_context,
matching_timestamp_ms)`.

`canonical_execution_id` and the two order hashes are the ones
derived by this milestone's `canonical_identity` module. The
remaining fields are already on the `option_fills` row.

Because Options remains user-signed (see Part M blocker), the
candidate is NOT enqueued into `hybrid_v2_execution_requests`. It
lives inside the Options-native execution intent lifecycle
(`option_execution_intents` + `option_execution_transactions`).

The correlation guarantee — one Options fill has exactly one
canonical HV2 execution ID — is preserved by:
* Sparse UNIQUE index on `option_fills.canonical_execution_id`
  (migration 0053)
* Deterministic derivation from order hashes + quantity
* Immutability trigger preventing UPDATE overwrites

Duplicate delivery of the same fill (idempotent re-insert) maps to
the same canonical execution ID; the UNIQUE index prevents the
second row from landing.

## FeesManagerV2 execution context (Part K)

Preserved from previous milestone's verdict:
`FEES_MANAGER_V2_IS_THE_CANONICAL_OPTIONS_FEE_MODEL`.

Fee context per execution candidate:
* Product kind: `Options`
* Fee basis: premium (`FEES_OPTION_FEE_BASIS = PremiumOrUnderlyingCapped`
  from migration 0043)
* Schedule / tier snapshot: read from FeesManagerV2 via eth_call
  at broadcast decision time
  (`src/options/broadcast_policy_data.rs:gather_inputs`)
* Maker/taker flow: derived from `OptionFill.taker_side` +
  `maker_order_id` / `taker_order_id`
* RFQ discounts: applied only for RFQ-originated fills
* Rebate budget: read from FeesManagerV2 at same time; rebate
  approved only if within budget

No pre-booking of fees at off-chain match time. Canonical fee /
rebate state comes from `TradingFeeCharged` events consumed by
`src/fees/service.rs:442`, projected into `hybrid_v2_fee_events`.

## Reorg semantics (Part O, brief scope)

When the on-chain settlement event is reorged (see
`src/hybrid_v2/reorg_recovery.rs`):
1. HV2 reducer detects orphaned `OptionOrderPairExecuted` event via
   canonical block ancestry.
2. `hybrid_v2_positions` delta is inverted.
3. `hybrid_v2_balances` delta is inverted.
4. `hybrid_v2_fee_events` orphaned row is invalidated.
5. `hybrid_v2_matched_executions` row is marked `Reorged`.

At the Options layer:
* `option_execution_intents.status` remains at whatever it was.
* If the same tx canonicalizes on the replacement branch → no
  further action; HV2 reducer applies the fresh events.
* If the tx does NOT canonicalize → intent moves to
  `broadcast_reverted_manual_intervention_required` (Options-native
  status).

The fill row (`option_fills`) is NOT removed. Its
`canonical_execution_id` stays populated. The match commit was
off-chain and remains valid; only the settlement is in flux.

## Restart safety (Part P, brief scope)

Every persisted state above survives process restart because:
* `option_orders`, `option_fills`, `option_execution_intents`,
  `option_execution_transactions` are all PostgreSQL rows.
* `canonical_order_hash` and `canonical_execution_id` are
  deterministically re-derivable if lost (pure function of persisted
  columns).
* HV2 execution correlation rebuilds from journal replay per
  `src/hybrid_v2/rebuild.rs`.

## Returned verdicts

Ship / affirm:

* `OPTIONS_HYBRID_V2_ECONOMIC_LIFECYCLE_MODEL_RESOLVED` — see table
  above.
* `OPTIONS_HYBRID_V2_CANONICAL_ORDER_IDENTITY_VALIDATED` —
  `derive_canonical_order_hash` shipped; 11 unit tests passing.
* `OPTIONS_HYBRID_V2_CANONICAL_EXECUTION_ID_VALIDATED` —
  `derive_canonical_execution_id_from_fill` shipped; sparse UNIQUE
  index + immutability trigger in migration 0053.
* `OPTIONS_HYBRID_V2_CANONICAL_POSITION_POLICY_VALIDATED` — frozen
  boundary: off-chain match does NOT create canonical position.
* `OPTIONS_HYBRID_V2_SETTLEMENT_TRUTH_TABLE_VALIDATED` — authored
  above.
* `OPTIONS_HYBRID_V2_EXECUTION_FEE_CONTEXT_VALIDATED` — FeesManagerV2
  posture preserved from previous milestone; execution candidate
  fee context documented.
* `OPTIONS_HYBRID_V2_FAILED_SETTLEMENT_POLICY_RESOLVED` — table
  above; frozen default is "no return to book, manual intervention
  where needed".
* `OPTIONS_HYBRID_V2_ECONOMIC_EXECUTION_RESTART_SAFE` — every
  persisted state survives restart; canonical hashes are
  deterministic.
* `OPTIONS_HYBRID_V2_ECONOMIC_CORE_SECURITY_VALIDATED` — canonical
  identity binds subaccount, deployment, chain, nonce, deadline;
  cross-subaccount collision impossible; cross-chain replay
  impossible.

Explicitly NOT returned in this session:

* `OPTIONS_HYBRID_V2_OPEN_ORDER_RESERVATION_MODEL_VALIDATED` — no
  off-chain reservation ledger exists; adding one is a dedicated
  milestone (F2 below).
* `OPTIONS_HYBRID_V2_PENDING_SETTLEMENT_EXPOSURE_VALIDATED` —
  current representation is `option_execution_intents` row
  existence; explicit exposure table is deferred (F3 below).
* `OPTIONS_HYBRID_V2_RESERVATION_TRANSITIONS_VALIDATED` — depends
  on reservation ledger (F2 blocker).
* `OPTIONS_HYBRID_V2_EXECUTION_CANDIDATE_VALIDATED` — candidate
  concept documented but not atomic-with-match at INSERT level;
  requires wiring INTO the fill INSERT (F1 below).
* `OPTIONS_HYBRID_V2_EXECUTION_ORCHESTRATOR_INTEGRATED` —
  **architecture blocker** (see Part M above).
* `OPTIONS_HYBRID_V2_CANONICAL_SETTLEMENT_TRANSITION_VALIDATED` —
  depends on pending exposure table (F3).
* `OPTIONS_HYBRID_V2_SETTLEMENT_REORG_SEMANTICS_VALIDATED` — behavior
  documented; no new reorg test binary.
* `OPTIONS_HYBRID_V2_ECONOMIC_CORE_POSTGRES_MATRIX_VALIDATED` — no
  new 42-case PG matrix in this session.
* `OPTIONS_HYBRID_V2_ECONOMIC_CORE_PROPERTIES_VALIDATED` — no new
  proptest suite in this session.
* `OPTIONS_HYBRID_V2_ECONOMIC_CORE_CI_GATE_VALIDATED` — no new CI
  gate in this session.
* `OPTIONS_HYBRID_V2_ECONOMIC_EXECUTION_CORE_V1_COMPLETE` — **NOT
  RETURNED**. Architecture blocker + follow-up wiring required.
* `READY_FOR_OPTIONS_HYBRID_V2_PRODUCT_CLOSURE_V1` — **NOT
  RETURNED**.

## Safety statements (reaffirmed)

* NO real public-chain transaction sent this session.
* Exact `eth_sendRawTransaction` real-chain calls: 0.
* No `/tmp/deopt_*` created. No PG containers provisioned.
* Base mainnet 8453 never contacted.
* Frontend repo untouched (HEAD `83e68a8`).
* Solidity repo untouched (HEAD `f080272`).

## Follow-up work (blocking full closure)

| ID | Item | Est. effort |
| --- | --- | --- |
| F1 | Wire `canonical_order_hash` INSERT-time population in `submit_option_order` + `option_fill_from_match` (both `src/db/repository.rs` and `src/options/store.rs`); ripple to 45 struct-literal sites | 1 milestone |
| F2 | Off-chain reservation ledger for Options open-order risk (product decision + reducer path + reorg semantics + API) | 2 milestones |
| F3 | Explicit `option_pending_settlement_exposure` table + reducer path + settlement release | 1–2 milestones |
| F4 | Options → HV2 `ExecutionOrchestrator` unification (see Part M blocker paths A/B) | Recommended: adopt Option A (0 milestones) |
| F5 | 42-case disposable PG E2E matrix per Part Q | 1–2 milestones |
| F6 | 15-property bounded proptest suite per Part R | 1 milestone |
| F7 | CI gate + operator runbook updates | 1 short milestone |

## Files

Backend HEAD before this session: `991155f`.
Backend HEAD after this session: (see git log).

Files added:
* `src/options/canonical_identity.rs` (module, 300+ lines including
  tests).
* `migrations/0053_options_canonical_execution_identity.sql`.
* `docs/OPTIONS_HYBRID_V2_ECONOMIC_EXECUTION_CORE_V1.md` (this file).

Files modified:
* `src/options/mod.rs` — added `pub mod canonical_identity`.

## Next stage

Because Part M is an architecture blocker requiring product review,
the honest next stage is one of:

1. `OPTIONS-HYBRID-V2-CANONICAL-IDENTITY-WIRING-V1` — Land F1
   (INSERT-time population of `canonical_order_hash` +
   `canonical_execution_id`).
2. `OPTIONS-HYBRID-V2-OFFCHAIN-RESERVATION-DESIGN-V1` — Product +
   engineering design for F2.
3. `OPTIONS-HYBRID-V2-PENDING-SETTLEMENT-EXPOSURE-V1` — F3.

The originally-scoped next stage
`OPTIONS-HYBRID-V2-PRODUCT-CLOSURE-V1` should NOT proceed until F1
lands minimum; F2 and F3 preferable.
