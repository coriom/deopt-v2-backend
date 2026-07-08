//! PERPS-PERSISTENCE-HISTORY-LIFECYCLE-V1 — integration tests for the
//! new read endpoints + lifecycle emission + rejection classifier.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use deopt_v2_backend::api::public_ws::{LifecycleChannel, LifecycleEvent, LifecyclePayload};
use deopt_v2_backend::api::{router, AppState};
use deopt_v2_backend::engine::EngineState;
use deopt_v2_backend::perps::{
    apply_perp_fill_for_account, classify_perp_rejection, emit_lifecycle_for_submit_outcome,
    emit_perp_rejection_lifecycle,
    price_reader::{InMemoryPerpOraclePriceReader, RawPriceRead},
    submit_perp_order_internal, PerpFillInput, PerpOrderSide, PerpSide, PerpTimeInForce,
    PerpsReadConfig, SubmitPerpOrderInput,
};
use deopt_v2_backend::types::{now_ms, AccountId};
use tower::ServiceExt;

const ONE: u128 = 100_000_000;
const PRICE_ETH_3000: u128 = 3000 * ONE;
const PRICE_ETH_3100: u128 = 3100 * ONE;
const MARGIN_10X_ETH: u128 = 300 * ONE;

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

fn fresh_price_reader() -> InMemoryPerpOraclePriceReader {
    InMemoryPerpOraclePriceReader::new().with_price(
        "ETH-PERP",
        RawPriceRead {
            price_1e8: PRICE_ETH_3000,
            updated_at_sec: (now_ms() / 1000) as u64,
            ok: true,
        },
    )
}

