//! Backend executor signer abstraction.
//!
//! Implements the `RemoteSigner` trait from
//! `deopt-v2-backend/docs/MAINNET_BE_SIGNER_SERVICE_DESIGN.md §6.1`.
//!
//! Two concrete implementations land in this milestone:
//!   * [`LocalDevSigner`] — wraps the existing [`ExecutorSigner`] for Sepolia
//!     rehearsal + unit tests; runtime-refused on mainnet chain id `8453`.
//!   * [`RemoteSignerClient`] — mTLS HTTPS/2 client of the signer
//!     microservice. The production wire transport is left as
//!     [`UnimplementedTransport`] (returns a structured `Transport` error);
//!     tests inject a `SignerTransport` mock.
//!
//! Hard rules baked into this module:
//!   * No raw private key passes through any tracked log path.
//!   * Mainnet chain id (`8453`) hard-refuses `LocalDevSigner` even if a
//!     startup-guard regression accidentally seated one — defence-in-depth.
//!   * `RemoteSignerClient` cross-checks the post-sign address against the
//!     configured expected EOA; mismatch → `PostSignFromMismatch`.

use crate::execution::signer::{ExecutorSigner, RecoverableSignature};
use crate::signing::eip712::keccak256;
use crate::types::{now_ms, AccountId};
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use uuid::Uuid;

/// Base mainnet chain id. Hard-coded — this module is the chain-side
/// backstop, not the user of an env-supplied chain id.
pub const MAINNET_CHAIN_ID: u64 = 8453;
pub const BASE_SEPOLIA_CHAIN_ID: u64 = 84532;
pub const ANVIL_CHAIN_ID: u64 = 31337;

/// Boxed-future return type matching the existing `RpcFuture` convention in
/// `src/execution/rpc.rs`. Keeps the trait object-safe without bringing in
/// `async_trait`.
pub type SignerFuture<'a, T> =
    Pin<Box<dyn Future<Output = std::result::Result<T, SignerError>> + Send + 'a>>;

// ---------------------------------------------------------------------------
// Signer backend mode
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SignerBackendKind {
    /// In-process k256 ECDSA signer (`ExecutorSigner::from_private_key`).
    /// Permitted only on testnet / anvil; refused on mainnet at startup.
    LocalDev,
    /// HTTPS/mTLS client of an external signer microservice backed by a
    /// KMS / HSM / MPC provider.
    Remote,
}

impl SignerBackendKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::LocalDev => "local_dev",
            Self::Remote => "remote",
        }
    }

    pub fn parse(value: &str) -> std::result::Result<Self, String> {
        match value {
            "local_dev" | "localdev" | "local-dev" => Ok(Self::LocalDev),
            "remote" => Ok(Self::Remote),
            other => Err(format!(
                "invalid BACKEND_SIGNER_MODE: {other} (expected `local_dev` or `remote`)"
            )),
        }
    }
}

impl fmt::Display for SignerBackendKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

// ---------------------------------------------------------------------------
// Request / response types (per design doc §3)
// ---------------------------------------------------------------------------

/// `should_broadcast` Approve metadata that the backend hands to the signer
/// service. Fields mirror the design doc §3.1 schema; `caller_identity` is
/// NOT carried in the request body (signer reads it from the mTLS layer).
#[derive(Clone, Debug)]
pub struct SignerRequest<'a> {
    pub request_id: Uuid,
    pub intent_id: Uuid,
    pub source_type: &'a str,
    pub chain_id: u64,
    pub target_contract: &'a AccountId,
    pub function_selector: &'a str,
    pub calldata_hash: [u8; 32],
    pub calldata_length: usize,
    pub transaction_to: &'a AccountId,
    pub transaction_value_wei: u128,
    pub gas_limit: u64,
    pub max_fee_per_gas_wei: &'a str,
    pub max_priority_fee_per_gas_wei: &'a str,
    pub nonce: u64,
    pub simulation_block: u64,
    pub deadline_ms: i64,
    pub policy_decision_id: Uuid,
    pub policy_fingerprint: [u8; 32],
    pub policy_decision_at_ms: i64,
    /// keccak256( 0x02 ‖ eip1559_payload ) — caller-computed.
    pub prehash: [u8; 32],
}

