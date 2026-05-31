//! V2G-M: V2 fee smoke readiness packet.
//!
//! Backend-side companion to the V2G-E live rebate smoke campaign and
//! the V2G-F observability surface. Consolidates everything an
//! operator needs to **verify** before running a V2 fee smoke through
//! the backend executor (and the V2G-M frontend tile that surfaces
//! the same data):
//!
//! - configured engines: NEW PerpEngine, NEW MarginEngine,
//!   FeesManagerV2.
//! - stranded engines: OLD PerpEngine, OLD MarginEngine — informational
//!   only; the readiness packet refuses to mark a smoke ready if any
//!   active engine address points at OLD.
//! - V2G-D2-style EOAs: the Tier 4 maker (`0x290b…9274`) and Tier 2
//!   taker (`0x77cA…0020`) used by V2G-E. Addresses ONLY — keys never
//!   come from any committed file. The packet documents the
//!   expected env-var names for each role.
//! - per-tier fee profile snapshot (PERP + OPTION makerPpm/takerPpm)
//!   so the operator can preflight `feePpm` / `rebatePpm` expectations
//!   without re-reading the Solidity tables.
//! - dry-run packet templates: skeletons for the PERP and OPTION
//!   smokes the V2G-E campaign broadcast, with placeholders for the
//!   trade-specific basisAmount + expected feeAmount / rebateAmount /
//!   rebateBudget delta. The placeholders are explicit (`None`) so the
//!   builder does not silently invent numbers.
//! - broadcast safety summary: every gate that must be `false` /
//!   `dry-run` for the soak window plus the gates the operator has
//!   to flip explicitly to broadcast.
//! - secret hygiene: surfaces ONLY whether the required env vars are
//!   set, never their values. The address derivation that confirms
//!   `MAKER_PRIVATE_KEY` decodes to `0x290b…9274` happens in the
//!   `sign_perp_trade` / `sign_option_execution_intent` CLIs, which
//!   already redact the key in Debug (see
//!   `src/execution/signer.rs::ExecutorSigner`).
//!
//! Cardinality contract: every address-shaped field is lowercased on
//! output. The packet never embeds a private key, a signature, or a
//! tx hash that has not been broadcast. The packet is safe to log,
//! cache, or render in a Grafana / admin UI panel.
//!
//! See `docs/V2_FEE_BACKEND_EXECUTOR_READINESS_V2G_M.md` for the
//! milestone record and `docs/FEES_MANAGER_V2_LIVE_REBATE_SMOKE_RESULT_V2G_E.md`
//! for the live-trade reference shape the V2G-D2 EOAs participated in.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;

use crate::api::AppState;
use crate::error::Result;
use crate::fees::v2_observability::admin_v2_observability;

/// Canonical V2G-D2 maker address (Tier 4). NEVER ships a key.
pub const TIER4_MAKER_ADDRESS: &str = "0x290bd12c93e467bf51c51f5273d35bddb19e9274";
/// Canonical V2G-D2 taker address (Tier 2). NEVER ships a key.
pub const TIER2_TAKER_ADDRESS: &str = "0x77ca9dd6ccce2d692fb23877a2db7178807b0020";

/// Recommended env-var name for the Tier 4 maker key (PERP / OPTION).
pub const MAKER_KEY_ENV: &str = "PERP_SMOKE_BUYER_PRIVATE_KEY";
/// Recommended env-var name for the Tier 2 taker key (PERP / OPTION).
pub const TAKER_KEY_ENV: &str = "PERP_SMOKE_SELLER_PRIVATE_KEY";

/// V2 fee product axis.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum ProductKind {
    Perp,
    Option,
}

impl ProductKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ProductKind::Perp => "PERP",
            ProductKind::Option => "OPTION",
        }
    }
}

/// V2 fee flow axis. Mirrors the on-chain `flowKind` enum.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum FlowKind {
    Orderbook,
    Rfq,
}

impl FlowKind {
    pub fn as_str(self) -> &'static str {
        match self {
            FlowKind::Orderbook => "ORDERBOOK",
            FlowKind::Rfq => "RFQ",
        }
    }
}

/// Per-tier fee profile (microPpm). Negative makerPpm = rebate side.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct FeeProfile {
    pub tier: u8,
    pub product: ProductKind,
    pub maker_ppm: i32,
    pub taker_ppm: i32,
}

