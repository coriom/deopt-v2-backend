use deopt_v2_backend::execution::{ExecutorSigner, PrivateKeySecret};
use deopt_v2_backend::signing::eip712::parse_evm_address;
use deopt_v2_backend::types::AccountId;
use serde::{Deserialize, Serialize};
use std::env;
use std::fmt;
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;
use std::str::FromStr;

const DEFAULT_CHAIN_ID: u64 = 84532;

fn main() -> ExitCode {
    match run() {
        Ok(output) => match serde_json::to_string_pretty(&output) {
            Ok(json) => {
                println!("{json}");
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("failed to serialize signing output: {error}");
                ExitCode::FAILURE
            }
        },
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> CliResult<SignOutput> {
    let args = CliArgs::parse(env::args().skip(1))?;
    let payload_json = fs::read_to_string(&args.payload)
        .map_err(|error| CliError::new(format!("failed to read payload file: {error}")))?;
    let payload = parse_payload(&payload_json)?;
    sign_payload(&payload, &args, env_private_key(args.role)?)
}

fn sign_payload(
    payload: &SigningPayloadFile,
    args: &CliArgs,
    private_key: PrivateKeySecret,
) -> CliResult<SignOutput> {
    validate_payload_shape(payload, args.chain_id)?;
    let digest = parse_digest(&payload.digest)?;
    let signer = ExecutorSigner::from_private_key(&private_key)
        .map_err(|error| CliError::new(error.to_string()))?;
    validate_role_address(
        args.role,
        signer.address(),
        &payload.message,
        args.allow_address_mismatch,
    )?;
    let signature = sign_digest(&signer, &digest)?;

    Ok(SignOutput {
        role: args.role,
        signer_address: signer.address().0.clone(),
        signature,
        digest: args.verbose.then(|| payload.digest.to_ascii_lowercase()),
        domain: args.verbose.then(|| payload.domain.clone()),
        message_intent_id: args
            .verbose
            .then(|| payload.message.intent_id.to_ascii_lowercase()),
    })
}

fn sign_digest(signer: &ExecutorSigner, digest: &[u8; 32]) -> CliResult<String> {
    let signature = signer
        .sign_prehash(digest)
        .map_err(|error| CliError::new(error.to_string()))?;
    let mut bytes = Vec::with_capacity(65);
    bytes.extend_from_slice(&signature.r);
    bytes.extend_from_slice(&signature.s);
    bytes.push(signature.y_parity + 27);
    Ok(hex_0x(&bytes))
}

fn parse_payload(input: &str) -> CliResult<SigningPayloadFile> {
    serde_json::from_str(input)
        .map_err(|error| CliError::new(format!("invalid payload JSON: {error}")))
}

fn validate_payload_shape(payload: &SigningPayloadFile, expected_chain_id: u64) -> CliResult<()> {
    if payload.primary_type != "PerpTrade" {
        return Err(CliError::new("payload primary_type must be PerpTrade"));
    }
    let first_field = payload
        .types
        .first()
        .ok_or_else(|| CliError::new("payload types must include PerpTrade fields"))?;
    if first_field.name != "intentId" || first_field.type_name != "bytes32" {
        return Err(CliError::new(
            "payload types first field must be intentId bytes32",
        ));
    }
    if payload.domain.chain_id != expected_chain_id {
        return Err(CliError::new(format!(
            "payload domain.chainId {} does not match expected chain id {expected_chain_id}",
            payload.domain.chain_id
        )));
    }
    parse_digest(&payload.message.intent_id)
        .map_err(|_| CliError::new("payload message.intentId must be bytes32 hex"))?;
    parse_evm_address(&AccountId::new(payload.message.buyer.clone()))
        .map_err(|_| CliError::new("payload message.buyer must be an EVM address"))?;
    parse_evm_address(&AccountId::new(payload.message.seller.clone()))
        .map_err(|_| CliError::new("payload message.seller must be an EVM address"))?;
    Ok(())
}

fn validate_role_address(
    role: Role,
    signer_address: &AccountId,
    message: &SigningPayloadMessage,
    allow_mismatch: bool,
) -> CliResult<()> {
    let expected = match role {
        Role::Buyer => &message.buyer,
        Role::Seller => &message.seller,
    };
    if normalize_address(&signer_address.0) == normalize_address(expected) {
        return Ok(());
    }
    if allow_mismatch {
        return Ok(());
    }
    Err(CliError::new(format!(
        "{role} signer address {} does not match payload {role} address {}",
        signer_address.0, expected
    )))
}

fn parse_digest(value: &str) -> CliResult<[u8; 32]> {
    let hex = value
        .strip_prefix("0x")
        .ok_or_else(|| CliError::new("digest must start with 0x"))?;
    if hex.len() != 64 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(CliError::new("digest must be 32-byte hex"));
    }
    let mut bytes = [0u8; 32];
    decode_hex_to_slice(hex, &mut bytes)?;
    Ok(bytes)
}

fn env_private_key(role: Role) -> CliResult<PrivateKeySecret> {
    let role_env = match role {
        Role::Buyer => "BUYER_PRIVATE_KEY",
        Role::Seller => "SELLER_PRIVATE_KEY",
    };
    let value = env::var(role_env)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            env::var("SIGNER_PRIVATE_KEY")
                .ok()
                .filter(|value| !value.trim().is_empty())
        })
        .ok_or_else(|| {
            CliError::new(format!(
                "{role_env} or SIGNER_PRIVATE_KEY is required for dev signing"
            ))
        })?;
    Ok(PrivateKeySecret::new(value))
}

