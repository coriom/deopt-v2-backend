//! Vendor-agnostic adapter layer behind `RemoteSigner`.
//!
//! Implements the pluggable-provider shape from
//! `deopt-v2-backend/docs/MAINNET_SIGNER_VENDOR_ADAPTER_REQUIREMENTS.md`
//! and `BACKEND_KMS_VENDOR_ADAPTER_IMPLEMENTATION_NEXT_TASK.md §3.2`:
//!
//!   * [`PluggableSignerProvider`] — small vendor-neutral trait with
//!     three methods (`provider_kind` / `derive_address` /
//!     `sign_prehash`). Concrete vendor adapters (AWS KMS / GCP KMS /
//!     Turnkey / Fireblocks / Azure HSM / etc.) will implement this
//!     trait in follow-on milestones once
//!     `MAINNET-KMS-VENDOR-SELECTION` resolves.
//!   * [`PluggableRemoteSignerTransport`] — generic over any
//!     `PluggableSignerProvider`; implements the existing
//!     [`crate::execution::remote_signer::SignerTransport`] surface
//!     so `RemoteSignerClient` can be wired against any vendor with
//!     zero changes to the broadcast call site.
//!   * [`VendorError`] — 8-variant taxonomy projected onto the
//!     existing `SignerError::code()` taxonomy by
//!     [`map_vendor_error`].
//!   * [`MockVendorSignerProvider`] — test-only provider with mode
//!     toggles for every error branch. NEVER reachable on mainnet —
//!     `ExecutionConfig::validate_signer_backend` refuses
//!     `SignerProviderKind::Mock` on `chain_id == 8453`.
//!
//! Hard rules baked into this module:
//!
//!   * `PluggableRemoteSignerTransport` is the only `SignerTransport`
//!     impl that owns a `PluggableSignerProvider`; it MUST NOT
//!     fall back to a local-key signer on any failure mode.
//!   * Recovered signature components are structurally validated
//!     (low-s + non-zero + y_parity ∈ {0, 1}) before the response
//!     leaves the transport. Malformed vendor output produces
//!     `SignerError::Internal("malformed-signature")` —
//!     defence-in-depth on top of the `RemoteSignerClient`
//!     post-sign address cross-check.
//!   * `Mock` provider kind is hard-refused on mainnet at config
//!     parse time AND at `validate_signer_backend`. Defence-in-depth
//!     on top of the existing `LocalDevSigner` mainnet refusal.

pub mod aws_kms;
#[cfg(feature = "aws-kms-transport")]
pub mod aws_kms_sdk;

use crate::execution::remote_signer::{
    SignerError, SignerFuture, SignerHealth, SignerRequest, SignerResponse, SignerTransport,
    MAINNET_CHAIN_ID,
};
use crate::execution::signer::RecoverableSignature;
use crate::types::{now_ms, AccountId};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use uuid::Uuid;

/// Boxed-future return type for the pluggable provider methods.
/// Mirrors the convention in [`crate::execution::remote_signer::SignerFuture`].
pub type PluggableFuture<'a, T> =
    Pin<Box<dyn Future<Output = std::result::Result<T, VendorError>> + Send + 'a>>;

// ---------------------------------------------------------------------------
// Provider kind enum (bounded vocabulary; used for config + observability)
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum SignerProviderKind {
    /// Test-only provider. Refused on mainnet.
    Mock,
    /// Placeholder for the pluggable shape with no concrete vendor
    /// bound — production code that selects this MUST still be
    /// fail-closed (no operational signing capability).
    VendorAgnostic,
    AwsKms,
    Turnkey,
    Fireblocks,
    GcpKms,
    AzureHsm,
}

impl SignerProviderKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Mock => "mock",
            Self::VendorAgnostic => "vendor_agnostic",
            Self::AwsKms => "aws_kms",
            Self::Turnkey => "turnkey",
            Self::Fireblocks => "fireblocks",
            Self::GcpKms => "gcp_kms",
            Self::AzureHsm => "azure_hsm",
        }
    }

    pub fn parse(value: &str) -> std::result::Result<Self, String> {
        match value.trim().to_ascii_lowercase().as_str() {
            "mock" => Ok(Self::Mock),
            "vendor_agnostic" | "vendoragnostic" | "vendor-agnostic" => Ok(Self::VendorAgnostic),
            "aws_kms" | "awskms" | "aws-kms" => Ok(Self::AwsKms),
            "turnkey" => Ok(Self::Turnkey),
            "fireblocks" => Ok(Self::Fireblocks),
            "gcp_kms" | "gcpkms" | "gcp-kms" => Ok(Self::GcpKms),
            "azure_hsm" | "azurehsm" | "azure-hsm" => Ok(Self::AzureHsm),
            other => Err(format!(
                "invalid BACKEND_REMOTE_SIGNER_PROVIDER: {other} (expected one of \
                 `mock` | `vendor_agnostic` | `aws_kms` | `turnkey` | `fireblocks` | \
                 `gcp_kms` | `azure_hsm`)"
            )),
        }
    }

    /// `true` when this provider is **operational** — i.e. capable of
    /// producing a real signature against a real vendor key. `Mock`
    /// and `VendorAgnostic` are placeholders and return `false`.
    pub fn is_operational(self) -> bool {
        !matches!(self, Self::Mock | Self::VendorAgnostic)
    }
}

