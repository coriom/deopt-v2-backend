use super::{
    OptionExecutionIntent, OptionExecutionIntentStatus, OptionExecutionSimulationResult,
    OptionExecutionSimulationStatus, OptionsConfig,
};
use crate::error::{BackendError, Result};
use crate::execution::rpc::{EthCallProvider, EthCallRequest};
use crate::execution::transaction::{hex_0x, ExecutionTransactionRequest};
use crate::execution::ExecutionConfig;
use crate::execution::RevertDiagnostics;
use crate::signing::eip712::{keccak256, parse_evm_address, EIP712_DOMAIN_TYPE};
use crate::signing::Eip712Domain;
use crate::types::{now_ms, AccountId};
use alloy_primitives::{Address, Bytes, B256, U256};
use alloy_sol_types::{sol, SolCall};

pub const OPTION_TRADE_TYPE: &str = "OptionTrade(bytes32 intentId,address buyer,address seller,uint256 optionId,address underlying,address settlementAsset,uint64 expiry,uint64 strike1e8,bool isCall,uint128 contractSize1e8,uint128 quantity,uint128 premiumPerContract,bool buyerIsMaker,uint256 buyerNonce,uint256 sellerNonce,uint256 deadline)";

pub const OPTION_EXECUTE_TRADE_SIGNATURE: &str =
    "executeTrade((bytes32,address,address,uint256,address,address,uint64,uint64,bool,uint128,uint128,uint128,bool,uint256,uint256,uint256),bytes,bytes)";