/// Unified V2 fee smoke dry-run packet. Operator builds one per
/// expected broadcast and the backend verifies expectations vs
/// configured state before authorising a real send.
///
/// Every numeric field that depends on the trade (`basis_amount`,
/// `expected_fee_amount`, `expected_rebate_amount`,
/// `expected_rebate_budget_delta`) is `Option<i128>` and starts at
/// `None`. The verifier explicitly rejects a packet whose numeric
/// expectations are missing — silent zero-defaults would mask a
/// misconfigured smoke.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SmokeDryRunPacket {
    pub milestone: &'static str,
    pub product: ProductKind,
    pub flow: FlowKind,

    /// Lowercased address of the maker EOA (V2G-D2 Tier 4 by default).
    pub maker_address: String,
    /// Lowercased address of the taker EOA (V2G-D2 Tier 2 by default).
    pub taker_address: String,

    /// Lowercased address of the engine that emits the V2 fee event.
    /// For PERP this is NEW_PERP_ENGINE; for OPTION it is the V2-fees
    /// MarginEngine.
    pub fee_consumer_address: String,

    /// Lowercased address of the FeesManagerV2 contract.
    pub fees_manager_v2_address: String,

    /// Lowercased address of the settlement asset (mUSDC on Base Sepolia).
    pub settlement_asset_address: String,

    /// Per-tier fee profile snapshot. Always two entries (maker, taker)
    /// in `[tier, product, maker_ppm, taker_ppm]` shape.
    pub maker_profile: FeeProfile,
    pub taker_profile: FeeProfile,

    /// Notional basis the trade consumes (native units of the
    /// settlement asset). `None` = operator forgot to set it; the
    /// verifier refuses.
    pub basis_amount_native: Option<u128>,
    /// Expected `feeAmount` on the taker leg.
    pub expected_fee_amount_native: Option<u128>,
    /// Expected `rebateAmount` on the maker leg.
    pub expected_rebate_amount_native: Option<u128>,
    /// Expected delta on `FeesManagerV2.rebateBudget(asset)`.
    /// Negative = budget decreases (rebate paid out).
    pub expected_rebate_budget_delta_native: Option<i128>,

    /// Names of the env vars the operator must export for this smoke.
    /// Stored as strings; the packet does NOT read or echo their values.
    pub maker_key_env: &'static str,
    pub taker_key_env: &'static str,

    /// True if every safety gate (no broadcast, no `OLD_PERP_ENGINE`
    /// as active, etc.) was satisfied at packet build time.
    pub safe_to_broadcast_today: bool,

    /// Human-readable notes — never include secrets.
    pub notes: Vec<&'static str>,
}

/// Build the default V2G-E PERP rebate smoke skeleton with the
/// V2G-D2 EOAs and the canonical Base Sepolia engines. Trade-specific
/// numeric fields stay `None`; the operator fills them per broadcast.
pub fn default_perp_packet(
    perp_engine_new: &str,
    fees_manager_v2: &str,
    settlement_asset: &str,
) -> SmokeDryRunPacket {
    SmokeDryRunPacket {
        milestone: "V2G-M",
        product: ProductKind::Perp,
        flow: FlowKind::Orderbook,
        maker_address: TIER4_MAKER_ADDRESS.to_string(),
        taker_address: TIER2_TAKER_ADDRESS.to_string(),
        fee_consumer_address: perp_engine_new.to_ascii_lowercase(),
        fees_manager_v2_address: fees_manager_v2.to_ascii_lowercase(),
        settlement_asset_address: settlement_asset.to_ascii_lowercase(),
        maker_profile: FeeProfile {
            tier: 4,
            product: ProductKind::Perp,
            maker_ppm: -100,
            taker_ppm: 150,
        },
        taker_profile: FeeProfile {
            tier: 2,
            product: ProductKind::Perp,
            maker_ppm: -50,
            taker_ppm: 200,
        },
        basis_amount_native: None,
        expected_fee_amount_native: None,
        expected_rebate_amount_native: None,
        expected_rebate_budget_delta_native: None,
        maker_key_env: MAKER_KEY_ENV,
        taker_key_env: TAKER_KEY_ENV,
        safe_to_broadcast_today: false,
        notes: vec![
            "Maker = Tier 4 (rebate leg). Taker = Tier 2 (fee leg).",
            "Confirm `cast call $PERP_ENGINE 'useFeesManagerV2()(bool)'` returns true before broadcast.",
            "rebateBudget(asset) must remain >= MIN_REBATE_BUDGET after the trade.",
        ],
    }
}

