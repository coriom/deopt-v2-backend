use crate::error::{BackendError, Result};
use crate::execution::config::PrivateKeySecret;
use crate::execution::transaction::hex_0x;
use crate::signing::eip712::keccak256;
use crate::types::AccountId;
use k256::ecdsa::SigningKey;
use std::fmt;

#[derive(Clone)]
pub struct ExecutorSigner {
    signing_key: SigningKey,
    address: AccountId,
}

impl fmt::Debug for ExecutorSigner {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ExecutorSigner")
            .field("address", &self.address)
            .field("private_key", &"<redacted>")
            .finish()
    }
}

impl ExecutorSigner {
    pub fn from_private_key(secret: &PrivateKeySecret) -> Result<Self> {
        let key_bytes = parse_private_key(secret.expose_secret())?;
        let signing_key = SigningKey::from_slice(&key_bytes).map_err(|error| {
            BackendError::Config(format!("invalid EXECUTOR_PRIVATE_KEY: {error}"))
        })?;
        let verifying_key = signing_key.verifying_key();
        let public_key = verifying_key.to_encoded_point(false);
        let public_key = public_key.as_bytes();
        let hash = keccak256(&public_key[1..]);
        let address = AccountId::new(hex_0x(&hash[12..]));
        Ok(Self {
            signing_key,
            address,
        })
    }

    pub fn address(&self) -> &AccountId {
        &self.address
    }

    pub fn sign_prehash(&self, hash: &[u8; 32]) -> Result<RecoverableSignature> {
        let (signature, recovery_id) =
            self.signing_key
                .sign_prehash_recoverable(hash)
                .map_err(|error| {
                    BackendError::BroadcastRejected(format!("transaction signing failed: {error}"))
                })?;
        Ok(RecoverableSignature {
            y_parity: recovery_id.to_byte(),
            r: signature.r().to_bytes().into(),
            s: signature.s().to_bytes().into(),
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecoverableSignature {
    pub y_parity: u8,
    pub r: [u8; 32],
    pub s: [u8; 32],
}

fn parse_private_key(value: &str) -> Result<[u8; 32]> {
    let hex = value.strip_prefix("0x").unwrap_or(value);
    if hex.len() != 64 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(BackendError::Config(
            "invalid EXECUTOR_PRIVATE_KEY: expected 32-byte hex".to_string(),
        ));
    }
    let mut bytes = [0u8; 32];
    for index in 0..32 {
        bytes[index] = u8::from_str_radix(&hex[index * 2..index * 2 + 2], 16).map_err(|error| {
            BackendError::Config(format!("invalid EXECUTOR_PRIVATE_KEY: {error}"))
        })?;
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_KEY: &str = "0x4c0883a69102937d6231471b5dbb6204fe5129617082792ae468d01a3f362318";

    #[test]
    fn signer_derives_executor_address_without_exposing_key_in_debug() {
        let signer =
            ExecutorSigner::from_private_key(&PrivateKeySecret::new(TEST_KEY.to_string())).unwrap();
        let debug = format!("{signer:?}");

        assert_eq!(
            signer.address().0,
            "0x2c7536e3605d9c16a7a3d7b1898e529396a65c23"
        );
        assert!(!debug.contains("4c0883"));
        assert!(debug.contains("<redacted>"));
    }

    #[test]
    fn invalid_private_key_is_rejected() {
        let error = ExecutorSigner::from_private_key(&PrivateKeySecret::new("0x1234".to_string()))
            .unwrap_err();

        assert!(error.to_string().contains("invalid EXECUTOR_PRIVATE_KEY"));
    }
}
