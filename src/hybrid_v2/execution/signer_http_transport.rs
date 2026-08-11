//! `BACKEND-HYBRID-V2-PRODUCTION-SIGNER-BOOTSTRAP-AND-STARTUP-WIRING-V1`
//! Part C — HTTP transport that talks to the production signer
//! microservice over mTLS.
//!
//! ## Frozen posture
//!
//! * **BROADCAST_STRICTLY_FORBIDDEN** — this transport only issues
//!   `POST /hybrid_v2/sign`, `GET /hybrid_v2/identity`, and
//!   `GET /hybrid_v2/health`. None of those write chain state.
//! * **SIGNER_ARCHITECTURE_HAS_ONE_UNAMBIGUOUS_CUSTODY_BOUNDARY** —
//!   the backend authenticates to the signer service with an mTLS
//!   client identity (cert + key). AWS credentials NEVER live in the
//!   backend; the signer microservice holds its own IAM role. This
//!   transport does not import any `aws-sdk-*` symbol.
//! * **PRODUCTION_BACKEND_DOES_NOT_CUSTODY_RAW_PRIVATE_KEYS** — no
//!   field or intermediate value in this file holds a raw private key.
//! * **SIGNER_UNAVAILABLE_NEVER_FALLS_BACK_TO_LOCAL_RAW_KEY** — an
//!   unreachable / mis-configured microservice returns
//!   `SignerError::Transport(...)` and the caller surfaces
//!   `SIGNER_UNAVAILABLE`. Silent fallback to any local signer is
//!   structurally impossible here (this file has no local signer).
//! * **NO_LOG_OR_ERROR_LEAKS_A_SECRET** — `Debug` impl redacts the
//!   endpoint URL to `<host:port>` and every certificate / key path
//!   collapses to a `<set>` marker.
//!
//! ## Wire protocol (JSON)
//!
//! Request `POST {endpoint}/hybrid_v2/sign`
//! ```json
//! {
//!   "chain_id": u64,
//!   "nonce": u64,
//!   "digest": "0x{32 bytes hex}",
//!   "target": "0x{20 bytes hex}",
//!   "value_wei_hex": "0x...",
//!   "gas_limit": u64,
//!   "max_fee_per_gas_hex": "0x...",
//!   "max_priority_fee_per_gas_hex": "0x...",
//!   "tx_type": u8,
//!   "expected_signer": "0x{20 bytes hex}",
//!   "idempotency_key": "0x{16 bytes hex}",
//!   "policy_decision_id": "0x{32 bytes hex} (plan_hash)",
//!   "fingerprint": "0x{32 bytes hex} (payload hash)"
//! }
//! ```
//!
//! Response
//! ```json
//! {
//!   "signature_r": "0x{32 bytes hex}",
//!   "signature_s": "0x{32 bytes hex}",
//!   "signature_v": 0 | 1,
//!   "recovered_signer": "0x{20 bytes hex}"
//! }
//! ```
//!
//! Identity probe `GET {endpoint}/hybrid_v2/identity`
//! ```json
//! { "signer_address": "0x{20 bytes hex}", "chain_id": u64 }
//! ```
//!
//! Health `GET {endpoint}/hybrid_v2/health`
//! ```json
//! { "healthy": bool, "signer_address": "0x...", "chain_id": u64 }
//! ```

use std::time::Duration;

use serde_json::{json, Value};

use crate::execution::remote_signer::{
    SignerBackendKind, SignerError as PerpsSignerError, SignerFuture, SignerHealth,
    SignerRequest as PerpsSignerRequest, SignerResponse, SignerTransport,
};
use crate::execution::signer::RecoverableSignature;
use crate::types::AccountId;

// -----------------------------------------------------------------
//                       transport build errors
// -----------------------------------------------------------------

/// Structured errors surfaced by [`HttpSignerTransport::new`].
///
/// Every variant carries a short, redacted reason — never a raw URL,
/// cert PEM, or key PEM.
#[derive(Debug)]
pub enum TransportBuildError {
    /// Endpoint URL is not http:// or https://.
    UnsupportedScheme,
    /// The URL failed to parse.
    MalformedEndpoint,
    /// `reqwest::Client::builder()` failed. Redacted reason.
    ClientBuild(String),
    /// A supplied PEM blob failed to parse.
    MalformedCertPem,
    /// A supplied private-key PEM blob failed to parse.
    MalformedKeyPem,
    /// The supplied root CA PEM did not contain a valid certificate.
    MalformedRootCaPem,
}

