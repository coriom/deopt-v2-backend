//! V2G-A: deterministic Merkle tree generation for `FeesManagerV2.claimTier`.
//!
//! The on-chain leaf format is `keccak256(abi.encode(account, tier,
//! volume28d, volumeSharePpm, stakedDeopt, validFrom, validUntil))`
//! (`FeesManagerV2.hashTierLeaf`, seven 32-byte ABI words). The
//! root/proof tree uses OpenZeppelin's standard
//! `MerkleProof.verifyCalldata` semantics: inner nodes are
//! `keccak256(abi.encodePacked(min, max))` of their two child hashes
//! (commutative sorted-pair hashing).
//!
//! This module exposes:
//!
//! - [`TierLeafInputs`] — the seven canonical fields hashed into a leaf.
//! - [`tier_leaf`] — pure leaf hasher.
//! - [`MerkleTree`] — builder + accessors for root and per-leaf proofs.
//!
//! Determinism guarantees:
//!
//! 1. The leaf format is byte-for-byte equal to
//!    `FeesManagerV2.hashTierLeaf` (cross-verified via the
//!    `solidity_hash_tier_leaf_vector` test below).
//! 2. The builder accepts leaves in the order callers provide them and
//!    never re-sorts; callers must sort externally (e.g. by trader
//!    address) before constructing a tree so two runs with the same
//!    snapshot produce the same root.
//! 3. Single-leaf trees use the leaf hash directly as the root with an
//!    empty proof — matches the Solidity test pattern in
//!    `test/fees/FeesManagerV2.t.sol::_setSingleLeafRoot`.

use crate::signing::eip712::keccak256;

/// Inputs to the canonical Merkle leaf hash. Mirrors the parameter
/// list of `FeesManagerV2.hashTierLeaf` exactly; the leaf is
/// `keccak256(abi.encode(account, tier, volume28d, volumeSharePpm,
/// stakedDeopt, validFrom, validUntil))`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TierLeafInputs {
    pub account: [u8; 20],
    pub tier: u8,
    pub volume_28d: [u8; 32],
    pub volume_share_ppm: u32,
    pub staked_deopt: [u8; 32],
    pub valid_from: u64,
    pub valid_until: u64,
}

impl TierLeafInputs {
    /// Convenience constructor that takes `u128` for volume and stake
    /// (the natural backend representation, since both are `1e8`-scaled
    /// venue notional) and zero-extends them into 32-byte words for
    /// ABI encoding.
    pub fn from_u128_volume_and_stake(
        account: [u8; 20],
        tier: u8,
        volume_28d_1e8: u128,
        volume_share_ppm: u32,
        staked_deopt_1e8: u128,
        valid_from: u64,
        valid_until: u64,
    ) -> Self {
        Self {
            account,
            tier,
            volume_28d: encode_u128_to_uint256(volume_28d_1e8),
            volume_share_ppm,
            staked_deopt: encode_u128_to_uint256(staked_deopt_1e8),
            valid_from,
            valid_until,
        }
    }
}

/// Compute the leaf hash matching `FeesManagerV2.hashTierLeaf`.
pub fn tier_leaf(inputs: &TierLeafInputs) -> [u8; 32] {
    let mut encoded = Vec::with_capacity(32 * 7);
    encoded.extend_from_slice(&encode_address(&inputs.account));
    encoded.extend_from_slice(&encode_u8(inputs.tier));
    encoded.extend_from_slice(&inputs.volume_28d);
    encoded.extend_from_slice(&encode_u32(inputs.volume_share_ppm));
    encoded.extend_from_slice(&inputs.staked_deopt);
    encoded.extend_from_slice(&encode_u64(inputs.valid_from));
    encoded.extend_from_slice(&encode_u64(inputs.valid_until));
    keccak256(&encoded)
}

/// A deterministic Merkle tree built from a pre-ordered slice of
/// canonical tier leaves.
///
/// Inner nodes use OpenZeppelin's commutative sorted-pair hashing:
/// `keccak256(abi.encodePacked(min(left, right), max(left, right)))`.
/// This matches `MerkleProof.verifyCalldata` used by
/// `FeesManagerV2.claimTier`.
#[derive(Clone, Debug)]
pub struct MerkleTree {
    leaves: Vec<[u8; 32]>,
    /// Level-by-level node storage. `levels[0] = leaves`,
    /// `levels.last() = [root]`. Used to materialise proofs.
    levels: Vec<Vec<[u8; 32]>>,
}

