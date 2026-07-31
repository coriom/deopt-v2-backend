//! Versioned opaque cursor for deterministic pagination.
//!
//! Wire form is `base64url(json)`. The cursor binds to:
//! - a version (v1 today)
//! - the deployment id
//! - the consistency mode
//! - a stable filter hash (SHA-256 truncated hex)
//! - the previous page's terminal key
//! - the indexer's block hash at page-issue time (for stale detection under
//!   consistency=indexed)
//!
//! Callers passing a cursor whose deployment / consistency / filter don't
//! match are rejected with `InvalidCursor`. Callers passing a cursor whose
//! bound block hash is no longer canonical are rejected with `StaleCursor`
//! (typed 409 CONFLICT so clients know to restart from page 1).

use crate::api::hybrid_v2_read::errors::{ApiError, ApiErrorCode};
use crate::api::hybrid_v2_read::metadata::ConsistencyMode;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const CURSOR_VERSION: u8 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CursorBinding {
    pub deployment_id: u64,
    pub consistency: ConsistencyMode,
    pub filter_hash: String,
    pub indexed_head_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cursor {
    pub version: u8,
    pub binding: CursorBinding,
    /// Last observed key on the previous page. Handler decides the shape
    /// (block/tx/log tuple, order hash, etc.) — kept opaque here.
    pub last_key: String,
}

#[derive(Debug, Clone)]
pub enum CursorError {
    Malformed,
    DeploymentMismatch,
    ConsistencyMismatch,
    FilterMismatch,
    Stale,
}

impl Cursor {
    pub fn encode(&self) -> String {
        let bytes = serde_json::to_vec(self).unwrap_or_default();
        base64_url_encode(&bytes)
    }
    pub fn decode(s: &str) -> Result<Self, ApiError> {
        let bytes = base64_url_decode(s).map_err(|_| {
            ApiError::new(ApiErrorCode::InvalidCursor, "malformed base64url payload")
        })?;
        serde_json::from_slice::<Cursor>(&bytes).map_err(|e| {
            ApiError::new(
                ApiErrorCode::InvalidCursor,
                format!("cursor JSON invalid: {}", e),
            )
        })
    }

    pub fn verify(&self, binding: &CursorBinding) -> Result<(), ApiError> {
        if self.version != CURSOR_VERSION {
            return Err(ApiError::new(
                ApiErrorCode::InvalidCursor,
                format!("unsupported cursor version {}", self.version),
            ));
        }
        if self.binding.deployment_id != binding.deployment_id {
            return Err(ApiError::new(
                ApiErrorCode::InvalidCursor,
                "cursor bound to a different deployment",
            ));
        }
        if self.binding.consistency != binding.consistency {
            return Err(ApiError::new(
                ApiErrorCode::InvalidCursor,
                "cursor bound to a different consistency mode",
            ));
        }
        if self.binding.filter_hash != binding.filter_hash {
            return Err(ApiError::new(
                ApiErrorCode::InvalidCursor,
                "cursor bound to a different filter",
            ));
        }
        if binding.consistency == ConsistencyMode::Indexed
            && !self
                .binding
                .indexed_head_hash
                .eq_ignore_ascii_case(&binding.indexed_head_hash)
            && !self.binding.indexed_head_hash.is_empty()
        {
            // For `indexed` consistency the cursor's block hash must still
            // match — otherwise the underlying data may have reorged.
            return Err(ApiError::new(
                ApiErrorCode::StaleCursor,
                "cursor bound to an orphaned block; restart pagination",
            ));
        }
        Ok(())
    }
}

/// Deterministic hash of the filter payload.
pub fn filter_hash(payload: &serde_json::Value) -> String {
    let mut hasher = Sha256::new();
    hasher.update(payload.to_string().as_bytes());
    let digest = hasher.finalize();
    let mut out = String::with_capacity(32);
    for b in digest.iter().take(16) {
        out.push_str(&format!("{:02x}", b));
    }
    out
}

// -----------------------------------------------------------------
//                        BASE64URL (no padding)
// -----------------------------------------------------------------

const B64_URL_ALPHABET: &[u8; 64] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

pub fn base64_url_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity((bytes.len() + 2) / 3 * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0];
        let b1 = chunk.get(1).copied().unwrap_or(0);
        let b2 = chunk.get(2).copied().unwrap_or(0);
        let n = ((b0 as u32) << 16) | ((b1 as u32) << 8) | (b2 as u32);
        out.push(B64_URL_ALPHABET[((n >> 18) & 0x3f) as usize] as char);
        out.push(B64_URL_ALPHABET[((n >> 12) & 0x3f) as usize] as char);
        if chunk.len() >= 2 {
            out.push(B64_URL_ALPHABET[((n >> 6) & 0x3f) as usize] as char);
        }
        if chunk.len() == 3 {
            out.push(B64_URL_ALPHABET[(n & 0x3f) as usize] as char);
        }
    }
    out
}

pub fn base64_url_decode(s: &str) -> Result<Vec<u8>, ()> {
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let bytes = s.as_bytes();
    let mut buf: u32 = 0;
    let mut bits: u32 = 0;
    for &b in bytes {
        let v = decode_char(b).ok_or(())?;
        buf = (buf << 6) | (v as u32);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            let byte = ((buf >> bits) & 0xff) as u8;
            out.push(byte);
        }
    }
    Ok(out)
}

fn decode_char(b: u8) -> Option<u8> {
    let idx = B64_URL_ALPHABET.iter().position(|c| *c == b)?;
    Some(idx as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64url_roundtrip() {
        for s in ["", "a", "abc", "hello world", "\x00\x7f"] {
            let enc = base64_url_encode(s.as_bytes());
            let dec = base64_url_decode(&enc).unwrap();
            assert_eq!(dec, s.as_bytes());
        }
    }

    #[test]
    fn cursor_roundtrip_and_binding_check() {
        let cursor = Cursor {
            version: 1,
            binding: CursorBinding {
                deployment_id: 1,
                consistency: ConsistencyMode::Indexed,
                filter_hash: "abc".into(),
                indexed_head_hash: "0xb1".into(),
            },
            last_key: "key-1".into(),
        };
        let enc = cursor.encode();
        let dec = Cursor::decode(&enc).unwrap();
        assert_eq!(cursor, dec);
        // Filter mismatch rejected.
        let wrong = CursorBinding {
            deployment_id: 1,
            consistency: ConsistencyMode::Indexed,
            filter_hash: "different".into(),
            indexed_head_hash: "0xb1".into(),
        };
        assert!(dec.verify(&wrong).is_err());
        // Stale cursor rejected.
        let stale = CursorBinding {
            deployment_id: 1,
            consistency: ConsistencyMode::Indexed,
            filter_hash: "abc".into(),
            indexed_head_hash: "0xb2".into(),
        };
        let err = dec.verify(&stale).unwrap_err();
        assert_eq!(err.code, ApiErrorCode::StaleCursor);
    }
}
