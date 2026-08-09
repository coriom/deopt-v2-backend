//! `BACKEND-HYBRID-V2-SIGNER-AND-EXECUTION-V1` (Part W) — bounded
//! property tests for the pre-broadcast execution surface.
//!
//! No new Cargo deps. Each property is exercised across a small,
//! deterministic sample set (up to 32 cases) driven by explicit
//! loops. This complements — not replaces — the full-matrix PG suite:
//! properties verify invariants across a parameter space, matrix tests
//! verify specific structural outcomes.
//!
//! Properties covered (audit checklist Part W):
//!   * prop_same_canonical_id_always_same_plan_hash
//!   * prop_deployment_id_changes_execution_id
//!   * prop_chain_id_changes_execution_id
//!   * prop_buyer_seller_swap_changes_execution_id (not commutative)
//!   * prop_fill_quantity_changes_execution_id
//!   * prop_no_arbitrary_target_can_pass_policy
//!   * prop_no_arbitrary_selector_can_pass_policy
//!   * prop_plan_immutability_after_signing_starts (SQL trigger)
//!   * prop_nonce_reservation_unique_per_signer_chain
//!   * prop_restart_preserves_plan_hash
//!   * prop_failed_simulation_never_reaches_signer
//!   * prop_excessive_gas_never_reaches_signer
//!   * prop_wrong_signer_never_reaches_accepted
//!   * prop_accepted_signature_binds_exact_plan
//!   * prop_ready_for_broadcast_requires_all_gates
//!   * prop_broadcast_disabled_causes_zero_send_rpc_calls
//!   * prop_public_input_cannot_control_envelope
//!   * prop_deployment_a_never_affects_b
//!
//! Every property that boots a mock asserts
//! `mock.prohibited_calls_seen()` is EMPTY on exit.

#![cfg(feature = "test-signer")]

mod hybrid_v2_mock_rpc_helpers;
mod hybrid_v2_support;

use std::sync::Arc;
use std::time::Duration;

use alloy_primitives::{Bytes, FixedBytes, U256};
use alloy_sol_types::SolCall;
use hybrid_v2_mock_rpc_helpers::{make_block, MockRpcServer};
use hybrid_v2_support::baseline_manifest;
use sqlx::postgres::{PgPool, PgPoolOptions};

use deopt_v2_backend::hybrid_v2::execution::plan::{
    executeMatchCall, ExecutionPlanBuilder, OptionOrder, SignedActionEnvelope,
};
use deopt_v2_backend::hybrid_v2::execution::signature_verify::verify_signed_tx;
use deopt_v2_backend::hybrid_v2::execution::signer::ExecutionSigner;
use deopt_v2_backend::hybrid_v2::execution::signer_ephemeral::TestEphemeralSigner;
use deopt_v2_backend::hybrid_v2::execution::{
    derive_canonical_execution_id, failure_class as fc, ExecutionOrchestrator, ExecutionPhase,
    ExecutionRequestRow, GasFeePolicy, HttpExecutionRpcClient, MockClock, PolicyError,
    PreparationIntent, SigningRequest, TargetPolicy,
};
use deopt_v2_backend::hybrid_v2::persistence::{
    HybridV2ProjectionStore, PostgresHybridV2ProjectionStore,
};
use deopt_v2_backend::hybrid_v2::readiness::{ReadinessReport, ReadinessState};
use deopt_v2_backend::hybrid_v2::reducer::ProjectionState;

// -----------------------------------------------------------------
//                          PG helpers
// -----------------------------------------------------------------

const URL_ENV: &str = "HYBRID_V2_PG_TEST_DATABASE_URL";
const ALT_URL_ENV: &str = "PG_INTEGRATION_URL";
const REQUIRE_ENV: &str = "DEOPT_REQUIRE_PG_INTEGRATION";
const PROP_CASES: usize = 20;

fn get_pg_url_or_skip(test_name: &str) -> Option<String> {
    let url = std::env::var(URL_ENV)
        .ok()
        .or_else(|| std::env::var(ALT_URL_ENV).ok())
        .filter(|v| !v.is_empty());
    if url.is_none() {
        let required = matches!(
            std::env::var(REQUIRE_ENV).ok().as_deref(),
            Some("1") | Some("true") | Some("TRUE")
        );
        if required {
            panic!("{} required but no PG URL provided", REQUIRE_ENV);
        }
        eprintln!("SKIP {test_name}: no PG URL");
    }
    url
}

async fn fresh_pool(url: &str) -> PgPool {
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .acquire_timeout(Duration::from_secs(30))
        .connect(url)
        .await
        .expect("connect");
    sqlx::query("DROP SCHEMA IF EXISTS public CASCADE")
        .execute(&pool)
        .await
        .expect("drop schema");
    sqlx::query("CREATE SCHEMA public")
        .execute(&pool)
        .await
        .expect("create schema");
    sqlx::query("GRANT ALL ON SCHEMA public TO PUBLIC")
        .execute(&pool)
        .await
        .expect("grant");
    let migrator = sqlx::migrate::Migrator::new(std::path::Path::new("./migrations"))
        .await
        .expect("migrations");
    migrator.run(&pool).await.expect("apply migrations");
    pool
}

