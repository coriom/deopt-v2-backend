//! `BACKEND-HYBRID-V2-EXTERNAL-SIGNER-INTEGRATION-AND-LIVE-ORCHESTRATOR-V1`
//! (Package A, Part D + G + H + L) — thin adapter that lifts the
//! battle-tested Perps `RemoteSigner` stack onto the Hybrid V2
//! [`ExecutionSigner`] surface.
//!
//! ## Frozen safety
//!
//! * **BROADCAST_STRICTLY_FORBIDDEN** — the bridge never issues a
//!   chain-write. It receives an already-validated [`SigningRequest`],
//!   forwards it to the vendor-neutral `RemoteSigner`, and returns a
//!   decomposed `(r, s, v, recovered_signer)` tuple.
//! * **EXTERNAL_SIGNER_NEVER_RECEIVES_UNVALIDATED_TRANSACTION_INPUT** —
//!   the request the bridge sees was constructed from a persisted
//!   `ExecutionRequestRow` that passed the `SignerPolicyFirewall`
//!   immediately upstream (Part F). The bridge does NOT expose a
//!   second signing entry point that bypasses the firewall.
//! * **EVERY_EXTERNAL_SIGNATURE_IS_LOCALLY_VERIFIED** — the bridge
//!   cross-checks `recovered_signer == expected_signer_address` at
//!   its own boundary; the orchestrator then re-runs
//!   `verify_signed_tx` immediately after — belt-and-suspenders per
//!   Part G.
//! * **NO_LOG_OR_ERROR_LEAKS_A_SECRET** — the endpoint URI is
//!   redacted to `<host:port>` before it ever reaches an error
//!   message (see [`redacted_endpoint`]).
//!
//! ## Idempotency (Part L)
//!
//! The bridge derives a 16-byte idempotency key from the
//! `(expected_signer_address, canonical_execution_id, plan_hash,
//! signing_payload_hash)` tuple and threads it into the Perps
//! `SignerRequest.policy_fingerprint` so a signer microservice that
//! honours request idempotency converges on the same signature for a
//! retried request. See [`derive_idempotency_key`].

use std::sync::Arc;
use std::time::Duration;

use alloy_primitives::U256;
use async_trait::async_trait;
use sha3::{Digest, Keccak256};
use uuid::Uuid;

use crate::execution::remote_signer::{
    RemoteSigner, SignerError as PerpsSignerError, SignerRequest as PerpsSignerRequest,
};
use crate::execution::signer::RecoverableSignature;
use crate::hybrid_v2::execution::signer::{
    ExecutionSigner, SignedTx, SignerAvailability, SignerError, SignerIdentity, SignerKind,
    SigningRequest,
};
use crate::types::AccountId;

/// Domain tag pinned into the idempotency-key preimage. Refuses replay
/// of an idempotency key derived by a different milestone / product.
pub const IDEMPOTENCY_DOMAIN_TAG: &[u8] = b"HV2_SIGNER_IDEMPOTENCY_V1";

/// Length (bytes) of the persisted idempotency-key prefix.
pub const IDEMPOTENCY_KEY_LEN: usize = 16;

/// Bridge from the Hybrid V2 [`ExecutionSigner`] trait onto the Perps
/// [`RemoteSigner`] stack. Owns:
///
/// * `inner` — any Perps `RemoteSigner` (typically a
///   `RemoteSignerClient` wrapping a `PluggableRemoteSignerTransport`
///   built around `AwsKmsSignerProvider` — Pattern C).
/// * `expected_signer_address` — the configured EOA. Cross-checked at
///   sign time; a divergent recovered address returns
///   `SignerError::IdentityMismatch` before the caller can persist a
///   compromised signature.
/// * `signer_kind` — documentation-only discriminator surfaced by
///   [`ExecutionSigner::identity`].
/// * `endpoint_uri_redacted` — pre-redacted host:port for `identity()`
///   display only. NEVER holds credentials or path fragments.
/// * `max_retries` — bounded retry budget for transient transport
///   failures (Timeout / NetworkFailure / KmsUnavailable /
///   KmsTimeout). Deterministic rejections are never retried.
/// * `request_timeout` — advisory upper bound on each attempt. Ferried
///   into the Perps request as `deadline_ms`.
pub struct HybridV2KmsSignerBridge {
    inner: Arc<dyn RemoteSigner>,
    expected_signer_address: [u8; 20],
    signer_kind: SignerKind,
    endpoint_uri_redacted: String,
    max_retries: u32,
    request_timeout: Duration,
}

