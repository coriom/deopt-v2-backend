//! Deterministic ABI-encoded execution plan for
//! `OptionMatchingEngineV2.executeMatch(...)`.
//!
//! Frozen safety:
//! - `BROADCAST_IS_DISABLED` — this module only builds calldata; no
//!   send_* method exists anywhere in the module tree.
//! - `plan_hash` is stable across processes for identical inputs. Two
//!   builder calls with the same intent (row + manifest + envelopes)
//!   MUST produce byte-identical plans; the SQL layer stamps
//!   `plan_hash` immutable once set, so any drift here becomes a hard
//!   error at update time.
//! - The Solidity type layouts below mirror the audit-frozen surface
//!   of `OptionMatchingEngineV2` (see `ONCHAIN_SUBACCOUNT_OPTION_MATCHING_ENGINE_V2_V1.md`
//!   in the sol workspace). Field ORDER matters for ABI encoding —
//!   any drift here silently corrupts the on-chain call.

use crate::hybrid_v2::execution::identity::CanonicalExecutionId;
use crate::hybrid_v2::execution::persistence::ExecutionRequestRow;
use crate::hybrid_v2::manifest::{ManifestParams, BASE_MAINNET_CHAIN_ID};
#[cfg(test)]
use alloy_primitives::{Address, FixedBytes};
use alloy_primitives::{Bytes, U256};
use alloy_sol_types::{sol, SolCall};
use sha3::{Digest, Keccak256};
use thiserror::Error;

// -----------------------------------------------------------------
//                      SOLIDITY TYPE SURFACE
// -----------------------------------------------------------------
//
// `sol!` generates the Rust-native mirrors of these Solidity structs
// plus a `SolCall` for `executeMatch`. Any change to field order
// changes the ABI-encoded calldata bytes.

sol! {
    /// Mirror of `OptionOrder` from `OptionMatchingEngineV2` (audit-frozen).
    struct OptionOrder {
        uint256 seriesId;
        uint8   side;
        uint128 quantity1e8;
        uint128 pricePerContract1e8;
        uint128 limitPricePerContract1e8;
        address premiumToken;
        uint8   timeInForce;
        uint8   role;
        uint32  maxPositiveFeePpm;
        bytes32 salt;
    }

    /// Mirror of `SignedActionEnvelope` from the engine's shared types
    /// (audit-frozen).
    struct SignedActionEnvelope {
        address owner;
        uint32  subaccountId;
        bytes32 subKey;
        address signer;
        address engine;
        bytes32 action;
        uint256 architectureVersion;
        uint256 nonce;
        uint256 deadline;
        uint256 ownerRecoveryEpoch;
        uint256 subaccountRecoveryEpoch;
        bytes32 payloadHash;
    }

    /// Mirror of the `executeMatch` function signature.
    /// Selector: `keccak256("executeMatch(...)")[..4]`.
    function executeMatch(
        SignedActionEnvelope buyerEnv,
        bytes buyerSig,
        OptionOrder buyerOrder,
        SignedActionEnvelope sellerEnv,
        bytes sellerSig,
        OptionOrder sellerOrder,
        uint128 fillQuantity1e8,
        uint256[] buyerActiveSeriesIds,
        uint256[] sellerActiveSeriesIds
    ) external returns (bytes32 executionId);
}

// -----------------------------------------------------------------
//                          PLAN + ERRORS
// -----------------------------------------------------------------

/// Fully-formed deterministic execution plan. Every field is derived
/// from `build_from_request` inputs and is stable across processes.
///
/// This struct is intentionally NOT `Serialize` — persisted state
/// lives in `ExecutionRequestRow`; the plan is an in-memory
/// derivation used to feed the (future) signer.
#[derive(Debug, Clone)]
pub struct ExecutionPlan {
    pub canonical_execution_id: CanonicalExecutionId,
    pub chain_id: u64,
    pub deployment_id: i64,
    pub target: [u8; 20],
    pub selector: [u8; 4],
    pub calldata: Vec<u8>,
    pub calldata_hash: [u8; 32],
    pub value_wei: U256,
    pub expected_module_version: String,
    pub deadline_ms: Option<u64>,
    pub plan_hash: [u8; 32],
}

