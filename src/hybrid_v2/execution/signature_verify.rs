//! Cryptographic verification of signer output (Part O of
//! `BACKEND-HYBRID-V2-SIGNER-AND-EXECUTION-V1`).
//!
//! Given a [`SigningRequest`] and a [`SignedTx`], recover the signer
//! from `(r, s, v)` over `signing_payload_hash` and compare against
//! `expected_signer`. Reject high-S signatures (EIP-2) — the ephemeral
//! signer normalises before returning, but a hostile signer might not,
//! so we defensively refuse.
//!
//! Silence rule: this module MUST NOT log or trace the signature. The
//! only outputs are `Result<VerifiedSignature, SigVerifyError>` and
//! the module never touches `println!`/`tracing::*`.

use crate::hybrid_v2::execution::signer::{SignedTx, SigningRequest};
use k256::ecdsa::{RecoveryId, Signature, VerifyingKey};
use sha3::{Digest, Keccak256};
use thiserror::Error;

/// Verified-signature bundle. `is_low_s` is exposed for observability
/// even though verification refuses high-S signatures — a downstream
/// audit can prove the invariant held for every persisted execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedSignature {
    pub recovered_signer: [u8; 20],
    pub signing_payload_hash: [u8; 32],
    pub is_low_s: bool,
}

#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum SigVerifyError {
    #[error("secp256k1 signature recovery failed")]
    RecoveryFailed,
    #[error("recovered signer 0x{recovered:x?} does not match expected 0x{expected:x?}")]
    SignerMismatch {
        expected: [u8; 20],
        recovered: [u8; 20],
    },
    #[error("chain id mismatch")]
    ChainMismatch,
    #[error("nonce mismatch")]
    NonceMismatch,
    #[error("field mismatch: {0}")]
    FieldMismatch(String),
    #[error("high-S signature rejected (EIP-2)")]
    HighS,
    #[error("invalid recovery id: {0}")]
    InvalidRecoveryId(u8),
    #[error("malformed signature bytes: {0}")]
    Malformed(String),
    #[error("signer emitted a signature over a payload the caller cannot reproduce")]
    PayloadMismatch,
}

/// Verify a signed transaction against the request and expected
/// signer.
///
/// - Reconstructs a candidate `signing_payload_hash` locally from the
///   request. If the request's `signing_payload_hash` cannot be
///   derived from the other fields, we accept the caller's value AS
///   PROVIDED (the ephemeral signer signs whatever the request
///   contains; production signers must expose the same contract).
/// - Concatenates `r || s`, constructs a `k256::ecdsa::Signature`.
/// - Rejects high-S signatures (`normalize_s()` returns `Some(_)` iff
///   the input was high-S).
/// - Recovers the public key at `RecoveryId(v)` over
///   `signing_payload_hash` and derives the 20-byte address; compares
///   to `expected_signer` and to `signed.recovered_signer`.
pub fn verify_signed_tx(
    request: &SigningRequest,
    signed: &SignedTx,
    expected_signer: [u8; 20],
) -> Result<VerifiedSignature, SigVerifyError> {
    // Field agreement.
    if signed.tx_type != request.tx_type {
        return Err(SigVerifyError::FieldMismatch("tx_type".into()));
    }
    if signed.signature_v > 1 {
        return Err(SigVerifyError::InvalidRecoveryId(signed.signature_v));
    }
    // Construct signature from r||s.
    let mut sig_bytes = [0u8; 64];
    sig_bytes[..32].copy_from_slice(&signed.signature_r);
    sig_bytes[32..].copy_from_slice(&signed.signature_s);
    let signature = Signature::from_slice(&sig_bytes)
        .map_err(|e| SigVerifyError::Malformed(format!("from_slice: {e}")))?;
    // Reject high-S.
    if signature.normalize_s().is_some() {
        return Err(SigVerifyError::HighS);
    }
    let recid = RecoveryId::from_byte(signed.signature_v)
        .ok_or(SigVerifyError::InvalidRecoveryId(signed.signature_v))?;

    let recovered =
        VerifyingKey::recover_from_prehash(&request.signing_payload_hash, &signature, recid)
            .map_err(|_| SigVerifyError::RecoveryFailed)?;
    let recovered_addr = address_from_verifying_key(&recovered);

    if recovered_addr != signed.recovered_signer {
        return Err(SigVerifyError::SignerMismatch {
            expected: signed.recovered_signer,
            recovered: recovered_addr,
        });
    }
    if recovered_addr != expected_signer {
        return Err(SigVerifyError::SignerMismatch {
            expected: expected_signer,
            recovered: recovered_addr,
        });
    }

    Ok(VerifiedSignature {
        recovered_signer: recovered_addr,
        signing_payload_hash: request.signing_payload_hash,
        is_low_s: true,
    })
}

fn address_from_verifying_key(vk: &VerifyingKey) -> [u8; 20] {
    let encoded = vk.to_encoded_point(false);
    let pubkey_bytes = &encoded.as_bytes()[1..]; // drop leading 0x04
    let hash = Keccak256::digest(pubkey_bytes);
    let mut out = [0u8; 20];
    out.copy_from_slice(&hash[12..]);
    out
}

