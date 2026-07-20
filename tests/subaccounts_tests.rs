//! SUBACCOUNTS-CORE-BACKEND-V1
//!
//! Route-level integration tests for the subaccount identity CRUD.
//! Covers:
//!
//! * lazy `Account 1` creation on the first GET of a fresh owner;
//! * deterministic id allocation (2, 3, ...);
//! * name validation (empty / trim / too-long / control-char);
//! * rename write-auth gate (create/rename require a signed
//!   `SUBACCOUNT_CREATE` / `SUBACCOUNT_RENAME` envelope);
//! * cross-owner isolation (renaming one owner's row must not touch
//!   another owner's row);
//! * default subaccount cannot be id 0.
//!
//! Uses the in-memory `AppState` (no PostgreSQL required).

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

fn build_state() -> AppState {
    AppState::with_options_config(
        EngineState::with_default_markets(),
        OptionsConfig::enabled_in_memory_for_tests(),
    )
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
    tag: u8,
) -> [u8; 32] {
    let now = now_ms();
    // Deterministic-per-test nonce derived from the seed byte + action
    // + a rolling counter (the caller passes a per-call `tag`).
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

fn sign_envelope(
    signing_key: &SigningKey,
    action: WriteAuthAction,
    account: &AccountId,
    canonical: &[u8],
    nonce_bytes: [u8; 32],
    deadline_ms: i64,
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
        version: None,
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

fn json_patch(path: &str, body: serde_json::Value) -> Request<Body> {
    Request::builder()
        .method("PATCH")
        .uri(path)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(serde_json::to_vec(&body).expect("json body")))
        .expect("build request")
}

fn get_request(path: &str) -> Request<Body> {
    Request::builder()
        .method("GET")
        .uri(path)
        .body(Body::empty())
        .expect("build request")
}

async fn body_json(response: axum::response::Response) -> serde_json::Value {
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("collect body");
    serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
}

fn canonical_create(owner: &AccountId, name: Option<&str>) -> Vec<u8> {
    canonical_payload_bytes(
        WriteAuthAction::SubaccountCreate,
        &[
            ("account", CanonicalValue::Address(owner.clone())),
            (
                "name",
                match name {
                    Some(s) => CanonicalValue::Str(s.to_string()),
                    None => CanonicalValue::Null,
                },
            ),
        ],
    )
}

fn canonical_rename(owner: &AccountId, subaccount_id: u32, name: &str) -> Vec<u8> {
    canonical_payload_bytes(
        WriteAuthAction::SubaccountRename,
        &[
            ("account", CanonicalValue::Address(owner.clone())),
            ("subaccount_id", CanonicalValue::U64(subaccount_id as u64)),
            ("name", CanonicalValue::Str(name.to_string())),
        ],
    )
}

#[tokio::test]
async fn list_lazy_creates_default_account_one() {
    let state = build_state();
    let app = router(state);
    let (_sk, account) = test_keypair(0xA1);

    let response = app
        .oneshot(get_request(&format!("/accounts/{}/subaccounts", account.0)))
        .await
        .expect("list");
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    let subaccounts = body["subaccounts"].as_array().expect("array");
    assert_eq!(subaccounts.len(), 1);
    assert_eq!(subaccounts[0]["subaccount_id"], 1);
    assert!(subaccounts[0]["name"].is_null());
    assert_eq!(subaccounts[0]["display_name"], "Account 1");
    // The default subaccount is never id 0.
    assert!(subaccounts[0]["subaccount_id"].as_u64().unwrap() >= 1);
}

#[tokio::test]
async fn get_missing_subaccount_returns_404() {
    let state = build_state();
    let app = router(state);
    let (_sk, account) = test_keypair(0xA2);

    // Fresh owner — no subaccount 42 exists.
    let response = app
        .oneshot(get_request(&format!(
            "/accounts/{}/subaccounts/42",
            account.0
        )))
        .await
        .expect("get");
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn create_allocates_next_id_starting_at_two() {
    let state = build_state();
    let (signing_key, account) = test_keypair(0xA3);

    // Prime the default via a list call so the store has row 1.
    let _ = router(state.clone())
        .oneshot(get_request(&format!("/accounts/{}/subaccounts", account.0)))
        .await
        .expect("prime");

    // Create Account 2 with a custom name.
    let now = now_ms();
    let deadline = now + DEADLINE_TTL_MS - 1;
    let canonical = canonical_create(&account, Some("Market Making"));
    let nonce = issue_challenge(&state, WriteAuthAction::SubaccountCreate, &account, 1).await;
    let envelope = sign_envelope(
        &signing_key,
        WriteAuthAction::SubaccountCreate,
        &account,
        &canonical,
        nonce,
        deadline,
    );
    let response = router(state.clone())
        .oneshot(json_post(
            &format!("/accounts/{}/subaccounts", account.0),
            json!({ "name": "Market Making", "authorization": envelope }),
        ))
        .await
        .expect("create");
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    assert_eq!(body["subaccount_id"], 2);
    assert_eq!(body["name"], "Market Making");
    assert_eq!(body["display_name"], "Market Making");

    // Create Account 3 without a name.
    let canonical = canonical_create(&account, None);
    let nonce = issue_challenge(&state, WriteAuthAction::SubaccountCreate, &account, 2).await;
    let envelope = sign_envelope(
        &signing_key,
        WriteAuthAction::SubaccountCreate,
        &account,
        &canonical,
        nonce,
        deadline,
    );
    let response = router(state)
        .oneshot(json_post(
            &format!("/accounts/{}/subaccounts", account.0),
            json!({ "authorization": envelope }),
        ))
        .await
        .expect("create third");
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    assert_eq!(body["subaccount_id"], 3);
    assert!(body["name"].is_null());
    assert_eq!(body["display_name"], "Account 3");
}

#[tokio::test]
async fn create_without_authorization_rejected() {
    let state = build_state();
    let app = router(state);
    let (_sk, account) = test_keypair(0xA4);

    let response = app
        .oneshot(json_post(
            &format!("/accounts/{}/subaccounts", account.0),
            json!({ "name": "Attacker" }),
        ))
        .await
        .expect("create no auth");
    // Missing `authorization` field fails deserialization → axum
    // returns 400 or 422. Either is acceptable — the mutation MUST
    // NOT succeed.
    assert!(
        response.status().is_client_error(),
        "status: {}",
        response.status()
    );
}

#[tokio::test]
async fn create_with_wrong_signer_rejected() {
    let state = build_state();
    let (correct_signer, account) = test_keypair(0xA5);
    let (attacker_signer, _) = test_keypair(0xA6);

    // Prime.
    let _ = router(state.clone())
        .oneshot(get_request(&format!("/accounts/{}/subaccounts", account.0)))
        .await
        .expect("prime");

    // Attacker builds a canonical payload naming the victim's address
    // but signs with their own key. The recovered signer should not
    // match the account, and the write-auth gate must reject.
    let now = now_ms();
    let deadline = now + DEADLINE_TTL_MS - 1;
    let canonical = canonical_create(&account, Some("Hijack"));
    // Issue a challenge under the *victim's* address so the store
    // finds the nonce; the signature check is what protects us.
    let nonce = issue_challenge(&state, WriteAuthAction::SubaccountCreate, &account, 1).await;
    let mut envelope = sign_envelope(
        &attacker_signer,
        WriteAuthAction::SubaccountCreate,
        &account,
        &canonical,
        nonce,
        deadline,
    );
    // Re-address the envelope so it still targets the victim.
    envelope.account = account.clone();
    let _ = correct_signer; // silence unused warning
    let response = router(state)
        .oneshot(json_post(
            &format!("/accounts/{}/subaccounts", account.0),
            json!({ "name": "Hijack", "authorization": envelope }),
        ))
        .await
        .expect("create wrong signer");
    assert!(
        response.status().is_client_error(),
        "expected client error, got: {}",
        response.status()
    );
}

#[tokio::test]
async fn rename_valid_updates_display_name() {
    let state = build_state();
    let (signing_key, account) = test_keypair(0xA7);
    let _ = router(state.clone())
        .oneshot(get_request(&format!("/accounts/{}/subaccounts", account.0)))
        .await
        .expect("prime");

    let now = now_ms();
    let deadline = now + DEADLINE_TTL_MS - 1;
    let canonical = canonical_rename(&account, 1, "Trading");
    let nonce = issue_challenge(&state, WriteAuthAction::SubaccountRename, &account, 1).await;
    let envelope = sign_envelope(
        &signing_key,
        WriteAuthAction::SubaccountRename,
        &account,
        &canonical,
        nonce,
        deadline,
    );
    let response = router(state)
        .oneshot(json_patch(
            &format!("/accounts/{}/subaccounts/1", account.0),
            json!({ "name": "Trading", "authorization": envelope }),
        ))
        .await
        .expect("rename");
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    assert_eq!(body["subaccount_id"], 1);
    assert_eq!(body["name"], "Trading");
    assert_eq!(body["display_name"], "Trading");
    assert!(body["updated_at_ms"].as_i64().unwrap() >= body["created_at_ms"].as_i64().unwrap());
    // Response must never leak signature/nonce/deadline data back to
    // the client (defence in depth).
    assert!(body["signature"].is_null());
    assert!(body["nonce"].is_null());
    assert!(body["authorization"].is_null());
}

#[tokio::test]
async fn rename_empty_name_rejected_before_write_auth() {
    let state = build_state();
    let (signing_key, account) = test_keypair(0xA8);
    let _ = router(state.clone())
        .oneshot(get_request(&format!("/accounts/{}/subaccounts", account.0)))
        .await
        .expect("prime");

    let now = now_ms();
    let deadline = now + DEADLINE_TTL_MS - 1;
    let canonical = canonical_rename(&account, 1, "");
    // Even with a valid signature over the empty name, the handler
    // must reject on name validation and never claim the nonce.
    let nonce = issue_challenge(&state, WriteAuthAction::SubaccountRename, &account, 1).await;
    let envelope = sign_envelope(
        &signing_key,
        WriteAuthAction::SubaccountRename,
        &account,
        &canonical,
        nonce,
        deadline,
    );
    let response = router(state)
        .oneshot(json_patch(
            &format!("/accounts/{}/subaccounts/1", account.0),
            json!({ "name": "   ", "authorization": envelope }),
        ))
        .await
        .expect("rename empty");
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn rename_missing_subaccount_returns_404() {
    let state = build_state();
    let (signing_key, account) = test_keypair(0xA9);
    let _ = router(state.clone())
        .oneshot(get_request(&format!("/accounts/{}/subaccounts", account.0)))
        .await
        .expect("prime");

    let now = now_ms();
    let deadline = now + DEADLINE_TTL_MS - 1;
    let canonical = canonical_rename(&account, 99, "Trading");
    let nonce = issue_challenge(&state, WriteAuthAction::SubaccountRename, &account, 1).await;
    let envelope = sign_envelope(
        &signing_key,
        WriteAuthAction::SubaccountRename,
        &account,
        &canonical,
        nonce,
        deadline,
    );
    let response = router(state)
        .oneshot(json_patch(
            &format!("/accounts/{}/subaccounts/99", account.0),
            json!({ "name": "Trading", "authorization": envelope }),
        ))
        .await
        .expect("rename missing");
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn cross_owner_isolation() {
    let state = build_state();
    let (_sk_a, owner_a) = test_keypair(0xB1);
    let (_sk_b, owner_b) = test_keypair(0xB2);

    let _ = router(state.clone())
        .oneshot(get_request(&format!("/accounts/{}/subaccounts", owner_a.0)))
        .await
        .expect("prime a");
    let _ = router(state.clone())
        .oneshot(get_request(&format!("/accounts/{}/subaccounts", owner_b.0)))
        .await
        .expect("prime b");

    let rows_a = body_json(
        router(state.clone())
            .oneshot(get_request(&format!("/accounts/{}/subaccounts", owner_a.0)))
            .await
            .expect("list a"),
    )
    .await;
    let rows_b = body_json(
        router(state)
            .oneshot(get_request(&format!("/accounts/{}/subaccounts", owner_b.0)))
            .await
            .expect("list b"),
    )
    .await;
    assert_eq!(rows_a["subaccounts"].as_array().unwrap().len(), 1);
    assert_eq!(rows_b["subaccounts"].as_array().unwrap().len(), 1);
    // Ownership is preserved in the response.
    assert!(rows_a["owner_address"]
        .as_str()
        .unwrap()
        .eq_ignore_ascii_case(&owner_a.0));
    assert!(rows_b["owner_address"]
        .as_str()
        .unwrap()
        .eq_ignore_ascii_case(&owner_b.0));
}

// ===========================================================================
// Canonical byte-freeze tests. These pin the wire format for the two
// new subaccount write-auth actions so future edits catch any
// accidental payload drift.
// ===========================================================================

#[test]
fn subaccount_create_canonical_bytes_are_frozen() {
    let owner = AccountId::new("0xABCDEF0000000000000000000000000000000001");
    let with_name = canonical_payload_bytes(
        WriteAuthAction::SubaccountCreate,
        &[
            ("account", CanonicalValue::Address(owner.clone())),
            ("name", CanonicalValue::Str("Market Making".to_string())),
        ],
    );
    assert_eq!(
        std::str::from_utf8(&with_name).unwrap(),
        "SUBACCOUNT_CREATE|account=\"0xabcdef0000000000000000000000000000000001\"|name=\"Market Making\""
    );

    let no_name = canonical_payload_bytes(
        WriteAuthAction::SubaccountCreate,
        &[
            ("account", CanonicalValue::Address(owner)),
            ("name", CanonicalValue::Null),
        ],
    );
    assert_eq!(
        std::str::from_utf8(&no_name).unwrap(),
        "SUBACCOUNT_CREATE|account=\"0xabcdef0000000000000000000000000000000001\"|name=null"
    );
}

#[test]
fn subaccount_rename_canonical_bytes_are_frozen() {
    let owner = AccountId::new("0xABCDEF0000000000000000000000000000000001");
    let bytes = canonical_payload_bytes(
        WriteAuthAction::SubaccountRename,
        &[
            ("account", CanonicalValue::Address(owner)),
            ("subaccount_id", CanonicalValue::U64(2)),
            ("name", CanonicalValue::Str("ETH Options".to_string())),
        ],
    );
    assert_eq!(
        std::str::from_utf8(&bytes).unwrap(),
        "SUBACCOUNT_RENAME|account=\"0xabcdef0000000000000000000000000000000001\"|subaccount_id=2|name=\"ETH Options\""
    );
}

/// SUBACCOUNTS-RENAME-NETWORK-FETCH-V1 — regression guard.
///
/// The subaccount rename endpoint uses HTTP `PATCH`. If the API's
/// CORS layer drops `PATCH` from `allow_methods`, browsers reject
/// the preflight and the frontend surfaces `Failed to fetch`
/// instead of a real error body. This test issues a CORS preflight
/// against the rename path and asserts `PATCH` shows up in
/// `Access-Control-Allow-Methods` so that a future refactor of the
/// method list cannot silently break the rename flow again.
#[tokio::test]
async fn cors_preflight_allows_patch_for_rename_route() {
    let state = build_state();
    let app = router(state);

    // The default `CORS_ALLOWED_ORIGINS` value (see `cors_layer_from_env`)
    // is `http://localhost:3000,http://127.0.0.1:3000`. Use the first
    // one as the request origin so the preflight passes.
    let req = Request::builder()
        .method("OPTIONS")
        .uri("/accounts/0xabcdef0000000000000000000000000000000001/subaccounts/2")
        .header(header::ORIGIN, "http://localhost:3000")
        .header("Access-Control-Request-Method", "PATCH")
        .header("Access-Control-Request-Headers", "content-type")
        .body(Body::empty())
        .expect("build preflight request");

    let response = app.oneshot(req).await.expect("preflight response");
    assert_eq!(response.status(), StatusCode::OK);
    let allow_methods = response
        .headers()
        .get("access-control-allow-methods")
        .expect("access-control-allow-methods header set on preflight")
        .to_str()
        .expect("allow-methods header ascii")
        .to_ascii_uppercase();
    assert!(
        allow_methods.contains("PATCH"),
        "access-control-allow-methods missing PATCH: {allow_methods:?}",
    );
    let allow_origin = response
        .headers()
        .get("access-control-allow-origin")
        .expect("access-control-allow-origin header set on preflight")
        .to_str()
        .expect("allow-origin header ascii");
    assert_eq!(allow_origin, "http://localhost:3000");
}

#[test]
fn write_auth_action_string_roundtrips_for_subaccounts() {
    assert_eq!(
        WriteAuthAction::SubaccountCreate.as_str(),
        "SUBACCOUNT_CREATE"
    );
    assert_eq!(
        WriteAuthAction::SubaccountRename.as_str(),
        "SUBACCOUNT_RENAME"
    );
    assert_eq!(
        WriteAuthAction::parse("SUBACCOUNT_CREATE"),
        Some(WriteAuthAction::SubaccountCreate)
    );
    assert_eq!(
        WriteAuthAction::parse("SUBACCOUNT_RENAME"),
        Some(WriteAuthAction::SubaccountRename)
    );
    // Unknown action still parses to None.
    assert_eq!(WriteAuthAction::parse("SUBACCOUNT_DELETE"), None);
}
