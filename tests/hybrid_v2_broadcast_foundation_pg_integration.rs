//! `BACKEND-HYBRID-V2-BROADCAST-AND-CONFIRMATION-V1` (Package A,
//! Parts C–I) — real-PostgreSQL coverage for the broadcast foundation:
//! migration 0051, hybrid_v2_broadcast_state row insert / update /
//! immutability trigger, phase transition matrix, and the outbox
//! happy-path / hash-mismatch / timeout / disabled-firewall / Base
//! mainnet refusal / write-methods-are-only-sendRawTransaction flows.
//!
//! Every test uses a deterministic in-process mock RPC. NO REAL
//! EXTERNAL CHAIN TRANSACTION IS EVER BROADCAST.

mod hybrid_v2_support;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use alloy_primitives::U256;
use async_trait::async_trait;
use hybrid_v2_support::baseline_manifest;
use sqlx::postgres::{PgPool, PgPoolOptions};

use alloy_sol_types::SolCall;
use deopt_v2_backend::hybrid_v2::execution::broadcast_firewall::{
    BroadcastFirewallConfig, BroadcastPolicyFirewall,
};
use deopt_v2_backend::hybrid_v2::execution::broadcast_outbox::{
    failure_class as bfc, provider_classification as pc, BroadcastOutbox,
};
use deopt_v2_backend::hybrid_v2::execution::broadcast_rpc::{
    BlockHeader, BroadcastRpcError, ExecutionBroadcastRpcClient, SendOutcome, TransactionSummary,
    TxReceipt,
};
use deopt_v2_backend::hybrid_v2::execution::broadcast_state::{
    BroadcastPhase, BroadcastStatePatch,
};
use deopt_v2_backend::hybrid_v2::execution::identity::CanonicalExecutionId;
use deopt_v2_backend::hybrid_v2::execution::orchestrator::MockClock;
use deopt_v2_backend::hybrid_v2::execution::persistence::ExecutionRequestRow;
use deopt_v2_backend::hybrid_v2::execution::plan::executeMatchCall;
use deopt_v2_backend::hybrid_v2::execution::rpc::BlockTag;
use deopt_v2_backend::hybrid_v2::execution::signer::SignedTx;
use deopt_v2_backend::hybrid_v2::execution::state::ExecutionPhase;
use deopt_v2_backend::hybrid_v2::execution::target_policy::TargetPolicy;
use deopt_v2_backend::hybrid_v2::execution::tx_serialization::serialize_signed_execution;
use deopt_v2_backend::hybrid_v2::execution::{ExecutionPlan, GasFeePolicy};
use deopt_v2_backend::hybrid_v2::persistence::{
    HybridV2ProjectionStore, PostgresHybridV2ProjectionStore,
};
use deopt_v2_backend::hybrid_v2::readiness::{ReadinessReport, ReadinessState};

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

// ------------------------- fixture builders ------------------------

// Must match `baseline_manifest`'s `option_matching_engine` address so
// the TargetPolicy allowlist accepts the plan's target.
const ENGINE_HEX: &str = "0x0000000000000000000000000000000000000006";

fn engine_bytes() -> [u8; 20] {
    let mut out = [0u8; 20];
    let s = ENGINE_HEX.trim_start_matches("0x");
    for i in 0..20 {
        out[i] = u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap();
    }
    out
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{:02x}", b));
    }
    s
}

fn make_plan(chain_id: u64, canonical_id: &str) -> ExecutionPlan {
    let calldata = vec![0xde, 0xad, 0xbe, 0xef];
    let calldata_hash = {
        use sha3::{Digest, Keccak256};
        let mut h = [0u8; 32];
        let d = Keccak256::digest(&calldata);
        h.copy_from_slice(&d[..]);
        h
    };
    ExecutionPlan {
        canonical_execution_id: CanonicalExecutionId(canonical_id.to_string()),
        chain_id,
        deployment_id: 1,
        target: engine_bytes(),
        selector: executeMatchCall::SELECTOR,
        calldata,
        calldata_hash,
        value_wei: U256::ZERO,
        expected_module_version: "OptionMatchingEngineV2".into(),
        deadline_ms: None,
        plan_hash: [0xee; 32],
    }
}

