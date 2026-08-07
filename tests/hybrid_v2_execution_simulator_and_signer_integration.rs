//! `BACKEND-HYBRID-V2-SIGNER-AND-EXECUTION-V1` (Package 2, parts I–P)
//! — real-PostgreSQL + in-process HTTP mock coverage for the
//! pre-broadcast execution simulator, gas policy, nonce reservation,
//! signer firewall, and cryptographic signature verification.
//!
//! Scope of this file:
//!   * `chain_mismatch_rejected_by_http_client` — bare RPC client
//!     rejects an out-of-band chain id.
//!   * `successful_simulation_and_gas_computation` — end-to-end
//!     happy path against a fake JSON-RPC server.
//!   * `revert_decoded_for_known_selector` — revert path is
//!     decoded and surfaced.
//!   * `nonce_reservation_race_serializes_via_unique` — two workers
//!     racing against real PG serialize on the UNIQUE constraint.
//!   * `signer_firewall_rejects_tampered_plan_hash` — persisted row
//!     with a mutated plan_hash is refused by the firewall.
//!   * `ephemeral_signer_round_trip_with_recovery_verification`
//!     — TestEphemeralSigner → verify_signed_tx round trip proves
//!     the k256 recovery path.

mod hybrid_v2_support;

use deopt_v2_backend::hybrid_v2::execution::gas_policy::GasFeePolicy;
use deopt_v2_backend::hybrid_v2::execution::nonce::NonceReserver;
use deopt_v2_backend::hybrid_v2::execution::plan::ExecutionPlan;
use deopt_v2_backend::hybrid_v2::execution::rpc::{
    BlockTag, EthCallRequest, ExecutionRpcClient, ExecutionRpcError, HttpExecutionRpcClient,
    KNOWN_CUSTOM_ERROR_SELECTORS,
};
use deopt_v2_backend::hybrid_v2::execution::signature_verify::verify_signed_tx;
use deopt_v2_backend::hybrid_v2::execution::signer::{ExecutionSigner, SigningRequest};
use deopt_v2_backend::hybrid_v2::execution::signer_ephemeral::TestEphemeralSigner;
use deopt_v2_backend::hybrid_v2::execution::signer_firewall::{
    FirewallRejection, SignerPolicyFirewall,
};
use deopt_v2_backend::hybrid_v2::execution::simulator::ExecutionSimulator;
use deopt_v2_backend::hybrid_v2::execution::target_policy::TargetPolicy;
use deopt_v2_backend::hybrid_v2::execution::{
    derive_canonical_execution_id, CanonicalExecutionId, ExecutionPhase, ExecutionRequestRow,
};
use deopt_v2_backend::hybrid_v2::persistence::{
    HybridV2ProjectionStore, PostgresHybridV2ProjectionStore,
};
use hybrid_v2_support::baseline_manifest;
use serde_json::{json, Value};
use sha3::{Digest, Keccak256};
use sqlx::postgres::{PgPool, PgPoolOptions};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

// ------------------- PG helpers (mirror the foundation suite) -------

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

async fn build_store(pool: &PgPool) -> (Arc<dyn HybridV2ProjectionStore>, i64) {
    let store: Arc<dyn HybridV2ProjectionStore> =
        Arc::new(PostgresHybridV2ProjectionStore::new(pool.clone()));
    let m = baseline_manifest(84532);
    let d = store
        .upsert_deployment(&m, "PENDING", 1_700_000_000_000)
        .await
        .expect("d");
    (store, d)
}

// ------------------- tiny in-process JSON-RPC mock -----------------

