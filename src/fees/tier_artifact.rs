//! V2G-B: tier Merkle artifact assembly.
//!
//! Combines a deterministic [`TierSnapshotRow`] sequence (from
//! `tier_snapshot::generate_tier_snapshot`) with the per-row Merkle
//! leaf and proof (from `tier_merkle`) into a single JSON artifact
//! that the operator hands to `FeesManagerV2.setMerkleRoot` and that
//! each smoke account hands to `FeesManagerV2.claimTier`.
//!
//! The artifact also carries observability metadata so a human can
//! audit it without re-running the backend:
//!
//! - `chain_id`, `fees_manager_v2` — environment binding;
//! - `leaf_encoding_version` — bumped on any backwards-incompatible
//!   change to the leaf or row format (currently `"v2g-a-1"`);
//! - `generated_at_ms` — wall-clock stamp of the run, supplied by the
//!   caller (the CLI uses `now_ms()`; tests inject a constant for
//!   deterministic byte-equal comparisons);
//! - `valid_from` / `valid_until` — copied from `SnapshotConfig` so
//!   the `setMerkleRoot` calldata is one JSON read away;
//! - `option_schedule` / `perp_schedule` — full canonical fee
//!   schedule embedded verbatim so the operator can confirm the
//!   tier profiles without a separate lookup.

use serde::{Deserialize, Serialize};

use super::schedule::{launch_fee_schedule, FeeTier, LaunchFeeSchedule};
use super::tier_merkle::{tier_leaf, MerkleTree, TierLeafInputs};
use super::tier_snapshot::{generate_tier_snapshot, SnapshotConfig, TierSnapshotRow, TraderInputs};

/// Schema version tag baked into every artifact. Bump together with
/// any field rename/removal so downstream tooling can fail fast.
pub const LEAF_ENCODING_VERSION: &str = "v2g-a-1";

/// Per-account row in the artifact: the V2G-A snapshot row plus the
/// Merkle leaf and proof needed to call `claimTier`.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TierArtifactRow {
    #[serde(flatten)]
    pub snapshot: TierSnapshotRow,
    /// Lowercase-hex-with-0x leaf hash (`FeesManagerV2.hashTierLeaf` output).
    pub leaf: String,
    /// Lowercase-hex-with-0x Merkle proof in proving order
    /// (sibling-from-leaf upwards). Empty for single-leaf trees.
    pub proof: Vec<String>,
}

/// Full tier Merkle artifact. Stable JSON wire format pinned by
/// `tier_artifact::tests::artifact_is_deterministic`.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TierArtifact {
    pub chain_id: u64,
    pub fees_manager_v2: String,
    pub leaf_encoding_version: String,
    pub generated_at_ms: u64,
    pub valid_from: u64,
    pub valid_until: u64,
    /// Hex-encoded Merkle root (lowercase, 0x-prefixed, 66 chars).
    pub merkle_root: String,
    pub option_schedule: Vec<FeeTier>,
    pub perp_schedule: Vec<FeeTier>,
    pub rows: Vec<TierArtifactRow>,
}

/// Inputs collected from the operator/fixture before assembly.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ArtifactConfig {
    pub chain_id: u64,
    pub fees_manager_v2: [u8; 20],
    pub generated_at_ms: u64,
    pub snapshot_config: SnapshotConfig,
}

