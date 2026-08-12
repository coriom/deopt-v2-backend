//! Fresh-admin-broadcast plan + signed-tx reconstruction helpers for
//! `BACKEND-HYBRID-V2-BROADCAST-LIVE-WIRING-CLOSURE-V1` (Part D).
//!
//! ## Purpose
//!
//! Between the pre-broadcast orchestrator (which persists an
//! `ExecutionRequestRow` at phase `ReadyForBroadcast` — carrying the
//! full plan/signed surface) and the broadcast outbox (which needs an
//! in-memory `ExecutionPlan` + `SignedTx` to serialize the exact
//! signed envelope) there was a gap: the admin `broadcast` handler
//! could only invoke `resume(...)` because rebuilding a plan from
//! the row required the raw calldata bytes, and only `calldata_hash`
//! was persisted.
//!
//! Migration 0052 added `calldata_bytes`. This module hydrates the
//! two structs the outbox needs from the persisted row and refuses
//! (with a structured error) whenever the row is incomplete, the
//! stored hash does not match the recomputed keccak of the persisted
//! bytes, or the persisted plan_hash would diverge from a fresh
//! recomputation over the reconstructed inputs.
//!
//! ## Frozen safety
//!
//! * `calldata_bytes` MUST be present. Legacy rows written before
//!   migration 0052 keep NULL — they cannot be first-submitted; the
//!   admin route refuses with `CALLDATA_BYTES_MISSING` and the caller
//!   must re-run the orchestrator (`prepare`) so a fresh row is
//!   written with bytes attached.
//! * keccak256(calldata_bytes) MUST equal `calldata_hash` — this
//!   defends against DB tampering and byte-shape drift.
//! * Reconstructed `plan_hash` MUST equal the persisted `plan_hash`.
//!   Any mismatch escalates to `PlanHashMismatch` — no RPC contact
//!   happens.
//! * `value_wei` MUST be zero — the executeMatch surface never sends
//!   ETH. A nonzero row is rejected with `ValueNonZero`.
//! * Chain id MUST match the deployment/manifest chain id.
//! * Target contract on the row MUST match the manifest's
//!   `option_matching_engine`. Any drift is treated as tampering.
//!
//! No RPC call, no signer contact, no keystore access happens here.
//! Reconstruction is a pure function of `(row, manifest)`.

use crate::hybrid_v2::execution::persistence::ExecutionRequestRow;
use crate::hybrid_v2::execution::plan::ExecutionPlan;
use crate::hybrid_v2::execution::signer::SignedTx;
use crate::hybrid_v2::manifest::ManifestParams;
use alloy_primitives::U256;
use sha3::{Digest, Keccak256};
use thiserror::Error;

/// Structured reconstruction failure surface. Every variant maps to
/// a bounded caller-visible error string; no variant leaks signature
/// bytes or raw calldata content.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ReconstructionError {
    #[error("calldata_bytes missing from persisted row")]
    CalldataBytesMissing,
    #[error("calldata_hash mismatch: persisted=0x{persisted_hex} recomputed=0x{recomputed_hex}")]
    CalldataHashMismatch {
        persisted_hex: String,
        recomputed_hex: String,
    },
    #[error("calldata_hash missing from persisted row")]
    CalldataHashMissing,
    #[error("plan_hash missing from persisted row")]
    PlanHashMissing,
    #[error("plan_hash mismatch: persisted=0x{persisted_hex} recomputed=0x{recomputed_hex}")]
    PlanHashMismatch {
        persisted_hex: String,
        recomputed_hex: String,
    },
    #[error("signature material missing (r/s/v or recovered_signer)")]
    SignatureMissing,
    #[error("recovered_signer missing from persisted row")]
    RecoveredSignerMissing,
    #[error("reserved_nonce missing from persisted row")]
    NonceMissing,
    #[error("gas_limit / max_fee / max_priority missing from persisted row")]
    GasFieldMissing,
    #[error("target_contract missing / malformed")]
    TargetContractMissing,
    #[error("selector missing / malformed")]
    SelectorMissing,
    #[error("chain_id mismatch: row={row} manifest={manifest}")]
    ChainIdMismatch { row: i64, manifest: u64 },
    #[error("manifest lookup failure: {0}")]
    ManifestLookupFailure(String),
    #[error("tx_value_wei must be zero for executeMatch (got {0})")]
    ValueNonZero(String),
    #[error("malformed hex on persisted row: {0}")]
    MalformedHex(String),
}

