//! Allowlist of `(chain_id, target_address, selector)` triples that
//! the Hybrid V2 execution orchestrator is permitted to touch.
//!
//! Frozen safety:
//! - `BASE_MAINNET_8453_IS_FORBIDDEN` — every access path rejects
//!   chain_id 8453 outright.
//! - The policy is DERIVED from the validated deployment manifest,
//!   not user-supplied. A malicious caller cannot smuggle a new
//!   target through by mutating the request row: the plan builder
//!   re-checks against the manifest, and the policy is the second
//!   independent guard.
//! - Only OptionMatchingEngineV2's `executeMatch` is enrolled at
//!   this milestone. Adding a second target/selector requires a
//!   docs update in the audit surface + a corresponding entry here.

use crate::hybrid_v2::execution::plan::executeMatchCall;
use crate::hybrid_v2::manifest::{ManifestParams, BASE_MAINNET_CHAIN_ID};
use alloy_primitives::U256;
use alloy_sol_types::SolCall;
use thiserror::Error;

/// A single allowlist entry — one `(chain_id, target, selector)`
/// triple with the ABI-input-type manifest for future documentation
/// generation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AllowedTarget {
    pub deployment_version: String,
    pub contract_name: String,
    pub address: [u8; 20],
    pub selector: [u8; 4],
    pub abi_input_types: Vec<String>,
    /// The value in wei that the plan is allowed to carry. For
    /// executeMatch this is `U256::ZERO` — any nonzero value fails
    /// the `ValueDisallowed` check in the plan builder.
    pub allowed_value_wei: U256,
}

/// Deployment-scoped allowlist. Constructed via
/// [`TargetPolicy::from_manifest`]. Reads are lock-free (owned Vec).
#[derive(Debug, Clone, Default)]
pub struct TargetPolicy {
    entries: Vec<AllowedTarget>,
    chain_id: u64,
}

/// Structured rejection reasons. Every variant is user-actionable
/// (i.e. surfaced to admin logs but not to end-users).
#[derive(Debug, Error, PartialEq, Eq)]
pub enum PolicyError {
    #[error("Base mainnet chain id {0} is forbidden for Hybrid V2 execution")]
    BaseMainnetForbidden(u64),
    #[error("chain id {actual} does not match the policy's chain id {expected}")]
    ChainMismatch { expected: u64, actual: u64 },
    #[error("target address {target_hex} is not in the allowlist")]
    UnknownTarget { target_hex: String },
    #[error("selector {selector_hex} is not allowed on target {target_hex}")]
    UnknownSelector {
        target_hex: String,
        selector_hex: String,
    },
    #[error("manifest is missing the OptionMatchingEngineV2 address")]
    ManifestMissingEngine,
    #[error("manifest declares an unsupported deployment version {version}")]
    UnknownDeploymentVersion { version: String },
    #[error("manifest option_matching_engine address unparseable: {0}")]
    UnparseableAddress(String),
}

impl TargetPolicy {
    /// Build a policy from a validated deployment manifest. The
    /// manifest MUST have already passed
    /// [`crate::hybrid_v2::manifest::ManifestValidator::validate`]
    /// — this function performs only the target-enrollment.
    pub fn from_manifest(manifest: &ManifestParams) -> Result<Self, PolicyError> {
        if manifest.chain_id == BASE_MAINNET_CHAIN_ID {
            return Err(PolicyError::BaseMainnetForbidden(manifest.chain_id));
        }
        let engine_hex = &manifest.module_addresses.option_matching_engine;
        let address = parse_address_bytes(engine_hex).map_err(PolicyError::UnparseableAddress)?;
        if address == [0u8; 20] {
            return Err(PolicyError::ManifestMissingEngine);
        }
        let entry = AllowedTarget {
            // The manifest tracks a numeric `deployment_version`; we
            // stringify it here so future policies can co-exist by
            // string discriminator (e.g. "hybrid_v2_options_v1").
            deployment_version: format!("hybrid_v2_options_v{}", manifest.deployment_version),
            contract_name: "OptionMatchingEngineV2".to_string(),
            address,
            selector: executeMatchCall::SELECTOR,
            abi_input_types: vec![
                "SignedActionEnvelope".into(),
                "bytes".into(),
                "OptionOrder".into(),
                "SignedActionEnvelope".into(),
                "bytes".into(),
                "OptionOrder".into(),
                "uint128".into(),
                "uint256[]".into(),
                "uint256[]".into(),
            ],
            allowed_value_wei: U256::ZERO,
        };
        Ok(TargetPolicy {
            entries: vec![entry],
            chain_id: manifest.chain_id,
        })
    }

    /// Static-only refusal for Base mainnet. Callable without a
    /// manifest so callers can gate ingress before construction.
    pub fn refuse_base_mainnet(chain_id: u64) -> Result<(), PolicyError> {
        if chain_id == BASE_MAINNET_CHAIN_ID {
            return Err(PolicyError::BaseMainnetForbidden(chain_id));
        }
        Ok(())
    }

