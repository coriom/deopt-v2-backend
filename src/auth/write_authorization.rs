//! Core write-authorization primitives: EIP-712 domain (salt variant),
//! canonical payload encoding, signature verification, and challenge
//! store traits.
//!
//! The frontend module (`src/lib/write-auth.ts`) and the backend MUST
//! produce identical canonical payload bytes and identical EIP-712
//! digests. The canonical encoding is frozen and covered by tests at
//! the bottom of this file.

use crate::error::{BackendError, Result as BackendResult};
use crate::signing::eip712::{keccak256, parse_evm_address};
use crate::signing::signature::{recover_eip712_signer, validate_signature_shape};
use crate::types::{AccountId, TimestampMs};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::fmt;

/// Fixed off-chain EIP-712 domain values. Locked at compile time —
/// must never become runtime-configurable.
pub const WRITE_AUTH_DOMAIN_NAME: &str = "DeOpt API Write";
pub const WRITE_AUTH_DOMAIN_VERSION: &str = "1";
pub const WRITE_AUTH_DOMAIN_CHAIN_ID: u64 = 84532;
pub const WRITE_AUTH_DOMAIN_SALT_PREIMAGE: &str = "deopt-api-write:base-sepolia:v1";

pub const ENVIRONMENT: &str = "base-sepolia";

/// EIP-712 domain typehash for the salt variant (no verifyingContract).
pub const WRITE_AUTH_DOMAIN_TYPE: &str =
    "EIP712Domain(string name,string version,uint256 chainId,bytes32 salt)";

/// EIP-712 primary type for write authorization envelopes.
pub const WRITE_AUTH_PRIMARY_TYPE: &str = "WriteAuthorization(string action,address account,bytes payload,uint256 nonce,uint256 deadline,string environment,string idempotencyKey)";

/// Cryptographic nonce length in bytes (uint256 wire shape).
pub const NONCE_BYTES_LEN: usize = 32;

/// Default challenge TTL (5 minutes).
pub const DEFAULT_CHALLENGE_TTL_MS: i64 = 5 * 60 * 1000;

/// Maximum outstanding (issued, unexpired) challenges per (account, action).
pub const MAX_OUTSTANDING_CHALLENGES_PER_ACCOUNT_ACTION: usize = 32;

/// Frozen salt bytes. `keccak256(WRITE_AUTH_DOMAIN_SALT_PREIMAGE)`.
pub static WRITE_AUTH_DOMAIN_SALT: once_cell::sync::Lazy<[u8; 32]> =
    once_cell::sync::Lazy::new(|| keccak256(WRITE_AUTH_DOMAIN_SALT_PREIMAGE.as_bytes()));

/// Per-action identifier. Frozen on the wire — renaming any variant is
/// a breaking change for all signed messages.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum WriteAuthAction {
    OptionOrderSubmit,
    OptionOrderCancel,
    ConditionalOrderCreate,
    ConditionalOrderCancel,
    OptionRfqCreate,
    OptionRfqQuoteSubmit,
    OptionRfqAccept,
    OptionRfqCancel,
    OptionExecutionIntentSignatureSubmit,
    /// OPTIONS-TWAP-ORDERS-V1 — taker create/cancel of a parent TWAP order.
    OptionTwapCreate,
    OptionTwapCancel,
    /// SUBACCOUNTS-CORE-BACKEND-V1 — owner allocates a new subaccount.
    /// Canonical payload embeds `account` (owner) and optional `name`
    /// only; `subaccount_id` is server-assigned so it cannot be signed
    /// ahead of insertion.
    SubaccountCreate,
    /// SUBACCOUNTS-CORE-BACKEND-V1 — owner renames an existing
    /// subaccount. Canonical payload embeds `account`, `subaccount_id`,
    /// and the new `name`.
    SubaccountRename,
}

impl WriteAuthAction {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::OptionOrderSubmit => "OPTION_ORDER_SUBMIT",
            Self::OptionOrderCancel => "OPTION_ORDER_CANCEL",
            Self::ConditionalOrderCreate => "CONDITIONAL_ORDER_CREATE",
            Self::ConditionalOrderCancel => "CONDITIONAL_ORDER_CANCEL",
            Self::OptionRfqCreate => "OPTION_RFQ_CREATE",
            Self::OptionRfqQuoteSubmit => "OPTION_RFQ_QUOTE_SUBMIT",
            Self::OptionRfqAccept => "OPTION_RFQ_ACCEPT",
            Self::OptionRfqCancel => "OPTION_RFQ_CANCEL",
            Self::OptionExecutionIntentSignatureSubmit => {
                "OPTION_EXECUTION_INTENT_SIGNATURE_SUBMIT"
            }
            Self::OptionTwapCreate => "OPTION_TWAP_CREATE",
            Self::OptionTwapCancel => "OPTION_TWAP_CANCEL",
            Self::SubaccountCreate => "SUBACCOUNT_CREATE",
            Self::SubaccountRename => "SUBACCOUNT_RENAME",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "OPTION_ORDER_SUBMIT" => Self::OptionOrderSubmit,
            "OPTION_ORDER_CANCEL" => Self::OptionOrderCancel,
            "CONDITIONAL_ORDER_CREATE" => Self::ConditionalOrderCreate,
            "CONDITIONAL_ORDER_CANCEL" => Self::ConditionalOrderCancel,
            "OPTION_RFQ_CREATE" => Self::OptionRfqCreate,
            "OPTION_RFQ_QUOTE_SUBMIT" => Self::OptionRfqQuoteSubmit,
            "OPTION_RFQ_ACCEPT" => Self::OptionRfqAccept,
            "OPTION_RFQ_CANCEL" => Self::OptionRfqCancel,
            "OPTION_EXECUTION_INTENT_SIGNATURE_SUBMIT" => {
                Self::OptionExecutionIntentSignatureSubmit
            }
            "OPTION_TWAP_CREATE" => Self::OptionTwapCreate,
            "OPTION_TWAP_CANCEL" => Self::OptionTwapCancel,
            "SUBACCOUNT_CREATE" => Self::SubaccountCreate,
            "SUBACCOUNT_RENAME" => Self::SubaccountRename,
            _ => return None,
        })
    }
}