/// Reconstruct an in-memory [`ExecutionPlan`] from a persisted
/// [`ExecutionRequestRow`] + the deployment manifest. Runs a full
/// integrity sweep: keccak(calldata_bytes) == calldata_hash,
/// recomputed plan_hash == persisted plan_hash, target address ==
/// manifest.option_matching_engine, chain id matches, value = 0.
pub fn reconstruct_plan(
    req: &ExecutionRequestRow,
    manifest: &ManifestParams,
) -> Result<ExecutionPlan, ReconstructionError> {
    // Chain identity.
    let manifest_chain = manifest.chain_id;
    if req.chain_id < 0 || (req.chain_id as u64) != manifest_chain {
        return Err(ReconstructionError::ChainIdMismatch {
            row: req.chain_id,
            manifest: manifest_chain,
        });
    }
    let chain_id_u64 = req.chain_id as u64;

    // calldata bytes + hash pair.
    let bytes = req
        .calldata_bytes
        .as_ref()
        .ok_or(ReconstructionError::CalldataBytesMissing)?
        .clone();
    let recomputed_hash = keccak_of(&bytes);
    let stored_hash_hex = req
        .calldata_hash
        .clone()
        .ok_or(ReconstructionError::CalldataHashMissing)?;
    let stored_hash = parse_bytes32(&stored_hash_hex)
        .map_err(|e| ReconstructionError::MalformedHex(format!("calldata_hash: {e}")))?;
    if stored_hash != recomputed_hash {
        return Err(ReconstructionError::CalldataHashMismatch {
            persisted_hex: hex_of(&stored_hash),
            recomputed_hex: hex_of(&recomputed_hash),
        });
    }

    // target + selector.
    let target = parse_address(&req.target_contract)
        .map_err(|_| ReconstructionError::TargetContractMissing)?;
    let manifest_target = parse_address(&manifest.module_addresses.option_matching_engine)
        .map_err(|_| {
            ReconstructionError::ManifestLookupFailure(
                "manifest.option_matching_engine malformed".to_string(),
            )
        })?;
    if target != manifest_target {
        // The persisted target must equal the manifest's canonical
        // executeMatch address. Anything else implies tampering /
        // manifest drift.
        return Err(ReconstructionError::ManifestLookupFailure(format!(
            "row target {} != manifest option_matching_engine {}",
            req.target_contract, manifest.module_addresses.option_matching_engine,
        )));
    }
    let selector =
        parse_selector(&req.selector).map_err(|_| ReconstructionError::SelectorMissing)?;

    // value must be zero for executeMatch.
    if req.tx_value_wei != "0" {
        return Err(ReconstructionError::ValueNonZero(req.tx_value_wei.clone()));
    }

    // Recompute plan_hash and compare to the persisted value.
    let recomputed_plan_hash = derive_plan_hash(
        chain_id_u64,
        &target,
        &selector,
        &recomputed_hash,
        &U256::ZERO,
        &req.canonical_execution_id,
    );
    let stored_plan_hash_hex = req
        .plan_hash
        .clone()
        .ok_or(ReconstructionError::PlanHashMissing)?;
    let stored_plan_hash = parse_bytes32(&stored_plan_hash_hex)
        .map_err(|e| ReconstructionError::MalformedHex(format!("plan_hash: {e}")))?;
    if stored_plan_hash != recomputed_plan_hash {
        return Err(ReconstructionError::PlanHashMismatch {
            persisted_hex: hex_of(&stored_plan_hash),
            recomputed_hex: hex_of(&recomputed_plan_hash),
        });
    }

    Ok(ExecutionPlan {
        canonical_execution_id: crate::hybrid_v2::execution::identity::CanonicalExecutionId(
            req.canonical_execution_id.clone(),
        ),
        chain_id: chain_id_u64,
        deployment_id: req.deployment_id,
        target,
        selector,
        calldata: bytes,
        calldata_hash: recomputed_hash,
        value_wei: U256::ZERO,
        expected_module_version: "OptionMatchingEngineV2".to_string(),
        deadline_ms: None,
        plan_hash: recomputed_plan_hash,
    })
}

