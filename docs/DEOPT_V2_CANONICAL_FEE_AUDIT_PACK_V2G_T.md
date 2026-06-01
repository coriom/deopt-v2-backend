# V2G-T — DeOpt v2 Canonical Fee Audit Pack

## Status

- Milestone: **V2G-T** — canonical reference doc that consolidates every
  V2 fee artifact (Solidity, backend, observability, tests, runbook,
  governance / live-state references) into a single audit-ready
  document. **Docs-only.** No code touched, no live state mutated.
- Date: 2026-06-01.
- Outcome:
  - Single canonical fee schedule table per product per tier per flow.
  - Single source-of-truth event map across V1 + V2.
  - Cross-linked to every prior `V2G-*` milestone doc.
  - Production-readiness checklist for mainnet handoff.
- Hard gates respected: no broadcast, no deploy, no chain mutation,
  no backend restart, no compose touch, no Prometheus reset, no
  `.env` edit, no DB writes, no Solidity / backend / frontend code
  changes, no private-key handling, no soak interruption.

---

## 1. Canonical Fee Schedule

All ppm values are signed (`int32` on chain). 1 ppm = 0.0001 % = 1e-6.
Source: `src/fees/FeesManagerV2.sol::_installLaunchSchedules`, pinned
by `test/fees/FeesManagerV2.t.sol::testV2GQ_AllFiveTierProfilesAreCanonical`.

### 1.1 OPTION — basis = total premium exchanged in settlement asset

| Tier | maker ppm | taker ppm | maker % | taker % | net for 1.0 mUSDC premium |
|---:|---:|---:|---:|---:|---:|
| 0 |   50 | 250 |  0.005% | 0.025% | maker pays 1; taker pays 3 (ceil) |
| 1 |    0 | 150 |  0.000% | 0.015% | maker pays 0; taker pays 2 (ceil) |
| 2 |  -10 | 125 | -0.001% | 0.0125% | maker **rebate** 1 (floor); taker pays 2 (ceil) |
| 3 |  -25 | 100 | -0.0025% | 0.010% | maker **rebate** 2 (floor); taker pays 1 (ceil) |
| 4 |  -50 |  75 | -0.005% | 0.0075% | maker **rebate** 5 (floor); taker pays 1 (ceil) |

(Negative maker ppm = rebate, paid out of the protocol rebate budget.)

### 1.2 OPTION RFQ — taker discount applies to positive ppm only

`_effectiveRatePpm`: if `flow == RFQ && ratePpm > 0`, the V2 fee math
multiplies by `(1 − discountPpm / 1_000_000)` with `ceil`. Maker
rebates are preserved unchanged (V2G-N "Design Option A").

| Tier | maker discount (ppm) | taker discount (ppm) | maker % discount | taker % discount | effective OPTION RFQ taker ppm | RFQ taker fee for 1e6 basis (ceil) |
|---:|---:|---:|---:|---:|---:|---:|
| 0 |          0 |        0 |    0%  |    0% | 250 | 250 |
| 1 |    250_000 |  100_000 |   25%  |   10% | 135 | 135 |
| 2 |    500_000 |  250_000 |   50%  |   25% |  94 |  94 |
| 3 |    750_000 |  500_000 |   75%  |   50% |  50 |  50 |
| 4 |  1_000_000 |  750_000 |  100%  |   75% |  19 |  19 |

Tier 4 maker `1_000_000` discount = 100 % — would zero the positive
fee but maker is on the **negative** ppm path, so the rebate of −50
ppm is preserved unchanged (pinned by
`testV2GN_OptionRfqMakerPreservesNegativeRebatesEvenAtHundredPercentDiscount`
and `testV2GO_RfqTier4MakerRebatePreservedThroughMarginEngine`).

### 1.3 PERP — basis = notional in settlement asset

| Tier | maker ppm | taker ppm | maker % | taker % | RFQ maker discount | RFQ taker discount |
|---:|---:|---:|---:|---:|---:|---:|
| 0 |    50 | 300 |  0.005%  | 0.030% | 0 | 0 |
| 1 |     0 | 250 |  0.000%  | 0.025% | 0 | 0 |
| 2 |   -50 | 200 | -0.005%  | 0.020% | 0 | 0 |
| 3 |   -75 | 175 | -0.0075% | 0.0175% | 0 | 0 |
| 4 |  -100 | 150 | -0.010%  | 0.015% | 0 | 0 |

PERP RFQ discounts are structurally supported (zero everywhere at
launch). PERP fees are economically unaffected by `FlowKind`.

