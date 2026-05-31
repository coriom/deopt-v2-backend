# V2G-B — Tier Merkle Artifact Pipeline

## Status

- Milestone: **V2G-B** (follows V2G-A).
- Date: 2026-05-30.
- Mode: backend + Solidity dry-run scripts + docs. **No live chain
  mutation. No broadcast. No real `.env` edit. No DB row deletion.
  No deployed-contract change.**
- Outcome: a deterministic Base Sepolia tier Merkle artifact lands on
  disk; three dry-run Solidity scripts (`setMerkleRoot`,
  `fundRebateBudget`, `claimTier`) verify the live state against the
  artifact without broadcasting; two smoke-preflight scripts
  (`SmokePerpV2Rebate`, `SmokeOptionV2Rebate`) confirm the chain is
  pre-wired for rebate smoke and surface the exact missing input
  (`MerkleRootUnset`) as a hard precondition.

## Files added

Backend:

| Path | Purpose |
| --- | --- |
| `src/fees/tier_artifact.rs` | Artifact assembly + 10 regression tests. |
| `src/bin/generate_tier_artifact.rs` | CLI: `--input` JSON → `--output` artifact JSON. |
| `fixtures/tier_snapshot/base_sepolia_v2g_b_smoke.json` | Operator-editable input (V2F-LM smoke EOAs at Tier 4 / Tier 2 / Tier 0). |
| `artifacts/tier_merkle/base_sepolia_v2g_b.json` | Generated artifact pinned for V2G-B dry-runs. |

Solidity:

| Path | Purpose |
| --- | --- |
| `script/SetFeesManagerV2MerkleRoot.s.sol` | Dry-run preflight + gated broadcast for `setMerkleRoot`. |
| `script/FundFeesManagerV2RebateBudget.s.sol` | Dry-run preflight + gated broadcast for `fundRebateBudget` (accounting-only, no allowance). |
| `script/ClaimFeesManagerV2Tier.s.sol` | Dry-run preflight + gated broadcast for `claimTier`. |
| `script/SmokePerpV2Rebate.s.sol` | Read-only PERP rebate preflight. |
| `script/SmokeOptionV2Rebate.s.sol` | Read-only OPTION rebate preflight. |

Docs:

| Path | Purpose |
| --- | --- |
| `docs/TIER_MERKLE_ARTIFACT_PIPELINE_V2G_B.md` | This file. |
| `docs/FEES_MANAGER_V2_REBATE_BROADCAST_PREFLIGHT_V2G_B.md` | Human broadcast gate packet. |

`docs/TIER_MERKLE_REBATE_SYSTEM_V2G_A.md` and
`docs/REBATE_LIVE_SMOKE_PLAN_V2G_A.md` are updated with V2G-B
cross-references.

## CLI usage

```bash
cargo run --bin generate_tier_artifact -- \
    --input  fixtures/tier_snapshot/base_sepolia_v2g_b_smoke.json \
    --output artifacts/tier_merkle/base_sepolia_v2g_b.json
```

Stdout reports row count and root. The CLI never reads the live
chain, never accesses Postgres, never signs anything. Output is
deterministic up to `generated_at_ms` (which the CLI fills from the
system clock).

## Input fixture schema

`fixtures/tier_snapshot/base_sepolia_v2g_b_smoke.json` defines:

```json
{
  "chain_id": 84532,
  "fees_manager_v2": "0x00dA0B98…",
  "valid_from": 1748563200,
  "valid_until": 1749168000,
  "accounts": [
    {
      "label": "<human-readable, optional>",
      "trader": "0x…",
      "option_volume_28d_1e8": "1250000000000000",
      "perp_volume_28d_1e8":   "1250000000000000",
      "volume_share_ppm": 0,
      "staked_deopt_1e8": "0"
    }
  ]
}
```

The shipped fixture seeds three accounts that match the canonical
"Tier 4 maker rebate candidate + Tier 2 + Tier 0 control" target
from NEXT_TASK.md Part 1, using the live V2F-LM smoke trader
addresses (`0x475f…bc0c`, `0x8b94…9a34`) plus the V2D-V/V2E-G
`.env`-resident smoke seller (`0xbaf0…b1c3`) as the Tier 0 control.

