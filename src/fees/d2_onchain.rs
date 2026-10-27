//! Read-only D2 policy verification. Approval and a queued Timelock operation are
//! never substitutes for a canonical, pinned on-chain readback.
use alloy_primitives::keccak256;
use serde_json::{json, Value};
use std::time::Duration;

pub const BASE_SEPOLIA_CHAIN_ID: u64 = 84532;
pub const D2_ADDRESS: &str = "0x9947f42cc29a32992bbf2030ee49e35c839eded5";
pub const TIMELOCK_ADDRESS: &str = "0xa67f8e8e673ce4bb2fb563b0e6e9fa8f70e3b588";
/// Canonical D2 deployment postflight runtime hash in the frozen protocol repo.
pub const D2_RUNTIME_KECCAK256: &str =
    "0x732101d67fb112f6d3159f32852071bbf995f8bd1995335cf9a2b0f167d08726";
pub const APPROVED_PERP_MAKER_PPM: i32 = 0;
pub const APPROVED_PERP_TAKER_PPM: i32 = 200;
const PPM_DENOMINATOR: u128 = 1_000_000;
pub const D2_OPERATION_IDS: [&str; 5] = [
    "0x1976519a08a3a7e65cbe1a2f77fcf0f5fa0f783ccb6c7ef0e6c87b7481caf45b",
    "0xb79986810ad3e08b5495a8edeaed0654da513ef6acfa15e8ff1d496ff2ef4a11",
    "0x48b66ca38c21064a6605af4038a657a191a56f327784dc1e0d0bffdc21075a62",
    "0xaaf8ae230522b4a6b08a365be6e87f2544392e729b559db44968912b3a177f76",
    "0x9ec2e37332d4dbb27791ae7f4de7e852147b344f58210e61adb7b2f95a8d5b49",
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FeeActivationState {
    PolicyApproved,
    TimelockQueued,
    CanonicallyExecuted,
    OnchainReadbackVerified,
    TradingEligible,
}

/// Constructible only after live canonical receipts and a fresh pinned D2
/// readback agree. This is a fee-source proof, not trading authorization.
pub struct VerifiedD2FeePolicy {
    snapshot: D2Snapshot,
}

impl VerifiedD2FeePolicy {
    pub fn snapshot(&self) -> &D2Snapshot {
        &self.snapshot
    }
    pub fn activation_state(&self) -> FeeActivationState {
        FeeActivationState::OnchainReadbackVerified
    }
    pub fn taker_fee(&self, basis: u128, now: u64, max_age_seconds: u64) -> Result<u128, String> {
        self.snapshot.validate(now, max_age_seconds)?;
        approved_taker_fee(basis).ok_or_else(|| "D2 fee arithmetic overflow".into())
    }
}

pub async fn verify_d2_fee_policy(
    rpc_url: &str,
    now: u64,
    max_age_seconds: u64,
    execution_tx_hashes: &[String; 5],
) -> Result<VerifiedD2FeePolicy, String> {
    let snapshot = read_pinned_d2(rpc_url).await?;
    snapshot.validate(now, max_age_seconds)?;
    verify_d2_execution_receipts(rpc_url, &snapshot, execution_tx_hashes).await?;
    Ok(VerifiedD2FeePolicy { snapshot })
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct D2Snapshot {
    pub chain_id: u64,
    pub block_number: u64,
    pub block_hash: String,
    pub block_timestamp: u64,
    pub contract: String,
    pub owner: String,
    pub recipient: String,
    pub rebate_funding_account: String,
    pub protocol_fee_vault: String,
    pub profiles: [(i32, i32); 5],
    pub rfq_discounts: [(u32, u32); 5],
}

impl D2Snapshot {
    /// This is only a fee-configuration gate. Other market, maintenance,
    /// accounting and explicit runtime authorization gates remain mandatory.
    pub fn validate(&self, now: u64, max_age_seconds: u64) -> Result<(), String> {
        if self.chain_id != BASE_SEPOLIA_CHAIN_ID || self.contract != D2_ADDRESS {
            return Err("wrong D2 chain or contract".into());
        }
        if !is_hash(&self.block_hash)
            || self.block_timestamp > now
            || now - self.block_timestamp > max_age_seconds
        {
            return Err("D2 block identity or freshness invalid".into());
        }
        if self.owner != TIMELOCK_ADDRESS || self.recipient != TIMELOCK_ADDRESS {
            return Err("D2 owner or recipient drift".into());
        }
        if self
            .profiles
            .iter()
            .any(|p| *p != (APPROVED_PERP_MAKER_PPM, APPROVED_PERP_TAKER_PPM))
        {
            return Err("D2 PERP profile drift or partial execution".into());
        }
        if self.rfq_discounts.iter().any(|p| *p != (0, 0)) {
            return Err("D2 PERP RFQ discount drift".into());
        }
        if self.rebate_funding_account != "0x0000000000000000000000000000000000000000" {
            return Err("D2 rebate funding account unexpectedly enabled".into());
        }
        if self.protocol_fee_vault != "0x0000000000000000000000000000000000000000" {
            return Err("D2 fee vault hook unexpectedly enabled".into());
        }
        Ok(())
    }
}

/// D2 charges positive fees with Math.mulDiv(..., Rounding.Ceil). A 200 ppm
/// rate is 2 bps, and there is no maker rebate when makerPpm == 0.
pub fn approved_taker_fee(basis: u128) -> Option<u128> {
    basis
        .checked_mul(APPROVED_PERP_TAKER_PPM as u128)?
        .checked_add(PPM_DENOMINATOR - 1)
        .map(|numerator| numerator / PPM_DENOMINATOR)
}

/// D2 selects a trader's tier from its claimed-tier storage (and claim
/// expiry), not from the backend's legacy rolling-volume schedule.
pub async fn read_pinned_account_tier(
    rpc_url: &str,
    snapshot: &D2Snapshot,
    account: &str,
    now: u64,
    max_age_seconds: u64,
) -> Result<u8, String> {
    snapshot.validate(now, max_age_seconds)?;
    let account_bytes = hex::decode(account.strip_prefix("0x").ok_or("account not hex")?)
        .map_err(|e| e.to_string())?;
    if account_bytes.len() != 20 {
        return Err("account is not 20 bytes".into());
    }
    let mut data = keccak256("currentTier(address)".as_bytes())[..4].to_vec();
    data.extend([0u8; 12]);
    data.extend(account_bytes);
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(8))
        .build()
        .map_err(|e| e.to_string())?;
    let result = rpc(
        &client,
        rpc_url,
        "eth_call",
        json!([{"to": D2_ADDRESS, "data": format!("0x{}", hex::encode(data))},
            {"blockHash": snapshot.block_hash}]),
    )
    .await?;
    let bytes = hex::decode(
        result
            .as_str()
            .ok_or("tier call non-hex")?
            .strip_prefix("0x")
            .ok_or("tier call non-hex")?,
    )
    .map_err(|e| e.to_string())?;
    if bytes.len() != 32 {
        return Err("tier ABI length invalid".into());
    }
    let canonical = rpc(
        &client,
        rpc_url,
        "eth_getBlockByNumber",
        json!([format!("0x{:x}", snapshot.block_number), false]),
    )
    .await?;
    if canonical
        .get("hash")
        .and_then(Value::as_str)
        .map(str::to_ascii_lowercase)
        != Some(snapshot.block_hash.clone())
    {
        return Err("D2 account-tier block reorganized".into());
    }
    let tier = unsigned_u32(&bytes)?;
    u8::try_from(tier)
        .ok()
        .filter(|tier| *tier < 5)
        .ok_or_else(|| "on-chain tier out of range".into())
}

pub async fn read_pinned_d2(rpc_url: &str) -> Result<D2Snapshot, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(8))
        .build()
        .map_err(|e| e.to_string())?;
    let chain_id = quantity(&rpc(&client, rpc_url, "eth_chainId", json!([])).await?)?;
    if chain_id != BASE_SEPOLIA_CHAIN_ID {
        return Err("wrong RPC chain".into());
    }
    // `safe` avoids relying on an unfinalized tip. Calls use EIP-1898's
    // blockHash selector; providers lacking it fail closed.
    let block = rpc(
        &client,
        rpc_url,
        "eth_getBlockByNumber",
        json!(["safe", false]),
    )
    .await?;
    let block_number = quantity(block.get("number").ok_or("missing block number")?)?;
    let block_timestamp = quantity(block.get("timestamp").ok_or("missing block timestamp")?)?;
    let block_hash = block
        .get("hash")
        .and_then(Value::as_str)
        .filter(|s| is_hash(s))
        .ok_or("missing block hash")?
        .to_ascii_lowercase();
    let code = rpc(
        &client,
        rpc_url,
        "eth_getCode",
        json!([D2_ADDRESS, {"blockHash": block_hash}]),
    )
    .await?;
    let code_hex = code.as_str().ok_or("D2 code absent")?;
    let code_bytes = hex::decode(code_hex.strip_prefix("0x").ok_or("D2 code not hex")?)
        .map_err(|e| e.to_string())?;
    if format!("0x{}", hex::encode(keccak256(&code_bytes))) != D2_RUNTIME_KECCAK256 {
        return Err("D2 runtime hash drift".into());
    }
    let mut profiles = [(0, 0); 5];
    let mut rfq_discounts = [(0, 0); 5];
    for tier in 0..5 {
        let args = [tier as u8, 1u8]; // ProductKind.PERP == 1
        let fee = call(
            &client,
            rpc_url,
            &block_hash,
            "getFeeProfile(uint8,uint8)",
            &args,
        )
        .await?;
        profiles[tier] = (signed_i32(word(&fee, 0)?)?, signed_i32(word(&fee, 1)?)?);
        let rfq = call(
            &client,
            rpc_url,
            &block_hash,
            "getRfqDiscountProfile(uint8,uint8)",
            &args,
        )
        .await?;
        rfq_discounts[tier] = (unsigned_u32(word(&rfq, 0)?)?, unsigned_u32(word(&rfq, 1)?)?);
    }
    let owner = address(word(
        &call(&client, rpc_url, &block_hash, "owner()", &[]).await?,
        0,
    )?)?;
    let recipient = address(word(
        &call(&client, rpc_url, &block_hash, "feeRecipient()", &[]).await?,
        0,
    )?)?;
    let rebate_funding_account = address(word(
        &call(&client, rpc_url, &block_hash, "rebateFundingAccount()", &[]).await?,
        0,
    )?)?;
    let protocol_fee_vault = address(word(
        &call(&client, rpc_url, &block_hash, "protocolFeeVault()", &[]).await?,
        0,
    )?)?;
    // Detect a changed canonical identity before returning the snapshot.
    let after = rpc(
        &client,
        rpc_url,
        "eth_getBlockByNumber",
        json!([format!("0x{block_number:x}"), false]),
    )
    .await?;
    if after
        .get("hash")
        .and_then(Value::as_str)
        .map(str::to_ascii_lowercase)
        != Some(block_hash.clone())
    {
        return Err("D2 pinned block reorganized during read".into());
    }
    Ok(D2Snapshot {
        chain_id,
        block_number,
        block_hash,
        block_timestamp,
        contract: D2_ADDRESS.into(),
        owner,
        recipient,
        rebate_funding_account,
        protocol_fee_vault,
        profiles,
        rfq_discounts,
    })
}