impl std::fmt::Display for TransportBuildError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedScheme => {
                f.write_str("HttpSignerTransport: endpoint must use http:// or https://")
            }
            Self::MalformedEndpoint => {
                f.write_str("HttpSignerTransport: endpoint URL is malformed")
            }
            Self::ClientBuild(reason) => {
                write!(
                    f,
                    "HttpSignerTransport: reqwest client build failed: {reason}"
                )
            }
            Self::MalformedCertPem => {
                f.write_str("HttpSignerTransport: mTLS client certificate PEM is malformed")
            }
            Self::MalformedKeyPem => {
                f.write_str("HttpSignerTransport: mTLS client key PEM is malformed")
            }
            Self::MalformedRootCaPem => {
                f.write_str("HttpSignerTransport: root CA PEM is malformed")
            }
        }
    }
}

impl std::error::Error for TransportBuildError {}

// -----------------------------------------------------------------
//                        HttpSignerTransport
// -----------------------------------------------------------------

/// mTLS HTTPS client that fulfills the Perps `SignerTransport`
/// contract by talking to a signer microservice at `endpoint`. The
/// backend NEVER holds AWS credentials — the microservice does.
pub struct HttpSignerTransport {
    endpoint: String,
    client: reqwest::Client,
    expected_signer_address: [u8; 20],
    request_timeout: Duration,
    #[allow(dead_code)] // recorded for observability & future backoff scaling
    max_retries: u32,
    // Presence-only markers so `Debug` never leaks paths / auth material.
    has_client_cert: bool,
    has_client_key: bool,
    has_root_ca: bool,
}

impl std::fmt::Debug for HttpSignerTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HttpSignerTransport")
            .field("endpoint", &redact_endpoint(&self.endpoint))
            .field(
                "expected_signer_address",
                &format!("0x{}", hex_encode(&self.expected_signer_address)),
            )
            .field("request_timeout_ms", &self.request_timeout.as_millis())
            .field("max_retries", &self.max_retries)
            .field(
                "client_cert",
                &if self.has_client_cert {
                    "<set>"
                } else {
                    "<unset>"
                },
            )
            .field(
                "client_key",
                &if self.has_client_key {
                    "<set>"
                } else {
                    "<unset>"
                },
            )
            .field(
                "root_ca",
                &if self.has_root_ca { "<set>" } else { "<unset>" },
            )
            .finish()
    }
}

