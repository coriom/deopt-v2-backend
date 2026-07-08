//! PERPS-SUBACCOUNTS-ENGINE-ROUTING-V1 — engine-level isolation tests.
//!
//! These integration tests exercise the deep engine ripple: position
//! store keyed by `(account, subaccount_id, market)`, fill applier
//! threading subaccount_id, funding + liquidation events carrying
//! subaccount metadata, and read endpoints filtering by subaccount.
//!
//! The Perps public trading gate remains fail-closed by default in
//! every test; these tests exercise the internal engine directly (via
//! `apply_perp_fill_for_account`) and the read HTTP routes without
//! ever flipping `PERPS_PUBLIC_TRADING_ENABLED`.

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use deopt_v2_backend::api::{router, AppState};
use deopt_v2_backend::engine::EngineState;
use deopt_v2_backend::options::OptionsConfig;
use deopt_v2_backend::perps::{
    apply_perp_fill_for_account, PerpFillInput, PerpSide, PerpsReadConfig,
};
use deopt_v2_backend::types::AccountId;
use tower::ServiceExt;

const ONE: u128 = 100_000_000;
const PRICE_ETH_3000: u128 = 300_000_000_000;
const MARGIN_10X_ETH: u128 = 300 * ONE;

fn addr(hex: &str) -> AccountId {
    AccountId::new(hex.to_string())
}

