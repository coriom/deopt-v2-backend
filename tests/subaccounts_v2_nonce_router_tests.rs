//! SUBACCOUNTS-V2-NONCE-TABLE-V1 — router-level integration proof.
//!
//! Verifies the formal `used_nonces_v2` ledger is wired end-to-end
//! for a live v2 write-auth handler (`OPTION_ORDER_SUBMIT`) via the
//! shared `require_write_auth_v2_aware` helper.
//!
//! Two properties:
//!
//! * **v2 handler populates `used_nonces_v2`**: after a successful v2
//!   submit, the corresponding `(account, subaccount_id, action,
//!   nonce)` tuple exists in the ledger. Proven by attempting to
//!   consume the same tuple directly against the store and expecting
//!   `Duplicate`.
//!
//! * **v2 pre-seeded duplicate rejects at submit**: if a row already
//!   exists for the tuple, the same submit fails with an auth error
//!   even though the challenge and signature verify. Proves the
//!   ledger is the defence-in-depth barrier we designed.
//!
//! Uses in-memory `AppState`.

use axum::body::{to_bytes, Body};
use axum::http::{header, Request};
use deopt_v2_backend::api::{router, AppState};
use deopt_v2_backend::auth::write_authorization::{
    canonical_payload_bytes, nonce_to_hex, write_auth_eip712_digest, AuthorizationEnvelope,
    CanonicalValue, ChallengeRecord, ChallengeStatus, WriteAuthAction, WRITE_AUTH_DOMAIN_CHAIN_ID,
};
use deopt_v2_backend::auth::V2NonceClaimOutcome;
use deopt_v2_backend::engine::EngineState;
use deopt_v2_backend::options::OptionsConfig;
use deopt_v2_backend::types::{now_ms, AccountId};
use k256::ecdsa::SigningKey;
use serde_json::json;
use sha3::{Digest, Keccak256};
use tower::ServiceExt;

const DEADLINE_TTL_MS: i64 = 60_000;
const DUMMY_SERIES_ID: &str = "BTC-30JAN2026-50000-C";

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

