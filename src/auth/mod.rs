//! ACCOUNT-WRITE-AUTH-HARDENING-V1
//!
//! Central authorization module for HTTP account-write surfaces.
//!
//! Every protected public mutation route binds a per-action EIP-712
//! signature to: action, account, canonical payload, server-issued
//! nonce, deadline, environment, and optional idempotency key. The
//! signed domain uses the `salt` variant (no verifyingContract): this
//! authorization is verified by the DeOpt backend API and MUST NOT
//! collide with any on-chain settlement signature.
//!
//! See `docs/ACCOUNT_WRITE_AUTH_HARDENING_V1_RESULT.md` for the full
//! schema and replay-protection guarantees.

pub mod write_authorization;

pub use write_authorization::{
    canonical_payload_bytes, write_auth_domain, write_auth_eip712_digest, AuthorizationEnvelope,
    CanonicalValue, ChallengeRecord, ChallengeStatus, UsedNonceV2Store, V2NonceClaimOutcome,
    WriteAuthAction, WriteAuthChallengeStore, WriteAuthError, WriteAuthVerified, ENVIRONMENT,
    WRITE_AUTH_DOMAIN_SALT, WRITE_AUTH_DOMAIN_VERSION, WRITE_AUTH_PRIMARY_TYPE,
};
