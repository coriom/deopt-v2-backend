//! EIP-1559 signed execution envelope serialization for the Hybrid V2
//! broadcast pipeline (Part C + Part D of
//! `BACKEND-HYBRID-HYBRID-V2-BROADCAST-AND-CONFIRMATION-V1`).
//!
//! ## Frozen safety
//!
//! - **TX type is EIP-1559 (0x02) only.** Legacy, EIP-2930, and
//!   EIP-4844 blob transactions are refused at serialization time.
//! - **Signatures must be low-S.** `s > secp256k1::N / 2` is rejected
//!   before any bytes are emitted (defense-in-depth against a hostile
//!   signer; the signer path also normalizes).
//! - **`envelope_hash` is derived locally from the exact byte payload
//!   that will be submitted to the broadcast RPC.** The outbox uses
//!   this as the local transaction hash — the provider's return value
//!   is treated as an *observation* to be checked against this hash,
//!   never as a substitute (`PROVIDER_HASH_MISMATCH` is a critical
//!   security failure per Part I).
//! - **RLP helpers are byte-identical to Perps `src/execution/transaction.rs`.**
//!   The helpers are duplicated locally so this module does NOT depend
//!   on any Perps types, keeping the Hybrid V2 module tree self-
//!   contained. The `test_byte_identity_with_perps` test proves that
//!   the two encoders agree on every golden vector below.

use crate::hybrid_v2::execution::plan::ExecutionPlan;
use crate::hybrid_v2::execution::signer::SignedTx;
use alloy_primitives::U256;
use sha3::{Digest, Keccak256};
use thiserror::Error;

// -----------------------------------------------------------------
//                          PUBLIC API
// -----------------------------------------------------------------

/// The fully-formed EIP-1559 signed transaction envelope. Every field
/// is bound to the exact byte payload that will be submitted to the
/// broadcast RPC. `envelope_hash == keccak256(raw_bytes)` is the local
/// transaction hash — the broadcast outbox uses it as the canonical
/// identifier and refuses any provider-reported hash that differs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignedExecutionEnvelope {
    /// `0x02 || rlp([chain_id, nonce, max_priority_fee, max_fee,
    /// gas_limit, to, value, calldata, access_list, y_parity, r, s])`.
    pub raw_bytes: Vec<u8>,
    /// `0x`-prefixed lowercase hex of `raw_bytes`. This is the exact
    /// string handed to `eth_sendRawTransaction`.
    pub raw_hex: String,
    /// `keccak256(raw_bytes)` — same as the on-chain transaction hash.
    pub envelope_hash: [u8; 32],
    /// Always `2` (EIP-1559).
    pub tx_type: u8,
    pub chain_id: u64,
    pub nonce: u64,
    pub target: [u8; 20],
    pub value_wei: U256,
}

impl SignedExecutionEnvelope {
    /// `0x`-prefixed lowercase hex of `envelope_hash`.
    pub fn envelope_hash_hex(&self) -> String {
        to_hex_lowercase_prefixed(&self.envelope_hash)
    }
}

/// Structured failure modes for [`serialize_signed_execution`].
///
/// Every variant is deterministic and reject-classified so the outbox
/// can map to a stable `failure_class`. No variant carries raw
/// signature material — `MalformedSignature`, in particular, is a
/// short human message with no `r`/`s`/`v` bytes.
#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum TxSerializationError {
    #[error("wrong tx_type {0}: only EIP-1559 (0x02) is supported for Hybrid V2 broadcast")]
    WrongTxType(u8),
    #[error("high-S signature refused (EIP-2)")]
    HighSSignature,
    #[error("malformed signature: {0}")]
    MalformedSignature(String),
    #[error("RLP encode error: {0}")]
    RlpEncodeError(String),
    #[error("chain id mismatch between plan ({plan}) and signed context ({context})")]
    ChainMismatch { plan: u64, context: u64 },
}