#[derive(Clone, Debug)]
pub struct SignerResponse {
    pub request_id: Uuid,
    pub signer_address: AccountId,
    pub signature: RecoverableSignature,
    /// Provider-issued audit correlation id (KMS request id when via remote);
    /// `None` for LocalDevSigner.
    pub kms_request_id: Option<String>,
    pub audit_log_id: Option<String>,
    pub remote_signer_request_id: Option<String>,
    pub created_at_ms: i64,
    pub policy_decision_id: Uuid,
}

/// Stable reject taxonomy lifted from
/// `MAINNET_BE_SIGNER_SERVICE_DESIGN.md §4.2`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SignerError {
    ChainNotAllowed,
    TargetNotAllowed,
    SelectorNotAllowed,
    ValueNotZero,
    GasCap,
    CalldataBindMismatch,
    NonceMismatch,
    DeadlineExpired,
    PolicyFingerprint,
    PolicyStale,
    DuplicateRequestId,
    DuplicateIntentSigned,
    CallerUnauthorized,
    RateLimit,
    PostSignFromMismatch,
    KmsUnavailable,
    KmsTimeout,
    Transport(String),
    Internal(String),
    /// Configuration prevents using the signer (e.g. LocalDevSigner asked to
    /// sign on mainnet, or RemoteSignerClient missing endpoint).
    ConfigRefusal(String),
}

impl SignerError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::ChainNotAllowed => "chain-not-allowed",
            Self::TargetNotAllowed => "target-not-allowed",
            Self::SelectorNotAllowed => "selector-not-allowed",
            Self::ValueNotZero => "value-not-zero",
            Self::GasCap => "gas-cap",
            Self::CalldataBindMismatch => "calldata-bind-mismatch",
            Self::NonceMismatch => "nonce-mismatch",
            Self::DeadlineExpired => "deadline-expired",
            Self::PolicyFingerprint => "policy-fingerprint",
            Self::PolicyStale => "policy-stale",
            Self::DuplicateRequestId => "duplicate-request-id",
            Self::DuplicateIntentSigned => "duplicate-intent-signed",
            Self::CallerUnauthorized => "caller-unauthorized",
            Self::RateLimit => "rate-limit",
            Self::PostSignFromMismatch => "post-sign-from-mismatch",
            Self::KmsUnavailable => "kms-unavailable",
            Self::KmsTimeout => "kms-timeout",
            Self::Transport(_) => "transport",
            Self::Internal(_) => "internal",
            Self::ConfigRefusal(_) => "config-refusal",
        }
    }

    /// Non-sensitive textual detail. Variants that carry a `String` ferry a
    /// short caller-supplied reason; secrets are never produced by this
    /// module, but callers must continue to enforce redaction at the
    /// boundary (see design doc §4.4).
    pub fn detail(&self) -> String {
        match self {
            Self::Transport(msg) | Self::Internal(msg) | Self::ConfigRefusal(msg) => msg.clone(),
            _ => String::new(),
        }
    }
}

impl fmt::Display for SignerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let detail = self.detail();
        if detail.is_empty() {
            write!(f, "signer:{}", self.code())
        } else {
            write!(f, "signer:{}:{}", self.code(), detail)
        }
    }
}

impl std::error::Error for SignerError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SignerHealth {
    pub mode: SignerBackendKind,
    pub signer_address: Option<AccountId>,
    pub remote_endpoint_present: bool,
    pub healthy: bool,
}

// ---------------------------------------------------------------------------
// RemoteSigner trait
// ---------------------------------------------------------------------------

