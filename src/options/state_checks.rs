use crate::error::{BackendError, Result};
use crate::execution::{EthCallProvider, EthCallRequest};
use crate::signing::eip712::{keccak256, parse_evm_address};
use crate::types::{AccountId, TimestampMs};
use alloy_primitives::U256;
use serde::Serialize;

use super::{OptionExecutionIntent, OptionExecutionTransaction};

pub const NONCES_VIEW: &str = "nonces(address)";
pub const MATCHING_MARGIN_ENGINE_VIEW: &str = "marginEngine()";
pub const POSITION_QUANTITY_VIEW: &str = "getPositionQuantity(address,uint256)";
pub const POSITIONS_VIEW: &str = "positions(address,uint256)";
pub const SERIES_SHORT_OPEN_INTEREST_VIEW: &str = "seriesShortOpenInterest(uint256)";
pub const MARGIN_COLLATERAL_VAULT_VIEW: &str = "collateralVault()";
pub const VAULT_BALANCES_VIEW: &str = "balances(address,address)";
pub const VAULT_BALANCE_WITH_YIELD_VIEW: &str = "balanceWithYield(address,address)";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OptionStateCheckEvaluation {
    pub details: serde_json::Value,
    pub overall_status: String,
    pub nonce_check_status: String,
    pub position_check_status: String,
    pub vault_check_status: String,
    pub strict_mismatch_reasons: Vec<String>,
}

