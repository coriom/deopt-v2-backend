use crate::signing::eip712::keccak256;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fmt;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RevertDiagnostics {
    pub raw_error: String,
    pub revert_data: Option<String>,
    pub revert_selector: Option<String>,
    pub decoded_error: DecodedRevertError,
}

impl RevertDiagnostics {
    pub fn missing(raw_error: impl Into<String>) -> Self {
        Self {
            raw_error: raw_error.into(),
            revert_data: None,
            revert_selector: None,
            decoded_error: DecodedRevertError::missing_revert_data(),
        }
    }
}

impl fmt::Display for RevertDiagnostics {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.raw_error)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DecodedRevertError {
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selector: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub args: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decoded: Option<Value>,
}

impl DecodedRevertError {
    pub fn missing_revert_data() -> Self {
        Self {
            kind: "missing_revert_data".to_string(),
            name: None,
            selector: None,
            args: None,
            decoded: None,
        }
    }
}

pub fn diagnostics_from_rpc_error(message: &str, data: Option<&Value>) -> RevertDiagnostics {
    let revert_data = data.and_then(extract_revert_data_value).map(normalize_hex);
    let decoded_error = match revert_data.as_deref() {
        Some(data) => decode_revert_data(data),
        None => DecodedRevertError::missing_revert_data(),
    };
    let revert_selector = selector_from_data(revert_data.as_deref());
    RevertDiagnostics {
        raw_error: message.to_string(),
        revert_data,
        revert_selector,
        decoded_error,
    }
}

pub fn extract_revert_data_from_json(error: &Value) -> Option<String> {
    error
        .get("data")
        .and_then(extract_revert_data_value)
        .map(normalize_hex)
}

pub fn decode_revert_data(revert_data: &str) -> DecodedRevertError {
    let normalized = normalize_hex(revert_data);
    let Some(hex) = normalized.strip_prefix("0x") else {
        return DecodedRevertError {
            kind: "malformed_revert_data".to_string(),
            name: None,
            selector: None,
            args: None,
            decoded: None,
        };
    };
    if hex.len() < 8 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return DecodedRevertError {
            kind: "malformed_revert_data".to_string(),
            name: None,
            selector: selector_from_data(Some(&normalized)),
            args: None,
            decoded: None,
        };
    }

    let selector = format!("0x{}", &hex[..8]);
    match selector.as_str() {
        "0x08c379a0" => decode_error_string(&normalized, &selector),
        "0x4e487b71" => decode_panic(&normalized, &selector),
        _ => decode_custom_error(&selector),
    }
}

fn extract_revert_data_value(value: &Value) -> Option<&str> {
    if let Some(data) = value.as_str().filter(|data| looks_like_hex_data(data)) {
        return Some(data);
    }
    value
        .get("originalError")
        .and_then(|original| original.get("data"))
        .and_then(Value::as_str)
        .filter(|data| looks_like_hex_data(data))
        .or_else(|| {
            value
                .get("result")
                .and_then(Value::as_str)
                .filter(|data| looks_like_hex_data(data))
        })
}

fn decode_error_string(revert_data: &str, selector: &str) -> DecodedRevertError {
    let bytes = match decode_hex_bytes(revert_data) {
        Some(bytes) => bytes,
        None => {
            return DecodedRevertError {
                kind: "malformed_revert_data".to_string(),
                name: Some("Error".to_string()),
                selector: Some(selector.to_string()),
                args: None,
                decoded: None,
            };
        }
    };
    let message = decode_abi_string(&bytes[4..]).unwrap_or_else(|| "<decode failed>".to_string());
    DecodedRevertError {
        kind: "solidity_error".to_string(),
        name: Some("Error".to_string()),
        selector: Some(selector.to_string()),
        args: Some(vec![message]),
        decoded: None,
    }
}

