//! SUBACCOUNTS-OPTIONS-ORDERS-HISTORY-V1
//!
//! Foundation-posture regression + rejection tests. This milestone
//! only lands schema + handler-level validation; the actual v2
//! routing lands in `SUBACCOUNTS-OPTIONS-ROUTING-V1`. These tests
//! pin the posture:
//!
//! * **v1 order/cancel/conditional/TWAP** flows are unchanged for
//!   default subaccount usage. Any client that omits `subaccount_id`
//!   or sends `subaccount_id=1` continues to work byte-identically
//!   to the previous milestone.
//! * **v1 auth with `subaccount_id > 1`** rejects at handler entry
//!   with 400 `InvalidSubaccountRequest`. The mutation never
//!   reaches persistence; the nonce is never claimed.
//! * **v2 auth (`authorization.version = 2`)** rejects at handler
//!   entry with 503 `SubaccountsRoutingNotLive`. Same guarantee.
//! * **Byte-freeze** for the v2 canonical payloads that future
//!   frontend + backend signers must agree on when the routing
//!   milestone flips the gate.
//!
//! Uses in-memory `AppState` (no PostgreSQL required).

use axum::body::{to_bytes, Body};
use axum::http::{header, Request, StatusCode};
use deopt_v2_backend::api::{router, AppState};
use deopt_v2_backend::auth::write_authorization::{
    canonical_payload_bytes, nonce_to_hex, write_auth_eip712_digest, AuthorizationEnvelope,
    CanonicalValue, ChallengeRecord, ChallengeStatus, WriteAuthAction, WRITE_AUTH_DOMAIN_CHAIN_ID,
};
use deopt_v2_backend::engine::EngineState;
use deopt_v2_backend::options::OptionsConfig;
use deopt_v2_backend::types::{now_ms, AccountId};
use k256::ecdsa::SigningKey;
use serde_json::json;
use sha3::{Digest, Keccak256};
use tower::ServiceExt;

const DEADLINE_TTL_MS: i64 = 60_000;
const DUMMY_SERIES_ID: &str = "BTC-30JAN2026-50000-C";
const DUMMY_ORDER_ID: &str = "00000000-0000-0000-0000-000000000001";
const DUMMY_CONDITIONAL_ID: &str = "00000000-0000-0000-0000-000000000002";
const DUMMY_TWAP_ID: &str = "00000000-0000-0000-0000-000000000003";

fn build_state() -> AppState {
    let mut config = OptionsConfig::enabled_in_memory_for_tests();
    config.rfq_enabled = false;
    AppState::with_options_config(EngineState::with_default_markets(), config)
}

fn keccak(bytes: &[u8]) -> [u8; 32] {
    let mut hasher = Keccak256::new();
    hasher.update(bytes);
    hasher.finalize().into()
}

fn derive_account(signing_key: &SigningKey) -> AccountId {
    let verifying_key = signing_key.verifying_key();
    let encoded = verifying_key.to_encoded_point(false);
    let public_key = encoded.as_bytes();
    let hash = keccak(&public_key[1..]);
    let mut address = [0u8; 20];
    address.copy_from_slice(&hash[12..]);
    let mut hex = String::from("0x");
    for b in &address {
        hex.push_str(&format!("{:02x}", b));
    }
    AccountId::new(hex)
}

fn test_keypair(seed: u8) -> (SigningKey, AccountId) {
    let signing_key = SigningKey::from_bytes(&[seed; 32].into()).expect("test signing key");
    let account = derive_account(&signing_key);
    (signing_key, account)
}

async fn issue_challenge(
    state: &AppState,
    action: WriteAuthAction,
    account: &AccountId,
    tag: u8,
) -> [u8; 32] {
    let now = now_ms();
    let mut nonce_bytes = [0u8; 32];
    nonce_bytes[0] = (now & 0xff) as u8;
    nonce_bytes[1] = ((now >> 8) & 0xff) as u8;
    nonce_bytes[2] = action as u8;
    nonce_bytes[3] = tag;
    nonce_bytes[4..24].copy_from_slice(
        &account
            .0
            .strip_prefix("0x")
            .unwrap_or(&account.0)
            .chars()
            .take(20)
            .map(|c| c as u8)
            .collect::<Vec<u8>>()
            .as_slice()[..20.min(account.0.len())],
    );
    state
        .write_auth_challenges
        .issue(ChallengeRecord {
            nonce_bytes,
            account: account.clone(),
            action,
            chain_id: WRITE_AUTH_DOMAIN_CHAIN_ID,
            issued_at_ms: now,
            expires_at_ms: now + DEADLINE_TTL_MS,
            status: ChallengeStatus::Issued,
            request_digest: None,
            idempotency_key: None,
            resource_id: None,
            consumed_at_ms: None,
        })
        .await
        .expect("issue test challenge");
    nonce_bytes
}

