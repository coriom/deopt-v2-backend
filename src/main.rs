use deopt_v2_backend::api::{router, AppState};
use deopt_v2_backend::config::AppConfig;
use deopt_v2_backend::db::PgRepository;
use deopt_v2_backend::engine::EngineState;
use deopt_v2_backend::error::BackendError;
use deopt_v2_backend::execution::{spawn_executor, Executor};
use deopt_v2_backend::hybrid_v2::{
    spawn_hybrid_v2_indexer_worker, HybridV2IndexerWorkerConfig, IndexerRuntime, ManifestParams,
    PostgresHybridV2ProjectionStore, RpcHybridV2ChainSource, RpcSourceConfig,
};
use deopt_v2_backend::indexer::{spawn_indexer, Indexer};
use deopt_v2_backend::mm::transport::webtransport::spawn_webtransport_gateway;
use deopt_v2_backend::options::conditional_orders::{
    spawn_conditional_orders_worker, ConditionalOrdersConfig,
};
use deopt_v2_backend::options::{
    spawn_option_confirmation_worker, spawn_option_event_indexer,
    spawn_option_reconciliation_worker,
};
use deopt_v2_backend::perps::{spawn_perps_funding_worker, spawn_perps_liquidation_worker};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{watch, RwLock};
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> deopt_v2_backend::Result<()> {
    let config = AppConfig::from_env()?;
    config
        .execution
        .validate_startup(config.persistence_enabled)?;
    config.perp_nonce_sync.validate_startup()?;
    config.option_nonce_sync.validate_startup()?;
    config
        .option_confirmation
        .validate_startup(config.persistence_enabled)?;
    config
        .option_event_indexer
        .validate_startup(config.persistence_enabled)?;
    config
        .option_reconciliation
        .validate_startup(config.persistence_enabled)?;
    config
        .indexer
        .validate_startup(config.persistence_enabled)?;
    config
        .reconciliation
        .validate_startup(config.persistence_enabled)?;
    config
        .confirmation
        .validate_startup(config.persistence_enabled)?;
    config.rfq.validate_startup(config.persistence_enabled)?;
    config
        .options
        .validate_startup(config.persistence_enabled)?;
    config
        .mm_permissions
        .validate_startup(config.persistence_enabled)?;
    config.fees.validate_startup(config.persistence_enabled)?;
    // BACKEND-HYBRID-V2-PERSISTED-RUNTIME-CORE-V1 — validate the
    // Hybrid V2 config at startup so mis-configuration (Base mainnet,
    // out-of-range bounds, missing deployment_id) fails fast rather
    // than after network state has been mutated. `disabled()` returns
    // Ok immediately.
    config.hybrid_v2.validate()?;
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::new(config.rust_log.clone()))
        .init();

    let addr = config.socket_addr()?;
    let repository = if config.persistence_enabled {
        let database_url = config.database_url.as_deref().ok_or_else(|| {
            deopt_v2_backend::error::BackendError::Config(
                "DATABASE_URL is required when PERSISTENCE_ENABLED=true".to_string(),
            )
        })?;
        let repository = PgRepository::connect(database_url).await?;
        repository.run_migrations().await?;
        Some(repository)
    } else {
        // SUBACCOUNTS-PERSISTENCE-SESSION-RELOAD-V1 — surface the
        // ephemeral-store posture at startup so an operator running
        // `cargo run` without `PERSISTENCE_ENABLED=true` sees
        // immediately (not after a mysterious data loss) that
        // subaccounts, write-auth challenges, and used-nonces v2 are
        // held in an in-memory store that resets on every restart.
        warn!(
            "PERSISTENCE_ENABLED=false: subaccounts, write-auth challenges, \
             and used-nonces-v2 are stored in memory and will NOT survive a \
             backend restart. Set PERSISTENCE_ENABLED=true + DATABASE_URL to \
             a Postgres URL to persist across restarts (see README)."
        );
        None
    };
    let mut state = AppState::with_all_config(
        EngineState::with_default_markets(),
        config.signature_verification_mode,
        config.eip712_domain.clone(),
        repository.clone(),
        config.execution.clone(),
        config.perp_nonce_sync.clone(),
        config.option_nonce_sync.clone(),
        config.confirmation.clone(),
        config.indexer.clone(),
        config.reconciliation.clone(),
        config.rfq.clone(),
        config.options.clone(),
        config.fees.clone(),
        config.chain_id,
    );
    state.option_confirmation_config = config.option_confirmation.clone();
    state.option_event_indexer_config = config.option_event_indexer.clone();
    state.option_reconciliation_config = config.option_reconciliation.clone();
    state.network_name = config.network_name.clone();
    state.persistence_enabled = config.persistence_enabled;
    state.database_configured = config.database_url.is_some();
    state.mm_gateway_config = config.mm_gateway.clone();
    state.mm_permissions_config = config.mm_permissions.clone();
    state.public_ws_config = config.public_ws.clone();
    state.fees_config = config.fees.clone();
    state.admin_config = config.admin.clone();
    state.metrics_config = config.metrics.clone();
    state.trading_views = config.trading_views.clone();
    state.perps_read_config = config.perps_read.clone();
    // PERPS-FRONTEND-TICKET-ENABLEMENT-V1 — propagate the strict
    // opt-in flag from env to `AppState`. Default remains false.
    state.perps_public_trading_enabled = config.perps_public_trading_enabled;
    // PERPS-SUBACCOUNTS-CORE-ROUTING-V1 — propagate closed-test flag
    // + allowlist. Independent of the public trading flag; both
    // remain fail-closed by default.
    state.perps_closed_test_enabled = config.perps_closed_test_enabled;
    state.perps_closed_test_allowlist = config.perps_closed_test_allowlist.clone();
    // PERPS-FUNDING-LIQUIDATION-WORKERS-V1 — periodic worker config.
    // Both configs default to `disabled()`; the spawn functions return
    // immediately unless `worker_enabled=true`, and the admin HTTP
    // ticks in `routes.rs` consult `tick_enabled` too.
    state.perps_funding_worker_config = config.perps_funding_worker.clone();
    state.perps_liquidation_worker_config = config.perps_liquidation_worker.clone();
    // OPTIONS-CONDITIONAL-ORDERS-PERSISTENT-E2E-V1 — read worker env
    // vars. Defaults are safe (enabled=false) so this is a no-op for
    // any operator who has not opted in.
    state.conditional_orders_config = ConditionalOrdersConfig::from_env();

    if config.execution.execution_enabled && config.execution.dry_run {
        if let Some(repository) = repository.clone() {
            spawn_executor(
                Executor::new(config.execution.clone(), repository),
                config.execution.poll_interval_ms,
            );
        }
    }
    if config.indexer.enabled {
        if let Some(repository) = repository.clone() {
            let indexer = Indexer::from_config_and_repository(config.indexer.clone(), repository)?;
            spawn_indexer(indexer, config.indexer.poll_interval_ms);
        }
    }
    spawn_option_confirmation_worker(state.clone());
    spawn_option_event_indexer(state.clone());
    spawn_option_reconciliation_worker(state.clone());
    // Default-off. Refuses to spawn when oracle/RPC missing — see
    // implementation in `src/options/conditional_orders.rs`.
    spawn_conditional_orders_worker(state.clone());
    // PERPS-FUNDING-LIQUIDATION-WORKERS-V1 — periodic funding +
    // liquidation workers. Both no-op unless the respective
    // `worker_enabled` flag is true; the `tick_enabled` kill-switch
    // controls whether the ticks execute or record a "skipped"
    // heartbeat, and the same kill-switch is honoured by the admin
    // HTTP ticks so the two surfaces cannot diverge.
    spawn_perps_funding_worker(state.clone());
    spawn_perps_liquidation_worker(state.clone());
    // BACKEND-HYBRID-V2-LIVE-CHAIN-SOURCE-AND-WORKER-ACTIVATION-V1 —
    // Real live-chain worker spawn. When `HYBRID_V2_ENABLED=true`:
    //   1. Persistence must be enabled (canonical route contract).
    //   2. Manifest is loaded from disk (`HYBRID_V2_MANIFEST_PATH`).
    //   3. A read-only `RpcHybridV2ChainSource` is constructed against
    //      the configured JSON-RPC endpoint and its chain identity is
    //      validated (mismatch or Base mainnet → fail-closed).
    //   4. `bootstrap_from_persistence` runs before the tokio task is
    //      spawned. `BootstrapResult::Diverged` fails startup so the
    //      operator triggers a rebuild instead of serving inconsistent
    //      state.
    //   5. `spawn_hybrid_v2_indexer_worker` binds the runtime, source,
    //      store and shutdown channel.
    //
    // Every failure path here returns before `axum::serve` binds — the
    // process exits with a non-zero status rather than starting the
    // HTTP surface with a mis-configured indexer behind it.
    let mut _hybrid_v2_shutdown_tx: Option<watch::Sender<bool>> = None;
    if config.hybrid_v2.enabled {
        if !config.persistence_enabled {
            warn!(
                deployment_id = config.hybrid_v2.deployment_id,
                chain_id = config.hybrid_v2.chain_id,
                "HYBRID_V2_ENABLED=true but PERSISTENCE_ENABLED=false — refusing to spawn the \
                 persisted Hybrid V2 indexer worker; canonical routes remain fail-closed."
            );
        } else if let Some(repo) = repository.clone() {
            let manifest = load_hybrid_v2_manifest(&config.hybrid_v2)?;
            if manifest.chain_id != config.hybrid_v2.chain_id {
                return Err(BackendError::Config(format!(
                    "HYBRID_V2 manifest chain_id={} disagrees with HYBRID_V2_CHAIN_ID={}",
                    manifest.chain_id, config.hybrid_v2.chain_id
                )));
            }
            let emitters = collect_manifest_emitters(&manifest);
            let rpc_endpoint = config.hybrid_v2.rpc_url.clone().ok_or_else(|| {
                BackendError::Config(
                    "HYBRID_V2_RPC_URL missing after validation — refusing to spawn".into(),
                )
            })?;
            let source_config = RpcSourceConfig {
                endpoint: rpc_endpoint,
                chain_id: config.hybrid_v2.chain_id,
                timeout: Duration::from_millis(config.hybrid_v2.rpc_timeout_ms),
                max_retries: config.hybrid_v2.rpc_max_retries,
                retry_backoff: Duration::from_millis(config.hybrid_v2.rpc_retry_backoff_ms),
                max_logs_per_range: config.hybrid_v2.rpc_max_logs_per_range,
                confirmation_depth: config.hybrid_v2.confirmation_depth,
            };
            let source = RpcHybridV2ChainSource::new(source_config, emitters).map_err(|e| {
                BackendError::Config(format!("hybrid_v2 rpc source construction: {e}"))
            })?;
            let redacted_host = source.redacted_endpoint();
            let observed = source.validate_chain_identity().await.map_err(|e| {
                BackendError::Config(format!(
                    "hybrid_v2 chain identity validation failed against endpoint {redacted_host}: {e}"
                ))
            })?;
            let source_arc: Arc<dyn deopt_v2_backend::hybrid_v2::ChainSource> = Arc::new(source);
            let store = Arc::new(PostgresHybridV2ProjectionStore::new(repo.pool().clone()));
            // BACKEND-HYBRID-V2-PROJECTION-PERSISTENCE-OPERATIONAL-CLOSURE-V1
            // Attach the projection store to AppState so mounted admin
            // routes (`/admin/hybrid_v2/...`) can drive rebuild +
            // reconciliation without another handle.
            state =
                state
                    .with_hybrid_v2_projection_store(store.clone()
                        as Arc<dyn deopt_v2_backend::hybrid_v2::HybridV2ProjectionStore>);
            let mut runtime = IndexerRuntime::new(config.hybrid_v2.deployment_id as u64, manifest)
                .with_persistence(store.clone(), config.hybrid_v2.deployment_id)
                .with_persistence_cursor_name(config.hybrid_v2.cursor_name.clone());
            let bootstrap = runtime
                .bootstrap_from_persistence()
                .await
                .map_err(|e| BackendError::Config(format!("hybrid_v2 bootstrap failed: {e}")))?;
            if let deopt_v2_backend::hybrid_v2::BootstrapResult::Diverged {
                persisted_head,
                replayed_head,
                detail,
            } = &bootstrap
            {
                return Err(BackendError::Config(format!(
                    "hybrid_v2 bootstrap DIVERGED (persisted_head={persisted_head}, \
                     replayed_head={replayed_head}); refusing to spawn worker: {detail}"
                )));
            }
            if let deopt_v2_backend::hybrid_v2::BootstrapResult::ChainForbidden { chain_id } =
                &bootstrap
            {
                return Err(BackendError::Config(format!(
                    "hybrid_v2 bootstrap refused: chain_id={chain_id} is not authorised",
                )));
            }
            let runtime_arc = Arc::new(RwLock::new(runtime));
            let worker_config = HybridV2IndexerWorkerConfig::new(
                config.hybrid_v2.deployment_id,
                config.hybrid_v2.poll_interval_ms,
            );
            let (shutdown_tx, shutdown_rx) = watch::channel(false);
            let _worker_handle = spawn_hybrid_v2_indexer_worker(
                runtime_arc,
                source_arc,
                store,
                worker_config,
                Some(shutdown_rx),
            );
            _hybrid_v2_shutdown_tx = Some(shutdown_tx);
            info!(
                deployment_id = config.hybrid_v2.deployment_id,
                chain_id = config.hybrid_v2.chain_id,
                observed_chain_id = observed,
                poll_interval_ms = config.hybrid_v2.poll_interval_ms,
                confirmation_depth = config.hybrid_v2.confirmation_depth,
                cursor_name = %config.hybrid_v2.cursor_name,
                rpc_endpoint = %redacted_host,
                bootstrap = ?bootstrap,
                "hybrid_v2 live indexer worker spawned"
            );
        }
    }
    // Build the router AFTER hybrid_v2 wiring so mounted operator
    // admin routes see the projection-store handle attached above.
    let app = router(state.clone());
    spawn_webtransport_gateway(config.mm_gateway.clone(), state).await?;

    info!(
        service = "deopt-v2-backend",
        %addr,
        chain_id = config.chain_id,
        network = %config.network_name,
        execution_enabled = config.execution.execution_enabled,
        confirmation_enabled = config.confirmation.enabled,
        option_confirmation_worker_enabled = config.option_confirmation.enabled,
        option_event_indexer_enabled = config.option_event_indexer.enabled,
        option_reconciliation_worker_enabled = config.option_reconciliation.enabled,
        rfq_enabled = config.rfq.enabled,
        options_enabled = config.options.enabled,
        fees_enabled = config.fees.enabled,
        rebates_enabled = config.fees.rebates_enabled,
        metrics_enabled = config.metrics.enabled,
        mm_gateway_enabled = config.mm_gateway.enabled,
        mm_permissions_enabled = config.mm_permissions.enabled,
        public_ws_enabled = config.public_ws.enabled,
        public_ws_path = %config.public_ws.path,
        indexer_enabled = config.indexer.enabled,
        reconciliation_enabled = config.reconciliation.enabled,
        executor_dry_run = config.execution.dry_run,
        signature_verification_mode = ?config.signature_verification_mode,
        persistence_enabled = config.persistence_enabled,
        hybrid_v2_enabled = config.hybrid_v2.enabled,
        hybrid_v2_deployment_id = config.hybrid_v2.deployment_id,
        "starting http server"
    );

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|error| deopt_v2_backend::error::BackendError::Config(error.to_string()))?;
    axum::serve(listener, app)
        .await
        .map_err(|error| deopt_v2_backend::error::BackendError::Config(error.to_string()))?;
    Ok(())
}

