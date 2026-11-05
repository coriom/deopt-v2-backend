//! Opt-in proof against a disposable Anvil and PostgreSQL 16 instance.
//! The broadcast artifact must come from the real local Foundry script.
use axum::extract::{Path, State};
use axum::response::IntoResponse;
use deopt_v2_backend::api::AppState;
use deopt_v2_backend::db::PgRepository;
use deopt_v2_backend::engine::EngineState;
use deopt_v2_backend::execution::{EthCallProvider, EthCallRequest, HttpJsonRpcProvider};
use deopt_v2_backend::options::{
    index_option_events_with_provider, list_option_execution_events, OptionEventIndexerConfig,
    OptionsConfig,
};
use deopt_v2_backend::types::AccountId;
use serde_json::Value;
use std::str::FromStr;

#[tokio::test]
#[ignore = "requires disposable local Anvil, PostgreSQL and Foundry broadcast"]
async fn real_receipt_is_indexed_in_postgres() {
    let rpc = std::env::var("OPTIONS_V4_RPC_URL").expect("local Anvil URL required");
    let pg = std::env::var("OPTIONS_V4_PG_URL").expect("disposable PostgreSQL URL required");
    let artifact =
        std::env::var("OPTIONS_V4_BROADCAST").expect("Foundry broadcast artifact required");
    assert!(rpc.starts_with("http://127.0.0.1:"), "Anvil must be local");
    assert!(pg.contains("127.0.0.1"), "PostgreSQL must be local");
    let broadcast: Value = serde_json::from_slice(&std::fs::read(artifact).expect("artifact"))
        .expect("broadcast JSON");
    let transactions = broadcast["transactions"].as_array().expect("transactions");
    let receipts = broadcast["receipts"].as_array().expect("receipts");
    let trade = transactions
        .iter()
        .rposition(|tx| {
            tx["contractName"] == "OptionMatchingEngine" && tx["transactionType"] == "CALL"
        })
        .expect("actual matching trade");
    let tx_hash = transactions[trade]["hash"].as_str().expect("tx hash");
    let receipt = &receipts[trade];
    assert_eq!(receipt["status"], "0x1");
    let block_hash = receipt["blockHash"].as_str().expect("block hash");
    let matching = transactions
        .iter()
        .find(|tx| {
            tx["contractName"] == "OptionMatchingEngine" && tx["transactionType"] == "CREATE"
        })
        .and_then(|tx| tx["contractAddress"].as_str())
        .expect("matching deployment");
    let margin = transactions
        .iter()
        .find(|tx| tx["contractName"] == "MarginEngine" && tx["transactionType"] == "CREATE")
        .and_then(|tx| tx["contractAddress"].as_str())
        .expect("margin deployment");
    let vault = transactions
        .iter()
        .find(|tx| tx["contractName"] == "CollateralVault" && tx["transactionType"] == "CREATE")
        .and_then(|tx| tx["contractAddress"].as_str())
        .expect("vault deployment");
    let registry = transactions
        .iter()
        .find(|tx| {
            tx["contractName"] == "OptionProductRegistry" && tx["transactionType"] == "CREATE"
        })
        .and_then(|tx| tx["contractAddress"].as_str())
        .expect("registry deployment");

    let mut repo = None;
    for _ in 0..12 {
        if let Ok(connected) = PgRepository::connect(&pg).await {
            repo = Some(connected);
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    let repo = repo.expect("connect disposable PG after host readiness");
    repo.run_migrations().await.expect("migrations");
    let mut state = AppState::with_options_config_and_repository(
        EngineState::with_default_markets(),
        OptionsConfig::enabled_in_memory_for_tests(),
        repo.clone(),
    );
    state.chain_id = 31337;
    state.execution_config.rpc_url = Some(rpc.clone());
    state.trading_views.margin_engine_address = Some(AccountId::new(margin));
    state.option_event_indexer_config = OptionEventIndexerConfig {
        enabled: true,
        poll_interval_ms: 1000,
        from_block: 0,
        batch_blocks: 1000,
        confirmation_blocks: 0,
        require_rpc: true,
        rpc_url: Some(rpc.clone()),
        matching_engine_address: AccountId::new(matching),
        margin_engine_address: AccountId::new(margin),
        collateral_vault_address: AccountId::new(vault),
        fees_manager_address: None,
        fees_manager_v2_address: None,
        old_margin_engine_address: None,
        protocol_fee_vault_address: None,
    };
    let provider = HttpJsonRpcProvider::new(rpc.clone());
    let first = index_option_events_with_provider(&state, &provider)
        .await
        .expect("index actual logs");
    assert!(first.events_indexed > 0, "real logs must persist");
    let events = list_option_execution_events(&state, 1000)
        .await
        .expect("persisted events");
    let indexed = events
        .iter()
        .find(|e| e.tx_hash.eq_ignore_ascii_case(tx_hash) && e.event_name == "OptionTradeExecuted")
        .expect("matching event in PG");
    assert_eq!(indexed.block_hash.as_deref(), Some(block_hash));
    assert!(indexed.option_id.is_some());
    assert!(indexed.buyer.is_some());
    assert!(indexed.seller.is_some());
    let option_id = alloy_primitives::U256::from_str(indexed.option_id.as_deref().unwrap())
        .expect("decoded canonical option ID");
    let mut calldata = Vec::with_capacity(68);
    calldata.extend_from_slice(
        &alloy_primitives::keccak256(b"getPositionQuantity(address,uint256)")[..4],
    );
    let check_quantity = |wallet: &str| {
        let mut call = calldata.clone();
        let address = alloy_primitives::Address::from_str(wallet).expect("event wallet address");
        call.extend_from_slice(&[0u8; 12]);
        call.extend_from_slice(address.as_slice());
        call.extend_from_slice(&option_id.to_be_bytes::<32>());
        EthCallRequest {
            from: AccountId::new(wallet),
            to: AccountId::new(margin),
            data: call,
            value: 0,
            gas_limit: None,
        }
    };
    let buyer_qty = provider
        .eth_call(check_quantity(indexed.buyer.as_deref().unwrap()))
        .await
        .expect("buyer canonical position")
        .output;
    let seller_qty = provider
        .eth_call(check_quantity(indexed.seller.as_deref().unwrap()))
        .await
        .expect("seller canonical position")
        .output;
    assert_eq!(buyer_qty.len(), 32);
    assert_eq!(buyer_qty[31], 1);
    assert!(buyer_qty[..31].iter().all(|b| *b == 0));
    assert_eq!(seller_qty, vec![0xff; 32]);
    let buyer_wallet = indexed.buyer.as_deref().unwrap().to_string();
    let seller_wallet = indexed.seller.as_deref().unwrap().to_string();
    let buyer_view = serde_json::to_value(
        deopt_v2_backend::api::trading::account_positions(
            State(state.clone()),
            Path(buyer_wallet.clone()),
        )
        .await
        .expect("receipt-backed buyer API")
        .0,
    )
    .unwrap();
    let seller_view = serde_json::to_value(
        deopt_v2_backend::api::trading::account_positions(
            State(state.clone()),
            Path(seller_wallet.clone()),
        )
        .await
        .expect("receipt-backed seller API")
        .0,
    )
    .unwrap();
    assert_eq!(buyer_view["data"]["source"], "onchain_reconciled");
    assert_eq!(
        buyer_view["data"]["positions"][0]["quantity_contracts"],
        "1"
    );
    assert_eq!(buyer_view["data"]["positions"][0]["side"], "long");
    assert_eq!(seller_view["data"]["positions"][0]["side"], "short");
    println!("CANONICAL_API_POSITION=buyer:long:1,seller:short:1");
    let second = index_option_events_with_provider(&state, &provider)
        .await
        .expect("idempotent tick");
    assert_eq!(second.events_indexed, 0);
    println!("REAL_ANVIL_TX={tx_hash}");
    println!("REAL_BLOCK_HASH={block_hash}");
    println!("RUST_PG_INDEXED_EVENTS={}", first.events_indexed);
    println!("ONCHAIN_POSITION=buyer:+1,seller:-1");
    println!(
        "OPTION_ID={}",
        indexed.option_id.as_deref().unwrap_or_default()
    );
    let client = reqwest::Client::new();
    rpc_request(
        &client,
        &rpc,
        "evm_increaseTime",
        serde_json::json!([691200]),
    )
    .await;
    rpc_request(&client, &rpc, "evm_mine", serde_json::json!([])).await;
    let mut set_price = selector("setSettlementPrice(uint256,uint256)");
    set_price.extend_from_slice(&option_id.to_be_bytes::<32>());
    set_price.extend_from_slice(
        &alloy_primitives::U256::from(2500u64 * 100_000_000).to_be_bytes::<32>(),
    );
    let price_tx = send_local(&client, &rpc, registry, &set_price).await;
    let mut settle = selector("settleAccount(uint256,address)");
    settle.extend_from_slice(&option_id.to_be_bytes::<32>());
    let seller = alloy_primitives::Address::from_str(indexed.seller.as_deref().unwrap()).unwrap();
    settle.extend_from_slice(&[0u8; 12]);
    settle.extend_from_slice(seller.as_slice());
    let seller_tx = send_local(&client, &rpc, margin, &settle).await;
    settle.truncate(36);
    let buyer = alloy_primitives::Address::from_str(indexed.buyer.as_deref().unwrap()).unwrap();
    settle.extend_from_slice(&[0u8; 12]);
    settle.extend_from_slice(buyer.as_slice());
    let buyer_tx = send_local(&client, &rpc, margin, &settle).await;
    let buyer_closed = provider
        .eth_call(check_quantity(indexed.buyer.as_deref().unwrap()))
        .await
        .expect("buyer settled position")
        .output;
    let seller_closed = provider
        .eth_call(check_quantity(indexed.seller.as_deref().unwrap()))
        .await
        .expect("seller settled position")
        .output;
    assert_eq!(buyer_closed, vec![0; 32]);
    assert_eq!(seller_closed, vec![0; 32]);
    let settlement_tick = index_option_events_with_provider(&state, &provider)
        .await
        .expect("index actual settlement receipts");
    assert!(settlement_tick.events_indexed >= 2);
    let settled_events = list_option_execution_events(&state, 1000)
        .await
        .expect("settlement rows");
    assert_eq!(
        settled_events
            .iter()
            .filter(|e| e.event_name == "AccountSettled"
                && (e.tx_hash == buyer_tx || e.tx_hash == seller_tx))
            .count(),
        2
    );
    let settled_view = serde_json::to_value(
        deopt_v2_backend::api::trading::account_positions(
            State(state.clone()),
            Path(buyer_wallet.clone()),
        )
        .await
        .expect("settled buyer API")
        .0,
    )
    .unwrap();
    assert_eq!(
        settled_view["data"]["positions"].as_array().unwrap().len(),
        0
    );
    println!("SETTLEMENT_PRICE_TX={price_tx}");
    println!("SELLER_SETTLEMENT_TX={seller_tx}");
    println!("BUYER_SETTLEMENT_TX={buyer_tx}");
    println!("CANONICAL_POSITION_AFTER_SETTLEMENT=buyer:0,seller:0");
    if let Ok(snapshot) = std::env::var("OPTIONS_V4_SNAPSHOT") {
        let revert: Value = client
            .post(&rpc)
            .json(&serde_json::json!({
                "jsonrpc":"2.0", "id":1, "method":"evm_revert", "params":[snapshot]
            }))
            .send()
            .await
            .expect("Anvil revert request")
            .json()
            .await
            .expect("Anvil revert JSON");
        assert!(
            revert.get("error").is_none(),
            "Anvil snapshot revert RPC error"
        );
        assert_eq!(revert["result"], true, "snapshot revert must succeed");
        let mine: Value = client
            .post(&rpc)
            .json(&serde_json::json!({
                "jsonrpc":"2.0", "id":2, "method":"anvil_mine", "params":["0x40"]
            }))
            .send()
            .await
            .expect("Anvil mine request")
            .json()
            .await
            .expect("Anvil mine JSON");
        assert!(mine.get("error").is_none(), "Anvil replacement chain");
        let reindexed = index_option_events_with_provider(&state, &provider)
            .await
            .expect("reorg replay");
        assert_eq!(reindexed.events_indexed, 0);
        let after = list_option_execution_events(&state, 1000)
            .await
            .expect("post-reorg events");
        assert!(!after
            .iter()
            .any(|e| e.tx_hash.eq_ignore_ascii_case(tx_hash)));
        let orphan = deopt_v2_backend::api::trading::account_positions(
            State(state.clone()),
            Path(buyer_wallet),
        )
        .await
        .expect_err("an orphaned receipt must not produce a position");
        assert_eq!(
            orphan.into_response().status(),
            axum::http::StatusCode::SERVICE_UNAVAILABLE
        );
        println!("REAL_REORG_ORPHAN_REMOVAL=PASS");
    }
}

fn selector(signature: &str) -> Vec<u8> {
    alloy_primitives::keccak256(signature.as_bytes())[..4].to_vec()
}

async fn rpc_request(client: &reqwest::Client, url: &str, method: &str, params: Value) -> Value {
    let answer: Value = client
        .post(url)
        .json(&serde_json::json!({
            "jsonrpc":"2.0", "id":1, "method":method, "params":params
        }))
        .send()
        .await
        .expect("local RPC request")
        .json()
        .await
        .expect("local RPC response");
    assert!(
        answer.get("error").is_none(),
        "local Anvil RPC {method} failed: {answer}"
    );
    answer["result"].clone()
}

async fn send_local(client: &reqwest::Client, url: &str, to: &str, data: &[u8]) -> String {
    let calldata = format!(
        "0x{}",
        data.iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    );
    let owner = "0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266"; // public Anvil account
    let hash = rpc_request(
        client,
        url,
        "eth_sendTransaction",
        serde_json::json!([{
            "from":owner, "to":to, "data":calldata
        }]),
    )
    .await
    .as_str()
    .expect("local transaction hash")
    .to_string();
    let receipt = rpc_request(
        client,
        url,
        "eth_getTransactionReceipt",
        serde_json::json!([&hash]),
    )
    .await;
    assert_eq!(
        receipt["status"], "0x1",
        "local settlement transaction failed"
    );
    hash
}