/// Verify five independently supplied public S2 transaction hashes against
/// canonical receipts and the exact custom Timelock execution events. An
/// absent hash, a queued operation or an outer success without the inner
/// Timelock log fails closed. Call this on every activation assessment; a
/// cached receipt proof is invalid after a chain reorganization.
pub async fn verify_d2_execution_receipts(
    rpc_url: &str,
    snapshot: &D2Snapshot,
    tx_hashes: &[String; 5],
) -> Result<(), String> {
    if snapshot.chain_id != BASE_SEPOLIA_CHAIN_ID || snapshot.contract != D2_ADDRESS {
        return Err("wrong D2 snapshot identity".into());
    }
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(8))
        .build()
        .map_err(|e| e.to_string())?;
    if quantity(&rpc(&client, rpc_url, "eth_chainId", json!([])).await?)? != BASE_SEPOLIA_CHAIN_ID {
        return Err("wrong RPC chain".into());
    }
    let event_topic = format!(
        "0x{}",
        hex::encode(keccak256(
            "TransactionExecuted(bytes32,address,uint256,bytes,uint256,bytes)".as_bytes()
        ))
    );
    let mut distinct = std::collections::HashSet::new();
    for (tier, tx_hash) in tx_hashes.iter().enumerate() {
        if !is_hash(tx_hash) || !distinct.insert(tx_hash.to_ascii_lowercase()) {
            return Err("missing or duplicate D2 S2 transaction hash".into());
        }
        let receipt = rpc(
            &client,
            rpc_url,
            "eth_getTransactionReceipt",
            json!([tx_hash]),
        )
        .await?;
        if !receipt
            .get("transactionHash")
            .and_then(Value::as_str)
            .is_some_and(|v| v.eq_ignore_ascii_case(tx_hash))
        {
            return Err("D2 receipt transaction identity mismatch".into());
        }
        if quantity(receipt.get("status").ok_or("receipt status missing")?)? != 1 {
            return Err(format!("D2 tier {tier} receipt failed"));
        }
        let number = quantity(receipt.get("blockNumber").ok_or("receipt block missing")?)?;
        if number > snapshot.block_number {
            return Err("D2 receipt newer than readback".into());
        }
        let canonical = rpc(
            &client,
            rpc_url,
            "eth_getBlockByNumber",
            json!([format!("0x{number:x}"), false]),
        )
        .await?;
        let receipt_hash = receipt
            .get("blockHash")
            .and_then(Value::as_str)
            .ok_or("receipt block hash missing")?;
        if canonical
            .get("hash")
            .and_then(Value::as_str)
            .map(str::to_ascii_lowercase)
            != Some(receipt_hash.to_ascii_lowercase())
        {
            return Err("noncanonical D2 receipt".into());
        }
        let logs = receipt
            .get("logs")
            .and_then(Value::as_array)
            .ok_or("receipt logs missing")?;
        let exact = logs
            .iter()
            .filter(|log| execution_log_matches(log, &event_topic, D2_OPERATION_IDS[tier]))
            .count();
        if exact != 1 {
            return Err(format!(
                "D2 tier {tier} exact Timelock execution event absent"
            ));
        }
        let mut queued_data = keccak256("queuedTransactions(bytes32)".as_bytes())[..4].to_vec();
        queued_data.extend(hex::decode(&D2_OPERATION_IDS[tier][2..]).map_err(|e| e.to_string())?);
        let queued = rpc(
            &client,
            rpc_url,
            "eth_call",
            json!([
                {"to": TIMELOCK_ADDRESS, "data": format!("0x{}", hex::encode(queued_data))},
                {"blockHash": snapshot.block_hash}
            ]),
        )
        .await?;
        let queued_bytes = hex::decode(
            queued
                .as_str()
                .ok_or("queuedTransactions non-hex")?
                .strip_prefix("0x")
                .ok_or("queuedTransactions non-hex")?,
        )
        .map_err(|e| e.to_string())?;
        if queued_bytes.len() != 32 || queued_bytes.iter().any(|b| *b != 0) {
            return Err(format!("D2 tier {tier} remains queued or malformed"));
        }
    }
    let snapshot_block = rpc(
        &client,
        rpc_url,
        "eth_getBlockByNumber",
        json!([format!("0x{:x}", snapshot.block_number), false]),
    )
    .await?;
    if snapshot_block
        .get("hash")
        .and_then(Value::as_str)
        .map(str::to_ascii_lowercase)
        != Some(snapshot.block_hash.clone())
    {
        return Err("D2 readback block reorganized".into());
    }
    Ok(())
}