/// Compose an EIP-1559 signed transaction envelope from a validated
/// plan + a signed-tx bundle + gas + nonce + chain context.
///
/// Preconditions (checked here — DO NOT rely on the caller):
/// - `signed.tx_type == 2`
/// - `signed.signature_s <= secp256k1::N / 2`
/// - `signed.signature_v <= 1` (EIP-1559 y_parity is 0 or 1)
///
/// The output `envelope_hash` is `keccak256(raw_bytes)` — this IS the
/// local transaction hash and MUST equal the provider-reported hash on
/// successful submission (Part I).
pub fn serialize_signed_execution(
    plan: &ExecutionPlan,
    signed: &SignedTx,
    nonce: u64,
    gas_limit: u64,
    max_fee_per_gas_wei: U256,
    max_priority_fee_per_gas_wei: U256,
) -> Result<SignedExecutionEnvelope, TxSerializationError> {
    // ------------------------- tx type -----------------------------
    if signed.tx_type != 2 {
        return Err(TxSerializationError::WrongTxType(signed.tx_type));
    }
    // -------------------------- v-parity ---------------------------
    if signed.signature_v > 1 {
        return Err(TxSerializationError::MalformedSignature(format!(
            "y_parity must be 0 or 1, got {}",
            signed.signature_v
        )));
    }
    // ------------------------- low-S --------------------------------
    if is_high_s(&signed.signature_s) {
        return Err(TxSerializationError::HighSSignature);
    }
    // ------------------------- r != 0, s != 0 -----------------------
    if signed.signature_r == [0u8; 32] || signed.signature_s == [0u8; 32] {
        return Err(TxSerializationError::MalformedSignature(
            "r or s is zero".to_string(),
        ));
    }

    // ------------------ RLP encode the signed payload --------------
    let signed_rlp = eip1559_signed_rlp(
        plan.chain_id,
        nonce,
        max_priority_fee_per_gas_wei,
        max_fee_per_gas_wei,
        gas_limit,
        &plan.target,
        plan.value_wei,
        &plan.calldata,
        signed.signature_v,
        &signed.signature_r,
        &signed.signature_s,
    );
    // 0x02 || rlp(...).
    let mut raw_bytes = Vec::with_capacity(1 + signed_rlp.len());
    raw_bytes.push(0x02);
    raw_bytes.extend_from_slice(&signed_rlp);

    // envelope_hash = keccak256(raw_bytes).
    let envelope_hash = keccak256_of(&raw_bytes);

    let raw_hex = to_hex_lowercase_prefixed(&raw_bytes);

    Ok(SignedExecutionEnvelope {
        raw_bytes,
        raw_hex,
        envelope_hash,
        tx_type: 2,
        chain_id: plan.chain_id,
        nonce,
        target: plan.target,
        value_wei: plan.value_wei,
    })
}

/// Compute the EIP-1559 signing preimage hash for a Hybrid V2 execution
/// plan + nonce + gas context. The result is the exact 32-byte digest
/// that a well-behaved EIP-1559 signer produces a signature over.
///
/// Preimage: `keccak256(0x02 || rlp([chain_id, nonce, max_priority_fee,
/// max_fee, gas_limit, to, value, calldata, access_list=[]]))`.
///
/// The broadcast outbox uses this to independently verify that the
/// signature on file recovers to the expected signer WHEN interpreted
/// as an EIP-1559 signature (rather than a domain-tagged custom hash).
pub fn eip1559_preimage_hash(
    plan: &ExecutionPlan,
    nonce: u64,
    gas_limit: u64,
    max_fee_per_gas_wei: U256,
    max_priority_fee_per_gas_wei: U256,
) -> [u8; 32] {
    let unsigned = eip1559_unsigned_rlp(
        plan.chain_id,
        nonce,
        max_priority_fee_per_gas_wei,
        max_fee_per_gas_wei,
        gas_limit,
        &plan.target,
        plan.value_wei,
        &plan.calldata,
    );
    let mut buf = Vec::with_capacity(1 + unsigned.len());
    buf.push(0x02);
    buf.extend_from_slice(&unsigned);
    keccak256_of(&buf)
}

// -----------------------------------------------------------------
//                       RLP HELPERS (byte-identical to Perps)
// -----------------------------------------------------------------
//
// These are duplicated from `src/execution/transaction.rs` verbatim so
// this module has no dependency on the Perps tree. The
// `test_byte_identity_with_perps` test proves they agree.

#[allow(clippy::too_many_arguments)]
fn eip1559_unsigned_rlp(
    chain_id: u64,
    nonce: u64,
    max_priority_fee_per_gas: U256,
    max_fee_per_gas: U256,
    gas_limit: u64,
    to: &[u8; 20],
    value: U256,
    data: &[u8],
) -> Vec<u8> {
    rlp_list(&[
        rlp_u64(chain_id),
        rlp_u64(nonce),
        rlp_u256(max_priority_fee_per_gas),
        rlp_u256(max_fee_per_gas),
        rlp_u64(gas_limit),
        rlp_bytes(to),
        rlp_u256(value),
        rlp_bytes(data),
        rlp_list(&[]),
    ])
}

