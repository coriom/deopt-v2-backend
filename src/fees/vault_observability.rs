//! V2G-R5-OBS-P0 — ProtocolFeeVault backend observability surface.
//!
//! Read-only operator + Prometheus exporter backing for the
//! ProtocolFeeVault cutover. Implemented BEFORE the vault is deployed
//! so the cutover runbook can verify metrics/alerts on a candidate
//! address without taking the chain mutation.
//!
//! ## Configuration contract (shell-only env, no `.env` edit required)
//!
//! | env var                                     | required | semantics                                                                 |
//! |---------------------------------------------|----------|---------------------------------------------------------------------------|
//! | `PROTOCOL_FEE_VAULT_ADDRESS`                | optional | EVM address of the vault. If unset/zero, all endpoints report `not_configured` and no PFV metrics are emitted. |
//! | `PROTOCOL_FEE_VAULT_RECONCILIATION_ASSETS`  | optional | comma-separated EVM addresses to query. Defaults to the lowercase asset list already used by `deopt_fees_manager_v2_rebate_budget_native`. |
//!
//! Reads `RPC_URL`, `COLLATERAL_VAULT_ADDRESS`, `FEES_MANAGER_V2_ADDRESS`
//! from the existing backend config so the operator does not have to
//! duplicate them.
//!
//! ## Surface
//!
//! - `GET /admin/fees/vault/summary`        — global + aggregate per-asset.
//! - `GET /admin/fees/vault/balances`       — per-asset bucket breakdown.
//! - `GET /admin/fees/vault/reconciliation` — drift-focused view.
//! - Prometheus gauges (per-asset, labelled `asset` lowercase 0x-prefixed):
//!   `deopt_protocol_fee_vault_fee_balance_native`,
//!   `_rebate_reserve_native`, `_gross_fees_collected_native`,
//!   `_rebates_paid_native`, `_net_revenue_native`,
//!   `_internal_collateral_vault_balance_native`,
//!   `_raw_erc20_balance_native`, `_drift_native`,
//!   `_reserve_shortfall_native`, plus the global `_rebates_paused`.
//!
//! Drift is computed as
//! `internal_cv_balance − (feeBalance + rebateReserve)`. Invariant 2 of
//! `ProtocolFeeVault.sol` requires it to be exactly zero; any non-zero
//! value triggers the `ProtocolFeeVaultDrift` alert. Raw ERC-20 dust
//! captures any token balance the vault contract holds directly (the
//! vault accounts via the CollateralVault ledger, so a non-zero raw
//! balance is unexpected and triggers `ProtocolFeeVaultRawErc20Dust`).
//!
//! ## Graceful "not_configured" contract
//!
//! When `PROTOCOL_FEE_VAULT_ADDRESS` is unset (the V2G-R5-OBS-P0
//! posture before the vault deploy lands), every endpoint returns a
//! 200 JSON body with `configured=false`. The Prometheus exporter
//! emits no vault gauges. The backend continues to start and serve all
//! other endpoints unmodified.

use std::collections::BTreeMap;

use alloy_primitives::U256;
use serde::Serialize;

use crate::error::{BackendError, Result};
use crate::execution::rpc::{EthCallProvider, EthCallRequest, HttpJsonRpcProvider};
use crate::signing::eip712::{keccak256, parse_evm_address};
use crate::types::AccountId;

/// View signatures on `IProtocolFeeVault`. Selectors are derived at
/// call-encode time so the test vector matches the on-chain ABI.
const VIEW_FEE_BALANCE: &str = "feeBalance(address)";
const VIEW_REBATE_RESERVE: &str = "rebateReserve(address)";
const VIEW_GROSS_FEES_COLLECTED: &str = "grossFeesCollected(address)";
const VIEW_REBATES_PAID: &str = "rebatesPaid(address)";
const VIEW_NET_REVENUE: &str = "netRevenue(address)";
const VIEW_BOOTSTRAPPED: &str = "bootstrapped(address)";
const VIEW_REBATES_PAUSED: &str = "rebatesPaused()";
const VIEW_GUARDIAN: &str = "guardian()";
const VIEW_REVENUE_RECEIVER: &str = "revenueReceiver()";
const VIEW_OWNER: &str = "owner()";
const VIEW_COLLATERAL_VAULT_ON_PFV: &str = "collateralVault()";
const VIEW_FEES_MANAGER_V2_ON_PFV: &str = "feesManagerV2()";

const CV_BALANCES_VIEW: &str = "balances(address,address)";
const ERC20_BALANCE_OF: &str = "balanceOf(address)";

const ZERO_ADDR: &str = "0x0000000000000000000000000000000000000000";