async fn build_store(pool: &PgPool) -> (Arc<dyn HybridV2ProjectionStore>, i64) {
    let store: Arc<dyn HybridV2ProjectionStore> =
        Arc::new(PostgresHybridV2ProjectionStore::new(pool.clone()));
    let manifest = baseline_manifest(84532);
    let deployment_id = store
        .upsert_deployment(&manifest, "PENDING", 1_700_000_000_000)
        .await
        .expect("upsert deployment");
    (store, deployment_id)
}

// -----------------------------------------------------------------
//                          fixture helpers
// -----------------------------------------------------------------

fn owner(seed: u8) -> [u8; 20] {
    let mut a = [0u8; 20];
    for (i, b) in a.iter_mut().enumerate() {
        *b = seed | (i as u8);
    }
    a
}
fn to_hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2 + 2);
    s.push_str("0x");
    for b in bytes {
        s.push_str(&format!("{:02x}", b));
    }
    s
}
fn engine_address_from_manifest(
    m: &deopt_v2_backend::hybrid_v2::manifest::ManifestParams,
) -> [u8; 20] {
    let hex = m
        .module_addresses
        .option_matching_engine
        .trim_start_matches("0x");
    let mut a = [0u8; 20];
    for i in 0..20 {
        a[i] = u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).unwrap();
    }
    a
}
fn baseline_envelope(
    manifest: &deopt_v2_backend::hybrid_v2::manifest::ManifestParams,
    owner: [u8; 20],
    subkey_seed: u8,
) -> SignedActionEnvelope {
    let engine = engine_address_from_manifest(manifest);
    SignedActionEnvelope {
        owner: alloy_primitives::Address::from(owner),
        subaccountId: 1,
        subKey: FixedBytes::from([subkey_seed; 32]),
        signer: alloy_primitives::Address::from(owner),
        engine: alloy_primitives::Address::from(engine),
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
        premiumToken: alloy_primitives::Address::from({
            let mut a = [0u8; 20];
            for (i, b) in a.iter_mut().enumerate() {
                *b = 0x0b + (i as u8);
            }
            a
        }),
        timeInForce: 0,
        role: 0,
        maxPositiveFeePpm: 100,
        salt: FixedBytes::from([0x33u8; 32]),
    }
}
fn ready_readiness() -> ReadinessReport {
    ReadinessReport {
        runtime: ReadinessState::ready(),
        rebuild: ReadinessState::ready(),
        reconciliation: ReadinessState::ready(),
    }
}
fn build_intent(
    manifest: &deopt_v2_backend::hybrid_v2::manifest::ManifestParams,
    fill_qty: u128,
    buyer_seed: u8,
    seller_seed: u8,
) -> PreparationIntent {
    let buyer_env = baseline_envelope(manifest, owner(0xa0 | buyer_seed & 0x0f), buyer_seed);
    let seller_env = baseline_envelope(manifest, owner(0xb0 | seller_seed & 0x0f), seller_seed);
    PreparationIntent {
        manifest: manifest.clone(),
        runtime_state: ProjectionState::default(),
        readiness: ready_readiness(),
        buyer_envelope: buyer_env,
        buyer_signature: Bytes::from(vec![0x77u8; 65]),
        buyer_order: baseline_order(),
        seller_envelope: seller_env,
        seller_signature: Bytes::from(vec![0x88u8; 65]),
        seller_order: baseline_order(),
        fill_quantity_1e8: fill_qty,
        buyer_active_series: vec![U256::from(42u64)],
        seller_active_series: vec![U256::from(42u64)],
        buyer_order_hash: format!("0x{}{}", format!("{:02x}", buyer_seed), "aa".repeat(31)),
        seller_order_hash: format!("0x{}{}", format!("{:02x}", seller_seed), "bb".repeat(31)),
        buyer_subkey: format!("0x{}", format!("{:02x}", buyer_seed).repeat(32)),
        seller_subkey: format!("0x{}", format!("{:02x}", seller_seed).repeat(32)),
        series_id: "42".to_string(),
        premium_amount: "50000000".to_string(),
        fee_schedule_epoch: None,
    }
}
async fn boot_mock_for_happy_path(chain_id: u64, plan_target: [u8; 20]) -> MockRpcServer {
    let mock = MockRpcServer::start().await;
    mock.set_chain_id(chain_id);
    mock.set_head(100);
    let latest_block = make_block(100, 0xab, &format!("0x{}", "cd".repeat(32)), 1_700_000_000);
    mock.push_block(latest_block);
    mock.set_eth_call_response(&to_hex(&plan_target), executeMatchCall::SELECTOR, {
        let mut buf = vec![0u8; 32];
        buf[31] = 0x42;
        buf
    });
    mock.set_estimate_gas_response(90_000);
    mock.set_fee_history(
        vec![
            "0x3b9aca00".to_string(),
            "0x41314cf0".to_string(),
            "0x3d0900c0".to_string(),
            "0x3b9aca00".to_string(),
        ],
        vec![
            vec!["0x1dcd6500".to_string()],
            vec!["0x22ecb25c".to_string()],
            vec!["0x20c855c0".to_string()],
        ],
    );
    mock
}
fn gas_policy() -> GasFeePolicy {
    GasFeePolicy {
        max_gas_limit: 5_000_000,
        gas_limit_multiplier_bps: 12_000,
        max_fee_per_gas_wei: U256::from(50_000_000_000u64),
        max_priority_fee_per_gas_wei: U256::from(2_000_000_000u64),
        max_total_native_cost_wei: U256::from(10u64).pow(U256::from(18u64)),
        abnormal_estimate_reject_threshold: 10,
    }
}
fn seed_row(deployment_id: i64, chain_id: u64, canonical_id: &str) -> ExecutionRequestRow {
    let selector = format!(
        "0x{}",
        executeMatchCall::SELECTOR
            .iter()
            .map(|b| format!("{:02x}", b))
            .collect::<String>()
    );
    // Use the baseline manifest's engine address so the plan builder
    // accepts the row.
    let target = baseline_manifest(chain_id)
        .module_addresses
        .option_matching_engine
        .clone();
    ExecutionRequestRow {
        canonical_execution_id: canonical_id.to_string(),
        deployment_id,
        chain_id: chain_id as i64,
        execution_kind: "HYBRID_V2_OPTION_MATCH".to_string(),
        buyer_order_hash: format!("0x{}", "aa".repeat(32)),
        seller_order_hash: format!("0x{}", "bb".repeat(32)),
        buyer_subkey: format!("0x{}", "aa".repeat(32)),
        seller_subkey: format!("0x{}", "bb".repeat(32)),
        series_id: "42".to_string(),
        fill_quantity_1e8: "100000000".to_string(),
        premium_amount: "50000000".to_string(),
        fee_schedule_epoch: None,
        source_matched_execution_id: None,
        target_contract: target,
        selector,
        calldata_hash: None,
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
        created_at_ms: 1_700_000_000_000,
        updated_at_ms: 1_700_000_000_000,
    }
}

