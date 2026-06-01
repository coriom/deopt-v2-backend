# Fee Model Target Gap Analysis V2C

Date: 2026-05-24

## Purpose

V2C ships two backend-only deliverables that bracket the fee work:

1. **On-chain fee event reconciliation** — backend now treats the indexed
   `TradingFeeCharged` log as the source of truth for live trading fees
   and reports it through the lifecycle endpoint and a new
   `GET /admin/fees/onchain` admin endpoint. The backend fee ledger is
   surfaced as an *explicit* status, not a failure: a disabled or empty
   ledger is reported as such while the on-chain summary still resolves.
2. **Gap analysis** (this document) — comparison between the current
   live Solidity `FeesManager` plus the backend fee ledger and the
   target schedule proposed by the operator, with a concrete migration
   plan split into V2D/V2E/V2F/V2G.

V2C does not touch Solidity, the frontend, deployment, executor signing,
broadcast, real `.env` secrets, or live fee rates. No
`option_execution_intents`, `option_execution_transactions`,
`execution_transactions`, or evidence rows are mutated by the new
admin surface.

## Live Anchor: V1S On-Chain Fees

| Field | Value |
| --- | --- |
| tx hash | `0x5964a7b3d2c18d051baaa780413d31c44d419ce530f45263cb4c46f720881125` |
| intent | `e6d2941b-65f7-413a-958f-74ab22c53b08` |
| `TradingFeeCharged` event count | `2` |
| buyer fee (taker, `appliedFee`) | `6` |
| seller fee (maker, `appliedFee`) | `4` |
| observed total | `10` |
| recipient | `0x009f38…` |

The on-chain summary, the admin endpoint, and the lifecycle
`fees.observed_total = "10"` all agree.

## Current Solidity Model

Audited contracts (read-only):

- `src/fees/FeesManager.sol`
- `src/fees/IFeesManager.sol`
- `src/margin/MarginEngineTypes.sol` (`TradingFeeCharged` definition)
- `src/margin/MarginEngineTrading.sol` (`_chargeTradingFee`)
- `src/margin/MarginEngineAdmin.sol` (`setFeeRecipient` /
  `clearFeeRecipient`)
- `src/perp/PerpEngineStorage.sol` (`feeRecipient`,
  `_resolvedFeeRecipient`)
- `src/perp/PerpEngineAdmin.sol` (admin setters)
- `src/perp/PerpEngineTrading.sol` (charges trading fee using the same
  `IFeesManager` interface)
- `src/perp/PerpEngineTypes.sol` (`event FeeRecipientSet`)
- `src/ProtocolConstants.sol` (`BPS = 10_000`)

### Precision

- All bps fields are `uint16` and divided by `BPS = 10_000`.
- The minimum representable rate is therefore **1 bps = 0.01%**.
- The maximum field value before the `feeBpsCap` guard is `10_000 bps =
  100%`.
- Sub-bps rates **cannot** be represented. The current code explicitly
  floors `0.5 bps` and `1.5 bps` to integer values in
  `FeesManager.getTierClassProfile`:
  - Tier1 maker `0.005% (0.5 bps)` → `0 bps` (floored).
  - Tier2 taker `0.015% (1.5 bps)` → `1 bps` (floored).
- The hybrid formula
  `fee = min(notionalImplicit * notionalFeeBps / BPS, premium * premiumCapBps / BPS)`
  inherits the same integer-bps floor.

### Tier model

- Enum `VolumeTierClass { Tier0, Tier1, Tier2 }` — **three** coarse
  classes, not five.
- Effective rate priority: active **override** > active **claimed
  tier** > **defaults**.
- Tier assignment is a Merkle-proof claim against
  `merkleRoot` for the current `epoch`. The operator pushes a root
  off-chain (`setMerkleRoot`) and traders self-claim via
  `claimTier(address trader, VolumeTierClass tierClass, uint64 expiry,
  bytes32[] proof)`. There is no on-chain volume oracle.

### Maker / taker support

- `FeeProfile { FeeParams maker; FeeParams taker; }` is per-trader.
- Maker vs. taker is selected by the engine when it calls
  `getFeeParams(trader, isMaker)`. `MarginEngineTrading._chargeTradingFee`
  passes the role; `PerpEngineTrading` does the same.

### Rebate support

