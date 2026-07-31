//! Exact serialization helpers.
//!
//! Frozen rules:
//! - Addresses lower-case hex, `0x`-prefixed.
//! - bytes32 lower-case hex, `0x`-prefixed.
//! - uint128 / uint256 economic amounts are always emitted as decimal strings.
//! - Never coerce integers to JSON floats.

use crate::api::hybrid_v2_read::errors::{ApiError, ApiErrorCode};

/// Normalise + validate an ethereum address string. Returns the lowercase
/// `0x` + 40-char-hex form.
pub fn parse_address(s: &str) -> Result<String, ApiError> {
    let stripped = s.trim().trim_start_matches("0x").trim_start_matches("0X");
    if stripped.len() != 40 {
        return Err(ApiError::new(
            ApiErrorCode::InvalidAddress,
            format!("expected 40 hex chars, got {}", stripped.len()),
        ));
    }
    if !stripped.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(ApiError::new(
            ApiErrorCode::InvalidAddress,
            "non-hex character in address",
        ));
    }
    let lower = stripped.to_ascii_lowercase();
    if lower.chars().all(|c| c == '0') {
        return Err(ApiError::new(
            ApiErrorCode::Account0Invalid,
            "zero address is not a user account",
        ));
    }
    Ok(format!("0x{}", lower))
}

/// Normalise + validate a bytes32 hex string. Returns lowercase `0x` + 64-char-hex.
pub fn parse_bytes32(s: &str) -> Result<String, ApiError> {
    let stripped = s.trim().trim_start_matches("0x").trim_start_matches("0X");
    if stripped.len() != 64 {
        return Err(ApiError::new(
            ApiErrorCode::InvalidSubKey,
            format!("expected 64 hex chars, got {}", stripped.len()),
        ));
    }
    if !stripped.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(ApiError::new(
            ApiErrorCode::InvalidSubKey,
            "non-hex character in subkey",
        ));
    }
    Ok(format!("0x{}", stripped.to_ascii_lowercase()))
}

/// Lowercase a hex address without validation. Used when normalising
/// projection-side keys for comparison.
pub fn to_lower_hex(s: &str) -> String {
    s.to_ascii_lowercase()
}