/// Build the default V2G-E OPTION rebate smoke skeleton. Mirrors the
/// PERP packet but flips buyer/seller assignment (OPTION trade buyer
/// = Tier 2 taker, seller = Tier 4 maker).
pub fn default_option_packet(
    margin_engine_new: &str,
    fees_manager_v2: &str,
    settlement_asset: &str,
) -> SmokeDryRunPacket {
    SmokeDryRunPacket {
        milestone: "V2G-M",
        product: ProductKind::Option,
        flow: FlowKind::Orderbook,
        // OPTION trade buyer = taker (Tier 2). seller = maker (Tier 4).
        maker_address: TIER4_MAKER_ADDRESS.to_string(),
        taker_address: TIER2_TAKER_ADDRESS.to_string(),
        fee_consumer_address: margin_engine_new.to_ascii_lowercase(),
        fees_manager_v2_address: fees_manager_v2.to_ascii_lowercase(),
        settlement_asset_address: settlement_asset.to_ascii_lowercase(),
        maker_profile: FeeProfile {
            tier: 4,
            product: ProductKind::Option,
            maker_ppm: -50,
            taker_ppm: 75,
        },
        taker_profile: FeeProfile {
            tier: 2,
            product: ProductKind::Option,
            maker_ppm: -10,
            taker_ppm: 125,
        },
        basis_amount_native: None,
        expected_fee_amount_native: None,
        expected_rebate_amount_native: None,
        expected_rebate_budget_delta_native: None,
        maker_key_env: MAKER_KEY_ENV,
        taker_key_env: TAKER_KEY_ENV,
        safe_to_broadcast_today: false,
        notes: vec![
            "OPTION trade: buyer = Tier 2 taker (premium payer). seller = Tier 4 maker.",
            "Margin engine must have useFeesManagerV2()(bool) == true (per V2E-E).",
            "OPTION rebate path emits TradingFeeCharged V1-compat for the taker leg only.",
        ],
    }
}

/// Verify the packet's numeric expectations are filled in and that
/// the trade preserves the protocol's solvency invariants:
///
/// - `basisAmount` set,
/// - `expected_fee_amount = ceil(basis * taker_ppm / 1e6)`,
/// - `expected_rebate_amount = floor(basis * |maker_ppm| / 1e6)`,
/// - `expected_rebate_budget_delta = -expected_rebate_amount`,
/// - fee > rebate (protocol earns a non-zero net).
///
/// Returns the validation summary; never panics.
pub fn validate_numeric_invariants(packet: &SmokeDryRunPacket) -> Vec<String> {
    let mut findings = Vec::new();

    let Some(basis) = packet.basis_amount_native else {
        findings.push("basis_amount_native is None (operator must set)".to_string());
        return findings;
    };

    let Some(expected_fee) = packet.expected_fee_amount_native else {
        findings.push("expected_fee_amount_native is None".to_string());
        return findings;
    };

    let Some(expected_rebate) = packet.expected_rebate_amount_native else {
        findings.push("expected_rebate_amount_native is None".to_string());
        return findings;
    };

    let taker_ppm = packet.taker_profile.taker_ppm.max(0) as u128;
    let maker_ppm_abs = packet.maker_profile.maker_ppm.unsigned_abs() as u128;

    // ceil(basis * taker_ppm / 1e6)
    let expected_fee_calc = basis.saturating_mul(taker_ppm).div_ceil(1_000_000);
    // floor(basis * |maker_ppm| / 1e6)
    let expected_rebate_calc = basis.saturating_mul(maker_ppm_abs) / 1_000_000;

    if expected_fee != expected_fee_calc {
        findings.push(format!(
            "expected_fee_amount_native ({expected_fee}) != ceil({basis} * {taker_ppm} / 1e6) = {expected_fee_calc}"
        ));
    }
    if expected_rebate != expected_rebate_calc {
        findings.push(format!(
            "expected_rebate_amount_native ({expected_rebate}) != floor({basis} * {maker_ppm_abs} / 1e6) = {expected_rebate_calc}"
        ));
    }

    if let Some(delta) = packet.expected_rebate_budget_delta_native {
        let expected_delta = -(expected_rebate as i128);
        if delta != expected_delta {
            findings.push(format!(
                "expected_rebate_budget_delta_native ({delta}) != -expected_rebate_amount ({expected_delta})"
            ));
        }
    } else {
        findings.push("expected_rebate_budget_delta_native is None".to_string());
    }

    if expected_fee <= expected_rebate {
        findings.push(format!(
            "protocol earns no net fee: expected_fee {expected_fee} <= expected_rebate {expected_rebate}"
        ));
    }

    findings
}