**Operator note:** if the smoke target uses different EOAs, edit
this fixture, re-run the CLI, and re-issue all of Step 2 below.
The artifact is pure-function of this file and the system clock.

## Generated artifact schema

`artifacts/tier_merkle/base_sepolia_v2g_b.json` is the output of
`generate_tier_artifact`. Top-level fields:

| Field | Type | Notes |
| --- | --- | --- |
| `chain_id` | u64 | Base Sepolia = 84532. |
| `fees_manager_v2` | string (0x… address) | Lowercase hex. |
| `leaf_encoding_version` | string | `"v2g-a-1"` for this pipeline. |
| `generated_at_ms` | u64 | UNIX milliseconds from the CLI run. |
| `valid_from`, `valid_until` | u64 (UNIX seconds) | Mirrors `setMerkleRoot` window. |
| `merkle_root` | string (0x… bytes32) | The value to pass into `setMerkleRoot`. |
| `option_schedule`, `perp_schedule` | arrays of `FeeTier` | Canonical V2G-A launch schedule embedded verbatim. |
| `rows` | array of `TierArtifactRow` | One row per fixture account. |

Each `TierArtifactRow` flattens a `TierSnapshotRow` (full schema in
`docs/TIER_SNAPSHOT_SCHEMA_V2G_A.md`) and adds:

| Field | Type | Notes |
| --- | --- | --- |
| `leaf` | string (0x… bytes32) | `keccak256(abi.encode(account, tier, vol28d, sharePpm, stake, validFrom, validUntil))`. |
| `proof` | array of strings | Sorted-pair Merkle proof (sibling-from-leaf upwards), empty for single-leaf trees. |

## Generated artifact summary (current Base Sepolia run)

- **Root**: `0xef08543cd3e15e63345b4ef17eceb7431a3353b19366d7ee41866dcf479bab4f`
- **Rows**: 3
- **Window**: `validFrom = 1748563200` → `validUntil = 1749168000`
  (1 week)
- **Row 0** — trader `0x475f…bc0c`, option tier **4**, perp tier
  **4**, total 28d volume `2_500_000_000_000_000`, proof length 2.
- **Row 1** — trader `0x8b94…9a34`, option tier **2**, perp tier
  **2**, total volume `250_000_000_000_000`, proof length 2.
- **Row 2** — trader `0xbaf0…b1c3`, option tier **0**, perp tier
  **0**, total volume `10_000_000_000`, proof length 1.

Determinism is pinned by 10 backend regression tests in
`src/fees/tier_artifact.rs::tests`.

## Test coverage

| Test | Property |
| --- | --- |
| `artifact_is_deterministic` | Same inputs + same `generated_at_ms` → byte-identical artifact and JSON. |
| `embedded_leaves_rebuild_the_same_root` | Operator audit path: rebuild the tree from the row leaves alone and confirm the root. |
| `every_row_proof_verifies_against_root` | Every embedded proof verifies on the artifact's reported root. |
| `tampered_leaf_does_not_verify_against_artifact_root` | Flipping one byte in a leaf breaks verification (no-forgery contract). |
| `artifact_validity_window_is_ordered` | `valid_from < valid_until` on the artifact and on every row. |
| `highest_tier_wins_in_artifact_rows` | Multi-axis OR resolution surfaces the highest qualifying tier. |
| `artifact_embeds_canonical_launch_schedule` | Embedded schedules equal the V2G-A `launch_fee_schedule()`. |
| `artifact_pins_leaf_encoding_version_tag` | `"v2g-a-1"` is the only value the assembly emits. |
| `empty_inputs_are_rejected` | Empty input set returns `Err`, never produces a zero-row JSON. |
| `embedded_leaf_matches_explicit_tier_leaf_hash` | Each row's `leaf` field equals `tier_leaf(...)` of the same inputs. |

## Solidity dry-run scripts

All three follow the V2F-I `SetPerpMatchingEnginePaused` pattern:
default to **preflight-only** (sanitized snapshot, no transactions
sent); broadcast guarded by a single per-script confirm flag.