impl fmt::Display for WriteAuthAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Authorization envelope attached to every protected write request.
/// `nonce` is the hex (0x-prefixed, 64-char) value returned by
/// `POST /auth/write-challenges`.
///
/// SUBACCOUNTS-CORE-BACKEND-V1 — the optional `version` field is
/// reserved for a future subaccount-aware canonical payload variant.
/// It is deserialised for forward compatibility but is NOT yet
/// incorporated into the EIP-712 digest computation, so existing v1
/// byte-freeze tests remain green. The follow-up milestone
/// `SUBACCOUNTS-WRITE-AUTH-V2-MIGRATION-V1` will branch the digest on
/// this field once existing trading actions migrate.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AuthorizationEnvelope {
    pub action: String,
    pub account: AccountId,
    pub nonce: String,
    pub deadline_ms: TimestampMs,
    pub signature: String,
    #[serde(default)]
    pub idempotency_key: Option<String>,
    /// Reserved — see the struct-level doc comment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<u16>,
}

/// Result of a successful verification + nonce claim. Returned to the
/// route handler so it can correlate the resulting resource_id with
/// the consumed challenge (for idempotent retries).
#[derive(Clone, Debug)]
pub struct WriteAuthVerified {
    pub action: WriteAuthAction,
    pub account: AccountId,
    pub nonce_bytes: [u8; 32],
    pub idempotency_key: Option<String>,
    /// True iff this is the first time the nonce was claimed (i.e.
    /// `ClaimOutcome::Fresh`). False for any flavour of IdempotentReplay.
    /// Used by the verifier to reject a replay whose prior attempt
    /// left no `resource_id` linked (see `AmbiguousPriorClaim`).
    pub was_fresh: bool,
    /// If non-None, the canonical payload was already consumed by an
    /// earlier identical request and resulted in `resource_id`. The
    /// route handler MUST return the existing resource (idempotent
    /// retry) rather than performing the mutation again.
    pub idempotent_resource_id: Option<String>,
}

/// Persisted challenge row. Used by `WriteAuthChallengeStore`.
#[derive(Clone, Debug)]
pub struct ChallengeRecord {
    pub nonce_bytes: [u8; 32],
    pub account: AccountId,
    pub action: WriteAuthAction,
    pub chain_id: u64,
    pub issued_at_ms: TimestampMs,
    pub expires_at_ms: TimestampMs,
    pub status: ChallengeStatus,
    pub request_digest: Option<[u8; 32]>,
    pub idempotency_key: Option<String>,
    pub resource_id: Option<String>,
    pub consumed_at_ms: Option<TimestampMs>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChallengeStatus {
    Issued,
    Consumed,
    Expired,
}

impl ChallengeStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Issued => "issued",
            Self::Consumed => "consumed",
            Self::Expired => "expired",
        }
    }
}

/// Stable error categories. Mapped to HTTP status codes in `api/routes.rs`.
#[derive(Debug, Clone, thiserror::Error)]
pub enum WriteAuthError {
    #[error("write authorization required")]
    AuthorizationRequired,
    #[error("invalid write signature")]
    InvalidSignature,
    #[error("signer does not match account")]
    SignerMismatch,
    #[error("write authorization expired")]
    Expired,
    #[error("write authorization nonce not found")]
    NonceNotFound,
    #[error("write authorization nonce already used")]
    NonceAlreadyUsed,
    #[error("write authorization action mismatch")]
    ActionMismatch,
    #[error("write authorization chain mismatch")]
    ChainMismatch,
    #[error("write authorization domain mismatch")]
    DomainMismatch,
    #[error("write authorization payload mismatch")]
    PayloadMismatch,
    #[error("idempotency key conflict")]
    IdempotencyConflict,
    #[error("write authorization persistence error")]
    Persistence,
    #[error("too many outstanding write challenges")]
    TooManyOutstandingChallenges,
    /// ACCOUNT-WRITE-AUTH-LIVE-PG-PROOF-V1 — emitted when a prior
    /// attempt at this nonce was claimed (status='consumed') but no
    /// `resource_id` was ever linked. This means a prior request
    /// either crashed between the mutation and the resource-link
    /// step, or the mutation itself failed AFTER the nonce was
    /// consumed. Replaying the same envelope cannot safely tell
    /// "mutation succeeded but link broke" apart from "mutation
    /// silently failed" — both surfaces force the caller to request
    /// a fresh challenge so the next attempt is unambiguous.
    #[error("write authorization prior claim is ambiguous; request a new challenge")]
    AmbiguousPriorClaim,
}

// `BackendError::WriteAuth` is auto-derived via `#[from]` on the
// `error::BackendError` enum variant, so no manual impl is needed.

/// A single field in a canonical payload. Frontend and backend MUST
/// emit identical bytes for the same value.
#[derive(Clone, Debug)]
pub enum CanonicalValue {
    Str(String),
    U64(u64),
    U128(u128),
    Bool(bool),
    Address(AccountId),
    Null,
}

impl CanonicalValue {
    fn encode(&self) -> String {
        match self {
            Self::Str(s) => format!("\"{}\"", escape_canonical_string(s)),
            Self::U64(v) => v.to_string(),
            Self::U128(v) => v.to_string(),
            Self::Bool(v) => v.to_string(),
            Self::Address(a) => format!("\"{}\"", a.0.to_lowercase()),
            Self::Null => "null".to_string(),
        }
    }
}

fn escape_canonical_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ => out.push(ch),
        }
    }
    out
}

