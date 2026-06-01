# V2G-R2 — FeesManagerV2 Admin / Setter / Consumer / Budget Behavior Matrix

## Status

- Milestone: **V2G-R2** — closes the offline test coverage of every
  owner-facing surface on `FeesManagerV2` that V2G-Q did not pin
  (tiers + Merkle root). With V2G-Q + V2G-R2 in place the offline
  FeesManagerV2 surface is now exhaustively tested.
- Date: 2026-06-01.
- Outcome:
  - **34 new Solidity tests** in `test/fees/FeesManagerV2.t.sol`
    (all prefixed `testV2GR2_`, all green).
  - **No backend / frontend changes.** Offline contract-side
    coverage only.
  - **Canonical schedules preserved.** No test mutates the
    constructor-installed OPTION / PERP fee profile or RFQ
    discount tables in a way that leaks to other tests — each
    setter test either touches a different tier or runs in
    isolation.
  - **Soak preserved.** PID 56199 + 4-container compose stack
    untouched.
- Hard gates respected: no broadcast, no deploy, no chain
  mutation, no backend restart, no compose touch, no
  Prometheus reset, no `.env` edit, no DB writes, no private-key
  handling, no soak interruption.

## Behavior matrix

Each cell maps to a test name in `FeesManagerV2.t.sol`.
Pre-existing coverage that already pinned the same property is
noted in the "Pre-existing" column so the V2G-R2 row stays
honest about what is *new* vs *redundant-for-belt-and-braces*.

### `setFeeRecipient`

| Property | New (V2G-R2) | Pre-existing |
|---|---|---|
| Rejects zero address | `testV2GR2_SetFeeRecipientRejectsZero` | `testFeeRecipientMustRemainNonZero` |
| Rejects non-owner caller | `testV2GR2_SetFeeRecipientRejectsNonOwner` | — |
| Updates state + emits `FeeRecipientSet(old, new)` | `testV2GR2_SetFeeRecipientUpdatesAndEmits` | — |

### `setRebateFundingAccount`

| Property | New (V2G-R2) | Pre-existing |
|---|---|---|
| **Accepts zero** (intentional — disables non-zero rebate consumption per NatSpec) | `testV2GR2_SetRebateFundingAccountAcceptsZeroToDisable` | — |
| Accepts non-zero + emits | `testV2GR2_SetRebateFundingAccountAcceptsNonZero` | — |
| Rejects non-owner | `testV2GR2_SetRebateFundingAccountRejectsNonOwner` | — |

The task spec asked for a "non-zero invariant" — the contract
deliberately does *not* enforce one (zero is the documented way
to disable rebates). V2G-R2 pins this design intent explicitly
to prevent a regression that adds a non-zero check.

### `setFeeConsumer`

| Property | New (V2G-R2) | Pre-existing |
|---|---|---|
| Enable + disable round-trip + emits | `testV2GR2_SetFeeConsumerEnableDisableCycle` | — |
| Rejects zero consumer | `testV2GR2_SetFeeConsumerRejectsZero` | — |
| Rejects non-owner | `testV2GR2_SetFeeConsumerRejectsNonOwner` | — |

### `consumeFees`

| Property | New (V2G-R2) | Pre-existing |
|---|---|---|
| Rejects unauthorized caller with `NotFeeConsumer(caller)` | `testV2GR2_ConsumeFeesRejectsUnauthorizedCaller` | `testConsumeFeesRequiresAuthorizedConsumer` |
| Accepts authorized caller + returns correct OPTION Tier-0 taker fee (250 ppm) | `testV2GR2_ConsumeFeesAcceptsAuthorizedCaller` | `testConsumePositiveFeeEmitsFeeChargedEvent` |
| Positive OPTION Tier-0 *maker* fee (50 ppm, premium basis) | `testV2GR2_ConsumePositiveMakerFeeAtTier0` | — |
| Positive OPTION Tier-0 taker fee on canonical basis (1e6 premium) | `testV2GR2_ConsumePositiveTakerFeeAtTier0` | partial (`testConsumePositiveFeeEmitsFeeChargedEvent` on different basis) |
| Negative OPTION Tier-4 maker rebate decreases budget by exact amount | `testV2GR2_ConsumeNegativeMakerRebateDecreasesBudget` | `testConsumeRebateDecrementsBudgetAndEmitsEvents` |
| Insufficient rebate budget reverts with `InsufficientRebateBudget(asset, avail, req)` | `testV2GR2_ConsumeRebateInsufficientBudgetReverts` | `testInsufficientRebateBudgetRevertsStrictly` |

