//! Production AWS KMS transport backed by the real `aws-sdk-kms`
//! crate. Compiled only when the `aws-kms-transport` Cargo feature is
//! enabled.
//!
//! Anchors:
//! - `MAINNET_KMS_VENDOR_SELECTION_DECISION.md §4 + §5` — preferred
//!   path + SignerError mapping table.
//! - `BACKEND_KMS_VENDOR_ADAPTER_IMPLEMENTATION_AWS_KMS_RESULT.md`
//!   — sibling mock transport + SPKI/DER pipeline.
//!
//! ## Hard rules
//!
//!   * NO real AWS account is required to build this module. The
//!     adapter takes a pre-constructed `aws_sdk_kms::Client` from the
//!     caller. Credentials live in the operator's signer
//!     microservice IAM role + secret store per Pattern C; this code
//!     never reads `AWS_ACCESS_KEY_ID` / `AWS_SECRET_ACCESS_KEY` /
//!     `AWS_SESSION_TOKEN` from the backend `.env`.
//!   * NO live AWS call is performed in tests. Tests construct the
//!     transport from a placeholder client and never `.send()` it.
//!     End-to-end mock coverage continues to come from the
//!     `MockAwsKmsTransport` in `aws_kms.rs`.
//!   * Production `RemoteSignerClient::new` continues to use
//!     `UnimplementedTransport`. This module exposes
//!     [`AwsKmsSdkTransport`] only via explicit caller wiring; no
//!     auto-selection from config.
//!   * `Sign` calls use `MessageType=Digest` +
//!     `SigningAlgorithm=EcdsaSha256` — REQUIRED for EVM-style
//!     EIP-1559 prehash signing. The backend never asks KMS to hash
//!     for us.

use crate::execution::signer_adapters::aws_kms::{
    AwsKmsError, AwsKmsFuture, AwsKmsTransport, GetPublicKeyResponse, SignDigestResponse,
    SignRequestMetadata,
};
use aws_sdk_kms::error::SdkError;
use aws_sdk_kms::operation::get_public_key::GetPublicKeyError;
use aws_sdk_kms::operation::sign::SignError;
use aws_sdk_kms::operation::RequestId;
use aws_sdk_kms::types::{MessageType, SigningAlgorithmSpec};
use aws_smithy_types::Blob;

/// Production `AwsKmsTransport` backed by the `aws-sdk-kms` crate.
///
/// Constructed by the caller (the signer microservice; see Pattern C)
/// from a fully-configured `aws_sdk_kms::Client`. The backend never
/// constructs the SDK client itself — credentials, region, endpoint
/// overrides, and retry strategy are all the caller's responsibility.
/// This struct only owns the client handle + maps SDK errors onto the
/// existing `AwsKmsError` taxonomy.
#[derive(Clone, Debug)]
pub struct AwsKmsSdkTransport {
    client: aws_sdk_kms::Client,
}

impl AwsKmsSdkTransport {
    /// Wrap a caller-constructed `aws_sdk_kms::Client`. The caller is
    /// responsible for credentials + region + endpoint configuration.
    pub fn new(client: aws_sdk_kms::Client) -> Self {
        Self { client }
    }

    /// Return a reference to the underlying client. Useful for the
    /// caller to also drive non-Sign / non-GetPublicKey operations
    /// (e.g. `DescribeKey` during operator-side rotation).
    pub fn client(&self) -> &aws_sdk_kms::Client {
        &self.client
    }
}

impl AwsKmsTransport for AwsKmsSdkTransport {
    fn get_public_key<'a>(&'a self, key_id: &'a str) -> AwsKmsFuture<'a, GetPublicKeyResponse> {
        let fut = self
            .client
            .get_public_key()
            .key_id(key_id.to_string())
            .send();
        Box::pin(async move {
            match fut.await {
                Ok(output) => {
                    let sdk_request_id = output.request_id();
                    let public_key_der = output
                        .public_key()
                        .ok_or_else(|| {
                            AwsKmsError::MalformedResponse(
                                "GetPublicKey: response missing PublicKey field".to_string(),
                            )
                        })?
                        .as_ref()
                        .to_vec();
                    let request_id =
                        sanitize_request_id(sdk_request_id, RequestIdOperation::GetPublicKey);
                    Ok(GetPublicKeyResponse {
                        public_key_der,
                        request_id,
                    })
                }
                Err(error) => Err(map_get_public_key_error(error)),
            }
        })
    }

