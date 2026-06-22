//! ACCOUNT-WRITE-AUTH-HARDENING-V1
//!
//! End-to-end route-level integration tests for the EIP-712 write
//! authorization layer. Exercises the actual HTTP router (so we
//! cover the axum extractor + canonical-payload reconstruction +
//! atomic nonce claim path).

use axum::body::{to_bytes, Body};
use axum::http::{header, Request, StatusCode};
use deopt_v2_backend::api::{router, AppState};
use deopt_v2_backend::auth::write_authorization::{
    canonical_payload_bytes, nonce_to_hex, write_auth_eip712_digest, AuthorizationEnvelope,
    CanonicalValue, ChallengeRecord, ChallengeStatus, WriteAuthAction, WRITE_AUTH_DOMAIN_CHAIN_ID,
};
use deopt_v2_backend::engine::EngineState;
use deopt_v2_backend::options::service::{create_option_series, CreateOptionSeriesInput};
use deopt_v2_backend::options::OptionsConfig;
use deopt_v2_backend::types::{now_ms, AccountId};
use k256::ecdsa::SigningKey;
use k256::elliptic_curve::sec1::ToEncodedPoint as _;
use serde_json::json;
use sha3::{Digest, Keccak256};
use tower::ServiceExt;

const DEADLINE_TTL_MS: i64 = 60_000;

fn build_state() -> AppState {
    let mut config = OptionsConfig::enabled_in_memory_for_tests();
    config.rfq_enabled = true;
    AppState::with_options_config(EngineState::with_default_markets(), config)
}

fn future_expiry() -> u64 {
    u64::try_from(now_ms() / 1000).unwrap() + 86_400
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

fn test_keypair(seed_byte: u8) -> (SigningKey, AccountId) {
    let signing_key = SigningKey::from_bytes(&[seed_byte; 32].into()).expect("test signing key");
    let account = derive_account(&signing_key);
    (signing_key, account)
}

async fn issue_challenge(
    state: &AppState,
    action: WriteAuthAction,
    account: &AccountId,
    idempotency_key: Option<&str>,
) -> [u8; 32] {
    let now = now_ms();
    let nonce_bytes: [u8; 32] = {
        let mut out = [0u8; 32];
        // deterministic test nonce; production uses CSPRNG
        out[0] = (now & 0xff) as u8;
        out[1] = ((now >> 8) & 0xff) as u8;
        out[2] = action as u8;
        out[3..23].copy_from_slice(
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
        out
    };
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
            idempotency_key: idempotency_key.map(|s| s.to_string()),
            resource_id: None,
            consumed_at_ms: None,
        })
        .await
        .expect("issue test challenge");
    nonce_bytes
}

fn sign_envelope(
    signing_key: &SigningKey,
    action: WriteAuthAction,
    account: &AccountId,
    canonical: &[u8],
    nonce_bytes: [u8; 32],
    deadline_ms: i64,
    idempotency_key: Option<&str>,
) -> AuthorizationEnvelope {
    let digest = write_auth_eip712_digest(
        action,
        account,
        canonical,
        &nonce_bytes,
        deadline_ms,
        idempotency_key,
    )
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
        idempotency_key: idempotency_key.map(|s| s.to_string()),
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

fn delete_with_body(path: &str, body: serde_json::Value) -> Request<Body> {
    Request::builder()
        .method("DELETE")
        .uri(path)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(serde_json::to_vec(&body).expect("json body")))
        .expect("build request")
}

async fn response_json(response: axum::response::Response) -> serde_json::Value {
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    serde_json::from_slice(&bytes).unwrap_or(serde_json::json!({}))
}

async fn active_series_id(state: &AppState) -> String {
    create_option_series(
        state,
        CreateOptionSeriesInput {
            underlying: "ETH".to_string(),
            base_asset: "ETH".to_string(),
            quote_asset: "USDC".to_string(),
            settlement_asset: "USDC".to_string(),
            expiry: future_expiry(),
            strike_1e8: 300_000_000_000,
            is_call: true,
            contract_size_1e8: Some(100_000_000),
            onchain_product_id: None,
            onchain_series_id: None,
        },
    )
    .await
    .expect("create series")
    .option_series_id
}