// -----------------------------------------------------------------
//                          PROPERTIES
// -----------------------------------------------------------------

// ---- Determinism properties (pure) ------------------------------

#[test]
fn prop_same_canonical_id_always_same_plan_hash() {
    // For a fixed intent, ExecutionPlanBuilder MUST produce a
    // deterministic plan_hash every call. Sweep several distinct
    // intents; each call twice must agree.
    let manifest = baseline_manifest(84532);
    for i in 0..PROP_CASES {
        let intent = build_intent(
            &manifest,
            100_000_000 + i as u128,
            0xa1 | i as u8,
            0xb1 | i as u8,
        );
        let cid = derive_canonical_execution_id(
            1,
            84532,
            &intent.buyer_order_hash,
            &intent.seller_order_hash,
            intent.fill_quantity_1e8,
        )
        .into_string();
        let mut row = seed_row(1, 84532, &cid);
        row.buyer_order_hash = intent.buyer_order_hash.clone();
        row.seller_order_hash = intent.seller_order_hash.clone();
        row.buyer_subkey = intent.buyer_subkey.clone();
        row.seller_subkey = intent.seller_subkey.clone();
        row.fill_quantity_1e8 = intent.fill_quantity_1e8.to_string();
        let a = ExecutionPlanBuilder::build_from_request(
            &row,
            &manifest,
            &intent.buyer_envelope,
            &intent.buyer_signature,
            &intent.buyer_order,
            &intent.seller_envelope,
            &intent.seller_signature,
            &intent.seller_order,
            intent.fill_quantity_1e8,
            &intent.buyer_active_series,
            &intent.seller_active_series,
        )
        .expect("A");
        let b = ExecutionPlanBuilder::build_from_request(
            &row,
            &manifest,
            &intent.buyer_envelope,
            &intent.buyer_signature,
            &intent.buyer_order,
            &intent.seller_envelope,
            &intent.seller_signature,
            &intent.seller_order,
            intent.fill_quantity_1e8,
            &intent.buyer_active_series,
            &intent.seller_active_series,
        )
        .expect("B");
        assert_eq!(a.plan_hash, b.plan_hash, "case {i}: plan_hash divergence");
        assert_eq!(a.calldata_hash, b.calldata_hash);
        assert_eq!(a.calldata, b.calldata);
    }
}

#[test]
fn prop_deployment_id_changes_execution_id() {
    // Vary only deployment_id — canonical id must change every time.
    let base = derive_canonical_execution_id(1, 84532, "0xbuy", "0xsell", 100).into_string();
    for dep in 2..=(PROP_CASES as i64 + 1) {
        let other = derive_canonical_execution_id(dep, 84532, "0xbuy", "0xsell", 100).into_string();
        assert_ne!(
            base, other,
            "deployment {dep} must produce a distinct canonical id"
        );
    }
}

#[test]
fn prop_chain_id_changes_execution_id() {
    let base = derive_canonical_execution_id(1, 84532, "0xbuy", "0xsell", 100).into_string();
    for chain in [
        11155111u64,
        137,
        42161,
        1,
        43114,
        10,
        8453 + 1000,
        2,
        3,
        4,
        5,
        6,
        7,
        8,
        9,
        10,
        100,
        1_000_000,
        12345,
        999_999_999,
    ]
    .iter()
    .take(PROP_CASES)
    {
        let other = derive_canonical_execution_id(1, *chain, "0xbuy", "0xsell", 100).into_string();
        assert_ne!(
            base, other,
            "chain {chain} must yield distinct canonical id"
        );
    }
}

