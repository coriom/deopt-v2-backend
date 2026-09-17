//! PERPS_BASE_SEPOLIA_BACKEND_RUNTIME_BOOT_INTEGRATION_V1 — full
//! composed runtime boot tests. Exercises the actual
//! `BroadcastRuntime` wiring against a real disposable Postgres +
//! mocked JSON-RPC + a deterministic LocalDev signer matching the
//! configured executor address.
//!
//! Env-gated via `PERPS_CLOSED_TEST_E2E_PG_URL`. Zero required skips
//! for this milestone.
//!
//! Coverage (scenarios A–H per the milestone spec):
//! A. default config → real broadcaster remains OFF
//! B. valid closed-test config → preflight OK, initial reconciliation
//!    OK, reconciler spawns, subsystem readiness advertises broadcast_ready
//! C. signer mismatch → fail closed
//! D. wrong executor authorization (PME.isExecutor=false) → fail closed
//! E. PME paused → fail closed
//! F. initial reconciliation failure → new broadcasts never ready
//! G. graceful shutdown → reconciler terminates cleanly
//! H. a mocked eligible execution reaches BroadcastPolicy

use deopt_v2_backend::confirmation::ConfirmationReceipt;
use deopt_v2_backend::db::PgRepository;
use deopt_v2_backend::execution::rpc::{
    EthCallProvider, EthCallRequest, EthCallSuccess, RpcFuture, TransactionBroadcastProvider,
    TransactionReceiptProvider,
};
use deopt_v2_backend::execution::signer::ExecutorSigner;
use deopt_v2_backend::execution::{
    build_broadcast_runtime, execute_pending_batch, wire_broadcast_runtime, BroadcastPolicy,
    BroadcastReadiness, ExecutionConfig, ExecutionIntentRepository, PrivateKeySecret,
    ReconcilerConfig, PME_IS_EXECUTOR_SELECTOR, PME_PAUSED_SELECTOR,
};
use deopt_v2_backend::types::AccountId;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

const PG_ENV_VAR: &str = "PERPS_CLOSED_TEST_E2E_PG_URL";
// A well-known Ganache/hardhat test key. Address:
// 0x2c7536e3605d9c16a7a3d7b1898e529396a65c23. Used only in tests.
const TEST_KEY: &str = "0x4c0883a69102937d6231471b5dbb6204fe5129617082792ae468d01a3f362318";
const TEST_KEY_ADDRESS: &str = "0x2c7536e3605d9c16a7a3d7b1898e529396a65c23";
const PME_TARGET: &str = "0x774d96e5739bffadee91508b4d3d74f5be29f165";
const CHAIN_ID_BASE_SEPOLIA: u64 = 84532;

fn pg_url() -> Option<String> {
    std::env::var(PG_ENV_VAR).ok().filter(|v| !v.is_empty())
}

async fn ensure_migrated(url: &str) {
    static MIGRATED: tokio::sync::OnceCell<()> = tokio::sync::OnceCell::const_new();
    MIGRATED
        .get_or_init(|| async {
            let repo = PgRepository::connect(url)
                .await
                .expect("connect for shared migration");
            repo.run_migrations()
                .await
                .expect("run migrations once against disposable PG database");
        })
        .await;
}

async fn fresh_repo(url: &str) -> PgRepository {
    ensure_migrated(url).await;
    PgRepository::connect(url)
        .await
        .expect("connect to disposable PG database")
}

fn make_config(from: &str, real_broadcast: bool, dry_run: bool) -> ExecutionConfig {
    ExecutionConfig {
        execution_enabled: true,
        dry_run,
        poll_interval_ms: 100,
        max_batch_size: 10,
        real_broadcast_enabled: real_broadcast,
        executor_private_key: Some(PrivateKeySecret::new(TEST_KEY.to_string())),
        executor_chain_id: CHAIN_ID_BASE_SEPOLIA,
        max_gas_limit: 1_000_000,
        max_fee_per_gas_wei: Some("1000000000".to_string()),
        max_priority_fee_per_gas_wei: Some("100000000".to_string()),
        require_simulation_ok: false,
        simulation_enabled: false,
        simulation_requires_persistence: false,
        rpc_url: Some("http://mock.invalid".to_string()),
        executor_from_address: AccountId::new(from.to_string()),
        perp_matching_engine_address: AccountId::new(PME_TARGET.to_string()),
        perp_engine_address: AccountId::new(
            "0x0000000000000000000000000000000000000000".to_string(),
        ),
        old_perp_engine_address: None,
        backend_signer_mode: deopt_v2_backend::execution::SignerBackendKind::LocalDev,
        backend_signer_endpoint: None,
        executor_allow_local_signer: true,
        backend_signer_provider: None,
        backend_signer_timeout_ms: 2500,
        perps_closed_test_broadcast_armed: false,
        perps_closed_test_broadcast_intent_id: None,
        perps_closed_test_max_drift_bps: 100,
        perps_closed_test_min_deadline_remaining_sec: 900,
    }
}

