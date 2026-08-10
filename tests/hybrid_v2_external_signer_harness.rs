//! `BACKEND-HYBRID-V2-EXTERNAL-SIGNER-INTEGRATION-AND-LIVE-ORCHESTRATOR-V1`
//! Part N — Shared external-signer test harness.
//!
//! Reuses:
//!   * [`MockVendorSignerProvider`] — Perps side, 10-mode vendor mock
//!     (`Success`, `Denied`, `Timeout`, `Unavailable`, `AuthFailed`,
//!     `RateLimited`, `MalformedSignature`, `AddressMismatch`,
//!     `Unknown`, `HealthFailed`).
//!   * [`HybridV2KmsSignerBridge`] — the bridge under test.
//!
//! Provides:
//!   * [`build_bridge_with_mode`] — one-line helper that returns a
//!     configured bridge with a specific mock mode.
//!   * [`build_scripted_bridge`] — bridge over a caller-controlled
//!     queue of raw `RemoteSigner` results (used to exercise
//!     identity-mismatch, replay, and altered-envelope paths that the
//!     mock adapter cannot express directly).
//!   * [`signed_response_for`] — synthesize a well-formed
//!     `SignerResponse` for a given prehash + returned address so
//!     tests can precisely control what the vendor returns.
//!   * [`baseline_signing_request`] — canonical Sepolia signing
//!     request template with mutable field builders for the altered-
//!     envelope proptests.
//!
//! Every helper is `#[allow(dead_code)]` at the module level because
//! individual integration binaries only pull a subset — a callers-only
//! module is expected to leave some helpers unused per binary.
//!
//! Frozen safety:
//!
//! * No helper here calls a broadcast RPC.
//! * No helper here loads or embeds a mainnet private key.
//! * All helpers accept `expected_signer_address` explicitly — the
//!   bridge cross-check is the authoritative surface.

#![cfg(feature = "test-signer")]
#![allow(dead_code)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use alloy_primitives::U256;
use uuid::Uuid;

use deopt_v2_backend::execution::config::PrivateKeySecret;
use deopt_v2_backend::execution::remote_signer::{
    RemoteSigner, RemoteSignerClient, SignerBackendKind, SignerError as PerpsSignerError,
    SignerFuture, SignerHealth, SignerRequest as PerpsSignerRequest, SignerResponse,
};
use deopt_v2_backend::execution::signer::{ExecutorSigner, RecoverableSignature};
use deopt_v2_backend::execution::signer_adapters::{
    MockProviderMode, MockVendorSignerProvider, PluggableRemoteSignerTransport,
};
use deopt_v2_backend::hybrid_v2::execution::signer::{SignerKind, SigningRequest};
use deopt_v2_backend::hybrid_v2::execution::HybridV2KmsSignerBridge;
use deopt_v2_backend::types::AccountId;

/// Well-known test key. Pinned across the perps signer tests; the
/// recovered address is [`TEST_ADDRESS_HEX`]. NEVER production key
/// material — reused only because its deterministic address makes
/// the bridge cross-check testable.
pub const TEST_KEY: &str = "0x4c0883a69102937d6231471b5dbb6204fe5129617082792ae468d01a3f362318";
pub const TEST_ADDRESS_HEX: &str = "0x2c7536e3605d9c16a7a3d7b1898e529396a65c23";

pub fn parse_address_hex(s: &str) -> [u8; 20] {
    let stripped = s.trim_start_matches("0x").trim_start_matches("0X");
    let mut a = [0u8; 20];
    for i in 0..20 {
        a[i] = u8::from_str_radix(&stripped[2 * i..2 * i + 2], 16).unwrap();
    }
    a
}

pub fn test_address_bytes() -> [u8; 20] {
    parse_address_hex(TEST_ADDRESS_HEX)
}

/// Build a bridge that sits atop the shared perps
/// `PluggableRemoteSignerTransport` + `MockVendorSignerProvider`.
/// Mode selects the failure class:
///
/// | mode | outcome |
/// |------|---------|
/// | `Success` | valid signature recovering to `TEST_ADDRESS_HEX` |
/// | `Denied` | vendor policy refuses; NOT retryable |
/// | `Timeout` | vendor SDK timeout; retryable |
/// | `Unavailable` | vendor 5xx; retryable |
/// | `AuthFailed` | vendor auth material rejected |
/// | `RateLimited` | vendor 429; retryable |
/// | `MalformedSignature` | signer returns invalid `y_parity` |
/// | `AddressMismatch` | signer returns different address |
/// | `Unknown` | unstructured vendor error |
/// | `HealthFailed` | `derive_address` fails; probe unreachable |
pub fn build_bridge_with_mode(
    mode: MockProviderMode,
    expected_hex: &str,
    max_retries: u32,
) -> Arc<HybridV2KmsSignerBridge> {
    let inner =
        ExecutorSigner::from_private_key(&PrivateKeySecret::new(TEST_KEY.to_string())).unwrap();
    let provider = Arc::new(MockVendorSignerProvider::new(mode, inner));
    let plug = Arc::new(PluggableRemoteSignerTransport::new(
        provider,
        AccountId::new(expected_hex),
    ));
    let client: Arc<dyn RemoteSigner> = Arc::new(RemoteSignerClient::with_transport(
        "https://mock-signer.invalid".to_string(),
        AccountId::new(expected_hex),
        plug,
    ));
    Arc::new(HybridV2KmsSignerBridge::new(
        client,
        parse_address_hex(expected_hex),
        SignerKind::RemoteKMS,
        "https://mock-signer.invalid".to_string(),
        max_retries,
        Duration::from_millis(2_500),
    ))
}