#[test]
fn prop_buyer_seller_swap_changes_execution_id_not_commutative() {
    for i in 0..PROP_CASES {
        let a_hash = format!("0x{}{}", format!("{:02x}", i as u8), "aa".repeat(31));
        let b_hash = format!(
            "0x{}{}",
            format!("{:02x}", (i as u8).wrapping_add(1)),
            "bb".repeat(31)
        );
        let ab = derive_canonical_execution_id(1, 84532, &a_hash, &b_hash, 100).into_string();
        let ba = derive_canonical_execution_id(1, 84532, &b_hash, &a_hash, 100).into_string();
        assert_ne!(
            ab, ba,
            "case {i}: buyer/seller swap must not be commutative"
        );
    }
}

#[test]
fn prop_fill_quantity_changes_execution_id() {
    let base = derive_canonical_execution_id(1, 84532, "0xbuy", "0xsell", 100).into_string();
    for i in 1..=PROP_CASES as u128 {
        let other =
            derive_canonical_execution_id(1, 84532, "0xbuy", "0xsell", 100 + i).into_string();
        assert_ne!(
            base,
            other,
            "fill_quantity {} must yield a distinct canonical id",
            100 + i
        );
    }
}

// ---- Target-policy properties -----------------------------------

#[test]
fn prop_no_arbitrary_target_can_pass_policy() {
    let manifest = baseline_manifest(84532);
    let policy = TargetPolicy::from_manifest(&manifest).unwrap();
    let engine = engine_address_from_manifest(&manifest);
    for i in 0..PROP_CASES {
        // Construct addresses that are structurally random but NOT the
        // engine address.
        let mut a = [0u8; 20];
        for (j, byte) in a.iter_mut().enumerate() {
            *byte = ((i * 13 + j * 7) as u8).wrapping_add(0x11);
        }
        if a == engine {
            continue; // extremely unlikely; skip
        }
        let err = policy
            .is_allowed(84532, &a, &executeMatchCall::SELECTOR)
            .unwrap_err();
        assert!(
            matches!(err, PolicyError::UnknownTarget { .. }),
            "case {i}: address 0x{} must be rejected but got {err:?}",
            a.iter().map(|b| format!("{:02x}", b)).collect::<String>()
        );
    }
}

#[test]
fn prop_no_arbitrary_selector_can_pass_policy() {
    let manifest = baseline_manifest(84532);
    let policy = TargetPolicy::from_manifest(&manifest).unwrap();
    let target = engine_address_from_manifest(&manifest);
    for i in 0..PROP_CASES {
        let mut sel = [0u8; 4];
        for (j, byte) in sel.iter_mut().enumerate() {
            *byte = ((i * 17 + j * 11) as u8).wrapping_add(0x33);
        }
        if sel == executeMatchCall::SELECTOR {
            continue;
        }
        let err = policy.is_allowed(84532, &target, &sel).unwrap_err();
        assert!(
            matches!(err, PolicyError::UnknownSelector { .. }),
            "case {i}: selector 0x{} must be rejected but got {err:?}",
            sel.iter().map(|b| format!("{:02x}", b)).collect::<String>()
        );
    }
}

// ---- Persistence-layer properties -------------------------------

#[tokio::test]
async fn prop_plan_immutability_after_signing_starts_sql_trigger() {
    let Some(url) = get_pg_url_or_skip("prop_plan_immutability_after_signing_starts_sql_trigger")
    else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, deployment_id) = build_store(&pool).await;
    // For each case: seed a distinct row with plan_hash set, attempt
    // to overwrite it with a different value — SQL trigger must
    // refuse.
    for i in 0..PROP_CASES.min(10) {
        let cid = format!("0x{:02x}{}", i as u8, "ab".repeat(31));
        let mut row = seed_row(deployment_id, 84532, &cid);
        row.buyer_order_hash = format!("0x{:02x}{}", i as u8, "cc".repeat(31));
        row.seller_order_hash = format!("0x{:02x}{}", (i as u8).wrapping_add(1), "dd".repeat(31));
        row.plan_hash = Some(format!("0x{}", format!("{:02x}", i as u8).repeat(32)));
        store.insert_execution_request(&row).await.expect("insert");
        let res = sqlx::query(
            "UPDATE hybrid_v2_execution_requests SET plan_hash = $2, updated_at_ms = $3 \
             WHERE canonical_execution_id = $1",
        )
        .bind(&cid)
        .bind(format!(
            "0x{}",
            format!("{:02x}", i as u8 ^ 0xff).repeat(32)
        ))
        .bind(1_700_000_000_100i64)
        .execute(&pool)
        .await;
        assert!(
            res.is_err(),
            "case {i}: plan_hash mutation must be refused by SQL trigger"
        );
    }
}