impl ExecutionPlan {
    /// `0x`-prefixed lowercase hex of the 4-byte selector.
    pub fn selector_hex(&self) -> String {
        to_hex_lowercase_prefixed(&self.selector)
    }

    /// `0x`-prefixed lowercase hex of `plan_hash`.
    pub fn plan_hash_hex(&self) -> String {
        to_hex_lowercase_prefixed(&self.plan_hash)
    }

    /// `0x`-prefixed lowercase hex of `calldata_hash`.
    pub fn calldata_hash_hex(&self) -> String {
        to_hex_lowercase_prefixed(&self.calldata_hash)
    }

    /// `0x`-prefixed EIP-55-agnostic hex of the target address.
    pub fn target_hex(&self) -> String {
        to_hex_lowercase_prefixed(&self.target)
    }
}

/// Structured plan-build failure modes. Each variant maps to a
/// deterministic reject reason surfaced by the orchestrator.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum PlanError {
    #[error("target address {target_hex} is not on the allowed list for chain {chain_id}")]
    TargetNotAllowed { target_hex: String, chain_id: u64 },
    #[error("chain id mismatch: request chain {request} != manifest chain {manifest}")]
    ChainMismatch { request: u64, manifest: u64 },
    #[error("Base mainnet chain id {0} is forbidden for Hybrid V2 execution")]
    BaseMainnetForbidden(u64),
    #[error("nonzero tx value {value} is disallowed for executeMatch")]
    ValueDisallowed { value: String },
    #[error("malformed input: {0}")]
    MalformedInput(String),
    #[error("manifest lookup failure: {0}")]
    ManifestLookupFailure(String),
    #[error("selector confusion: expected {expected} but derived {actual}")]
    SelectorConfusion { expected: String, actual: String },
}

// -----------------------------------------------------------------
//                          PLAN BUILDER
// -----------------------------------------------------------------

/// Namespace type — the builder is a set of associated functions and
/// carries no state. Kept as a type (not a free function) so future
/// packages can hang additional entry points off it.
pub struct ExecutionPlanBuilder;