fn normalize_address(value: &str) -> String {
    value.to_ascii_lowercase()
}

fn hex_0x(bytes: &[u8]) -> String {
    let mut hex = String::with_capacity(2 + bytes.len() * 2);
    hex.push_str("0x");
    for byte in bytes {
        hex.push_str(&format!("{byte:02x}"));
    }
    hex
}

fn decode_hex_to_slice(hex: &str, out: &mut [u8]) -> CliResult<()> {
    if hex.len() != out.len() * 2 {
        return Err(CliError::new("hex length does not match output length"));
    }
    for (index, byte) in out.iter_mut().enumerate() {
        let high = decode_hex_nibble(hex.as_bytes()[index * 2])?;
        let low = decode_hex_nibble(hex.as_bytes()[index * 2 + 1])?;
        *byte = (high << 4) | low;
    }
    Ok(())
}

fn decode_hex_nibble(byte: u8) -> CliResult<u8> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => Err(CliError::new("hex contains non-hex character")),
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct CliArgs {
    payload: PathBuf,
    role: Role,
    allow_address_mismatch: bool,
    verbose: bool,
    chain_id: u64,
}

impl CliArgs {
    fn parse(args: impl IntoIterator<Item = String>) -> CliResult<Self> {
        let mut payload = None;
        let mut role = None;
        let mut allow_address_mismatch = false;
        let mut verbose = false;
        let mut chain_id = DEFAULT_CHAIN_ID;
        let mut args = args.into_iter();

        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--payload" => {
                    payload = Some(PathBuf::from(
                        args.next()
                            .ok_or_else(|| CliError::new("--payload requires a path"))?,
                    ));
                }
                "--role" => {
                    role =
                        Some(Role::from_str(&args.next().ok_or_else(|| {
                            CliError::new("--role requires buyer or seller")
                        })?)?);
                }
                "--allow-address-mismatch" => allow_address_mismatch = true,
                "--verbose" => verbose = true,
                "--chain-id" => {
                    let value = args
                        .next()
                        .ok_or_else(|| CliError::new("--chain-id requires a value"))?;
                    chain_id = value
                        .parse::<u64>()
                        .map_err(|error| CliError::new(format!("invalid --chain-id: {error}")))?;
                }
                "--help" | "-h" => return Err(CliError::new(usage())),
                other => {
                    return Err(CliError::new(format!(
                        "unknown argument: {other}\n{}",
                        usage()
                    )));
                }
            }
        }

        Ok(Self {
            payload: payload.ok_or_else(|| CliError::new("--payload is required"))?,
            role: role.ok_or_else(|| CliError::new("--role is required"))?,
            allow_address_mismatch,
            verbose,
            chain_id,
        })
    }
}

fn usage() -> &'static str {
    "usage: cargo run --bin sign_perp_trade -- --payload <path> --role buyer|seller [--chain-id <id>] [--allow-address-mismatch] [--verbose]"
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum Role {
    Buyer,
    Seller,
}

impl fmt::Display for Role {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Buyer => formatter.write_str("buyer"),
            Self::Seller => formatter.write_str("seller"),
        }
    }
}

impl FromStr for Role {
    type Err = CliError;

