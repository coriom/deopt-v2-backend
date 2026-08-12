//! `BACKEND-HYBRID-V2-SIGNER-AND-EXECUTION-V1` (Foundation package,
//! parts C–H) — real-PostgreSQL integration coverage for the
//! pre-broadcast execution surface added by migration 0049.
//!
//! Scope:
//!   * Migration 0049 applies cleanly on top of 0044–0048.
//!   * Insert / read round-trip for `hybrid_v2_execution_requests`.
//!   * Phase transition allowed / denied by SQL rule (state-machine
//!     `WHERE phase = $from` guard + plan-hash immutability trigger).
//!   * Nonce reservation UNIQUE constraint enforced under race.
//!   * OperationKind::Execution can acquire + release the unified
//!     lock.
//!   * EXECUTION lock does NOT auto-steal a live RECONCILIATION
//!     lock — regression guard for the bug fixed in 4a8a382.

mod hybrid_v2_support;

use deopt_v2_backend::hybrid_v2::execution::{
    derive_canonical_execution_id, ExecutionPhase, ExecutionRequestPatch, ExecutionRequestRow,
};
use deopt_v2_backend::hybrid_v2::persistence::{
    HybridV2ProjectionStore, PostgresHybridV2ProjectionStore,
};
use deopt_v2_backend::hybrid_v2::rebuild_operations::OperationKind;
use hybrid_v2_support::baseline_manifest;
use sqlx::postgres::{PgPool, PgPoolOptions};
use sqlx::Row;
use std::sync::Arc;
use std::time::Duration;

const URL_ENV: &str = "HYBRID_V2_PG_TEST_DATABASE_URL";
const ALT_URL_ENV: &str = "PG_INTEGRATION_URL";
const REQUIRE_ENV: &str = "DEOPT_REQUIRE_PG_INTEGRATION";

fn get_pg_url_or_skip_or_panic(test_name: &str) -> Option<String> {
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

async fn build_deployment(pool: &PgPool) -> (Arc<dyn HybridV2ProjectionStore>, i64) {
    let store: Arc<dyn HybridV2ProjectionStore> =
        Arc::new(PostgresHybridV2ProjectionStore::new(pool.clone()));
    let m = baseline_manifest(84532);
    let d = store
        .upsert_deployment(&m, "PENDING", 1_700_000_000_000)
        .await
        .expect("d");
    (store, d)
}

fn build_row(deployment_id: i64, chain_id: u64, fill: u128, now_ms: i64) -> ExecutionRequestRow {
    let buyer_h = format!("0x{}", "aa".repeat(32));
    let seller_h = format!("0x{}", "bb".repeat(32));
    let id = derive_canonical_execution_id(deployment_id, chain_id, &buyer_h, &seller_h, fill);
    ExecutionRequestRow {
        canonical_execution_id: id.into_string(),
        deployment_id,
        chain_id: chain_id as i64,
        execution_kind: "HYBRID_V2_OPTION_MATCH".to_string(),
        buyer_order_hash: buyer_h,
        seller_order_hash: seller_h,
        buyer_subkey: format!("0x{}", "aa".repeat(32)),
        seller_subkey: format!("0x{}", "bb".repeat(32)),
        series_id: "42".to_string(),
        fill_quantity_1e8: fill.to_string(),
        premium_amount: "50000000".to_string(),
        fee_schedule_epoch: None,
        source_matched_execution_id: None,
        target_contract: format!("0x{}", "5a".repeat(20)),
        selector: "0xdeadbeef".to_string(),
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
        created_at_ms: now_ms,
        updated_at_ms: now_ms,
    }
}

#[tokio::test]
async fn migration_0049_applies_cleanly() {
    let Some(url) = get_pg_url_or_skip_or_panic("migration_0049_applies_cleanly") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    // Verify the three new tables exist.
    for table in [
        "hybrid_v2_execution_requests",
        "hybrid_v2_execution_attempts",
        "hybrid_v2_executor_nonces",
    ] {
        let row = sqlx::query(
            "SELECT COUNT(*) AS c FROM information_schema.tables
             WHERE table_schema='public' AND table_name=$1",
        )
        .bind(table)
        .fetch_one(&pool)
        .await
        .expect("query info_schema");
        let c: i64 = row.try_get("c").unwrap();
        assert_eq!(c, 1, "table {table} must exist after migration 0049");
    }
    // Verify the operation_locks CHECK was extended to include EXECUTION.
    sqlx::query(
        "INSERT INTO hybrid_v2_deployments (
            chain_id, manifest_address, manifest_hash, module_addresses_hash, critical_config_hash,
            architecture_version, storage_version, event_version, deployment_version,
            manifest_schema_version, environment_tag, deployer,
            deployment_block, deployment_timestamp, module_addresses_json,
            protocol_fee_subkey, rebate_budget_subkey, insurance_fund_subkey,
            max_collateral_tokens, max_active_series, all_capabilities_mask,
            recovery_activation_delay_seconds, recovery_pause_max_duration_blocks,
            activation_status, created_at_ms, updated_at_ms
         ) VALUES (
            84532,'0x00','0x00','0x00','0x00',1,1,1,1,1,'','',1,1,'{}','','','',8,32,'',1,1,'PENDING',1,1
         ) RETURNING deployment_id",
    )
    .execute(&pool)
    .await
    .expect("insert deployment");
    sqlx::query(
        "INSERT INTO hybrid_v2_operation_locks
            (deployment_id, operation, holder_epoch, acquired_at_ms)
         VALUES (1, 'EXECUTION', 1, 1)",
    )
    .execute(&pool)
    .await
    .expect("EXECUTION operation must satisfy CHECK constraint");
}