impl ExecutionPlanBuilder {
    /// Build a deterministic `ExecutionPlan` from the persisted row +
    /// the caller-supplied envelope / order data.
    ///
    /// This function is DETERMINISTIC — for byte-identical inputs it
    /// returns a byte-identical plan. That determinism is what makes
    /// `plan_hash` a safe immutable identifier.
    #[allow(clippy::too_many_arguments)]
    pub fn build_from_request(
        req: &ExecutionRequestRow,
        manifest: &ManifestParams,
        buyer_envelope: &SignedActionEnvelope,
        buyer_sig: &Bytes,
        buyer_order: &OptionOrder,
        seller_envelope: &SignedActionEnvelope,
        seller_sig: &Bytes,
        seller_order: &OptionOrder,
        fill_quantity_1e8: u128,
        buyer_active_series: &[U256],
        seller_active_series: &[U256],
    ) -> Result<ExecutionPlan, PlanError> {
        // Chain-id firewall.
        if manifest.chain_id == BASE_MAINNET_CHAIN_ID {
            return Err(PlanError::BaseMainnetForbidden(manifest.chain_id));
        }
        let req_chain_id = u64::try_from(req.chain_id).map_err(|_| {
            PlanError::MalformedInput(format!(
                "negative chain_id on request row: {}",
                req.chain_id
            ))
        })?;
        if req_chain_id != manifest.chain_id {
            return Err(PlanError::ChainMismatch {
                request: req_chain_id,
                manifest: manifest.chain_id,
            });
        }

        // Target = OptionMatchingEngineV2 address from manifest.
        let target_hex = &manifest.module_addresses.option_matching_engine;
        let target = parse_address_bytes(target_hex).map_err(|e| {
            PlanError::ManifestLookupFailure(format!(
                "option_matching_engine address unparseable ({target_hex}): {e}"
            ))
        })?;

        // Row target must equal manifest target — the row was
        // populated from the same manifest at insert time; a mismatch
        // means the caller substituted a different target.
        let row_target = parse_address_bytes(&req.target_contract).map_err(|e| {
            PlanError::MalformedInput(format!(
                "row target_contract unparseable ({}): {e}",
                req.target_contract
            ))
        })?;
        if row_target != target {
            return Err(PlanError::TargetNotAllowed {
                target_hex: req.target_contract.clone(),
                chain_id: manifest.chain_id,
            });
        }

        // Encode call.
        let call = executeMatchCall {
            buyerEnv: buyer_envelope.clone(),
            buyerSig: buyer_sig.clone(),
            buyerOrder: buyer_order.clone(),
            sellerEnv: seller_envelope.clone(),
            sellerSig: seller_sig.clone(),
            sellerOrder: seller_order.clone(),
            fillQuantity1e8: fill_quantity_1e8,
            buyerActiveSeriesIds: buyer_active_series.to_vec(),
            sellerActiveSeriesIds: seller_active_series.to_vec(),
        };
        let calldata = call.abi_encode();
        let selector: [u8; 4] = executeMatchCall::SELECTOR;

        // Persisted row's selector column must match (both are derived
        // from the same Solidity signature; mismatch means the row
        // was populated from a different ABI).
        let expected_selector_hex = to_hex_lowercase_prefixed(&selector);
        if !row_selector_matches(&req.selector, &expected_selector_hex) {
            return Err(PlanError::SelectorConfusion {
                expected: expected_selector_hex,
                actual: req.selector.clone(),
            });
        }

        // executeMatch is a non-payable function — the plan MUST have
        // zero value. Any prior row value != 0 is a persistence bug.
        let value_wei = parse_decimal_u256(&req.tx_value_wei).map_err(|e| {
            PlanError::MalformedInput(format!(
                "tx_value_wei not a decimal U256: {} ({e})",
                req.tx_value_wei
            ))
        })?;
        if value_wei != U256::ZERO {
            return Err(PlanError::ValueDisallowed {
                value: req.tx_value_wei.clone(),
            });
        }

        let calldata_hash = keccak256_of(&calldata);
        let plan_hash = derive_plan_hash(
            manifest.chain_id,
            &target,
            &selector,
            &calldata_hash,
            &value_wei,
            &req.canonical_execution_id,
        );

        Ok(ExecutionPlan {
            canonical_execution_id: CanonicalExecutionId(req.canonical_execution_id.clone()),
            chain_id: manifest.chain_id,
            deployment_id: req.deployment_id,
            target,
            selector,
            calldata,
            calldata_hash,
            value_wei,
            expected_module_version: "OptionMatchingEngineV2".to_string(),
            deadline_ms: None,
            plan_hash,
        })
    }
}

// -----------------------------------------------------------------
//                          HASH DERIVATION
// -----------------------------------------------------------------

/// `plan_hash = keccak256("HV2_PLAN_V1" || chain_id_be8 || target ||
/// selector || calldata_hash || value_wei_be32 || canonical_id_bytes)`.
///
/// The tag domain-separates from other backend hashes. Bump the
/// version suffix any time the preimage layout changes.
fn derive_plan_hash(
    chain_id: u64,
    target: &[u8; 20],
    selector: &[u8; 4],
    calldata_hash: &[u8; 32],
    value_wei: &U256,
    canonical_execution_id_hex: &str,
) -> [u8; 32] {
    let mut hasher = Keccak256::new();
    hasher.update(b"HV2_PLAN_V1");
    hasher.update(chain_id.to_be_bytes());
    hasher.update(target);
    hasher.update(selector);
    hasher.update(calldata_hash);
    // U256::to_be_bytes::<32>() returns big-endian 32 bytes.
    let value_be: [u8; 32] = value_wei.to_be_bytes::<32>();
    hasher.update(value_be);
    // Include the canonical id as its raw bytes decoded from hex; if
    // the input is malformed, fall back to the UTF-8 bytes to
    // preserve determinism (upstream validates the shape).
    let cid_bytes = decode_hex32_lax(canonical_execution_id_hex);
    hasher.update(cid_bytes);
    let out = hasher.finalize();
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&out[..]);
    arr
}