/// Boot a tiny HTTP JSON-RPC server on `127.0.0.1:0`. `handler` maps
/// (method, params) → serde_json::Value that is embedded as `result`.
/// A `Value::Null` from the handler is treated as literal null result.
/// Returning `Err(String)` triggers a JSON-RPC error response.
async fn start_mock_rpc<F>(handler: F) -> (SocketAddr, tokio::task::JoinHandle<()>)
where
    F: Fn(&str, &Value) -> Result<Value, MockRpcErr> + Send + Sync + 'static,
{
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().unwrap();
    let handler = Arc::new(handler);
    let handle = tokio::spawn(async move {
        loop {
            let (mut sock, _) = match listener.accept().await {
                Ok(v) => v,
                Err(_) => continue,
            };
            let handler = handler.clone();
            tokio::spawn(async move {
                let mut buf = vec![0u8; 8192];
                let n = match sock.read(&mut buf).await {
                    Ok(0) => return,
                    Ok(n) => n,
                    Err(_) => return,
                };
                let req = &buf[..n];
                let body_start = req
                    .windows(4)
                    .position(|w| w == b"\r\n\r\n")
                    .map(|p| p + 4)
                    .unwrap_or(0);
                let body = &req[body_start..];
                let mut idx: Option<u64> = None;
                let payload_value: Value = serde_json::from_slice(body).unwrap_or(Value::Null);
                let method = payload_value
                    .get("method")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let params = payload_value
                    .get("params")
                    .cloned()
                    .unwrap_or(Value::Array(vec![]));
                if let Some(v) = payload_value.get("id").and_then(|v| v.as_u64()) {
                    idx = Some(v);
                }
                let response_body = match handler(method, &params) {
                    Ok(result) => json!({
                        "jsonrpc": "2.0",
                        "id": idx.unwrap_or(1),
                        "result": result,
                    }),
                    Err(MockRpcErr {
                        code,
                        message,
                        data,
                    }) => {
                        let mut err_obj = serde_json::Map::new();
                        err_obj.insert("code".into(), Value::from(code));
                        err_obj.insert("message".into(), Value::String(message));
                        if let Some(d) = data {
                            err_obj.insert("data".into(), Value::String(d));
                        }
                        json!({
                            "jsonrpc": "2.0",
                            "id": idx.unwrap_or(1),
                            "error": Value::Object(err_obj),
                        })
                    }
                };
                let body = serde_json::to_vec(&response_body).unwrap();
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = sock.write_all(response.as_bytes()).await;
                let _ = sock.write_all(&body).await;
                let _ = sock.shutdown().await;
            });
        }
    });
    (addr, handle)
}

struct MockRpcErr {
    code: i64,
    message: String,
    data: Option<String>,
}

// ------------------- helpers -----------------------------------------

fn dummy_plan(chain_id: u64) -> ExecutionPlan {
    ExecutionPlan {
        canonical_execution_id: CanonicalExecutionId(format!("0x{}", "aa".repeat(32))),
        chain_id,
        deployment_id: 1,
        target: [0xcc; 20],
        selector: [0x12, 0x34, 0x56, 0x78],
        calldata: vec![0x12, 0x34, 0x56, 0x78, 0x00, 0x00],
        calldata_hash: [0xdd; 32],
        value_wei: alloy_primitives::U256::ZERO,
        expected_module_version: "OptionMatchingEngineV2".into(),
        deadline_ms: None,
        plan_hash: [0xee; 32],
    }
}

fn gas_policy() -> GasFeePolicy {
    GasFeePolicy {
        max_gas_limit: 5_000_000,
        gas_limit_multiplier_bps: 12_000,
        max_fee_per_gas_wei: alloy_primitives::U256::from(50_000_000_000u64),
        max_priority_fee_per_gas_wei: alloy_primitives::U256::from(2_000_000_000u64),
        max_total_native_cost_wei: alloy_primitives::U256::from(10u64)
            .pow(alloy_primitives::U256::from(18u64)),
        abnormal_estimate_reject_threshold: 10,
    }
}

// ============================ TESTS ==================================

#[tokio::test]
async fn chain_mismatch_rejected_by_simulator() {
    let (addr, _h) = start_mock_rpc(|method, _| match method {
        "eth_chainId" => Ok(Value::String("0x14a34".into())), // 84532
        _ => Err(MockRpcErr {
            code: -32601,
            message: "method not found".into(),
            data: None,
        }),
    })
    .await;
    let endpoint = format!("http://{addr}");
    let client = HttpExecutionRpcClient::new(endpoint, Duration::from_secs(1), 0).expect("client");
    let plan = dummy_plan(11155111);
    let sim = ExecutionSimulator::new(&client, [0x01; 20]);
    let err = sim.simulate(&plan).await.expect_err("chain mismatch");
    // Simulator returns ChainMismatch because the mock reports 84532
    // but the plan is 11155111.
    use deopt_v2_backend::hybrid_v2::execution::simulator::SimulationError;
    assert!(matches!(err, SimulationError::ChainMismatch { .. }));
}