- **None.** All `FeeParams` fields are `uint16` (unsigned) — the
  contract cannot represent a negative maker rate. There is no rebate
  accrual, no rebate event, and no rebate sink in `FeesManager` or in
  either engine's `_charge*Fee` path.
- `MarginEngineTrading._chargeTradingFee` returns early if `appliedFee
  == 0`; it does **not** distribute anything to the maker.

### Premium cap

- `premiumCapBps` is also `uint16` integer bps, so the cap inherits
  the same sub-bps limitation.
- Caller-side `option_capped_amount_1e8` (backend) supports finer
  precision (`micro_bps`), but only as an off-chain preview — the
  on-chain `quoteFee` is what mints the `TradingFeeCharged` event.

### Volume tier mechanism

- Merkle claim only. Eligibility is whatever the off-chain attester
  signs into the root. There is no on-chain 28D volume tracker, no
  volume-share metric, and no DEOPT-stake gate.

### Staking support

- **None on-chain.** `FeesManager` has no reference to a staking
  contract, DEOPT balance, or veToken view. Staking-based eligibility
  must be folded into the Merkle root by the off-chain attester or
  added via a new on-chain view.

### RFQ discount support

- **None on-chain.** RFQ vs. orderbook is not a parameter of any
  `quoteFee` call; the same `FeeProfile` is applied. The backend
  already discounts RFQ rates in `fees::schedule::resolve_rates_from_volume`
  via `discount_positive_fee(rate, rfq_*_discount_pct)`, but that
  preview is off-chain only.

### Perps gaps

- `PerpEngineStorage.feeRecipient` exists and `_resolvedFeeRecipient`
  falls back to `insuranceFund`, but **`PerpEngineAdmin` has no
  `setFeeRecipient` setter** (greps in `src/perp/PerpEngineAdmin.sol`
  return zero hits). The event `FeeRecipientSet` is declared in
  `PerpEngineTypes.sol` but is never emitted from the perp admin
  surface. The only way to set `perp.feeRecipient` today is through a
  storage-pointer or migration; there is no public ABI entry point.
  `MarginEngineAdmin` does have `setFeeRecipient` / `clearFeeRecipient`
  / `FeeRecipientSet`.

## Backend (Current)

Files audited:

- `src/fees/types.rs`
- `src/fees/schedule.rs`
- `src/fees/service.rs`
- `src/fees/store.rs`
- `src/db/repository.rs` (`insert_fee_event`, `admin_recent_fee_events`,
  the new `admin_count_fee_events_by_source`)
- `src/options/lifecycle.rs` (`LifecycleFees`, `build_fees_view`)
- `src/options/event_indexer.rs`
  (`decode_trading_fee_charged_log` produces the decoded JSON consumed
  by V2C)

### Precision

- `FeeTier.maker_fee_micro_bps`, `maker_rebate_micro_bps`,
  `taker_fee_micro_bps` are `u64` **micro-bps** where
  `MICRO_BPS_PER_BPS = 10_000` and `RATE_DENOMINATOR = 100_000_000`.
- Smallest representable rate: `1 micro_bps = 0.0001%`.
- Backend already supports the target rates (0.0075%, -0.005%, etc.)
  in the preview path — see `fees::schedule::launch_fee_schedule`.

### Tier model

- Backend already encodes the target **five tiers** in both
  `launch_fee_schedule().option` and `.perp` via
  `FeeTier { tier, min_28d_volume_1e8, min_volume_share_micro_bps,
  min_staked_deopt_1e8, maker_fee_micro_bps, maker_rebate_micro_bps,
  taker_fee_micro_bps, rfq_maker_discount_pct, rfq_taker_discount_pct }`.
- `resolve_rates_from_volume` currently only consults
  `rolling_volume_1e8` (28D volume); the volume-share and
  staked-DEOPT branches of the OR-clause are defined in the tier
  struct but not consulted.

### Maker / taker / rebate

- `FeeParticipantRole::{Maker, Taker}` is used to choose between
  `maker_fee_micro_bps` and `taker_fee_micro_bps`.
- Maker rebates (`rebate_rate_micro_bps`, `rebate_amount_1e8`) are
  tracked alongside fees and gated by:
  - `state.fees_config.rebates_enabled`
  - `state.mm_permissions_config.enabled`
  - The maker being a permissioned MM with the relevant
    `can_*` capability.
  - The maker’s rebate is funded by the protocol fee column in the
    backend ledger today (no taker-funded rebate accounting).

