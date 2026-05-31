# V2G-B — FeesManagerV2 Rebate Broadcast Gate Packet

This is the operator-facing checklist for taking the V2G-B artifacts
live. Each gate below is a **human-signed broadcast**; V2G-B itself
never broadcasts. Every gate is preflighted by an idempotent
read-only Solidity script (verified live on Base Sepolia under V2G-B
— see `docs/TIER_MERKLE_ARTIFACT_PIPELINE_V2G_B.md`).

## V2G-C result (2026-05-30)

Gates 1–3 of this packet were executed in the V2G-C milestone with
a **fresh artifact** (`artifacts/tier_merkle/base_sepolia_v2g_c.json`,
root `0xa29d5b1de46fb2e498999093400fd65c1240a6e8abeb106504e8bc4edb1e2553`,
window 2026-05-30 → 2026-06-13) because the V2G-B artifact window
had drifted into 2025 and would have reverted with `TierExpired`.

| Gate | Tx | Block | Status |
|---|---|---|---|
| 1 — `setMerkleRoot` | `0xddcd50c0…f494` | 42193291 | ✅ |
| 2 — mUSDC top-up (1 mUSDC) | `0x5b9aa6f4…92fd` | 42193525 | ✅ |
| 3 — `fundRebateBudget(1 mUSDC)` | `0xac24a417…3c99` | 42193593 | ✅ |

Gates 4–6 (claimTier × 2, then PERP + OPTION rebate trades) carry
forward to V2G-D / V2G-E. See
`docs/FEES_MANAGER_V2_ROOT_BUDGET_SETUP_V2G_C.md` for the full
broadcast record and post-state verification.

The smoke preflights now revert with
`MakerHasNoNegativeRebateTier(0, 50)` — the exact V2G-D blocker.

## Hard gates (carry forward to every step)

- Do **not** broadcast from the backend host. Use the operator
  signer workstation.
- Do **not** print private keys; type them into the signer at
  broadcast time, never into chat or scripts.
- Do **not** edit real `.env` during the smoke. Snapshot the
  running config into the incident channel; roll config changes
  through normal deployment after the smoke.
- Do **not** delete `option_execution_events` rows.
- Do **not** point any script at `OLD_PERP_ENGINE`.
- Do **not** weaken any of the per-script preflight
  `*_CONFIRM` gates.

## Pinned live state (verified V2G-B preflight, Base Sepolia)

| Contract / signer | Address |
| --- | --- |
| `FEES_MANAGER_V2_ADDRESS` | `0x00dA0B9876bcBf0c79CB5BcAcfEBAFb8C7Ad774f` |
| `FeesManagerV2.owner()` | `0xc35F7A8A103A9A4464adfaa76B9B514093D23C27` (DEPLOYER) |
| `FeesManagerV2.rebateFundingAccount()` | `0xa67f8E8E673ce4bb2Fb563B0e6E9FA8F70E3b588` |
| `feeRecipient` | `0xa67f8E8E673ce4bb2Fb563B0e6E9FA8F70E3b588` |
| `NEW_PERP_ENGINE` (use this) | `0xc6C592100723Fe0C66343A16e95eC34cC0c2141c` |
| `OLD_PERP_ENGINE` (do not use) | `0xB36395b67D0798ADA981731c9Fa5239F4362b53B` |
| `NEW_MARGIN_ENGINE` | `0x287Cef479be5889eEfCa847F9e73C860898f48Cc` |
| `REBATE_TOKEN` (mUSDC) | `0x6eAe407f5640B006faC9965182e238582A3B412E` |
| `FeesManagerV2.merkleRoot()` | `0x000…000` (unset; gate 1 sets it) |
| `FeesManagerV2.rebateBudget(mUSDC)` | `0` (unset; gate 3 funds it) |
| `IERC20(mUSDC).balanceOf(rebateFundingAccount)` | `0` (gate 2 tops it up) |

Pinned artifact:

- File: `artifacts/tier_merkle/base_sepolia_v2g_b.json`
- Root: `0xef08543cd3e15e63345b4ef17eceb7431a3353b19366d7ee41866dcf479bab4f`
- Window: `validFrom = 1748563200` → `validUntil = 1749168000`
- Rows: 3 (Tier 4 / Tier 2 / Tier 0)