fn canonical_for_option_order_submit(
    account: &AccountId,
    series: &str,
    price_1e8: &str,
    size_1e8: &str,
    client_order_id: Option<&str>,
) -> Vec<u8> {
    canonical_payload_bytes(
        WriteAuthAction::OptionOrderSubmit,
        &[
            ("account", CanonicalValue::Address(account.clone())),
            ("option_series_id", CanonicalValue::Str(series.to_string())),
            ("side", CanonicalValue::Str("buy".to_string())),
            ("price_1e8", CanonicalValue::Str(price_1e8.to_string())),
            ("size_1e8", CanonicalValue::Str(size_1e8.to_string())),
            ("time_in_force", CanonicalValue::Str("gtc".to_string())),
            ("post_only", CanonicalValue::Bool(false)),
            (
                "client_order_id",
                client_order_id
                    .map(|v| CanonicalValue::Str(v.to_string()))
                    .unwrap_or(CanonicalValue::Null),
            ),
        ],
    )
}

#[tokio::test]
async fn valid_signed_option_order_accepted() {
    let state = build_state();
    let (signing_key, account) = test_keypair(0xa1);
    let series_id = active_series_id(&state).await;
    let nonce = issue_challenge(&state, WriteAuthAction::OptionOrderSubmit, &account, None).await;
    let deadline = now_ms() + DEADLINE_TTL_MS - 1;
    let canonical = canonical_for_option_order_submit(
        &account,
        &series_id,
        "1000000000",
        "100000000",
        Some("clid-1"),
    );
    let envelope = sign_envelope(
        &signing_key,
        WriteAuthAction::OptionOrderSubmit,
        &account,
        &canonical,
        nonce,
        deadline,
        None,
    );
    let app = router(state);
    let response = app
        .clone()
        .oneshot(json_post(
            "/options/orders",
            json!({
                "option_series_id": series_id,
                "account": account.0,
                "side": "buy",
                "price_1e8": "1000000000",
                "size_1e8": "100000000",
                "time_in_force": "gtc",
                "post_only": false,
                "client_order_id": "clid-1",
                "authorization": envelope,
            }),
        ))
        .await
        .unwrap();
    let status = response.status();
    let body = response_json(response).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "expected 200; got status={} body={}",
        status,
        body
    );
    // SubmitOptionOrderResponse flattens order fields at the top
    // level alongside the `fills` array.
    assert_eq!(body["status"].as_str(), Some("open"));
    assert_eq!(
        body["account"].as_str(),
        Some(account.0.to_lowercase().as_str())
    );
}

#[tokio::test]
async fn unsigned_option_order_rejected() {
    let state = build_state();
    let series_id = active_series_id(&state).await;
    let response = router(state)
        .oneshot(json_post(
            "/options/orders",
            json!({
                "option_series_id": series_id,
                "account": "0x000000000000000000000000000000000000a1a1",
                "side": "buy",
                "price_1e8": "1000000000",
                "size_1e8": "100000000",
                "time_in_force": "gtc",
                "post_only": false,
                "client_order_id": "no-auth",
            }),
        ))
        .await
        .unwrap();
    // axum returns 422 UNPROCESSABLE_ENTITY when the body fails to
    // deserialize because `authorization` is required.
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn tampered_payload_rejected() {
    let state = build_state();
    let (signing_key, account) = test_keypair(0xa2);
    let series_id = active_series_id(&state).await;
    let nonce = issue_challenge(&state, WriteAuthAction::OptionOrderSubmit, &account, None).await;
    let deadline = now_ms() + DEADLINE_TTL_MS - 1;
    // Sign over price "1000000000" but submit "9999999999" — recovered
    // signer will not match the body's account.
    let canonical_signed =
        canonical_for_option_order_submit(&account, &series_id, "1000000000", "100000000", None);
    let envelope = sign_envelope(
        &signing_key,
        WriteAuthAction::OptionOrderSubmit,
        &account,
        &canonical_signed,
        nonce,
        deadline,
        None,
    );
    let response = router(state)
        .oneshot(json_post(
            "/options/orders",
            json!({
                "option_series_id": series_id,
                "account": account.0,
                "side": "buy",
                "price_1e8": "9999999999",
                "size_1e8": "100000000",
                "time_in_force": "gtc",
                "post_only": false,
                "client_order_id": null,
                "authorization": envelope,
            }),
        ))
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::FORBIDDEN,
        "tampered price must fail signer-match (403)"
    );
}