/// Backend-side signer abstraction. All concrete impls must NEVER expose the
/// raw key, log secrets, or fall back to a different signer on failure.
pub trait RemoteSigner: Send + Sync {
    /// Stable EOA address the signer signs with. Used by the broadcast call
    /// site to construct the EIP-1559 `from` and by callers to cross-check
    /// the post-sign signature recovery.
    fn signer_address(&self) -> &AccountId;

    /// Discriminator for logs / metrics.
    fn kind(&self) -> SignerBackendKind;

    /// Sign an option execution transaction prehash. Implementations MUST
    /// uphold the design doc §4 allowlist (when applicable) and §5
    /// policy-fingerprint bind before invoking the underlying signer.
    fn sign_option_execution_tx<'a>(
        &'a self,
        request: SignerRequest<'a>,
    ) -> SignerFuture<'a, SignerResponse>;

    /// Liveness + identity check; no signature emitted.
    fn health_check(&self) -> SignerFuture<'_, SignerHealth>;
}

// ---------------------------------------------------------------------------
// LocalDevSigner
// ---------------------------------------------------------------------------

/// Wraps the existing in-process `ExecutorSigner`. Permitted only on
/// testnet / anvil; mainnet is rejected at runtime as a defence-in-depth
/// belt for the startup guard.
pub struct LocalDevSigner {
    inner: ExecutorSigner,
}

impl LocalDevSigner {
    pub fn from_executor_signer(inner: ExecutorSigner) -> Self {
        Self { inner }
    }
}

impl RemoteSigner for LocalDevSigner {
    fn signer_address(&self) -> &AccountId {
        self.inner.address()
    }

    fn kind(&self) -> SignerBackendKind {
        SignerBackendKind::LocalDev
    }

    fn sign_option_execution_tx<'a>(
        &'a self,
        request: SignerRequest<'a>,
    ) -> SignerFuture<'a, SignerResponse> {
        let SignerRequest {
            request_id,
            policy_decision_id,
            chain_id,
            prehash,
            ..
        } = request;
        let address = self.inner.address().clone();
        Box::pin(async move {
            // Defence-in-depth: LocalDevSigner refuses mainnet signing even
            // if a startup guard regression seated one here. The canonical
            // refusal still lives in `ExecutionConfig::validate_startup`.
            if chain_id == MAINNET_CHAIN_ID {
                return Err(SignerError::ConfigRefusal(
                    "local-dev-signer refused on mainnet chain id".to_string(),
                ));
            }
            let signature = self
                .inner
                .sign_prehash(&prehash)
                .map_err(|err| SignerError::Internal(format!("local sign failed: {err}")))?;
            Ok(SignerResponse {
                request_id,
                signer_address: address,
                signature,
                kms_request_id: None,
                audit_log_id: None,
                remote_signer_request_id: None,
                created_at_ms: now_ms(),
                policy_decision_id,
            })
        })
    }

    fn health_check(&self) -> SignerFuture<'_, SignerHealth> {
        let address = self.inner.address().clone();
        Box::pin(async move {
            Ok(SignerHealth {
                mode: SignerBackendKind::LocalDev,
                signer_address: Some(address),
                remote_endpoint_present: false,
                healthy: true,
            })
        })
    }
}

impl fmt::Debug for LocalDevSigner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LocalDevSigner")
            .field("signer_address", self.inner.address())
            .field("private_key", &"<redacted>")
            .finish()
    }
}

// ---------------------------------------------------------------------------
// RemoteSignerClient + transport
// ---------------------------------------------------------------------------

/// Transport surface for [`RemoteSignerClient`]. Production wires a real
/// HTTPS/mTLS client (left as `UnimplementedTransport` in this milestone);
/// tests inject mocks via [`RemoteSignerClient::with_transport`].
pub trait SignerTransport: Send + Sync {
    fn send_sign_request<'a>(
        &'a self,
        endpoint: &'a str,
        request: SignerRequest<'a>,
    ) -> SignerFuture<'a, SignerResponse>;

    fn send_health_check<'a>(&'a self, endpoint: &'a str) -> SignerFuture<'a, SignerHealth>;
}

