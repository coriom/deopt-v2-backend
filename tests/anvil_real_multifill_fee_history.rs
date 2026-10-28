//! Ingests an actual two-fill receipt from the disposable Anvil fork fixture.
//! It never constructs synthetic receipt logs or submits a public transaction.
use alloy_primitives::keccak256;
use deopt_v2_backend::db::PgRepository;
use deopt_v2_backend::fees::v2_intent_receipt::{prove_direct_intent_fill, INTENT_EVENT_SIGNATURE};
use deopt_v2_backend::fees::v2_perp_settlement::reconcile_confirmed_v2_intent_tx;
use deopt_v2_backend::indexer::EthLog;
use serde_json::{json, Value};

const RPC: &str = "http://127.0.0.1:18545";
const EXECUTIONS: [&str; 5] = [
    "0xb8bcb5359598046cd5ea9110e32cb303674b7ba5825e215e823e1941602c3c87",
    "0xc5c7ae3f6cd81253da5090aec01eb235952bf204ed2ec322d8e78c948616c955",
    "0x4fcbade6e314e9a1c12f779cfa48ff869ba7481448dae6d168c55d2beab7807e",
    "0xecd3ba1221fa073937ec366a584f5e9006bc725e877e237415c52128d1da32b0",
    "0xc6ef082b121947377426437087da5bec0172173aa65240a14d4d7a0f1cf34940",
];

async fn rpc(method: &str, params: Value) -> Value {
    let response: Value = reqwest::Client::new()
        .post(RPC)
        .json(&json!({"jsonrpc":"2.0","id":1,"method":method,"params":params}))
        .send()
        .await
        .expect("local Anvil RPC")
        .json()
        .await
        .expect("local JSON RPC");
    assert!(
        response.get("error").is_none(),
        "local Anvil RPC {method} failed: {response}"
    );
    response["result"].clone()
}