    fn sign_digest<'a>(
        &'a self,
        key_id: &'a str,
        digest: [u8; 32],
        _metadata: SignRequestMetadata,
    ) -> AwsKmsFuture<'a, SignDigestResponse> {
        // AWS KMS does NOT consume request-correlation metadata on
        // the `Sign` API; the signer microservice persists the
        // policy_decision_id / policy_fingerprint binding outside the
        // SDK call. The metadata parameter is accepted here so the
        // trait signature matches the mock + stays Pattern-C-aligned.
        let blob = Blob::new(digest.to_vec());
        let fut = self
            .client
            .sign()
            .key_id(key_id.to_string())
            .message(blob)
            .message_type(MessageType::Digest)
            .signing_algorithm(SigningAlgorithmSpec::EcdsaSha256)
            .send();
        Box::pin(async move {
            match fut.await {
                Ok(output) => {
                    let sdk_request_id = output.request_id();
                    let signature_der = output
                        .signature()
                        .ok_or_else(|| {
                            AwsKmsError::MalformedResponse(
                                "Sign: response missing Signature field".to_string(),
                            )
                        })?
                        .as_ref()
                        .to_vec();
                    let request_id = sanitize_request_id(sdk_request_id, RequestIdOperation::Sign);
                    Ok(SignDigestResponse {
                        signature_der,
                        request_id,
                    })
                }
                Err(error) => Err(map_sign_error(error)),
            }
        })
    }
}

// ---------------------------------------------------------------------------
// SDK error mapping
// ---------------------------------------------------------------------------

/// Project a `SdkError<GetPublicKeyError, _>` onto the existing
/// [`AwsKmsError`] taxonomy.
pub fn map_get_public_key_error<R>(error: SdkError<GetPublicKeyError, R>) -> AwsKmsError
where
    R: std::fmt::Debug + Send + Sync + 'static,
{
    if let SdkError::TimeoutError(_) = &error {
        return AwsKmsError::Timeout;
    }
    if let SdkError::DispatchFailure(failure) = &error {
        return AwsKmsError::ServiceUnavailable(redact_sdk_reason(&format!(
            "dispatch-failure: {failure:?}"
        )));
    }
    if let SdkError::ResponseError(_) = &error {
        return AwsKmsError::ServiceUnavailable("response-error".to_string());
    }
    match error.into_service_error() {
        GetPublicKeyError::DependencyTimeoutException(_) => AwsKmsError::Timeout,
        GetPublicKeyError::DisabledException(_) => AwsKmsError::KeyDisabled,
        GetPublicKeyError::InvalidGrantTokenException(_) => {
            AwsKmsError::AccessDenied("invalid-grant-token".to_string())
        }
        GetPublicKeyError::InvalidKeyUsageException(_) => {
            AwsKmsError::KeyInvalid("invalid-key-usage".to_string())
        }
        GetPublicKeyError::KeyUnavailableException(_) => {
            AwsKmsError::KeyInvalid("key-unavailable".to_string())
        }
        GetPublicKeyError::KmsInternalException(_) => {
            AwsKmsError::ServiceUnavailable("kms-internal".to_string())
        }
        GetPublicKeyError::KmsInvalidStateException(_) => {
            AwsKmsError::KeyInvalid("kms-invalid-state".to_string())
        }
        GetPublicKeyError::NotFoundException(_) => AwsKmsError::KeyInvalid("not-found".to_string()),
        GetPublicKeyError::UnsupportedOperationException(_) => {
            AwsKmsError::KeyInvalid("unsupported-operation".to_string())
        }
        other => map_unhandled_or_generic(&format!("{other:?}")),
    }
}

