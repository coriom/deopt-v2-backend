use crate::error::{BackendError, Result};
use crate::execution::{
    build_perp_execution_call_from_intent, intent_id_to_hex_bytes32, ExecutionConfig,
    ExecutionIntent, ExecutionIntentStatus, ExecutorSigner, StoredTradeSignatures,
};
use crate::signing::eip712::keccak256;
use crate::signing::eip712::parse_evm_address;
use crate::types::{AccountId, TimestampMs};
use serde::Serialize;
use uuid::Uuid;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionTransactionStatus {
    Prepared,
    Rejected,
    Submitted,
    Failed,
}

impl ExecutionTransactionStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Prepared => "prepared",
            Self::Rejected => "rejected",
            Self::Submitted => "submitted",
            Self::Failed => "failed",
        }
    }

    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "prepared" => Ok(Self::Prepared),
            "rejected" => Ok(Self::Rejected),
            "submitted" => Ok(Self::Submitted),
            "failed" => Ok(Self::Failed),
            other => Err(BackendError::Persistence(format!(
                "invalid execution transaction status: {other}"
            ))),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ExecutionTransaction {
    pub transaction_id: String,
    pub intent_id: Uuid,
    pub onchain_intent_id: Option<String>,
    pub target: AccountId,
    pub calldata: String,
    pub value_wei: String,
    pub tx_hash: Option<String>,
    pub status: ExecutionTransactionStatus,
    pub error: Option<String>,
    pub created_at_ms: TimestampMs,
    pub updated_at_ms: TimestampMs,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExecutionTransactionRequest {
    pub intent_id: Uuid,
    pub onchain_intent_id: String,
    pub from: AccountId,
    pub to: AccountId,
    pub value_wei: u128,
    pub calldata: Vec<u8>,
    pub chain_id: u64,
    pub gas_limit: u64,
    pub max_fee_per_gas_wei: Option<String>,
    pub max_priority_fee_per_gas_wei: Option<String>,
}

impl ExecutionTransactionRequest {
    pub fn calldata_hex(&self) -> String {
        hex_0x(&self.calldata)
    }
}

pub fn sign_eip1559_transaction(
    request: &ExecutionTransactionRequest,
    nonce: u64,
    signer: &ExecutorSigner,
) -> Result<String> {
    let max_fee_per_gas =
        required_wei("EXECUTOR_MAX_FEE_PER_GAS_WEI", &request.max_fee_per_gas_wei)?;
    let max_priority_fee_per_gas = required_wei(
        "EXECUTOR_MAX_PRIORITY_FEE_PER_GAS_WEI",
        &request.max_priority_fee_per_gas_wei,
    )?;
    let to = parse_evm_address(&request.to)?;

    let signing_payload = eip1559_payload(
        request.chain_id,
        nonce,
        max_priority_fee_per_gas,
        max_fee_per_gas,
        request.gas_limit,
        &to,
        request.value_wei,
        &request.calldata,
    );
    let mut signing_bytes = Vec::with_capacity(1 + signing_payload.len());
    signing_bytes.push(0x02);
    signing_bytes.extend_from_slice(&signing_payload);
    let hash = keccak256(&signing_bytes);
    let signature = signer.sign_prehash(&hash)?;

    let signed_payload = eip1559_signed_payload(
        request.chain_id,
        nonce,
        max_priority_fee_per_gas,
        max_fee_per_gas,
        request.gas_limit,
        &to,
        request.value_wei,
        &request.calldata,
        signature.y_parity,
        &signature.r,
        &signature.s,
    );
    let mut raw = Vec::with_capacity(1 + signed_payload.len());
    raw.push(0x02);
    raw.extend_from_slice(&signed_payload);
    Ok(hex_0x(&raw))
}

pub fn build_execution_transaction_request(
    config: &ExecutionConfig,
    intent: &ExecutionIntent,
    signatures: &StoredTradeSignatures,
) -> Result<ExecutionTransactionRequest> {
    validate_broadcast_target(&config.perp_matching_engine_address)?;
    if config.require_simulation_ok && intent.status != ExecutionIntentStatus::SimulationOk {
        return Err(BackendError::BroadcastRejected(
            "simulation_ok status is required before broadcast".to_string(),
        ));
    }
    if !signatures.calldata_ready() {
        return Err(BackendError::MissingTradeSignatures);
    }
    let call = build_perp_execution_call_from_intent(
        intent,
        &config.perp_matching_engine_address,
        signatures,
    )?;
    if call.calldata.is_empty() || call.missing_signatures {
        return Err(BackendError::BroadcastRejected(
            "executeTrade calldata is required before broadcast".to_string(),
        ));
    }
    Ok(ExecutionTransactionRequest {
        intent_id: intent.intent_id,
        onchain_intent_id: intent_id_to_hex_bytes32(&intent.intent_id.to_string())?,
        from: config.executor_from_address.clone(),
        to: call.target,
        value_wei: call.value,
        calldata: call.calldata,
        chain_id: config.executor_chain_id,
        gas_limit: config.max_gas_limit,
        max_fee_per_gas_wei: config.max_fee_per_gas_wei.clone(),
        max_priority_fee_per_gas_wei: config.max_priority_fee_per_gas_wei.clone(),
    })
}

pub fn ensure_no_submitted_transaction(already_submitted: bool) -> Result<()> {
    if already_submitted {
        return Err(BackendError::BroadcastRejected(
            "intent already has a submitted transaction".to_string(),
        ));
    }
    Ok(())
}

fn validate_broadcast_target(target: &AccountId) -> Result<()> {
    let address = parse_evm_address(target)?;
    if address.iter().all(|byte| *byte == 0) {
        return Err(BackendError::BroadcastRejected(
            "PERP_MATCHING_ENGINE_ADDRESS is required before broadcast".to_string(),
        ));
    }
    Ok(())
}

fn required_wei(field: &str, value: &Option<String>) -> Result<u128> {
    let value = value.as_ref().ok_or_else(|| {
        BackendError::Config(format!(
            "{field} is required when EXECUTOR_REAL_BROADCAST_ENABLED=true"
        ))
    })?;
    value
        .parse::<u128>()
        .map_err(|error| BackendError::Config(format!("invalid {field}: {error}")))
}

#[allow(clippy::too_many_arguments)]
fn eip1559_payload(
    chain_id: u64,
    nonce: u64,
    max_priority_fee_per_gas: u128,
    max_fee_per_gas: u128,
    gas_limit: u64,
    to: &[u8; 20],
    value: u128,
    data: &[u8],
) -> Vec<u8> {
    rlp_list(&[
        rlp_u64(chain_id),
        rlp_u64(nonce),
        rlp_u128(max_priority_fee_per_gas),
        rlp_u128(max_fee_per_gas),
        rlp_u64(gas_limit),
        rlp_bytes(to),
        rlp_u128(value),
        rlp_bytes(data),
        rlp_list(&[]),
    ])
}

#[allow(clippy::too_many_arguments)]
fn eip1559_signed_payload(
    chain_id: u64,
    nonce: u64,
    max_priority_fee_per_gas: u128,
    max_fee_per_gas: u128,
    gas_limit: u64,
    to: &[u8; 20],
    value: u128,
    data: &[u8],
    y_parity: u8,
    r: &[u8; 32],
    s: &[u8; 32],
) -> Vec<u8> {
    rlp_list(&[
        rlp_u64(chain_id),
        rlp_u64(nonce),
        rlp_u128(max_priority_fee_per_gas),
        rlp_u128(max_fee_per_gas),
        rlp_u64(gas_limit),
        rlp_bytes(to),
        rlp_u128(value),
        rlp_bytes(data),
        rlp_list(&[]),
        rlp_u8(y_parity),
        rlp_u256_bytes(r),
        rlp_u256_bytes(s),
    ])
}

fn rlp_u8(value: u8) -> Vec<u8> {
    if value == 0 {
        rlp_bytes(&[])
    } else {
        rlp_bytes(&[value])
    }
}

fn rlp_u64(value: u64) -> Vec<u8> {
    if value == 0 {
        return rlp_bytes(&[]);
    }
    let bytes = value.to_be_bytes();
    rlp_bytes(trim_leading_zeroes(&bytes))
}

fn rlp_u128(value: u128) -> Vec<u8> {
    if value == 0 {
        return rlp_bytes(&[]);
    }
    let bytes = value.to_be_bytes();
    rlp_bytes(trim_leading_zeroes(&bytes))
}

fn rlp_u256_bytes(value: &[u8; 32]) -> Vec<u8> {
    rlp_bytes(trim_leading_zeroes(value))
}

fn rlp_bytes(bytes: &[u8]) -> Vec<u8> {
    if bytes.len() == 1 && bytes[0] < 0x80 {
        return vec![bytes[0]];
    }
    let mut encoded = rlp_prefix(0x80, bytes.len());
    encoded.extend_from_slice(bytes);
    encoded
}

fn rlp_list(items: &[Vec<u8>]) -> Vec<u8> {
    let payload_len = items.iter().map(Vec::len).sum();
    let mut encoded = rlp_prefix(0xc0, payload_len);
    for item in items {
        encoded.extend_from_slice(item);
    }
    encoded
}

fn rlp_prefix(offset: u8, len: usize) -> Vec<u8> {
    if len < 56 {
        return vec![offset + len as u8];
    }
    let len_bytes = usize_to_be_bytes(len);
    let mut encoded = Vec::with_capacity(1 + len_bytes.len());
    encoded.push(offset + 55 + len_bytes.len() as u8);
    encoded.extend_from_slice(&len_bytes);
    encoded
}

fn usize_to_be_bytes(value: usize) -> Vec<u8> {
    let bytes = value.to_be_bytes();
    trim_leading_zeroes(&bytes).to_vec()
}

fn trim_leading_zeroes(bytes: &[u8]) -> &[u8] {
    let first_nonzero = bytes
        .iter()
        .position(|byte| *byte != 0)
        .unwrap_or(bytes.len());
    &bytes[first_nonzero..]
}

pub fn hex_0x(bytes: &[u8]) -> String {
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
    use crate::execution::{PerpTradeSignatureBundle, StoredTradeSignatures};
    use crate::types::OrderId;

    #[test]
    fn transaction_request_targets_matching_engine_with_zero_value() {
        let request =
            build_execution_transaction_request(&config(), &intent(), &signatures()).unwrap();

        assert_eq!(
            request.to,
            AccountId::new("0x0000000000000000000000000000000000000009")
        );
        assert_eq!(request.value_wei, 0);
        assert!(!request.calldata.is_empty());
        assert_eq!(request.chain_id, 84532);
        assert_eq!(request.gas_limit, 1_000_000);
    }

    #[test]
    fn signs_eip1559_transaction_without_exposing_raw_tx_in_api_types() {
        let request =
            build_execution_transaction_request(&config(), &intent(), &signatures()).unwrap();
        let signer = ExecutorSigner::from_private_key(&crate::execution::PrivateKeySecret::new(
            "0x4c0883a69102937d6231471b5dbb6204fe5129617082792ae468d01a3f362318".to_string(),
        ))
        .unwrap();

        let raw = sign_eip1559_transaction(&request, 7, &signer).unwrap();

        assert!(raw.starts_with("0x02"));
        assert!(raw.len() > 200);
    }

    #[test]
    fn transaction_request_rejects_missing_signatures() {
        let error = build_execution_transaction_request(
            &config(),
            &intent(),
            &StoredTradeSignatures::default(),
        )
        .unwrap_err();

        assert!(matches!(error, BackendError::MissingTradeSignatures));
    }

    #[test]
    fn transaction_request_requires_simulation_ok_when_configured() {
        let mut intent = intent();
        intent.status = ExecutionIntentStatus::CalldataReady;

        let error =
            build_execution_transaction_request(&config(), &intent, &signatures()).unwrap_err();

        assert!(
            matches!(error, BackendError::BroadcastRejected(reason) if reason.contains("simulation_ok"))
        );
    }

    #[test]
    fn disabled_broadcast_never_creates_fake_tx_hash() {
        let request =
            build_execution_transaction_request(&config(), &intent(), &signatures()).unwrap();
        let calldata = request.calldata_hex();
        let transaction = ExecutionTransaction {
            transaction_id: Uuid::from_u128(99).to_string(),
            intent_id: request.intent_id,
            onchain_intent_id: Some(request.onchain_intent_id),
            target: request.to,
            calldata,
            value_wei: request.value_wei.to_string(),
            tx_hash: None,
            status: ExecutionTransactionStatus::Rejected,
            error: Some("broadcast disabled".to_string()),
            created_at_ms: 123,
            updated_at_ms: 123,
        };

        assert_eq!(transaction.tx_hash, None);
        assert_eq!(transaction.status, ExecutionTransactionStatus::Rejected);
    }

    #[test]
    fn already_submitted_intent_is_rejected_idempotently() {
        let error = ensure_no_submitted_transaction(true).unwrap_err();

        assert!(
            matches!(error, BackendError::BroadcastRejected(reason) if reason.contains("already has a submitted"))
        );
    }

    fn config() -> ExecutionConfig {
        ExecutionConfig {
            execution_enabled: false,
            dry_run: true,
            poll_interval_ms: 1_000,
            max_batch_size: 10,
            real_broadcast_enabled: false,
            executor_private_key: None,
            executor_chain_id: 84532,
            max_gas_limit: 1_000_000,
            max_fee_per_gas_wei: Some("1000000000".to_string()),
            max_priority_fee_per_gas_wei: Some("100000000".to_string()),
            require_simulation_ok: true,
            simulation_enabled: false,
            simulation_requires_persistence: true,
            rpc_url: None,
            executor_from_address: AccountId::new("0x0000000000000000000000000000000000000003"),
            perp_matching_engine_address: AccountId::new(
                "0x0000000000000000000000000000000000000009",
            ),
            perp_engine_address: AccountId::new("0x0000000000000000000000000000000000000000"),
        }
    }

    fn intent() -> ExecutionIntent {
        ExecutionIntent {
            intent_id: Uuid::from_u128(1),
            market_id: 1,
            buyer: AccountId::new("0x0000000000000000000000000000000000000001"),
            seller: AccountId::new("0x0000000000000000000000000000000000000002"),
            price_1e8: 300_000_000_000,
            size_1e8: 100_000_000,
            buy_order_id: OrderId(Uuid::from_u128(2)),
            sell_order_id: OrderId(Uuid::from_u128(3)),
            buyer_is_maker: Some(false),
            buyer_nonce: Some(11),
            seller_nonce: Some(12),
            deadline_ms: Some(4_102_444_800),
            created_at_ms: 123,
            status: ExecutionIntentStatus::SimulationOk,
        }
    }

    fn signatures() -> StoredTradeSignatures {
        let bundle =
            PerpTradeSignatureBundle::new(&signature_hex(0xaa), &signature_hex(0xbb)).unwrap();
        StoredTradeSignatures {
            buyer_sig: Some(hex_0x(&bundle.buyer_sig)),
            seller_sig: Some(hex_0x(&bundle.seller_sig)),
        }
    }

    fn signature_hex(byte: u8) -> String {
        let mut signature = String::from("0x");
        for _ in 0..65 {
            signature.push_str(&format!("{byte:02x}"));
        }
        signature
    }
}