fn make_row(
    chain_id: u64,
    plan: &ExecutionPlan,
    signer: [u8; 20],
    deployment_id: i64,
) -> ExecutionRequestRow {
    ExecutionRequestRow {
        canonical_execution_id: plan.canonical_execution_id.as_str().to_string(),
        deployment_id,
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
        target_contract: format!("0x{}", hex_encode(&plan.target)),
        selector: format!("0x{}", hex_encode(&plan.selector)),
        calldata_hash: Some(format!("0x{}", hex_encode(&plan.calldata_hash))),
        calldata_bytes: None,
        plan_hash: Some(format!("0x{}", hex_encode(&plan.plan_hash))),
        tx_value_wei: "0".into(),
        simulation_block_number: Some(100),
        simulation_block_hash: Some(format!("0x{}", "cc".repeat(32))),
        simulation_gas_estimate: Some(500_000),
        simulation_result_json: Some(serde_json::json!({})),
        signer_identity: Some(format!("0x{}", hex_encode(&signer))),
        signing_payload_hash: Some(format!("0x{}", "ff".repeat(32))),
        signature_r: Some(format!("0x{}", "11".repeat(32))),
        signature_s: Some(format!("0x{}", "22".repeat(32))),
        signature_v: Some(0),
        recovered_signer: Some(format!("0x{}", hex_encode(&signer))),
        gas_limit: Some(1_000_000),
        max_fee_per_gas_wei: Some("2000000000".into()),
        max_priority_fee_per_gas_wei: Some("500000000".into()),
        reserved_nonce: Some(42),
        phase: ExecutionPhase::SignatureVerified,
        failure_class: None,
        failure_detail: None,
        retry_count: 0,
        holder_epoch: None,
        signer_request_idempotency_key: None,
        created_at_ms: 1,
        updated_at_ms: 1,
    }
}

fn make_signed(signer: [u8; 20]) -> SignedTx {
    SignedTx {
        signature_r: [0x11; 32],
        signature_s: [0x22; 32],
        signature_v: 0,
        recovered_signer: signer,
        tx_type: 2,
    }
}

fn make_readiness_ready() -> ReadinessReport {
    ReadinessReport {
        runtime: ReadinessState::ready(),
        rebuild: ReadinessState::ready(),
        reconciliation: ReadinessState::ready(),
    }
}

fn gas_policy() -> GasFeePolicy {
    GasFeePolicy {
        max_gas_limit: 5_000_000,
        gas_limit_multiplier_bps: 12_000,
        max_fee_per_gas_wei: U256::from(10_000_000_000u64),
        max_priority_fee_per_gas_wei: U256::from(2_000_000_000u64),
        max_total_native_cost_wei: U256::from(10u64).pow(U256::from(18u64)),
        abnormal_estimate_reject_threshold: 10,
    }
}

// ------------------------- mock RPC --------------------------------

#[derive(Default)]
struct MockRpc {
    inner: Mutex<MockRpcInner>,
}

#[derive(Default)]
struct MockRpcInner {
    send_responses: Vec<Result<SendOutcome, BroadcastRpcError>>,
    written_methods: Vec<&'static str>,
}

impl MockRpc {
    fn accept_with(hash: [u8; 32]) -> Arc<Self> {
        let inner = Mutex::new(MockRpcInner {
            send_responses: vec![Ok(SendOutcome::Accepted {
                provider_tx_hash: hash,
            })],
            written_methods: Vec::new(),
        });
        Arc::new(Self { inner })
    }

    fn with_response(res: Result<SendOutcome, BroadcastRpcError>) -> Arc<Self> {
        let inner = Mutex::new(MockRpcInner {
            send_responses: vec![res],
            written_methods: Vec::new(),
        });
        Arc::new(Self { inner })
    }

    fn recorded_write_methods(&self) -> Vec<&'static str> {
        self.inner.lock().unwrap().written_methods.clone()
    }
}

