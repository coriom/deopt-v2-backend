# V2G-N — OPTION RFQ Fee Discounts

## Status

- Milestone: **V2G-N** — closes the OPTION RFQ fee-discount audit gap
  carried over from `FEE_MODEL_TARGET_GAP_ANALYSIS_V2C.md` §"RFQ
  discount support" and §"V2G — Staking / Volume Share / RFQ discount
  integration".
- Date: 2026-05-31.
- Outcome:
  - **No code change to deployed Solidity.** The math + state +
    default schedule are already correctly implemented in
    `FeesManagerV2`. V2G-C documented this as a backend-only gap;
    V2G-N pins that the contract layer is also complete.
  - **8 new Solidity tests** in
    `test/fees/FeesManagerV2.t.sol` exhaustively cover the canonical
    OPTION RFQ table for all 5 tiers + the Tier 4 100% discount edge
    case + the Tier 0 RFQ==ORDERBOOK invariant + PERP RFQ unchanged.
  - **1 new backend decode test** in
    `src/fees/onchain_summary.rs` pins that the V2 fee event indexer
    decodes `flowKind="rfq"` verbatim without any enum re-mapping.
  - **Design decision recorded** (Option A — discount applies only to
    positive ppm; negative maker rebates are preserved unchanged even
    at 100% discount).
  - **Integration gap** documented but not closed (MarginEngine
    currently hardcodes `FlowKind.ORDERBOOK`; rewiring that is a
    deployed-contract concern and is queued as the V2G-O follow-up).
- Hard gates respected: no broadcast, no chain mutation, no DB
  writes, no real `.env` edit, no private-key handling, no
  governance/timelock action, no `docker compose down -v`, no
  Prometheus reset, no backend restart, no soak interruption.

## Current RFQ implementation status

| Layer | State | Source-of-truth file |
|-------|-------|----------------------|
| `IFeesManagerV2.FlowKind` enum (`ORDERBOOK`, `RFQ`) | **Present** | `src/fees/IFeesManagerV2.sol` |
| `RfqDiscountProfile` struct + getter / setter | **Present** | `src/fees/IFeesManagerV2.sol`, `src/fees/FeesManagerV2.sol::{getRfqDiscountProfile, setRfqDiscountProfile}` |
| `_rfqDiscounts` storage mapping | **Present** | `src/fees/FeesManagerV2.sol::_rfqDiscounts` |
| `_effectiveRatePpm` discount math | **Correct (Option A)** | `src/fees/FeesManagerV2.sol::_effectiveRatePpm` lines 373-399 |
| Default OPTION launch schedule | **Matches canonical table** | `src/fees/FeesManagerV2.sol::_installLaunchSchedules` lines 449-453 |
| Default PERP launch schedule | RFQ discounts = 0% for all tiers (interface-supported, no fee effect today) | `src/fees/FeesManagerV2.sol::_installLaunchSchedules` lines 461-463 |
| `quoteFees(..., FlowKind.RFQ, ...)` end-to-end | **Working** (V2G-N tests pin this) | `test/fees/FeesManagerV2.t.sol::testV2GN_*` |
| MarginEngine integration | **Stub-only — always passes `FlowKind.ORDERBOOK`** | `src/margin/MarginEngineTrading.sol` line 90 |
| PerpEngine integration | **Stub-only — always passes `FlowKind.ORDERBOOK`** | `src/perp/PerpEngineTrading.sol` line 320 |
| Backend event indexer decode | **RFQ-ready** (parses `flowKind` as free-form string) | `src/fees/onchain_summary.rs::normalize_v2_*` |
| Backend admin endpoint `/admin/fees/onchain` | **RFQ-ready** (renders `flow_kind` verbatim) | `src/fees/onchain_summary.rs::SummaryAggregator` |
| V2G-G metric pipeline | **No change needed** (consumer label only, not flow label) | `src/monitoring.rs::append_*_fee_v2_consumer_metric` |
| Frontend admin tile | Renders `flow_kind` verbatim from the existing JSON | `src/app/admin/admin-dashboard.tsx` (no V2G-N change) |

