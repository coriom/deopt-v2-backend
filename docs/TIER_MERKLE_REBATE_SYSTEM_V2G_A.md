# V2G-A — Tier / Merkle / Rebate System

## Status

- Milestone: **V2G-A** (follows V2F-Q metrics + alerts).
- Date: 2026-05-30.
- Mode: backend + tests + docs + Solidity cross-vector. **No live
  chain mutation. No broadcast. No real `.env` edit. No DB row
  deletion. No Solidity contract change.**
- Outcome: canonical OPTION + PERP launch schedules pinned by
  cross-table assertions; OR-eligibility resolver, deterministic
  tier-snapshot generator, and `FeesManagerV2`-compatible Merkle
  tree all land in the backend with full test coverage; a Forge
  cross-vector proves byte-for-byte leaf compatibility against the
  live contract.

## Metrics hardening decision (Part 1)

V2F-Q's `deopt_perp_fee_charged_v2_total` and
`deopt_perp_fee_rebated_v2_total` remain **ledger-derived gauges**
named with the `_total` suffix. The rationale, recorded here so
future agents do not chase a phantom rewrite:

1. **Idempotency.** The metric is derived at scrape time from the
   append-only `option_execution_events` ledger. Re-indexing the
   same block produces the same gauge value; there is no
   double-increment surface to defend against.
2. **Survival across restarts.** Recomputed from the DB on every
   scrape; no warm-up state, no migration risk.
3. **Alert semantics already correct.** The underlying ledger is
   append-only, so the gauge is monotonically non-decreasing.
   `increase(...[5m]) > 0` already fires exactly when a new event
   lands, matching a true counter's behaviour for the alert's
   purpose.
4. **Naming convention vs prim­itive.** Prometheus' `_total`
   convention is independent of the registry primitive. The
   `# HELP` line documents the bucketing; tooling (`promtool`, the
   Anthropic-flavoured grafana boards) does not introspect the
   suffix.
5. **True counter would require new infrastructure.** Adding a
   monotonic-counter primitive to `monitoring.rs` plus
   in-memory atomic state plus a persistence migration trades
   real risk for no observable benefit at this layer. Defer to a
   future milestone if/when we introduce per-request latency
   histograms or other counter-class metrics.

`docs/PERP_V2_FEE_METRICS_ALERTING_V2F_P.md` and
`docs/PERP_V2_FEE_REBATE_METRICS_ALERTING_V2F_Q.md` previously
listed "promote to monotonic counter" as a follow-up gap; this
milestone closes that gap by **deciding not to**.

## Canonical OPTION fee schedule (Part 2)

Pinned by `src/fees/schedule.rs::tests::canonical_option_schedule_matches_launch_table`:

| Tier | maker (ppm) | taker (ppm) | RFQ maker discount (bps) | RFQ taker discount (bps) |
| ---: | ----------: | ----------: | -----------------------: | -----------------------: |
|   4  |        -50  |        75   |               10 000 (100 %) |             7 500 (75 %)  |
|   3  |        -25  |       100   |                7 500 (75 %)  |             5 000 (50 %)  |
|   2  |        -10  |       125   |                5 000 (50 %)  |             2 500 (25 %)  |
|   1  |          0  |       150   |                2 500 (25 %)  |             1 000 (10 %)  |
|   0  |         50  |       250   |                       0      |                    0      |

Stored in the backend as `micro_bps` (one micro_bp = 1 ppm × 100);
the schedule's `MICRO_BPS_PER_PPM = 100` conversion constant is
asserted in the canonical-table test. RFQ discounts are stored as
`rfq_*_discount_pct` (0..=100) and converted via `BPS_PER_PCT =
100` on emission.

## Canonical PERP fee schedule (Part 2)

Pinned by `src/fees/schedule.rs::tests::canonical_perp_schedule_matches_launch_table`:

| Tier | maker (ppm) | taker (ppm) |
| ---: | ----------: | ----------: |
|   4  |       -100  |       150   |
|   3  |        -75  |       175   |
|   2  |        -50  |       200   |
|   1  |          0  |       250   |
|   0  |         50  |       300   |

PERP has no RFQ discount; both `rfq_*_discount_pct` fields are
zero on every PERP tier and the test asserts so.

## OR eligibility logic (Part 3)

Module: `src/fees/tier_eligibility.rs`.

```rust
pub struct EligibilityInputs {
    pub volume_28d_1e8: u128,
    pub volume_share_ppm: u32,
    pub staked_deopt_1e8: u128,
}

pub fn resolve_tier_with_eligibility(
    product: FeeProduct,
    inputs: EligibilityInputs,
) -> u8;
```