/// Scripted `RemoteSigner` — callers push a queue of results.
/// Each `sign_option_execution_tx` call pops the head. When the queue
/// is exhausted, subsequent calls return `Transport("scripted-exhausted")`.
pub struct ScriptedRemoteSigner {
    queue: Mutex<Vec<Result<SignerResponse, PerpsSignerError>>>,
    addr: AccountId,
    calls: Mutex<u32>,
    delay: Mutex<Option<Duration>>,
    health: Mutex<Result<SignerHealth, PerpsSignerError>>,
}

impl ScriptedRemoteSigner {
    pub fn new(addr: AccountId, results: Vec<Result<SignerResponse, PerpsSignerError>>) -> Self {
        let health_addr = addr.clone();
        Self {
            queue: Mutex::new(results),
            addr,
            calls: Mutex::new(0),
            delay: Mutex::new(None),
            health: Mutex::new(Ok(SignerHealth {
                mode: SignerBackendKind::Remote,
                signer_address: Some(health_addr),
                remote_endpoint_present: true,
                healthy: true,
            })),
        }
    }

    pub fn push(&self, result: Result<SignerResponse, PerpsSignerError>) {
        self.queue.lock().unwrap().push(result);
    }

    pub fn calls(&self) -> u32 {
        *self.calls.lock().unwrap()
    }

    pub fn set_delay(&self, d: Duration) {
        *self.delay.lock().unwrap() = Some(d);
    }

    pub fn set_health(&self, h: Result<SignerHealth, PerpsSignerError>) {
        *self.health.lock().unwrap() = h;
    }
}

impl RemoteSigner for ScriptedRemoteSigner {
    fn signer_address(&self) -> &AccountId {
        &self.addr
    }

    fn kind(&self) -> SignerBackendKind {
        SignerBackendKind::Remote
    }

    fn sign_option_execution_tx<'a>(
        &'a self,
        _req: PerpsSignerRequest<'a>,
    ) -> SignerFuture<'a, SignerResponse> {
        *self.calls.lock().unwrap() += 1;
        let delay = *self.delay.lock().unwrap();
        let next = {
            let mut q = self.queue.lock().unwrap();
            if q.is_empty() {
                Err(PerpsSignerError::Transport("scripted-exhausted".into()))
            } else {
                q.remove(0)
            }
        };
        Box::pin(async move {
            if let Some(d) = delay {
                tokio::time::sleep(d).await;
            }
            next
        })
    }

    fn health_check(&self) -> SignerFuture<'_, SignerHealth> {
        let out = self.health.lock().unwrap().clone();
        Box::pin(async move { out })
    }
}

/// Build a bridge over a scripted remote signer. Used for:
///   * identity mismatch (script a response with a different address)
///   * retry-after-transient (Timeout, Timeout, Success)
///   * delayed response (set_delay + short bridge request_timeout)
///   * duplicated request/response (script identical Ok twice)
///   * signer identity change between calls
pub fn build_scripted_bridge(
    expected_hex: &str,
    scripted: Arc<ScriptedRemoteSigner>,
    max_retries: u32,
    request_timeout: Duration,
) -> Arc<HybridV2KmsSignerBridge> {
    let inner: Arc<dyn RemoteSigner> = scripted;
    Arc::new(HybridV2KmsSignerBridge::new(
        inner,
        parse_address_hex(expected_hex),
        SignerKind::RemoteKMS,
        "https://scripted-signer.invalid".to_string(),
        max_retries,
        request_timeout,
    ))
}

