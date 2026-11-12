//! Disposable Anvil + PostgreSQL + real Next.js test of an inactive physical series.
use deopt_v2_backend::api::AppState;
use deopt_v2_backend::db::PgRepository;
use deopt_v2_backend::engine::EngineState;
use deopt_v2_backend::execution::HttpJsonRpcProvider;
use deopt_v2_backend::options::physical_series_indexer::{
    index_physical_series_with_provider, list_physical_series,
};
use deopt_v2_backend::options::{
    index_option_events_with_provider, onchain_option_series_key, OptionEventIndexerConfig,
    OptionsConfig,
};
use deopt_v2_backend::types::AccountId;
use serde_json::{json, Value};
use sqlx::Row;
use std::process::Command;
use tower_http::cors::CorsLayer;

async fn rpc(url: &str, method: &str, params: Value) -> Value {
    let result: Value = reqwest::Client::new()
        .post(url)
        .json(&json!({"jsonrpc":"2.0","id":1,"method":method,"params":params}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(result.get("error").is_none(), "{method}: {result}");
    result["result"].clone()
}

fn env(name: &str) -> String {
    std::env::var(name).unwrap()
}

fn created(artifact: &Value, name: &str) -> String {
    artifact["transactions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|tx| tx["contractName"] == name && tx["transactionType"] == "CREATE")
        .and_then(|tx| tx["contractAddress"].as_str())
        .unwrap()
        .to_owned()
}

fn token_balance(rpc_url: &str, token: &str, wallet: &str) -> String {
    let result = Command::new("cast")
        .args([
            "call",
            token,
            "balanceOf(address)(uint256)",
            wallet,
            "--rpc-url",
            rpc_url,
        ])
        .output()
        .unwrap();
    assert!(result.status.success(), "legacy balance read failed");
    String::from_utf8(result.stdout).unwrap().trim().to_owned()
}

fn create_atomic(rpc_url: &str) -> Value {
    let output = Command::new("cast")
        .args([
            "send",
            &env("OPTIONS_V28_OWNER"),
            "createAndDeactivate(address,address,address,uint64,uint64)",
            &env("OPTIONS_V28_PHYSICAL_REGISTRY"),
            &env("OPTIONS_V28_WETH"),
            &env("OPTIONS_V28_USDC"),
            &env("OPTIONS_V28_EXPIRY"),
            "210000000000",
            "--from",
            "0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266",
            "--unlocked",
            "--rpc-url",
            rpc_url,
            "--json",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "cast send: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let receipt: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(receipt["status"], "0x1", "{receipt}");
    receipt
}

#[tokio::test]
#[ignore = "requires disposable local Anvil, PostgreSQL 16 and Playwright"]
async fn inactive_series_is_canonical_in_postgres_and_browser() {
    let rpc_url = env("OPTIONS_V18_RPC_URL");
    let pg_url = env("OPTIONS_V18_PG_URL");
    assert!(rpc_url.starts_with("http://127.0.0.1:"));
    assert!(pg_url.contains("127.0.0.1"));
    assert_eq!(rpc(&rpc_url, "eth_chainId", json!([])).await, "0x14a34");
    let _ = rpc(&rpc_url, "anvil_nodeInfo", json!([])).await;
    let physical = env("OPTIONS_V28_PHYSICAL_REGISTRY");
    let legacy = env("OPTIONS_V28_LEGACY_REGISTRY");
    let legacy_id = env("OPTIONS_V28_OPTION_ID");
    let legacy_artifact: Value =
        serde_json::from_slice(&std::fs::read(env("OPTIONS_V19_LEGACY_BROADCAST")).unwrap())
            .unwrap();
    let historical_vault = created(&legacy_artifact, "CollateralVault");
    let historical_usdc = created(&legacy_artifact, "ScenarioOptionERC20");
    let historical_balance = token_balance(&rpc_url, &historical_usdc, &historical_vault);
    assert_ne!(
        historical_balance, "0",
        "representative legacy collateral required"
    );
    let repo = PgRepository::connect(&pg_url).await.unwrap();
    repo.run_migrations().await.unwrap();
    let mut options = OptionsConfig::enabled_in_memory_for_tests();
    options.execution_enabled = false;
    let mut state = AppState::with_options_config_and_repository(
        EngineState::with_default_markets(),
        options,
        repo.clone(),
    );
    state.chain_id = 84532;
    state.execution_config.rpc_url = Some(rpc_url.clone());
    state.trading_views.physical_registry_address = Some(AccountId::new(physical.clone()));
    state.trading_views.physical_engine_address = Some(AccountId::new(env("OPTIONS_V28_ENGINE")));
    state.trading_views.physical_matching_engine_address =
        Some(AccountId::new(env("OPTIONS_V28_MATCHER")));
    state.option_event_indexer_config = OptionEventIndexerConfig {
        enabled: true,
        poll_interval_ms: 1000,
        from_block: 0,
        batch_blocks: 1000,
        confirmation_blocks: 2,
        require_rpc: true,
        rpc_url: Some(rpc_url.clone()),
        matching_engine_address: AccountId::new(env("OPTIONS_V28_MATCHER")),
        margin_engine_address: AccountId::new(env("OPTIONS_V28_ENGINE")),
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
    let provider = HttpJsonRpcProvider::new(rpc_url.clone());
    index_physical_series_with_provider(&state, &provider)
        .await
        .unwrap();
    assert!(list_physical_series(repo.pool(), 84532, &physical)
        .await
        .unwrap()
        .is_empty());

    let snapshot = rpc(&rpc_url, "evm_snapshot", json!([])).await;
    let first = create_atomic(&rpc_url);
    assert_eq!(
        first["logs"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|log| {
                log["address"]
                    .as_str()
                    .is_some_and(|a| a.eq_ignore_ascii_case(&physical))
            })
            .count(),
        4
    ); // creation, deactivation, Paused and GlobalPauseSet in one real receipt
    index_physical_series_with_provider(&state, &provider)
        .await
        .unwrap();
    assert!(
        list_physical_series(repo.pool(), 84532, &physical)
            .await
            .unwrap()
            .is_empty(),
        "unconfirmed events must not appear"
    );
    rpc(&rpc_url, "evm_mine", json!([])).await;
    rpc(&rpc_url, "evm_mine", json!([])).await;
    assert_eq!(
        index_physical_series_with_provider(&state, &provider)
            .await
            .unwrap(),
        2
    );
    let series = list_physical_series(repo.pool(), 84532, &physical)
        .await
        .unwrap();
    assert_eq!(series.len(), 1);
    assert_eq!(series[0].option_id, legacy_id);
    assert!(!series[0].is_active);
    assert_eq!(series[0].creation_tx_hash, series[0].last_tx_hash);
    assert_eq!(
        series[0].series_id,
        onchain_option_series_key(84532, &physical, &legacy_id).unwrap()
    );
    assert_ne!(
        series[0].series_id,
        onchain_option_series_key(84532, &legacy, &legacy_id).unwrap()
    );
    assert_eq!(
        series[0].last_block_hash.to_ascii_lowercase(),
        first["blockHash"].as_str().unwrap().to_ascii_lowercase()
    );
    let event_count: i64 = sqlx::query(
        "SELECT count(*) AS count FROM physical_option_series_events WHERE registry_address=$1",
    )
    .bind(physical.to_ascii_lowercase())
    .fetch_one(repo.pool())
    .await
    .unwrap()
    .get("count");
    assert_eq!(event_count, 2);
    let other_count: i64 = sqlx::query(
        "SELECT count(*) AS count FROM physical_option_series_projection WHERE registry_address=$1",
    )
    .bind(legacy.to_ascii_lowercase())
    .fetch_one(repo.pool())
    .await
    .unwrap()
    .get("count");
    assert_eq!(other_count, 0);
    let economic_events: i64 = sqlx::query("SELECT count(*) AS count FROM option_execution_events")
        .fetch_one(repo.pool())
        .await
        .unwrap()
        .get("count");
    assert_eq!(
        economic_events, 0,
        "series indexing cannot invent fills or fees"
    );
    assert_eq!(
        index_physical_series_with_provider(&state, &provider)
            .await
            .unwrap(),
        0
    );

    let api_port = env("OPTIONS_V19_API_PORT");
    let front_port = env("OPTIONS_V19_FRONTEND_PORT");
    let listener = tokio::net::TcpListener::bind(format!("127.0.0.1:{api_port}"))
        .await
        .unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            deopt_v2_backend::api::router(state.clone()).layer(CorsLayer::permissive()),
        )
        .await
        .unwrap();
    });
    let client = reqwest::Client::new();
    let endpoint = format!("http://127.0.0.1:{api_port}/options/physical-stack");
    let response: Value = client
        .get(&endpoint)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(response["status"], "ok", "{response}");
    assert_eq!(response["data"]["series_count_indexed"], 1);
    assert_eq!(response["data"]["series"][0]["is_active"], false);
    assert_eq!(
        response["data"]["series"][0]["series_id"],
        series[0].series_id
    );
    println!("V28_CREATE_DISABLE_TX={}", first["transactionHash"]);
    println!("V28_CREATE_DISABLE_BLOCK_HASH={}", first["blockHash"]);

    assert_eq!(rpc(&rpc_url, "evm_revert", json!([snapshot])).await, true);
    rpc(&rpc_url, "evm_mine", json!([])).await;
    rpc(&rpc_url, "evm_mine", json!([])).await;
    rpc(&rpc_url, "evm_mine", json!([])).await;
    // The production indexer instance carries the physical address. Reuse its
    // configuration below via a fresh state to model worker restart.
    let mut resumed = AppState::with_options_config_and_repository(
        EngineState::with_default_markets(),
        OptionsConfig::enabled_in_memory_for_tests(),
        repo.clone(),
    );
    resumed.chain_id = 84532;
    resumed.trading_views.physical_registry_address = Some(AccountId::new(physical.clone()));
    resumed.option_event_indexer_config = OptionEventIndexerConfig {
        enabled: true,
        poll_interval_ms: 1000,
        from_block: 0,
        batch_blocks: 1000,
        confirmation_blocks: 2,
        require_rpc: true,
        rpc_url: Some(rpc_url.clone()),
        matching_engine_address: AccountId::new(env("OPTIONS_V28_MATCHER")),
        margin_engine_address: AccountId::new(env("OPTIONS_V28_ENGINE")),
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
    index_physical_series_with_provider(&resumed, &provider)
        .await
        .unwrap();
    assert!(list_physical_series(repo.pool(), 84532, &physical)
        .await
        .unwrap()
        .is_empty());
    let after_reorg: Value = client
        .get(&endpoint)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(after_reorg["data"]["series_count_onchain"], "0");
    assert_eq!(after_reorg["data"]["series_count_indexed"], 0);
    let second = create_atomic(&rpc_url);
    rpc(&rpc_url, "evm_mine", json!([])).await;
    rpc(&rpc_url, "evm_mine", json!([])).await;
    assert_eq!(
        index_physical_series_with_provider(&resumed, &provider)
            .await
            .unwrap(),
        2
    );
    let final_series = list_physical_series(repo.pool(), 84532, &physical)
        .await
        .unwrap();
    assert_eq!(final_series.len(), 1);
    assert_eq!(
        final_series[0].last_block_hash.to_ascii_lowercase(),
        second["blockHash"].as_str().unwrap().to_ascii_lowercase()
    );
    assert_ne!(first["blockHash"], second["blockHash"]);
    assert_eq!(
        token_balance(&rpc_url, &historical_usdc, &historical_vault),
        historical_balance,
        "new stack must not move historical collateral"
    );
    let worker_tick = index_option_events_with_provider(&resumed, &provider)
        .await
        .unwrap();
    assert!(
        worker_tick.enabled,
        "production Options worker must consume this Registry"
    );
    assert_eq!(
        list_physical_series(repo.pool(), 84532, &physical)
            .await
            .unwrap()
            .len(),
        1
    );

    let front = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("deopt-v2-frontend");
    let output = tokio::task::spawn_blocking(move || {
        Command::new("npx")
            .current_dir(front)
            .args([
                "playwright",
                "test",
                "tests/e2e/options-v28-inactive-series.spec.ts",
                "--project=chromium",
            ])
            .env("E2E_BASE_URL", format!("http://127.0.0.1:{front_port}"))
            .env(
                "OPTIONS_V28_EXPECTED_SERIES",
                final_series[0].series_id.clone(),
            )
            .output()
            .unwrap()
    })
    .await
    .unwrap();
    assert!(
        output.status.success(),
        "Playwright:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    println!("V28_PLAYWRIGHT=PASS");
    println!("V28_REORG_RECOVERY=PASS");
    server.abort();
}