#[tokio::test]
#[ignore = "requires local Anvil fork, generated real receipt, and disposable PostgreSQL"]
async fn real_anvil_multifill_receipt_to_postgres_and_api() {
    let path =
        std::env::var("DEOPT_ANVIL_MULTIFILL_FIXTURE").expect("real local fixture path required");
    let url = std::env::var("DEOPT_ANVIL_MULTIFILL_PG_URL").expect("disposable PG URL required");
    assert!(path == "/tmp/deopt_anvil_multifill_actual.json");
    assert!(
        url.starts_with("postgres://")
            && url.contains("@127.0.0.1:15439/")
            && url.ends_with("/deopt_anvil_fees"),
        "disposable local PG required"
    );
    let fixture: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let tx_hash = fixture["receipt"]["transactionHash"].as_str().unwrap();
    let block_hash = fixture["receipt"]["blockHash"].as_str().unwrap();
    let original = rpc("eth_getTransactionReceipt", json!([tx_hash])).await;
    assert_eq!(
        original, fixture["receipt"],
        "fixture must be actual local receipt"
    );
    assert_eq!(original["status"], "0x1");
    assert_eq!(original["blockHash"], block_hash);
    let transaction = rpc("eth_getTransactionByHash", json!([tx_hash])).await;
    assert_eq!(transaction, fixture["transaction"]);
    let trace = rpc(
        "debug_traceTransaction",
        json!([tx_hash, {"tracer":"callTracer"}]),
    )
    .await;
    assert_eq!(trace, fixture["trace"]);
    assert_eq!(trace["input"], transaction["input"]);
    assert_eq!(
        trace["to"].as_str().unwrap().to_ascii_lowercase(),
        fixture["fixture"]["wrapper"]
            .as_str()
            .unwrap()
            .to_ascii_lowercase()
    );
    let nested = trace["calls"].as_array().unwrap();
    let expected_hashes = fixture["fixture"]["callHashes"].as_array().unwrap();
    assert_eq!((nested.len(), expected_hashes.len()), (2, 2));
    for (call, expected) in nested.iter().zip(expected_hashes) {
        assert!(call.get("error").is_none());
        assert_eq!(
            call["to"].as_str().unwrap().to_ascii_lowercase(),
            "0xeea4e5d891b90d3d99e9c4e2c61069911477110e"
        );
        let input = hex::decode(call["input"].as_str().unwrap().trim_start_matches("0x")).unwrap();
        assert_eq!(
            format!("{:#x}", keccak256(&input)),
            expected.as_str().unwrap()
        );
        let engine = call["calls"]
            .as_array()
            .unwrap()
            .iter()
            .find(|inner| {
                inner["to"].as_str().is_some_and(|to| {
                    to.eq_ignore_ascii_case("0xd0901de8f6de72aecc716ca1465c4cf58a0b99fb")
                }) && inner["type"] == "CALL"
            })
            .unwrap();
        assert_eq!(
            engine["calls"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|inner| inner["to"].as_str().is_some_and(
                    |to| to.eq_ignore_ascii_case("0x9947f42cc29a32992bbf2030ee49e35c839eded5")
                ) && inner["type"] == "CALL")
                .count(),
            2
        );
    }
    let d2_logs = original["logs"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|log| {
            log["address"].as_str().is_some_and(|address| {
                address.eq_ignore_ascii_case("0x9947f42cc29a32992bbf2030ee49e35c839eded5")
            })
        })
        .count();
    let d6_logs = original["logs"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|log| {
            log["address"].as_str().is_some_and(|address| {
                address.eq_ignore_ascii_case("0xeea4e5d891b90d3d99e9c4e2c61069911477110e")
            })
        })
        .count();
    assert_eq!((d2_logs, d6_logs), (2, 2));
    let chain = rpc("eth_chainId", json!([])).await;
    assert_eq!(chain, "0x14a34");
    let client = rpc("web3_clientVersion", json!([])).await;
    assert!(client
        .as_str()
        .unwrap()
        .to_ascii_lowercase()
        .contains("anvil"));
    let block = rpc(
        "eth_getBlockByNumber",
        json!([original["blockNumber"], false]),
    )
    .await;
    assert_eq!(block["hash"], block_hash);
    let latest = rpc("eth_getBlockByNumber", json!(["latest", false])).await;
    let now = u64::from_str_radix(
        latest["timestamp"]
            .as_str()
            .unwrap()
            .trim_start_matches("0x"),
        16,
    )
    .unwrap();
    let trade_timestamp = u64::from_str_radix(
        block["timestamp"]
            .as_str()
            .unwrap()
            .trim_start_matches("0x"),
        16,
    )
    .unwrap();
    let intent_topic = format!("{:#x}", keccak256(INTENT_EVENT_SIGNATURE.as_bytes()));
    let intent_events: Vec<_> = original["logs"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|log| {
            log["address"].as_str().is_some_and(|address| {
                address.eq_ignore_ascii_case("0xeea4e5d891b90d3d99e9c4e2c61069911477110e")
            }) && log["topics"][0]
                .as_str()
                .is_some_and(|topic| topic.eq_ignore_ascii_case(&intent_topic))
        })
        .collect();
    assert_eq!(intent_events.len(), nested.len());
    let mut signed_hashes = Vec::new();
    for (nested_call, event_json) in nested.iter().zip(intent_events.iter()) {
        // callTracer is queried for this exact canonical local transaction;
        // D6 emits exactly one intent event at the end of each successful call.
        // Each nested call's EIP-712 signatures and both intent hashes must
        // match its corresponding real log. No local subaccount ownership is inferred.
        let mut inner_tx = transaction.clone();
        inner_tx["to"] = json!(nested_call["to"]);
        inner_tx["input"] = json!(nested_call["input"]);
        let mut single_event_receipt = original.clone();
        single_event_receipt["logs"] = Value::Array(
            original["logs"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|log| {
                    !log["address"].as_str().is_some_and(|address| {
                        address.eq_ignore_ascii_case("0xeea4e5d891b90d3d99e9c4e2c61069911477110e")
                    }) || log["logIndex"] == event_json["logIndex"]
                })
                .cloned()
                .collect(),
        );
        let event: EthLog = serde_json::from_value((*event_json).clone()).unwrap();
        let signed =
            prove_direct_intent_fill(&inner_tx, &single_event_receipt, &event, trade_timestamp)
                .unwrap();
        signed_hashes.push((
            signed.buyer_intent_hash.clone(),
            signed.seller_intent_hash.clone(),
        ));
        assert_eq!(
            signed.trade.log_index.to_string(),
            u64::from_str_radix(
                event_json["logIndex"]
                    .as_str()
                    .unwrap()
                    .trim_start_matches("0x"),
                16
            )
            .unwrap()
            .to_string()
        );
        assert_eq!(
            signed.trade.buyer.to_ascii_lowercase(),
            fixture["fixture"]["buyers"][0]
                .as_str()
                .unwrap()
                .to_ascii_lowercase()
        );
        assert!(fixture["fixture"]["sellers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|seller| seller
                .as_str()
                .unwrap()
                .eq_ignore_ascii_case(&signed.trade.seller)));
    }
    assert_eq!(
        signed_hashes[0].0, signed_hashes[1].0,
        "one maker intent is partially filled twice"
    );
    assert_ne!(
        signed_hashes[0].1, signed_hashes[1].1,
        "distinct signed taker intents required"
    );
    assert_eq!(
        signed_hashes[0].0,
        fixture["fixture"]["buyerIntentHash"]
            .as_str()
            .unwrap()
            .to_ascii_lowercase()
    );
    let buyer_hash = fixture["fixture"]["buyerIntentHash"].as_str().unwrap();
    let getter = format!(
        "0x{}{}",
        hex::encode(&keccak256("intentFilled(bytes32)".as_bytes())[..4]),
        buyer_hash.trim_start_matches("0x")
    );
    let filled = rpc(
        "eth_call",
        json!([{"to":"0xeeA4e5d891B90D3d99E9C4e2c61069911477110E","data":getter}, "latest"]),
    )
    .await;
    assert_eq!(
        u128::from_str_radix(filled.as_str().unwrap().trim_start_matches("0x"), 16).unwrap(),
        300_000_000
    );
    let executions: [String; 5] = EXECUTIONS.map(str::to_owned);

    let repo = PgRepository::connect(&url).await.unwrap();
    repo.run_migrations().await.unwrap();
    let inserted = reconcile_confirmed_v2_intent_tx(&repo, RPC, &executions, tx_hash, now)
        .await
        .unwrap();
    if std::env::var("DEOPT_REQUIRE_NEW_INGEST").as_deref() == Ok("1") {
        assert!(
            inserted,
            "fresh disposable DB must insert the actual receipt once"
        );
    }
    assert!(
        !reconcile_confirmed_v2_intent_tx(&repo, RPC, &executions, tx_hash, now)
            .await
            .unwrap(),
        "receipt replay must be idempotent"
    );

    let buyer = fixture["fixture"]["buyers"][0].as_str().unwrap();
    let seller_a = fixture["fixture"]["sellers"][0].as_str().unwrap();
    let seller_b = fixture["fixture"]["sellers"][1].as_str().unwrap();
    let buyer_rows = repo
        .list_canonical_v2_perp_history(buyer, 10)
        .await
        .unwrap();
    let seller_a_rows = repo
        .list_canonical_v2_perp_history(seller_a, 10)
        .await
        .unwrap();
    let seller_b_rows = repo
        .list_canonical_v2_perp_history(seller_b, 10)
        .await
        .unwrap();
    assert_eq!(
        (buyer_rows.len(), seller_a_rows.len(), seller_b_rows.len()),
        (2, 1, 1)
    );
    assert!(buyer_rows
        .iter()
        .all(|row| row.fee_native == "0" && row.role == "maker" && row.subaccount_id.is_none()));
    assert!(seller_a_rows
        .iter()
        .all(|row| row.role == "taker" && row.subaccount_id.is_none()));
    assert!(seller_b_rows
        .iter()
        .all(|row| row.role == "taker" && row.subaccount_id.is_none()));
    assert_ne!(seller_a_rows[0].fee_native, seller_b_rows[0].fee_native);
    let price: u128 = fixture["fixture"]["price"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    let expected_fee = |size: u128| {
        let basis_native = (size * price / 100_000_000) / 100;
        (basis_native * 200).div_ceil(1_000_000).to_string()
    };
    assert_eq!(seller_a_rows[0].fee_native, expected_fee(100_000_000));
    assert_eq!(seller_b_rows[0].fee_native, expected_fee(200_000_000));
    assert_ne!(
        seller_a_rows[0].trade_log_index,
        seller_b_rows[0].trade_log_index
    );
    assert_eq!(
        seller_a_rows[0].fee_recipient.to_ascii_lowercase(),
        "0xa67f8e8e673ce4bb2fb563b0e6e9fa8f70e3b588"
    );

    // Read the very same committed rows through the production router.
    use deopt_v2_backend::api::{router, AppState};
    use deopt_v2_backend::engine::EngineState;
    use tower::ServiceExt;
    let mut state = AppState::new(EngineState::with_default_markets());
    state.chain_id = 84532;
    state.repository = Some(repo.clone());
    let url_path = format!("/accounts/{buyer}/perps/canonical-fees?limit=1");
    let response = router(state.clone())
        .oneshot(
            axum::http::Request::builder()
                .uri(url_path)
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 64 * 1024)
        .await
        .unwrap();
    let page: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(page["identity_scope"], "wallet");
    assert_eq!(page["fees"].as_array().unwrap().len(), 1);
    assert_eq!(page["fees"][0]["subaccount_id"], Value::Null);
    assert!(page["next_cursor"].is_object());
    let cursor = &page["next_cursor"];
    let next_path = format!(
        "/accounts/{buyer}/perps/canonical-fees?limit=1&before_block={}&before_log_index={}",
        cursor["before_block"], cursor["before_log_index"]
    );
    let response = router(state)
        .oneshot(
            axum::http::Request::builder()
                .uri(next_path)
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 64 * 1024)
        .await
        .unwrap();
    let next: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(next["fees"].as_array().unwrap().len(), 1);
    assert_ne!(
        page["fees"][0]["trade_log_index"],
        next["fees"][0]["trade_log_index"]
    );
    println!(
        "REAL_ANVIL_MULTIFILL_INGESTED tx={tx_hash} block={block_hash} buyer_fills=2 seller_fees=2 inserted={inserted}"
    );
}

#[tokio::test]
#[ignore = "destructively reverts only the disposable local Anvil snapshot; run after ingestion"]
async fn real_anvil_reorg_fails_closed_without_erasing_committed_accounting() {
    assert_eq!(std::env::var("DEOPT_ANVIL_REORG_CHECK").as_deref(), Ok("1"));
    let path = "/tmp/deopt_anvil_multifill_actual.json";
    let url = "postgres://postgres@127.0.0.1:15439/deopt_anvil_fees";
    let fixture: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert!(rpc("web3_clientVersion", json!([]))
        .await
        .as_str()
        .unwrap()
        .to_ascii_lowercase()
        .contains("anvil"));
    assert_eq!(rpc("eth_chainId", json!([])).await, "0x14a34");
    let repo = PgRepository::connect(url).await.unwrap();
    let buyer = fixture["fixture"]["buyers"][0].as_str().unwrap();
    assert_eq!(
        repo.list_canonical_v2_perp_history(buyer, 10)
            .await
            .unwrap()
            .len(),
        2,
        "run successful ingestion before the destructive local reorg rehearsal"
    );
    let tx_hash = fixture["receipt"]["transactionHash"].as_str().unwrap();
    let snapshot = fixture["fixture"]["preTradeSnapshot"].as_str().unwrap();
    assert_eq!(rpc("evm_revert", json!([snapshot])).await, true);
    assert!(rpc("eth_getTransactionReceipt", json!([tx_hash]))
        .await
        .is_null());
    let latest = rpc("eth_getBlockByNumber", json!(["latest", false])).await;
    let now = u64::from_str_radix(
        latest["timestamp"]
            .as_str()
            .unwrap()
            .trim_start_matches("0x"),
        16,
    )
    .unwrap();
    let executions: [String; 5] = EXECUTIONS.map(str::to_owned);
    assert!(
        reconcile_confirmed_v2_intent_tx(&repo, RPC, &executions, tx_hash, now)
            .await
            .is_err(),
        "a disappeared canonical receipt must not be re-accepted"
    );
    assert_eq!(
        repo.list_canonical_v2_perp_history(buyer, 10)
            .await
            .unwrap()
            .len(),
        2,
        "persisted settled rows require operator reconciliation, not silent rollback"
    );
    println!("REAL_ANVIL_REORG_DETECTED tx={tx_hash} persisted_rows=2 manual_reconciliation_required=true");
}