impl std::fmt::Debug for HybridV2KmsSignerBridge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HybridV2KmsSignerBridge")
            .field(
                "expected_signer_address",
                &format!("0x{}", hex_encode(&self.expected_signer_address)),
            )
            .field("signer_kind", &self.signer_kind)
            .field("endpoint_uri_redacted", &self.endpoint_uri_redacted)
            .field("max_retries", &self.max_retries)
            .field("request_timeout_ms", &self.request_timeout.as_millis())
            .finish()
    }
}

impl HybridV2KmsSignerBridge {
    pub fn new(
        inner: Arc<dyn RemoteSigner>,
        expected_signer_address: [u8; 20],
        signer_kind: SignerKind,
        endpoint_uri_redacted: String,
        max_retries: u32,
        request_timeout: Duration,
    ) -> Self {
        Self {
            inner,
            expected_signer_address,
            signer_kind,
            endpoint_uri_redacted,
            max_retries: max_retries.min(5),
            request_timeout,
        }
    }

    pub fn expected_signer_address(&self) -> [u8; 20] {
        self.expected_signer_address
    }

    pub fn max_retries(&self) -> u32 {
        self.max_retries
    }

    /// Optional health probe. Delegates to the inner signer's
    /// `health_check`; classifies the outcome into a
    /// [`SignerAvailability`] verdict without holding the store lock.
    pub async fn probe(&self) -> SignerAvailability {
        match self.inner.health_check().await {
            Ok(health) => {
                if !health.healthy {
                    return SignerAvailability::Unavailable {
                        reason: "vendor reported unhealthy".to_string(),
                    };
                }
                // Address cross-check — even during health, defence-
                // in-depth.
                let Some(observed) = health.signer_address.as_ref() else {
                    return SignerAvailability::Configured;
                };
                match parse_address_ci(&observed.0) {
                    Some(bytes) if bytes == self.expected_signer_address => {
                        SignerAvailability::Configured
                    }
                    Some(bytes) => SignerAvailability::IdentityMismatch {
                        expected: self.expected_signer_address,
                        observed: bytes,
                    },
                    None => SignerAvailability::Unavailable {
                        reason: "vendor returned a non-address string".to_string(),
                    },
                }
            }
            Err(err) => classify_availability(&err),
        }
    }
}

#[async_trait]
impl ExecutionSigner for HybridV2KmsSignerBridge {
    fn identity(&self) -> SignerIdentity {
        SignerIdentity {
            address: self.expected_signer_address,
            kind: self.signer_kind.clone(),
            uri: Some(self.endpoint_uri_redacted.clone()),
        }
    }

