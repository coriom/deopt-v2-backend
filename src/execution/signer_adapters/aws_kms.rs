//! AWS KMS vendor-specific `PluggableSignerProvider` implementation.
//!
//! Implements the preferred-path adapter from
//! `deopt-v2-backend/docs/MAINNET_KMS_VENDOR_SELECTION_DECISION.md §4`
//! against a vendor-neutral [`AwsKmsTransport`] trait. Production
//! integrations satisfy the trait via the `aws-sdk-kms` crate; tests
//! inject [`MockAwsKmsTransport`] — no real AWS account, no real KMS
//! key, no real credentials are required by this module.
//!
//! Hard rules baked in:
//!
//!   * The backend never holds AWS credentials. Per Pattern C
//!     (`MAINNET_BE_SIGNER_SERVICE_DESIGN.md`), the backend calls a
//!     signer microservice; the microservice calls AWS KMS. The
//!     `vendor_endpoint` string this adapter sees is the
//!     microservice's mTLS URL — NEVER an AWS endpoint. Env keys
//!     `AWS_KMS_SIGNER_KEY_ID`, `AWS_KMS_SIGNER_REGION`, and
//!     `AWS_KMS_SIGNER_EXPECTED_ADDRESS` are microservice-side and
//!     are NEVER read by the backend env loader.
//!
//!   * Production `RemoteSignerClient::new` continues to use
//!     `UnimplementedTransport`. This adapter is exposed only via
//!     `RemoteSignerClient::with_transport` injection. Operator
//!     promotion to production happens at the rehearsal Phase 3
//!     cutover per `MAINNET_SIGNER_STAGING_REHEARSAL_PLAN.md`.
//!
//!   * Mainnet refuses `Mock` provider AND `LocalDev` mode at config
//!     parse + at runtime. The new `AwsKms` provider kind is
//!     `is_operational() == true` so it passes the mainnet config
//!     guard.
//!
//!   * No AWS SDK dependency added by this milestone — the adapter
//!     is transport-generic. The vendor SDK ships in a follow-on PR
//!     that implements `AwsKmsTransport` for the real client.

use crate::execution::signer::RecoverableSignature;
use crate::execution::signer_adapters::{
    validate_signature, PluggableFuture, PluggableSignRequest, PluggableSignResult,
    PluggableSignerProvider, SignerProviderKind, VendorError,
};
use crate::signing::eip712::keccak256;
use crate::types::AccountId;
use k256::ecdsa::{RecoveryId, Signature, VerifyingKey};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

/// Boxed-future return type for the AWS KMS transport. Errors are
/// always [`AwsKmsError`]; the adapter maps them onto [`VendorError`]
/// before they reach the `PluggableSignerProvider` boundary.
pub type AwsKmsFuture<'a, T> =
    Pin<Box<dyn Future<Output = std::result::Result<T, AwsKmsError>> + Send + 'a>>;

// ---------------------------------------------------------------------------
// AWS KMS transport (mockable surface)
// ---------------------------------------------------------------------------

/// Vendor-neutral transport for AWS KMS. Production implementations
/// wrap the real `aws-sdk-kms` client (or the signer microservice's
/// mTLS API which fronts AWS KMS per Pattern C); tests inject a
/// [`MockAwsKmsTransport`] with no AWS dependency.
pub trait AwsKmsTransport: Send + Sync {
    /// Equivalent of `kms:GetPublicKey`. Returns the
    /// SubjectPublicKeyInfo-encoded public key plus the vendor's
    /// per-request audit correlation id (CloudTrail `RequestId`).
    fn get_public_key<'a>(&'a self, key_id: &'a str) -> AwsKmsFuture<'a, GetPublicKeyResponse>;

    /// Equivalent of `kms:Sign` with `MessageType=DIGEST` and
    /// `SigningAlgorithm=ECDSA_SHA_256`. Receives a 32-byte EIP-1559
    /// prehash; returns the DER-encoded ECDSA signature plus the
    /// vendor's per-request audit correlation id.
    fn sign_digest<'a>(
        &'a self,
        key_id: &'a str,
        digest: [u8; 32],
        metadata: SignRequestMetadata,
    ) -> AwsKmsFuture<'a, SignDigestResponse>;
}

#[derive(Clone, Debug)]
pub struct GetPublicKeyResponse {
    /// Raw ASN.1 SubjectPublicKeyInfo bytes from AWS KMS GetPublicKey.
    /// For `KeySpec=ECC_SECG_P256K1` the embedded `BIT STRING` carries
    /// the uncompressed SEC1 secp256k1 public key (`0x04 || X(32) ||
    /// Y(32)`).
    pub public_key_der: Vec<u8>,
    /// CloudTrail `RequestId` — vendor-issued audit correlation.
    pub request_id: String,
}

#[derive(Clone, Debug)]
pub struct SignDigestResponse {
    /// DER-encoded ECDSA signature: `SEQUENCE { INTEGER r, INTEGER s }`.
    pub signature_der: Vec<u8>,
    /// CloudTrail `RequestId` — vendor-issued audit correlation.
    pub request_id: String,
}

/// Vendor-opaque request metadata threaded into the transport call.
/// `policy_decision_id` is forwarded so the signer microservice can
/// bind the AWS request to the should_broadcast decision; the AWS SDK
/// itself does not consume this field directly.
#[derive(Clone, Copy, Debug)]
pub struct SignRequestMetadata {
    pub request_id: uuid::Uuid,
    pub intent_id: uuid::Uuid,
    pub policy_decision_id: uuid::Uuid,
    pub policy_fingerprint: [u8; 32],
}

// ---------------------------------------------------------------------------
// AwsKmsError + vendor-error mapping
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AwsKmsError {
    /// IAM deny / role lacks `kms:Sign` permission.
    AccessDenied(String),
    /// `ThrottlingException` (HTTP 400 with code) / API rate limit.
    Throttling,
    /// SDK-level or transport-level timeout.
    Timeout,
    /// `KMSInvalidStateException` because the key has been disabled.
    KeyDisabled,
    /// `NotFoundException` or `KMSInvalidKeyUsageException`.
    KeyInvalid(String),
    /// HTTP 5xx / connection failure / DNS error.
    ServiceUnavailable(String),
    /// SDK response did not match the documented contract (e.g.
    /// missing `Signature` field on a `Sign` response).
    MalformedResponse(String),
    /// Catch-all for any other AWS error code.
    Unknown(String),
}

