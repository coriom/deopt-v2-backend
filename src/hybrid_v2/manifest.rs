//! Deployment manifest ingestion + validation.
//!
//! Two source shapes are supported (per Part D):
//! 1. `ManifestParams` — the exact struct written on-chain by
//!    `DeploymentManifestV1`. Ingested from an RPC call or a tracked
//!    JSON manifest.
//! 2. `NOT_DEPLOYED` template (from
//!    `resources/hybrid-v2/base-sepolia-template-v1.json`) —
//!    ALWAYS rejected as a live deployment. The template is a schema
//!    reference, not a live identity.
//!
//! Frozen safety:
//! - Base mainnet (`chain_id == 8453`) is rejected outright.
//! - Base Sepolia (`chain_id == 84532`) is accepted.
//! - Local dev chain IDs (31337, 1337) are accepted ONLY under the
//!   `NetworkPolicy::LocalTestOnly` policy — never silently coerced
//!   into a public network identity.
//! - The manifest hash MUST match `expected_manifest_hash` when
//!   supplied. Every ingestion path recomputes the expected value from
//!   the pinned Solidity surface where possible.

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const BASE_MAINNET_CHAIN_ID: u64 = 8453;
pub const BASE_SEPOLIA_CHAIN_ID: u64 = 84532;
pub const LOCAL_ANVIL_CHAIN_ID: u64 = 31337;
pub const LOCAL_HARDHAT_CHAIN_ID: u64 = 1337;

/// Which chain IDs a manifest source is allowed to declare.
///
/// This is a deployment-time policy — the ingestion module never
/// silently changes it. A test fixture MUST use `LocalTestOnly`; a
/// public testnet manifest MUST use `PublicTestnet`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkPolicy {
    /// Only 31337 / 1337 accepted. For local + integration tests.
    LocalTestOnly,
    /// Base Sepolia only (84532). Explicitly rejects local chain IDs
    /// to prevent silent coercion.
    BaseSepoliaOnly,
    /// Any public testnet; Base mainnet still rejected.
    PublicTestnet,
}

impl NetworkPolicy {
    pub fn permits(&self, chain_id: u64) -> bool {
        if chain_id == BASE_MAINNET_CHAIN_ID {
            return false;
        }
        match self {
            NetworkPolicy::LocalTestOnly => {
                chain_id == LOCAL_ANVIL_CHAIN_ID || chain_id == LOCAL_HARDHAT_CHAIN_ID
            }
            NetworkPolicy::BaseSepoliaOnly => chain_id == BASE_SEPOLIA_CHAIN_ID,
            NetworkPolicy::PublicTestnet => chain_id != BASE_MAINNET_CHAIN_ID,
        }
    }
}