fn decode_panic(revert_data: &str, selector: &str) -> DecodedRevertError {
    let bytes = decode_hex_bytes(revert_data).unwrap_or_default();
    let code = bytes
        .get(4..36)
        .and_then(word_to_u128)
        .map(|value| format!("0x{value:x}"))
        .unwrap_or_else(|| "<decode failed>".to_string());
    DecodedRevertError {
        kind: "panic".to_string(),
        name: Some("Panic".to_string()),
        selector: Some(selector.to_string()),
        args: Some(vec![code]),
        decoded: None,
    }
}

fn decode_custom_error(selector: &str) -> DecodedRevertError {
    if let Some(name) = known_custom_error_name(selector) {
        return DecodedRevertError {
            kind: "custom_error".to_string(),
            name: Some(name.to_string()),
            selector: Some(selector.to_string()),
            args: Some(Vec::new()),
            decoded: None,
        };
    }

    DecodedRevertError {
        kind: "unknown_custom_error".to_string(),
        name: None,
        selector: Some(selector.to_string()),
        args: None,
        decoded: None,
    }
}

fn known_custom_error_name(selector: &str) -> Option<&'static str> {
    known_custom_errors().iter().find_map(|(signature, name)| {
        (selector_for_signature(signature) == selector).then_some(*name)
    })
}

fn known_custom_errors() -> &'static [(&'static str, &'static str)] {
    &[
        ("InvalidSignature()", "InvalidSignature"),
        ("InvalidTrade()", "InvalidTrade"),
        ("Expired()", "Expired"),
        ("Unauthorized()", "Unauthorized"),
        ("NotAuthorized()", "NotAuthorized"),
        ("InvalidNonce()", "InvalidNonce"),
        ("NonceAlreadyUsed()", "NonceAlreadyUsed"),
        ("MarketNotActive()", "MarketNotActive"),
        ("MarketCloseOnly()", "MarketCloseOnly"),
        ("InsufficientMargin()", "InsufficientMargin"),
        ("InsufficientCollateral()", "InsufficientCollateral"),
        ("OraclePriceUnavailable()", "OraclePriceUnavailable"),
        ("StalePrice()", "StalePrice"),
        ("OracleStale()", "OracleStale"),
        ("InvalidPrice()", "InvalidPrice"),
        ("InvalidSize()", "InvalidSize"),
    ]
}

fn selector_for_signature(signature: &str) -> String {
    let hash = keccak256(signature.as_bytes());
    format!(
        "0x{:02x}{:02x}{:02x}{:02x}",
        hash[0], hash[1], hash[2], hash[3]
    )
}

fn selector_from_data(revert_data: Option<&str>) -> Option<String> {
    let hex = revert_data?.strip_prefix("0x")?;
    (hex.len() >= 8 && hex[..8].bytes().all(|byte| byte.is_ascii_hexdigit()))
        .then(|| format!("0x{}", &hex[..8].to_ascii_lowercase()))
}

fn decode_abi_string(bytes: &[u8]) -> Option<String> {
    let offset = word_to_usize(bytes.get(0..32)?)?;
    let length = word_to_usize(bytes.get(offset..offset + 32)?)?;
    let data_start = offset + 32;
    let data_end = data_start.checked_add(length)?;
    let string_bytes = bytes.get(data_start..data_end)?;
    String::from_utf8(string_bytes.to_vec()).ok()
}

fn word_to_usize(word: &[u8]) -> Option<usize> {
    let value = word_to_u128(word)?;
    usize::try_from(value).ok()
}

fn word_to_u128(word: &[u8]) -> Option<u128> {
    if word.len() != 32 || word[..16].iter().any(|byte| *byte != 0) {
        return None;
    }
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&word[16..]);
    Some(u128::from_be_bytes(bytes))
}

fn decode_hex_bytes(value: &str) -> Option<Vec<u8>> {
    let hex = value.strip_prefix("0x")?;
    if hex.len() % 2 != 0 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    let mut bytes = vec![0u8; hex.len() / 2];
    for (index, byte) in bytes.iter_mut().enumerate() {
        let high = decode_hex_nibble(hex.as_bytes()[index * 2])?;
        let low = decode_hex_nibble(hex.as_bytes()[index * 2 + 1])?;
        *byte = (high << 4) | low;
    }
    Some(bytes)
}

