# Backend FeesManagerV2 Event Decoding and Lifecycle Support — V2D-E

Date: 2026-05-25

## Purpose

V2D-D shipped `FeesManagerV2.sol` (signed-ppm fee model) and wired it into
the option `MarginEngine.applyTrade` path. The Solidity layer now coexists
with V1:

- `TradingFeeCharged` (V1) is still emitted by `MarginEngine` for backward
  compatibility on positive fees.
- `FeesManagerV2` emits the new event family (`FeeChargedV2`,
  `FeeRebatedV2`, rebate-budget accounting events, and admin/lifecycle
  events).

V2D-E is the backend-only follow-up that:

1. **Decodes** every `FeesManagerV2` event the operator cares about,
   alongside the existing V1 events. V1 decoding is unchanged.
2. **Summarizes** V1, V2, and mixed events through one shared module
   (`fees::onchain_summary`), used by both the lifecycle aggregator and
   the `/admin/fees/onchain` endpoint.
3. **Avoids double-counting** when the same fee flow emits both a V1
   compatibility log and a V2 source-of-truth log.

V2D-E does not touch Solidity, the frontend, the executor signing path,
broadcast, real `.env` values, live fee rates, deploy scripts, generic
`execution_transactions`, `option_execution_transactions`,
`option_execution_intents`, or any evidence rows.

## Hard Rules Verification

| Rule | Status |
| --- | --- |
| Do not submit transactions | ✅ no `eth_sendRawTransaction` reference added |
| Do not broadcast | ✅ no broadcast call introduced |
| Do not call `/executor/broadcast` | ✅ not called |
| Do not call `POST /options/execution-intents/:id/broadcast` | ✅ not called |
| Do not create new option execution intents | ✅ no insert into `option_execution_intents` |
| Do not create `option_execution_transactions` | ✅ no insert into that table |
| Do not create generic `execution_transactions` | ✅ no insert; tests assert `state.trade_signatures.is_empty()` and `state.repository.is_none()` |
| Do not cleanup evidence rows | ✅ no DELETE/UPDATE on `option_execution_events` |
| Do not modify Solidity | ✅ `../deopt-v2-sol/` untouched |
| Do not modify frontend | ✅ no frontend changes |
| Do not deploy contracts | ✅ no deployment script changes |
| Do not change live fee rates | ✅ V2 rates are in Solidity only; backend reads them as events |
| Do not touch `.env` | ✅ env loader only adds an optional `OPTION_EVENT_INDEXER_FEES_MANAGER_V2_ADDRESS` reader |
| Do not print secrets | ✅ no secret printing path added |

## Required Solidity Event Audit (read-only)

Extracted verbatim from
`../deopt-v2-sol/src/fees/IFeesManagerV2.sol`:

```solidity
event FeeChargedV2(
    address indexed consumer,
    address indexed trader,
    address indexed recipient,
    address settlementAsset,
    uint8 productKind,
    uint8 flowKind,
    bool isMaker,
    int32 feePpm,
    uint256 basisAmount,
    uint256 feeAmount
);

event FeeRebatedV2(
    address indexed consumer,
    address indexed trader,
    address indexed recipient,
    address settlementAsset,
    uint8 productKind,
    uint8 flowKind,
    int32 rebatePpm,
    uint256 basisAmount,
    uint256 rebateAmount
);

event RebateBudgetFunded(address indexed settlementAsset, uint256 amount);
event RebateBudgetWithdrawn(address indexed settlementAsset, address indexed to, uint256 amount);
event RebateBudgetSpent(address indexed settlementAsset, uint256 amount);

event FeeRecipientSet(address indexed oldRecipient, address indexed newRecipient);
event FeeConsumerSet(address indexed consumer, bool allowed);
event MerkleRootSet(bytes32 indexed root, uint64 validFrom, uint64 validUntil);
event TierClaimed(address indexed account, uint8 tier, uint64 validUntil);
```

> Note: the V2 signatures for `MerkleRootSet` and `TierClaimed` differ
> from V1 (V1 had `MerkleRootSet(bytes32,bytes32,uint64)` and
> `TierClaimed(address,uint8,uint64,uint64)`). The two families
> therefore have **different `topic0`**, so the indexer can decode both
> without ambiguity. To keep the persisted `event_name` unambiguous, V2
> rows are stored as `MerkleRootSetV2` / `TierClaimedV2` /
> `FeeRecipientSetV2` / `FeeConsumerSetV2`.

