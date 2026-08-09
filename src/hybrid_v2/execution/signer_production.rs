//! Production signer verdict for Hybrid V2 execution (Part N of
//! `BACKEND-HYBRID-V2-SIGNER-AND-EXECUTION-V1`).
//!
//! The default production posture is `SIGNER_UNAVAILABLE`. This module
//! exposes [`ProductionSignerUnavailable`], the concrete impl of
//! [`ExecutionSigner`] mounted when `SignerBackend::Production` is
//! selected.
//!
//! Every call to `sign_execution` returns `SignerError::SignerUnavailable(...)`.
//! This is the deliberate, honest posture: this milestone stops at the
//! signer boundary. Full signer integration (remote KMS / hardware
//! signer / Safe module) is a distinct future milestone that MUST NOT
//! be shipped without an audit.
//!
//! No private-key bytes exist anywhere in this file (or in any
//! non-test-gated file in this module tree).

use crate::hybrid_v2::execution::signer::{
    ExecutionSigner, SignedTx, SignerAvailability, SignerError, SignerIdentity, SignerKind,
    SigningRequest,
};
use async_trait::async_trait;

/// Sentinel signer — always returns `SignerUnavailable`. Used as the
/// default `Production` binding so a build that never wires a real
/// signer fails safely instead of silently degrading.
#[derive(Debug, Clone)]
pub struct ProductionSignerUnavailable {
    reason: String,
}

impl ProductionSignerUnavailable {
    pub fn new(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
        }
    }

    /// Convenience: the default reason string when no external signer
    /// has been provisioned.
    pub fn default_reason() -> Self {
        Self::new(
            "BACKEND_HYBRID_V2_SIGNER_INTERFACE_READY_EXTERNAL_SIGNER_REQUIRED — no production \
             signer is wired for this build",
        )
    }
}

#[async_trait]
impl ExecutionSigner for ProductionSignerUnavailable {
    fn identity(&self) -> SignerIdentity {
        SignerIdentity {
            address: [0u8; 20],
            kind: SignerKind::None,
            uri: None,
        }
    }

    async fn sign_execution(&self, _: SigningRequest) -> Result<SignedTx, SignerError> {
        Err(SignerError::SignerUnavailable(self.reason.clone()))
    }

    fn availability(&self) -> SignerAvailability {
        SignerAvailability::NotConfigured
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::U256;

    fn req() -> SigningRequest {
        SigningRequest {
            chain_id: 84532,
            nonce: 0,
            target: [0xcc; 20],
            value_wei: U256::ZERO,
            calldata_hash: [0; 32],
            gas_limit: 100_000,
            max_fee_per_gas_wei: U256::from(1u64),
            max_priority_fee_per_gas_wei: U256::from(1u64),
            tx_type: 2,
            plan_hash: [0; 32],
            signing_payload_hash: [0; 32],
            calldata: vec![],
        }
    }

    #[tokio::test]
    async fn always_unavailable() {
        let signer = ProductionSignerUnavailable::default_reason();
        let err = signer.sign_execution(req()).await.unwrap_err();
        assert!(matches!(err, SignerError::SignerUnavailable(_)));
        // Identity is a zero-address so the caller cannot mistakenly
        // trust an all-zero signer.
        assert_eq!(signer.identity().address, [0u8; 20]);
        assert_eq!(signer.identity().kind, SignerKind::None);
    }
}
