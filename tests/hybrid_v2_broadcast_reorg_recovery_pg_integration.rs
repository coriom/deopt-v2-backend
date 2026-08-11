//! `BACKEND-HYBRID-V2-BROADCAST-AND-CONFIRMATION-V1` Package C (Part Q)
//! — real-PostgreSQL coverage for the [`BroadcastReorgRecovery`] driver.
//!
//! Every scenario exercises a Reorged row and asserts the six
//! deterministic outcomes: RemainedCanonical, ReminedInReplacement,
//! ReturnedToPending, DisappearedCanRebroadcast, DifferentTxConsumedNonce,
//! Unresolved. Frozen safety:
//!  * NO REAL EXTERNAL CHAIN CALL.
//!  * `mock.write_method_calls()` MUST be empty across every test —
//!    recovery never sends bytes.
//!  * `mock.non_send_write_methods()` MUST be empty — the mock exposes
//!    only `eth_sendRawTransaction` as a write.

mod hybrid_v2_broadcast_mock_rpc;
mod hybrid_v2_support;

use std::sync::Arc;
use std::time::Duration;

use alloy_primitives::U256;
use hybrid_v2_broadcast_mock_rpc::MockBroadcastRpc;
use hybrid_v2_support::baseline_manifest;
use sqlx::postgres::{PgPool, PgPoolOptions};

use deopt_v2_backend::hybrid_v2::execution::broadcast_nonce_policy::BroadcastNonceInvestigator;
use deopt_v2_backend::hybrid_v2::execution::broadcast_reorg_recovery::{
    reorg_failure_class, BroadcastReorgRecovery, ReorgRecoveryOutcome,
};
use deopt_v2_backend::hybrid_v2::execution::broadcast_rpc::{
    BroadcastRpcError, ExecutionBroadcastRpcClient, TransactionSummary, TxReceipt,
};
use deopt_v2_backend::hybrid_v2::execution::broadcast_state::{
    BroadcastPhase, BroadcastStatePatch,
};
use deopt_v2_backend::hybrid_v2::execution::persistence::ExecutionRequestRow;
use deopt_v2_backend::hybrid_v2::execution::state::ExecutionPhase;
use deopt_v2_backend::hybrid_v2::persistence::{
    HybridV2ProjectionStore, PostgresHybridV2ProjectionStore,
};

const URL_ENV: &str = "HYBRID_V2_PG_TEST_DATABASE_URL";
const ALT_URL_ENV: &str = "PG_INTEGRATION_URL";
const REQUIRE_ENV: &str = "DEOPT_REQUIRE_PG_INTEGRATION";

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
            panic!("{REQUIRE_ENV} required but no PG URL provided");
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

const CID: &str = "0xf0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0";
const OUR_HASH: [u8; 32] = [0x11u8; 32];
const SIGNER: [u8; 20] = [0x77u8; 20];
const OLD_BLOCK: u64 = 500;
const OLD_HASH: [u8; 32] = [0xAB; 32];

fn hex_of(b: &[u8]) -> String {
    let mut s = String::with_capacity(b.len() * 2);
    for x in b {
        s.push_str(&format!("{:02x}", x));
    }
    format!("0x{s}")
}