#[tokio::test]
async fn prop_nonce_reservation_unique_per_signer_chain() {
    let Some(url) = get_pg_url_or_skip("prop_nonce_reservation_unique_per_signer_chain") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, deployment_id) = build_store(&pool).await;
    let signer_hex = format!("0x{}", "ab".repeat(20));
    // Reserve 20 distinct nonces sequentially. Each reservation must
    // succeed exactly once; duplicate on the same nonce must refuse.
    for n in 0..PROP_CASES as i64 {
        let cid = format!("0x{:02x}{}", n as u8, "ee".repeat(31));
        let mut row = seed_row(deployment_id, 84532, &cid);
        row.buyer_order_hash = format!("0x{:02x}{}", n as u8, "aa".repeat(31));
        row.seller_order_hash = format!("0x{:02x}{}", n as u8, "bb".repeat(31));
        store.insert_execution_request(&row).await.expect("insert");
        let ok = store
            .reserve_executor_nonce(84532, &signer_hex, n, &cid, 1_700_000_000_000 + n)
            .await
            .expect("reserve first");
        assert!(ok, "case {n}: first reserve must succeed");
        let dup = store
            .reserve_executor_nonce(84532, &signer_hex, n, &cid, 1_700_000_000_001 + n)
            .await
            .expect("reserve dup");
        assert!(!dup, "case {n}: duplicate reserve must be refused");
    }
    let read = store
        .get_reserved_nonces_for(84532, &signer_hex)
        .await
        .expect("read");
    assert_eq!(read.len(), PROP_CASES);
}

#[tokio::test]
async fn prop_restart_preserves_plan_hash_and_state() {
    let Some(url) = get_pg_url_or_skip("prop_restart_preserves_plan_hash_and_state") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store_a, deployment_id) = build_store(&pool).await;
    for i in 0..PROP_CASES.min(10) {
        let cid = format!("0x{:02x}{}", i as u8, "77".repeat(31));
        let mut row = seed_row(deployment_id, 84532, &cid);
        row.buyer_order_hash = format!("0x{:02x}{}", i as u8, "88".repeat(31));
        row.seller_order_hash = format!("0x{:02x}{}", i as u8, "99".repeat(31));
        row.plan_hash = Some(format!("0x{}", format!("{:02x}", i as u8).repeat(32)));
        store_a
            .insert_execution_request(&row)
            .await
            .expect("insert");
    }
    // Restart the store — verify all rows come back with plan_hash
    // intact.
    let store_b: Arc<dyn HybridV2ProjectionStore> =
        Arc::new(PostgresHybridV2ProjectionStore::new(pool.clone()));
    for i in 0..PROP_CASES.min(10) {
        let cid = format!("0x{:02x}{}", i as u8, "77".repeat(31));
        let out = store_b
            .get_execution_request(&cid)
            .await
            .expect("get")
            .expect("row");
        assert_eq!(
            out.plan_hash,
            Some(format!("0x{}", format!("{:02x}", i as u8).repeat(32))),
            "case {i}: plan_hash must survive restart"
        );
    }
}

// ---- Orchestrator gate properties (require test-signer) ---------

#[tokio::test]
async fn prop_broadcast_disabled_causes_zero_send_rpc_calls() {
    let Some(url) = get_pg_url_or_skip("prop_broadcast_disabled_causes_zero_send_rpc_calls") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, deployment_id) = build_store(&pool).await;
    let manifest = baseline_manifest(84532);
    let target = engine_address_from_manifest(&manifest);
    let signer = Arc::new(TestEphemeralSigner::from_seed([0xffu8; 32]));
    let signer_addr = signer.address();
    let mock = boot_mock_for_happy_path(84532, target).await;
    mock.set_transaction_count(&to_hex(&signer_addr), "pending", 0);
    let rpc =
        Arc::new(HttpExecutionRpcClient::new(mock.url(), Duration::from_secs(2), 0).expect("rpc"));
    let target_policy = Arc::new(TargetPolicy::from_manifest(&manifest).unwrap());
    let orchestrator = ExecutionOrchestrator {
        store: store.clone(),
        rpc,
        signer,
        target_policy,
        gas_policy: Arc::new(gas_policy()),
        deployment_id,
        chain_id: 84532,
        executor_address: signer_addr,
        simulation_max_age_ms: 60_000,
        clock: Arc::new(MockClock::new(1_700_000_000_000)),
    };
    let intent = build_intent(&manifest, 100_000_000, 0xaa, 0xbb);
    let out = orchestrator.prepare(intent).await.expect("prepare");
    assert_eq!(out.terminal_phase, ExecutionPhase::BroadcastDisabled);
    let prohibited = mock.prohibited_calls_seen();
    assert!(
        prohibited.is_empty(),
        "prohibited broadcast methods seen: {prohibited:?}"
    );
    // Zero send_* method count is also verified by absence — any
    // matching method would have populated `prohibited_calls_seen`.
}