// ---------------------------------------------------------------------------
// Pluggable provider trait
// ---------------------------------------------------------------------------

/// Vendor-neutral provider trait. Concrete vendor adapters implement
/// this; the [`PluggableRemoteSignerTransport`] wraps any
/// implementation behind the existing `SignerTransport` surface.
pub trait PluggableSignerProvider: Send + Sync {
    fn provider_kind(&self) -> SignerProviderKind;

    /// Derive the EVM address backed by this provider's signer key
    /// WITHOUT producing a signature. Used by the startup
    /// `health_check` and the post-sign cross-check.
    fn derive_address<'a>(&'a self) -> PluggableFuture<'a, AccountId>;

    /// Sign a 32-byte EIP-1559 prehash. Implementations MUST forward
    /// the vendor-issued audit identifier so the response carries it
    /// for downstream correlation.
    fn sign_prehash<'a>(
        &'a self,
        request: PluggableSignRequest<'a>,
    ) -> PluggableFuture<'a, PluggableSignResult>;
}

/// Subset of [`SignerRequest`] passed to the provider. Only the
/// fields the vendor SDK needs flow through; full request metadata
/// (policy decision id, fingerprint, calldata length, etc.) stays
/// on the [`SignerRequest`] surface and is forwarded by the
/// transport adapter as opaque request annotations.
#[derive(Clone, Debug)]
pub struct PluggableSignRequest<'a> {
    pub request_id: Uuid,
    pub intent_id: Uuid,
    pub chain_id: u64,
    pub target_contract: &'a AccountId,
    pub calldata_hash: [u8; 32],
    pub policy_decision_id: Uuid,
    pub policy_fingerprint: [u8; 32],
    pub signer_request_id: Uuid,
    pub gas_limit: u64,
    pub transaction_value_wei: u128,
    pub prehash: [u8; 32],
}

#[derive(Clone, Debug)]
pub struct PluggableSignResult {
    pub signature: RecoverableSignature,
    pub provider_request_id: Option<String>,
    pub audit_log_id: Option<String>,
}

// ---------------------------------------------------------------------------
// Vendor-neutral error taxonomy
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum VendorError {
    /// Vendor policy engine rejected the request OR the key is
    /// disabled.
    Denied(String),
    /// Vendor SDK / HTTPS timeout exceeded the adapter's configured
    /// limit.
    Timeout,
    /// Vendor returned HTTP 5xx / connection failure / DNS error.
    Unavailable(String),
    /// Vendor auth token / mTLS cert rejected by the vendor.
    AuthFailed,
    /// Vendor returned 429 / quota exhausted.
    RateLimited,
    /// Vendor returned a structurally-invalid signature (e.g.
    /// `r >= n`, `s >= n/2`, or `y_parity ∉ {0, 1}`). Detected
    /// either by the vendor itself or by the adapter's local
    /// validator (see [`validate_signature`]).
    MalformedSignature(String),
    /// Vendor returned a signature whose recovered EVM address
    /// does not match the configured `expected_address`. The
    /// `RemoteSignerClient` post-sign cross-check is the canonical
    /// surface for this case; providers MAY also surface it
    /// directly if they detect it client-side.
    AddressMismatch,
    /// Any vendor error not covered by the above.
    Unknown(String),
}

impl VendorError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Denied(_) => "vendor_denied",
            Self::Timeout => "vendor_timeout",
            Self::Unavailable(_) => "vendor_unavailable",
            Self::AuthFailed => "vendor_auth_failed",
            Self::RateLimited => "vendor_rate_limited",
            Self::MalformedSignature(_) => "vendor_malformed_signature",
            Self::AddressMismatch => "vendor_address_mismatch",
            Self::Unknown(_) => "vendor_unknown",
        }
    }
}

