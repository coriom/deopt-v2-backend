//! `BACKEND-HYBRID-V2-EXTERNAL-SIGNER-INTEGRATION-AND-LIVE-ORCHESTRATOR-V1`
//! (Package A, Part E) — signer builder that dispatches on
//! `SignerProvider` and hands back an
//! `Arc<dyn ExecutionSigner>` for the orchestrator.
//!
//! Frozen posture:
//!
//! * **NO_NEW_CARGO_DEPS** — the builder only wires types that are
//!   already in the crate graph (`RemoteSignerClient`,
//!   `PluggableRemoteSignerTransport`, `AwsKmsSignerProvider`).
//! * **AWS_TRANSPORT_GATED_BY_FEATURE** — `SignerProvider::KmsAws`
//!   requires the `aws-kms-transport` cargo feature at build time.
//!   Without it the builder returns `SignerUnavailable("aws-kms-
//!   transport feature not enabled at build time")` so an operator
//!   who ships a non-AWS build with `HV2_SIGNER_PROVIDER=kms_aws`
//!   fails closed rather than silently degrading.
//! * **UNIMPLEMENTED_VENDORS_FAIL_CLOSED** — `KmsGcp`, `Turnkey`,
//!   `Fireblocks` return `SignerUnavailable("provider X not yet
//!   integrated")` at build time. NEVER invent HTTP protocols for a
//!   vendor we can't actually reach.
//! * **MOCK_ONLY_UNDER_TEST_SIGNER** — `SignerProvider::Mock` is
//!   accepted only under `#[cfg(any(test, feature = "test-signer"))]`.
//!   Non-test builds refuse it at build time.

use std::sync::Arc;
use std::time::Duration;

use crate::error::{BackendError, Result};
use crate::hybrid_v2::config::{HybridV2ExecutionConfig, SignerProvider};
use crate::hybrid_v2::execution::signer::{ExecutionSigner, SignerBackend, SignerKind};
use crate::hybrid_v2::execution::signer_kms_bridge::{redacted_endpoint, HybridV2KmsSignerBridge};
use crate::hybrid_v2::execution::signer_production::ProductionSignerUnavailable;

/// Concrete builder for the Hybrid V2 [`ExecutionSigner`]. See the
/// module docs for the frozen posture; see
/// [`HybridV2ExecutionConfig::validate_startup`] for the invariants
/// this builder assumes have already been checked.
pub struct HybridV2SignerBuilder;

impl HybridV2SignerBuilder {
    /// Build the signer that the orchestrator will hold. Assumes the
    /// caller has already invoked `config.validate_startup(chain_id)`
    /// — this method re-checks the Production-specific field presence
    /// as a belt-and-suspenders guard but does NOT re-validate every
    /// bound.
    pub fn build(config: &HybridV2ExecutionConfig) -> Result<Arc<dyn ExecutionSigner>> {
        match &config.signer_kind {
            SignerBackend::Production => Self::build_production(config),
            SignerBackend::TestEphemeral(seed) => Self::build_test_ephemeral(*seed),
        }
    }

    #[cfg(any(test, feature = "test-signer"))]
    fn build_test_ephemeral(seed: [u8; 32]) -> Result<Arc<dyn ExecutionSigner>> {
        let signer =
            crate::hybrid_v2::execution::signer_ephemeral::TestEphemeralSigner::from_seed(seed);
        Ok(Arc::new(signer))
    }

    #[cfg(not(any(test, feature = "test-signer")))]
    fn build_test_ephemeral(_seed: [u8; 32]) -> Result<Arc<dyn ExecutionSigner>> {
        // Non-test build with the `test-signer` feature disabled MUST
        // NOT instantiate an in-process signer that holds raw key
        // bytes. The config validator rejects this variant earlier
        // (see `HybridV2ExecutionConfig::from_env`); the runtime
        // refusal here is belt-and-suspenders.
        Err(BackendError::Config(
            "SignerBackend::TestEphemeral requires the `test-signer` cargo feature; \
             non-test builds refuse to instantiate a key-in-memory signer"
                .to_string(),
        ))
    }

