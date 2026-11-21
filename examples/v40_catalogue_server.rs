//! Local-only V40 HTTP harness: public reads from the historical Registry.
//! Run with `CARGO_BUILD_JOBS=1 cargo run --offline --example v40_catalogue_server`.
//! Listens on loopback only; never connects to PostgreSQL or signs transactions.

use axum::{middleware, routing::get, Router};
use deopt_v2_backend::{
    api::{trading, AppState},
    engine::EngineState,
    read_only_release::reject_http_writes,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut state = AppState::new(EngineState::with_default_markets());
    state.read_only_release = true;
    state.chain_id = 84532;
    state.execution_config.rpc_url = Some("https://sepolia.base.org".to_string());
    let app = Router::new()
        .route("/options/products", get(trading::list_products))
        .route("/options/products/:product_id", get(trading::get_product))
        .route("/options/series/:series_id/details", get(trading::series_details))
        .with_state(state.clone())
        .layer(middleware::from_fn_with_state(state, reject_http_writes));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:31041").await?;
    axum::serve(listener, app).await?;
    Ok(())
}