### Volume / staking / share / RFQ discount

- 28D rolling **volume** bucket: implemented in `VolumeBucket` and
  `fee_rolling_volume_since` (in-memory and SQL).
- 28D **volume share**: schedule field exists
  (`min_volume_share_micro_bps`), no totals tracker yet — would need
  a `protocol_total_volume_1e8` aggregate to compute share.
- **Staked DEOPT**: schedule field exists
  (`min_staked_deopt_1e8`), no balance source wired. Would need either
  an RPC view on a future staking contract or a backend snapshot table.
- **RFQ discount**: applied in `discount_positive_fee` and visible in
  the preview path. The on-chain `quoteFee` does not see it, so a live
  RFQ trade currently bills the same rate as an orderbook trade
  whenever the operator routes it through `MarginEngine.applyTrade`.

### On-chain reconciliation (new in V2C)

- Lifecycle endpoint `fees` block now ships:
  - `source_of_truth = "onchain"`
  - `trading_fee_event_count`
  - `observed_total` (sum of `appliedFee` per indexed log)
  - `by_trader`, `by_recipient`, `by_side`
  - `total_by_recipient` (legacy field retained for backward-compat)
  - `backend_ledger_status ∈ {"disabled", "missing_or_disabled",
    "present"}` (explicit; never a failure)
  - `reconciliation_status ∈ {"onchain_observed", "no_onchain_events"}`
- New endpoint `GET /admin/fees/onchain?tx_hash=…&limit=…` returns the
  same shape per tx plus an aggregate over the filtered set. Existing
  `GET /admin/fees/summary`, `/admin/fees/events`,
  `/admin/fees/volumes`, `/admin/fees/rebates` endpoints are
  unchanged.

## Target Model Requirements

| Requirement | Status |
| --- | --- |
| Five tiers | Backend ✅ (schedule), Solidity ❌ (three) |
| OR-eligibility (28D volume \| 28D share \| staked DEOPT) | Backend ⚠️ (fields ready, only volume gates today); Solidity ❌ (Merkle claim only) |
| Sub-bps precision (0.0075%, 0.0125%) | Backend ✅ (micro-bps), Solidity ❌ (uint16 integer bps) |
| Negative maker rebates (-0.005%, -0.0025%, -0.001%, -0.005%, -0.0075%, -0.01%) | Backend ⚠️ (positive rebate accruals exist; signed maker fee does not), Solidity ❌ (unsigned `uint16` cannot encode negative) |
| Options maker/taker fees | Both ✅ |
| Perps maker/taker fees | Both ✅ |
| RFQ maker/taker discounts (100%/75%/50%/25%/10%/0%) | Backend ✅ (preview + decode), Solidity ✅ math/state/default schedule per V2G-N; **MarginEngine integration ❌** (hardcodes `FlowKind.ORDERBOOK`, queued for V2G-O). See `docs/OPTION_RFQ_FEE_DISCOUNTS_V2G_N.md`. |
| Fee recipient (margin engine) | Both ✅ (`setFeeRecipient` + `FeeRecipientSet`) |
| Fee recipient (perp engine) | Storage ✅ but admin setter ❌ — see "Perps gaps" |
| Backend preview ↔ on-chain alignment | Documented in V2C; preview can differ from on-chain when off-chain rates are sub-bps |
| On-chain `TradingFeeCharged` as source of truth | ✅ V2C reconciler + lifecycle + admin endpoint |

### Mapping target rates to ppm

We standardise on **signed ppm** with `1 ppm = 0.0001%`. The full
target schedule lands inside `int32` (range ±100 000 ppm = ±10%):

Options:

| Tier | maker (%) | maker (ppm) | taker (%) | taker (ppm) | rfq maker disc. | rfq taker disc. |
| --- | --- | --- | --- | --- | --- | --- |
| 4 (≥$25M / ≥5% / ≥250k) | -0.005% | -50 | 0.0075% | 75 | 100% | 75% |
| 3 (≥$10M / ≥2.5% / ≥100k) | -0.0025% | -25 | 0.010% | 100 | 75% | 50% |
| 2 (≥$2.5M / ≥1% / ≥50k) | -0.001% | -10 | 0.0125% | 125 | 50% | 25% |
| 1 (≥$500k / ≥0.25% / ≥10k) | 0.000% | 0 | 0.015% | 150 | 25% | 10% |
| 0 (else) | 0.005% | 50 | 0.025% | 250 | 0% | 0% |