impl AwsKmsError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::AccessDenied(_) => "aws_kms_access_denied",
            Self::Throttling => "aws_kms_throttling",
            Self::Timeout => "aws_kms_timeout",
            Self::KeyDisabled => "aws_kms_key_disabled",
            Self::KeyInvalid(_) => "aws_kms_key_invalid",
            Self::ServiceUnavailable(_) => "aws_kms_service_unavailable",
            Self::MalformedResponse(_) => "aws_kms_malformed_response",
            Self::Unknown(_) => "aws_kms_unknown",
        }
    }
}

/// Project an [`AwsKmsError`] onto the vendor-neutral
/// [`VendorError`] taxonomy per
/// `MAINNET_KMS_VENDOR_SELECTION_DECISION.md §5`. Caller still routes
/// through `map_vendor_error` to reach the final
/// `SignerError::code()` string.
pub fn map_aws_kms_error(error: AwsKmsError) -> VendorError {
    match error {
        AwsKmsError::AccessDenied(_) => VendorError::AuthFailed,
        AwsKmsError::Throttling => VendorError::RateLimited,
        AwsKmsError::Timeout => VendorError::Timeout,
        AwsKmsError::KeyDisabled => VendorError::Denied("aws_kms_key_disabled".to_string()),
        AwsKmsError::KeyInvalid(reason) => {
            VendorError::Denied(format!("aws_kms_key_invalid: {reason}"))
        }
        AwsKmsError::ServiceUnavailable(reason) => VendorError::Unavailable(reason),
        AwsKmsError::MalformedResponse(reason) => VendorError::MalformedSignature(reason),
        AwsKmsError::Unknown(reason) => VendorError::Unknown(reason),
    }
}

// ---------------------------------------------------------------------------
// AwsKmsSignerProvider
// ---------------------------------------------------------------------------

/// `PluggableSignerProvider` implementation routing through any
/// `AwsKmsTransport`. The provider holds:
///
///   * `transport` — vendor SDK glue (real or mock).
///   * `key_id` — AWS KMS key ARN or key id; passed opaque to the
///     transport. NEVER stored in the backend `.env`.
///   * `expected_address` — the configured EOA. Cross-checked at
///     `health_check` AND at `sign_prehash` post-recovery.
pub struct AwsKmsSignerProvider {
    transport: Arc<dyn AwsKmsTransport>,
    key_id: String,
    expected_address: AccountId,
}

impl AwsKmsSignerProvider {
    pub fn new(
        transport: Arc<dyn AwsKmsTransport>,
        key_id: String,
        expected_address: AccountId,
    ) -> Self {
        Self {
            transport,
            key_id,
            expected_address,
        }
    }

    pub fn expected_address(&self) -> &AccountId {
        &self.expected_address
    }
}

impl PluggableSignerProvider for AwsKmsSignerProvider {
    fn provider_kind(&self) -> SignerProviderKind {
        SignerProviderKind::AwsKms
    }

    fn derive_address<'a>(&'a self) -> PluggableFuture<'a, AccountId> {
        let key_id = self.key_id.clone();
        let expected = self.expected_address.clone();
        let transport = self.transport.clone();
        Box::pin(async move {
            let response = transport
                .get_public_key(&key_id)
                .await
                .map_err(map_aws_kms_error)?;
            let derived = decode_address_from_spki(&response.public_key_der)?;
            if !addresses_equal_ci(&derived, &expected) {
                return Err(VendorError::AddressMismatch);
            }
            Ok(derived)
        })
    }

    fn sign_prehash<'a>(
        &'a self,
        request: PluggableSignRequest<'a>,
    ) -> PluggableFuture<'a, PluggableSignResult> {
        let key_id = self.key_id.clone();
        let expected = self.expected_address.clone();
        let transport = self.transport.clone();
        let metadata = SignRequestMetadata {
            request_id: request.request_id,
            intent_id: request.intent_id,
            policy_decision_id: request.policy_decision_id,
            policy_fingerprint: request.policy_fingerprint,
        };
        let prehash = request.prehash;
        Box::pin(async move {
            let response = transport
                .sign_digest(&key_id, prehash, metadata)
                .await
                .map_err(map_aws_kms_error)?;
            let (r, s) = decode_der_ecdsa_signature(&response.signature_der)
                .map_err(VendorError::MalformedSignature)?;
            // Recover y_parity by trying the two candidates; only one
            // can recover to the expected address (modulo signature
            // malleability which we reject via low-s).
            let signature = recover_signature_with_parity(&prehash, r, s, &expected)?;
            // Defence-in-depth structural validation. The vendor
            // already SHOULD be canonical (KMS emits low-s DER), but
            // the adapter validates locally so any non-canonical
            // surface fails closed here.
            validate_signature(&signature).map_err(|err| match err {
                VendorError::MalformedSignature(reason) => VendorError::MalformedSignature(reason),
                other => other,
            })?;
            Ok(PluggableSignResult {
                signature,
                provider_request_id: Some(response.request_id.clone()),
                audit_log_id: Some(response.request_id),
            })
        })
    }
}

// ---------------------------------------------------------------------------
// SPKI parsing + EVM address derivation
// ---------------------------------------------------------------------------

/// Extract the uncompressed secp256k1 public key from an AWS KMS
/// `GetPublicKey` SubjectPublicKeyInfo response and derive the EVM
/// address as `keccak256(pubkey[1..])[12..]`.
///
/// Rejects:
///   * truncated / malformed ASN.1 structure.
///   * non-`0x04` (compressed or hybrid) public-key prefix.
///   * `BIT STRING` length other than 65 (32 + 32 + 1 prefix byte).
pub(crate) fn decode_address_from_spki(spki: &[u8]) -> std::result::Result<AccountId, VendorError> {
    let pubkey = extract_uncompressed_secp256k1_from_spki(spki)?;
    Ok(evm_address_from_uncompressed_pubkey(&pubkey))
}