fn execution_log_matches(log: &Value, event_topic: &str, operation_id: &str) -> bool {
    log.get("address")
        .and_then(Value::as_str)
        .is_some_and(|v| v.eq_ignore_ascii_case(TIMELOCK_ADDRESS))
        && log
            .get("topics")
            .and_then(Value::as_array)
            .is_some_and(|topics| {
                topics.len() == 3
                    && topics[0]
                        .as_str()
                        .is_some_and(|v| v.eq_ignore_ascii_case(event_topic))
                    && topics[1]
                        .as_str()
                        .is_some_and(|v| v.eq_ignore_ascii_case(operation_id))
                    && topics[2].as_str().is_some_and(|v| {
                        v.len() == 66
                            && v.get(26..)
                                .is_some_and(|tail| tail.eq_ignore_ascii_case(&D2_ADDRESS[2..]))
                    })
            })
}

async fn call(
    client: &reqwest::Client,
    url: &str,
    block_hash: &str,
    signature: &str,
    args: &[u8],
) -> Result<Vec<u8>, String> {
    let mut data = keccak256(signature.as_bytes())[..4].to_vec();
    for arg in args {
        data.extend([0u8; 31]);
        data.push(*arg);
    }
    let result = rpc(client, url, "eth_call", json!([{"to": D2_ADDRESS, "data": format!("0x{}", hex::encode(data))}, {"blockHash": block_hash}])).await?;
    let hex = result.as_str().ok_or("eth_call returned non-hex")?;
    hex::decode(hex.strip_prefix("0x").ok_or("eth_call lacked 0x")?).map_err(|e| e.to_string())
}

