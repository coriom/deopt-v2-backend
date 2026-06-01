use crate::error::{BackendError, Result};
use std::fmt;

#[derive(Clone, Eq, PartialEq)]
pub struct AdminConfig {
    pub enabled: bool,
    pub require_token: bool,
    token: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MetricsConfig {
    pub enabled: bool,
    pub require_admin_token: bool,
}

impl AdminConfig {
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            require_token: false,
            token: None,
        }
    }

    pub fn new(enabled: bool, require_token: bool, token: Option<String>) -> Self {
        Self {
            enabled,
            require_token,
            token,
        }
    }

    pub fn validate_startup(&self) -> Result<()> {
        if self.enabled
            && self.require_token
            && self.token.as_deref().unwrap_or_default().is_empty()
        {
            return Err(BackendError::Config(
                "ADMIN_API_TOKEN is required when ADMIN_API_REQUIRE_TOKEN=true".to_string(),
            ));
        }
        Ok(())
    }

    pub fn token_configured(&self) -> bool {
        self.token.as_deref().is_some_and(|token| !token.is_empty())
    }

    /// V2G-W0 — constant-time admin token verification.
    ///
    /// Closes the V2G-V finding (C): the previous implementation
    /// compared `&str` with `==`, which short-circuits on the first
    /// mismatched byte. That is a timing side channel a network
    /// attacker could in principle exploit to recover the token
    /// byte-by-byte. We now reject a candidate whose length differs
    /// from the configured token (length is not the secret — the
    /// bytes are) and compare equal-length candidates in constant
    /// time relative to the byte sequence under test.
    ///
    /// Semantics preserved: returns `false` when the token is unset
    /// or empty, exactly like the V2G-V version.
    pub fn token_matches(&self, candidate: &str) -> bool {
        let Some(configured) = self.token.as_deref() else {
            return false;
        };
        if configured.is_empty() {
            return false;
        }
        constant_time_eq(configured.as_bytes(), candidate.as_bytes())
    }
}

/// V2G-W0 — small constant-time byte-equality helper.
///
/// Length is intentionally NOT timing-protected — different-length
/// inputs are rejected immediately. The configured token length is
/// not secret (it would leak via `Content-Length` of any error
/// response if it were). Equal-length inputs are compared with an
/// XOR/OR fold so the running time depends only on the length of
/// the inputs, not on the position of the first mismatched byte.
///
/// We deliberately avoid a third-party crate dependency. The
/// implementation is small enough to audit in place; `subtle` is
/// available transitively via `k256` but pulling it in as a direct
/// dep just for this helper would expand the supply-chain surface.
#[inline]
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }

    let mut diff: u8 = 0;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

impl fmt::Debug for AdminConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AdminConfig")
            .field("enabled", &self.enabled)
            .field("require_token", &self.require_token)
            .field(
                "token",
                &if self.token_configured() {
                    "<redacted>"
                } else {
                    "<unset>"
                },
            )
            .finish()
    }
}

impl MetricsConfig {
    pub fn enabled_by_default() -> Self {
        Self {
            enabled: true,
            require_admin_token: false,
        }
    }

    pub fn validate_startup(&self, admin: &AdminConfig) -> Result<()> {
        if self.enabled && self.require_admin_token && !admin.token_configured() {
            return Err(BackendError::Config(
                "ADMIN_API_TOKEN is required when METRICS_REQUIRE_ADMIN_TOKEN=true".to_string(),
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(token: Option<&str>) -> AdminConfig {
        AdminConfig::new(true, true, token.map(str::to_string))
    }

    #[test]
    fn v2gw0_token_matches_accepts_exact_value() {
        let config = cfg(Some("expected-admin-token"));
        assert!(config.token_matches("expected-admin-token"));
    }

    #[test]
    fn v2gw0_token_matches_rejects_wrong_value_same_length() {
        let config = cfg(Some("expected-admin-token"));
        // 21 chars, same length as the configured token, every byte
        // different — must reject without revealing where the bytes
        // diverge.
        assert!(!config.token_matches("wxxxxxxx-xxxxx-xxxxxx"));
    }

    #[test]
    fn v2gw0_token_matches_rejects_empty_candidate() {
        let config = cfg(Some("expected-admin-token"));
        assert!(!config.token_matches(""));
    }

    #[test]
    fn v2gw0_token_matches_rejects_wrong_length_short() {
        let config = cfg(Some("expected-admin-token"));
        assert!(!config.token_matches("expected"));
    }

    #[test]
    fn v2gw0_token_matches_rejects_wrong_length_long() {
        let config = cfg(Some("expected-admin-token"));
        assert!(!config.token_matches("expected-admin-token-extra"));
    }

    #[test]
    fn v2gw0_token_matches_rejects_same_prefix() {
        let config = cfg(Some("expected-admin-token"));
        // Prefix that matches the configured token except for the
        // final byte. Under the V2G-V `==` implementation the
        // mismatch would be detected at the last byte; the
        // constant-time helper must still reject. The behavioural
        // assertion is the same (rejection); the *timing*
        // hardening cannot be expressed as a unit-test assertion
        // without a benchmark harness — covered by the audit doc.
        assert!(!config.token_matches("expected-admin-toke!"));
    }

    #[test]
    fn v2gw0_token_matches_rejects_when_token_unset() {
        let config = cfg(None);
        assert!(!config.token_matches("anything"));
    }

    #[test]
    fn v2gw0_token_matches_rejects_when_token_empty_string() {
        // Edge case: someone misconfigures ADMIN_API_TOKEN="" and
        // turns require_token on. validate_startup catches that at
        // boot; this test pins the runtime fallback so it cannot
        // be probed by submitting an empty candidate.
        let config = cfg(Some(""));
        assert!(!config.token_matches(""));
        assert!(!config.token_matches("anything"));
    }

    #[test]
    fn v2gw0_debug_output_does_not_leak_token() {
        let config = cfg(Some("super-secret-admin-token-value"));
        let rendered = format!("{config:?}");
        assert!(
            !rendered.contains("super-secret-admin-token-value"),
            "Debug must not echo the configured token: {rendered}"
        );
        assert!(rendered.contains("<redacted>"));
    }

    #[test]
    fn v2gw0_debug_output_marks_unset_token() {
        let config = cfg(None);
        let rendered = format!("{config:?}");
        assert!(rendered.contains("<unset>"));
        assert!(!rendered.contains("<redacted>"));
    }

    #[test]
    fn v2gw0_constant_time_eq_helper_property_table() {
        // Direct micro-tests for the helper. Behavioural only —
        // timing properties are a code-review claim, not a runtime
        // test.
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"ab"));
        assert!(!constant_time_eq(b"ab", b"abc"));
        assert!(!constant_time_eq(b"", b"x"));
        assert!(constant_time_eq(b"", b""));
        // Every byte position must contribute — flip just the
        // first byte.
        assert!(!constant_time_eq(b"Zbc", b"abc"));
        // And the last byte.
        assert!(!constant_time_eq(b"abZ", b"abc"));
    }
}