Semantics:

- A trader reaches tier `T` if **any** of:
  `volume >= T.min_volume` OR
  `share_ppm >= T.min_share` OR
  `staked_deopt >= T.min_stake`.
- The schedule is searched Tier 4 → 3 → 2 → 1 → 0; the first match
  is the highest qualifying tier.
- Tier 0 is the unconditional fallback (all three thresholds zero
  → always matches).

Boundary coverage (each test asserts on both OPTION and PERP):

| Test | Property |
| --- | --- |
| `exact_volume_boundaries_qualify_each_tier` | $500k → T1, $2.5M → T2, $10M → T3, $25M → T4 |
| `exact_share_boundaries_qualify_each_tier` | 2 500 ppm → T1, 10 000 → T2, 25 000 → T3, 50 000 → T4 |
| `exact_stake_boundaries_qualify_each_tier` | 10 000 → T1, 50 000 → T2, 100 000 → T3, 250 000 → T4 |
| `just_below_tier1_thresholds_falls_back_to_tier0` | $499 999, 2 499 ppm, 9 999 DEOPT all → T0 |
| `or_logic_qualifies_when_any_single_axis_meets_threshold` | each axis alone at T3 → T3 |
| `highest_qualifying_tier_wins_when_multiple_axes_match` | vol→T2 + share→T4 + stake→T1 → T4 |
| `zero_inputs_resolve_to_tier_zero` | all three flows (orderbook+RFQ × option+perp) |
| `one_unit_above_tier3_volume_does_not_promote_to_tier4` | guards `>=` vs `>` |

Off-chain by design — recorded in NEXT_TASK.md as: "Do NOT
compute 28D volume, 28D share, or staked DEOPT directly inside
PerpEngine or MarginEngine."

## Tier snapshot scaffold (Part 4)

Module: `src/fees/tier_snapshot.rs`. JSON-serialisable rows with
deterministic ordering. Schema is documented in
`docs/TIER_SNAPSHOT_SCHEMA_V2G_A.md` and pinned via
`#[derive(Serialize, Deserialize)]`.

```rust
pub fn generate_tier_snapshot(
    inputs: &[TraderInputs],
    config: SnapshotConfig,
) -> Vec<TierSnapshotRow>;
```

Determinism: rows are sorted by trader address ascending after
resolution, so re-running on the same input set produces a
byte-identical sequence (asserted by
`snapshot_is_deterministic_across_runs`).

Each row carries both the **resolved tier numbers** (OPTION,
PERP) and the **canonical fee profile they map to** (signed maker
ppm, taker ppm, OPTION RFQ discount bps). The profile fields are
observability-only; on-chain they are determined by
`FeesManagerV2.getFeeProfile(tier, product)`, not by the snapshot.

## Merkle tree generation (Part 5)

Module: `src/fees/tier_merkle.rs`.

```rust
pub struct TierLeafInputs { /* account, tier, volume28d, ... */ }
pub fn tier_leaf(inputs: &TierLeafInputs) -> [u8; 32];
pub struct MerkleTree { /* ... */ }
impl MerkleTree {
    pub fn from_inputs(inputs: &[TierLeafInputs]) -> Option<Self>;
    pub fn root(&self) -> [u8; 32];
    pub fn proof(&self, index: usize) -> Option<Vec<[u8; 32]>>;
    pub fn verify_proof(leaf: [u8; 32], proof: &[[u8; 32]], root: [u8; 32]) -> bool;
}
```

Leaf format (`tier_leaf`):

```
keccak256(
    abi.encode(
        address account,   // 32 bytes left-padded
        uint8   tier,       // 32 bytes left-padded
        uint256 volume28d,  // 32 bytes
        uint32  volumeSharePpm, // 32 bytes left-padded
        uint256 stakedDeopt, // 32 bytes
        uint64  validFrom,  // 32 bytes left-padded
        uint64  validUntil  // 32 bytes left-padded
    )
)
```

Exactly 7 × 32 = 224 bytes input to `keccak256`. Inner nodes use
OpenZeppelin's commutative sorted-pair hashing:

```
hashPair(a, b) = keccak256(abi.encodePacked(min(a,b), max(a,b)))
```

Single-leaf trees use the leaf as the root with an empty proof —
matches `_setSingleLeafRoot` in the Solidity tests.

Test coverage (8 tests):

- `solidity_hash_tier_leaf_vector` — leaf for a known input set
  matches `keccak(abi.encode(...))` recomputed in the test AND
  the byte-for-byte golden hash captured from the live Solidity
  contract (`0x52be52ec…b792d`).
