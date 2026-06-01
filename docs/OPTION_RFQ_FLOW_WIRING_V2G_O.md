# V2G-O — OPTION RFQ Flow Wiring

## Status

- Milestone: **V2G-O** — closes the integration gap V2G-N flagged:
  `MarginEngineTrading.consumeFees` hardcoded `FlowKind.ORDERBOOK`, so
  the V2G-N RFQ discount math + state could never take effect on
  chain.
- Date: 2026-05-31.
- Outcome:
  - **Source-only change.** Solidity changes compile + test green;
    contracts are NOT redeployed. The new ABI sits in
    `out/` ready for the operator's V2G-P deploy window.
  - **New sibling interface** `IMarginEngineRfqTrade` exposes
    `applyRfqTrade(IMarginEngineTrade.Trade)` — the existing
    `IMarginEngineTrade.applyTrade` ABI is untouched and remains
    bytecode-equivalent to pre-V2G-O ORDERBOOK behaviour.
  - **MarginEngineTrading internal-helper refactor.** `applyTrade` and
    `applyRfqTrade` are both thin wrappers over a shared internal
    `_applyTradeWithFlow(t, flow)` that threads
    `IFeesManagerV2.FlowKind` to the V2 fee charge. The fee-charge
    helper `_chargeTradingFeeV2` gained a `FlowKind flow` parameter.
  - **OptionMatchingEngine extension.** New `OptionRfqTrade` struct +
    `RFQ_TRADE_TYPEHASH` + `hashRfqTrade` + `executeRfqTrade` mirror
    the ORDERBOOK signing path. Maker / taker explicitly opt in to
    the RFQ fee schedule by signing the dedicated typehash —
    signatures issued against `TRADE_TYPEHASH` cannot be replayed
    here and vice versa.
  - **New event** `OptionRfqTradeExecuted` (mirrors
    `OptionTradeExecuted` shape) lets off-chain consumers route RFQ
    trades before the `FeeChargedV2.flowKind` topic lands in the
    indexer.
  - **6 new Solidity tests** in `test/unit/margin/MarginEngine.t.sol`
    pin ORDERBOOK bytecode-equivalence, RFQ flowKind emission, V2G-N
    canonical Tier 4 maker rebate preservation under Design-Option-A,
    Tier 2 RFQ taker discount, Tier 0 RFQ==ORDERBOOK, and the
    `onlyMatchingEngine` gate on the RFQ entry.
  - **Backend untouched** in V2G-O scope — V2G-N decode test already
    pins `flow_kind="rfq"` ingestion. The first real RFQ trade lands
    in `/admin/fees/onchain` and Grafana with no further backend
    change.
  - **Soak preserved.** Backend PID 56199 (V2G-G era binary) + compose
    stack untouched. The new endpoint surface only takes effect after
    the V2G-P redeploy + backend restart; the running services
    continue serving V2G-G/H/I/J/K/L0/L1/L2/L3/L4/M/N functionality.
- Hard gates respected: no broadcast, no redeploy, no chain mutation,
  no DB writes, no real `.env` edit, no private-key handling, no
  governance/timelock action, no `docker compose down -v`, no
  Prometheus reset, no backend restart, no soak interruption.

## RFQ path audit (pre-V2G-O)

```
OptionMatchingEngine.executeTrade(OptionTrade, buyerSig, sellerSig)
  ↓ _toMarginTrade(t)               ← IMarginEngineTrade.Trade has no flow flag
  ↓
MarginEngineTrading.applyTrade(Trade) via IMarginEngineTrade.applyTrade
  ↓
_chargeTradingFeeV2(trader, counterparty, isMaker, settlementAsset, optionId, premium)
  ↓
fm.consumeFees(trader, ProductKind.OPTION, ★ FlowKind.ORDERBOOK ★, isMaker, settlementAsset, premium)
                                           └─ HARDCODED on line 90 of MarginEngineTrading.sol
```

