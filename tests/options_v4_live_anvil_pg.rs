//! Opt-in proof against a disposable Anvil and PostgreSQL 16 instance.
//! The broadcast artifact must come from the real local Foundry script.
use alloy_primitives::{B256, U256};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::{
    routing::{get, post},
    Extension, Json,
};
use deopt_v2_backend::api::AppState;
use deopt_v2_backend::db::PgRepository;
use deopt_v2_backend::engine::EngineState;
use deopt_v2_backend::execution::{EthCallProvider, EthCallRequest, HttpJsonRpcProvider};
use deopt_v2_backend::options::{
    encode_option_execute_trade_calldata, index_option_events_with_provider,
    list_option_execution_events, option_trade_digest_bytes, OptionEventIndexerConfig,
    OptionTradePayload, OptionTradeSignatureBundle, OptionsConfig,
};
use deopt_v2_backend::signing::{recover_eip712_signer, Eip712Domain};
use deopt_v2_backend::types::AccountId;
use serde_json::Value;
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use tower_http::cors::CorsLayer;

#[derive(Clone)]
struct LocalTradeAdapter {
    state: AppState,
    rpc: String,
    matching: String,
    payload: OptionTradePayload,
    domain: Eip712Domain,
    typed_data: Value,
    tx_hash: Arc<Mutex<Option<String>>>,
    indexed_count: Arc<Mutex<u64>>,
}

#[derive(serde::Deserialize)]
struct BrowserSignatures {
    buyer_signature: String,
    seller_signature: String,
}

async fn local_trade(Extension(adapter): Extension<LocalTradeAdapter>) -> Json<Value> {
    Json(adapter.typed_data)
}