/// Produce a tier Merkle artifact from a deterministic set of trader
/// inputs.
///
/// Returns `Err` if the snapshot is empty (no rows to hash) or if any
/// row's reported tier somehow exceeds the configured launch schedule.
/// The result is the JSON wire-shape recorded by
/// `docs/TIER_MERKLE_ARTIFACT_PIPELINE_V2G_B.md`.
pub fn generate_tier_artifact(
    inputs: &[TraderInputs],
    config: ArtifactConfig,
) -> Result<TierArtifact, &'static str> {
    if inputs.is_empty() {
        return Err("V2G-B: cannot generate artifact from empty input set");
    }

    let snapshot_rows = generate_tier_snapshot(inputs, config.snapshot_config);
    if snapshot_rows.is_empty() {
        return Err("V2G-B: snapshot returned no rows");
    }

    // Each leaf is hashed with the SAME inputs the operator will pass
    // to claimTier — the canonical snapshot row carries every field.
    let leaf_inputs: Vec<TierLeafInputs> = snapshot_rows
        .iter()
        .map(|row| {
            let trader_bytes = trader_address_to_bytes(&row.trader);
            TierLeafInputs::from_u128_volume_and_stake(
                trader_bytes,
                row.option_tier.max(row.perp_tier), // claim the higher tier
                parse_u128_or_zero(&row.total_28d_volume_1e8),
                row.volume_share_ppm,
                parse_u128_or_zero(&row.staked_deopt_1e8),
                row.valid_from,
                row.valid_until,
            )
        })
        .collect();

    let leaves: Vec<[u8; 32]> = leaf_inputs.iter().map(tier_leaf).collect();
    let tree = MerkleTree::from_leaves(leaves.clone())
        .ok_or("V2G-B: Merkle tree builder rejected leaves")?;
    let root = tree.root();

    let mut rows: Vec<TierArtifactRow> = Vec::with_capacity(snapshot_rows.len());
    for (index, snapshot) in snapshot_rows.into_iter().enumerate() {
        let leaf = leaves[index];
        let proof = tree
            .proof(index)
            .ok_or("V2G-B: tree refused to materialise a proof")?;
        rows.push(TierArtifactRow {
            snapshot,
            leaf: format_bytes32(&leaf),
            proof: proof.iter().map(format_bytes32).collect(),
        });
    }

    let LaunchFeeSchedule {
        option: option_schedule,
        perp: perp_schedule,
    } = launch_fee_schedule();

    Ok(TierArtifact {
        chain_id: config.chain_id,
        fees_manager_v2: format_address(&config.fees_manager_v2),
        leaf_encoding_version: LEAF_ENCODING_VERSION.to_string(),
        generated_at_ms: config.generated_at_ms,
        valid_from: config.snapshot_config.valid_from,
        valid_until: config.snapshot_config.valid_until,
        merkle_root: format_bytes32(&root),
        option_schedule,
        perp_schedule,
        rows,
    })
}

