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
    OptionTradePayload, OptionTradeSignatureBundle, OptionsConfig, OptionSeries,
    OptionSeriesSource, OptionSeriesStatus, OptionExecutionSignatureMode,
};
use deopt_v2_backend::execution::config::PrivateKeySecret;
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
    calldata: Arc<Mutex<Option<Vec<u8>>>>,
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
    *adapter.calldata.lock().unwrap() = Some(calldata.clone());
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

fn created_series_log<'a>(broadcast: &'a Value, registry: &str) -> &'a Value {
    let topic = format!("0x{}", hex::encode(alloy_primitives::keccak256(
        b"SeriesCreated(uint256,address,address,uint64,uint64,uint128,bool,bool)"
    )));
    broadcast["receipts"].as_array().expect("receipts").iter()
        .flat_map(|receipt| receipt["logs"].as_array().into_iter().flatten())
        .find(|log| {
            log["address"].as_str().is_some_and(|address| address.eq_ignore_ascii_case(registry))
                && log["topics"][0].as_str().is_some_and(|value| value.eq_ignore_ascii_case(&topic))
        })
        .expect("real SeriesCreated log from dedicated Registry")
}

async fn browser_trade_adapter(
    broadcast: &Value,
    state: &AppState,
    rpc: &str,
    matching: &str,
    registry: &str,
) -> LocalTradeAdapter {
    let log = created_series_log(broadcast, registry);
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
    if std::env::var("OPTIONS_V5_PRODUCT").as_deref() == Ok("1") {
        state.repository.as_ref().expect("disposable PG").insert_option_series(&OptionSeries {
            option_series_id: "local-call-v5".into(),
            underlying: payload.underlying.0.clone(),
            base_asset: payload.underlying.0.clone(),
            quote_asset: payload.settlement_asset.0.clone(),
            settlement_asset: payload.settlement_asset.0.clone(),
            expiry,
            strike_1e8: strike as u128,
            is_call: true,
            contract_size_1e8: contract_size,
            status: OptionSeriesStatus::Active,
            source: OptionSeriesSource::Onchain,
            onchain_product_id: None,
            onchain_series_id: Some(id.to_string()),
            created_at_ms: (timestamp as i64) * 1000,
            updated_at_ms: (timestamp as i64) * 1000,
        }).await.expect("seed real deployed series metadata");
    }
    LocalTradeAdapter {
        state: state.clone(),
        rpc: rpc.into(),
        matching: matching.into(),
        payload,
        domain,
        typed_data,
        tx_hash: Arc::new(Mutex::new(None)),
        indexed_count: Arc::new(Mutex::new(0)),
        calldata: Arc::new(Mutex::new(None)),
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
    let verified_settlement = std::env::var("OPTIONS_V13_VERIFIED_SETTLEMENT").as_deref() == Ok("1");
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
    let fees_manager = transactions
        .iter()
        .find(|tx| tx["contractName"] == "FeesManagerV2" && tx["transactionType"] == "CREATE")
        .and_then(|tx| tx["contractAddress"].as_str())
        .expect("local fee manager deployment");
    let registry = transactions
        .iter()
        .find(|tx| {
            tx["contractName"] == "OptionProductRegistry" && tx["transactionType"] == "CREATE"
        })
        .and_then(|tx| tx["contractAddress"].as_str())
        .expect("registry deployment");
    let deployment = |name: &str| {
        transactions.iter()
            .find(|tx| tx["contractName"] == name && tx["transactionType"] == "CREATE")
            .and_then(|tx| tx["contractAddress"].as_str())
            .expect("V13 contract deployment")
    };
    let verifier = verified_settlement.then(|| deployment("VerifiedOptionSettlementOwner"));
    let historical_feed = verified_settlement.then(|| deployment("LocalHistoricalChainlinkFeed"));

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
    let v5 = std::env::var("OPTIONS_V5_PRODUCT").as_deref() == Ok("1");
    let mut options_config = OptionsConfig::enabled_in_memory_for_tests();
    if v5 {
        options_config.execution_enabled = true;
        options_config.matching_engine_address = AccountId::new(matching);
        options_config.execution_eip712_domain.verifying_contract = AccountId::new(matching);
        options_config.execution_signature_mode = OptionExecutionSignatureMode::Strict;
        options_config.execution_simulation_enabled = true;
        options_config.execution_simulation_rpc_url = Some(rpc.clone());
        options_config.execution_simulation_gas_limit = 1_500_000;
        options_config.execution_broadcast_enabled = true;
        options_config.execution_broadcast_gas_limit = 1_500_000;
    }
    let mut state = AppState::with_options_config_and_repository(
        EngineState::with_default_markets(),
        options_config,
        repo.clone(),
    );
    let local_chain = std::env::var("OPTIONS_V4_CHAIN_ID")
        .unwrap_or_else(|_| "31337".into())
        .parse::<u64>()
        .expect("local chain ID");
    assert!(local_chain == 31337 || local_chain == 84532);
    state.chain_id = local_chain;
    state.execution_config.rpc_url = Some(rpc.clone());
    if v5 {
        state.execution_config.execution_enabled = true;
        state.execution_config.real_broadcast_enabled = true;
        state.execution_config.dry_run = false;
        state.execution_config.executor_chain_id = local_chain;
        state.execution_config.executor_allow_local_signer = true;
        state.execution_config.max_fee_per_gas_wei = Some("10000000000".into());
        state.execution_config.max_priority_fee_per_gas_wei = Some("1000000000".into());
        state.execution_config.executor_private_key = Some(PrivateKeySecret::new(
            "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80".into(),
        ));
        state.execution_config.executor_from_address = AccountId::new(
            "0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266",
        );
    }
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
        fees_manager_v2_address: Some(AccountId::new(fees_manager)),
        old_margin_engine_address: None,
        protocol_fee_vault_address: None,
    };
    let provider = HttpJsonRpcProvider::new(rpc.clone());
    let (tx_hash, receipt, browser_indexed, browser_calldata, pre_balances) = if browser {
        assert_eq!(
            local_chain, 84532,
            "browser bench must use local chain 84532"
        );
        let adapter = browser_trade_adapter(&broadcast, &state, &rpc, matching, registry).await;
        let pre_provider = HttpJsonRpcProvider::new(rpc.clone());
        let buyer_before = vault_balance(&pre_provider, vault, &adapter.payload.buyer.0, &adapter.payload.settlement_asset.0).await;
        let seller_before = vault_balance(&pre_provider, vault, &adapter.payload.seller.0, &adapter.payload.settlement_asset.0).await;
        let recipient_before = vault_balance(&pre_provider, vault, "0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266", &adapter.payload.settlement_asset.0).await;
        let asset = adapter.payload.settlement_asset.0.clone();
        let option_id = adapter.payload.option_id.to_string();
        run_browser(&state, adapter.clone(), &option_id, "trade").await;
        let hash = adapter
            .tx_hash
            .lock()
            .unwrap()
            .clone()
            .expect("browser submitted trade");
        let indexed = *adapter.indexed_count.lock().unwrap();
        let calldata = adapter
            .calldata
            .lock()
            .unwrap()
            .clone()
            .expect("browser trade calldata");
        let receipt = rpc_request(
            &reqwest::Client::new(),
            &rpc,
            "eth_getTransactionReceipt",
            serde_json::json!([hash]),
        )
        .await;
        (hash, receipt, indexed, Some(calldata), Some((buyer_before, seller_before, recipient_before, asset)))
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
            None,
            None,
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
    if let Some((buyer_before, seller_before, recipient_before, asset)) = pre_balances.as_ref() {
        let premium = indexed.quantity_contracts.as_deref().unwrap().parse::<u128>().unwrap()
            * indexed.premium_per_contract_native.as_deref().unwrap().parse::<u128>().unwrap();
        let transfer: Vec<_> = events.iter().filter(|e| e.tx_hash.eq_ignore_ascii_case(&tx_hash)
            && e.event_name == "InternalTransfer"
            && e.decoded.as_ref().and_then(|d| d["from"].as_str()).is_some_and(|v| v.eq_ignore_ascii_case(indexed.buyer.as_deref().unwrap()))
            && e.decoded.as_ref().and_then(|d| d["to"].as_str()).is_some_and(|v| v.eq_ignore_ascii_case(indexed.seller.as_deref().unwrap()))
            && e.decoded.as_ref().and_then(|d| d["token"].as_str()).is_some_and(|v| v.eq_ignore_ascii_case(asset))
        ).collect();
        assert_eq!(transfer.len(), 1, "exactly one vault premium transfer");
        assert_eq!(transfer[0].decoded.as_ref().unwrap()["amount"], premium.to_string());
        assert_eq!(transfer[0].block_hash, indexed.block_hash);
        let buyer_after = vault_balance(&provider, vault, indexed.buyer.as_deref().unwrap(), asset).await;
        let seller_after = vault_balance(&provider, vault, indexed.seller.as_deref().unwrap(), asset).await;
        let recipient = "0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266";
        let recipient_after = vault_balance(&provider, vault, recipient, asset).await;
        let fee_events: Vec<_> = events.iter().filter(|e| e.tx_hash.eq_ignore_ascii_case(&tx_hash)
            && e.event_name == "FeeChargedV2" && e.block_hash == indexed.block_hash).collect();
        assert_eq!(fee_events.len(), 2, "both sides must pay a real OPTION fee");
        let mut buyer_fee = 0u128;
        let mut seller_fee = 0u128;
        for fee in fee_events {
            let d = fee.decoded.as_ref().unwrap();
            assert_eq!(d["productKind"], "option");
            assert!(d["consumer"].as_str().unwrap().eq_ignore_ascii_case(margin));
            assert!(d["settlementAsset"].as_str().unwrap().eq_ignore_ascii_case(asset));
            assert!(d["recipient"].as_str().unwrap().eq_ignore_ascii_case(recipient));
            assert_eq!(d["basisAmount"], premium.to_string());
            let trader = d["trader"].as_str().unwrap();
            let amount = d["feeAmount"].as_str().unwrap().parse::<u128>().unwrap();
            let ppm = d["feePpm"].as_i64().unwrap() as u128;
            assert_eq!(amount, premium * ppm / 1_000_000);
            let transfers: Vec<_> = events.iter().filter(|e| e.tx_hash.eq_ignore_ascii_case(&tx_hash)
                && e.event_name == "InternalTransfer" && e.block_hash == indexed.block_hash
                && e.decoded.as_ref().is_some_and(|v| v["from"].as_str().is_some_and(|x| x.eq_ignore_ascii_case(trader))
                    && v["to"].as_str().is_some_and(|x| x.eq_ignore_ascii_case(recipient))
                    && v["token"].as_str().is_some_and(|x| x.eq_ignore_ascii_case(asset))
                    && v["amount"].as_str().and_then(|x| x.parse::<u128>().ok()) == Some(amount))).collect();
            assert_eq!(transfers.len(), 1, "fee must be backed by a vault transfer");
            if trader.eq_ignore_ascii_case(indexed.buyer.as_deref().unwrap()) { buyer_fee = amount; }
            else if trader.eq_ignore_ascii_case(indexed.seller.as_deref().unwrap()) { seller_fee = amount; }
            else { panic!("unexpected fee trader"); }
        }
        assert_eq!(buyer_fee, 25_000);
        assert_eq!(seller_fee, 5_000);
        assert_eq!(*buyer_before - buyer_after, premium + buyer_fee);
        assert_eq!(seller_after - *seller_before, premium - seller_fee);
        assert_eq!(recipient_after - *recipient_before, buyer_fee + seller_fee);
        assert_eq!(buyer_after + seller_after + recipient_after, *buyer_before + *seller_before + *recipient_before);
        let mut excess_withdraw = selector("withdraw(address,uint256)");
        let token = alloy_primitives::Address::from_str(asset).unwrap();
        excess_withdraw.extend_from_slice(&[0u8; 12]);
        excess_withdraw.extend_from_slice(token.as_slice());
        excess_withdraw.extend_from_slice(&U256::from(10_000u64 * 1_000_000).to_be_bytes::<32>());
        assert!(local_call_reverted_as(&reqwest::Client::new(), &rpc, vault,
            &excess_withdraw, indexed.seller.as_deref().unwrap()).await,
            "direct vault withdrawal must obey real account risk");
        println!("PREMIUM_VAULT_CONSERVATION=PASS_NATIVE_{premium}");
        println!("OPTION_FEES_VAULT_CONSERVATION=PASS_BUYER_{buyer_fee}_SELLER_{seller_fee}");
        println!("EXCESS_DIRECT_VAULT_WITHDRAWAL=REJECTED_REAL_RISK_MODULE");
    }
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
    let client = reqwest::Client::new();
    if let Some(calldata) = browser_calldata.as_deref() {
        expect_local_revert(&client, &rpc, matching, calldata).await;
        println!("ONCHAIN_TRADE_REPLAY=REJECTED");
    }
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
    let price_tx = if verified_settlement {
        let feed = historical_feed.expect("local historical feed");
        let verifier = verifier.expect("verified settlement owner");
        let series_log = created_series_log(&broadcast, registry);
        let series_data = hex::decode(series_log["data"].as_str().unwrap().trim_start_matches("0x")).unwrap();
        let expiry = u64::try_from(series_word(&series_data, 0)).unwrap();
        assert!(expiry > 10);
        expect_local_revert(&client, &rpc, registry, &set_price).await;
        for (round_id, timestamp) in [(101u64, expiry - 10), (102u64, expiry + 10)] {
            let mut round = selector("setRound(uint80,int256,uint256)");
            round.extend_from_slice(&U256::from(round_id).to_be_bytes::<32>());
            round.extend_from_slice(&U256::from(2500u64 * 100_000_000).to_be_bytes::<32>());
            round.extend_from_slice(&U256::from(timestamp).to_be_bytes::<32>());
            send_local(&client, &rpc, feed, &round).await;
        }
        let mut propose = selector("propose(uint256,uint80,uint80)");
        propose.extend_from_slice(&option_id.to_be_bytes::<32>());
        propose.extend_from_slice(&U256::from(101).to_be_bytes::<32>());
        propose.extend_from_slice(&U256::from(102).to_be_bytes::<32>());
        send_local(&client, &rpc, verifier, &propose).await;
        rpc_request(&client, &rpc, "evm_increaseTime", serde_json::json!([172800])).await;
        rpc_request(&client, &rpc, "evm_mine", serde_json::json!([])).await;
        let mut single_id = selector("resolve(uint256)");
        single_id.extend_from_slice(&option_id.to_be_bytes::<32>());
        send_local(&client, &rpc, verifier, &single_id).await;
        single_id[..4].copy_from_slice(&selector("finalize(uint256)"));
        let finalized = send_local(&client, &rpc, verifier, &single_id).await;
        println!("VERIFIED_LOCAL_SETTLEMENT=CHAINLINK_PROOF_DIRECT_PRICE_REJECTED");
        finalized
    } else {
        send_local(&client, &rpc, registry, &set_price).await
    };
    let settlement_before = if let Some((_, _, _, asset)) = pre_balances.as_ref() {
        Some((
            vault_balance(&provider, vault, &buyer_wallet, asset).await,
            vault_balance(&provider, vault, &seller_wallet, asset).await,
        ))
    } else { None };
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
    expect_local_revert(&client, &rpc, margin, &settle).await;
    println!("DOUBLE_SETTLEMENT=REJECTED");
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
    if let (Some((_, _, _, asset)), Some((buyer_before_settle, seller_before_settle))) =
        (pre_balances.as_ref(), settlement_before) {
        for (wallet, tx, balance_before) in [
            (&buyer_wallet, &buyer_tx, buyer_before_settle),
            (&seller_wallet, &seller_tx, seller_before_settle),
        ] {
            let settled: Vec<_> = settled_events.iter().filter(|e| e.event_name == "AccountSettled"
                && e.tx_hash == *tx && e.account.as_deref().is_some_and(|a| a.eq_ignore_ascii_case(wallet))).collect();
            assert_eq!(settled.len(), 1);
            let d = settled[0].decoded.as_ref().unwrap();
            let collected = d["collectedFromTrader"].as_str().unwrap().parse::<u128>().unwrap();
            let paid = d["paidToTrader"].as_str().unwrap().parse::<u128>().unwrap();
            let transfers: Vec<_> = settled_events.iter().filter(|e| e.tx_hash == *tx && e.event_name == "InternalTransfer"
                && e.decoded.as_ref().and_then(|v| v["token"].as_str()).is_some_and(|v| v.eq_ignore_ascii_case(asset))).collect();
            let incoming: u128 = transfers.iter().filter(|e| e.decoded.as_ref().unwrap()["to"].as_str().unwrap().eq_ignore_ascii_case(wallet))
                .map(|e| e.decoded.as_ref().unwrap()["amount"].as_str().unwrap().parse::<u128>().unwrap()).sum();
            let outgoing: u128 = transfers.iter().filter(|e| e.decoded.as_ref().unwrap()["from"].as_str().unwrap().eq_ignore_ascii_case(wallet))
                .map(|e| e.decoded.as_ref().unwrap()["amount"].as_str().unwrap().parse::<u128>().unwrap()).sum();
            assert_eq!(incoming, paid);
            assert_eq!(outgoing, collected);
            let balance_after = vault_balance(&provider, vault, wallet, asset).await;
            assert_eq!(balance_after.checked_add(outgoing), balance_before.checked_add(incoming));
        }
        println!("SETTLEMENT_VAULT_TRANSFERS=PASS");
        println!("SETTLEMENT_WALLET_BALANCES=PASS");
    }
    if let Some((buyer_before, seller_before, recipient_before, asset)) = pre_balances.as_ref() {
        let buyer_after = vault_balance(&provider, vault, &buyer_wallet, asset).await;
        let seller_after = vault_balance(&provider, vault, &seller_wallet, asset).await;
        let recipient_after = vault_balance(&provider, vault, "0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266", asset).await;
        let buyer_settled = settled_events.iter().find(|e| e.event_name == "AccountSettled"
            && e.tx_hash == buyer_tx && e.account.as_deref().is_some_and(|a| a.eq_ignore_ascii_case(&buyer_wallet))).unwrap();
        let seller_settled = settled_events.iter().find(|e| e.event_name == "AccountSettled"
            && e.tx_hash == seller_tx && e.account.as_deref().is_some_and(|a| a.eq_ignore_ascii_case(&seller_wallet))).unwrap();
        let buyer_paid = buyer_settled.decoded.as_ref().unwrap()["paidToTrader"].as_str().unwrap().parse::<u128>().unwrap();
        let seller_collected = seller_settled.decoded.as_ref().unwrap()["collectedFromTrader"].as_str().unwrap().parse::<u128>().unwrap();
        assert_eq!(buyer_settled.decoded.as_ref().unwrap()["badDebt"], "0");
        assert_eq!(seller_settled.decoded.as_ref().unwrap()["badDebt"], "0");
        let premium = indexed.quantity_contracts.as_deref().unwrap().parse::<u128>().unwrap()
            * indexed.premium_per_contract_native.as_deref().unwrap().parse::<u128>().unwrap();
        let fees: Vec<_> = events.iter().filter(|e| e.tx_hash.eq_ignore_ascii_case(&tx_hash)
            && e.event_name == "FeeChargedV2").collect();
        let fee_for = |wallet: &str| -> u128 { fees.iter()
            .find(|e| e.account.as_deref().is_some_and(|a| a.eq_ignore_ascii_case(wallet)))
            .unwrap().decoded.as_ref().unwrap()["feeAmount"].as_str().unwrap().parse().unwrap() };
        let buyer_fee = fee_for(&buyer_wallet);
        let seller_fee = fee_for(&seller_wallet);
        assert_eq!(buyer_after + premium + buyer_fee, *buyer_before + buyer_paid);
        assert_eq!(seller_after + seller_collected + seller_fee, *seller_before + premium);
        assert_eq!(recipient_after, *recipient_before + buyer_fee + seller_fee);
        assert_eq!(buyer_after + seller_after + recipient_after,
            *buyer_before + *seller_before + *recipient_before);
        let buyer_realized = buyer_paid as i128 - premium as i128 - buyer_fee as i128;
        let seller_realized = premium as i128 - seller_collected as i128 - seller_fee as i128;
        println!("REALIZED_CASHFLOW_NATIVE=buyer:{buyer_realized},seller:{seller_realized}");
        println!("FULL_VAULT_CONSERVATION=PASS");
    }
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
        let adapter = browser_trade_adapter(&broadcast, &state, &rpc, matching, registry).await;
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
            let adapter = browser_trade_adapter(&broadcast, &state, &rpc, matching, registry).await;
            run_browser(&state, adapter.clone(), &option_id.to_string(), "reorg").await;
            println!("PLAYWRIGHT_REORG_POSITION_REMOVED=PASS");
            let calldata = browser_calldata.as_deref().expect("browser trade calldata");
            rpc_request(
                &client,
                &rpc,
                "eth_call",
                serde_json::json!([{
                "from":"0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266",
                "to":matching, "data":format!("0x{}",hex::encode(calldata))
            }, "latest"]),
            )
            .await;
            if std::env::var("OPTIONS_V5_PRODUCT").as_deref() == Ok("1") {
                let oracle = transactions.iter().find(|tx| tx["contractName"] == "ScenarioOptionOracle"
                    && tx["transactionType"] == "CREATE")
                    .and_then(|tx| tx["contractAddress"].as_str()).expect("local oracle deployment");
                let mut set_oracle = selector("setPrice(address,address,uint256,uint256,bool)");
                for address in [&adapter.payload.underlying.0, &adapter.payload.settlement_asset.0] {
                    let parsed = alloy_primitives::Address::from_str(address).unwrap();
                    set_oracle.extend_from_slice(&[0u8; 12]);
                    set_oracle.extend_from_slice(parsed.as_slice());
                }
                set_oracle.extend_from_slice(&U256::from(2000u64 * 100_000_000).to_be_bytes::<32>());
                let latest = rpc_request(&client, &rpc, "eth_getBlockByNumber", serde_json::json!(["latest",false])).await;
                let timestamp = u64::from_str_radix(latest["timestamp"].as_str().unwrap().trim_start_matches("0x"),16).unwrap();
                set_oracle.extend_from_slice(&U256::from(timestamp).to_be_bytes::<32>());
                let mut invalid = set_oracle.clone();
                invalid.extend_from_slice(&[0u8; 32]);
                send_local(&client, &rpc, oracle, &invalid).await;
                expect_local_revert(&client, &rpc, matching, calldata).await;
                println!("INVALID_ORACLE_CONTRACT_REJECTION=PASS");
                let mut stale = selector("setPrice(address,address,uint256,uint256,bool)");
                for address in [&adapter.payload.underlying.0, &adapter.payload.settlement_asset.0] {
                    let parsed = alloy_primitives::Address::from_str(address).unwrap();
                    stale.extend_from_slice(&[0u8; 12]);
                    stale.extend_from_slice(parsed.as_slice());
                }
                stale.extend_from_slice(&U256::from(2000u64 * 100_000_000).to_be_bytes::<32>());
                stale.extend_from_slice(&U256::from(1).to_be_bytes::<32>());
                stale.extend_from_slice(&U256::from(1).to_be_bytes::<32>());
                send_local(&client, &rpc, oracle, &stale).await;
                expect_local_revert(&client, &rpc, matching, calldata).await;
                println!("STALE_ORACLE_CONTRACT_REJECTION=PASS");
                set_oracle.extend_from_slice(&U256::from(1).to_be_bytes::<32>());
                send_local(&client, &rpc, oracle, &set_oracle).await;

                let collateral_snapshot = rpc_request(&client, &rpc, "evm_snapshot", serde_json::json!([])).await;
                let seller = adapter.payload.seller.0.clone();
                rpc_request(&client, &rpc, "anvil_impersonateAccount", serde_json::json!([seller])).await;
                let mut withdraw = selector("withdraw(address,uint256)");
                let token = alloy_primitives::Address::from_str(&adapter.payload.settlement_asset.0).unwrap();
                withdraw.extend_from_slice(&[0u8;12]);
                withdraw.extend_from_slice(token.as_slice());
                withdraw.extend_from_slice(&U256::from(10_000u64 * 1_000_000).to_be_bytes::<32>());
                send_local_as(&client, &rpc, vault, &withdraw, &seller).await;
                expect_local_revert(&client, &rpc, matching, calldata).await;
                println!("INSUFFICIENT_COLLATERAL_CONTRACT_REJECTION=PASS");
                assert_eq!(rpc_request(&client, &rpc, "evm_revert", serde_json::json!([collateral_snapshot])).await, true);
            }
            rpc_request(&client, &rpc, "evm_increaseTime", serde_json::json!([7200])).await;
            rpc_request(&client, &rpc, "evm_mine", serde_json::json!([])).await;
            if std::env::var("OPTIONS_V5_PRODUCT").as_deref() == Ok("1") {
                let oracle = transactions.iter().find(|tx| tx["contractName"] == "ScenarioOptionOracle"
                    && tx["transactionType"] == "CREATE")
                    .and_then(|tx| tx["contractAddress"].as_str()).expect("local oracle deployment");
                let latest = rpc_request(&client, &rpc, "eth_getBlockByNumber", serde_json::json!(["latest",false])).await;
                let timestamp = u64::from_str_radix(latest["timestamp"].as_str().unwrap().trim_start_matches("0x"),16).unwrap();
                let mut refresh = selector("setPrice(address,address,uint256,uint256,bool)");
                for address in [&adapter.payload.underlying.0, &adapter.payload.settlement_asset.0] {
                    let parsed = alloy_primitives::Address::from_str(address).unwrap();
                    refresh.extend_from_slice(&[0u8; 12]);
                    refresh.extend_from_slice(parsed.as_slice());
                }
                refresh.extend_from_slice(&U256::from(2000u64 * 100_000_000).to_be_bytes::<32>());
                refresh.extend_from_slice(&U256::from(timestamp).to_be_bytes::<32>());
                refresh.extend_from_slice(&U256::from(1).to_be_bytes::<32>());
                send_local(&client, &rpc, oracle, &refresh).await;
            }
            expect_local_revert(&client, &rpc, matching, calldata).await;
            println!("EXPIRED_OPTION_TRADE=REJECTED");
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
    let v5_product = std::env::var("OPTIONS_V5_PRODUCT").as_deref() == Ok("1") && phase == "trade";
    let local_routes = axum::Router::new()
        .route("/local/options-v4/trade", get(local_trade))
        .route("/local/options-v4/execute", post(local_execute))
        .layer(Extension(adapter.clone()));
    let app = if v5_product {
        deopt_v2_backend::api::router(browser_state)
    } else {
        deopt_v2_backend::api::router(browser_state).merge(local_routes)
    }
    .layer(CorsLayer::permissive()); // disposable loopback test server only
    let listener = tokio::net::TcpListener::bind(format!("127.0.0.1:{api_port}"))
        .await
        .expect("local-only Rust API listener");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("local Rust API");
    });
    let watcher = if v5_product {
        let local_state = state.clone();
        let local_adapter = adapter.clone();
        let api = format!("http://127.0.0.1:{api_port}");
        Some(tokio::spawn(async move {
            let repo = local_state.repository.as_ref().expect("disposable PG");
            let client = reqwest::Client::new();
            for _ in 0..120 {
                let ready = repo.list_option_execution_intents().await.expect("intent list")
                    .into_iter().find(|intent| intent.calldata.is_some());
                if let Some(intent) = ready {
                    let intent_id = intent.intent_id.to_string();
                    let simulation = client.post(format!("{api}/options/execution-intents/{intent_id}/simulate"))
                        .send().await.expect("real simulation handler");
                    assert!(simulation.status().is_success(), "real simulation failed: {}", simulation.text().await.unwrap());
                    let broadcast = client.post(format!("{api}/options/execution-intents/{intent_id}/broadcast"))
                        .send().await.expect("real broadcast handler");
                    assert!(broadcast.status().is_success(), "real broadcast failed: {}", broadcast.text().await.unwrap());
                    let body: Value = broadcast.json().await.expect("broadcast JSON");
                    let hash = body["tx_hash"].as_str().expect("tx hash").to_owned();
                    for _ in 0..20 {
                        let receipt = rpc_request(&client, &local_adapter.rpc, "eth_getTransactionReceipt", serde_json::json!([hash])).await;
                        if receipt.is_object() && receipt["status"] == "0x1" { break; }
                        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                    }
                    let provider = HttpJsonRpcProvider::new(local_adapter.rpc.clone());
                    let tick = index_option_events_with_provider(&local_state, &provider).await.expect("index broadcast receipt");
                    assert!(tick.events_indexed > 0, "broadcast receipt must index");
                    *local_adapter.tx_hash.lock().unwrap() = Some(hash);
                    *local_adapter.indexed_count.lock().unwrap() = tick.events_indexed;
                    *local_adapter.calldata.lock().unwrap() = Some(hex::decode(intent.calldata.expect("calldata").trim_start_matches("0x")).unwrap());
                    return;
                }
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            }
            panic!("product form did not produce a fully signed intent");
        }))
    } else { None };
    let frontend = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("deopt-v2-frontend");
    let buyer = "0x0376AAc07Ad725E01357B1725B5ceC61aE10473c".to_owned();
    let seller = "0xb040e0fAac56886b0f29Af446544AED0A154ED29".to_owned();
    let option_id = option_id.to_owned();
    let phase = phase.to_owned();
    let spec = if v5_product { "tests/e2e/options-v5-product.spec.ts" } else { "tests/e2e/options-v4-live-position.spec.ts" };
    let output = tokio::task::spawn_blocking(move || {
        std::process::Command::new("npx")
            .current_dir(frontend)
            .args([
                "playwright",
                "test",
                spec,
                "--project=chromium",
            ])
            .env("E2E_BASE_URL", format!("http://127.0.0.1:{frontend_port}"))
            .env("OPTIONS_V4_API_URL", format!("http://127.0.0.1:{api_port}"))
            .env("OPTIONS_V4_BUYER", buyer)
            .env("OPTIONS_V4_SELLER", seller)
            .env("OPTIONS_V4_OPTION_ID", option_id)
            .env("OPTIONS_V4_PHASE", phase)
            .env("OPTIONS_V5_PRODUCT", if v5_product || std::env::var("OPTIONS_V5_PRODUCT").as_deref() == Ok("1") { "1" } else { "0" })
            .output()
    })
    .await
    .expect("Playwright join")
    .expect("Playwright process");
    if !output.status.success() {
        if let Some(watcher) = watcher { watcher.abort(); }
        server.abort();
        panic!("Playwright failed:\n{}\n{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
    }
    if let Some(watcher) = watcher {
        watcher.await.expect("real Rust route broadcaster");
    }
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

async fn vault_balance(provider: &HttpJsonRpcProvider, vault: &str, wallet: &str, asset: &str) -> u128 {
    let mut data = selector("balances(address,address)");
    for address in [wallet, asset] {
        let parsed = alloy_primitives::Address::from_str(address).expect("fixture address");
        data.extend_from_slice(&[0u8; 12]);
        data.extend_from_slice(parsed.as_slice());
    }
    let output = provider.eth_call(EthCallRequest {
        from: AccountId::new(wallet), to: AccountId::new(vault), data,
        value: 0, gas_limit: None,
    }).await.expect("canonical vault balance").output;
    assert_eq!(output.len(), 32);
    u128::try_from(U256::from_be_slice(&output)).expect("fixture balance fits u128")
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

async fn expect_local_revert(client: &reqwest::Client, url: &str, to: &str, data: &[u8]) {
    assert!(
        local_call_reverted(client, url, to, data).await,
        "expected a real local contract revert"
    );
}

async fn local_call_reverted(client: &reqwest::Client, url: &str, to: &str, data: &[u8]) -> bool {
    local_call_reverted_as(client, url, to, data, "0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266").await
}

async fn local_call_reverted_as(client: &reqwest::Client, url: &str, to: &str, data: &[u8], from: &str) -> bool {
    let response: Value = client
        .post(url)
        .json(&serde_json::json!({
            "jsonrpc":"2.0", "id":1, "method":"eth_call",
            "params":[{"from":from,
                       "to":to, "data":format!("0x{}", hex::encode(data))}, "latest"]
        }))
        .send()
        .await
        .expect("local revert request")
        .json()
        .await
        .expect("local revert JSON");
    response.get("error").is_some() && response.get("result").is_none()
}

async fn send_local(client: &reqwest::Client, url: &str, to: &str, data: &[u8]) -> String {
    send_local_as(client, url, to, data, "0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266").await
}

async fn send_local_as(client: &reqwest::Client, url: &str, to: &str, data: &[u8], from: &str) -> String {
    let calldata = format!(
        "0x{}",
        data.iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    );
    let hash = rpc_request(
        client,
        url,
        "eth_sendTransaction",
        serde_json::json!([{
            "from":from, "to":to, "data":calldata
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