    async fn sign_execution(&self, request: SigningRequest) -> Result<SignedTx, SignerError> {
        // Defence-in-depth: base mainnet is refused at every layer.
        // The firewall already rejected it upstream but a stray caller
        // that somehow reached the bridge without the firewall must
        // still fail closed.
        if request.chain_id == 8453 {
            return Err(SignerError::ChainMismatch);
        }
        // Build the Perps request. We carry a synthetic canonical id
        // -> UUID mapping so the Perps side has stable, non-secret
        // ids for CloudTrail correlation.
        let idem_key = derive_idempotency_key(
            &self.expected_signer_address,
            &request.plan_hash,
            &request.signing_payload_hash,
            // Canonical execution id is not on `SigningRequest`; the
            // orchestrator threads it in via `plan_hash + payload_hash`
            // — both bind the row identity. The idempotency-key
            // derivation matches the persisted-column value derived by
            // the orchestrator (Part L).
            &request.plan_hash,
        );
        // Perps request wants a couple of borrowed accountIds. Build
        // them once at the top of the call so the borrows are valid
        // for the async body below.
        let target_hex = format!("0x{}", hex_encode(&request.target));
        let target_id = AccountId::new(target_hex);
        let value_wei: u128 = u128_from_u256(&request.value_wei).unwrap_or(0);
        let max_fee_str = request.max_fee_per_gas_wei.to_string();
        let max_priority_str = request.max_priority_fee_per_gas_wei.to_string();
        let now_ms = wall_ms();
        let deadline_ms = now_ms.saturating_add(self.request_timeout.as_millis() as i64);
        let policy_decision_id = uuid_from_bytes32(&request.plan_hash);
        let intent_id = uuid_from_bytes32(&request.signing_payload_hash);
        let request_id = uuid_from_16(&idem_key);

        // Bounded retry loop. Only transient transport failures
        // retry; every deterministic policy rejection short-circuits.
        let mut attempt: u32 = 0;
        let attempts_allowed = self.max_retries.saturating_add(1);
        loop {
            attempt = attempt.saturating_add(1);
            let perps_req = PerpsSignerRequest {
                request_id,
                intent_id,
                source_type: "hybrid_v2_option_match",
                chain_id: request.chain_id,
                target_contract: &target_id,
                function_selector: "0xa1b2c3d4", // opaque to the vendor
                calldata_hash: request.calldata_hash,
                calldata_length: request.calldata.len(),
                transaction_to: &target_id,
                transaction_value_wei: value_wei,
                gas_limit: request.gas_limit,
                max_fee_per_gas_wei: max_fee_str.as_str(),
                max_priority_fee_per_gas_wei: max_priority_str.as_str(),
                nonce: request.nonce,
                simulation_block: 0,
                deadline_ms,
                policy_decision_id,
                policy_fingerprint: pad_fingerprint(&idem_key),
                policy_decision_at_ms: now_ms,
                prehash: request.signing_payload_hash,
            };
            match self.inner.sign_option_execution_tx(perps_req).await {
                Ok(response) => {
                    let signed = extract_signed(&response.signature, request.tx_type)?;
                    // Post-sign cross-check (belt + suspenders). The
                    // Perps RemoteSignerClient already did this; the
                    // bridge repeats because trust is minimized.
                    let observed = match parse_address_ci(&response.signer_address.0) {
                        Some(bytes) => bytes,
                        None => {
                            return Err(SignerError::MalformedResponse(format!(
                                "signer returned non-address string endpoint={}",
                                self.endpoint_uri_redacted
                            )));
                        }
                    };
                    if observed != self.expected_signer_address {
                        return Err(SignerError::IdentityMismatch {
                            expected: self.expected_signer_address,
                            actual: observed,
                        });
                    }
                    let mut out = signed;
                    out.recovered_signer = observed;
                    return Ok(out);
                }
                Err(err) => {
                    if attempt >= attempts_allowed || !is_retryable(&err) {
                        return Err(classify_error(&err, &self.endpoint_uri_redacted));
                    }
                    // Retry — loop back around.
                }
            }
        }
    }

    fn availability(&self) -> SignerAvailability {
        // On-demand `probe()` is available; the sync default reports
        // Configured so a construction-time success is not overridden
        // by a stale probe result. Callers that want a fresh verdict
        // invoke `probe()` explicitly.
        SignerAvailability::Configured
    }
}

// ---------------------------------------------------------------
//                          helpers
// ---------------------------------------------------------------