/// Reason emitted when the vault address is absent — endpoints stay
/// 200 OK, no chain calls are issued.
const NOT_CONFIGURED_REASON: &str = "PROTOCOL_FEE_VAULT_ADDRESS is not set";
/// Reason emitted when the address is configured but no RPC URL is
/// available; endpoints stay 200 OK and the metric exporter skips.
const RPC_MISSING_REASON: &str = "RPC_URL is not configured";

/// Source-of-config for the observability surface. Built from
/// `read_config_from_env_and_state` and consumed by the snapshot +
/// metrics code paths.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VaultObservabilityConfig {
    /// Vault contract address. `None` when `PROTOCOL_FEE_VAULT_ADDRESS`
    /// is unset or `address(0)`. All public entry points special-case
    /// this branch.
    pub vault_address: Option<AccountId>,
    /// CollateralVault address — same one the option indexer already
    /// targets. Used to query `balances(vault, asset)`.
    pub collateral_vault_address: Option<AccountId>,
    /// FeesManagerV2 address from the option indexer. Used for the
    /// `feesManagerV2()` cross-check in the summary endpoint.
    pub fees_manager_v2_address: Option<AccountId>,
    /// JSON-RPC endpoint URL — same one the rest of the backend uses
    /// for view calls.
    pub rpc_url: Option<String>,
    /// Settlement assets to scan. Each entry is the lowercase
    /// 0x-prefixed EVM address.
    pub assets: Vec<AccountId>,
}

impl VaultObservabilityConfig {
    /// True when the vault address is unset. Used to short-circuit the
    /// snapshot + metrics paths into the "not_configured" branch.
    pub fn is_configured(&self) -> bool {
        self.vault_address.is_some()
    }
}

/// Read configuration from process env. Pure: no chain calls, no
/// allocations beyond the returned struct. Falls back to the rebate
/// budget asset list when `PROTOCOL_FEE_VAULT_RECONCILIATION_ASSETS`
/// is unset.
pub fn build_config(
    rpc_url: Option<String>,
    collateral_vault_address: Option<AccountId>,
    fees_manager_v2_address: Option<AccountId>,
    fallback_assets: Vec<String>,
) -> VaultObservabilityConfig {
    let vault_address = std::env::var("PROTOCOL_FEE_VAULT_ADDRESS")
        .ok()
        .and_then(|v| sanitize_address(&v));

    let asset_strings = std::env::var("PROTOCOL_FEE_VAULT_RECONCILIATION_ASSETS")
        .ok()
        .map(|s| {
            s.split(',')
                .filter_map(sanitize_asset_string)
                .collect::<Vec<_>>()
        })
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| {
            fallback_assets
                .into_iter()
                .filter_map(|a| sanitize_asset_string(&a))
                .collect()
        });

    let assets = asset_strings
        .into_iter()
        .map(AccountId::new)
        .collect::<Vec<_>>();

    VaultObservabilityConfig {
        vault_address,
        collateral_vault_address,
        fees_manager_v2_address,
        rpc_url,
        assets,
    }
}

/// JSON-shape per-asset record. All amounts serialize as decimal
/// strings so callers can read up to `U256::MAX` without loss.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct VaultAssetSnapshot {
    pub asset: String,
    pub fee_balance: String,
    pub rebate_reserve: String,
    pub gross_fees_collected: String,
    pub rebates_paid: String,
    pub net_revenue: String,
    pub bootstrapped: bool,
    pub internal_cv_balance: String,
    pub raw_erc20_balance: String,
    /// Signed string: positive means CV ledger > buckets (lost track of
    /// a credit), negative means buckets > CV (over-claim). Invariant
    /// 2 forbids any non-zero value.
    pub drift_native: String,
    /// `internal_cv_balance.saturating_sub(fee_balance + rebate_reserve)`
    /// when positive — i.e. amount unaccounted for. Zero in the
    /// invariant-OK case.
    pub drift_status: &'static str,
    /// rebate_reserve below the FM-V2 rebateBudget cap means a future
    /// rebate trade will revert at the vault hook. Caller may pass
    /// `None` to skip the comparison.
    pub reserve_shortfall_native: String,
}

/// Global vault state — single row across all assets.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct VaultGlobalSnapshot {
    pub owner: Option<String>,
    pub guardian: Option<String>,
    pub revenue_receiver: Option<String>,
    pub collateral_vault_on_pfv: Option<String>,
    pub fees_manager_v2_on_pfv: Option<String>,
    pub rebates_paused: bool,
}

