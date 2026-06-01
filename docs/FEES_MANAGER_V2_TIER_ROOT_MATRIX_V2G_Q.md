# V2G-Q — FeesManagerV2 Tier + Merkle Root Behavior Matrix

## Status

- Milestone: **V2G-Q** — closes the offline test coverage gap for the
  FeesManagerV2 tier / Merkle-root subsystem. Pins per-tier profile
  canonicalization, OR-threshold leaf semantics, replay /
  upgrade / downgrade lifecycle, root rotation, and the dual
  validity-window enforcement (leaf window + root window).
- Date: 2026-06-01.
- Outcome:
  - **18 new Solidity tests** in `test/fees/FeesManagerV2.t.sol`
    (the canonical FeesManagerV2 suite). Live deployed bytecode is
    NOT touched.
  - **No backend or frontend changes.** This milestone is pure
    contract-side test coverage + the matrix doc you are reading.
  - **OPTION + PERP schedules pinned by direct equality.** Any
    drift in `_installLaunchSchedules` breaks
    `testV2GQ_AllFiveTierProfilesAreCanonical` immediately.
  - **Threshold semantics clarified.** The on-chain contract is
    value-agnostic — the V2C → V2G OR-logic between volume28d,
    volumeSharePpm, and stakedDeopt is enforced **off-chain by
    the operator's Merkle tree composition**. V2G-Q pins this by
    showing that three sibling leaves for one (account, tier) all
    verify against a single root, and that any tuple not in the
    published tree fails with `ProofInvalid` regardless of whether
    it sits above or below the operator's published threshold
    boundary.
  - **Soak preserved.** Backend PID 56199 + 4-container compose
    stack untouched.
- Hard gates respected: no broadcast, no chain mutation, no
  redeploy, no backend restart, no compose touch, no Prometheus
  reset, no `.env` edit, no DB writes, no private-key handling,
  no governance/timelock action, no soak interruption.

## Canonical tier schedules

These values are tested by
`testV2GQ_AllFiveTierProfilesAreCanonical` and must match
`src/fees/FeesManagerV2.sol::_installLaunchSchedules`. Any drift
fails the test.

### OPTION (basis = premium)

| Tier | maker ppm | taker ppm | RFQ maker discount | RFQ taker discount |
|---|---:|---:|---:|---:|
| 0 |   50 | 250 |        0 |        0 |
| 1 |    0 | 150 |  250_000 |  100_000 |
| 2 |  -10 | 125 |  500_000 |  250_000 |
| 3 |  -25 | 100 |  750_000 |  500_000 |
| 4 |  -50 |  75 | 1_000_000 |  750_000 |

### PERP (basis = notional)

| Tier | maker ppm | taker ppm | RFQ maker discount | RFQ taker discount |
|---|---:|---:|---:|---:|
| 0 |    50 | 300 | 0 | 0 |
| 1 |     0 | 250 | 0 | 0 |
| 2 |   -50 | 200 | 0 | 0 |
| 3 |   -75 | 175 | 0 | 0 |
| 4 |  -100 | 150 | 0 | 0 |

PERP RFQ discount is intentionally 0 everywhere — the structural
plumbing exists (V2G-N) but no on-chain economic effect.

## Behavior matrix

Each cell is a function name in `FeesManagerV2.t.sol`. ✓ = green,
no asterisk needed.

### Tier 0–4 profile reads

| Subject | Test | Status |
|---|---|---|
| All five tiers' OPTION + PERP fee profiles + OPTION RFQ discounts in one sweep. | `testV2GQ_AllFiveTierProfilesAreCanonical` | ✓ |
| Existing pre-V2G-Q canonical coverage of OPTION RFQ taker walk (V2G-N). | `testV2GN_OptionRfqTakerTableWalk` | ✓ |
| Existing pre-V2G-Q canonical coverage of OPTION RFQ Tier 4 100% maker discount preservation (V2G-N). | `testV2GN_OptionRfqMakerPreservesNegativeRebatesEvenAtHundredPercentDiscount` | ✓ |
| Existing pre-V2G-Q canonical PERP unaffected by RFQ (V2G-N). | `testV2GN_PerpRfqUnaffectedAtEveryTierForBothLegs` | ✓ |
| Existing pre-V2G-Q canonical OPTION ORDERBOOK unchanged for every tier (V2G-N). | `testV2GN_OptionOrderbookUnchangedForEveryTier` | ✓ |

