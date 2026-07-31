//! Test fixtures shared across the runtime / rebuild / reconciliation
//! / repository suites.

use deopt_v2_backend::hybrid_v2::chain_source::RawBlock;
use deopt_v2_backend::hybrid_v2::decoder::{build_raw_log, pack_data, CanonicalRawLog, PackField};
use deopt_v2_backend::hybrid_v2::manifest::{
    ActivationStatus, ManifestModuleAddresses, ManifestParams,
};
use deopt_v2_backend::hybrid_v2::topics::TopicCatalogue;

pub fn baseline_manifest(chain_id: u64) -> ManifestParams {
    ManifestParams {
        chain_id,
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

pub fn block(
    number: u64,
    hash: &str,
    parent: &str,
    ts: u64,
    logs: Vec<CanonicalRawLog>,
) -> RawBlock {
    RawBlock {
        number,
        hash: hash.into(),
        parent_hash: parent.into(),
        timestamp: ts,
        logs,
    }
}

fn topic_hex(event: &str) -> String {
    TopicCatalogue::get()
        .lookup_by_event(event)
        .unwrap_or_else(|| panic!("missing event {}", event))
        .topic0_hex_lower
        .to_string()
}

pub fn pad_bytes32(s: &str) -> String {
    let stripped = s.trim_start_matches("0x");
    let mut out = String::from("0x");
    for _ in stripped.len()..64 {
        out.push('0');
    }
    out.push_str(stripped);
    out
}

pub fn pad_address(a: &str) -> String {
    let stripped = a.trim_start_matches("0x");
    let mut s = String::from("0x");
    for _ in stripped.len()..40 {
        s.push('0');
    }
    s.push_str(stripped);
    s.to_ascii_lowercase()
}

fn addr_topic(a: &str) -> String {
    let stripped = a.trim_start_matches("0x");
    let mut s = String::from("0x");
    for _ in 0..24 {
        s.push('0');
    }
    // Pad address to 20 bytes / 40 hex chars.
    let mut hex = String::new();
    for _ in stripped.len()..40 {
        hex.push('0');
    }
    hex.push_str(stripped);
    s.push_str(&hex[hex.len() - 40..]);
    s
}

fn sid_topic(id: u32) -> String {
    let mut s = String::from("0x");
    for _ in 0..56 {
        s.push('0');
    }
    for b in id.to_be_bytes() {
        s.push_str(&format!("{:02x}", b));
    }
    s
}

fn pack_hex(fields: &[PackField]) -> String {
    let data = pack_data(fields);
    let mut out = String::from("0x");
    for b in data {
        out.push_str(&format!("{:02x}", b));
    }
    out
}

pub fn subaccount_created_log(
    m: &ManifestParams,
    owner: &str,
    id: u32,
    subkey: &str,
) -> CanonicalRawLog {
    let t = topic_hex("SubaccountCreated");
    let data = pack_hex(&[PackField::U256("0"), PackField::U16(1)]);
    build_raw_log(
        &m.module_addresses.subaccount_registry,
        &[&t, &addr_topic(owner), &sid_topic(id), &pad_bytes32(subkey)],
        &data,
    )
}

pub fn deposit_log(
    m: &ManifestParams,
    subkey: &str,
    owner: &str,
    id: u32,
    token: &str,
    amount: &str,
) -> CanonicalRawLog {
    let t = topic_hex("Deposit");
    let data = pack_hex(&[
        PackField::Address(token),
        PackField::U256(amount),
        PackField::Address(owner),
        PackField::U16(1),
    ]);
    build_raw_log(
        &m.module_addresses.collateral_vault,
        &[&t, &pad_bytes32(subkey), &addr_topic(owner), &sid_topic(id)],
        &data,
    )
}

pub fn withdraw_log(
    m: &ManifestParams,
    subkey: &str,
    owner: &str,
    id: u32,
    token: &str,
    amount: &str,
) -> CanonicalRawLog {
    let t = topic_hex("Withdraw");
    let data = pack_hex(&[
        PackField::Address(token),
        PackField::U256(amount),
        PackField::Address(owner),
        PackField::U16(1),
    ]);
    build_raw_log(
        &m.module_addresses.collateral_vault,
        &[&t, &pad_bytes32(subkey), &addr_topic(owner), &sid_topic(id)],
        &data,
    )
}

pub fn reservation_lock_log(
    m: &ManifestParams,
    subkey: &str,
    token: &str,
    engine: &str,
    amount: &str,
) -> CanonicalRawLog {
    let t = topic_hex("CollateralLocked");
    let data = pack_hex(&[PackField::U256(amount), PackField::U16(1)]);
    build_raw_log(
        &m.module_addresses.collateral_vault,
        &[
            &t,
            &pad_bytes32(subkey),
            &addr_topic(token),
            &addr_topic(engine),
        ],
        &data,
    )
}

pub fn reservation_unlock_log(
    m: &ManifestParams,
    subkey: &str,
    token: &str,
    engine: &str,
    amount: &str,
) -> CanonicalRawLog {
    let t = topic_hex("CollateralUnlocked");
    let data = pack_hex(&[PackField::U256(amount), PackField::U16(1)]);
    build_raw_log(
        &m.module_addresses.collateral_vault,
        &[
            &t,
            &pad_bytes32(subkey),
            &addr_topic(token),
            &addr_topic(engine),
        ],
        &data,
    )
}

pub fn order_filled_log(
    m: &ManifestParams,
    subkey: &str,
    order_hash: &str,
    filled_delta: &str,
    total_qty: &str,
    terminal: bool,
) -> CanonicalRawLog {
    let t = topic_hex("OptionOrderFilled");
    let data = pack_hex(&[
        PackField::U256("1"),
        PackField::U8(0),
        PackField::U8(0),
        PackField::U128(filled_delta),
        PackField::U128(total_qty),
        PackField::U128(filled_delta),
        PackField::U128("0"),
        PackField::Bool(terminal),
        PackField::U8(0),
        PackField::Address("0x00000000000000000000000000000000000000a1"),
        PackField::U16(1),
    ]);
    build_raw_log(
        &m.module_addresses.option_matching_engine,
        &[&t, &pad_bytes32(subkey), &pad_bytes32(order_hash)],
        &data,
    )
}

pub fn matching_pair_log(
    m: &ManifestParams,
    exec_id: &str,
    buyer_oh: &str,
    seller_oh: &str,
    buyer_sk: &str,
    seller_sk: &str,
    qty: &str,
    premium: &str,
) -> CanonicalRawLog {
    let t = topic_hex("OptionOrderPairExecuted");
    let series = "0x0000000000000000000000000000000000000000000000000000000000000042";
    let data = pack_hex(&[
        PackField::U256("0"),
        PackField::Bytes32(&pad_bytes32(buyer_oh)),
        PackField::Bytes32(&pad_bytes32(seller_oh)),
        PackField::Address("0x0000000000000000000000000000000000001111"),
        PackField::Address("0x0000000000000000000000000000000000002222"),
        PackField::U32(1),
        PackField::U32(1),
        PackField::U128(qty),
        PackField::U128(premium),
        PackField::U256("42"),
        PackField::Address("0x0000000000000000000000000000000000003333"),
        PackField::U8(0),
        PackField::U8(1),
        PackField::U128("0"),
        PackField::U128("0"),
        PackField::Address("0x0000000000000000000000000000000000004444"),
        PackField::U16(1),
    ]);
    // We use the buyer subKey as topic3 in this fixture (seller subkey lives
    // in payload via convention).
    let _ = (buyer_sk, seller_sk, series);
    build_raw_log(
        &m.module_addresses.option_matching_engine,
        &[
            &t,
            &pad_bytes32(exec_id),
            &pad_bytes32(buyer_oh),
            &pad_bytes32(buyer_sk),
        ],
        &data,
    )
}

pub fn premium_log(
    m: &ManifestParams,
    from_sk: &str,
    to_sk: &str,
    token: &str,
    amount: &str,
) -> CanonicalRawLog {
    let t = topic_hex("OptionPremiumTransferred");
    let data = pack_hex(&[
        PackField::U256(amount),
        PackField::Address("0x0000000000000000000000000000000000005555"),
        PackField::U16(1),
    ]);
    build_raw_log(
        &m.module_addresses.collateral_vault,
        &[
            &t,
            &pad_bytes32(from_sk),
            &pad_bytes32(to_sk),
            &addr_topic(token),
        ],
        &data,
    )
}
