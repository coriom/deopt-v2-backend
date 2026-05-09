use deopt_v2_backend::error::{BackendError, Result};
use deopt_v2_backend::execution::{ExecutorSigner, PrivateKeySecret};
use deopt_v2_backend::types::AccountId;
use serde::Serialize;
use serde_json::Value;
use std::env;

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct SignRfqQuoteOutput {
    signer_address: AccountId,
    signature: String,
}

fn main() -> Result<()> {
    let payload_path = payload_path()?;
    let private_key = env::var("MM_PRIVATE_KEY").map_err(|_| {
        BackendError::Config("MM_PRIVATE_KEY is required for sign_rfq_quote".to_string())
    })?;
    let payload = std::fs::read_to_string(&payload_path).map_err(|error| {
        BackendError::Config(format!(
            "failed to read payload file {payload_path}: {error}"
        ))
    })?;
    let payload: Value = serde_json::from_str(&payload)
        .map_err(|error| BackendError::Config(format!("invalid payload JSON: {error}")))?;
    let digest = payload
        .get("digest")
        .and_then(Value::as_str)
        .ok_or_else(|| BackendError::Config("payload JSON must include digest".to_string()))?;
    let digest = parse_digest(digest)?;

    let signer = ExecutorSigner::from_private_key(&PrivateKeySecret::new(private_key))?;
    if let Some(mm_account) = payload
        .get("message")
        .and_then(|message| message.get("mmAccount"))
        .and_then(Value::as_str)
    {
        if !addresses_equal(signer.address(), &AccountId::new(mm_account.to_string())) {
            return Err(BackendError::SignatureSignerMismatch);
        }
    }

    let signature = signer.sign_prehash(&digest)?;
    let mut bytes = Vec::with_capacity(65);
    bytes.extend_from_slice(&signature.r);
    bytes.extend_from_slice(&signature.s);
    bytes.push(signature.y_parity + 27);

    let output = SignRfqQuoteOutput {
        signer_address: signer.address().clone(),
        signature: hex_0x(&bytes),
    };
    println!(
        "{}",
        serde_json::to_string_pretty(&output)
            .map_err(|error| BackendError::Config(error.to_string()))?
    );
    Ok(())
}

fn payload_path() -> Result<String> {
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--payload" {
            return args
                .next()
                .ok_or_else(|| BackendError::Config("--payload requires a file path".to_string()));
        }
    }
    Err(BackendError::Config(
        "usage: sign_rfq_quote --payload /path/to/rfq_quote_payload.json".to_string(),
    ))
}

fn parse_digest(value: &str) -> Result<[u8; 32]> {
    let Some(hex) = value.strip_prefix("0x") else {
        return Err(BackendError::Config(
            "digest must be 0x-prefixed bytes32 hex".to_string(),
        ));
    };
    if hex.len() != 64 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(BackendError::Config(
            "digest must be 0x-prefixed bytes32 hex".to_string(),
        ));
    }
    let mut bytes = [0u8; 32];
    for index in 0..32 {
        bytes[index] = u8::from_str_radix(&hex[index * 2..index * 2 + 2], 16)
            .map_err(|error| BackendError::Config(format!("invalid digest: {error}")))?;
    }
    Ok(bytes)
}

fn addresses_equal(left: &AccountId, right: &AccountId) -> bool {
    left.0.eq_ignore_ascii_case(&right.0)
}

fn hex_0x(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(2 + bytes.len() * 2);
    encoded.push_str("0x");
    for byte in bytes {
        encoded.push(HEX[(byte >> 4) as usize] as char);
        encoded.push(HEX[(byte & 0x0f) as usize] as char);
    }
    encoded
}