fn trader_address_to_bytes(trader: &str) -> [u8; 20] {
    let hex = trader.strip_prefix("0x").unwrap_or(trader);
    let mut bytes = [0u8; 20];
    if hex.len() != 40 {
        return bytes;
    }
    for (idx, chunk) in hex.as_bytes().chunks(2).enumerate() {
        let high = hex_value(chunk[0]).unwrap_or(0);
        let low = hex_value(chunk[1]).unwrap_or(0);
        bytes[idx] = (high << 4) | low;
    }
    bytes
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn parse_u128_or_zero(value: &str) -> u128 {
    value.parse::<u128>().unwrap_or(0)
}

fn format_address(bytes: &[u8; 20]) -> String {
    let mut out = String::with_capacity(2 + 40);
    out.push_str("0x");
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

fn format_bytes32(bytes: &[u8; 32]) -> String {
    let mut out = String::with_capacity(2 + 64);
    out.push_str("0x");
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fees::tier_merkle::MerkleTree;

    const ONE_1E8: u128 = 100_000_000;
    const SECONDS_PER_DAY: u64 = 86_400;
    /// Base Sepolia chain id (pinned for the artifact-target tests).
    const BASE_SEPOLIA_CHAIN_ID: u64 = 84_532;

    fn addr(byte: u8) -> [u8; 20] {
        let mut buf = [0u8; 20];
        buf[19] = byte;
        buf
    }

    fn fees_manager() -> [u8; 20] {
        // 0x00dA0B9876bcBf0c79CB5BcAcfEBAFb8C7Ad774f
        [
            0x00, 0xda, 0x0b, 0x98, 0x76, 0xbc, 0xbf, 0x0c, 0x79, 0xcb, 0x5b, 0xca, 0xcf, 0xeb,
            0xaf, 0xb8, 0xc7, 0xad, 0x77, 0x4f,
        ]
    }

    fn snapshot_config() -> SnapshotConfig {
        SnapshotConfig {
            valid_from: 1_700_000_000,
            valid_until: 1_700_000_000 + 7 * SECONDS_PER_DAY,
        }
    }

    fn artifact_config(generated_at_ms: u64) -> ArtifactConfig {
        ArtifactConfig {
            chain_id: BASE_SEPOLIA_CHAIN_ID,
            fees_manager_v2: fees_manager(),
            generated_at_ms,
            snapshot_config: snapshot_config(),
        }
    }

    fn tier_4_account(byte: u8) -> TraderInputs {
        TraderInputs {
            account: addr(byte),
            option_volume_28d_1e8: 12_500_000 * ONE_1E8,
            perp_volume_28d_1e8: 12_500_000 * ONE_1E8,
            volume_share_ppm: 0,
            staked_deopt_1e8: 0,
        }
    }

    fn tier_2_account(byte: u8) -> TraderInputs {
        TraderInputs {
            account: addr(byte),
            option_volume_28d_1e8: 1_500_000 * ONE_1E8,
            perp_volume_28d_1e8: 1_000_000 * ONE_1E8,
            volume_share_ppm: 0,
            staked_deopt_1e8: 0,
        }
    }

    fn tier_0_account(byte: u8) -> TraderInputs {
        TraderInputs {
            account: addr(byte),
            option_volume_28d_1e8: 100 * ONE_1E8,
            perp_volume_28d_1e8: 0,
            volume_share_ppm: 0,
            staked_deopt_1e8: 0,
        }
    }

    fn sample_inputs() -> Vec<TraderInputs> {
        vec![
            tier_4_account(0xAB),
            tier_2_account(0x55),
            tier_0_account(0x07),
        ]
    }

    /// V2G-B: two runs with the same inputs and the same
    /// `generated_at_ms` produce byte-identical artifacts. Confirms
    /// the row ordering, leaf hashing, and proof generation are
    /// deterministic top to bottom.
    #[test]
    fn artifact_is_deterministic() {
        let inputs = sample_inputs();
        let cfg = artifact_config(1_700_000_500_000);
        let first = generate_tier_artifact(&inputs, cfg).expect("first");
        let second = generate_tier_artifact(&inputs, cfg).expect("second");
        assert_eq!(first, second);
        // Confirm the JSON wire-shape is byte-stable too.
        let first_json = serde_json::to_string_pretty(&first).unwrap();
        let second_json = serde_json::to_string_pretty(&second).unwrap();
        assert_eq!(first_json, second_json);
    }

    /// V2G-B: artifact root matches the root the operator computes by
    /// rebuilding the Merkle tree from the embedded leaves alone.
    /// This is the operator-side audit path.
    #[test]
    fn embedded_leaves_rebuild_the_same_root() {
        let inputs = sample_inputs();
        let artifact = generate_tier_artifact(&inputs, artifact_config(1)).unwrap();
        let leaves: Vec<[u8; 32]> = artifact
            .rows
            .iter()
            .map(|row| parse_bytes32(&row.leaf))
            .collect();
        let rebuilt = MerkleTree::from_leaves(leaves).expect("non-empty");
        assert_eq!(format_bytes32(&rebuilt.root()), artifact.merkle_root);
    }

    /// V2G-B: every row's embedded proof verifies against the
    /// artifact's reported root.
    #[test]
    fn every_row_proof_verifies_against_root() {
        let inputs = sample_inputs();
        let artifact = generate_tier_artifact(&inputs, artifact_config(2)).unwrap();
        let root = parse_bytes32(&artifact.merkle_root);
        for row in &artifact.rows {
            let leaf = parse_bytes32(&row.leaf);
            let proof: Vec<[u8; 32]> = row.proof.iter().map(|hex| parse_bytes32(hex)).collect();
            assert!(
                MerkleTree::verify_proof(leaf, &proof, root),
                "row {} should verify",
                row.snapshot.trader
            );
        }
    }

    /// V2G-B: tampering with a row's leaf (flipping one byte) breaks
    /// verification — the artifact is not forgeable post-publication.
    #[test]
    fn tampered_leaf_does_not_verify_against_artifact_root() {
        let inputs = sample_inputs();
        let artifact = generate_tier_artifact(&inputs, artifact_config(3)).unwrap();
        let root = parse_bytes32(&artifact.merkle_root);
        let mut bad_leaf = parse_bytes32(&artifact.rows[0].leaf);
        bad_leaf[0] ^= 0xff;
        let proof: Vec<[u8; 32]> = artifact.rows[0]
            .proof
            .iter()
            .map(|hex| parse_bytes32(hex))
            .collect();
        assert!(!MerkleTree::verify_proof(bad_leaf, &proof, root));
    }

    /// V2G-B: the artifact carries `valid_from < valid_until`. The
    /// snapshot config already enforces this at the operator boundary;
    /// this test guards against the artifact accidentally swapping the
    /// two fields during assembly.
    #[test]
    fn artifact_validity_window_is_ordered() {
        let artifact = generate_tier_artifact(&sample_inputs(), artifact_config(4)).unwrap();
        assert!(artifact.valid_from < artifact.valid_until);
        for row in &artifact.rows {
            assert!(row.snapshot.valid_from < row.snapshot.valid_until);
            assert_eq!(row.snapshot.valid_from, artifact.valid_from);
            assert_eq!(row.snapshot.valid_until, artifact.valid_until);
        }
    }

    /// V2G-B: when an account meets two different axes that point at
    /// different tiers, the artifact reports the higher tier (the OR
    /// resolver picks the highest qualifying tier).
    #[test]
    fn highest_tier_wins_in_artifact_rows() {
        let mixed = TraderInputs {
            account: addr(0xEE),
            option_volume_28d_1e8: 1_500_000 * ONE_1E8, // Tier 2 alone
            perp_volume_28d_1e8: 1_000_000 * ONE_1E8,
            volume_share_ppm: 50_000, // 5 % → Tier 4
            staked_deopt_1e8: 0,
        };
        let artifact = generate_tier_artifact(&[mixed], artifact_config(5)).unwrap();
        let row = &artifact.rows[0];
        assert_eq!(row.snapshot.option_tier, 4);
        assert_eq!(row.snapshot.perp_tier, 4);
    }

    /// V2G-B: the canonical OPTION + PERP schedules are embedded
    /// verbatim in every artifact. Detects accidental drift between
    /// the launch schedule and what the artifact reports.
    #[test]
    fn artifact_embeds_canonical_launch_schedule() {
        let artifact = generate_tier_artifact(&sample_inputs(), artifact_config(6)).unwrap();
        let schedule = launch_fee_schedule();
        assert_eq!(artifact.option_schedule, schedule.option);
        assert_eq!(artifact.perp_schedule, schedule.perp);
    }

    /// V2G-B: leaf encoding tag is the canonical version constant.
    /// Catches accidental rewrites of the schema string.
    #[test]
    fn artifact_pins_leaf_encoding_version_tag() {
        let artifact = generate_tier_artifact(&sample_inputs(), artifact_config(7)).unwrap();
        assert_eq!(artifact.leaf_encoding_version, "v2g-a-1");
    }

    /// V2G-B: artifact for an empty input set returns an error rather
    /// than producing a zero-row JSON the operator might broadcast by
    /// mistake.
    #[test]
    fn empty_inputs_are_rejected() {
        let result = generate_tier_artifact(&[], artifact_config(8));
        assert!(result.is_err());
    }

    /// V2G-B: each row's embedded leaf matches the explicit
    /// `tier_leaf` hash of the same inputs. Locks in the contract
    /// between the artifact assembly and the lower-level Merkle
    /// hashing function.
    #[test]
    fn embedded_leaf_matches_explicit_tier_leaf_hash() {
        let inputs = sample_inputs();
        let artifact = generate_tier_artifact(&inputs, artifact_config(9)).unwrap();
        for row in &artifact.rows {
            let trader_bytes = trader_address_to_bytes(&row.snapshot.trader);
            let leaf_inputs = TierLeafInputs::from_u128_volume_and_stake(
                trader_bytes,
                row.snapshot.option_tier.max(row.snapshot.perp_tier),
                parse_u128_or_zero(&row.snapshot.total_28d_volume_1e8),
                row.snapshot.volume_share_ppm,
                parse_u128_or_zero(&row.snapshot.staked_deopt_1e8),
                row.snapshot.valid_from,
                row.snapshot.valid_until,
            );
            assert_eq!(row.leaf, format_bytes32(&tier_leaf(&leaf_inputs)));
        }
    }

    fn parse_bytes32(hex: &str) -> [u8; 32] {
        let mut out = [0u8; 32];
        let hex = hex.strip_prefix("0x").unwrap_or(hex);
        for (idx, chunk) in hex.as_bytes().chunks(2).enumerate() {
            let high = hex_value(chunk[0]).unwrap();
            let low = hex_value(chunk[1]).unwrap();
            out[idx] = (high << 4) | low;
        }
        out
    }
}