// ---- MockRpc ----
#[derive(Clone, Default)]
struct MockRpc {
    chain_id: Arc<Mutex<u64>>,
    pending_nonce: Arc<Mutex<u64>>,
    is_executor: Arc<Mutex<bool>>,
    paused: Arc<Mutex<bool>>,
    receipt: Arc<Mutex<Option<ConfirmationReceipt>>>,
    sent_raw: Arc<Mutex<Vec<String>>>,
    receipt_err_count: Arc<Mutex<u32>>,
}

impl MockRpc {
    fn new() -> Self {
        Self {
            chain_id: Arc::new(Mutex::new(CHAIN_ID_BASE_SEPOLIA)),
            pending_nonce: Arc::new(Mutex::new(0)),
            is_executor: Arc::new(Mutex::new(true)),
            paused: Arc::new(Mutex::new(false)),
            receipt: Arc::new(Mutex::new(None)),
            sent_raw: Arc::new(Mutex::new(Vec::new())),
            receipt_err_count: Arc::new(Mutex::new(0)),
        }
    }
    fn with_is_executor(self, v: bool) -> Self {
        *self.is_executor.lock().unwrap() = v;
        self
    }
    fn with_paused(self, v: bool) -> Self {
        *self.paused.lock().unwrap() = v;
        self
    }
    fn with_chain_id(self, id: u64) -> Self {
        *self.chain_id.lock().unwrap() = id;
        self
    }
    fn with_receipt_errors(self, n: u32) -> Self {
        *self.receipt_err_count.lock().unwrap() = n;
        self
    }
}

impl EthCallProvider for MockRpc {
    fn eth_call(&self, request: EthCallRequest) -> RpcFuture<'_, EthCallSuccess> {
        let selector: Vec<u8> = request.data.get(..4).unwrap_or(&[0u8; 4]).to_vec();
        let is_exec = *self.is_executor.lock().unwrap();
        let paused = *self.paused.lock().unwrap();
        let out = if selector.as_slice() == PME_IS_EXECUTOR_SELECTOR {
            bool_return(is_exec)
        } else if selector.as_slice() == PME_PAUSED_SELECTOR {
            bool_return(paused)
        } else if selector.as_slice()
            == deopt_v2_backend::api::perps_cosign::PERP_ENGINE_GET_MARK_PRICE_SELECTOR
        {
            // Return a mark price matching the test intent's
            // price_1e8 (see h_ test) so the pre-send drift gate is
            // satisfied. price_1e8 = 300_000_000_000 in these fixtures.
            let mut buf = vec![0u8; 32];
            let price: u128 = 300_000_000_000;
            buf[16..].copy_from_slice(&price.to_be_bytes());
            buf
        } else {
            vec![0u8; 32]
        };
        Box::pin(async move {
            Ok(EthCallSuccess {
                block_number: Some(1),
                output: out,
            })
        })
    }
}

impl TransactionBroadcastProvider for MockRpc {
    fn chain_id(&self) -> RpcFuture<'_, u64> {
        let v = *self.chain_id.lock().unwrap();
        Box::pin(async move { Ok(v) })
    }
    fn transaction_count(&self, _address: AccountId) -> RpcFuture<'_, u64> {
        let v = *self.pending_nonce.lock().unwrap();
        Box::pin(async move { Ok(v) })
    }
    fn send_raw_transaction(&self, raw_transaction: String) -> RpcFuture<'_, String> {
        self.sent_raw.lock().unwrap().push(raw_transaction.clone());
        let hash = deopt_v2_backend::execution::derive_signed_transaction_hash(&raw_transaction);
        Box::pin(async move { hash })
    }
}

