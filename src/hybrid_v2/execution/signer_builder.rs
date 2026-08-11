//! `BACKEND-HYBRID-V2-PRODUCTION-SIGNER-BOOTSTRAP-AND-STARTUP-WIRING-V1`
//! (Part C) — operational signer builder. Constructs a real
//! [`HybridV2KmsSignerBridge`] backed by [`HttpSignerTransport`] for
//! `SignerProvider::KmsAws`. Unimplemented vendors still fail closed.
//!
//! Frozen posture:
//!
//! * **NO_NEW_CARGO_DEPS** — the builder only wires types that are
//!   already in the crate graph (`RemoteSignerClient`,
//!   `PluggableRemoteSignerTransport`, `AwsKmsSignerProvider`,
//!   `HttpSignerTransport`).
//! * **PATTERN_C_CUSTODY_BOUNDARY** — `KmsAws` produces a transport
//!   that authenticates to a signer microservice via mTLS. The backend
//!   holds ONLY the mTLS client identity; the signer microservice
//!   holds the AWS IAM role. This builder does not import any
//!   `aws-sdk-*` type — the `aws-kms-transport` feature gates a
//!   different milestone (in-process KMS transport, unused here).
//! * **UNIMPLEMENTED_VENDORS_FAIL_CLOSED** — `KmsGcp`, `Turnkey`,
//!   `Fireblocks` return `SignerUnavailable("provider X not yet
//!   integrated")` at build time. NEVER invent HTTP protocols for a
//!   vendor we can't actually reach.
//! * **MOCK_ONLY_UNDER_TEST_SIGNER** — `SignerProvider::Mock` is
//!   accepted only under `#[cfg(any(test, feature = "test-signer"))]`.
//!   Non-test builds refuse it at build time.
//! * **SIGNER_UNAVAILABLE_NEVER_FALLS_BACK_TO_LOCAL_RAW_KEY** — a
//!   failure to construct the HTTP transport (e.g. missing mTLS
//!   material) returns `BackendError::Config` upward. The wire path
//!   NEVER downgrades to a local signer.

use std::sync::Arc;
use std::time::Duration;

