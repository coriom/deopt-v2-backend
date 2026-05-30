# V2F-N — Backend Perp V2 Fee Cutover and Observability

## Status

- Milestone: **V2F-N**
- Date: 2026-05-30
- Mode: backend-only, read-only verification + regression tests; **no
  Solidity, no broadcast, no live mutation**.
- Live state surfaced by this milestone (Base Sepolia):
  - `OLD_PERP_ENGINE` = `0xB36395b67D0798ADA981731c9Fa5239F4362b53B`
    *(stranded under A3 Base Sepolia fallback; backend does not index it
    as a perp matching engine — see "OLD stranded warning" below)*
  - `NEW_PERP_ENGINE` = `0xc6C592100723Fe0C66343A16e95eC34cC0c2141c`
  - `FEES_MANAGER_V2` = `0x00dA0B9876bcBf0c79CB5BcAcfEBAFb8C7Ad774f`
  - `PERP_MATCHING_ENGINE` = `0x774d96E5739bffadEE91508b4D3D74F5BE29F165`
  - `NEW.useFeesManagerV2()` = `true`
  - `NEW.feesManagerV2()` = `FEES_MANAGER_V2`
  - `FeesManagerV2.isFeeConsumer(NEW)` = `true`

## V2F-LM smoke transaction (reference)

| Field | Value |
| --- | --- |
| `tx_hash` | `0x400acedf36381034ae37c983cc50e80d11a81587ca8065fbaef40293ff63a79a` |
| `block` | `42188599` |
| `productKind` (raw / label) | `1` / `perp` |
| `flowKind` (raw / label) | `0` / `orderbook` |
| `FeeChargedV2` count | `2` |
| `FeeRebatedV2` count | `0` |
| Buyer/taker | `0x8B94A83D1AD3bD2337b1886E7962CA8E0bba9A34`, `feePpm=300`, `basisAmount=30`, `feeAmount=1` |
| Seller/maker | `0x475Fe397FA56884952D350aa9EE1c3946964BC0C`, `feePpm=50`, `basisAmount=30`, `feeAmount=1` |
| `feeRecipient` delta | `+2` native mUSDC |
| `merkleRoot` | `0x0…0` |
| `rebateBudget(mUSDC)` | `0` |

## Backend capability audit

### Already supported (no code change required)

1. **`FeeChargedV2` / `FeeRebatedV2` decoding for PERP.**
   `src/options/event_indexer.rs::decode_fee_charged_v2_log` and
   `decode_fee_rebated_v2_log` decode the `productKind` raw byte through
   `product_kind_label` (`1 → "perp"`) and `flow_kind_label`
   (`0 → "orderbook"`). The decoded payload preserves `productKind`,
   `productKindRaw`, `flowKind`, `flowKindRaw`, `isMaker`, `feePpm`,
   `basisAmount`, and `feeAmount`. The same code path is used for
   option-flow and perp-flow events because the V2 fee schema is
   product-agnostic at the event level.
2. **Indexer subscription.**
   `OptionEventIndexerConfig::emitter_contracts()` subscribes to
   `fees_manager_v2` by **address**, not by engine. Because
   `FeesManagerV2` is a single shared contract called by both
   `MarginEngineV2` (options) and `PerpEngineV2` (perps), every
   `FeeChargedV2` / `FeeRebatedV2` emitted by either engine's
   `chargeFee` call is indexed by the existing
   `OPTION_EVENT_INDEXER_FEES_MANAGER_V2_ADDRESS` filter — no per-engine
   wiring is needed. The `option_execution_events` table name is a
   historical artefact; rows include perp-flow fee events alongside
   option-flow events.
3. **`/admin/fees/onchain?tx_hash=<perp tx>` works.**
   `src/fees/service.rs::admin_onchain_fees` calls
   `repository.list_option_execution_events_by_tx_hash(tx_hash)`, which
   is product-agnostic. `summarize_admin_onchain` (in
   `src/fees/onchain_summary.rs`) iterates over all
   `FeeChargedV2`/`FeeRebatedV2`/`TradingFeeCharged` events for the tx
   and applies the V1/V2 source-priority policy uniformly. Per-event
   payloads in the `events` array expose `product_kind`, `flow_kind`,
   `basis_amount`, `fee_amount`, `fee_ppm`, `is_maker`, `trader`, and
   `recipient`, so an admin UI can label a row as PERP without backend
   changes.