### 1.4 ppm mapping cheat-sheet

| ppm | percent | basis = 10_000 | basis = 1_000_000 | basis = 1e8 |
|---:|---:|---:|---:|---:|
| 1 | 0.0001% | 1 (ceil) | 1 (ceil) | 100 |
| 50 | 0.005% | 1 (ceil) | 50 | 5_000 |
| 100 | 0.01% | 1 (ceil) | 100 | 10_000 |
| 250 | 0.025% | 3 (ceil) | 250 | 25_000 |
| 1000 (=MAX_TAKER_FEE_PPM) | 0.10% | 10 | 1_000 | 100_000 |
| −50 | −0.005% (rebate) | 0 (floor) | 50 | 5_000 |
| −1000 (=MAX_MAKER_REBATE_PPM) | −0.10% | 10 (floor) | 1_000 | 100_000 |

**Rounding policy** (`FeesManagerV2._amountFromRate`):
- Positive ppm: `ceil(basis × ratePpm / 1_000_000)` (protocol-favoured).
- Negative ppm (rebate): `floor(basis × |ratePpm| / 1_000_000)` (protocol-favoured — never overpays the maker).
- Zero ppm: 0.

`MAX_TAKER_FEE_PPM = 1000`, `MAX_MAKER_REBATE_PPM = −1000`,
`PPM_DENOMINATOR = 1_000_000`. Setter `setFeeProfile` rejects
out-of-range values with `InvalidFeeRate`.

---

## 2. Tier Qualification

### 2.1 Qualification rules

The on-chain contract is **value-agnostic**: the operator publishes
a Merkle tree whose leaves are
`keccak256(abi.encode(account, tier, volume28d, volumeSharePpm, stakedDeopt, validFrom, validUntil))`
(per `hashTierLeaf`). The contract enforces `MerkleProof.verifyCalldata`
against this leaf — it does NOT compare the numeric metrics against
any threshold. Threshold OR-logic is therefore an off-chain policy:

| Tier | Criterion (any one suffices — operator publishes leaves accordingly) |
|---:|---|
| 0 | default (no claim required) |
| 1 | 28-day volume ≥ 5M USDC notional **OR** volume share ≥ 10_000 ppm **OR** staked DEOPT ≥ 50_000 |
| 2 | 28-day volume ≥ 10M USDC notional **OR** volume share ≥ 25_000 ppm **OR** staked DEOPT ≥ 100_000 |
| 3 | 28-day volume ≥ 25M USDC notional **OR** volume share ≥ 50_000 ppm **OR** staked DEOPT ≥ 250_000 |
| 4 | 28-day volume ≥ 100M USDC notional **OR** volume share ≥ 100_000 ppm **OR** staked DEOPT ≥ 1_000_000 |

The OR-logic is implemented by publishing one leaf per qualifying
metric for the same `(account, tier)`. Pinned offline by
`testV2GQ_VolumeOrShareOrStakedThresholdLeavesAllVerify`.

### 2.2 Merkle root + validity window

- `setMerkleRoot(newRoot, validFrom, validUntil)` — owner-only;
  rejects inverted window (`validFrom > validUntil` non-zero) with
  `InvalidMerkleRootWindow`.
- `claimTier(account, tier, vol, share, staked, validFrom, validUntil, proof)`:
  - `msg.sender == account` (`NotAccount`).
  - `tier < TIER_COUNT (=5)` (`InvalidTier`).
  - `validFrom ≤ block.timestamp ≤ validUntil` if non-zero (`TierNotYetValid` / `TierExpired`).
  - `rootValidFrom ≤ block.timestamp ≤ rootValidUntil` (root-window gate independent of leaf window).
  - Proof verifies via OZ `MerkleProof.verifyCalldata`.
- `currentTier(account)` returns 0 if `block.timestamp > claimedTier.validUntil`.

### 2.3 Lifecycle

| Action | Effect |
|---|---|
| First claim | Writes `_claimedTiers[account]`. |
| Replay same leaf | Idempotent — same tier remains. |
| Upgrade (new leaf) | `_claimedTiers[account]` overwrites; current tier increases. |
| Downgrade (new leaf) | Contract does NOT refuse — `_claimedTiers[account]` overwrites; current tier decreases. (Operator policy decides whether to publish such a leaf.) |
| Root rotation | Prior `_claimedTiers` survive (independent of live root); old proofs no longer verify against the new root. |
| Expiry (`block.timestamp > validUntil`) | `currentTier` returns 0; new claim with valid window resumes. |