#[async_trait]
impl ExecutionBroadcastRpcClient for MockRpc {
    async fn chain_id(&self) -> Result<u64, BroadcastRpcError> {
        Ok(84532)
    }
    async fn head_block_number(&self) -> Result<u64, BroadcastRpcError> {
        Ok(100)
    }
    async fn finalized_block_number(&self) -> Result<Option<u64>, BroadcastRpcError> {
        Ok(Some(50))
    }
    async fn transaction_count(
        &self,
        _address: [u8; 20],
        _block_tag: BlockTag,
    ) -> Result<u64, BroadcastRpcError> {
        Ok(0)
    }
    async fn transaction_by_hash(
        &self,
        _tx_hash: [u8; 32],
    ) -> Result<Option<TransactionSummary>, BroadcastRpcError> {
        Ok(None)
    }
    async fn receipt_by_hash(
        &self,
        _tx_hash: [u8; 32],
    ) -> Result<Option<TxReceipt>, BroadcastRpcError> {
        Ok(None)
    }
    async fn block_header_by_number(
        &self,
        _number: u64,
    ) -> Result<Option<BlockHeader>, BroadcastRpcError> {
        Ok(None)
    }
    async fn block_header_by_hash(
        &self,
        _hash: [u8; 32],
    ) -> Result<Option<BlockHeader>, BroadcastRpcError> {
        Ok(None)
    }
    async fn send_raw_transaction(
        &self,
        _raw_tx_bytes: &[u8],
    ) -> Result<SendOutcome, BroadcastRpcError> {
        let mut g = self.inner.lock().unwrap();
        g.written_methods.push("eth_sendRawTransaction");
        g.send_responses.remove(0)
    }
}

// ------------------------- test scaffolding -----------------------

async fn build_store(pool: &PgPool) -> (Arc<PostgresHybridV2ProjectionStore>, i64) {
    let store = Arc::new(PostgresHybridV2ProjectionStore::new(pool.clone()));
    let manifest = baseline_manifest(84532);
    let dyn_store: &dyn HybridV2ProjectionStore = store.as_ref();
    let deployment_id = dyn_store
        .upsert_deployment(&manifest, "PENDING", 1_700_000_000_000)
        .await
        .expect("upsert deployment");
    (store, deployment_id)
}

// -----------------------------------------------------------------
//                             TESTS
// -----------------------------------------------------------------

#[tokio::test]
async fn migration_0051_applies_and_insert_is_idempotent() {
    let Some(url) = get_pg_url_or_skip("migration_0051_applies_and_insert_is_idempotent") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, deployment_id) = build_store(&pool).await;
    let signer = [0x77u8; 20];
    let plan = make_plan(84532, &format!("0x{}", "aa".repeat(32)));
    let row = make_row(84532, &plan, signer, deployment_id);
    let dyn_store: &dyn HybridV2ProjectionStore = store.as_ref();
    dyn_store.insert_execution_request(&row).await.unwrap();

    // First insert returns true, second false.
    let first = dyn_store
        .insert_broadcast_state(&row.canonical_execution_id, 100)
        .await
        .expect("first insert");
    let second = dyn_store
        .insert_broadcast_state(&row.canonical_execution_id, 200)
        .await
        .expect("second insert");
    assert!(first);
    assert!(!second);

    let fetched = dyn_store
        .get_broadcast_state(&row.canonical_execution_id)
        .await
        .unwrap()
        .expect("row exists");
    assert_eq!(fetched.phase, BroadcastPhase::BroadcastDisabled);
    assert_eq!(fetched.tx_hash, None);
    assert_eq!(fetched.submission_attempt_count, 0);
}