async fn local_execute(
    Extension(adapter): Extension<LocalTradeAdapter>,
    Json(input): Json<BrowserSignatures>,
) -> std::result::Result<Json<Value>, StatusCode> {
    if adapter.tx_hash.lock().unwrap().is_some() {
        return Err(StatusCode::CONFLICT);
    }
    let digest = option_trade_digest_bytes(&adapter.payload, &adapter.domain)
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    let buyer = recover_eip712_signer(&digest, &input.buyer_signature)
        .map_err(|_| StatusCode::UNAUTHORIZED)?;
    let seller = recover_eip712_signer(&digest, &input.seller_signature)
        .map_err(|_| StatusCode::UNAUTHORIZED)?;
    if !buyer.0.eq_ignore_ascii_case(&adapter.payload.buyer.0)
        || !seller.0.eq_ignore_ascii_case(&adapter.payload.seller.0)
    {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let bundle = OptionTradeSignatureBundle::new(&input.buyer_signature, &input.seller_signature)
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    let calldata = encode_option_execute_trade_calldata(&adapter.payload, &bundle)
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    let client = reqwest::Client::new();
    let hash = send_local(&client, &adapter.rpc, &adapter.matching, &calldata).await;
    let provider = HttpJsonRpcProvider::new(adapter.rpc.clone());
    let tick = index_option_events_with_provider(&adapter.state, &provider)
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    if tick.events_indexed == 0 {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }
    *adapter.indexed_count.lock().unwrap() = tick.events_indexed;
    *adapter.tx_hash.lock().unwrap() = Some(hash.clone());
    Ok(Json(
        serde_json::json!({"tx_hash":hash,"indexed_events":tick.events_indexed}),
    ))
}

fn series_word(data: &[u8], index: usize) -> U256 {
    U256::from_be_slice(&data[index * 32..(index + 1) * 32])
}

async fn browser_trade_adapter(
    broadcast: &Value,
    state: &AppState,
    rpc: &str,
    matching: &str,
) -> LocalTradeAdapter {
    let txs = broadcast["transactions"].as_array().unwrap();
    let receipts = broadcast["receipts"].as_array().unwrap();
    let create = txs
        .iter()
        .position(|tx| {
            tx["contractName"] == "OptionProductRegistry"
                && tx["function"]
                    .as_str()
                    .unwrap_or_default()
                    .starts_with("createSeries")
        })
        .expect("actual local series creation");
    let log = &receipts[create]["logs"][0];
    let topics = log["topics"].as_array().expect("SeriesCreated topics");
    assert_eq!(topics.len(), 4);
    let id = U256::from_str_radix(&topics[1].as_str().unwrap()[2..], 16).unwrap();
    let underlying = format!("0x{}", &topics[2].as_str().unwrap()[26..]);
    let settlement = format!("0x{}", &topics[3].as_str().unwrap()[26..]);
    let data = hex::decode(log["data"].as_str().unwrap().trim_start_matches("0x")).unwrap();
    assert_eq!(data.len(), 160);
    let expiry = u64::try_from(series_word(&data, 0)).unwrap();
    let strike = u64::try_from(series_word(&data, 1)).unwrap();
    let contract_size = u128::try_from(series_word(&data, 2)).unwrap();
    assert_eq!(series_word(&data, 3), U256::from(1));
    assert_eq!(series_word(&data, 4), U256::from(1));
    let client = reqwest::Client::new();
    let latest = rpc_request(
        &client,
        rpc,
        "eth_getBlockByNumber",
        serde_json::json!(["latest", false]),
    )
    .await;
    let timestamp = u128::from_str_radix(
        latest["timestamp"]
            .as_str()
            .unwrap()
            .trim_start_matches("0x"),
        16,
    )
    .unwrap();
    let payload = OptionTradePayload {
        intent_id: B256::from(alloy_primitives::keccak256(b"deopt-v4-browser-local-trade")),
        buyer: AccountId::new("0x0376AAc07Ad725E01357B1725B5ceC61aE10473c"),
        seller: AccountId::new("0xb040e0fAac56886b0f29Af446544AED0A154ED29"),
        option_id: id,
        underlying: AccountId::new(underlying),
        settlement_asset: AccountId::new(settlement),
        expiry,
        strike_1e8: strike,
        is_call: true,
        contract_size_1e8: contract_size,
        quantity: 1,
        premium_per_contract: 100_000_000,
        buyer_is_maker: true,
        buyer_nonce: 0,
        seller_nonce: 0,
        deadline: timestamp + 3600,
    };
    payload
        .validate()
        .expect("series event and local trade metadata agree");
    let domain = Eip712Domain {
        name: "DeOptV2-OptionMatchingEngine".into(),
        version: "1".into(),
        chain_id: 84532,
        verifying_contract: AccountId::new(matching),
    };
    let typed_data = serde_json::json!({
        "domain":{"name":domain.name,"version":domain.version,"chainId":84532,"verifyingContract":matching},
        "primaryType":"OptionTrade",
        "types":{"OptionTrade":[
            {"name":"intentId","type":"bytes32"},{"name":"buyer","type":"address"},
            {"name":"seller","type":"address"},{"name":"optionId","type":"uint256"},
            {"name":"underlying","type":"address"},{"name":"settlementAsset","type":"address"},
            {"name":"expiry","type":"uint64"},{"name":"strike1e8","type":"uint64"},
            {"name":"isCall","type":"bool"},{"name":"contractSize1e8","type":"uint128"},
            {"name":"quantity","type":"uint128"},{"name":"premiumPerContract","type":"uint128"},
            {"name":"buyerIsMaker","type":"bool"},{"name":"buyerNonce","type":"uint256"},
            {"name":"sellerNonce","type":"uint256"},{"name":"deadline","type":"uint256"}
        ]},
        "message":{
            "intentId":format!("{:#x}",payload.intent_id),"buyer":payload.buyer.0,
            "seller":payload.seller.0,"optionId":payload.option_id.to_string(),
            "underlying":payload.underlying.0,"settlementAsset":payload.settlement_asset.0,
            "expiry":payload.expiry,"strike1e8":payload.strike_1e8,"isCall":true,
            "contractSize1e8":payload.contract_size_1e8.to_string(),"quantity":"1",
            "premiumPerContract":payload.premium_per_contract.to_string(),"buyerIsMaker":true,
            "buyerNonce":"0","sellerNonce":"0","deadline":payload.deadline.to_string()
        }
    });
    LocalTradeAdapter {
        state: state.clone(),
        rpc: rpc.into(),
        matching: matching.into(),
        payload,
        domain,
        typed_data,
        tx_hash: Arc::new(Mutex::new(None)),
        indexed_count: Arc::new(Mutex::new(0)),
    }
}

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
    let browser = std::env::var("OPTIONS_V4_BROWSER").as_deref() == Ok("1");
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
    let local_chain = std::env::var("OPTIONS_V4_CHAIN_ID")
        .unwrap_or_else(|_| "31337".into())
        .parse::<u64>()
        .expect("local chain ID");
    assert!(local_chain == 31337 || local_chain == 84532);
    state.chain_id = local_chain;
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
    let (tx_hash, receipt, browser_indexed) = if browser {
        assert_eq!(
            local_chain, 84532,
            "browser bench must use local chain 84532"
        );
        let adapter = browser_trade_adapter(&broadcast, &state, &rpc, matching).await;
        let option_id = adapter.payload.option_id.to_string();
        run_browser(&state, adapter.clone(), &option_id, "trade").await;
        let hash = adapter
            .tx_hash
            .lock()
            .unwrap()
            .clone()
            .expect("browser submitted trade");
        let indexed = *adapter.indexed_count.lock().unwrap();
        let receipt = rpc_request(
            &reqwest::Client::new(),
            &rpc,
            "eth_getTransactionReceipt",
            serde_json::json!([hash]),
        )
        .await;
        (hash, receipt, indexed)
    } else {
        let trade = transactions
            .iter()
            .rposition(|tx| {
                tx["contractName"] == "OptionMatchingEngine" && tx["transactionType"] == "CALL"
            })
            .expect("Foundry submitted local trade");
        (
            transactions[trade]["hash"].as_str().unwrap().to_owned(),
            receipts[trade].clone(),
            0,
        )
    };
    assert_eq!(receipt["status"], "0x1", "actual trade receipt");
    let block_hash = receipt["blockHash"]
        .as_str()
        .expect("block hash")
        .to_owned();
    let first = index_option_events_with_provider(&state, &provider)
        .await
        .expect("index actual logs");
    assert!(
        first.events_indexed + browser_indexed > 0,
        "real logs must persist"
    );
    let events = list_option_execution_events(&state, 1000)
        .await
        .expect("persisted events");
    let indexed = events
        .iter()
        .find(|e| e.tx_hash.eq_ignore_ascii_case(&tx_hash) && e.event_name == "OptionTradeExecuted")
        .expect("matching event in PG");
    assert_eq!(indexed.block_hash.as_deref(), Some(block_hash.as_str()));
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
    println!(
        "RUST_PG_INDEXED_EVENTS={}",
        first.events_indexed + browser_indexed
    );
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
    if browser {
        let adapter = browser_trade_adapter(&broadcast, &state, &rpc, matching).await;
        run_browser(&state, adapter, &option_id.to_string(), "settled").await;
        println!("PLAYWRIGHT_SETTLED_POSITION=PASS");
    }
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
        if browser {
            assert!(
                reindexed.events_indexed > 0,
                "canonical pre-trade events must be replayed"
            );
        }
        let stable = index_option_events_with_provider(&state, &provider)
            .await
            .expect("stable replacement-chain tick");
        assert_eq!(
            stable.events_indexed, 0,
            "replayed events must be idempotent"
        );
        let after = list_option_execution_events(&state, 1000)
            .await
            .expect("post-reorg events");
        assert!(!after
            .iter()
            .any(|e| e.tx_hash.eq_ignore_ascii_case(&tx_hash)));
        let after_reorg = deopt_v2_backend::api::trading::account_positions(
            State(state.clone()),
            Path(buyer_wallet),
        )
        .await;
        if browser {
            let body =
                serde_json::to_value(after_reorg.expect("deployed contracts survive reorg").0)
                    .unwrap();
            assert_eq!(body["data"]["source"], "onchain_reconciled");
            assert_eq!(body["data"]["positions"].as_array().unwrap().len(), 0);
            let adapter = browser_trade_adapter(&broadcast, &state, &rpc, matching).await;
            run_browser(&state, adapter, &option_id.to_string(), "reorg").await;
            println!("PLAYWRIGHT_REORG_POSITION_REMOVED=PASS");
        } else {
            let orphan = after_reorg.expect_err("a reverted deployment must fail closed");
            assert_eq!(
                orphan.into_response().status(),
                StatusCode::SERVICE_UNAVAILABLE
            );
        }
        println!("REAL_REORG_ORPHAN_REMOVAL=PASS");
    }
}