/// Build canonical payload bytes for a given action.
///
/// Format: `action|key1=val1|key2=val2|...` where fields are emitted
/// in the order provided by the caller. The encoding is frozen and
/// MUST stay byte-identical to `src/lib/write-auth.ts::canonicalPayload`.
pub fn canonical_payload_bytes(
    action: WriteAuthAction,
    fields: &[(&'static str, CanonicalValue)],
) -> Vec<u8> {
    let mut out = String::new();
    out.push_str(action.as_str());
    for (key, value) in fields {
        out.push('|');
        out.push_str(key);
        out.push('=');
        out.push_str(&value.encode());
    }
    out.into_bytes()
}

#[derive(Clone, Debug)]
pub struct WriteAuthDomain;

pub fn write_auth_domain() -> WriteAuthDomain {
    WriteAuthDomain
}

/// Compute the EIP-712 digest the client must sign for a write
/// authorization. The `payload` argument is the canonical payload
/// bytes (see `canonical_payload_bytes`). Returned digest is the
/// `keccak256(0x19 0x01 || domainSeparator || messageHash)` value
/// that the wallet recovers from.
pub fn write_auth_eip712_digest(
    action: WriteAuthAction,
    account: &AccountId,
    payload: &[u8],
    nonce_bytes: &[u8; 32],
    deadline_ms: TimestampMs,
    idempotency_key: Option<&str>,
) -> BackendResult<[u8; 32]> {
    let domain_separator = compute_domain_separator();
    let message_hash = compute_message_hash(
        action,
        account,
        payload,
        nonce_bytes,
        deadline_ms,
        idempotency_key,
    )?;

    let mut encoded = Vec::with_capacity(2 + 32 + 32);
    encoded.extend_from_slice(b"\x19\x01");
    encoded.extend_from_slice(&domain_separator);
    encoded.extend_from_slice(&message_hash);
    Ok(keccak256(&encoded))
}

fn compute_domain_separator() -> [u8; 32] {
    let mut encoded = Vec::with_capacity(160);
    encoded.extend_from_slice(&keccak256(WRITE_AUTH_DOMAIN_TYPE.as_bytes()));
    encoded.extend_from_slice(&keccak256(WRITE_AUTH_DOMAIN_NAME.as_bytes()));
    encoded.extend_from_slice(&keccak256(WRITE_AUTH_DOMAIN_VERSION.as_bytes()));
    encoded.extend_from_slice(&encode_u256(WRITE_AUTH_DOMAIN_CHAIN_ID as u128));
    encoded.extend_from_slice(&*WRITE_AUTH_DOMAIN_SALT);
    keccak256(&encoded)
}

fn compute_message_hash(
    action: WriteAuthAction,
    account: &AccountId,
    payload: &[u8],
    nonce_bytes: &[u8; 32],
    deadline_ms: TimestampMs,
    idempotency_key: Option<&str>,
) -> BackendResult<[u8; 32]> {
    let account_bytes = parse_evm_address(account)?;
    let mut encoded = Vec::with_capacity(7 * 32);
    encoded.extend_from_slice(&keccak256(WRITE_AUTH_PRIMARY_TYPE.as_bytes()));
    encoded.extend_from_slice(&keccak256(action.as_str().as_bytes()));
    encoded.extend_from_slice(&encode_address(&account_bytes));
    encoded.extend_from_slice(&keccak256(payload));
    encoded.extend_from_slice(nonce_bytes);
    let deadline_u128: u128 = u128::try_from(deadline_ms.max(0)).unwrap_or(0);
    encoded.extend_from_slice(&encode_u256(deadline_u128));
    encoded.extend_from_slice(&keccak256(ENVIRONMENT.as_bytes()));
    encoded.extend_from_slice(&keccak256(idempotency_key.unwrap_or("").as_bytes()));
    Ok(keccak256(&encoded))
}

fn encode_address(address: &[u8; 20]) -> [u8; 32] {
    let mut word = [0u8; 32];
    word[12..].copy_from_slice(address);
    word
}

fn encode_u256(value: u128) -> [u8; 32] {
    let mut word = [0u8; 32];
    word[16..].copy_from_slice(&value.to_be_bytes());
    word
}

/// Parse a 0x-prefixed 64-char hex nonce into 32 bytes. Returns
/// `NonceNotFound` on shape mismatch — callers treat a malformed
/// nonce the same as an unknown one (no oracle for valid nonces).
pub fn parse_nonce_hex(value: &str) -> std::result::Result<[u8; 32], WriteAuthError> {
    let hex = value
        .strip_prefix("0x")
        .ok_or(WriteAuthError::NonceNotFound)?;
    if hex.len() != 64 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(WriteAuthError::NonceNotFound);
    }
    let mut out = [0u8; 32];
    for (i, byte) in out.iter_mut().enumerate() {
        let high = decode_nibble(hex.as_bytes()[i * 2])?;
        let low = decode_nibble(hex.as_bytes()[i * 2 + 1])?;
        *byte = (high << 4) | low;
    }
    Ok(out)
}

pub fn nonce_to_hex(nonce: &[u8; 32]) -> String {
    let mut out = String::with_capacity(66);
    out.push_str("0x");
    for byte in nonce {
        out.push_str(&format!("{:02x}", byte));
    }
    out
}

fn decode_nibble(byte: u8) -> std::result::Result<u8, WriteAuthError> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => Err(WriteAuthError::NonceNotFound),
    }
}

/// Outcome of an atomic challenge claim. Used by the store to signal
/// the three terminal cases without leaking SQL.
#[derive(Debug)]
pub enum ClaimOutcome {
    /// Nonce was issued and we just claimed it for this payload.
    Fresh,
    /// Nonce was already consumed by a request with the SAME
    /// `request_digest`. The route handler MUST return the existing
    /// `resource_id` (idempotent retry).
    IdempotentReplay { resource_id: Option<String> },
    /// Nonce was already consumed but for a DIFFERENT payload — replay
    /// of one nonce against a second mutation is forbidden.
    PayloadMismatch,
    /// Nonce existed but is expired.
    Expired,
    /// Nonce was never issued (or already pruned).
    NotFound,
    /// ACCOUNT-WRITE-AUTH-LIVE-PG-PROOF-V1 — another (different) nonce
    /// with the same `(account, action, idempotency_key)` was already
    /// consumed. The current nonce row stays in `issued` status; the
    /// caller is rejected with `WriteAuthError::IdempotencyConflict`
    /// so they cannot smuggle a second mutation under one idempotency
    /// key. Only fires when the envelope supplies a non-null
    /// idempotency key.
    IdempotencyKeyConflict,
}