Bottom line: **math + state + decode are fully ready.** What's
missing for live OPTION RFQ fee discounts to take effect on chain is
the MarginEngine integration (passing the actual RFQ flag through),
which requires a contract redeploy + governance touch — explicitly
out of V2G-N scope per the hard rules.

## Canonical OPTION RFQ fee table

Pinned in both `FeesManagerV2._installLaunchSchedules` and the V2G-N
test table (`testV2GN_*`).

| Tier | maker ppm (ORDERBOOK) | taker ppm (ORDERBOOK) | RFQ maker discount | RFQ taker discount | RFQ maker effective ppm | RFQ taker effective ppm |
|------|----------------------:|----------------------:|--------------------:|--------------------:|------------------------:|------------------------:|
| 4    | -50 (rebate)          | 75                    | 100% (1_000_000 ppm) | 75% (750_000 ppm)  | **-50** (preserved)     | **19** (ceil(75·0.25))  |
| 3    | -25 (rebate)          | 100                   | 75% (750_000 ppm)    | 50% (500_000 ppm)  | **-25** (preserved)     | **50**                  |
| 2    | -10 (rebate)          | 125                   | 50% (500_000 ppm)    | 25% (250_000 ppm)  | **-10** (preserved)     | **94** (ceil(125·0.75)) |
| 1    | 0                     | 150                   | 25% (250_000 ppm)    | 10% (100_000 ppm)  | **0** (no-op)            | **135**                 |
| 0    | 50                    | 250                   | 0%                   | 0%                  | **50** (== ORDERBOOK)    | **250** (== ORDERBOOK)  |

Mapping to the operator-facing canonical schedule:

| Operator notation | Canonical OPTION RFQ table |
|-------------------|----------------------------|
| `-0.005%` | maker ppm = -50 |
| `-0.0025%` | maker ppm = -25 |
| `-0.001%` | maker ppm = -10 |
| `0.000%` | maker ppm = 0 |
| `0.005%` | maker ppm = 50 |
| `0.0075%` | taker ppm = 75 |
| `0.010%` | taker ppm = 100 |
| `0.0125%` | taker ppm = 125 |
| `0.015%` | taker ppm = 150 |
| `0.025%` | taker ppm = 250 |
| `100% / 75% / 50% / 25% / 0%` (maker) | `1_000_000 / 750_000 / 500_000 / 250_000 / 0` ppm |
| `75% / 50% / 25% / 10% / 0%` (taker) | `750_000 / 500_000 / 250_000 / 100_000 / 0` ppm |

All values match the contract default install.

## Design decision: discounting negative maker rebates

**Chosen: Option A — discount only applies when `ratePpm > 0`.**

The contract's `_effectiveRatePpm` short-circuits on `ratePpm <= 0`
(lines 381-383):

```solidity
if (flow != FlowKind.RFQ || ratePpm <= 0) {
    return ratePpm;
}
```

Comparison of the three options the task surfaced:

| Option | Behaviour on a Tier 4 maker (-50 ppm) under RFQ | Net effect | Accounting risk |
|--------|--------------------------------------------------|-----------|-----------------|
| **A** (chosen) | Maker rebate stays at -50 ppm | Rebate magnitude unchanged | None — rebate budget burn rate identical to ORDERBOOK; protocol P&L unchanged |
| B | Discount applied to absolute value of maker ppm (50 ppm × `(1 - 1.0)` = 0 ppm) | Rebate floored to zero | Hidden rebate cancellation; UI / docs misalign with actual maker compensation |
| C | Maker ppm forced to 0 regardless of sign before rebate compute | Rebate = 0 always under RFQ for Tier 4 | Same as B; additionally surprising for operators who set `makerDiscount = 1_000_000` expecting amplification |

Option A is the safest:

- Symmetry with ORDERBOOK: a maker who would receive a -50 ppm rebate
  on the orderbook still receives -50 ppm on RFQ. RFQ-vs-orderbook
  selection by the maker doesn't accidentally cut their rebate.
- Rebate-budget accounting: the `rebateBudget` accumulator's burn rate
  is identical to ORDERBOOK, so the `FeesManagerV2RebateBudgetLow`
  alert threshold logic (V2G-G) doesn't need an RFQ-specific arm.
- Predictable Tier 4 economics: at 100% maker discount, the maker
  still gets -50 ppm rebate. Discount knob is interpreted as "reduce
  the fee" (positive direction) — not "amplify the rebate" (negative
  direction).
- Operator UI: `quoteFees` returns the **same** `appliedPpm` for both
  ORDERBOOK and RFQ on the maker rebate side. V2G-G metrics + V2G-M
  smoke readiness packets surface this without special-casing.

This is what `FeesManagerV2` implements today, and what the V2G-N
test suite pins as the invariant.

## Tests added

### Solidity (`test/fees/FeesManagerV2.t.sol`)

| Test | What it asserts |
|------|------------------|
| `testV2GN_OptionRfqTakerTableWalk` | Per-tier OPTION RFQ taker effective ppm matches the canonical table (250 / 135 / 94 / 50 / 19). ORDERBOOK taker ppm also matches per tier. |
| `testV2GN_OptionRfqMakerPreservesNegativeRebatesEvenAtHundredPercentDiscount` | Per-tier OPTION RFQ maker ppm matches ORDERBOOK ppm for every tier (negative rebates pass through). `isRebate` flag pinned per tier. |
| `testV2GN_OptionRfqTier4HundredPercentMakerDiscountKeepsRebateUnchanged` | Tier 4 OPTION RFQ maker stays at -50 ppm / rebateAmount = 50 native units; taker = 19 ppm / feeAmount = 19. Explicit Design-Option-A pin. |
| `testV2GN_OptionRfqTier0EqualsOrderbookForBothLegs` | Tier 0 RFQ == ORDERBOOK on both maker + taker (basis 12_345 chosen to exercise the rounding paths). |
| `testV2GN_PerpRfqUnaffectedAtEveryTierForBothLegs` | PERP RFQ ppm == ORDERBOOK ppm at every tier on both legs (PERP discount = 0%). Future-proofs against accidental PERP RFQ activation. |
| `testV2GN_OptionOrderbookUnchangedForEveryTier` | Per-tier OPTION ORDERBOOK ppm pinned (50 / 0 / -10 / -25 / -50 maker; 250 / 150 / 125 / 100 / 75 taker). Regression-pin for V2G-N additivity. |
| `testV2GN_RfqDiscountIgnoresNegativeOrZeroPpm` | Even with a 100% maker discount installed on Tier 0 and the Tier 0 profile flipped to negative maker ppm, the maker leg stays at the negative ppm. Direct invariant pin for `ratePpm <= 0` short-circuit. |
| `testV2GN_RfqDiscountSetterRejectsOverflow` | `setRfqDiscountProfile` reverts with `InvalidDiscount` when either leg discount > `PPM_DENOMINATOR`. |

All 8 pass. Total Solidity test count: existing → existing + 8 (V2G-E
208-test baseline + V2G-N 8 = 216 expected after `forge test --no-match-path 'test/fork/*'`).

### Backend (`src/fees/onchain_summary.rs`)

| Test | What it asserts |
|------|------------------|
| `v2g_n_indexer_decodes_option_rfq_flow_kind_verbatim` | Synthetic OPTION RFQ trade (charged + rebated legs) decodes through `normalize_fee_events` → `aggregate` with `flow_kind="rfq"` preserved verbatim on every per-event payload. Aggregator counts the legs correctly (`charged=94, rebated=10, net=84`). |

Backend test count: 694 → **695** (V2G-M kept us at 694; V2G-N adds 1).

### Frontend

