//! Test-only in-process ephemeral signer for the Hybrid V2 pipeline.
//!
//! **Gating.** Only compiled under `#[cfg(any(test, feature =
//! "test-signer"))]`. Non-test builds cannot instantiate this type —
//! the module is invisible to them. Production builds MUST use
//! `SignerBackend::Production`, which yields
//! [`crate::hybrid_v2::execution::signer_production::ProductionSignerUnavailable`].
//!
//! **Key material.** The signer holds a `k256::ecdsa::SigningKey`
//! purely in-process. Nothing is persisted; nothing is exported over
//! the wire. The `Debug` impl deliberately omits the key bytes.
//!
//! This exists so unit + PG-integration tests can validate the
//! cryptographic verification path end-to-end without touching a real
//! external signer.

use crate::hybrid_v2::execution::signer::{
    ExecutionSigner, SignedTx, SignerError, SignerIdentity, SignerKind, SigningRequest,
};
use async_trait::async_trait;
use k256::ecdsa::{Signature, SigningKey};
use sha3::{Digest, Keccak256};

pub struct TestEphemeralSigner {
    key: SigningKey,
    address: [u8; 20],
}

impl std::fmt::Debug for TestEphemeralSigner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Deliberately do NOT print `key`. Address is safe to log.
        f.debug_struct("TestEphemeralSigner")
            .field("address", &format!("0x{}", hex_encode(&self.address)))
            .field("key", &"<redacted>")
            .finish()
    }
}

impl TestEphemeralSigner {
    /// Generate a fresh random signer. Uses `rand::thread_rng` for
    /// key material — test-only, never production.
    pub fn generate() -> Self {
        let mut rng = rand::thread_rng();
        let key = SigningKey::random(&mut rng);
        let address = derive_address(&key);
        Self { key, address }
    }

    /// Deterministic signer from a 32-byte seed. Useful for
    /// reproducible tests.
    pub fn from_seed(seed: [u8; 32]) -> Self {
        // Derive a valid non-zero private key by hashing the seed with
        // a domain tag until we get a valid scalar (extremely likely
        // on the first try).
        let mut candidate = seed;
        loop {
            if let Ok(key) = SigningKey::from_bytes(candidate.as_ref().into()) {
                let address = derive_address(&key);
                return Self { key, address };
            }
            let mut h = Keccak256::new();
            h.update(b"HV2_TEST_SEED_V1");
            h.update(candidate);
            let out = h.finalize();
            candidate.copy_from_slice(&out[..]);
        }
    }

    pub fn address(&self) -> [u8; 20] {
        self.address
    }
}

#[async_trait]
impl ExecutionSigner for TestEphemeralSigner {
    fn identity(&self) -> SignerIdentity {
        SignerIdentity {
            address: self.address,
            kind: SignerKind::TestEphemeral,
            uri: None,
        }
    }

    async fn sign_execution(&self, request: SigningRequest) -> Result<SignedTx, SignerError> {
        // Local re-verification (the firewall does this independently,
        // but a well-behaved signer defends itself too).
        let recomputed = Keccak256::digest(&request.calldata);
        let mut got = [0u8; 32];
        got.copy_from_slice(&recomputed[..]);
        if got != request.calldata_hash {
            return Err(SignerError::CalldataHashMismatch);
        }

        // Sign the caller-provided signing_payload_hash directly (a
        // pre-hashed 32-byte digest). Production signers would derive
        // it locally from the EIP-1559 RLP preimage.
        let (signature, recid) = self
            .key
            .sign_prehash_recoverable(&request.signing_payload_hash)
            .map_err(|e| SignerError::MalformedResponse(format!("sign_prehash: {e}")))?;
        // Normalize S — reject high-S; the caller (Part O) also
        // rejects at verification time. `normalize_s` returns Some
        // iff normalization was necessary (the input WAS high-S) —
        // in that case the recovery id parity flips.
        let (normalized, recid): (Signature, k256::ecdsa::RecoveryId) = if let Some(n) =
            signature.normalize_s()
        {
            let flipped = k256::ecdsa::RecoveryId::from_byte(recid.to_byte() ^ 1).unwrap_or(recid);
            (n, flipped)
        } else {
            (signature, recid)
        };

        let bytes = normalized.to_bytes();
        let mut r = [0u8; 32];
        let mut s = [0u8; 32];
        r.copy_from_slice(&bytes[..32]);
        s.copy_from_slice(&bytes[32..64]);

        Ok(SignedTx {
            signature_r: r,
            signature_s: s,
            signature_v: recid.to_byte(),
            recovered_signer: self.address,
            tx_type: request.tx_type,
        })
    }
}

fn derive_address(key: &SigningKey) -> [u8; 20] {
    let verifying = key.verifying_key();
    // Uncompressed SEC1 = 65 bytes: 0x04 || X(32) || Y(32).
    let encoded = verifying.to_encoded_point(false);
    let pubkey_bytes = &encoded.as_bytes()[1..]; // drop 0x04 prefix, 64 bytes remain
    let hash = Keccak256::digest(pubkey_bytes);
    let mut out = [0u8; 20];
    out.copy_from_slice(&hash[12..]);
    out
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{:02x}", b));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::U256;

    fn req_matching_calldata(calldata: Vec<u8>) -> SigningRequest {
        let calldata_hash = {
            let mut h = Keccak256::new();
            h.update(&calldata);
            let out = h.finalize();
            let mut a = [0u8; 32];
            a.copy_from_slice(&out[..]);
            a
        };
        // For the test the signing_payload_hash is just a keccak256
        // over "HV2_TEST_PAYLOAD" || calldata_hash — enough to prove
        // the signature/recovery loop closes.
        let mut h = Keccak256::new();
        h.update(b"HV2_TEST_PAYLOAD");
        h.update(calldata_hash);
        let out = h.finalize();
        let mut signing_payload_hash = [0u8; 32];
        signing_payload_hash.copy_from_slice(&out[..]);
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
            signing_payload_hash,
            calldata,
        }
    }

    #[tokio::test]
    async fn round_trip_sign_recovers_signer_address() {
        let signer = TestEphemeralSigner::from_seed([0x42u8; 32]);
        let req = req_matching_calldata(vec![0x12u8, 0x34, 0x56]);
        let signed = signer.sign_execution(req).await.unwrap();
        assert_eq!(signed.recovered_signer, signer.address());
        assert_eq!(signed.tx_type, 2);
        assert!(signed.signature_v <= 1);
    }

    #[tokio::test]
    async fn calldata_hash_mismatch_rejected() {
        let signer = TestEphemeralSigner::from_seed([0x42u8; 32]);
        let mut req = req_matching_calldata(vec![0x00u8]);
        req.calldata_hash = [0xffu8; 32];
        let err = signer.sign_execution(req).await.unwrap_err();
        assert!(matches!(err, SignerError::CalldataHashMismatch));
    }

    #[test]
    fn seed_yields_deterministic_address() {
        let a = TestEphemeralSigner::from_seed([0x01u8; 32]);
        let b = TestEphemeralSigner::from_seed([0x01u8; 32]);
        assert_eq!(a.address(), b.address());
    }
}