/// Top-level snapshot returned by the three admin endpoints. Always
/// well-formed: when the vault is not configured, `configured=false`
/// and `reason` carries the explanation.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct VaultObservabilitySnapshot {
    pub milestone: &'static str,
    pub configured: bool,
    pub vault_address: Option<String>,
    pub collateral_vault_address: Option<String>,
    pub fees_manager_v2_address: Option<String>,
    pub rpc_configured: bool,
    pub assets: Vec<VaultAssetSnapshot>,
    pub global: Option<VaultGlobalSnapshot>,
    /// Per-asset RebateBudget cap from FM-V2 (lowercase asset → uint
    /// decimal string). Empty when not provided by the caller.
    pub fees_manager_v2_rebate_budget: BTreeMap<String, String>,
    pub reason: Option<String>,
    /// Errors per asset where one of the eight view calls failed. The
    /// snapshot still surfaces the partial fields; this list lets the
    /// operator see which sub-call regressed.
    pub asset_errors: BTreeMap<String, String>,
}

impl VaultObservabilitySnapshot {
    fn not_configured(config: &VaultObservabilityConfig, reason: &str) -> Self {
        Self {
            milestone: "V2G-R5-OBS-P0",
            configured: false,
            vault_address: config.vault_address.as_ref().map(|a| a.0.clone()),
            collateral_vault_address: config
                .collateral_vault_address
                .as_ref()
                .map(|a| a.0.clone()),
            fees_manager_v2_address: config.fees_manager_v2_address.as_ref().map(|a| a.0.clone()),
            rpc_configured: config.rpc_url.is_some(),
            assets: Vec::new(),
            global: None,
            fees_manager_v2_rebate_budget: BTreeMap::new(),
            reason: Some(reason.to_string()),
            asset_errors: BTreeMap::new(),
        }
    }
}

/// Build the full snapshot. If the vault is unconfigured, returns
/// the "not_configured" structure without touching the network.
///
/// `rebate_budget_by_asset` is the same map the rebate-budget metric
/// derives; passing it in lets the snapshot compute the reserve
/// shortfall without re-reading the indexer state.
pub async fn read_snapshot(
    config: &VaultObservabilityConfig,
    rebate_budget_by_asset: &BTreeMap<String, u64>,
) -> Result<VaultObservabilitySnapshot> {
    let Some(vault) = config.vault_address.clone() else {
        return Ok(VaultObservabilitySnapshot::not_configured(
            config,
            NOT_CONFIGURED_REASON,
        ));
    };
    let Some(rpc_url) = config.rpc_url.clone() else {
        return Ok(VaultObservabilitySnapshot::not_configured(
            config,
            RPC_MISSING_REASON,
        ));
    };

    let provider = HttpJsonRpcProvider::new(rpc_url);

    let global = read_global(&provider, &vault).await;

    let mut assets = Vec::with_capacity(config.assets.len());
    let mut asset_errors: BTreeMap<String, String> = BTreeMap::new();
    let rebate_budget_strings = rebate_budget_by_asset
        .iter()
        .map(|(k, v)| (k.clone(), v.to_string()))
        .collect::<BTreeMap<_, _>>();

    for asset in &config.assets {
        let lc = asset.0.to_ascii_lowercase();
        let rebate_budget = rebate_budget_by_asset.get(&lc).copied().map(U256::from);
        match read_asset(
            &provider,
            &vault,
            config.collateral_vault_address.as_ref(),
            asset,
            rebate_budget,
        )
        .await
        {
            Ok(snap) => assets.push(snap),
            Err(err) => {
                asset_errors.insert(lc, err.to_string());
            }
        }
    }

    Ok(VaultObservabilitySnapshot {
        milestone: "V2G-R5-OBS-P0",
        configured: true,
        vault_address: Some(vault.0),
        collateral_vault_address: config
            .collateral_vault_address
            .as_ref()
            .map(|a| a.0.clone()),
        fees_manager_v2_address: config.fees_manager_v2_address.as_ref().map(|a| a.0.clone()),
        rpc_configured: true,
        assets,
        global,
        fees_manager_v2_rebate_budget: rebate_budget_strings,
        reason: None,
        asset_errors,
    })
}