Operator **must** verify the window is still in the future at gate
1 time; if `validFrom < now() - safety_margin`, regenerate the
artifact with a fresh window before signing.

## Gate 1 — `setMerkleRoot`

Exact command:

```bash
DEPLOYER_PRIVATE_KEY=<owner key, never written to disk> \
FEES_MANAGER_V2_ADDRESS=0x00dA0B9876bcBf0c79CB5BcAcfEBAFb8C7Ad774f \
FEES_MANAGER_V2_MERKLE_ROOT=0xef08543cd3e15e63345b4ef17eceb7431a3353b19366d7ee41866dcf479bab4f \
FEES_MANAGER_V2_VALID_FROM=1748563200 \
FEES_MANAGER_V2_VALID_UNTIL=1749168000 \
SET_FEES_MANAGER_V2_MERKLE_ROOT_CONFIRM=true \
forge script script/SetFeesManagerV2MerkleRoot.s.sol:SetFeesManagerV2MerkleRoot \
    --rpc-url $RPC_URL --broadcast
```

- Expected tx count: **1**.
- Allowed target: `FEES_MANAGER_V2_ADDRESS`.
- Forbidden targets: any other address. Script reverts otherwise.
- Allowed signer: `FeesManagerV2.owner()` only. Anyone else trips
  `NotOwner(caller, owner)`.
- Post-broadcast verification (run before gate 2):

  ```bash
  cast call $FEES_MANAGER_V2_ADDRESS 'merkleRoot()(bytes32)'
  cast call $FEES_MANAGER_V2_ADDRESS 'rootValidFrom()(uint64)'
  cast call $FEES_MANAGER_V2_ADDRESS 'rootValidUntil()(uint64)'
  ```

  Each return value must equal the input above. Rerun the V2G-B
  smoke preflight; it should now revert with the **next** missing
  precondition (typically `MakerHasNoNegativeRebateTier(0, 50)`).

## Gate 2 — mUSDC top-up to `rebateFundingAccount`

The rebate funding account `0xa67f…b588` currently holds **0
mUSDC**. Before gate 3, top it up to at least the planned rebate
budget (suggested: `100_000_000` = 100 mUSDC, 6 decimals).

Exact command (mUSDC contract has a public mint on Base Sepolia
testnet; operator picks the right one for the deployment):

```bash
cast send $REBATE_TOKEN 'mint(address,uint256)' \
    $REBATE_FUNDING_ACCOUNT $REBATE_BUDGET_AMOUNT \
    --rpc-url $RPC_URL --private-key <mUSDC minter key>
```

- Expected tx count: **1**.
- Allowed target: `REBATE_TOKEN` (mUSDC) only.
- Forbidden targets: any other address.
- Post-broadcast verification:

  ```bash
  cast call $REBATE_TOKEN 'balanceOf(address)(uint256)' $REBATE_FUNDING_ACCOUNT
  ```

  Must return at least `REBATE_BUDGET_AMOUNT`.

## Gate 3 — `fundRebateBudget`

Exact command:

```bash
DEPLOYER_PRIVATE_KEY=<owner key> \
FEES_MANAGER_V2_ADDRESS=0x00dA0B9876bcBf0c79CB5BcAcfEBAFb8C7Ad774f \
REBATE_TOKEN=0x6eAe407f5640B006faC9965182e238582A3B412E \
REBATE_BUDGET_AMOUNT=100000000 \
REBATE_FUNDING_ACCOUNT=0xa67f8E8E673ce4bb2Fb563B0e6E9FA8F70E3b588 \
FUND_FEES_MANAGER_V2_REBATE_BUDGET_CONFIRM=true \
forge script script/FundFeesManagerV2RebateBudget.s.sol:FundFeesManagerV2RebateBudget \
    --rpc-url $RPC_URL --broadcast
```

- Expected tx count: **1**.
- Allowed target: `FEES_MANAGER_V2_ADDRESS`.
- Forbidden targets: any other address. **In particular, this
  script never calls `IERC20.approve`** — `fundRebateBudget` is
  accounting-only (`rebateBudget[asset] += amount` on the
  contract; no token transfer in `FeesManagerV2`).