#[tokio::test]
async fn successful_simulation_and_gas_computation() {
    let (addr, _h) = start_mock_rpc(|method, _| match method {
        "eth_chainId" => Ok(Value::String("0x14a34".into())),
        "eth_blockNumber" => Ok(Value::String("0x64".into())), // 100
        "eth_getBlockByNumber" => Ok(json!({
            "hash": format!("0x{}", "ab".repeat(32)),
            "number": "0x64",
            "parentHash": format!("0x{}", "cd".repeat(32)),
            "timestamp": "0x1",
        })),
        "eth_call" => Ok(Value::String("0x".into())),
        "eth_estimateGas" => Ok(Value::String("0x15f90".into())), // 90000
        "eth_feeHistory" => Ok(json!({
            "oldestBlock": "0x1",
            "baseFeePerGas": [
                "0x3b9aca00","0x41314cf0","0x3d0900c0","0x3b9aca00"
            ],
            "gasUsedRatio": [0.5,0.6,0.55,0.5],
            "reward": [
                ["0x1dcd6500"],["0x22ecb25c"],["0x20c855c0"]
            ],
        })),
        _ => Err(MockRpcErr {
            code: -32601,
            message: "not found".into(),
            data: None,
        }),
    })
    .await;
    let endpoint = format!("http://{addr}");
    let client = HttpExecutionRpcClient::new(endpoint, Duration::from_secs(1), 0).expect("client");
    let plan = dummy_plan(84532);
    let sim = ExecutionSimulator::new(&client, [0x01; 20]);
    let outcome = sim.simulate(&plan).await.expect("simulation ok");
    assert!(outcome.eth_call_ok);
    assert_eq!(outcome.gas_estimate, 90_000);

    // Compute gas via policy — must accept and return a bounded output.
    let fee_history = client
        .fee_history(4, BlockTag::Number(100), &[10.0])
        .await
        .expect("fee history");
    let out = gas_policy()
        .compute(&outcome, &fee_history)
        .expect("gas policy ok");
    assert_eq!(out.gas_limit, 108_000); // 90000 * 12000/10000 = 108000
    assert!(out.max_fee_per_gas_wei > alloy_primitives::U256::ZERO);
}

#[tokio::test]
async fn revert_decoded_for_known_selector() {
    // Take the first known custom selector and reflect it as a revert.
    let (sel, expected_name) = KNOWN_CUSTOM_ERROR_SELECTORS[0];
    let hex = format!(
        "0x{}",
        sel.iter().map(|b| format!("{b:02x}")).collect::<String>()
    );
    let (addr, _h) = start_mock_rpc(move |method, _| match method {
        "eth_chainId" => Ok(Value::String("0x14a34".into())),
        "eth_blockNumber" => Ok(Value::String("0x1".into())),
        "eth_getBlockByNumber" => Ok(json!({
            "hash": format!("0x{}", "aa".repeat(32)),
            "number": "0x1",
            "parentHash": format!("0x{}", "aa".repeat(32)),
            "timestamp": "0x1",
        })),
        "eth_call" => Err(MockRpcErr {
            code: -32000,
            message: "execution reverted".into(),
            data: Some(hex.clone()),
        }),
        _ => Err(MockRpcErr {
            code: -32601,
            message: "not found".into(),
            data: None,
        }),
    })
    .await;
    let endpoint = format!("http://{addr}");
    let client = HttpExecutionRpcClient::new(endpoint, Duration::from_secs(1), 0).expect("client");
    let plan = dummy_plan(84532);
    let sim = ExecutionSimulator::new(&client, [0x01; 20]);
    let out = sim.simulate(&plan).await.expect("simulation completes");
    assert!(!out.eth_call_ok);
    use deopt_v2_backend::hybrid_v2::execution::rpc::DecodedRevert;
    match out.revert {
        Some(DecodedRevert::Custom { name, .. }) => assert_eq!(name, expected_name),
        other => panic!("wanted Custom decoded revert, got {other:?}"),
    }
}