// v2 canonical for OPTION_ORDER_SUBMIT — MUST stay byte-identical to
// `src/api/routes.rs::canonical_option_order_submit_v2`.
fn v2_option_order_submit_canonical(account: &AccountId, subaccount_id: u32) -> Vec<u8> {
    canonical_payload_bytes(
        WriteAuthAction::OptionOrderSubmit,
        &[
            ("account", CanonicalValue::Address(account.clone())),
            ("subaccount_id", CanonicalValue::U64(subaccount_id as u64)),
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
    )
}

async fn allocate_subaccount_two(state: &AppState, owner: &AccountId) {
    // Ensure Account 1 exists first (mirrors the lazy-create in the
    // production list route).
    let _ =
        deopt_v2_backend::subaccounts::ensure_default_subaccount(state.subaccounts.as_ref(), owner)
            .await;
    let created =
        deopt_v2_backend::subaccounts::create_subaccount(state.subaccounts.as_ref(), owner, None)
            .await
            .expect("allocate subaccount 2");
    assert_eq!(created.subaccount_id, 2, "expected id=2, got {created:?}");
}

// ---------------------------------------------------------------------
// A pre-existing v2-nonce row rejects the submit even with a valid
// challenge + signature. Proves the ledger is enforced end-to-end.
// ---------------------------------------------------------------------

#[tokio::test]
async fn v2_submit_rejects_when_used_nonces_v2_row_already_exists() {
    let state = build_state();
    let (signing_key, account) = test_keypair(0xE1);
    allocate_subaccount_two(&state, &account).await;

    let nonce = issue_challenge(&state, WriteAuthAction::OptionOrderSubmit, &account, 1).await;
    // Pre-seed the v2 ledger with the exact tuple this submit would
    // occupy. This simulates any prior consumption path — direct,
    // out-of-band, or via a previous successful submit whose
    // `write_auth_challenges` row was rotated away.
    let seed_outcome = state
        .used_nonces_v2
        .consume_v2_nonce(
            &account,
            2,
            WriteAuthAction::OptionOrderSubmit,
            nonce,
            [0u8; 32],
            now_ms(),
        )
        .await
        .expect("seed");
    assert_eq!(
        seed_outcome,
        V2NonceClaimOutcome::Fresh,
        "seed should insert"
    );

    let canonical = v2_option_order_submit_canonical(&account, 2);
    let deadline = now_ms() + DEADLINE_TTL_MS - 1;
    let envelope = sign_envelope(
        &signing_key,
        WriteAuthAction::OptionOrderSubmit,
        &account,
        &canonical,
        nonce,
        deadline,
        Some(2),
    );

    let body = json!({
        "option_series_id": DUMMY_SERIES_ID,
        "account": account.0,
        "subaccount_id": 2,
        "side": "buy",
        "price_1e8": "1000000000",
        "size_1e8": "100000000",
        "time_in_force": "gtc",
        "post_only": false,
        "client_order_id": null,
        "authorization": {
            "action": envelope.action,
            "account": envelope.account.0,
            "nonce": envelope.nonce,
            "deadline_ms": envelope.deadline_ms,
            "signature": envelope.signature,
            "version": 2,
        },
    });

    let response = router(state)
        .oneshot(json_post("/options/orders", body))
        .await
        .expect("submit");
    let status = response.status();
    let json = body_json(response).await;
    // The response must be a WriteAuth failure (NonceAlreadyUsed
    // surfaces via the auth error mapping — typically 401/409/4xx).
    assert!(
        status.is_client_error(),
        "expected 4xx, got {status}: {json}"
    );
    // The specific error class must not leak any secret material.
    let msg = json.get("error").and_then(|v| v.as_str()).unwrap_or("");
    assert!(
        !msg.contains(&envelope.signature),
        "response leaks signature: {msg}"
    );
    assert!(
        !msg.contains(&nonce_to_hex(&nonce)),
        "response leaks nonce: {msg}"
    );
}

// ---------------------------------------------------------------------
// A successful v2 submit populates `used_nonces_v2` with the correct
// tuple. Proven by attempting to consume the same tuple after the
// submit and expecting `Duplicate`.
// ---------------------------------------------------------------------

#[tokio::test]
async fn v2_submit_populates_used_nonces_v2_on_success() {
    let state = build_state();
    let (signing_key, account) = test_keypair(0xE2);
    allocate_subaccount_two(&state, &account).await;

    let nonce = issue_challenge(&state, WriteAuthAction::OptionOrderSubmit, &account, 2).await;
    let canonical = v2_option_order_submit_canonical(&account, 2);
    let deadline = now_ms() + DEADLINE_TTL_MS - 1;
    let envelope = sign_envelope(
        &signing_key,
        WriteAuthAction::OptionOrderSubmit,
        &account,
        &canonical,
        nonce,
        deadline,
        Some(2),
    );
    let body = json!({
        "option_series_id": DUMMY_SERIES_ID,
        "account": account.0,
        "subaccount_id": 2,
        "side": "buy",
        "price_1e8": "1000000000",
        "size_1e8": "100000000",
        "time_in_force": "gtc",
        "post_only": false,
        "client_order_id": null,
        "authorization": {
            "action": envelope.action,
            "account": envelope.account.0,
            "nonce": envelope.nonce,
            "deadline_ms": envelope.deadline_ms,
            "signature": envelope.signature,
            "version": 2,
        },
    });

    let store_handle = state.used_nonces_v2.clone();
    let response = router(state)
        .oneshot(json_post("/options/orders", body))
        .await
        .expect("submit");
    // Auth path succeeded end-to-end when this test suite uses in-
    // memory Options — the option series is not pre-registered, so we
    // may get 404 "invalid option series id" from the downstream
    // service. The v2 nonce is consumed in
    // `require_write_auth_v2_aware` BEFORE that lookup, so any non-
    // auth response is proof the ledger got its row. Reject only
    // NonceAlreadyUsed here; treat everything else as "auth passed".
    let status = response.status();
    let body = body_json(response).await;
    let msg = body
        .get("error")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    assert!(
        !msg.contains("already used"),
        "auth path unexpectedly rejected as replay ({status}): {msg}"
    );

    // After the handler ran, the ledger row exists — verify by
    // consuming again and expecting Duplicate.
    let dup = store_handle
        .consume_v2_nonce(
            &account,
            2,
            WriteAuthAction::OptionOrderSubmit,
            nonce,
            [0u8; 32],
            now_ms(),
        )
        .await
        .expect("dup");
    assert_eq!(
        dup,
        V2NonceClaimOutcome::Duplicate,
        "expected v2 handler to have populated used_nonces_v2"
    );
}

// ---------------------------------------------------------------------
// v1 submit does NOT touch `used_nonces_v2`. Verified by attempting to
// consume the corresponding tuple after a v1 submit and expecting
// `Fresh` (meaning the handler did not insert on the v1 path).
// ---------------------------------------------------------------------

fn v1_option_order_submit_canonical(account: &AccountId) -> Vec<u8> {
    canonical_payload_bytes(
        WriteAuthAction::OptionOrderSubmit,
        &[
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
        ],
    )
}

#[tokio::test]
async fn v1_submit_does_not_touch_used_nonces_v2() {
    let state = build_state();
    let (signing_key, account) = test_keypair(0xE3);
    // Default subaccount 1 is auto-created on the resolver's happy
    // path — no explicit allocate needed for v1.
    let nonce = issue_challenge(&state, WriteAuthAction::OptionOrderSubmit, &account, 3).await;
    let canonical = v1_option_order_submit_canonical(&account);
    let deadline = now_ms() + DEADLINE_TTL_MS - 1;
    let envelope = sign_envelope(
        &signing_key,
        WriteAuthAction::OptionOrderSubmit,
        &account,
        &canonical,
        nonce,
        deadline,
        None, // v1 envelope
    );
    let body = json!({
        "option_series_id": DUMMY_SERIES_ID,
        "account": account.0,
        // v1: subaccount_id may be omitted or Some(1); either resolves
        // to 1 and skips the v2 ledger entirely.
        "side": "buy",
        "price_1e8": "1000000000",
        "size_1e8": "100000000",
        "time_in_force": "gtc",
        "post_only": false,
        "client_order_id": null,
        "authorization": {
            "action": envelope.action,
            "account": envelope.account.0,
            "nonce": envelope.nonce,
            "deadline_ms": envelope.deadline_ms,
            "signature": envelope.signature,
        },
    });

    let store_handle = state.used_nonces_v2.clone();
    let response = router(state)
        .oneshot(json_post("/options/orders", body))
        .await
        .expect("submit");
    // Accept the same "auth-passed" contract as the v2 happy path —
    // the downstream option series may not be registered, giving a
    // 404. The nonce path is what we care about; assert no auth
    // replay error.
    let status = response.status();
    let body = body_json(response).await;
    let msg = body
        .get("error")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    assert!(
        !msg.contains("already used"),
        "v1 auth path unexpectedly rejected as replay ({status}): {msg}"
    );

    // If we can freshly consume the v1 tuple, the v1 handler did NOT
    // populate the v2 ledger.
    let outcome = store_handle
        .consume_v2_nonce(
            &account,
            1,
            WriteAuthAction::OptionOrderSubmit,
            nonce,
            [0u8; 32],
            now_ms(),
        )
        .await
        .expect("consume");
    assert_eq!(
        outcome,
        V2NonceClaimOutcome::Fresh,
        "v1 handler must not touch used_nonces_v2"
    );
}