4. **Double-counting policy.**
   `classify_event_model` returns `mixed` whenever a tx contains both V1
   and V2 fee events; `aggregate` then drives totals **only** from V2
   logs and exposes `source_priority = "v2"`. This guards against the
   PerpEngineV2 V1 compatibility breadcrumb being added to V2 charges.
   No PERP-specific code is needed because the policy keys off the
   event name, not the product kind.
5. **PERP totals path is independent of the option lifecycle.**
   `summarize_admin_onchain` and the lifecycle wrapper
   `summarize_fees_for_lifecycle` both call the same product-agnostic
   aggregator. A perp tx with no option lifecycle (no
   `OptionTradeExecuted` / `MinedSuccess` link) is summarized purely
   from the FeeChargedV2 events recorded by the FeesManagerV2 emitter
   role.

### Gap closed by this milestone

The PERP path had no regression tests pinning `productKind = "perp"`
through the decoder, the aggregator, or the admin endpoint. Every
existing V2 test was option-flavoured (`productKind = "option"`,
`flowKind = "orderbook"`). The V2F-N changes add PERP-flavoured tests
that:

- decode a raw `FeeChargedV2` ABI log with `productKindRaw = 1` and
  assert the decoded label is `"perp"`;
- replay the V2F-LM `2 × FeeChargedV2` payload shape through the
  aggregator and assert `observed_total = 2`, `by_side.taker = 1`,
  `by_side.maker = 1`, `productKind=perp` preserved on every payload;
- exercise `/admin/fees/onchain?tx_hash=0x400acedf…` against the V2F-LM
  PERP fee shape and assert the full admin JSON.

### Live read-only `/admin/fees/onchain` verification

The task explicitly allowed local non-mutating admin ticks, but the
endpoint can only return PERP V2 events that have already been indexed
into `option_execution_events` for the requested `tx_hash`. The local
dev environment here is not running the indexer against the Base
Sepolia paid RPC tier, so a live HTTP call against
`/admin/fees/onchain?tx_hash=0x400acedf…` would return
`event_model = "none"` with `reconciliation_status = "no_onchain_events"`
until the production indexer (paid Alchemy tier on the V2D-T2 cursor)
catches up past block 42188599. **No code change is needed for this** —
the path is already correct, the endpoint already accepts the
`tx_hash` parameter, and the per-tx grouping handles PERP events
identically to option events.

The regression tests added by V2F-N substitute for the live HTTP call
by replaying the V2F-LM event payload shape against the in-memory
options store and asserting the admin JSON contract.

## Backend changes

### `src/options/event_indexer.rs`

- Added `fee_charged_v2_perp_log_decodes_with_perp_product_kind` test
  that builds a raw ABI log with `productKindRaw = 1`, `flowKindRaw = 0`,
  `feePpm = 300`, `basisAmount = 30`, `feeAmount = 1`, transaction
  hash `0x400acedf…`, and asserts the decoded payload exposes
  `productKind = "perp"` (and `productKindRaw = 1`) alongside the
  preserved `basisAmount`, `feePpm`, `feeAmount`, `flowKind`,
  `isMaker`, `trader`, and `recipient` fields.
- Added `fee_charged_v2_perp_log` helper that mirrors the V2F-LM live
  smoke event addresses (FeesManagerV2 emitter, PerpEngineV2 consumer,
  V2F-LM taker trader and recipient, native settlement asset).

### `src/fees/onchain_summary.rs`

- Added `v2f_lm_perp_payloads_expose_perp_product_kind_and_totals`
  test: replays the V2F-LM `2 × FeeChargedV2` taker/maker shape through
  `normalize_fee_events`, `aggregate`, and `collect_event_payloads`.
  Asserts:
  - `event_model = "v2"`, `source_priority = ""` (pure V2 tx, no V1
    breadcrumb).
  - `fee_charged_v2_count = 2`, `fee_rebated_v2_count = 0`,
    `trading_fee_event_count = 0`.
  - `charged_total = 2`, `rebated_total = 0`, `net_protocol_fee = 2`.
  - `by_side.taker = 1`, `by_side.maker = 1` (V2F-LM split).
  - `by_trader` keyed by lowercase trader address for taker and maker.
  - Every per-event payload exposes `product_kind = "perp"`,
    `flow_kind = "orderbook"`, `basis_amount = "30"`, `fee_amount = "1"`.