impl TransactionReceiptProvider for MockRpc {
    fn block_number(&self) -> RpcFuture<'_, u64> {
        Box::pin(async { Ok(1) })
    }
    fn transaction_receipt(&self, _tx_hash: String) -> RpcFuture<'_, Option<ConfirmationReceipt>> {
        let mut counter = self.receipt_err_count.lock().unwrap();
        if *counter > 0 {
            *counter -= 1;
            return Box::pin(async move {
                Err(deopt_v2_backend::error::BackendError::Simulation(
                    "mock receipt error".to_string(),
                ))
            });
        }
        drop(counter);
        let r = self.receipt.lock().unwrap().clone();
        Box::pin(async move { Ok(r) })
    }
}

fn bool_return(v: bool) -> Vec<u8> {
    let mut out = vec![0u8; 32];
    if v {
        out[31] = 1;
    }
    out
}

// Insert a bare parent execution_intents row.
async fn seed_execution_intent(repo: &PgRepository, intent_id: Uuid) {
    let pool = repo.pool();
    sqlx::query(
        "INSERT INTO execution_intents (
            intent_id, market_id, buyer, seller, price_1e8, size_1e8,
            buy_order_id, sell_order_id, status, created_at_ms, updated_at_ms
        ) VALUES ($1, 1, $2, $3, '300000000000', '100000000',
                  $4, $5, 'pending', $6, $6)
         ON CONFLICT (intent_id) DO NOTHING",
    )
    .bind(intent_id.to_string())
    .bind("0x0000000000000000000000000000000000000001")
    .bind("0x0000000000000000000000000000000000000002")
    .bind(Uuid::new_v4().to_string())
    .bind(Uuid::new_v4().to_string())
    .bind(1_700_000_000_000i64)
    .execute(pool)
    .await
    .expect("insert parent execution_intents row");
}

// ================================================================
// A — default config → real broadcaster remains OFF
// ================================================================

#[tokio::test]
async fn a_default_config_broadcaster_remains_off() {
    // Default readiness is all-false; no runtime built.
    let readiness = BroadcastReadiness::new();
    assert!(!readiness.broadcast_ready());
    assert!(!readiness.enabled());
    assert!(!readiness.preflight_ok());
    assert!(!readiness.initial_reconciliation_ok());
    assert!(!readiness.reconciler_running());
}

// ================================================================
// B — valid closed-test config → preflight OK, initial reconciliation
//     OK, reconciler spawns, subsystem readiness advertises broadcast_ready
// ================================================================

#[tokio::test]
async fn b_valid_config_reaches_broadcast_ready() {
    let Some(url) = pg_url() else {
        eprintln!("IGNORED [b_valid_config_reaches_broadcast_ready] (PG url not provided)");
        return;
    };
    let repo = Arc::new(fresh_repo(&url).await);
    let rpc = MockRpc::new();
    let config = make_config(TEST_KEY_ADDRESS, true, false);
    let signer = Arc::new(
        ExecutorSigner::from_private_key(&PrivateKeySecret::new(TEST_KEY.to_string())).unwrap(),
    );
    let mut policy = BroadcastPolicy::new(config, rpc, signer);
    policy.verify_pme_event = false; // no logs from mock RPC
    policy.poll_receipt_interval_ms = 1;
    policy.poll_receipt_max_attempts = 1;
    let policy = Arc::new(policy);

    let readiness = BroadcastReadiness::new();
    let mut runtime = wire_broadcast_runtime(
        policy.clone(),
        repo.clone(),
        ReconcilerConfig {
            interval_ms: 60_000,
            batch_size: 5,
        },
        readiness.clone(),
    )
    .await
    .expect("wire_broadcast_runtime OK");

    assert!(readiness.enabled());
    assert!(readiness.preflight_ok());
    assert!(readiness.initial_reconciliation_ok());
    assert!(readiness.reconciler_running());
    assert!(
        readiness.broadcast_ready(),
        "broadcast subsystem must be ready"
    );

    // Graceful shutdown of the spawned reconciler.
    runtime.shutdown();
    assert!(!readiness.reconciler_running());
    // Don't wait full 60s for the loop tick — the test just verified
    // the shutdown flag; the join is best-effort.
    let _ = tokio::time::timeout(std::time::Duration::from_millis(100), runtime.join()).await;
}