### `fundRebateBudget`

| Property | New (V2G-R2) | Pre-existing |
|---|---|---|
| Increments `rebateBudget[asset]` + emits `RebateBudgetFunded(asset, amount)` + sums on second call | `testV2GR2_FundRebateBudgetAccountingAndEvent` | — |
| Rejects zero asset | `testV2GR2_FundRebateBudgetRejectsZeroAsset` | — |
| Rejects non-owner | `testV2GR2_FundRebateBudgetRejectsNonOwner` | — |

### `withdrawRebateBudget`

| Property | New (V2G-R2) | Pre-existing |
|---|---|---|
| Decreases budget + emits `RebateBudgetWithdrawn(asset, to, amount)` | `testV2GR2_WithdrawRebateBudgetDecreasesAndEmits` | `testOwnerCanWithdrawAccountingOnlyRebateBudget` (partial) |
| Reverts over budget with `InsufficientRebateBudget(asset, avail, req)` | `testV2GR2_WithdrawRebateBudgetRejectsOverBudget` | — |
| Rejects zero asset | `testV2GR2_WithdrawRebateBudgetRejectsZeroAsset` | — |
| Rejects zero `to` | `testV2GR2_WithdrawRebateBudgetRejectsZeroTo` | — |
| Rejects non-owner | `testV2GR2_WithdrawRebateBudgetRejectsNonOwner` | — |

### `setFeeProfile` boundaries

| Property | New (V2G-R2) | Pre-existing |
|---|---|---|
| Accepts `makerPpm = MAX_MAKER_REBATE_PPM` AND `takerPpm = MAX_TAKER_FEE_PPM` + emits `FeeProfileUpdated` | `testV2GR2_SetFeeProfileAcceptsMaxRebateAndMaxTaker` | — |
| Rejects `makerPpm = MAX_MAKER_REBATE_PPM − 1` with `InvalidFeeRate` | `testV2GR2_SetFeeProfileRejectsMakerBelowMaxRebate` | — |
| Rejects `takerPpm = MAX_TAKER_FEE_PPM + 1` with `InvalidFeeRate` | `testV2GR2_SetFeeProfileRejectsTakerAboveMax` | — |
| Rejects `makerPpm = MAX_TAKER_FEE_PPM + 1` (positive-side maker cap) | `testV2GR2_SetFeeProfileRejectsTakerAboveMakerCap` | — |
| Rejects negative `takerPpm` (taker side is non-negative only) | `testV2GR2_SetFeeProfileRejectsNegativeTaker` | — |
| Rejects `tier >= TIER_COUNT` with `InvalidTier` | `testV2GR2_SetFeeProfileRejectsInvalidTier` | — |
| Rejects non-owner | `testV2GR2_SetFeeProfileRejectsNonOwner` | — |

### `setRfqDiscountProfile` boundaries