pub struct RemoteSignerClient {
    endpoint: String,
    expected_address: AccountId,
    transport: Arc<dyn SignerTransport>,
}

impl RemoteSignerClient {
    /// Production constructor. The transport defaults to
    /// [`UnimplementedTransport`] which returns a structured `Transport`
    /// error — the wire transport will be wired in a follow-on track once a
    /// KMS vendor is selected (Q-CD-5).
    pub fn new(endpoint: String, expected_address: AccountId) -> Self {
        Self {
            endpoint,
            expected_address,
            transport: Arc::new(UnimplementedTransport),
        }
    }

    /// Test/integration constructor — inject a [`SignerTransport`] mock.
    pub fn with_transport(
        endpoint: String,
        expected_address: AccountId,
        transport: Arc<dyn SignerTransport>,
    ) -> Self {
        Self {
            endpoint,
            expected_address,
            transport,
        }
    }
}

impl RemoteSigner for RemoteSignerClient {
    fn signer_address(&self) -> &AccountId {
        &self.expected_address
    }

    fn kind(&self) -> SignerBackendKind {
        SignerBackendKind::Remote
    }

    fn sign_option_execution_tx<'a>(
        &'a self,
        request: SignerRequest<'a>,
    ) -> SignerFuture<'a, SignerResponse> {
        let policy_decision_id = request.policy_decision_id;
        let expected_address = self.expected_address.clone();
        let fut = self
            .transport
            .send_sign_request(self.endpoint.as_str(), request);
        Box::pin(async move {
            let response = fut.await?;
            // §4.A15 post-sign identity cross-check.
            if !addresses_equal(&response.signer_address, &expected_address) {
                return Err(SignerError::PostSignFromMismatch);
            }
            // §5.1 policy decision id round-trip integrity.
            if response.policy_decision_id != policy_decision_id {
                return Err(SignerError::Internal(
                    "signer echoed mismatched policy_decision_id".to_string(),
                ));
            }
            Ok(response)
        })
    }

    fn health_check(&self) -> SignerFuture<'_, SignerHealth> {
        let endpoint = self.endpoint.clone();
        let fut = self.transport.send_health_check(self.endpoint.as_str());
        Box::pin(async move {
            let mut health = fut.await?;
            health.mode = SignerBackendKind::Remote;
            health.remote_endpoint_present = !endpoint.is_empty();
            Ok(health)
        })
    }
}

impl fmt::Debug for RemoteSignerClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RemoteSignerClient")
            .field("endpoint", &"<redacted-url>")
            .field("expected_address", &self.expected_address)
            .field("transport", &"<dyn SignerTransport>")
            .finish()
    }
}

/// Production HTTPS/mTLS transport placeholder. Returns a structured error
/// so any accidental mainnet invocation FAILS-CLOSED rather than silently
/// signing with an unsafe path. The real wire transport lands in a
/// follow-on track tied to KMS vendor selection (Q-CD-5).
struct UnimplementedTransport;

impl SignerTransport for UnimplementedTransport {
    fn send_sign_request<'a>(
        &'a self,
        _endpoint: &'a str,
        _request: SignerRequest<'a>,
    ) -> SignerFuture<'a, SignerResponse> {
        Box::pin(async move {
            Err(SignerError::Transport(
                "production HTTPS/mTLS transport not yet wired; see \
                 BACKEND_SIGNER_INTERFACE_KMS_HSM_ADAPTER_NEXT_TASK"
                    .to_string(),
            ))
        })
    }

    fn send_health_check<'a>(&'a self, _endpoint: &'a str) -> SignerFuture<'a, SignerHealth> {
        Box::pin(async move {
            Err(SignerError::Transport(
                "production HTTPS/mTLS transport not yet wired".to_string(),
            ))
        })
    }
}

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