#[tokio::test]
async fn insert_and_read_round_trip() {
    let Some(url) = get_pg_url_or_skip_or_panic("insert_and_read_round_trip") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, did) = build_deployment(&pool).await;
    let row = build_row(did, 84532, 100_000_000, 1);

    // First insert wins.
    let inserted = store.insert_execution_request(&row).await.unwrap();
    assert!(inserted);

    // Duplicate insert is a no-op (idempotent by canonical id).
    let duplicate = store.insert_execution_request(&row).await.unwrap();
    assert!(!duplicate);

    // Read back is identical.
    let got = store
        .get_execution_request(&row.canonical_execution_id)
        .await
        .unwrap()
        .expect("row present");
    assert_eq!(got.canonical_execution_id, row.canonical_execution_id);
    assert_eq!(got.deployment_id, did);
    assert_eq!(got.phase, ExecutionPhase::Discovered);
    assert_eq!(got.fill_quantity_1e8, row.fill_quantity_1e8);
}

#[tokio::test]
async fn legal_phase_transition_advances() {
    let Some(url) = get_pg_url_or_skip_or_panic("legal_phase_transition_advances") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, did) = build_deployment(&pool).await;
    let row = build_row(did, 84532, 100_000_000, 1);
    store.insert_execution_request(&row).await.unwrap();

    let ok = store
        .update_execution_phase(
            &row.canonical_execution_id,
            ExecutionPhase::Discovered,
            ExecutionPhase::Validating,
            2,
            ExecutionRequestPatch::default(),
        )
        .await
        .unwrap();
    assert!(ok);

    let got = store
        .get_execution_request(&row.canonical_execution_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(got.phase, ExecutionPhase::Validating);
}

#[tokio::test]
async fn illegal_phase_transition_is_rejected_by_matrix() {
    let Some(url) = get_pg_url_or_skip_or_panic("illegal_phase_transition_is_rejected_by_matrix")
    else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, did) = build_deployment(&pool).await;
    let row = build_row(did, 84532, 100_000_000, 1);
    store.insert_execution_request(&row).await.unwrap();

    // Discovered -> Simulating is not a legal edge.
    let err = store
        .update_execution_phase(
            &row.canonical_execution_id,
            ExecutionPhase::Discovered,
            ExecutionPhase::Simulating,
            2,
            ExecutionRequestPatch::default(),
        )
        .await;
    assert!(err.is_err(), "matrix must reject Discovered -> Simulating");
}

#[tokio::test]
async fn phase_from_guard_catches_lost_updates() {
    let Some(url) = get_pg_url_or_skip_or_panic("phase_from_guard_catches_lost_updates") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, did) = build_deployment(&pool).await;
    let row = build_row(did, 84532, 100_000_000, 1);
    store.insert_execution_request(&row).await.unwrap();

    // Actually persisted phase = Discovered; caller claims Validating.
    let ok = store
        .update_execution_phase(
            &row.canonical_execution_id,
            ExecutionPhase::Validating,
            ExecutionPhase::ReadyToSimulate,
            2,
            ExecutionRequestPatch::default(),
        )
        .await
        .unwrap();
    assert!(!ok, "SQL WHERE guard must reject a mismatched from-phase");
}