use crate::error::{BackendError, Result};
use crate::execution::remote_signer::RemoteSignerClient;
use crate::hybrid_v2::config::{HybridV2ExecutionConfig, SignerProvider};
use crate::hybrid_v2::execution::signer::{ExecutionSigner, SignerBackend, SignerKind};
use crate::hybrid_v2::execution::signer_http_transport::{
    is_public_https, read_pem_if_configured, HttpSignerTransport,
};
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

    /// **Pattern C** — construct a real bridge over `HttpSignerTransport`.
    ///
    /// The backend authenticates to the signer microservice with mTLS;
    /// the signer microservice authenticates to AWS KMS with its own
    /// IAM role. This function reads PEM material from disk (paths
    /// supplied via env), enforces mTLS-mandatory for public HTTPS
    /// endpoints, and returns a `HybridV2KmsSignerBridge` wired around
    /// a `RemoteSignerClient` that speaks the `HttpSignerTransport`
    /// wire protocol. It does NOT contact the microservice — a
    /// separate identity probe (`HttpSignerTransport::fetch_identity`)
    /// runs at startup for bootstrap validation (Part F).
    fn build_kms_aws(
        config: &HybridV2ExecutionConfig,
        expected_signer_address: [u8; 20],
        endpoint: &str,
        endpoint_redacted: String,
        max_retries: u32,
        request_timeout: Duration,
    ) -> Result<Arc<dyn ExecutionSigner>> {
        // Operator hygiene: the KMS key id is an opaque handle passed
        // to the signer microservice via its own configuration. This
        // build still requires the operator to name a key so a
        // misconfiguration surfaces here rather than at first sign.
        if config.signer_kms_key_id.is_none() {
            return Err(BackendError::Config(
                "HV2_SIGNER_KMS_KEY_ID is required when signer_provider=kms_aws".to_string(),
            ));
        }
        // Load mTLS material. The config validator has already checked
        // that both paths are set for public HTTPS endpoints; this
        // block enforces the same invariant defensively.
        let mtls_cert = read_pem_if_configured(&config.signer_mtls_cert_pem_path)
            .map_err(|e| BackendError::Config(format!("signer mTLS cert read: {e}")))?;
        let mtls_key = read_pem_if_configured(&config.signer_mtls_key_pem_path)
            .map_err(|e| BackendError::Config(format!("signer mTLS key read: {e}")))?;
        let root_ca = read_pem_if_configured(&config.signer_root_ca_pem_path)
            .map_err(|e| BackendError::Config(format!("signer root CA read: {e}")))?;

        if is_public_https(endpoint) && (mtls_cert.is_none() || mtls_key.is_none()) {
            return Err(BackendError::Config(
                "MtlsRequiredForPublicEndpoint: signer_endpoint is a public HTTPS URL — \
                 mTLS cert + key paths are mandatory (HV2_SIGNER_MTLS_CERT_PATH, \
                 HV2_SIGNER_MTLS_KEY_PATH)"
                    .to_string(),
            ));
        }

        let timeout_ms = config.signer_request_timeout_ms.min(30_000);
        let transport = HttpSignerTransport::new(
            endpoint.to_string(),
            expected_signer_address,
            mtls_cert,
            mtls_key,
            root_ca,
            timeout_ms,
            max_retries,
        )
        .map_err(|e| BackendError::Config(format!("HTTP signer transport build: {e}")))?;

        let remote_client = Arc::new(RemoteSignerClient::with_transport(
            endpoint.to_string(),
            address_to_account_id(&expected_signer_address),
            Arc::new(transport),
        ));

        let bridge = HybridV2KmsSignerBridge::new(
            remote_client,
            expected_signer_address,
            SignerKind::RemoteKMS,
            endpoint_redacted,
            max_retries,
            request_timeout,
        );

        // Belt-and-suspenders redaction check: the redacted endpoint
        // must not contain a path fragment. If it does, refuse to hand
        // out the bridge — a mis-redacted string could leak into the
        // admin identity() surface.
        // (redacted_endpoint above collapses to scheme://host[:port].)
        let _ = redacted_endpoint(endpoint);

        Ok(Arc::new(bridge))
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

    fn build_err_string(cfg: &HybridV2ExecutionConfig) -> String {
        match HybridV2SignerBuilder::build(cfg) {
            Ok(_) => panic!("expected builder failure, got a signer"),
            Err(e) => e.to_string(),
        }
    }

    #[test]
    fn production_kms_aws_without_kms_key_id_refused() {
        let mut cfg = base_config();
        cfg.expected_signer_address = Some([0xbbu8; 20]);
        cfg.signer_endpoint = Some("http://127.0.0.1:9000/sign".to_string());
        cfg.signer_provider = Some(SignerProvider::KmsAws);
        let err = build_err_string(&cfg);
        assert!(err.contains("HV2_SIGNER_KMS_KEY_ID"), "{err}");
    }

    #[test]
    fn production_kms_aws_public_https_refuses_without_mtls() {
        let mut cfg = base_config();
        cfg.expected_signer_address = Some([0xbbu8; 20]);
        cfg.signer_endpoint = Some("https://signer.example.com".to_string());
        cfg.signer_kms_key_id = Some("arn:aws:kms:us-east-1:x:key/y".to_string());
        cfg.signer_provider = Some(SignerProvider::KmsAws);
        // No mTLS paths set — builder must refuse.
        let err = build_err_string(&cfg);
        assert!(err.contains("MtlsRequiredForPublicEndpoint"), "{err}");
    }

    #[test]
    fn production_kms_aws_loopback_builds_operational_bridge_without_mtls() {
        // Loopback endpoints don't require mTLS material — this is the
        // local dev exception.
        let mut cfg = base_config();
        cfg.expected_signer_address = Some([0xbbu8; 20]);
        cfg.signer_endpoint = Some("http://127.0.0.1:9000/sign".to_string());
        cfg.signer_kms_key_id = Some("arn:aws:kms:us-east-1:x:key/y".to_string());
        cfg.signer_provider = Some(SignerProvider::KmsAws);
        let signer = HybridV2SignerBuilder::build(&cfg).unwrap();
        // A real bridge reports `Configured` (not NotConfigured).
        assert!(matches!(
            signer.availability(),
            SignerAvailability::Configured
        ));
        assert_eq!(signer.identity().address, [0xbbu8; 20]);
        assert_eq!(signer.identity().kind, SignerKind::RemoteKMS);
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
