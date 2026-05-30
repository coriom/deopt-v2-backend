# V2G-A — Rebate Live Smoke Plan

Operator runbook for taking the V2G-A backend artifacts live
on-chain. This plan **never broadcasts on its own**: every
on-chain step below is a human-signed transaction. The backend
work prepares the calldata, proof, and verification queries.

Repos:

- backend: `~/DEOPT/deopt-v2-backend`
- solidity: `~/DEOPT/deopt-v2-sol`
- frontend (optional): `~/DEOPT/deopt-v2-frontend`

Live state (Base Sepolia, post-V2F-LM):

| Contract | Address |
| --- | --- |
| `FEES_MANAGER_V2` | `0x00dA0B9876bcBf0c79CB5BcAcfEBAFb8C7Ad774f` |
| `NEW_PERP_ENGINE` | `0xc6C592100723Fe0C66343A16e95eC34cC0c2141c` |
| `NEW_MARGIN_ENGINE` | `0x287Cef479be5889eEfCa847F9e73C860898f48Cc` |
| `OLD_PERP_ENGINE` | `0xB36395b67D0798ADA981731c9Fa5239F4362b53B` (stranded A3) |

mUSDC (settlement asset) address: fetch from the deployment manifest
at smoke time (`base-sepolia.manifest.draft.json`); the smoke plan
treats it symbolically as `mUSDC`.

## Hard gates

- Do **not** broadcast from the backend host.
- Do **not** edit real `.env` files during smoke prep.
- Do **not** delete `option_execution_events` rows.
- Do **not** point at `OLD_PERP_ENGINE`.
- Do **not** print private keys; the operator types them into the
  signer at broadcast time, never into chat.

## Step 0 — Prerequisites (backend-only, no broadcast)

1. Decide the snapshot window. Suggested first run:
   - `validFrom = block.timestamp at smoke start`
   - `validUntil = validFrom + 7 * 86_400` (one week)
2. Decide the rebate budget. Suggested first run: `100 mUSDC`
   (with 6 decimals, that is `100_000_000`). Small enough that
   any contract regression is bounded; large enough that two
   Tier 4 rebates (≈ 0.01 % maker rate × small notional) leave a
   visible residual.
3. Decide the two smoke accounts. Reuse the V2D-V buyer/seller
   from `.env` so the existing key material covers signing.
4. Run the backend test suite once (`cargo test --all-targets
   --all-features --no-fail-fast`) and confirm the V2G-A
   suite is green; if any test fails, abort the smoke.

## Step 1 — Generate snapshot + Merkle artifacts (backend)

This step is **read-only** on chain. It produces JSON
artifacts; nothing is broadcast.

1. Construct `TraderInputs` so each smoke account qualifies
   Tier 4 via the volume axis alone (the simplest path):
   `option_volume_28d_1e8 = perp_volume_28d_1e8 = 12_500_000 *
   1e8` per account. Share and stake stay zero.
2. Call `generate_tier_snapshot(inputs, SnapshotConfig {
   valid_from, valid_until })` and save the result to
   `out/v2g_a_snapshot.json`. The CLI/binary for this lands in
   the V2G-B milestone (see "Next milestone" in
   `TIER_MERKLE_REBATE_SYSTEM_V2G_A.md`); for V2G-A the operator
   can call the function from a Rust scratch binary or from a
   `cargo test --features smoke-artifact` harness.
3. For each row build `TierLeafInputs` (`volume_28d_1e8` =
   total combined volume from the snapshot row, share/stake/
   tier/validFrom/validUntil from the same row).
4. Build the Merkle tree via
   `tier_merkle::MerkleTree::from_inputs(...)`, save root +
   per-account proof to `out/v2g_a_merkle.json`.
5. Run `tier_merkle::MerkleTree::verify_proof(leaf, &proof,
   root)` for every row as a sanity check; abort if any row
   fails.

Outputs:

```
out/v2g_a_snapshot.json     # sorted by trader; one row per account
out/v2g_a_merkle.json       # { root, per_account: { trader, proof[], tier } }
```

## Step 2 — `setMerkleRoot` (human-signed)

Calldata (preview only — operator signs the broadcast):

```
FeesManagerV2.setMerkleRoot(
    root,        // bytes32 from out/v2g_a_merkle.json
    validFrom,   // uint64 from the snapshot config
    validUntil   // uint64 from the snapshot config
)
```

Verification:

```
cast call $FEES_MANAGER_V2 'merkleRoot()(bytes32)'
cast call $FEES_MANAGER_V2 'rootValidFrom()(uint64)'
cast call $FEES_MANAGER_V2 'rootValidUntil()(uint64)'
```

Acceptance: each return value equals the operator-signed input.

Watch for:

- `setMerkleRoot` reverts on `InvalidMerkleRootWindow` if
  `validFrom > validUntil`. Backend snapshot config enforces
  ordering; if the operator passes wrong values, abort and
  regenerate.

## Step 3 — Ensure rebate funding plumbing (human-signed)

Pre-check (read-only):

```
cast call $FEES_MANAGER_V2 'rebateFundingAccount()(address)'
```

If zero, the owner must call
`setRebateFundingAccount(funder)` first. `funder` is the EOA
that will hold and approve the rebate budget.

Funder pre-funds and approves:

```
cast send $MUSDC 'approve(address,uint256)' \
    $FEES_MANAGER_V2 $BUDGET_AMOUNT --from $FUNDER   # signed by funder
```

## Step 4 — `fundRebateBudget` (human-signed by owner)

```
FeesManagerV2.fundRebateBudget(mUSDC, BUDGET_AMOUNT)
```

Verification:

```
cast call $FEES_MANAGER_V2 'rebateBudget(address)(uint256)' $MUSDC
```

Acceptance: return value increases by `BUDGET_AMOUNT`.

Watch for the `RebateBudgetFunded(mUSDC, BUDGET_AMOUNT)` event.
The V2F-N indexer already decodes it; the backend's
`option_execution_events` table should pick it up on the next
indexer tick.

## Step 5 — `claimTier` per account (human-signed by each account)

Each smoke account calls:

```
FeesManagerV2.claimTier(
    account,
    tier,           // 4
    volume28d,      // from snapshot row, native uint256
    volumeSharePpm, // from snapshot row, uint32
    stakedDeopt,    // from snapshot row, native uint256
    validFrom,
    validUntil,
    proof           // from out/v2g_a_merkle.json
)
```

Verification per account:

```
cast call $FEES_MANAGER_V2 'currentTier(address)(uint8)' $ACCOUNT
cast call $FEES_MANAGER_V2 'claimedTiers(address)((uint8,uint64))' $ACCOUNT
```

Acceptance: `currentTier()` returns `4` and the
`(tier, validUntil)` tuple matches the snapshot.

Watch for `TierClaimed(account, tier, validUntil)` events.

## Step 6 — PERP rebate smoke (human-signed)

Maker = one of the smoke accounts (Tier 4 → -100 ppm rebate).
Taker = the other (will pay 150 ppm).

1. Maker posts a tiny PERP order via the existing matching-engine
   flow. The V2D-V broadcast pattern in
   `docs/MARGIN_ENGINE_V2_TINY_TRADE_BROADCAST_RESULT_V2D_V.md`
   is the closest template.
2. Taker submits the crossing order.
3. The trade clears; `PerpEngineV2.executeTrade` calls
   `FeesManagerV2.consumeFees(...)` for each leg.

Expected on-chain events (V2F-N decoder catches both):