/// Synthesize a well-formed `SignerResponse` for the given prehash
/// with a caller-controlled `signer_address` string. Used to exercise
/// identity mismatch paths (vendor returns the right signature but
/// claims a different address).
pub fn signed_response_for(prehash: [u8; 32], returned_address: &str) -> SignerResponse {
    let signer =
        ExecutorSigner::from_private_key(&PrivateKeySecret::new(TEST_KEY.to_string())).unwrap();
    let sig: RecoverableSignature = signer.sign_prehash(&prehash).unwrap();
    SignerResponse {
        request_id: Uuid::from_u128(0),
        signer_address: AccountId::new(returned_address),
        signature: sig,
        kms_request_id: Some("mock".to_string()),
        audit_log_id: Some("mock".to_string()),
        remote_signer_request_id: Some("mock".to_string()),
        created_at_ms: 1_700_000_000_000,
        policy_decision_id: Uuid::from_u128(0),
    }
}

/// Malformed signed response: overrides `y_parity` to a non-{0, 1}
/// value. The bridge rejects this at its own boundary via
/// `SignerError::MalformedResponse`.
pub fn malformed_signed_response(prehash: [u8; 32], returned_address: &str) -> SignerResponse {
    let mut resp = signed_response_for(prehash, returned_address);
    resp.signature.y_parity = 7;
    resp
}

/// Canonical Sepolia signing request template. Fields marked "//
/// TAMPER" are the ones altered-envelope tests mutate to prove the
/// bridge cross-checks catch it.
pub fn baseline_signing_request() -> SigningRequest {
    SigningRequest {
        chain_id: 84532, // TAMPER: 8453 is mainnet — bridge refuses
        nonce: 7,        // TAMPER: nonce is bound into signing payload
        target: [0xccu8; 20],
        value_wei: U256::ZERO,
        calldata_hash: [0xddu8; 32], // TAMPER: must equal keccak(calldata)
        gas_limit: 100_000,
        max_fee_per_gas_wei: U256::from(1u64),
        max_priority_fee_per_gas_wei: U256::from(1u64),
        tx_type: 2,
        plan_hash: [0xeeu8; 32],
        signing_payload_hash: [0xffu8; 32], // TAMPER: signer must derive locally
        calldata: vec![0x01, 0x02],
    }
}

// =================================================================
//                          HARNESS TESTS
// =================================================================
// These tests validate the harness itself so a broken helper never
// lets a downstream integration test pass falsely.

#[cfg(test)]
mod harness_tests {
    use super::*;
    use deopt_v2_backend::hybrid_v2::execution::signer::{
        ExecutionSigner, SignerAvailability, SignerError,
    };

    #[tokio::test]
    async fn build_bridge_with_mode_success_round_trips() {
        let bridge = build_bridge_with_mode(MockProviderMode::Success, TEST_ADDRESS_HEX, 0);
        let req = baseline_signing_request();
        let signed = bridge.sign_execution(req).await.expect("success");
        assert_eq!(signed.recovered_signer, test_address_bytes());
        assert!(signed.signature_v <= 1);
    }

    #[tokio::test]
    async fn build_bridge_with_mode_denied_returns_deterministic_refusal() {
        let bridge = build_bridge_with_mode(MockProviderMode::Denied, TEST_ADDRESS_HEX, 3);
        let err = bridge
            .sign_execution(baseline_signing_request())
            .await
            .unwrap_err();
        // Denied → deterministic refusal — bridge classifies as
        // SignerUnavailable (vendor rejection) at its boundary.
        assert!(
            matches!(err, SignerError::SignerUnavailable(_)),
            "denied maps to unavailable-class: {err:?}"
        );
    }

    #[tokio::test]
    async fn build_bridge_with_mode_timeout_maps_to_signer_timeout() {
        let bridge = build_bridge_with_mode(MockProviderMode::Timeout, TEST_ADDRESS_HEX, 0);
        let err = bridge
            .sign_execution(baseline_signing_request())
            .await
            .unwrap_err();
        assert!(matches!(err, SignerError::Timeout));
    }

    #[tokio::test]
    async fn build_bridge_with_mode_auth_failed_reports_signer_unavailable() {
        let bridge = build_bridge_with_mode(MockProviderMode::AuthFailed, TEST_ADDRESS_HEX, 0);
        let err = bridge
            .sign_execution(baseline_signing_request())
            .await
            .unwrap_err();
        assert!(matches!(err, SignerError::SignerUnavailable(_)));
    }

    #[tokio::test]
    async fn build_bridge_with_mode_malformed_signature_rejected() {
        let bridge =
            build_bridge_with_mode(MockProviderMode::MalformedSignature, TEST_ADDRESS_HEX, 0);
        let err = bridge
            .sign_execution(baseline_signing_request())
            .await
            .unwrap_err();
        // Malformed y_parity is caught by the transport-layer
        // validator, projected to `SignerError::Internal("malformed-…")`
        // in the Perps stack. The bridge classifier maps Internal →
        // MalformedResponse.
        assert!(matches!(err, SignerError::MalformedResponse(_)));
    }