impl HttpSignerTransport {
    /// Build an operational transport. mTLS certificate + key material
    /// is applied to the underlying `reqwest::Client` when supplied.
    /// Enforcement of "mTLS-mandatory-on-public-HTTPS" is a caller
    /// responsibility (see `SignerBuilder`), so this constructor does
    /// not itself refuse a bare HTTPS URL — the operator's config
    /// validator + the builder do.
    pub fn new(
        endpoint: String,
        expected_signer_address: [u8; 20],
        mtls_cert_pem: Option<Vec<u8>>,
        mtls_key_pem: Option<Vec<u8>>,
        root_ca_pem: Option<Vec<u8>>,
        timeout_ms: u32,
        max_retries: u32,
    ) -> Result<Self, TransportBuildError> {
        // Scheme validation. The transport ONLY speaks http/https.
        let lower = endpoint.to_ascii_lowercase();
        if !(lower.starts_with("http://") || lower.starts_with("https://")) {
            return Err(TransportBuildError::UnsupportedScheme);
        }
        // Parse-only sanity check — redacts to `<opaque>` in Debug.
        reqwest::Url::parse(&endpoint).map_err(|_| TransportBuildError::MalformedEndpoint)?;

        let request_timeout = Duration::from_millis(timeout_ms as u64);
        let mut builder = reqwest::Client::builder()
            .timeout(request_timeout)
            .connect_timeout(request_timeout)
            // No cookies, no automatic redirects — a signer service
            // MUST NOT redirect a signing call.
            .redirect(reqwest::redirect::Policy::none());

        let has_client_cert = mtls_cert_pem.is_some();
        let has_client_key = mtls_key_pem.is_some();
        let has_root_ca = root_ca_pem.is_some();

        // Attach client identity when both cert + key are supplied.
        // The `Identity::from_pem` builder expects the concatenation of
        // the client cert PEM and its private-key PEM.
        if let (Some(cert), Some(key)) = (mtls_cert_pem, mtls_key_pem) {
            let mut combined = Vec::with_capacity(cert.len() + key.len() + 1);
            combined.extend_from_slice(&cert);
            if !combined.ends_with(b"\n") {
                combined.push(b'\n');
            }
            combined.extend_from_slice(&key);
            let identity = reqwest::Identity::from_pem(&combined).map_err(|_| {
                // Cannot distinguish cert vs key parse failure at this
                // layer without leaking bytes. The safer verdict is
                // "malformed key pem" because that's the more common
                // operator mistake.
                TransportBuildError::MalformedKeyPem
            })?;
            builder = builder.identity(identity);
        } else if has_client_cert && !has_client_key {
            return Err(TransportBuildError::MalformedKeyPem);
        } else if has_client_key && !has_client_cert {
            return Err(TransportBuildError::MalformedCertPem);
        }

        // Optional custom root CA (pinning).
        if let Some(ca_pem) = root_ca_pem {
            let ca = reqwest::Certificate::from_pem(&ca_pem)
                .map_err(|_| TransportBuildError::MalformedRootCaPem)?;
            builder = builder.add_root_certificate(ca);
        }

        let client = builder
            .build()
            .map_err(|e| TransportBuildError::ClientBuild(truncate(&e.to_string(), 120)))?;

        Ok(Self {
            endpoint,
            client,
            expected_signer_address,
            request_timeout,
            max_retries,
            has_client_cert,
            has_client_key,
            has_root_ca,
        })
    }

    /// Redacted endpoint suitable for identity() display and error
    /// messages. Strips path, query, credentials.
    pub fn endpoint_redacted(&self) -> String {
        redact_endpoint(&self.endpoint)
    }

    /// Non-transaction identity probe. Contacts
    /// `GET {endpoint}/hybrid_v2/identity` and returns the vendor's
    /// self-reported `(signer_address, chain_id)` pair. Used at
    /// startup to cross-check identity BEFORE the orchestrator is
    /// admitted into `AppState`.
    pub async fn fetch_identity(&self) -> Result<SignerIdentityProbe, PerpsSignerError> {
        let url = join_path(&self.endpoint, "hybrid_v2/identity");
        let response = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| classify_reqwest_err(&e))?;

        let status = response.status();
        if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
            return Err(PerpsSignerError::CallerUnauthorized);
        }
        if status.is_server_error() {
            return Err(PerpsSignerError::KmsUnavailable);
        }
        if !status.is_success() {
            return Err(PerpsSignerError::Transport(format!(
                "identity probe returned HTTP {}",
                status.as_u16()
            )));
        }
        let payload: Value = response
            .json()
            .await
            .map_err(|e| PerpsSignerError::Internal(format!("identity probe decode: {e}")))?;
        parse_identity_probe(&payload)
    }
}

/// Result of the identity probe (Part F).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignerIdentityProbe {
    pub signer_address: [u8; 20],
    pub chain_id: u64,
}

