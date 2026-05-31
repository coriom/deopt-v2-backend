# V2G-C — FeesManagerV2 Root + Rebate Budget Setup

## Status

- Milestone: **V2G-C** (follows V2G-B).
- Date: 2026-05-30.
- Mode: human-gated Base Sepolia setup; **3 owner-signed broadcasts,
  fully verified**. No rebate trades. No DB destructive action. No
  `.env` edit. No deployed-contract change.
- Outcome: `FeesManagerV2` is live with the V2G-C Merkle root, the
  rebate funding account is topped up with 1 mUSDC, and the
  accounting-only `rebateBudget(mUSDC)` is set to `1_000_000`. The
  PERP and OPTION smoke preflights now advance from
  `MerkleRootUnset()` to `MakerHasNoNegativeRebateTier(0, 50)` —
  the exact V2G-D blocker.

## Artifact used

V2G-B's window (`validFrom = 1748563200`) was 2025-05-30 and was
**stale**; every claim against it would revert with `TierExpired`.
V2G-C regenerated a fresh artifact:

- Fixture: `fixtures/tier_snapshot/base_sepolia_v2g_c_smoke.json`
- Artifact: `artifacts/tier_merkle/base_sepolia_v2g_c.json`
- **Root**: `0xa29d5b1de46fb2e498999093400fd65c1240a6e8abeb106504e8bc4edb1e2553`
- **Window**: `validFrom = 1780099200` (2026-05-30 00:00 UTC) →
  `validUntil = 1781308800` (2026-06-13 00:00 UTC) — 14 days.
- Rows (deterministic ascending order by trader):
  1. `0x475fe397fa56884952d350aa9ee1c3946964bc0c` — Tier 4 / Tier 4,
     proof length 2, leaf
     `0x6b8c…44ec` (see artifact for full hash).
  2. `0x8b94a83d1ad3bd2337b1886e7962ca8e0bba9a34` — Tier 2 / Tier 2,
     proof length 2.
  3. `0xbaf0976a00a0dcc84df5b15d927695c8b014b1c3` — Tier 0 / Tier 0,
     proof length 1.

The new fixture/artifact carry the same three smoke accounts as
V2G-B; only the validity window changed. The V2G-B artifact stays
on disk for forensic comparison but is **no longer the operator
target** — explicitly superseded by V2G-C.

## Pre-broadcast read-only snapshot (Phase 2)

Cast'd against the live RPC before any gate:

```
FeesManagerV2.owner()             = 0xc35F7A8A103A9A4464adfaa76B9B514093D23C27 (DEPLOYER)
FeesManagerV2.merkleRoot()        = 0x000…000
FeesManagerV2.rootValidFrom()     = 0
FeesManagerV2.rootValidUntil()    = 0
FeesManagerV2.rebateFundingAccount() = 0xa67f8E8E673ce4bb2Fb563B0e6E9FA8F70E3b588
FeesManagerV2.feeRecipient()      = 0xa67f8E8E673ce4bb2Fb563B0e6E9FA8F70E3b588
FeesManagerV2.rebateBudget(mUSDC) = 0
mUSDC.balanceOf(funder)           = 0
mUSDC.name()                      = "Mock USDC"
mUSDC.decimals()                  = 6
mUSDC.owner()                     = 0xc35F7A8A103A9A4464adfaa76B9B514093D23C27 (same DEPLOYER)
mUSDC.totalSupply()               = 20_002_000_000   (before top-up)
```

The mUSDC contract is the testnet `TestnetMockERC20` (from
`script/DeployTestnetAssets.s.sol`) with `mint(address, uint256)`
gated `onlyOwner`. Same DEPLOYER controls both contracts.

## Human Gate 1 — `setMerkleRoot` ✅

- tx: `0xddcd50c0dec5f7f4f64581a4a13f424961378f20caf8d47a26b400469827f494`
- block: `42193291`
- gas: 70 338 @ 0.006 gwei = `0.000000422 ETH`
- to: `0x00dA0B9876bcBf0c79CB5BcAcfEBAFb8C7Ad774f` (FeesManagerV2 only)
- caller: `0xc35F7A8A…` (DEPLOYER)
- status: `1` (success)

Post-broadcast verification (all green):