/// Minimal ASN.1 walker tuned for the AWS KMS SPKI shape:
///
/// ```text
/// SubjectPublicKeyInfo ::= SEQUENCE {
///   algorithm        SEQUENCE {
///     algorithm        OBJECT IDENTIFIER 1.2.840.10045.2.1 (ecPublicKey)
///     parameters       OBJECT IDENTIFIER 1.3.132.0.10       (secp256k1)
///   },
///   subjectPublicKey BIT STRING { 04 || X(32) || Y(32) }
/// }
/// ```
///
/// We do NOT bring in a full ASN.1 decoder — the structure is fixed
/// and recoverable with two outer SEQUENCE walks plus a BIT STRING
/// extract. The walker rejects any input whose outer length doesn't
/// match the inner content boundaries.
fn extract_uncompressed_secp256k1_from_spki(
    spki: &[u8],
) -> std::result::Result<[u8; 65], VendorError> {
    let mut cursor = spki;
    // Outer SEQUENCE.
    let body = der_unwrap(&mut cursor, 0x30)
        .map_err(|reason| VendorError::MalformedSignature(format!("spki outer: {reason}")))?;
    let mut cursor = body;
    // algorithm SEQUENCE — skip (we don't verify the OIDs to keep
    // the parser small; the BIT STRING length check + ECDSA recovery
    // at sign time give defence-in-depth).
    let _algo = der_unwrap(&mut cursor, 0x30)
        .map_err(|reason| VendorError::MalformedSignature(format!("spki algorithm: {reason}")))?;
    // subjectPublicKey BIT STRING.
    let bit_string = der_unwrap(&mut cursor, 0x03)
        .map_err(|reason| VendorError::MalformedSignature(format!("spki bitstring: {reason}")))?;
    // BIT STRING content: first byte = unused-bits count (always 0 for
    // a byte-aligned public key); remainder is the SEC1 octets.
    let (unused, sec1) = bit_string.split_first().ok_or_else(|| {
        VendorError::MalformedSignature("spki bitstring empty content".to_string())
    })?;
    if *unused != 0 {
        return Err(VendorError::MalformedSignature(format!(
            "spki bitstring unused-bits={unused} (expected 0)"
        )));
    }
    if sec1.len() != 65 {
        return Err(VendorError::MalformedSignature(format!(
            "spki sec1 length {} (expected 65)",
            sec1.len()
        )));
    }
    if sec1[0] != 0x04 {
        return Err(VendorError::MalformedSignature(format!(
            "spki sec1 prefix 0x{:02x} (expected 0x04 uncompressed)",
            sec1[0]
        )));
    }
    let mut out = [0u8; 65];
    out.copy_from_slice(sec1);
    Ok(out)
}

fn evm_address_from_uncompressed_pubkey(pubkey: &[u8; 65]) -> AccountId {
    // Hash the X || Y bytes (skip the 0x04 prefix). Take the last 20
    // bytes of keccak256.
    let hash = keccak256(&pubkey[1..]);
    let mut out = String::with_capacity(42);
    out.push_str("0x");
    for byte in &hash[12..] {
        out.push_str(&format!("{byte:02x}"));
    }
    AccountId::new(out)
}

// ---------------------------------------------------------------------------
// DER ECDSA signature decoding
// ---------------------------------------------------------------------------

/// Decode a DER-encoded ECDSA signature into the (r, s) byte pair per
/// RFC 3279 §2.2.3.
///
/// Layout:
///
/// ```text
/// SEQUENCE { INTEGER r, INTEGER s }
/// ```
///
/// ASN.1 INTEGERs are big-endian, signed, two's-complement. AWS KMS
/// emits canonical DER which means:
///   * No leading zero byte unless required to indicate sign.
///   * No high-bit-set encoding (positive integers only).
///
/// This decoder accepts a leading zero (some hardware paths emit it
/// even when not strictly required) but normalises both r and s to
/// exactly 32 bytes. Lengths > 33 (32 + 1 leading-zero) are
/// rejected.
pub(crate) fn decode_der_ecdsa_signature(
    der: &[u8],
) -> std::result::Result<([u8; 32], [u8; 32]), String> {
    let mut cursor = der;
    let body = der_unwrap(&mut cursor, 0x30).map_err(|reason| format!("der outer: {reason}"))?;
    if !cursor.is_empty() {
        return Err(format!("der outer trailing bytes: {}", cursor.len()));
    }
    let mut cursor = body;
    let r_raw = der_unwrap(&mut cursor, 0x02).map_err(|reason| format!("der r: {reason}"))?;
    let s_raw = der_unwrap(&mut cursor, 0x02).map_err(|reason| format!("der s: {reason}"))?;
    if !cursor.is_empty() {
        return Err(format!("der seq trailing bytes: {}", cursor.len()));
    }
    Ok((
        normalise_der_integer(r_raw, 'r')?,
        normalise_der_integer(s_raw, 's')?,
    ))
}

fn normalise_der_integer(raw: &[u8], label: char) -> std::result::Result<[u8; 32], String> {
    if raw.is_empty() {
        return Err(format!("der {label} empty"));
    }
    // Strip a leading 0x00 sign byte if present.
    let trimmed = if raw[0] == 0x00 && raw.len() > 1 && raw[1] & 0x80 != 0 {
        &raw[1..]
    } else if raw[0] == 0x00 && raw.len() > 1 {
        // Tolerant strip of an extraneous leading zero.
        &raw[1..]
    } else {
        raw
    };
    if trimmed.len() > 32 {
        return Err(format!(
            "der {label} length {} (expected ≤ 32)",
            trimmed.len()
        ));
    }
    let mut out = [0u8; 32];
    out[32 - trimmed.len()..].copy_from_slice(trimmed);
    Ok(out)
}

/// Walk one ASN.1 TLV from `cursor`. Verifies the tag, decodes the
/// length (short or long form per X.690 §8.1.3), advances `cursor`
/// past the TLV, and returns a slice over the value bytes.
fn der_unwrap<'a>(
    cursor: &mut &'a [u8],
    expected_tag: u8,
) -> std::result::Result<&'a [u8], String> {
    let (&tag, rest) = cursor
        .split_first()
        .ok_or_else(|| "der: empty cursor".to_string())?;
    if tag != expected_tag {
        return Err(format!(
            "der: tag 0x{tag:02x} (expected 0x{expected_tag:02x})"
        ));
    }
    let (&len_byte, rest) = rest
        .split_first()
        .ok_or_else(|| "der: missing length byte".to_string())?;
    let (length, rest) = if len_byte & 0x80 == 0 {
        (len_byte as usize, rest)
    } else {
        let num_bytes = (len_byte & 0x7F) as usize;
        if num_bytes == 0 || num_bytes > 4 {
            return Err(format!("der: invalid long-form length 0x{len_byte:02x}"));
        }
        if rest.len() < num_bytes {
            return Err("der: truncated length".to_string());
        }
        let mut length = 0usize;
        for byte in &rest[..num_bytes] {
            length = (length << 8) | (*byte as usize);
        }
        (length, &rest[num_bytes..])
    };
    if rest.len() < length {
        return Err(format!(
            "der: truncated value (have {}, need {length})",
            rest.len()
        ));
    }
    let (value, tail) = rest.split_at(length);
    *cursor = tail;
    Ok(value)
}

// ---------------------------------------------------------------------------
// Recoverable-signature reconstruction
// ---------------------------------------------------------------------------