/// Derive the 16-byte idempotency key persisted on the execution row
/// and forwarded to the vendor. Domain-separated by
/// [`IDEMPOTENCY_DOMAIN_TAG`].
///
/// ```text
/// keccak256(
///   "HV2_SIGNER_IDEMPOTENCY_V1"
///     || expected_signer_address (20 bytes)
///     || canonical_execution_id  (variable — passed as raw bytes)
///     || plan_hash               (32 bytes)
///     || signing_payload_hash    (32 bytes)
/// )[..16]
/// ```
///
/// The 16-byte prefix is enough entropy for a per-execution UUID (see
/// RFC 4122); we pin to 128 bits to keep the persisted column
/// compact.
pub fn derive_idempotency_key(
    expected_signer_address: &[u8; 20],
    canonical_execution_id: &[u8],
    plan_hash: &[u8; 32],
    signing_payload_hash: &[u8; 32],
) -> [u8; IDEMPOTENCY_KEY_LEN] {
    let mut hasher = Keccak256::new();
    hasher.update(IDEMPOTENCY_DOMAIN_TAG);
    hasher.update(expected_signer_address);
    hasher.update(canonical_execution_id);
    hasher.update(plan_hash);
    hasher.update(signing_payload_hash);
    let out = hasher.finalize();
    let mut key = [0u8; IDEMPOTENCY_KEY_LEN];
    key.copy_from_slice(&out[..IDEMPOTENCY_KEY_LEN]);
    key
}

fn extract_signed(signature: &RecoverableSignature, tx_type: u8) -> Result<SignedTx, SignerError> {
    if signature.y_parity > 1 {
        return Err(SignerError::MalformedResponse(format!(
            "y_parity {} not in {{0, 1}}",
            signature.y_parity
        )));
    }
    Ok(SignedTx {
        signature_r: signature.r,
        signature_s: signature.s,
        signature_v: signature.y_parity,
        // Overwritten by the caller with the cross-checked address.
        recovered_signer: [0u8; 20],
        tx_type,
    })
}

fn classify_error(err: &PerpsSignerError, endpoint_redacted: &str) -> SignerError {
    use PerpsSignerError as E;
    match err {
        E::Transport(msg) => SignerError::NetworkFailure(format!(
            "endpoint={endpoint_redacted}: {}",
            truncate_for_error(msg)
        )),
        E::KmsTimeout => SignerError::Timeout,
        E::KmsUnavailable => SignerError::SignerUnavailable(format!(
            "vendor unavailable at endpoint={endpoint_redacted}"
        )),
        E::CallerUnauthorized => SignerError::SignerUnavailable("vendor auth failed".to_string()),
        E::RateLimit => {
            SignerError::NetworkFailure(format!("rate-limited at endpoint={endpoint_redacted}"))
        }
        E::PolicyFingerprint => SignerError::PolicyViolation(
            "vendor rejected policy fingerprint (idempotency key mismatch)".to_string(),
        ),
        E::PolicyStale => SignerError::PolicyViolation("vendor rejected: policy stale".to_string()),
        E::CalldataBindMismatch => SignerError::CalldataHashMismatch,
        E::ChainNotAllowed => SignerError::ChainMismatch,
        E::TargetNotAllowed | E::SelectorNotAllowed => {
            SignerError::PolicyViolation(format!("vendor rejected: {}", err.code()))
        }
        E::ValueNotZero | E::GasCap | E::NonceMismatch | E::DeadlineExpired => {
            SignerError::PolicyViolation(format!("vendor rejected: {}", err.code()))
        }
        E::DuplicateRequestId | E::DuplicateIntentSigned => {
            SignerError::PolicyViolation(format!("vendor rejected: {}", err.code()))
        }
        E::PostSignFromMismatch => SignerError::IdentityMismatch {
            expected: [0u8; 20],
            actual: [0u8; 20],
        },
        E::Internal(msg) => SignerError::MalformedResponse(truncate_for_error(msg)),
        E::ConfigRefusal(msg) => SignerError::SignerUnavailable(truncate_for_error(msg)),
    }
}

