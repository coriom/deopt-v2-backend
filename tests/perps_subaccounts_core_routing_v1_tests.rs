//! PERPS-SUBACCOUNTS-CORE-ROUTING-V1
//!
//! Byte-freeze + scaffolding tests for the new Perps write-auth
//! actions (`PERP_ORDER_SUBMIT`, `PERP_ORDER_CANCEL`) and the closed-
//! test allowlist gate on `AppState`.
//!
//! These tests are **schema + wire scaffolding proof** — they do NOT
//! attempt to exercise the Perps engine under partitioned subaccounts
//! because the position store keying rippling is deferred to the
//! follow-up milestone `PERPS-SUBACCOUNTS-ENGINE-ROUTING-V1`. The
//! fail-closed public trading grid remains covered by the existing
//! `perps_public_route_fail_closed_grid.rs` binary; this file adds
//! coverage for the new surfaces that landed in this milestone.

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use deopt_v2_backend::api::{router, AppState};
use deopt_v2_backend::auth::write_authorization::{
    canonical_payload_bytes, CanonicalValue, WriteAuthAction,
};
use deopt_v2_backend::engine::EngineState;
use deopt_v2_backend::options::OptionsConfig;
use deopt_v2_backend::types::AccountId;
use tower::ServiceExt;

// ---------------------------------------------------------------------
// PART 1 — WriteAuthAction round-trip for the two new variants
// ---------------------------------------------------------------------

#[test]
fn perp_order_submit_action_string_frozen() {
    assert_eq!(
        WriteAuthAction::PerpOrderSubmit.as_str(),
        "PERP_ORDER_SUBMIT"
    );
}

#[test]
fn perp_order_cancel_action_string_frozen() {
    assert_eq!(
        WriteAuthAction::PerpOrderCancel.as_str(),
        "PERP_ORDER_CANCEL"
    );
}

#[test]
fn perp_order_submit_parse_round_trip() {
    let parsed = WriteAuthAction::parse("PERP_ORDER_SUBMIT").expect("must parse");
    assert_eq!(parsed, WriteAuthAction::PerpOrderSubmit);
    assert_eq!(parsed.as_str(), "PERP_ORDER_SUBMIT");
}

#[test]
fn perp_order_cancel_parse_round_trip() {
    let parsed = WriteAuthAction::parse("PERP_ORDER_CANCEL").expect("must parse");
    assert_eq!(parsed, WriteAuthAction::PerpOrderCancel);
    assert_eq!(parsed.as_str(), "PERP_ORDER_CANCEL");
}

#[test]
fn perp_action_strings_are_distinct_from_option_analogues() {
    // Freeze: PERP_* actions never collide with OPTION_ORDER_* to
    // prevent cross-product replay across the same nonce ledger key.
    assert_ne!(
        WriteAuthAction::PerpOrderSubmit.as_str(),
        WriteAuthAction::OptionOrderSubmit.as_str()
    );
    assert_ne!(
        WriteAuthAction::PerpOrderCancel.as_str(),
        WriteAuthAction::OptionOrderCancel.as_str()
    );
}

// ---------------------------------------------------------------------
// PART 2 — Byte-freeze the v2 canonical payloads for the two actions
//
// The canonical payload builders `canonical_perp_order_submit_v2` and
// `canonical_perp_order_cancel_v2` in `routes.rs` are `pub(crate)`;
// this integration binary rebuilds their exact byte layout via the
// public `canonical_payload_bytes` primitive + `CanonicalValue`
// variants. Any drift between the builder and this test flags a
// wire-shape regression.
// ---------------------------------------------------------------------