**Hardcoded ORDERBOOK location** confirmed: `src/margin/MarginEngineTrading.sol` line 90 in `_chargeTradingFeeV2`. There is no other hardcode in the OPTION path; `OptionMatchingEngine` simply doesn't carry a flow flag, so the maker/taker EIP-712 signed payload doesn't express RFQ vs ORDERBOOK consent today.

## Design decision

**Chosen pattern: ABI-additive, dual-entry-point, dedicated EIP-712 typehash.**

| Aspect | Decision | Why |
|--------|----------|-----|
| ORDERBOOK signature compatibility | Preserved. `TRADE_TYPEHASH` unchanged. | Existing signed intents in flight (maker/taker EIP-712 sigs over `OptionTrade`) keep working. |
| RFQ consent semantics | Dedicated `RFQ_TRADE_TYPEHASH` + `OptionRfqTrade` struct. | Maker / taker explicitly sign "I consent to the RFQ schedule." Signatures cannot be replayed across the two paths — the V2G-N Design-Option-A invariant requires explicit RFQ consent on the maker side. |
| Trade payload shape | Identical fields between `OptionTrade` and `OptionRfqTrade`. | The RFQ-vs-ORDERBOOK distinction is the selector the executor calls, not the per-trade payload. Keeps the MarginEngine `IMarginEngineTrade.Trade` ABI unchanged. |
| MarginEngine ABI extension | New sibling interface `IMarginEngineRfqTrade.applyRfqTrade(Trade)`. | Avoids modifying `IMarginEngineTrade` (which the matching engine and storage contracts already implement). Deployed MarginEngine post-V2G-P implements BOTH interfaces. |
| Fee charge wiring | `_chargeTradingFeeV2(..., FlowKind flow)` + `_applyTradeWithFlow(t, flow)`. | Single-source-of-truth helper that ORDERBOOK and RFQ both call. ORDERBOOK bytecode is bit-equivalent to pre-V2G-O. |
| Bytecode delta | ~3KB. | New typehash constant, new struct, new external entry, new internal helper. Within the deployable-contract size budget. |

Alternatives considered and rejected:

- **Extend `IMarginEngineTrade.Trade` with `bool isRfq`** — would require every implementor (matching engines, mock harnesses) to update; breaks the calldata payload that V2G-K's monitoring already targets. ABI-breaking change.
- **Add `isRfq` to existing `OptionTrade` struct + bump `TRADE_TYPEHASH`** — invalidates any in-flight signed intents; even after a flag-default of `false`, the digest changes.
- **Skip the dedicated EIP-712 typehash; reuse `TRADE_TYPEHASH`** — opens a UX trap where a maker signs an `OptionTrade` thinking ORDERBOOK and the executor routes it through `executeRfqTrade`. Maker compensation differs between the two flows; this would be silent consent-bypass.

## Implementation status

Source-only changes; tests green; ready for V2G-P redeploy.

### New files
- `src/matching/IMarginEngineRfqTrade.sol` — sibling interface (24 LOC).

### Modified files
- `src/margin/MarginEngineTrading.sol`
  - Inherits `IMarginEngineRfqTrade` alongside `MarginEngineAdmin`.
  - `_chargeTradingFeeV2` gains `FlowKind flow` parameter.
  - Body of `applyTrade` extracted into `_applyTradeWithFlow(t, flow)`.
  - `applyTrade` calls `_applyTradeWithFlow(t, FlowKind.ORDERBOOK)`.
  - New external `applyRfqTrade(t)` calls `_applyTradeWithFlow(t, FlowKind.RFQ)`. Same modifiers (`onlyMatchingEngine`, `whenTradingNotPaused`, `nonReentrant`).