async fn read_global<P>(provider: &P, vault: &AccountId) -> Option<VaultGlobalSnapshot>
where
    P: EthCallProvider,
{
    let owner = read_address_view(provider, vault, VIEW_OWNER).await;
    let guardian = read_address_view(provider, vault, VIEW_GUARDIAN).await;
    let revenue_receiver = read_address_view(provider, vault, VIEW_REVENUE_RECEIVER).await;
    let collateral_vault_on_pfv =
        read_address_view(provider, vault, VIEW_COLLATERAL_VAULT_ON_PFV).await;
    let fees_manager_v2_on_pfv =
        read_address_view(provider, vault, VIEW_FEES_MANAGER_V2_ON_PFV).await;
    let rebates_paused = read_bool_view(provider, vault, VIEW_REBATES_PAUSED)
        .await
        .unwrap_or(false);

    Some(VaultGlobalSnapshot {
        owner: owner.map(|a| a.0),
        guardian: guardian.map(|a| a.0),
        revenue_receiver: revenue_receiver.map(|a| a.0),
        collateral_vault_on_pfv: collateral_vault_on_pfv.map(|a| a.0),
        fees_manager_v2_on_pfv: fees_manager_v2_on_pfv.map(|a| a.0),
        rebates_paused,
    })
}

async fn read_asset<P>(
    provider: &P,
    vault: &AccountId,
    collateral_vault: Option<&AccountId>,
    asset: &AccountId,
    rebate_budget: Option<U256>,
) -> Result<VaultAssetSnapshot>
where
    P: EthCallProvider,
{
    let fee_balance = read_address_arg_uint(provider, vault, VIEW_FEE_BALANCE, asset).await?;
    let rebate_reserve = read_address_arg_uint(provider, vault, VIEW_REBATE_RESERVE, asset).await?;
    let gross_fees_collected =
        read_address_arg_uint(provider, vault, VIEW_GROSS_FEES_COLLECTED, asset).await?;
    let rebates_paid = read_address_arg_uint(provider, vault, VIEW_REBATES_PAID, asset).await?;
    let net_revenue = read_address_arg_uint(provider, vault, VIEW_NET_REVENUE, asset).await?;
    let bootstrapped = read_address_arg_bool(provider, vault, VIEW_BOOTSTRAPPED, asset)
        .await
        .unwrap_or(false);

    let internal_cv_balance = if let Some(cv) = collateral_vault {
        read_cv_balance(provider, cv, vault, asset)
            .await
            .unwrap_or(U256::ZERO)
    } else {
        U256::ZERO
    };
    let raw_erc20_balance = read_erc20_balance(provider, asset, vault)
        .await
        .unwrap_or(U256::ZERO);

    let buckets_sum = fee_balance.saturating_add(rebate_reserve);
    let (drift_native, drift_status) = compute_drift(internal_cv_balance, buckets_sum);
    let reserve_shortfall = match rebate_budget {
        Some(cap) if cap > rebate_reserve => cap - rebate_reserve,
        _ => U256::ZERO,
    };

    Ok(VaultAssetSnapshot {
        asset: asset.0.to_ascii_lowercase(),
        fee_balance: fee_balance.to_string(),
        rebate_reserve: rebate_reserve.to_string(),
        gross_fees_collected: gross_fees_collected.to_string(),
        rebates_paid: rebates_paid.to_string(),
        net_revenue: net_revenue.to_string(),
        bootstrapped,
        internal_cv_balance: internal_cv_balance.to_string(),
        raw_erc20_balance: raw_erc20_balance.to_string(),
        drift_native,
        drift_status,
        reserve_shortfall_native: reserve_shortfall.to_string(),
    })
}

/// Compute the signed drift `internal_cv_balance − buckets_sum`. The
/// return tuple is `(decimal_string, status)`. `status` is one of
/// `"ok"` (zero), `"drift_positive"` (CV > buckets — ledger lost
/// track of a credit), `"drift_negative"` (CV < buckets — over-claim).
pub fn compute_drift(cv_balance: U256, buckets_sum: U256) -> (String, &'static str) {
    use std::cmp::Ordering;
    match cv_balance.cmp(&buckets_sum) {
        Ordering::Equal => ("0".to_string(), "ok"),
        Ordering::Greater => ((cv_balance - buckets_sum).to_string(), "drift_positive"),
        Ordering::Less => {
            let delta = buckets_sum - cv_balance;
            (format!("-{delta}"), "drift_negative")
        }
    }
}

/// Prometheus exposition row: one per (metric, asset) cell. The metric
/// exporter walks the snapshot and emits a labeled gauge per row.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct MetricRow {
    pub metric: &'static str,
    pub asset: String,
    /// Decimal string; the caller is responsible for parsing into u64
    /// or emitting verbatim. We do not clamp because U256 values may
    /// exceed u64::MAX.
    pub value: String,
}