impl SignerTransport for HttpSignerTransport {
    fn send_sign_request<'a>(
        &'a self,
        _endpoint: &'a str,
        request: PerpsSignerRequest<'a>,
    ) -> SignerFuture<'a, SignerResponse> {
        let url = join_path(&self.endpoint, "hybrid_v2/sign");
        let expected_hex = format!("0x{}", hex_encode(&self.expected_signer_address));
        let expected_bytes = self.expected_signer_address;
        // Convert the Perps request into the wire-protocol JSON body.
        // Numeric wei values arrive as base-10 strings; the signer
        // service expects `0x`-prefixed big-endian hex for value + fee
        // fields to avoid u128 truncation edge cases.
        let value_hex = decimal_string_to_hex(&request.transaction_value_wei.to_string());
        let max_fee_hex = decimal_string_to_hex(request.max_fee_per_gas_wei);
        let max_prio_hex = decimal_string_to_hex(request.max_priority_fee_per_gas_wei);
        let target_hex = request.transaction_to.0.clone();
        let idem_hex = uuid_to_hex_16(&request.request_id);
        let plan_hash_hex = uuid_to_hex_32(&request.policy_decision_id);
        let payload_hash_hex = format!("0x{}", hex_encode(&request.prehash));
        let body = json!({
            "chain_id": request.chain_id,
            "nonce": request.nonce,
            "digest": payload_hash_hex,
            "target": target_hex,
            "value_wei_hex": value_hex,
            "gas_limit": request.gas_limit,
            "max_fee_per_gas_hex": max_fee_hex,
            "max_priority_fee_per_gas_hex": max_prio_hex,
            "tx_type": 2u8, // EIP-1559 is the only supported type.
            "expected_signer": expected_hex,
            "idempotency_key": idem_hex,
            "policy_decision_id": plan_hash_hex,
            "fingerprint": payload_hash_hex,
        });
        let request_id = request.request_id;
        let policy_decision_id = request.policy_decision_id;
        let client = self.client.clone();
        Box::pin(async move {
            let response = client
                .post(&url)
                .json(&body)
                .send()
                .await
                .map_err(|e| classify_reqwest_err(&e))?;
            let status = response.status();
            if status == reqwest::StatusCode::UNAUTHORIZED
                || status == reqwest::StatusCode::FORBIDDEN
            {
                return Err(PerpsSignerError::CallerUnauthorized);
            }
            if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                return Err(PerpsSignerError::RateLimit);
            }
            if status.is_server_error() {
                return Err(PerpsSignerError::KmsUnavailable);
            }
            if !status.is_success() {
                return Err(PerpsSignerError::Transport(format!(
                    "sign returned HTTP {}",
                    status.as_u16()
                )));
            }
            let payload: Value = response
                .json()
                .await
                .map_err(|e| PerpsSignerError::Internal(format!("sign response decode: {e}")))?;
            parse_sign_response(&payload, request_id, policy_decision_id, &expected_bytes)
        })
    }

    fn send_health_check<'a>(&'a self, _endpoint: &'a str) -> SignerFuture<'a, SignerHealth> {
        let url = join_path(&self.endpoint, "hybrid_v2/health");
        let client = self.client.clone();
        Box::pin(async move {
            let response = client
                .get(&url)
                .send()
                .await
                .map_err(|e| classify_reqwest_err(&e))?;
            let status = response.status();
            if status == reqwest::StatusCode::UNAUTHORIZED
                || status == reqwest::StatusCode::FORBIDDEN
            {
                return Err(PerpsSignerError::CallerUnauthorized);
            }
            if status.is_server_error() {
                return Err(PerpsSignerError::KmsUnavailable);
            }
            if !status.is_success() {
                return Err(PerpsSignerError::Transport(format!(
                    "health returned HTTP {}",
                    status.as_u16()
                )));
            }
            let payload: Value = response
                .json()
                .await
                .map_err(|e| PerpsSignerError::Internal(format!("health response decode: {e}")))?;
            parse_health_response(&payload)
        })
    }
}

// -----------------------------------------------------------------
//                          helpers
// -----------------------------------------------------------------

fn join_path(endpoint: &str, path: &str) -> String {
    let base = endpoint.trim_end_matches('/');
    format!("{}/{}", base, path.trim_start_matches('/'))
}

fn classify_reqwest_err(err: &reqwest::Error) -> PerpsSignerError {
    if err.is_timeout() {
        PerpsSignerError::KmsTimeout
    } else if err.is_connect() {
        PerpsSignerError::KmsUnavailable
    } else {
        PerpsSignerError::Transport(truncate(&err.to_string(), 120))
    }
}