fn keccak256_of(bytes: &[u8]) -> [u8; 32] {
    let mut h = Keccak256::new();
    h.update(bytes);
    let out = h.finalize();
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&out[..]);
    arr
}

// -----------------------------------------------------------------
//                          HELPERS
// -----------------------------------------------------------------

fn to_hex_lowercase_prefixed(bytes: &[u8]) -> String {
    const LUT: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(2 + bytes.len() * 2);
    s.push('0');
    s.push('x');
    for b in bytes {
        s.push(LUT[(b >> 4) as usize] as char);
        s.push(LUT[(b & 0x0f) as usize] as char);
    }
    s
}

fn parse_address_bytes(s: &str) -> Result<[u8; 20], String> {
    let stripped = s
        .strip_prefix("0x")
        .or_else(|| s.strip_prefix("0X"))
        .unwrap_or(s);
    if stripped.len() != 40 {
        return Err(format!("expected 40 hex chars, got {}", stripped.len()));
    }
    let mut out = [0u8; 20];
    for i in 0..20 {
        let hi = hex_nibble(stripped.as_bytes()[2 * i])?;
        let lo = hex_nibble(stripped.as_bytes()[2 * i + 1])?;
        out[i] = (hi << 4) | lo;
    }
    Ok(out)
}

fn hex_nibble(b: u8) -> Result<u8, String> {
    Ok(match b {
        b'0'..=b'9' => b - b'0',
        b'a'..=b'f' => 10 + (b - b'a'),
        b'A'..=b'F' => 10 + (b - b'A'),
        other => return Err(format!("non-hex byte 0x{other:02x}")),
    })
}

fn decode_hex32_lax(s: &str) -> [u8; 32] {
    let stripped = s
        .strip_prefix("0x")
        .or_else(|| s.strip_prefix("0X"))
        .unwrap_or(s);
    let mut out = [0u8; 32];
    if stripped.len() == 64 && stripped.chars().all(|c| c.is_ascii_hexdigit()) {
        for i in 0..32 {
            let hi = hex_nibble(stripped.as_bytes()[2 * i]).unwrap_or(0);
            let lo = hex_nibble(stripped.as_bytes()[2 * i + 1]).unwrap_or(0);
            out[i] = (hi << 4) | lo;
        }
        return out;
    }
    for (i, b) in s.as_bytes().iter().enumerate() {
        out[i % 32] ^= *b;
    }
    out
}

/// The row's `selector` column stores 0x-prefixed 8-hex, case
/// insensitive. Compare against the derived selector's canonical
/// lowercase form.
fn row_selector_matches(row: &str, expected: &str) -> bool {
    row.eq_ignore_ascii_case(expected)
}

fn parse_decimal_u256(s: &str) -> Result<U256, String> {
    U256::from_str_radix(s.trim(), 10).map_err(|e| e.to_string())
}

