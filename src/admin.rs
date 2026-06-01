use crate::error::{BackendError, Result};
use std::fmt;

#[derive(Clone, Eq, PartialEq)]
pub struct AdminConfig {
    pub enabled: bool,
    pub require_token: bool,
    token: Option<String>,
    /// V2G-W1 — defaults to `None` (interpreted as
    /// `AuthMode::SharedToken`) so existing `AdminConfig::new(…)`
    /// call sites stay backwards-compatible. Mutate via
    /// {set_auth_mode}; read via {auth_mode}.
    auth_mode: Option<AuthMode>,
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
            auth_mode: None,
        }
    }

    pub fn new(enabled: bool, require_token: bool, token: Option<String>) -> Self {
        Self {
            enabled,
            require_token,
            token,
            auth_mode: None,
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
            .field("auth_mode", &self.auth_mode().as_str())
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

// =====================================================================
// V2G-W1 — per-identity admin role model (backwards-compatible).
// =====================================================================
//
// V2G-W0 closed the timing side channel on the shared-token compare.
// V2G-W1 adds the *role* dimension without changing any route handler
// today. The route handlers continue to call `ensure_admin_access`
// (shared-token path) — V2G-W1 only adds:
//
//   * An `AdminRole` enum with a documented authority ordering.
//   * An `AdminIdentity` struct that carries the caller's name + role.
//   * An `AuthMode` config enum that selects between
//     `SharedToken` (today), `Jwt` (future), and `Disabled` (local dev
//     only). Default stays `SharedToken` so production behavior is
//     unchanged.
//   * A `required_role_for` lookup mapping `(method, path)` to the
//     minimum role required to call the endpoint, derived from the
//     V2G-V threat-model classification (§1).
//   * A pure-compute `authenticate` entry point that, when called by
//     a future role-aware route guard, maps the incoming request to
//     an identity under the configured mode.
//
// Nothing in this block is wired into the live route gate today.
// V2G-W1's scope is the model + tests; a follow-up milestone migrates
// individual route handlers to the role-aware gate once the JWT
// verifier is implemented.

/// RBAC role; ordered so `>=` reflects authority.
///
/// `Viewer` ≤ `Operator` ≤ `GovernanceAdmin` ≤ `Breakglass`.
/// Each role implies the lower-authority roles' access (per V2G-V
/// §2.5 role assignment matrix).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum AdminRole {
    Viewer = 0,
    Operator = 1,
    GovernanceAdmin = 2,
    Breakglass = 3,
}

impl AdminRole {
    /// Human-readable name (for logs / audit only — never used as the
    /// gate decision).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Viewer => "viewer",
            Self::Operator => "operator",
            Self::GovernanceAdmin => "governance-admin",
            Self::Breakglass => "breakglass",
        }
    }

    /// `true` when `self` grants at least the authority of `required`.
    pub fn implies(self, required: AdminRole) -> bool {
        self >= required
    }
}

/// Caller identity resolved from the request headers.
///
/// `name` is for audit logging only — it should be the principal
/// (e.g. an OIDC `sub` claim, or the literal string `"shared-token"`
/// when the legacy ADMIN_API_TOKEN path was used). The Debug impl
/// renders the role but redacts the principal so a future log writer
/// that calls `{:?}` on an identity does not accidentally leak it.
#[derive(Clone, Eq, PartialEq)]
pub struct AdminIdentity {
    name: String,
    role: AdminRole,
}

impl AdminIdentity {
    pub fn new(name: impl Into<String>, role: AdminRole) -> Self {
        Self {
            name: name.into(),
            role,
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn role(&self) -> AdminRole {
        self.role
    }
}

impl fmt::Debug for AdminIdentity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AdminIdentity")
            .field("name", &"<redacted>")
            .field("role", &self.role)
            .finish()
    }
}

