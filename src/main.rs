use deopt_v2_backend::api::{router, AppState};
use deopt_v2_backend::config::AppConfig;
use deopt_v2_backend::db::PgRepository;
use deopt_v2_backend::engine::EngineState;
use deopt_v2_backend::execution::{spawn_executor, Executor};
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
    let app = router(state.clone());

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
    // BACKEND-HYBRID-V2-PERSISTED-RUNTIME-CORE-V1 — the persisted
    // Hybrid V2 indexer worker is fully wired at the code layer
    // (`hybrid_v2::worker::spawn_hybrid_v2_indexer_worker`) and
    // integration-tested against a real Postgres + `InMemoryChainSource`.
    // Production activation additionally requires a live-chain
    // `ChainSource` (RPC or similar) which lands in the next stage of
    // this milestone tree. When `HYBRID_V2_ENABLED=true` today we
    // therefore log the configured state and defer the spawn; when
    // `false` (default) we silently skip so unconfigured backends keep
    // starting normally.
    if config.hybrid_v2.enabled {
        if !config.persistence_enabled {
            warn!(
                deployment_id = config.hybrid_v2.deployment_id,
                chain_id = config.hybrid_v2.chain_id,
                "HYBRID_V2_ENABLED=true but PERSISTENCE_ENABLED=false — refusing to spawn the \
                 persisted Hybrid V2 indexer worker; canonical routes remain fail-closed."
            );
        } else {
            info!(
                deployment_id = config.hybrid_v2.deployment_id,
                chain_id = config.hybrid_v2.chain_id,
                poll_interval_ms = config.hybrid_v2.poll_interval_ms,
                confirmation_depth = config.hybrid_v2.confirmation_depth,
                cursor_name = %config.hybrid_v2.cursor_name,
                "hybrid_v2 indexer worker configured; deferred until an RPC ChainSource lands \
                 in the next stage (writer path is fully tested via InMemoryChainSource)"
            );
        }
    }
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