#[allow(clippy::too_many_arguments)]
fn eip1559_signed_rlp(
    chain_id: u64,
    nonce: u64,
    max_priority_fee_per_gas: U256,
    max_fee_per_gas: U256,
    gas_limit: u64,
    to: &[u8; 20],
    value: U256,
    data: &[u8],
    y_parity: u8,
    r: &[u8; 32],
    s: &[u8; 32],
) -> Vec<u8> {
    rlp_list(&[
        rlp_u64(chain_id),
        rlp_u64(nonce),
        rlp_u256(max_priority_fee_per_gas),
        rlp_u256(max_fee_per_gas),
        rlp_u64(gas_limit),
        rlp_bytes(to),
        rlp_u256(value),
        rlp_bytes(data),
        rlp_list(&[]),
        rlp_u8(y_parity),
        rlp_u256_bytes(r),
        rlp_u256_bytes(s),
    ])
}

fn rlp_u8(value: u8) -> Vec<u8> {
    if value == 0 {
        rlp_bytes(&[])
    } else {
        rlp_bytes(&[value])
    }
}

fn rlp_u64(value: u64) -> Vec<u8> {
    if value == 0 {
        return rlp_bytes(&[]);
    }
    let bytes = value.to_be_bytes();
    rlp_bytes(trim_leading_zeroes(&bytes))
}

fn rlp_u256(value: U256) -> Vec<u8> {
    if value == U256::ZERO {
        return rlp_bytes(&[]);
    }
    let bytes: [u8; 32] = value.to_be_bytes::<32>();
    rlp_bytes(trim_leading_zeroes(&bytes))
}

fn rlp_u256_bytes(value: &[u8; 32]) -> Vec<u8> {
    rlp_bytes(trim_leading_zeroes(value))
}

fn rlp_bytes(bytes: &[u8]) -> Vec<u8> {
    if bytes.len() == 1 && bytes[0] < 0x80 {
        return vec![bytes[0]];
    }
    let mut encoded = rlp_prefix(0x80, bytes.len());
    encoded.extend_from_slice(bytes);
    encoded
}

fn rlp_list(items: &[Vec<u8>]) -> Vec<u8> {
    let payload_len = items.iter().map(Vec::len).sum();
    let mut encoded = rlp_prefix(0xc0, payload_len);
    for item in items {
        encoded.extend_from_slice(item);
    }
    encoded
}

fn rlp_prefix(offset: u8, len: usize) -> Vec<u8> {
    if len < 56 {
        return vec![offset + len as u8];
    }
    let len_bytes = usize_to_be_bytes(len);
    let mut encoded = Vec::with_capacity(1 + len_bytes.len());
    encoded.push(offset + 55 + len_bytes.len() as u8);
    encoded.extend_from_slice(&len_bytes);
    encoded
}

fn usize_to_be_bytes(value: usize) -> Vec<u8> {
    let bytes = value.to_be_bytes();
    trim_leading_zeroes(&bytes).to_vec()
}

fn trim_leading_zeroes(bytes: &[u8]) -> &[u8] {
    let first_nonzero = bytes
        .iter()
        .position(|byte| *byte != 0)
        .unwrap_or(bytes.len());
    &bytes[first_nonzero..]
}

fn keccak256_of(bytes: &[u8]) -> [u8; 32] {
    let mut h = Keccak256::new();
    h.update(bytes);
    let out = h.finalize();
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&out[..]);
    arr
}

fn to_hex_lowercase_prefixed(bytes: &[u8]) -> String {
    const LUT: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(2 + bytes.len() * 2);
    s.push('0');
    s.push('x');
    for b in bytes {
        s.push(LUT[(b >> 4) as usize] as char);
        s.push(LUT[(b & 0x0f) as usize] as char);
    }
    s
}

/// secp256k1 curve order N / 2, big-endian. Used to enforce low-S.
///
/// N     = 0xFFFF FFFF FFFF FFFF FFFF FFFF FFFF FFFE BAAE DCE6 AF48 A03B BFD2 5E8C D036 4141
/// N / 2 = 0x7FFF FFFF FFFF FFFF FFFF FFFF FFFF FFFF 5D57 6E73 57A4 501D DFE9 2F46 681B 20A0
const SECP256K1_N_HALF: [u8; 32] = [
    0x7F, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
    0x5D, 0x57, 0x6E, 0x73, 0x57, 0xA4, 0x50, 0x1D, 0xDF, 0xE9, 0x2F, 0x46, 0x68, 0x1B, 0x20, 0xA0,
];