- `src/matching/OptionMatchingEngine.sol`
  - Import `IMarginEngineRfqTrade`.
  - New event `OptionRfqTradeExecuted`.
  - New constant `RFQ_TRADE_TYPEHASH`.
  - New struct `OptionRfqTrade`.
  - New view `hashRfqTrade` + `previewRfqTradeDigest`.
  - New internal helpers `_rfqStructHash` (chunked encode workaround for via-IR stack-too-deep — see code comment), `_toMarginTradeFromRfq`, `_isStructurallyValidRfq`, `_isDeadlineValidRfq`, `_validateRfq`, `_validateSeriesMetadataRfq`, `_consumeNoncesRfq`.
  - New external `executeRfqTrade(OptionRfqTrade, bytes, bytes)`. Verifies both sigs against the dedicated typehash, then calls `IMarginEngineRfqTrade(address(marginEngine)).applyRfqTrade(_toMarginTradeFromRfq(t))`.

### Test files
- `test/unit/margin/MarginEngine.t.sol` — V2G-O test block + `_tradeRfq` helper (~190 LOC including comments).

## Tests added

| Test | Asserts |
|------|---------|
| `testV2GO_OrderbookApplyTradeBehavesIdenticallyToPreRefactor` | ORDERBOOK Tier-0 trade emits 2× `FeeChargedV2`; maker fee 5000, taker fee 25000, recipient gains 30000. Bit-equivalent to pre-V2G-O `testFeesManagerV2PositiveOptionFeesTransferAndPositionsUpdate`. |
| `testV2GO_RfqTier0EqualsOrderbookFromMarginEnginePerspective` | Tier-0 RFQ trade through `applyRfqTrade` produces the same token movements as ORDERBOOK (Tier-0 RFQ discount = 0%). |
| `testV2GO_RfqTradeEmitsFeeChargedV2WithFlowKindOne` | Both `FeeChargedV2` legs of an RFQ trade carry `productKind=OPTION` AND `flowKind=RFQ` (= 1) in their data payload. Pins the indexer-side `flow_kind="rfq"` decode V2G-N tested. |
| `testV2GO_RfqTier4MakerRebatePreservedThroughMarginEngine` | Tier 4 maker RFQ trade: rebate amount = `floor(premium × 50 / 1e6) = 5000`; rebate budget decreases by exactly 5000; one `FeeRebatedV2` + one `FeeChargedV2`. Design-Option-A negative-ppm preservation pinned through the full MarginEngine path. |
| `testV2GO_RfqTier2TakerLegPicksUpDiscountedFee` | Tier 2 RFQ taker pays `ceil(premium × 94 / 1e6) = 9400` (25% RFQ discount on 125 ppm) instead of 12500. Maker rebate stays at -10 ppm = 1000. Exercises the canonical V2G-N RFQ table row. |
| `testV2GO_RfqApplyTradeRequiresAuthorizedMatchingEngine` | Direct (non-pranked) `applyRfqTrade` call reverts with `NotAuthorized` — same gate as `applyTrade`. |

All 6 pass. `forge test --match-contract FeesManagerV2Test` (V2G-N suite) and `forge test --match-contract MarginEngineTest` both stay green.

## Backend tests

V2G-O does not touch backend code. The V2G-N test `v2g_n_indexer_decodes_option_rfq_flow_kind_verbatim` already pins that the indexer surfaces `flow_kind="rfq"` verbatim. Once the V2G-P redeploy lands and the first real RFQ trade fires, the existing `/admin/fees/onchain` payloads carry `flow_kind="rfq"` with no further backend change.

**V2G-P0 follow-up (2026-06-01).** The backend signing library has
now been extended with the RFQ surface — `OPTION_RFQ_TRADE_TYPE`,
`option_rfq_trade_typehash`, `option_rfq_trade_digest`,
`encode_option_execute_rfq_trade_calldata`, and a dedicated
`executeRfqTrade` `alloy_sol_types` definition in
`src/options/execution.rs`. The on-chain `RFQ_TRADE_TYPEHASH`
`0x6c660d979559d8526032a642d665ecefe15ca18cf062c24b6cd36058f98a123b`
is pinned by a backend unit test so a future contract-side change
breaks the test immediately. See
`docs/OPTION_RFQ_LIVE_DEPLOYMENT_PREFLIGHT_V2G_P0.md`.