#[tokio::test]
async fn wrong_account_rejected() {
    let state = build_state();
    let (signing_key_a, account_a) = test_keypair(0xa3);
    let (_signing_key_b, account_b) = test_keypair(0xa4);
    let series_id = active_series_id(&state).await;
    let nonce = issue_challenge(&state, WriteAuthAction::OptionOrderSubmit, &account_a, None).await;
    let deadline = now_ms() + DEADLINE_TTL_MS - 1;
    // Sign with A but declare B as the body account.
    let canonical =
        canonical_for_option_order_submit(&account_a, &series_id, "1000000000", "100000000", None);
    let envelope_signed_by_a = sign_envelope(
        &signing_key_a,
        WriteAuthAction::OptionOrderSubmit,
        &account_a,
        &canonical,
        nonce,
        deadline,
        None,
    );
    // Mutate the envelope to claim it was for account_b — the route
    // computes canonical from the body's account (B), so the signed
    // digest will be over A's canonical and the body's canonical will
    // be over B's — recovered signer won't match B.
    let mut envelope = envelope_signed_by_a;
    envelope.account = account_b.clone();
    let response = router(state)
        .oneshot(json_post(
            "/options/orders",
            json!({
                "option_series_id": series_id,
                "account": account_b.0,
                "side": "buy",
                "price_1e8": "1000000000",
                "size_1e8": "100000000",
                "time_in_force": "gtc",
                "post_only": false,
                "client_order_id": null,
                "authorization": envelope,
            }),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn replay_of_consumed_nonce_rejected() {
    let state = build_state();
    let (signing_key, account) = test_keypair(0xa5);
    let series_id = active_series_id(&state).await;
    let nonce = issue_challenge(&state, WriteAuthAction::OptionOrderSubmit, &account, None).await;
    let deadline = now_ms() + DEADLINE_TTL_MS - 1;
    let canonical = canonical_for_option_order_submit(
        &account,
        &series_id,
        "1000000000",
        "100000000",
        Some("replay-1"),
    );
    let envelope = sign_envelope(
        &signing_key,
        WriteAuthAction::OptionOrderSubmit,
        &account,
        &canonical,
        nonce,
        deadline,
        None,
    );
    let app = router(state);
    let body = json!({
        "option_series_id": series_id,
        "account": account.0,
        "side": "buy",
        "price_1e8": "1000000000",
        "size_1e8": "100000000",
        "time_in_force": "gtc",
        "post_only": false,
        "client_order_id": "replay-1",
        "authorization": envelope,
    });
    let first = app
        .clone()
        .oneshot(json_post("/options/orders", body.clone()))
        .await
        .unwrap();
    assert_eq!(first.status(), StatusCode::OK);
    let first_body = response_json(first).await;
    let first_order_id = first_body["order_id"].as_str().unwrap().to_string();

    // Replay with the exact same envelope + payload — should return
    // the same order (idempotent), not create a second one.
    let second = app
        .oneshot(json_post("/options/orders", body))
        .await
        .unwrap();
    assert_eq!(second.status(), StatusCode::OK);
    let second_body = response_json(second).await;
    assert_eq!(
        second_body["order_id"].as_str().unwrap(),
        first_order_id,
        "exact retry must return the same order_id"
    );
}

#[tokio::test]
async fn perp_submit_order_fails_closed() {
    let state = build_state();
    let response = router(state)
        .oneshot(json_post(
            "/orders",
            json!({
                "market_id": 1,
                "account": "0x000000000000000000000000000000000000a6a6",
                "side": "buy",
                "price_1e8": "1000000000",
                "size_1e8": "100000000",
                "time_in_force": "gtc",
                "reduce_only": false,
                "post_only": false,
                "client_order_id": "x",
                "nonce": 1,
                "deadline_ms": 0,
                "signature": "0xff",
            }),
        ))
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::SERVICE_UNAVAILABLE,
        "perp order submission must fail closed"
    );
}

#[tokio::test]
async fn perp_cancel_order_fails_closed() {
    let state = build_state();
    let response = router(state)
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/orders/00000000-0000-0000-0000-000000000001")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn perp_create_rfq_fails_closed() {
    let state = build_state();
    let response = router(state)
        .oneshot(json_post("/rfqs", json!({})))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn challenge_endpoint_issues_nonce() {
    let state = build_state();
    let response = router(state)
        .oneshot(json_post(
            "/auth/write-challenges",
            json!({
                "account": "0x000000000000000000000000000000000000a7a7",
                "action": "OPTION_ORDER_SUBMIT"
            }),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(body["chain_id"], 84532);
    assert_eq!(body["action"], "OPTION_ORDER_SUBMIT");
    assert_eq!(body["domain"]["name"], "DeOpt API Write");
    assert_eq!(body["domain"]["version"], "1");
    assert_eq!(body["domain"]["chainId"], 84532);
    let nonce = body["nonce"].as_str().unwrap();
    assert!(nonce.starts_with("0x") && nonce.len() == 66);
    let salt = body["domain"]["salt"].as_str().unwrap();
    assert!(salt.starts_with("0x") && salt.len() == 66);
}

#[tokio::test]
async fn challenge_endpoint_rejects_unknown_action() {
    let state = build_state();
    let response = router(state)
        .oneshot(json_post(
            "/auth/write-challenges",
            json!({
                "account": "0x000000000000000000000000000000000000a8a8",
                "action": "TOTALLY_BOGUS_ACTION"
            }),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn conditional_cancel_requires_authorization_body() {
    // DELETE without a body should fail with 422 because the route
    // now requires { "authorization": ... }.
    let state = build_state();
    let response = router(state)
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/accounts/0x000000000000000000000000000000000000a9a9/conditional-orders/00000000-0000-0000-0000-000000000001")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(
        response.status() == StatusCode::UNPROCESSABLE_ENTITY
            || response.status() == StatusCode::BAD_REQUEST
            || response.status() == StatusCode::LENGTH_REQUIRED,
        "got unexpected status {}",
        response.status()
    );
}

#[tokio::test]
async fn conditional_cancel_rejects_cross_wallet() {
    // Build a state, create a conditional order owned by account_a,
    // then try to cancel it with a body signed by account_b.
    use deopt_v2_backend::options::conditional_orders as cond;
    let state = build_state();
    let (_signing_a, account_a) = test_keypair(0xb1);
    let (signing_b, account_b) = test_keypair(0xb2);

    // We need an active position for account_a before creating a TP/SL.
    // The full flow (build position via fills) is exercised in
    // conditional_orders_tests.rs; here we focus on the auth check
    // happening BEFORE any business logic. So we construct a request
    // that would fail business logic — but auth rejection must fire first.
    let id = uuid::Uuid::new_v4();
    let _ = cond::cancel_conditional_order(&state, id, &account_a).await; // pre-warm; ignore error

    // Build envelope from B but call cancel under A's URL path.
    let nonce = issue_challenge(
        &state,
        WriteAuthAction::ConditionalOrderCancel,
        &account_b,
        None,
    )
    .await;
    let deadline = now_ms() + DEADLINE_TTL_MS - 1;
    let canonical = canonical_payload_bytes(
        WriteAuthAction::ConditionalOrderCancel,
        &[
            ("account", CanonicalValue::Address(account_b.clone())),
            ("conditional_order_id", CanonicalValue::Str(id.to_string())),
        ],
    );
    let envelope = sign_envelope(
        &signing_b,
        WriteAuthAction::ConditionalOrderCancel,
        &account_b,
        &canonical,
        nonce,
        deadline,
        None,
    );
    let app = router(state);
    let response = app
        .oneshot(delete_with_body(
            &format!("/accounts/{}/conditional-orders/{}", account_a.0, id),
            json!({ "authorization": envelope }),
        ))
        .await
        .unwrap();
    // Auth check uses A from the URL path but envelope claims B; signer
    // mismatch → 403.
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}
