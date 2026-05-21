use deopt_v2_backend::error::{BackendError, Result};
use deopt_v2_backend::execution::{ExecutorSigner, PrivateKeySecret};
use deopt_v2_backend::types::AccountId;
use serde::Serialize;
use serde_json::Value;
use std::env;

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct SignOptionExecutionIntentOutput {
    signer_address: AccountId,
    signature: String,
}

fn main() -> Result<()> {
    let args = parse_args()?;
    let private_key = env::var(&args.private_key_env).map_err(|_| {
        BackendError::Config(format!(
            "{} is required for sign_option_execution_intent",
            args.private_key_env
        ))
    })?;
    let payload = std::fs::read_to_string(&args.payload_file).map_err(|error| {
        BackendError::Config(format!(
            "failed to read payload file {}: {error}",
            args.payload_file
        ))
    })?;
    let payload: Value = serde_json::from_str(&payload)
        .map_err(|error| BackendError::Config(format!("invalid payload JSON: {error}")))?;
    let digest_hex = payload
        .get("digest")
        .and_then(Value::as_str)
        .ok_or_else(|| BackendError::Config("payload JSON must include digest".to_string()))?;
    let digest = parse_digest(digest_hex)?;

    let signer = ExecutorSigner::from_private_key(&PrivateKeySecret::new(private_key))?;

    // Optional: verify signer is the expected buyer or seller.
    if let Some(message) = payload.get("message") {
        let buyer = message.get("buyer").and_then(Value::as_str);
        let seller = message.get("seller").and_then(Value::as_str);
        let addr = signer.address().0.to_lowercase();
        let matches_buyer = buyer.is_some_and(|a| a.to_lowercase() == addr);
        let matches_seller = seller.is_some_and(|a| a.to_lowercase() == addr);
        if (buyer.is_some() || seller.is_some()) && !matches_buyer && !matches_seller {
            return Err(BackendError::Config(format!(
                "signer {} does not match buyer ({}) or seller ({})",
                signer.address().0,
                buyer.unwrap_or("none"),
                seller.unwrap_or("none"),
            )));
        }
    }

    let signature = signer.sign_prehash(&digest)?;
    let mut bytes = Vec::with_capacity(65);
    bytes.extend_from_slice(&signature.r);
    bytes.extend_from_slice(&signature.s);
    bytes.push(signature.y_parity + 27);

    let output = SignOptionExecutionIntentOutput {
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

struct CliArgs {
    payload_file: String,
    private_key_env: String,
}

fn parse_args() -> Result<CliArgs> {
    let mut payload_file = None;
    let mut private_key_env = "BUYER_PRIVATE_KEY".to_string();
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--payload-file" => {
                payload_file = Some(args.next().ok_or_else(|| {
                    BackendError::Config("--payload-file requires a path".to_string())
                })?);
            }
            "--private-key-env" => {
                private_key_env = args.next().ok_or_else(|| {
                    BackendError::Config("--private-key-env requires an env var name".to_string())
                })?;
            }
            other => {
                return Err(BackendError::Config(format!("unknown argument: {other}")));
            }
        }
    }
    let payload_file = payload_file.ok_or_else(|| {
        BackendError::Config(
            "usage: sign_option_execution_intent --payload-file <path> [--private-key-env <VAR>]"
                .to_string(),
        )
    })?;
    Ok(CliArgs {
        payload_file,
        private_key_env,
    })
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
            .map_err(|error| BackendError::Config(format!("invalid digest hex: {error}")))?;
    }
    Ok(bytes)
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

#[cfg(test)]
mod tests {
    use super::*;
    use deopt_v2_backend::signing::signature::recover_eip712_signer;

    const TEST_KEY: &str = "0x4c0883a69102937d6231471b5dbb6204fe5129617082792ae468d01a3f362318";
    const TEST_ADDR: &str = "0x2c7536e3605d9c16a7a3d7b1898e529396a65c23";

    #[test]
    fn parse_digest_valid() {
        let hex = format!("0x{}", "ab".repeat(32));
        let result = parse_digest(&hex);
        assert!(result.is_ok());
        assert!(result.unwrap().iter().all(|b| *b == 0xab));
    }

    #[test]
    fn parse_digest_rejects_missing_prefix() {
        let hex = "ab".repeat(32);
        assert!(parse_digest(&hex).is_err());
    }

    #[test]
    fn parse_digest_rejects_wrong_length() {
        assert!(parse_digest("0x1234").is_err());
        assert!(parse_digest(&format!("0x{}", "ab".repeat(33))).is_err());
    }

    #[test]
    fn parse_digest_rejects_non_hex() {
        assert!(parse_digest(&format!("0x{}zz", "ab".repeat(31))).is_err());
    }

    #[test]
    fn signing_output_has_correct_shape() {
        let signer =
            ExecutorSigner::from_private_key(&PrivateKeySecret::new(TEST_KEY.to_string())).unwrap();
        let digest = [0u8; 32];
        let sig = signer.sign_prehash(&digest).unwrap();
        let mut bytes = Vec::with_capacity(65);
        bytes.extend_from_slice(&sig.r);
        bytes.extend_from_slice(&sig.s);
        bytes.push(sig.y_parity + 27);
        let hex = hex_0x(&bytes);

        assert_eq!(hex.len(), 132); // 0x + 130 hex chars
        assert!(hex.starts_with("0x"));
    }

    #[test]
    fn recovered_signer_matches_expected() {
        let signer =
            ExecutorSigner::from_private_key(&PrivateKeySecret::new(TEST_KEY.to_string())).unwrap();
        assert_eq!(signer.address().0, TEST_ADDR);

        let digest = [1u8; 32];
        let sig = signer.sign_prehash(&digest).unwrap();
        let mut bytes = Vec::with_capacity(65);
        bytes.extend_from_slice(&sig.r);
        bytes.extend_from_slice(&sig.s);
        bytes.push(sig.y_parity + 27);
        let hex = hex_0x(&bytes);

        let recovered = recover_eip712_signer(&digest, &hex).unwrap();
        assert_eq!(recovered.0.to_lowercase(), TEST_ADDR.to_lowercase());
    }

    #[test]
    fn private_key_not_in_output() {
        let signer =
            ExecutorSigner::from_private_key(&PrivateKeySecret::new(TEST_KEY.to_string())).unwrap();
        let output = SignOptionExecutionIntentOutput {
            signer_address: signer.address().clone(),
            signature: "0x".to_string() + &"00".repeat(65),
        };
        let json = serde_json::to_string(&output).unwrap();
        assert!(!json.contains("4c0883"));
        assert!(!json.contains("private"));
        assert!(!json.contains("secret"));
    }

    #[test]
    fn invalid_payload_missing_digest_is_rejected() {
        let payload =
            serde_json::json!({"message": {"buyer": "0x1111111111111111111111111111111111111111"}});
        let digest = payload.get("digest").and_then(Value::as_str);
        assert!(digest.is_none());
    }
}