#[tokio::test]
async fn tx_hash_immutability_trigger_rejects_divergent_overwrite() {
    let Some(url) = get_pg_url_or_skip("tx_hash_immutability_trigger_rejects_divergent_overwrite")
    else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, deployment_id) = build_store(&pool).await;
    let signer = [0x77u8; 20];
    let plan = make_plan(84532, &format!("0x{}", "aa".repeat(32)));
    let row = make_row(84532, &plan, signer, deployment_id);
    let dyn_store: &dyn HybridV2ProjectionStore = store.as_ref();
    dyn_store.insert_execution_request(&row).await.unwrap();
    dyn_store
        .insert_broadcast_state(&row.canonical_execution_id, 1)
        .await
        .unwrap();
    let tx_hash_a = format!("0x{}", "01".repeat(32));
    let tx_hash_b = format!("0x{}", "02".repeat(32));

    dyn_store
        .set_broadcast_tx_hash(
            &row.canonical_execution_id,
            &tx_hash_a,
            &tx_hash_a,
            &tx_hash_a,
            2,
        )
        .await
        .expect("first set");

    // Idempotent re-set with the SAME hash succeeds.
    dyn_store
        .set_broadcast_tx_hash(
            &row.canonical_execution_id,
            &tx_hash_a,
            &tx_hash_a,
            &tx_hash_a,
            3,
        )
        .await
        .expect("idempotent re-set");

    // Divergent overwrite FAILS.
    let err = dyn_store
        .set_broadcast_tx_hash(
            &row.canonical_execution_id,
            &tx_hash_b,
            &tx_hash_b,
            &tx_hash_b,
            4,
        )
        .await
        .expect_err("must reject divergent overwrite");
    let s = err.to_string();
    assert!(
        s.contains("immutable") || s.contains("no matching row"),
        "expected immutability error, got: {s}"
    );
}