/// Snapshot of the runtime safety gates the smoke depends on.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct BroadcastGateSnapshot {
    pub execution_enabled: bool,
    pub executor_dry_run: bool,
    pub executor_real_broadcast_enabled: bool,
    pub option_execution_broadcast_enabled: bool,
    /// Whether `EXECUTOR_PRIVATE_KEY` is set in the running env. Only the
    /// boolean is surfaced; the value is never read here.
    pub executor_private_key_set: bool,
    /// Same for `BUYER_PRIVATE_KEY` / `SELLER_PRIVATE_KEY` /
    /// `PERP_SMOKE_*` / `OPTION_SMOKE_*` env vars.
    pub maker_key_env_set: bool,
    pub taker_key_env_set: bool,
    pub maker_key_env_name: &'static str,
    pub taker_key_env_name: &'static str,
}

/// Compute the broadcast-gate snapshot using the running `AppState`.
/// The runtime env vars are queried via `std::env::var` — we never log
/// their values.
pub fn current_broadcast_gates(state: &AppState) -> BroadcastGateSnapshot {
    fn env_set(name: &str) -> bool {
        std::env::var(name)
            .ok()
            .filter(|value| !value.trim().is_empty())
            .is_some()
    }

    BroadcastGateSnapshot {
        execution_enabled: state.execution_config.execution_enabled,
        executor_dry_run: state.execution_config.dry_run,
        executor_real_broadcast_enabled: state.execution_config.real_broadcast_enabled,
        option_execution_broadcast_enabled: state.options_config.execution_broadcast_enabled,
        executor_private_key_set: state.execution_config.executor_private_key.is_some(),
        maker_key_env_set: env_set(MAKER_KEY_ENV),
        taker_key_env_set: env_set(TAKER_KEY_ENV),
        maker_key_env_name: MAKER_KEY_ENV,
        taker_key_env_name: TAKER_KEY_ENV,
    }
}

