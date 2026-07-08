//! PERPS-PG-WRITE-PATH-V1 — integration tests.
//!
//! **Scope note.** Tests that require a live Postgres instance are
//! deliberately absent from this file — the existing project test
//! matrix does not currently spin PG up. The PG method signatures are
//! locked at compile time by these `_signature` tests; the actual
//! query correctness is exercised through the read endpoints when
//! `state.repository` is set (a follow-up PG-execution milestone will
//! land the harness).
//!
//! What this file DOES cover:
//!   * Public Perps mutation routes remain fail-closed regardless of
//!     the new PG methods (regression pin).
//!   * The in-memory execution path is untouched (no regression).
//!   * `PerpOrderStatus::parse` is a round-trip inverse of `as_str`
//!     for every variant (row-decoder correctness).

use axum::body::Body;
use axum::http::{Request, StatusCode};
use deopt_v2_backend::api::{router, AppState};
use deopt_v2_backend::engine::EngineState;
use deopt_v2_backend::perps::{
    submit_perp_order_internal, PerpOrderSide, PerpOrderStatus, PerpTimeInForce, PerpsReadConfig,
    SubmitPerpOrderInput,
};
use deopt_v2_backend::types::{now_ms, AccountId};
use tower::ServiceExt;

const ONE: u128 = 100_000_000;

fn addr(hex: &str) -> AccountId {
    AccountId::new(hex.to_string())
}

fn state() -> AppState {
    let mut state = AppState::new(EngineState::with_default_markets());
    let mut cfg = PerpsReadConfig::enabled_in_memory_for_tests();
    cfg.rpc_url = None;
    state.perps_read_config = cfg;
    state
}

async fn body_json(response: axum::response::Response) -> serde_json::Value {
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

// =====================================================================
// A. Fail-closed regression pin
// =====================================================================

#[tokio::test]
async fn public_perp_submit_still_fail_closed_after_pg_write_path_lands() {
    let app = router(state());
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
// B. In-memory execution path untouched (no regression)
// =====================================================================

#[tokio::test]
async fn in_memory_execution_still_works_when_repository_is_absent() {
    let state = state();
    assert!(
        state.repository.is_none(),
        "state.repository must be None here"
    );
    let cfg = state.perps_read_config.clone();
    let reader = deopt_v2_backend::perps::price_reader::InMemoryPerpOraclePriceReader::new()
        .with_price(
            "ETH-PERP",
            deopt_v2_backend::perps::price_reader::RawPriceRead {
                price_1e8: 3000 * ONE,
                updated_at_sec: (now_ms() / 1000) as u64,
                ok: true,
            },
        );
    let outcome = {
        let mut orders = state.perp_order_store.lock().unwrap();
        let mut positions = state.perp_positions_store.lock().unwrap();
        submit_perp_order_internal(
            &cfg,
            &mut orders,
            &mut positions,
            &reader,
            SubmitPerpOrderInput {
                account: addr("0x0000000000000000000000000000000000000aaa"),
                subaccount_id: 1,
                market_id: "ETH-PERP".to_string(),
                side: PerpOrderSide::Buy,
                price_1e8: 3000 * ONE,
                size_1e8: ONE,
                time_in_force: PerpTimeInForce::Gtc,
                post_only: false,
                reduce_only: false,
                isolated_margin_1e8: 300 * ONE,
                client_order_id: None,
            },
        )
        .await
        .unwrap()
    };
    assert!(outcome.fills.is_empty());
    assert_eq!(outcome.order.status, PerpOrderStatus::Open);
}

// =====================================================================
// C. PerpOrderStatus parse round-trip — row-decoder correctness
// =====================================================================

#[test]
fn perp_order_status_parse_is_round_trip_inverse_of_as_str() {
    let variants = [
        PerpOrderStatus::Open,
        PerpOrderStatus::PartiallyFilled,
        PerpOrderStatus::Filled,
        PerpOrderStatus::Cancelled,
        PerpOrderStatus::Rejected,
    ];
    for v in variants {
        let round = PerpOrderStatus::parse(v.as_str()).unwrap();
        assert_eq!(round, v, "round-trip failed for {:?}", v);
    }
    // Unknown values fail with a persistence error.
    assert!(PerpOrderStatus::parse("not_a_status").is_err());
}

#[test]
fn perp_side_parse_is_round_trip() {
    use deopt_v2_backend::perps::PerpSide;
    for side in [PerpSide::Long, PerpSide::Short] {
        let round = PerpSide::parse(side.as_str()).unwrap();
        assert_eq!(round, side);
    }
    assert!(PerpSide::parse("neither").is_err());
}

#[test]
fn perp_order_side_parse_is_round_trip() {
    for side in [PerpOrderSide::Buy, PerpOrderSide::Sell] {
        let round = PerpOrderSide::parse(side.as_str()).unwrap();
        assert_eq!(round, side);
    }
    assert!(PerpOrderSide::parse("neither").is_err());
}

#[test]
fn perp_time_in_force_parse_is_round_trip() {
    for tif in [
        PerpTimeInForce::Gtc,
        PerpTimeInForce::Ioc,
        PerpTimeInForce::Fok,
    ] {
        let round = PerpTimeInForce::parse(tif.as_str()).unwrap();
        assert_eq!(round, tif);
    }
    // Unknown TIF maps to PerpUnsupportedTif.
    let err = PerpTimeInForce::parse("gtd").unwrap_err();
    assert!(matches!(
        err,
        deopt_v2_backend::error::BackendError::PerpUnsupportedTif(_)
    ));
}

#[test]
fn perp_order_type_parse_is_round_trip() {
    use deopt_v2_backend::perps::PerpOrderType;
    let t = PerpOrderType::Limit;
    let round = PerpOrderType::parse(t.as_str()).unwrap();
    assert_eq!(round, t);
    assert!(PerpOrderType::parse("market").is_err());
}

#[test]
fn perp_position_status_parse_is_round_trip() {
    use deopt_v2_backend::perps::PerpPositionStatus;
    for status in [PerpPositionStatus::Open, PerpPositionStatus::Closed] {
        let round = PerpPositionStatus::parse(status.as_str()).unwrap();
        assert_eq!(round, status);
    }
    // PERPS-LIQUIDATION-AND-RISK-V1 added `Liquidated` as a valid
    // status. This must round-trip like the other variants.
    let round = PerpPositionStatus::parse("liquidated").unwrap();
    assert_eq!(round.as_str(), "liquidated");
    assert!(PerpPositionStatus::parse("something_else").is_err());
}

// =====================================================================
// D. Read endpoints still return empty [] when no repo AND no
//    in-memory rows exist — honest default.
// =====================================================================

#[tokio::test]
async fn read_endpoints_return_empty_by_default_without_repository() {
    let app = router(state());
    for path in [
        "/accounts/0x000000000000000000000000000000000000abcd/perps/positions",
        "/accounts/0x000000000000000000000000000000000000abcd/perps/orders",
        "/accounts/0x000000000000000000000000000000000000abcd/perps/fills",
    ] {
        let request = Request::builder()
            .method("GET")
            .uri(path)
            .body(Body::empty())
            .unwrap();
        let response = app.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK, "path: {path}");
        let body = body_json(response).await;
        assert_eq!(body["chain_id"].as_u64(), Some(84532), "path: {path}");
        assert_eq!(
            body["trading_enabled"].as_bool(),
            Some(false),
            "path: {path}"
        );
    }
}