/// Exact parameter set observed from `DeploymentManifestV1`. Field
/// names mirror the Solidity struct 1:1 so the ingestion code is
/// audit-friendly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestParams {
    pub chain_id: u64,
    pub manifest_address: String,
    pub manifest_hash: String,
    pub module_addresses_hash: String,
    pub critical_config_hash: String,
    pub architecture_version: u16,
    pub storage_version: u16,
    pub event_version: u16,
    pub deployment_version: u16,
    pub manifest_schema_version: u16,
    pub environment_tag: String,
    pub deployer: String,
    pub deployment_block: u64,
    pub deployment_timestamp: u64,
    pub module_addresses: ManifestModuleAddresses,
    pub protocol_fee_subkey: String,
    pub rebate_budget_subkey: String,
    pub insurance_fund_subkey: String,
    pub max_collateral_tokens: u16,
    pub max_active_series: u32,
    pub all_capabilities_mask: String,
    pub recovery_activation_delay_seconds: u64,
    pub recovery_pause_max_duration_blocks: u64,
    #[serde(default)]
    pub activation_status: ActivationStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestModuleAddresses {
    pub subaccount_registry: String,
    pub collateral_vault: String,
    pub options_positions_ledger: String,
    pub risk_module: String,
    pub margin_engine: String,
    pub option_matching_engine: String,
    pub escape_controller: String,
    pub recovery_finalizer: String,
    pub oracle_adapter: String,
    pub options_risk_provider: String,
    pub quote_token: String,
    pub fees_manager_v2: Option<String>,
    pub option_execution_fee_adapter: Option<String>,
    pub protocol_timelock: Option<String>,
    pub governance: Option<String>,
    pub guardian: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ActivationStatus {
    NotDeployed,
    Proposed,
    #[default]
    Pending,
    Active,
    Sunset,
}

#[derive(Debug, Error)]
pub enum ManifestValidationError {
    #[error("Base mainnet chain id 8453 is not authorised for HybridV2 experimental manifests")]
    BaseMainnetForbidden,
    #[error("chain id {chain_id} is not permitted by policy {policy:?}")]
    ChainIdRejectedByPolicy {
        chain_id: u64,
        policy: NetworkPolicy,
    },
    #[error("manifest status is NOT_DEPLOYED — template cannot be ingested as a live deployment")]
    TemplateNotLive,
    #[error("architecture version {actual} != expected {expected}")]
    ArchitectureVersionMismatch { expected: u16, actual: u16 },
    #[error("storage version {actual} != expected {expected}")]
    StorageVersionMismatch { expected: u16, actual: u16 },
    #[error("event version {actual} != expected {expected}")]
    EventVersionMismatch { expected: u16, actual: u16 },
    #[error("manifest schema version {actual} != expected {expected}")]
    ManifestSchemaVersionMismatch { expected: u16, actual: u16 },
    #[error("deployment version is zero")]
    DeploymentVersionZero,
    #[error("max collateral tokens {actual} != frozen 8")]
    MaxCollateralTokensMismatch { actual: u16 },
    #[error("max active series {actual} != frozen 32")]
    MaxActiveSeriesMismatch { actual: u32 },
    #[error("required module {slot} address is zero")]
    RequiredModuleZero { slot: &'static str },
    #[error("expected manifest hash {expected} but ingested {actual}")]
    ManifestHashMismatch { expected: String, actual: String },
    #[error("expected module addresses hash {expected} but ingested {actual}")]
    ModuleAddressesHashMismatch { expected: String, actual: String },
    #[error("expected critical config hash {expected} but ingested {actual}")]
    CriticalConfigHashMismatch { expected: String, actual: String },
    #[error("protocol subaccount subkey is zero: {which}")]
    ProtocolSubKeyZero { which: &'static str },
    #[error("json decoding failed: {0}")]
    JsonError(#[from] serde_json::Error),
}

/// Configuration-provided expected identity — the pinned commit's
/// architecture/storage/event/manifest-schema versions are always known
/// at compile time (via `PinnedSnapshots::parse_source_metadata`).
///
/// Optional hashes let a strict deployment pin the manifest-hash /
/// addresses-hash / critical-config-hash before ingestion; when
/// omitted the validator only enforces the version block.
#[derive(Debug, Clone, Default)]
pub struct ExpectedManifestIdentity {
    pub expected_manifest_hash: Option<String>,
    pub expected_module_addresses_hash: Option<String>,
    pub expected_critical_config_hash: Option<String>,
}

pub struct ManifestValidator {
    policy: NetworkPolicy,
    expected_architecture_version: u16,
    expected_storage_version: u16,
    expected_event_version: u16,
    expected_manifest_schema_version: u16,
    expected_identity: ExpectedManifestIdentity,
}

impl ManifestValidator {
    pub fn new(
        policy: NetworkPolicy,
        expected_architecture_version: u16,
        expected_storage_version: u16,
        expected_event_version: u16,
        expected_manifest_schema_version: u16,
        expected_identity: ExpectedManifestIdentity,
    ) -> Self {
        Self {
            policy,
            expected_architecture_version,
            expected_storage_version,
            expected_event_version,
            expected_manifest_schema_version,
            expected_identity,
        }
    }

    /// Validate + coerce a manifest into an ingestable form. Rejects
    /// every failure mode listed in Part D.
    pub fn validate(
        &self,
        params: ManifestParams,
    ) -> Result<ManifestParams, ManifestValidationError> {
        // 1. Chain id and network policy.
        if params.chain_id == BASE_MAINNET_CHAIN_ID {
            return Err(ManifestValidationError::BaseMainnetForbidden);
        }
        if !self.policy.permits(params.chain_id) {
            return Err(ManifestValidationError::ChainIdRejectedByPolicy {
                chain_id: params.chain_id,
                policy: self.policy,
            });
        }

        // 2. Template rejection.
        if params.activation_status == ActivationStatus::NotDeployed {
            return Err(ManifestValidationError::TemplateNotLive);
        }

        // 3. Version block.
        if params.architecture_version != self.expected_architecture_version {
            return Err(ManifestValidationError::ArchitectureVersionMismatch {
                expected: self.expected_architecture_version,
                actual: params.architecture_version,
            });
        }
        if params.storage_version != self.expected_storage_version {
            return Err(ManifestValidationError::StorageVersionMismatch {
                expected: self.expected_storage_version,
                actual: params.storage_version,
            });
        }
        if params.event_version != self.expected_event_version {
            return Err(ManifestValidationError::EventVersionMismatch {
                expected: self.expected_event_version,
                actual: params.event_version,
            });
        }
        if params.manifest_schema_version != self.expected_manifest_schema_version {
            return Err(ManifestValidationError::ManifestSchemaVersionMismatch {
                expected: self.expected_manifest_schema_version,
                actual: params.manifest_schema_version,
            });
        }
        if params.deployment_version == 0 {
            return Err(ManifestValidationError::DeploymentVersionZero);
        }

        // 4. Frozen bounds.
        if params.max_collateral_tokens != 8 {
            return Err(ManifestValidationError::MaxCollateralTokensMismatch {
                actual: params.max_collateral_tokens,
            });
        }
        if params.max_active_series != 32 {
            return Err(ManifestValidationError::MaxActiveSeriesMismatch {
                actual: params.max_active_series,
            });
        }

        // 5. Required addresses non-zero. String-shape is trusted (the
        // Solidity manifest constructor already rejects zero addresses
        // per WP-11); we mirror the check defensively.
        let modules = &params.module_addresses;
        require_nonzero_addr("subaccount_registry", &modules.subaccount_registry)?;
        require_nonzero_addr("collateral_vault", &modules.collateral_vault)?;
        require_nonzero_addr(
            "options_positions_ledger",
            &modules.options_positions_ledger,
        )?;
        require_nonzero_addr("risk_module", &modules.risk_module)?;
        require_nonzero_addr("margin_engine", &modules.margin_engine)?;
        require_nonzero_addr("option_matching_engine", &modules.option_matching_engine)?;
        require_nonzero_addr("escape_controller", &modules.escape_controller)?;
        require_nonzero_addr("recovery_finalizer", &modules.recovery_finalizer)?;
        require_nonzero_addr("oracle_adapter", &modules.oracle_adapter)?;
        require_nonzero_addr("options_risk_provider", &modules.options_risk_provider)?;
        require_nonzero_addr("quote_token", &modules.quote_token)?;

        // 6. Protocol subKeys.
        require_nonzero_bytes32("protocol_fee_subkey", &params.protocol_fee_subkey)?;
        require_nonzero_bytes32("rebate_budget_subkey", &params.rebate_budget_subkey)?;
        require_nonzero_bytes32("insurance_fund_subkey", &params.insurance_fund_subkey)?;

        // 7. Optional identity pins.
        if let Some(expected) = &self.expected_identity.expected_manifest_hash {
            if !hash_eq(expected, &params.manifest_hash) {
                return Err(ManifestValidationError::ManifestHashMismatch {
                    expected: expected.clone(),
                    actual: params.manifest_hash.clone(),
                });
            }
        }
        if let Some(expected) = &self.expected_identity.expected_module_addresses_hash {
            if !hash_eq(expected, &params.module_addresses_hash) {
                return Err(ManifestValidationError::ModuleAddressesHashMismatch {
                    expected: expected.clone(),
                    actual: params.module_addresses_hash.clone(),
                });
            }
        }
        if let Some(expected) = &self.expected_identity.expected_critical_config_hash {
            if !hash_eq(expected, &params.critical_config_hash) {
                return Err(ManifestValidationError::CriticalConfigHashMismatch {
                    expected: expected.clone(),
                    actual: params.critical_config_hash.clone(),
                });
            }
        }

        Ok(params)
    }
}

fn require_nonzero_addr(slot: &'static str, addr: &str) -> Result<(), ManifestValidationError> {
    let stripped = addr.strip_prefix("0x").unwrap_or(addr);
    if stripped.chars().all(|c| c == '0') {
        return Err(ManifestValidationError::RequiredModuleZero { slot });
    }
    Ok(())
}

fn require_nonzero_bytes32(which: &'static str, hex: &str) -> Result<(), ManifestValidationError> {
    let stripped = hex.strip_prefix("0x").unwrap_or(hex);
    if stripped.chars().all(|c| c == '0') {
        return Err(ManifestValidationError::ProtocolSubKeyZero { which });
    }
    Ok(())
}

fn hash_eq(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}