/// Project a `SdkError<SignError, _>` onto the existing
/// [`AwsKmsError`] taxonomy.
pub fn map_sign_error<R>(error: SdkError<SignError, R>) -> AwsKmsError
where
    R: std::fmt::Debug + Send + Sync + 'static,
{
    if let SdkError::TimeoutError(_) = &error {
        return AwsKmsError::Timeout;
    }
    if let SdkError::DispatchFailure(failure) = &error {
        return AwsKmsError::ServiceUnavailable(redact_sdk_reason(&format!(
            "dispatch-failure: {failure:?}"
        )));
    }
    if let SdkError::ResponseError(_) = &error {
        return AwsKmsError::ServiceUnavailable("response-error".to_string());
    }
    match error.into_service_error() {
        SignError::DependencyTimeoutException(_) => AwsKmsError::Timeout,
        SignError::DisabledException(_) => AwsKmsError::KeyDisabled,
        SignError::DryRunOperationException(_) => AwsKmsError::KeyInvalid("dry-run".to_string()),
        SignError::InvalidGrantTokenException(_) => {
            AwsKmsError::AccessDenied("invalid-grant-token".to_string())
        }
        SignError::InvalidKeyUsageException(_) => {
            AwsKmsError::KeyInvalid("invalid-key-usage".to_string())
        }
        SignError::KeyUnavailableException(_) => {
            AwsKmsError::KeyInvalid("key-unavailable".to_string())
        }
        SignError::KmsInternalException(_) => {
            AwsKmsError::ServiceUnavailable("kms-internal".to_string())
        }
        SignError::KmsInvalidStateException(_) => {
            AwsKmsError::KeyInvalid("kms-invalid-state".to_string())
        }
        SignError::NotFoundException(_) => AwsKmsError::KeyInvalid("not-found".to_string()),
        other => map_unhandled_or_generic(&format!("{other:?}")),
    }
}

/// Project an unknown / unhandled SDK error onto the
/// [`AwsKmsError::Unknown`] variant. Also performs a small string
/// match on documented codes that may not be modelled by an explicit
/// variant in the SDK enum (e.g. `ThrottlingException` is emitted as
/// `ProvideErrorMetadata` metadata when not lifted by the smithy
/// model).
fn map_unhandled_or_generic(diag: &str) -> AwsKmsError {
    let lower = diag.to_ascii_lowercase();
    if lower.contains("throttling") || lower.contains("toomanyrequests") {
        return AwsKmsError::Throttling;
    }
    if lower.contains("access denied") || lower.contains("accessdenied") {
        return AwsKmsError::AccessDenied(redact_sdk_reason(diag));
    }
    if lower.contains("timeout") {
        return AwsKmsError::Timeout;
    }
    if lower.contains("disabled") {
        return AwsKmsError::KeyDisabled;
    }
    if lower.contains("invalid") || lower.contains("notfound") {
        return AwsKmsError::KeyInvalid(redact_sdk_reason(diag));
    }
    if lower.contains("unavailable") || lower.contains("internal") {
        return AwsKmsError::ServiceUnavailable(redact_sdk_reason(diag));
    }
    AwsKmsError::Unknown(redact_sdk_reason(diag))
}

/// Short, length-bounded reason string; SDK diagnostics may include
/// AWS-internal request bodies that we never want to surface to logs
/// at INFO level. Strip control chars + cap to 80 chars (matches the
/// `MAINNET_SIGNER_VENDOR_ADAPTER_REQUIREMENTS.md §5` redaction
/// contract enforced by `map_vendor_error`).
fn redact_sdk_reason(diag: &str) -> String {
    let cleaned: String = diag.chars().filter(|c| !c.is_control()).take(80).collect();
    cleaned
}

// ---------------------------------------------------------------------------
// Request id correlation
// ---------------------------------------------------------------------------

/// Operation discriminator for the request-id sanitizer + synthetic
/// fallback. The prefix is stable so audit consumers can route by
/// operation regardless of whether the id is the real AWS one or the
/// synthetic fallback.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RequestIdOperation {
    GetPublicKey,
    Sign,
}