fn base_input(
    account: AccountId,
    side: PerpOrderSide,
    price: u128,
    size: u128,
) -> SubmitPerpOrderInput {
    SubmitPerpOrderInput {
        account,
        subaccount_id: 1,
        market_id: "ETH-PERP".to_string(),
        side,
        price_1e8: price,
        size_1e8: size,
        time_in_force: PerpTimeInForce::Gtc,
        post_only: false,
        reduce_only: false,
        isolated_margin_1e8: MARGIN_10X_ETH,
        client_order_id: None,
    }
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

fn drain(rx: &mut tokio::sync::broadcast::Receiver<LifecycleEvent>) -> Vec<LifecycleEvent> {
    let mut out = Vec::new();
    loop {
        match rx.try_recv() {
            Ok(ev) => out.push(ev),
            Err(_) => break,
        }
    }
    out
}

// =====================================================================
// A. Read endpoints — empty + populated
// =====================================================================

#[tokio::test]
async fn perps_account_orders_endpoint_returns_empty_by_default() {
    let app = router(state());
    let response = app
        .oneshot(get(
            "/accounts/0x000000000000000000000000000000000000abcd/perps/orders",
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    assert_eq!(body["orders"].as_array().map(|a| a.len()), Some(0));
    assert_eq!(body["chain_id"].as_u64(), Some(84532));
    assert_eq!(body["trading_enabled"].as_bool(), Some(false));
}

#[tokio::test]
async fn perps_account_fills_endpoint_returns_empty_by_default() {
    let app = router(state());
    let response = app
        .oneshot(get(
            "/accounts/0x000000000000000000000000000000000000abcd/perps/fills",
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    assert_eq!(body["fills"].as_array().map(|a| a.len()), Some(0));
    assert_eq!(body["chain_id"].as_u64(), Some(84532));
    assert_eq!(body["trading_enabled"].as_bool(), Some(false));
}

#[tokio::test]
async fn perps_account_orders_endpoint_surfaces_internal_ledger() {
    let state = state();
    let cfg = state.perps_read_config.clone();
    let reader = fresh_price_reader();
    let alice = addr("0x000000000000000000000000000000000000aaaa");
    {
        let mut orders = state.perp_order_store.lock().unwrap();
        let mut positions = state.perp_positions_store.lock().unwrap();
        submit_perp_order_internal(
            &cfg,
            &mut orders,
            &mut positions,
            &reader,
            base_input(alice.clone(), PerpOrderSide::Buy, PRICE_ETH_3000, ONE),
        )
        .await
        .unwrap();
    }
    let app = router(state);
    let response = app
        .oneshot(get(
            "/accounts/0x000000000000000000000000000000000000aaaa/perps/orders",
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    let orders = body["orders"].as_array().unwrap();
    assert_eq!(orders.len(), 1);
    let row = &orders[0];
    assert_eq!(row["market_id"].as_str(), Some("ETH-PERP"));
    assert_eq!(row["side"].as_str(), Some("buy"));
    assert_eq!(row["status"].as_str(), Some("open"));
    assert_eq!(row["price_1e8"].as_str(), Some("300000000000"));
    assert_eq!(row["trading_enabled"].as_bool(), Some(false));
}

#[tokio::test]
async fn perps_account_fills_endpoint_reflects_role_from_viewer_perspective() {
    let state = state();
    let cfg = state.perps_read_config.clone();
    let reader = fresh_price_reader();
    let alice = addr("0x000000000000000000000000000000000000aaaa");
    let bob = addr("0x000000000000000000000000000000000000bbbb");
    {
        let mut orders = state.perp_order_store.lock().unwrap();
        let mut positions = state.perp_positions_store.lock().unwrap();
        // Maker sells @ $3000
        submit_perp_order_internal(
            &cfg,
            &mut orders,
            &mut positions,
            &reader,
            base_input(alice.clone(), PerpOrderSide::Sell, PRICE_ETH_3000, ONE),
        )
        .await
        .unwrap();
        // Taker buys @ $3100 → fills at $3000
        submit_perp_order_internal(
            &cfg,
            &mut orders,
            &mut positions,
            &reader,
            base_input(bob.clone(), PerpOrderSide::Buy, PRICE_ETH_3100, ONE),
        )
        .await
        .unwrap();
    }
    let app = router(state);
    // Alice: maker, effective side = buy (opposite of taker's sell...)
    // Wait: taker is buy, so alice as maker was sell. viewer_side for
    // maker = taker.opposite() = sell. Let's verify.
    let response = app
        .clone()
        .oneshot(get(
            "/accounts/0x000000000000000000000000000000000000aaaa/perps/fills",
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    let fills = body["fills"].as_array().unwrap();
    assert_eq!(fills.len(), 1);
    let row = &fills[0];
    assert_eq!(row["liquidity_role"].as_str(), Some("maker"));
    assert_eq!(row["side"].as_str(), Some("sell"));
    // Bob's view.
    let response = app
        .oneshot(get(
            "/accounts/0x000000000000000000000000000000000000bbbb/perps/fills",
        ))
        .await
        .unwrap();
    let body = body_json(response).await;
    let fills = body["fills"].as_array().unwrap();
    assert_eq!(fills.len(), 1);
    let row = &fills[0];
    assert_eq!(row["liquidity_role"].as_str(), Some("taker"));
    assert_eq!(row["side"].as_str(), Some("buy"));
}

// =====================================================================
// B. Lifecycle emission — from internal execution service
// =====================================================================

#[tokio::test]
async fn submit_outcome_emits_order_fill_and_position_lifecycle_frames() {
    let state = state();
    let cfg = state.perps_read_config.clone();
    let reader = fresh_price_reader();
    let alice = addr("0x000000000000000000000000000000000000aaaa");
    let bob = addr("0x000000000000000000000000000000000000bbbb");
    // Maker sells.
    let _ = {
        let mut orders = state.perp_order_store.lock().unwrap();
        let mut positions = state.perp_positions_store.lock().unwrap();
        submit_perp_order_internal(
            &cfg,
            &mut orders,
            &mut positions,
            &reader,
            base_input(alice.clone(), PerpOrderSide::Sell, PRICE_ETH_3000, ONE),
        )
        .await
        .unwrap()
    };
    let mut rx = state.lifecycle_events.subscribe();
    // Taker buys → fills.
    let outcome = {
        let mut orders = state.perp_order_store.lock().unwrap();
        let mut positions = state.perp_positions_store.lock().unwrap();
        submit_perp_order_internal(
            &cfg,
            &mut orders,
            &mut positions,
            &reader,
            base_input(bob.clone(), PerpOrderSide::Buy, PRICE_ETH_3100, ONE),
        )
        .await
        .unwrap()
    };
    // Emit lifecycle for the taker outcome (this is what a future
    // public route will call after the submit succeeds).
    {
        let positions = state.perp_positions_store.lock().unwrap();
        emit_lifecycle_for_submit_outcome(&state.lifecycle_events, &positions, &outcome);
    }

    let events = drain(&mut rx);

    // We expect: 1 PerpOrderUpdated for taker + 2 PerpFillCreated
    // (one per side of the single fill) + 2 PerpPositionUpdated (taker + maker).
    let order_events: Vec<_> = events
        .iter()
        .filter(|e| matches!(e.payload, LifecyclePayload::PerpOrderUpdated { .. }))
        .collect();
    let fill_events: Vec<_> = events
        .iter()
        .filter(|e| matches!(e.payload, LifecyclePayload::PerpFillCreated { .. }))
        .collect();
    let position_events: Vec<_> = events
        .iter()
        .filter(|e| matches!(e.payload, LifecyclePayload::PerpPositionUpdated { .. }))
        .collect();

    assert_eq!(order_events.len(), 1);
    assert_eq!(fill_events.len(), 2);
    assert!(position_events.len() >= 2);

    // Channel routing.
    let order_event = order_events[0];
    assert!(matches!(
        order_event.channel,
        LifecycleChannel::AccountPerpOrders
    ));
    assert_eq!(order_event.account, bob); // taker

    // Fill routing — one to taker, one to maker.
    let fill_accounts: Vec<&AccountId> = fill_events.iter().map(|e| &e.account).collect();
    assert!(fill_accounts.iter().any(|a| **a == bob));
    assert!(fill_accounts.iter().any(|a| **a == alice));
    for e in &fill_events {
        assert!(matches!(e.channel, LifecycleChannel::AccountPerpFills));
    }

    // Position routing — one to taker, one to maker.
    let pos_accounts: Vec<&AccountId> = position_events.iter().map(|e| &e.account).collect();
    assert!(pos_accounts.iter().any(|a| **a == bob));
    assert!(pos_accounts.iter().any(|a| **a == alice));
    for e in &position_events {
        assert!(matches!(e.channel, LifecycleChannel::AccountPerpPositions));
    }
}

#[tokio::test]
async fn perp_lifecycle_frames_contain_no_secret_fields() {
    let state = state();
    let cfg = state.perps_read_config.clone();
    let reader = fresh_price_reader();
    let alice = addr("0x000000000000000000000000000000000000aaaa");
    let bob = addr("0x000000000000000000000000000000000000bbbb");
    let _ = {
        let mut orders = state.perp_order_store.lock().unwrap();
        let mut positions = state.perp_positions_store.lock().unwrap();
        submit_perp_order_internal(
            &cfg,
            &mut orders,
            &mut positions,
            &reader,
            base_input(alice.clone(), PerpOrderSide::Sell, PRICE_ETH_3000, ONE),
        )
        .await
        .unwrap()
    };
    let mut rx = state.lifecycle_events.subscribe();
    let outcome = {
        let mut orders = state.perp_order_store.lock().unwrap();
        let mut positions = state.perp_positions_store.lock().unwrap();
        submit_perp_order_internal(
            &cfg,
            &mut orders,
            &mut positions,
            &reader,
            base_input(bob.clone(), PerpOrderSide::Buy, PRICE_ETH_3100, ONE),
        )
        .await
        .unwrap()
    };
    {
        let positions = state.perp_positions_store.lock().unwrap();
        emit_lifecycle_for_submit_outcome(&state.lifecycle_events, &positions, &outcome);
    }
    let events = drain(&mut rx);
    for ev in &events {
        let json = serde_json::to_string(ev).unwrap();
        for banned in [
            "signature",
            "nonce",
            "envelope",
            "authorization",
            "bearer",
            "password",
        ] {
            assert!(
                !json.to_ascii_lowercase().contains(banned),
                "perp lifecycle frame contained banned field `{banned}`: {json}"
            );
        }
    }
}

// =====================================================================
// C. Rejection classifier + lifecycle emission
// =====================================================================

#[tokio::test]
async fn rejection_lifecycle_emits_for_post_only_would_match() {
    let state = state();
    let cfg = state.perps_read_config.clone();
    let reader = fresh_price_reader();
    let alice = addr("0x000000000000000000000000000000000000aaaa");
    let bob = addr("0x000000000000000000000000000000000000bbbb");
    // Maker sell so bob's post-only buy will cross.
    let _ = {
        let mut orders = state.perp_order_store.lock().unwrap();
        let mut positions = state.perp_positions_store.lock().unwrap();
        submit_perp_order_internal(
            &cfg,
            &mut orders,
            &mut positions,
            &reader,
            base_input(alice, PerpOrderSide::Sell, PRICE_ETH_3000, ONE),
        )
        .await
        .unwrap()
    };
    let mut rx = state.lifecycle_events.subscribe();
    let err = {
        let mut orders = state.perp_order_store.lock().unwrap();
        let mut positions = state.perp_positions_store.lock().unwrap();
        submit_perp_order_internal(
            &cfg,
            &mut orders,
            &mut positions,
            &reader,
            SubmitPerpOrderInput {
                post_only: true,
                ..base_input(bob.clone(), PerpOrderSide::Buy, PRICE_ETH_3100, ONE)
            },
        )
        .await
        .unwrap_err()
    };
    // Now emit the rejection frame explicitly (this is what a future
    // public route will do after the internal service returns Err).
    emit_perp_rejection_lifecycle(
        &state.lifecycle_events,
        &bob,
        1,
        &err,
        Some("ETH-PERP".to_string()),
        Some(PerpOrderSide::Buy),
        Some(PRICE_ETH_3100),
        Some(ONE),
        Some("gtc".to_string()),
        Some(true),
        Some(false),
        None,
    );

    let events = drain(&mut rx);
    let rejections: Vec<_> = events
        .iter()
        .filter(|e| matches!(e.payload, LifecyclePayload::PerpOrderRejected { .. }))
        .collect();
    assert_eq!(rejections.len(), 1);
    let ev = rejections[0];
    assert_eq!(ev.account, bob);
    assert!(matches!(ev.channel, LifecycleChannel::AccountPerpOrders));
    match &ev.payload {
        LifecyclePayload::PerpOrderRejected {
            market_id,
            side,
            reason_code,
            reason_source,
            reason_message,
            post_only,
            ..
        } => {
            assert_eq!(market_id.as_deref(), Some("ETH-PERP"));
            assert_eq!(side.as_deref(), Some("buy"));
            assert_eq!(reason_code, "post_only_would_match");
            assert_eq!(reason_source, "matching_policy");
            assert!(reason_message
                .as_deref()
                .unwrap_or("")
                .contains("post-only"));
            assert_eq!(post_only, &Some(true));
        }
        _ => unreachable!(),
    }
}

#[tokio::test]
async fn classify_perp_rejection_covers_all_recordable_variants() {
    // Sanity — every variant the internal execution can raise should
    // classify to a stable (code, source) pair.
    use deopt_v2_backend::error::BackendError;
    assert!(classify_perp_rejection(&BackendError::PerpZeroSize).is_some());
    assert!(classify_perp_rejection(&BackendError::PerpZeroPrice).is_some());
    assert!(classify_perp_rejection(&BackendError::PerpPostOnlyWouldMatch).is_some());
    assert!(classify_perp_rejection(&BackendError::PerpFokNotFillable).is_some());
    assert!(classify_perp_rejection(&BackendError::PerpReduceOnlyViolation).is_some());
    assert!(classify_perp_rejection(&BackendError::PerpSelfTrade).is_some());
    assert!(classify_perp_rejection(&BackendError::PerpPositionFlip).is_some());
    assert!(classify_perp_rejection(&BackendError::PerpsMarketNotFound("X".to_string())).is_some());
    assert!(
        classify_perp_rejection(&BackendError::PerpMarkPriceUnavailable("stale".to_string()))
            .is_some()
    );
    assert!(
        classify_perp_rejection(&BackendError::PerpDuplicateClientOrderId("c1".to_string()))
            .is_some()
    );
    assert!(
        classify_perp_rejection(&BackendError::PerpInvalidTifCombination(
            "po+ioc".to_string()
        ))
        .is_some()
    );
    // Silence unused-import warnings from apply/fill imports the
    // integration file pulls in for symmetry with other perp test
    // files.
    let mut store = deopt_v2_backend::perps::PerpPositionsStore::new();
    let market = PerpsReadConfig::enabled_in_memory_for_tests().markets[0].clone();
    let _ = apply_perp_fill_for_account(
        &mut store,
        &market,
        PerpFillInput {
            account: addr("0x000000000000000000000000000000000000aaaa"),
            subaccount_id: 1,
            market_id: "ETH-PERP".to_string(),
            side: PerpSide::Long,
            size_1e8: ONE,
            price_1e8: PRICE_ETH_3000,
            margin_1e8: MARGIN_10X_ETH,
        },
    );
}

// =====================================================================
// D. Fail-closed regression pin
// =====================================================================

#[tokio::test]
async fn public_perp_submit_still_fail_closed_after_history_endpoints_ship() {
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