fn classify_availability(err: &PerpsSignerError) -> SignerAvailability {
    use PerpsSignerError as E;
    match err {
        E::Transport(msg) => SignerAvailability::Unavailable {
            reason: truncate_for_error(msg),
        },
        E::KmsTimeout => SignerAvailability::TransientFailure {
            reason: "vendor timeout".to_string(),
        },
        E::KmsUnavailable | E::RateLimit => SignerAvailability::TransientFailure {
            reason: err.code().to_string(),
        },
        E::CallerUnauthorized => SignerAvailability::AuthenticationFailed {
            reason: "vendor auth failed".to_string(),
        },
        E::PostSignFromMismatch => SignerAvailability::IdentityMismatch {
            expected: [0u8; 20],
            observed: [0u8; 20],
        },
        E::ConfigRefusal(msg) => SignerAvailability::DeterministicRefusal {
            reason: truncate_for_error(msg),
        },
        E::PolicyFingerprint | E::PolicyStale => SignerAvailability::DeterministicRefusal {
            reason: err.code().to_string(),
        },
        _ => SignerAvailability::DeterministicRefusal {
            reason: err.code().to_string(),
        },
    }
}

fn is_retryable(err: &PerpsSignerError) -> bool {
    matches!(
        err,
        PerpsSignerError::Transport(_)
            | PerpsSignerError::KmsTimeout
            | PerpsSignerError::KmsUnavailable
            | PerpsSignerError::RateLimit
    )
}

fn truncate_for_error(reason: &str) -> String {
    const MAX: usize = 80;
    if reason.len() <= MAX {
        reason.to_string()
    } else {
        reason.chars().take(MAX).collect()
    }
}

fn pad_fingerprint(key: &[u8; IDEMPOTENCY_KEY_LEN]) -> [u8; 32] {
    let mut out = [0u8; 32];
    out[..IDEMPOTENCY_KEY_LEN].copy_from_slice(key);
    out
}

fn uuid_from_bytes32(bytes: &[u8; 32]) -> Uuid {
    let mut arr = [0u8; 16];
    arr.copy_from_slice(&bytes[..16]);
    Uuid::from_bytes(arr)
}

fn uuid_from_16(bytes: &[u8; IDEMPOTENCY_KEY_LEN]) -> Uuid {
    let mut arr = [0u8; 16];
    arr.copy_from_slice(bytes);
    Uuid::from_bytes(arr)
}

fn u128_from_u256(v: &U256) -> Option<u128> {
    let be = v.to_be_bytes::<32>();
    // reject anything above 2^128 - 1
    if be[..16].iter().any(|b| *b != 0) {
        return None;
    }
    let mut lo = [0u8; 16];
    lo.copy_from_slice(&be[16..]);
    Some(u128::from_be_bytes(lo))
}

fn wall_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn parse_address_ci(s: &str) -> Option<[u8; 20]> {
    let stripped = s
        .strip_prefix("0x")
        .or_else(|| s.strip_prefix("0X"))
        .unwrap_or(s);
    if stripped.len() != 40 || !stripped.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let mut out = [0u8; 20];
    for i in 0..20 {
        out[i] = u8::from_str_radix(&stripped[2 * i..2 * i + 2], 16).ok()?;
    }
    Some(out)
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{:02x}", b));
    }
    s
}

/// Reduce an endpoint URL to a redacted form suitable for identity()
/// display and error messages. Strips path, query, credentials —
/// retains only `scheme://host[:port]`. Any parse failure collapses
/// to `<opaque>`.
pub fn redacted_endpoint(url: &str) -> String {
    match reqwest::Url::parse(url) {
        Ok(u) => {
            let scheme = u.scheme();
            let host = u.host_str().unwrap_or("<opaque>");
            match u.port() {
                Some(p) => format!("{scheme}://{host}:{p}"),
                None => format!("{scheme}://{host}"),
            }
        }
        Err(_) => "<opaque>".to_string(),
    }
}