impl RequestIdOperation {
    fn prefix(self) -> &'static str {
        match self {
            Self::GetPublicKey => "aws-kms-get-public-key",
            Self::Sign => "aws-kms-sign",
        }
    }

    fn synthetic_prefix(self) -> &'static str {
        match self {
            Self::GetPublicKey => "aws-kms-get-public-key-synthetic",
            Self::Sign => "aws-kms-sign-synthetic",
        }
    }
}

/// Maximum length of the operation-prefixed request id we accept from
/// the SDK. AWS request ids are typically 36-char UUIDs but the
/// `RequestId` trait returns `Option<&str>` so a malformed transport
/// shim could theoretically inject a long string. The cap bounds
/// downstream log line size + protects metric labels.
const REQUEST_ID_MAX_LEN: usize = 80;

/// Promote the SDK-provided AWS RequestId to the
/// `<operation_prefix>-<request-id>` shape used downstream, falling
/// back to a synthetic UUID when:
///   * the SDK returned `None`.
///   * the value is empty after trimming.
///   * the value contains control characters (defence-in-depth
///     against a misbehaving transport shim).
///   * the value contains URL-shape characters (`://`, `/`, `?`,
///     `=`) — a real CloudTrail RequestId is a hex/UUID-like token
///     and MUST NOT carry URL-routing tokens; if we see one,
///     something upstream is wrong and we fail closed to the
///     synthetic fallback.
///   * the value exceeds [`REQUEST_ID_MAX_LEN`] after the prefix is
///     applied.
fn sanitize_request_id(raw: Option<&str>, operation: RequestIdOperation) -> String {
    let candidate = raw
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .filter(|s| !s.chars().any(|c| c.is_control()))
        .filter(|s| !contains_url_shape_chars(s));
    if let Some(id) = candidate {
        let with_prefix = format!("{}-{}", operation.prefix(), id);
        if with_prefix.len() <= REQUEST_ID_MAX_LEN {
            return with_prefix;
        }
    }
    format!("{}-{}", operation.synthetic_prefix(), uuid::Uuid::new_v4())
}