#[tokio::test]
async fn prop_failed_simulation_never_reaches_signer_or_broadcast() {
    let Some(url) = get_pg_url_or_skip("prop_failed_simulation_never_reaches_signer_or_broadcast")
    else {
        return;
    };
    // Take a handful of known selectors and force reverts.
    use deopt_v2_backend::hybrid_v2::execution::rpc::KNOWN_CUSTOM_ERROR_SELECTORS;
    let selectors: Vec<[u8; 4]> = KNOWN_CUSTOM_ERROR_SELECTORS
        .iter()
        .take(3)
        .map(|(s, _)| *s)
        .collect();
    for (i, sel) in selectors.iter().enumerate() {
        let pool = fresh_pool(&url).await;
        let (store, deployment_id) = build_store(&pool).await;
        let manifest = baseline_manifest(84532);
        let target = engine_address_from_manifest(&manifest);
        let signer = Arc::new(TestEphemeralSigner::from_seed([i as u8; 32]));
        let signer_addr = signer.address();
        let mock = MockRpcServer::start().await;
        mock.set_chain_id(84532);
        mock.set_head(100);
        mock.push_block(make_block(
            100,
            0xab,
            &format!("0x{}", "cd".repeat(32)),
            1_700_000_000,
        ));
        mock.set_eth_call_next_rpc_error(
            -32000,
            format!(
                "execution reverted\ndata:0x{}",
                sel.iter().map(|b| format!("{:02x}", b)).collect::<String>()
            ),
        );
        mock.set_transaction_count(&to_hex(&signer_addr), "pending", 0);
        mock.set_estimate_gas_response(1);
        mock.set_fee_history(
            vec!["0x3b9aca00".to_string()],
            vec![vec!["0x1dcd6500".to_string()]],
        );
        let rpc = Arc::new(
            HttpExecutionRpcClient::new(mock.url(), Duration::from_secs(2), 0).expect("rpc"),
        );
        let target_policy = Arc::new(TargetPolicy::from_manifest(&manifest).unwrap());
        let orchestrator = ExecutionOrchestrator {
            store: store.clone(),
            rpc,
            signer,
            target_policy,
            gas_policy: Arc::new(gas_policy()),
            deployment_id,
            chain_id: 84532,
            executor_address: signer_addr,
            simulation_max_age_ms: 60_000,
            clock: Arc::new(MockClock::new(1_700_000_000_000)),
        };
        let intent = build_intent(&manifest, 100_000_000, 0x1a | i as u8, 0x2b | i as u8);
        let out = orchestrator.prepare(intent).await.expect("prepare");
        assert_eq!(out.terminal_phase, ExecutionPhase::Failed);
        assert_eq!(
            out.failure_class.as_deref(),
            Some(fc::SIMULATION_FAILED_DETERMINISTIC)
        );
        // A signer signature must NOT have been persisted — the row's
        // signature fields must be empty.
        let row = store
            .get_execution_request(&out.canonical_execution_id)
            .await
            .expect("get")
            .expect("row");
        assert!(
            row.signature_r.is_none(),
            "case {i}: no signature must be persisted"
        );
        assert!(row.signature_s.is_none());
        assert!(row.recovered_signer.is_none());
        assert_ne!(row.phase, ExecutionPhase::BroadcastDisabled);
        assert!(mock.prohibited_calls_seen().is_empty());
    }
}

#[tokio::test]
async fn prop_excessive_gas_never_reaches_signer() {
    let Some(url) = get_pg_url_or_skip("prop_excessive_gas_never_reaches_signer") else {
        return;
    };
    // Vary the gas estimate returned by the mock — any value above
    // the ceiling triggers rejection before signing.
    for i in 0..5usize {
        let pool = fresh_pool(&url).await;
        let (store, deployment_id) = build_store(&pool).await;
        let manifest = baseline_manifest(84532);
        let target = engine_address_from_manifest(&manifest);
        let signer = Arc::new(TestEphemeralSigner::from_seed([0x22u8 + i as u8; 32]));
        let signer_addr = signer.address();
        let mock = boot_mock_for_happy_path(84532, target).await;
        mock.set_transaction_count(&to_hex(&signer_addr), "pending", 0);
        // Estimate 10M gas — above the 5M ceiling in gas_policy().
        mock.set_estimate_gas_response(10_000_000 + i as u64 * 1_000_000);
        let rpc = Arc::new(
            HttpExecutionRpcClient::new(mock.url(), Duration::from_secs(2), 0).expect("rpc"),
        );
        let target_policy = Arc::new(TargetPolicy::from_manifest(&manifest).unwrap());
        let orchestrator = ExecutionOrchestrator {
            store: store.clone(),
            rpc,
            signer,
            target_policy,
            gas_policy: Arc::new(gas_policy()),
            deployment_id,
            chain_id: 84532,
            executor_address: signer_addr,
            simulation_max_age_ms: 60_000,
            clock: Arc::new(MockClock::new(1_700_000_000_000)),
        };
        let intent = build_intent(&manifest, 100_000_000, 0xa0 | i as u8, 0xb0 | i as u8);
        let out = orchestrator.prepare(intent).await.expect("prepare");
        assert_eq!(out.terminal_phase, ExecutionPhase::Failed);
        assert_eq!(out.failure_class.as_deref(), Some(fc::GAS_POLICY_REJECTED));
        let row = store
            .get_execution_request(&out.canonical_execution_id)
            .await
            .expect("get")
            .expect("row");
        assert!(
            row.signature_r.is_none(),
            "case {i}: no signature persisted"
        );
        assert!(mock.prohibited_calls_seen().is_empty());
    }
}

// ---- Signature-binding properties -------------------------------