/// Compute the `should_broadcast` ↔ signer policy fingerprint per design
/// doc §5.1:
///
/// `keccak256( policy_decision_id ‖ calldata_hash ‖ nonce ‖ simulation_block ‖ deadline_ms )`
///
/// All integers are encoded big-endian; the UUID is encoded as its raw 16
/// bytes. This binds a signed transaction to the exact policy decision,
/// the exact calldata, a freshness anchor (simulation block), and a
/// deadline; tampering with any of these inputs by the signer-microservice
/// caller produces a different fingerprint and the signer rejects with
/// `policy-fingerprint`.
pub fn policy_fingerprint(
    decision_id: Uuid,
    calldata_hash: &[u8; 32],
    nonce: u64,
    simulation_block: u64,
    deadline_ms: i64,
) -> [u8; 32] {
    let mut input = Vec::with_capacity(16 + 32 + 8 + 8 + 8);
    input.extend_from_slice(decision_id.as_bytes());
    input.extend_from_slice(calldata_hash);
    input.extend_from_slice(&nonce.to_be_bytes());
    input.extend_from_slice(&simulation_block.to_be_bytes());
    input.extend_from_slice(&deadline_ms.to_be_bytes());
    keccak256(&input)
}

fn addresses_equal(a: &AccountId, b: &AccountId) -> bool {
    a.0.eq_ignore_ascii_case(&b.0)
}