Full lifecycle matrix: `docs/FEES_MANAGER_V2_TIER_ROOT_MATRIX_V2G_Q.md`.

---

## 3. Event Source-of-Truth Map

### 3.1 V2 events (canonical)

| Event | Emitter | Indexed | Fields (decoded JSON keys in `OptionExecutionEvent.decoded`) |
|---|---|---|---|
| `FeeChargedV2` | `FeesManagerV2.consumeFees` (taker / positive-maker leg) | yes | `consumer`, `trader`, `recipient`, `settlementAsset`, `productKind` (`option`/`perp`), `flowKind` (`orderbook`/`rfq`), `isMaker`, `feePpm`, `basisAmount`, `feeAmount` |
| `FeeRebatedV2` | `FeesManagerV2.consumeFees` (negative-maker leg) | yes | `consumer`, `trader`, `recipient`, `settlementAsset`, `productKind`, `flowKind`, `rebatePpm` (signed, negative), `basisAmount`, `rebateAmount` |
| `RebateBudgetSpent` | `FeesManagerV2.consumeFees` (rebate sub-step) | indexed; surfaced via Prometheus only | `settlementAsset`, `amount` |

### 3.2 V1 events (compatibility breadcrumbs only)

| Event | Emitter | Counted in V2 totals? |
|---|---|---|
| `TradingFeeCharged` (OPTION) | `MarginEngine` legacy compatibility | NO — V1 event remains visible in the event list; V2 totals win in `event_model="mixed"`. |
| `TradingFeeCharged` (PERP) | `PerpEngine` legacy compatibility | NO — same policy. |

### 3.3 Aggregation policy

`src/fees/onchain_summary.rs::classify_event_model`:

| Family present | `event_model` | `source_priority` | Totals driver |
|---|---|---|---|
| neither | `none` | `""` | — |
| only V1 | `v1` | `""` | V1 `appliedFee` |
| only V2 | `v2` | `""` | V2 `feeAmount` + `rebateAmount` |
| both | `mixed` | `"v2"` | V2 only; V1 zeros out |

Aggregation idempotency: dedup key
`(FeeEventModel, tx_hash, log_index, source_contract)`. DB primary
gate at `option_execution_events.UNIQUE(chain_id, tx_hash, log_index)`;
backend dedup pass at `normalize_fee_events` is defence-in-depth.
Full matrix: `docs/FEE_RECONCILIATION_IDEMPOTENCY_V2G_S.md`.

---

## 4. Accounting Rules

For a slice of indexed events, after dedup + V1/V2 policy:

| Quantity | Formula |
|---|---|
| **gross_fees** | `sum FeeChargedV2.feeAmount` (or V1 `appliedFee` in v1-only mode) |
| **rebates_paid** | `sum FeeRebatedV2.rebateAmount` |
| **net_protocol_fee** | `gross_fees − rebates_paid` |
| **by_product** | per-product map (`option`/`perp`/`unknown`) of `feeAmount` |
| **by_flow** | per-flow map (`orderbook`/`rfq`/`unknown`) of `feeAmount` |
| **by_side** | per-side map (`maker`/`taker`/`unknown`) of `feeAmount` |
| **by_trader** | per-trader map of charged `feeAmount` |
| **by_recipient** | per-recipient map (the destination of each fee, usually the V2 fee recipient EOA / contract) |
| **rebated_by_trader** | per-maker-trader map of `rebateAmount` |
| **rebated_by_product** | per-product map of `rebateAmount` |
| **rebated_by_flow** | per-flow map of `rebateAmount` |

Cardinality contracts:
- Maker rebates do NOT appear in `by_trader` (positive fees only) — they appear in `rebated_by_trader`.
- A maker on the positive ppm path (`Tier 0`/`Tier 1`) DOES appear in `by_trader`.
- `by_recipient` map: rebate legs route to the trader; positive-fee legs route to the configured fee-recipient.

Full surface: `AggregatedFees`, `LifecycleFees`, `OnchainFeeTxSummary` in `src/fees/onchain_summary.rs`.

---

## 5. Live Validation References (Base Sepolia)

### 5.1 V2G-E live smoke results

| What | Tx hash | Doc |
|---|---|---|
| PERP V2 rebate (maker negative, taker positive) | `0x5c15e9233d49729cf21058a89f49bc6fdf0f7295cda5a7f313c96556728aa394` | `docs/FEES_MANAGER_V2_LIVE_REBATE_SMOKE_RESULT_V2G_E.md` |
| OPTION V2 rebate (maker negative, taker positive) | `0x9a85cbced2216bf3c18049111cce68883cb0b035e194b3dcbaaf4fe7d5293149` | same |

