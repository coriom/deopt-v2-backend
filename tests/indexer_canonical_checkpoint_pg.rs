//! Disposable PostgreSQL acceptance for the legacy PERP indexer cursor.
//! The Hybrid V2 canonical journal has its own persisted reorg recovery tests.
use deopt_v2_backend::db::PgRepository;
use deopt_v2_backend::error::{BackendError, Result};
use deopt_v2_backend::indexer::config::IndexerConfig;
use deopt_v2_backend::indexer::events::EthLog;
use deopt_v2_backend::indexer::runner::{
    EthGetLogsFilter, Indexer, IndexerRpcProvider, RpcFuture, PERP_MATCHING_ENGINE_CURSOR,
};
use deopt_v2_backend::indexer::trade_executed_topic0;
use deopt_v2_backend::types::AccountId;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

#[derive(Clone, Default)]
struct FakeRpc(Arc<Mutex<State>>);
#[derive(Default)]
struct State {
    head: u64,
    hashes: HashMap<u64, String>,
    logs: Vec<EthLog>,
    timeout: bool,
}
impl FakeRpc {
    fn set(&self, number: u64, byte: &str) {
        let mut state = self.0.lock().unwrap();
        state.head = number;
        state
            .hashes
            .insert(number, format!("0x{}", byte.repeat(32)));
    }
    fn add(&self, log: EthLog) {
        self.0.lock().unwrap().logs.push(log);
    }
    fn timeout(&self, yes: bool) {
        self.0.lock().unwrap().timeout = yes;
    }
}
impl IndexerRpcProvider for FakeRpc {
    fn chain_id(&self) -> RpcFuture<'_, u64> {
        Box::pin(async { Ok(84532) })
    }
    fn block_number(&self) -> RpcFuture<'_, u64> {
        Box::pin(async move { Ok(self.0.lock().unwrap().head) })
    }
    fn block_hash(&self, n: u64) -> RpcFuture<'_, String> {
        Box::pin(async move {
            self.0
                .lock()
                .unwrap()
                .hashes
                .get(&n)
                .cloned()
                .ok_or_else(|| BackendError::Indexer("mock block absent".into()))
        })
    }
    fn get_logs(&self, filter: EthGetLogsFilter) -> RpcFuture<'_, Vec<EthLog>> {
        Box::pin(async move {
            let state = self.0.lock().unwrap();
            if state.timeout {
                return Err(BackendError::Indexer("mock timeout".into()));
            }
            let from = u64::from_str_radix(&filter.from_block[2..], 16).unwrap();
            let to = u64::from_str_radix(&filter.to_block[2..], 16).unwrap();
            Ok(state
                .logs
                .iter()
                .filter(|log| {
                    let n =
                        u64::from_str_radix(&log.block_number.as_ref().unwrap()[2..], 16).unwrap();
                    n >= from && n <= to
                })
                .cloned()
                .collect())
        })
    }
}
fn word(n: u64) -> String {
    format!("{n:064x}")
}
fn trade(block: u64, hash: &str, tx: &str) -> EthLog {
    EthLog {
        address: "0x1111111111111111111111111111111111111111".into(),
        topics: vec![
            trade_executed_topic0(),
            format!("0x{}", word(7)),
            format!("0x{}{}", "0".repeat(24), "22".repeat(20)),
            format!("0x{}{}", "0".repeat(24), "33".repeat(20)),
        ],
        data: format!(
            "0x{}{}{}{}{}{}",
            word(1),
            word(100),
            word(20_000),
            word(1),
            word(1),
            word(2)
        ),
        block_number: Some(format!("0x{block:x}")),
        block_hash: Some(hash.into()),
        transaction_hash: Some(tx.into()),
        log_index: Some("0x0".into()),
    }
}

#[tokio::test]
async fn durable_cursor_duplicate_restart_timeout_and_reorg_stop() -> Result<()> {
    let Ok(url) = std::env::var("INDEXER_PG_TEST_URL") else {
        eprintln!("SKIP: INDEXER_PG_TEST_URL absent");
        return Ok(());
    };
    assert!(
        url.contains("deopt_indexer_test"),
        "dedicated disposable database required"
    );
    let repo = PgRepository::connect(&url).await?;
    sqlx::query("DROP SCHEMA IF EXISTS public CASCADE")
        .execute(repo.pool())
        .await
        .map_err(|e| BackendError::Persistence(e.to_string()))?;
    sqlx::query("CREATE SCHEMA public")
        .execute(repo.pool())
        .await
        .map_err(|e| BackendError::Persistence(e.to_string()))?;
    repo.run_migrations().await?;
    let rpc = FakeRpc::default();
    rpc.set(0, "00");
    rpc.set(1, "11");
    let hash1 = format!("0x{}", "11".repeat(32));
    let tx1 = format!("0x{}", "aa".repeat(32));
    rpc.add(trade(1, &hash1, &tx1));
    let config = IndexerConfig {
        enabled: true,
        start_block: 0,
        poll_interval_ms: 100,
        max_block_range: 1,
        require_persistence: true,
        rpc_url: None,
        perp_matching_engine_address: AccountId::new("0x1111111111111111111111111111111111111111"),
        perp_matching_engine_v2_address: None,
    };
    let indexer = Indexer::new(config.clone(), repo.clone(), rpc.clone());
    assert_eq!(indexer.tick().await?.events_indexed, 1);
    assert_eq!(indexer.tick().await?.events_indexed, 0); // duplicate tick
    assert_eq!(repo.list_indexed_perp_trades(10).await?.len(), 1);
    assert_eq!(
        repo.get_indexer_cursor_hash(84532, PERP_MATCHING_ENGINE_CURSOR)
            .await?,
        Some(hash1.clone())
    );
    rpc.set(2, "22");
    rpc.timeout(true);
    assert!(indexer.tick().await.is_err());
    assert_eq!(
        repo.get_indexer_cursor(84532, PERP_MATCHING_ENGINE_CURSOR)
            .await?,
        Some(1)
    );
    rpc.timeout(false);
    let restarted = Indexer::new(config, repo.clone(), rpc.clone());
    assert_eq!(restarted.tick().await?.to_block, 2);
    assert_eq!(repo.list_indexed_perp_trades(10).await?.len(), 1);
    // A short reorg is detected before any more writes. The legacy ledger
    // requires operator reconciliation; Hybrid V2 rebuilds canonically.
    rpc.set(2, "33");
    assert!(restarted.tick().await.is_err());
    assert_eq!(
        repo.get_indexer_cursor(84532, PERP_MATCHING_ENGINE_CURSOR)
            .await?,
        Some(2)
    );
    assert_eq!(repo.list_indexed_perp_trades(10).await?.len(), 1);
    Ok(())
}