| Field | Expected | Observed |
|---|---|---|
| `merkleRoot()` | V2G-C root | `0xa29d5b1de46f…b1e2553` ✅ |
| `rootValidFrom()` | `1780099200` | `1780099200` ✅ |
| `rootValidUntil()` | `1781308800` | `1781308800` ✅ |
| `owner()` | unchanged | `0xc35F7A8A…` ✅ |
| `rebateFundingAccount()` | unchanged | `0xa67f…b588` ✅ |
| `feeRecipient()` | unchanged | `0xa67f…b588` ✅ |
| `rebateBudget(mUSDC)` | untouched | `0` ✅ |
| `mUSDC.balanceOf(funder)` | untouched | `0` ✅ |

## Human Gate 2 — mUSDC top-up to `rebateFundingAccount` ✅

Script: `script/TopUpRebateFundingAccount.s.sol` (new V2G-C
helper). Targets the testnet `TestnetMockERC20.mint` only.

- tx: `0x5b9aa6f4c73eb6700df884aff23e799f15502f299a260a0a7a834ecd5cc992fd`
- block: `42193525`
- gas: 53 176 @ 0.006 gwei = `0.000000319 ETH`
- to: `0x6eAe407f5640B006faC9965182e238582A3B412E` (mUSDC only)
- caller: `0xc35F7A8A…` (mUSDC.owner() == DEPLOYER)
- status: `1` (success)
- topped up: **`1_000_000` = 1 mUSDC**

Post-broadcast verification (all green):

| Field | Expected | Observed |
|---|---|---|
| `mUSDC.balanceOf(funder)` | `0 + 1_000_000` | `1_000_000` ✅ |
| `mUSDC.totalSupply()` | `+ 1_000_000` | `20_003_000_000` ✅ (was `20_002_000_000`) |
| `FeesManagerV2.merkleRoot()` | unchanged | `0xa29d5b1d…` ✅ |
| `FeesManagerV2.rebateBudget(mUSDC)` | untouched | `0` ✅ |
| `FeesManagerV2.feeRecipient()` | unchanged | `0xa67f…b588` ✅ |
| `FeesManagerV2.rebateFundingAccount()` | unchanged | `0xa67f…b588` ✅ |

## Human Gate 3 — `fundRebateBudget(mUSDC, 1_000_000)` ✅

- tx: `0xac24a417964f69dd0f2a20b1184f706e4ed3905343c791d186e9d90107843c99`
- block: `42193593`
- gas: 48 005 @ 0.006 gwei = `0.000000288 ETH`
- to: `0x00dA0B9876bcBf0c79CB5BcAcfEBAFb8C7Ad774f` (FeesManagerV2 only)
- caller: `0xc35F7A8A…` (DEPLOYER)
- status: `1` (success)
- accounting delta: `rebateBudget[mUSDC]` `0 → 1_000_000`

Confirmed accounting-only: no `IERC20.approve`, no token transfer.
The mUSDC balance held by the funding account is untouched.

Post-broadcast verification (all green):

| Field | Expected | Observed |
|---|---|---|
| `FeesManagerV2.rebateBudget(mUSDC)` | `1_000_000` | `1_000_000` ✅ |
| `mUSDC.balanceOf(funder)` | `1_000_000` (unchanged) | `1_000_000` ✅ |
| `FeesManagerV2.merkleRoot()` | unchanged | `0xa29d5b1d…` ✅ |
| `FeesManagerV2.rootValidFrom()` | unchanged | `1780099200` ✅ |
| `FeesManagerV2.rootValidUntil()` | unchanged | `1781308800` ✅ |
| `FeesManagerV2.feeRecipient()` | unchanged | `0xa67f…b588` ✅ |
| `FeesManagerV2.rebateFundingAccount()` | unchanged | `0xa67f…b588` ✅ |

## Post-Setup Preflight Progression (Phase 8)

All four dry-runs executed against the live chain immediately after
gate 3. **No broadcasts.**

### Tier 0 seller `claimTier` dry-run

Using SELLER_PRIVATE_KEY (`.env`-resident) as the claimant for row 2
(`0xbaf0…b1c3`, Tier 0, proof length 1):

```
FeesManagerV2.merkleRoot()                = 0xa29d5b1de46f…  (matches artifact)
FeesManagerV2.currentTier(account)        = 0  (unclaimed, fallback)
CLAIM_FEES_MANAGER_V2_TIER_CONFIRM not set; preflight done, no transactions sent.
```

Script completes without any of the `claimTier` reverts
(`NoMerkleRoot`, `ProofInvalid`, `TierNotYetValid`, `TierExpired`,
`CallerNotAccount`) — confirms the V2G-C artifact verifies
end-to-end against the live root.