| Script | Confirm flag | What it broadcasts | Required role |
| --- | --- | --- | --- |
| `SetFeesManagerV2MerkleRoot` | `SET_FEES_MANAGER_V2_MERKLE_ROOT_CONFIRM=true` | `setMerkleRoot(root, validFrom, validUntil)` | `FeesManagerV2.owner()` |
| `FundFeesManagerV2RebateBudget` | `FUND_FEES_MANAGER_V2_REBATE_BUDGET_CONFIRM=true` | `fundRebateBudget(token, amount)` (accounting-only — no `approve`) | `FeesManagerV2.owner()` |
| `ClaimFeesManagerV2Tier` | `CLAIM_FEES_MANAGER_V2_TIER_CONFIRM=true` | `claimTier(...)` | The claimant EOA itself |

Hard refuses (all three):
- target address has no code;
- caller is not the required role (only checked under `confirmed=true`);
- post-state mismatch (root unchanged / budget unchanged / tier
  unchanged → reverts with a descriptive error).

`fundRebateBudget` is **accounting-only** in `FeesManagerV2`
(`rebateBudget[asset] += amount`); it does **not** pull ERC20
tokens. The script therefore **never** issues an `IERC20.approve`.
The optional `REBATE_TOKEN_BALANCE_CHECK=true` (default) reads
`IERC20.balanceOf(rebateFundingAccount)` so the operator can see
whether the funding account is liquid enough to cover the
would-be rebate disbursements during the smoke (the actual
ERC20 movement happens inside the consumer's settlement path, not
inside `FeesManagerV2`).

### Live dry-run results on Base Sepolia

`SetFeesManagerV2MerkleRoot` (preflight, no broadcast):

```
chainId 84532
caller (sanitized, no key) 0xc0A76c2A…  (BUYER smoke EOA — not the owner)
FEES_MANAGER_V2_ADDRESS 0x00dA0B9876…
FEES_MANAGER_V2_MERKLE_ROOT 0xef08543c…
FEES_MANAGER_V2_VALID_FROM  1748563200
FEES_MANAGER_V2_VALID_UNTIL 1749168000
State snapshot: before
 FeesManagerV2.owner() 0xc35F7A8A…  (DEPLOYER, V2D-S)
 FeesManagerV2.merkleRoot() 0x000…000
 FeesManagerV2.rootValidFrom() 0
 FeesManagerV2.rootValidUntil() 0
SET_FEES_MANAGER_V2_MERKLE_ROOT_CONFIRM not set; preflight done, no transactions sent.
```

`FundFeesManagerV2RebateBudget` (preflight, no broadcast):

```
REBATE_BUDGET_AMOUNT 100000000  (100 mUSDC, 6 decimals)
State snapshot: before
 FeesManagerV2.owner() 0xc35F7A8A…
 FeesManagerV2.rebateFundingAccount() 0xa67f8E8E…  (matches env)
 FeesManagerV2.rebateBudget(token) 0
 IERC20(token).balanceOf(rebateFundingAccount) 0  ← funding account holds NO mUSDC; needs minting before smoke
FUND_FEES_MANAGER_V2_REBATE_BUDGET_CONFIRM not set; preflight done, no transactions sent.
```

`ClaimFeesManagerV2Tier` (preflight, no broadcast):

```
caller (sanitized, no key) 0xbAf0976a…  (SELLER smoke EOA == CLAIM_ACCOUNT)
CLAIM_TIER 0
CLAIM_PROOF_LEN 1
CLAIM_PROOF_0 0x66ff7d75…
State snapshot: before
 FeesManagerV2.merkleRoot() 0x000…000   ← root not yet set
 FeesManagerV2.currentTier(account) 0
 FeesManagerV2.claimedTiers(account).validUntil 0
CLAIM_FEES_MANAGER_V2_TIER_CONFIRM not set; preflight done, no transactions sent.
```

## Smoke preflight scripts

| Script | Type | Verified live |
| --- | --- | --- |
| `SmokePerpV2Rebate` | Read-only preconditions checker | ✅ correctly diagnoses `MerkleRootUnset()` as the next blocker |
| `SmokeOptionV2Rebate` | Read-only preconditions checker | (same pattern) |

The PERP preflight against the live chain confirmed the V2F-LM
acceptance state intact:

```
PerpEngine.useFeesManagerV2() true
PerpEngine.feesManagerV2() 0x00dA0B98…  (== FEES_MANAGER_V2)
OldPerpEngine.useFeesManagerV2() false  (stranded)
FeesManagerV2.isFeeConsumer(NEW) true
FeesManagerV2.isFeeConsumer(OLD) false
FeesManagerV2.rebateBudget(token) 0
FeesManagerV2.merkleRoot() 0x000…000
FeesManagerV2.currentTier(MAKER) 0
PERP makerPpm at MAKER tier (negative = rebate) 50  ← positive 50 ppm; needs Tier ≥ 2
Error: script failed: MerkleRootUnset()
```

The preflight reverts with the exact precondition the next human
gate must satisfy. No live chain mutation occurred.

## Validation commands run

```
cargo fmt --all                                                  ✅ no diff
cargo clippy --all-targets --all-features -- -D warnings         ✅ clean
cargo test --all-targets --all-features --no-fail-fast           ✅ all suites pass (10 new artifact tests)
cargo build --all-targets --all-features                         ✅ Finished
forge build                                                       ✅ Compiler run successful
forge test --match-path test/fees/V2G_A_LeafCrossVector.t.sol    ✅ 1 passing (V2G-A cross-vector)
forge script SetFeesManagerV2MerkleRoot --rpc-url $RPC_URL        ✅ preflight, no tx
forge script FundFeesManagerV2RebateBudget --rpc-url $RPC_URL     ✅ preflight, no tx
forge script ClaimFeesManagerV2Tier --rpc-url $RPC_URL            ✅ preflight, no tx
forge script SmokePerpV2Rebate --rpc-url $RPC_URL                 ✅ correctly reverts MerkleRootUnset()
```

## Exact blockers

All are **human-signed-only** broadcasts (see
`docs/FEES_MANAGER_V2_REBATE_BROADCAST_PREFLIGHT_V2G_B.md` for the
full packet):

1. **`setMerkleRoot`** signed by the FeesManagerV2 owner
   (`0xc35F7A8A…`). Required input: the artifact `merkle_root`.
2. **mUSDC top-up to `rebateFundingAccount`** (`0xa67f8E8E…`).
   The funding account currently holds 0 mUSDC; needs at least
   the rebate budget amount before the smoke.
3. **`fundRebateBudget`** signed by the FeesManagerV2 owner.
4. **`claimTier`** signed by each smoke account (`0x475f…bc0c`,
   `0x8b94…9a34`, `0xbaf0…b1c3`). Proof + leaf data from the
   artifact rows.
5. **A tiny PERP trade** at Tier 4 maker via the backend
   executor.
6. **A tiny OPTION trade** at Tier 4 maker via the backend
   executor.

V2G-B carries none of these out; each is a separate human gate.

## V2G-C result (2026-05-30)

The V2G-B artifact (`base_sepolia_v2g_b.json`, root
`0xef08543c…`) was superseded by V2G-C because its window had
drifted into 2025. Operator-target artifact:
`artifacts/tier_merkle/base_sepolia_v2g_c.json` (root
`0xa29d5b1de46fb2e498999093400fd65c1240a6e8abeb106504e8bc4edb1e2553`,
window 2026-05-30 → 2026-06-13). All three V2G-B gate scripts
(`setMerkleRoot`, `fundRebateBudget`, plus the new V2G-C
`TopUpRebateFundingAccount`) successfully broadcast on Base
Sepolia under V2G-C. Full record:
`docs/FEES_MANAGER_V2_ROOT_BUDGET_SETUP_V2G_C.md`.

## Next recommended milestone

**V2G-C — Owner-signed root + budget setup**: produces a fresh
artifact with a same-day window, runs the three owner-signed
broadcasts under the V2G-B scripts with explicit `*_CONFIRM=true`,
and confirms the V2G-B smoke-preflight scripts then revert on the
next blocker (`MakerHasNoNegativeRebateTier` for an unclaimed
Tier 0 maker, etc.). No trades broadcast yet.

Then **V2G-D**: claimTier for all smoke accounts.

Then **V2G-E**: the two rebate smoke trades.