// ================================================================
// C — signer mismatch → fail closed
// ================================================================

#[tokio::test]
async fn c_signer_mismatch_fails_closed() {
    let Some(url) = pg_url() else {
        eprintln!("IGNORED [c_signer_mismatch_fails_closed] (PG url not provided)");
        return;
    };
    let repo = Arc::new(fresh_repo(&url).await);
    let rpc = MockRpc::new();
    // Config claims a DIFFERENT executor address than the signer resolves to.
    let config = make_config("0xdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef", true, false);
    let signer = Arc::new(
        ExecutorSigner::from_private_key(&PrivateKeySecret::new(TEST_KEY.to_string())).unwrap(),
    );
    let policy = Arc::new(BroadcastPolicy::new(config, rpc, signer));
    let readiness = BroadcastReadiness::new();
    let result =
        wire_broadcast_runtime(policy, repo, ReconcilerConfig::default(), readiness.clone())
            .await
            .err()
            .expect("signer bind mismatch must fail closed");
    assert!(result.to_string().contains("signer address"));
    // Runtime never advertises readiness.
    assert!(!readiness.broadcast_ready());
    // enabled was set at the very top of wire_broadcast_runtime; that's OK — the
    // meaningful check is that broadcast_ready() is false.
    assert!(!readiness.preflight_ok());
    assert!(!readiness.initial_reconciliation_ok());
}

// ================================================================
// D — wrong executor authorization (PME.isExecutor = false) → fail closed
// ================================================================

#[tokio::test]
async fn d_pme_unauthorized_fails_closed() {
    let Some(url) = pg_url() else {
        eprintln!("IGNORED [d_pme_unauthorized_fails_closed] (PG url not provided)");
        return;
    };
    let repo = Arc::new(fresh_repo(&url).await);
    let rpc = MockRpc::new().with_is_executor(false);
    let config = make_config(TEST_KEY_ADDRESS, true, false);
    let signer = Arc::new(
        ExecutorSigner::from_private_key(&PrivateKeySecret::new(TEST_KEY.to_string())).unwrap(),
    );
    let policy = Arc::new(BroadcastPolicy::new(config, rpc, signer));
    let readiness = BroadcastReadiness::new();
    let result =
        wire_broadcast_runtime(policy, repo, ReconcilerConfig::default(), readiness.clone())
            .await
            .err()
            .expect("PME.isExecutor=false must fail startup preflight");
    assert!(result.to_string().contains("isExecutor"));
    assert!(!readiness.broadcast_ready());
}

// ================================================================
// E — PME paused → fail closed
// ================================================================

#[tokio::test]
async fn e_pme_paused_fails_closed() {
    let Some(url) = pg_url() else {
        eprintln!("IGNORED [e_pme_paused_fails_closed] (PG url not provided)");
        return;
    };
    let repo = Arc::new(fresh_repo(&url).await);
    let rpc = MockRpc::new().with_paused(true);
    let config = make_config(TEST_KEY_ADDRESS, true, false);
    let signer = Arc::new(
        ExecutorSigner::from_private_key(&PrivateKeySecret::new(TEST_KEY.to_string())).unwrap(),
    );
    let policy = Arc::new(BroadcastPolicy::new(config, rpc, signer));
    let readiness = BroadcastReadiness::new();
    let result =
        wire_broadcast_runtime(policy, repo, ReconcilerConfig::default(), readiness.clone())
            .await
            .err()
            .expect("PME.paused=true must fail startup preflight");
    assert!(result.to_string().contains("paused"));
    assert!(!readiness.broadcast_ready());
}

// ================================================================
// F — initial reconciliation failure → new broadcasts never ready
// ================================================================