#[tokio::test]
async fn phase_transitions_respect_matrix() {
    let Some(url) = get_pg_url_or_skip("phase_transitions_respect_matrix") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, deployment_id) = build_store(&pool).await;
    let signer = [0x77u8; 20];
    let plan = make_plan(84532, &format!("0x{}", "aa".repeat(32)));
    let row = make_row(84532, &plan, signer, deployment_id);
    let dyn_store: &dyn HybridV2ProjectionStore = store.as_ref();
    dyn_store.insert_execution_request(&row).await.unwrap();
    dyn_store
        .insert_broadcast_state(&row.canonical_execution_id, 1)
        .await
        .unwrap();

    // Legal: BROADCAST_DISABLED -> READY_FOR_BROADCAST.
    let ok = dyn_store
        .update_broadcast_phase(
            &row.canonical_execution_id,
            BroadcastPhase::BroadcastDisabled,
            BroadcastPhase::ReadyForBroadcast,
            2,
            Default::default(),
        )
        .await
        .unwrap();
    assert!(ok);

    // Illegal: READY_FOR_BROADCAST -> SUBMITTED (must go through Broadcasting).
    let err = dyn_store
        .update_broadcast_phase(
            &row.canonical_execution_id,
            BroadcastPhase::ReadyForBroadcast,
            BroadcastPhase::Submitted,
            3,
            Default::default(),
        )
        .await
        .expect_err("must reject illegal transition");
    assert!(err
        .to_string()
        .contains("illegal broadcast phase transition"));

    // Legal: READY_FOR_BROADCAST -> Broadcasting.
    dyn_store
        .update_broadcast_phase(
            &row.canonical_execution_id,
            BroadcastPhase::ReadyForBroadcast,
            BroadcastPhase::Broadcasting,
            4,
            Default::default(),
        )
        .await
        .unwrap();

    // Legal: Broadcasting -> Submitted.
    dyn_store
        .update_broadcast_phase(
            &row.canonical_execution_id,
            BroadcastPhase::Broadcasting,
            BroadcastPhase::Submitted,
            5,
            BroadcastStatePatch {
                provider_classification: Some(pc::ACCEPTED.to_string()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
}

async fn seed_row_for_broadcast(
    store: &PostgresHybridV2ProjectionStore,
    deployment_id: i64,
    signer: [u8; 20],
    canonical_id: &str,
) -> (ExecutionRequestRow, ExecutionPlan) {
    let plan = make_plan(84532, canonical_id);
    let row = make_row(84532, &plan, signer, deployment_id);
    let dyn_store: &dyn HybridV2ProjectionStore = store;
    dyn_store.insert_execution_request(&row).await.unwrap();
    (row, plan)
}

fn build_outbox(
    store: Arc<PostgresHybridV2ProjectionStore>,
    rpc: Arc<dyn ExecutionBroadcastRpcClient>,
    clock: Arc<MockClock>,
    deployment_id: i64,
) -> BroadcastOutbox {
    let store_dyn: Arc<dyn HybridV2ProjectionStore> = store;
    BroadcastOutbox {
        store: store_dyn,
        rpc,
        clock,
        deployment_id,
    }
}

#[tokio::test]
async fn outbox_happy_path_submits_and_records_hash() {
    let Some(url) = get_pg_url_or_skip("outbox_happy_path_submits_and_records_hash") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, deployment_id) = build_store(&pool).await;
    let signer = [0x77u8; 20];
    let cid = format!("0x{}", "aa".repeat(32));
    let (row, plan) = seed_row_for_broadcast(&store, deployment_id, signer, &cid).await;
    let signed = make_signed(signer);
    let env = serialize_signed_execution(
        &plan,
        &signed,
        42,
        1_000_000,
        U256::from(2_000_000_000u64),
        U256::from(500_000_000u64),
    )
    .unwrap();
    let rpc: Arc<dyn ExecutionBroadcastRpcClient> = MockRpc::accept_with(env.envelope_hash);
    let clock = Arc::new(MockClock::new(1_000));
    let outbox = build_outbox(store.clone(), rpc.clone(), clock, deployment_id);

    let target_policy = TargetPolicy::from_manifest(&baseline_manifest(84532)).unwrap();
    let gp = gas_policy();
    let allowed = [84532u64];
    let store_dyn: &dyn HybridV2ProjectionStore = store.as_ref();
    let firewall = BroadcastPolicyFirewall {
        store: store_dyn,
        target_policy: &target_policy,
        gas_policy: &gp,
        broadcast_config: BroadcastFirewallConfig {
            broadcast_enabled: true,
            pre_send_hash_probe: false,
        },
        configured_chain_id: 84532,
        deployment_id,
        simulation_max_age_ms: 24 * 3_600_000,
        allowed_broadcast_chain_ids: &allowed,
        // Wall-clock is far ahead of updated_at_ms=1 but simulation
        // window is 24h so we still pass.
        now_ms: 3_600_000,
        rpc: None,
    };
    let outcome = outbox
        .submit(
            row,
            plan,
            signed,
            signer,
            42,
            1_000_000,
            U256::from(2_000_000_000u64),
            U256::from(500_000_000u64),
            make_readiness_ready(),
            &firewall,
        )
        .await
        .expect("submit");
    assert_eq!(outcome.phase, BroadcastPhase::Submitted);
    assert_eq!(
        outcome.provider_classification.as_deref(),
        Some(pc::ACCEPTED)
    );

    // Verify persisted row.
    let persisted = store_dyn
        .get_broadcast_state(&cid)
        .await
        .unwrap()
        .expect("row");
    assert_eq!(persisted.phase, BroadcastPhase::Submitted);
    assert_eq!(
        persisted.tx_hash.as_deref(),
        Some(env.envelope_hash_hex().as_str())
    );

    // Frozen: only eth_sendRawTransaction on the wire.
    let mock = rpc.as_ref() as *const dyn ExecutionBroadcastRpcClient as *const MockRpc;
    let writes = unsafe { (*mock).recorded_write_methods() };
    assert_eq!(writes, vec!["eth_sendRawTransaction"]);
}

#[tokio::test]
async fn outbox_provider_hash_mismatch_escalates_manual_intervention() {
    let Some(url) =
        get_pg_url_or_skip("outbox_provider_hash_mismatch_escalates_manual_intervention")
    else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, deployment_id) = build_store(&pool).await;
    let signer = [0x77u8; 20];
    let cid = format!("0x{}", "ab".repeat(32));
    let (row, plan) = seed_row_for_broadcast(&store, deployment_id, signer, &cid).await;
    let signed = make_signed(signer);
    // Give the provider a hash that does NOT match the envelope.
    let rpc: Arc<dyn ExecutionBroadcastRpcClient> = MockRpc::accept_with([0xEE; 32]);
    let clock = Arc::new(MockClock::new(1_000));
    let outbox = build_outbox(store.clone(), rpc.clone(), clock, deployment_id);
    let target_policy = TargetPolicy::from_manifest(&baseline_manifest(84532)).unwrap();
    let gp = gas_policy();
    let allowed = [84532u64];
    let store_dyn: &dyn HybridV2ProjectionStore = store.as_ref();
    let firewall = BroadcastPolicyFirewall {
        store: store_dyn,
        target_policy: &target_policy,
        gas_policy: &gp,
        broadcast_config: BroadcastFirewallConfig {
            broadcast_enabled: true,
            pre_send_hash_probe: false,
        },
        configured_chain_id: 84532,
        deployment_id,
        simulation_max_age_ms: 24 * 3_600_000,
        allowed_broadcast_chain_ids: &allowed,
        now_ms: 3_600_000,
        rpc: None,
    };
    let outcome = outbox
        .submit(
            row,
            plan,
            signed,
            signer,
            42,
            1_000_000,
            U256::from(2_000_000_000u64),
            U256::from(500_000_000u64),
            make_readiness_ready(),
            &firewall,
        )
        .await
        .expect("submit");
    assert_eq!(outcome.phase, BroadcastPhase::ManualInterventionRequired);
    assert_eq!(
        outcome.failure_class.as_deref(),
        Some(bfc::PROVIDER_HASH_MISMATCH)
    );

    let persisted = store_dyn
        .get_broadcast_state(&cid)
        .await
        .unwrap()
        .expect("row");
    assert_eq!(persisted.phase, BroadcastPhase::ManualInterventionRequired);
}