async fn seed_execution_request(store: &PostgresHybridV2ProjectionStore, reserved_nonce: i64) {
    let row = ExecutionRequestRow {
        canonical_execution_id: CID.into(),
        deployment_id: 1,
        chain_id: 84532,
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
        target_contract: format!("0x{}", "ee".repeat(20)),
        selector: "0x00000000".into(),
        calldata_hash: Some(format!("0x{}", "cd".repeat(32))),
        plan_hash: Some(format!("0x{}", "ee".repeat(32))),
        tx_value_wei: "0".into(),
        simulation_block_number: Some(1),
        simulation_block_hash: Some(format!("0x{}", "cc".repeat(32))),
        simulation_gas_estimate: Some(500_000),
        simulation_result_json: Some(serde_json::json!({})),
        signer_identity: Some(hex_of(&SIGNER)),
        signing_payload_hash: Some(format!("0x{}", "ff".repeat(32))),
        signature_r: Some(format!("0x{}", "11".repeat(32))),
        signature_s: Some(format!("0x{}", "22".repeat(32))),
        signature_v: Some(0),
        recovered_signer: Some(hex_of(&SIGNER)),
        gas_limit: Some(1_000_000),
        max_fee_per_gas_wei: Some("2000000000".into()),
        max_priority_fee_per_gas_wei: Some("500000000".into()),
        reserved_nonce: Some(reserved_nonce),
        phase: ExecutionPhase::SignatureVerified,
        failure_class: None,
        failure_detail: None,
        retry_count: 0,
        holder_epoch: None,
        signer_request_idempotency_key: None,
        created_at_ms: 1,
        updated_at_ms: 1,
    };
    let dyn_store: &dyn HybridV2ProjectionStore = store;
    dyn_store.insert_execution_request(&row).await.unwrap();
}