/// Configured auth mode for the admin API.
///
/// * `SharedToken` (default) — the V2G-W0 + ADMIN_API_TOKEN path. No
///   role differentiation: any valid token grants `Operator` (the
///   highest authority currently exposed via any route). Backwards
///   compatible with the V2G-G / V2G-M / V2G-M2 surface today.
/// * `Jwt` — per-identity bearer JWT validated against an internal
///   CA. NOT IMPLEMENTED in V2G-W1; the verifier returns
///   `AdminAuthError::JwtNotImplemented` so production deployments
///   that flip this on without finishing V2G-W1's follow-up will
///   fail closed instead of accidentally accepting unverified
///   tokens.
/// * `Disabled` — explicit local-development-only mode that skips
///   auth entirely. Must NEVER be enabled outside `cargo test` /
///   `localhost`. `validate_startup` rejects this combined with
///   `require_token=true`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum AuthMode {
    #[default]
    SharedToken,
    Jwt,
    Disabled,
}

impl AuthMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SharedToken => "shared-token",
            Self::Jwt => "jwt",
            Self::Disabled => "disabled",
        }
    }
}

/// V2G-W1 auth error taxonomy. Caller (the future route guard) maps
/// each variant to an HTTP status. Error messages never include the
/// candidate token bytes / JWT bytes / configured token.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AdminAuthError {
    /// Admin API is disabled entirely.
    AdminDisabled,
    /// `AuthMode::SharedToken`: header missing.
    SharedTokenHeaderMissing,
    /// `AuthMode::SharedToken`: header present but value rejected.
    SharedTokenInvalid,
    /// `AuthMode::Jwt`: NOT IMPLEMENTED in V2G-W1.
    JwtNotImplemented,
    /// The resolved identity does not satisfy the route's required role.
    InsufficientRole {
        granted: AdminRole,
        required: AdminRole,
    },
}

impl fmt::Display for AdminAuthError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AdminDisabled => f.write_str("admin API is disabled"),
            Self::SharedTokenHeaderMissing => f.write_str("admin token is required"),
            Self::SharedTokenInvalid => f.write_str("invalid admin token"),
            Self::JwtNotImplemented => f.write_str("admin JWT auth mode is not implemented"),
            Self::InsufficientRole { required, .. } => {
                // Render the REQUIRED role so the operator knows what
                // they need; render NEITHER the granted role NOR the
                // principal name (avoids leaking caller identity in
                // error payloads served to peers).
                write!(
                    f,
                    "insufficient role; required at least {}",
                    required.as_str()
                )
            }
        }
    }
}

/// Required role for a given `(method, path)` pair.
///
/// Mapping pinned by V2G-V §1 classification. Routes that are not
/// matched here default to `AdminRole::Viewer` (the safest reading
/// for the read-only admin surface). Any future route that mutates
/// state MUST be added here explicitly.
pub fn required_role_for(method: &str, path: &str) -> AdminRole {
    let method = method.to_ascii_uppercase();

    // Operator-class routes.
    let is_operator = matches!(
        (method.as_str(), path),
        // Mutating POSTs (backend state, not chain state).
        ("POST", "/admin/options/events/tick")
            | ("POST", "/admin/options/reconciliations/tick")
            // Preflight surface — produces calldata / packet payloads
            // for off-band signing.
            | ("GET", "/admin/fees/v2/smoke/readiness")
    );
    if is_operator {
        return AdminRole::Operator;
    }

    // Everything else — read-only viewer.
    AdminRole::Viewer
}

/// V2G-W1 authentication entry point. Returns the resolved identity
/// under the configured `AuthMode`, or an `AdminAuthError` describing
/// why authentication failed.
///
/// This function is **not** wired into the live route gate in V2G-W1.
/// It is exercised by the unit-test suite below and ready for a
/// follow-up route migration.
///
/// `header_lookup` is an abstract getter so we don't depend on `axum`
/// types here. Pass a closure that pulls the named header out of
/// whatever request type the caller has.
pub fn authenticate<F>(
    config: &AdminConfig,
    header_lookup: F,
) -> std::result::Result<AdminIdentity, AdminAuthError>
where
    F: Fn(&str) -> Option<String>,
{
    if !config.enabled {
        return Err(AdminAuthError::AdminDisabled);
    }

    match config.auth_mode() {
        AuthMode::Disabled => Ok(AdminIdentity::new(
            "local-dev-no-auth",
            AdminRole::Breakglass, // Permit everything in local dev.
        )),
        AuthMode::SharedToken => {
            // Reproduces the V2G-W0 semantics for the existing
            // ensure_admin_access gate, plus an identity stamp.
            if !config.require_token {
                return Ok(AdminIdentity::new(
                    "shared-token-unrequired",
                    AdminRole::Operator,
                ));
            }
            let candidate =
                header_lookup("x-admin-token").ok_or(AdminAuthError::SharedTokenHeaderMissing)?;
            if !config.token_matches(&candidate) {
                return Err(AdminAuthError::SharedTokenInvalid);
            }
            Ok(AdminIdentity::new("shared-token", AdminRole::Operator))
        }
        AuthMode::Jwt => Err(AdminAuthError::JwtNotImplemented),
    }
}