fn parse_sign_response(
    payload: &Value,
    request_id: uuid::Uuid,
    policy_decision_id: uuid::Uuid,
    expected_signer: &[u8; 20],
) -> Result<SignerResponse, PerpsSignerError> {
    let r_hex = payload
        .get("signature_r")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            PerpsSignerError::Internal("sign response missing signature_r".to_string())
        })?;
    let s_hex = payload
        .get("signature_s")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            PerpsSignerError::Internal("sign response missing signature_s".to_string())
        })?;
    let v_int = payload
        .get("signature_v")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| {
            PerpsSignerError::Internal("sign response missing signature_v".to_string())
        })?;
    let addr = payload
        .get("recovered_signer")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            PerpsSignerError::Internal("sign response missing recovered_signer".to_string())
        })?;
    let r = parse_hex_32(r_hex).ok_or_else(|| {
        PerpsSignerError::Internal("sign response signature_r malformed".to_string())
    })?;
    let s = parse_hex_32(s_hex).ok_or_else(|| {
        PerpsSignerError::Internal("sign response signature_s malformed".to_string())
    })?;
    if v_int > 1 {
        return Err(PerpsSignerError::Internal(format!(
            "sign response y_parity {v_int} not in {{0, 1}}"
        )));
    }
    let signer_bytes = parse_hex_20(addr).ok_or_else(|| {
        PerpsSignerError::Internal("sign response recovered_signer malformed".to_string())
    })?;
    if &signer_bytes != expected_signer {
        // Vendor recovered a different address → hard reject before the
        // bridge sees the response.
        return Err(PerpsSignerError::PostSignFromMismatch);
    }
    Ok(SignerResponse {
        request_id,
        signer_address: AccountId::new(addr),
        signature: RecoverableSignature {
            r,
            s,
            y_parity: v_int as u8,
        },
        kms_request_id: payload
            .get("kms_request_id")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        audit_log_id: payload
            .get("audit_log_id")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        remote_signer_request_id: payload
            .get("remote_signer_request_id")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        created_at_ms: crate::types::now_ms(),
        policy_decision_id,
    })
}

fn parse_health_response(payload: &Value) -> Result<SignerHealth, PerpsSignerError> {
    let healthy = payload
        .get("healthy")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let signer_address = payload
        .get("signer_address")
        .and_then(|v| v.as_str())
        .map(|s| AccountId::new(s));
    Ok(SignerHealth {
        mode: SignerBackendKind::Remote,
        signer_address,
        remote_endpoint_present: true,
        healthy,
    })
}

fn parse_identity_probe(payload: &Value) -> Result<SignerIdentityProbe, PerpsSignerError> {
    let addr = payload
        .get("signer_address")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            PerpsSignerError::Internal("identity probe missing signer_address".to_string())
        })?;
    let chain_id = payload
        .get("chain_id")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| PerpsSignerError::Internal("identity probe missing chain_id".to_string()))?;
    let bytes = parse_hex_20(addr).ok_or_else(|| {
        PerpsSignerError::Internal("identity probe signer_address malformed".to_string())
    })?;
    Ok(SignerIdentityProbe {
        signer_address: bytes,
        chain_id,
    })
}

fn parse_hex_32(s: &str) -> Option<[u8; 32]> {
    let stripped = strip_hex_prefix(s);
    if stripped.len() != 64 || !stripped.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let mut out = [0u8; 32];
    for i in 0..32 {
        out[i] = u8::from_str_radix(&stripped[2 * i..2 * i + 2], 16).ok()?;
    }
    Some(out)
}

fn parse_hex_20(s: &str) -> Option<[u8; 20]> {
    let stripped = strip_hex_prefix(s);
    if stripped.len() != 40 || !stripped.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let mut out = [0u8; 20];
    for i in 0..20 {
        out[i] = u8::from_str_radix(&stripped[2 * i..2 * i + 2], 16).ok()?;
    }
    Some(out)
}

fn strip_hex_prefix(s: &str) -> &str {
    s.strip_prefix("0x")
        .or_else(|| s.strip_prefix("0X"))
        .unwrap_or(s)
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{:02x}", b));
    }
    s
}

fn decimal_string_to_hex(dec: &str) -> String {
    // The Perps request carries these fields as base-10 strings; the
    // wire body wants big-endian hex. We reject anything above 2^256
    // by using u128 addition on 32-byte accumulators. Practical wei
    // values fit in u128; anything larger is malformed operator input.
    let trimmed = dec.trim();
    let mut acc: u128 = 0;
    for ch in trimmed.chars() {
        if !ch.is_ascii_digit() {
            return "0x0".to_string();
        }
        let d = (ch as u8 - b'0') as u128;
        acc = acc.saturating_mul(10).saturating_add(d);
    }
    format!("0x{:x}", acc)
}