    #[tokio::test]
    async fn build_bridge_with_mode_address_mismatch_rejected() {
        let bridge = build_bridge_with_mode(MockProviderMode::AddressMismatch, TEST_ADDRESS_HEX, 0);
        let err = bridge
            .sign_execution(baseline_signing_request())
            .await
            .unwrap_err();
        // AddressMismatch surfaces at the bridge boundary — either via
        // PostSignFromMismatch (Perps side) → IdentityMismatch at the
        // bridge, OR via SignerUnavailable if the plug transport
        // rejects earlier.
        assert!(
            matches!(
                err,
                SignerError::IdentityMismatch { .. } | SignerError::SignerUnavailable(_)
            ),
            "unexpected err {err:?}"
        );
    }

    #[tokio::test]
    async fn scripted_bridge_replays_identical_response_twice() {
        // Duplicated response — the bridge does not de-dup, it just
        // returns the (valid) result of each call. Idempotency is
        // orchestrated one layer up via the persisted idempotency key.
        let addr = TEST_ADDRESS_HEX;
        let baseline = baseline_signing_request();
        // Deriving the actual signing payload from the SigningRequest
        // requires the orchestrator; here we simply script the same
        // Ok twice and observe the bridge accepts each independently.
        // For a well-formed signature test, use build_bridge_with_mode.
        // For the "duplicated response" property, we script a
        // deterministic Ok and observe the count reaches two on two
        // sign_execution calls.
        let resp1 = signed_response_for(baseline.signing_payload_hash, addr);
        let resp2 = signed_response_for(baseline.signing_payload_hash, addr);
        let scripted = Arc::new(ScriptedRemoteSigner::new(
            AccountId::new(addr),
            vec![Ok(resp1), Ok(resp2)],
        ));
        let bridge = build_scripted_bridge(addr, scripted.clone(), 0, Duration::from_millis(500));
        let a = bridge.sign_execution(baseline.clone()).await.unwrap();
        let b = bridge.sign_execution(baseline).await.unwrap();
        // Because the vendor returned the same (r, s, v), the bridge
        // hands back identical decomposed tuples.
        assert_eq!(a.signature_r, b.signature_r);
        assert_eq!(a.signature_s, b.signature_s);
        assert_eq!(a.signature_v, b.signature_v);
        assert_eq!(scripted.calls(), 2);
    }

    #[tokio::test]
    async fn scripted_bridge_altered_envelope_chain_id_mainnet_short_circuits() {
        // The bridge refuses chain_id == 8453 BEFORE calling the vendor.
        let addr = TEST_ADDRESS_HEX;
        let scripted = Arc::new(ScriptedRemoteSigner::new(AccountId::new(addr), vec![]));
        let bridge = build_scripted_bridge(addr, scripted.clone(), 0, Duration::from_millis(500));
        let mut req = baseline_signing_request();
        req.chain_id = 8453;
        let err = bridge.sign_execution(req).await.unwrap_err();
        assert!(matches!(err, SignerError::ChainMismatch));
        assert_eq!(
            scripted.calls(),
            0,
            "vendor must NOT be called on mainnet chain id"
        );
    }

    #[tokio::test]
    async fn scripted_bridge_delayed_response_completes_within_timeout() {
        // A 50ms delay under a 500ms request_timeout succeeds. Wall-
        // clock fragile — kept short + not asserted to sub-ms.
        let addr = TEST_ADDRESS_HEX;
        let baseline = baseline_signing_request();
        let resp = signed_response_for(baseline.signing_payload_hash, addr);
        let scripted = Arc::new(ScriptedRemoteSigner::new(
            AccountId::new(addr),
            vec![Ok(resp)],
        ));
        scripted.set_delay(Duration::from_millis(50));
        let bridge = build_scripted_bridge(addr, scripted.clone(), 0, Duration::from_millis(500));
        let signed = bridge.sign_execution(baseline).await.expect("ok");
        assert_eq!(signed.recovered_signer, test_address_bytes());
    }

    #[tokio::test]
    async fn health_failed_probe_reports_unavailable() {
        // Only the Perps `derive_address` fails — the plug transport
        // classifies as `KmsUnavailable`. The bridge probe therefore
        // reports a TransientFailure/Unavailable-class verdict.
        let bridge = build_bridge_with_mode(MockProviderMode::HealthFailed, TEST_ADDRESS_HEX, 0);
        let verdict = bridge.probe().await;
        assert!(
            matches!(
                verdict,
                SignerAvailability::TransientFailure { .. }
                    | SignerAvailability::Unavailable { .. }
            ),
            "unexpected probe verdict {verdict:?}"
        );
    }
}