/// Storage abstraction for write-auth challenges. In production this
/// is backed by `write_auth_challenges` (migration 0029) via
/// `PgRepository`. Tests use the in-memory implementation below.
#[async_trait]
pub trait WriteAuthChallengeStore: Send + Sync {
    async fn issue(&self, record: ChallengeRecord) -> std::result::Result<(), WriteAuthError>;

    async fn count_outstanding(
        &self,
        account: &AccountId,
        action: WriteAuthAction,
        now_ms: TimestampMs,
    ) -> std::result::Result<usize, WriteAuthError>;

    /// Atomically transition `issued` -> `consumed` for nonce
    /// `nonce_bytes`, requiring (account, action, chain_id) to match
    /// and the row to not be expired. Records `request_digest` on
    /// success; rejects if a previously-consumed row has a different
    /// digest. See `ClaimOutcome` for the four terminal cases.
    async fn claim(
        &self,
        nonce_bytes: [u8; 32],
        account: &AccountId,
        action: WriteAuthAction,
        chain_id: u64,
        request_digest: [u8; 32],
        idempotency_key: Option<&str>,
        now_ms: TimestampMs,
    ) -> std::result::Result<ClaimOutcome, WriteAuthError>;

    /// Lookup an idempotent prior result by (account, action,
    /// idempotency_key). Returns the previously-stored resource_id
    /// when a successful consumption matches.
    async fn lookup_idempotent_result(
        &self,
        account: &AccountId,
        action: WriteAuthAction,
        idempotency_key: &str,
    ) -> std::result::Result<Option<String>, WriteAuthError>;

    /// Attach a resource_id to a consumed challenge so future exact-
    /// retries return the same resource.
    async fn attach_resource(
        &self,
        nonce_bytes: [u8; 32],
        resource_id: &str,
    ) -> std::result::Result<(), WriteAuthError>;
}

/// SUBACCOUNTS-V2-NONCE-TABLE-V1 — outcome of a v2 nonce claim.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum V2NonceClaimOutcome {
    /// First time this `(account, subaccount_id, action, nonce)` was
    /// seen — row was inserted.
    Fresh,
    /// Row already exists — the caller MUST reject with
    /// `NonceAlreadyUsed`. `used_nonces_v2` is append-only; this
    /// outcome is emitted whether or not the stored `request_digest`
    /// matches, because a v2 replay always crosses the challenge store
    /// first and only reaches `consume_v2_nonce` on a fresh claim.
    Duplicate,
}

/// SUBACCOUNTS-V2-NONCE-TABLE-V1 — persistent v2-scoped nonce
/// consumption ledger. Independent of `WriteAuthChallengeStore`
/// (v1 nonce claim + idempotency). Only invoked when
/// `AuthorizationEnvelope.version == Some(2)`.
#[async_trait]
pub trait UsedNonceV2Store: Send + Sync {
    /// Atomically inserts one row into `used_nonces_v2`. Duplicate PK
    /// (case-insensitive `account`, `subaccount_id`, `action`,
    /// `nonce_bytes`) yields `Duplicate`.
    async fn consume_v2_nonce(
        &self,
        account: &AccountId,
        subaccount_id: u32,
        action: WriteAuthAction,
        nonce_bytes: [u8; 32],
        request_digest: [u8; 32],
        now_ms: TimestampMs,
    ) -> std::result::Result<V2NonceClaimOutcome, WriteAuthError>;
}