    /// Look up the allowlist entry matching the given
    /// `(chain_id, target, selector)` triple. Rejects Base mainnet,
    /// chain mismatch, unknown target, wrong selector, or an
    /// unrelated selector on a known target.
    pub fn is_allowed(
        &self,
        chain_id: u64,
        target: &[u8; 20],
        selector: &[u8; 4],
    ) -> Result<&AllowedTarget, PolicyError> {
        Self::refuse_base_mainnet(chain_id)?;
        if chain_id != self.chain_id {
            return Err(PolicyError::ChainMismatch {
                expected: self.chain_id,
                actual: chain_id,
            });
        }
        let target_hex = to_hex_lowercase_prefixed(target);
        let Some(entry_for_target) = self.entries.iter().find(|e| &e.address == target) else {
            return Err(PolicyError::UnknownTarget { target_hex });
        };
        if &entry_for_target.selector != selector {
            return Err(PolicyError::UnknownSelector {
                target_hex,
                selector_hex: to_hex_lowercase_prefixed(selector),
            });
        }
        Ok(entry_for_target)
    }

    /// Read-only accessor for downstream diagnostics.
    pub fn entries(&self) -> &[AllowedTarget] {
        &self.entries
    }

    pub fn chain_id(&self) -> u64 {
        self.chain_id
    }
}

// -----------------------------------------------------------------
//                          HELPERS
// -----------------------------------------------------------------

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

// -----------------------------------------------------------------
//                          UNIT TESTS
// -----------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hybrid_v2::manifest::{ActivationStatus, ManifestModuleAddresses};

    const ENGINE_HEX: &str = "0x5a5EBF9A9CCd7c012518569DE8283982982670f6";

    fn manifest(chain_id: u64) -> ManifestParams {
        ManifestParams {
            chain_id,
            manifest_address: format!("0x{}", "01".repeat(20)),
            manifest_hash: format!("0x{}", "11".repeat(32)),
            module_addresses_hash: format!("0x{}", "22".repeat(32)),
            critical_config_hash: format!("0x{}", "33".repeat(32)),
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
                option_matching_engine: ENGINE_HEX.to_string(),
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
            protocol_fee_subkey: format!("0x{}", "01".repeat(32)),
            rebate_budget_subkey: format!("0x{}", "02".repeat(32)),
            insurance_fund_subkey: format!("0x{}", "03".repeat(32)),
            max_collateral_tokens: 8,
            max_active_series: 32,
            all_capabilities_mask: format!("0x{}", "ff".repeat(32)),
            recovery_activation_delay_seconds: 3600,
            recovery_pause_max_duration_blocks: 1000,
            activation_status: ActivationStatus::Active,
        }
    }

    fn engine_address() -> [u8; 20] {
        parse_address_bytes(ENGINE_HEX).unwrap()
    }

    #[test]
    fn refuse_base_mainnet_bare() {
        assert!(matches!(
            TargetPolicy::refuse_base_mainnet(BASE_MAINNET_CHAIN_ID),
            Err(PolicyError::BaseMainnetForbidden(_))
        ));
        assert!(TargetPolicy::refuse_base_mainnet(84532).is_ok());
    }

    #[test]
    fn manifest_construction_rejects_base_mainnet() {
        let err = TargetPolicy::from_manifest(&manifest(BASE_MAINNET_CHAIN_ID)).unwrap_err();
        assert!(matches!(err, PolicyError::BaseMainnetForbidden(_)));
    }

    #[test]
    fn happy_path_admits_engine_and_selector() {
        let policy = TargetPolicy::from_manifest(&manifest(84532)).unwrap();
        let entry = policy
            .is_allowed(84532, &engine_address(), &executeMatchCall::SELECTOR)
            .unwrap();
        assert_eq!(entry.contract_name, "OptionMatchingEngineV2");
        assert_eq!(entry.allowed_value_wei, U256::ZERO);
    }

    #[test]
    fn rejects_unknown_target() {
        let policy = TargetPolicy::from_manifest(&manifest(84532)).unwrap();
        let err = policy
            .is_allowed(84532, &[0xccu8; 20], &executeMatchCall::SELECTOR)
            .unwrap_err();
        assert!(matches!(err, PolicyError::UnknownTarget { .. }));
    }

    #[test]
    fn rejects_unknown_selector_on_known_target() {
        let policy = TargetPolicy::from_manifest(&manifest(84532)).unwrap();
        let err = policy
            .is_allowed(84532, &engine_address(), &[0xde, 0xad, 0xbe, 0xef])
            .unwrap_err();
        assert!(matches!(err, PolicyError::UnknownSelector { .. }));
    }

    #[test]
    fn rejects_chain_mismatch() {
        let policy = TargetPolicy::from_manifest(&manifest(84532)).unwrap();
        let err = policy
            .is_allowed(11155111, &engine_address(), &executeMatchCall::SELECTOR)
            .unwrap_err();
        assert!(matches!(err, PolicyError::ChainMismatch { .. }));
    }

    #[test]
    fn base_mainnet_is_rejected_via_is_allowed_too() {
        let policy = TargetPolicy::from_manifest(&manifest(84532)).unwrap();
        let err = policy
            .is_allowed(
                BASE_MAINNET_CHAIN_ID,
                &engine_address(),
                &executeMatchCall::SELECTOR,
            )
            .unwrap_err();
        assert!(matches!(err, PolicyError::BaseMainnetForbidden(_)));
    }

    #[test]
    fn rejects_missing_engine_address() {
        let mut m = manifest(84532);
        m.module_addresses.option_matching_engine = format!("0x{}", "00".repeat(20));
        let err = TargetPolicy::from_manifest(&m).unwrap_err();
        assert!(matches!(err, PolicyError::ManifestMissingEngine));
    }
}
