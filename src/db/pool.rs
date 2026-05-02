use crate::error::{BackendError, Result};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgPool, PgPoolOptions};
use std::path::Path;
use std::time::Duration;

pub async fn connect(database_url: &str) -> Result<PgPool> {
    PgPoolOptions::new()
        .max_connections(5)
        .acquire_timeout(Duration::from_secs(5))
        .connect(database_url)
        .await
        .map_err(|error| BackendError::Persistence(error.to_string()))
}

pub async fn run_migrations(pool: &PgPool) -> Result<()> {
    let migrator = Migrator::new(Path::new("./migrations"))
        .await
        .map_err(|error| BackendError::Persistence(error.to_string()))?;
    migrator
        .run(pool)
        .await
        .map_err(|error| BackendError::Persistence(error.to_string()))
}