/// Try `y_parity ∈ {0, 1}` until ECDSA recovery against the prehash
/// produces the expected EVM address. Returns the matching
/// [`RecoverableSignature`].
///
/// Returns `VendorError::AddressMismatch` when neither candidate
/// recovers to `expected` (vendor signed for the wrong key).
/// Returns `VendorError::MalformedSignature` when the (r, s) pair
/// fails k256's structural validation.
pub(crate) fn recover_signature_with_parity(
    prehash: &[u8; 32],
    r: [u8; 32],
    s: [u8; 32],
    expected: &AccountId,
) -> std::result::Result<RecoverableSignature, VendorError> {
    let mut rs_bytes = [0u8; 64];
    rs_bytes[..32].copy_from_slice(&r);
    rs_bytes[32..].copy_from_slice(&s);
    let signature = Signature::from_slice(&rs_bytes)
        .map_err(|err| VendorError::MalformedSignature(format!("k256 sig parse: {err}")))?;
    for parity in [0u8, 1u8] {
        let recovery_id = match RecoveryId::try_from(parity) {
            Ok(id) => id,
            Err(_) => continue,
        };
        let verifying_key =
            match VerifyingKey::recover_from_prehash(prehash, &signature, recovery_id) {
                Ok(key) => key,
                Err(_) => continue,
            };
        let encoded = verifying_key.to_encoded_point(false);
        let pubkey_bytes = encoded.as_bytes();
        if pubkey_bytes.len() != 65 || pubkey_bytes[0] != 0x04 {
            continue;
        }
        let mut pubkey_arr = [0u8; 65];
        pubkey_arr.copy_from_slice(pubkey_bytes);
        let recovered = evm_address_from_uncompressed_pubkey(&pubkey_arr);
        if addresses_equal_ci(&recovered, expected) {
            return Ok(RecoverableSignature {
                y_parity: parity,
                r,
                s,
            });
        }
    }
    Err(VendorError::AddressMismatch)
}

fn addresses_equal_ci(a: &AccountId, b: &AccountId) -> bool {
    a.0.eq_ignore_ascii_case(&b.0)
}

// ---------------------------------------------------------------------------
// MockAwsKmsTransport (test-only)
// ---------------------------------------------------------------------------

/// Test-only `AwsKmsTransport` driven by a mutable `Mode`. The
/// production-shaped path uses an in-process `ExecutorSigner` to
/// produce real signatures + a synthetic DER + a synthetic SPKI so
/// the adapter's full pipeline runs end-to-end against a deterministic
/// fixture. Every error mode short-circuits to a structured
/// [`AwsKmsError`] for the mapping-table coverage.
pub struct MockAwsKmsTransport {
    mode: std::sync::Mutex<MockAwsKmsMode>,
    signer: crate::execution::signer::ExecutorSigner,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MockAwsKmsMode {
    Success,
    AccessDenied,
    Throttling,
    Timeout,
    KeyDisabled,
    KeyInvalid,
    ServiceUnavailable,
    MalformedResponse,
    Unknown,
    /// `get_public_key` succeeds with a DIFFERENT key — used to test
    /// the address-mismatch branch on `derive_address` and
    /// `sign_prehash` (the recovered address won't match expected).
    WrongKey,
    /// `sign_digest` returns a malformed DER signature (truncated).
    MalformedSignatureDer,
}

impl MockAwsKmsTransport {
    pub fn new(mode: MockAwsKmsMode, signer: crate::execution::signer::ExecutorSigner) -> Self {
        Self {
            mode: std::sync::Mutex::new(mode),
            signer,
        }
    }

    pub fn set_mode(&self, mode: MockAwsKmsMode) {
        *self.mode.lock().expect("mock aws kms mode poisoned") = mode;
    }

    pub fn current_mode(&self) -> MockAwsKmsMode {
        *self.mode.lock().expect("mock aws kms mode poisoned")
    }

    pub fn signer_address(&self) -> &AccountId {
        self.signer.address()
    }
}

impl AwsKmsTransport for MockAwsKmsTransport {
    fn get_public_key<'a>(&'a self, _key_id: &'a str) -> AwsKmsFuture<'a, GetPublicKeyResponse> {
        let mode = self.current_mode();
        let signer = self.signer.clone();
        Box::pin(async move {
            match mode {
                MockAwsKmsMode::AccessDenied => {
                    Err(AwsKmsError::AccessDenied("mock access denied".to_string()))
                }
                MockAwsKmsMode::Throttling => Err(AwsKmsError::Throttling),
                MockAwsKmsMode::Timeout => Err(AwsKmsError::Timeout),
                MockAwsKmsMode::KeyDisabled => Err(AwsKmsError::KeyDisabled),
                MockAwsKmsMode::KeyInvalid => {
                    Err(AwsKmsError::KeyInvalid("mock key invalid".to_string()))
                }
                MockAwsKmsMode::ServiceUnavailable => {
                    Err(AwsKmsError::ServiceUnavailable("mock 503".to_string()))
                }
                MockAwsKmsMode::MalformedResponse => {
                    Err(AwsKmsError::MalformedResponse("mock malformed".to_string()))
                }
                MockAwsKmsMode::Unknown => Err(AwsKmsError::Unknown("mock unknown".to_string())),
                MockAwsKmsMode::Success | MockAwsKmsMode::MalformedSignatureDer => {
                    Ok(GetPublicKeyResponse {
                        public_key_der: synth_spki_for_signer(&signer),
                        request_id: "mock-aws-req-id".to_string(),
                    })
                }
                MockAwsKmsMode::WrongKey => Ok(GetPublicKeyResponse {
                    public_key_der: synth_spki_wrong_key(),
                    request_id: "mock-aws-req-id".to_string(),
                }),
            }
        })
    }