#[tokio::test]
async fn outbox_transport_timeout_goes_to_submission_unknown() {
    let Some(url) = get_pg_url_or_skip("outbox_transport_timeout_goes_to_submission_unknown")
    else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, deployment_id) = build_store(&pool).await;
    let signer = [0x77u8; 20];
    let cid = format!("0x{}", "ac".repeat(32));
    let (row, plan) = seed_row_for_broadcast(&store, deployment_id, signer, &cid).await;
    let signed = make_signed(signer);
    let rpc: Arc<dyn ExecutionBroadcastRpcClient> =
        MockRpc::with_response(Err(BroadcastRpcError::Timeout));
    let clock = Arc::new(MockClock::new(1_000));
    let outbox = build_outbox(store.clone(), rpc.clone(), clock, deployment_id);
    let target_policy = TargetPolicy::from_manifest(&baseline_manifest(84532)).unwrap();
    let gp = gas_policy();
    let allowed = [84532u64];
    let store_dyn: &dyn HybridV2ProjectionStore = store.as_ref();
    let firewall = BroadcastPolicyFirewall {
        store: store_dyn,
        target_policy: &target_policy,
        gas_policy: &gp,
        broadcast_config: BroadcastFirewallConfig {
            broadcast_enabled: true,
            pre_send_hash_probe: false,
        },
        configured_chain_id: 84532,
        deployment_id,
        simulation_max_age_ms: 24 * 3_600_000,
        allowed_broadcast_chain_ids: &allowed,
        now_ms: 3_600_000,
        rpc: None,
    };
    let outcome = outbox
        .submit(
            row,
            plan,
            signed,
            signer,
            42,
            1_000_000,
            U256::from(2_000_000_000u64),
            U256::from(500_000_000u64),
            make_readiness_ready(),
            &firewall,
        )
        .await
        .expect("submit");
    assert_eq!(outcome.phase, BroadcastPhase::SubmissionUnknown);
    assert_eq!(
        outcome.failure_class.as_deref(),
        Some(bfc::TRANSPORT_AMBIGUOUS)
    );

    // The tx_hash must have been persisted BEFORE the ambiguous call —
    // this is the frozen safety invariant that a restart mid-call can
    // still recover the same hash.
    let persisted = store_dyn
        .get_broadcast_state(&cid)
        .await
        .unwrap()
        .expect("row");
    assert!(
        persisted.tx_hash.is_some(),
        "tx_hash must be persisted before the RPC call"
    );
}