### Tier 4 / Tier 2 `claimTier` dry-runs

The Tier 4 maker (`0x475f…bc0c`) and Tier 2 taker (`0x8b94…9a34`)
private keys are **not** in this repo's `.env`. The script
correctly reverts with `CallerNotAccount(...)` when invoked with a
mismatched key — confirming the safety check. The V2G-D milestone
must supply each row's own `CLAIMANT_PRIVATE_KEY`.

### `SmokePerpV2Rebate` preflight

```
PerpEngine.useFeesManagerV2()             = true
PerpEngine.feesManagerV2()                = 0x00dA0B9876…  (FEES_MANAGER_V2)
OldPerpEngine.useFeesManagerV2()          = false  (stranded)
FeesManagerV2.isFeeConsumer(NEW)          = true
FeesManagerV2.isFeeConsumer(OLD)          = false
FeesManagerV2.rebateBudget(mUSDC)         = 1_000_000  ← gate 3 result
FeesManagerV2.merkleRoot()                = 0xa29d5b1de46f…  ← gate 1 result
FeesManagerV2.currentTier(MAKER)          = 0
FeesManagerV2.currentTier(TAKER)          = 0
PERP makerPpm at MAKER tier (= 0)         = 50  (Tier 0 default — needs claim)
Error: script failed: MakerHasNoNegativeRebateTier(0, 50)
```

### `SmokeOptionV2Rebate` preflight

```
MarginEngine.useFeesManagerV2()           = true
MarginEngine.feesManagerV2()              = 0x00dA0B9876…  (FEES_MANAGER_V2)
FeesManagerV2.isFeeConsumer(MarginEngine) = true
FeesManagerV2.rebateBudget(mUSDC)         = 1_000_000  ← gate 3 result
FeesManagerV2.merkleRoot()                = 0xa29d5b1de46f…  ← gate 1 result
FeesManagerV2.currentTier(MAKER)          = 0
FeesManagerV2.currentTier(TAKER)          = 0
OPTION makerPpm at MAKER tier (= 0)       = 50  (Tier 0 default — needs claim)
Error: script failed: MakerHasNoNegativeRebateTier(0, 50)
```

Both preflights have **advanced past `MerkleRootUnset()`** to the
canonical V2G-D blocker. The V2F-LM acceptance state is intact
(NEW wired, OLD stranded, no fee-consumer drift), the V2G-C root
+ rebate budget are live, and the only remaining precondition is
`claimTier` per smoke account.

## Remaining human gates

| Gate | Action | Allowed signer | Expected tx | Blocker raised | Milestone |
|---|---|---|---|---|---|
| 4a | `claimTier` for Tier 4 maker (`0x475f…bc0c`) | Tier 4 account's own EOA | 1 | flips `currentTier` 0 → 4, flips `makerPpm` 50 → −100 (PERP), 50 → −50 (OPTION) | V2G-D |
| 4b | `claimTier` for Tier 2 taker (`0x8b94…9a34`) | Tier 2 account's own EOA | 1 | flips `currentTier` 0 → 2 | V2G-D |
| 5 | PERP rebate trade (V2D-V executor pattern) | matching engine + executor backend | 1 trade tx | emits FeeChargedV2 + FeeRebatedV2, decrements `rebateBudget(mUSDC)` | V2G-E |
| 6 | OPTION rebate trade (V2E-G executor pattern) | matching engine + executor backend | 1 trade tx | same | V2G-E |

Tier 0 claim (`0xbaf0…b1c3`) is optional — the contract returns
Tier 0 by default for unclaimed accounts.

The V2G-D milestone is **blocked on operator possession of the
Tier 4 + Tier 2 EOA private keys** (`0x475f…bc0c` and `0x8b94…9a34`).
Document this as the next human input.

## Files added / changed (V2G-C)

Backend:

- `fixtures/tier_snapshot/base_sepolia_v2g_c_smoke.json` — fresh fixture (NEW).
- `artifacts/tier_merkle/base_sepolia_v2g_c.json` — fresh artifact (NEW).
- `docs/FEES_MANAGER_V2_ROOT_BUDGET_SETUP_V2G_C.md` — this file (NEW).
- `docs/FEES_MANAGER_V2_REBATE_BROADCAST_PREFLIGHT_V2G_B.md` — updated with the V2G-C tx hashes and gate progression (see below).
- `docs/REBATE_LIVE_SMOKE_PLAN_V2G_A.md` — updated with the V2G-C reference (see below).