async fn run_browser(state: &AppState, adapter: LocalTradeAdapter, option_id: &str, phase: &str) {
    let api_port = std::env::var("OPTIONS_V4_API_PORT").expect("local API port");
    let frontend_port = std::env::var("OPTIONS_V4_FRONTEND_PORT").expect("local frontend port");
    let mut browser_state = state.clone();
    browser_state.wallet_read_auth.enabled = true;
    browser_state.wallet_read_auth.origin = format!("http://127.0.0.1:{frontend_port}");
    let local_routes = axum::Router::new()
        .route("/local/options-v4/trade", get(local_trade))
        .route("/local/options-v4/execute", post(local_execute))
        .layer(Extension(adapter));
    let app = deopt_v2_backend::api::router(browser_state)
        .merge(local_routes)
        .layer(CorsLayer::permissive()); // disposable loopback test server only
    let listener = tokio::net::TcpListener::bind(format!("127.0.0.1:{api_port}"))
        .await
        .expect("local-only Rust API listener");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("local Rust API");
    });
    let frontend = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("deopt-v2-frontend");
    let buyer = "0x0376AAc07Ad725E01357B1725B5ceC61aE10473c".to_owned();
    let seller = "0xb040e0fAac56886b0f29Af446544AED0A154ED29".to_owned();
    let option_id = option_id.to_owned();
    let phase = phase.to_owned();
    let output = tokio::task::spawn_blocking(move || {
        std::process::Command::new("npx")
            .current_dir(frontend)
            .args([
                "playwright",
                "test",
                "tests/e2e/options-v4-live-position.spec.ts",
                "--project=chromium",
            ])
            .env("E2E_BASE_URL", format!("http://127.0.0.1:{frontend_port}"))
            .env("OPTIONS_V4_API_URL", format!("http://127.0.0.1:{api_port}"))
            .env("OPTIONS_V4_BUYER", buyer)
            .env("OPTIONS_V4_SELLER", seller)
            .env("OPTIONS_V4_OPTION_ID", option_id)
            .env("OPTIONS_V4_PHASE", phase)
            .output()
    })
    .await
    .expect("Playwright join")
    .expect("Playwright process");
    server.abort();
    assert!(
        output.status.success(),
        "Playwright failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    println!("PLAYWRIGHT_LIVE_POSITION=PASS");
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