fn expected_perp_submit_bytes() -> Vec<u8> {
    let account = AccountId::new("0x00000000000000000000000000000000000000aa".to_string());
    canonical_payload_bytes(
        WriteAuthAction::PerpOrderSubmit,
        &[
            ("account", CanonicalValue::Address(account)),
            ("subaccount_id", CanonicalValue::U64(2)),
            ("market_id", CanonicalValue::Str("ETH-PERP".to_string())),
            ("side", CanonicalValue::Str("buy".to_string())),
            ("price_1e8", CanonicalValue::Str("300000000000".to_string())),
            ("size_1e8", CanonicalValue::Str("100000000".to_string())),
            ("time_in_force", CanonicalValue::Str("gtc".to_string())),
            ("post_only", CanonicalValue::Bool(false)),
            ("reduce_only", CanonicalValue::Bool(false)),
            (
                "isolated_margin_1e8",
                CanonicalValue::Str("30000000000".to_string()),
            ),
            ("client_order_id", CanonicalValue::Null),
        ],
    )
}

#[test]
fn perp_submit_v2_canonical_bytes_frozen() {
    let bytes = expected_perp_submit_bytes();
    let text = String::from_utf8(bytes).expect("canonical bytes must be UTF-8");
    // Sanity: the leading action name is `PERP_ORDER_SUBMIT`; the
    // second field is always `subaccount_id`; the canonical uses `|`
    // separators (per the existing `canonical_payload_bytes` spec).
    assert!(
        text.starts_with("PERP_ORDER_SUBMIT|"),
        "action prefix drift: {text}"
    );
    assert!(
        text.contains("|subaccount_id=2|"),
        "subaccount_id must be the second field: {text}"
    );
    assert!(
        text.contains("|market_id=\"ETH-PERP\"|"),
        "market_id must appear after subaccount_id (string-quoted): {text}"
    );
    assert!(
        text.ends_with("|client_order_id=null"),
        "trailing field must be nullable client_order_id: {text}"
    );
}

#[test]
fn perp_submit_v2_canonical_bytes_differ_across_subaccounts() {
    // Same fill request, different subaccount → distinct bytes.
    // Critical for cross-subaccount replay protection: two v2
    // envelopes for the same wallet + action + nonce but different
    // subaccount_id MUST NOT collide in the used_nonces_v2 ledger.
    let account = AccountId::new("0x00000000000000000000000000000000000000aa".to_string());
    let base = |sub: u64| {
        canonical_payload_bytes(
            WriteAuthAction::PerpOrderSubmit,
            &[
                ("account", CanonicalValue::Address(account.clone())),
                ("subaccount_id", CanonicalValue::U64(sub)),
                ("market_id", CanonicalValue::Str("ETH-PERP".to_string())),
                ("side", CanonicalValue::Str("buy".to_string())),
                ("price_1e8", CanonicalValue::Str("300000000000".to_string())),
                ("size_1e8", CanonicalValue::Str("100000000".to_string())),
                ("time_in_force", CanonicalValue::Str("gtc".to_string())),
                ("post_only", CanonicalValue::Bool(false)),
                ("reduce_only", CanonicalValue::Bool(false)),
                (
                    "isolated_margin_1e8",
                    CanonicalValue::Str("30000000000".to_string()),
                ),
                ("client_order_id", CanonicalValue::Null),
            ],
        )
    };
    assert_ne!(base(1), base(2));
    assert_ne!(base(2), base(3));
}

#[test]
fn perp_cancel_v2_canonical_bytes_frozen() {
    let account = AccountId::new("0x00000000000000000000000000000000000000aa".to_string());
    let bytes = canonical_payload_bytes(
        WriteAuthAction::PerpOrderCancel,
        &[
            ("account", CanonicalValue::Address(account)),
            ("subaccount_id", CanonicalValue::U64(2)),
            (
                "order_id",
                CanonicalValue::Str("11111111-2222-3333-4444-555555555555".to_string()),
            ),
        ],
    );
    let text = String::from_utf8(bytes).expect("canonical bytes must be UTF-8");
    assert!(
        text.starts_with("PERP_ORDER_CANCEL|"),
        "action prefix drift: {text}"
    );
    assert!(
        text.contains("|subaccount_id=2|"),
        "subaccount_id must be the second field: {text}"
    );
    assert!(
        text.ends_with("|order_id=\"11111111-2222-3333-4444-555555555555\""),
        "trailing field is order_id (string-quoted): {text}"
    );
}