/// Verify an `AuthorizationEnvelope` against the canonical payload and
/// atomically claim its nonce.
///
/// Returns `Ok(verified)` on success. The caller MUST check
/// `verified.idempotent_resource_id` first: if `Some`, return the
/// existing resource without performing the mutation again.
#[allow(clippy::too_many_arguments)]
pub async fn verify_and_claim(
    store: &dyn WriteAuthChallengeStore,
    envelope: &AuthorizationEnvelope,
    expected_action: WriteAuthAction,
    expected_account: &AccountId,
    canonical_payload: &[u8],
    chain_id: u64,
    now_ms: TimestampMs,
) -> std::result::Result<WriteAuthVerified, WriteAuthError> {
    if chain_id != WRITE_AUTH_DOMAIN_CHAIN_ID {
        return Err(WriteAuthError::ChainMismatch);
    }

    // Reject envelope/action mismatch.
    let envelope_action =
        WriteAuthAction::parse(&envelope.action).ok_or(WriteAuthError::ActionMismatch)?;
    if envelope_action != expected_action {
        return Err(WriteAuthError::ActionMismatch);
    }

    // Account binding (case-insensitive — addresses normalize to lower).
    if !envelope.account.0.eq_ignore_ascii_case(&expected_account.0) {
        return Err(WriteAuthError::SignerMismatch);
    }

    // Deadline.
    if envelope.deadline_ms <= now_ms {
        return Err(WriteAuthError::Expired);
    }

    // Signature shape (132 chars incl. 0x).
    validate_signature_shape(&envelope.signature).map_err(|_| WriteAuthError::InvalidSignature)?;

    // Parse nonce shape.
    let nonce_bytes = parse_nonce_hex(&envelope.nonce)?;

    // Compute the digest the wallet must have signed and recover the signer.
    let digest = write_auth_eip712_digest(
        expected_action,
        expected_account,
        canonical_payload,
        &nonce_bytes,
        envelope.deadline_ms,
        envelope.idempotency_key.as_deref(),
    )
    .map_err(|_| WriteAuthError::InvalidSignature)?;

    let recovered_signer =
        recover_eip712_signer(&digest, &envelope.signature).map_err(|err| match err {
            BackendError::MalformedSignature
            | BackendError::UnsupportedSignatureV
            | BackendError::SignatureRecoveryFailed => WriteAuthError::InvalidSignature,
            BackendError::MalformedAccountAddress => WriteAuthError::SignerMismatch,
            _ => WriteAuthError::InvalidSignature,
        })?;

    if !recovered_signer.0.eq_ignore_ascii_case(&expected_account.0) {
        return Err(WriteAuthError::SignerMismatch);
    }

    // Request digest binds payload+nonce+deadline+idempotency. Stored
    // so the next time the same nonce arrives we can distinguish
    // exact-retry from a same-nonce-different-payload attack.
    let request_digest = keccak256(&digest);

    let claim = store
        .claim(
            nonce_bytes,
            expected_account,
            expected_action,
            chain_id,
            request_digest,
            envelope.idempotency_key.as_deref(),
            now_ms,
        )
        .await?;

    let (was_fresh, mut idempotent_resource_id) = match claim {
        ClaimOutcome::Fresh => (true, None),
        ClaimOutcome::IdempotentReplay { resource_id } => (false, resource_id),
        ClaimOutcome::PayloadMismatch => return Err(WriteAuthError::PayloadMismatch),
        ClaimOutcome::Expired => return Err(WriteAuthError::Expired),
        ClaimOutcome::NotFound => return Err(WriteAuthError::NonceNotFound),
        ClaimOutcome::IdempotencyKeyConflict => return Err(WriteAuthError::IdempotencyConflict),
    };

    // Idempotency key check (separate from nonce-binding). If the
    // caller supplied an idempotency key and the same (account, action,
    // key) has previously resolved to a resource, prefer that. Only
    // consult the idempotency-key index when the nonce-bound replay
    // did not already supply a resource_id.
    if idempotent_resource_id.is_none() {
        if let Some(key) = envelope.idempotency_key.as_deref() {
            idempotent_resource_id = store
                .lookup_idempotent_result(expected_account, expected_action, key)
                .await?;
        }
    }

    // ACCOUNT-WRITE-AUTH-LIVE-PG-PROOF-V1 — defence-in-depth against
    // partial prior state. If the nonce was already consumed but no
    // resource_id is linked (and the idempotency-key fallback did not
    // resolve one either), the prior attempt is ambiguous: we cannot
    // tell from the store alone whether the business mutation actually
    // ran. Re-executing risks a duplicate; silently returning success
    // risks a phantom resource. Fail loud and force the caller to
    // request a fresh challenge so the next attempt is unambiguous.
    if !was_fresh && idempotent_resource_id.is_none() {
        return Err(WriteAuthError::AmbiguousPriorClaim);
    }

    Ok(WriteAuthVerified {
        action: expected_action,
        account: expected_account.clone(),
        nonce_bytes,
        idempotency_key: envelope.idempotency_key.clone(),
        was_fresh,
        idempotent_resource_id,
    })
}

/// Generate a fresh 32-byte challenge nonce using the OS CSPRNG.
pub fn generate_nonce() -> [u8; 32] {
    use rand::RngCore;
    let mut bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    bytes
}

// ===========================================================================
// In-memory store (used as the default fallback inside AppState; suitable
// for unit/integration tests that do not run a real PostgreSQL instance).
//
// NOT a production replacement for `PgRepository` — the in-memory map is
// not shared across processes and is wiped on restart, so production
// deployments MUST wire a PgRepository instance via `AppState`.
// ===========================================================================

pub mod memory_store {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;

    #[derive(Default)]
    pub struct InMemoryChallengeStore {
        inner: Mutex<HashMap<[u8; 32], ChallengeRecord>>,
    }

    impl InMemoryChallengeStore {
        pub fn new() -> Self {
            Self::default()
        }
    }

    #[async_trait]
    impl WriteAuthChallengeStore for InMemoryChallengeStore {
        async fn issue(&self, record: ChallengeRecord) -> std::result::Result<(), WriteAuthError> {
            let mut guard = self.inner.lock().expect("InMemoryChallengeStore poisoned");
            if guard.contains_key(&record.nonce_bytes) {
                return Err(WriteAuthError::NonceAlreadyUsed);
            }
            guard.insert(record.nonce_bytes, record);
            Ok(())
        }

        async fn count_outstanding(
            &self,
            account: &AccountId,
            action: WriteAuthAction,
            now_ms: TimestampMs,
        ) -> std::result::Result<usize, WriteAuthError> {
            let guard = self.inner.lock().expect("InMemoryChallengeStore poisoned");
            Ok(guard
                .values()
                .filter(|r| {
                    r.account.0.eq_ignore_ascii_case(&account.0)
                        && r.action == action
                        && r.status == ChallengeStatus::Issued
                        && r.expires_at_ms > now_ms
                })
                .count())
        }

        async fn claim(
            &self,
            nonce_bytes: [u8; 32],
            account: &AccountId,
            action: WriteAuthAction,
            chain_id: u64,
            request_digest: [u8; 32],
            idempotency_key: Option<&str>,
            now_ms: TimestampMs,
        ) -> std::result::Result<ClaimOutcome, WriteAuthError> {
            let mut guard = self.inner.lock().expect("InMemoryChallengeStore poisoned");
            // ACCOUNT-WRITE-AUTH-LIVE-PG-PROOF-V1 — symmetric idempotency
            // partial-unique-index enforcement. Before promoting this
            // nonce row to 'consumed', check that no OTHER consumed row
            // already owns (account, action, idempotency_key). The PG
            // store enforces this via `write_auth_challenges_idempotency`;
            // the in-memory store enforces it here so dev/test behaviour
            // matches production.
            if let Some(key) = idempotency_key {
                let already_consumed = guard.values().any(|r| {
                    r.account.0.eq_ignore_ascii_case(&account.0)
                        && r.action == action
                        && r.status == ChallengeStatus::Consumed
                        && r.idempotency_key.as_deref() == Some(key)
                        && r.nonce_bytes != nonce_bytes
                });
                if already_consumed {
                    return Ok(ClaimOutcome::IdempotencyKeyConflict);
                }
            }
            let Some(record) = guard.get_mut(&nonce_bytes) else {
                return Ok(ClaimOutcome::NotFound);
            };
            if !record.account.0.eq_ignore_ascii_case(&account.0)
                || record.action != action
                || record.chain_id != chain_id
            {
                return Ok(ClaimOutcome::NotFound);
            }
            if record.expires_at_ms <= now_ms {
                record.status = ChallengeStatus::Expired;
                return Ok(ClaimOutcome::Expired);
            }
            match record.status {
                ChallengeStatus::Issued => {
                    record.status = ChallengeStatus::Consumed;
                    record.request_digest = Some(request_digest);
                    record.consumed_at_ms = Some(now_ms);
                    Ok(ClaimOutcome::Fresh)
                }
                ChallengeStatus::Consumed => {
                    if record.request_digest == Some(request_digest) {
                        Ok(ClaimOutcome::IdempotentReplay {
                            resource_id: record.resource_id.clone(),
                        })
                    } else {
                        Ok(ClaimOutcome::PayloadMismatch)
                    }
                }
                ChallengeStatus::Expired => Ok(ClaimOutcome::Expired),
            }
        }