/// Reject URL-structural punctuation that should NEVER appear in a
/// real AWS RequestId. Mirrors the redaction shape used by
/// `last_signer_error_code` at the pluggable layer.
fn contains_url_shape_chars(value: &str) -> bool {
    value.contains("://")
        || value.contains('/')
        || value.contains('?')
        || value.contains('=')
        || value.contains('@')
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::execution::signer_adapters::aws_kms::{
        map_aws_kms_error, AwsKmsError, AwsKmsTransport,
    };

    /// The struct constructs from a placeholder client without any
    /// AWS network call. Confirms the type-system wiring works under
    /// `--features aws-kms-transport` and the production transport is
    /// not auto-selected by any default-path constructor.
    #[test]
    fn transport_constructs_without_live_aws_call() {
        // Build a no-credentials / no-region client. We never invoke
        // `.send()` on it, so AWS is never contacted.
        let config = aws_sdk_kms::Config::builder()
            .behavior_version(aws_sdk_kms::config::BehaviorVersion::latest())
            .build();
        let client = aws_sdk_kms::Client::from_conf(config);
        let transport = AwsKmsSdkTransport::new(client);
        // Trait-bound check: ensure the transport satisfies the
        // `AwsKmsTransport` trait at compile time without invoking it.
        let _: &dyn AwsKmsTransport = &transport;
    }

    #[test]
    fn redact_sdk_reason_caps_length_and_strips_control_chars() {
        let raw = "AWS internal error: secret=abc\nstacktrace=...".to_string() + &"x".repeat(200);
        let redacted = redact_sdk_reason(&raw);
        assert!(redacted.len() <= 80);
        assert!(!redacted.contains('\n'));
    }

    #[test]
    fn map_unhandled_or_generic_routes_throttling() {
        let mapped = map_unhandled_or_generic("ThrottlingException: Rate exceeded");
        assert_eq!(mapped.code(), "aws_kms_throttling");
    }

    #[test]
    fn map_unhandled_or_generic_routes_access_denied() {
        let mapped = map_unhandled_or_generic("AccessDeniedException: not authorized");
        assert_eq!(mapped.code(), "aws_kms_access_denied");
    }

    #[test]
    fn map_unhandled_or_generic_routes_disabled() {
        let mapped = map_unhandled_or_generic("Disabled");
        assert_eq!(mapped.code(), "aws_kms_key_disabled");
    }

    #[test]
    fn map_unhandled_or_generic_routes_internal() {
        let mapped = map_unhandled_or_generic("KmsInternalException");
        assert_eq!(mapped.code(), "aws_kms_service_unavailable");
    }

    #[test]
    fn map_unhandled_or_generic_routes_invalid_key() {
        let mapped = map_unhandled_or_generic("NotFoundException: alias/op_be");
        assert_eq!(mapped.code(), "aws_kms_key_invalid");
    }

    #[test]
    fn map_unhandled_or_generic_routes_timeout() {
        let mapped = map_unhandled_or_generic("DependencyTimeoutException");
        assert_eq!(mapped.code(), "aws_kms_timeout");
    }

    #[test]
    fn map_unhandled_or_generic_routes_unknown() {
        let mapped = map_unhandled_or_generic("CompletelyNewVendorErrorXYZ");
        assert_eq!(mapped.code(), "aws_kms_unknown");
    }

    /// Every `AwsKmsError` variant that the SDK-mapping helpers can
    /// produce projects onto the existing `VendorError` taxonomy
    /// (which `map_vendor_error` then projects onto `SignerError`).
    /// Pin every variant so a future SDK upgrade that introduces a
    /// new error category surfaces as a compile-time hint via the
    /// `aws_kms` taxonomy tests.
    #[test]
    fn sdk_mapping_targets_align_with_vendor_error_taxonomy() {
        use crate::execution::signer_adapters::VendorError;
        for (sdk_err, expected_code) in [
            (
                AwsKmsError::AccessDenied("x".to_string()),
                "vendor_auth_failed",
            ),
            (AwsKmsError::Throttling, "vendor_rate_limited"),
            (AwsKmsError::Timeout, "vendor_timeout"),
            (AwsKmsError::KeyDisabled, "vendor_denied"),
            (AwsKmsError::KeyInvalid("x".to_string()), "vendor_denied"),
            (
                AwsKmsError::ServiceUnavailable("x".to_string()),
                "vendor_unavailable",
            ),
            (
                AwsKmsError::MalformedResponse("x".to_string()),
                "vendor_malformed_signature",
            ),
            (AwsKmsError::Unknown("x".to_string()), "vendor_unknown"),
        ] {
            let vendor = map_aws_kms_error(sdk_err.clone());
            assert_eq!(
                vendor.code(),
                expected_code,
                "AwsKmsError {sdk_err:?} should map to {expected_code}, got {:?}",
                vendor.code()
            );
            // Defensive: ensure projection terminates and that no
            // VendorError variant carries an arbitrarily-long
            // sanitized reason from the SDK diag — capped at 80 by
            // `map_vendor_error`'s truncate.
            match vendor {
                VendorError::Unavailable(r)
                | VendorError::Unknown(r)
                | VendorError::MalformedSignature(r)
                | VendorError::Denied(r) => {
                    assert!(r.len() <= 200, "reason should be bounded");
                }
                _ => {}
            }
        }
    }

    // -------- Request id sanitizer + synthetic fallback --------

    #[test]
    fn sanitize_request_id_preserves_real_cloudtrail_id_with_operation_prefix() {
        // Typical CloudTrail RequestId is a 36-char UUID; the
        // operation prefix lifts it above 36 chars but well under
        // REQUEST_ID_MAX_LEN.
        let raw = Some("4f8a1bd9-9f1a-4f9e-9b6e-2d4f6a8c1e3b");
        let id = sanitize_request_id(raw, RequestIdOperation::Sign);
        assert!(id.starts_with("aws-kms-sign-"));
        assert!(id.contains("4f8a1bd9-9f1a-4f9e-9b6e-2d4f6a8c1e3b"));
        assert!(!id.contains("synthetic"));
    }

    #[test]
    fn sanitize_request_id_get_public_key_prefix_routes_correctly() {
        let id = sanitize_request_id(Some("abc-123"), RequestIdOperation::GetPublicKey);
        assert!(id.starts_with("aws-kms-get-public-key-"));
        assert!(id.ends_with("abc-123"));
    }

    #[test]
    fn sanitize_request_id_missing_falls_back_to_synthetic() {
        let id = sanitize_request_id(None, RequestIdOperation::Sign);
        assert!(id.starts_with("aws-kms-sign-synthetic-"));
    }

    #[test]
    fn sanitize_request_id_empty_string_falls_back_to_synthetic() {
        let id = sanitize_request_id(Some(""), RequestIdOperation::Sign);
        assert!(id.starts_with("aws-kms-sign-synthetic-"));
    }

    #[test]
    fn sanitize_request_id_whitespace_only_falls_back_to_synthetic() {
        let id = sanitize_request_id(Some("   "), RequestIdOperation::Sign);
        assert!(id.starts_with("aws-kms-sign-synthetic-"));
    }

    #[test]
    fn sanitize_request_id_rejects_control_characters() {
        let id = sanitize_request_id(Some("abc\nlogspoof"), RequestIdOperation::Sign);
        assert!(id.starts_with("aws-kms-sign-synthetic-"));
        assert!(!id.contains('\n'));
    }

    #[test]
    fn sanitize_request_id_rejects_url_shape() {
        for adversarial in [
            "https://example.com/eviltoken",
            "?accessKey=AKIA1234",
            "evil/path",
            "user@host",
            "scheme://target",
        ] {
            let id = sanitize_request_id(Some(adversarial), RequestIdOperation::Sign);
            assert!(
                id.starts_with("aws-kms-sign-synthetic-"),
                "URL-shape `{adversarial}` should fall back to synthetic, got {id}"
            );
        }
    }

    #[test]
    fn sanitize_request_id_caps_length_to_80_chars() {
        // Build a candidate that would land between 60-79 chars after
        // the operation prefix — accept; one above the cap — reject.
        let short = "x".repeat(50);
        let with_prefix_short = sanitize_request_id(Some(&short), RequestIdOperation::Sign);
        assert!(!with_prefix_short.starts_with("aws-kms-sign-synthetic-"));
        assert!(with_prefix_short.len() <= REQUEST_ID_MAX_LEN);

        // Trigger the > REQUEST_ID_MAX_LEN reject path.
        let too_long = "x".repeat(200);
        let fallback = sanitize_request_id(Some(&too_long), RequestIdOperation::Sign);
        assert!(fallback.starts_with("aws-kms-sign-synthetic-"));
    }

    #[test]
    fn synthetic_fallback_uniqueness_pins_disambiguation_contract() {
        // The synthetic fallback MUST embed a fresh UUID per call so
        // two consecutive Nones don't collapse to the same audit
        // token.
        let first = sanitize_request_id(None, RequestIdOperation::Sign);
        let second = sanitize_request_id(None, RequestIdOperation::Sign);
        assert!(first.starts_with("aws-kms-sign-synthetic-"));
        assert!(second.starts_with("aws-kms-sign-synthetic-"));
        assert_ne!(first, second, "synthetic ids must be unique per call");
    }

    #[test]
    fn synthetic_fallback_distinct_per_operation() {
        let sign = sanitize_request_id(None, RequestIdOperation::Sign);
        let gpk = sanitize_request_id(None, RequestIdOperation::GetPublicKey);
        assert!(sign.starts_with("aws-kms-sign-synthetic-"));
        assert!(gpk.starts_with("aws-kms-get-public-key-synthetic-"));
    }

    #[test]
    fn contains_url_shape_chars_helper_pins_url_patterns() {
        assert!(contains_url_shape_chars("https://example.com"));
        assert!(contains_url_shape_chars("path/segment"));
        assert!(contains_url_shape_chars("?query=1"));
        assert!(contains_url_shape_chars("key=value"));
        assert!(contains_url_shape_chars("user@host"));
        assert!(!contains_url_shape_chars("plain-uuid-1234"));
        assert!(!contains_url_shape_chars(
            "4f8a1bd9-9f1a-4f9e-9b6e-2d4f6a8c1e3b"
        ));
    }
}