Perps:

| Tier | maker (%) | maker (ppm) | taker (%) | taker (ppm) |
| --- | --- | --- | --- | --- |
| 4 | -0.010% | -100 | 0.015% | 150 |
| 3 | -0.0075% | -75 | 0.0175% | 175 |
| 2 | -0.005% | -50 | 0.020% | 200 |
| 1 | 0.000% | 0 | 0.025% | 250 |
| 0 | 0.005% | 50 | 0.030% | 300 |

Every rate is at most 3 decimal places of bps → `int32` ppm is
sufficient. `int64` ppm gives ±9.2e12 ppm = ±9.2e8 % of headroom which
is comically over-budget; we recommend `int32` ppm for storage and
`int64` for intermediate arithmetic.

## Precision Recommendation

Adopt **signed ppm** (`int32` storage, `int64` arithmetic), where
`1 ppm = 0.0001%`. Why not the alternatives:

- **`uint16` bps**: insufficient — cannot encode `0.5 bps` or
  `1.25 bps`, cannot encode rebates.
- **`int64` 1e8 rate** (`fee_rate / 1e8`): equally expressive but
  burns three orders of magnitude of storage and makes the encoded
  rate harder to read at the audit level. The backend already mixes
  `1e8` for sizes and prices; reusing it for rates risks conflating
  rate-scale and notional-scale fields.
- **`int32` ppm**: maps 1:1 to the target schedule, leaves enough
  range to absorb a future cap raise (cap could be `±100_000 ppm =
  ±10%` and still fit), and matches the unit operators already use to
  reason about exchange-style fees.

### Fee formula (V2D Solidity)

```solidity
// rates in signed ppm (1 ppm = 1e-6 = 0.0001%)
// PPM_DENOMINATOR = 1_000_000
int256 notionalFee = int256(notionalImplicit) * notionalFeePpm / int256(PPM_DENOMINATOR);
int256 premiumCap  = int256(premium)         * premiumCapPpm  / int256(PPM_DENOMINATOR);
// signed min: negative makers get a negative appliedFee (= rebate);
// the engine debits the recipient (or rebate sink) and credits the trader.
int256 appliedFee  = _signedMin(notionalFee, premiumCap);
```

The existing backend `RATE_DENOMINATOR = 100_000_000` already encodes
the same dimensionless rate ratio (`micro_bps / 1e8 = ppm * 1e2 / 1e8 =
ppm * 1e-6`), so the V2E preview can keep its current internal scale
and only needs a `ppm <-> micro_bps` thin conversion at the I/O
boundary.

## Rebate Funding Model

Mandatory decisions for V2D before any negative rate ships on-chain:

1. **Source of funds.** Maker rebates **must be funded by taker fees
   on the same trade** as the default policy. When the trade is
   maker-only (`takerFeePpm == 0`), the rebate must draw from a
   bounded incentive budget held by the protocol treasury — *never*
   from the insurance fund.
2. **Per-trade cap.** `|appliedRebate| <= appliedTakerFee +
   incentiveBudgetPerTrade`. If the cap binds, the rebate is reduced
   (not the taker fee) so the trade still nets a non-negative
   protocol revenue.
3. **Per-epoch incentive budget.** `MAX_REBATE_BUDGET_PER_EPOCH` is a
   storage value set by the owner with an `IncentiveBudgetSet` event;
   each rebate decrements `epochRebateBudgetUsed`. When the budget is
   exhausted, the rate floors at `0 ppm` for the rest of the epoch
   and the contract emits an `IncentiveBudgetExhausted` event.
4. **Insurance fund is forbidden as a rebate sink.** Settlement
   shortfall remains the only insurance-fund draw path.
5. **Event model.**
   ```solidity
   event TradingRebateCredited(
       address indexed trader,
       address indexed funder,            // taker, or treasury
       address indexed settlementAsset,
       uint256 optionId,
       int256  appliedRebate,             // always negative
       uint256 fundedFromTakerFee,
       uint256 fundedFromIncentiveBudget
   );
   ```
   The existing `TradingFeeCharged` event keeps its current ABI for
   positive fees; rebates surface through the new event so historical
   indexers can keep working without re-decoding.