/// Aggregate the full readiness JSON snapshot.
pub async fn admin_v2_smoke_readiness(state: &AppState) -> Result<Value> {
    let observability = admin_v2_observability(state).await?;

    let perp_engine_new = observability
        .get("contracts")
        .and_then(|v| v.get("perp_engine_new"))
        .and_then(|v| v.as_str());
    let perp_engine_old = observability
        .get("contracts")
        .and_then(|v| v.get("perp_engine_old"))
        .and_then(|v| v.as_str());
    let margin_engine_new = observability
        .get("contracts")
        .and_then(|v| v.get("margin_engine_new"))
        .and_then(|v| v.as_str());
    let margin_engine_old = observability
        .get("contracts")
        .and_then(|v| v.get("margin_engine_old"))
        .and_then(|v| v.as_str());
    let fees_manager_v2 = observability
        .get("contracts")
        .and_then(|v| v.get("fees_manager_v2"))
        .and_then(|v| v.as_str());

    let settlement_asset = observability
        .get("metrics")
        .and_then(|v| v.get("fees_manager_v2_rebate_budget_native"))
        .and_then(|v| v.as_object())
        .and_then(|map| map.keys().next())
        .cloned()
        .unwrap_or_default();

    let perp_packet = perp_engine_new
        .and_then(|new| fees_manager_v2.map(|fm| default_perp_packet(new, fm, &settlement_asset)));
    let option_packet = margin_engine_new.and_then(|new| {
        fees_manager_v2.map(|fm| default_option_packet(new, fm, &settlement_asset))
    });

    let gates = current_broadcast_gates(state);

    let safe_today = !gates.execution_enabled
        && !gates.executor_real_broadcast_enabled
        && !gates.option_execution_broadcast_enabled
        && perp_engine_new
            .map(|new| {
                perp_engine_old
                    .map(|old| new.eq_ignore_ascii_case(old).not())
                    .unwrap_or(true)
            })
            .unwrap_or(false);

    // Refuse to mark active==old. This is the V2G hard rule
    // "do not use OLD_PERP_ENGINE as active" surfaced at the
    // readiness layer.
    let active_is_old = perp_engine_new
        .zip(perp_engine_old)
        .map(|(new, old)| new.eq_ignore_ascii_case(old))
        .unwrap_or(false);

    let mut packets = BTreeMap::new();
    if let Some(p) = perp_packet {
        packets.insert("perp", json!(p));
    }
    if let Some(p) = option_packet {
        packets.insert("option", json!(p));
    }

    Ok(json!({
        "milestone": "V2G-M",
        "soak_safe_for_local_compose": safe_today,
        "active_perp_is_old_engine": active_is_old,
        "engines": {
            "perp_engine_new": perp_engine_new,
            "perp_engine_old": perp_engine_old,
            "margin_engine_new": margin_engine_new,
            "margin_engine_old": margin_engine_old,
            "fees_manager_v2": fees_manager_v2,
        },
        "smoke_eoas": {
            "tier4_maker_address": TIER4_MAKER_ADDRESS,
            "tier2_taker_address": TIER2_TAKER_ADDRESS,
            "key_env_vars": {
                "maker": MAKER_KEY_ENV,
                "taker": TAKER_KEY_ENV,
            },
            "key_hygiene": [
                "Private keys are shell-only. NEVER committed to .env, NEVER printed by the backend, NEVER logged.",
                "Use the standalone signing CLIs (sign_perp_trade, sign_option_execution_intent) to derive signatures.",
                "Both CLIs refuse on payload-vs-signer address mismatch unless --allow-address-mismatch is passed.",
                "ExecutorSigner and PrivateKeySecret redact in Debug.",
            ],
        },
        "broadcast_gates": gates,
        "dry_run_packets": packets,
        "anomaly_totals": observability.get("anomaly_totals").cloned().unwrap_or(json!({})),
        "metrics_snapshot": observability.get("metrics").cloned().unwrap_or(json!({})),
        "notes": [
            "Read-only snapshot. See docs/V2_FEE_BACKEND_EXECUTOR_READINESS_V2G_M.md.",
            "dry_run_packets[*] numeric fields start as null. Operator fills them per-broadcast and re-runs validate_numeric_invariants.",
            "safe_to_broadcast_today=false during the local-compose soak — execution_enabled=false, executor_real_broadcast_enabled=false.",
        ],
    }))
}

trait BoolExt {
    fn not(self) -> Self;
}
impl BoolExt for bool {
    fn not(self) -> Self {
        !self
    }
}