## What V2D-E Changed

### Config

`OptionEventIndexerConfig` gains a new optional field:

```rust
pub fees_manager_v2_address: Option<AccountId>,
```

read from `OPTION_EVENT_INDEXER_FEES_MANAGER_V2_ADDRESS` (with
`FEES_MANAGER_V2` as a shorthand). When unset, V2 indexing is a no-op
and V1 behavior is fully unchanged. When set, the indexer subscribes a
new `fees_manager_v2` emitter role on this contract, listening for the
nine V2 topics.

### Indexer

`src/options/event_indexer.rs`:

- New `FEE_CHARGED_V2_SIGNATURE`, `FEE_REBATED_V2_SIGNATURE`,
  `REBATE_BUDGET_FUNDED_SIGNATURE`, `REBATE_BUDGET_WITHDRAWN_SIGNATURE`,
  `REBATE_BUDGET_SPENT_SIGNATURE`, `FEE_RECIPIENT_SET_V2_SIGNATURE`,
  `FEE_CONSUMER_SET_SIGNATURE`, `MERKLE_ROOT_SET_V2_SIGNATURE`,
  `TIER_CLAIMED_V2_SIGNATURE` constants and matching `*_topic0()` helpers.
- `event_topics_for_emitter_role("fees_manager_v2")` exposes the nine
  V2 topics. The V1 role is unchanged.
- `decode_option_execution_event` dispatches to nine new decoders.
- Each decoder produces an `OptionExecutionEvent` with `event_name`
  one of `FeeChargedV2`, `FeeRebatedV2`, `RebateBudgetFunded`,
  `RebateBudgetWithdrawn`, `RebateBudgetSpent`, `FeeRecipientSetV2`,
  `FeeConsumerSetV2`, `MerkleRootSetV2`, `TierClaimedV2`.
- Two new ABI helpers: `decode_data_address` (reads a non-indexed
  address from a 32-byte word) and `decode_data_i32` (reads a signed
  `int32` ABI-encoded as a sign-extended 32-byte word; non-extending
  bytes are rejected).

### Persisted fields

Each V2 event row populates `OptionExecutionEvent` with:

| Field | V1 (TradingFeeCharged) | V2 (FeeChargedV2 / FeeRebatedV2) |
| --- | --- | --- |
| `event_name` | `TradingFeeCharged` | `FeeChargedV2` / `FeeRebatedV2` |
| `event_signature` | `TradingFeeCharged(...)` | new V2 signatures |
| `contract_address` | indexed log address (lowercased) | same |
| `tx_hash`, `log_index`, `block_number` | from log | from log |
| `account` | `trader` (topic2) | `trader` (topic2) |
| `premium_per_contract_native` | `appliedFee` | `feeAmount` / `rebateAmount` |
| `decoded.consumer` | n/a | topic1 |
| `decoded.recipient` | topic2 | topic3 |
| `decoded.settlementAsset` | topic3 | data word 0 |
| `decoded.productKind` | n/a | `"option"` / `"perp"` |
| `decoded.flowKind` | n/a | `"orderbook"` / `"rfq"` |
| `decoded.isMaker` | data word 1 (bool) | data word 3 (V2 charged) / always true (V2 rebated) |
| `decoded.feePpm` / `decoded.rebatePpm` | n/a | signed int32 |
| `decoded.basisAmount` | n/a | uint256 |
| `decoded.feeAmount` / `decoded.rebateAmount` | n/a | uint256 |

Budget and config events persist the relevant scalar fields under
intuitive JSON keys (`settlementAsset`, `amount`, `to`, `consumer`,
`allowed`, `oldRecipient`, `newRecipient`, `root`, `validFrom`,
`validUntil`, `account`, `tier`).

### Shared on-chain summary module

New `src/fees/onchain_summary.rs` exposes:

- `normalize_fee_events(&[OptionExecutionEvent]) -> NormalizedFees` —
  groups events by family (V1 charged, V2 charged, V2 rebated).
- `classify_event_model(&NormalizedFees) -> "v1" | "v2" | "mixed" | "none"`.
- `aggregate(&NormalizedFees) -> AggregatedFees` — applies the
  double-counting policy: when both V1 and V2 are present, V2 totals
  win (`source_priority = "v2"`), V1 events become compatibility
  evidence and contribute zero to the totals.