/// Reconstruct an in-memory [`SignedTx`] from a persisted row. Refuses
/// when any of `signature_r`, `signature_s`, `signature_v`, or
/// `recovered_signer` is missing / malformed.
pub fn reconstruct_signed(req: &ExecutionRequestRow) -> Result<SignedTx, ReconstructionError> {
    let (Some(r_hex), Some(s_hex), Some(v)) = (
        req.signature_r.clone(),
        req.signature_s.clone(),
        req.signature_v,
    ) else {
        return Err(ReconstructionError::SignatureMissing);
    };
    let r = parse_bytes32(&r_hex)
        .map_err(|e| ReconstructionError::MalformedHex(format!("signature_r: {e}")))?;
    let s = parse_bytes32(&s_hex)
        .map_err(|e| ReconstructionError::MalformedHex(format!("signature_s: {e}")))?;
    // EIP-1559 y-parity is 0 or 1 — anything else is rejected as
    // malformed. `SignedTx::signature_v` is `u8`; the row stores it as
    // `i16` so we downcast defensively.
    if !(0..=1).contains(&v) {
        return Err(ReconstructionError::SignatureMissing);
    }
    let recovered_hex = req
        .recovered_signer
        .as_deref()
        .ok_or(ReconstructionError::RecoveredSignerMissing)?;
    let recovered = parse_address(recovered_hex)
        .map_err(|e| ReconstructionError::MalformedHex(format!("recovered_signer: {e}")))?;
    Ok(SignedTx {
        signature_r: r,
        signature_s: s,
        signature_v: v as u8,
        recovered_signer: recovered,
        tx_type: 2,
    })
}

// -----------------------------------------------------------------
//                          HELPERS
// -----------------------------------------------------------------

fn keccak_of(bytes: &[u8]) -> [u8; 32] {
    let mut h = Keccak256::new();
    h.update(bytes);
    let out = h.finalize();
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&out[..]);
    arr
}

/// Local mirror of `plan::derive_plan_hash` — that function is
/// private. Kept byte-identical (same domain tag, same preimage
/// layout). If either drifts, unit tests below will catch it because
/// the round-trip check in `reconstruct_plan_matches_orchestrator_output`
/// (see integration tests) compares the recomputed hash against a
/// live orchestrator run.
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
    let value_be: [u8; 32] = value_wei.to_be_bytes::<32>();
    hasher.update(value_be);
    let cid_bytes = decode_hex32_lax(canonical_execution_id_hex);
    hasher.update(cid_bytes);
    let out = hasher.finalize();
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&out[..]);
    arr
}

/// Same lax decoder used by `plan::decode_hex32_lax` — returns raw
/// 32 bytes when the input is a valid 0x-prefixed 32-byte hex string,
/// otherwise the UTF-8 bytes of the input (upstream validates shape).
fn decode_hex32_lax(s: &str) -> Vec<u8> {
    let stripped = s
        .strip_prefix("0x")
        .or_else(|| s.strip_prefix("0X"))
        .unwrap_or(s);
    if stripped.len() == 64 && stripped.chars().all(|c| c.is_ascii_hexdigit()) {
        let mut out = Vec::with_capacity(32);
        for i in 0..32 {
            let byte = u8::from_str_radix(&stripped[2 * i..2 * i + 2], 16).unwrap();
            out.push(byte);
        }
        return out;
    }
    s.as_bytes().to_vec()
}

