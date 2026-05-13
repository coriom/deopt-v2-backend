use crate::error::{BackendError, Result};
use std::fmt;

#[derive(Clone, Eq, PartialEq)]
pub struct AdminConfig {
    pub enabled: bool,
    pub require_token: bool,
    token: Option<String>,
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

    pub fn token_matches(&self, candidate: &str) -> bool {
        self.token.as_deref() == Some(candidate)
    }
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