        async fn lookup_idempotent_result(
            &self,
            account: &AccountId,
            action: WriteAuthAction,
            idempotency_key: &str,
        ) -> std::result::Result<Option<String>, WriteAuthError> {
            let guard = self.inner.lock().expect("InMemoryChallengeStore poisoned");
            Ok(guard
                .values()
                .find(|r| {
                    r.account.0.eq_ignore_ascii_case(&account.0)
                        && r.action == action
                        && r.idempotency_key.as_deref() == Some(idempotency_key)
                        && r.status == ChallengeStatus::Consumed
                        && r.resource_id.is_some()
                })
                .and_then(|r| r.resource_id.clone()))
        }

        async fn attach_resource(
            &self,
            nonce_bytes: [u8; 32],
            resource_id: &str,
        ) -> std::result::Result<(), WriteAuthError> {
            let mut guard = self.inner.lock().expect("InMemoryChallengeStore poisoned");
            if let Some(record) = guard.get_mut(&nonce_bytes) {
                record.resource_id = Some(resource_id.to_string());
            }
            Ok(())
        }
    }

    /// SUBACCOUNTS-V2-NONCE-TABLE-V1 — test/in-memory backend for
    /// `UsedNonceV2Store`. Uses a HashSet keyed by the same tuple as
    /// the PG unique index so dev/test behaviour matches production.
    #[derive(Default)]
    pub struct InMemoryUsedNonceV2Store {
        inner: Mutex<std::collections::HashSet<(String, u32, WriteAuthAction, [u8; 32])>>,
    }

    impl InMemoryUsedNonceV2Store {
        pub fn new() -> Self {
            Self::default()
        }
    }

    #[async_trait]
    impl UsedNonceV2Store for InMemoryUsedNonceV2Store {
        async fn consume_v2_nonce(
            &self,
            account: &AccountId,
            subaccount_id: u32,
            action: WriteAuthAction,
            nonce_bytes: [u8; 32],
            _request_digest: [u8; 32],
            _now_ms: TimestampMs,
        ) -> std::result::Result<V2NonceClaimOutcome, WriteAuthError> {
            let mut guard = self
                .inner
                .lock()
                .expect("InMemoryUsedNonceV2Store poisoned");
            let key = (account.0.to_lowercase(), subaccount_id, action, nonce_bytes);
            if guard.contains(&key) {
                Ok(V2NonceClaimOutcome::Duplicate)
            } else {
                guard.insert(key);
                Ok(V2NonceClaimOutcome::Fresh)
            }
        }
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::memory_store::InMemoryChallengeStore;
    use super::*;
    use k256::ecdsa::SigningKey;

    fn test_keypair() -> (SigningKey, AccountId) {
        // Deterministic test key. NEVER load real keys here.
        let signing_key = SigningKey::from_bytes(&[7u8; 32].into()).expect("test key");
        let verifying_key = signing_key.verifying_key();
        let encoded = verifying_key.to_encoded_point(false);
        let public_key = encoded.as_bytes();
        let hash = keccak256(&public_key[1..]);
        let mut address = [0u8; 20];
        address.copy_from_slice(&hash[12..]);
        let mut hex = String::from("0x");
        for b in &address {
            hex.push_str(&format!("{:02x}", b));
        }
        (signing_key, AccountId::new(hex))
    }

    fn sign_digest(signing_key: &SigningKey, digest: &[u8; 32]) -> String {
        let (signature, recovery_id) = signing_key
            .sign_prehash_recoverable(digest)
            .expect("test sign");
        let sig_bytes = signature.to_bytes();
        let v: u8 = recovery_id.into();
        let mut out = String::from("0x");
        for b in &sig_bytes {
            out.push_str(&format!("{:02x}", b));
        }
        out.push_str(&format!("{:02x}", v + 27));
        out
    }

    fn build_envelope(
        signing_key: &SigningKey,
        action: WriteAuthAction,
        account: &AccountId,
        payload: &[u8],
        nonce_bytes: [u8; 32],
        deadline_ms: TimestampMs,
        idempotency_key: Option<&str>,
    ) -> AuthorizationEnvelope {
        let digest = write_auth_eip712_digest(
            action,
            account,
            payload,
            &nonce_bytes,
            deadline_ms,
            idempotency_key,
        )
        .expect("digest");
        let signature = sign_digest(signing_key, &digest);
        AuthorizationEnvelope {
            action: action.as_str().to_string(),
            account: account.clone(),
            nonce: nonce_to_hex(&nonce_bytes),
            deadline_ms,
            signature,
            idempotency_key: idempotency_key.map(|s| s.to_string()),
            version: None,
        }
    }