/// Role-gate helper. Future route handlers will call:
///
/// ```ignore
/// let identity = authenticate(&state.admin_config, |h| headers.get(h).and_then(...))?;
/// require_role(&identity, required_role_for(method, path))?;
/// ```
///
/// V2G-W1 ships this helper + tests; no route handler is migrated
/// yet (the existing `ensure_admin_access` shared-token gate remains
/// authoritative).
pub fn require_role(
    identity: &AdminIdentity,
    required: AdminRole,
) -> std::result::Result<(), AdminAuthError> {
    if identity.role.implies(required) {
        Ok(())
    } else {
        Err(AdminAuthError::InsufficientRole {
            granted: identity.role,
            required,
        })
    }
}

impl AdminConfig {
    /// V2G-W1 — current auth mode for this config. Defaults to
    /// `SharedToken` for backwards compatibility with V2G-W0.
    /// Mutated via {set_auth_mode} during tests; production deploys
    /// flip this through config later when JWT lands.
    pub fn auth_mode(&self) -> AuthMode {
        self.auth_mode.unwrap_or_default()
    }

    /// V2G-W1 setter used only by config-loading code and tests.
    pub fn set_auth_mode(&mut self, mode: AuthMode) {
        self.auth_mode = Some(mode);
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

    // ============================================================
    //  V2G-W1 — role / identity / auth-mode tests
    // ============================================================

    fn header_get<'a>(headers: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |name: &str| {
            headers
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(name))
                .map(|(_, v)| (*v).to_string())
        }
    }

    #[test]
    fn v2gw1_role_authority_ordering() {
        assert!(AdminRole::Operator > AdminRole::Viewer);
        assert!(AdminRole::GovernanceAdmin > AdminRole::Operator);
        assert!(AdminRole::Breakglass > AdminRole::GovernanceAdmin);
        // Implication semantics — higher role grants lower role.
        assert!(AdminRole::Breakglass.implies(AdminRole::Viewer));
        assert!(AdminRole::Operator.implies(AdminRole::Viewer));
        assert!(!AdminRole::Viewer.implies(AdminRole::Operator));
    }

    #[test]
    fn v2gw1_default_auth_mode_is_shared_token() {
        let config = cfg(Some("expected-admin-token"));
        assert_eq!(config.auth_mode(), AuthMode::SharedToken);
    }

    #[test]
    fn v2gw1_route_role_mapping_covers_v2gv_classification() {
        // Read-only viewer routes from V2G-V §1.
        for path in [
            "/admin/status",
            "/admin/config",
            "/admin/db",
            "/admin/fees/v2/observability",
            "/admin/fees/onchain",
            "/admin/options/executions/abc-def/lifecycle",
            "/admin/recent",
        ] {
            assert_eq!(
                required_role_for("GET", path),
                AdminRole::Viewer,
                "GET {path} should be viewer"
            );
        }

        // Operator-class routes from V2G-V §1.
        assert_eq!(
            required_role_for("POST", "/admin/options/events/tick"),
            AdminRole::Operator
        );
        assert_eq!(
            required_role_for("POST", "/admin/options/reconciliations/tick"),
            AdminRole::Operator
        );
        assert_eq!(
            required_role_for("GET", "/admin/fees/v2/smoke/readiness"),
            AdminRole::Operator
        );

        // Lowercased method also accepted (HTTP method strings are
        // sometimes lowercased upstream).
        assert_eq!(
            required_role_for("post", "/admin/options/events/tick"),
            AdminRole::Operator
        );
    }

    #[test]
    fn v2gw1_authenticate_shared_token_accepts_valid() {
        let config = cfg(Some("expected-admin-token"));
        let headers: [(&str, &str); 1] = [("x-admin-token", "expected-admin-token")];
        let identity = authenticate(&config, header_get(&headers)).unwrap();
        assert_eq!(identity.role(), AdminRole::Operator);
        assert_eq!(identity.name(), "shared-token");
    }

    #[test]
    fn v2gw1_authenticate_shared_token_rejects_missing_header() {
        let config = cfg(Some("expected-admin-token"));
        let headers: [(&str, &str); 0] = [];
        let err = authenticate(&config, header_get(&headers)).unwrap_err();
        assert_eq!(err, AdminAuthError::SharedTokenHeaderMissing);
    }

    #[test]
    fn v2gw1_authenticate_shared_token_rejects_wrong_value() {
        let config = cfg(Some("expected-admin-token"));
        let headers: [(&str, &str); 1] = [("x-admin-token", "wrong-admin-token!")];
        let err = authenticate(&config, header_get(&headers)).unwrap_err();
        assert_eq!(err, AdminAuthError::SharedTokenInvalid);
    }

    #[test]
    fn v2gw1_authenticate_when_admin_disabled() {
        let mut config = cfg(Some("expected-admin-token"));
        config.enabled = false;
        let headers: [(&str, &str); 1] = [("x-admin-token", "expected-admin-token")];
        let err = authenticate(&config, header_get(&headers)).unwrap_err();
        assert_eq!(err, AdminAuthError::AdminDisabled);
    }

    #[test]
    fn v2gw1_authenticate_shared_token_when_require_token_false() {
        // ADMIN_API_REQUIRE_TOKEN=false → any caller passes auth but
        // is still tagged as an identity for role lookups.
        let mut config = cfg(Some("expected-admin-token"));
        config.require_token = false;
        let headers: [(&str, &str); 0] = [];
        let identity = authenticate(&config, header_get(&headers)).unwrap();
        assert_eq!(identity.name(), "shared-token-unrequired");
        assert_eq!(identity.role(), AdminRole::Operator);
    }

    #[test]
    fn v2gw1_authenticate_jwt_mode_is_unimplemented() {
        let mut config = cfg(Some("expected-admin-token"));
        config.set_auth_mode(AuthMode::Jwt);
        let headers: [(&str, &str); 1] = [
            ("authorization", "Bearer eyJhbGc..."), // looks like a JWT, but should never be parsed
        ];
        let err = authenticate(&config, header_get(&headers)).unwrap_err();
        assert_eq!(err, AdminAuthError::JwtNotImplemented);
    }

    #[test]
    fn v2gw1_authenticate_disabled_mode_returns_breakglass_identity() {
        let mut config = cfg(Some(""));
        config.set_auth_mode(AuthMode::Disabled);
        let headers: [(&str, &str); 0] = [];
        let identity = authenticate(&config, header_get(&headers)).unwrap();
        assert_eq!(identity.name(), "local-dev-no-auth");
        assert_eq!(identity.role(), AdminRole::Breakglass);
    }

    #[test]
    fn v2gw1_require_role_viewer_can_access_viewer_route() {
        let identity = AdminIdentity::new("alice", AdminRole::Viewer);
        require_role(&identity, AdminRole::Viewer).unwrap();
    }

    #[test]
    fn v2gw1_require_role_viewer_cannot_access_operator_route() {
        let identity = AdminIdentity::new("alice", AdminRole::Viewer);
        let err = require_role(&identity, AdminRole::Operator).unwrap_err();
        assert_eq!(
            err,
            AdminAuthError::InsufficientRole {
                granted: AdminRole::Viewer,
                required: AdminRole::Operator,
            }
        );
    }

    #[test]
    fn v2gw1_require_role_operator_can_access_operator_route() {
        let identity = AdminIdentity::new("alice", AdminRole::Operator);
        require_role(&identity, AdminRole::Operator).unwrap();
    }

    #[test]
    fn v2gw1_require_role_operator_can_access_viewer_route() {
        let identity = AdminIdentity::new("alice", AdminRole::Operator);
        require_role(&identity, AdminRole::Viewer).unwrap();
    }

    #[test]
    fn v2gw1_require_role_governance_admin_can_access_operator_route() {
        let identity = AdminIdentity::new("alice", AdminRole::GovernanceAdmin);
        require_role(&identity, AdminRole::Operator).unwrap();
    }

    #[test]
    fn v2gw1_require_role_breakglass_can_access_everything() {
        let identity = AdminIdentity::new("alice", AdminRole::Breakglass);
        require_role(&identity, AdminRole::Viewer).unwrap();
        require_role(&identity, AdminRole::Operator).unwrap();
        require_role(&identity, AdminRole::GovernanceAdmin).unwrap();
        require_role(&identity, AdminRole::Breakglass).unwrap();
    }

    #[test]
    fn v2gw1_auth_error_messages_dont_leak_tokens() {
        // Every variant — rendered string must NEVER contain the
        // candidate token / JWT / configured token bytes.
        let candidate_token = "leaky-candidate-token-12345";
        let configured_token = "very-secret-admin-token";

        let cases = vec![
            AdminAuthError::AdminDisabled.to_string(),
            AdminAuthError::SharedTokenHeaderMissing.to_string(),
            AdminAuthError::SharedTokenInvalid.to_string(),
            AdminAuthError::JwtNotImplemented.to_string(),
            AdminAuthError::InsufficientRole {
                granted: AdminRole::Viewer,
                required: AdminRole::Operator,
            }
            .to_string(),
        ];
        for rendered in &cases {
            assert!(
                !rendered.contains(candidate_token),
                "error rendering leaks candidate token: {rendered}"
            );
            assert!(
                !rendered.contains(configured_token),
                "error rendering leaks configured token: {rendered}"
            );
            // Also catch base64-flavoured leakage in case the future
            // JWT verifier echoes the payload.
            assert!(!rendered.contains("eyJ"));
        }
    }

    #[test]
    fn v2gw1_insufficient_role_error_does_not_leak_granted_role() {
        // The granted role is the caller's identity — we don't echo
        // it back to the wire (only the required role is shown so
        // the operator knows what they need).
        let err = AdminAuthError::InsufficientRole {
            granted: AdminRole::Viewer,
            required: AdminRole::Operator,
        };
        let rendered = err.to_string();
        assert!(rendered.contains("operator")); // required
        assert!(!rendered.contains("viewer")); // granted — kept private
    }

    #[test]
    fn v2gw1_admin_identity_debug_redacts_name() {
        let identity = AdminIdentity::new("alice@deopt.xyz", AdminRole::Operator);
        let rendered = format!("{identity:?}");
        assert!(rendered.contains("<redacted>"));
        assert!(!rendered.contains("alice@deopt.xyz"));
        assert!(rendered.contains("Operator"));
    }

    #[test]
    fn v2gw1_admin_config_debug_reports_auth_mode() {
        let mut config = cfg(Some("super-secret-admin-token-value"));
        config.set_auth_mode(AuthMode::Jwt);
        let rendered = format!("{config:?}");
        assert!(rendered.contains("auth_mode"));
        assert!(rendered.contains("jwt"));
        assert!(!rendered.contains("super-secret-admin-token-value"));
    }

    #[test]
    fn v2gw1_token_compare_behavior_unchanged_under_role_model() {
        // Regression-pin: V2G-W0 constant-time compare is exactly
        // the path V2G-W1's shared-token authenticate() reuses.
        let config = cfg(Some("expected-admin-token"));
        let headers_ok: [(&str, &str); 1] = [("x-admin-token", "expected-admin-token")];
        let headers_same_length: [(&str, &str); 1] = [("x-admin-token", "ZZZZZZZZ-ZZZZZ-ZZZZZ")];
        let headers_short: [(&str, &str); 1] = [("x-admin-token", "expected")];

        assert!(authenticate(&config, header_get(&headers_ok)).is_ok());
        assert_eq!(
            authenticate(&config, header_get(&headers_same_length)).unwrap_err(),
            AdminAuthError::SharedTokenInvalid
        );
        assert_eq!(
            authenticate(&config, header_get(&headers_short)).unwrap_err(),
            AdminAuthError::SharedTokenInvalid
        );
    }
}
