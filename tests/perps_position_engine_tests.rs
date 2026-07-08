//! PERPS-ISOLATED-MARGIN-POSITION-ENGINE-V1 — HTTP-level integration
//! tests for the read-only account positions endpoint plus regression
//! pins on Perps mutation fail-closed behaviour.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use deopt_v2_backend::api::{router, AppState};
use deopt_v2_backend::engine::EngineState;
use deopt_v2_backend::perps::{
    apply_perp_fill_for_account, PerpFillInput, PerpSide, PerpsReadConfig,
};
use deopt_v2_backend::types::AccountId;
use tower::ServiceExt;

fn state_with_perps_seeded() -> AppState {
    // Reader disabled by default (no RPC in tests). The endpoint
    // must still serve the internal ledger; unavailable mark price
    // surfaces as `null` risk fields + `price_stale: true`.
    let mut state = AppState::new(EngineState::with_default_markets());
    state.perps_read_config = PerpsReadConfig::enabled_in_memory_for_tests();
    state.perps_read_config.rpc_url = None;
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
// A. Empty list is the honest default answer.
// =====================================================================

#[tokio::test]
async fn positions_endpoint_returns_empty_list_by_default() {
    let state = state_with_perps_seeded();
    let app = router(state);
    let response = app
        .oneshot(get(
            "/accounts/0x000000000000000000000000000000000000abcd/perps/positions",
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    let positions = body["positions"].as_array().expect("positions array");
    assert!(positions.is_empty(), "expected empty list, got {body}");
    assert_eq!(body["chain_id"].as_u64(), Some(84532));
    assert_eq!(body["trading_enabled"].as_bool(), Some(false));
}

// =====================================================================
// B. Persisted position round-trips through the endpoint with honest
//    risk fields (mark price null, price_stale true).
// =====================================================================

#[tokio::test]
async fn positions_endpoint_surfaces_internal_ledger_with_honest_unavailable_mark() {
    let state = state_with_perps_seeded();
    let cfg = state.perps_read_config.clone();
    let market = cfg.market_by_symbol("ETH-PERP").unwrap().clone();
    let account = AccountId::new("0x000000000000000000000000000000000000dEaD".to_string());
    {
        let mut store = state.perp_positions_store.lock().unwrap();
        apply_perp_fill_for_account(
            &mut store,
            &market,
            PerpFillInput {
                account: account.clone(),
                subaccount_id: 1,
                market_id: "ETH-PERP".to_string(),
                side: PerpSide::Long,
                size_1e8: 100_000_000,
                price_1e8: 300_000_000_000,
                margin_1e8: 30_000_000_000,
            },
        )
        .unwrap();
    }
    let app = router(state);
    let response = app
        .oneshot(get(
            "/accounts/0x000000000000000000000000000000000000dEaD/perps/positions",
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    let positions = body["positions"].as_array().unwrap();
    assert_eq!(positions.len(), 1);
    let row = &positions[0];
    assert_eq!(row["market_id"].as_str(), Some("ETH-PERP"));
    assert_eq!(row["side"].as_str(), Some("long"));
    assert_eq!(row["size_1e8"].as_str(), Some("100000000"));
    assert_eq!(row["entry_price_1e8"].as_str(), Some("300000000000"));
    assert_eq!(row["margin_1e8"].as_str(), Some("30000000000"));
    assert_eq!(row["realized_pnl_1e8"].as_str(), Some("0"));
    // Without a live mark, risk fields are null / price is stale.
    // The estimated liquidation price CAN still be computed
    // (function of entry/size/margin/mm), so it's surfaced.
    assert!(row["mark_price_1e8"].is_null());
    assert!(row["notional_1e8"].is_null());
    assert!(row["unrealized_pnl_1e8"].is_null());
    assert!(row["maintenance_margin_requirement_1e8"].is_null());
    assert!(row["margin_ratio_bps"].is_null());
    assert!(row["estimated_liquidation_price_1e8"].is_string());
    assert_eq!(row["price_stale"].as_bool(), Some(true));
    assert_eq!(row["trading_enabled"].as_bool(), Some(false));
    assert_eq!(row["status"].as_str(), Some("open"));
    assert!(row["initial_margin_requirement_1e8"].is_string());
}

// =====================================================================
// C. Case-insensitive account lookup — the endpoint must return the
//    same row regardless of the URL-path address casing.
// =====================================================================

#[tokio::test]
async fn positions_endpoint_is_case_insensitive_on_account() {
    let state = state_with_perps_seeded();
    let cfg = state.perps_read_config.clone();
    let market = cfg.market_by_symbol("ETH-PERP").unwrap().clone();
    let account = AccountId::new("0x000000000000000000000000000000000000abcd".to_string());
    {
        let mut store = state.perp_positions_store.lock().unwrap();
        apply_perp_fill_for_account(
            &mut store,
            &market,
            PerpFillInput {
                account,
                subaccount_id: 1,
                market_id: "ETH-PERP".to_string(),
                side: PerpSide::Long,
                size_1e8: 100_000_000,
                price_1e8: 300_000_000_000,
                margin_1e8: 30_000_000_000,
            },
        )
        .unwrap();
    }
    let app = router(state);
    let response = app
        .oneshot(get(
            "/accounts/0x000000000000000000000000000000000000ABCD/perps/positions",
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    assert_eq!(body["positions"].as_array().map(|a| a.len()), Some(1));
}

// =====================================================================
// D. Perps mutation routes MUST remain fail-closed even after the
//    positions endpoint exists (regression pin).
// =====================================================================

#[tokio::test]
async fn perp_submit_order_still_fail_closed_after_positions_endpoint_exists() {
    let state = state_with_perps_seeded();
    let app = router(state);
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
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body = body_json(response).await;
    assert!(body["error"]
        .as_str()
        .unwrap_or("")
        .to_lowercase()
        .contains("perp"));
}

// =====================================================================
// E. `/markets` still filters perps (regression pin).
// =====================================================================

#[tokio::test]
async fn markets_still_filters_perps_after_positions_endpoint_exists() {
    let state = state_with_perps_seeded();
    let app = router(state);
    let response = app.oneshot(get("/markets")).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    for market in body.as_array().expect("markets array") {
        assert_ne!(market["kind"].as_str().unwrap_or(""), "perp");
    }
}

// =====================================================================
// F. Positions on a market that isn't in the config are hidden.
//    Test relies on the in-memory ledger accepting the seed even when
//    the market is absent — but the endpoint refuses to surface a row
//    it cannot enrich with risk fields.
// =====================================================================

#[tokio::test]
async fn positions_endpoint_hides_rows_whose_market_is_unconfigured() {
    let state = state_with_perps_seeded();
    // Seed a position via the direct store so the fill validator
    // doesn't reject the unknown symbol.
    let account = AccountId::new("0x000000000000000000000000000000000000FeeD".to_string());
    {
        let mut store = state.perp_positions_store.lock().unwrap();
        let position = deopt_v2_backend::perps::positions::new_position_skeleton(
            account.clone(),
            1,
            "SOL-PERP".to_string(), // absent from PerpsReadConfig::enabled_in_memory_for_tests()
            PerpSide::Long,
            100_000_000,
            100_000_000_000,
            10_000_000_000,
        );
        store.insert_open(position).unwrap();
    }
    let app = router(state);
    let response = app
        .oneshot(get(
            "/accounts/0x000000000000000000000000000000000000FeeD/perps/positions",
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    // The row exists internally but the endpoint filters it because
    // the market row isn't in cfg. This is intentional: the risk
    // parameters are unknowable, and surfacing a row with unknown
    // maintenance bps would risk implying trading is enabled.
    assert_eq!(body["positions"].as_array().map(|a| a.len()), Some(0));
}