    async fn issue_test_challenge(
        store: &InMemoryChallengeStore,
        action: WriteAuthAction,
        account: &AccountId,
        nonce_bytes: [u8; 32],
        expires_at_ms: TimestampMs,
        idempotency_key: Option<&str>,
    ) {
        store
            .issue(ChallengeRecord {
                nonce_bytes,
                account: account.clone(),
                action,
                chain_id: WRITE_AUTH_DOMAIN_CHAIN_ID,
                issued_at_ms: 0,
                expires_at_ms,
                status: ChallengeStatus::Issued,
                request_digest: None,
                idempotency_key: idempotency_key.map(|s| s.to_string()),
                resource_id: None,
                consumed_at_ms: None,
            })
            .await
            .expect("issue");
    }

    #[test]
    fn canonical_payload_encoding_is_frozen() {
        let payload = canonical_payload_bytes(
            WriteAuthAction::OptionOrderSubmit,
            &[
                (
                    "account",
                    CanonicalValue::Address(AccountId::new(
                        "0xABCDEF0000000000000000000000000000000001",
                    )),
                ),
                (
                    "series",
                    CanonicalValue::Str("BTC-30JAN2026-50000-C".to_string()),
                ),
                ("price_1e8", CanonicalValue::U128(50_000_000_000)),
                ("post_only", CanonicalValue::Bool(true)),
                ("client_order_id", CanonicalValue::Null),
            ],
        );
        assert_eq!(
            std::str::from_utf8(&payload).unwrap(),
            "OPTION_ORDER_SUBMIT|account=\"0xabcdef0000000000000000000000000000000001\"|series=\"BTC-30JAN2026-50000-C\"|price_1e8=50000000000|post_only=true|client_order_id=null"
        );
    }

    #[test]
    fn domain_salt_is_frozen() {
        // keccak256("deopt-api-write:base-sepolia:v1")
        let expected = keccak256(b"deopt-api-write:base-sepolia:v1");
        assert_eq!(*WRITE_AUTH_DOMAIN_SALT, expected);
    }

    #[tokio::test]
    async fn valid_envelope_round_trips() {
        let (signing_key, account) = test_keypair();
        let store = InMemoryChallengeStore::new();
        let nonce = [1u8; 32];
        let payload = canonical_payload_bytes(
            WriteAuthAction::OptionOrderSubmit,
            &[("series", CanonicalValue::Str("X".to_string()))],
        );
        let deadline = 1_000_000;
        issue_test_challenge(
            &store,
            WriteAuthAction::OptionOrderSubmit,
            &account,
            nonce,
            deadline + 1,
            None,
        )
        .await;
        let envelope = build_envelope(
            &signing_key,
            WriteAuthAction::OptionOrderSubmit,
            &account,
            &payload,
            nonce,
            deadline,
            None,
        );
        let verified = verify_and_claim(
            &store,
            &envelope,
            WriteAuthAction::OptionOrderSubmit,
            &account,
            &payload,
            WRITE_AUTH_DOMAIN_CHAIN_ID,
            deadline - 1,
        )
        .await
        .expect("verify");
        assert_eq!(verified.action, WriteAuthAction::OptionOrderSubmit);
        assert!(verified.idempotent_resource_id.is_none());
    }

    #[tokio::test]
    async fn replay_with_same_payload_returns_idempotent() {
        let (signing_key, account) = test_keypair();
        let store = InMemoryChallengeStore::new();
        let nonce = [2u8; 32];
        let payload = canonical_payload_bytes(WriteAuthAction::OptionOrderSubmit, &[]);
        let deadline = 2_000_000;
        issue_test_challenge(
            &store,
            WriteAuthAction::OptionOrderSubmit,
            &account,
            nonce,
            deadline + 1,
            None,
        )
        .await;
        let envelope = build_envelope(
            &signing_key,
            WriteAuthAction::OptionOrderSubmit,
            &account,
            &payload,
            nonce,
            deadline,
            None,
        );
        let first = verify_and_claim(
            &store,
            &envelope,
            WriteAuthAction::OptionOrderSubmit,
            &account,
            &payload,
            WRITE_AUTH_DOMAIN_CHAIN_ID,
            deadline - 1,
        )
        .await
        .expect("first");
        assert!(first.idempotent_resource_id.is_none());
        // Pretend the mutation produced this resource_id; attach it.
        store
            .attach_resource(nonce, "order-abc")
            .await
            .expect("attach");
        let second = verify_and_claim(
            &store,
            &envelope,
            WriteAuthAction::OptionOrderSubmit,
            &account,
            &payload,
            WRITE_AUTH_DOMAIN_CHAIN_ID,
            deadline - 1,
        )
        .await
        .expect("second");
        assert_eq!(second.idempotent_resource_id.as_deref(), Some("order-abc"));
    }

    #[tokio::test]
    async fn replay_with_different_payload_rejected() {
        let (signing_key, account) = test_keypair();
        let store = InMemoryChallengeStore::new();
        let nonce = [3u8; 32];
        let payload_a = canonical_payload_bytes(
            WriteAuthAction::OptionOrderSubmit,
            &[("p", CanonicalValue::U64(1))],
        );
        let payload_b = canonical_payload_bytes(
            WriteAuthAction::OptionOrderSubmit,
            &[("p", CanonicalValue::U64(2))],
        );
        let deadline = 3_000_000;
        issue_test_challenge(
            &store,
            WriteAuthAction::OptionOrderSubmit,
            &account,
            nonce,
            deadline + 1,
            None,
        )
        .await;
        let envelope_a = build_envelope(
            &signing_key,
            WriteAuthAction::OptionOrderSubmit,
            &account,
            &payload_a,
            nonce,
            deadline,
            None,
        );
        verify_and_claim(
            &store,
            &envelope_a,
            WriteAuthAction::OptionOrderSubmit,
            &account,
            &payload_a,
            WRITE_AUTH_DOMAIN_CHAIN_ID,
            deadline - 1,
        )
        .await
        .expect("a");
        // Now try to reuse the nonce with payload_b. The signature for payload_b would not match the envelope_a signature, so we'd reject on InvalidSignature before even reaching the claim. But the operator-attacker case is: same envelope with different payload bytes on the wire — that fails PayloadMismatch via the digest.
        let result = verify_and_claim(
            &store,
            &envelope_a,
            WriteAuthAction::OptionOrderSubmit,
            &account,
            &payload_b,
            WRITE_AUTH_DOMAIN_CHAIN_ID,
            deadline - 1,
        )
        .await;
        // The signature in envelope_a was signed over payload_a, so re-verifying against payload_b produces a different digest and the recovered signer mismatches.
        assert!(matches!(
            result,
            Err(WriteAuthError::SignerMismatch) | Err(WriteAuthError::InvalidSignature)
        ));
    }