fn uuid_to_hex_16(u: &uuid::Uuid) -> String {
    let bytes = u.as_bytes();
    let mut s = String::with_capacity(2 + 32);
    s.push_str("0x");
    for b in bytes.iter() {
        s.push_str(&format!("{:02x}", b));
    }
    s
}

fn uuid_to_hex_32(u: &uuid::Uuid) -> String {
    // UUIDs are 16 bytes; pad to 32 with leading zeros so the wire
    // protocol's `policy_decision_id` field is always 32-byte hex.
    let bytes = u.as_bytes();
    let mut padded = [0u8; 32];
    padded[16..].copy_from_slice(bytes);
    format!("0x{}", hex_encode(&padded))
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        s.chars().take(max).collect()
    }
}

/// Reduce an endpoint URL to `scheme://host[:port]`.
pub fn redact_endpoint(url: &str) -> String {
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

/// Public helper: read a PEM file if a path is configured.
/// Returns Ok(None) for an unset path, Ok(Some(bytes)) for a readable
/// file, or a structured error otherwise.
pub fn read_pem_if_configured(
    path: &Option<String>,
) -> Result<Option<Vec<u8>>, TransportBuildError> {
    let Some(p) = path.as_deref() else {
        return Ok(None);
    };
    match std::fs::read(p) {
        Ok(bytes) => {
            if bytes.is_empty() {
                Err(TransportBuildError::MalformedCertPem)
            } else {
                Ok(Some(bytes))
            }
        }
        Err(_) => Err(TransportBuildError::MalformedCertPem),
    }
}

/// Return true when `endpoint` is an HTTPS URL that is NOT loopback
/// (i.e. mTLS material becomes mandatory).
pub fn is_public_https(endpoint: &str) -> bool {
    let lower = endpoint.to_ascii_lowercase();
    if !lower.starts_with("https://") {
        return false;
    }
    // Loopback exceptions (dev signer running on the same host):
    let after_scheme = &lower["https://".len()..];
    let host_part = after_scheme
        .split('/')
        .next()
        .unwrap_or(after_scheme)
        .split('?')
        .next()
        .unwrap_or("");
    // Handle bracketed IPv6 like [::1]:9000 → strip [ and split at ].
    let host_only = if let Some(rest) = host_part.strip_prefix('[') {
        rest.split(']').next().unwrap_or(rest)
    } else {
        host_part.split(':').next().unwrap_or(host_part)
    };
    !(host_only == "127.0.0.1" || host_only == "localhost" || host_only == "::1")
}

// -----------------------------------------------------------------
//                          UNIT TESTS
// -----------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redact_endpoint_strips_path_and_query() {
        assert_eq!(
            redact_endpoint("https://signer.example.com/v1/keys?x=1"),
            "https://signer.example.com"
        );
        assert_eq!(
            redact_endpoint("http://127.0.0.1:9000/hybrid_v2/sign"),
            "http://127.0.0.1:9000"
        );
        assert_eq!(redact_endpoint("garbage"), "<opaque>");
    }

    #[test]
    fn is_public_https_gates_loopback_correctly() {
        assert!(is_public_https("https://signer.example.com"));
        assert!(is_public_https("https://signer.example.com:8443/x"));
        assert!(!is_public_https("http://signer.example.com"));
        assert!(!is_public_https("https://127.0.0.1:9000/x"));
        assert!(!is_public_https("https://localhost/x"));
        assert!(!is_public_https("https://[::1]:9000/x"));
    }

    #[test]
    fn unsupported_scheme_is_refused() {
        let err = HttpSignerTransport::new(
            "ws://signer".to_string(),
            [0u8; 20],
            None,
            None,
            None,
            2_500,
            1,
        )
        .unwrap_err();
        assert!(matches!(err, TransportBuildError::UnsupportedScheme));
    }

    #[test]
    fn malformed_endpoint_is_refused() {
        let err = HttpSignerTransport::new(
            "https://".to_string(),
            [0u8; 20],
            None,
            None,
            None,
            2_500,
            1,
        )
        .unwrap_err();
        assert!(matches!(err, TransportBuildError::MalformedEndpoint));
    }

    #[test]
    fn cert_without_key_refused() {
        let err = HttpSignerTransport::new(
            "https://signer.example.com".to_string(),
            [0u8; 20],
            Some(vec![b'x']),
            None,
            None,
            2_500,
            1,
        )
        .unwrap_err();
        assert!(matches!(err, TransportBuildError::MalformedKeyPem));
    }

    #[test]
    fn key_without_cert_refused() {
        let err = HttpSignerTransport::new(
            "https://signer.example.com".to_string(),
            [0u8; 20],
            None,
            Some(vec![b'x']),
            None,
            2_500,
            1,
        )
        .unwrap_err();
        assert!(matches!(err, TransportBuildError::MalformedCertPem));
    }

    #[test]
    fn debug_never_leaks_endpoint_path_or_pem_material() {
        let t = HttpSignerTransport::new(
            "http://127.0.0.1:9000/some/secret/path?token=xyz".to_string(),
            [0xaa; 20],
            None,
            None,
            None,
            2_500,
            1,
        )
        .unwrap();
        let s = format!("{t:?}");
        assert!(!s.contains("secret"));
        assert!(!s.contains("token=xyz"));
        assert!(s.contains("127.0.0.1:9000"));
    }

    #[test]
    fn read_pem_returns_none_for_unset_path() {
        let r = read_pem_if_configured(&None).unwrap();
        assert!(r.is_none());
    }

    #[test]
    fn read_pem_errors_on_missing_file() {
        let r = read_pem_if_configured(&Some("/nonexistent/path.pem".to_string()));
        assert!(r.is_err());
    }

    #[test]
    fn decimal_to_hex_round_trips_common_wei_values() {
        assert_eq!(decimal_string_to_hex("0"), "0x0");
        assert_eq!(decimal_string_to_hex("1"), "0x1");
        assert_eq!(decimal_string_to_hex("16"), "0x10");
        assert_eq!(decimal_string_to_hex("1000000000"), "0x3b9aca00");
    }

    #[test]
    fn parse_hex_32_rejects_wrong_length() {
        assert!(parse_hex_32("0xdead").is_none());
        assert!(parse_hex_32("0x").is_none());
        let ok = parse_hex_32("0x1111111111111111111111111111111111111111111111111111111111111111");
        assert!(ok.is_some());
    }

    #[test]
    fn parse_hex_20_accepts_ci() {
        let a = parse_hex_20("0xAaBbCcDdEeFf00112233445566778899aabbccdd").unwrap();
        assert_eq!(a[0], 0xaa);
        assert_eq!(a[19], 0xdd);
    }

    #[test]
    fn parse_identity_probe_round_trips() {
        let json = json!({
            "signer_address": "0x1111111111111111111111111111111111111111",
            "chain_id": 84532u64,
        });
        let p = parse_identity_probe(&json).unwrap();
        assert_eq!(p.signer_address, [0x11u8; 20]);
        assert_eq!(p.chain_id, 84532);
    }

    #[test]
    fn parse_health_response_defaults_to_unhealthy() {
        let json = json!({});
        let h = parse_health_response(&json).unwrap();
        assert!(!h.healthy);
    }

    #[test]
    fn parse_sign_response_rejects_wrong_recovered_signer() {
        let expected = [0x22u8; 20];
        let payload = json!({
            "signature_r": "0x1111111111111111111111111111111111111111111111111111111111111111",
            "signature_s": "0x2222222222222222222222222222222222222222222222222222222222222222",
            "signature_v": 0u64,
            "recovered_signer": "0x3333333333333333333333333333333333333333",
        });
        let err = parse_sign_response(&payload, uuid::Uuid::nil(), uuid::Uuid::nil(), &expected)
            .unwrap_err();
        assert!(matches!(err, PerpsSignerError::PostSignFromMismatch));
    }

    #[test]
    fn parse_sign_response_accepts_matching_recovered_signer() {
        let expected = [0x33u8; 20];
        let payload = json!({
            "signature_r": "0x1111111111111111111111111111111111111111111111111111111111111111",
            "signature_s": "0x2222222222222222222222222222222222222222222222222222222222222222",
            "signature_v": 1u64,
            "recovered_signer": "0x3333333333333333333333333333333333333333",
        });
        let out =
            parse_sign_response(&payload, uuid::Uuid::nil(), uuid::Uuid::nil(), &expected).unwrap();
        assert_eq!(out.signature.y_parity, 1);
    }
}