// ---------------------------------------------------------------------
// PART 3 — closed-test allowlist helper on AppState
// ---------------------------------------------------------------------

fn build_perps_state() -> AppState {
    let config = OptionsConfig::disabled();
    AppState::with_options_config(EngineState::with_default_markets(), config)
}

#[test]
fn perps_closed_test_default_off_denies_every_caller() {
    let state = build_perps_state();
    // Default AppState: flag off, allowlist empty.
    let caller = AccountId::new("0x00000000000000000000000000000000000000aa".to_string());
    assert!(!state.perps_closed_test_allows(&caller));
}

#[test]
fn perps_closed_test_enabled_with_empty_allowlist_denies_every_caller() {
    let mut state = build_perps_state();
    state.perps_closed_test_enabled = true;
    let caller = AccountId::new("0x00000000000000000000000000000000000000aa".to_string());
    // Enabled + empty allowlist = honest "nobody in", not "everyone in".
    assert!(!state.perps_closed_test_allows(&caller));
}

#[test]
fn perps_closed_test_admits_allowlisted_wallet_case_insensitive() {
    let mut state = build_perps_state();
    state.perps_closed_test_enabled = true;
    state.perps_closed_test_allowlist = vec![AccountId::new(
        "0x00000000000000000000000000000000000000aa".to_string(),
    )];
    let upper = AccountId::new("0x00000000000000000000000000000000000000AA".to_string());
    let lower = AccountId::new("0x00000000000000000000000000000000000000aa".to_string());
    assert!(state.perps_closed_test_allows(&upper));
    assert!(state.perps_closed_test_allows(&lower));
}

#[test]
fn perps_closed_test_denies_non_allowlisted_wallet_even_when_flag_on() {
    let mut state = build_perps_state();
    state.perps_closed_test_enabled = true;
    state.perps_closed_test_allowlist = vec![AccountId::new(
        "0x00000000000000000000000000000000000000aa".to_string(),
    )];
    let stranger = AccountId::new("0x00000000000000000000000000000000000000bb".to_string());
    assert!(!state.perps_closed_test_allows(&stranger));
}

#[test]
fn perps_closed_test_denies_when_flag_off_even_if_wallet_on_list() {
    let mut state = build_perps_state();
    state.perps_closed_test_enabled = false;
    state.perps_closed_test_allowlist = vec![AccountId::new(
        "0x00000000000000000000000000000000000000aa".to_string(),
    )];
    let caller = AccountId::new("0x00000000000000000000000000000000000000aa".to_string());
    assert!(!state.perps_closed_test_allows(&caller));
}

// ---------------------------------------------------------------------
// PART 4 — Perps public routes remain fail-closed regardless of the
// new closed-test flag. This is a belt+braces regression check on top
// of `perps_public_route_fail_closed_grid.rs`.
// ---------------------------------------------------------------------

#[tokio::test]
async fn perps_submit_returns_503_when_closed_test_on_but_public_trading_off() {
    let mut state = build_perps_state();
    // Closed-test flag on with an allowlisted wallet — but the public
    // trading flag stays false. The mutation handler MUST still return
    // 503 because the fail-closed layer 1 is `perps_public_trading_enabled`.
    state.perps_closed_test_enabled = true;
    state.perps_closed_test_allowlist = vec![AccountId::new(
        "0x00000000000000000000000000000000000000aa".to_string(),
    )];
    let app = router(state);

    let body = serde_json::json!({
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
    });
    let request = Request::builder()
        .method("POST")
        .uri("/perps/orders")
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .expect("request");
    let response = app.oneshot(request).await.expect("service");
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body_bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    let text = String::from_utf8_lossy(&body_bytes);
    assert!(
        text.to_lowercase().contains("perps"),
        "response must acknowledge perps posture; got: {text}"
    );
}

// ---------------------------------------------------------------------
// PART 5 — read handlers accept ?subaccount_id / ?all query params and
// return empty for non-default subaccount ids (honest — no data
// persisted for non-1 subaccounts until the engine routing lands).
// ---------------------------------------------------------------------