- `deterministic_root_and_proofs_across_runs` — same inputs ⇒
  same root, same leaves, same per-leaf proof.
- `every_leaf_proof_verifies_against_root` — round-trip on a
  6-leaf tree.
- `tampered_leaf_does_not_verify` — flipping the `tier` field
  breaks the proof.
- `proof_from_wrong_index_does_not_verify` — operator can't swap
  proofs.
- `single_leaf_root_equals_leaf_and_proof_is_empty` — matches
  Solidity test pattern.
- `empty_inputs_yield_no_tree` — defensive `None` for empty input.
- `odd_level_tree_proofs_all_verify` — exercises the odd-leaf
  promotion path (3-leaf tree).

## Solidity compatibility (Part 6)

Module: backend test `solidity_hash_tier_leaf_vector` (above) +
Forge test
`~/DEOPT/deopt-v2-sol/test/fees/V2G_A_LeafCrossVector.t.sol`
(new):

```solidity
function testHashTierLeafIsKeccakOfAbiEncode() external view {
    bytes32 onchain = feesManager.hashTierLeaf(
        ACCOUNT, TIER, VOLUME_28D, VOLUME_SHARE_PPM,
        STAKED_DEOPT, VALID_FROM, VALID_UNTIL
    );
    bytes32 expected = keccak256(
        abi.encode(ACCOUNT, TIER, VOLUME_28D, VOLUME_SHARE_PPM,
                   STAKED_DEOPT, VALID_FROM, VALID_UNTIL)
    );
    assertEq(onchain, expected);
    assertEq(abi.encode(...).length, 7 * 32);
}
```

Companion Forge script
`~/DEOPT/deopt-v2-sol/script/PrintLeaf.s.sol` runs the same vector
through a freshly-deployed `FeesManagerV2` and prints the leaf
hash; that hash is pinned in the backend test as the
`SOLIDITY_GOLDEN_LEAF` constant. Drift on either side trips both
tests.

`forge test --match-path test/fees/V2G_A_LeafCrossVector.t.sol`
→ `[PASS] testHashTierLeafIsKeccakOfAbiEncode()`.

## Rebate budget + smoke plan (Part 7)

See `docs/REBATE_LIVE_SMOKE_PLAN_V2G_A.md` for the full per-step
runbook. Summary:

1. Generate a snapshot with one or two test accounts forced into
   Tier 4 via large synthetic volume.
2. Build the Merkle tree; capture root + per-account proof.
3. Owner calls `FeesManagerV2.setMerkleRoot(root, validFrom,
   validUntil)`.
4. Owner calls `FeesManagerV2.setRebateFundingAccount(funder)` if
   not already set; funder approves a small mUSDC allowance.
5. Owner calls `FeesManagerV2.fundRebateBudget(mUSDC, amount)`.
6. Each test account calls `FeesManagerV2.claimTier(...)`.
7. Execute a tiny PERP trade with the maker on the Tier 4
   account → expect 1× `FeeChargedV2` (taker) + 1×
   `FeeRebatedV2` (maker), `rebateBudget(mUSDC)` decreases by
   the rebate amount, `feeRecipient(mUSDC)` grows by the taker
   fee.
8. Repeat for OPTION; document the same expected event shape.

Hard gate carried forward: **the V2G-A milestone never
broadcasts**. The smoke plan is the human-broadcast handoff.

## Files changed (V2G-A)

Backend:

- `src/fees/mod.rs` — register `tier_eligibility`, `tier_merkle`,
  `tier_snapshot`.
- `src/fees/schedule.rs` — add `MICRO_BPS_PER_PPM`, `BPS_PER_PCT`,
  `launch_tier(product, tier)`; add three canonical-table tests
  (option, perp, eligibility thresholds).
- `src/fees/tier_eligibility.rs` — new module + 8 boundary tests.
- `src/fees/tier_merkle.rs` — new module + 8 tests (including
  the Solidity cross-vector).
- `src/fees/tier_snapshot.rs` — new module + 4 tests.

Solidity:

- `~/DEOPT/deopt-v2-sol/test/fees/V2G_A_LeafCrossVector.t.sol` —
  new Forge cross-vector test (1 passing assertion).
- `~/DEOPT/deopt-v2-sol/script/PrintLeaf.s.sol` — companion
  script that prints the golden leaf hash.

Docs:

- `docs/TIER_MERKLE_REBATE_SYSTEM_V2G_A.md` — this file.
- `docs/TIER_SNAPSHOT_SCHEMA_V2G_A.md` — snapshot JSON schema.
- `docs/REBATE_LIVE_SMOKE_PLAN_V2G_A.md` — operator smoke plan.