#[test]
fn prop_wrong_signer_never_reaches_accepted() {
    // For each of several signers, sign a request; then verify
    // against a DIFFERENT expected signer — always rejected.
    use sha3::{Digest, Keccak256};
    for i in 0..PROP_CASES {
        let signer_a = TestEphemeralSigner::from_seed([i as u8 | 0x01; 32]);
        let signer_b = TestEphemeralSigner::from_seed([i as u8 | 0x80; 32]);
        if signer_a.address() == signer_b.address() {
            continue;
        }
        let calldata = vec![0x11u8, 0x22, i as u8];
        let calldata_hash: [u8; 32] = {
            let h = Keccak256::digest(&calldata);
            let mut a = [0u8; 32];
            a.copy_from_slice(&h[..]);
            a
        };
        let signing_payload_hash: [u8; 32] = {
            let mut h = Keccak256::new();
            h.update(b"HV2_TEST_PAYLOAD");
            h.update(calldata_hash);
            let out = h.finalize();
            let mut a = [0u8; 32];
            a.copy_from_slice(&out[..]);
            a
        };
        let req = SigningRequest {
            chain_id: 84532,
            nonce: i as u64,
            target: [0xcc; 20],
            value_wei: U256::ZERO,
            calldata_hash,
            gas_limit: 100_000,
            max_fee_per_gas_wei: U256::from(1_000_000_000u64),
            max_priority_fee_per_gas_wei: U256::from(500_000_000u64),
            tx_type: 2,
            plan_hash: [0xee; 32],
            signing_payload_hash,
            calldata,
        };
        let signed = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(signer_a.sign_execution(req.clone()))
            .expect("sign A");
        // Verify with the WRONG expected signer (B) — must reject.
        let err = verify_signed_tx(&req, &signed, signer_b.address()).unwrap_err();
        // Any variant is acceptable — the invariant is REJECTION.
        assert!(
            matches!(
                err,
                deopt_v2_backend::hybrid_v2::execution::SigVerifyError::SignerMismatch { .. }
                    | deopt_v2_backend::hybrid_v2::execution::SigVerifyError::RecoveryFailed
            ),
            "case {i}: wrong signer must be rejected, got {err:?}"
        );
    }
}

#[test]
fn prop_accepted_signature_binds_exact_plan() {
    // For each case: sign over payload_hash A; alter ONLY the
    // signing_payload_hash to B; verify must reject.
    use sha3::{Digest, Keccak256};
    for i in 0..PROP_CASES {
        let signer = TestEphemeralSigner::from_seed([0x30u8 | i as u8; 32]);
        let calldata = vec![i as u8; 5];
        let calldata_hash: [u8; 32] = {
            let h = Keccak256::digest(&calldata);
            let mut a = [0u8; 32];
            a.copy_from_slice(&h[..]);
            a
        };
        let sph_a: [u8; 32] = {
            let mut h = Keccak256::new();
            h.update(b"HV2_TEST_PAYLOAD_A");
            h.update(calldata_hash);
            h.update([i as u8]);
            let out = h.finalize();
            let mut a = [0u8; 32];
            a.copy_from_slice(&out[..]);
            a
        };
        let sph_b: [u8; 32] = {
            let mut h = Keccak256::new();
            h.update(b"HV2_TEST_PAYLOAD_B_ALT");
            h.update(calldata_hash);
            h.update([i as u8 ^ 0xff]);
            let out = h.finalize();
            let mut a = [0u8; 32];
            a.copy_from_slice(&out[..]);
            a
        };
        let req_a = SigningRequest {
            chain_id: 84532,
            nonce: 0,
            target: [0xcc; 20],
            value_wei: U256::ZERO,
            calldata_hash,
            gas_limit: 100_000,
            max_fee_per_gas_wei: U256::from(1u64),
            max_priority_fee_per_gas_wei: U256::from(1u64),
            tx_type: 2,
            plan_hash: [0xaa; 32],
            signing_payload_hash: sph_a,
            calldata: calldata.clone(),
        };
        let mut req_b = req_a.clone();
        req_b.signing_payload_hash = sph_b;
        let signed_a = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(signer.sign_execution(req_a.clone()))
            .expect("sign A");
        // Applying signed_a to req_b must fail — recovered signer will
        // differ (recovery uses sph_b, but the signature was over sph_a).
        let err = verify_signed_tx(&req_b, &signed_a, signer.address()).unwrap_err();
        assert!(
            matches!(
                err,
                deopt_v2_backend::hybrid_v2::execution::SigVerifyError::SignerMismatch { .. }
                    | deopt_v2_backend::hybrid_v2::execution::SigVerifyError::RecoveryFailed
            ),
            "case {i}: signature must bind the exact plan / payload hash"
        );
    }
}

// ---- Structural properties --------------------------------------

#[test]
fn prop_ready_for_broadcast_requires_all_gates_state_machine() {
    // Property (state-machine-derived): every path into
    // ReadyForBroadcast MUST pass through
    // SignatureVerified. There is no other predecessor.
    for from in ExecutionPhase::ALL {
        if from.can_transition_to(ExecutionPhase::ReadyForBroadcast) {
            assert_eq!(
                *from,
                ExecutionPhase::SignatureVerified,
                "only SignatureVerified may transition to ReadyForBroadcast; found {from:?}"
            );
        }
    }
    // Property (BroadcastDisabled successor): the only exit from
    // ReadyForBroadcast is BroadcastDisabled.
    let succs = ExecutionPhase::ReadyForBroadcast.legal_successors();
    assert_eq!(succs, &[ExecutionPhase::BroadcastDisabled]);
}