fn parse_bytes32(s: &str) -> Result<[u8; 32], String> {
    let stripped = s
        .strip_prefix("0x")
        .or_else(|| s.strip_prefix("0X"))
        .ok_or_else(|| format!("no 0x prefix: {s}"))?;
    if stripped.len() != 64 {
        return Err(format!(
            "expected 32 bytes, got {} hex chars",
            stripped.len()
        ));
    }
    let mut out = [0u8; 32];
    for i in 0..32 {
        out[i] = u8::from_str_radix(&stripped[2 * i..2 * i + 2], 16)
            .map_err(|e| format!("hex parse: {e}"))?;
    }
    Ok(out)
}

fn parse_selector(s: &str) -> Result<[u8; 4], String> {
    let stripped = s
        .strip_prefix("0x")
        .or_else(|| s.strip_prefix("0X"))
        .ok_or_else(|| format!("no 0x prefix: {s}"))?;
    if stripped.len() != 8 {
        return Err(format!(
            "expected 4 bytes, got {} hex chars",
            stripped.len()
        ));
    }
    let mut out = [0u8; 4];
    for i in 0..4 {
        out[i] = u8::from_str_radix(&stripped[2 * i..2 * i + 2], 16)
            .map_err(|e| format!("hex parse: {e}"))?;
    }
    Ok(out)
}

fn parse_address(s: &str) -> Result<[u8; 20], String> {
    let stripped = s
        .strip_prefix("0x")
        .or_else(|| s.strip_prefix("0X"))
        .ok_or_else(|| format!("no 0x prefix: {s}"))?;
    if stripped.len() != 40 {
        return Err(format!(
            "expected 20-byte address, got {} hex chars",
            stripped.len()
        ));
    }
    let mut out = [0u8; 20];
    for i in 0..20 {
        out[i] = u8::from_str_radix(&stripped[2 * i..2 * i + 2], 16)
            .map_err(|e| format!("hex parse: {e}"))?;
    }
    Ok(out)
}

fn hex_of(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{:02x}", b));
    }
    s
}

