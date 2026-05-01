use super::SignedOrder;
use crate::error::{BackendError, Result};
use serde::{Deserialize, Serialize};
use std::str::FromStr;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SignatureVerificationMode {
    #[default]
    Disabled,
    Strict,
}

impl FromStr for SignatureVerificationMode {
    type Err = BackendError;

    fn from_str(value: &str) -> Result<Self> {
        match value {
            "disabled" => Ok(Self::Disabled),
            "strict" => Ok(Self::Strict),
            other => Err(BackendError::Config(format!(
                "invalid SIGNATURE_VERIFICATION_MODE: {other}"
            ))),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct SignatureVerifier;

impl SignatureVerifier {
    pub fn verify(order: &SignedOrder, mode: SignatureVerificationMode) -> Result<()> {
        validate_signature_shape(&order.signature)?;

        match mode {
            SignatureVerificationMode::Disabled => Ok(()),
            SignatureVerificationMode::Strict => {
                Err(BackendError::StrictSignatureVerificationUnavailable)
            }
        }
    }
}

pub fn validate_signature_shape(signature: &str) -> Result<()> {
    let Some(hex) = signature.strip_prefix("0x") else {
        return Err(BackendError::MalformedSignature);
    };

    if hex.len() != 130 {
        return Err(BackendError::MalformedSignature);
    }

    if !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(BackendError::MalformedSignature);
    }

    Ok(())
}