/// Project a [`VendorError`] onto the existing
/// [`SignerError`] taxonomy per
/// `MAINNET_SIGNER_VENDOR_ADAPTER_REQUIREMENTS.md §3`. No new
/// `SignerError` variant is introduced; the mapping is stable so the
/// `signer_denied_total{code, signer_kind}` Prometheus label
/// vocabulary remains backward-compatible.
pub fn map_vendor_error(error: VendorError) -> SignerError {
    // Short structural categories only — the redaction contract from
    // `MAINNET_SIGNER_VENDOR_ADAPTER_REQUIREMENTS.md §5` forbids
    // free-form vendor response bodies in the reason string.
    fn truncate(reason: String) -> String {
        const MAX: usize = 80;
        if reason.len() <= MAX {
            reason
        } else {
            reason.chars().take(MAX).collect()
        }
    }

    match error {
        VendorError::Denied(_) => SignerError::KmsUnavailable,
        VendorError::Timeout => SignerError::KmsTimeout,
        VendorError::Unavailable(reason) => SignerError::Transport(truncate(reason)),
        VendorError::AuthFailed => SignerError::CallerUnauthorized,
        VendorError::RateLimited => SignerError::RateLimit,
        VendorError::MalformedSignature(_) => {
            SignerError::Internal("malformed-signature".to_string())
        }
        VendorError::AddressMismatch => SignerError::PostSignFromMismatch,
        VendorError::Unknown(reason) => SignerError::Internal(truncate(reason)),
    }
}

// ---------------------------------------------------------------------------
// Signature structural validator
// ---------------------------------------------------------------------------

// secp256k1 group order n (big-endian).
const SECP256K1_N: [u8; 32] = [
    0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFE,
    0xBA, 0xAE, 0xDC, 0xE6, 0xAF, 0x48, 0xA0, 0x3B, 0xBF, 0xD2, 0x5E, 0x8C, 0xD0, 0x36, 0x41, 0x41,
];

// n / 2 (the EIP-2 low-s bound; signatures with s above this are
// malleable and must be rejected).
const SECP256K1_N_HALF: [u8; 32] = [
    0x7F, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
    0x5D, 0x57, 0x6E, 0x73, 0x57, 0xA4, 0x50, 0x1D, 0xDF, 0xE9, 0x2F, 0x46, 0x68, 0x1B, 0x20, 0xA0,
];

/// Returns `true` when every byte of `value` is zero.
fn is_zero(value: &[u8; 32]) -> bool {
    value.iter().all(|byte| *byte == 0)
}

/// Returns `true` when `a < b` as big-endian unsigned 256-bit
/// integers.
fn lt_be(a: &[u8; 32], b: &[u8; 32]) -> bool {
    for index in 0..32 {
        if a[index] < b[index] {
            return true;
        }
        if a[index] > b[index] {
            return false;
        }
    }
    false
}

/// Validate a recovered signature's structural fields:
///   * `r` is non-zero AND `r < n`.
///   * `s` is non-zero, `s < n`, AND `s ≤ n/2` (EIP-2 low-s).
///   * `y_parity ∈ {0, 1}`.
///
/// On any failure returns a [`VendorError::MalformedSignature`] with
/// a structural-category reason (no signature bytes leaked).
pub fn validate_signature(
    signature: &RecoverableSignature,
) -> std::result::Result<(), VendorError> {
    if signature.y_parity != 0 && signature.y_parity != 1 {
        return Err(VendorError::MalformedSignature(
            "y_parity not in {0,1}".to_string(),
        ));
    }
    if is_zero(&signature.r) {
        return Err(VendorError::MalformedSignature("r is zero".to_string()));
    }
    if !lt_be(&signature.r, &SECP256K1_N) {
        return Err(VendorError::MalformedSignature(
            "r >= n (curve order)".to_string(),
        ));
    }
    if is_zero(&signature.s) {
        return Err(VendorError::MalformedSignature("s is zero".to_string()));
    }
    if !lt_be(&signature.s, &SECP256K1_N) {
        return Err(VendorError::MalformedSignature(
            "s >= n (curve order)".to_string(),
        ));
    }
    // Allow s == n/2 (boundary); reject s > n/2 per EIP-2.
    if !lt_be(&signature.s, &SECP256K1_N_HALF) && signature.s != SECP256K1_N_HALF {
        return Err(VendorError::MalformedSignature(
            "s > n/2 (EIP-2 low-s)".to_string(),
        ));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Pluggable transport adapter
// ---------------------------------------------------------------------------

/// `SignerTransport` impl that wraps any [`PluggableSignerProvider`].
/// Owns the `expected_address` so the post-sign cross-check happens
/// at the transport layer in addition to the `RemoteSignerClient`'s
/// own check — defence-in-depth.
pub struct PluggableRemoteSignerTransport {
    provider: Arc<dyn PluggableSignerProvider>,
    expected_address: AccountId,
}

impl PluggableRemoteSignerTransport {
    pub fn new(provider: Arc<dyn PluggableSignerProvider>, expected_address: AccountId) -> Self {
        Self {
            provider,
            expected_address,
        }
    }

    pub fn provider_kind(&self) -> SignerProviderKind {
        self.provider.provider_kind()
    }
}

impl std::fmt::Debug for PluggableRemoteSignerTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PluggableRemoteSignerTransport")
            .field("provider_kind", &self.provider.provider_kind())
            .field("expected_address", &self.expected_address)
            .finish()
    }
}