impl OptionStateCheckEvaluation {
    pub fn skipped_no_provider(
        checked_at_ms: TimestampMs,
        strict: bool,
        require_rpc: bool,
        reason: &str,
    ) -> Self {
        let details = serde_json::json!({
            "enabled": true,
            "checked_at_ms": checked_at_ms,
            "strict": strict,
            "require_rpc": require_rpc,
            "overall_status": "skipped",
            "nonce_check_status": "skipped",
            "position_check_status": "skipped",
            "vault_check_status": "skipped",
            "reason": reason,
        });
        Self {
            details,
            overall_status: "skipped".to_string(),
            nonce_check_status: "skipped".to_string(),
            position_check_status: "skipped".to_string(),
            vault_check_status: "skipped".to_string(),
            strict_mismatch_reasons: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct StateCheckItem {
    status: String,
    view: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    expected_min: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    expected_delta: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    actual: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
}

impl StateCheckItem {
    fn ok_with_min(view: &'static str, expected_min: U256, actual: U256) -> Self {
        Self {
            status: "ok".to_string(),
            view,
            expected_min: Some(expected_min.to_string()),
            expected_delta: None,
            actual: Some(actual.to_string()),
            reason: None,
        }
    }

    fn ok_with_delta(view: &'static str, expected_delta: i128, actual: i128) -> Self {
        Self {
            status: "ok".to_string(),
            view,
            expected_min: None,
            expected_delta: Some(expected_delta.to_string()),
            actual: Some(actual.to_string()),
            reason: None,
        }
    }

    fn failed_min(view: &'static str, expected_min: U256, actual: U256, reason: String) -> Self {
        Self {
            status: "failed".to_string(),
            view,
            expected_min: Some(expected_min.to_string()),
            expected_delta: None,
            actual: Some(actual.to_string()),
            reason: Some(reason),
        }
    }

    fn failed_delta(
        view: &'static str,
        expected_delta: i128,
        actual: i128,
        reason: String,
    ) -> Self {
        Self {
            status: "failed".to_string(),
            view,
            expected_min: None,
            expected_delta: Some(expected_delta.to_string()),
            actual: Some(actual.to_string()),
            reason: Some(reason),
        }
    }

    fn skipped(view: &'static str, reason: impl Into<String>) -> Self {
        Self {
            status: "skipped".to_string(),
            view,
            expected_min: None,
            expected_delta: None,
            actual: None,
            reason: Some(reason.into()),
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn evaluate_option_state_checks<P>(
    provider: &P,
    intent: &OptionExecutionIntent,
    transaction: &OptionExecutionTransaction,
    matching_engine_hint: &AccountId,
    margin_engine_hint: &AccountId,
    collateral_vault_hint: &AccountId,
    strict: bool,
    require_rpc: bool,
    checked_at_ms: TimestampMs,
) -> OptionStateCheckEvaluation
where
    P: EthCallProvider,
{
    let matching_engine = usable_address(matching_engine_hint)
        .or_else(|| usable_address(&transaction.to))
        .unwrap_or_else(|| AccountId::new(""));

    let mut strict_mismatch_reasons = Vec::new();
    let mut warnings = Vec::new();

    if usable_address(&matching_engine).is_none() {
        let nonce_status = "skipped".to_string();
        let position_status = "skipped".to_string();
        let vault_status = "skipped".to_string();
        let overall_status = "skipped".to_string();
        let details = serde_json::json!({
            "enabled": true,
            "checked_at_ms": checked_at_ms,
            "strict": strict,
            "require_rpc": require_rpc,
            "overall_status": overall_status,
            "nonce_check_status": nonce_status,
            "position_check_status": position_status,
            "vault_check_status": vault_status,
            "reason": "matching_engine_address_unconfigured",
        });
        return OptionStateCheckEvaluation {
            details,
            overall_status,
            nonce_check_status: nonce_status,
            position_check_status: position_status,
            vault_check_status: vault_status,
            strict_mismatch_reasons,
        };
    }

    let buyer_nonce = check_nonce(
        provider,
        &matching_engine,
        "buyer_nonce",
        &intent.buyer,
        intent.buyer_nonce,
    )
    .await;
    let seller_nonce = check_nonce(
        provider,
        &matching_engine,
        "seller_nonce",
        &intent.seller,
        intent.seller_nonce,
    )
    .await;
    collect_item_issue(
        "buyer_nonce",
        &buyer_nonce,
        &mut strict_mismatch_reasons,
        &mut warnings,
    );
    collect_item_issue(
        "seller_nonce",
        &seller_nonce,
        &mut strict_mismatch_reasons,
        &mut warnings,
    );
    let nonce_check_status = aggregate_required_status([&buyer_nonce, &seller_nonce]);

    let margin_engine = match usable_address(margin_engine_hint) {
        Some(address) => Some(address),
        None => read_address_view(provider, &matching_engine, MATCHING_MARGIN_ENGINE_VIEW).await,
    };

    let option_id = parse_u256(&intent.onchain_option_id).ok();
    let (buyer_position, seller_position, open_interest, collateral_vault) = if let (
        Some(margin_engine),
        Some(option_id),
    ) =
        (margin_engine.as_ref(), option_id.as_ref())
    {
        let buyer_position = check_position(
            provider,
            margin_engine,
            "buyer_position",
            &intent.buyer,
            option_id,
            intent.quantity_contracts,
            PositionSide::Buyer,
        )
        .await;
        let seller_position = check_position(
            provider,
            margin_engine,
            "seller_position",
            &intent.seller,
            option_id,
            intent.quantity_contracts,
            PositionSide::Seller,
        )
        .await;
        let open_interest = read_open_interest(
            provider,
            margin_engine,
            option_id,
            intent.quantity_contracts,
        )
        .await;
        let collateral_vault = match usable_address(collateral_vault_hint) {
            Some(address) => Some(address),
            None => read_address_view(provider, margin_engine, MARGIN_COLLATERAL_VAULT_VIEW).await,
        };
        (
            buyer_position,
            seller_position,
            open_interest,
            collateral_vault,
        )
    } else {
        let reason = if margin_engine.is_none() {
            "margin_engine_view_unavailable"
        } else {
            "option_id_not_uint256"
        };
        (
            StateCheckItem::skipped(POSITION_QUANTITY_VIEW, reason),
            StateCheckItem::skipped(POSITION_QUANTITY_VIEW, reason),
            serde_json::json!({
                "status": "skipped",
                "view": SERIES_SHORT_OPEN_INTEREST_VIEW,
                "reason": reason,
            }),
            None,
        )
    };

    collect_item_issue(
        "buyer_position",
        &buyer_position,
        &mut strict_mismatch_reasons,
        &mut warnings,
    );
    collect_item_issue(
        "seller_position",
        &seller_position,
        &mut strict_mismatch_reasons,
        &mut warnings,
    );
    let position_check_status = aggregate_required_status([&buyer_position, &seller_position]);
    let vault = read_vault_observations(
        provider,
        collateral_vault.as_ref(),
        &intent.buyer,
        &intent.seller,
        &intent.settlement_asset,
    )
    .await;
    let vault_check_status = vault
        .get("status")
        .and_then(|value| value.as_str())
        .unwrap_or("skipped")
        .to_string();

    let overall_status = aggregate_overall_status(&nonce_check_status, &position_check_status);
    let details = serde_json::json!({
        "enabled": true,
        "checked_at_ms": checked_at_ms,
        "strict": strict,
        "require_rpc": require_rpc,
        "overall_status": overall_status,
        "nonce_check_status": nonce_check_status,
        "position_check_status": position_check_status,
        "vault_check_status": vault_check_status,
        "matching_engine_address": matching_engine.0.to_ascii_lowercase(),
        "margin_engine_address": margin_engine.as_ref().map(|value| value.0.to_ascii_lowercase()),
        "collateral_vault_address": collateral_vault.as_ref().map(|value| value.0.to_ascii_lowercase()),
        "buyer_nonce": buyer_nonce,
        "seller_nonce": seller_nonce,
        "buyer_position": buyer_position,
        "seller_position": seller_position,
        "open_interest": open_interest,
        "vault": vault,
        "warnings": warnings,
        "mismatches": strict_mismatch_reasons,
    });

    OptionStateCheckEvaluation {
        details,
        overall_status,
        nonce_check_status,
        position_check_status,
        vault_check_status,
        strict_mismatch_reasons,
    }
}

async fn check_nonce<P>(
    provider: &P,
    matching_engine: &AccountId,
    label: &str,
    account: &AccountId,
    expected_previous: Option<u128>,
) -> StateCheckItem
where
    P: EthCallProvider,
{
    let Some(expected_previous) = expected_previous else {
        return StateCheckItem::skipped(NONCES_VIEW, format!("{label}_missing_expected_nonce"));
    };
    let expected_min = U256::from(expected_previous.saturating_add(1));
    let output = match call_view(
        provider,
        matching_engine,
        encode_address_call(NONCES_VIEW, account),
    )
    .await
    {
        Ok(output) => output,
        Err(_) => return StateCheckItem::skipped(NONCES_VIEW, "nonces_view_unavailable"),
    };
    let actual = match decode_uint256(&output) {
        Ok(value) => value,
        Err(_) => return StateCheckItem::skipped(NONCES_VIEW, "nonces_view_invalid_output"),
    };
    if actual >= expected_min {
        StateCheckItem::ok_with_min(NONCES_VIEW, expected_min, actual)
    } else {
        StateCheckItem::failed_min(
            NONCES_VIEW,
            expected_min,
            actual,
            format!("{label} actual nonce is below expected minimum"),
        )
    }
}

#[derive(Clone, Copy)]
enum PositionSide {
    Buyer,
    Seller,
}

async fn check_position<P>(
    provider: &P,
    margin_engine: &AccountId,
    label: &str,
    account: &AccountId,
    option_id: &U256,
    quantity: u128,
    side: PositionSide,
) -> StateCheckItem
where
    P: EthCallProvider,
{
    let expected_delta = match side {
        PositionSide::Buyer => match i128::try_from(quantity) {
            Ok(value) => value,
            Err(_) => {
                return StateCheckItem::skipped(POSITION_QUANTITY_VIEW, "quantity_exceeds_int128")
            }
        },
        PositionSide::Seller => match i128::try_from(quantity) {
            Ok(value) => value.saturating_neg(),
            Err(_) => {
                return StateCheckItem::skipped(POSITION_QUANTITY_VIEW, "quantity_exceeds_int128")
            }
        },
    };
    let output = match call_view(
        provider,
        margin_engine,
        encode_address_u256_call(POSITION_QUANTITY_VIEW, account, option_id),
    )
    .await
    {
        Ok(output) => output,
        Err(_) => {
            return StateCheckItem::skipped(
                POSITION_QUANTITY_VIEW,
                "getPositionQuantity_view_unavailable",
            )
        }
    };
    let actual = match decode_int128(&output) {
        Ok(value) => value,
        Err(_) => {
            return StateCheckItem::skipped(
                POSITION_QUANTITY_VIEW,
                "getPositionQuantity_invalid_output",
            )
        }
    };
    let passes = match side {
        PositionSide::Buyer => actual >= expected_delta,
        PositionSide::Seller => actual <= expected_delta,
    };
    if passes {
        StateCheckItem::ok_with_delta(POSITION_QUANTITY_VIEW, expected_delta, actual)
    } else {
        StateCheckItem::failed_delta(
            POSITION_QUANTITY_VIEW,
            expected_delta,
            actual,
            format!("{label} does not include expected trade delta"),
        )
    }
}

async fn read_open_interest<P>(
    provider: &P,
    margin_engine: &AccountId,
    option_id: &U256,
    quantity: u128,
) -> serde_json::Value
where
    P: EthCallProvider,
{
    let output = match call_view(
        provider,
        margin_engine,
        encode_u256_call(SERIES_SHORT_OPEN_INTEREST_VIEW, option_id),
    )
    .await
    {
        Ok(output) => output,
        Err(_) => {
            return serde_json::json!({
                "status": "skipped",
                "view": SERIES_SHORT_OPEN_INTEREST_VIEW,
                "reason": "seriesShortOpenInterest_view_unavailable",
            })
        }
    };
    match decode_uint256(&output) {
        Ok(actual) => serde_json::json!({
            "status": "ok",
            "view": SERIES_SHORT_OPEN_INTEREST_VIEW,
            "comparison": "observed_only",
            "expected_min_from_trade": quantity.to_string(),
            "actual": actual.to_string(),
        }),
        Err(_) => serde_json::json!({
            "status": "skipped",
            "view": SERIES_SHORT_OPEN_INTEREST_VIEW,
            "reason": "seriesShortOpenInterest_invalid_output",
        }),
    }
}

async fn read_vault_observations<P>(
    provider: &P,
    collateral_vault: Option<&AccountId>,
    buyer: &AccountId,
    seller: &AccountId,
    settlement_asset: &AccountId,
) -> serde_json::Value
where
    P: EthCallProvider,
{
    let Some(collateral_vault) = collateral_vault else {
        return serde_json::json!({
            "status": "skipped",
            "view": VAULT_BALANCES_VIEW,
            "reason": "collateral_vault_view_unavailable",
        });
    };

    let buyer_balance =
        read_vault_balance(provider, collateral_vault, buyer, settlement_asset).await;
    let seller_balance =
        read_vault_balance(provider, collateral_vault, seller, settlement_asset).await;
    let status = if buyer_balance.is_some() && seller_balance.is_some() {
        "ok"
    } else {
        "skipped"
    };
    serde_json::json!({
        "status": status,
        "view": VAULT_BALANCES_VIEW,
        "comparison": "observed_only",
        "reason": "insufficient_pre_trade_baseline_for_balance_delta",
        "buyer_settlement_balance": buyer_balance,
        "seller_settlement_balance": seller_balance,
    })
}

async fn read_vault_balance<P>(
    provider: &P,
    collateral_vault: &AccountId,
    account: &AccountId,
    token: &AccountId,
) -> Option<String>
where
    P: EthCallProvider,
{
    let output = call_view(
        provider,
        collateral_vault,
        encode_two_address_call(VAULT_BALANCES_VIEW, account, token),
    )
    .await
    .ok()?;
    decode_uint256(&output).ok().map(|value| value.to_string())
}

async fn read_address_view<P>(
    provider: &P,
    contract: &AccountId,
    signature: &'static str,
) -> Option<AccountId>
where
    P: EthCallProvider,
{
    let output = call_view(provider, contract, encode_no_arg_call(signature))
        .await
        .ok()?;
    decode_address(&output)
        .ok()
        .and_then(|address| usable_address(&address))
}

async fn call_view<P>(provider: &P, to: &AccountId, data: Result<Vec<u8>>) -> Result<Vec<u8>>
where
    P: EthCallProvider,
{
    let data = data?;
    provider
        .eth_call(EthCallRequest {
            from: zero_address(),
            to: to.clone(),
            data,
            value: 0,
            gas_limit: None,
        })
        .await
        .map(|success| success.output)
}

fn collect_item_issue(
    label: &str,
    item: &StateCheckItem,
    mismatches: &mut Vec<String>,
    warnings: &mut Vec<String>,
) {
    match item.status.as_str() {
        "failed" => {
            let reason = item.reason.clone().unwrap_or_else(|| "failed".to_string());
            mismatches.push(format!("{label}: {reason}"));
        }
        "skipped" => {
            let reason = item.reason.clone().unwrap_or_else(|| "skipped".to_string());
            warnings.push(format!("{label}: {reason}"));
        }
        _ => {}
    }
}

fn aggregate_required_status<const N: usize>(items: [&StateCheckItem; N]) -> String {
    if items.iter().any(|item| item.status == "failed") {
        return "failed".to_string();
    }
    if items.iter().all(|item| item.status == "ok") {
        return "ok".to_string();
    }
    if items.iter().all(|item| item.status == "skipped") {
        return "skipped".to_string();
    }
    "warning".to_string()
}

fn aggregate_overall_status(nonce_status: &str, position_status: &str) -> String {
    if nonce_status == "failed" || position_status == "failed" {
        return "failed".to_string();
    }
    if nonce_status == "ok" && position_status == "ok" {
        return "ok".to_string();
    }
    "warning".to_string()
}

fn usable_address(address: &AccountId) -> Option<AccountId> {
    if address.0.trim().is_empty()
        || address
            .0
            .eq_ignore_ascii_case("0x0000000000000000000000000000000000000000")
    {
        return None;
    }
    parse_evm_address(address)
        .ok()
        .map(|_| AccountId::new(address.0.to_ascii_lowercase()))
}

fn encode_no_arg_call(signature: &str) -> Result<Vec<u8>> {
    Ok(selector(signature).to_vec())
}

fn encode_address_call(signature: &str, account: &AccountId) -> Result<Vec<u8>> {
    let address = parse_evm_address(account)?;
    let mut calldata = Vec::with_capacity(36);
    calldata.extend_from_slice(&selector(signature));
    calldata.extend_from_slice(&encode_address_word(&address));
    Ok(calldata)
}

fn encode_two_address_call(
    signature: &str,
    left: &AccountId,
    right: &AccountId,
) -> Result<Vec<u8>> {
    let left = parse_evm_address(left)?;
    let right = parse_evm_address(right)?;
    let mut calldata = Vec::with_capacity(68);
    calldata.extend_from_slice(&selector(signature));
    calldata.extend_from_slice(&encode_address_word(&left));
    calldata.extend_from_slice(&encode_address_word(&right));
    Ok(calldata)
}

fn encode_address_u256_call(signature: &str, account: &AccountId, value: &U256) -> Result<Vec<u8>> {
    let address = parse_evm_address(account)?;
    let mut calldata = Vec::with_capacity(68);
    calldata.extend_from_slice(&selector(signature));
    calldata.extend_from_slice(&encode_address_word(&address));
    calldata.extend_from_slice(&value.to_be_bytes::<32>());
    Ok(calldata)
}

fn encode_u256_call(signature: &str, value: &U256) -> Result<Vec<u8>> {
    let mut calldata = Vec::with_capacity(36);
    calldata.extend_from_slice(&selector(signature));
    calldata.extend_from_slice(&value.to_be_bytes::<32>());
    Ok(calldata)
}

fn selector(signature: &str) -> [u8; 4] {
    let hash = keccak256(signature.as_bytes());
    [hash[0], hash[1], hash[2], hash[3]]
}

fn encode_address_word(address: &[u8; 20]) -> [u8; 32] {
    let mut word = [0u8; 32];
    word[12..].copy_from_slice(address);
    word
}

fn decode_uint256(output: &[u8]) -> Result<U256> {
    if output.len() != 32 {
        return Err(BackendError::Simulation(
            "view returned invalid uint256 output".to_string(),
        ));
    }
    Ok(U256::from_be_slice(output))
}

fn decode_int128(output: &[u8]) -> Result<i128> {
    if output.len() != 32 {
        return Err(BackendError::Simulation(
            "view returned invalid int128 output".to_string(),
        ));
    }
    let negative = output[16] & 0x80 != 0;
    let pad = if negative { 0xff } else { 0x00 };
    if output[..16].iter().any(|byte| *byte != pad) {
        return Err(BackendError::Simulation(
            "view returned out-of-range int128 output".to_string(),
        ));
    }
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&output[16..32]);
    Ok(i128::from_be_bytes(bytes))
}

fn decode_address(output: &[u8]) -> Result<AccountId> {
    if output.len() != 32 || output[..12].iter().any(|byte| *byte != 0) {
        return Err(BackendError::Simulation(
            "view returned invalid address output".to_string(),
        ));
    }
    Ok(AccountId::new(hex_0x(&output[12..32])))
}

fn parse_u256(value: &str) -> Result<U256> {
    let value = value.trim();
    if let Some(hex) = value.strip_prefix("0x") {
        if hex.is_empty() || hex.len() > 64 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(BackendError::InvalidOptionExecutionIntentState(
                "optionId must be a uint256".to_string(),
            ));
        }
        let mut bytes = [0u8; 32];
        let byte_len = hex.len().div_ceil(2);
        let offset = 32usize.checked_sub(byte_len).ok_or_else(|| {
            BackendError::InvalidOptionExecutionIntentState("optionId exceeds uint256".to_string())
        })?;
        decode_hex_to_right_aligned_slice(hex, &mut bytes[offset..]).map_err(|_| {
            BackendError::InvalidOptionExecutionIntentState(
                "optionId must be a uint256".to_string(),
            )
        })?;
        return Ok(U256::from_be_slice(&bytes));
    }
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(BackendError::InvalidOptionExecutionIntentState(
            "optionId must be a uint256".to_string(),
        ));
    }
    U256::from_str_radix(value, 10).map_err(|error| {
        BackendError::InvalidOptionExecutionIntentState(format!(
            "optionId must be a uint256: {error}"
        ))
    })
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
    for (index, slot) in out.iter_mut().enumerate() {
        let high = decode_hex_nibble(hex.as_bytes()[index * 2])?;
        let low = decode_hex_nibble(hex.as_bytes()[index * 2 + 1])?;
        *slot = (high << 4) | low;
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

fn zero_address() -> AccountId {
    AccountId::new("0x0000000000000000000000000000000000000000")
}

fn hex_0x(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(2 + bytes.len() * 2);
    out.push_str("0x");
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

#[cfg(test)]
pub(crate) fn encode_test_uint256(value: u128) -> Vec<u8> {
    U256::from(value).to_be_bytes::<32>().to_vec()
}

#[cfg(test)]
pub(crate) fn encode_test_int128(value: i128) -> Vec<u8> {
    let mut out = if value < 0 { [0xffu8; 32] } else { [0u8; 32] };
    out[16..32].copy_from_slice(&value.to_be_bytes());
    out.to_vec()
}

#[cfg(test)]
pub(crate) fn encode_test_address(address: &AccountId) -> Vec<u8> {
    let parsed = parse_evm_address(address).unwrap();
    encode_address_word(&parsed).to_vec()
}

#[cfg(test)]
pub(crate) fn selector_hex(signature: &str) -> String {
    hex_0x(&selector(signature))
}
