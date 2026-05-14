use deopt_v2_backend::api::{router, AppState};
use deopt_v2_backend::config::AppConfig;
use deopt_v2_backend::db::PgRepository;
use deopt_v2_backend::engine::EngineState;
use deopt_v2_backend::execution::{spawn_executor, Executor};
use deopt_v2_backend::indexer::{spawn_indexer, Indexer};
use deopt_v2_backend::mm::transport::webtransport::spawn_webtransport_gateway;
use tracing::info;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> deopt_v2_backend::Result<()> {
    let config = AppConfig::from_env()?;
    config
        .execution
        .validate_startup(config.persistence_enabled)?;
    config.perp_nonce_sync.validate_startup()?;
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
        None
    };
    let mut state = AppState::with_all_config(
        EngineState::with_default_markets(),
        config.signature_verification_mode,
        config.eip712_domain.clone(),
        repository.clone(),
        config.execution.clone(),
        config.perp_nonce_sync.clone(),
        config.confirmation.clone(),
        config.indexer.clone(),
        config.reconciliation.clone(),
        config.rfq.clone(),
        config.options.clone(),
        config.chain_id,
    );
    state.network_name = config.network_name.clone();
    state.persistence_enabled = config.persistence_enabled;
    state.database_configured = config.database_url.is_some();
    state.mm_gateway_config = config.mm_gateway.clone();
    state.mm_permissions_config = config.mm_permissions.clone();
    state.admin_config = config.admin.clone();
    let app = router(state.clone());

    if config.execution.execution_enabled {
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
    spawn_webtransport_gateway(config.mm_gateway.clone(), state).await?;

    info!(
        service = "deopt-v2-backend",
        %addr,
        chain_id = config.chain_id,
        network = %config.network_name,
        execution_enabled = config.execution.execution_enabled,
        confirmation_enabled = config.confirmation.enabled,
        rfq_enabled = config.rfq.enabled,
        options_enabled = config.options.enabled,
        mm_gateway_enabled = config.mm_gateway.enabled,
        mm_permissions_enabled = config.mm_permissions.enabled,
        indexer_enabled = config.indexer.enabled,
        reconciliation_enabled = config.reconciliation.enabled,
        executor_dry_run = config.execution.dry_run,
        signature_verification_mode = ?config.signature_verification_mode,
        persistence_enabled = config.persistence_enabled,
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