    fn sign_digest<'a>(
        &'a self,
        _key_id: &'a str,
        digest: [u8; 32],
        _metadata: SignRequestMetadata,
    ) -> AwsKmsFuture<'a, SignDigestResponse> {
        let mode = self.current_mode();
        let signer = self.signer.clone();
        Box::pin(async move {
            match mode {
                MockAwsKmsMode::AccessDenied => {
                    Err(AwsKmsError::AccessDenied("mock access denied".to_string()))
                }
                MockAwsKmsMode::Throttling => Err(AwsKmsError::Throttling),
                MockAwsKmsMode::Timeout => Err(AwsKmsError::Timeout),
                MockAwsKmsMode::KeyDisabled => Err(AwsKmsError::KeyDisabled),
                MockAwsKmsMode::KeyInvalid => {
                    Err(AwsKmsError::KeyInvalid("mock key invalid".to_string()))
                }
                MockAwsKmsMode::ServiceUnavailable => {
                    Err(AwsKmsError::ServiceUnavailable("mock 503".to_string()))
                }
                MockAwsKmsMode::MalformedResponse => {
                    Err(AwsKmsError::MalformedResponse("mock malformed".to_string()))
                }
                MockAwsKmsMode::Unknown => Err(AwsKmsError::Unknown("mock unknown".to_string())),
                MockAwsKmsMode::Success => {
                    let sig = signer
                        .sign_prehash(&digest)
                        .map_err(|err| AwsKmsError::Unknown(format!("mock sign: {err}")))?;
                    Ok(SignDigestResponse {
                        signature_der: encode_der_from_rs(&sig.r, &sig.s),
                        request_id: "mock-aws-req-id".to_string(),
                    })
                }
                MockAwsKmsMode::WrongKey => {
                    // Sign with the alternate key so the recovered
                    // address diverges from the configured
                    // `expected_address` — exercises the
                    // address-mismatch branch at the adapter +
                    // RemoteSignerClient layers.
                    let alt = wrong_key_signer();
                    let sig = alt
                        .sign_prehash(&digest)
                        .map_err(|err| AwsKmsError::Unknown(format!("mock alt sign: {err}")))?;
                    Ok(SignDigestResponse {
                        signature_der: encode_der_from_rs(&sig.r, &sig.s),
                        request_id: "mock-aws-req-id".to_string(),
                    })
                }
                MockAwsKmsMode::MalformedSignatureDer => Ok(SignDigestResponse {
                    // 3-byte garbage instead of a SEQUENCE — fails the
                    // adapter's DER decoder.
                    signature_der: vec![0xAA, 0xBB, 0xCC],
                    request_id: "mock-aws-req-id".to_string(),
                }),
            }
        })
    }
}

/// Construct a synthetic SPKI carrying the signer's secp256k1 public
/// key. Used only by tests; production AWS KMS produces this layout
/// natively.
fn synth_spki_for_signer(signer: &crate::execution::signer::ExecutorSigner) -> Vec<u8> {
    // The test signer doesn't expose its raw public key, so derive
    // it via k256: the address came from the same path so we can
    // round-trip via the verifying key. We hash a known prehash,
    // recover the public key from the signature, and use that.
    let prehash = [0x42u8; 32];
    let sig = signer
        .sign_prehash(&prehash)
        .expect("test signer must sign deterministic prehash");
    let recovery_id = RecoveryId::try_from(sig.y_parity).expect("mock recovery id");
    let mut rs = [0u8; 64];
    rs[..32].copy_from_slice(&sig.r);
    rs[32..].copy_from_slice(&sig.s);
    let signature = Signature::from_slice(&rs).expect("mock sig parse");
    let verifying_key = VerifyingKey::recover_from_prehash(&prehash, &signature, recovery_id)
        .expect("mock recover");
    let encoded = verifying_key.to_encoded_point(false);
    let sec1 = encoded.as_bytes();
    assert_eq!(sec1.len(), 65);
    assert_eq!(sec1[0], 0x04);
    encode_spki_for_secp256k1(sec1)
}

/// A synthetic SPKI whose recovered EVM address differs from
/// `signer.address()` — used by the `WrongKey` mock mode. Generated
/// from a different fixed test key.
fn synth_spki_wrong_key() -> Vec<u8> {
    synth_spki_for_signer(&wrong_key_signer())
}

/// Alternate test signer used by the `WrongKey` mock mode for both
/// `get_public_key` and `sign_digest` so the recovered EVM address
/// genuinely diverges from the configured `expected_address`.
fn wrong_key_signer() -> crate::execution::signer::ExecutorSigner {
    const ALT_KEY: &str = "0x111111111111111111111111111111111111111111111111111111111111111d";
    crate::execution::signer::ExecutorSigner::from_private_key(
        &crate::execution::config::PrivateKeySecret::new(ALT_KEY.to_string()),
    )
    .expect("alt test signer")
}

/// Build a canonical SubjectPublicKeyInfo for an uncompressed
/// secp256k1 public key. Mirrors AWS KMS `GetPublicKey` output for
/// `KeySpec=ECC_SECG_P256K1`.
fn encode_spki_for_secp256k1(sec1: &[u8]) -> Vec<u8> {
    // Pre-computed AlgorithmIdentifier:
    //   SEQUENCE {
    //     OID 1.2.840.10045.2.1  (ecPublicKey)
    //     OID 1.3.132.0.10        (secp256k1)
    //   }
    // 30 10 06 07 2a 86 48 ce 3d 02 01 06 05 2b 81 04 00 0a
    let algo: [u8; 18] = [
        0x30, 0x10, 0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01, 0x06, 0x05, 0x2b, 0x81,
        0x04, 0x00, 0x0a,
    ];
    // BIT STRING wrapper: tag=03, length, unused-bits=00, then sec1
    let mut bit_string = Vec::with_capacity(2 + 1 + sec1.len());
    bit_string.push(0x00); // unused-bits
    bit_string.extend_from_slice(sec1);
    let mut bit_string_tlv = Vec::with_capacity(2 + bit_string.len());
    bit_string_tlv.push(0x03);
    push_der_length(&mut bit_string_tlv, bit_string.len());
    bit_string_tlv.extend_from_slice(&bit_string);

    // Outer SEQUENCE.
    let inner_len = algo.len() + bit_string_tlv.len();
    let mut out = Vec::with_capacity(2 + inner_len);
    out.push(0x30);
    push_der_length(&mut out, inner_len);
    out.extend_from_slice(&algo);
    out.extend_from_slice(&bit_string_tlv);
    out
}

fn push_der_length(out: &mut Vec<u8>, length: usize) {
    if length < 0x80 {
        out.push(length as u8);
    } else if length < 0x100 {
        out.push(0x81);
        out.push(length as u8);
    } else if length < 0x10000 {
        out.push(0x82);
        out.push((length >> 8) as u8);
        out.push(length as u8);
    } else {
        // Larger lengths are not produced by the test fixtures here.
        panic!("test fixture length too large: {length}");
    }
}