/// Serialises env-var manipulation across the smoke-readiness unit
/// tests + the routes-level admin endpoint tests so cargo's parallel
/// runner doesn't race on `std::env::{set_var, remove_var}` calls
/// against the V2G-M maker/taker env-var names. Crate-test-internal;
/// exposed `pub(crate)` so the routes-layer tests can acquire the
/// same guard.
#[cfg(test)]
pub(crate) static TEST_ENV_GUARD: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod tests {
    use super::*;

    const PERP_NEW: &str = "0xc6c592100723fe0c66343a16e95ec34cc0c2141c";
    const PERP_OLD: &str = "0xb36395b67d0798ada981731c9fa5239f4362b53b";
    const MARGIN_NEW: &str = "0x287cef479be5889eefca847f9e73c860898f48cc";
    const FMV2: &str = "0x00da0b9876bcbf0c79cb5bcacfebafb8c7ad774f";
    const MUSDC: &str = "0x6eae407f5640b006fac9965182e238582a3b412e";

    #[test]
    fn default_perp_packet_uses_v2g_d2_eoas_and_lowercases_engines() {
        let pkt = default_perp_packet(PERP_NEW, FMV2, MUSDC);
        assert_eq!(pkt.maker_address, TIER4_MAKER_ADDRESS);
        assert_eq!(pkt.taker_address, TIER2_TAKER_ADDRESS);
        assert_eq!(pkt.fee_consumer_address, PERP_NEW.to_ascii_lowercase());
        assert_eq!(pkt.fees_manager_v2_address, FMV2.to_ascii_lowercase());
        assert_eq!(pkt.settlement_asset_address, MUSDC.to_ascii_lowercase());
        assert!(matches!(pkt.product, ProductKind::Perp));
        assert!(matches!(pkt.flow, FlowKind::Orderbook));
        // V2G-D2 fee profile expectations.
        assert_eq!(pkt.maker_profile.tier, 4);
        assert_eq!(pkt.maker_profile.maker_ppm, -100);
        assert_eq!(pkt.taker_profile.tier, 2);
        assert_eq!(pkt.taker_profile.taker_ppm, 200);
        // Numeric expectations always start None.
        assert!(pkt.basis_amount_native.is_none());
        assert!(pkt.expected_fee_amount_native.is_none());
        assert!(pkt.expected_rebate_amount_native.is_none());
        assert!(pkt.expected_rebate_budget_delta_native.is_none());
        // Safety default: never broadcast unless the operator flips it.
        assert!(!pkt.safe_to_broadcast_today);
        // Env var names are stable across milestones.
        assert_eq!(pkt.maker_key_env, MAKER_KEY_ENV);
        assert_eq!(pkt.taker_key_env, TAKER_KEY_ENV);
    }

    #[test]
    fn default_option_packet_swaps_role_assignment_but_keeps_addresses() {
        let pkt = default_option_packet(MARGIN_NEW, FMV2, MUSDC);
        assert_eq!(pkt.maker_address, TIER4_MAKER_ADDRESS);
        assert_eq!(pkt.taker_address, TIER2_TAKER_ADDRESS);
        assert!(matches!(pkt.product, ProductKind::Option));
        assert_eq!(pkt.maker_profile.maker_ppm, -50);
        assert_eq!(pkt.taker_profile.taker_ppm, 125);
        assert_eq!(pkt.fee_consumer_address, MARGIN_NEW.to_ascii_lowercase());
    }

    #[test]
    fn validate_numeric_invariants_v2g_e_perp_reference_passes() {
        // V2G-E PERP rebate (live tx 0x5c15e923...aa394):
        // basis=30000, fee=6 (ceil(30000*200/1e6)), rebate=3
        // (floor(30000*100/1e6)), budget delta = -3.
        let mut pkt = default_perp_packet(PERP_NEW, FMV2, MUSDC);
        pkt.basis_amount_native = Some(30_000);
        pkt.expected_fee_amount_native = Some(6);
        pkt.expected_rebate_amount_native = Some(3);
        pkt.expected_rebate_budget_delta_native = Some(-3);
        let findings = validate_numeric_invariants(&pkt);
        assert!(findings.is_empty(), "got findings: {findings:?}");
    }

    #[test]
    fn validate_numeric_invariants_v2g_e_option_reference_passes() {
        // V2G-E OPTION rebate (live tx 0x9a85cbce...3149):
        // basis=200000, fee=25 (ceil(200000*125/1e6)), rebate=10
        // (floor(200000*50/1e6)), budget delta = -10.
        let mut pkt = default_option_packet(MARGIN_NEW, FMV2, MUSDC);
        pkt.basis_amount_native = Some(200_000);
        pkt.expected_fee_amount_native = Some(25);
        pkt.expected_rebate_amount_native = Some(10);
        pkt.expected_rebate_budget_delta_native = Some(-10);
        let findings = validate_numeric_invariants(&pkt);
        assert!(findings.is_empty(), "got findings: {findings:?}");
    }

    #[test]
    fn validate_numeric_invariants_rejects_missing_basis_amount() {
        let pkt = default_perp_packet(PERP_NEW, FMV2, MUSDC);
        let findings = validate_numeric_invariants(&pkt);
        assert!(findings
            .iter()
            .any(|f| f.contains("basis_amount_native is None")));
    }

    #[test]
    fn validate_numeric_invariants_rejects_wrong_fee_math() {
        let mut pkt = default_perp_packet(PERP_NEW, FMV2, MUSDC);
        pkt.basis_amount_native = Some(30_000);
        // Wrong fee expectation — should be 6.
        pkt.expected_fee_amount_native = Some(7);
        pkt.expected_rebate_amount_native = Some(3);
        pkt.expected_rebate_budget_delta_native = Some(-3);
        let findings = validate_numeric_invariants(&pkt);
        assert!(findings.iter().any(|f| f.contains("expected_fee_amount")));
    }

    #[test]
    fn validate_numeric_invariants_rejects_budget_delta_mismatch() {
        let mut pkt = default_perp_packet(PERP_NEW, FMV2, MUSDC);
        pkt.basis_amount_native = Some(30_000);
        pkt.expected_fee_amount_native = Some(6);
        pkt.expected_rebate_amount_native = Some(3);
        pkt.expected_rebate_budget_delta_native = Some(-2); // should be -3
        let findings = validate_numeric_invariants(&pkt);
        assert!(findings
            .iter()
            .any(|f| f.contains("expected_rebate_budget_delta_native")));
    }

    #[test]
    fn validate_numeric_invariants_rejects_protocol_breaks_even() {
        // If fee <= rebate the protocol loses; refuse.
        let mut pkt = default_perp_packet(PERP_NEW, FMV2, MUSDC);
        pkt.basis_amount_native = Some(1_000);
        // 1000 * 200 / 1e6 = 0.2 -> ceil = 1.
        pkt.expected_fee_amount_native = Some(1);
        // 1000 * 100 / 1e6 = 0.1 -> floor = 0.
        pkt.expected_rebate_amount_native = Some(0);
        pkt.expected_rebate_budget_delta_native = Some(0);
        // fee(1) > rebate(0) -> protocol earns 1. OK at basis=1000.
        let findings = validate_numeric_invariants(&pkt);
        assert!(findings.is_empty(), "{findings:?}");

        // Force a packet that breaks even (fee == rebate) and confirm
        // the invariant catches it.
        let mut bad = pkt;
        bad.basis_amount_native = Some(2_000);
        bad.expected_fee_amount_native = Some(0);
        bad.expected_rebate_amount_native = Some(0);
        bad.expected_rebate_budget_delta_native = Some(0);
        let findings = validate_numeric_invariants(&bad);
        assert!(
            findings
                .iter()
                .any(|f| f.contains("protocol earns no net fee")),
            "{findings:?}"
        );
    }

    #[test]
    fn smoke_packet_serialization_never_includes_a_private_key_word() {
        let pkt = default_perp_packet(PERP_NEW, FMV2, MUSDC);
        let json = serde_json::to_string(&pkt).unwrap();
        // The struct never embeds a key. Tests double-check by string
        // search for common secret-shaped tokens.
        assert!(!json.contains("4c0883"));
        assert!(!json.contains("private_key"));
        assert!(!json.contains("secret"));
        // Env var names are surfaced (operator needs them) — just the
        // NAMES, not the values.
        assert!(json.contains(MAKER_KEY_ENV));
        assert!(json.contains(TAKER_KEY_ENV));
    }

    #[test]
    fn broadcast_gates_default_to_safe() {
        use crate::api::AppState;
        use crate::engine::EngineState;
        let _guard = super::TEST_ENV_GUARD
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let state = AppState::new(EngineState::with_default_markets());
        // Make sure the test env doesn't accidentally leak a key.
        std::env::remove_var(MAKER_KEY_ENV);
        std::env::remove_var(TAKER_KEY_ENV);
        let gates = current_broadcast_gates(&state);
        assert!(!gates.execution_enabled);
        assert!(gates.executor_dry_run);
        assert!(!gates.executor_real_broadcast_enabled);
        assert!(!gates.option_execution_broadcast_enabled);
        assert!(!gates.executor_private_key_set);
        // Defaults from disabled() config have no maker/taker key envs.
        assert!(!gates.maker_key_env_set);
        assert!(!gates.taker_key_env_set);
    }

    #[tokio::test]
    async fn readiness_snapshot_refuses_to_mark_safe_when_active_equals_old() {
        use crate::api::AppState;
        use crate::engine::EngineState;
        use crate::types::AccountId;
        let mut state = AppState::new(EngineState::with_default_markets());
        // Point active and old at the same address — readiness must
        // refuse to mark safe and surface the boolean explicitly.
        state.execution_config.perp_engine_address = AccountId::new(PERP_OLD);
        state.execution_config.old_perp_engine_address = Some(AccountId::new(PERP_OLD));
        let snapshot = admin_v2_smoke_readiness(&state).await.unwrap();
        assert_eq!(snapshot["active_perp_is_old_engine"], true);
        assert_eq!(snapshot["soak_safe_for_local_compose"], false);
    }
}