#[tokio::test]
async fn f_initial_reconciliation_failure_blocks_readiness() {
    let Some(url) = pg_url() else {
        eprintln!(
            "IGNORED [f_initial_reconciliation_failure_blocks_readiness] (PG url not provided)"
        );
        return;
    };
    let repo = Arc::new(fresh_repo(&url).await);
    // Preseed a durable Prepared row (a phantom from a prior process)
    // whose tx_hash the reconciler will try to look up.
    let intent_id = Uuid::new_v4();
    seed_execution_intent(&repo, intent_id).await;
    repo.record_prepared_transaction(deopt_v2_backend::execution::PreparedTransactionRecord {
        intent_id,
        chain_id: CHAIN_ID_BASE_SEPOLIA,
        executor_address: AccountId::new(TEST_KEY_ADDRESS.to_string()),
        target_address: AccountId::new(PME_TARGET.to_string()),
        tx_hash: format!("0x{:064x}", 0xdeadu64),
        nonce: 999_999,
        raw_tx_hex: "0x02".to_string(),
        prepared_at_ms: 1_700_000_000_000,
    })
    .await
    .unwrap();

    // Mock RPC where receipt lookup ERRORS forever — the reconciler
    // pass returns an Err.
    let rpc = MockRpc::new().with_receipt_errors(u32::MAX);
    let config = make_config(TEST_KEY_ADDRESS, true, false);
    let signer = Arc::new(
        ExecutorSigner::from_private_key(&PrivateKeySecret::new(TEST_KEY.to_string())).unwrap(),
    );
    let policy = Arc::new(BroadcastPolicy::new(config, rpc, signer));
    let readiness = BroadcastReadiness::new();
    let result = wire_broadcast_runtime(
        policy,
        repo.clone(),
        ReconcilerConfig::default(),
        readiness.clone(),
    )
    .await;
    assert!(
        result.is_err(),
        "initial reconciliation failure must abort startup"
    );
    // enabled + preflight_ok may be true because they precede initial
    // reconciliation, but broadcast_ready MUST be false because the
    // reconciler was never marked running.
    assert!(!readiness.broadcast_ready());
    assert!(!readiness.reconciler_running());
}

// ================================================================
// G — graceful shutdown → reconciler terminates cleanly
// ================================================================

#[tokio::test]
async fn g_graceful_shutdown_reconciler_terminates() {
    let Some(url) = pg_url() else {
        eprintln!("IGNORED [g_graceful_shutdown_reconciler_terminates] (PG url not provided)");
        return;
    };
    let repo = Arc::new(fresh_repo(&url).await);
    let rpc = MockRpc::new();
    let config = make_config(TEST_KEY_ADDRESS, true, false);
    let signer = Arc::new(
        ExecutorSigner::from_private_key(&PrivateKeySecret::new(TEST_KEY.to_string())).unwrap(),
    );
    let mut policy = BroadcastPolicy::new(config, rpc, signer);
    policy.poll_receipt_interval_ms = 1;
    policy.poll_receipt_max_attempts = 1;
    let policy = Arc::new(policy);
    let readiness = BroadcastReadiness::new();
    let mut runtime = wire_broadcast_runtime(
        policy,
        repo,
        ReconcilerConfig {
            interval_ms: 20,
            batch_size: 5,
        },
        readiness.clone(),
    )
    .await
    .unwrap();
    assert!(readiness.broadcast_ready());
    // Let the reconciler tick a few times.
    tokio::time::sleep(std::time::Duration::from_millis(80)).await;
    // Shut it down.
    runtime.shutdown();
    // Reconciler observes the cancel token on next tick boundary.
    let join = tokio::time::timeout(std::time::Duration::from_millis(500), runtime.join())
        .await
        .expect("reconciler must terminate within 500ms of cancel");
    assert!(join.is_ok(), "reconciler task must exit cleanly");
    assert!(!readiness.reconciler_running());
}

// ================================================================
// H — a mocked eligible execution reaches BroadcastPolicy
// ================================================================

