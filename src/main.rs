use deopt_v2_backend::api::{router, AppState};
use deopt_v2_backend::config::AppConfig;
use deopt_v2_backend::db::PgRepository;
use deopt_v2_backend::engine::EngineState;
use tracing::info;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> deopt_v2_backend::Result<()> {
    let config = AppConfig::from_env()?;
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
    let state = AppState::with_signature_mode_domain_and_repository(
        EngineState::with_default_markets(),
        config.signature_verification_mode,
        config.eip712_domain.clone(),
        repository,
    );
    let app = router(state);

    info!(
        service = "deopt-v2-backend",
        %addr,
        chain_id = config.chain_id,
        network = %config.network_name,
        execution_enabled = config.execution_enabled,
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