impl MerkleTree {
    /// Build a tree from the provided leaves. The empty tree is
    /// rejected; callers should not invoke `setMerkleRoot` against an
    /// empty snapshot (`FeesManagerV2.claimTier` reverts on
    /// `merkleRoot == bytes32(0)` anyway).
    pub fn from_leaves(leaves: Vec<[u8; 32]>) -> Option<Self> {
        if leaves.is_empty() {
            return None;
        }
        let mut levels = Vec::new();
        levels.push(leaves.clone());
        while levels.last().map(|level| level.len()).unwrap_or(0) > 1 {
            let parent = build_parent_level(levels.last().expect("non-empty"));
            levels.push(parent);
        }
        Some(Self { leaves, levels })
    }

    /// Build a tree from `(inputs, leaf)` pairs already computed by the
    /// caller via [`tier_leaf`]. Equivalent to mapping `tier_leaf` over
    /// the inputs and calling [`MerkleTree::from_leaves`]; kept as a
    /// helper so callers don't have to reach into the lower-level
    /// hashing API.
    pub fn from_inputs(inputs: &[TierLeafInputs]) -> Option<Self> {
        let leaves: Vec<[u8; 32]> = inputs.iter().map(tier_leaf).collect();
        Self::from_leaves(leaves)
    }

    /// Root hash. Single-leaf trees return the leaf itself, matching
    /// `_setSingleLeafRoot` in the Solidity tests.
    pub fn root(&self) -> [u8; 32] {
        *self
            .levels
            .last()
            .expect("V2G-A: built tree always has at least one level")
            .first()
            .expect("V2G-A: top level always has the root")
    }

    /// Number of leaves in the tree.
    pub fn leaf_count(&self) -> usize {
        self.leaves.len()
    }

    /// Read-only view of the leaves in the order they were provided
    /// to [`MerkleTree::from_leaves`].
    pub fn leaves(&self) -> &[[u8; 32]] {
        &self.leaves
    }

    /// Generate a proof for the leaf at `index`. For a single-leaf
    /// tree this is the empty vector (the leaf is the root). Returns
    /// `None` if `index` is out of range.
    pub fn proof(&self, index: usize) -> Option<Vec<[u8; 32]>> {
        if index >= self.leaves.len() {
            return None;
        }
        let mut proof = Vec::new();
        let mut idx = index;
        for level in &self.levels[..self.levels.len() - 1] {
            let sibling_idx = if idx % 2 == 0 { idx + 1 } else { idx - 1 };
            if sibling_idx < level.len() {
                proof.push(level[sibling_idx]);
            }
            // Otherwise the node has no sibling at this level (odd-sized
            // level); the parent is just the lone child.
            idx /= 2;
        }
        Some(proof)
    }

    /// Verify a proof against this tree's root for `leaf` using
    /// OZ-style sorted-pair hashing. Public for tests and operator
    /// dry-runs; the contract performs the same check on-chain.
    pub fn verify_proof(leaf: [u8; 32], proof: &[[u8; 32]], root: [u8; 32]) -> bool {
        let mut current = leaf;
        for sibling in proof {
            current = hash_sorted_pair(current, *sibling);
        }
        current == root
    }
}

fn build_parent_level(level: &[[u8; 32]]) -> Vec<[u8; 32]> {
    let mut parent = Vec::with_capacity(level.len().div_ceil(2));
    let mut i = 0;
    while i < level.len() {
        if i + 1 < level.len() {
            parent.push(hash_sorted_pair(level[i], level[i + 1]));
            i += 2;
        } else {
            // Lone node — promotes to the next level unchanged
            // (matches OZ's odd-leaf handling: no duplication).
            parent.push(level[i]);
            i += 1;
        }
    }
    parent
}

fn hash_sorted_pair(a: [u8; 32], b: [u8; 32]) -> [u8; 32] {
    let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
    let mut buf = [0u8; 64];
    buf[..32].copy_from_slice(&lo);
    buf[32..].copy_from_slice(&hi);
    keccak256(&buf)
}

fn encode_address(address: &[u8; 20]) -> [u8; 32] {
    let mut word = [0u8; 32];
    word[12..].copy_from_slice(address);
    word
}

fn encode_u8(value: u8) -> [u8; 32] {
    let mut word = [0u8; 32];
    word[31] = value;
    word
}