// A minimal ExecutionRpcClient for tests that don't need HTTP.
struct FakeChainRpc {
    chain_id: u64,
    pending_nonce: u64,
}

#[async_trait::async_trait]
impl ExecutionRpcClient for FakeChainRpc {
    async fn chain_id(&self) -> Result<u64, ExecutionRpcError> {
        Ok(self.chain_id)
    }
    async fn head_block_number(&self) -> Result<u64, ExecutionRpcError> {
        Ok(1)
    }
    async fn head_block_hash(&self) -> Result<[u8; 32], ExecutionRpcError> {
        Ok([0; 32])
    }
    async fn transaction_count(&self, _: [u8; 20], _: BlockTag) -> Result<u64, ExecutionRpcError> {
        Ok(self.pending_nonce)
    }
    async fn eth_call(
        &self,
        _: EthCallRequest,
        _: BlockTag,
    ) -> Result<deopt_v2_backend::hybrid_v2::execution::rpc::EthCallOutcome, ExecutionRpcError>
    {
        unreachable!()
    }
    async fn estimate_gas(&self, _: EthCallRequest, _: BlockTag) -> Result<u64, ExecutionRpcError> {
        unreachable!()
    }
    async fn fee_history(
        &self,
        _: u64,
        _: BlockTag,
        _: &[f64],
    ) -> Result<deopt_v2_backend::hybrid_v2::execution::rpc::FeeHistory, ExecutionRpcError> {
        unreachable!()
    }
}

#[tokio::test]
async fn nonce_reservation_race_serializes_via_unique() {
    let Some(url) = get_pg_url_or_skip("nonce_reservation_race_serializes_via_unique") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, did) = build_store(&pool).await;
    let signer = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string();

    // Insert two real execution rows so the FK on hybrid_v2_executor_nonces
    // can point at them.
    let plan_a = dummy_plan(84532);
    let row_a = build_execution_row(did, 84532, &plan_a, 1_000);
    store.insert_execution_request(&row_a).await.unwrap();
    let mut plan_b = plan_a.clone();
    plan_b.canonical_execution_id = CanonicalExecutionId(format!("0x{}", "bb".repeat(32)));
    let mut row_b = build_execution_row(did, 84532, &plan_b, 1_000);
    // build_execution_row derives canonical_execution_id from fixed
    // hashes → override with a distinct one via unique quantity.
    row_b.canonical_execution_id = derive_canonical_execution_id(
        did,
        84532,
        &row_b.buyer_order_hash,
        &row_b.seller_order_hash,
        200,
    )
    .into_string();
    row_b.fill_quantity_1e8 = "200".to_string();
    store.insert_execution_request(&row_b).await.unwrap();

    let rpc_a = Arc::new(FakeChainRpc {
        chain_id: 84532,
        pending_nonce: 42,
    });
    let rpc_b = Arc::new(FakeChainRpc {
        chain_id: 84532,
        pending_nonce: 42,
    });

    let store_clone = store.clone();
    let signer_a = signer.clone();
    let id_a = row_a.canonical_execution_id.clone();
    let a = tokio::spawn(async move {
        let reserver = NonceReserver {
            store: &*store_clone,
            rpc: &*rpc_a,
            signer_identity: signer_a,
            chain_id: 84532,
        };
        reserver.reserve_for(&id_a, 100).await.unwrap()
    });
    let store_clone2 = store.clone();
    let signer_b = signer.clone();
    let id_b = row_b.canonical_execution_id.clone();
    let b = tokio::spawn(async move {
        let reserver = NonceReserver {
            store: &*store_clone2,
            rpc: &*rpc_b,
            signer_identity: signer_b,
            chain_id: 84532,
        };
        reserver.reserve_for(&id_b, 101).await.unwrap()
    });
    let (ra, rb) = tokio::try_join!(a, b).unwrap();
    // At least one race outcome MUST hold nonce=42. The other MUST be
    // 43 (increment on unique-conflict). They MUST be distinct.
    assert_ne!(ra.nonce, rb.nonce);
    assert_eq!(ra.nonce.min(rb.nonce), 42);
    assert_eq!(ra.nonce.max(rb.nonce), 43);
}

