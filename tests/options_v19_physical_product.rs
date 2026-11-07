//! Browser product flow against local Anvil, Rust and disposable PostgreSQL.
//! The trade goes through the normal intent/simulation/broadcast handlers.
use alloy_primitives::U256;
use deopt_v2_backend::api::AppState;
use deopt_v2_backend::db::PgRepository;
use deopt_v2_backend::engine::EngineState;
use deopt_v2_backend::execution::config::PrivateKeySecret;
use deopt_v2_backend::execution::{HttpJsonRpcProvider, TransactionReceiptProvider};
use deopt_v2_backend::options::{
    index_option_events_with_provider, OptionEventIndexerConfig, OptionExecutionSignatureMode,
    OptionSeries, OptionSeriesSource, OptionSeriesStatus, OptionsConfig,
};
use deopt_v2_backend::types::AccountId;
use serde_json::Value;
use std::str::FromStr;
use tower_http::cors::CorsLayer;

fn creation(artifact: &Value, contract: &str) -> String {
    artifact["transactions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|tx| tx["contractName"] == contract && tx["transactionType"] == "CREATE")
        .and_then(|tx| tx["contractAddress"].as_str())
        .unwrap()
        .to_owned()
}

fn series_from_artifact(artifact: &Value, registry: &str, chain_id: u64) -> OptionSeries {
    let topic = format!(
        "0x{}",
        hex::encode(alloy_primitives::keccak256(
            b"SeriesCreated(uint256,address,address,uint64,uint64,uint128,bool,bool)"
        ))
    );
    let log = artifact["receipts"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|receipt| receipt["logs"].as_array().into_iter().flatten())
        .find(|log| {
            log["address"]
                .as_str()
                .is_some_and(|address| address.eq_ignore_ascii_case(registry))
                && log["topics"][0]
                    .as_str()
                    .is_some_and(|value| value.eq_ignore_ascii_case(&topic))
        })
        .expect("real SeriesCreated receipt");
    let topics = log["topics"].as_array().unwrap();
    let option_id =
        U256::from_str_radix(topics[1].as_str().unwrap().trim_start_matches("0x"), 16).unwrap();
    let underlying = format!("0x{}", &topics[2].as_str().unwrap()[26..]);
    let quote = format!("0x{}", &topics[3].as_str().unwrap()[26..]);
    let data = hex::decode(log["data"].as_str().unwrap().trim_start_matches("0x")).unwrap();
    let word = |index: usize| U256::from_be_slice(&data[index * 32..(index + 1) * 32]);
    let expiry = u64::try_from(word(0)).unwrap();
    let strike = u128::try_from(word(1)).unwrap();
    let size = u128::try_from(word(2)).unwrap();
    OptionSeries {
        option_series_id: deopt_v2_backend::options::onchain_option_series_key(
            chain_id,
            registry,
            &option_id.to_string(),
        )
        .unwrap(),
        underlying: underlying.clone(),
        base_asset: underlying,
        quote_asset: quote.clone(),
        settlement_asset: quote,
        expiry,
        strike_1e8: strike,
        is_call: true,
        contract_size_1e8: size,
        status: OptionSeriesStatus::Active,
        source: OptionSeriesSource::Onchain,
        onchain_product_id: None,
        onchain_series_id: Some(option_id.to_string()),
        onchain_chain_id: Some(chain_id),
        onchain_registry_address: Some(registry.to_ascii_lowercase()),
        created_at_ms: 1,
        updated_at_ms: 1,
    }
}

async fn rpc(url: &str, method: &str, params: Value) -> Value {
    let response: Value = reqwest::Client::new()
        .post(url)
        .json(&serde_json::json!({"jsonrpc":"2.0","id":1,"method":method,"params":params}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(response.get("error").is_none(), "{method} RPC failed");
    response["result"].clone()
}

async fn token_balance(url: &str, token: &str, account: &str) -> U256 {
    let mut data = alloy_primitives::keccak256(b"balanceOf(address)")[..4].to_vec();
    data.extend_from_slice(&[0u8; 12]);
    data.extend_from_slice(
        alloy_primitives::Address::from_str(account)
            .unwrap()
            .as_slice(),
    );
    let result = rpc(
        url,
        "eth_call",
        serde_json::json!([{
        "to": token, "data": format!("0x{}", hex::encode(data))
    }, "latest"]),
    )
    .await;
    U256::from_str_radix(result.as_str().unwrap().trim_start_matches("0x"), 16).unwrap()
}

#[tokio::test]
#[ignore = "requires local Anvil, disposable PostgreSQL and Chromium"]
async fn physical_product_browser_exercises_and_releases() {
    let rpc_url = std::env::var("OPTIONS_V18_RPC_URL").unwrap();
    let pg_url = std::env::var("OPTIONS_V18_PG_URL").unwrap();
    assert!(rpc_url.starts_with("http://127.0.0.1:"));
    assert!(pg_url.contains("127.0.0.1"));
    let physical: Value = serde_json::from_slice(
        &std::fs::read(std::env::var("OPTIONS_V18_BROADCAST").unwrap()).unwrap(),
    )
    .unwrap();
    let legacy: Value = serde_json::from_slice(
        &std::fs::read(std::env::var("OPTIONS_V19_LEGACY_BROADCAST").unwrap()).unwrap(),
    )
    .unwrap();
    let engine = creation(&physical, "CoveredCallMarginEngine");
    let matching = creation(&physical, "OptionMatchingEngine");
    let registry = creation(&physical, "OptionProductRegistry");
    let legacy_registry = creation(&legacy, "OptionProductRegistry");
    let legacy_vault = creation(&legacy, "CollateralVault");
    assert_ne!(
        registry.to_ascii_lowercase(),
        legacy_registry.to_ascii_lowercase()
    );
    assert_ne!(
        engine.to_ascii_lowercase(),
        legacy_vault.to_ascii_lowercase()
    );
    let physical_series = series_from_artifact(&physical, &registry, 84532);
    let legacy_series = series_from_artifact(&legacy, &legacy_registry, 84532);
    let legacy_vault_before =
        token_balance(&rpc_url, &legacy_series.settlement_asset, &legacy_vault).await;
    assert!(
        legacy_vault_before > U256::ZERO,
        "legacy Vault needs representative funds"
    );

    let repo = PgRepository::connect(&pg_url).await.unwrap();
    repo.run_migrations().await.unwrap();
    repo.insert_option_series(&physical_series).await.unwrap();
    repo.insert_option_series(&legacy_series).await.unwrap();
    let mut options = OptionsConfig::enabled_in_memory_for_tests();
    options.execution_enabled = true;
    options.matching_engine_address = AccountId::new(matching.clone());
    options.execution_registry_address = AccountId::new(registry.clone());
    options.execution_eip712_domain.verifying_contract = AccountId::new(matching.clone());
    options.execution_signature_mode = OptionExecutionSignatureMode::Strict;
    options.execution_simulation_enabled = true;
    options.execution_simulation_rpc_url = Some(rpc_url.clone());
    options.execution_simulation_gas_limit = 1_500_000;
    options.execution_broadcast_enabled = true;
    options.execution_broadcast_gas_limit = 1_500_000;
    let mut state = AppState::with_options_config_and_repository(
        EngineState::with_default_markets(),
        options,
        repo.clone(),
    );
    state.chain_id = 84532;
    state.execution_config.rpc_url = Some(rpc_url.clone());
    state.execution_config.execution_enabled = true;
    state.execution_config.real_broadcast_enabled = true;
    state.execution_config.dry_run = false;
    state.execution_config.executor_chain_id = 84532;
    state.execution_config.executor_allow_local_signer = true;
    state.execution_config.max_fee_per_gas_wei = Some("10000000000".into());
    state.execution_config.max_priority_fee_per_gas_wei = Some("1000000000".into());
    state.execution_config.executor_private_key = Some(PrivateKeySecret::new(
        "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80".into(),
    ));
    state.execution_config.executor_from_address =
        AccountId::new("0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266");
    state.trading_views.physical_engine_address = Some(AccountId::new(engine.clone()));
    state.option_event_indexer_config = OptionEventIndexerConfig {
        enabled: true,
        poll_interval_ms: 1000,
        from_block: 0,
        batch_blocks: 1000,
        confirmation_blocks: 0,
        require_rpc: true,
        rpc_url: Some(rpc_url.clone()),
        matching_engine_address: AccountId::new(matching.clone()),
        margin_engine_address: AccountId::new(engine.clone()),
        secondary_matching_engine_address: None,
        secondary_margin_engine_address: None,
        collateral_vault_address: AccountId::new("0x0000000000000000000000000000000000000001"),
        secondary_collateral_vault_address: None,
        fees_manager_address: None,
        fees_manager_v2_address: None,
        secondary_fees_manager_v2_address: None,
        old_margin_engine_address: None,
        protocol_fee_vault_address: None,
    };
    state
        .option_event_indexer_config
        .validate_startup(true)
        .unwrap();
    let api_port = std::env::var("OPTIONS_V19_API_PORT").unwrap();
    let frontend_port = std::env::var("OPTIONS_V19_FRONTEND_PORT").unwrap();
    state.wallet_read_auth.enabled = true;
    state.wallet_read_auth.origin = format!("http://127.0.0.1:{frontend_port}");
    let listener = tokio::net::TcpListener::bind(format!("127.0.0.1:{api_port}"))
        .await
        .unwrap();
    let server_state = state.clone();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            deopt_v2_backend::api::router(server_state).layer(CorsLayer::permissive()),
        )
        .await
        .unwrap();
    });
    let index_state = state.clone();
    let index_rpc = rpc_url.clone();
    let indexer = tokio::spawn(async move {
        let provider = HttpJsonRpcProvider::new(index_rpc);
        loop {
            let _ = index_option_events_with_provider(&index_state, &provider).await;
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
    });
    let watcher_repo = repo.clone();
    let watcher_rpc = rpc_url.clone();
    let watcher_api = format!("http://127.0.0.1:{api_port}");
    let watcher = tokio::spawn(async move {
        let client = reqwest::Client::new();
        for _ in 0..240 {
            if let Some(intent) = watcher_repo
                .list_option_execution_intents()
                .await
                .unwrap()
                .into_iter()
                .find(|intent| intent.calldata.is_some())
            {
                let id = intent.intent_id;
                let simulation = client
                    .post(format!(
                        "{watcher_api}/options/execution-intents/{id}/simulate"
                    ))
                    .send()
                    .await
                    .unwrap();
                assert!(
                    simulation.status().is_success(),
                    "physical simulation: {}",
                    simulation.text().await.unwrap()
                );
                let broadcast = client
                    .post(format!(
                        "{watcher_api}/options/execution-intents/{id}/broadcast"
                    ))
                    .send()
                    .await
                    .unwrap();
                assert!(
                    broadcast.status().is_success(),
                    "physical broadcast: {}",
                    broadcast.text().await.unwrap()
                );
                let body: Value = broadcast.json().await.unwrap();
                let hash = body["tx_hash"].as_str().unwrap().to_owned();
                let provider = HttpJsonRpcProvider::new(watcher_rpc);
                for _ in 0..50 {
                    if provider
                        .transaction_receipt(hash.clone())
                        .await
                        .unwrap()
                        .is_some_and(|receipt| receipt.status == Some(1))
                    {
                        println!("V19_PRODUCT_TRADE_TX={hash}");
                        return hash;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                }
                panic!("physical product trade receipt missing");
            }
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
        panic!("physical product form did not create signed intent");
    });
    let front = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("deopt-v2-frontend");
    let output = tokio::task::spawn_blocking(move || {
        std::process::Command::new("npx")
            .current_dir(front)
            .args([
                "playwright",
                "test",
                "tests/e2e/options-v19-physical-product.spec.ts",
                "--project=chromium",
            ])
            .env("E2E_BASE_URL", format!("http://127.0.0.1:{frontend_port}"))
            .env("OPTIONS_V19_RPC_URL", rpc_url)
            .env("OPTIONS_V19_PHYSICAL_REGISTRY", registry)
            .env("OPTIONS_V19_LEGACY_REGISTRY", legacy_registry)
            .env("OPTIONS_V19_SERIES_ID", physical_series.option_series_id)
            .output()
    })
    .await
    .unwrap()
    .unwrap();
    if !output.status.success() {
        watcher.abort();
        indexer.abort();
        server.abort();
        panic!(
            "physical Playwright failed:\n{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let trade_hash = watcher.await.unwrap();
    indexer.abort();
    let _ = indexer.await;
    let events_before_restart = repo
        .list_option_execution_events_by_tx_hash(&trade_hash)
        .await
        .unwrap();
    let resumed_state = state.clone();
    let resumed_rpc = std::env::var("OPTIONS_V18_RPC_URL").unwrap();
    tokio::spawn(async move {
        let provider = HttpJsonRpcProvider::new(resumed_rpc);
        index_option_events_with_provider(&resumed_state, &provider)
            .await
            .expect("persistent cursor must resume after indexer task restart");
    })
    .await
    .unwrap();
    let events_after_restart = repo
        .list_option_execution_events_by_tx_hash(&trade_hash)
        .await
        .unwrap();
    assert_eq!(events_before_restart.len(), events_after_restart.len());
    println!("V19_INDEXER_RESTART_CURSOR=PASS");
    server.abort();
    let events = repo
        .list_option_execution_events_by_tx_hash(&trade_hash)
        .await
        .unwrap();
    assert!(events.iter().any(|event| event.event_name == "CoveredFill"));
    let legacy_vault_after = token_balance(
        &std::env::var("OPTIONS_V18_RPC_URL").unwrap(),
        &legacy_series.settlement_asset,
        &legacy_vault,
    )
    .await;
    assert_eq!(legacy_vault_before, legacy_vault_after);
    println!("V19_LEGACY_VAULT_UNCHANGED=PASS");
    println!("V19_PLAYWRIGHT_PRODUCT=PASS");
    println!("{}", String::from_utf8_lossy(&output.stdout));
}