The V2C reconciler already exposes `by_side` totals which makes the
post-V2D audit trivial: `by_side["taker"] - by_side["maker"]` is the
expected funding gap when rebates ship.

## Recommended Implementation Plan

### V2D — `FeesManagerV2` Solidity precision and tier model

- Replace `FeeParams { uint16 notionalFeeBps; uint16 premiumCapBps }`
  with `FeeParams { int32 notionalFeePpm; int32 premiumCapPpm }` and
  bump the implementation to a new `FeesManagerV2` contract behind
  the existing `IFeesManager` symbol (interface ABI change is
  unavoidable; minor revision bump).
- Expand `VolumeTierClass` from three to five tiers
  (`Tier0..Tier4`); keep enum-friendly ordering.
- Keep the existing Merkle-claim flow as the *attester-side*
  eligibility input: a trader's tier is whatever the most recent
  active claim says, regardless of whether it was earned by volume,
  share, or staking — the OR-logic happens off-chain at root
  computation time. (Pure on-chain volume/share/stake checks are
  V2G.)
- Add the `TradingRebateCredited` event in `MarginEngineTrading`
  and `PerpEngineTrading`; keep `TradingFeeCharged` for the
  taker leg.
- Add `setFeeRecipient` / `clearFeeRecipient` / emit
  `FeeRecipientSet` on `PerpEngineAdmin` (closes the perps setter
  gap).
- Add `feeBpsCap` analogue (`feePpmCap`) defaulting to `100_000` (=
  10%), apply on each set.
- Tests: hybrid formula sign behaviour, cap clamp, ownership
  transfer, claim flow at the five-tier enum, rebate event
  emission.

### V2E — Backend fee preview and drift checks

- Switch the backend rate I/O at the public boundary to **signed
  ppm**:
  - `LaunchFeeSchedule` JSON exports `*_ppm` (signed) instead of
    `_micro_bps` (unsigned) at the admin endpoints; keep internal
    `micro_bps` arithmetic intact, do the conversion in the
    serializer.
  - Add a `fees::schedule::PpmRate { ppm: i32 }` newtype to avoid
    accidental sign loss.
- Add `fees::preview::quote_trade_fee(payer, role, premium_1e8,
  notional_1e8)` that returns the same `FeeQuote` shape as Solidity
  V2D, so the executor can do a pre-broadcast drift check against
  the indexed `TradingFeeCharged` event after the trade lands.
- Extend `admin_onchain_fees` to compare each indexed event with the
  preview and surface a `drift_status` (`"match"`, `"backend_higher"`,
  `"backend_lower"`, `"no_preview"`).
- Persist the preview into `option_execution_intents.preview_fee_ppm`
  /`preview_fee_amount_1e8` once V2D rates ship, so V2E drift checks
  can be performed without rebuilding intent state.

### V2F — On-chain fee admin dashboard

- Frontend admin console consumes `GET /admin/fees/onchain` and
  renders:
  - Per-tx breakdown (`tx_hash`, `observed_total`, `by_trader`,
    `by_recipient`, `by_side`).
  - Drift indicator (powered by V2E).
  - Backend ledger status badge (using the V2C
    `backend_ledger_status` enum).
- Add `GET /admin/fees/onchain/by-intent/:intent_id` once the V2E
  preview lands so the dashboard can pivot from intent to fee
  evidence without manual `tx_hash` lookups.
- Optional cron metric: `fee_drift_ppm_p99` exposed under
  `/metrics`.

### V2G — Staking / Volume Share / RFQ discount integration

- Add an on-chain `IDeoptStaking.balanceOf(address)` adapter and
  store the address on `FeesManagerV2`. The OR-eligibility check
  becomes either fully on-chain (read three sources, max-tier them)
  or stays Merkle-only with the off-chain attester proving any of
  the three predicates.
- Add `IVolumeOracle.share28d(address)` for on-chain share
  enforcement. Pushing 28D rolling share onchain is non-trivial; the
  Merkle approach is cheaper and stays consistent with V2D.