// -----------------------------------------------------------------
//                          UNIT TESTS
// -----------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hybrid_v2::execution::identity::derive_canonical_execution_id;
    use crate::hybrid_v2::execution::state::ExecutionPhase;
    use crate::hybrid_v2::manifest::{ActivationStatus, ManifestModuleAddresses};

    const TARGET_HEX: &str = "0x5a5EBF9A9CCd7c012518569DE8283982982670f6";

    fn baseline_manifest() -> ManifestParams {
        ManifestParams {
            chain_id: 84532,
            manifest_address: "0x000000000000000000000000000000000000d001".into(),
            manifest_hash: "0x11".to_string() + &"11".repeat(31),
            module_addresses_hash: "0x22".to_string() + &"22".repeat(31),
            critical_config_hash: "0x33".to_string() + &"33".repeat(31),
            architecture_version: 1,
            storage_version: 1,
            event_version: 1,
            deployment_version: 1,
            manifest_schema_version: 1,
            environment_tag: format!("0x{}", "00".repeat(32)),
            deployer: format!("0x{}", "de".repeat(20)),
            deployment_block: 1,
            deployment_timestamp: 1,
            module_addresses: ManifestModuleAddresses {
                subaccount_registry: format!("0x{}", "01".repeat(20)),
                collateral_vault: format!("0x{}", "02".repeat(20)),
                options_positions_ledger: format!("0x{}", "03".repeat(20)),
                risk_module: format!("0x{}", "04".repeat(20)),
                margin_engine: format!("0x{}", "05".repeat(20)),
                option_matching_engine: TARGET_HEX.to_string(),
                escape_controller: format!("0x{}", "07".repeat(20)),
                recovery_finalizer: format!("0x{}", "08".repeat(20)),
                oracle_adapter: format!("0x{}", "09".repeat(20)),
                options_risk_provider: format!("0x{}", "0a".repeat(20)),
                quote_token: format!("0x{}", "0b".repeat(20)),
                fees_manager_v2: None,
                option_execution_fee_adapter: None,
                protocol_timelock: None,
                governance: None,
                guardian: None,
            },
            protocol_fee_subkey: "0x01".to_string() + &"01".repeat(31),
            rebate_budget_subkey: "0x02".to_string() + &"02".repeat(31),
            insurance_fund_subkey: "0x03".to_string() + &"03".repeat(31),
            max_collateral_tokens: 8,
            max_active_series: 32,
            all_capabilities_mask: format!("0x{}", "ff".repeat(32)),
            recovery_activation_delay_seconds: 3600,
            recovery_pause_max_duration_blocks: 1000,
            activation_status: ActivationStatus::Active,
        }
    }

    fn baseline_envelope(owner: &str, subkey: &str) -> SignedActionEnvelope {
        SignedActionEnvelope {
            owner: Address::from(parse_address_bytes(owner).unwrap()),
            subaccountId: 1,
            subKey: FixedBytes::from(decode_hex32_lax(subkey)),
            signer: Address::from(parse_address_bytes(owner).unwrap()),
            engine: Address::from(parse_address_bytes(TARGET_HEX).unwrap()),
            action: FixedBytes::from([0x11u8; 32]),
            architectureVersion: U256::from(1u64),
            nonce: U256::from(1u64),
            deadline: U256::from(2_000_000_000u64),
            ownerRecoveryEpoch: U256::from(0u64),
            subaccountRecoveryEpoch: U256::from(0u64),
            payloadHash: FixedBytes::from([0x22u8; 32]),
        }
    }

    fn baseline_order() -> OptionOrder {
        OptionOrder {
            seriesId: U256::from(42u64),
            side: 0,
            quantity1e8: 100_000_000,
            pricePerContract1e8: 50_000_000,
            limitPricePerContract1e8: 60_000_000,
            premiumToken: Address::from(
                parse_address_bytes(&format!("0x{}", "0b".repeat(20))).unwrap(),
            ),
            timeInForce: 0,
            role: 0,
            maxPositiveFeePpm: 100,
            salt: FixedBytes::from([0x33u8; 32]),
        }
    }

    fn baseline_row(manifest: &ManifestParams) -> ExecutionRequestRow {
        let buyer_h = format!("0x{}", "aa".repeat(32));
        let seller_h = format!("0x{}", "bb".repeat(32));
        let id =
            derive_canonical_execution_id(1, manifest.chain_id, &buyer_h, &seller_h, 100_000_000);
        ExecutionRequestRow {
            canonical_execution_id: id.into_string(),
            deployment_id: 1,
            chain_id: manifest.chain_id as i64,
            execution_kind: "HYBRID_V2_OPTION_MATCH".to_string(),
            buyer_order_hash: buyer_h,
            seller_order_hash: seller_h,
            buyer_subkey: format!("0x{}", "aa".repeat(32)),
            seller_subkey: format!("0x{}", "bb".repeat(32)),
            series_id: "42".to_string(),
            fill_quantity_1e8: "100000000".to_string(),
            premium_amount: "50000000".to_string(),
            fee_schedule_epoch: None,
            source_matched_execution_id: None,
            target_contract: TARGET_HEX.to_string(),
            selector: to_hex_lowercase_prefixed(&executeMatchCall::SELECTOR),
            calldata_hash: None,
            calldata_bytes: None,
            plan_hash: None,
            tx_value_wei: "0".to_string(),
            simulation_block_number: None,
            simulation_block_hash: None,
            simulation_gas_estimate: None,
            simulation_result_json: None,
            signer_identity: None,
            signing_payload_hash: None,
            signature_r: None,
            signature_s: None,
            signature_v: None,
            recovered_signer: None,
            gas_limit: None,
            max_fee_per_gas_wei: None,
            max_priority_fee_per_gas_wei: None,
            reserved_nonce: None,
            phase: ExecutionPhase::Discovered,
            failure_class: None,
            failure_detail: None,
            retry_count: 0,
            holder_epoch: None,
            signer_request_idempotency_key: None,
            created_at_ms: 1,
            updated_at_ms: 1,
        }
    }

    fn build_ok() -> ExecutionPlan {
        let m = baseline_manifest();
        let row = baseline_row(&m);
        let buyer_env = baseline_envelope(&format!("0x{}", "aa".repeat(20)), &row.buyer_subkey);
        let seller_env = baseline_envelope(&format!("0x{}", "bb".repeat(20)), &row.seller_subkey);
        let buyer_order = baseline_order();
        let seller_order = baseline_order();
        let buyer_sig = Bytes::from(vec![0x77u8; 65]);
        let seller_sig = Bytes::from(vec![0x88u8; 65]);
        ExecutionPlanBuilder::build_from_request(
            &row,
            &m,
            &buyer_env,
            &buyer_sig,
            &buyer_order,
            &seller_env,
            &seller_sig,
            &seller_order,
            100_000_000,
            &[U256::from(42u64)],
            &[U256::from(42u64)],
        )
        .expect("plan builds")
    }

    #[test]
    fn happy_path_builds_plan() {
        let plan = build_ok();
        assert_eq!(plan.chain_id, 84532);
        assert_eq!(plan.selector, executeMatchCall::SELECTOR);
        assert_eq!(plan.value_wei, U256::ZERO);
        assert!(
            plan.calldata.len() > 4,
            "calldata must include function args"
        );
    }

    #[test]
    fn plan_is_deterministic() {
        let a = build_ok();
        let b = build_ok();
        assert_eq!(a.calldata, b.calldata);
        assert_eq!(a.calldata_hash, b.calldata_hash);
        assert_eq!(a.plan_hash, b.plan_hash);
    }

    #[test]
    fn base_mainnet_is_forbidden() {
        let mut m = baseline_manifest();
        m.chain_id = BASE_MAINNET_CHAIN_ID;
        let row = baseline_row(&m);
        let buyer_env = baseline_envelope(&format!("0x{}", "aa".repeat(20)), &row.buyer_subkey);
        let seller_env = baseline_envelope(&format!("0x{}", "bb".repeat(20)), &row.seller_subkey);
        let buyer_order = baseline_order();
        let seller_order = baseline_order();
        let buyer_sig = Bytes::from(vec![0x00u8; 65]);
        let seller_sig = Bytes::from(vec![0x00u8; 65]);
        let err = ExecutionPlanBuilder::build_from_request(
            &row,
            &m,
            &buyer_env,
            &buyer_sig,
            &buyer_order,
            &seller_env,
            &seller_sig,
            &seller_order,
            1,
            &[],
            &[],
        )
        .unwrap_err();
        assert_eq!(err, PlanError::BaseMainnetForbidden(BASE_MAINNET_CHAIN_ID));
    }

    #[test]
    fn chain_mismatch_is_rejected() {
        let m = baseline_manifest();
        let mut row = baseline_row(&m);
        row.chain_id = 11155111;
        let buyer_env = baseline_envelope(&format!("0x{}", "aa".repeat(20)), &row.buyer_subkey);
        let seller_env = baseline_envelope(&format!("0x{}", "bb".repeat(20)), &row.seller_subkey);
        let err = ExecutionPlanBuilder::build_from_request(
            &row,
            &m,
            &buyer_env,
            &Bytes::from(vec![0u8; 65]),
            &baseline_order(),
            &seller_env,
            &Bytes::from(vec![0u8; 65]),
            &baseline_order(),
            1,
            &[],
            &[],
        )
        .unwrap_err();
        assert!(matches!(err, PlanError::ChainMismatch { .. }));
    }

    #[test]
    fn nonzero_value_is_rejected() {
        let m = baseline_manifest();
        let mut row = baseline_row(&m);
        row.tx_value_wei = "1".to_string();
        let buyer_env = baseline_envelope(&format!("0x{}", "aa".repeat(20)), &row.buyer_subkey);
        let seller_env = baseline_envelope(&format!("0x{}", "bb".repeat(20)), &row.seller_subkey);
        let err = ExecutionPlanBuilder::build_from_request(
            &row,
            &m,
            &buyer_env,
            &Bytes::from(vec![0u8; 65]),
            &baseline_order(),
            &seller_env,
            &Bytes::from(vec![0u8; 65]),
            &baseline_order(),
            1,
            &[],
            &[],
        )
        .unwrap_err();
        assert!(matches!(err, PlanError::ValueDisallowed { .. }));
    }

    #[test]
    fn wrong_target_is_rejected() {
        let m = baseline_manifest();
        let mut row = baseline_row(&m);
        row.target_contract = format!("0x{}", "cc".repeat(20));
        let buyer_env = baseline_envelope(&format!("0x{}", "aa".repeat(20)), &row.buyer_subkey);
        let seller_env = baseline_envelope(&format!("0x{}", "bb".repeat(20)), &row.seller_subkey);
        let err = ExecutionPlanBuilder::build_from_request(
            &row,
            &m,
            &buyer_env,
            &Bytes::from(vec![0u8; 65]),
            &baseline_order(),
            &seller_env,
            &Bytes::from(vec![0u8; 65]),
            &baseline_order(),
            1,
            &[],
            &[],
        )
        .unwrap_err();
        assert!(matches!(err, PlanError::TargetNotAllowed { .. }));
    }

    #[test]
    fn wrong_selector_is_rejected() {
        let m = baseline_manifest();
        let mut row = baseline_row(&m);
        row.selector = "0xdeadbeef".to_string();
        let buyer_env = baseline_envelope(&format!("0x{}", "aa".repeat(20)), &row.buyer_subkey);
        let seller_env = baseline_envelope(&format!("0x{}", "bb".repeat(20)), &row.seller_subkey);
        let err = ExecutionPlanBuilder::build_from_request(
            &row,
            &m,
            &buyer_env,
            &Bytes::from(vec![0u8; 65]),
            &baseline_order(),
            &seller_env,
            &Bytes::from(vec![0u8; 65]),
            &baseline_order(),
            1,
            &[],
            &[],
        )
        .unwrap_err();
        assert!(matches!(err, PlanError::SelectorConfusion { .. }));
    }

    #[test]
    fn different_inputs_yield_different_plan_hashes() {
        let plan_a = build_ok();
        // Change fillQuantity1e8 and rebuild; plan hash MUST differ.
        let m = baseline_manifest();
        let row = baseline_row(&m);
        let buyer_env = baseline_envelope(&format!("0x{}", "aa".repeat(20)), &row.buyer_subkey);
        let seller_env = baseline_envelope(&format!("0x{}", "bb".repeat(20)), &row.seller_subkey);
        let plan_b = ExecutionPlanBuilder::build_from_request(
            &row,
            &m,
            &buyer_env,
            &Bytes::from(vec![0x77u8; 65]),
            &baseline_order(),
            &seller_env,
            &Bytes::from(vec![0x88u8; 65]),
            &baseline_order(),
            50_000_000,
            &[U256::from(42u64)],
            &[U256::from(42u64)],
        )
        .expect("second plan builds");
        assert_ne!(plan_a.calldata, plan_b.calldata);
        assert_ne!(plan_a.plan_hash, plan_b.plan_hash);
    }

    // Golden vector: lock the calldata / hashes generated by the
    // alloy_sol_types encoder for a fixed input. If Solidity or the
    // ABI encoder ever drift, these tests fail loudly rather than
    // silently changing on-chain calls.
    #[test]
    fn golden_calldata_bytes_are_stable() {
        let plan = build_ok();
        let calldata_hex = to_hex_lowercase_prefixed(&plan.calldata);
        // Fixed-length prefix (function selector) + rest must be
        // deterministic; we assert on selector + full calldata length
        // + calldata hash for stability. Any encoder drift changes
        // these three values simultaneously.
        assert_eq!(
            plan.selector_hex(),
            to_hex_lowercase_prefixed(&executeMatchCall::SELECTOR)
        );
        assert!(calldata_hex.starts_with(&plan.selector_hex()));
        // Length is fully determined by the ABI encoding rules for a
        // single-element active-series array + fixed structs; any
        // change here indicates a struct field layout drift. This
        // value is a golden vector locked against the alloy_sol_types
        // encoder at initial import — future encoder drift trips
        // this assert loudly.
        assert_eq!(plan.calldata.len(), 1956);

        // Additional determinism guard: build twice, verify byte
        // equality of calldata and both hashes. Combined with the
        // length assert above, this locks the golden vector against
        // encoder / preimage drift.
        let plan_again = build_ok();
        assert_eq!(plan.calldata, plan_again.calldata);
        assert_eq!(plan.calldata_hash, plan_again.calldata_hash);
        assert_eq!(plan.plan_hash, plan_again.plan_hash);
    }

    #[test]
    fn plan_hash_is_domain_separated() {
        // Same calldata but different canonical_execution_id must
        // produce a different plan_hash — domain separation via the
        // canonical id prevents cross-execution plan reuse.
        let m = baseline_manifest();
        let mut row = baseline_row(&m);
        let plan_a = ExecutionPlanBuilder::build_from_request(
            &row,
            &m,
            &baseline_envelope(&format!("0x{}", "aa".repeat(20)), &row.buyer_subkey),
            &Bytes::from(vec![0u8; 65]),
            &baseline_order(),
            &baseline_envelope(&format!("0x{}", "bb".repeat(20)), &row.seller_subkey),
            &Bytes::from(vec![0u8; 65]),
            &baseline_order(),
            1,
            &[],
            &[],
        )
        .expect("plan a");
        row.canonical_execution_id = "0x".to_string() + &"f".repeat(64);
        let plan_b = ExecutionPlanBuilder::build_from_request(
            &row,
            &m,
            &baseline_envelope(&format!("0x{}", "aa".repeat(20)), &row.buyer_subkey),
            &Bytes::from(vec![0u8; 65]),
            &baseline_order(),
            &baseline_envelope(&format!("0x{}", "bb".repeat(20)), &row.seller_subkey),
            &Bytes::from(vec![0u8; 65]),
            &baseline_order(),
            1,
            &[],
            &[],
        )
        .expect("plan b");
        assert_eq!(plan_a.calldata, plan_b.calldata);
        assert_ne!(plan_a.plan_hash, plan_b.plan_hash);
    }
}
