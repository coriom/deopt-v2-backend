//! Real local Anvil receipts into disposable PostgreSQL. No hosted broadcast.
use alloy_primitives::{keccak256, Address, U256};
use deopt_v2_backend::api::AppState;
use deopt_v2_backend::db::PgRepository;
use deopt_v2_backend::engine::EngineState;
use deopt_v2_backend::execution::HttpJsonRpcProvider;
use deopt_v2_backend::options::{
    index_option_events_with_provider, OptionEventIndexerConfig, OptionsConfig,
};
use deopt_v2_backend::types::AccountId;
use serde_json::Value;
use std::str::FromStr;

#[tokio::test]
#[ignore = "requires local Anvil and disposable PostgreSQL 16"]
async fn physical_trade_exercise_and_lapse_are_persisted() {
    let rpc = std::env::var("OPTIONS_V18_RPC_URL").expect("local Anvil URL");
    let pg = std::env::var("OPTIONS_V18_PG_URL").expect("disposable PG URL");
    assert!(rpc.starts_with("http://127.0.0.1:"));
    assert!(pg.contains("127.0.0.1"));
    let artifact = std::env::var("OPTIONS_V18_BROADCAST").expect("Foundry broadcast artifact");
    let broadcast: Value = serde_json::from_slice(&std::fs::read(artifact).unwrap()).unwrap();
    let tx = broadcast["transactions"].as_array().unwrap();
    let deployed = |name: &str| -> String {
        tx.iter()
            .find(|row| row["contractName"] == name && row["transactionType"] == "CREATE")
            .and_then(|row| row["contractAddress"].as_str())
            .unwrap()
            .to_string()
    };
    let engine = deployed("CoveredCallMarginEngine");
    let matching = deployed("OptionMatchingEngine");
    let trade_hash = std::env::var("OPTIONS_V18_TRADE_HASH").unwrap();
    let exercise_hash = std::env::var("OPTIONS_V18_EXERCISE_HASH").unwrap();
    let release_hash = std::env::var("OPTIONS_V18_RELEASE_HASH").unwrap();

    let repo = PgRepository::connect(&pg).await.expect("local PG");
    repo.run_migrations().await.expect("local migrations");
    let mut state = AppState::with_options_config_and_repository(
        EngineState::with_default_markets(),
        OptionsConfig::enabled_in_memory_for_tests(),
        repo.clone(),
    );
    state.chain_id = 84532;
    state.option_event_indexer_config = OptionEventIndexerConfig {
        enabled: true,
        poll_interval_ms: 1000,
        from_block: 0,
        batch_blocks: 1000,
        confirmation_blocks: 0,
        require_rpc: true,
        rpc_url: Some(rpc.clone()),
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
    let provider = HttpJsonRpcProvider::new(rpc.clone());
    let first = index_option_events_with_provider(&state, &provider)
        .await
        .expect("index real receipts");
    assert!(
        first.events_indexed >= 4,
        "trade, covered fill, exercise and lapse must be indexed"
    );

    let trade = repo
        .list_option_execution_events_by_tx_hash(&trade_hash)
        .await
        .unwrap();
    let fills: Vec<_> = trade
        .iter()
        .filter(|event| event.event_name == "CoveredFill")
        .collect();
    assert_eq!(fills.len(), 2, "both physical fills share one receipt");
    assert_ne!(fills[0].log_index, fills[1].log_index);
    let fill = fills[0];
    assert_eq!(fill.chain_id, 84532);
    assert!(fill.block_hash.is_some());
    assert_eq!(fill.decoded.as_ref().unwrap()["quantity"], "1");
    assert_eq!(fill.decoded.as_ref().unwrap()["premium"], "100000000");
    assert_eq!(
        fill.decoded.as_ref().unwrap()["lockedUnderlying"],
        "1000000000000000000"
    );
    assert_eq!(trade
        .iter()
        .filter(|event| event.event_name == "OptionTradeExecuted"
            && event.contract_address.eq_ignore_ascii_case(&matching))
        .count(), 2);

    let exercised = repo
        .list_option_execution_events_by_tx_hash(&exercise_hash)
        .await
        .unwrap();
    let delivery = exercised
        .iter()
        .find(|event| event.event_name == "PhysicalExercise")
        .expect("delivery in PG");
    assert_eq!(
        delivery.decoded.as_ref().unwrap()["strikePaid"],
        "2250000000"
    );
    assert_eq!(
        delivery.decoded.as_ref().unwrap()["underlyingDelivered"],
        "1000000000000000000"
    );
    let lapsed = repo
        .list_option_execution_events_by_tx_hash(&release_hash)
        .await
        .unwrap();
    let refund = lapsed
        .iter()
        .find(|event| event.event_name == "LongExpired")
        .expect("lapse in PG");
    assert_eq!(
        refund.decoded.as_ref().unwrap()["underlyingReturned"],
        "1000000000000000000"
    );
    assert_eq!(fill.option_id, delivery.option_id);
    assert_eq!(fill.option_id, refund.option_id);
    assert!(state
        .option_event_indexer_config
        .position_stack_index(fill)
        .is_some());

    let again = index_option_events_with_provider(&state, &provider)
        .await
        .expect("idempotent rescan");
    assert_eq!(again.events_indexed, 0);
    println!("V18_TWO_PHYSICAL_FILLS_ONE_RECEIPT_POSTGRES=PASS trade={trade_hash}");
    println!("V18_PHYSICAL_RECEIPTS_INDEXED_POSTGRES=PASS trade={trade_hash} exercise={exercise_hash} lapse={release_hash}");

    let snapshot = std::env::var("OPTIONS_V18_REORG_SNAPSHOT").expect("post-trade snapshot");
    let client = reqwest::Client::new();
    for (id, method, params) in [
        (1, "evm_revert", serde_json::json!([snapshot])),
        (2, "anvil_mine", serde_json::json!(["0x40"])),
    ] {
        let reply: Value = client
            .post(&rpc)
            .json(&serde_json::json!({
                "jsonrpc":"2.0", "id":id, "method":method, "params":params
            }))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert!(reply.get("error").is_none(), "{method} failed");
        if method == "evm_revert" {
            assert_eq!(reply["result"], true);
        }
    }
    index_option_events_with_provider(&state, &provider)
        .await
        .expect("reorg replay");
    assert!(repo
        .list_option_execution_events_by_tx_hash(&exercise_hash)
        .await
        .unwrap()
        .is_empty());
    assert!(repo
        .list_option_execution_events_by_tx_hash(&release_hash)
        .await
        .unwrap()
        .is_empty());
    assert!(repo
        .list_option_execution_events_by_tx_hash(&trade_hash)
        .await
        .unwrap()
        .iter()
        .any(|event| event.event_name == "CoveredFill"));
    let buyer = Address::from_str(fill.buyer.as_deref().unwrap()).unwrap();
    let seller = Address::from_str(fill.seller.as_deref().unwrap()).unwrap();
    let mut calldata = keccak256(b"openLots(address,address)")[..4].to_vec();
    calldata.extend_from_slice(&[0u8; 12]);
    calldata.extend_from_slice(buyer.as_slice());
    calldata.extend_from_slice(&[0u8; 12]);
    calldata.extend_from_slice(seller.as_slice());
    let answer: Value = client
        .post(&rpc)
        .json(&serde_json::json!({
            "jsonrpc":"2.0", "id":3, "method":"eth_call", "params":[{
                "to":engine, "data":format!("0x{}", hex::encode(calldata))
            }, "latest"]
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(
        answer.get("error").is_none(),
        "canonical covered lot read failed"
    );
    let onchain = U256::from_str_radix(
        answer["result"].as_str().unwrap().trim_start_matches("0x"),
        16,
    )
    .unwrap();
    let projected = repo
        .list_option_execution_events_by_tx_hash(&trade_hash)
        .await
        .unwrap()
        .into_iter()
        .filter(|event| event.event_name == "CoveredFill")
        .map(|event| event.quantity_contracts.unwrap().parse::<u64>().unwrap())
        .sum::<u64>();
    assert_eq!(onchain, U256::from(projected));
    println!("V18_PHYSICAL_REORG_ORPHANS_REMOVED=PASS");
    println!("V18_POST_REORG_COVERED_LOTS_CANONICAL=PASS quantity={projected}");
}
