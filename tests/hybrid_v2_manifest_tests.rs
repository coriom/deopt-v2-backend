//! BACKEND WP-01 manifest ingestion + validation tests.

use deopt_v2_backend::hybrid_v2::manifest::{
    ActivationStatus, ExpectedManifestIdentity, ManifestModuleAddresses, ManifestParams,
    ManifestValidationError, ManifestValidator, NetworkPolicy,
};

fn baseline_params() -> ManifestParams {
    ManifestParams {
        chain_id: 84532,
        manifest_address: "0x000000000000000000000000000000000000d001".into(),
        manifest_hash: "0x1111111111111111111111111111111111111111111111111111111111111111".into(),
        module_addresses_hash: "0x2222222222222222222222222222222222222222222222222222222222222222"
            .into(),
        critical_config_hash: "0x3333333333333333333333333333333333333333333333333333333333333333"
            .into(),
        architecture_version: 1,
        storage_version: 1,
        event_version: 1,
        deployment_version: 1,
        manifest_schema_version: 1,
        environment_tag: "0x6c6f63616c00000000000000000000000000000000000000000000000000000".into(),
        deployer: "0x000000000000000000000000000000000000dead".into(),
        deployment_block: 100,
        deployment_timestamp: 1_700_000_000,
        module_addresses: ManifestModuleAddresses {
            subaccount_registry: "0x0000000000000000000000000000000000000001".into(),
            collateral_vault: "0x0000000000000000000000000000000000000002".into(),
            options_positions_ledger: "0x0000000000000000000000000000000000000003".into(),
            risk_module: "0x0000000000000000000000000000000000000004".into(),
            margin_engine: "0x0000000000000000000000000000000000000005".into(),
            option_matching_engine: "0x0000000000000000000000000000000000000006".into(),
            escape_controller: "0x0000000000000000000000000000000000000007".into(),
            recovery_finalizer: "0x0000000000000000000000000000000000000008".into(),
            oracle_adapter: "0x0000000000000000000000000000000000000009".into(),
            options_risk_provider: "0x000000000000000000000000000000000000000a".into(),
            quote_token: "0x000000000000000000000000000000000000000b".into(),
            fees_manager_v2: None,
            option_execution_fee_adapter: None,
            protocol_timelock: None,
            governance: Some("0x00000000000000000000000000000000000000a1".into()),
            guardian: Some("0x00000000000000000000000000000000000000a2".into()),
        },
        protocol_fee_subkey: "0xaa00000000000000000000000000000000000000000000000000000000000001"
            .into(),
        rebate_budget_subkey: "0xaa00000000000000000000000000000000000000000000000000000000000002"
            .into(),
        insurance_fund_subkey: "0xaa00000000000000000000000000000000000000000000000000000000000003"
            .into(),
        max_collateral_tokens: 8,
        max_active_series: 32,
        all_capabilities_mask: "65535".into(),
        recovery_activation_delay_seconds: 3600,
        recovery_pause_max_duration_blocks: 100_800,
        activation_status: ActivationStatus::Active,
    }
}

fn base_sepolia_validator() -> ManifestValidator {
    ManifestValidator::new(
        NetworkPolicy::BaseSepoliaOnly,
        1,
        1,
        1,
        1,
        ExpectedManifestIdentity::default(),
    )
}

#[test]
fn valid_base_sepolia_manifest_accepted() {
    let out = base_sepolia_validator()
        .validate(baseline_params())
        .expect("valid Base Sepolia manifest must be accepted");
    assert_eq!(out.chain_id, 84532);
}

#[test]
fn base_mainnet_rejected_outright() {
    let mut p = baseline_params();
    p.chain_id = 8453;
    let err = base_sepolia_validator().validate(p).unwrap_err();
    assert!(matches!(err, ManifestValidationError::BaseMainnetForbidden));
}

#[test]
fn local_chain_rejected_by_base_sepolia_policy() {
    let mut p = baseline_params();
    p.chain_id = 31337;
    let err = base_sepolia_validator().validate(p).unwrap_err();
    assert!(matches!(
        err,
        ManifestValidationError::ChainIdRejectedByPolicy { .. }
    ));
}

#[test]
fn local_chain_accepted_by_local_only_policy() {
    let v = ManifestValidator::new(
        NetworkPolicy::LocalTestOnly,
        1,
        1,
        1,
        1,
        ExpectedManifestIdentity::default(),
    );
    let mut p = baseline_params();
    p.chain_id = 31337;
    v.validate(p)
        .expect("local chain permitted under LocalTestOnly");
}

#[test]
fn not_deployed_template_rejected() {
    let mut p = baseline_params();
    p.activation_status = ActivationStatus::NotDeployed;
    let err = base_sepolia_validator().validate(p).unwrap_err();
    assert!(matches!(err, ManifestValidationError::TemplateNotLive));
}

#[test]
fn zero_deployment_version_rejected() {
    let mut p = baseline_params();
    p.deployment_version = 0;
    let err = base_sepolia_validator().validate(p).unwrap_err();
    assert!(matches!(
        err,
        ManifestValidationError::DeploymentVersionZero
    ));
}

#[test]
fn max_collateral_tokens_mismatch_rejected() {
    let mut p = baseline_params();
    p.max_collateral_tokens = 4;
    let err = base_sepolia_validator().validate(p).unwrap_err();
    assert!(matches!(
        err,
        ManifestValidationError::MaxCollateralTokensMismatch { actual: 4 }
    ));
}

#[test]
fn max_active_series_mismatch_rejected() {
    let mut p = baseline_params();
    p.max_active_series = 8;
    let err = base_sepolia_validator().validate(p).unwrap_err();
    assert!(matches!(
        err,
        ManifestValidationError::MaxActiveSeriesMismatch { actual: 8 }
    ));
}

#[test]
fn zero_required_module_address_rejected() {
    let mut p = baseline_params();
    p.module_addresses.subaccount_registry = "0x0000000000000000000000000000000000000000".into();
    let err = base_sepolia_validator().validate(p).unwrap_err();
    assert!(matches!(
        err,
        ManifestValidationError::RequiredModuleZero {
            slot: "subaccount_registry"
        }
    ));
}

#[test]
fn zero_protocol_fee_subkey_rejected() {
    let mut p = baseline_params();
    p.protocol_fee_subkey =
        "0x0000000000000000000000000000000000000000000000000000000000000000".into();
    let err = base_sepolia_validator().validate(p).unwrap_err();
    assert!(matches!(
        err,
        ManifestValidationError::ProtocolSubKeyZero { .. }
    ));
}

#[test]
fn optional_pinned_manifest_hash_enforced() {
    let expected = "0xdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef";
    let v = ManifestValidator::new(
        NetworkPolicy::BaseSepoliaOnly,
        1,
        1,
        1,
        1,
        ExpectedManifestIdentity {
            expected_manifest_hash: Some(expected.into()),
            ..Default::default()
        },
    );
    let err = v.validate(baseline_params()).unwrap_err();
    assert!(matches!(
        err,
        ManifestValidationError::ManifestHashMismatch { .. }
    ));
}

#[test]
fn version_mismatch_rejected() {
    let mut p = baseline_params();
    p.architecture_version = 2;
    let err = base_sepolia_validator().validate(p).unwrap_err();
    assert!(matches!(
        err,
        ManifestValidationError::ArchitectureVersionMismatch {
            expected: 1,
            actual: 2
        }
    ));
}