fn build_execution_row(
    deployment_id: i64,
    chain_id: u64,
    plan: &ExecutionPlan,
    now_ms: i64,
) -> ExecutionRequestRow {
    let buyer_h = format!("0x{}", "aa".repeat(32));
    let seller_h = format!("0x{}", "bb".repeat(32));
    let id = derive_canonical_execution_id(deployment_id, chain_id, &buyer_h, &seller_h, 100);
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
        fill_quantity_1e8: "100".to_string(),
        premium_amount: "50000000".to_string(),
        fee_schedule_epoch: None,
        source_matched_execution_id: None,
        target_contract: format!(
            "0x{}",
            plan.target
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        ),
        selector: format!(
            "0x{}",
            plan.selector
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        ),
        calldata_hash: Some(format!(
            "0x{}",
            plan.calldata_hash
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        )),
        plan_hash: Some(format!(
            "0x{}",
            plan.plan_hash
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        )),
        tx_value_wei: "0".to_string(),
        simulation_block_number: Some(100),
        simulation_block_hash: Some(format!("0x{}", "ff".repeat(32))),
        simulation_gas_estimate: Some(90_000),
        simulation_result_json: None,
        signer_identity: None,
        signing_payload_hash: None,
        signature_r: None,
        signature_s: None,
        signature_v: None,
        recovered_signer: None,
        gas_limit: Some(108_000),
        max_fee_per_gas_wei: Some("2000000000".into()),
        max_priority_fee_per_gas_wei: Some("1000000000".into()),
        reserved_nonce: Some(42),
        phase: ExecutionPhase::AwaitingSignature,
        failure_class: None,
        failure_detail: None,
        retry_count: 0,
        holder_epoch: None,
        created_at_ms: now_ms,
        updated_at_ms: now_ms,
    }
}