- Allowed signer: `FeesManagerV2.owner()` only.
- Post-broadcast verification:

  ```bash
  cast call $FEES_MANAGER_V2_ADDRESS 'rebateBudget(address)(uint256)' $REBATE_TOKEN
  ```

  Must equal the input `REBATE_BUDGET_AMOUNT`.

## Gate 4a..4c — `claimTier` per smoke account

One transaction per account. The operator runs these from the
**claimant's own EOA**, not from the owner.

```bash
# Each invocation reads from the artifact row for that account.
CLAIMANT_PRIVATE_KEY=<account key> \
FEES_MANAGER_V2_ADDRESS=0x00dA0B9876bcBf0c79CB5BcAcfEBAFb8C7Ad774f \
CLAIM_ACCOUNT=<trader from row> \
CLAIM_TIER=<row.option_tier (== row.perp_tier for current snapshot)> \
CLAIM_VOLUME_28D=<row.total_28d_volume_1e8> \
CLAIM_VOLUME_SHARE_PPM=<row.volume_share_ppm> \
CLAIM_STAKED_DEOPT=<row.staked_deopt_1e8> \
CLAIM_VALID_FROM=<row.valid_from> \
CLAIM_VALID_UNTIL=<row.valid_until> \
CLAIM_PROOF_LEN=<row.proof.length> \
CLAIM_PROOF_0=<row.proof[0]> \
[CLAIM_PROOF_1=<row.proof[1]>] \
CLAIM_FEES_MANAGER_V2_TIER_CONFIRM=true \
forge script script/ClaimFeesManagerV2Tier.s.sol:ClaimFeesManagerV2Tier \
    --rpc-url $RPC_URL --broadcast
```

- Expected tx count: **3 total** (one per smoke account; Tier 4 +
  Tier 2 + Tier 0).
- Allowed target: `FEES_MANAGER_V2_ADDRESS`.
- Forbidden targets: any other address.
- Allowed signer: each row's `trader` EOA only; the script
  reverts otherwise.
- Post-broadcast verification:

  ```bash
  cast call $FEES_MANAGER_V2_ADDRESS 'currentTier(address)(uint8)' <trader>
  cast call $FEES_MANAGER_V2_ADDRESS 'claimedTiers(address)((uint8,uint64))' <trader>
  ```

  Must return the row's tier and `validUntil` exactly.

Note: claiming Tier 0 explicitly is optional — the contract
already returns Tier 0 for unclaimed accounts. The Tier 0 row in
the artifact exists as a sanity check; the operator can skip its
claim broadcast and the smoke still works.

## Gate 5 — PERP rebate smoke trade

Driven by the backend executor (V2D-V / V2E-G pattern), not by a
Solidity script. Preflight first:

```bash
FEES_MANAGER_V2_ADDRESS=0x00dA0B9876bcBf0c79CB5BcAcfEBAFb8C7Ad774f \
PERP_ENGINE=0xc6C592100723Fe0C66343A16e95eC34cC0c2141c \
OLD_PERP_ENGINE=0xB36395b67D0798ADA981731c9Fa5239F4362b53B \
REBATE_TOKEN=0x6eAe407f5640B006faC9965182e238582A3B412E \
MAKER_ACCOUNT=<row[0].trader, Tier 4 candidate> \
TAKER_ACCOUNT=<row[2].trader, Tier 0 control> \
MIN_REBATE_BUDGET=10 \
forge script script/SmokePerpV2Rebate.s.sol:SmokePerpV2Rebate \
    --rpc-url $RPC_URL
```

This must pass (no revert) before the operator broadcasts the
trade.

Trade broadcast: identical to the V2D-V tiny PERP trade pattern
(see `docs/MARGIN_ENGINE_V2_TINY_TRADE_BROADCAST_RESULT_V2D_V.md`).
The maker is the Tier 4 account; the taker crosses. Expected
events:

```
FeeChargedV2(consumer = NEW_PERP_ENGINE,
             trader   = taker,
             productKind = PERP, flowKind = ORDERBOOK,
             feePpm = 150, basisAmount = notional,
             feeAmount = ceil(notional * 150 / 1e6))
FeeRebatedV2(consumer = NEW_PERP_ENGINE,
             trader   = maker,
             productKind = PERP, flowKind = ORDERBOOK,
             rebatePpm = -100, basisAmount = notional,
             rebateAmount = floor(notional * 100 / 1e6))
RebateBudgetSpent(mUSDC, rebateAmount)
```

Post-broadcast verification:

```bash
curl -s "http://127.0.0.1:8080/admin/fees/onchain?tx_hash=<tx>" \
    -H "X-Admin-Token: $ADMIN_API_TOKEN" | jq .
curl -s http://127.0.0.1:8080/metrics \
    | grep -E 'deopt_perp_fee_(charged|rebated)_v2_total\{consumer="new"\}'
cast call $FEES_MANAGER_V2_ADDRESS 'rebateBudget(address)(uint256)' $REBATE_TOKEN
```

Acceptance:
- `event_model == "v2"`,
  `fee_charged_v2_count == 1`,
  `fee_rebated_v2_count == 1`.
- `observed_total_charged == ceil(notional * 150 / 1e6)`.
- `observed_total_rebated == floor(notional * 100 / 1e6)`.
- Both `consumer="new"` metric arms advanced by 1.
- `rebateBudget(mUSDC)` decreased by the rebate amount.

## Gate 6 — OPTION rebate smoke trade

Same pattern as gate 5, with `SmokeOptionV2Rebate` preflight and
the V2E-G OPTION executor for the broadcast. Expected event ppm
values: taker `feePpm=75` (Tier 4 taker), maker `rebatePpm=-50`
(Tier 4 maker rebate).

## Failure-mode quick reference

| Symptom | Source | Likely cause | Action |
| --- | --- | --- | --- |
| `MerkleRootUnset()` from preflight | smoke preflight | Gate 1 not done | Run gate 1. |
| `MakerHasNoNegativeRebateTier(0, 50)` | smoke preflight | Maker still at Tier 0 | Run gate 4a for the Tier 4 account. |
| `RebateBudgetBelowMinimum(0, N)` | smoke preflight | Gate 3 not done | Run gate 3. |
| `RebateFundingAccountUnset()` | `fundRebateBudget` script | rebateFundingAccount = 0 | Owner-signed `setRebateFundingAccount(0xa67f…)`. |
| `NotOwner(caller, owner)` | set/fund scripts | Wrong key | Use the DEPLOYER key. |
| `CallerNotAccount(caller, account)` | claim script | Wrong key for the claim row | Use the row trader's key. |
| `ProofInvalid()` from `claimTier` | live broadcast | Drifted artifact / wrong row | Regenerate artifact; never edit by hand. |
| `InsufficientRebateBudget(...)` mid-trade | live trade | Budget exhausted | Owner re-funds via gate 3 with a higher amount. **Do not** weaken the revert. |

## After-action

Capture for each completed gate:

- the tx hash;
- the `cast call` verification outputs (root / budget / tier /
  balance);
- the relevant slice of `/admin/fees/onchain?tx_hash=…` for gates
  5 + 6;
- the V2F-P / V2F-Q metric snapshot.

Promote the run into `docs/REBATE_SMOKE_DRY_RUN_V2G_C.md` (or the
next milestone label) with all of the above plus a forward
reference to this packet.

## V2G-D2 supersession (appended 2026-05-30)

The Tier 4 / Tier 2 claimant addresses pinned in this packet
(`0x475Fe397…BC0C`, `0x8B94A83D…9A34`) cannot be claimed for —
`PERP_SMOKE_SELLER_PRIVATE_KEY` is missing. **V2G-D2** publishes a
new root (`0xd8a627d7a9b600370e6f490fdd789150d7f9c4ea2f09752c88121d1f758fc2df`,
window `1780099200 → 1781913600`) keyed to fresh operator-controlled
EOAs (`0x290bD12C…9274` Tier 4, `0x77cA9DD6…0020` Tier 2). All
operator gates from this packet are reissued in
`docs/FEES_MANAGER_V2_RECOVERY_V2G_D2.md` with the new claim
packets and proofs. The V2G-C-set `rebateBudget(mUSDC) = 1_000_000`
is preserved.