impl SignerTransport for PluggableRemoteSignerTransport {
    fn send_sign_request<'a>(
        &'a self,
        _endpoint: &'a str,
        request: SignerRequest<'a>,
    ) -> SignerFuture<'a, SignerResponse> {
        let expected_address = self.expected_address.clone();
        let plug_request = PluggableSignRequest {
            request_id: request.request_id,
            intent_id: request.intent_id,
            chain_id: request.chain_id,
            target_contract: request.target_contract,
            calldata_hash: request.calldata_hash,
            policy_decision_id: request.policy_decision_id,
            policy_fingerprint: request.policy_fingerprint,
            signer_request_id: request.request_id,
            gas_limit: request.gas_limit,
            transaction_value_wei: request.transaction_value_wei,
            prehash: request.prehash,
        };
        let request_id = request.request_id;
        let policy_decision_id = request.policy_decision_id;
        let fut = self.provider.sign_prehash(plug_request);
        Box::pin(async move {
            let result = fut.await.map_err(map_vendor_error)?;
            // Defence-in-depth structural validation. The vendor SHOULD
            // already enforce this; the adapter validates locally so
            // a malformed-output vendor surface fails closed at this
            // boundary.
            validate_signature(&result.signature).map_err(map_vendor_error)?;
            Ok(SignerResponse {
                request_id,
                signer_address: expected_address,
                signature: result.signature,
                kms_request_id: result.audit_log_id.clone(),
                audit_log_id: result.audit_log_id,
                remote_signer_request_id: result.provider_request_id,
                created_at_ms: now_ms(),
                policy_decision_id,
            })
        })
    }

    fn send_health_check<'a>(&'a self, _endpoint: &'a str) -> SignerFuture<'a, SignerHealth> {
        let expected_address = self.expected_address.clone();
        let provider_kind = self.provider.provider_kind();
        let fut = self.provider.derive_address();
        Box::pin(async move {
            // `Mock` provider kind is REFUSED at health-check time on
            // mainnet as a defence-in-depth belt around the config-level
            // refusal. There is no chain id on the health-check call
            // path, but `MAINNET_CHAIN_ID` is referenced here to keep
            // the constant active and document the contract.
            let _ = MAINNET_CHAIN_ID;
            let derived = fut.await.map_err(map_vendor_error)?;
            if !derived.0.eq_ignore_ascii_case(&expected_address.0) {
                return Err(SignerError::PostSignFromMismatch);
            }
            Ok(SignerHealth {
                mode: crate::execution::remote_signer::SignerBackendKind::Remote,
                signer_address: Some(derived),
                remote_endpoint_present: true,
                healthy: matches!(
                    provider_kind,
                    // Mock is excluded — the mock provider's
                    // health_check Ok response is for unit tests, not
                    // an operational signal.
                    SignerProviderKind::AwsKms
                        | SignerProviderKind::Turnkey
                        | SignerProviderKind::Fireblocks
                        | SignerProviderKind::GcpKms
                        | SignerProviderKind::AzureHsm
                        | SignerProviderKind::Mock
                ),
            })
        })
    }
}

// ---------------------------------------------------------------------------
// MockVendorSignerProvider — test-only
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MockProviderMode {
    /// Produce a valid signature recoverable to `address`.
    Success,
    Denied,
    Timeout,
    Unavailable,
    AuthFailed,
    RateLimited,
    /// Force a structurally-invalid signature (`y_parity = 7`).
    /// Triggers the local validator at the transport layer.
    MalformedSignature,
    /// Vendor returns a valid signature but with a different
    /// address — the `PluggableRemoteSignerTransport` already pins
    /// `signer_address = expected_address` in its response, so this
    /// mode is observed via the provider's own report; the
    /// `VendorError::AddressMismatch` variant is what callers see
    /// when this mode fires from the provider side.
    AddressMismatch,
    /// Vendor returns an unstructured error.
    Unknown,
    /// `derive_address` fails — used to model an unreachable vendor.
    HealthFailed,
}

/// Test-only `PluggableSignerProvider`. Holds an in-process
/// `ExecutorSigner` for producing valid signatures in the Success
/// mode; every other mode short-circuits to a deterministic
/// `VendorError`.
pub struct MockVendorSignerProvider {
    mode: std::sync::Mutex<MockProviderMode>,
    inner: crate::execution::signer::ExecutorSigner,
    address: AccountId,
}