No frontend change. The V2G-M admin tile already renders `flow_kind`
verbatim from the underlying admin endpoint. When real RFQ events
land, the tile shows them without a code change.

## Files changed

Solidity:

- `test/fees/FeesManagerV2.t.sol` — V2G-N test block appended (8 tests,
  ~190 lines including comments).

Backend:

- `src/fees/onchain_summary.rs` — V2G-N RFQ decode-readiness test
  appended.

Docs:

- **NEW** `docs/OPTION_RFQ_FEE_DISCOUNTS_V2G_N.md` (this file).
- `docs/FEE_MODEL_TARGET_GAP_ANALYSIS_V2C.md` — V2G-N closure note
  pointing here (RFQ discount support row updated to ✅ on the
  Solidity side, with MarginEngine integration explicitly carried
  to V2G-O).

Sol / Frontend: no production source changes (Solidity tests only).

## Validation

| Command | Result |
|---|---|
| `forge fmt` | clean (no diff) |
| `forge fmt --check` | ✅ |
| `forge build` | ✅ |
| `forge test --no-match-path 'test/fork/*'` | ✅ (all tests, including the 8 new V2G-N + the existing FeesManagerV2 suite) |
| `cargo fmt --all -- --check` | ✅ |
| `cargo clippy --all-targets --all-features -- -D warnings` | ✅ |
| `cargo build --all-targets --all-features` | ✅ |
| `cargo test --all-targets --all-features --no-fail-fast` | ✅ **695 / 0 / 0** |
| Soak: `/health`, `/-/healthy`, 4/4 compose containers running | ✅ throughout |

## Soak stack status

PID 56199 + the 4-container compose stack remained healthy across
every `forge` / `cargo` invocation. The soak record at
`docs/V2_FEE_OBSERVABILITY_7_DAY_SOAK_V2G_K.md` does **not** get a
new soak day ticked — V2G-N is unrelated to the canonical day-1 24h
gate (still reserved for `2026-06-01T17:38Z`). The backend process
was not stopped or restarted.

## Remaining blockers

1. **MarginEngine integration gap.** `MarginEngineTrading.sol`
   hardcodes `IFeesManagerV2.FlowKind.ORDERBOOK`. Even though the
   discount math is wired and tested, OPTION RFQ trades that flow
   through the MarginEngine today still bill the ORDERBOOK ppm. To
   actually deliver RFQ discounts on chain, `MarginEngineTrading`
   needs an `isRfq` bool plumbed through `applyTrade` → `consumeFees`.
   This is a **contract redeploy + governance touch** and is queued
   as V2G-O.
2. **PerpEngine integration gap.** Same shape as MarginEngine.
   PERP RFQ default discount is 0% so the on-chain billing is
   unchanged, but the structural plumbing is missing for any future
   PERP RFQ rate to ever take effect.
3. **V2G-M endpoint pickup requires backend restart.** Carried over
   from V2G-M — the live PID 56199 is the V2G-G era binary; the
   V2G-N test additions don't change that.
4. **Canonical V2G-K day-1 24h gate** still reserved for
   `2026-06-01T17:38Z`.

## Next recommended milestone

**V2G-O — wire `FlowKind.RFQ` through the MarginEngine + OptionMatchingEngine path so OPTION RFQ discounts take effect on chain.**

- Add an `isRfq` bool to `OptionMatchingEngine.executeTrade` (or
  reuse `OptionTrade.flowKind`).
- Plumb the bool to `MarginEngine.applyTrade(..., isRfq)`.
- Translate to `IFeesManagerV2.FlowKind.RFQ` at the
  `MarginEngineTrading::consumeFees` call site.
- Reuse the V2G-N test table as the integration-test reference
  (apply via a smoke against the V2G-D2 EOAs).
- Cut the redeploy under governance with an audit-light review note
  (no new fee math is introduced — only a flow flag is passed).
- After the redeploy lands, re-run the V2G-J synthetic drill to
  confirm Alertmanager routing of any new RFQ-specific anomaly.