| Property | New (V2G-R2) | Pre-existing |
|---|---|---|
| Accepts `discountPpm == PPM_DENOMINATOR` (100% discount, the V2G-N Tier-4 maker case) + emits | `testV2GR2_SetRfqDiscountProfileAcceptsAtPpmDenominator` | — |
| Rejects `discountPpm > PPM_DENOMINATOR` with `InvalidDiscount` | (pre-existing) | `testV2GN_RfqDiscountSetterRejectsOverflow` |
| Rejects `tier >= TIER_COUNT` with `InvalidTier` | `testV2GR2_SetRfqDiscountProfileRejectsInvalidTier` | — |
| Rejects non-owner | `testV2GR2_SetRfqDiscountProfileRejectsNonOwner` | — |

### `productFeeBasis`

| Property | New (V2G-R2) | Pre-existing |
|---|---|---|
| `OPTION ⇒ PREMIUM`, `PERP ⇒ NOTIONAL` | `testV2GR2_ProductFeeBasisGetters` | `testLaunchConstantsAndProductBasisAreCorrect` (partial — same getters under a different name) |

### Event emission summary

V2G-R2 pins event emission for **every** owner setter via
`vm.expectEmit`:

| Event | Pinned by |
|---|---|
| `FeeRecipientSet(oldRecipient, newRecipient)` | `testV2GR2_SetFeeRecipientUpdatesAndEmits` |
| `RebateFundingAccountSet(oldAccount, newAccount)` | `testV2GR2_SetRebateFundingAccountAcceptsZeroToDisable` + `testV2GR2_SetRebateFundingAccountAcceptsNonZero` |
| `FeeConsumerSet(consumer, allowed)` | `testV2GR2_SetFeeConsumerEnableDisableCycle` |
| `FeeProfileUpdated(tier, product, makerPpm, takerPpm)` | `testV2GR2_SetFeeProfileAcceptsMaxRebateAndMaxTaker` |
| `RfqDiscountProfileUpdated(tier, product, makerDiscount, takerDiscount)` | `testV2GR2_SetRfqDiscountProfileAcceptsAtPpmDenominator` |
| `RebateBudgetFunded(asset, amount)` | `testV2GR2_FundRebateBudgetAccountingAndEvent` |
| `RebateBudgetWithdrawn(asset, to, amount)` | `testV2GR2_WithdrawRebateBudgetDecreasesAndEmits` |

`MerkleRootSet` and `TierClaimed` are V2G-Q's responsibility and
already covered there.

## What V2G-R2 intentionally does NOT do

| Skipped | Reason |
|---|---|
| Touch live chain | Hard gate. |
| Modify deployed bytecode | Hard gate. |
| Re-test V2G-Q tier / Merkle behavior | Already covered. |
| Re-test V2G-N OPTION RFQ discount math | Already covered. |
| Add backend tests | No backend code path was modified. |

## Canonical schedule preservation

Two of the V2G-R2 setter tests touch storage:
- `testV2GR2_SetFeeProfileAcceptsMaxRebateAndMaxTaker` writes a
  `(MAX_MAKER_REBATE_PPM, MAX_TAKER_FEE_PPM)` profile to Tier-0
  OPTION.
- `testV2GR2_SetRfqDiscountProfileAcceptsAtPpmDenominator` writes
  a `(PPM_DENOMINATOR, PPM_DENOMINATOR)` RFQ discount to Tier-2
  OPTION.

Both run against a per-test fresh contract (each `setUp`
deploys a new `FeesManagerV2`), so the writes do not leak into
the V2G-Q `testV2GQ_AllFiveTierProfilesAreCanonical` drift
guard. The constructor-installed schedules remain the canonical
source of truth.

## Combined coverage with V2G-Q