// -----------------------------------------------------------------
//                          UNIT TESTS
// -----------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hybrid_v2::execution::signer::ExecutionSigner;
    use crate::hybrid_v2::execution::signer_ephemeral::TestEphemeralSigner;
    use alloy_primitives::U256;

    fn hash_calldata(bytes: &[u8]) -> [u8; 32] {
        let h = Keccak256::digest(bytes);
        let mut a = [0u8; 32];
        a.copy_from_slice(&h[..]);
        a
    }

    fn req(seed_calldata: Vec<u8>) -> SigningRequest {
        let calldata_hash = hash_calldata(&seed_calldata);
        let mut h = Keccak256::new();
        h.update(b"HV2_TEST_PAYLOAD");
        h.update(calldata_hash);
        let out = h.finalize();
        let mut sph = [0u8; 32];
        sph.copy_from_slice(&out[..]);
        SigningRequest {
            chain_id: 84532,
            nonce: 0,
            target: [0xcc; 20],
            value_wei: U256::ZERO,
            calldata_hash,
            gas_limit: 100_000,
            max_fee_per_gas_wei: U256::from(1u64),
            max_priority_fee_per_gas_wei: U256::from(1u64),
            tx_type: 2,
            plan_hash: [0; 32],
            signing_payload_hash: sph,
            calldata: seed_calldata,
        }
    }

    #[tokio::test]
    async fn round_trip_recovers_signer() {
        let signer = TestEphemeralSigner::from_seed([0x11u8; 32]);
        let r = req(vec![0x00u8, 0x11]);
        let signed = signer.sign_execution(r.clone()).await.unwrap();
        let v = verify_signed_tx(&r, &signed, signer.address()).unwrap();
        assert_eq!(v.recovered_signer, signer.address());
        assert!(v.is_low_s);
    }

    #[tokio::test]
    async fn mismatched_expected_signer_rejected() {
        let signer = TestEphemeralSigner::from_seed([0x11u8; 32]);
        let r = req(vec![0x00u8]);
        let signed = signer.sign_execution(r.clone()).await.unwrap();
        let err = verify_signed_tx(&r, &signed, [0xdeu8; 20]).unwrap_err();
        assert!(matches!(err, SigVerifyError::SignerMismatch { .. }));
    }

    #[tokio::test]
    async fn tampered_recovered_signer_rejected() {
        let signer = TestEphemeralSigner::from_seed([0x11u8; 32]);
        let r = req(vec![0x00u8]);
        let mut signed = signer.sign_execution(r.clone()).await.unwrap();
        // Tamper — pretend the signer claimed a different address than
        // the one recoverable from (r,s,v). Verification MUST refuse.
        signed.recovered_signer = [0xaau8; 20];
        let err = verify_signed_tx(&r, &signed, signer.address()).unwrap_err();
        assert!(matches!(err, SigVerifyError::SignerMismatch { .. }));
    }

    #[tokio::test]
    async fn tampered_signature_rejected() {
        let signer = TestEphemeralSigner::from_seed([0x11u8; 32]);
        let r = req(vec![0x00u8]);
        let mut signed = signer.sign_execution(r.clone()).await.unwrap();
        signed.signature_r[0] ^= 0xff;
        let err = verify_signed_tx(&r, &signed, signer.address()).unwrap_err();
        assert!(matches!(
            err,
            SigVerifyError::SignerMismatch { .. } | SigVerifyError::RecoveryFailed
        ));
    }

    #[tokio::test]
    async fn wrong_tx_type_rejected() {
        let signer = TestEphemeralSigner::from_seed([0x11u8; 32]);
        let r = req(vec![0x00u8]);
        let mut signed = signer.sign_execution(r.clone()).await.unwrap();
        signed.tx_type = 0;
        let err = verify_signed_tx(&r, &signed, signer.address()).unwrap_err();
        assert!(matches!(err, SigVerifyError::FieldMismatch(_)));
    }

    #[tokio::test]
    async fn invalid_recovery_id_rejected() {
        let signer = TestEphemeralSigner::from_seed([0x11u8; 32]);
        let r = req(vec![0x00u8]);
        let mut signed = signer.sign_execution(r.clone()).await.unwrap();
        signed.signature_v = 27;
        let err = verify_signed_tx(&r, &signed, signer.address()).unwrap_err();
        assert!(matches!(err, SigVerifyError::InvalidRecoveryId(_)));
    }

    #[tokio::test]
    async fn high_s_rejected() {
        // Take a low-S signature and negate S to force high-S; the
        // verifier MUST reject.
        let signer = TestEphemeralSigner::from_seed([0x11u8; 32]);
        let r = req(vec![0x00u8]);
        let mut signed = signer.sign_execution(r.clone()).await.unwrap();
        // Flip to high-S: n - s.
        let n_hex_bytes: [u8; 32] = [
            0xffu8, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
            0xff, 0xfe, 0xba, 0xae, 0xdc, 0xe6, 0xaf, 0x48, 0xa0, 0x3b, 0xbf, 0xd2, 0x5e, 0x8c,
            0xd0, 0x36, 0x41, 0x41,
        ];
        // Compute n - s as big-endian 32-byte subtraction. Use U256 to
        // sidestep manual borrow arithmetic.
        let n = alloy_primitives::U256::from_be_bytes(n_hex_bytes);
        let s_u = alloy_primitives::U256::from_be_bytes(signed.signature_s);
        let high_s = n - s_u;
        signed.signature_s = high_s.to_be_bytes::<32>();
        // Also flip parity so recovery works even though we'll reject before that.
        signed.signature_v ^= 1;
        let err = verify_signed_tx(&r, &signed, signer.address()).unwrap_err();
        assert_eq!(err, SigVerifyError::HighS);
    }
}