/// Load a Hybrid V2 deployment manifest from `HYBRID_V2_MANIFEST_PATH`.
/// Fails-closed when the path is unset or unreadable.
fn load_hybrid_v2_manifest(
    cfg: &deopt_v2_backend::hybrid_v2::HybridV2Config,
) -> deopt_v2_backend::Result<ManifestParams> {
    let path = cfg.manifest_path.as_deref().ok_or_else(|| {
        BackendError::Config(
            "HYBRID_V2_MANIFEST_PATH unset — cannot bind emitter addresses without a manifest"
                .into(),
        )
    })?;
    let bytes = std::fs::read(path)
        .map_err(|e| BackendError::Config(format!("hybrid_v2 manifest read {path}: {e}")))?;
    let manifest: ManifestParams = serde_json::from_slice(&bytes)
        .map_err(|e| BackendError::Config(format!("hybrid_v2 manifest parse {path}: {e}")))?;
    Ok(manifest)
}

/// Collect every canonical emitter address declared by the manifest,
/// lowercased and 0x-prefixed. Includes optional modules and the
/// manifest address itself.
fn collect_manifest_emitters(manifest: &ManifestParams) -> Vec<String> {
    let m = &manifest.module_addresses;
    let mut set: std::collections::HashSet<String> = std::collections::HashSet::new();
    for addr in [
        &m.subaccount_registry,
        &m.collateral_vault,
        &m.options_positions_ledger,
        &m.risk_module,
        &m.margin_engine,
        &m.option_matching_engine,
        &m.escape_controller,
        &m.recovery_finalizer,
    ] {
        set.insert(addr.to_ascii_lowercase());
    }
    if let Some(a) = &m.fees_manager_v2 {
        set.insert(a.to_ascii_lowercase());
    }
    if let Some(a) = &m.option_execution_fee_adapter {
        set.insert(a.to_ascii_lowercase());
    }
    set.insert(manifest.manifest_address.to_ascii_lowercase());
    set.into_iter().collect()
}