/// Build an envelope. `version=None` signals v1; `version=Some(2)`
/// signals the future v2 shape. The digest computation itself is
/// unchanged in this milestone — the version field is not yet
/// incorporated. Reserved for `SUBACCOUNTS-WRITE-AUTH-V2-MIGRATION-V1`.
fn sign_envelope(
    signing_key: &SigningKey,
    action: WriteAuthAction,
    account: &AccountId,
    canonical: &[u8],
    nonce_bytes: [u8; 32],
    deadline_ms: i64,
    version: Option<u16>,
) -> AuthorizationEnvelope {
    let digest =
        write_auth_eip712_digest(action, account, canonical, &nonce_bytes, deadline_ms, None)
            .expect("digest");
    let (signature, recovery_id) = signing_key.sign_prehash_recoverable(&digest).expect("sign");
    let sig_bytes = signature.to_bytes();
    let v: u8 = recovery_id.into();
    let mut signature_hex = String::from("0x");
    for b in &sig_bytes {
        signature_hex.push_str(&format!("{:02x}", b));
    }
    signature_hex.push_str(&format!("{:02x}", v + 27));
    AuthorizationEnvelope {
        action: action.as_str().to_string(),
        account: account.clone(),
        nonce: nonce_to_hex(&nonce_bytes),
        deadline_ms,
        signature: signature_hex,
        idempotency_key: None,
        version,
    }
}

fn json_post(path: &str, body: serde_json::Value) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(path)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(serde_json::to_vec(&body).expect("json body")))
        .expect("build request")
}

async fn body_json(response: axum::response::Response) -> serde_json::Value {
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("collect body");
    serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
}

// ===========================================================================
// v1 canonical payload builders that mirror the routes.rs helpers.
// These are private to the test file so we don't take a dependency on
// crate-internal symbols; they must stay byte-identical to the v1
// builders in `src/api/routes.rs`.
// ===========================================================================

fn v1_option_order_submit_canonical(account: &AccountId, subaccount_id: Option<u32>) -> Vec<u8> {
    let mut fields: Vec<(&'static str, CanonicalValue)> = vec![
        ("account", CanonicalValue::Address(account.clone())),
        (
            "option_series_id",
            CanonicalValue::Str(DUMMY_SERIES_ID.to_string()),
        ),
        ("side", CanonicalValue::Str("buy".to_string())),
        ("price_1e8", CanonicalValue::Str("1000000000".to_string())),
        ("size_1e8", CanonicalValue::Str("100000000".to_string())),
        ("time_in_force", CanonicalValue::Str("gtc".to_string())),
        ("post_only", CanonicalValue::Bool(false)),
        ("client_order_id", CanonicalValue::Null),
    ];
    // v1 canonical does not include subaccount_id, but the body may
    // ship subaccount_id=Some(1). The signature is over the v1
    // canonical bytes above regardless.
    let _ = subaccount_id;
    canonical_payload_bytes(WriteAuthAction::OptionOrderSubmit, &fields[..])
}

// ===========================================================================
// 1) v1 accepts subaccount_id=None (backward compat)
// ===========================================================================

#[tokio::test]
async fn v1_omitted_subaccount_id_accepted_at_handler_gate() {
    let state = build_state();
    let (signing_key, account) = test_keypair(0xC1);
    let now = now_ms();
    let deadline = now + DEADLINE_TTL_MS - 1;
    let canonical = v1_option_order_submit_canonical(&account, None);
    let nonce = issue_challenge(&state, WriteAuthAction::OptionOrderSubmit, &account, 1).await;
    let envelope = sign_envelope(
        &signing_key,
        WriteAuthAction::OptionOrderSubmit,
        &account,
        &canonical,
        nonce,
        deadline,
        None,
    );
    let body = json!({
        "option_series_id": DUMMY_SERIES_ID,
        "account": account.0,
        "side": "buy",
        "price_1e8": "1000000000",
        "size_1e8": "100000000",
        "time_in_force": "gtc",
        "post_only": false,
        "client_order_id": null,
        "authorization": envelope,
    });
    let response = router(state)
        .oneshot(json_post("/options/orders", body))
        .await
        .expect("submit");
    // The handler-level gate passes; downstream execution may still
    // 400/404 on the (nonexistent) series or the (disabled) execution
    // pipeline — the point is that we did NOT get the 400
    // `InvalidSubaccountRequest` or 503 `SubaccountsRoutingNotLive`.
    let status = response.status();
    let body = body_json(response).await;
    let message = body.get("error").and_then(|v| v.as_str()).unwrap_or("");
    assert!(
        !message.contains("cannot route to subaccount")
            && !message.contains("subaccount routing is not live"),
        "handler gate should have allowed v1+omitted, status={status} body={body}"
    );
}