| Axis | Covered by |
|---|---|
| Tier 0–4 profile canonicalization (OPTION + PERP, ORDERBOOK + RFQ) | V2G-Q |
| Threshold OR-logic (volume / share / staked leaves) | V2G-Q |
| Threshold boundary & off-tuple rejection | V2G-Q |
| Expiry, validity windows, root-window vs leaf-window | V2G-Q |
| Replay / upgrade / downgrade lifecycle | V2G-Q |
| Root rotation invalidates old proofs | V2G-Q |
| `claimTier` authorization (NotAccount / NoMerkleRoot / InvalidTier) | V2G-Q |
| `setFeeRecipient` / `setRebateFundingAccount` / `setFeeConsumer` | **V2G-R2** |
| `consumeFees` authorization + positive maker/taker + rebate | **V2G-R2** |
| `fundRebateBudget` / `withdrawRebateBudget` accounting + guards | **V2G-R2** |
| `setFeeProfile` / `setRfqDiscountProfile` boundaries | **V2G-R2** |
| `productFeeBasis` getters | **V2G-R2** (+V2G-N partial) |
| Owner-only access control on every setter | **V2G-R2** |
| Event emission on every setter | **V2G-R2** |

**V2G-Q + V2G-R2 together cover the entire offline behavior
surface of FeesManagerV2.** The only remaining contract-side
gap is the FM-V2 ↔ ProtocolFeeVault hook ABI extension that
V2G-R2 (this milestone, code-side) will add as part of the
V2G-R2 implementation work — see V2G-R0/R1 design / impl docs.

## Validations

| Command | Result |
|---|---|
| `forge fmt` | clean |
| `forge fmt --check` | ✅ |
| `forge build` | ✅ |
| `forge test --no-match-path 'test/fork/*' --match-test "testV2GR2_"` | ✅ 34 / 0 / 0 |
| `forge test --no-match-path 'test/fork/*'` | ✅ — see full-run totals |
| Backend `cargo` | not run — backend not touched |
| Frontend | not run — frontend not touched |

## Monitoring soak preservation

| Check | State at V2G-R2 close |
|---|---|
| Backend PID 56199 alive | ✅ (no restart) |
| `/health` | ✅ |
| Prometheus `/-/healthy` | ✅ |
| Compose containers up | ✅ (16h+ uptime carried across O / P0 / P1 / Q / R0 / R1 / R2) |
| Day-1 24h soak gate `2026-06-01T17:38Z` | reserved — not yet ticked |
| No `docker compose down` | ✅ |
| No Prometheus reset | ✅ |
| No backend restart | ✅ |
| No `.env` edit (real secrets) | ✅ |

## Remaining blockers

1. **V2G-K day-1 24h gate** still reserved for
   `2026-06-01T17:38Z`. No live activity before that tick.
2. **`CollateralVault.transferFromInternalAccount` extension**
   still queued (V2G-R3 / V2G-R5).
3. **FeesManagerV2 hook ABI extension** for `onFeeCharged` /
   `onRebatePaid` queued (V2G-R3).
4. **No live `OptionMatchingEngine` on Base Sepolia** — V2G-P
   broadcast pending.
5. **V2G-M endpoint pickup** still queued for backend restart.

## V2G-T canonical reference

V2G-R2's admin / setter / budget matrix is consolidated into
Section 10 (Audit Checklist) and Section 4 (Accounting Rules) of
the canonical V2 fee audit pack at
`docs/DEOPT_V2_CANONICAL_FEE_AUDIT_PACK_V2G_T.md`.

## Next recommended milestone

**V2G-R3 — wire FeesManagerV2 ↔ ProtocolFeeVault hooks + add the
CollateralVault ABI extension.** Specifically:

1. Add `onFeeCharged` / `onRebatePaid` calls inside
   `FeesManagerV2.consumeFees`, gated by a new
   `setFeeRecipientCallback(bool)` flag for safe rollout. ABI is
   additive — does not break existing engine integrations.
2. Add `CollateralVault.transferFromInternalAccount(asset, to,
   amount)` with `msg.sender == from` gating (~15 LOC + tests).
3. Integration tests that exercise the full path:
   engine → `consumeFees` → CV transfer → FM-V2 callback → vault
   hooks → vault state assertions.
4. Hold the V2G-R5 operator broadcast scripts until V2G-R3 is
   green AND the V2G-K day-1 gate has cleared.