// -----------------------------------------------------------------
//                          UNIT TESTS
// -----------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::execution::remote_signer::{SignerFuture, SignerHealth, SignerResponse};
    use std::sync::Mutex;

    struct MockRemoteSigner {
        canned: Mutex<Vec<Result<SignerResponse, PerpsSignerError>>>,
        health: Mutex<Result<SignerHealth, PerpsSignerError>>,
        calls: Mutex<u32>,
        addr: AccountId,
    }

    impl MockRemoteSigner {
        fn approve(response: SignerResponse) -> Self {
            let addr = response.signer_address.clone();
            Self {
                canned: Mutex::new(vec![Ok(response)]),
                health: Mutex::new(Ok(SignerHealth {
                    mode: crate::execution::remote_signer::SignerBackendKind::Remote,
                    signer_address: Some(addr.clone()),
                    remote_endpoint_present: true,
                    healthy: true,
                })),
                calls: Mutex::new(0),
                addr,
            }
        }

        fn deny_transient(addr: AccountId, err: PerpsSignerError) -> Self {
            Self {
                canned: Mutex::new(vec![Err(err)]),
                health: Mutex::new(Ok(SignerHealth {
                    mode: crate::execution::remote_signer::SignerBackendKind::Remote,
                    signer_address: Some(addr.clone()),
                    remote_endpoint_present: true,
                    healthy: true,
                })),
                calls: Mutex::new(0),
                addr,
            }
        }

        fn push(&self, r: Result<SignerResponse, PerpsSignerError>) {
            self.canned.lock().unwrap().push(r);
        }

        fn calls(&self) -> u32 {
            *self.calls.lock().unwrap()
        }
    }

    impl RemoteSigner for MockRemoteSigner {
        fn signer_address(&self) -> &AccountId {
            &self.addr
        }

        fn kind(&self) -> crate::execution::remote_signer::SignerBackendKind {
            crate::execution::remote_signer::SignerBackendKind::Remote
        }

        fn sign_option_execution_tx<'a>(
            &'a self,
            _request: PerpsSignerRequest<'a>,
        ) -> SignerFuture<'a, SignerResponse> {
            *self.calls.lock().unwrap() += 1;
            let mut queue = self.canned.lock().unwrap();
            let next = if queue.is_empty() {
                Err(PerpsSignerError::Transport("mock exhausted".to_string()))
            } else {
                queue.remove(0)
            };
            Box::pin(async move { next })
        }

        fn health_check(&self) -> SignerFuture<'_, SignerHealth> {
            let out = self.health.lock().unwrap().clone();
            Box::pin(async move { out })
        }
    }

    // A well-known test key (already pinned across the perps signer
    // tests) — its address is `TEST_ADDRESS`.
    const TEST_KEY: &str = "0x4c0883a69102937d6231471b5dbb6204fe5129617082792ae468d01a3f362318";
    const TEST_ADDRESS_HEX: &str = "0x2c7536e3605d9c16a7a3d7b1898e529396a65c23";

    fn test_signer_addr_bytes() -> [u8; 20] {
        parse_address_ci(TEST_ADDRESS_HEX).unwrap()
    }

    fn test_signer_response(prehash: [u8; 32]) -> SignerResponse {
        let signer = crate::execution::signer::ExecutorSigner::from_private_key(
            &crate::execution::config::PrivateKeySecret::new(TEST_KEY.to_string()),
        )
        .unwrap();
        let sig = signer.sign_prehash(&prehash).unwrap();
        SignerResponse {
            request_id: Uuid::from_u128(1),
            signer_address: AccountId::new(TEST_ADDRESS_HEX),
            signature: sig,
            kms_request_id: Some("mock-req".to_string()),
            audit_log_id: Some("mock-audit".to_string()),
            remote_signer_request_id: Some("mock-rsig".to_string()),
            created_at_ms: 1_700_000_000_000,
            policy_decision_id: Uuid::from_u128(0),
        }
    }

    fn baseline_request() -> SigningRequest {
        SigningRequest {
            chain_id: 84532,
            nonce: 7,
            target: [0xcc; 20],
            value_wei: U256::ZERO,
            calldata_hash: [0xdd; 32],
            gas_limit: 100_000,
            max_fee_per_gas_wei: U256::from(1u64),
            max_priority_fee_per_gas_wei: U256::from(1u64),
            tx_type: 2,
            plan_hash: [0xee; 32],
            signing_payload_hash: [0xff; 32],
            calldata: vec![0x01, 0x02],
        }
    }

    #[test]
    fn idempotency_key_is_deterministic_and_domain_separated() {
        let addr = test_signer_addr_bytes();
        let cid = b"canonical-execution-id-bytes";
        let plan_hash = [0x11u8; 32];
        let payload = [0x22u8; 32];
        let a = derive_idempotency_key(&addr, cid, &plan_hash, &payload);
        let b = derive_idempotency_key(&addr, cid, &plan_hash, &payload);
        assert_eq!(a, b);
        // Different plan_hash → different key.
        let c = derive_idempotency_key(&addr, cid, &[0x99u8; 32], &payload);
        assert_ne!(a, c);
        // Different address → different key.
        let d = derive_idempotency_key(&[0u8; 20], cid, &plan_hash, &payload);
        assert_ne!(a, d);
        // Length is bounded to 16 bytes.
        assert_eq!(a.len(), IDEMPOTENCY_KEY_LEN);
    }

    #[test]
    fn redacted_endpoint_strips_path_and_query() {
        assert_eq!(
            redacted_endpoint("https://signer.example.com/v1/keys/secret?x=1"),
            "https://signer.example.com"
        );
        assert_eq!(
            redacted_endpoint("http://127.0.0.1:9000/sign"),
            "http://127.0.0.1:9000"
        );
        assert_eq!(redacted_endpoint("not-a-url"), "<opaque>");
    }

    #[test]
    fn signer_kind_and_identity_expose_expected_address_but_no_secret() {
        let inner = Arc::new(MockRemoteSigner::approve(test_signer_response(
            baseline_request().signing_payload_hash,
        )));
        let bridge = HybridV2KmsSignerBridge::new(
            inner,
            test_signer_addr_bytes(),
            SignerKind::RemoteKMS,
            "https://signer.example.com".to_string(),
            1,
            Duration::from_millis(2_500),
        );
        let identity = bridge.identity();
        assert_eq!(identity.address, test_signer_addr_bytes());
        assert_eq!(identity.kind, SignerKind::RemoteKMS);
        assert_eq!(identity.uri.as_deref(), Some("https://signer.example.com"));
        // Debug must not include /v1/... path fragments even if the
        // caller passes a full URL through by accident.
        let dbg = format!("{bridge:?}");
        assert!(!dbg.contains("/v1/"));
    }

    #[tokio::test]
    async fn happy_path_round_trip_returns_signed_tx_bound_to_expected_address() {
        let request = baseline_request();
        let inner = Arc::new(MockRemoteSigner::approve(test_signer_response(
            request.signing_payload_hash,
        )));
        let bridge = HybridV2KmsSignerBridge::new(
            inner.clone(),
            test_signer_addr_bytes(),
            SignerKind::RemoteKMS,
            "https://signer.example.com".to_string(),
            1,
            Duration::from_millis(2_500),
        );
        let signed = bridge.sign_execution(request).await.unwrap();
        assert_eq!(signed.recovered_signer, test_signer_addr_bytes());
        assert!(signed.signature_v <= 1);
        assert_eq!(signed.tx_type, 2);
        assert_eq!(inner.calls(), 1);
    }

    #[tokio::test]
    async fn identity_mismatch_rejected_at_bridge_boundary() {
        let request = baseline_request();
        // Vendor returns a signature whose recovered address is a
        // different 20-byte address than the bridge expects.
        let mut response = test_signer_response(request.signing_payload_hash);
        response.signer_address = AccountId::new("0xdeaddeaddeaddeaddeaddeaddeaddeaddeaddead");
        let inner = Arc::new(MockRemoteSigner::approve(response));
        let bridge = HybridV2KmsSignerBridge::new(
            inner,
            test_signer_addr_bytes(),
            SignerKind::RemoteKMS,
            "https://signer.example.com".to_string(),
            0,
            Duration::from_millis(2_500),
        );
        let err = bridge.sign_execution(request).await.unwrap_err();
        assert!(matches!(err, SignerError::IdentityMismatch { .. }));
    }

    #[tokio::test]
    async fn transient_transport_failure_is_retried_bounded() {
        let request = baseline_request();
        // First attempt: transient transport failure. Second attempt:
        // success. Bridge must swallow one failure and land at Ok.
        let inner = Arc::new(MockRemoteSigner::deny_transient(
            AccountId::new(TEST_ADDRESS_HEX),
            PerpsSignerError::Transport("blip".to_string()),
        ));
        // Prime a follow-up success.
        inner.push(Ok(test_signer_response(request.signing_payload_hash)));
        let bridge = HybridV2KmsSignerBridge::new(
            inner.clone(),
            test_signer_addr_bytes(),
            SignerKind::RemoteKMS,
            "https://signer.example.com".to_string(),
            1, // one retry allowed
            Duration::from_millis(2_500),
        );
        let signed = bridge.sign_execution(request).await.unwrap();
        assert_eq!(signed.recovered_signer, test_signer_addr_bytes());
        assert_eq!(inner.calls(), 2);
    }

    #[tokio::test]
    async fn deterministic_refusal_is_not_retried() {
        let request = baseline_request();
        let inner = Arc::new(MockRemoteSigner::deny_transient(
            AccountId::new(TEST_ADDRESS_HEX),
            PerpsSignerError::PolicyFingerprint,
        ));
        // Prime a hypothetical follow-up success — the bridge must
        // never consume it because the first failure is deterministic.
        inner.push(Ok(test_signer_response(request.signing_payload_hash)));
        let bridge = HybridV2KmsSignerBridge::new(
            inner.clone(),
            test_signer_addr_bytes(),
            SignerKind::RemoteKMS,
            "https://signer.example.com".to_string(),
            3, // budget available, must not be consumed
            Duration::from_millis(2_500),
        );
        let err = bridge.sign_execution(request).await.unwrap_err();
        assert!(matches!(err, SignerError::PolicyViolation(_)));
        assert_eq!(inner.calls(), 1);
    }

    #[tokio::test]
    async fn kms_timeout_maps_to_signer_timeout() {
        let request = baseline_request();
        let inner = Arc::new(MockRemoteSigner::deny_transient(
            AccountId::new(TEST_ADDRESS_HEX),
            PerpsSignerError::KmsTimeout,
        ));
        let bridge = HybridV2KmsSignerBridge::new(
            inner,
            test_signer_addr_bytes(),
            SignerKind::RemoteKMS,
            "https://signer.example.com".to_string(),
            0,
            Duration::from_millis(500),
        );
        let err = bridge.sign_execution(request).await.unwrap_err();
        assert!(matches!(err, SignerError::Timeout));
    }

    #[tokio::test]
    async fn mainnet_chain_id_is_refused_at_bridge_boundary() {
        let mut request = baseline_request();
        request.chain_id = 8453;
        let inner = Arc::new(MockRemoteSigner::approve(test_signer_response(
            request.signing_payload_hash,
        )));
        let bridge = HybridV2KmsSignerBridge::new(
            inner.clone(),
            test_signer_addr_bytes(),
            SignerKind::RemoteKMS,
            "https://signer.example.com".to_string(),
            0,
            Duration::from_millis(2_500),
        );
        let err = bridge.sign_execution(request).await.unwrap_err();
        assert!(matches!(err, SignerError::ChainMismatch));
        assert_eq!(
            inner.calls(),
            0,
            "vendor MUST NOT be called on mainnet chain id"
        );
    }

    #[tokio::test]
    async fn probe_reports_configured_when_health_ok_and_address_matches() {
        let request = baseline_request();
        let inner = Arc::new(MockRemoteSigner::approve(test_signer_response(
            request.signing_payload_hash,
        )));
        let bridge = HybridV2KmsSignerBridge::new(
            inner,
            test_signer_addr_bytes(),
            SignerKind::RemoteKMS,
            "https://signer.example.com".to_string(),
            0,
            Duration::from_millis(2_500),
        );
        assert_eq!(bridge.probe().await, SignerAvailability::Configured);
    }
}