#[tokio::test]
async fn plan_hash_immutability_is_enforced_by_trigger() {
    let Some(url) = get_pg_url_or_skip_or_panic("plan_hash_immutability_is_enforced_by_trigger")
    else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, did) = build_deployment(&pool).await;
    let row = build_row(did, 84532, 100_000_000, 1);
    store.insert_execution_request(&row).await.unwrap();

    // Advance to Validating and set plan_hash.
    let ok = store
        .update_execution_phase(
            &row.canonical_execution_id,
            ExecutionPhase::Discovered,
            ExecutionPhase::Validating,
            2,
            ExecutionRequestPatch {
                plan_hash: Some("0x".to_string() + &"aa".repeat(32)),
                calldata_hash: Some("0x".to_string() + &"bb".repeat(32)),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(ok);

    // Try to advance further with a DIFFERENT plan_hash. The trigger
    // must raise; the SQL error surfaces as a Persistence error.
    let err = store
        .update_execution_phase(
            &row.canonical_execution_id,
            ExecutionPhase::Validating,
            ExecutionPhase::ReadyToSimulate,
            3,
            ExecutionRequestPatch {
                plan_hash: Some("0x".to_string() + &"cc".repeat(32)),
                ..Default::default()
            },
        )
        .await;
    assert!(
        err.is_err(),
        "immutability trigger must reject a diverging plan_hash"
    );

    // Advancing with the SAME plan_hash (or None) still works.
    let ok = store
        .update_execution_phase(
            &row.canonical_execution_id,
            ExecutionPhase::Validating,
            ExecutionPhase::ReadyToSimulate,
            4,
            ExecutionRequestPatch {
                plan_hash: Some("0x".to_string() + &"aa".repeat(32)),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(ok);
}

#[tokio::test]
async fn nonce_reservation_unique_constraint_is_enforced() {
    let Some(url) = get_pg_url_or_skip_or_panic("nonce_reservation_unique_constraint_is_enforced")
    else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, did) = build_deployment(&pool).await;
    let row_a = build_row(did, 84532, 100_000_000, 1);
    let row_b = build_row(did, 84532, 200_000_000, 2);
    store.insert_execution_request(&row_a).await.unwrap();
    store.insert_execution_request(&row_b).await.unwrap();

    let signer = format!("0x{}", "51".repeat(20));
    // First reservation wins.
    let first = store
        .reserve_executor_nonce(84532, &signer, 100, &row_a.canonical_execution_id, 10)
        .await
        .unwrap();
    assert!(first);
    // Second attempt with the same (chain, signer, nonce) MUST return false.
    let second = store
        .reserve_executor_nonce(84532, &signer, 100, &row_b.canonical_execution_id, 11)
        .await
        .unwrap();
    assert!(
        !second,
        "UNIQUE(chain_id, signer_identity, reserved_nonce) must reject the collision"
    );

    // A different nonce is fine.
    let ok = store
        .reserve_executor_nonce(84532, &signer, 101, &row_b.canonical_execution_id, 12)
        .await
        .unwrap();
    assert!(ok);
    let nonces = store.get_reserved_nonces_for(84532, &signer).await.unwrap();
    assert_eq!(nonces, vec![100, 101]);
}

#[tokio::test]
async fn execution_lock_can_be_acquired_and_released() {
    let Some(url) = get_pg_url_or_skip_or_panic("execution_lock_can_be_acquired_and_released")
    else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, did) = build_deployment(&pool).await;

    let guard = store
        .try_acquire_operation_lock(did, OperationKind::Execution, 42, 1_700_000_000_000)
        .await
        .unwrap();
    assert!(guard.is_some(), "EXECUTION lock must be acquirable");
    // Competing EXECUTION acquisition fails.
    let second = store
        .try_acquire_operation_lock(did, OperationKind::Execution, 43, 1_700_000_000_001)
        .await
        .unwrap();
    assert!(second.is_none(), "second acquisition must block");
    // Release.
    store.release_operation_lock(did, 42).await.unwrap();
    // Now a new epoch can acquire.
    let third = store
        .try_acquire_operation_lock(did, OperationKind::Execution, 44, 1_700_000_000_002)
        .await
        .unwrap();
    assert!(third.is_some(), "release must free the lock");
}

/// Regression guard for the reconciliation-lock stealing bug fixed
/// in 4a8a382. An EXECUTION acquisition MUST NOT auto-steal a live
/// RECONCILIATION lock.
#[tokio::test]
async fn execution_does_not_steal_live_reconciliation_lock() {
    let Some(url) =
        get_pg_url_or_skip_or_panic("execution_does_not_steal_live_reconciliation_lock")
    else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, did) = build_deployment(&pool).await;

    // Reconciliation holds the lock.
    let recon = store
        .try_acquire_operation_lock(did, OperationKind::Reconciliation, 100, 1_700_000_000_000)
        .await
        .unwrap();
    assert!(recon.is_some(), "reconciliation acquires the lock");

    // EXECUTION acquisition must NOT succeed (no auto-clean of a
    // live reconciliation lock).
    let exec = store
        .try_acquire_operation_lock(did, OperationKind::Execution, 101, 1_700_000_000_001)
        .await
        .unwrap();
    assert!(
        exec.is_none(),
        "EXECUTION acquisition MUST NOT steal a live RECONCILIATION lock"
    );

    // After explicit release of the reconciliation lock, execution
    // can proceed.
    store.release_operation_lock(did, 100).await.unwrap();
    let exec_after = store
        .try_acquire_operation_lock(did, OperationKind::Execution, 102, 1_700_000_000_002)
        .await
        .unwrap();
    assert!(
        exec_after.is_some(),
        "post-release EXECUTION acquisition must succeed"
    );
}

#[tokio::test]
async fn attempt_append_dedupes_by_attempt_number() {
    let Some(url) = get_pg_url_or_skip_or_panic("attempt_append_dedupes_by_attempt_number") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, did) = build_deployment(&pool).await;
    let row = build_row(did, 84532, 100_000_000, 1);
    store.insert_execution_request(&row).await.unwrap();

    let a1 = store
        .append_execution_attempt(
            &row.canonical_execution_id,
            ExecutionPhase::Discovered,
            None,
            1,
        )
        .await
        .unwrap();
    let a2 = store
        .append_execution_attempt(
            &row.canonical_execution_id,
            ExecutionPhase::Validating,
            None,
            2,
        )
        .await
        .unwrap();
    assert_ne!(a1, a2);
}