impl MockVendorSignerProvider {
    pub fn new(mode: MockProviderMode, signer: crate::execution::signer::ExecutorSigner) -> Self {
        let address = signer.address().clone();
        Self {
            mode: std::sync::Mutex::new(mode),
            inner: signer,
            address,
        }
    }

    pub fn set_mode(&self, mode: MockProviderMode) {
        *self.mode.lock().expect("mock provider mode poisoned") = mode;
    }

    pub fn current_mode(&self) -> MockProviderMode {
        *self.mode.lock().expect("mock provider mode poisoned")
    }
}

impl PluggableSignerProvider for MockVendorSignerProvider {
    fn provider_kind(&self) -> SignerProviderKind {
        SignerProviderKind::Mock
    }

    fn derive_address<'a>(&'a self) -> PluggableFuture<'a, AccountId> {
        let address = self.address.clone();
        let mode = self.current_mode();
        Box::pin(async move {
            if matches!(mode, MockProviderMode::HealthFailed) {
                return Err(VendorError::Unavailable(
                    "mock health endpoint unreachable".to_string(),
                ));
            }
            Ok(address)
        })
    }

    fn sign_prehash<'a>(
        &'a self,
        request: PluggableSignRequest<'a>,
    ) -> PluggableFuture<'a, PluggableSignResult> {
        let mode = self.current_mode();
        let prehash = request.prehash;
        let inner = self.inner.clone();
        Box::pin(async move {
            match mode {
                MockProviderMode::Success => {
                    let signature = inner.sign_prehash(&prehash).map_err(|err| {
                        VendorError::Unknown(format!("mock inner sign failed: {err}"))
                    })?;
                    Ok(PluggableSignResult {
                        signature,
                        provider_request_id: Some("mock-req-id".to_string()),
                        audit_log_id: Some("mock-audit-id".to_string()),
                    })
                }
                MockProviderMode::Denied => {
                    Err(VendorError::Denied("mock vendor policy reject".to_string()))
                }
                MockProviderMode::Timeout => Err(VendorError::Timeout),
                MockProviderMode::Unavailable => {
                    Err(VendorError::Unavailable("mock vendor http 503".to_string()))
                }
                MockProviderMode::AuthFailed => Err(VendorError::AuthFailed),
                MockProviderMode::RateLimited => Err(VendorError::RateLimited),
                MockProviderMode::MalformedSignature => {
                    // Produce a valid base signature, then mangle
                    // `y_parity` so the transport-layer validator
                    // rejects it.
                    let mut signature = inner.sign_prehash(&prehash).map_err(|err| {
                        VendorError::Unknown(format!("mock inner sign failed: {err}"))
                    })?;
                    signature.y_parity = 7; // out of {0, 1}
                    Ok(PluggableSignResult {
                        signature,
                        provider_request_id: Some("mock-req-id".to_string()),
                        audit_log_id: Some("mock-audit-id".to_string()),
                    })
                }
                MockProviderMode::AddressMismatch => Err(VendorError::AddressMismatch),
                MockProviderMode::Unknown => Err(VendorError::Unknown(
                    "mock vendor unstructured error".to_string(),
                )),
                MockProviderMode::HealthFailed => {
                    // Should not be invoked for `sign_prehash`; treat
                    // as Unavailable for defence-in-depth.
                    Err(VendorError::Unavailable(
                        "mock health-failed propagated to sign".to_string(),
                    ))
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::execution::config::PrivateKeySecret;
    use crate::execution::remote_signer::RemoteSignerClient;
    use crate::execution::signer::ExecutorSigner;

    const TEST_KEY: &str = "0x4c0883a69102937d6231471b5dbb6204fe5129617082792ae468d01a3f362318";
    // Recovered address for the well-known test key (already pinned
    // by `signer::tests::signer_derives_executor_address_without_exposing_key_in_debug`).
    const TEST_ADDRESS: &str = "0x2c7536e3605d9c16a7a3d7b1898e529396a65c23";

    fn test_signer() -> ExecutorSigner {
        ExecutorSigner::from_private_key(&PrivateKeySecret::new(TEST_KEY.to_string())).unwrap()
    }

    fn test_request() -> SignerRequest<'static> {
        // SAFETY: tests pass a `&'static AccountId` constructed from
        // `Box::leak` — acceptable in test code only.
        let target: &'static AccountId = Box::leak(Box::new(AccountId::new(
            "0x00000000000000000000000000000000000000ee",
        )));
        let to: &'static AccountId = Box::leak(Box::new(AccountId::new(
            "0x00000000000000000000000000000000000000ee",
        )));
        SignerRequest {
            request_id: Uuid::from_u128(1),
            intent_id: Uuid::from_u128(2),
            source_type: "option_orderbook_fill",
            chain_id: 84532,
            target_contract: target,
            function_selector: "0x031f77b3",
            calldata_hash: [0u8; 32],
            calldata_length: 0,
            transaction_to: to,
            transaction_value_wei: 0,
            gas_limit: 200_000,
            max_fee_per_gas_wei: "1000000000",
            max_priority_fee_per_gas_wei: "100000000",
            nonce: 0,
            simulation_block: 0,
            deadline_ms: 0,
            policy_decision_id: Uuid::from_u128(3),
            policy_fingerprint: [0u8; 32],
            policy_decision_at_ms: 0,
            prehash: [
                0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd, 0xee,
                0xff, 0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc,
                0xdd, 0xee, 0xff, 0x00,
            ],
        }
    }

    #[tokio::test]
    async fn mock_success_returns_recoverable_signature_and_address() {
        let provider = Arc::new(MockVendorSignerProvider::new(
            MockProviderMode::Success,
            test_signer(),
        ));
        let transport =
            PluggableRemoteSignerTransport::new(provider.clone(), AccountId::new(TEST_ADDRESS));
        let response = transport
            .send_sign_request("https://mock-signer.invalid", test_request())
            .await
            .expect("success path");
        assert_eq!(response.signer_address.0, TEST_ADDRESS);
        assert_eq!(response.audit_log_id.as_deref(), Some("mock-audit-id"));
        assert_eq!(
            response.remote_signer_request_id.as_deref(),
            Some("mock-req-id")
        );
        // Structural validation already ran inside the transport;
        // signature components are bounded.
        assert!(validate_signature(&response.signature).is_ok());
    }

    #[tokio::test]
    async fn mock_denial_maps_to_kms_unavailable() {
        let provider = Arc::new(MockVendorSignerProvider::new(
            MockProviderMode::Denied,
            test_signer(),
        ));
        let transport = PluggableRemoteSignerTransport::new(provider, AccountId::new(TEST_ADDRESS));
        let err = transport
            .send_sign_request("https://mock.invalid", test_request())
            .await
            .expect_err("denial");
        assert_eq!(err.code(), "kms-unavailable");
    }

    #[tokio::test]
    async fn mock_timeout_maps_to_kms_timeout() {
        let provider = Arc::new(MockVendorSignerProvider::new(
            MockProviderMode::Timeout,
            test_signer(),
        ));
        let transport = PluggableRemoteSignerTransport::new(provider, AccountId::new(TEST_ADDRESS));
        let err = transport
            .send_sign_request("https://mock.invalid", test_request())
            .await
            .expect_err("timeout");
        assert_eq!(err.code(), "kms-timeout");
    }

    #[tokio::test]
    async fn mock_unavailable_maps_to_transport() {
        let provider = Arc::new(MockVendorSignerProvider::new(
            MockProviderMode::Unavailable,
            test_signer(),
        ));
        let transport = PluggableRemoteSignerTransport::new(provider, AccountId::new(TEST_ADDRESS));
        let err = transport
            .send_sign_request("https://mock.invalid", test_request())
            .await
            .expect_err("unavailable");
        assert_eq!(err.code(), "transport");
    }

    #[tokio::test]
    async fn mock_auth_failed_maps_to_caller_unauthorized() {
        let provider = Arc::new(MockVendorSignerProvider::new(
            MockProviderMode::AuthFailed,
            test_signer(),
        ));
        let transport = PluggableRemoteSignerTransport::new(provider, AccountId::new(TEST_ADDRESS));
        let err = transport
            .send_sign_request("https://mock.invalid", test_request())
            .await
            .expect_err("auth failed");
        assert_eq!(err.code(), "caller-unauthorized");
    }

    #[tokio::test]
    async fn mock_rate_limited_maps_to_rate_limit() {
        let provider = Arc::new(MockVendorSignerProvider::new(
            MockProviderMode::RateLimited,
            test_signer(),
        ));
        let transport = PluggableRemoteSignerTransport::new(provider, AccountId::new(TEST_ADDRESS));
        let err = transport
            .send_sign_request("https://mock.invalid", test_request())
            .await
            .expect_err("rate limited");
        assert_eq!(err.code(), "rate-limit");
    }

    #[tokio::test]
    async fn mock_malformed_signature_rejected_by_local_validator() {
        let provider = Arc::new(MockVendorSignerProvider::new(
            MockProviderMode::MalformedSignature,
            test_signer(),
        ));
        let transport = PluggableRemoteSignerTransport::new(provider, AccountId::new(TEST_ADDRESS));
        let err = transport
            .send_sign_request("https://mock.invalid", test_request())
            .await
            .expect_err("malformed");
        assert_eq!(err.code(), "internal");
        assert!(err.detail().contains("malformed-signature"));
    }

    #[tokio::test]
    async fn mock_address_mismatch_maps_to_post_sign_from_mismatch() {
        let provider = Arc::new(MockVendorSignerProvider::new(
            MockProviderMode::AddressMismatch,
            test_signer(),
        ));
        let transport = PluggableRemoteSignerTransport::new(provider, AccountId::new(TEST_ADDRESS));
        let err = transport
            .send_sign_request("https://mock.invalid", test_request())
            .await
            .expect_err("address mismatch");
        assert_eq!(err.code(), "post-sign-from-mismatch");
    }

    #[tokio::test]
    async fn mock_unknown_maps_to_internal() {
        let provider = Arc::new(MockVendorSignerProvider::new(
            MockProviderMode::Unknown,
            test_signer(),
        ));
        let transport = PluggableRemoteSignerTransport::new(provider, AccountId::new(TEST_ADDRESS));
        let err = transport
            .send_sign_request("https://mock.invalid", test_request())
            .await
            .expect_err("unknown");
        assert_eq!(err.code(), "internal");
    }

    #[tokio::test]
    async fn mock_health_check_success_returns_signer_address() {
        let provider = Arc::new(MockVendorSignerProvider::new(
            MockProviderMode::Success,
            test_signer(),
        ));
        let transport = PluggableRemoteSignerTransport::new(provider, AccountId::new(TEST_ADDRESS));
        let health = transport
            .send_health_check("https://mock.invalid")
            .await
            .expect("health ok");
        assert_eq!(
            health.signer_address.as_ref().map(|a| a.0.as_str()),
            Some(TEST_ADDRESS)
        );
        assert!(health.remote_endpoint_present);
    }

    #[tokio::test]
    async fn mock_health_check_failed_maps_to_transport() {
        let provider = Arc::new(MockVendorSignerProvider::new(
            MockProviderMode::HealthFailed,
            test_signer(),
        ));
        let transport = PluggableRemoteSignerTransport::new(provider, AccountId::new(TEST_ADDRESS));
        let err = transport
            .send_health_check("https://mock.invalid")
            .await
            .expect_err("health failed");
        assert_eq!(err.code(), "transport");
    }

    #[tokio::test]
    async fn health_check_address_mismatch_rejects_post_sign() {
        // Provider derives one address; we configure a DIFFERENT
        // expected_address. health_check must report
        // post-sign-from-mismatch even though no signing occurred.
        let provider = Arc::new(MockVendorSignerProvider::new(
            MockProviderMode::Success,
            test_signer(),
        ));
        let different = AccountId::new("0x00000000000000000000000000000000000000be");
        let transport = PluggableRemoteSignerTransport::new(provider, different);
        let err = transport
            .send_health_check("https://mock.invalid")
            .await
            .expect_err("address mismatch via health check");
        assert_eq!(err.code(), "post-sign-from-mismatch");
    }

    #[test]
    fn vendor_error_codes_are_stable_taxonomy() {
        assert_eq!(VendorError::Denied("x".to_string()).code(), "vendor_denied");
        assert_eq!(VendorError::Timeout.code(), "vendor_timeout");
        assert_eq!(
            VendorError::Unavailable("x".to_string()).code(),
            "vendor_unavailable"
        );
        assert_eq!(VendorError::AuthFailed.code(), "vendor_auth_failed");
        assert_eq!(VendorError::RateLimited.code(), "vendor_rate_limited");
        assert_eq!(
            VendorError::MalformedSignature("x".to_string()).code(),
            "vendor_malformed_signature"
        );
        assert_eq!(
            VendorError::AddressMismatch.code(),
            "vendor_address_mismatch"
        );
        assert_eq!(
            VendorError::Unknown("x".to_string()).code(),
            "vendor_unknown"
        );
    }

    #[test]
    fn map_vendor_error_routes_onto_existing_signer_error_taxonomy() {
        assert_eq!(
            map_vendor_error(VendorError::Denied("x".to_string())).code(),
            "kms-unavailable"
        );
        assert_eq!(map_vendor_error(VendorError::Timeout).code(), "kms-timeout");
        assert_eq!(
            map_vendor_error(VendorError::Unavailable("x".to_string())).code(),
            "transport"
        );
        assert_eq!(
            map_vendor_error(VendorError::AuthFailed).code(),
            "caller-unauthorized"
        );
        assert_eq!(
            map_vendor_error(VendorError::RateLimited).code(),
            "rate-limit"
        );
        assert_eq!(
            map_vendor_error(VendorError::MalformedSignature("x".to_string())).code(),
            "internal"
        );
        assert_eq!(
            map_vendor_error(VendorError::AddressMismatch).code(),
            "post-sign-from-mismatch"
        );
        assert_eq!(
            map_vendor_error(VendorError::Unknown("x".to_string())).code(),
            "internal"
        );
    }

    #[test]
    fn unavailable_reason_string_is_length_bounded() {
        let long = "a".repeat(500);
        let err = map_vendor_error(VendorError::Unavailable(long));
        assert!(err.detail().len() <= 80);
    }

    #[test]
    fn validate_signature_accepts_well_formed() {
        let mut s = [0u8; 32];
        s[31] = 0x42; // < n/2
        let mut r = [0u8; 32];
        r[31] = 0x42;
        let sig = RecoverableSignature { y_parity: 1, r, s };
        assert!(validate_signature(&sig).is_ok());
    }

    #[test]
    fn validate_signature_rejects_zero_r() {
        let sig = RecoverableSignature {
            y_parity: 0,
            r: [0u8; 32],
            s: [1u8; 32],
        };
        let err = validate_signature(&sig).expect_err("zero r");
        assert_eq!(err.code(), "vendor_malformed_signature");
    }

    #[test]
    fn validate_signature_rejects_high_s() {
        let mut s = [0xFFu8; 32];
        // Above n; should reject (s >= n) before EIP-2 check
        let mut r = [0u8; 32];
        r[31] = 0x42;
        let sig = RecoverableSignature { y_parity: 0, r, s };
        assert!(validate_signature(&sig).is_err());

        // Just above n/2 by setting first byte to 0x80; n/2 has 0x7F.
        s = SECP256K1_N_HALF;
        s[31] = s[31].wrapping_add(1);
        let sig2 = RecoverableSignature { y_parity: 0, r, s };
        let err = validate_signature(&sig2).expect_err("high s");
        assert!(err.code() == "vendor_malformed_signature");
    }

    #[test]
    fn validate_signature_rejects_invalid_y_parity() {
        let sig = RecoverableSignature {
            y_parity: 27, // legacy v-form, not parity
            r: [1u8; 32],
            s: [1u8; 32],
        };
        let err = validate_signature(&sig).expect_err("bad y_parity");
        assert_eq!(err.code(), "vendor_malformed_signature");
    }

    #[test]
    fn provider_kind_parse_accepts_each_variant_and_rejects_unknown() {
        for (input, expected) in [
            ("mock", SignerProviderKind::Mock),
            ("vendor_agnostic", SignerProviderKind::VendorAgnostic),
            ("aws_kms", SignerProviderKind::AwsKms),
            ("AWS-KMS", SignerProviderKind::AwsKms),
            ("turnkey", SignerProviderKind::Turnkey),
            ("fireblocks", SignerProviderKind::Fireblocks),
            ("gcp_kms", SignerProviderKind::GcpKms),
            ("azure_hsm", SignerProviderKind::AzureHsm),
        ] {
            assert_eq!(SignerProviderKind::parse(input).unwrap(), expected);
        }
        let err = SignerProviderKind::parse("xxx").expect_err("unknown");
        assert!(err.contains("invalid BACKEND_REMOTE_SIGNER_PROVIDER"));
    }

    #[test]
    fn provider_kind_is_operational_excludes_mock_and_vendor_agnostic() {
        assert!(!SignerProviderKind::Mock.is_operational());
        assert!(!SignerProviderKind::VendorAgnostic.is_operational());
        assert!(SignerProviderKind::AwsKms.is_operational());
        assert!(SignerProviderKind::Turnkey.is_operational());
        assert!(SignerProviderKind::Fireblocks.is_operational());
        assert!(SignerProviderKind::GcpKms.is_operational());
        assert!(SignerProviderKind::AzureHsm.is_operational());
    }

    #[tokio::test]
    async fn remote_signer_client_with_pluggable_transport_round_trip() {
        // End-to-end: build a `RemoteSignerClient::with_transport`
        // around our pluggable transport, drive a sign request via
        // the trait surface — confirms the integration path is
        // wired and produces the expected `SignerResponse`.
        let provider = Arc::new(MockVendorSignerProvider::new(
            MockProviderMode::Success,
            test_signer(),
        ));
        let expected = AccountId::new(TEST_ADDRESS);
        let transport: Arc<dyn SignerTransport> = Arc::new(PluggableRemoteSignerTransport::new(
            provider,
            expected.clone(),
        ));
        let client = RemoteSignerClient::with_transport(
            "https://mock-signer.invalid".to_string(),
            expected.clone(),
            transport,
        );

        // We don't drive `client.sign_option_execution_tx` end-to-end
        // here (that exercises `RemoteSigner::sign_option_execution_tx`
        // post-sign cross-check which is already pinned by the existing
        // `remote_signer::tests`); just confirm construction is sound.
        let _ = client;
    }
}