fn state() -> AppState {
    let cfg = PerpsReadConfig::enabled_in_memory_for_tests();
    let mut state = AppState::with_options_config(
        EngineState::with_default_markets(),
        OptionsConfig::disabled(),
    );
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
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

// =====================================================================
// PART 1 — Position store isolation across subaccounts
// =====================================================================

#[test]
fn account_1_and_account_2_positions_do_not_net() {
    // Same wallet, same market, opposite sides on subaccount 1 vs 2.
    // The store must permit both to coexist and MUST NOT net them.
    let state = state();
    let market = state.perps_read_config.markets[0].clone();
    let account = addr("0x0000000000000000000000000000000000000aaa");
    let mut store = state.perp_positions_store.lock().unwrap();

    apply_perp_fill_for_account(
        &mut store,
        &market,
        PerpFillInput {
            account: account.clone(),
            subaccount_id: 1,
            market_id: "ETH-PERP".to_string(),
            side: PerpSide::Long,
            size_1e8: ONE,
            price_1e8: PRICE_ETH_3000,
            margin_1e8: MARGIN_10X_ETH,
        },
    )
    .unwrap();

    apply_perp_fill_for_account(
        &mut store,
        &market,
        PerpFillInput {
            account: account.clone(),
            subaccount_id: 2,
            market_id: "ETH-PERP".to_string(),
            side: PerpSide::Short,
            size_1e8: ONE,
            price_1e8: PRICE_ETH_3000,
            margin_1e8: MARGIN_10X_ETH,
        },
    )
    .unwrap();

    let acc1 = store.get_active(&account, 1, "ETH-PERP").unwrap();
    let acc2 = store.get_active(&account, 2, "ETH-PERP").unwrap();
    // The two positions coexist independently, with distinct sides.
    assert_eq!(acc1.side, PerpSide::Long);
    assert_eq!(acc1.subaccount_id, 1);
    assert_eq!(acc1.size_1e8, ONE);
    assert_eq!(acc2.side, PerpSide::Short);
    assert_eq!(acc2.subaccount_id, 2);
    assert_eq!(acc2.size_1e8, ONE);
}

#[test]
fn account_2_reduce_does_not_reduce_account_1() {
    let state = state();
    let market = state.perps_read_config.markets[0].clone();
    let account = addr("0x0000000000000000000000000000000000000aaa");
    let mut store = state.perp_positions_store.lock().unwrap();

    // Long positions on both subaccounts.
    for sub in [1u32, 2u32] {
        apply_perp_fill_for_account(
            &mut store,
            &market,
            PerpFillInput {
                account: account.clone(),
                subaccount_id: sub,
                market_id: "ETH-PERP".to_string(),
                side: PerpSide::Long,
                size_1e8: 2 * ONE,
                price_1e8: PRICE_ETH_3000,
                margin_1e8: 600 * ONE,
            },
        )
        .unwrap();
    }

    // Reduce Account 2 by half via an opposite-side fill.
    apply_perp_fill_for_account(
        &mut store,
        &market,
        PerpFillInput {
            account: account.clone(),
            subaccount_id: 2,
            market_id: "ETH-PERP".to_string(),
            side: PerpSide::Short,
            size_1e8: ONE,
            price_1e8: PRICE_ETH_3000,
            margin_1e8: 0,
        },
    )
    .unwrap();

    let a1 = store.get_active(&account, 1, "ETH-PERP").unwrap();
    let a2 = store.get_active(&account, 2, "ETH-PERP").unwrap();
    // Account 1 unchanged (still 2 ETH).
    assert_eq!(a1.size_1e8, 2 * ONE);
    // Account 2 halved.
    assert_eq!(a2.size_1e8, ONE);
}

// =====================================================================
// PART 2 — Read endpoints filter by subaccount
// =====================================================================

#[tokio::test]
async fn positions_read_default_returns_account_1_only() {
    let state = state();
    let market = state.perps_read_config.markets[0].clone();
    let account = addr("0x0000000000000000000000000000000000000aaa");
    {
        let mut store = state.perp_positions_store.lock().unwrap();
        for sub in [1u32, 2u32, 3u32] {
            apply_perp_fill_for_account(
                &mut store,
                &market,
                PerpFillInput {
                    account: account.clone(),
                    subaccount_id: sub,
                    market_id: "ETH-PERP".to_string(),
                    side: PerpSide::Long,
                    size_1e8: ONE,
                    price_1e8: PRICE_ETH_3000,
                    margin_1e8: MARGIN_10X_ETH,
                },
            )
            .unwrap();
        }
    }
    let app = router(state);
    let response = app
        .clone()
        .oneshot(get(
            "/accounts/0x0000000000000000000000000000000000000aaa/perps/positions",
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let json = body_json(response).await;
    let positions = json["positions"].as_array().unwrap();
    // Default (no query params) filters to subaccount 1.
    assert_eq!(positions.len(), 1);
    assert_eq!(positions[0]["subaccount_id"], 1);
}

#[tokio::test]
async fn positions_read_subaccount_2_returns_account_2_only() {
    let state = state();
    let market = state.perps_read_config.markets[0].clone();
    let account = addr("0x0000000000000000000000000000000000000aaa");
    {
        let mut store = state.perp_positions_store.lock().unwrap();
        for sub in [1u32, 2u32, 3u32] {
            apply_perp_fill_for_account(
                &mut store,
                &market,
                PerpFillInput {
                    account: account.clone(),
                    subaccount_id: sub,
                    market_id: "ETH-PERP".to_string(),
                    side: PerpSide::Long,
                    size_1e8: ONE,
                    price_1e8: PRICE_ETH_3000,
                    margin_1e8: MARGIN_10X_ETH,
                },
            )
            .unwrap();
        }
    }
    let app = router(state);
    let response = app
        .clone()
        .oneshot(get(
            "/accounts/0x0000000000000000000000000000000000000aaa/perps/positions?subaccount_id=2",
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let json = body_json(response).await;
    let positions = json["positions"].as_array().unwrap();
    assert_eq!(positions.len(), 1);
    assert_eq!(positions[0]["subaccount_id"], 2);
}

#[tokio::test]
async fn positions_read_all_true_returns_every_subaccount() {
    let state = state();
    let market = state.perps_read_config.markets[0].clone();
    let account = addr("0x0000000000000000000000000000000000000aaa");
    {
        let mut store = state.perp_positions_store.lock().unwrap();
        for sub in [1u32, 2u32, 3u32] {
            apply_perp_fill_for_account(
                &mut store,
                &market,
                PerpFillInput {
                    account: account.clone(),
                    subaccount_id: sub,
                    market_id: "ETH-PERP".to_string(),
                    side: PerpSide::Long,
                    size_1e8: ONE,
                    price_1e8: PRICE_ETH_3000,
                    margin_1e8: MARGIN_10X_ETH,
                },
            )
            .unwrap();
        }
    }
    let app = router(state);
    let response = app
        .clone()
        .oneshot(get(
            "/accounts/0x0000000000000000000000000000000000000aaa/perps/positions?all=true",
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let json = body_json(response).await;
    let positions = json["positions"].as_array().unwrap();
    assert_eq!(positions.len(), 3);
    let mut subaccount_ids: Vec<u64> = positions
        .iter()
        .map(|p| p["subaccount_id"].as_u64().unwrap())
        .collect();
    subaccount_ids.sort();
    assert_eq!(subaccount_ids, vec![1, 2, 3]);
}

// =====================================================================
// PART 3 — Public Perps mutation surface stays fail-closed regardless
// of what closed-test / subaccount plumbing lands.
// =====================================================================

#[tokio::test]
async fn public_perps_submit_still_503_after_engine_ripple() {
    let state = state();
    // Default AppState: perps_public_trading_enabled=false,
    // perps_closed_test_enabled=false. Every mutation route MUST
    // return 503 regardless of the subaccount_id supplied.
    let app = router(state);
    let request = Request::builder()
        .method("POST")
        .uri("/perps/orders")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::json!({
                "market_id": "ETH-PERP",
                "account": "0x00000000000000000000000000000000000000aa",
                "side": "buy",
                "price_1e8": "300000000000",
                "size_1e8": "100000000",
                "time_in_force": "gtc",
                "post_only": false,
                "reduce_only": false,
                "isolated_margin_1e8": "30000000000",
                "subaccount_id": 2,
            })
            .to_string(),
        ))
        .unwrap();
    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn public_perps_cancel_still_503_after_engine_ripple() {
    let state = state();
    let app = router(state);
    let request = Request::builder()
        .method("DELETE")
        .uri("/perps/orders/11111111-2222-3333-4444-555555555555?account=0x00000000000000000000000000000000000000aa")
        .body(Body::empty())
        .unwrap();
    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
}

// =====================================================================
// PART 4 — Closed-test allowlist gate is invoked at the mutation route
// entry, even though the default posture is fail-closed via the
// public trading flag.
// =====================================================================

#[tokio::test]
async fn public_perps_submit_still_503_when_closed_test_on_and_non_allowlisted() {
    // PERPS-V2-WRITE-AUTH-ENFORCEMENT-V1 — closed-test mode opens the
    // path for allowlisted callers (progressing to auth verification);
    // non-allowlisted callers still see 503. This asserts the
    // non-allowlisted branch under the new gate.
    let mut state = state();
    state.perps_closed_test_enabled = true;
    // Allowlist contains only `0xaa`; the caller below is `0xbb`.
    state.perps_closed_test_allowlist = vec![addr("0x00000000000000000000000000000000000000aa")];
    let app = router(state);
    let request = Request::builder()
        .method("POST")
        .uri("/perps/orders")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::json!({
                "market_id": "ETH-PERP",
                "account": "0x00000000000000000000000000000000000000bb",
                "side": "buy",
                "price_1e8": "300000000000",
                "size_1e8": "100000000",
                "time_in_force": "gtc",
                "post_only": false,
                "reduce_only": false,
                "isolated_margin_1e8": "30000000000",
                "subaccount_id": 1,
            })
            .to_string(),
        ))
        .unwrap();
    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
}

// =====================================================================
// PART 5 — WS payloads carry subaccount metadata (wire-shape freeze via
// JSON serialization).
// =====================================================================

#[test]
fn perp_order_updated_ws_payload_serializes_subaccount_id() {
    use deopt_v2_backend::api::public_ws::LifecyclePayload;
    let payload = LifecyclePayload::PerpOrderUpdated {
        order_id: "abc".to_string(),
        market_id: "ETH-PERP".to_string(),
        subaccount_id: 7,
        side: "buy".to_string(),
        status: "open".to_string(),
        price_1e8: "0".to_string(),
        size_1e8: "0".to_string(),
        remaining_size_1e8: "0".to_string(),
        filled_size_1e8: "0".to_string(),
        time_in_force: "gtc".to_string(),
        post_only: false,
        reduce_only: false,
        client_order_id: None,
        terminal_reason_code: None,
        updated_at_ms: 0,
    };
    let json = serde_json::to_value(&payload).unwrap();
    assert_eq!(json["subaccount_id"], 7);
}

#[test]
fn perp_fill_created_ws_payload_serializes_two_sided_subaccounts() {
    use deopt_v2_backend::api::public_ws::LifecyclePayload;
    let payload = LifecyclePayload::PerpFillCreated {
        fill_id: "abc".to_string(),
        market_id: "ETH-PERP".to_string(),
        order_id: "o1".to_string(),
        counterparty_order_id: "o2".to_string(),
        taker_subaccount_id: 3,
        maker_subaccount_id: 5,
        liquidity_role: "taker".to_string(),
        side: "buy".to_string(),
        price_1e8: "0".to_string(),
        size_1e8: "0".to_string(),
        created_at_ms: 0,
    };
    let json = serde_json::to_value(&payload).unwrap();
    assert_eq!(json["taker_subaccount_id"], 3);
    assert_eq!(json["maker_subaccount_id"], 5);
}

#[test]
fn perp_position_updated_ws_payload_serializes_subaccount_id() {
    use deopt_v2_backend::api::public_ws::LifecyclePayload;
    let payload = LifecyclePayload::PerpPositionUpdated {
        position_id: "abc".to_string(),
        market_id: "ETH-PERP".to_string(),
        subaccount_id: 4,
        side: "long".to_string(),
        size_1e8: "0".to_string(),
        entry_price_1e8: "0".to_string(),
        margin_1e8: "0".to_string(),
        realized_pnl_1e8: "0".to_string(),
        status: "open".to_string(),
        updated_at_ms: 0,
    };
    let json = serde_json::to_value(&payload).unwrap();
    assert_eq!(json["subaccount_id"], 4);
}

#[test]
fn perp_position_liquidated_ws_payload_serializes_subaccount_id() {
    use deopt_v2_backend::api::public_ws::LifecyclePayload;
    let payload = LifecyclePayload::PerpPositionLiquidated {
        liquidation_id: "abc".to_string(),
        market_id: "ETH-PERP".to_string(),
        subaccount_id: 8,
        position_id: "p1".to_string(),
        side: "long".to_string(),
        size_1e8: "0".to_string(),
        mark_price_1e8: "0".to_string(),
        realized_pnl_1e8: "0".to_string(),
        bad_debt_1e8: "0".to_string(),
        liquidation_fee_1e8: "0".to_string(),
        reason_code: "margin_breach".to_string(),
        created_at_ms: 0,
    };
    let json = serde_json::to_value(&payload).unwrap();
    assert_eq!(json["subaccount_id"], 8);
}

#[test]
fn perp_funding_payment_ws_payload_serializes_subaccount_id() {
    use deopt_v2_backend::api::public_ws::LifecyclePayload;
    let payload = LifecyclePayload::PerpFundingPaymentCreated {
        funding_event_id: "abc".to_string(),
        market_id: "ETH-PERP".to_string(),
        subaccount_id: 6,
        position_id: "p1".to_string(),
        side: "long".to_string(),
        position_size_1e8: "0".to_string(),
        funding_index_before_1e18: "0".to_string(),
        funding_index_after_1e18: "0".to_string(),
        funding_delta_1e18: "0".to_string(),
        payment_1e8: "0".to_string(),
        margin_before_1e8: "0".to_string(),
        margin_after_1e8: "0".to_string(),
        bad_debt_1e8: "0".to_string(),
        reason_code: "funding_settlement".to_string(),
        created_at_ms: 0,
    };
    let json = serde_json::to_value(&payload).unwrap();
    assert_eq!(json["subaccount_id"], 6);
}

// =====================================================================
// PART 6 — Wire-compat: legacy payloads that omit subaccount_id
// deserialise with the default `1` via serde default helper.
// =====================================================================

#[test]
fn legacy_perp_order_updated_payload_defaults_to_subaccount_1() {
    use deopt_v2_backend::api::public_ws::LifecyclePayload;
    // JSON without any subaccount_id field — mirrors what a pre-
    // milestone client would send / an old test fixture.
    let raw = serde_json::json!({
        "type": "perp_order_updated",
        "order_id": "abc",
        "market_id": "ETH-PERP",
        "side": "buy",
        "status": "open",
        "price_1e8": "0",
        "size_1e8": "0",
        "remaining_size_1e8": "0",
        "filled_size_1e8": "0",
        "time_in_force": "gtc",
        "post_only": false,
        "reduce_only": false,
        "client_order_id": null,
        "terminal_reason_code": null,
        "updated_at_ms": 0,
    });
    let payload: LifecyclePayload = serde_json::from_value(raw).expect("legacy shape parses");
    match payload {
        LifecyclePayload::PerpOrderUpdated { subaccount_id, .. } => {
            assert_eq!(subaccount_id, 1);
        }
        other => panic!("unexpected variant: {other:?}"),
    }
}

#[test]
fn legacy_perp_fill_created_payload_defaults_both_sides_to_subaccount_1() {
    use deopt_v2_backend::api::public_ws::LifecyclePayload;
    let raw = serde_json::json!({
        "type": "perp_fill_created",
        "fill_id": "abc",
        "market_id": "ETH-PERP",
        "order_id": "o1",
        "counterparty_order_id": "o2",
        "liquidity_role": "taker",
        "side": "buy",
        "price_1e8": "0",
        "size_1e8": "0",
        "created_at_ms": 0,
    });
    let payload: LifecyclePayload = serde_json::from_value(raw).expect("legacy shape parses");
    match payload {
        LifecyclePayload::PerpFillCreated {
            taker_subaccount_id,
            maker_subaccount_id,
            ..
        } => {
            assert_eq!(taker_subaccount_id, 1);
            assert_eq!(maker_subaccount_id, 1);
        }
        other => panic!("unexpected variant: {other:?}"),
    }
}

// =====================================================================
// PART 7 — No response body or WS payload leaks secrets. Since the
// scaffolding milestone's byte-freeze suite already covers write-auth
// canonicals, here we spot-check the WS payload set for absence of
// signature / nonce / auth envelope fields on the emitted JSON.
// =====================================================================

#[test]
fn perp_ws_payload_never_carries_auth_material() {
    use deopt_v2_backend::api::public_ws::LifecyclePayload;
    let payload = LifecyclePayload::PerpFillCreated {
        fill_id: "abc".to_string(),
        market_id: "ETH-PERP".to_string(),
        order_id: "o1".to_string(),
        counterparty_order_id: "o2".to_string(),
        taker_subaccount_id: 1,
        maker_subaccount_id: 1,
        liquidity_role: "taker".to_string(),
        side: "buy".to_string(),
        price_1e8: "0".to_string(),
        size_1e8: "0".to_string(),
        created_at_ms: 0,
    };
    let json_text = serde_json::to_string(&payload).unwrap();
    for banned in &[
        "signature",
        "signed",
        "nonce",
        "authorization",
        "eip712",
        "envelope",
        "private_key",
        "secret",
        "bearer",
        "cookie",
    ] {
        assert!(
            !json_text.to_lowercase().contains(banned),
            "WS payload leaked banned term `{banned}`: {json_text}"
        );
    }
}