    #[tokio::test]
    async fn expired_envelope_rejected() {
        let (signing_key, account) = test_keypair();
        let store = InMemoryChallengeStore::new();
        let nonce = [4u8; 32];
        let payload = canonical_payload_bytes(WriteAuthAction::OptionOrderSubmit, &[]);
        let deadline = 100;
        issue_test_challenge(
            &store,
            WriteAuthAction::OptionOrderSubmit,
            &account,
            nonce,
            deadline + 1,
            None,
        )
        .await;
        let envelope = build_envelope(
            &signing_key,
            WriteAuthAction::OptionOrderSubmit,
            &account,
            &payload,
            nonce,
            deadline,
            None,
        );
        let err = verify_and_claim(
            &store,
            &envelope,
            WriteAuthAction::OptionOrderSubmit,
            &account,
            &payload,
            WRITE_AUTH_DOMAIN_CHAIN_ID,
            deadline + 5,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, WriteAuthError::Expired));
    }

    #[tokio::test]
    async fn wrong_chain_rejected() {
        let (signing_key, account) = test_keypair();
        let store = InMemoryChallengeStore::new();
        let nonce = [5u8; 32];
        let payload = canonical_payload_bytes(WriteAuthAction::OptionOrderSubmit, &[]);
        let deadline = 100_000;
        issue_test_challenge(
            &store,
            WriteAuthAction::OptionOrderSubmit,
            &account,
            nonce,
            deadline + 1,
            None,
        )
        .await;
        let envelope = build_envelope(
            &signing_key,
            WriteAuthAction::OptionOrderSubmit,
            &account,
            &payload,
            nonce,
            deadline,
            None,
        );
        let err = verify_and_claim(
            &store,
            &envelope,
            WriteAuthAction::OptionOrderSubmit,
            &account,
            &payload,
            1,
            0,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, WriteAuthError::ChainMismatch));
    }

    #[tokio::test]
    async fn unknown_nonce_rejected() {
        let (signing_key, account) = test_keypair();
        let store = InMemoryChallengeStore::new();
        let nonce = [6u8; 32];
        let payload = canonical_payload_bytes(WriteAuthAction::OptionOrderSubmit, &[]);
        let deadline = 100_000;
        // Note: do NOT issue the nonce.
        let envelope = build_envelope(
            &signing_key,
            WriteAuthAction::OptionOrderSubmit,
            &account,
            &payload,
            nonce,
            deadline,
            None,
        );
        let err = verify_and_claim(
            &store,
            &envelope,
            WriteAuthAction::OptionOrderSubmit,
            &account,
            &payload,
            WRITE_AUTH_DOMAIN_CHAIN_ID,
            0,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, WriteAuthError::NonceNotFound));
    }

    #[tokio::test]
    async fn action_swap_rejected() {
        let (signing_key, account) = test_keypair();
        let store = InMemoryChallengeStore::new();
        let nonce = [7u8; 32];
        let payload = canonical_payload_bytes(WriteAuthAction::OptionOrderSubmit, &[]);
        let deadline = 100_000;
        issue_test_challenge(
            &store,
            WriteAuthAction::OptionOrderCancel,
            &account,
            nonce,
            deadline + 1,
            None,
        )
        .await;
        let envelope = build_envelope(
            &signing_key,
            WriteAuthAction::OptionOrderSubmit,
            &account,
            &payload,
            nonce,
            deadline,
            None,
        );
        let err = verify_and_claim(
            &store,
            &envelope,
            WriteAuthAction::OptionOrderCancel,
            &account,
            &payload,
            WRITE_AUTH_DOMAIN_CHAIN_ID,
            0,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, WriteAuthError::ActionMismatch));
    }

    #[tokio::test]
    async fn account_swap_rejected() {
        let (signing_key, account) = test_keypair();
        let store = InMemoryChallengeStore::new();
        let nonce = [8u8; 32];
        let payload = canonical_payload_bytes(WriteAuthAction::OptionOrderSubmit, &[]);
        let deadline = 100_000;
        let other_account = AccountId::new("0x0000000000000000000000000000000000000001");
        issue_test_challenge(
            &store,
            WriteAuthAction::OptionOrderSubmit,
            &account,
            nonce,
            deadline + 1,
            None,
        )
        .await;
        let envelope = build_envelope(
            &signing_key,
            WriteAuthAction::OptionOrderSubmit,
            &account,
            &payload,
            nonce,
            deadline,
            None,
        );
        let err = verify_and_claim(
            &store,
            &envelope,
            WriteAuthAction::OptionOrderSubmit,
            &other_account,
            &payload,
            WRITE_AUTH_DOMAIN_CHAIN_ID,
            0,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, WriteAuthError::SignerMismatch));
    }

    #[tokio::test]
    async fn malformed_signature_rejected() {
        let (_, account) = test_keypair();
        let store = InMemoryChallengeStore::new();
        let envelope = AuthorizationEnvelope {
            action: "OPTION_ORDER_SUBMIT".to_string(),
            account: account.clone(),
            nonce: nonce_to_hex(&[9u8; 32]),
            deadline_ms: 100_000,
            signature: "not-a-signature".to_string(),
            idempotency_key: None,
            version: None,
        };
        let payload = canonical_payload_bytes(WriteAuthAction::OptionOrderSubmit, &[]);
        let err = verify_and_claim(
            &store,
            &envelope,
            WriteAuthAction::OptionOrderSubmit,
            &account,
            &payload,
            WRITE_AUTH_DOMAIN_CHAIN_ID,
            0,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, WriteAuthError::InvalidSignature));
    }
}