async fn install_reorged_row(store: &PostgresHybridV2ProjectionStore) {
    let dyn_store: &dyn HybridV2ProjectionStore = store;
    dyn_store.insert_broadcast_state(CID, 1_000).await.unwrap();
    let hex = hex_of(&OUR_HASH);
    dyn_store
        .set_broadcast_tx_hash(CID, &hex, &hex, &hex, 1_001)
        .await
        .unwrap();
    dyn_store
        .update_broadcast_phase(
            CID,
            BroadcastPhase::BroadcastDisabled,
            BroadcastPhase::Broadcasting,
            1_002,
            BroadcastStatePatch {
                submission_attempt_count: Some(1),
                first_submission_at_ms: Some(1_002),
                last_submission_at_ms: Some(1_002),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    dyn_store
        .update_broadcast_phase(
            CID,
            BroadcastPhase::Broadcasting,
            BroadcastPhase::Submitted,
            1_003,
            BroadcastStatePatch::default(),
        )
        .await
        .unwrap();
    dyn_store
        .update_broadcast_phase(
            CID,
            BroadcastPhase::Submitted,
            BroadcastPhase::MinedSuccess,
            1_004,
            BroadcastStatePatch {
                receipt_tx_hash: Some(hex_of(&OUR_HASH)),
                receipt_block_number: Some(OLD_BLOCK as i64),
                receipt_block_hash: Some(hex_of(&OLD_HASH)),
                receipt_status: Some(1),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    dyn_store
        .update_broadcast_phase(
            CID,
            BroadcastPhase::MinedSuccess,
            BroadcastPhase::Reorged,
            1_005,
            BroadcastStatePatch {
                canonicality_state: Some("ORPHANED".into()),
                reorg_count: Some(1),
                ..Default::default()
            },
        )
        .await
        .unwrap();
}

async fn build_store(pool: &PgPool) -> Arc<PostgresHybridV2ProjectionStore> {
    let store = Arc::new(PostgresHybridV2ProjectionStore::new(pool.clone()));
    let manifest = baseline_manifest(84532);
    let dyn_store: &dyn HybridV2ProjectionStore = store.as_ref();
    dyn_store
        .upsert_deployment(&manifest, "PENDING", 1_700_000_000_000)
        .await
        .unwrap();
    store
}

// -----------------------------------------------------------------
//                            TESTS
// -----------------------------------------------------------------

#[tokio::test]
async fn remined_in_replacement_transitions_reorged_to_confirming() {
    let Some(url) = get_pg_url_or_skip("remined_in_replacement_transitions_reorged_to_confirming")
    else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let store = build_store(&pool).await;
    seed_execution_request(&store, 42).await;
    install_reorged_row(&store).await;

    let rpc = Arc::new(MockBroadcastRpc::new());
    rpc.set_receipt_response(
        OUR_HASH,
        Some(TxReceipt {
            tx_hash: OUR_HASH,
            block_number: 600,
            block_hash: [0xCD; 32],
            status: 1,
            gas_used: 400_000,
            effective_gas_price_wei: U256::from(1_000_000_000u64),
            cumulative_gas_used: 400_000,
            from: SIGNER,
            to: Some([0xEE; 20]),
        }),
    );
    rpc.set_block_hash_at(600, [0xCD; 32]);
    let rpc_dyn: Arc<dyn ExecutionBroadcastRpcClient> = rpc.clone();
    let dyn_store: &dyn HybridV2ProjectionStore = store.as_ref();
    let investigator = BroadcastNonceInvestigator {
        store: dyn_store,
        rpc: rpc_dyn.as_ref(),
        executor_address: SIGNER,
        chain_id: 84532,
    };
    let recovery = BroadcastReorgRecovery {
        store: dyn_store,
        rpc: rpc_dyn.as_ref(),
        nonce_investigator: &investigator,
        deployment_id: 1,
        chain_id: 84532,
        executor_address: SIGNER,
    };
    let outcome = recovery.recover(CID).await.expect("recover");
    assert_eq!(
        outcome,
        ReorgRecoveryOutcome::ReminedInReplacement {
            block_number: 600,
            block_hash: [0xCD; 32],
        }
    );
    let row = dyn_store.get_broadcast_state(CID).await.unwrap().unwrap();
    assert_eq!(row.phase, BroadcastPhase::Confirming);
    assert_eq!(row.reorg_count, 2);
    assert_eq!(row.confirmation_count, 0);
    assert!(rpc.write_method_calls().is_empty());
    assert!(rpc.non_send_write_methods().is_empty());
}

#[tokio::test]
async fn remained_canonical_leaves_row_untouched() {
    let Some(url) = get_pg_url_or_skip("remained_canonical_leaves_row_untouched") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let store = build_store(&pool).await;
    seed_execution_request(&store, 42).await;
    install_reorged_row(&store).await;

    // Original block is still canonical (false alarm).
    let rpc = Arc::new(MockBroadcastRpc::new());
    rpc.set_receipt_response(
        OUR_HASH,
        Some(TxReceipt {
            tx_hash: OUR_HASH,
            block_number: OLD_BLOCK,
            block_hash: OLD_HASH,
            status: 1,
            gas_used: 400_000,
            effective_gas_price_wei: U256::from(1_000_000_000u64),
            cumulative_gas_used: 400_000,
            from: SIGNER,
            to: Some([0xEE; 20]),
        }),
    );
    rpc.set_block_hash_at(OLD_BLOCK, OLD_HASH);
    let rpc_dyn: Arc<dyn ExecutionBroadcastRpcClient> = rpc.clone();
    let dyn_store: &dyn HybridV2ProjectionStore = store.as_ref();
    let investigator = BroadcastNonceInvestigator {
        store: dyn_store,
        rpc: rpc_dyn.as_ref(),
        executor_address: SIGNER,
        chain_id: 84532,
    };
    let recovery = BroadcastReorgRecovery {
        store: dyn_store,
        rpc: rpc_dyn.as_ref(),
        nonce_investigator: &investigator,
        deployment_id: 1,
        chain_id: 84532,
        executor_address: SIGNER,
    };
    let outcome = recovery.recover(CID).await.expect("recover");
    assert_eq!(outcome, ReorgRecoveryOutcome::RemainedCanonical);
    let row = dyn_store.get_broadcast_state(CID).await.unwrap().unwrap();
    assert_eq!(row.phase, BroadcastPhase::Reorged);
    assert!(rpc.write_method_calls().is_empty());
}

#[tokio::test]
async fn returned_to_pending_when_receipt_absent_but_tx_in_mempool() {
    let Some(url) = get_pg_url_or_skip("returned_to_pending_when_receipt_absent_but_tx_in_mempool")
    else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let store = build_store(&pool).await;
    seed_execution_request(&store, 42).await;
    install_reorged_row(&store).await;

    let rpc = Arc::new(MockBroadcastRpc::new());
    // No receipt; tx is known to mempool without a block.
    rpc.set_transaction_response(
        OUR_HASH,
        Some(TransactionSummary {
            tx_hash: OUR_HASH,
            from: SIGNER,
            to: Some([0xEE; 20]),
            nonce: 42,
            block_number: None,
            block_hash: None,
            value_wei: U256::ZERO,
            input_bytes_len: 4,
            input_hash: None,
            max_fee_per_gas: None,
            max_priority_fee_per_gas: None,
            tx_type: 2,
        }),
    );
    let rpc_dyn: Arc<dyn ExecutionBroadcastRpcClient> = rpc.clone();
    let dyn_store: &dyn HybridV2ProjectionStore = store.as_ref();
    let investigator = BroadcastNonceInvestigator {
        store: dyn_store,
        rpc: rpc_dyn.as_ref(),
        executor_address: SIGNER,
        chain_id: 84532,
    };
    let recovery = BroadcastReorgRecovery {
        store: dyn_store,
        rpc: rpc_dyn.as_ref(),
        nonce_investigator: &investigator,
        deployment_id: 1,
        chain_id: 84532,
        executor_address: SIGNER,
    };
    let outcome = recovery.recover(CID).await.expect("recover");
    assert_eq!(outcome, ReorgRecoveryOutcome::ReturnedToPending);
    let row = dyn_store.get_broadcast_state(CID).await.unwrap().unwrap();
    assert_eq!(row.phase, BroadcastPhase::Pending);
    assert!(rpc.write_method_calls().is_empty());
}

#[tokio::test]
async fn disappeared_can_rebroadcast_when_nonce_released() {
    let Some(url) = get_pg_url_or_skip("disappeared_can_rebroadcast_when_nonce_released") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let store = build_store(&pool).await;
    seed_execution_request(&store, 42).await;
    install_reorged_row(&store).await;

    let rpc = Arc::new(MockBroadcastRpc::new());
    // No receipt, no mempool entry, pending_nonce < our_nonce.
    rpc.set_transaction_count(
        SIGNER,
        deopt_v2_backend::hybrid_v2::execution::rpc::BlockTag::Pending,
        41,
    );
    let rpc_dyn: Arc<dyn ExecutionBroadcastRpcClient> = rpc.clone();
    let dyn_store: &dyn HybridV2ProjectionStore = store.as_ref();
    let investigator = BroadcastNonceInvestigator {
        store: dyn_store,
        rpc: rpc_dyn.as_ref(),
        executor_address: SIGNER,
        chain_id: 84532,
    };
    let recovery = BroadcastReorgRecovery {
        store: dyn_store,
        rpc: rpc_dyn.as_ref(),
        nonce_investigator: &investigator,
        deployment_id: 1,
        chain_id: 84532,
        executor_address: SIGNER,
    };
    let outcome = recovery.recover(CID).await.expect("recover");
    assert_eq!(outcome, ReorgRecoveryOutcome::DisappearedCanRebroadcast);
    // Frozen safety: row remains Reorged.
    let row = dyn_store.get_broadcast_state(CID).await.unwrap().unwrap();
    assert_eq!(row.phase, BroadcastPhase::Reorged);
    assert!(rpc.write_method_calls().is_empty());
}

#[tokio::test]
async fn different_tx_consumed_nonce_escalates_to_manual_intervention() {
    let Some(url) =
        get_pg_url_or_skip("different_tx_consumed_nonce_escalates_to_manual_intervention")
    else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let store = build_store(&pool).await;
    seed_execution_request(&store, 42).await;
    install_reorged_row(&store).await;

    let rpc = Arc::new(MockBroadcastRpc::new());
    // pending_nonce > our_nonce → DifferentTxConsumedNonce.
    rpc.set_transaction_count(
        SIGNER,
        deopt_v2_backend::hybrid_v2::execution::rpc::BlockTag::Pending,
        43,
    );
    let rpc_dyn: Arc<dyn ExecutionBroadcastRpcClient> = rpc.clone();
    let dyn_store: &dyn HybridV2ProjectionStore = store.as_ref();
    let investigator = BroadcastNonceInvestigator {
        store: dyn_store,
        rpc: rpc_dyn.as_ref(),
        executor_address: SIGNER,
        chain_id: 84532,
    };
    let recovery = BroadcastReorgRecovery {
        store: dyn_store,
        rpc: rpc_dyn.as_ref(),
        nonce_investigator: &investigator,
        deployment_id: 1,
        chain_id: 84532,
        executor_address: SIGNER,
    };
    let outcome = recovery.recover(CID).await.expect("recover");
    assert!(matches!(
        outcome,
        ReorgRecoveryOutcome::DifferentTxConsumedNonce { .. }
    ));
    let row = dyn_store.get_broadcast_state(CID).await.unwrap().unwrap();
    assert_eq!(row.phase, BroadcastPhase::ManualInterventionRequired);
    assert_eq!(
        row.failure_class.as_deref(),
        Some(reorg_failure_class::REORG_NONCE_STOLEN)
    );
    assert!(rpc.write_method_calls().is_empty());
}

#[tokio::test]
async fn unresolved_when_investigator_returns_ambiguous() {
    let Some(url) = get_pg_url_or_skip("unresolved_when_investigator_returns_ambiguous") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let store = build_store(&pool).await;
    seed_execution_request(&store, 42).await;
    install_reorged_row(&store).await;

    let rpc = Arc::new(MockBroadcastRpc::new());
    // pending_nonce == our_nonce with no mempool observation → Ambiguous.
    rpc.set_transaction_count(
        SIGNER,
        deopt_v2_backend::hybrid_v2::execution::rpc::BlockTag::Pending,
        42,
    );
    let rpc_dyn: Arc<dyn ExecutionBroadcastRpcClient> = rpc.clone();
    let dyn_store: &dyn HybridV2ProjectionStore = store.as_ref();
    let investigator = BroadcastNonceInvestigator {
        store: dyn_store,
        rpc: rpc_dyn.as_ref(),
        executor_address: SIGNER,
        chain_id: 84532,
    };
    let recovery = BroadcastReorgRecovery {
        store: dyn_store,
        rpc: rpc_dyn.as_ref(),
        nonce_investigator: &investigator,
        deployment_id: 1,
        chain_id: 84532,
        executor_address: SIGNER,
    };
    let outcome = recovery.recover(CID).await.expect("recover");
    assert_eq!(outcome, ReorgRecoveryOutcome::Unresolved);
    let row = dyn_store.get_broadcast_state(CID).await.unwrap().unwrap();
    assert_eq!(row.phase, BroadcastPhase::Reorged);
    assert!(rpc.write_method_calls().is_empty());
}

#[tokio::test]
async fn non_reorged_row_is_a_noop() {
    let Some(url) = get_pg_url_or_skip("non_reorged_row_is_a_noop") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let store = build_store(&pool).await;
    seed_execution_request(&store, 42).await;
    let dyn_store: &dyn HybridV2ProjectionStore = store.as_ref();
    // Row exists but is at BROADCAST_DISABLED — recovery should NO-OP.
    dyn_store.insert_broadcast_state(CID, 1_000).await.unwrap();
    let rpc = Arc::new(MockBroadcastRpc::new());
    let rpc_dyn: Arc<dyn ExecutionBroadcastRpcClient> = rpc.clone();
    let investigator = BroadcastNonceInvestigator {
        store: dyn_store,
        rpc: rpc_dyn.as_ref(),
        executor_address: SIGNER,
        chain_id: 84532,
    };
    let recovery = BroadcastReorgRecovery {
        store: dyn_store,
        rpc: rpc_dyn.as_ref(),
        nonce_investigator: &investigator,
        deployment_id: 1,
        chain_id: 84532,
        executor_address: SIGNER,
    };
    let outcome = recovery.recover(CID).await.expect("recover");
    assert_eq!(outcome, ReorgRecoveryOutcome::RemainedCanonical);
    let row = dyn_store.get_broadcast_state(CID).await.unwrap().unwrap();
    assert_eq!(row.phase, BroadcastPhase::BroadcastDisabled);
    assert!(rpc.write_method_calls().is_empty());
}

#[tokio::test]
async fn transient_rpc_error_surfaces_as_recoverable() {
    let Some(url) = get_pg_url_or_skip("transient_rpc_error_surfaces_as_recoverable") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let store = build_store(&pool).await;
    seed_execution_request(&store, 42).await;
    install_reorged_row(&store).await;

    let rpc = Arc::new(MockBroadcastRpc::new());
    // First receipt lookup returns 5xx.
    rpc.set_provider_5xx_next(1);
    let rpc_dyn: Arc<dyn ExecutionBroadcastRpcClient> = rpc.clone();
    let dyn_store: &dyn HybridV2ProjectionStore = store.as_ref();
    let investigator = BroadcastNonceInvestigator {
        store: dyn_store,
        rpc: rpc_dyn.as_ref(),
        executor_address: SIGNER,
        chain_id: 84532,
    };
    let recovery = BroadcastReorgRecovery {
        store: dyn_store,
        rpc: rpc_dyn.as_ref(),
        nonce_investigator: &investigator,
        deployment_id: 1,
        chain_id: 84532,
        executor_address: SIGNER,
    };
    let err = recovery.recover(CID).await.expect_err("rpc failure");
    assert!(matches!(
        err,
        deopt_v2_backend::hybrid_v2::execution::broadcast_reorg_recovery::ReorgRecoveryError::RpcFailure(_)
    ));
    // Row unchanged.
    let row = dyn_store.get_broadcast_state(CID).await.unwrap().unwrap();
    assert_eq!(row.phase, BroadcastPhase::Reorged);
    assert!(rpc.write_method_calls().is_empty());
}

#[tokio::test]
async fn recovery_never_calls_send_raw_transaction() {
    // Explicit invariant test: across every outcome branch above the mock's
    // write_method_calls() is empty. Here we exercise the receipt-present
    // canonical branch and assert the invariant one more time — providing
    // a single grep target.
    let Some(url) = get_pg_url_or_skip("recovery_never_calls_send_raw_transaction") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let store = build_store(&pool).await;
    seed_execution_request(&store, 42).await;
    install_reorged_row(&store).await;
    let rpc = Arc::new(MockBroadcastRpc::new());
    rpc.set_receipt_response(
        OUR_HASH,
        Some(TxReceipt {
            tx_hash: OUR_HASH,
            block_number: 700,
            block_hash: [0xDE; 32],
            status: 1,
            gas_used: 400_000,
            effective_gas_price_wei: U256::from(1_000_000_000u64),
            cumulative_gas_used: 400_000,
            from: SIGNER,
            to: Some([0xEE; 20]),
        }),
    );
    rpc.set_block_hash_at(700, [0xDE; 32]);
    let rpc_dyn: Arc<dyn ExecutionBroadcastRpcClient> = rpc.clone();
    let dyn_store: &dyn HybridV2ProjectionStore = store.as_ref();
    let investigator = BroadcastNonceInvestigator {
        store: dyn_store,
        rpc: rpc_dyn.as_ref(),
        executor_address: SIGNER,
        chain_id: 84532,
    };
    let recovery = BroadcastReorgRecovery {
        store: dyn_store,
        rpc: rpc_dyn.as_ref(),
        nonce_investigator: &investigator,
        deployment_id: 1,
        chain_id: 84532,
        executor_address: SIGNER,
    };
    let _ = recovery.recover(CID).await.expect("recover");
    assert!(rpc.write_method_calls().is_empty());
    assert!(rpc.non_send_write_methods().is_empty());
    // Every downstream test binary agrees on this invariant.
    let _: BroadcastRpcError = BroadcastRpcError::Timeout; // suppress unused import
}