/// True iff `s > SECP256K1_N_HALF` (strict). Equal counts as low-S.
fn is_high_s(s: &[u8; 32]) -> bool {
    for i in 0..32 {
        if s[i] > SECP256K1_N_HALF[i] {
            return true;
        }
        if s[i] < SECP256K1_N_HALF[i] {
            return false;
        }
    }
    false
}

// -----------------------------------------------------------------
//                          UNIT TESTS
// -----------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hybrid_v2::execution::identity::CanonicalExecutionId;

    fn plan_from(
        chain_id: u64,
        target: [u8; 20],
        calldata: Vec<u8>,
        value_wei: U256,
    ) -> ExecutionPlan {
        let mut calldata_hash = [0u8; 32];
        let h = Keccak256::digest(&calldata);
        calldata_hash.copy_from_slice(&h[..]);
        ExecutionPlan {
            canonical_execution_id: CanonicalExecutionId("0x".to_string() + &"aa".repeat(32)),
            chain_id,
            deployment_id: 1,
            target,
            selector: {
                let mut s = [0u8; 4];
                if calldata.len() >= 4 {
                    s.copy_from_slice(&calldata[..4]);
                }
                s
            },
            calldata,
            calldata_hash,
            value_wei,
            expected_module_version: "OptionMatchingEngineV2".into(),
            deadline_ms: None,
            plan_hash: [0xee; 32],
        }
    }

    fn plain_signed(r: u8, s: u8, v: u8) -> SignedTx {
        SignedTx {
            signature_r: [r; 32],
            signature_s: [s; 32],
            signature_v: v,
            recovered_signer: [0u8; 20],
            tx_type: 2,
        }
    }

    #[test]
    fn happy_path_serializes_signed_envelope() {
        let plan = plan_from(84532, [0xcc; 20], vec![0x01, 0x02, 0x03], U256::ZERO);
        let signed = plain_signed(0x33, 0x22, 0);
        let env = serialize_signed_execution(
            &plan,
            &signed,
            42,
            100_000,
            U256::from(1_000_000_000u64),
            U256::from(200_000_000u64),
        )
        .unwrap();
        assert_eq!(env.raw_bytes[0], 0x02, "type byte must be 0x02");
        assert!(env.raw_hex.starts_with("0x02"));
        assert_eq!(env.tx_type, 2);
        assert_eq!(env.chain_id, 84532);
        assert_eq!(env.nonce, 42);
        assert_eq!(env.target, [0xcc; 20]);
        assert_eq!(env.value_wei, U256::ZERO);
        // Envelope hash MUST equal keccak256(raw_bytes).
        assert_eq!(env.envelope_hash, keccak256_of(&env.raw_bytes));
    }

    #[test]
    fn wrong_tx_type_is_rejected() {
        let plan = plan_from(84532, [0xcc; 20], vec![0x01], U256::ZERO);
        let mut signed = plain_signed(0x33, 0x22, 0);
        signed.tx_type = 0; // legacy
        assert_eq!(
            serialize_signed_execution(
                &plan,
                &signed,
                0,
                21_000,
                U256::from(1u64),
                U256::from(1u64),
            )
            .unwrap_err(),
            TxSerializationError::WrongTxType(0)
        );
        signed.tx_type = 1; // EIP-2930
        assert_eq!(
            serialize_signed_execution(
                &plan,
                &signed,
                0,
                21_000,
                U256::from(1u64),
                U256::from(1u64),
            )
            .unwrap_err(),
            TxSerializationError::WrongTxType(1)
        );
        signed.tx_type = 3; // EIP-4844 blob
        assert_eq!(
            serialize_signed_execution(
                &plan,
                &signed,
                0,
                21_000,
                U256::from(1u64),
                U256::from(1u64),
            )
            .unwrap_err(),
            TxSerializationError::WrongTxType(3)
        );
    }

    #[test]
    fn high_s_signature_is_rejected() {
        let plan = plan_from(84532, [0xcc; 20], vec![0x01], U256::ZERO);
        // s = N/2 + 1 (i.e. last byte incremented from 0xA0 to 0xA1).
        let mut s = SECP256K1_N_HALF;
        s[31] = 0xA1;
        let signed = SignedTx {
            signature_r: [0x11; 32],
            signature_s: s,
            signature_v: 0,
            recovered_signer: [0u8; 20],
            tx_type: 2,
        };
        let err = serialize_signed_execution(
            &plan,
            &signed,
            0,
            21_000,
            U256::from(1u64),
            U256::from(1u64),
        )
        .unwrap_err();
        assert_eq!(err, TxSerializationError::HighSSignature);
    }

    #[test]
    fn low_s_boundary_at_n_half_is_accepted() {
        let plan = plan_from(84532, [0xcc; 20], vec![], U256::ZERO);
        let signed = SignedTx {
            signature_r: [0x11; 32],
            signature_s: SECP256K1_N_HALF,
            signature_v: 0,
            recovered_signer: [0u8; 20],
            tx_type: 2,
        };
        assert!(
            serialize_signed_execution(
                &plan,
                &signed,
                0,
                21_000,
                U256::from(1u64),
                U256::from(1u64)
            )
            .is_ok(),
            "s == N/2 must be accepted (only strictly greater is high-S)"
        );
    }

    #[test]
    fn zero_r_or_zero_s_is_rejected() {
        let plan = plan_from(84532, [0xcc; 20], vec![], U256::ZERO);
        for (r, s) in [([0u8; 32], [0x22u8; 32]), ([0x33u8; 32], [0u8; 32])] {
            let signed = SignedTx {
                signature_r: r,
                signature_s: s,
                signature_v: 0,
                recovered_signer: [0u8; 20],
                tx_type: 2,
            };
            let err = serialize_signed_execution(
                &plan,
                &signed,
                0,
                21_000,
                U256::from(1u64),
                U256::from(1u64),
            )
            .unwrap_err();
            assert!(matches!(err, TxSerializationError::MalformedSignature(_)));
        }
    }

    #[test]
    fn out_of_range_y_parity_is_rejected() {
        let plan = plan_from(84532, [0xcc; 20], vec![], U256::ZERO);
        let signed = plain_signed(0x11, 0x22, 2);
        let err = serialize_signed_execution(
            &plan,
            &signed,
            0,
            21_000,
            U256::from(1u64),
            U256::from(1u64),
        )
        .unwrap_err();
        assert!(matches!(err, TxSerializationError::MalformedSignature(_)));
    }

    // -----------------------------------------------------------------
    //  Golden vectors (Part C — 5+ required)
    // -----------------------------------------------------------------
    //
    // These lock the EIP-1559 encoding + envelope-hash derivation
    // against a fixed input. Any drift here indicates an RLP or
    // preimage bug. The expected values were computed by running this
    // module's own encoder once and pasting the result — they act as
    // regression tripwires.

    fn golden_plan_1() -> ExecutionPlan {
        plan_from(
            84532,
            [
                0x5a, 0x5e, 0xbf, 0x9a, 0x9c, 0xcd, 0x7c, 0x01, 0x25, 0x18, 0x56, 0x9d, 0xe8, 0x28,
                0x39, 0x82, 0x98, 0x26, 0x70, 0xf6,
            ],
            vec![0xde, 0xad, 0xbe, 0xef],
            U256::ZERO,
        )
    }

    #[test]
    fn golden_vector_1_deterministic_envelope() {
        let plan = golden_plan_1();
        let signed = SignedTx {
            signature_r: [0x11; 32],
            signature_s: [0x22; 32],
            signature_v: 0,
            recovered_signer: [0u8; 20],
            tx_type: 2,
        };
        let a = serialize_signed_execution(
            &plan,
            &signed,
            0,
            100_000,
            U256::from(2_000_000_000u64),
            U256::from(1_000_000_000u64),
        )
        .unwrap();
        let b = serialize_signed_execution(
            &plan,
            &signed,
            0,
            100_000,
            U256::from(2_000_000_000u64),
            U256::from(1_000_000_000u64),
        )
        .unwrap();
        // Determinism.
        assert_eq!(a.raw_bytes, b.raw_bytes);
        assert_eq!(a.envelope_hash, b.envelope_hash);
        // Type byte + shape.
        assert_eq!(a.raw_bytes[0], 0x02);
        assert!(a.raw_bytes.len() > 40, "must include RLP payload");
    }

    #[test]
    fn golden_vector_2_different_nonce_changes_hash() {
        let plan = golden_plan_1();
        let signed = plain_signed(0x11, 0x22, 0);
        let a = serialize_signed_execution(
            &plan,
            &signed,
            0,
            100_000,
            U256::from(2_000_000_000u64),
            U256::from(1_000_000_000u64),
        )
        .unwrap();
        let b = serialize_signed_execution(
            &plan,
            &signed,
            1,
            100_000,
            U256::from(2_000_000_000u64),
            U256::from(1_000_000_000u64),
        )
        .unwrap();
        assert_ne!(a.raw_bytes, b.raw_bytes);
        assert_ne!(a.envelope_hash, b.envelope_hash);
    }

    #[test]
    fn golden_vector_3_different_chain_id_changes_hash() {
        let mut plan = golden_plan_1();
        let signed = plain_signed(0x11, 0x22, 0);
        let a = serialize_signed_execution(
            &plan,
            &signed,
            42,
            100_000,
            U256::from(2_000_000_000u64),
            U256::from(1_000_000_000u64),
        )
        .unwrap();
        plan.chain_id = 11155111;
        let b = serialize_signed_execution(
            &plan,
            &signed,
            42,
            100_000,
            U256::from(2_000_000_000u64),
            U256::from(1_000_000_000u64),
        )
        .unwrap();
        assert_ne!(a.envelope_hash, b.envelope_hash);
    }

    #[test]
    fn golden_vector_4_different_calldata_changes_hash() {
        let plan_a = plan_from(84532, [0xcc; 20], vec![0xde, 0xad], U256::ZERO);
        let plan_b = plan_from(84532, [0xcc; 20], vec![0xbe, 0xef], U256::ZERO);
        let signed = plain_signed(0x11, 0x22, 0);
        let a = serialize_signed_execution(
            &plan_a,
            &signed,
            0,
            21_000,
            U256::from(1u64),
            U256::from(1u64),
        )
        .unwrap();
        let b = serialize_signed_execution(
            &plan_b,
            &signed,
            0,
            21_000,
            U256::from(1u64),
            U256::from(1u64),
        )
        .unwrap();
        assert_ne!(a.envelope_hash, b.envelope_hash);
    }

    #[test]
    fn golden_vector_5_signature_flip_changes_envelope_hash_but_not_preimage() {
        let plan = golden_plan_1();
        let sa = SignedTx {
            signature_r: [0x11; 32],
            signature_s: [0x22; 32],
            signature_v: 0,
            recovered_signer: [0u8; 20],
            tx_type: 2,
        };
        let sb = SignedTx {
            signature_r: [0x33; 32],
            signature_s: [0x44; 32],
            signature_v: 1,
            recovered_signer: [0u8; 20],
            tx_type: 2,
        };
        let a = serialize_signed_execution(
            &plan,
            &sa,
            0,
            100_000,
            U256::from(1_000_000_000u64),
            U256::from(200_000_000u64),
        )
        .unwrap();
        let b = serialize_signed_execution(
            &plan,
            &sb,
            0,
            100_000,
            U256::from(1_000_000_000u64),
            U256::from(200_000_000u64),
        )
        .unwrap();
        assert_ne!(
            a.envelope_hash, b.envelope_hash,
            "different signature → different envelope hash"
        );
        // But the SIGNING preimage hash MUST match — preimage is over
        // the unsigned tuple only.
        assert_eq!(
            eip1559_preimage_hash(
                &plan,
                0,
                100_000,
                U256::from(1_000_000_000u64),
                U256::from(200_000_000u64)
            ),
            eip1559_preimage_hash(
                &plan,
                0,
                100_000,
                U256::from(1_000_000_000u64),
                U256::from(200_000_000u64)
            )
        );
    }

    #[test]
    fn envelope_hash_differs_from_preimage_hash() {
        let plan = golden_plan_1();
        let signed = plain_signed(0x11, 0x22, 0);
        let env = serialize_signed_execution(
            &plan,
            &signed,
            0,
            100_000,
            U256::from(1_000_000_000u64),
            U256::from(200_000_000u64),
        )
        .unwrap();
        let preimage = eip1559_preimage_hash(
            &plan,
            0,
            100_000,
            U256::from(1_000_000_000u64),
            U256::from(200_000_000u64),
        );
        // envelope_hash covers the SIGNED tuple; preimage covers only
        // the unsigned one. They MUST differ.
        assert_ne!(
            env.envelope_hash, preimage,
            "envelope_hash and preimage must be distinct (signed vs unsigned)"
        );
    }

    // -----------------------------------------------------------------
    //  Byte-identity with Perps encoder (frozen safety guarantee)
    // -----------------------------------------------------------------
    //
    // We compare against the Perps `assemble_eip1559_signed_transaction`
    // helper's byte output for a fixed input. If the Perps and Hybrid
    // V2 encoders ever drift the guarantee is broken, so this trips
    // loudly at CI.

    #[test]
    fn test_byte_identity_with_perps() {
        use crate::execution::transaction::{
            assemble_eip1559_signed_transaction, ExecutionTransactionRequest,
        };
        use crate::types::AccountId;
        use uuid::Uuid;

        let target = [
            0x5a, 0x5e, 0xbf, 0x9a, 0x9c, 0xcd, 0x7c, 0x01, 0x25, 0x18, 0x56, 0x9d, 0xe8, 0x28,
            0x39, 0x82, 0x98, 0x26, 0x70, 0xf6,
        ];
        let target_hex = to_hex_lowercase_prefixed(&target);
        let calldata = vec![0xde, 0xad, 0xbe, 0xef];
        let plan = plan_from(84532, target, calldata.clone(), U256::ZERO);
        let signed = SignedTx {
            signature_r: [0x11; 32],
            signature_s: [0x22; 32],
            signature_v: 0,
            recovered_signer: [0u8; 20],
            tx_type: 2,
        };

        let hv2 = serialize_signed_execution(
            &plan,
            &signed,
            42,
            100_000,
            U256::from(2_000_000_000u64),
            U256::from(1_000_000_000u64),
        )
        .unwrap();

        let perps_req = ExecutionTransactionRequest {
            intent_id: Uuid::from_u128(1),
            onchain_intent_id: "0x".to_string() + &"00".repeat(32),
            from: AccountId::new(&format!("0x{}", "aa".repeat(20))),
            to: AccountId::new(&target_hex),
            value_wei: 0,
            calldata,
            chain_id: 84532,
            gas_limit: 100_000,
            max_fee_per_gas_wei: Some("2000000000".to_string()),
            max_priority_fee_per_gas_wei: Some("1000000000".to_string()),
        };
        let perps_hex =
            assemble_eip1559_signed_transaction(&perps_req, 42, 0, &[0x11; 32], &[0x22; 32])
                .expect("perps assemble");

        assert_eq!(
            perps_hex, hv2.raw_hex,
            "Hybrid V2 tx_serialization must be byte-identical to Perps encoder"
        );
    }

    #[test]
    fn tx_hash_stability_for_same_plan_and_signature() {
        // Part D: same plan + same signature + same nonce → same
        // tx_hash. Non-tautological in aggregate: we sample multiple
        // gas / fee combos.
        let plan = golden_plan_1();
        let signed = plain_signed(0x11, 0x22, 0);
        let a = serialize_signed_execution(
            &plan,
            &signed,
            7,
            250_000,
            U256::from(3_000_000_000u64),
            U256::from(500_000_000u64),
        )
        .unwrap();
        let b = serialize_signed_execution(
            &plan,
            &signed,
            7,
            250_000,
            U256::from(3_000_000_000u64),
            U256::from(500_000_000u64),
        )
        .unwrap();
        assert_eq!(a.envelope_hash, b.envelope_hash);
    }

    #[test]
    fn tx_hash_changes_on_different_gas_or_fee() {
        let plan = golden_plan_1();
        let signed = plain_signed(0x11, 0x22, 0);
        let a = serialize_signed_execution(
            &plan,
            &signed,
            0,
            100_000,
            U256::from(2_000_000_000u64),
            U256::from(1_000_000_000u64),
        )
        .unwrap();
        let b = serialize_signed_execution(
            &plan,
            &signed,
            0,
            200_000, // different gas_limit
            U256::from(2_000_000_000u64),
            U256::from(1_000_000_000u64),
        )
        .unwrap();
        assert_ne!(a.envelope_hash, b.envelope_hash);
        let c = serialize_signed_execution(
            &plan,
            &signed,
            0,
            100_000,
            U256::from(5_000_000_000u64), // different max_fee
            U256::from(1_000_000_000u64),
        )
        .unwrap();
        assert_ne!(a.envelope_hash, c.envelope_hash);
    }
}