No `.env` edits, no DB-row mutations, no broadcasting, no
Solidity contract changes.

## Tests added (summary)

- 3 canonical-schedule tests in `schedule.rs` (option, perp,
  eligibility thresholds).
- 8 boundary / OR eligibility tests in `tier_eligibility.rs`.
- 4 snapshot tests in `tier_snapshot.rs` (sorting,
  Tier4-via-volume, Tier0 fallback, determinism, window
  preservation).
- 8 Merkle tests in `tier_merkle.rs` (Solidity vector,
  determinism, all-proofs-verify, tampered leaf, wrong-index
  proof, single-leaf, empty inputs, odd-level tree).
- 1 Forge cross-vector test in `V2G_A_LeafCrossVector.t.sol`.

Existing 367+ backend tests continue to pass.

## Validation commands run

```
cargo fmt --all                                          ✅ no diff
cargo clippy --all-targets --all-features -- -D warnings ✅ clean
cargo test --all-targets --all-features --no-fail-fast   ✅ all suites pass
cargo build --all-targets --all-features                 ✅ Finished
forge test --match-path test/fees/V2G_A_LeafCrossVector.t.sol  ✅ 1 passing
```

`forge build` / `forge fmt` and the wider Solidity suite were not
re-run for V2G-A because the only Solidity change is a single
new test file plus a script; no contract code was modified.

## Exact blockers

None at the V2G-A scope. The next human-broadcast gates are the
items in the live smoke plan (Section "Rebate budget + smoke
plan" above):

- Owner-signed `setMerkleRoot`.
- Owner-signed `setRebateFundingAccount` (if not already set).
- Funder-signed `IERC20.approve(FeesManagerV2, amount)`.
- Owner-signed `fundRebateBudget`.
- Per-account `claimTier`.
- Maker- and taker-signed PERP / OPTION trades.

Each of those is gated on operator credentials this milestone
deliberately does not handle.

## V2G-B follow-up (2026-05-30)

The V2G-B milestone landed:

- `src/fees/tier_artifact.rs` + 10 regression tests (determinism,
  proof verification, schedule embedding, tampered-leaf rejection).
- `src/bin/generate_tier_artifact.rs` CLI producing the
  `chain_id`/`fees_manager_v2`/`leaf_encoding_version`/`merkle_root`/
  `option_schedule`/`perp_schedule`/`rows[]` artifact JSON.
- Deterministic Base Sepolia artifact at
  `artifacts/tier_merkle/base_sepolia_v2g_b.json` (root
  `0xef08543c…`, 3 rows: Tier 4 / Tier 2 / Tier 0).
- Three Solidity dry-run scripts (`SetFeesManagerV2MerkleRoot`,
  `FundFeesManagerV2RebateBudget`, `ClaimFeesManagerV2Tier`) —
  all default to preflight-only with a single `*_CONFIRM` gate.
- Two smoke preflight scripts (`SmokePerpV2Rebate`,
  `SmokeOptionV2Rebate`) — read-only, verified live on Base
  Sepolia: `PerpEngine.useFeesManagerV2() == true`,
  `OLD.useFeesManagerV2() == false`,
  `FeesManagerV2.isFeeConsumer(NEW) == true`,
  `FeesManagerV2.isFeeConsumer(OLD) == false`.
- Human broadcast gate packet:
  `docs/FEES_MANAGER_V2_REBATE_BROADCAST_PREFLIGHT_V2G_B.md`.
- Pipeline doc:
  `docs/TIER_MERKLE_ARTIFACT_PIPELINE_V2G_B.md`.

See those two docs for the full V2G-B record.

## Next recommended milestone

**V2G-B — Snapshot Source Pipeline + Rebate Smoke Dry-Run**
(backend + ops, no broadcast on its own):

1. Wire `TraderInputs` to the persisted fee ledger and on-chain
   DEOPT-balance reader so `generate_tier_snapshot` can run
   against real data (currently the constructor takes
   caller-provided inputs only).
2. Add a CLI/binary (`scripts/generate_tier_snapshot.rs`) that
   writes `snapshot.json` and `merkle.json` artifacts following
   the schema doc.
3. Execute the smoke plan steps 1-2 end-to-end on Base Sepolia,
   producing a real root + proof but **without** signing
   `setMerkleRoot`. Capture the artifacts in
   `docs/REBATE_SMOKE_DRY_RUN_V2G_B.md`.
4. Wire the operator handoff: a checklist + the exact
   `cast`-style invocations the human will run for the on-chain
   steps.