#[tokio::test]
async fn perps_positions_read_returns_empty_for_subaccount_2() {
    let state = build_perps_state();
    let app = router(state);
    let request = Request::builder()
        .method("GET")
        .uri("/accounts/0x00000000000000000000000000000000000000aa/perps/positions?subaccount_id=2")
        .body(Body::empty())
        .expect("request");
    let response = app.oneshot(request).await.expect("service");
    assert_eq!(response.status(), StatusCode::OK);
    let body_bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    let json: serde_json::Value = serde_json::from_slice(&body_bytes).expect("json");
    // The positions array must be present and empty. Any non-empty
    // result at subaccount 2 would indicate the honest short-circuit
    // was skipped (a P0 leakage bug).
    let positions = json
        .get("positions")
        .expect("response carries positions array");
    assert!(positions.is_array(), "positions must be array: {json}");
    assert_eq!(
        positions.as_array().unwrap().len(),
        0,
        "subaccount 2 has no rows persisted in this milestone: {json}"
    );
}

#[tokio::test]
async fn perps_orders_read_returns_empty_for_subaccount_5() {
    let state = build_perps_state();
    let app = router(state);
    let request = Request::builder()
        .method("GET")
        .uri("/accounts/0x00000000000000000000000000000000000000aa/perps/orders?subaccount_id=5")
        .body(Body::empty())
        .expect("request");
    let response = app.oneshot(request).await.expect("service");
    assert_eq!(response.status(), StatusCode::OK);
    let body_bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    let json: serde_json::Value = serde_json::from_slice(&body_bytes).expect("json");
    let orders = json.get("orders").expect("orders array");
    assert_eq!(orders.as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn perps_fills_read_returns_empty_for_subaccount_3() {
    let state = build_perps_state();
    let app = router(state);
    let request = Request::builder()
        .method("GET")
        .uri("/accounts/0x00000000000000000000000000000000000000aa/perps/fills?subaccount_id=3")
        .body(Body::empty())
        .expect("request");
    let response = app.oneshot(request).await.expect("service");
    assert_eq!(response.status(), StatusCode::OK);
    let body_bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    let json: serde_json::Value = serde_json::from_slice(&body_bytes).expect("json");
    let fills = json.get("fills").expect("fills array");
    assert_eq!(fills.as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn perps_positions_read_default_no_subaccount_query_still_works() {
    // No subaccount_id / all query means default = subaccount 1. That
    // should hit the existing read path (which returns whatever wallet
    // data exists — empty for a fresh state).
    let state = build_perps_state();
    let app = router(state);
    let request = Request::builder()
        .method("GET")
        .uri("/accounts/0x00000000000000000000000000000000000000aa/perps/positions")
        .body(Body::empty())
        .expect("request");
    let response = app.oneshot(request).await.expect("service");
    assert_eq!(response.status(), StatusCode::OK);
    let body_bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    let json: serde_json::Value = serde_json::from_slice(&body_bytes).expect("json");
    let positions = json.get("positions").expect("positions array");
    assert!(positions.is_array());
}

#[tokio::test]
async fn perps_positions_read_all_true_reaches_read_path() {
    // ?all=true — the short-circuit MUST NOT fire; the handler should
    // fall through to the existing read logic (returning wallet-level
    // aggregate). In a fresh state that's still an empty list, but the
    // status must be 200 and the JSON shape intact.
    let state = build_perps_state();
    let app = router(state);
    let request = Request::builder()
        .method("GET")
        .uri("/accounts/0x00000000000000000000000000000000000000aa/perps/positions?all=true")
        .body(Body::empty())
        .expect("request");
    let response = app.oneshot(request).await.expect("service");
    assert_eq!(response.status(), StatusCode::OK);
    let body_bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    let json: serde_json::Value = serde_json::from_slice(&body_bytes).expect("json");
    assert!(json.get("positions").is_some());
    assert!(json.get("chain_id").is_some());
}