    fn build_production(config: &HybridV2ExecutionConfig) -> Result<Arc<dyn ExecutionSigner>> {
        let expected_signer_address = config.expected_signer_address.ok_or_else(|| {
            BackendError::Config(
                "IncompleteProductionSignerConfig: expected_signer_address required".to_string(),
            )
        })?;
        let endpoint = config.signer_endpoint.as_deref().ok_or_else(|| {
            BackendError::Config(
                "IncompleteProductionSignerConfig: signer_endpoint required".to_string(),
            )
        })?;
        let provider = config.signer_provider.ok_or_else(|| {
            BackendError::Config(
                "IncompleteProductionSignerConfig: signer_provider required".to_string(),
            )
        })?;
        let request_timeout = Duration::from_millis(config.signer_request_timeout_ms as u64);
        let max_retries = config.signer_max_retries;
        let endpoint_redacted = redacted_endpoint(endpoint);

        match provider {
            SignerProvider::KmsAws => Self::build_kms_aws(
                config,
                expected_signer_address,
                endpoint,
                endpoint_redacted,
                max_retries,
                request_timeout,
            ),
            SignerProvider::KmsGcp => Ok(Arc::new(ProductionSignerUnavailable::new(
                "signer provider `kms_gcp` is not yet integrated in this build; \
                 pin an operational vendor via HV2_SIGNER_PROVIDER=kms_aws + \
                 the `aws-kms-transport` feature",
            ))),
            SignerProvider::Turnkey => Ok(Arc::new(ProductionSignerUnavailable::new(
                "signer provider `turnkey` is not yet integrated in this build",
            ))),
            SignerProvider::Fireblocks => Ok(Arc::new(ProductionSignerUnavailable::new(
                "signer provider `fireblocks` is not yet integrated in this build",
            ))),
            SignerProvider::Mock => Self::build_mock(
                config,
                expected_signer_address,
                endpoint,
                endpoint_redacted,
                max_retries,
                request_timeout,
            ),
        }
    }

    fn build_kms_aws(
        config: &HybridV2ExecutionConfig,
        _expected_signer_address: [u8; 20],
        _endpoint: &str,
        _endpoint_redacted: String,
        _max_retries: u32,
        _request_timeout: Duration,
    ) -> Result<Arc<dyn ExecutionSigner>> {
        // Enforce operator hygiene at build time.
        if config.signer_kms_key_id.is_none() {
            return Err(BackendError::Config(
                "HV2_SIGNER_KMS_KEY_ID is required when signer_provider=kms_aws".to_string(),
            ));
        }
        #[cfg(feature = "aws-kms-transport")]
        {
            // Per Pattern C the backend does NOT construct an
            // `aws_sdk_kms::Client` directly — that lives inside the
            // signer microservice which authenticates to AWS with its
            // own IAM role. The Perps `AwsKmsSdkTransport::new(client)`
            // takes a caller-supplied client; the follow-on operator
            // wiring milestone will assemble it from `aws-config`.
            // Until then, `KmsAws` returns a fail-closed signer with
            // a structured reason so the admin route surfaces
            // `SIGNER_UNAVAILABLE` per Part H instead of silently
            // degrading.
            //
            // This is the honest verdict: an operator with a real KMS
            // microservice injects the wired `HybridV2KmsSignerBridge`
            // via a separate AppState builder path. Without the
            // injection, the admin route reports
            // `EXECUTION_ORCHESTRATOR_NOT_WIRED`. Never invent an HTTP
            // protocol for a provider we cannot actually reach.
            Ok(Arc::new(ProductionSignerUnavailable::new(
                "aws-kms-transport SDK transport constructor requires an operator-supplied \
                 aws_sdk_kms::Client (Pattern C); the follow-on operator wiring milestone \
                 injects the bridge directly. Until then this build reports SIGNER_UNAVAILABLE.",
            )))
        }
        #[cfg(not(feature = "aws-kms-transport"))]
        {
            Ok(Arc::new(ProductionSignerUnavailable::new(
                "aws-kms-transport feature not enabled at build time — rebuild with \
                 `cargo build --features aws-kms-transport` to enable the KMS transport",
            )))
        }
    }

    #[cfg(any(test, feature = "test-signer"))]
    fn build_mock(
        _config: &HybridV2ExecutionConfig,
        expected_signer_address: [u8; 20],
        endpoint: &str,
        endpoint_redacted: String,
        max_retries: u32,
        request_timeout: Duration,
    ) -> Result<Arc<dyn ExecutionSigner>> {
        use crate::execution::config::PrivateKeySecret;
        use crate::execution::remote_signer::RemoteSignerClient;
        use crate::execution::signer::ExecutorSigner;
        use crate::execution::signer_adapters::{
            MockProviderMode, MockVendorSignerProvider, PluggableRemoteSignerTransport,
        };
        // Reuse the perps test-key convention so the recovered address
        // is deterministic. Callers that need a specific address for
        // an integration test should either use TestEphemeral or
        // provide expected_signer_address matching this key.
        const TEST_KEY: &str = "0x4c0883a69102937d6231471b5dbb6204fe5129617082792ae468d01a3f362318";
        let inner = ExecutorSigner::from_private_key(&PrivateKeySecret::new(TEST_KEY.to_string()))
            .map_err(|e| BackendError::Config(format!("mock inner signer init: {e}")))?;
        let provider = Arc::new(MockVendorSignerProvider::new(
            MockProviderMode::Success,
            inner,
        ));
        let plug = Arc::new(PluggableRemoteSignerTransport::new(
            provider,
            address_to_account_id(&expected_signer_address),
        ));
        let client = Arc::new(RemoteSignerClient::with_transport(
            endpoint.to_string(),
            address_to_account_id(&expected_signer_address),
            plug,
        ));
        Ok(Arc::new(HybridV2KmsSignerBridge::new(
            client,
            expected_signer_address,
            SignerKind::RemoteKMS,
            endpoint_redacted,
            max_retries,
            request_timeout,
        )))
    }