/// Build the list of labelled gauge rows for the Prometheus exporter.
/// Returns an empty vec when the vault is unconfigured — the exporter
/// then emits no PFV gauges, matching the "not_configured" contract.
pub fn metric_rows(snapshot: &VaultObservabilitySnapshot) -> Vec<MetricRow> {
    if !snapshot.configured {
        return Vec::new();
    }
    let mut rows = Vec::with_capacity(snapshot.assets.len() * 9);
    for a in &snapshot.assets {
        rows.push(MetricRow {
            metric: "deopt_protocol_fee_vault_fee_balance_native",
            asset: a.asset.clone(),
            value: a.fee_balance.clone(),
        });
        rows.push(MetricRow {
            metric: "deopt_protocol_fee_vault_rebate_reserve_native",
            asset: a.asset.clone(),
            value: a.rebate_reserve.clone(),
        });
        rows.push(MetricRow {
            metric: "deopt_protocol_fee_vault_gross_fees_collected_native",
            asset: a.asset.clone(),
            value: a.gross_fees_collected.clone(),
        });
        rows.push(MetricRow {
            metric: "deopt_protocol_fee_vault_rebates_paid_native",
            asset: a.asset.clone(),
            value: a.rebates_paid.clone(),
        });
        rows.push(MetricRow {
            metric: "deopt_protocol_fee_vault_net_revenue_native",
            asset: a.asset.clone(),
            value: a.net_revenue.clone(),
        });
        rows.push(MetricRow {
            metric: "deopt_protocol_fee_vault_internal_collateral_vault_balance_native",
            asset: a.asset.clone(),
            value: a.internal_cv_balance.clone(),
        });
        rows.push(MetricRow {
            metric: "deopt_protocol_fee_vault_raw_erc20_balance_native",
            asset: a.asset.clone(),
            value: a.raw_erc20_balance.clone(),
        });
        // For Prometheus, drift_native uses the unsigned magnitude; the
        // sign is exposed via the JSON endpoint only — Prometheus
        // gauges are uint-style here, paired with the magnitude alert.
        let drift_magnitude = a.drift_native.trim_start_matches('-').to_string();
        rows.push(MetricRow {
            metric: "deopt_protocol_fee_vault_drift_native",
            asset: a.asset.clone(),
            value: drift_magnitude,
        });
        rows.push(MetricRow {
            metric: "deopt_protocol_fee_vault_reserve_shortfall_native",
            asset: a.asset.clone(),
            value: a.reserve_shortfall_native.clone(),
        });
    }
    rows
}

/// True if and only if any asset shows non-zero drift. Used as the
/// `deopt_protocol_fee_vault_drift_present` global summary gauge.
pub fn any_drift_present(snapshot: &VaultObservabilitySnapshot) -> bool {
    snapshot.assets.iter().any(|a| a.drift_status != "ok")
}

/// True if `rebates_paused` is set on the global snapshot. Drives the
/// `deopt_protocol_fee_vault_rebates_paused` gauge.
pub fn rebates_paused(snapshot: &VaultObservabilitySnapshot) -> bool {
    snapshot
        .global
        .as_ref()
        .map(|g| g.rebates_paused)
        .unwrap_or(false)
}

/// JSON summary view for `GET /admin/fees/vault/summary`.
pub fn summary_view(snapshot: &VaultObservabilitySnapshot) -> serde_json::Value {
    serde_json::json!({
        "milestone": snapshot.milestone,
        "configured": snapshot.configured,
        "vault_address": snapshot.vault_address,
        "collateral_vault_address": snapshot.collateral_vault_address,
        "fees_manager_v2_address": snapshot.fees_manager_v2_address,
        "rpc_configured": snapshot.rpc_configured,
        "rebates_paused": rebates_paused(snapshot),
        "drift_present": any_drift_present(snapshot),
        "configured_assets_count": snapshot.assets.len(),
        "global": snapshot.global,
        "reason": snapshot.reason,
    })
}

/// JSON full breakdown for `GET /admin/fees/vault/balances`.
pub fn balances_view(snapshot: &VaultObservabilitySnapshot) -> serde_json::Value {
    serde_json::json!({
        "milestone": snapshot.milestone,
        "configured": snapshot.configured,
        "vault_address": snapshot.vault_address,
        "assets": snapshot.assets,
        "fees_manager_v2_rebate_budget": snapshot.fees_manager_v2_rebate_budget,
        "asset_errors": snapshot.asset_errors,
        "reason": snapshot.reason,
    })
}