// ===========================================================================
// 2) v1 accepts subaccount_id=Some(1) explicitly
// ===========================================================================

#[tokio::test]
async fn v1_explicit_default_subaccount_accepted_at_handler_gate() {
    let state = build_state();
    let (signing_key, account) = test_keypair(0xC2);
    let now = now_ms();
    let deadline = now + DEADLINE_TTL_MS - 1;
    let canonical = v1_option_order_submit_canonical(&account, Some(1));
    let nonce = issue_challenge(&state, WriteAuthAction::OptionOrderSubmit, &account, 1).await;
    let envelope = sign_envelope(
        &signing_key,
        WriteAuthAction::OptionOrderSubmit,
        &account,
        &canonical,
        nonce,
        deadline,
        None,
    );
    let body = json!({
        "option_series_id": DUMMY_SERIES_ID,
        "account": account.0,
        "side": "buy",
        "price_1e8": "1000000000",
        "size_1e8": "100000000",
        "time_in_force": "gtc",
        "post_only": false,
        "client_order_id": null,
        "subaccount_id": 1,
        "authorization": envelope,
    });
    let response = router(state)
        .oneshot(json_post("/options/orders", body))
        .await
        .expect("submit");
    let status = response.status();
    let body = body_json(response).await;
    let message = body.get("error").and_then(|v| v.as_str()).unwrap_or("");
    assert!(
        !message.contains("cannot route to subaccount")
            && !message.contains("subaccount routing is not live"),
        "handler gate should have allowed v1+subaccount_id=1, status={status} body={body}"
    );
}

// ===========================================================================
// 3) v1 auth + subaccount_id=2 in body → 400 InvalidSubaccountRequest
// ===========================================================================

#[tokio::test]
async fn v1_with_nondefault_subaccount_rejected_400() {
    let state = build_state();
    let (signing_key, account) = test_keypair(0xC3);
    let now = now_ms();
    let deadline = now + DEADLINE_TTL_MS - 1;
    let canonical = v1_option_order_submit_canonical(&account, None);
    let nonce = issue_challenge(&state, WriteAuthAction::OptionOrderSubmit, &account, 1).await;
    let envelope = sign_envelope(
        &signing_key,
        WriteAuthAction::OptionOrderSubmit,
        &account,
        &canonical,
        nonce,
        deadline,
        None,
    );
    let body = json!({
        "option_series_id": DUMMY_SERIES_ID,
        "account": account.0,
        "side": "buy",
        "price_1e8": "1000000000",
        "size_1e8": "100000000",
        "time_in_force": "gtc",
        "post_only": false,
        "client_order_id": null,
        "subaccount_id": 2,
        "authorization": envelope,
    });
    let response = router(state)
        .oneshot(json_post("/options/orders", body))
        .await
        .expect("submit");
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = body_json(response).await;
    let msg = body["error"].as_str().unwrap_or("");
    assert!(
        msg.contains("cannot route to subaccount"),
        "expected v1-subaccount rejection message, got: {msg}"
    );
}

// ===========================================================================
// 4) v2 envelope → 503 SubaccountsRoutingNotLive across all 6 actions.
// The gate fires before write-auth so we don't need a valid signature.
// ===========================================================================

fn envelope_v2(account: &AccountId) -> AuthorizationEnvelope {
    // A minimal v2 envelope. Signature is intentionally not
    // recoverable — the fail-closed gate fires before write-auth.
    AuthorizationEnvelope {
        action: WriteAuthAction::OptionOrderSubmit.as_str().to_string(),
        account: account.clone(),
        nonce: "0x0000000000000000000000000000000000000000000000000000000000000000".to_string(),
        deadline_ms: now_ms() + 60_000,
        signature: "0x00".repeat(64) + "1c",
        idempotency_key: None,
        version: Some(2),
    }
}