#[tokio::test]
async fn signer_firewall_rejects_tampered_plan_hash() {
    let Some(url) = get_pg_url_or_skip("signer_firewall_rejects_tampered_plan_hash") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, did) = build_store(&pool).await;
    let plan = dummy_plan(84532);
    let mut row = build_execution_row(did, 84532, &plan, 1_000);
    // Row is inserted with the plan's plan_hash…
    store
        .insert_execution_request(&row)
        .await
        .expect("insert execution row");

    // …but a caller passes a tampered plan (different plan_hash).
    let mut tampered = plan.clone();
    tampered.plan_hash = [0xffu8; 32];

    // Rebuild the persisted row's plan_hash to match the original plan
    // (bit-identical). The row's `plan_hash` column reflects the
    // ORIGINAL plan, so the firewall must reject when the plan input
    // differs.
    row.plan_hash = Some(format!(
        "0x{}",
        plan.plan_hash
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    ));

    let tp_manifest = baseline_manifest(84532);
    // TargetPolicy expects the plan's target to match the manifest
    // engine address, so for this firewall test we use a plan whose
    // target IS the engine.
    let engine_addr = {
        let hex = tp_manifest
            .module_addresses
            .option_matching_engine
            .trim_start_matches("0x");
        let mut a = [0u8; 20];
        for i in 0..20 {
            a[i] = u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).unwrap();
        }
        a
    };
    let mut plan2 = plan.clone();
    plan2.target = engine_addr;
    // Rewrite the row with the corrected target too.
    row.target_contract = format!(
        "0x{}",
        engine_addr
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    );
    // Update the row in the store — insert-only so we mutate directly
    // via SQL for the sake of the test.
    sqlx::query("UPDATE hybrid_v2_execution_requests SET target_contract=$1 WHERE canonical_execution_id=$2")
        .bind(&row.target_contract)
        .bind(&row.canonical_execution_id)
        .execute(&pool)
        .await
        .unwrap();

    let tp = TargetPolicy::from_manifest(&tp_manifest).unwrap();
    // Manifest engine selector allowlist matches executeMatch — so
    // point the plan's selector at the engine's selector.
    use alloy_sol_types::SolCall;
    plan2.selector = deopt_v2_backend::hybrid_v2::execution::plan::executeMatchCall::SELECTOR;
    sqlx::query(
        "UPDATE hybrid_v2_execution_requests SET selector=$1 WHERE canonical_execution_id=$2",
    )
    .bind(format!(
        "0x{}",
        plan2
            .selector
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    ))
    .bind(&row.canonical_execution_id)
    .execute(&pool)
    .await
    .unwrap();
    // Reload the row so field states match DB.
    let live = store
        .get_execution_request(&row.canonical_execution_id)
        .await
        .unwrap()
        .unwrap();

    let gp = gas_policy();
    let fw = SignerPolicyFirewall {
        target_policy: &tp,
        gas_policy: &gp,
        configured_chain_id: 84532,
        deployment_id: did,
        store: &*store,
        simulation_max_age_ms: 60_000,
        now_ms: 1_100,
    };
    let reservation = deopt_v2_backend::hybrid_v2::execution::nonce::NonceReservation {
        chain_id: 84532,
        signer_identity: "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
        nonce: 42,
        canonical_execution_id: row.canonical_execution_id.clone(),
        reserved_at_ms: 1_000,
    };

    // Tampered plan → firewall must refuse.
    let mut tampered_full = plan2.clone();
    tampered_full.canonical_execution_id = CanonicalExecutionId(row.canonical_execution_id.clone());
    tampered_full.plan_hash = [0xffu8; 32];
    let err = fw
        .revalidate(&live, &tampered_full, &reservation, [0xaau8; 20])
        .await
        .unwrap_err();
    assert_eq!(err, FirewallRejection::PlanHashChanged);

    // Untampered plan → firewall must ACCEPT (canonical id must match
    // the row's canonical_execution_id).
    let mut ok_plan = plan2.clone();
    ok_plan.canonical_execution_id = CanonicalExecutionId(row.canonical_execution_id.clone());
    // Also ensure calldata_hash + plan_hash on the row match the plan.
    sqlx::query(
        "UPDATE hybrid_v2_execution_requests
         SET calldata_hash=$1, plan_hash=$2
         WHERE canonical_execution_id=$3",
    )
    .bind(format!(
        "0x{}",
        ok_plan
            .calldata_hash
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    ))
    .bind(format!(
        "0x{}",
        ok_plan
            .plan_hash
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    ))
    .bind(&row.canonical_execution_id)
    .execute(&pool)
    .await
    .unwrap();
    let live2 = store
        .get_execution_request(&row.canonical_execution_id)
        .await
        .unwrap()
        .unwrap();
    fw.revalidate(&live2, &ok_plan, &reservation, [0xaau8; 20])
        .await
        .expect("untampered plan must pass");
}

#[tokio::test]
async fn ephemeral_signer_round_trip_with_recovery_verification() {
    let signer = TestEphemeralSigner::from_seed([0x33u8; 32]);
    let calldata = vec![0x11u8, 0x22, 0x33, 0x44, 0x55];
    let calldata_hash = {
        let h = Keccak256::digest(&calldata);
        let mut a = [0u8; 32];
        a.copy_from_slice(&h[..]);
        a
    };
    let signing_payload_hash = {
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
        nonce: 42,
        target: [0xcc; 20],
        value_wei: alloy_primitives::U256::ZERO,
        calldata_hash,
        gas_limit: 108_000,
        max_fee_per_gas_wei: alloy_primitives::U256::from(2_000_000_000u64),
        max_priority_fee_per_gas_wei: alloy_primitives::U256::from(1_000_000_000u64),
        tx_type: 2,
        plan_hash: [0x99u8; 32],
        signing_payload_hash,
        calldata,
    };
    let signed = signer.sign_execution(req.clone()).await.expect("signed");
    let verified = verify_signed_tx(&req, &signed, signer.address()).expect("verified");
    assert_eq!(verified.recovered_signer, signer.address());
    assert!(verified.is_low_s);
}