// -----------------------------------------------------------------
//                          UNIT TESTS
// -----------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hybrid_v2::execution::state::ExecutionPhase;

    fn hex_20(b: &[u8; 20]) -> String {
        let mut s = String::with_capacity(40);
        for x in b.iter() {
            s.push_str(&format!("{:02x}", x));
        }
        format!("0x{s}")
    }

    fn hex_32(b: &[u8; 32]) -> String {
        let mut s = String::with_capacity(64);
        for x in b.iter() {
            s.push_str(&format!("{:02x}", x));
        }
        format!("0x{s}")
    }

    fn test_manifest(chain_id: u64, target: [u8; 20]) -> ManifestParams {
        use crate::hybrid_v2::manifest::{ActivationStatus, ManifestModuleAddresses};
        ManifestParams {
            chain_id,
            manifest_hash: format!("0x{}", "11".repeat(32)),
            manifest_address: format!("0x{}", "d0".repeat(20)),
            module_addresses_hash: format!("0x{}", "22".repeat(32)),
            critical_config_hash: format!("0x{}", "33".repeat(32)),
            architecture_version: 1,
            storage_version: 1,
            event_version: 1,
            deployment_version: 1,
            manifest_schema_version: 1,
            environment_tag: format!("0x{}", "6c".repeat(31) + "00"),
            deployer: format!("0x{}", "de".repeat(20)),
            deployment_block: 100,
            deployment_timestamp: 1_700_000_000,
            module_addresses: ManifestModuleAddresses {
                subaccount_registry: format!("0x{}", "11".repeat(20)),
                collateral_vault: format!("0x{}", "22".repeat(20)),
                options_positions_ledger: format!("0x{}", "33".repeat(20)),
                risk_module: format!("0x{}", "44".repeat(20)),
                margin_engine: format!("0x{}", "55".repeat(20)),
                option_matching_engine: hex_20(&target),
                escape_controller: format!("0x{}", "77".repeat(20)),
                recovery_finalizer: format!("0x{}", "88".repeat(20)),
                oracle_adapter: format!("0x{}", "99".repeat(20)),
                options_risk_provider: format!("0x{}", "aa".repeat(20)),
                quote_token: format!("0x{}", "bb".repeat(20)),
                fees_manager_v2: None,
                option_execution_fee_adapter: None,
                protocol_timelock: None,
                governance: None,
                guardian: None,
            },
            protocol_fee_subkey: format!("0x{}", "aa".repeat(32)),
            rebate_budget_subkey: format!("0x{}", "bb".repeat(32)),
            insurance_fund_subkey: format!("0x{}", "cc".repeat(32)),
            max_collateral_tokens: 8,
            max_active_series: 32,
            all_capabilities_mask: "65535".into(),
            recovery_activation_delay_seconds: 3600,
            recovery_pause_max_duration_blocks: 100_800,
            activation_status: ActivationStatus::Active,
        }
    }

    fn baseline_row(chain_id: u64, target: [u8; 20], calldata: Vec<u8>) -> ExecutionRequestRow {
        let calldata_hash = keccak_of(&calldata);
        let cid = format!("0x{}", "cd".repeat(32));
        let plan_hash = derive_plan_hash(
            chain_id,
            &target,
            &[0x00u8; 4],
            &calldata_hash,
            &U256::ZERO,
            &cid,
        );
        ExecutionRequestRow {
            canonical_execution_id: cid,
            deployment_id: 1,
            chain_id: chain_id as i64,
            execution_kind: "HYBRID_V2_OPTION_MATCH".into(),
            buyer_order_hash: format!("0x{}", "aa".repeat(32)),
            seller_order_hash: format!("0x{}", "bb".repeat(32)),
            buyer_subkey: format!("0x{}", "aa".repeat(32)),
            seller_subkey: format!("0x{}", "bb".repeat(32)),
            series_id: "42".into(),
            fill_quantity_1e8: "100000000".into(),
            premium_amount: "50000000".into(),
            fee_schedule_epoch: None,
            source_matched_execution_id: None,
            target_contract: hex_20(&target),
            selector: "0x00000000".into(),
            calldata_hash: Some(hex_32(&calldata_hash)),
            calldata_bytes: Some(calldata),
            plan_hash: Some(hex_32(&plan_hash)),
            tx_value_wei: "0".into(),
            simulation_block_number: Some(1),
            simulation_block_hash: Some(format!("0x{}", "cc".repeat(32))),
            simulation_gas_estimate: Some(500_000),
            simulation_result_json: Some(serde_json::json!({})),
            signer_identity: Some(format!("0x{}", "77".repeat(20))),
            signing_payload_hash: Some(format!("0x{}", "ff".repeat(32))),
            signature_r: Some(format!("0x{}", "11".repeat(32))),
            signature_s: Some(format!("0x{}", "22".repeat(32))),
            signature_v: Some(0),
            recovered_signer: Some(format!("0x{}", "77".repeat(20))),
            gas_limit: Some(1_000_000),
            max_fee_per_gas_wei: Some("2000000000".into()),
            max_priority_fee_per_gas_wei: Some("500000000".into()),
            reserved_nonce: Some(42),
            phase: ExecutionPhase::ReadyForBroadcast,
            failure_class: None,
            failure_detail: None,
            retry_count: 0,
            holder_epoch: None,
            signer_request_idempotency_key: None,
            created_at_ms: 1,
            updated_at_ms: 1,
        }
    }

    #[test]
    fn reconstruct_plan_happy_path_integrity_holds() {
        let target = [0xeeu8; 20];
        let row = baseline_row(84532, target, vec![0xaa, 0xbb, 0xcc]);
        let m = test_manifest(84532, target);
        let plan = reconstruct_plan(&row, &m).expect("plan");
        assert_eq!(plan.chain_id, 84532);
        assert_eq!(plan.target, target);
        assert_eq!(plan.value_wei, U256::ZERO);
        assert_eq!(plan.calldata, vec![0xaa, 0xbb, 0xcc]);
    }

    #[test]
    fn reconstruct_plan_refuses_missing_bytes() {
        let target = [0xeeu8; 20];
        let mut row = baseline_row(84532, target, vec![0x01]);
        row.calldata_bytes = None;
        let m = test_manifest(84532, target);
        assert_eq!(
            reconstruct_plan(&row, &m).unwrap_err(),
            ReconstructionError::CalldataBytesMissing
        );
    }

    #[test]
    fn reconstruct_plan_refuses_hash_mismatch() {
        let target = [0xeeu8; 20];
        let mut row = baseline_row(84532, target, vec![0x01, 0x02]);
        // Overwrite calldata_hash with something different.
        row.calldata_hash = Some(format!("0x{}", "de".repeat(32)));
        let m = test_manifest(84532, target);
        match reconstruct_plan(&row, &m).unwrap_err() {
            ReconstructionError::CalldataHashMismatch { .. } => {}
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn reconstruct_plan_refuses_nonzero_value() {
        let target = [0xeeu8; 20];
        let mut row = baseline_row(84532, target, vec![0x01]);
        row.tx_value_wei = "1".into();
        let m = test_manifest(84532, target);
        assert_eq!(
            reconstruct_plan(&row, &m).unwrap_err(),
            ReconstructionError::ValueNonZero("1".into())
        );
    }

    #[test]
    fn reconstruct_plan_refuses_chain_mismatch() {
        let target = [0xeeu8; 20];
        let row = baseline_row(84532, target, vec![0x01]);
        let m = test_manifest(1, target); // different chain
        match reconstruct_plan(&row, &m).unwrap_err() {
            ReconstructionError::ChainIdMismatch { .. } => {}
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn reconstruct_plan_refuses_target_mismatch() {
        let target = [0xeeu8; 20];
        let row = baseline_row(84532, target, vec![0x01]);
        let m = test_manifest(84532, [0xddu8; 20]);
        match reconstruct_plan(&row, &m).unwrap_err() {
            ReconstructionError::ManifestLookupFailure(_) => {}
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn reconstruct_plan_refuses_plan_hash_mismatch() {
        let target = [0xeeu8; 20];
        let mut row = baseline_row(84532, target, vec![0x01]);
        row.plan_hash = Some(format!("0x{}", "de".repeat(32)));
        let m = test_manifest(84532, target);
        match reconstruct_plan(&row, &m).unwrap_err() {
            ReconstructionError::PlanHashMismatch { .. } => {}
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn reconstruct_signed_happy_path() {
        let target = [0xeeu8; 20];
        let row = baseline_row(84532, target, vec![0x01]);
        let signed = reconstruct_signed(&row).expect("signed");
        assert_eq!(signed.tx_type, 2);
        assert_eq!(signed.signature_v, 0);
        assert_eq!(signed.recovered_signer, [0x77u8; 20]);
    }

    #[test]
    fn reconstruct_signed_refuses_missing_signature() {
        let target = [0xeeu8; 20];
        let mut row = baseline_row(84532, target, vec![0x01]);
        row.signature_r = None;
        assert_eq!(
            reconstruct_signed(&row).unwrap_err(),
            ReconstructionError::SignatureMissing
        );
    }

    #[test]
    fn reconstruct_signed_refuses_missing_recovered_signer() {
        let target = [0xeeu8; 20];
        let mut row = baseline_row(84532, target, vec![0x01]);
        row.recovered_signer = None;
        assert_eq!(
            reconstruct_signed(&row).unwrap_err(),
            ReconstructionError::RecoveredSignerMissing
        );
    }

    #[test]
    fn reconstruct_signed_refuses_out_of_range_v() {
        let target = [0xeeu8; 20];
        let mut row = baseline_row(84532, target, vec![0x01]);
        row.signature_v = Some(27);
        assert_eq!(
            reconstruct_signed(&row).unwrap_err(),
            ReconstructionError::SignatureMissing
        );
    }
}