fn encode_u32(value: u32) -> [u8; 32] {
    let mut word = [0u8; 32];
    word[28..].copy_from_slice(&value.to_be_bytes());
    word
}

fn encode_u64(value: u64) -> [u8; 32] {
    let mut word = [0u8; 32];
    word[24..].copy_from_slice(&value.to_be_bytes());
    word
}

fn encode_u128_to_uint256(value: u128) -> [u8; 32] {
    let mut word = [0u8; 32];
    word[16..].copy_from_slice(&value.to_be_bytes());
    word
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addr(byte: u8) -> [u8; 20] {
        let mut buf = [0u8; 20];
        buf[19] = byte;
        buf
    }

    fn vec_inputs(count: u8) -> Vec<TierLeafInputs> {
        (0..count)
            .map(|i| {
                TierLeafInputs::from_u128_volume_and_stake(
                    addr(i + 1),
                    (i % 5) + 1,
                    u128::from(i + 1) * 1_000_000,
                    u32::from(i + 1) * 1_000,
                    u128::from(i + 1) * 1_000,
                    1_700_000_000,
                    1_700_000_000 + 7 * 86_400,
                )
            })
            .collect()
    }

    /// V2G-A — cross-vector with Solidity. The leaf for a known input
    /// set is byte-for-byte equal to
    /// `keccak256(abi.encode(account, tier, volume28d,
    /// volumeSharePpm, stakedDeopt, validFrom, validUntil))`.
    ///
    /// The expected hash below is also pinned against the live
    /// `FeesManagerV2.hashTierLeaf` output (extracted via the
    /// `forge script script/PrintLeaf.s.sol` companion script and
    /// asserted byte-for-byte by the
    /// `test/fees/V2G_A_LeafCrossVector.t.sol::testHashTierLeafIsKeccakOfAbiEncode`
    /// forge test). Any drift in encoding (e.g. accidentally
    /// switching to `abi.encodePacked`) changes this hash and both
    /// sides fail.
    #[test]
    fn solidity_hash_tier_leaf_vector() {
        // Account 0x000…0001, tier 4, volume28d = 25_000_000 * 1e8,
        // volumeSharePpm = 50_000 (5 %), stakedDeopt = 250_000 * 1e8,
        // validFrom = 1_700_000_000, validUntil = 1_700_000_000 + 7 * 86_400.
        let account = addr(1);
        let inputs = TierLeafInputs::from_u128_volume_and_stake(
            account,
            4,
            25_000_000u128 * 100_000_000u128,
            50_000,
            250_000u128 * 100_000_000u128,
            1_700_000_000,
            1_700_000_000 + 7 * 86_400,
        );

        // Compute the encoding manually to assert layout, then keccak it.
        let mut expected_buf = Vec::with_capacity(7 * 32);
        expected_buf.extend_from_slice(&encode_address(&account));
        expected_buf.extend_from_slice(&encode_u8(4));
        expected_buf.extend_from_slice(&encode_u128_to_uint256(25_000_000u128 * 100_000_000u128));
        expected_buf.extend_from_slice(&encode_u32(50_000));
        expected_buf.extend_from_slice(&encode_u128_to_uint256(250_000u128 * 100_000_000u128));
        expected_buf.extend_from_slice(&encode_u64(1_700_000_000));
        expected_buf.extend_from_slice(&encode_u64(1_700_000_000 + 7 * 86_400));
        let expected_leaf = keccak256(&expected_buf);

        assert_eq!(tier_leaf(&inputs), expected_leaf);
        // The expected encoding is exactly 224 bytes (7 × 32). Any
        // accidental switch to encodePacked would produce a shorter
        // buffer and a different hash.
        assert_eq!(expected_buf.len(), 224);

        // Solidity-side ground truth: the same vector under
        // `FeesManagerV2.hashTierLeaf` on a fresh deployment produces
        // exactly this hash (captured via
        // `forge script script/PrintLeaf.s.sol`). If the backend
        // encoder drifts, this assertion catches it immediately even
        // without rerunning forge.
        const SOLIDITY_GOLDEN_LEAF: [u8; 32] = [
            0x52, 0xbe, 0x52, 0xec, 0xfc, 0xe4, 0x72, 0x78, 0x7d, 0x35, 0x12, 0x01, 0x0d, 0xd6,
            0x55, 0x98, 0x1b, 0x22, 0x86, 0x3f, 0xe9, 0x22, 0x4b, 0xef, 0x3f, 0x00, 0xfe, 0x3a,
            0x08, 0xbb, 0x79, 0x2d,
        ];
        assert_eq!(tier_leaf(&inputs), SOLIDITY_GOLDEN_LEAF);
    }

    /// V2G-A determinism: running [`MerkleTree::from_inputs`] twice on
    /// the same ordered input slice produces the same root, the same
    /// per-leaf leaves, and the same per-leaf proof.
    #[test]
    fn deterministic_root_and_proofs_across_runs() {
        let inputs = vec_inputs(7);
        let first = MerkleTree::from_inputs(&inputs).expect("non-empty");
        let second = MerkleTree::from_inputs(&inputs).expect("non-empty");
        assert_eq!(first.root(), second.root());
        for i in 0..inputs.len() {
            assert_eq!(first.proof(i), second.proof(i));
            assert_eq!(first.leaves()[i], second.leaves()[i]);
        }
    }

    /// V2G-A: every leaf in a multi-leaf tree verifies against the
    /// tree's root with its own proof.
    #[test]
    fn every_leaf_proof_verifies_against_root() {
        let inputs = vec_inputs(6);
        let tree = MerkleTree::from_inputs(&inputs).expect("non-empty");
        let root = tree.root();
        for (i, leaf) in tree.leaves().iter().copied().enumerate() {
            let proof = tree.proof(i).expect("in-range index");
            assert!(
                MerkleTree::verify_proof(leaf, &proof, root),
                "leaf {i} should verify against the tree's root"
            );
        }
    }

    /// V2G-A: a leaf with a tampered field (e.g. wrong tier) does
    /// **not** verify against the original root, even when paired
    /// with the original proof. Guards the no-forgery contract.
    #[test]
    fn tampered_leaf_does_not_verify() {
        let mut inputs = vec_inputs(4);
        let tree = MerkleTree::from_inputs(&inputs).expect("non-empty");
        let root = tree.root();
        let proof = tree.proof(2).expect("index 2");
        inputs[2].tier = 4; // tamper
        let tampered_leaf = tier_leaf(&inputs[2]);
        assert!(!MerkleTree::verify_proof(tampered_leaf, &proof, root));
    }

    /// V2G-A: a proof from a *different* index does not verify against
    /// the correct leaf. Guards against operator proof swaps.
    #[test]
    fn proof_from_wrong_index_does_not_verify() {
        let inputs = vec_inputs(5);
        let tree = MerkleTree::from_inputs(&inputs).expect("non-empty");
        let root = tree.root();
        let leaf_0 = tree.leaves()[0];
        let wrong_proof = tree.proof(3).expect("in-range");
        assert!(!MerkleTree::verify_proof(leaf_0, &wrong_proof, root));
    }

    /// V2G-A: single-leaf tree special case — the leaf is the root and
    /// the proof is empty. Matches the Solidity test pattern
    /// `_setSingleLeafRoot` (which passes `new bytes32[](0)`).
    #[test]
    fn single_leaf_root_equals_leaf_and_proof_is_empty() {
        let inputs = vec_inputs(1);
        let leaf = tier_leaf(&inputs[0]);
        let tree = MerkleTree::from_inputs(&inputs).expect("non-empty");
        assert_eq!(tree.root(), leaf);
        let proof = tree.proof(0).expect("in-range");
        assert!(proof.is_empty());
        assert!(MerkleTree::verify_proof(leaf, &proof, tree.root()));
    }

    /// V2G-A: empty input slice is rejected (Builder returns `None`).
    /// Callers should never invoke `setMerkleRoot` against an empty
    /// snapshot; the contract reverts on a zero root anyway.
    #[test]
    fn empty_inputs_yield_no_tree() {
        let inputs: Vec<TierLeafInputs> = Vec::new();
        assert!(MerkleTree::from_inputs(&inputs).is_none());
    }

    /// V2G-A: odd-sized levels promote the lone node unchanged. We
    /// exercise this directly by building a tree with three leaves
    /// and asserting all three proofs verify.
    #[test]
    fn odd_level_tree_proofs_all_verify() {
        let inputs = vec_inputs(3);
        let tree = MerkleTree::from_inputs(&inputs).expect("non-empty");
        let root = tree.root();
        for (i, leaf) in tree.leaves().iter().copied().enumerate() {
            let proof = tree.proof(i).expect("in-range");
            assert!(
                MerkleTree::verify_proof(leaf, &proof, root),
                "odd-level leaf {i} must verify"
            );
        }
    }
}
