use super::eip712::{eip712_digest, keccak256, parse_evm_address, Eip712Domain};
use super::SignedOrder;
use crate::error::{BackendError, Result};
use k256::ecdsa::{RecoveryId, Signature, VerifyingKey};
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
    pub fn verify(
        order: &SignedOrder,
        mode: SignatureVerificationMode,
        domain: &Eip712Domain,
    ) -> Result<()> {
        validate_signature_shape(&order.signature)?;

        match mode {
            SignatureVerificationMode::Disabled => Ok(()),
            SignatureVerificationMode::Strict => verify_strict(order, domain),
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

fn verify_strict(order: &SignedOrder, domain: &Eip712Domain) -> Result<()> {
    let expected_account = parse_evm_address(&order.account)?;
    let signature = parse_signature(&order.signature)?;
    let digest = eip712_digest(order, domain)?;
    let recovered_account = recover_signer(&digest, &signature)?;

    if recovered_account != expected_account {
        return Err(BackendError::SignatureSignerMismatch);
    }

    Ok(())
}

fn recover_signer(digest: &[u8; 32], signature: &ParsedSignature) -> Result<[u8; 20]> {
    let k256_signature =
        Signature::from_slice(&signature.rs).map_err(|_| BackendError::MalformedSignature)?;
    let recovery_id = RecoveryId::try_from(signature_v_to_recovery_id(signature.v)?)
        .map_err(|_| BackendError::UnsupportedSignatureV)?;
    let verifying_key = VerifyingKey::recover_from_prehash(digest, &k256_signature, recovery_id)
        .map_err(|_| BackendError::SignatureRecoveryFailed)?;
    Ok(address_from_verifying_key(&verifying_key))
}

fn address_from_verifying_key(verifying_key: &VerifyingKey) -> [u8; 20] {
    let encoded = verifying_key.to_encoded_point(false);
    let public_key = encoded.as_bytes();
    let hash = keccak256(&public_key[1..]);

    let mut address = [0u8; 20];
    address.copy_from_slice(&hash[12..]);
    address
}

fn signature_v_to_recovery_id(v: u8) -> Result<u8> {
    match v {
        0 | 1 => Ok(v),
        27 | 28 => Ok(v - 27),
        _ => Err(BackendError::UnsupportedSignatureV),
    }
}

struct ParsedSignature {
    rs: [u8; 64],
    v: u8,
}

fn parse_signature(signature: &str) -> Result<ParsedSignature> {
    let hex = signature
        .strip_prefix("0x")
        .ok_or(BackendError::MalformedSignature)?;
    let mut bytes = [0u8; 65];
    decode_hex_to_slice(hex, &mut bytes).map_err(|_| BackendError::MalformedSignature)?;

    let mut rs = [0u8; 64];
    rs.copy_from_slice(&bytes[..64]);
    Ok(ParsedSignature { rs, v: bytes[64] })
}

fn decode_hex_to_slice(hex: &str, out: &mut [u8]) -> std::result::Result<(), ()> {
    if hex.len() != out.len() * 2 {
        return Err(());
    }

    for (index, byte) in out.iter_mut().enumerate() {
        let high = decode_hex_nibble(hex.as_bytes()[index * 2])?;
        let low = decode_hex_nibble(hex.as_bytes()[index * 2 + 1])?;
        *byte = (high << 4) | low;
    }

    Ok(())
}

fn decode_hex_nibble(byte: u8) -> std::result::Result<u8, ()> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => Err(()),
    }
}