**V2G-P1 follow-up (2026-06-01).** Operator-facing artifacts now shipped:
- `script/PreflightOptionRfqEntryPoints.s.sol` — read-only bytecode-scan selector probe.
- `script/SmokeOptionRfqV2Fees.s.sol` — read-only RFQ-fee preflight asserting `FlowKind.RFQ` on both quote legs.
- `script/SmokeOptionRfqV2FeesExecute.s.sol` — scaffold producing an offline-signable EIP-712 digest under a confirm gate; does NOT broadcast.
- `src/options/rfq_operator_packet.rs` — safe-by-default packet generator with explicit `OPTION_RFQ_OPERATOR_BROADCAST_CONFIRM` gate.
See `docs/OPTION_RFQ_OPERATOR_PACKET_V2G_P1.md`.

## Validation

Solidity:

| Command | Result |
|---|---|
| `forge fmt` | clean |
| `forge fmt --check` | ✅ |
| `forge build` | ✅ (warnings: pre-existing lint notes only) |
| `forge test --no-match-path 'test/fork/*' --match-contract MarginEngineTest` | ✅ |
| `forge test --no-match-path 'test/fork/*' --match-contract FeesManagerV2Test` | ✅ (24 tests — V2G-N + base, unaffected) |
| `forge test --no-match-path 'test/fork/*'` | ✅ (full suite, +6 V2G-O tests on top of V2G-N's +8) |

Backend / Frontend: untouched in V2G-O scope.

## Monitoring soak preservation

| Check | State at V2G-O close |
|---|---|
| Backend PID 56199 still alive on `0.0.0.0:8080` | ✅ |
| `/health` returns `{"ok":true,...}` | ✅ |
| Prometheus `/-/healthy` | ✅ |
| Compose 4/4 containers up | ✅ (15h uptime carried through V2G-N + V2G-O) |
| V2 fee baseline metrics unchanged | ✅ (`{consumer="new"}`: PERP charged=3 / rebated=1, OPTION charged=3 / rebated=1; mUSDC budget=999987) |

No `docker compose down`, no Prometheus reset, no backend restart, no `.env` edit.

## Remaining blockers

1. **V2G-P contract redeploy + governance touch.** The V2G-O bytecode sits in `out/`. Until the operator broadcasts a redeploy of `MarginEngine` + `OptionMatchingEngine` (or runs an upgrade path through governance), the live chain still serves V2G-G-era bytecode. `executeRfqTrade` against the live contract reverts at the `IMarginEngineRfqTrade(address(marginEngine)).applyRfqTrade` cast — by design.
2. **Backend restart for V2G-M endpoint pickup** still queued (carried over).
3. **Canonical V2G-K day-1 24h gate** still reserved for `2026-06-01T17:38Z`.
4. **Frontend signing CLI extension.** The V2G-D2 signing CLIs (`sign_option_execution_intent`) currently sign `TRADE_TYPEHASH` only. After V2G-P, the operator will need an `--rfq` flag (or a sibling `sign_option_execution_rfq_intent` CLI) to sign `RFQ_TRADE_TYPEHASH`. Queued for V2G-P operator-tooling follow-up.

## Next recommended milestone

**V2G-P — operator broadcasts the V2G-O redeploy and exercises the live RFQ path.**

- Cut the V2G-O bytecode under governance: deploy a new MarginEngine + OptionMatchingEngine with the V2G-O code, swap the V2G-K/L-era addresses for the new ones via the matching-engine setter, run a tiny mainnet-shaped smoke (or testnet OPTION RFQ trade) through `executeRfqTrade`.
- Confirm the first `FeeChargedV2` event with `flowKind=1` surfaces in `/admin/fees/onchain` and the V2G-G Grafana dashboard.
- Extend the V2G-D2 signing flow to produce `OptionRfqTrade`-typehash signatures (operator CLI flag).
- Tick the V2G-K day-N row with the RFQ-live observation.
- File V2G-P closure note.