- Wire RFQ discounts into Solidity by passing an `rfqMode` flag to
  `quoteFee` / `computeFee` (signature change → V2D-or-later
  contract bump), or by adding `quoteFeeRfq` and dispatching from
  `MarginEngineTrading._chargeTradingFee` based on the trade origin.
  Backend already encodes the discounts per tier so the on-chain
  side only needs the discount table and a `bool isRfq` argument.
- Migrate the V2E preview to consult the on-chain staking and volume
  views so backend and Solidity converge on the exact eligibility
  rule.

## Limitations

- V2C does not bridge backend ledger entries to on-chain fee events
  — it surfaces both side-by-side and lets the operator reconcile.
  V2E is the layer that will compute drift.
- Negative rates are recommended but not yet on-chain (`FeesManager`
  remains unsigned). A live rebate cannot ship until V2D lands.
- Volume-share and staked-DEOPT eligibility branches are declared in
  the backend schedule but only volume gating is evaluated at
  preview time.
- The V2C admin endpoint pages through indexed events; very high
  event-count tx hashes (thousands of logs) should be queried with
  an explicit `tx_hash` filter to keep the response bounded.
- Perps `feeRecipient` remains read-only via storage; V2D must
  expose the admin setter.

## Live V1S Fee Verification Result

Run date: 2026-05-25.

Boot config (env overrides — secrets never printed):

| Key | Value |
| --- | --- |
| `HOST` | `127.0.0.1` |
| `PORT` | `8080` |
| `PERSISTENCE_ENABLED` | `true` |
| `DATABASE_URL` | sourced from `.env` (not printed) |
| `ADMIN_API_ENABLED` | `true` |
| `ADMIN_API_REQUIRE_TOKEN` | `false` |
| `OPTION_EVENT_INDEXER_ENABLED` | `false` |
| `OPTION_CONFIRMATION_WORKER_ENABLED` | `false` |
| `OPTION_RECONCILIATION_WORKER_ENABLED` | `false` |
| `FEES_ENABLED` | `false` (env default) |

All workers are disabled so the verification run cannot mutate
`option_execution_events`, `option_execution_transactions`, or
`option_execution_reconciliations` as a side effect of starting the
server. `RPC_URL` is loaded only because env validation tolerates it;
no RPC call is made by the admin GET path.

### DB Baseline (pre-call)

```
option_execution_intents          | 3
option_execution_transactions     | 2
option_execution_events           | 19
option_execution_reconciliations  | 1
execution_transactions            | 1
fee_events                        | 28
v1s_trading_fee_count             | 2
```

V1S row anchors:

- intent `e6d2941b-65f7-413a-958f-74ab22c53b08`, status
  `broadcast_confirmed`, source `option_orderbook_fill`, source_id
  `81b3e1a8-52ef-4bc7-a947-98b60df8e842`.
- tx hash
  `0x5964a7b3d2c18d051baaa780413d31c44d419ce530f45263cb4c46f720881125`.
- per-event raw evidence (from
  `option_execution_events.decoded`):

  | log_index | trader | isMaker | appliedFee | recipient |
  | --- | --- | --- | --- | --- |
  | 28 | `0xc0a76c2a6c6b70c0b065a05e64417886416cc976` | `false` | `6` | `0x009f38440f058d095b61e0e2ee7fabdf05be7500` |
  | 34 | `0xbaf0976a00a0dcc84df5b15d927695c8b014b1c3` | `true`  | `4` | `0x009f38440f058d095b61e0e2ee7fabdf05be7500` |

  Backend fee ledger rows matching the V1S intent's source_id: `0`
  (so `backend_ledger_status` must explicitly report this, never fail).

### `/health`

```
{"ok":true,"service":"deopt-v2-backend"}
```

### `GET /admin/options/executions/e6d2941b-…/lifecycle` (excerpt)

