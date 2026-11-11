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

/// The hosted read-only API must never perform DDL with its runtime identity.
pub async fn verify_read_only_schema(pool: &PgPool) -> Result<()> {
    let (applied, highest, failed): (i64, i64, i64) = sqlx::query_as(
        "SELECT COUNT(*)::BIGINT, COALESCE(MAX(version),0)::BIGINT, \
         COUNT(*) FILTER (WHERE NOT success)::BIGINT FROM _sqlx_migrations",
    )
    .fetch_one(pool)
    .await
    .map_err(|error| BackendError::Persistence(error.to_string()))?;
    if (applied, highest, failed) != (74, 74, 0) {
        return Err(BackendError::Persistence(
            "read-only release requires exactly 74 successful migrations".into(),
        ));
    }
    Ok(())
}
