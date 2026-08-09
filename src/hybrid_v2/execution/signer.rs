//! Signer abstraction for Hybrid V2 execution (Part L of
//! `BACKEND-HYBRID-V2-SIGNER-AND-EXECUTION-V1`).
//!
//! ## Signer verdict
//!
//! `BACKEND_HYBRID_V2_SIGNER_INTERFACE_READY_EXTERNAL_SIGNER_REQUIRED`.
//!
//! The trait accepts a canonically-shaped [`SigningRequest`] and
//! returns a decomposed [`SignedTx`] `(r, s, v, recovered_signer)`.
//! It never exposes a raw hex transaction ready for a broadcast RPC —
//! reconstruction of the broadcastable payload is deliberately out of
//! scope for this milestone, and the omission is a broadcast
//! kill-switch by construction.
//!
//! Two implementations ship:
//!   * [`crate::hybrid_v2::execution::signer_production::ProductionSignerUnavailable`]
//!     — default. Returns `SignerError::SignerUnavailable(...)` for
//!     every request. This is the honest verdict: production signer
//!     integration (remote KMS / hardware) is a separate milestone.
//!   * [`crate::hybrid_v2::execution::signer_ephemeral::TestEphemeralSigner`]
//!     — gated behind `#[cfg(any(test, feature = "test-signer"))]`.
//!     Holds a k256 SigningKey in-process, used only by unit + PG
//!     tests to prove the round-trip.
//!
//! **No production key material is ever committed to this repo.**

use alloy_primitives::U256;
use async_trait::async_trait;
use thiserror::Error;

/// Structural identifier for a signer. Address is authoritative;
/// `kind` + `uri` are for operator logs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignerIdentity {
    pub address: [u8; 20],
    pub kind: SignerKind,
    pub uri: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SignerKind {
    /// No signer is configured — `ProductionSignerUnavailable`.
    None,
    /// In-process ephemeral signer (test-only).
    TestEphemeral,
    /// External remote KMS backend (future).
    RemoteKMS,
    /// Multi-sig / Safe module signer (future).
    RemoteSafe,
}

/// Selector for which signer backend to instantiate. Default is
/// `Production` — which yields `ProductionSignerUnavailable`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SignerBackend {
    /// Production posture — signer must be provided externally.
    /// Without it the pipeline yields `SIGNER_UNAVAILABLE` (Part N).
    Production,
    /// Test-only ephemeral signer, seeded deterministically. Only
    /// available under `#[cfg(any(test, feature = "test-signer"))]`;
    /// non-test builds reject this variant at construction time.
    TestEphemeral([u8; 32]),
}

impl Default for SignerBackend {
    fn default() -> Self {
        SignerBackend::Production
    }
}

/// Fully-formed signing request. The pipeline populates every field
/// from the deterministic plan + reserved nonce + gas policy output.
/// The signer MUST re-verify `calldata_hash` and `plan_hash` before
/// producing a signature; the firewall (Part M) enforces the same
/// checks independently at the boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SigningRequest {
    pub chain_id: u64,
    pub nonce: u64,
    pub target: [u8; 20],
    pub value_wei: U256,
    /// keccak256(calldata) — the signer must recompute and verify.
    pub calldata_hash: [u8; 32],
    pub gas_limit: u64,
    pub max_fee_per_gas_wei: U256,
    pub max_priority_fee_per_gas_wei: U256,
    /// 2 = EIP-1559.
    pub tx_type: u8,
    pub plan_hash: [u8; 32],
    /// The exact 32-byte preimage-hash the signer signs (typically
    /// keccak256 over the EIP-1559 preimage). The signer must derive
    /// this locally from the other fields and verify equality.
    pub signing_payload_hash: [u8; 32],
    /// Raw calldata bytes. Provided so a signer that needs to
    /// reconstruct the preimage has the full context — but the signer
    /// MUST verify `keccak256(calldata) == calldata_hash` before use.
    pub calldata: Vec<u8>,
}

/// Signed transaction output — decomposed into (r, s, v) rather than a
/// raw hex transaction, deliberately, so that reconstructing the
/// broadcastable payload requires an explicit, separate milestone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignedTx {
    pub signature_r: [u8; 32],
    pub signature_s: [u8; 32],
    /// EIP-1559: `y_parity` — always 0 or 1.
    pub signature_v: u8,
    pub recovered_signer: [u8; 20],
    pub tx_type: u8,
}