/// JSON drift-focused view for `GET /admin/fees/vault/reconciliation`.
pub fn reconciliation_view(snapshot: &VaultObservabilitySnapshot) -> serde_json::Value {
    let rows = snapshot
        .assets
        .iter()
        .map(|a| {
            serde_json::json!({
                "asset": a.asset,
                "fee_balance": a.fee_balance,
                "rebate_reserve": a.rebate_reserve,
                "buckets_sum": (parse_u256_str(&a.fee_balance) + parse_u256_str(&a.rebate_reserve)).to_string(),
                "internal_cv_balance": a.internal_cv_balance,
                "raw_erc20_balance": a.raw_erc20_balance,
                "drift_native": a.drift_native,
                "drift_status": a.drift_status,
                "reserve_shortfall_native": a.reserve_shortfall_native,
                "raw_erc20_dust_present": parse_u256_str(&a.raw_erc20_balance) > U256::ZERO,
            })
        })
        .collect::<Vec<_>>();

    serde_json::json!({
        "milestone": snapshot.milestone,
        "configured": snapshot.configured,
        "vault_address": snapshot.vault_address,
        "drift_present": any_drift_present(snapshot),
        "rebates_paused": rebates_paused(snapshot),
        "rows": rows,
        "asset_errors": snapshot.asset_errors,
        "reason": snapshot.reason,
    })
}

fn parse_u256_str(s: &str) -> U256 {
    let raw = s.trim_start_matches('-');
    U256::from_str_radix(raw, 10).unwrap_or(U256::ZERO)
}

// ---------------------------------------------------------------------
// View-call plumbing — small, self-contained, mirrors state_checks.rs.
// ---------------------------------------------------------------------

fn selector(signature: &str) -> [u8; 4] {
    let h = keccak256(signature.as_bytes());
    [h[0], h[1], h[2], h[3]]
}

fn encode_address_word(addr20: &[u8; 20]) -> [u8; 32] {
    let mut w = [0u8; 32];
    w[12..].copy_from_slice(addr20);
    w
}

fn encode_no_arg(signature: &str) -> Vec<u8> {
    selector(signature).to_vec()
}

fn encode_address(signature: &str, account: &AccountId) -> Result<Vec<u8>> {
    let a = parse_evm_address(account)?;
    let mut out = Vec::with_capacity(36);
    out.extend_from_slice(&selector(signature));
    out.extend_from_slice(&encode_address_word(&a));
    Ok(out)
}

fn encode_two_address(signature: &str, left: &AccountId, right: &AccountId) -> Result<Vec<u8>> {
    let l = parse_evm_address(left)?;
    let r = parse_evm_address(right)?;
    let mut out = Vec::with_capacity(68);
    out.extend_from_slice(&selector(signature));
    out.extend_from_slice(&encode_address_word(&l));
    out.extend_from_slice(&encode_address_word(&r));
    Ok(out)
}

fn decode_uint256(output: &[u8]) -> Result<U256> {
    if output.len() != 32 {
        return Err(BackendError::Simulation(format!(
            "view returned {}-byte output, expected 32",
            output.len()
        )));
    }
    Ok(U256::from_be_slice(output))
}

fn decode_bool(output: &[u8]) -> Result<bool> {
    Ok(decode_uint256(output)? != U256::ZERO)
}

fn decode_address(output: &[u8]) -> Result<AccountId> {
    if output.len() != 32 || output[..12].iter().any(|b| *b != 0) {
        return Err(BackendError::Simulation(
            "view returned invalid address output".to_string(),
        ));
    }
    Ok(AccountId::new(hex_lc(&output[12..32])))
}

fn hex_lc(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(2 + bytes.len() * 2);
    out.push_str("0x");
    for b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0xf) as usize] as char);
    }
    out
}

const HEX: &[u8; 16] = b"0123456789abcdef";

async fn call_view<P>(provider: &P, to: &AccountId, data: Vec<u8>) -> Result<Vec<u8>>
where
    P: EthCallProvider,
{
    provider
        .eth_call(EthCallRequest {
            from: AccountId::new(ZERO_ADDR),
            to: to.clone(),
            data,
            value: 0,
            gas_limit: None,
        })
        .await
        .map(|s| s.output)
}