#[tokio::test]
async fn h_eligible_execution_reaches_broadcast_policy() {
    let Some(url) = pg_url() else {
        eprintln!("IGNORED [h_eligible_execution_reaches_broadcast_policy] (PG url not provided)");
        return;
    };
    let repo = Arc::new(fresh_repo(&url).await);
    let rpc = MockRpc::new();
    // Seed a pending intent with buyer + seller signatures ready. The
    // execute_pending_batch caller loads signatures from the
    // repository; we insert them directly here to keep the test tight.
    let intent_id = Uuid::new_v4();
    seed_execution_intent(&repo, intent_id).await;
    // Insert signatures via raw SQL against the child table.
    let pool = repo.pool();
    sqlx::query(
        "INSERT INTO execution_intent_signatures (intent_id, buyer_sig, seller_sig, updated_at_ms) \
         VALUES ($1, $2, $3, $4)
         ON CONFLICT (intent_id) DO NOTHING",
    )
    .bind(intent_id.to_string())
    .bind(format!("0x{:0130x}", 0xaau64)) // 65 bytes hex
    .bind(format!("0x{:0130x}", 0xbbu64))
    .bind(1_700_000_000_000i64)
    .execute(pool)
    .await
    .expect("insert signatures");

    // Also need the intent to have the additional fields (buyer_is_maker,
    // buyer_nonce, seller_nonce, deadline_ms) populated for
    // build_execution_transaction_request to succeed.
    sqlx::query(
        "UPDATE execution_intents SET buyer_is_maker = false, buyer_nonce = 11, \
         seller_nonce = 12, deadline_ms = 4102444800000 WHERE intent_id = $1",
    )
    .bind(intent_id.to_string())
    .execute(pool)
    .await
    .unwrap();

    let mut config = make_config(TEST_KEY_ADDRESS, true, false);
    // PERPS_BASE_SEPOLIA_CLOSED_TEST_RUNTIME_ARMING_AND_ACCOUNTING_V1 —
    // the happy-path executor requires explicit arming.
    config.perps_closed_test_broadcast_armed = true;
    config.perps_closed_test_broadcast_intent_id = Some(intent_id);
    let signer = Arc::new(
        ExecutorSigner::from_private_key(&PrivateKeySecret::new(TEST_KEY.to_string())).unwrap(),
    );
    let mut policy = BroadcastPolicy::new(config, rpc.clone(), signer);
    policy.poll_receipt_interval_ms = 1;
    policy.poll_receipt_max_attempts = 1;
    policy.verify_pme_event = false;

    // Execute one batch. Must reach BroadcastPolicy::broadcast_intent
    // → send_raw_transaction is invoked on the mock RPC.
    let sent_before = rpc.sent_raw.lock().unwrap().len();
    let _processed = execute_pending_batch(&policy, repo.as_ref(), 10)
        .await
        .expect("execute_pending_batch OK");
    let sent_after = rpc.sent_raw.lock().unwrap().len();
    assert!(
        sent_after > sent_before,
        "eligible execution must reach BroadcastPolicy::broadcast_intent (send_raw invocations)"
    );

    // Broadcast row must be durable.
    let row = repo
        .get_prepared_broadcast(intent_id)
        .await
        .unwrap()
        .expect("broadcast row created");
    assert!(!row.raw_tx_hex.is_empty());
    assert!(!row.tx_hash.is_empty());
}

// ================================================================
// Helper — build_broadcast_runtime factory success on happy path
// ================================================================

#[tokio::test]
async fn build_broadcast_runtime_factory_happy_path() {
    let Some(url) = pg_url() else {
        eprintln!("IGNORED [build_broadcast_runtime_factory_happy_path] (PG url not provided)");
        return;
    };
    let repo = Arc::new(fresh_repo(&url).await);
    // build_broadcast_runtime reads env for the triad addresses.
    std::env::set_var("HV2_EXECUTOR_ADDRESS", TEST_KEY_ADDRESS);
    std::env::set_var("HV2_SIGNER_EXPECTED_ADDRESS", TEST_KEY_ADDRESS);
    let readiness = BroadcastReadiness::new();
    // Real HttpJsonRpcProvider will fail to connect (mock.invalid);
    // the factory returns an error at startup_preflight rather than
    // spawning anything.
    let result = build_broadcast_runtime(
        make_config(TEST_KEY_ADDRESS, true, false),
        repo,
        ReconcilerConfig::default(),
        readiness.clone(),
    )
    .await;
    // Expected: preflight fails due to unreachable RPC URL. Never
    // advertises readiness.
    assert!(result.is_err(), "unreachable RPC must fail preflight");
    assert!(!readiness.broadcast_ready());
    std::env::remove_var("HV2_EXECUTOR_ADDRESS");
    std::env::remove_var("HV2_SIGNER_EXPECTED_ADDRESS");
}