```
FeeChargedV2(
    consumer = NEW_PERP_ENGINE,
    trader   = taker,
    recipient = FEE_RECIPIENT,
    productKind = PERP,
    flowKind = ORDERBOOK,
    isMaker = false,
    feePpm = 150,
    basisAmount = notional,
    feeAmount = ceil(notional * 150 / 1e6)
)
FeeRebatedV2(
    consumer = NEW_PERP_ENGINE,
    trader   = maker,
    recipient = maker,
    productKind = PERP,
    flowKind = ORDERBOOK,
    rebatePpm = -100,
    basisAmount = notional,
    rebateAmount = floor(notional * 100 / 1e6)
)
RebateBudgetSpent(mUSDC, rebateAmount)
```

Verification:

```
# 1. Check the per-tx admin summary picks up the rebate.
curl -s "http://127.0.0.1:8080/admin/fees/onchain?tx_hash=<tx>" \
    -H "X-Admin-Token: $ADMIN_API_TOKEN" | jq .

# 2. Check the rebate budget decreased.
cast call $FEES_MANAGER_V2 'rebateBudget(address)(uint256)' $MUSDC

# 3. Check the maker vault delta (depends on settlement plumbing;
#    see the Solidity vault contract for the exact view).

# 4. Scrape /metrics and confirm the V2F-P/V2F-Q counters increment:
curl -s http://127.0.0.1:8080/metrics \
    | grep -E 'deopt_perp_fee_(charged|rebated)_v2_total\{consumer="new"\}'
```

Acceptance:

- `event_model = "v2"`, `fee_charged_v2_count = 1`,
  `fee_rebated_v2_count = 1`.
- `observed_total_charged = ceil(notional * 150 / 1e6)`.
- `observed_total_rebated = floor(notional * 100 / 1e6)`.
- `consumer="new"` arms of both counters advanced by 1.
- `rebateBudget(mUSDC)` decreased by the rebate amount.

## Step 7 — OPTION rebate smoke (human-signed)

Identical shape, just on OPTION. Reuse the V2E-G option broadcast
pattern (`docs/FEES_MANAGER_V2_TINY_TRADE_BROADCAST_RESULT_V2E_G.md`).

Expected event shape:

```
FeeChargedV2(productKind = OPTION, flowKind = ORDERBOOK,
             feePpm = 75 (Tier 4 taker), …)
FeeRebatedV2(productKind = OPTION, flowKind = ORDERBOOK,
             rebatePpm = -50 (Tier 4 maker rebate), …)
```

Same admin / metrics / rebateBudget assertions.

## Failure modes & remediation

| Symptom | Cause | Remediation |
| --- | --- | --- |
| `claimTier` reverts with `ProofInvalid` | Proof / leaf order drift | Regenerate from `out/v2g_a_snapshot.json`; abort smoke until V2G-A unit tests pass. |
| `consumeFees` reverts with `InsufficientRebateBudget` | Budget exhausted mid-smoke | Owner calls `fundRebateBudget` again; rerun. **Do not** weaken the revert. |
| `currentTier()` returns `0` after claim | Window expired (`validUntil < block.timestamp`) | Regenerate snapshot with a fresh window, set a new root. |
| Indexer doesn't pick up the rebate | Indexer cursor behind | Run admin tick (`POST /admin/options/events/tick`); see V2F-O for the catch-up pattern. |
| `consumer=="unknown"` counter increments | OLD or third-party engine emitted the event | Stop. Follow `docs/RUNBOOK_PERP_V2_FEE_ALERTS.md` → `PerpFeeConsumerUnknown`. |

## After-action

Once all four legs (PERP charge + PERP rebate + OPTION charge +
OPTION rebate) are observed, capture:

- the tx hashes,
- the admin `/admin/fees/onchain?tx_hash=…` JSON per tx,
- the metric snapshot (`deopt_perp_fee_*_v2_total{consumer}`),
- the on-chain `rebateBudget(mUSDC)` before/after.

These land in `docs/REBATE_LIVE_SMOKE_RESULT_V2G_*.md` (one file
per smoke run); name them with the milestone that ran the smoke
(e.g. V2G-B for the first dry-run, V2G-C for the broadcast).
Carry the OLD-stranded alert state forward in every result doc.