- `summarize_fees_for_lifecycle` — used by `LifecycleFees`.
- `summarize_admin_onchain` — used by the `/admin/fees/onchain`
  endpoint to render an overall + per-tx breakdown.

### Lifecycle fee read model

`LifecycleFees` (in `src/options/lifecycle.rs`) gains new fields while
keeping every existing field for backward compatibility:

| Field | V2D-E meaning |
| --- | --- |
| `source_of_truth` | always `"onchain"` (V2C contract preserved) |
| `event_model` | `"v1"` / `"v2"` / `"mixed"` / `"none"` |
| `source_priority` | `"v2"` when mixed; `""` otherwise |
| `trading_fee_event_count` | V1 count (preserved) |
| `fee_charged_v2_count`, `fee_rebated_v2_count` | new V2 counters |
| `observed_total` | charged total per the selected model (alias of `observed_total_charged`, V1Z-compatible) |
| `observed_total_charged`, `observed_total_rebated`, `net_protocol_fee` | new V2 fields |
| `by_trader`, `by_recipient`, `by_side`, `total_by_recipient` | preserved; only positive fees feed these maps |
| `rebated_by_trader` | new V2 map; rebates received per trader |
| `backend_ledger_status` | preserved (V2C semantics) |
| `reconciliation_status` | `"onchain_observed"` when any V1 or V2 fee event exists, else `"no_onchain_events"` |

### Admin endpoint `/admin/fees/onchain`

The response shape gains the same V2 fields and per-tx event-model
metadata. Existing fields (`source_of_truth`, `backend_ledger_enabled`,
`backend_ledger_status`, `filter.tx_hash`, `filter.limit`,
`trading_fee_event_count`, `observed_total`, `by_trader`, `by_recipient`,
`by_side`, `reconciliation_status`, `transactions[*]`, `events[*]`)
remain in place so V1-era consumers do not break.

The unfiltered call now widens its load filter to include V2 events
(`TradingFeeCharged | FeeChargedV2 | FeeRebatedV2`).

## Source-of-Truth Rules

1. The indexed on-chain log is always the source of truth.
2. When only V1 events are present, totals match V1 behavior exactly —
   no behavioral drift for historical V1S trades.
3. When V2 events are present (with or without V1 compatibility logs),
   the V2 events drive every total. `event_model` reports `"v2"` (pure
   V2) or `"mixed"` (V2 with V1 compat alongside, `source_priority="v2"`).
4. The backend fee ledger remains informational; its presence/absence
   is reported via `backend_ledger_status` and never fails the read.

## Double-Count Prevention

The V2D Solidity engine emits a V1 `TradingFeeCharged` log **and** a
V2 `FeeChargedV2` log for the same positive-fee flow. If the backend
summed both, the `observed_total_charged` would double-count.

The policy in `fees::onchain_summary::aggregate` is:

| `event_model` | charged total source | rebated total source |
| --- | --- | --- |
| `"v1"` | V1 `TradingFeeCharged` | none (V1 has no rebates) |
| `"v2"` | V2 `FeeChargedV2` | V2 `FeeRebatedV2` |
| `"mixed"` | V2 `FeeChargedV2` (V1 → 0 contribution) | V2 `FeeRebatedV2` |
| `"none"` | 0 | 0 |

V1 events that are demoted to compatibility evidence still appear in
the `events[]` list with `event_model: "v1"` and the original
`applied_fee` payload, so an operator can reconcile manually if needed.

The `trading_fee_event_count`, `fee_charged_v2_count`, and
`fee_rebated_v2_count` counters always reflect the **observed** number
of events of each kind, regardless of which model drives the totals.

## Tests Added