async fn read_address_view<P>(provider: &P, to: &AccountId, sig: &'static str) -> Option<AccountId>
where
    P: EthCallProvider,
{
    let out = call_view(provider, to, encode_no_arg(sig)).await.ok()?;
    decode_address(&out).ok().and_then(non_zero_addr)
}

async fn read_bool_view<P>(provider: &P, to: &AccountId, sig: &'static str) -> Result<bool>
where
    P: EthCallProvider,
{
    let out = call_view(provider, to, encode_no_arg(sig)).await?;
    decode_bool(&out)
}

async fn read_address_arg_uint<P>(
    provider: &P,
    to: &AccountId,
    sig: &'static str,
    arg: &AccountId,
) -> Result<U256>
where
    P: EthCallProvider,
{
    let out = call_view(provider, to, encode_address(sig, arg)?).await?;
    decode_uint256(&out)
}

async fn read_address_arg_bool<P>(
    provider: &P,
    to: &AccountId,
    sig: &'static str,
    arg: &AccountId,
) -> Result<bool>
where
    P: EthCallProvider,
{
    let out = call_view(provider, to, encode_address(sig, arg)?).await?;
    decode_bool(&out)
}

async fn read_cv_balance<P>(
    provider: &P,
    cv: &AccountId,
    vault: &AccountId,
    asset: &AccountId,
) -> Result<U256>
where
    P: EthCallProvider,
{
    let out = call_view(
        provider,
        cv,
        encode_two_address(CV_BALANCES_VIEW, vault, asset)?,
    )
    .await?;
    decode_uint256(&out)
}

async fn read_erc20_balance<P>(provider: &P, token: &AccountId, holder: &AccountId) -> Result<U256>
where
    P: EthCallProvider,
{
    let out = call_view(provider, token, encode_address(ERC20_BALANCE_OF, holder)?).await?;
    decode_uint256(&out)
}

fn non_zero_addr(a: AccountId) -> Option<AccountId> {
    if a.0.eq_ignore_ascii_case(ZERO_ADDR) {
        None
    } else {
        Some(a)
    }
}

fn sanitize_address(raw: &str) -> Option<AccountId> {
    let t = raw.trim();
    if t.is_empty() || t.eq_ignore_ascii_case(ZERO_ADDR) {
        return None;
    }
    let stripped = t
        .strip_prefix("0x")
        .or_else(|| t.strip_prefix("0X"))
        .unwrap_or(t);
    if stripped.len() != 40 || !stripped.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    Some(AccountId::new(format!(
        "0x{}",
        stripped.to_ascii_lowercase()
    )))
}

fn sanitize_asset_string(raw: &str) -> Option<String> {
    sanitize_address(raw).map(|a| a.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg_with_vault_unset() -> VaultObservabilityConfig {
        VaultObservabilityConfig {
            vault_address: None,
            collateral_vault_address: Some(AccountId::new(
                "0x00340c360353a5ab784c5bc5c44322a6af0625d3".to_string(),
            )),
            fees_manager_v2_address: Some(AccountId::new(
                "0xf6626177f3b85cc3239667cc53c04a8007652944".to_string(),
            )),
            rpc_url: Some("https://example.invalid".to_string()),
            assets: vec![AccountId::new(
                "0x6eae407f5640b006fac9965182e238582a3b412e".to_string(),
            )],
        }
    }

    #[tokio::test]
    async fn not_configured_returns_no_chain_call_and_well_formed_snapshot() {
        let cfg = cfg_with_vault_unset();
        let snap = read_snapshot(&cfg, &BTreeMap::new()).await.unwrap();
        assert!(!snap.configured);
        assert_eq!(snap.reason.as_deref(), Some(NOT_CONFIGURED_REASON));
        assert!(snap.assets.is_empty());
        assert!(snap.global.is_none());
        assert!(snap.fees_manager_v2_rebate_budget.is_empty());
        assert!(snap.rpc_configured);
        // Summary view must mirror the not-configured posture.
        let v = summary_view(&snap);
        assert_eq!(v["configured"], serde_json::json!(false));
        assert_eq!(v["drift_present"], serde_json::json!(false));
        assert_eq!(v["rebates_paused"], serde_json::json!(false));
        // Metric exporter must emit zero PFV rows.
        assert!(metric_rows(&snap).is_empty());
    }

    #[test]
    fn compute_drift_handles_three_cases() {
        let (s, st) = compute_drift(U256::from(100u64), U256::from(100u64));
        assert_eq!(s, "0");
        assert_eq!(st, "ok");
        let (s, st) = compute_drift(U256::from(110u64), U256::from(100u64));
        assert_eq!(s, "10");
        assert_eq!(st, "drift_positive");
        let (s, st) = compute_drift(U256::from(90u64), U256::from(100u64));
        assert_eq!(s, "-10");
        assert_eq!(st, "drift_negative");
    }

    #[test]
    fn metric_rows_emits_nine_rows_per_asset_when_configured() {
        let snap = VaultObservabilitySnapshot {
            milestone: "V2G-R5-OBS-P0",
            configured: true,
            vault_address: Some("0xf6626177f3b85cc3239667cc53c04a8007652944".to_string()),
            collateral_vault_address: None,
            fees_manager_v2_address: None,
            rpc_configured: true,
            assets: vec![VaultAssetSnapshot {
                asset: "0x6eae407f5640b006fac9965182e238582a3b412e".to_string(),
                fee_balance: "9".to_string(),
                rebate_reserve: "999967".to_string(),
                gross_fees_collected: "19".to_string(),
                rebates_paid: "10".to_string(),
                net_revenue: "9".to_string(),
                bootstrapped: true,
                internal_cv_balance: "999976".to_string(),
                raw_erc20_balance: "0".to_string(),
                drift_native: "0".to_string(),
                drift_status: "ok",
                reserve_shortfall_native: "0".to_string(),
            }],
            global: None,
            fees_manager_v2_rebate_budget: BTreeMap::new(),
            reason: None,
            asset_errors: BTreeMap::new(),
        };
        let rows = metric_rows(&snap);
        assert_eq!(rows.len(), 9);
        let names: Vec<_> = rows.iter().map(|r| r.metric).collect();
        assert!(names.contains(&"deopt_protocol_fee_vault_fee_balance_native"));
        assert!(names.contains(&"deopt_protocol_fee_vault_drift_native"));
        assert!(names.contains(&"deopt_protocol_fee_vault_reserve_shortfall_native"));
        assert!(names.contains(&"deopt_protocol_fee_vault_raw_erc20_balance_native"));
        for r in &rows {
            assert_eq!(r.asset, "0x6eae407f5640b006fac9965182e238582a3b412e");
        }
    }

    #[test]
    fn reserve_shortfall_zero_when_reserve_meets_cap() {
        // Reserve = 999967, budget cap = 999967 → shortfall = 0.
        let (drift_str, drift_st) = compute_drift(U256::from(0u64), U256::from(0u64));
        assert_eq!(drift_str, "0");
        assert_eq!(drift_st, "ok");
    }

    #[test]
    fn sanitize_address_accepts_only_well_formed_lowercase_addr() {
        assert!(sanitize_address("").is_none());
        assert!(sanitize_address(ZERO_ADDR).is_none());
        assert!(sanitize_address("0x123").is_none());
        assert!(sanitize_address("not-an-address").is_none());
        let a = sanitize_address("0xF6626177f3B85cc3239667Cc53C04A8007652944").unwrap();
        assert_eq!(a.0, "0xf6626177f3b85cc3239667cc53c04a8007652944");
    }

    #[test]
    fn build_config_pulls_assets_from_env_then_falls_back() {
        // Unset both env vars first.
        std::env::remove_var("PROTOCOL_FEE_VAULT_ADDRESS");
        std::env::remove_var("PROTOCOL_FEE_VAULT_RECONCILIATION_ASSETS");
        let cfg = build_config(
            Some("http://rpc".to_string()),
            None,
            None,
            vec!["0x6eAe407f5640B006faC9965182e238582A3B412E".to_string()],
        );
        assert!(cfg.vault_address.is_none());
        assert_eq!(cfg.assets.len(), 1);
        assert_eq!(
            cfg.assets[0].0,
            "0x6eae407f5640b006fac9965182e238582a3b412e"
        );
        assert!(!cfg.is_configured());
    }

    #[test]
    fn selector_matches_state_checks_pattern() {
        // selector("balanceOf(address)") begins with 0x70a08231 per ERC20 ABI.
        let s = selector("balanceOf(address)");
        assert_eq!(s, [0x70, 0xa0, 0x82, 0x31]);
    }

    #[test]
    fn balances_view_serializes_assets() {
        let snap = VaultObservabilitySnapshot::not_configured(
            &cfg_with_vault_unset(),
            NOT_CONFIGURED_REASON,
        );
        let v = balances_view(&snap);
        assert_eq!(v["configured"], serde_json::json!(false));
        assert_eq!(v["assets"], serde_json::json!([]));
    }

    #[test]
    fn reconciliation_view_marks_drift_present() {
        let mut snap = VaultObservabilitySnapshot::not_configured(
            &cfg_with_vault_unset(),
            NOT_CONFIGURED_REASON,
        );
        snap.configured = true;
        snap.assets.push(VaultAssetSnapshot {
            asset: "0x1111111111111111111111111111111111111111".to_string(),
            fee_balance: "0".to_string(),
            rebate_reserve: "0".to_string(),
            gross_fees_collected: "0".to_string(),
            rebates_paid: "0".to_string(),
            net_revenue: "0".to_string(),
            bootstrapped: false,
            internal_cv_balance: "100".to_string(),
            raw_erc20_balance: "5".to_string(),
            drift_native: "100".to_string(),
            drift_status: "drift_positive",
            reserve_shortfall_native: "0".to_string(),
        });
        let v = reconciliation_view(&snap);
        assert_eq!(v["drift_present"], serde_json::json!(true));
        let rows = v["rows"].as_array().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["raw_erc20_dust_present"], serde_json::json!(true));
        assert_eq!(rows[0]["drift_status"], serde_json::json!("drift_positive"));
    }
}