### Threshold OR logic

| Subject | Test | Status |
|---|---|---|
| Operator publishes 3 sibling leaves (volume / share / staked) for the same (account, tier); all three proofs verify against the root. | `testV2GQ_VolumeOrShareOrStakedThresholdLeavesAllVerify` | ✓ |
| Exact-tuple leaf accepted. | `testV2GQ_ExactThresholdBoundaryAcceptsLeafExactly` | ✓ |
| Below-threshold tuple rejected with `ProofInvalid` (because that exact tuple wasn't published). | `testV2GQ_BelowThresholdMetricsFailWithProofInvalid` | ✓ |
| Above-threshold tuple also rejected with `ProofInvalid` (contract is value-agnostic, not range-checking). | `testV2GQ_AboveThresholdMetricsAlsoFailWithProofInvalid` | ✓ |

The contract does **not** enforce "user must meet threshold X" —
it enforces "the operator's Merkle tree includes this exact
(account, tier, vol, share, staked, validFrom, validUntil)
tuple." Threshold qualification is therefore an *operator
policy* implemented in the off-chain tree builder. The contract
guarantees:

- Any user can claim a tier iff the operator published a leaf for
  them.
- A user cannot fabricate a different metric tuple and have it
  verify.
- A user with a leaf earned by *any one* criterion (volume OR
  share OR staked) can claim — the operator publishes one leaf
  per qualifying criterion.

### Expiry + window enforcement

| Subject | Test | Status |
|---|---|---|
| `currentTier(account)` returns 0 after `validUntil`. | `testV2GQ_ExpiredClaimFallsBackToTier0` (V2G-Q) + `testTierExpiryFallsBackToTier0` (pre-V2G-Q) | ✓ |
| `claimTier` after `validUntil` reverts `TierExpired`. | `testV2GQ_ClaimAfterValidUntilRevertsWithTierExpired` | ✓ |
| `claimTier` before `validFrom` reverts `TierNotYetValid`. | `testV2GQ_ClaimBeforeValidFromRevertsWithTierNotYetValid` | ✓ |
| `rootValidFrom` gates `claimTier` even if the leaf's own `validFrom` has elapsed. | `testV2GQ_RootValidFromGatesClaimsAcrossWindow` | ✓ |
| `rootValidUntil` gates `claimTier` even if the leaf's own `validUntil` is far in the future. | `testV2GQ_RootValidUntilGatesClaimsAcrossWindow` | ✓ |
| `setMerkleRoot` rejects inverted window (`validFrom > validUntil`, with `validUntil != 0`). | `testV2GQ_SetMerkleRootRejectsInvertedWindow` | ✓ |

### Lifecycle

| Subject | Test | Status |
|---|---|---|
| Re-claiming the same (account, tier, metrics, window) is idempotent — same tier remains, no revert. | `testV2GQ_ReplayOfSameClaimOverwritesIdempotently` | ✓ |
| Upgrade: tier 2 → tier 4 via a new leaf. | `testV2GQ_UpgradeClaimRaisesTier` | ✓ |
| Downgrade: tier 4 → tier 1 via a new leaf — the contract does NOT refuse downgrades. | `testV2GQ_DowngradeClaimLowersTier` | ✓ |
| Root rotation does NOT retroactively clear claimed tiers — `_claimedTiers` is independent of the live `merkleRoot`. | `testV2GQ_RootRotationKeepsExistingClaimButInvalidatesOldProofs` | ✓ |
| After rotation, old proofs no longer verify against the new root. | same test as above | ✓ |
| Invalid proof reverts `ProofInvalid`. | `testClaimTierRejectsInvalidProof` (pre-V2G-Q) | ✓ |

### Authorization

| Subject | Test | Status |
|---|---|---|
| `claimTier` reverts `NotAccount` when `msg.sender != account`. | `testV2GQ_ClaimTierRejectsThirdPartyCaller` | ✓ |
| `claimTier` with no Merkle root configured reverts `NoMerkleRoot`. | `testV2GQ_ClaimTierWithNoRootRevertsWithNoMerkleRoot` | ✓ |
| `claimTier` with `tier >= TIER_COUNT` reverts `InvalidTier`. | `testV2GQ_ClaimTierRejectsOutOfRangeTier` | ✓ |

## What V2G-Q intentionally does NOT pin

| Not pinned | Reason |
|---|---|
| Off-chain Merkle-tree composition (volume / share / staked threshold *values* per tier). | These are operator policy, not contract invariants. See `docs/FEES_MANAGER_V2_DESIGN_SPEC_V2D_A.md` for the policy mapping V2G-Q does not duplicate. |
| Live-chain replays. | This is offline-only. |
| `setMerkleRoot` access control (already covered by `Ownable` and tested elsewhere). | Existing coverage. |
| Eth-sig verification of the claimer. | The contract uses `msg.sender == account`, not a signed message. The V2G-D2 EOA registry handles the signed-claim flow above the contract layer. |

## OPTION / PERP schedule canonicalization

`testV2GQ_AllFiveTierProfilesAreCanonical` is the single drift
guard for both products. If the constructor's
`_installLaunchSchedules` changes any of these constants without
updating the test, the suite fails immediately. Operators should
treat that test failure as a tier-economics change requiring
governance.

## Validations

| Command | Result |
|---|---|
| `forge fmt` | clean |
| `forge fmt --check` | ✅ |
| `forge build` | ✅ |
| `forge test --no-match-path 'test/fork/*'` | ✅ **244 / 0 / 0** (V2G-P1 baseline 226 + V2G-Q +18) |
| `forge test --match-test "testV2GQ_"` | ✅ 18 / 0 / 0 |
| Backend `cargo` checks | not run — backend not touched in V2G-Q |
| Frontend checks | not run — frontend not touched in V2G-Q |

## Monitoring soak preservation

| Check | State at V2G-Q close |
|---|---|
| Backend PID 56199 alive | ✅ (no restart) |
| `/health` | ✅ |
| Prometheus `/-/healthy` | ✅ |
| Compose containers up | ✅ (16h+ uptime carried across V2G-O / P0 / P1 / Q) |
| Day-1 24h soak gate `2026-06-01T17:38Z` | ✅ tick window approaches; gate still reserved |
| No `docker compose down` | ✅ |
| No Prometheus reset | ✅ |
| No backend restart | ✅ |
| No `.env` edit (real secrets) | ✅ |

## Remaining blockers

1. **V2G-K day-1 24h soak gate** still reserved for
   `2026-06-01T17:38Z`. The V2G-P broadcast window can begin only
   after that tick clears.
2. **`SmokeOptionRfqV2FeesExecute.s.sol`** is still a scaffold —
   needs the operator broadcast block (V2G-P2).
3. **No live `OptionMatchingEngine` on Base Sepolia.** V2G-P
   (V2G-O bytecode broadcast) still pending.
4. **V2G-M endpoint pickup requires backend restart.**

## V2G-R2 follow-up (2026-06-01)

V2G-R2 has closed the orthogonal admin / setter / consumer /
budget axis with 34 new tests (`testV2GR2_*`) covering every
owner-facing surface on FeesManagerV2. See
`docs/FEES_MANAGER_V2_ADMIN_BUDGET_MATRIX_V2G_R2.md`.

**Together V2G-Q + V2G-R2 cover the entire offline behavior
surface of FeesManagerV2.** The only remaining contract-side
work before V2G-R5 broadcast is the FM-V2 ↔ ProtocolFeeVault
hook ABI extension (V2G-R3) plus the CollateralVault
`transferFromInternalAccount` extension.

## V2G-T canonical reference

V2G-Q's tier + root matrix is consolidated into Section 2 of the
canonical V2 fee audit pack at
`docs/DEOPT_V2_CANONICAL_FEE_AUDIT_PACK_V2G_T.md`. The audit pack
is the single reference doc operators / auditors should read first.

## Next recommended milestone

**V2G-R — close the FeesManagerV2 setter / consumer / budget
behavior matrix** (the orthogonal axis to V2G-Q's tier+root
matrix). Targets:

- `setFeeRecipient` non-zero invariant + ownership.
- `setRebateFundingAccount` ownership.
- `setFeeConsumer` ownership; `consumeFees` `onlyFeeConsumer`.
- `fundRebateBudget` + `withdrawRebateBudget` accounting; `to`
  non-zero refusal; insufficient-budget revert; idempotent
  overflow check.
- `setRfqDiscountProfile` ownership; `InvalidDiscount` boundary
  cases for both legs.
- `setFeeProfile` boundary cases against `MAX_TAKER_FEE_PPM` and
  `MAX_MAKER_REBATE_PPM`.

Combine with V2G-Q to give 100% offline coverage of
FeesManagerV2 before V2G-P (the broadcast) lands.