#[tokio::test]
async fn outbox_broadcast_disabled_never_touches_rpc() {
    let Some(url) = get_pg_url_or_skip("outbox_broadcast_disabled_never_touches_rpc") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, deployment_id) = build_store(&pool).await;
    let signer = [0x77u8; 20];
    let cid = format!("0x{}", "ad".repeat(32));
    let (row, plan) = seed_row_for_broadcast(&store, deployment_id, signer, &cid).await;
    let signed = make_signed(signer);
    // The mock rpc must not be called at all.
    let rpc: Arc<dyn ExecutionBroadcastRpcClient> =
        MockRpc::with_response(Ok(SendOutcome::Accepted {
            provider_tx_hash: [0u8; 32],
        }));
    let clock = Arc::new(MockClock::new(1_000));
    let outbox = build_outbox(store.clone(), rpc.clone(), clock, deployment_id);
    let target_policy = TargetPolicy::from_manifest(&baseline_manifest(84532)).unwrap();
    let gp = gas_policy();
    let allowed = [84532u64];
    let store_dyn: &dyn HybridV2ProjectionStore = store.as_ref();
    let firewall = BroadcastPolicyFirewall {
        store: store_dyn,
        target_policy: &target_policy,
        gas_policy: &gp,
        broadcast_config: BroadcastFirewallConfig {
            broadcast_enabled: false, // disabled
            pre_send_hash_probe: false,
        },
        configured_chain_id: 84532,
        deployment_id,
        simulation_max_age_ms: 24 * 3_600_000,
        allowed_broadcast_chain_ids: &allowed,
        now_ms: 3_600_000,
        rpc: None,
    };
    let outcome = outbox
        .submit(
            row,
            plan,
            signed,
            signer,
            42,
            1_000_000,
            U256::from(2_000_000_000u64),
            U256::from(500_000_000u64),
            make_readiness_ready(),
            &firewall,
        )
        .await
        .expect("submit");
    assert_eq!(outcome.phase, BroadcastPhase::ManualInterventionRequired);
    assert_eq!(
        outcome.failure_class.as_deref(),
        Some(bfc::FIREWALL_REJECTED)
    );
    // ZERO write methods invoked.
    let mock = rpc.as_ref() as *const dyn ExecutionBroadcastRpcClient as *const MockRpc;
    let writes = unsafe { (*mock).recorded_write_methods() };
    assert!(writes.is_empty(), "expected zero writes, got {writes:?}");
}

#[tokio::test]
async fn outbox_only_eth_send_raw_transaction_is_ever_written() {
    let Some(url) = get_pg_url_or_skip("outbox_only_eth_send_raw_transaction_is_ever_written")
    else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, deployment_id) = build_store(&pool).await;
    let signer = [0x77u8; 20];
    let cid = format!("0x{}", "ae".repeat(32));
    let (row, plan) = seed_row_for_broadcast(&store, deployment_id, signer, &cid).await;
    let signed = make_signed(signer);
    let env = serialize_signed_execution(
        &plan,
        &signed,
        42,
        1_000_000,
        U256::from(2_000_000_000u64),
        U256::from(500_000_000u64),
    )
    .unwrap();
    let rpc: Arc<dyn ExecutionBroadcastRpcClient> = MockRpc::accept_with(env.envelope_hash);
    let clock = Arc::new(MockClock::new(1_000));
    let outbox = build_outbox(store.clone(), rpc.clone(), clock, deployment_id);
    let target_policy = TargetPolicy::from_manifest(&baseline_manifest(84532)).unwrap();
    let gp = gas_policy();
    let allowed = [84532u64];
    let store_dyn: &dyn HybridV2ProjectionStore = store.as_ref();
    let firewall = BroadcastPolicyFirewall {
        store: store_dyn,
        target_policy: &target_policy,
        gas_policy: &gp,
        broadcast_config: BroadcastFirewallConfig {
            broadcast_enabled: true,
            pre_send_hash_probe: false,
        },
        configured_chain_id: 84532,
        deployment_id,
        simulation_max_age_ms: 24 * 3_600_000,
        allowed_broadcast_chain_ids: &allowed,
        now_ms: 3_600_000,
        rpc: None,
    };
    outbox
        .submit(
            row,
            plan,
            signed,
            signer,
            42,
            1_000_000,
            U256::from(2_000_000_000u64),
            U256::from(500_000_000u64),
            make_readiness_ready(),
            &firewall,
        )
        .await
        .expect("submit");
    let mock = rpc.as_ref() as *const dyn ExecutionBroadcastRpcClient as *const MockRpc;
    let writes = unsafe { (*mock).recorded_write_methods() };
    // The ONLY write invocation is eth_sendRawTransaction.
    assert_eq!(writes, vec!["eth_sendRawTransaction"]);
}