#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum SignerError {
    #[error("signer unavailable: {0}")]
    SignerUnavailable(String),
    #[error("signer policy violation: {0}")]
    PolicyViolation(String),
    #[error("signer network failure: {0}")]
    NetworkFailure(String),
    #[error("signer timeout")]
    Timeout,
    #[error("signer returned malformed payload: {0}")]
    MalformedResponse(String),
    #[error("signer identity mismatch expected 0x{expected:x?} actual 0x{actual:x?}")]
    IdentityMismatch {
        expected: [u8; 20],
        actual: [u8; 20],
    },
    #[error("plan hash mismatch — signer refuses tampered plan")]
    PlanHashMismatch,
    #[error("calldata hash mismatch — signer refuses tampered calldata")]
    CalldataHashMismatch,
    #[error("chain id mismatch — signer refuses non-configured chain")]
    ChainMismatch,
}

/// Availability model surfaced by [`ExecutionSigner::availability`] (Part H).
///
/// The orchestrator inspects this before invoking `sign_execution` and
/// surfaces the reason via the admin route's structured 503 body when
/// the wired signer is not `Configured`. Signer outage MUST NOT crash
/// the read-side backend — `AppState` construction downgrades to
/// `orchestrator = None`, and every non-`Configured` variant carries
/// enough context for an operator to diagnose the root cause without
/// re-hitting the vendor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SignerAvailability {
    /// No production signer wired yet — e.g. `ProductionSignerUnavailable`.
    NotConfigured,
    /// Signer is wired and construction succeeded. `probe()` may still
    /// return a transient/deterministic failure downstream — this is
    /// the "wire is complete" verdict, not "vendor is definitely up".
    Configured,
    /// Last observed health check failed with a transport reason.
    /// Distinct from `AuthenticationFailed` and `IdentityMismatch`.
    Unavailable { reason: String },
    /// Vendor rejected the auth material. Operator must inspect
    /// `HV2_SIGNER_AUTH_REFERENCE` and the microservice's own auth path.
    AuthenticationFailed { reason: String },
    /// Vendor produced a signature that recovered to a different EOA.
    /// Fatal — DO NOT auto-retry, DO NOT auto-fallback.
    IdentityMismatch {
        expected: [u8; 20],
        observed: [u8; 20],
    },
    /// Timeout / 5xx / rate-limit — retryable.
    TransientFailure { reason: String },
    /// Deterministic vendor policy refusal (`kms_key_disabled`, etc).
    /// NOT retryable.
    DeterministicRefusal { reason: String },
    /// Operator explicitly disabled the signer via config
    /// (`execution_enabled = false` or the admin kill-switch).
    Disabled,
}

#[async_trait]
pub trait ExecutionSigner: Send + Sync {
    fn identity(&self) -> SignerIdentity;
    async fn sign_execution(&self, request: SigningRequest) -> Result<SignedTx, SignerError>;
    /// Availability verdict. Default implementation returns
    /// `Configured` — every signer that overrides this reports its
    /// own state. `ProductionSignerUnavailable` overrides to
    /// `NotConfigured`; the KMS bridge exposes its live status via
    /// `probe()` and reports `Configured` at construction time.
    fn availability(&self) -> SignerAvailability {
        SignerAvailability::Configured
    }
}

// -----------------------------------------------------------------
//                          UNIT TESTS
// -----------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_backend_is_production() {
        assert_eq!(SignerBackend::default(), SignerBackend::Production);
    }

    #[test]
    fn signed_tx_carries_no_raw_transaction_field() {
        // Structural regression: no field of SignedTx should be a raw
        // transaction blob or hex string ready for a broadcast RPC.
        // Assert by name enumeration.
        let tx = SignedTx {
            signature_r: [0; 32],
            signature_s: [0; 32],
            signature_v: 0,
            recovered_signer: [0; 20],
            tx_type: 2,
        };
        // Debug format lists field names; verify no forbidden token.
        let s = format!("{tx:?}");
        assert!(!s.to_ascii_lowercase().contains("raw"));
        assert!(!s.to_ascii_lowercase().contains("rlp"));
    }
}