| Suite | Test | Asserts |
| --- | --- | --- |
| `options::event_indexer` | `fee_charged_v2_log_decodes_topics_and_signed_ppm` | topics, productKind, flowKind, signed positive ppm, `feeAmount` |
| `options::event_indexer` | `fee_rebated_v2_log_decodes_negative_ppm_and_amount` | negative `rebatePpm` (signed int32 sign-extended), rebate amount |
| `options::event_indexer` | `rebate_budget_funded_decodes_amount` | settlementAsset + amount |
| `options::event_indexer` | `rebate_budget_withdrawn_decodes_two_addresses` | settlementAsset + to + amount |
| `options::event_indexer` | `rebate_budget_spent_decodes_amount` | amount |
| `options::event_indexer` | `fee_recipient_set_v2_decodes_topics` | old/new recipient |
| `options::event_indexer` | `fee_consumer_set_decodes_bool` | consumer, allowed |
| `options::event_indexer` | `merkle_root_set_v2_decodes_window` | validFrom, validUntil |
| `options::event_indexer` | `tier_claimed_v2_decodes_account_and_tier` | account, tier, validUntil |
| `options::event_indexer` | `fees_manager_v2_emitter_role_subscribes_to_v2_topics_and_decodes` | indexer subscribes 9 V2 topics on V2 address and persists `FeeChargedV2` |
| `options::event_indexer` | `v1_default_indexer_does_not_subscribe_to_v2_topics` | V1 config does not request the V2 emitter role |
| `fees::onchain_summary` | `empty_events_classify_as_none` | event_model none |
| `fees::onchain_summary` | `v1_only_classifies_as_v1_and_sums_applied_fees` | V1-only totals + by_side |
| `fees::onchain_summary` | `v2_only_classifies_as_v2_and_uses_v2_totals` | V2 charged+rebated, `net_protocol_fee` |
| `fees::onchain_summary` | `mixed_v1_and_v2_does_not_double_count` | `event_model = "mixed"`, `source_priority = "v2"`, V2 total wins |
| `options::lifecycle` | `fees_view_v1_only_unchanged` | V1S behavior preserved |
| `options::lifecycle` | `fees_view_v2_only_summarizes_charged_and_rebated_and_net` | V2 lifecycle fields |
| `options::lifecycle` | `fees_view_mixed_v1_and_v2_does_not_double_count` | mixed lifecycle policy |
| `options::lifecycle` | `fees_view_v1_compatibility_field_set_total_by_recipient_for_v2` | `total_by_recipient` alias kept under V2 |
| `api::routes` | `admin_fees_onchain_exposes_observed_trading_fee_events` (V1, pre-existing) | V1 behavior unchanged |
| `api::routes` | `admin_fees_onchain_exposes_v2_charged_and_rebated_totals` | V2 charged + rebated + net |
| `api::routes` | `admin_fees_onchain_mixed_v1_v2_uses_v2_priority` | mixed scenario via admin endpoint |
| `api::routes` | `admin_endpoints_do_not_mutate_state` (pre-existing) | no row mutation across all admin GETs |

Every V2 test asserts `state.repository.is_none()` and
`state.trade_signatures.is_empty()` (the existing
`assert_no_generic_execution_rows` invariant), and the existing
`/admin/fees/onchain` no-mutation test still passes — confirming the
new code path does not touch the trade-signature mailbox, the generic
`execution_transactions` repository surface, or any broadcast helper.

## Validation Commands Run

- `cargo fmt --all` — clean
- `cargo clippy --all-targets --all-features -- -D warnings` — clean
- `cargo test --all-targets --all-features` — all lib + integration
  suites green (no V1 regressions)
- `cargo build --all-targets --all-features` — clean

## Remaining Deferred Work

- **Live Base Sepolia deployment / wiring** — operator still needs to
  set `OPTION_EVENT_INDEXER_FEES_MANAGER_V2_ADDRESS` and observe a real
  V2 trade end-to-end. Backend is ready; the deploy/wire step is out of
  scope for V2D-E.
- **V2 fee drift checks** — comparing `feePpm` in the indexed log
  against the backend preview (`fees::schedule::resolve_rates_from_volume`)
  is V2E follow-up work.
- **Perps integration** — once perps emit `FeeChargedV2` /
  `FeeRebatedV2`, the same decoders handle them; no backend code change
  expected, but a perps-specific reconciliation policy may want its own
  test suite.
- **RFQ flow support** — `decoded.flowKind` already carries `"rfq"`,
  but downstream surfaces (perp/option RFQ ledgers) are not yet wired.
- **Frontend fee dashboard update** — surface `event_model`,
  `source_priority`, `observed_total_rebated`, `net_protocol_fee`,
  and `rebated_by_trader` on the admin console.
- **`/admin/fees/onchain/by-intent/:intent_id`** — V2F task; currently
  the lifecycle endpoint surfaces the same view for one intent and the
  admin endpoint pages by tx hash.