```json
{
  "status": "broadcast_confirmed",
  "fees": {
    "source_of_truth": "onchain",
    "trading_fee_event_count": 2,
    "observed_total": "10",
    "by_trader": {
      "0xbaf0976a00a0dcc84df5b15d927695c8b014b1c3": "4",
      "0xc0a76c2a6c6b70c0b065a05e64417886416cc976": "6"
    },
    "by_recipient": {
      "0x009f38440f058d095b61e0e2ee7fabdf05be7500": "10"
    },
    "by_side": { "maker": "4", "taker": "6" },
    "total_by_recipient": {
      "0x009f38440f058d095b61e0e2ee7fabdf05be7500": "10"
    },
    "backend_ledger_status": "disabled",
    "reconciliation_status": "onchain_observed"
  },
  "reconciliation": { "status": "reconciled", "strict": true,
                       "trading_fee_event_count": 2,
                       "decoded_event_count": 19 },
  "health": { "stage": "reconciled", "is_terminal_success": true,
              "warnings": [], "errors": [] }
}
```

All assertions hold:

- `source_of_truth = "onchain"`.
- `trading_fee_event_count = 2`.
- `observed_total = "10"`.
- `by_side.taker = "6"`, `by_side.maker = "4"`.
- `by_trader` splits buyer (taker) `6` / seller (maker) `4`.
- `backend_ledger_status = "disabled"` is *explicit*; the lifecycle
  call succeeds despite the backend fee ledger being off, per V2C
  contract.

### `GET /admin/fees/onchain?tx_hash=0x5964a7b3…`

```json
{
  "source_of_truth": "onchain",
  "backend_ledger_enabled": false,
  "backend_ledger_status": "disabled",
  "trading_fee_event_count": 2,
  "observed_total": "10",
  "by_trader": {
    "0xbaf0976a00a0dcc84df5b15d927695c8b014b1c3": "4",
    "0xc0a76c2a6c6b70c0b065a05e64417886416cc976": "6"
  },
  "by_recipient": {
    "0x009f38440f058d095b61e0e2ee7fabdf05be7500": "10"
  },
  "by_side": { "maker": "4", "taker": "6" },
  "reconciliation_status": "onchain_observed",
  "filter": { "tx_hash": "0x5964a7b3…", "limit": 50 },
  "transactions": [ { "tx_hash": "0x5964a7b3…",
                      "trading_fee_event_count": 2,
                      "observed_total": "10",
                      "by_side": { "maker": "4", "taker": "6" } } ]
}
```

The unfiltered call
(`GET /admin/fees/onchain?limit=5`) returned the same totals
(`trading_fee_event_count = 2`, `observed_total = "10"`,
`backend_ledger_status = "disabled"`) because V1S is the only mined
option execution with indexed fees in this DB.

### No-Mutation Verification (post-call)

DB counts after both GETs (and one extra unfiltered onchain call):

```
option_execution_intents          | 3   (unchanged)
option_execution_transactions     | 2   (unchanged)
option_execution_events           | 19  (unchanged)
option_execution_reconciliations  | 1   (unchanged)
execution_transactions            | 1   (unchanged)
fee_events                        | 28  (unchanged)
v1s_trading_fee_count             | 2   (unchanged)
```

Process / log evidence:

- Server-side log contains six startup INFO lines and zero broadcast,
  signing, RPC, or insert lines. Every worker logged "disabled" at
  startup.
- `grep -E "broadcast|eth_sendRaw|/executor|sign\\(|HttpJsonRpc|
  insert.*execution_transaction"` against `src/fees/service.rs` and
  `src/options/lifecycle.rs` returned only documentation strings
  and the existing `LifecycleBroadcast` struct field; no new code
  path calls a broadcast/send helper.
- `POST /options/execution-intents/:id/broadcast`: **not called**.
- `POST /executor/broadcast/:intent_id`: **not called**.
- `eth_sendRawTransaction`: **not called**.
- V1L preserved evidence row (`tx 0xe832365b…`): untouched.
- No Solidity, frontend, deployment, or `.env` changes; no private
  keys printed (env was sourced, values redacted in any echo).

### Validation Commands Run

- `cargo fmt --all` — clean.
- `cargo clippy --all-targets --all-features -- -D warnings` — clean.
- `cargo test --all-targets --all-features` — all suites green
  (lib + 7 integration suites, 0 failures).
- `cargo build --all-targets --all-features` — clean.

### Remaining Blocker (live V1S)

None for V1S on-chain fee reconciliation. Drift comparison against
the backend ledger remains deferred to V2E because the live ledger
holds zero rows for the V1S `source_id` and `FEES_ENABLED` is `false`
in the operator env.

## Validation

- `cargo fmt --all`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test --all-targets --all-features`
- `cargo build --all-targets --all-features`