- Added `v2f_lm_mixed_v1_v2_perp_does_not_double_count` test: same
  PERP taker payload plus a `TradingFeeCharged` V1 breadcrumb for the
  same trader. Asserts `event_model = "mixed"`,
  `source_priority = "v2"`, and `charged_total = 1` (V2 wins; V1 does
  not double-count).

### `src/api/routes.rs`

- Added `admin_fees_onchain_summarizes_v2f_lm_perp_fee_tx` integration
  test against the in-memory router: pre-seeds two
  `FeeChargedV2` event rows shaped like the V2F-LM live tx, then
  `GET /admin/fees/onchain?tx_hash=0x400acedf…`. Asserts the response:
  - `event_model = "v2"`, `fee_charged_v2_count = 2`,
    `fee_rebated_v2_count = 0`, `trading_fee_event_count = 0`.
  - `observed_total = "2"`, `observed_total_charged = "2"`,
    `observed_total_rebated = "0"`, `net_protocol_fee = "2"`.
  - `by_side.taker = "1"`, `by_side.maker = "1"`, `by_trader[taker]
    = "1"`, `by_trader[maker] = "1"`.
  - `transactions[0].tx_hash = 0x400acedf…`, per-tx counts match.
  - `events[]` contains exactly two PERP entries with
    `product_kind = "perp"`, `flow_kind = "orderbook"`,
    `basis_amount = "30"`, and the V2F-LM `fee_ppm`/`is_maker` split.
- Added `admin_fees_onchain_v2f_lm_perp_mixed_does_not_double_count`
  integration test: same PERP taker/maker pair plus a V1
  `TradingFeeCharged` breadcrumb on the same tx. Asserts
  `event_model = "mixed"`, `source_priority = "v2"`,
  `observed_total = "2"` (not `"3"`), `fee_charged_v2_count = 2`,
  `trading_fee_event_count = 1`. This pins the double-counting
  policy on the PERP path specifically.
- Added `build_fee_charged_v2_perp_log_row` test fixture (PERP-shaped
  `OptionExecutionEvent` row).

## No double-counting proof

Two independent tests exercise the mixed-V1+V2 PERP path:

1. **Aggregator-level**:
   `fees::onchain_summary::tests::v2f_lm_mixed_v1_v2_perp_does_not_double_count`
   asserts that when one V1 `TradingFeeCharged` breadcrumb is recorded
   alongside the V2F-LM `FeeChargedV2` taker leg for the same trader,
   `aggregate` returns `event_model = "mixed"`,
   `source_priority = "v2"`, and `charged_total = 1` (only V2 counted).
2. **Endpoint-level**:
   `api::routes::tests::admin_fees_onchain_v2f_lm_perp_mixed_does_not_double_count`
   asserts the admin JSON contract under the same scenario:
   `observed_total = "2"` (taker 1 + maker 1, both V2) rather than
   `"3"` (which would include the V1 breadcrumb).

The policy is implemented in `classify_event_model` /
`aggregate` (`src/fees/onchain_summary.rs:367-413`). The policy is keyed
off the event name (`TradingFeeCharged` vs `FeeChargedV2` /
`FeeRebatedV2`), not the product kind, so PERP gets the same
double-counting guarantee as options.

## No rebate proof

The V2F-LM transaction emitted **0** `FeeRebatedV2` events
(`rebateBudget(mUSDC) = 0`, `merkleRoot = 0x0`). The aggregator tests
above assert `fee_rebated_v2_count = 0`, `rebated_total = 0`, and
`net_protocol_fee = 2 = charged_total`.

## OLD stranded warning

`OLD_PERP_ENGINE = 0xB36395b67D0798ADA981731c9Fa5239F4362b53B` remains
under the A3 Base Sepolia fallback. The backend does **not** index
trades or fees emitted by OLD because:

- The `OPTION_EVENT_INDEXER_*_ADDRESS` env vars and the
  `PERP_MATCHING_ENGINE_ADDRESS` env var both point at the V2 contract
  set (`NEW_PERP_ENGINE = 0xc6C5…141c` and
  `PERP_MATCHING_ENGINE = 0x774d…F165`).
- FeesManagerV2 is configured with `isFeeConsumer(NEW) = true`; if
  OLD ever calls FeesManagerV2 it would be rejected, so no fee
  attribution drift is possible.

Operators should not retire the OLD address in observability
dashboards while it remains live on-chain: surface OLD as a stranded
emitter so any unexpected event from it is visible. No code change in
this milestone touches OLD; this doc records the stranded state.

## Frontend / admin UI changes

The existing admin UI is fed by `/admin/fees/onchain` and the lifecycle
JSON. Both already pass `product_kind`, `flow_kind`, and `basis_amount`
through the per-event payloads built by
`onchain_summary::collect_event_payloads`. No frontend change is
needed for the V2F-N milestone — the V2 fee cards added in V2E-H are
product-agnostic.

If the admin UI hard-codes "OPTION" labels anywhere, the right fix is
in the frontend repo (`~/DEOPT/deopt-v2-frontend`) and is out of scope
for this milestone. A frontend follow-up doc would land at
`~/DEOPT/deopt-v2-frontend/docs/PERP_V2_FEE_ADMIN_OBSERVABILITY_V2F_N.md`
if any frontend code change is made.

## Validations

```
cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features --no-fail-fast
cargo build --all-targets --all-features
```

Commands and exit status are recorded in the conversation transcript
for V2F-N (`backend validations` task).

## V2F-O live verification confirmation (2026-05-30)

The follow-up V2F-O milestone caught the indexer past block 42188599
(`option_events_base_sepolia.last_indexed_block: 42149417 → 42189720`)
and exercised the live `/admin/fees/onchain?tx_hash=0x400acedf…`
endpoint. Every expected V2F-LM value matched: `event_model = "v2"`,
2× `FeeChargedV2` with `product_kind = "perp"`, `flow_kind =
"orderbook"`, `basis_amount = "30"`, 1 taker + 1 maker, total charged
= 2 native mUSDC, 0 rebates, recipient =
`0xa67f8e8e…b588`. PerpEngineV2 emitted zero V1 `TradingFeeCharged`
breadcrumbs for this tx, so the pure-V2 case is exercised end-to-end
on live data. See
`docs/PERP_V2_FEE_LIVE_OBSERVABILITY_VERIFICATION_V2F_O.md` for the
full JSON, indexer cursor evidence, and acceptance table.

## Remaining gaps

- ~~The live HTTP `/admin/fees/onchain?tx_hash=0x400acedf…` call was not
  exercised against the Base Sepolia paid-tier indexer in this
  milestone because the local dev environment is not currently running
  the indexer past block 42188599. The regression tests pin the
  endpoint contract; a follow-up "indexer caught up past 42188599"
  ticket should re-verify with a real HTTP request.~~ Closed by V2F-O
  on 2026-05-30; see section above.
- Frontend label sweep for `productKind = "perp"` is deferred — V2F-N
  did not change the frontend repo.
- A `FeeRebatedV2` PERP regression test will be added when the first
  PERP rebate event lands on Base Sepolia (V2F-LM was rebate-zero, so
  there is no live shape to assert against yet).

## Acceptance checklist

- [x] V2F-LM PERP fee tx decoded (`fee_charged_v2_perp_log_decodes_*`).
- [x] `productKind = "perp"` exposed in decoded payload and
  `/admin/fees/onchain` events list.
- [x] `flowKind = "orderbook"` exposed.
- [x] `basisAmount = "30"` exposed on every PERP event payload.
- [x] `2` `FeeChargedV2` counted.
- [x] `0` `FeeRebatedV2` counted.
- [x] `observed_total_charged = "2"`, `net_protocol_fee = "2"`.
- [x] No double-counting (mixed V1+V2 path tested at aggregator and
  endpoint level).
- [x] Docs created (this file).
- [x] Validations: see "Validations" section.