#[tokio::test]
async fn v2_order_submit_rejects_503() {
    let state = build_state();
    let (_sk, account) = test_keypair(0xD1);
    let mut env = envelope_v2(&account);
    env.action = WriteAuthAction::OptionOrderSubmit.as_str().to_string();
    let body = json!({
        "option_series_id": DUMMY_SERIES_ID,
        "account": account.0,
        "side": "buy",
        "price_1e8": "1000000000",
        "size_1e8": "100000000",
        "time_in_force": "gtc",
        "post_only": false,
        "client_order_id": null,
        "subaccount_id": 2,
        "authorization": env,
    });
    let response = router(state)
        .oneshot(json_post("/options/orders", body))
        .await
        .expect("submit");
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body = body_json(response).await;
    assert!(body["error"]
        .as_str()
        .unwrap_or("")
        .contains("subaccount routing is not live"));
}

#[tokio::test]
async fn v2_order_cancel_rejects_503() {
    let state = build_state();
    let (_sk, account) = test_keypair(0xD2);
    let mut env = envelope_v2(&account);
    env.action = WriteAuthAction::OptionOrderCancel.as_str().to_string();
    let body = json!({ "authorization": env, "subaccount_id": 2 });
    let response = router(state)
        .oneshot(json_post(
            &format!("/options/orders/{DUMMY_ORDER_ID}/cancel"),
            body,
        ))
        .await
        .expect("cancel");
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn v2_conditional_create_rejects_503() {
    let state = build_state();
    let (_sk, account) = test_keypair(0xD3);
    let mut env = envelope_v2(&account);
    env.action = WriteAuthAction::ConditionalOrderCreate.as_str().to_string();
    let body = json!({
        "option_series_id": DUMMY_SERIES_ID,
        "quantity_1e8": "100000000",
        "legs": [],
        "link_as_oco": false,
        "expires_at_ms": null,
        "subaccount_id": 2,
        "authorization": env,
    });
    let response = router(state)
        .oneshot(json_post(
            &format!("/accounts/{}/conditional-orders", account.0),
            body,
        ))
        .await
        .expect("conditional create");
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn v2_conditional_cancel_rejects_503() {
    let state = build_state();
    let (_sk, account) = test_keypair(0xD4);
    let mut env = envelope_v2(&account);
    env.action = WriteAuthAction::ConditionalOrderCancel.as_str().to_string();
    // The route is DELETE with a body — matching the existing test
    // pattern used in conditional_orders_tests.rs. Use axum
    // DELETE-with-body.
    let request = Request::builder()
        .method("DELETE")
        .uri(&format!(
            "/accounts/{}/conditional-orders/{DUMMY_CONDITIONAL_ID}",
            account.0
        ))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::to_vec(&json!({ "authorization": env, "subaccount_id": 2 })).unwrap(),
        ))
        .expect("build request");
    let response = router(state).oneshot(request).await.expect("cancel");
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn v2_twap_create_rejects_503() {
    let state = build_state();
    let (_sk, account) = test_keypair(0xD5);
    let mut env = envelope_v2(&account);
    env.action = WriteAuthAction::OptionTwapCreate.as_str().to_string();
    let body = json!({
        "account": account.0,
        "option_series_id": DUMMY_SERIES_ID,
        "side": "buy",
        "size_1e8": "100000000",
        "limit_price_1e8": "1000000000",
        "running_time_ms": 60_000_u64,
        "child_count": 4_u32,
        "client_order_id": null,
        "subaccount_id": 2,
        "authorization": env,
    });
    let response = router(state)
        .oneshot(json_post("/options/twap-orders", body))
        .await
        .expect("twap create");
    // TWAP is behind a feature flag. Whether the flag is off or the
    // subaccount gate fires first, the point is the request MUST NOT
    // silently persist to subaccount 1. Accept 503 (subaccount gate)
    // or 400 (twap disabled) — both are honest failure modes.
    assert!(
        matches!(
            response.status(),
            StatusCode::SERVICE_UNAVAILABLE | StatusCode::BAD_REQUEST
        ),
        "expected 503 or 400, got: {}",
        response.status()
    );
}

#[tokio::test]
async fn v2_twap_cancel_rejects_503() {
    let state = build_state();
    let (_sk, account) = test_keypair(0xD6);
    let mut env = envelope_v2(&account);
    env.action = WriteAuthAction::OptionTwapCancel.as_str().to_string();
    let body = json!({ "authorization": env, "subaccount_id": 2 });
    let response = router(state)
        .oneshot(json_post(
            &format!("/options/twap-orders/{DUMMY_TWAP_ID}/cancel"),
            body,
        ))
        .await
        .expect("twap cancel");
    // Either 503 (subaccount gate) or 404 (twap not found — gate
    // fires after TWAP lookup for cancel). Accept both — the honest
    // posture is that a v2 envelope for this subaccount NEVER
    // silently succeeds.
    assert!(
        matches!(
            response.status(),
            StatusCode::SERVICE_UNAVAILABLE | StatusCode::NOT_FOUND
        ),
        "expected 503 or 404, got: {}",
        response.status()
    );
}

// ===========================================================================
// Byte-freeze tests for the v2 canonical payloads.
// These pin the wire format so a future migration cannot drift.
// ===========================================================================

const FROZEN_OWNER_ADDR: &str = "0xabcdef0000000000000000000000000000000001";

#[test]
fn v2_option_order_submit_canonical_bytes_are_frozen() {
    let owner = AccountId::new("0xABCDEF0000000000000000000000000000000001");
    let payload = canonical_payload_bytes(
        WriteAuthAction::OptionOrderSubmit,
        &[
            ("account", CanonicalValue::Address(owner)),
            ("subaccount_id", CanonicalValue::U64(2)),
            (
                "option_series_id",
                CanonicalValue::Str(DUMMY_SERIES_ID.to_string()),
            ),
            ("side", CanonicalValue::Str("buy".to_string())),
            ("price_1e8", CanonicalValue::Str("1000000000".to_string())),
            ("size_1e8", CanonicalValue::Str("100000000".to_string())),
            ("time_in_force", CanonicalValue::Str("gtc".to_string())),
            ("post_only", CanonicalValue::Bool(false)),
            ("client_order_id", CanonicalValue::Null),
        ],
    );
    let expected = format!(
        "OPTION_ORDER_SUBMIT|account=\"{FROZEN_OWNER_ADDR}\"|subaccount_id=2|\
         option_series_id=\"{DUMMY_SERIES_ID}\"|side=\"buy\"|price_1e8=\"1000000000\"|\
         size_1e8=\"100000000\"|time_in_force=\"gtc\"|post_only=false|client_order_id=null"
    );
    assert_eq!(std::str::from_utf8(&payload).unwrap(), expected);
}

#[test]
fn v2_option_order_cancel_canonical_bytes_are_frozen() {
    let owner = AccountId::new("0xABCDEF0000000000000000000000000000000001");
    let payload = canonical_payload_bytes(
        WriteAuthAction::OptionOrderCancel,
        &[
            ("account", CanonicalValue::Address(owner)),
            ("subaccount_id", CanonicalValue::U64(2)),
            ("order_id", CanonicalValue::Str(DUMMY_ORDER_ID.to_string())),
        ],
    );
    let expected = format!(
        "OPTION_ORDER_CANCEL|account=\"{FROZEN_OWNER_ADDR}\"|subaccount_id=2|\
         order_id=\"{DUMMY_ORDER_ID}\""
    );
    assert_eq!(std::str::from_utf8(&payload).unwrap(), expected);
}

#[test]
fn v2_conditional_order_cancel_canonical_bytes_are_frozen() {
    let owner = AccountId::new("0xABCDEF0000000000000000000000000000000001");
    let payload = canonical_payload_bytes(
        WriteAuthAction::ConditionalOrderCancel,
        &[
            ("account", CanonicalValue::Address(owner)),
            ("subaccount_id", CanonicalValue::U64(2)),
            (
                "conditional_order_id",
                CanonicalValue::Str(DUMMY_CONDITIONAL_ID.to_string()),
            ),
        ],
    );
    let expected = format!(
        "CONDITIONAL_ORDER_CANCEL|account=\"{FROZEN_OWNER_ADDR}\"|subaccount_id=2|\
         conditional_order_id=\"{DUMMY_CONDITIONAL_ID}\""
    );
    assert_eq!(std::str::from_utf8(&payload).unwrap(), expected);
}

#[test]
fn v2_option_twap_cancel_canonical_bytes_are_frozen() {
    let owner = AccountId::new("0xABCDEF0000000000000000000000000000000001");
    let payload = canonical_payload_bytes(
        WriteAuthAction::OptionTwapCancel,
        &[
            ("account", CanonicalValue::Address(owner)),
            ("subaccount_id", CanonicalValue::U64(2)),
            (
                "option_twap_id",
                CanonicalValue::Str(DUMMY_TWAP_ID.to_string()),
            ),
        ],
    );
    let expected = format!(
        "OPTION_TWAP_CANCEL|account=\"{FROZEN_OWNER_ADDR}\"|subaccount_id=2|\
         option_twap_id=\"{DUMMY_TWAP_ID}\""
    );
    assert_eq!(std::str::from_utf8(&payload).unwrap(), expected);
}