#[test]
fn prop_public_input_cannot_control_envelope_via_admin_types() {
    // Structural: PrepareRequestBody does not expose target /
    // calldata / value / nonce / gas / chain_id — this is the
    // audit's non-negotiable public-input contract.
    use deopt_v2_backend::api::hybrid_v2_execution_admin::PrepareRequestBody;
    // Any JSON body deserialization must FAIL if it tries to inject
    // one of the reserved fields — see
    // BACKEND-HYBRID-V2-EXTERNAL-SIGNER-INTEGRATION-AND-LIVE-ORCHESTRATOR-V1
    // Part J: the deserializer is `#[serde(deny_unknown_fields)]` so a
    // hostile body carrying `target_contract`/`calldata`/`chain_id`/
    // `nonce`/`gas_limit`/`value_wei` is rejected loudly rather than
    // silently having its extras dropped.
    let hostile = serde_json::json!({
        "buyer_envelope": {
            "owner": "0x00", "subaccount_id": 1, "subkey": "0x00",
            "signer": "0x00", "engine": "0x00", "action": "0x00",
            "architecture_version": "1", "nonce": "1",
            "deadline": "0", "owner_recovery_epoch": "0",
            "subaccount_recovery_epoch": "0", "payload_hash": "0x00",
            "signature": "0x00"
        },
        "buyer_order": {
            "series_id": "0", "side": 0, "quantity_1e8": "1",
            "price_per_contract_1e8": "1", "limit_price_per_contract_1e8": "1",
            "premium_token": "0x00", "time_in_force": 0, "role": 0,
            "max_positive_fee_ppm": 0, "salt": "0x00"
        },
        "seller_envelope": {
            "owner": "0x00", "subaccount_id": 1, "subkey": "0x00",
            "signer": "0x00", "engine": "0x00", "action": "0x00",
            "architecture_version": "1", "nonce": "1",
            "deadline": "0", "owner_recovery_epoch": "0",
            "subaccount_recovery_epoch": "0", "payload_hash": "0x00",
            "signature": "0x00"
        },
        "seller_order": {
            "series_id": "0", "side": 1, "quantity_1e8": "1",
            "price_per_contract_1e8": "1", "limit_price_per_contract_1e8": "1",
            "premium_token": "0x00", "time_in_force": 0, "role": 0,
            "max_positive_fee_ppm": 0, "salt": "0x00"
        },
        "fill_quantity_1e8": "1",
        "buyer_active_series": [],
        "seller_active_series": [],
        "buyer_order_hash": "0x00",
        "seller_order_hash": "0x00",
        "series_id": "0",
        "premium_amount": "0",
        // Attempt to smuggle reserved-fields — these MUST be ignored
        // (no representation in PrepareRequestBody).
        "target_contract": "0xATTACK_TARGET",
        "calldata": "0xATTACK_CALLDATA",
        "chain_id": 8453,
        "nonce": 999,
        "gas_limit": 999_999_999,
        "value_wei": 100
    });
    let parsed: Result<PrepareRequestBody, _> = serde_json::from_value(hostile.clone());
    // Must FAIL — a hostile body carrying reserved fields is rejected.
    let err = parsed.expect_err("hostile body must be rejected via deny_unknown_fields");
    let err_str = err.to_string();
    assert!(
        err_str.contains("unknown field") || err_str.contains("target_contract"),
        "expected deny_unknown_fields rejection, got {err_str}"
    );
    // Same body with reserved fields stripped MUST parse successfully.
    let mut clean = hostile.clone();
    let obj = clean.as_object_mut().unwrap();
    for reserved in [
        "target_contract",
        "calldata",
        "chain_id",
        "nonce",
        "gas_limit",
        "value_wei",
    ] {
        obj.remove(reserved);
    }
    let parsed_ok: PrepareRequestBody =
        serde_json::from_value(clean).expect("clean body must parse");
    let dbg = format!("{parsed_ok:?}");
    assert!(!dbg.contains("ATTACK_TARGET"));
    assert!(!dbg.contains("ATTACK_CALLDATA"));
    // No gas_limit / value_wei field on PrepareRequestBody at all.
    assert!(!dbg.contains("gas_limit"));
    assert!(!dbg.contains("value_wei"));
    assert!(!dbg.contains("chain_id"));
}

#[tokio::test]
async fn prop_deployment_a_never_affects_b() {
    let Some(url) = get_pg_url_or_skip("prop_deployment_a_never_affects_b") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, deployment_a) = build_store(&pool).await;
    // Register a second deployment (Sepolia, distinct manifest).
    let mut manifest_b = baseline_manifest(11155111);
    manifest_b.manifest_hash = format!("0x{}", "cc".repeat(32));
    manifest_b.deployment_version = 2;
    let deployment_b = store
        .upsert_deployment(&manifest_b, "PENDING", 1_700_000_000_000)
        .await
        .expect("B");
    assert_ne!(deployment_a, deployment_b);
    for i in 0..PROP_CASES.min(5) {
        // Seed a row in A; list B; must remain empty.
        let cid_a = format!("0x{:02x}{}", i as u8, "aa".repeat(31));
        let mut row_a = seed_row(deployment_a, 84532, &cid_a);
        row_a.buyer_order_hash = format!("0x{:02x}{}", i as u8, "01".repeat(31));
        row_a.seller_order_hash = format!("0x{:02x}{}", i as u8, "02".repeat(31));
        store
            .insert_execution_request(&row_a)
            .await
            .expect("A insert");
        let list_b = store
            .list_execution_requests_by_deployment(deployment_b, 100)
            .await
            .expect("list B");
        assert!(
            list_b.iter().all(|r| r.canonical_execution_id != cid_a),
            "case {i}: deployment A's row must not surface under deployment B"
        );
    }
}
