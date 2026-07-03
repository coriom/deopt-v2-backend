//! PERPS-MINIMAL-MARKET-AND-PRICE-V1 — HTTP-level integration tests.
//!
//! These tests exercise the router around the three new read-only
//! Perps endpoints and pin the invariants the milestone brief requires:
//!
//!   * Read routes return 503 `PerpsReadDisabled` in the default
//!     (safety) configuration.
//!   * Perp mutation routes remain fail-closed with `PerpsNotLive`
//!     (regression pin — must not accidentally flip live).
//!   * `/markets` does not leak perp markets as tradable (regression
//!     pin — inherited from prior milestone; re-asserted here so a
//!     future change to /perps/markets can't accidentally re-open the
//!     `/markets` leak).
//!   * When the reader config is enabled but the RPC endpoint is
//!     unreachable, the endpoint surfaces `PerpsPriceUnavailable`
//!     (503) — it does NOT fabricate a price.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use deopt_v2_backend::api::{router, AppState};
use deopt_v2_backend::engine::EngineState;
use deopt_v2_backend::perps::PerpsReadConfig;
use deopt_v2_backend::types::AccountId;
use tower::ServiceExt;

fn state_with_perps_read_disabled() -> AppState {
    AppState::new(EngineState::with_default_markets())
}

fn state_with_perps_read_enabled_but_dead_rpc() -> AppState {
    let mut state = AppState::new(EngineState::with_default_markets());
    let mut cfg = PerpsReadConfig::enabled_in_memory_for_tests();
    // Point at a definitely-unreachable local port so the eth_call
    // fails at transport level. We are asserting we surface a 503, not
    // fabricate a number.
    cfg.rpc_url = Some("http://127.0.0.1:1".to_string());
    state.perps_read_config = cfg;
    state
}

fn get(uri: &str) -> Request<Body> {
    Request::builder()
        .method("GET")
        .uri(uri)
        .body(Body::empty())
        .unwrap()
}

async fn body_json(response: axum::response::Response) -> serde_json::Value {
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

// =====================================================================
// A. Disabled config — every read route returns 503 PerpsReadDisabled
// =====================================================================

#[tokio::test]
async fn perps_markets_returns_503_when_read_disabled() {
    let app = router(state_with_perps_read_disabled());
    let response = app.oneshot(get("/perps/markets")).await.unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body = body_json(response).await;
    assert!(
        body["error"]
            .as_str()
            .unwrap_or("")
            .to_lowercase()
            .contains("disabled"),
        "expected `disabled` in error body: {body}"
    );
}

#[tokio::test]
async fn perps_market_single_returns_503_when_read_disabled() {
    let app = router(state_with_perps_read_disabled());
    let response = app.oneshot(get("/perps/markets/ETH-PERP")).await.unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn perps_market_price_returns_503_when_read_disabled() {
    let app = router(state_with_perps_read_disabled());
    let response = app
        .oneshot(get("/perps/markets/ETH-PERP/price"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
}

// =====================================================================
// B. Perps mutation routes MUST remain fail-closed (regression pin)
// =====================================================================

#[tokio::test]
async fn perp_submit_order_still_fail_closed_regardless_of_read_config() {
    let app = router(state_with_perps_read_enabled_but_dead_rpc());
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/orders")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{
                        "market_id": 1,
                        "account": "0x0000000000000000000000000000000000000abc",
                        "side": "buy",
                        "price_1e8": "1000",
                        "size_1e8": "10",
                        "time_in_force": "gtc",
                        "reduce_only": false,
                        "post_only": false,
                        "client_order_id": null,
                        "signed_nonce": null,
                        "signed_deadline_ms": null
                    }"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::SERVICE_UNAVAILABLE,
        "enabling PERPS_READ_ENABLED must NEVER flip the perp mutation gate"
    );
    let body = body_json(response).await;
    assert!(
        body["error"]
            .as_str()
            .unwrap_or("")
            .to_lowercase()
            .contains("perp"),
        "expected `perp` in error body: {body}"
    );
}

// =====================================================================
// C. /markets must not leak perp markets as tradable (regression pin)
// =====================================================================

#[tokio::test]
async fn markets_still_filters_out_perps_even_when_read_layer_is_configured() {
    let app = router(state_with_perps_read_enabled_but_dead_rpc());
    let response = app.oneshot(get("/markets")).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    let arr = body.as_array().expect("markets returns array");
    for market in arr {
        assert_ne!(
            market["kind"].as_str().unwrap_or(""),
            "perp",
            "/markets leaked a perp market as tradable: {market}"
        );
    }
}

// =====================================================================
// D. Enabled + dead RPC — never fabricates. Price returns 503.
//    Market list returns rows with `unknown` status (per-row resilience;
//    the row still exists, but its liveness flag is honestly unknown).
// =====================================================================

#[tokio::test]
async fn perps_market_price_returns_503_when_rpc_unreachable() {
    // Reader is enabled, but the RPC endpoint doesn't exist.
    // The correct behavior is a 503 `PerpsPriceUnavailable` — NOT a
    // fabricated zero/mock price.
    let app = router(state_with_perps_read_enabled_but_dead_rpc());
    let response = app
        .oneshot(get("/perps/markets/ETH-PERP/price"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body = body_json(response).await;
    assert!(
        !body.get("index_price_1e8").is_some(),
        "must not include a fabricated price on RPC failure: {body}"
    );
    assert!(
        !body.get("mark_price_1e8").is_some(),
        "must not include a fabricated price on RPC failure: {body}"
    );
}

#[tokio::test]
async fn perps_markets_survives_reader_failure_without_fabricating_status() {
    // With a dead RPC, every per-row reader call errors — the service
    // layer must fall back to `PerpMarketStatus::Unknown`, NEVER to
    // `read_only` (which would imply a live check succeeded).
    let app = router(state_with_perps_read_enabled_but_dead_rpc());
    let response = app.oneshot(get("/perps/markets")).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    let markets = body["markets"].as_array().expect("markets array");
    assert_eq!(markets.len(), 2, "expected ETH-PERP + BTC-PERP");
    for market in markets {
        assert_eq!(
            market["status"].as_str().unwrap_or(""),
            "unknown",
            "reader failure MUST NOT surface as `read_only`: {market}"
        );
        assert_eq!(
            market["trading_enabled"].as_bool(),
            Some(false),
            "trading_enabled must always be false in V1: {market}"
        );
    }
    assert_eq!(body["trading_enabled"].as_bool(), Some(false));
    assert_eq!(body["chain_id"].as_u64(), Some(84532));
}

// =====================================================================
// E. Malformed symbol — 404 PerpsMarketNotFound
// =====================================================================

#[tokio::test]
async fn perps_market_unknown_symbol_returns_404() {
    let app = router(state_with_perps_read_enabled_but_dead_rpc());
    let response = app.oneshot(get("/perps/markets/NOPE-PERP")).await.unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn perps_market_price_unknown_symbol_returns_404() {
    let app = router(state_with_perps_read_enabled_but_dead_rpc());
    let response = app
        .oneshot(get("/perps/markets/NOPE-PERP/price"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

// =====================================================================
// F. Silence the unused-import warning in the disabled path fixtures.
// =====================================================================

#[test]
fn test_utilities_silence_dead_code_warnings() {
    let _ = AccountId::new("0x0000000000000000000000000000000000000000");
}
