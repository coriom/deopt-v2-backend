//! PERPS_CLOSE_PNL_BUG_RETIRED_AND_V2_ACCOUNTING_DESIGN
//!
//! Operator tool: administratively retire an execution intent that must
//! NEVER be signed / cosigned / simulated / armed / broadcast (e.g. the
//! target trade would trigger a confirmed accounting defect in the
//! deployed engine).
//!
//! Usage:
//!   DATABASE_URL=postgres://deopt:deopt@127.0.0.1:5432/deopt_v2_backend \
//!     retire_execution_intent <intent-uuid>
//!
//! Fails closed if the target UUID is Prepared / Submitted / Confirmed /
//! Failed / already Abandoned. The row is preserved for audit; only the
//! `status` column transitions to `abandoned`.
use deopt_v2_backend::db::PgRepository;
use deopt_v2_backend::error::BackendError;
use deopt_v2_backend::types::now_ms;
use std::env;
use std::process::ExitCode;
use uuid::Uuid;

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    match run().await {
        Ok(msg) => {
            println!("{msg}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("retire_execution_intent: {error}");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<String, BackendError> {
    let uuid_str = env::args().nth(1).ok_or_else(|| {
        BackendError::Config("usage: retire_execution_intent <intent-uuid>".to_string())
    })?;
    let intent_id = Uuid::parse_str(&uuid_str)
        .map_err(|error| BackendError::Config(format!("invalid uuid: {error}")))?;
    let database_url = env::var("DATABASE_URL")
        .map_err(|_| BackendError::Config("DATABASE_URL not set".to_string()))?;

    let repo = PgRepository::connect(&database_url).await?;

    let previous = repo.retire_execution_intent(intent_id, now_ms()).await?;
    Ok(format!(
        "retired intent_id={intent_id} previous_status={previous:?} new_status=abandoned"
    ))
}