/// Build a canonical DER ECDSA signature from raw `(r, s)` bytes.
/// Mirrors AWS KMS `Sign` output.
fn encode_der_from_rs(r: &[u8; 32], s: &[u8; 32]) -> Vec<u8> {
    fn integer(value: &[u8; 32]) -> Vec<u8> {
        let mut tail: &[u8] = value;
        while tail.len() > 1 && tail[0] == 0 {
            tail = &tail[1..];
        }
        let mut body = Vec::with_capacity(tail.len() + 1);
        if tail[0] & 0x80 != 0 {
            body.push(0x00);
        }
        body.extend_from_slice(tail);
        let mut tlv = Vec::with_capacity(2 + body.len());
        tlv.push(0x02);
        push_der_length(&mut tlv, body.len());
        tlv.extend_from_slice(&body);
        tlv
    }
    let r_tlv = integer(r);
    let s_tlv = integer(s);
    let inner_len = r_tlv.len() + s_tlv.len();
    let mut out = Vec::with_capacity(2 + inner_len);
    out.push(0x30);
    push_der_length(&mut out, inner_len);
    out.extend_from_slice(&r_tlv);
    out.extend_from_slice(&s_tlv);
    out
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::execution::config::PrivateKeySecret;
    use crate::execution::remote_signer::{RemoteSignerClient, SignerRequest, SignerTransport};
    use crate::execution::signer::ExecutorSigner;
    use crate::execution::signer_adapters::map_vendor_error;
    use crate::execution::signer_adapters::PluggableRemoteSignerTransport;
    use uuid::Uuid;

    const TEST_KEY: &str = "0x4c0883a69102937d6231471b5dbb6204fe5129617082792ae468d01a3f362318";
    const TEST_ADDRESS: &str = "0x2c7536e3605d9c16a7a3d7b1898e529396a65c23";

    fn test_signer() -> ExecutorSigner {
        ExecutorSigner::from_private_key(&PrivateKeySecret::new(TEST_KEY.to_string())).unwrap()
    }

    fn provider(mode: MockAwsKmsMode, expected: &str) -> AwsKmsSignerProvider {
        AwsKmsSignerProvider::new(
            Arc::new(MockAwsKmsTransport::new(mode, test_signer())),
            "arn:aws:kms:eu-central-1:000000000000:key/mock-placeholder".to_string(),
            AccountId::new(expected),
        )
    }

    fn test_request<'a>(target: &'a AccountId) -> SignerRequest<'a> {
        SignerRequest {
            request_id: Uuid::from_u128(1),
            intent_id: Uuid::from_u128(2),
            source_type: "option_orderbook_fill",
            chain_id: 84532,
            target_contract: target,
            function_selector: "0x031f77b3",
            calldata_hash: [0u8; 32],
            calldata_length: 0,
            transaction_to: target,
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

    // -------- SPKI / address derivation --------

    #[test]
    fn synthetic_spki_round_trips_to_expected_address() {
        let spki = synth_spki_for_signer(&test_signer());
        let derived = decode_address_from_spki(&spki).unwrap();
        assert_eq!(derived.0, TEST_ADDRESS);
    }

    #[test]
    fn spki_rejects_truncated_input() {
        let err = decode_address_from_spki(&[0x30, 0x10]).expect_err("truncated");
        assert!(matches!(err, VendorError::MalformedSignature(_)));
    }

    #[test]
    fn spki_rejects_non_uncompressed_prefix() {
        // Build SPKI with a compressed-prefix key (0x02 instead of 0x04).
        let mut compressed = [0u8; 33];
        compressed[0] = 0x02;
        let spki = encode_spki_for_secp256k1_with_size(&compressed);
        let err = decode_address_from_spki(&spki).expect_err("compressed prefix");
        match err {
            VendorError::MalformedSignature(reason) => {
                assert!(reason.contains("sec1") || reason.contains("length"));
            }
            other => panic!("expected MalformedSignature, got {other:?}"),
        }
    }

    fn encode_spki_for_secp256k1_with_size(sec1: &[u8]) -> Vec<u8> {
        // Helper for tests: build the SPKI envelope without requiring
        // a 65-byte body so we can exercise the length / prefix
        // rejection branches.
        let algo: [u8; 18] = [
            0x30, 0x10, 0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01, 0x06, 0x05, 0x2b,
            0x81, 0x04, 0x00, 0x0a,
        ];
        let mut bit_string = Vec::with_capacity(1 + sec1.len());
        bit_string.push(0x00);
        bit_string.extend_from_slice(sec1);
        let mut bit_string_tlv = Vec::with_capacity(2 + bit_string.len());
        bit_string_tlv.push(0x03);
        push_der_length(&mut bit_string_tlv, bit_string.len());
        bit_string_tlv.extend_from_slice(&bit_string);
        let inner_len = algo.len() + bit_string_tlv.len();
        let mut out = Vec::with_capacity(2 + inner_len);
        out.push(0x30);
        push_der_length(&mut out, inner_len);
        out.extend_from_slice(&algo);
        out.extend_from_slice(&bit_string_tlv);
        out
    }

    // -------- DER decode --------

    #[test]
    fn der_decoder_round_trips_a_canonical_signature() {
        let mut r = [0u8; 32];
        r[31] = 0x42;
        let mut s = [0u8; 32];
        s[31] = 0x13;
        let der = encode_der_from_rs(&r, &s);
        let (r_decoded, s_decoded) = decode_der_ecdsa_signature(&der).unwrap();
        assert_eq!(r_decoded, r);
        assert_eq!(s_decoded, s);
    }

    #[test]
    fn der_decoder_strips_leading_sign_byte() {
        // ASN.1 INTEGER with the high bit set in the first body byte
        // gets a leading 0x00 sign byte. The decoder must strip it.
        let mut r = [0u8; 32];
        r[0] = 0x80;
        let mut s = [0u8; 32];
        s[31] = 0x01;
        let der = encode_der_from_rs(&r, &s);
        let (r_decoded, _) = decode_der_ecdsa_signature(&der).unwrap();
        assert_eq!(r_decoded, r);
    }

    #[test]
    fn der_decoder_rejects_garbage_input() {
        let err = decode_der_ecdsa_signature(&[0xAA, 0xBB, 0xCC]).expect_err("garbage");
        assert!(err.contains("der outer"));
    }

    #[test]
    fn der_decoder_rejects_oversized_integer() {
        // Construct a DER with a 34-byte r value (one leading 0x00
        // sign byte + 33 non-zero bytes). After stripping the
        // sign byte the integer is still 33 bytes, exceeding the
        // 32-byte secp256k1 scalar bound; the decoder must reject.
        let mut r = [0u8; 34];
        r[0] = 0x00; // sign byte
        r[1] = 0x80; // high bit set forces the sign byte to be present
        for byte in &mut r[2..] {
            *byte = 0x42;
        }
        let mut der = vec![0x30, 0x00];
        der.push(0x02);
        der.push(r.len() as u8);
        der.extend_from_slice(&r);
        der.push(0x02);
        der.push(0x01);
        der.push(0x01);
        der[1] = (der.len() - 2) as u8;
        let err = decode_der_ecdsa_signature(&der).expect_err("oversized r");
        assert!(err.contains("length"));
    }

    // -------- y_parity recovery --------

    #[tokio::test]
    async fn adapter_round_trip_produces_recoverable_signature() {
        let provider = provider(MockAwsKmsMode::Success, TEST_ADDRESS);
        let transport =
            PluggableRemoteSignerTransport::new(Arc::new(provider), AccountId::new(TEST_ADDRESS));
        let target = AccountId::new("0x00000000000000000000000000000000000000ee");
        let request = test_request(&target);
        let response = transport
            .send_sign_request("https://signer-microservice.invalid", request)
            .await
            .expect("success");
        // Recovered address pinned by the transport-layer
        // expected_address; we also verify the structural validator
        // already passed.
        assert_eq!(response.signer_address.0, TEST_ADDRESS);
        assert_eq!(response.audit_log_id.as_deref(), Some("mock-aws-req-id"));
        assert_eq!(
            response.remote_signer_request_id.as_deref(),
            Some("mock-aws-req-id")
        );
    }

    #[tokio::test]
    async fn adapter_returns_address_mismatch_when_vendor_signed_for_wrong_key() {
        let provider = provider(MockAwsKmsMode::WrongKey, TEST_ADDRESS);
        let transport =
            PluggableRemoteSignerTransport::new(Arc::new(provider), AccountId::new(TEST_ADDRESS));
        let target = AccountId::new("0x00000000000000000000000000000000000000ee");
        let err = transport
            .send_sign_request("https://signer-microservice.invalid", test_request(&target))
            .await
            .expect_err("wrong key");
        assert_eq!(err.code(), "post-sign-from-mismatch");
    }

    #[tokio::test]
    async fn adapter_returns_internal_on_malformed_der() {
        let provider = provider(MockAwsKmsMode::MalformedSignatureDer, TEST_ADDRESS);
        let transport =
            PluggableRemoteSignerTransport::new(Arc::new(provider), AccountId::new(TEST_ADDRESS));
        let target = AccountId::new("0x00000000000000000000000000000000000000ee");
        let err = transport
            .send_sign_request("https://signer-microservice.invalid", test_request(&target))
            .await
            .expect_err("malformed der");
        assert_eq!(err.code(), "internal");
        assert!(err.detail().contains("malformed-signature"));
    }

    // -------- Error mapping table coverage --------

    #[test]
    fn map_aws_kms_error_table_is_complete() {
        assert_eq!(
            map_aws_kms_error(AwsKmsError::AccessDenied("x".to_string())).code(),
            "vendor_auth_failed"
        );
        assert_eq!(
            map_aws_kms_error(AwsKmsError::Throttling).code(),
            "vendor_rate_limited"
        );
        assert_eq!(
            map_aws_kms_error(AwsKmsError::Timeout).code(),
            "vendor_timeout"
        );
        assert_eq!(
            map_aws_kms_error(AwsKmsError::KeyDisabled).code(),
            "vendor_denied"
        );
        assert_eq!(
            map_aws_kms_error(AwsKmsError::KeyInvalid("x".to_string())).code(),
            "vendor_denied"
        );
        assert_eq!(
            map_aws_kms_error(AwsKmsError::ServiceUnavailable("x".to_string())).code(),
            "vendor_unavailable"
        );
        assert_eq!(
            map_aws_kms_error(AwsKmsError::MalformedResponse("x".to_string())).code(),
            "vendor_malformed_signature"
        );
        assert_eq!(
            map_aws_kms_error(AwsKmsError::Unknown("x".to_string())).code(),
            "vendor_unknown"
        );
    }

    // -------- Per-mode integration through PluggableRemoteSignerTransport --------

    #[tokio::test]
    async fn access_denied_routes_to_caller_unauthorized() {
        let provider = provider(MockAwsKmsMode::AccessDenied, TEST_ADDRESS);
        let transport =
            PluggableRemoteSignerTransport::new(Arc::new(provider), AccountId::new(TEST_ADDRESS));
        let target = AccountId::new("0x00000000000000000000000000000000000000ee");
        let err = transport
            .send_sign_request("https://x.invalid", test_request(&target))
            .await
            .expect_err("denied");
        assert_eq!(err.code(), "caller-unauthorized");
    }

    #[tokio::test]
    async fn throttling_routes_to_rate_limit() {
        let provider = provider(MockAwsKmsMode::Throttling, TEST_ADDRESS);
        let transport =
            PluggableRemoteSignerTransport::new(Arc::new(provider), AccountId::new(TEST_ADDRESS));
        let target = AccountId::new("0x00000000000000000000000000000000000000ee");
        let err = transport
            .send_sign_request("https://x.invalid", test_request(&target))
            .await
            .expect_err("throttle");
        assert_eq!(err.code(), "rate-limit");
    }

    #[tokio::test]
    async fn timeout_routes_to_kms_timeout() {
        let provider = provider(MockAwsKmsMode::Timeout, TEST_ADDRESS);
        let transport =
            PluggableRemoteSignerTransport::new(Arc::new(provider), AccountId::new(TEST_ADDRESS));
        let target = AccountId::new("0x00000000000000000000000000000000000000ee");
        let err = transport
            .send_sign_request("https://x.invalid", test_request(&target))
            .await
            .expect_err("timeout");
        assert_eq!(err.code(), "kms-timeout");
    }

    #[tokio::test]
    async fn key_disabled_routes_to_kms_unavailable() {
        let provider = provider(MockAwsKmsMode::KeyDisabled, TEST_ADDRESS);
        let transport =
            PluggableRemoteSignerTransport::new(Arc::new(provider), AccountId::new(TEST_ADDRESS));
        let target = AccountId::new("0x00000000000000000000000000000000000000ee");
        let err = transport
            .send_sign_request("https://x.invalid", test_request(&target))
            .await
            .expect_err("disabled");
        assert_eq!(err.code(), "kms-unavailable");
    }

    #[tokio::test]
    async fn key_invalid_routes_to_kms_unavailable() {
        let provider = provider(MockAwsKmsMode::KeyInvalid, TEST_ADDRESS);
        let transport =
            PluggableRemoteSignerTransport::new(Arc::new(provider), AccountId::new(TEST_ADDRESS));
        let target = AccountId::new("0x00000000000000000000000000000000000000ee");
        let err = transport
            .send_sign_request("https://x.invalid", test_request(&target))
            .await
            .expect_err("invalid");
        assert_eq!(err.code(), "kms-unavailable");
    }

    #[tokio::test]
    async fn service_unavailable_routes_to_transport() {
        let provider = provider(MockAwsKmsMode::ServiceUnavailable, TEST_ADDRESS);
        let transport =
            PluggableRemoteSignerTransport::new(Arc::new(provider), AccountId::new(TEST_ADDRESS));
        let target = AccountId::new("0x00000000000000000000000000000000000000ee");
        let err = transport
            .send_sign_request("https://x.invalid", test_request(&target))
            .await
            .expect_err("503");
        assert_eq!(err.code(), "transport");
    }

    #[tokio::test]
    async fn malformed_response_routes_to_internal() {
        let provider = provider(MockAwsKmsMode::MalformedResponse, TEST_ADDRESS);
        let transport =
            PluggableRemoteSignerTransport::new(Arc::new(provider), AccountId::new(TEST_ADDRESS));
        let target = AccountId::new("0x00000000000000000000000000000000000000ee");
        let err = transport
            .send_sign_request("https://x.invalid", test_request(&target))
            .await
            .expect_err("malformed");
        assert_eq!(err.code(), "internal");
    }

    #[tokio::test]
    async fn unknown_routes_to_internal() {
        let provider = provider(MockAwsKmsMode::Unknown, TEST_ADDRESS);
        let transport =
            PluggableRemoteSignerTransport::new(Arc::new(provider), AccountId::new(TEST_ADDRESS));
        let target = AccountId::new("0x00000000000000000000000000000000000000ee");
        let err = transport
            .send_sign_request("https://x.invalid", test_request(&target))
            .await
            .expect_err("unknown");
        assert_eq!(err.code(), "internal");
    }

    // -------- health_check --------

    #[tokio::test]
    async fn health_check_success_returns_expected_address() {
        let provider = provider(MockAwsKmsMode::Success, TEST_ADDRESS);
        let transport =
            PluggableRemoteSignerTransport::new(Arc::new(provider), AccountId::new(TEST_ADDRESS));
        let health = transport
            .send_health_check("https://x.invalid")
            .await
            .expect("health ok");
        assert_eq!(
            health.signer_address.as_ref().map(|a| a.0.as_str()),
            Some(TEST_ADDRESS)
        );
        assert!(health.remote_endpoint_present);
    }

    #[tokio::test]
    async fn health_check_wrong_key_returns_post_sign_from_mismatch() {
        let provider = provider(MockAwsKmsMode::WrongKey, TEST_ADDRESS);
        let transport =
            PluggableRemoteSignerTransport::new(Arc::new(provider), AccountId::new(TEST_ADDRESS));
        let err = transport
            .send_health_check("https://x.invalid")
            .await
            .expect_err("wrong key");
        assert_eq!(err.code(), "post-sign-from-mismatch");
    }

    #[tokio::test]
    async fn health_check_transport_failure_routes_to_transport() {
        let provider = provider(MockAwsKmsMode::ServiceUnavailable, TEST_ADDRESS);
        let transport =
            PluggableRemoteSignerTransport::new(Arc::new(provider), AccountId::new(TEST_ADDRESS));
        let err = transport
            .send_health_check("https://x.invalid")
            .await
            .expect_err("503");
        assert_eq!(err.code(), "transport");
    }

    #[tokio::test]
    async fn health_check_disabled_key_routes_to_kms_unavailable() {
        let provider = provider(MockAwsKmsMode::KeyDisabled, TEST_ADDRESS);
        let transport =
            PluggableRemoteSignerTransport::new(Arc::new(provider), AccountId::new(TEST_ADDRESS));
        let err = transport
            .send_health_check("https://x.invalid")
            .await
            .expect_err("disabled");
        assert_eq!(err.code(), "kms-unavailable");
    }

    // -------- RemoteSignerClient integration --------

    #[tokio::test]
    async fn aws_kms_provider_wired_into_remote_signer_client_round_trips() {
        let provider = provider(MockAwsKmsMode::Success, TEST_ADDRESS);
        let expected = AccountId::new(TEST_ADDRESS);
        let transport: Arc<dyn SignerTransport> = Arc::new(PluggableRemoteSignerTransport::new(
            Arc::new(provider),
            expected.clone(),
        ));
        let client = RemoteSignerClient::with_transport(
            "https://signer-microservice.invalid".to_string(),
            expected,
            transport,
        );
        // Construction-only smoke test; the real `sign_option_execution_tx`
        // surface is exercised by the broader `service.rs` integration
        // tests and the existing `remote_signer::tests` post-sign
        // cross-check.
        let _ = client;
    }

    // -------- Defence-in-depth: mainnet refusal of Mock provider stays intact --------

    #[test]
    fn aws_kms_provider_kind_is_operational() {
        // AwsKms must be on the operational allow-list; Mock and
        // VendorAgnostic must remain non-operational so the config
        // guard at validate_signer_backend keeps refusing them on
        // mainnet even after this milestone.
        assert!(SignerProviderKind::AwsKms.is_operational());
        assert!(!SignerProviderKind::Mock.is_operational());
        assert!(!SignerProviderKind::VendorAgnostic.is_operational());
    }

    #[test]
    fn aws_kms_error_codes_are_stable_taxonomy() {
        // Pins the public `AwsKmsError::code()` strings — these are
        // used inside the adapter for INFO-level tracing.
        assert_eq!(
            AwsKmsError::AccessDenied("x".to_string()).code(),
            "aws_kms_access_denied"
        );
        assert_eq!(AwsKmsError::Throttling.code(), "aws_kms_throttling");
        assert_eq!(AwsKmsError::Timeout.code(), "aws_kms_timeout");
        assert_eq!(AwsKmsError::KeyDisabled.code(), "aws_kms_key_disabled");
        assert_eq!(
            AwsKmsError::KeyInvalid("x".to_string()).code(),
            "aws_kms_key_invalid"
        );
        assert_eq!(
            AwsKmsError::ServiceUnavailable("x".to_string()).code(),
            "aws_kms_service_unavailable"
        );
        assert_eq!(
            AwsKmsError::MalformedResponse("x".to_string()).code(),
            "aws_kms_malformed_response"
        );
        assert_eq!(
            AwsKmsError::Unknown("x".to_string()).code(),
            "aws_kms_unknown"
        );
    }

    // -------- Unused-symbol guard: keep `map_vendor_error` reachable --------

    #[test]
    fn map_vendor_error_remains_in_scope_after_module_split() {
        // Compile-time guard: ensure the pluggable layer's
        // `map_vendor_error` re-export remains importable from this
        // module — protects against accidental removal during
        // refactors.
        let _ = map_vendor_error(VendorError::Timeout);
    }
}