    #[cfg(not(any(test, feature = "test-signer")))]
    fn build_mock(
        _config: &HybridV2ExecutionConfig,
        _expected_signer_address: [u8; 20],
        _endpoint: &str,
        _endpoint_redacted: String,
        _max_retries: u32,
        _request_timeout: Duration,
    ) -> Result<Arc<dyn ExecutionSigner>> {
        Err(BackendError::Config(
            "signer_provider=mock is refused outside test / test-signer builds".to_string(),
        ))
    }
}

#[allow(dead_code)]
fn address_to_account_id(bytes: &[u8; 20]) -> crate::types::AccountId {
    let mut s = String::with_capacity(42);
    s.push_str("0x");
    for b in bytes {
        s.push_str(&format!("{:02x}", b));
    }
    crate::types::AccountId::new(s)
}

// -----------------------------------------------------------------
//                          UNIT TESTS
// -----------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hybrid_v2::execution::signer::SignerAvailability;

    fn base_config() -> HybridV2ExecutionConfig {
        let mut c = HybridV2ExecutionConfig::disabled();
        c.execution_enabled = true;
        c.executor_address = [0xaau8; 20];
        c
    }

    #[test]
    fn production_without_expected_address_errors() {
        let cfg = base_config();
        let err = match HybridV2SignerBuilder::build(&cfg) {
            Ok(_) => panic!("expected IncompleteProductionSignerConfig, got a signer"),
            Err(e) => e.to_string(),
        };
        assert!(err.contains("IncompleteProductionSignerConfig"), "{err}");
    }

    #[test]
    fn production_kms_gcp_returns_unavailable_signer() {
        let mut cfg = base_config();
        cfg.expected_signer_address = Some([0xbbu8; 20]);
        cfg.signer_endpoint = Some("https://signer.example.com".to_string());
        cfg.signer_provider = Some(SignerProvider::KmsGcp);
        let signer = HybridV2SignerBuilder::build(&cfg).unwrap();
        // GCP is not yet integrated — availability reports NotConfigured.
        assert!(matches!(
            signer.availability(),
            SignerAvailability::NotConfigured
        ));
    }

    #[cfg(not(feature = "aws-kms-transport"))]
    #[test]
    fn production_kms_aws_without_feature_returns_unavailable_signer() {
        let mut cfg = base_config();
        cfg.expected_signer_address = Some([0xbbu8; 20]);
        cfg.signer_endpoint = Some("https://signer.example.com".to_string());
        cfg.signer_kms_key_id = Some("arn:aws:kms:us-east-1:x:key/y".to_string());
        cfg.signer_provider = Some(SignerProvider::KmsAws);
        let signer = HybridV2SignerBuilder::build(&cfg).unwrap();
        assert!(matches!(
            signer.availability(),
            SignerAvailability::NotConfigured
        ));
    }

    #[test]
    fn production_mock_returns_configured_signer_under_tests() {
        // Under `cargo test` the `#[cfg(test)]` branch is on so mock
        // is permitted. The bridge reports Configured at construction.
        let mut cfg = base_config();
        let expected = super::parse_test_address();
        cfg.expected_signer_address = Some(expected);
        cfg.signer_endpoint = Some("http://127.0.0.1:9000/sign".to_string());
        cfg.signer_provider = Some(SignerProvider::Mock);
        let signer = HybridV2SignerBuilder::build(&cfg).unwrap();
        assert!(matches!(
            signer.availability(),
            SignerAvailability::Configured
        ));
        assert_eq!(signer.identity().address, expected);
    }

    #[test]
    fn test_ephemeral_backend_builds_under_test_signer_feature() {
        let mut cfg = base_config();
        cfg.signer_kind = SignerBackend::TestEphemeral([0x42u8; 32]);
        let signer = HybridV2SignerBuilder::build(&cfg).unwrap();
        assert!(matches!(
            signer.availability(),
            SignerAvailability::Configured
        ));
        // Signer address is the seed-derived ephemeral address.
        assert_ne!(signer.identity().address, [0u8; 20]);
    }
}

#[cfg(test)]
pub(crate) fn parse_test_address() -> [u8; 20] {
    // Pinned to the well-known perps test key so the mock provider
    // rounds-trips consistently.
    let hex = "2c7536e3605d9c16a7a3d7b1898e529396a65c23";
    let mut out = [0u8; 20];
    for i in 0..20 {
        out[i] = u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).unwrap();
    }
    out
}
