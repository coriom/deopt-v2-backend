use super::executor::{ExecutionIntentRepository, Executor};
use crate::error::Result;
use tokio::task::JoinHandle;
use tokio::time::{sleep, Duration};
use tracing::error;

pub fn spawn_executor<R>(executor: Executor<R>, poll_interval_ms: u64) -> JoinHandle<()>
where
    R: ExecutionIntentRepository + 'static,
{
    tokio::spawn(async move {
        loop {
            if let Err(error) = tick_once(&executor).await {
                error!(%error, "executor dry-run tick failed");
            }
            sleep(Duration::from_millis(poll_interval_ms)).await;
        }
    })
}

async fn tick_once<R>(executor: &Executor<R>) -> Result<()>
where
    R: ExecutionIntentRepository,
{
    executor.tick().await.map(|_| ())
}