### 5.2 V2G-D2 EOA tier registry (Base Sepolia)

| Role | Address | Tier | Status |
|---|---|---|---|
| OPTION RFQ taker (V2G-O test target) | `0x77ca9dd6ccce2d692fb23877a2db7178807b0020` | 2 | Claimed |
| OPTION RFQ maker (V2G-O test target) | `0x290bd12c93e467bf51c51f5273d35bddb19e9274` | 4 | Claimed |

### 5.3 Live state (Base Sepolia, per `deopt-v2-sol/deployments/base-sepolia.manifest.draft.json`)

| Surface | Address |
|---|---|
| `MarginEngine` (V2 fees-enabled, NEW) | `0x287Cef479be5889eEfCa847F9e73C860898f48Cc` |
| `MarginEngine` (legacy non-V2) | `0x6c5665de05e7314cb63cd77f82dfa86508a5b5f8` |
| `OptionMatchingEngine` | **`null` — never deployed** |
| `PerpEngine` (V2, NEW) | `0xc6c592100723fe0c66343a16e95ec34cc0c2141c` |
| `PerpEngine` (legacy, OLD — stranded; must not be used) | `0xb36395b67d0798ada981731c9fa5239f4362b53b` |
| `FeesManagerV2` | `0x00dA0B9876bcBf0c79CB5BcAcfEBAFb8C7Ad774f` |
| `FeesManagerV2.feeRecipient` | `0xa67f8e8e673ce4bb2fb563b0e6e9fa8f70e3b588` (ProtocolTimelock) |
| `FeesManagerV2.rebateFundingAccount` | `0xa67f8e8e673ce4bb2fb563b0e6e9fa8f70e3b588` (ProtocolTimelock) |
| `FeesManagerV2.rebateBudget(mUSDC)` after V2G-E | **`999987`** (started at `1_000_000`, decremented by V2G-E's two rebate consumptions: 8 PERP + 5 OPTION = 13) |
| `FeesManagerV2.merkleRoot` | active V2G-D2 root (validity window per snapshot publication) |

---

## 6. Metrics

### 6.1 Prometheus surface

| Metric | Labels | Source |
|---|---|---|
| `deopt_fees_charged_v2_total` | `consumer`, `product` (`option`/`perp`), `flow` (`orderbook`/`rfq`) | V2G-G fee observability indexer |
| `deopt_fees_rebated_v2_total` | `consumer`, `product`, `flow` | same |
| `deopt_rebate_budget_balance` | `asset` | V2G-G (gauge — read from FM-V2 `rebateBudget`) |
| `deopt_fees_charged_v1_legacy_total` | `consumer` | V2G-G — V1 compatibility log counter (no totals contribution) |
| `deopt_fees_unknown_consumer_total` | `consumer` | V2G-G — alerts when an event arrives from a `consumer` not in the allow-list (legacy / unknown engine guard) |

### 6.2 Cardinality contracts

- `consumer` label is the **engine** address (`MarginEngine` or `PerpEngine`), NOT the trader.
- `product` is `option` or `perp` only (no `unknown` in V2 — V1-only events drive the V1 counter).
- `flow` is `orderbook` or `rfq`. The first real RFQ trade will populate the `flow="rfq"` series — confirmed RFQ-ready by `testv2g_n_indexer_decodes_option_rfq_flow_kind_verbatim`.

### 6.3 Alerts (from V2F-P/Q + V2G-G)

| Alert | Condition |
|---|---|
| `FeeRebateBudgetLow` | `deopt_rebate_budget_balance{asset="mUSDC"}` < ops-defined floor |
| `FeeRebateBudgetExhausted` | rebate consumption attempted while budget = 0 (the next `consumeFees` rebate call would revert) |
| `FeeOldConsumer` | `deopt_fees_unknown_consumer_total{consumer=OLD_PERP_ENGINE}` > 0 (must never trigger — OLD_PERP_ENGINE is stranded) |
| `FeeUnknownConsumer` | `deopt_fees_unknown_consumer_total{consumer=…}` > 0 for any allow-list miss |
| `FeeChargedRebatedSkew` | `delta(fees_charged_v2_total)` and `delta(fees_rebated_v2_total)` skew exceeds expected ratio (configurable) |

Runbook: `docs/RUNBOOK_PERP_V2_FEE_ALERTS.md`.

---

## 7. Admin Endpoints

| Endpoint | Method | Status | Description |
|---|---|---|---|
| `/admin/fees/onchain` | GET | live in PID 56199 | Per-tx fee event summary; supports `?tx_hash=…` filter. Replay-safe (V2G-S). |
| `/admin/fees/v2/observability` | GET | live in PID 56199 | V2 fee event aggregation snapshot. |
| `/admin/fees/v2/smoke/readiness` | GET | **code-ready, not live** | V2G-M readiness probe. Code in `target/`; binds only after next backend restart. |
| `/admin/option_executions/{intent_id}/lifecycle` | GET | live | Per-intent lifecycle view including `LifecycleFees` (now with `by_product` / `by_flow`). |

After the V2G-M / V2G-S backend restart, the new V2G-S JSON fields
(`by_product`, `by_flow`, `rebated_by_product`, `rebated_by_flow`)
will surface on both `/admin/fees/onchain` and the lifecycle view.

---

## 8. Current Deployment Status

| Surface | Status |
|---|---|
| `NEW_PERP_ENGINE` `0xc6c592…2141c` | **active** — wired to FeesManagerV2, consumes V2 fees; V2G-E live verified |
| `OLD_PERP_ENGINE` `0xb36395…2b53b` | **stranded** — must NOT be re-used as the active perp engine; surfaced via `FeeOldConsumer` alert if it ever emits |
| `MarginEngine` `0x287Cef…48Cc` | active for OPTION fee path but **lacks V2G-O RFQ entrypoint** (`applyRfqTrade`); first OPTION RFQ trade will revert |
| `OptionMatchingEngine` | **never deployed** on Base Sepolia (`null` in manifest) — V2G-P deploy session pending |
| OPTION RFQ Solidity code | offline-ready (V2G-O); 6 tests green; bytecode in `out/` |
| Backend OPTION RFQ signing surface | offline-ready (V2G-P0 + V2G-P1); 10 packet-builder tests green |
| `ProtocolFeeVault` | offline-ready (V2G-R0/R1); 45 tests green (40 unit + 5 invariant); not deployed |
| `CollateralVault.transferFromInternalAccount` extension | interface defined (V2G-R1); not implemented on live CV |
| Backend PID 56199 | V2G-G era binary; predates V2G-M readiness endpoint and V2G-S by_product/by_flow surfaces; awaiting next restart window |

---

## 9. Production Gaps

| Gap | Owner | Blocking? | Doc |
|---|---|---|---|
| Target-host monitoring cutover (canonical Grafana/Prometheus, not the local L0 stack) | Ops | Yes for mainnet | `V2_FEE_OBSERVABILITY_TARGET_CUTOVER_V2G_J.md` |
| Backend restart after day-1 24h gate clears | Ops | Yes for new endpoint pickup | `V2_FEE_BACKEND_EXECUTOR_READINESS_V2G_M.md` |
| OPTION RFQ deploy + rewire (V2G-P) | Operator + Governance | Yes for live RFQ | `OPTION_RFQ_LIVE_DEPLOYMENT_PREFLIGHT_V2G_P0.md`, `OPTION_RFQ_OPERATOR_PACKET_V2G_P1.md` |
| `ProtocolFeeVault` deploy + FM-V2 recipient rotation (V2G-R5) | Operator + Governance | No for OPTION RFQ, yes for revenue separation | `PROTOCOL_FEE_VAULT_DESIGN_V2G_R.md`, `PROTOCOL_FEE_VAULT_IMPLEMENTATION_V2G_R1.md` |
| FM-V2 hook ABI extension (`onFeeCharged` / `onRebatePaid`) for vault Option β | Solidity | No for OPTION RFQ; precondition for vault on-chain accounting | V2G-R3 (queued) |
| `CollateralVault.transferFromInternalAccount` extension | Solidity | Yes for vault revenue withdrawal | V2G-R3 (queued) |
| Reorg-aware aggregation (confirmed_at/reorged_at columns + filter) | Backend + DB migration | No for current Base Sepolia (short reorgs); higher importance on mainnet | V2G-T (queued) |
| Governance / timelock hardening (transfer-ownership + 2-step on every owner-controlled contract) | Governance | Yes for mainnet | not yet documented; carry into V2G-U |
| External audit | External | Yes for mainnet | external |
| Mainnet deployment runbook | Ops | Yes for mainnet | not yet drafted |

---

## 10. Audit Checklist

### 10.1 Math + rounding
- ✅ Positive-fee `ceil(basis × ppm / 1_000_000)` — `testQuoteOptionsUsesPremiumBasisAndRoundsPositiveFeeUp`.
- ✅ Negative-rebate `floor(basis × |ppm| / 1_000_000)` — `testNegativeRebatesRoundDown`.
- ✅ Zero ppm → zero fee — covered in `testQuoteOptionsUsesPremiumBasisAndRoundsPositiveFeeUp` + Tier-1 maker check.
- ✅ RFQ discount applies to positive ppm only — `testRfqDiscountsReducePositiveFeesOnly`.
- ✅ 100 % RFQ discount floors positive fee to 0 — `testOneHundredPercentRfqDiscountFloorsPositiveFeeToZero`.
- ✅ All ppm boundaries (`MAX_TAKER_FEE_PPM = 1000`, `MAX_MAKER_REBATE_PPM = −1000`) — V2G-R2 boundary tests.

### 10.2 Budget
- ✅ `fundRebateBudget` increments, multi-call sums, owner-only — V2G-R2.
- ✅ `withdrawRebateBudget` decrements, rejects over-budget, owner-only, zero-asset / zero-`to` guard — V2G-R2.
- ✅ `consumeFees` rebate path reverts `InsufficientRebateBudget` strictly — `testInsufficientRebateBudgetRevertsStrictly` + V2G-R2 reaffirmation.
- ✅ Rebate funding account zero ⇒ rebate consumption reverts — `testRebateFundingAccountMustBeSetForNonZeroRebateConsumption`.

### 10.3 Merkle / claim
- ✅ Leaf shape pinned via `hashTierLeaf` — V2G-N + V2G-Q.
- ✅ OR-logic across volume / share / staked leaves — `testV2GQ_VolumeOrShareOrStakedThresholdLeavesAllVerify`.
- ✅ Validity windows (leaf + root) — `testV2GQ_RootValidFromGatesClaimsAcrossWindow` / `…AcrossWindow` / `testV2GQ_ClaimBeforeValidFromRevertsWithTierNotYetValid` / `…ClaimAfterValidUntilRevertsWithTierExpired`.
- ✅ Inverted-window setter rejected — `testV2GQ_SetMerkleRootRejectsInvertedWindow`.
- ✅ Root rotation does not retroactively clear claimed tiers — `testV2GQ_RootRotationKeepsExistingClaimButInvalidatesOldProofs`.
- ✅ Replay / upgrade / downgrade — `testV2GQ_ReplayOfSameClaimOverwritesIdempotently`, `…UpgradeClaimRaisesTier`, `…DowngradeClaimLowersTier`.

### 10.4 Authorization
- ✅ `claimTier` `NotAccount` gate — `testV2GQ_ClaimTierRejectsThirdPartyCaller`.
- ✅ `consumeFees` `onlyFeeConsumer` gate — `testConsumeFeesRequiresAuthorizedConsumer` + V2G-R2.
- ✅ Every setter `onlyOwner` + zero-address / boundary guards — V2G-R2 (34 tests).
- ✅ ProtocolFeeVault hooks `onlyFeesManagerV2` — V2G-R1 unit tests.

### 10.5 Replay / idempotency
- ✅ Aggregation deduped by `(model, tx_hash, log_index, source_contract)` — V2G-S (11 tests).
- ✅ Overlapping-block-range scans don't double-count — `v2gs_overlapping_block_range_replay_safe`.
- ✅ Admin per-tx summary deterministic under replay — `v2gs_admin_summary_per_tx_deterministic_under_replay`.
- ✅ DB primary gate `UNIQUE (chain_id, tx_hash, log_index)` — `src/db/repository.rs:3808`.
- ⏳ Reorg-aware aggregation — queued for V2G-T impl milestone.

### 10.6 Old / unknown consumer
- ✅ Prometheus alert `FeeOldConsumer` — fires if `OLD_PERP_ENGINE` ever emits a V2 fee event.
- ✅ Prometheus alert `FeeUnknownConsumer` — fires for any allow-list miss.
- ✅ Indexer surfaces `consumer` label verbatim from event — no enum filter that could mask drift.

### 10.7 Emergency disables
- ✅ FM-V2 `setUseFeesManagerV2(false)` on the engine instantly switches the path back to V1 (no rebates, V1 positive-fee model).
- ✅ FM-V2 `setRebateFundingAccount(address(0))` disables non-zero rebate consumption (next rebate call reverts cleanly).
- ✅ FM-V2 `setFeeConsumer(consumer, false)` removes an engine from the allowlist.
- ✅ FM-V2 `setMerkleRoot(0x0, 0, 0)` revokes all claims (every subsequent claim reverts `NoMerkleRoot`).
- ⏳ Per-product / per-flow pause flag — not yet implemented (consider V2G-U).
- ⏳ ProtocolFeeVault `pauseRebates` — implemented offline (V2G-R1), not deployed.

### 10.8 Monitoring
- ✅ `/health` exposes liveness — confirmed by 18 h+ soak.
- ✅ Prometheus scrape exposes all V2 fee counters/gauges — V2G-G.
- ✅ Local L0 compose stack runs Prometheus + Alertmanager + Grafana + webhook-sink — V2G-L0/L1/L2/L3.
- ✅ Day-1 24 h soak gate elapsed at wall-clock UTC `2026-06-01T17:38Z` and was canonically validated by the V2G-L4 checkpoint run (`docs/V2_FEE_OBSERVABILITY_LOCAL_COMPOSE_DAY1_CANONICAL_V2G_L4.md`). The gate is wall-clock UTC, NOT backend-process / container uptime.
- ⏳ Target-host monitoring cutover (canonical Grafana/Prometheus) — queued for V2G-J close.

---

## 11. Cross-link Index

### 11.1 Backend (`deopt-v2-backend/docs/`)

| Milestone | Doc |
|---|---|
| V2G-A | `REBATE_LIVE_SMOKE_PLAN_V2G_A.md`, `TIER_MERKLE_REBATE_SYSTEM_V2G_A.md`, `TIER_SNAPSHOT_SCHEMA_V2G_A.md` |
| V2G-B | `FEES_MANAGER_V2_REBATE_BROADCAST_PREFLIGHT_V2G_B.md`, `TIER_MERKLE_ARTIFACT_PIPELINE_V2G_B.md` |
| V2G-C | `FEES_MANAGER_V2_ROOT_BUDGET_SETUP_V2G_C.md` |
| V2G-D2/D3 | `FEES_MANAGER_V2_RECOVERY_V2G_D2.md`, `FEES_MANAGER_V2_CLAIM_TIER_RESULT_V2G_D3.md` |
| V2G-E | `FEES_MANAGER_V2_LIVE_REBATE_SMOKE_RESULT_V2G_E.md` |
| V2G-F | `FEE_OBSERVABILITY_ENV_HYGIENE_CLOSURE_V2G_F.md` |
| V2G-G | `V2_FEE_PRODUCTION_OBSERVABILITY_V2G_G.md` |
| V2G-H/I/J | `V2_FEE_OBSERVABILITY_LIVE_STACK_WIRING_V2G_H.md`, `…LIVE_ACTIVATION_V2G_I.md`, `…TARGET_CUTOVER_V2G_J.md` |
| V2G-K | `V2_FEE_OBSERVABILITY_7_DAY_SOAK_V2G_K.md` |
| V2G-L0..L3 | `V2_FEE_OBSERVABILITY_LOCAL_STACK_BOOTSTRAP_V2G_L0.md`, `…LOCAL_COMPOSE_SOAK_V2G_L1.md`, `…LOCAL_COMPOSE_LIVE_V2G_L2.md`, `…LOCAL_COMPOSE_DAY1_V2G_L3.md` |
| V2G-M | `V2_FEE_BACKEND_EXECUTOR_READINESS_V2G_M.md` |
| V2G-N | `OPTION_RFQ_FEE_DISCOUNTS_V2G_N.md` |
| V2G-O | `OPTION_RFQ_FLOW_WIRING_V2G_O.md` |
| V2G-P0/P1 | `OPTION_RFQ_LIVE_DEPLOYMENT_PREFLIGHT_V2G_P0.md`, `OPTION_RFQ_OPERATOR_PACKET_V2G_P1.md` |
| V2G-Q | `FEES_MANAGER_V2_TIER_ROOT_MATRIX_V2G_Q.md` |
| V2G-R0/R1/R2 | `PROTOCOL_FEE_VAULT_DESIGN_V2G_R.md`, (sol-side) `PROTOCOL_FEE_VAULT_IMPLEMENTATION_V2G_R1.md`, `FEES_MANAGER_V2_ADMIN_BUDGET_MATRIX_V2G_R2.md` |
| V2G-S | `FEE_RECONCILIATION_IDEMPOTENCY_V2G_S.md` |
| **V2G-T** | **this file** |

### 11.2 Solidity (`deopt-v2-sol/docs/`)

| Milestone | Doc |
|---|---|
| V2C | `FEE_MODEL_TARGET_GAP_ANALYSIS_V2C.md` (in backend docs dir — canonical location) |
| V2D-A..R | FeesManagerV2 design / implementation / deploy / wire / enable / option-integration / new-margin-engine preflight + result docs |
| V2E-A..E | FM-V2 preflight + deploy/wire/enable broadcast results |
| V2E-F/G/H/I | Tiny-trade preflight + result + V1V2 closure |
| V2F-A..N | Perp FM-V2 gap → deploy → wire → enable → smoke → live observability |
| V2G-R1 | `PROTOCOL_FEE_VAULT_IMPLEMENTATION_V2G_R1.md` |

### 11.3 Frontend (`deopt-v2-frontend/docs/`)

| Milestone | Doc |
|---|---|
| Admin lifecycle UI | `ADMIN_OPTION_LIFECYCLE_VIEW_V2A.md` |
| Admin V2 fee observability UI | `ADMIN_V2_FEE_OBSERVABILITY_V2E_H.md` |

---

## 12. Soak Preservation Status

| Check | State at V2G-T close |
|---|---|
| Backend PID 56199 alive | ✅ 18 h 47 m + (started ~`2026-05-31T17:38Z`) |
| `/health` | ✅ `{"ok":true,"service":"deopt-v2-backend"}` |
| Prometheus `/-/healthy` | ✅ |
| Compose containers up | ✅ 4/4 (grafana, prometheus, alertmanager, webhook-sink) Up 18 h+ |
| Day-1 24 h soak gate `2026-06-01T17:38Z` (wall-clock UTC) | reserved at the time of V2G-T close; canonically validated later by V2G-L4. Gate is wall-clock UTC, NOT backend-process or container uptime. |
| No `docker compose down` | ✅ |
| No backend restart | ✅ |
| No Prometheus reset | ✅ |
| No `.env` edit | ✅ |
| No DB writes | ✅ |
| No Solidity / backend / frontend code changes | ✅ — V2G-T is docs-only |

---

## 13. Validation

V2G-T touches `deopt-v2-backend/docs/` only. Lightweight checks:

| Command | Result |
|---|---|
| `git diff --check deopt-v2-backend/docs/DEOPT_V2_CANONICAL_FEE_AUDIT_PACK_V2G_T.md` | ✅ no whitespace errors |
| `git status --short` (Solidity) | ✅ clean — Solidity tree untouched |
| `git status --short` (frontend) | ✅ clean — frontend untouched |
| Cargo / forge / npm | **not run** — heavy test suites are skipped because no source code was touched in V2G-T |

---

## 14. Remaining Blockers

1. **V2G-M endpoint pickup** — requires backend restart at next maintenance window. New JSON fields (`by_product`, `by_flow`, `rebated_by_product`, `rebated_by_flow`) and `/admin/fees/v2/smoke/readiness` not bound until then.
2. **OPTION RFQ live deploy** (V2G-P) — V2G-O bytecode + V2G-P1 operator packet ready; broadcast pending the operator window now that the day-1 gate has cleared.
3. **ProtocolFeeVault live deploy** (V2G-R5) — vault + tests in `deopt-v2-sol/`; awaiting V2G-R3 (FM-V2 hook ABI + CV `transferFromInternalAccount` extension).
4. **Reorg-aware aggregation** — backend cannot currently filter reorged-out rows; requires non-destructive schema migration. Queued for a future V2G-T impl milestone.
5. **Governance / timelock hardening** — every owner-controlled contract should move to 2-step ownership under timelock before mainnet. Not yet drafted.
6. **External audit** — required before mainnet.
7. **Target-host monitoring cutover** (V2G-J close) — canonical Grafana / Prometheus host still pending; local L0 stack carries the soak today.

## 15. Next Recommended Milestone

**V2G-U — governance + ownership hardening pass + mainnet readiness checklist.**

1. Audit every owner-controlled contract (`FeesManagerV2`, `MarginEngine`, `PerpEngine`, `OptionMatchingEngine`, `MatchingEngine`, `PerpMatchingEngine`, `CollateralVault`, `RiskModule`, `OptionProductRegistry`, `RiskGovernor`, `ProtocolFeeVault`) for:
   - 2-step `Ownable` posture.
   - Timelock-only setter authorization.
   - Pause / guardian role separation.
2. Draft `MAINNET_DEPLOYMENT_RUNBOOK_V2G_U.md` covering deployment order, post-deploy verification, monitoring cutover, day-1 gate, rollback paths.
3. Pin a per-contract checklist of "before-broadcast invariants" the operator must verify.
4. External audit handoff package.

V2G-U is orthogonal to V2G-P (RFQ broadcast) and V2G-R5 (vault deploy); it can ship in parallel with either, but mainnet readiness depends on its closure.