fn decode_hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn normalize_hex(value: &str) -> String {
    let Some(hex) = value.strip_prefix("0x") else {
        return value.to_string();
    };
    format!("0x{}", hex.to_ascii_lowercase())
}

fn looks_like_hex_data(value: &str) -> bool {
    value
        .strip_prefix("0x")
        .is_some_and(|hex| hex.len() >= 8 && hex.bytes().all(|byte| byte.is_ascii_hexdigit()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn decodes_error_string() {
        let data = error_string_data("Invalid signature");
        let decoded = decode_revert_data(&data);

        assert_eq!(decoded.kind, "solidity_error");
        assert_eq!(decoded.name.as_deref(), Some("Error"));
        assert_eq!(decoded.selector.as_deref(), Some("0x08c379a0"));
        assert_eq!(decoded.args, Some(vec!["Invalid signature".to_string()]));
    }

    #[test]
    fn decodes_panic() {
        let data = "0x4e487b710000000000000000000000000000000000000000000000000000000000000011";
        let decoded = decode_revert_data(data);

        assert_eq!(decoded.kind, "panic");
        assert_eq!(decoded.name.as_deref(), Some("Panic"));
        assert_eq!(decoded.args, Some(vec!["0x11".to_string()]));
    }

    #[test]
    fn decodes_unknown_custom_error_selector() {
        let decoded = decode_revert_data("0x12345678");

        assert_eq!(decoded.kind, "unknown_custom_error");
        assert_eq!(decoded.selector.as_deref(), Some("0x12345678"));
    }

    #[test]
    fn decodes_known_custom_error_selector() {
        let selector = selector_for_signature("InvalidSignature()");
        let decoded = decode_revert_data(&selector);

        assert_eq!(decoded.kind, "custom_error");
        assert_eq!(decoded.name.as_deref(), Some("InvalidSignature"));
        assert_eq!(decoded.selector.as_deref(), Some(selector.as_str()));
        assert_eq!(decoded.args, Some(Vec::new()));
    }

    #[test]
    fn extracts_revert_data_from_direct_error_data_string() {
        let error = json!({
            "message": "execution reverted",
            "data": "0x12345678"
        });

        assert_eq!(
            extract_revert_data_from_json(&error).as_deref(),
            Some("0x12345678")
        );
    }

    #[test]
    fn extracts_revert_data_from_original_error_data() {
        let error = json!({
            "message": "execution reverted",
            "data": {
                "originalError": {
                    "data": "0x12345678"
                }
            }
        });

        assert_eq!(
            extract_revert_data_from_json(&error).as_deref(),
            Some("0x12345678")
        );
    }

    #[test]
    fn extracts_revert_data_from_data_result() {
        let error = json!({
            "message": "execution reverted",
            "data": {
                "result": "0x12345678"
            }
        });

        assert_eq!(
            extract_revert_data_from_json(&error).as_deref(),
            Some("0x12345678")
        );
    }

    #[test]
    fn missing_revert_data_returns_missing_revert_data() {
        let diagnostics = diagnostics_from_rpc_error("execution reverted", None);

        assert_eq!(diagnostics.revert_data, None);
        assert_eq!(diagnostics.revert_selector, None);
        assert_eq!(diagnostics.decoded_error.kind, "missing_revert_data");
    }

    fn error_string_data(message: &str) -> String {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&[0x08, 0xc3, 0x79, 0xa0]);
        bytes.extend_from_slice(&word(32));
        bytes.extend_from_slice(&word(message.len() as u128));
        bytes.extend_from_slice(message.as_bytes());
        while bytes.len() % 32 != 4 {
            bytes.push(0);
        }
        hex_0x(&bytes)
    }

    fn word(value: u128) -> [u8; 32] {
        let mut word = [0u8; 32];
        word[16..].copy_from_slice(&value.to_be_bytes());
        word
    }

    fn hex_0x(bytes: &[u8]) -> String {
        let mut encoded = String::from("0x");
        for byte in bytes {
            encoded.push_str(&format!("{byte:02x}"));
        }
        encoded
    }
}