Solidity:

- `~/DEOPT/deopt-v2-sol/script/TopUpRebateFundingAccount.s.sol` — new V2G-C helper (NEW).

No `.env` mutation, no DB-row mutation, no Solidity-contract change,
no `--broadcast` from this agent.

## Validation commands run

```
cargo fmt --all                                                  ✅ no diff
cargo clippy --all-targets --all-features -- -D warnings         ✅ clean
cargo test --all-targets --all-features --no-fail-fast           ✅ all suites pass (405 lib tests)
cargo build --all-targets --all-features                         ✅ Finished
forge build                                                      ✅ Compiler run successful with warnings (preexisting lints)
forge script SetFeesManagerV2MerkleRoot --rpc-url $RPC_URL       ✅ preflight, then broadcast (gate 1)
forge script TopUpRebateFundingAccount --rpc-url $RPC_URL        ✅ preflight, then broadcast (gate 2)
forge script FundFeesManagerV2RebateBudget --rpc-url $RPC_URL    ✅ preflight, then broadcast (gate 3)
forge script ClaimFeesManagerV2Tier --rpc-url $RPC_URL           ✅ Tier 0 dry-run clean; Tier 4/2 reverts CallerNotAccount (no key in repo)
forge script SmokePerpV2Rebate --rpc-url $RPC_URL                ✅ advanced to MakerHasNoNegativeRebateTier(0, 50)
forge script SmokeOptionV2Rebate --rpc-url $RPC_URL              ✅ advanced to MakerHasNoNegativeRebateTier(0, 50)
```

## Exact blockers carried forward

1. **Tier 4 maker private key** (`0x475fe397fa56884952d350aa9ee1c3946964bc0c`)
   — required by `ClaimFeesManagerV2Tier.s.sol` for V2G-D gate 4a.
2. **Tier 2 taker private key** (`0x8b94a83d1ad3bd2337b1886e7962ca8e0bba9a34`)
   — required for V2G-D gate 4b.

No backend or contract blockers. All live state matches the V2G-C
artifact byte-for-byte; the V2F-LM acceptance state is intact.

## Next recommended milestone

**V2G-D — claimTier broadcasts**: operator runs gates 4a and 4b
using the per-account keys (one tx per account, signed by the
account itself). After completion, both smoke preflights flip
from `MakerHasNoNegativeRebateTier(0, 50)` to **clean pass**
(`makerPpm = -100` for PERP / `-50` for OPTION at Tier 4), and
V2G-E (the two rebate smoke trades through the V2D-V/V2E-G
executor patterns) becomes unlocked.

## V2G-D2 supersession (appended 2026-05-30)

V2G-D was **paused and superseded by V2G-D2** because
`PERP_SMOKE_SELLER_PRIVATE_KEY` (the Tier 4 maker key for
`0x475Fe397FA56884952D350aa9EE1c3946964BC0C`) is missing and
cannot be recovered. The V2G-C root commits to leaves nobody can
sign for, so the only safe path is to publish a new root keyed to
fresh operator-controlled EOAs.

V2G-D2 deliverables (see `FEES_MANAGER_V2_RECOVERY_V2G_D2.md`):

- New EOAs: `0x290bD12C93E467Bf51c51f5273D35bdDb19e9274` (Tier 4)
  and `0x77cA9DD6cCce2D692FB23877a2db7178807b0020` (Tier 2).
- New artifact: `artifacts/tier_merkle/base_sepolia_v2g_d2.json`,
  root `0xd8a627d7a9b600370e6f490fdd789150d7f9c4ea2f09752c88121d1f758fc2df`,
  window `1780099200 → 1781913600` (21 days).
- Live `rebateBudget(mUSDC) = 1_000_000` is **preserved**; the
  V2G-C `fundRebateBudget` broadcast (tx
  `0xac24a417…43c99`) is not re-run. The V2G-C `setMerkleRoot`
  broadcast (tx `0xddcd50c0…7f494`) is superseded by a single
  V2G-D2 `setMerkleRoot` broadcast that overwrites the root and
  extends the window.

V2G-D's "Tier 4 / Tier 2 reverts CallerNotAccount (no key in repo)"
line from the Phase 8 progression remains accurate as the
**trigger** for the V2G-D2 recovery; nothing about the V2G-C
on-chain state was wrong.