/// `Result<T>` from this module is the project-wide `BackendError` variant;
/// individual signer outcomes use `SignerError` directly. Helper to lift a
/// `SignerError` into the broader project `Result` for ergonomic mapping at
/// the call site.
pub fn signer_error_into_backend(err: SignerError) -> crate::error::BackendError {
    crate::error::BackendError::BroadcastRejected(format!("{err}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::execution::config::PrivateKeySecret;
    use std::sync::Mutex;

    const TEST_KEY: &str = "0x4c0883a69102937d6231471b5dbb6204fe5129617082792ae468d01a3f362318";
    const EXPECTED_TEST_ADDRESS: &str = "0x2c7536e3605d9c16a7a3d7b1898e529396a65c23";

    fn build_local_signer() -> LocalDevSigner {
        let inner = ExecutorSigner::from_private_key(&PrivateKeySecret::new(TEST_KEY.to_string()))
            .expect("test key must parse");
        LocalDevSigner::from_executor_signer(inner)
    }

    fn sample_request(chain_id: u64) -> SignerRequest<'static> {
        SignerRequest {
            request_id: Uuid::nil(),
            intent_id: Uuid::nil(),
            source_type: "option_orderbook_fill",
            chain_id,
            target_contract: leak_account("0x000000000000000000000000000000000000beef"),
            function_selector: "0xa1b2c3d4",
            calldata_hash: [0x11; 32],
            calldata_length: 200,
            transaction_to: leak_account("0x000000000000000000000000000000000000beef"),
            transaction_value_wei: 0,
            gas_limit: 500_000,
            max_fee_per_gas_wei: "1000000000",
            max_priority_fee_per_gas_wei: "100000000",
            nonce: 7,
            simulation_block: 1000,
            deadline_ms: 9_999_999_999_000,
            policy_decision_id: Uuid::from_u128(42),
            policy_fingerprint: [0x22; 32],
            policy_decision_at_ms: 1_700_000_000_000,
            prehash: [0xab; 32],
        }
    }

    fn leak_account(s: &str) -> &'static AccountId {
        // Test-only: build a 'static AccountId by leaking. Used for the
        // SignerRequest borrow lifetime in unit tests.
        Box::leak(Box::new(AccountId::new(s)))
    }

    #[tokio::test]
    async fn local_dev_signer_round_trips_under_sepolia_chain_id() {
        let signer = build_local_signer();
        assert_eq!(signer.signer_address().0, EXPECTED_TEST_ADDRESS);
        let response = signer
            .sign_option_execution_tx(sample_request(BASE_SEPOLIA_CHAIN_ID))
            .await
            .expect("sepolia sign must succeed");
        assert_eq!(response.signer_address.0, EXPECTED_TEST_ADDRESS);
        assert_eq!(response.kms_request_id, None);
        assert_eq!(response.policy_decision_id, Uuid::from_u128(42));
    }

    #[tokio::test]
    async fn local_dev_signer_round_trips_under_anvil_chain_id() {
        let signer = build_local_signer();
        let response = signer
            .sign_option_execution_tx(sample_request(ANVIL_CHAIN_ID))
            .await
            .expect("anvil sign must succeed");
        assert_eq!(response.signer_address.0, EXPECTED_TEST_ADDRESS);
    }

    #[tokio::test]
    async fn local_dev_signer_refuses_mainnet_at_runtime() {
        let signer = build_local_signer();
        let err = signer
            .sign_option_execution_tx(sample_request(MAINNET_CHAIN_ID))
            .await
            .expect_err("mainnet sign must be refused at runtime by local-dev signer");
        assert_eq!(err.code(), "config-refusal");
        assert!(format!("{err}").contains("mainnet"));
    }

    // ----- mock transport for RemoteSignerClient tests -----

    struct MockTransport {
        canned: Mutex<Option<std::result::Result<SignerResponse, SignerError>>>,
        canned_health: Mutex<Option<std::result::Result<SignerHealth, SignerError>>>,
        sign_calls: Mutex<u64>,
    }

    impl MockTransport {
        fn approve(response: SignerResponse) -> Self {
            Self {
                canned: Mutex::new(Some(Ok(response))),
                canned_health: Mutex::new(None),
                sign_calls: Mutex::new(0),
            }
        }
        fn deny(err: SignerError) -> Self {
            Self {
                canned: Mutex::new(Some(Err(err))),
                canned_health: Mutex::new(None),
                sign_calls: Mutex::new(0),
            }
        }
        fn sign_calls_count(&self) -> u64 {
            *self.sign_calls.lock().unwrap()
        }
        fn with_health(self, result: std::result::Result<SignerHealth, SignerError>) -> Self {
            *self.canned_health.lock().unwrap() = Some(result);
            self
        }
    }

    impl SignerTransport for MockTransport {
        fn send_sign_request<'a>(
            &'a self,
            _endpoint: &'a str,
            _request: SignerRequest<'a>,
        ) -> SignerFuture<'a, SignerResponse> {
            *self.sign_calls.lock().unwrap() += 1;
            let canned = self
                .canned
                .lock()
                .unwrap()
                .take()
                .unwrap_or(Err(SignerError::Internal("mock not primed".to_string())));
            Box::pin(async move { canned })
        }

        fn send_health_check<'a>(&'a self, _endpoint: &'a str) -> SignerFuture<'a, SignerHealth> {
            let canned = self
                .canned_health
                .lock()
                .unwrap()
                .take()
                .unwrap_or(Err(SignerError::Internal("mock not primed".to_string())));
            Box::pin(async move { canned })
        }
    }

    fn canned_signed_response() -> SignerResponse {
        let signer = build_local_signer();
        // Reuse the local signer to produce a valid recoverable signature
        // bound to the same EOA — emulates the post-sign cross-check
        // success path.
        let signature = signer
            .inner
            .sign_prehash(&[0xcd; 32])
            .expect("test signature");
        SignerResponse {
            request_id: Uuid::nil(),
            signer_address: AccountId::new(EXPECTED_TEST_ADDRESS),
            signature,
            kms_request_id: Some("kms-req-123".to_string()),
            audit_log_id: Some("audit-456".to_string()),
            remote_signer_request_id: Some("rsig-789".to_string()),
            created_at_ms: 1_700_000_000_000,
            policy_decision_id: Uuid::from_u128(42),
        }
    }

    #[tokio::test]
    async fn remote_signer_client_round_trips_with_mock_approval() {
        let transport = Arc::new(MockTransport::approve(canned_signed_response()));
        let client = RemoteSignerClient::with_transport(
            "https://signer.invalid".to_string(),
            AccountId::new(EXPECTED_TEST_ADDRESS),
            transport.clone(),
        );
        let response = client
            .sign_option_execution_tx(sample_request(MAINNET_CHAIN_ID))
            .await
            .expect("mock signer must approve");
        assert_eq!(response.signer_address.0, EXPECTED_TEST_ADDRESS);
        assert_eq!(response.kms_request_id.as_deref(), Some("kms-req-123"));
        assert_eq!(transport.sign_calls_count(), 1);
    }

    #[tokio::test]
    async fn remote_signer_client_propagates_mock_denial() {
        let transport = Arc::new(MockTransport::deny(SignerError::PolicyFingerprint));
        let client = RemoteSignerClient::with_transport(
            "https://signer.invalid".to_string(),
            AccountId::new(EXPECTED_TEST_ADDRESS),
            transport,
        );
        let err = client
            .sign_option_execution_tx(sample_request(MAINNET_CHAIN_ID))
            .await
            .expect_err("denial must surface");
        assert_eq!(err.code(), "policy-fingerprint");
    }

    #[tokio::test]
    async fn remote_signer_client_detects_post_sign_address_mismatch() {
        let mut response = canned_signed_response();
        response.signer_address = AccountId::new("0xdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef");
        let transport = Arc::new(MockTransport::approve(response));
        let client = RemoteSignerClient::with_transport(
            "https://signer.invalid".to_string(),
            AccountId::new(EXPECTED_TEST_ADDRESS),
            transport,
        );
        let err = client
            .sign_option_execution_tx(sample_request(MAINNET_CHAIN_ID))
            .await
            .expect_err("address mismatch must reject");
        assert_eq!(err.code(), "post-sign-from-mismatch");
    }

    #[tokio::test]
    async fn remote_signer_client_detects_policy_decision_id_mismatch() {
        let mut response = canned_signed_response();
        response.policy_decision_id = Uuid::from_u128(999);
        let transport = Arc::new(MockTransport::approve(response));
        let client = RemoteSignerClient::with_transport(
            "https://signer.invalid".to_string(),
            AccountId::new(EXPECTED_TEST_ADDRESS),
            transport,
        );
        let err = client
            .sign_option_execution_tx(sample_request(MAINNET_CHAIN_ID))
            .await
            .expect_err("mismatched decision id must reject");
        assert_eq!(err.code(), "internal");
    }

    #[tokio::test]
    async fn production_remote_signer_returns_transport_error() {
        // The default `RemoteSignerClient::new` uses `UnimplementedTransport`,
        // which fails-closed until the real wire transport is wired.
        let client = RemoteSignerClient::new(
            "https://signer.invalid".to_string(),
            AccountId::new(EXPECTED_TEST_ADDRESS),
        );
        let err = client
            .sign_option_execution_tx(sample_request(MAINNET_CHAIN_ID))
            .await
            .expect_err("production stub must fail closed");
        assert_eq!(err.code(), "transport");
        assert!(format!("{err}").contains("not yet wired"));
    }

    #[tokio::test]
    async fn remote_signer_health_check_round_trips() {
        let transport = Arc::new(
            MockTransport::approve(canned_signed_response()).with_health(Ok(SignerHealth {
                mode: SignerBackendKind::LocalDev, // overwritten by client
                signer_address: Some(AccountId::new(EXPECTED_TEST_ADDRESS)),
                remote_endpoint_present: false,
                healthy: true,
            })),
        );
        let client = RemoteSignerClient::with_transport(
            "https://signer.invalid".to_string(),
            AccountId::new(EXPECTED_TEST_ADDRESS),
            transport,
        );
        let report = client
            .health_check()
            .await
            .expect("mock health-check must succeed");
        assert_eq!(report.mode, SignerBackendKind::Remote);
        assert!(report.remote_endpoint_present);
        assert!(report.healthy);
    }

    #[tokio::test]
    async fn local_dev_signer_health_check_reports_localdev() {
        let signer = build_local_signer();
        let report = signer
            .health_check()
            .await
            .expect("health check must succeed");
        assert_eq!(report.mode, SignerBackendKind::LocalDev);
        assert!(!report.remote_endpoint_present);
    }

    // ----- redaction / display tests -----

    #[test]
    fn signer_error_display_contains_no_secrets() {
        let err = SignerError::Transport("anything".to_string());
        let display = format!("{err}");
        assert!(display.starts_with("signer:transport:"));
        assert!(!display.contains(TEST_KEY));
    }

    #[test]
    fn local_dev_signer_debug_redacts_private_key() {
        let signer = build_local_signer();
        let debug = format!("{signer:?}");
        assert!(debug.contains("<redacted>"));
        assert!(!debug.contains("4c0883a69102937d6231471b5dbb6204fe5129617082792ae468d01a3f362318"));
    }

    #[test]
    fn remote_signer_client_debug_redacts_endpoint() {
        let client = RemoteSignerClient::new(
            "https://signer-secret.invalid/sign".to_string(),
            AccountId::new(EXPECTED_TEST_ADDRESS),
        );
        let debug = format!("{client:?}");
        assert!(debug.contains("<redacted-url>"));
        assert!(!debug.contains("signer-secret.invalid"));
    }

    #[test]
    fn signer_backend_kind_parse_round_trips() {
        assert_eq!(
            SignerBackendKind::parse("local_dev").unwrap(),
            SignerBackendKind::LocalDev
        );
        assert_eq!(
            SignerBackendKind::parse("localdev").unwrap(),
            SignerBackendKind::LocalDev
        );
        assert_eq!(
            SignerBackendKind::parse("remote").unwrap(),
            SignerBackendKind::Remote
        );
        assert!(SignerBackendKind::parse("anything").is_err());
    }

    // ----- policy_fingerprint vector test -----

    #[test]
    fn policy_fingerprint_is_deterministic_and_input_sensitive() {
        let decision_id = Uuid::from_u128(0x0123_4567_89ab_cdef_0123_4567_89ab_cdef);
        let calldata_hash = [0x33; 32];
        let baseline = policy_fingerprint(
            decision_id,
            &calldata_hash,
            42,
            1_000_000,
            1_700_000_000_000,
        );
        // determinism
        assert_eq!(
            baseline,
            policy_fingerprint(
                decision_id,
                &calldata_hash,
                42,
                1_000_000,
                1_700_000_000_000
            )
        );
        // sensitivity: any input change yields a different fingerprint
        assert_ne!(
            baseline,
            policy_fingerprint(
                Uuid::from_u128(1),
                &calldata_hash,
                42,
                1_000_000,
                1_700_000_000_000
            )
        );
        let mut alt = calldata_hash;
        alt[0] ^= 0xff;
        assert_ne!(
            baseline,
            policy_fingerprint(decision_id, &alt, 42, 1_000_000, 1_700_000_000_000)
        );
        assert_ne!(
            baseline,
            policy_fingerprint(
                decision_id,
                &calldata_hash,
                43,
                1_000_000,
                1_700_000_000_000
            )
        );
        assert_ne!(
            baseline,
            policy_fingerprint(
                decision_id,
                &calldata_hash,
                42,
                1_000_001,
                1_700_000_000_000
            )
        );
        assert_ne!(
            baseline,
            policy_fingerprint(
                decision_id,
                &calldata_hash,
                42,
                1_000_000,
                1_700_000_000_001
            )
        );
    }

    #[test]
    fn signer_error_helper_wraps_into_backend_broadcast_rejected() {
        let backend_err = signer_error_into_backend(SignerError::PolicyFingerprint);
        let message = backend_err.to_string();
        assert!(message.contains("signer:policy-fingerprint"));
    }
}