async fn rpc(
    client: &reqwest::Client,
    url: &str,
    method: &str,
    params: Value,
) -> Result<Value, String> {
    let response: Value = client
        .post(url)
        .json(&json!({"jsonrpc":"2.0","id":1,"method":method,"params":params}))
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;
    if response.get("error").is_some() {
        return Err(format!("{method} RPC error"));
    }
    response
        .get("result")
        .filter(|r| !r.is_null())
        .cloned()
        .ok_or_else(|| format!("{method} missing result"))
}

fn quantity(v: &Value) -> Result<u64, String> {
    let s = v.as_str().ok_or("quantity is not a string")?;
    u64::from_str_radix(s.strip_prefix("0x").ok_or("quantity lacks 0x")?, 16)
        .map_err(|e| e.to_string())
}
fn is_hash(s: &str) -> bool {
    s.len() == 66 && s.starts_with("0x") && s[2..].bytes().all(|c| c.is_ascii_hexdigit())
}
fn word(data: &[u8], index: usize) -> Result<&[u8], String> {
    data.get(index * 32..(index + 1) * 32)
        .ok_or_else(|| "short ABI result".into())
}
fn signed_i32(w: &[u8]) -> Result<i32, String> {
    let raw: [u8; 4] = w[28..32].try_into().map_err(|_| "short int32")?;
    let value = i32::from_be_bytes(raw);
    if w[..28]
        .iter()
        .any(|b| *b != if value < 0 { 0xff } else { 0 })
    {
        return Err("invalid int32 sign extension".into());
    }
    Ok(value)
}
fn unsigned_u32(w: &[u8]) -> Result<u32, String> {
    if w[..28].iter().any(|b| *b != 0) {
        return Err("invalid uint32 padding".into());
    }
    Ok(u32::from_be_bytes(
        w[28..32].try_into().map_err(|_| "short uint32")?,
    ))
}
fn address(w: &[u8]) -> Result<String, String> {
    if w[..12].iter().any(|b| *b != 0) {
        return Err("invalid address padding".into());
    }
    Ok(format!("0x{}", hex::encode(&w[12..32])))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn valid() -> D2Snapshot {
        D2Snapshot {
            chain_id: BASE_SEPOLIA_CHAIN_ID,
            block_number: 1,
            block_hash: format!("0x{}", "ab".repeat(32)),
            block_timestamp: 1000,
            contract: D2_ADDRESS.into(),
            owner: TIMELOCK_ADDRESS.into(),
            recipient: TIMELOCK_ADDRESS.into(),
            rebate_funding_account: "0x0000000000000000000000000000000000000000".into(),
            protocol_fee_vault: "0x0000000000000000000000000000000000000000".into(),
            profiles: [(0, 200); 5],
            rfq_discounts: [(0, 0); 5],
        }
    }
    #[test]
    fn all_tiers_and_identity_fail_closed() {
        let x = valid();
        assert!(x.validate(1020, 60).is_ok());
        for tier in 0..5 {
            let mut y = x.clone();
            y.profiles[tier] = (0, 250);
            assert!(y.validate(1020, 60).is_err());
        }
        let mut y = x.clone();
        y.chain_id = 1;
        assert!(y.validate(1020, 60).is_err());
        let mut y = x.clone();
        y.contract = "0x0000000000000000000000000000000000000000".into();
        assert!(y.validate(1020, 60).is_err());
        let mut y = x.clone();
        y.recipient = y.contract.clone();
        assert!(y.validate(1020, 60).is_err());
        let mut y = x.clone();
        y.rfq_discounts[1] = (1, 0);
        assert!(y.validate(1020, 60).is_err());
        assert!(x.validate(1100, 60).is_err());
    }
    #[test]
    fn d2_ppm_rounds_positive_fees_up() {
        assert_eq!(approved_taker_fee(0), Some(0));
        assert_eq!(approved_taker_fee(1), Some(1));
        assert_eq!(approved_taker_fee(5_000), Some(1));
        assert_eq!(approved_taker_fee(5_001), Some(2));
        assert_eq!(approved_taker_fee(u128::MAX), None);
        assert_eq!(signed_i32(&[0u8; 32]), Ok(0));
    }
    #[test]
    fn outer_receipt_without_exact_inner_event_cannot_activate() {
        let topic = format!(
            "0x{}",
            hex::encode(keccak256(
                "TransactionExecuted(bytes32,address,uint256,bytes,uint256,bytes)".as_bytes()
            ))
        );
        let good = json!({"address": TIMELOCK_ADDRESS, "topics": [topic, D2_OPERATION_IDS[0],
            format!("0x{}{}", "0".repeat(24), &D2_ADDRESS[2..]) ]});
        assert!(execution_log_matches(&good, &topic, D2_OPERATION_IDS[0]));
        assert!(!execution_log_matches(&good, &topic, D2_OPERATION_IDS[1]));
        let wrong_target = json!({"address": TIMELOCK_ADDRESS, "topics": [topic, D2_OPERATION_IDS[0],
            format!("0x{}{}", "0".repeat(24), "11".repeat(20))]});
        assert!(!execution_log_matches(
            &wrong_target,
            &topic,
            D2_OPERATION_IDS[0]
        ));
        assert!(!execution_log_matches(
            &json!({"status":"0x1","logs":[]}),
            &topic,
            D2_OPERATION_IDS[0]
        ));
    }
}