    fn from_str(value: &str) -> CliResult<Self> {
        match value {
            "buyer" => Ok(Self::Buyer),
            "seller" => Ok(Self::Seller),
            _ => Err(CliError::new("--role must be buyer or seller")),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
struct SigningPayloadFile {
    domain: SigningPayloadDomain,
    primary_type: String,
    types: Vec<SigningPayloadTypeField>,
    message: SigningPayloadMessage,
    digest: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct SigningPayloadDomain {
    name: String,
    version: String,
    #[serde(rename = "chainId")]
    chain_id: u64,
    #[serde(rename = "verifyingContract")]
    verifying_contract: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
struct SigningPayloadTypeField {
    name: String,
    #[serde(rename = "type")]
    type_name: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
struct SigningPayloadMessage {
    #[serde(rename = "intentId")]
    intent_id: String,
    buyer: String,
    seller: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct SignOutput {
    role: Role,
    signer_address: String,
    signature: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    digest: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    domain: Option<SigningPayloadDomain>,
    #[serde(skip_serializing_if = "Option::is_none")]
    message_intent_id: Option<String>,
}

type CliResult<T> = std::result::Result<T, CliError>;

#[derive(Clone, Debug, Eq, PartialEq)]
struct CliError {
    message: String,
}

impl CliError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for CliError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use k256::ecdsa::{RecoveryId, Signature, VerifyingKey};

    const TEST_KEY: &str = "0x4c0883a69102937d6231471b5dbb6204fe5129617082792ae468d01a3f362318";
    const TEST_ADDRESS: &str = "0x2c7536e3605d9c16a7a3d7b1898e529396a65c23";

    #[test]
    fn parses_payload_digest() {
        let digest =
            parse_digest("0x1111111111111111111111111111111111111111111111111111111111111111")
                .unwrap();

        assert_eq!(digest, [0x11; 32]);
    }

    #[test]
    fn rejects_missing_digest_prefix() {
        let error =
            parse_digest("1111111111111111111111111111111111111111111111111111111111111111")
                .unwrap_err();

        assert!(error.to_string().contains("digest must start with 0x"));
    }

    #[test]
    fn rejects_payload_missing_digest() {
        let input = r#"{
            "domain": {
                "name": "DeOptV2-PerpMatchingEngine",
                "version": "1",
                "chainId": 84532,
                "verifyingContract": "0x0000000000000000000000000000000000000001"
            },
            "primary_type": "PerpTrade",
            "types": [{"name": "intentId", "type": "bytes32"}],
            "message": {
                "intentId": "0x1111111111111111111111111111111111111111111111111111111111111111",
                "buyer": "0x0000000000000000000000000000000000000001",
                "seller": "0x0000000000000000000000000000000000000002"
            }
        }"#;

        let error = parse_payload(input).unwrap_err();

        assert!(error.to_string().contains("missing field `digest`"));
    }

    #[test]
    fn rejects_non_perp_trade_payload() {
        let mut payload = payload();
        payload.primary_type = "Order".to_string();

        let error = validate_payload_shape(&payload, DEFAULT_CHAIN_ID).unwrap_err();

        assert!(error.to_string().contains("primary_type must be PerpTrade"));
    }

    #[test]
    fn rejects_payload_missing_intent_id_first_field() {
        let mut payload = payload();
        payload.types[0].name = "buyer".to_string();

        let error = validate_payload_shape(&payload, DEFAULT_CHAIN_ID).unwrap_err();

        assert!(error
            .to_string()
            .contains("first field must be intentId bytes32"));
    }

    #[test]
    fn rejects_payload_missing_intent_id_message() {
        let mut payload = payload();
        payload.message.intent_id = "0x1234".to_string();

        let error = validate_payload_shape(&payload, DEFAULT_CHAIN_ID).unwrap_err();

        assert!(error
            .to_string()
            .contains("message.intentId must be bytes32 hex"));
    }

    #[test]
    fn rejects_invalid_role() {
        let error = Role::from_str("maker").unwrap_err();

        assert!(error.to_string().contains("--role must be buyer or seller"));
    }

    #[test]
    fn signer_derives_expected_address_and_redacts_debug() {
        let signer =
            ExecutorSigner::from_private_key(&PrivateKeySecret::new(TEST_KEY.to_string())).unwrap();
        let debug = format!("{signer:?}");

        assert_eq!(signer.address().0, TEST_ADDRESS);
        assert!(!debug.contains("4c0883"));
        assert!(debug.contains("<redacted>"));
    }

    #[test]
    fn role_address_mismatch_is_rejected_by_default() {
        let message = SigningPayloadMessage {
            intent_id: "0x1111111111111111111111111111111111111111111111111111111111111111"
                .to_string(),
            buyer: "0x0000000000000000000000000000000000000001".to_string(),
            seller: "0x0000000000000000000000000000000000000002".to_string(),
        };

        let error = validate_role_address(
            Role::Buyer,
            &AccountId::new(TEST_ADDRESS.to_string()),
            &message,
            false,
        )
        .unwrap_err();

        assert!(error.to_string().contains("does not match"));
    }

    #[test]
    fn role_address_mismatch_can_be_explicitly_allowed() {
        let message = SigningPayloadMessage {
            intent_id: "0x1111111111111111111111111111111111111111111111111111111111111111"
                .to_string(),
            buyer: "0x0000000000000000000000000000000000000001".to_string(),
            seller: "0x0000000000000000000000000000000000000002".to_string(),
        };

        validate_role_address(
            Role::Buyer,
            &AccountId::new(TEST_ADDRESS.to_string()),
            &message,
            true,
        )
        .unwrap();
    }

    #[test]
    fn signature_is_65_bytes_and_recovers_to_signer() {
        let mut payload = payload();
        payload.message.buyer = TEST_ADDRESS.to_string();
        let args = CliArgs {
            payload: PathBuf::from("/tmp/payload.json"),
            role: Role::Buyer,
            allow_address_mismatch: false,
            verbose: false,
            chain_id: DEFAULT_CHAIN_ID,
        };
        let output =
            sign_payload(&payload, &args, PrivateKeySecret::new(TEST_KEY.to_string())).unwrap();

        let bytes = decode_signature(&output.signature);
        let digest = parse_digest(&payload.digest).unwrap();
        let recovered = recover_address(&digest, &bytes);

        assert_eq!(output.signature.len(), 132);
        assert_eq!(bytes.len(), 65);
        assert!(matches!(bytes[64], 27 | 28));
        assert_eq!(recovered, TEST_ADDRESS);
    }

    #[test]
    fn malformed_key_is_rejected() {
        let mut payload = payload();
        payload.message.buyer = TEST_ADDRESS.to_string();
        let args = CliArgs {
            payload: PathBuf::from("/tmp/payload.json"),
            role: Role::Buyer,
            allow_address_mismatch: false,
            verbose: false,
            chain_id: DEFAULT_CHAIN_ID,
        };

        let error =
            sign_payload(&payload, &args, PrivateKeySecret::new("0x1234".to_string())).unwrap_err();

        assert!(error.to_string().contains("invalid EXECUTOR_PRIVATE_KEY"));
    }

    fn payload() -> SigningPayloadFile {
        SigningPayloadFile {
            domain: SigningPayloadDomain {
                name: "DeOptV2-PerpMatchingEngine".to_string(),
                version: "1".to_string(),
                chain_id: DEFAULT_CHAIN_ID,
                verifying_contract: "0x0000000000000000000000000000000000000001".to_string(),
            },
            primary_type: "PerpTrade".to_string(),
            types: vec![
                SigningPayloadTypeField {
                    name: "intentId".to_string(),
                    type_name: "bytes32".to_string(),
                },
                SigningPayloadTypeField {
                    name: "buyer".to_string(),
                    type_name: "address".to_string(),
                },
            ],
            message: SigningPayloadMessage {
                intent_id: "0x1111111111111111111111111111111111111111111111111111111111111111"
                    .to_string(),
                buyer: "0x0000000000000000000000000000000000000001".to_string(),
                seller: "0x0000000000000000000000000000000000000002".to_string(),
            },
            digest: "0x2222222222222222222222222222222222222222222222222222222222222222"
                .to_string(),
        }
    }

    fn decode_signature(signature: &str) -> Vec<u8> {
        let hex = signature.strip_prefix("0x").unwrap();
        let mut bytes = vec![0u8; 65];
        decode_hex_to_slice(hex, &mut bytes).unwrap();
        bytes
    }

    fn recover_address(digest: &[u8; 32], bytes: &[u8]) -> String {
        let signature = Signature::from_slice(&bytes[..64]).unwrap();
        let recovery_id = RecoveryId::try_from(bytes[64] - 27).unwrap();
        let verifying_key =
            VerifyingKey::recover_from_prehash(digest, &signature, recovery_id).unwrap();
        let encoded = verifying_key.to_encoded_point(false);
        let public_key = encoded.as_bytes();
        let hash = deopt_v2_backend::signing::eip712::keccak256(&public_key[1..]);
        hex_0x(&hash[12..])
    }
}