sol! {
    struct OptionTrade {
        bytes32 intentId;
        address buyer;
        address seller;
        uint256 optionId;
        address underlying;
        address settlementAsset;
        uint64 expiry;
        uint64 strike1e8;
        bool isCall;
        uint128 contractSize1e8;
        uint128 quantity;
        uint128 premiumPerContract;
        bool buyerIsMaker;
        uint256 buyerNonce;
        uint256 sellerNonce;
        uint256 deadline;
    }

    function executeTrade(OptionTrade t, bytes buyerSig, bytes sellerSig);
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OptionTradePayload {
    pub intent_id: B256,
    pub buyer: AccountId,
    pub seller: AccountId,
    pub option_id: U256,
    pub underlying: AccountId,
    pub settlement_asset: AccountId,
    pub expiry: u64,
    pub strike_1e8: u64,
    pub is_call: bool,
    pub contract_size_1e8: u128,
    pub quantity: u128,
    pub premium_per_contract: u128,
    pub buyer_is_maker: bool,
    pub buyer_nonce: u128,
    pub seller_nonce: u128,
    pub deadline: u128,
}

impl OptionTradePayload {
    pub fn from_intent(intent: &OptionExecutionIntent) -> Result<Self> {
        let buyer_nonce = intent.buyer_nonce.ok_or_else(|| {
            BackendError::MissingExecutionMetadata("option buyer_nonce".to_string())
        })?;
        let seller_nonce = intent.seller_nonce.ok_or_else(|| {
            BackendError::MissingExecutionMetadata("option seller_nonce".to_string())
        })?;
        let payload = Self {
            intent_id: parse_bytes32(&intent.onchain_intent_id, "intentId")?,
            buyer: intent.buyer.clone(),
            seller: intent.seller.clone(),
            option_id: parse_u256(&intent.onchain_option_id, "optionId")?,
            underlying: intent.underlying.clone(),
            settlement_asset: intent.settlement_asset.clone(),
            expiry: intent.expiry,
            strike_1e8: u64::try_from(intent.strike_1e8).map_err(|_| {
                BackendError::InvalidOptionExecutionIntentState(
                    "strike_1e8 exceeds uint64".to_string(),
                )
            })?,
            is_call: intent.is_call,
            contract_size_1e8: intent.contract_size_1e8,
            quantity: intent.quantity_contracts,
            premium_per_contract: intent.premium_per_contract_native,
            buyer_is_maker: intent.buyer_is_maker,
            buyer_nonce,
            seller_nonce,
            deadline: u128::from(intent.deadline),
        };
        payload.validate()?;
        Ok(payload)
    }

    pub fn validate(&self) -> Result<()> {
        if self.intent_id == B256::ZERO {
            return Err(BackendError::InvalidOptionExecutionIntentState(
                "intentId must be nonzero".to_string(),
            ));
        }
        let buyer = parse_evm_address(&self.buyer)?;
        let seller = parse_evm_address(&self.seller)?;
        if buyer == seller {
            return Err(BackendError::InvalidOptionExecutionIntentState(
                "buyer and seller must differ".to_string(),
            ));
        }
        let underlying = parse_evm_address(&self.underlying)?;
        let settlement_asset = parse_evm_address(&self.settlement_asset)?;
        if is_zero_address(&buyer) || is_zero_address(&seller) {
            return Err(BackendError::InvalidOptionExecutionIntentState(
                "buyer and seller must be nonzero".to_string(),
            ));
        }
        if is_zero_address(&underlying) || is_zero_address(&settlement_asset) {
            return Err(BackendError::InvalidOptionExecutionIntentState(
                "underlying and settlementAsset must be nonzero addresses".to_string(),
            ));
        }
        if self.option_id == U256::ZERO {
            return Err(BackendError::InvalidOptionExecutionIntentState(
                "optionId must be nonzero".to_string(),
            ));
        }
        if self.expiry == 0 || self.strike_1e8 == 0 || self.contract_size_1e8 == 0 {
            return Err(BackendError::InvalidOptionExecutionIntentState(
                "option metadata must be nonzero".to_string(),
            ));
        }
        if self.quantity == 0 {
            return Err(BackendError::ZeroSize);
        }
        if self.premium_per_contract == 0 {
            return Err(BackendError::ZeroPrice);
        }
        if !option_id_matches_registry_metadata(self)? {
            return Err(BackendError::InvalidOptionExecutionIntentState(
                "optionId does not match option metadata for either isEuropean value".to_string(),
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OptionTradeSignatureBundle {
    pub buyer_signature: Vec<u8>,
    pub seller_signature: Vec<u8>,
}

impl OptionTradeSignatureBundle {
    pub fn new(buyer_signature: &str, seller_signature: &str) -> Result<Self> {
        Ok(Self {
            buyer_signature: decode_signature(buyer_signature)?,
            seller_signature: decode_signature(seller_signature)?,
        })
    }
}

pub fn option_execution_intent_id_to_b256(intent_id: &str) -> Result<B256> {
    let mapped = B256::from(keccak256(intent_id.as_bytes()));
    if mapped == B256::ZERO {
        return Err(BackendError::InvalidOptionExecutionIntentId);
    }
    Ok(mapped)
}

pub fn option_execution_intent_id_to_hex_bytes32(intent_id: &str) -> Result<String> {
    Ok(hex_0x(
        option_execution_intent_id_to_b256(intent_id)?.as_slice(),
    ))
}

pub fn normalize_u256_string(value: &str, field: &str) -> Result<String> {
    Ok(parse_u256(value, field)?.to_string())
}

pub fn option_trade_digest(payload: &OptionTradePayload, domain: &Eip712Domain) -> Result<String> {
    Ok(hex_0x(&option_trade_digest_bytes(payload, domain)?))
}

pub fn option_trade_digest_bytes(
    payload: &OptionTradePayload,
    domain: &Eip712Domain,
) -> Result<[u8; 32]> {
    let domain_separator = domain_separator(domain)?;
    let trade_hash = option_trade_hash(payload)?;
    let mut encoded = Vec::with_capacity(66);
    encoded.extend_from_slice(b"\x19\x01");
    encoded.extend_from_slice(&domain_separator);
    encoded.extend_from_slice(&trade_hash);
    Ok(keccak256(&encoded))
}

pub fn option_trade_typehash() -> [u8; 32] {
    keccak256(OPTION_TRADE_TYPE.as_bytes())
}

pub fn option_execute_trade_selector() -> [u8; 4] {
    executeTradeCall::SELECTOR
}

pub fn expected_option_execute_trade_selector() -> [u8; 4] {
    let hash = keccak256(OPTION_EXECUTE_TRADE_SIGNATURE.as_bytes());
    [hash[0], hash[1], hash[2], hash[3]]
}

pub fn option_product_registry_option_id(
    underlying: &AccountId,
    settlement_asset: &AccountId,
    expiry: u64,
    strike_1e8: u64,
    contract_size_1e8: u128,
    is_call: bool,
    is_european: bool,
) -> Result<U256> {
    let underlying = parse_evm_address(underlying)?;
    let settlement_asset = parse_evm_address(settlement_asset)?;
    let mut encoded = Vec::with_capacity(224);
    encoded.extend_from_slice(&encode_address(&underlying));
    encoded.extend_from_slice(&encode_address(&settlement_asset));
    encoded.extend_from_slice(&encode_u64(expiry));
    encoded.extend_from_slice(&encode_u64(strike_1e8));
    encoded.extend_from_slice(&encode_u128(contract_size_1e8));
    encoded.extend_from_slice(&encode_bool(is_call));
    encoded.extend_from_slice(&encode_bool(is_european));
    Ok(U256::from_be_slice(&keccak256(&encoded)))
}

pub fn encode_option_execute_trade_calldata(
    payload: &OptionTradePayload,
    signatures: &OptionTradeSignatureBundle,
) -> Result<Vec<u8>> {
    payload.validate()?;
    let call = executeTradeCall {
        t: OptionTrade {
            intentId: payload.intent_id,
            buyer: Address::from(parse_evm_address(&payload.buyer)?),
            seller: Address::from(parse_evm_address(&payload.seller)?),
            optionId: payload.option_id,
            underlying: Address::from(parse_evm_address(&payload.underlying)?),
            settlementAsset: Address::from(parse_evm_address(&payload.settlement_asset)?),
            expiry: payload.expiry,
            strike1e8: payload.strike_1e8,
            isCall: payload.is_call,
            contractSize1e8: payload.contract_size_1e8,
            quantity: payload.quantity,
            premiumPerContract: payload.premium_per_contract,
            buyerIsMaker: payload.buyer_is_maker,
            buyerNonce: U256::from(payload.buyer_nonce),
            sellerNonce: U256::from(payload.seller_nonce),
            deadline: U256::from(payload.deadline),
        },
        buyerSig: Bytes::from(signatures.buyer_signature.clone()),
        sellerSig: Bytes::from(signatures.seller_signature.clone()),
    };
    Ok(call.abi_encode())
}

pub fn build_option_execution_transaction_request(
    execution_config: &ExecutionConfig,
    options_config: &OptionsConfig,
    intent: &OptionExecutionIntent,
) -> Result<ExecutionTransactionRequest> {
    validate_broadcast_target(&options_config.matching_engine_address)?;
    validate_broadcast_intent(options_config, intent)?;
    let calldata = decode_0x_hex(
        intent
            .calldata
            .as_deref()
            .ok_or_else(missing_broadcast_calldata_error)?,
        "option execution calldata",
    )?;
    Ok(ExecutionTransactionRequest {
        intent_id: intent.intent_id,
        onchain_intent_id: intent.onchain_intent_id.clone(),
        from: execution_config.executor_from_address.clone(),
        to: options_config.matching_engine_address.clone(),
        value_wei: 0,
        calldata,
        chain_id: execution_config.executor_chain_id,
        gas_limit: option_execution_broadcast_gas_limit(execution_config, options_config)?,
        max_fee_per_gas_wei: execution_config.max_fee_per_gas_wei.clone(),
        max_priority_fee_per_gas_wei: execution_config.max_priority_fee_per_gas_wei.clone(),
    })
}

pub async fn simulate_option_execution_intent<P>(
    provider: &P,
    intent: &OptionExecutionIntent,
    matching_engine_address: &AccountId,
    from: &AccountId,
    gas_limit: u64,
) -> Result<OptionExecutionSimulationResult>
where
    P: EthCallProvider,
{
    validate_simulation_intent(intent)?;
    validate_simulation_target(matching_engine_address)?;
    parse_evm_address(from)?;
    let calldata = decode_0x_hex(
        intent
            .calldata
            .as_deref()
            .ok_or_else(missing_calldata_error)?,
        "option execution calldata",
    )?;
    let request = EthCallRequest {
        from: from.clone(),
        to: matching_engine_address.clone(),
        data: calldata,
        value: 0,
        gas_limit: (gas_limit > 0).then_some(gas_limit),
    };
    let simulated_at_ms = now_ms();
    match provider.eth_call(request).await {
        Ok(success) => Ok(OptionExecutionSimulationResult {
            intent_id: intent.intent_id,
            simulation_status: OptionExecutionSimulationStatus::SimulationOk,
            block_number: success.block_number,
            error: None,
            revert_data: None,
            revert_selector: None,
            simulated_at_ms,
        }),
        Err(error) => {
            let diagnostics = diagnostics_from_backend_error(&error);
            Ok(OptionExecutionSimulationResult {
                intent_id: intent.intent_id,
                simulation_status: OptionExecutionSimulationStatus::SimulationFailed,
                block_number: None,
                error: Some(error.to_string()),
                revert_data: diagnostics.revert_data,
                revert_selector: diagnostics.revert_selector,
                simulated_at_ms,
            })
        }
    }
}

pub fn option_execution_simulation_unavailable(
    intent_id: uuid::Uuid,
    error: impl Into<String>,
) -> OptionExecutionSimulationResult {
    OptionExecutionSimulationResult {
        intent_id,
        simulation_status: OptionExecutionSimulationStatus::SimulationUnavailable,
        block_number: None,
        error: Some(error.into()),
        revert_data: None,
        revert_selector: None,
        simulated_at_ms: now_ms(),
    }
}

pub fn option_execution_simulation_pending(
    intent: &OptionExecutionIntent,
) -> OptionExecutionSimulationResult {
    OptionExecutionSimulationResult {
        intent_id: intent.intent_id,
        simulation_status: intent
            .simulation_status
            .unwrap_or(OptionExecutionSimulationStatus::SimulationPending),
        block_number: intent.simulation_block_number,
        error: intent.simulation_error.clone(),
        revert_data: intent.simulation_revert_data.clone(),
        revert_selector: intent.simulation_revert_selector.clone(),
        simulated_at_ms: intent.simulated_at_ms.unwrap_or(0),
    }
}

pub fn validate_simulation_intent(intent: &OptionExecutionIntent) -> Result<()> {
    if intent.buyer_signature.is_none() || intent.seller_signature.is_none() {
        return Err(BackendError::MissingTradeSignatures);
    }
    let calldata = intent
        .calldata
        .as_deref()
        .ok_or_else(missing_calldata_error)?;
    if calldata == "0x" {
        return Err(missing_calldata_error());
    }
    if intent.status != OptionExecutionIntentStatus::CalldataReady {
        return Err(BackendError::InvalidOptionExecutionIntentState(
            "option execution intent must be calldata_ready for simulation".to_string(),
        ));
    }
    Ok(())
}

pub fn validate_simulation_target(target: &AccountId) -> Result<()> {
    let address = parse_evm_address(target).map_err(|_| {
        BackendError::Config(
            "OPTION_MATCHING_ENGINE_ADDRESS must be a valid address for option execution simulation"
                .to_string(),
        )
    })?;
    if is_zero_address(&address) {
        return Err(BackendError::Config(
            "OPTION_MATCHING_ENGINE_ADDRESS is required for option execution simulation"
                .to_string(),
        ));
    }
    Ok(())
}

fn validate_broadcast_intent(
    options_config: &OptionsConfig,
    intent: &OptionExecutionIntent,
) -> Result<()> {
    if intent.buyer_signature.is_none() || intent.seller_signature.is_none() {
        return Err(BackendError::MissingTradeSignatures);
    }
    let calldata = intent
        .calldata
        .as_deref()
        .ok_or_else(missing_broadcast_calldata_error)?;
    if calldata == "0x" {
        return Err(missing_broadcast_calldata_error());
    }
    if options_config.execution_require_simulation_ok
        && intent.simulation_status != Some(OptionExecutionSimulationStatus::SimulationOk)
    {
        return Err(BackendError::BroadcastRejected(
            "simulation_ok status is required before option execution broadcast".to_string(),
        ));
    }
    if !matches!(
        intent.status,
        OptionExecutionIntentStatus::CalldataReady
            | OptionExecutionIntentStatus::SimulationReady
            | OptionExecutionIntentStatus::SimulationOk
    ) {
        return Err(BackendError::InvalidOptionExecutionIntentState(
            "option execution intent must be calldata_ready before broadcast".to_string(),
        ));
    }
    Ok(())
}

fn validate_broadcast_target(target: &AccountId) -> Result<()> {
    let address = parse_evm_address(target).map_err(|_| {
        BackendError::Config(
            "OPTION_MATCHING_ENGINE_ADDRESS must be a valid address for option execution broadcast"
                .to_string(),
        )
    })?;
    if is_zero_address(&address) {
        return Err(BackendError::Config(
            "OPTION_MATCHING_ENGINE_ADDRESS is required for option execution broadcast".to_string(),
        ));
    }
    Ok(())
}

fn option_execution_broadcast_gas_limit(
    execution_config: &ExecutionConfig,
    options_config: &OptionsConfig,
) -> Result<u64> {
    if options_config.execution_broadcast_gas_limit > 0 {
        return Ok(options_config.execution_broadcast_gas_limit);
    }
    if execution_config.max_gas_limit == 0 {
        return Err(BackendError::Config(
            "EXECUTOR_MAX_GAS_LIMIT must be greater than zero for option execution broadcast"
                .to_string(),
        ));
    }
    Ok(execution_config.max_gas_limit)
}

fn missing_calldata_error() -> BackendError {
    BackendError::InvalidOptionExecutionIntentState(
        "option execution calldata is required for simulation".to_string(),
    )
}

fn missing_broadcast_calldata_error() -> BackendError {
    BackendError::InvalidOptionExecutionIntentState(
        "option execution calldata is required before broadcast".to_string(),
    )
}

fn diagnostics_from_backend_error(error: &BackendError) -> RevertDiagnostics {
    match error {
        BackendError::SimulationReverted(diagnostics) => diagnostics.as_ref().clone(),
        other => RevertDiagnostics::missing(other.to_string()),
    }
}

fn domain_separator(domain: &Eip712Domain) -> Result<[u8; 32]> {
    let verifying_contract = parse_evm_address(&domain.verifying_contract)?;
    let mut encoded = Vec::with_capacity(160);
    encoded.extend_from_slice(&keccak256(EIP712_DOMAIN_TYPE.as_bytes()));
    encoded.extend_from_slice(&keccak256(domain.name.as_bytes()));
    encoded.extend_from_slice(&keccak256(domain.version.as_bytes()));
    encoded.extend_from_slice(&encode_u64(domain.chain_id));
    encoded.extend_from_slice(&encode_address(&verifying_contract));
    Ok(keccak256(&encoded))
}

fn option_trade_hash(payload: &OptionTradePayload) -> Result<[u8; 32]> {
    payload.validate()?;
    let buyer = parse_evm_address(&payload.buyer)?;
    let seller = parse_evm_address(&payload.seller)?;
    let underlying = parse_evm_address(&payload.underlying)?;
    let settlement_asset = parse_evm_address(&payload.settlement_asset)?;

    let mut encoded = Vec::with_capacity(544);
    encoded.extend_from_slice(&option_trade_typehash());
    encoded.extend_from_slice(payload.intent_id.as_slice());
    encoded.extend_from_slice(&encode_address(&buyer));
    encoded.extend_from_slice(&encode_address(&seller));
    encoded.extend_from_slice(&encode_u256(&payload.option_id));
    encoded.extend_from_slice(&encode_address(&underlying));
    encoded.extend_from_slice(&encode_address(&settlement_asset));
    encoded.extend_from_slice(&encode_u64(payload.expiry));
    encoded.extend_from_slice(&encode_u64(payload.strike_1e8));
    encoded.extend_from_slice(&encode_bool(payload.is_call));
    encoded.extend_from_slice(&encode_u128(payload.contract_size_1e8));
    encoded.extend_from_slice(&encode_u128(payload.quantity));
    encoded.extend_from_slice(&encode_u128(payload.premium_per_contract));
    encoded.extend_from_slice(&encode_bool(payload.buyer_is_maker));
    encoded.extend_from_slice(&encode_u128(payload.buyer_nonce));
    encoded.extend_from_slice(&encode_u128(payload.seller_nonce));
    encoded.extend_from_slice(&encode_u128(payload.deadline));
    Ok(keccak256(&encoded))
}

fn option_id_matches_registry_metadata(payload: &OptionTradePayload) -> Result<bool> {
    let european_id = option_product_registry_option_id(
        &payload.underlying,
        &payload.settlement_asset,
        payload.expiry,
        payload.strike_1e8,
        payload.contract_size_1e8,
        payload.is_call,
        true,
    )?;
    let american_id = option_product_registry_option_id(
        &payload.underlying,
        &payload.settlement_asset,
        payload.expiry,
        payload.strike_1e8,
        payload.contract_size_1e8,
        payload.is_call,
        false,
    )?;
    Ok(payload.option_id == european_id || payload.option_id == american_id)
}

fn parse_u256(value: &str, field: &str) -> Result<U256> {
    let value = value.trim();
    if value.is_empty() {
        return Err(BackendError::InvalidOptionExecutionIntentState(format!(
            "{field} is required"
        )));
    }
    if let Some(hex) = value.strip_prefix("0x") {
        if hex.is_empty() || hex.len() > 64 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(BackendError::InvalidOptionExecutionIntentState(format!(
                "{field} must be a uint256"
            )));
        }
        let mut bytes = vec![0u8; 32];
        let offset = 32usize.checked_sub(hex.len().div_ceil(2)).ok_or_else(|| {
            BackendError::InvalidOptionExecutionIntentState(format!("{field} exceeds uint256"))
        })?;
        decode_hex_to_right_aligned_slice(hex, &mut bytes[offset..]).map_err(|_| {
            BackendError::InvalidOptionExecutionIntentState(format!("{field} must be a uint256"))
        })?;
        return Ok(U256::from_be_slice(&bytes));
    }
    if !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(BackendError::InvalidOptionExecutionIntentState(format!(
            "{field} must be a uint256"
        )));
    }
    U256::from_str_radix(value, 10).map_err(|error| {
        BackendError::InvalidOptionExecutionIntentState(format!(
            "{field} must be a uint256: {error}"
        ))
    })
}

fn parse_bytes32(value: &str, field: &str) -> Result<B256> {
    let hex = value.strip_prefix("0x").ok_or_else(|| {
        BackendError::InvalidOptionExecutionIntentState(format!(
            "{field} must be 0x-prefixed bytes32"
        ))
    })?;
    if hex.len() != 64 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(BackendError::InvalidOptionExecutionIntentState(format!(
            "{field} must be 0x-prefixed bytes32"
        )));
    }
    let mut bytes = [0u8; 32];
    decode_hex_to_slice(hex, &mut bytes).map_err(|_| {
        BackendError::InvalidOptionExecutionIntentState(format!(
            "{field} must be 0x-prefixed bytes32"
        ))
    })?;
    Ok(B256::from(bytes))
}

fn decode_signature(signature: &str) -> Result<Vec<u8>> {
    let Some(hex) = signature.strip_prefix("0x") else {
        return Err(BackendError::MalformedSignature);
    };
    if hex.len() != 130 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(BackendError::MalformedSignature);
    }
    let mut bytes = vec![0u8; 65];
    decode_hex_to_slice(hex, &mut bytes).map_err(|_| BackendError::MalformedSignature)?;
    Ok(bytes)
}

fn decode_0x_hex(value: &str, field: &str) -> Result<Vec<u8>> {
    let hex = value.strip_prefix("0x").ok_or_else(|| {
        BackendError::InvalidOptionExecutionIntentState(format!("{field} must be 0x-prefixed hex"))
    })?;
    if hex.is_empty() || hex.len() % 2 != 0 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(BackendError::InvalidOptionExecutionIntentState(format!(
            "{field} must be non-empty 0x-prefixed hex"
        )));
    }
    let mut bytes = vec![0u8; hex.len() / 2];
    decode_hex_to_slice(hex, &mut bytes).map_err(|_| {
        BackendError::InvalidOptionExecutionIntentState(format!(
            "{field} must be non-empty 0x-prefixed hex"
        ))
    })?;
    Ok(bytes)
}

fn encode_address(address: &[u8; 20]) -> [u8; 32] {
    let mut word = [0u8; 32];
    word[12..].copy_from_slice(address);
    word
}

fn encode_bool(value: bool) -> [u8; 32] {
    let mut word = [0u8; 32];
    word[31] = u8::from(value);
    word
}

fn encode_u64(value: u64) -> [u8; 32] {
    let mut word = [0u8; 32];
    word[24..].copy_from_slice(&value.to_be_bytes());
    word
}

fn encode_u128(value: u128) -> [u8; 32] {
    let mut word = [0u8; 32];
    word[16..].copy_from_slice(&value.to_be_bytes());
    word
}

fn encode_u256(value: &U256) -> [u8; 32] {
    value.to_be_bytes()
}

fn is_zero_address(address: &[u8; 20]) -> bool {
    address.iter().all(|byte| *byte == 0)
}

fn decode_hex_to_right_aligned_slice(hex: &str, out: &mut [u8]) -> std::result::Result<(), ()> {
    if hex.len() > out.len() * 2 {
        return Err(());
    }
    if hex.len() % 2 == 1 {
        let first = decode_hex_nibble(hex.as_bytes()[0])?;
        out[0] = first;
        decode_hex_to_slice(&hex[1..], &mut out[1..])
    } else {
        decode_hex_to_slice(hex, out)
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::AccountId;

    #[test]
    fn option_trade_typehash_is_stable() {
        assert_eq!(
            hex_0x(&option_trade_typehash()),
            "0x13e38748a4a77f7a9e8be3a50bff4ada1cf49f08922ee9091926c38fc8532e52"
        );
    }

    #[test]
    fn option_execute_trade_selector_matches_signature() {
        assert_eq!(
            option_execute_trade_selector(),
            expected_option_execute_trade_selector()
        );
    }

    #[test]
    fn option_trade_digest_is_deterministic() {
        let domain = Eip712Domain {
            name: "DeOptV2-OptionMatchingEngine".to_string(),
            version: "1".to_string(),
            chain_id: 84532,
            verifying_contract: AccountId::new("0x00000000000000000000000000000000000000ee"),
        };
        let payload = payload();

        assert_eq!(
            option_trade_digest(&payload, &domain).unwrap(),
            option_trade_digest(&payload, &domain).unwrap()
        );
    }

    #[test]
    fn option_execute_trade_calldata_has_selector() {
        let calldata =
            encode_option_execute_trade_calldata(&payload(), &signature_bundle()).unwrap();

        assert!(!calldata.is_empty());
        assert_eq!(&calldata[..4], option_execute_trade_selector().as_slice());
    }

    #[test]
    fn option_product_registry_option_id_matches_active_base_sepolia_series() {
        let option_id = option_product_registry_option_id(
            &AccountId::new("0x4DeEBc5f537F3b8ba0E3393807B4D699D72bDd02"),
            &AccountId::new("0x6eAe407f5640B006faC9965182e238582A3B412E"),
            1_893_456_000,
            300_000_000_000,
            100_000_000,
            true,
            true,
        )
        .unwrap();

        assert_eq!(
            option_id.to_string(),
            "24145907678156652148089862289363692212069910767044828147380657249455352740183"
        );
    }

    #[test]
    fn option_trade_payload_rejects_option_id_metadata_mismatch() {
        let mut payload = payload();
        payload.expiry += 1;

        let error = payload.validate().unwrap_err();

        assert!(matches!(
            error,
            BackendError::InvalidOptionExecutionIntentState(message)
                if message.contains("optionId does not match option metadata")
        ));
    }

    #[test]
    fn option_execute_trade_calldata_decodes_expected_trade_fields() {
        let payload = payload();
        let calldata = encode_option_execute_trade_calldata(&payload, &signature_bundle()).unwrap();
        let decoded = executeTradeCall::abi_decode(&calldata, true).unwrap();

        assert_eq!(decoded.t.intentId, payload.intent_id);
        assert_eq!(
            decoded.t.buyer,
            Address::from(parse_evm_address(&payload.buyer).unwrap())
        );
        assert_eq!(
            decoded.t.seller,
            Address::from(parse_evm_address(&payload.seller).unwrap())
        );
        assert_eq!(decoded.t.optionId, payload.option_id);
        assert_eq!(
            decoded.t.underlying,
            Address::from(parse_evm_address(&payload.underlying).unwrap())
        );
        assert_eq!(
            decoded.t.settlementAsset,
            Address::from(parse_evm_address(&payload.settlement_asset).unwrap())
        );
        assert_eq!(decoded.t.expiry, payload.expiry);
        assert_eq!(decoded.t.strike1e8, payload.strike_1e8);
        assert_eq!(decoded.t.isCall, payload.is_call);
        assert_eq!(decoded.t.contractSize1e8, 100_000_000);
        assert_eq!(decoded.t.quantity, 3);
        assert_eq!(decoded.t.premiumPerContract, payload.premium_per_contract);
        assert_eq!(decoded.t.buyerIsMaker, payload.buyer_is_maker);
        assert_eq!(decoded.t.buyerNonce, U256::from(payload.buyer_nonce));
        assert_eq!(decoded.t.sellerNonce, U256::from(payload.seller_nonce));
        assert_eq!(decoded.t.deadline, U256::from(payload.deadline));
    }

    fn payload() -> OptionTradePayload {
        let underlying = AccountId::new("0x0000000000000000000000000000000000000010");
        let settlement_asset = AccountId::new("0x0000000000000000000000000000000000000020");
        let expiry = 4_102_444_800;
        let strike_1e8 = 300_000_000_000;
        let contract_size_1e8 = 100_000_000;
        let is_call = true;
        OptionTradePayload {
            intent_id: option_execution_intent_id_to_b256("00000000-0000-0000-0000-000000000001")
                .unwrap(),
            buyer: AccountId::new("0x0000000000000000000000000000000000000001"),
            seller: AccountId::new("0x0000000000000000000000000000000000000002"),
            option_id: option_product_registry_option_id(
                &underlying,
                &settlement_asset,
                expiry,
                strike_1e8,
                contract_size_1e8,
                is_call,
                true,
            )
            .unwrap(),
            underlying,
            settlement_asset,
            expiry,
            strike_1e8,
            is_call,
            contract_size_1e8,
            quantity: 3,
            premium_per_contract: 10_000_000,
            buyer_is_maker: false,
            buyer_nonce: 0,
            seller_nonce: 0,
            deadline: 0,
        }
    }

    fn signature_bundle() -> OptionTradeSignatureBundle {
        OptionTradeSignatureBundle::new(&signature_hex(0xaa), &signature_hex(0xbb)).unwrap()
    }

    fn signature_hex(byte: u8) -> String {
        let mut signature = String::from("0x");
        for _ in 0..65 {
            signature.push_str(&format!("{byte:02x}"));
        }
        signature
    }
}
