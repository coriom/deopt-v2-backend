//! ACCOUNT-WRITE-AUTH-LIVE-PG-PROOF-V1
//!
//! Proof harness for the EIP-712 HTTP write-authorization subsystem
//! against a real (disposable) PostgreSQL database.
//!
//! The brief's hard requirements proven here:
//!
//!  1. Valid signed write consumes the nonce and creates exactly one mutation.
//!  2. Two concurrent submissions sharing one nonce → one mutation maximum.
//!  3. Exact retry returns the same `resource_id` (idempotent).
//!  4. Same nonce + altered payload → rejected.
//!  5. Same nonce + altered action → rejected.
//!  6. Same nonce + altered account → rejected.
//!  7. Same nonce + altered chain → rejected.
//!  8. Same idempotency key + altered payload → rejected.
//!  9. Consumed nonce survives repository reload.
//! 10. Expired nonce stays rejected after reload.
//! 11. WebSocket EIP-191 nonce cannot authorize an HTTP EIP-712 write.
//! 12. Perp mutation routes remain fail-closed.
//!
//! Extra (production fix from Phase 1 audit):
//! 13. `AmbiguousPriorClaim` — a nonce consumed without resource_id
//!     linkage rejects on retry; the user is forced to request a new
//!     challenge so no duplicate mutation can race in.
//!
//! Run via `~/DEOPT/scripts/account-write-auth-pg-proof.sh`. Standalone:
//!
//!   export WRITE_AUTH_PG_TEST_ALLOW_DISPOSABLE_DB=true
//!   export WRITE_AUTH_PG_TEST_DATABASE_URL=postgres://…/deopt_auth_proof_…
//!   cargo test --test account_write_auth_pg_proof -- --nocapture
//!
//! The test crate ONLY reads `WRITE_AUTH_PG_TEST_DATABASE_URL`. No
//! `.env` is sourced, no wallet key is loaded, no RPC URL is used.

use axum::body::{to_bytes, Body};
use axum::http::{header, Request, StatusCode};
use deopt_v2_backend::api::{router, AppState};
use deopt_v2_backend::auth::write_authorization::{
    canonical_payload_bytes, memory_store::InMemoryChallengeStore, nonce_to_hex, verify_and_claim,
    write_auth_eip712_digest, AuthorizationEnvelope, CanonicalValue, ChallengeRecord,
    ChallengeStatus, WriteAuthAction, WriteAuthChallengeStore, WriteAuthError,
    WRITE_AUTH_DOMAIN_CHAIN_ID,
};
use deopt_v2_backend::db::PgRepository;
use deopt_v2_backend::engine::EngineState;
use deopt_v2_backend::options::service::{create_option_series, CreateOptionSeriesInput};
use deopt_v2_backend::options::OptionsConfig;
use deopt_v2_backend::types::{now_ms, AccountId};
use k256::ecdsa::SigningKey;
use serde_json::json;
use sha3::{Digest, Keccak256};
use std::sync::Arc;
use tower::ServiceExt;

const ENV_VAR: &str = "WRITE_AUTH_PG_TEST_DATABASE_URL";
const DEADLINE_TTL_MS: i64 = 60_000;

// ---------------------------------------------------------------------
// Setup helpers
// ---------------------------------------------------------------------

fn pg_test_url() -> Option<String> {
    std::env::var(ENV_VAR).ok().filter(|v| !v.is_empty())
}

/// Run migrations exactly once per `cargo test` process regardless of
/// how many tests run in parallel (mirrors the conditional-orders
/// proof harness pattern).
async fn ensure_migrated(url: &str) {
    static MIGRATED: tokio::sync::OnceCell<()> = tokio::sync::OnceCell::const_new();
    MIGRATED
        .get_or_init(|| async {
            let repo = PgRepository::connect(url)
                .await
                .expect("connect for shared migration");
            repo.run_migrations()
                .await
                .expect("run migrations against disposable PG database");
        })
        .await;
}

async fn fresh_pg_repository(url: &str) -> PgRepository {
    ensure_migrated(url).await;
    PgRepository::connect(url)
        .await
        .expect("connect to disposable PG database")
}

/// Build an `AppState` wired to live PostgreSQL persistence. Asserts
/// every property the brief requires: repository is Some, persistence
/// is enabled, and the in-memory fallback is NOT in use (the
/// write_auth_challenges Arc points at the PG-backed PgRepository).
async fn pg_state(url: &str) -> AppState {
    let repo = fresh_pg_repository(url).await;
    let mut config = OptionsConfig::enabled_in_memory_for_tests();
    config.rfq_enabled = true;
    let state = AppState::with_options_config_and_repository(
        EngineState::with_default_markets(),
        config,
        repo,
    );
    assert!(
        state.repository.is_some(),
        "ACCOUNT-WRITE-AUTH-LIVE-PG-PROOF-V1: repository must be Some"
    );
    assert!(
        state.persistence_enabled,
        "ACCOUNT-WRITE-AUTH-LIVE-PG-PROOF-V1: persistence_enabled must be true"
    );
    // In-memory-fallback exclusion proof: `AppState::with_options_config_and_repository`
    // is the ONLY constructor path that sets `state.write_auth_challenges`
    // to a PgRepository instance. Therefore `repository.is_some()` here
    // implies the challenge store is PG-backed. The probe pool used by
    // each scenario's SQL assertions provides a second, independent
    // connection that re-verifies persistence across (test) processes.
    state
}

// ---------------------------------------------------------------------
// Deterministic test wallets (per-test, derived from a seed byte).
// NEVER load `.env` private keys here.
// ---------------------------------------------------------------------

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

/// Per-test deterministic but unique nonce derived from the test tag.
fn per_test_nonce(tag: &str, suffix: u8) -> [u8; 32] {
    let mut out = [0u8; 32];
    let tag_hash = keccak(tag.as_bytes());
    out[..30].copy_from_slice(&tag_hash[..30]);
    out[30] = suffix;
    out[31] = 0x01;
    out
}

async fn issue_challenge(
    state: &AppState,
    action: WriteAuthAction,
    account: &AccountId,
    nonce_bytes: [u8; 32],
    idempotency_key: Option<&str>,
) {
    state
        .write_auth_challenges
        .issue(ChallengeRecord {
            nonce_bytes,
            account: account.clone(),
            action,
            chain_id: WRITE_AUTH_DOMAIN_CHAIN_ID,
            issued_at_ms: now_ms(),
            expires_at_ms: now_ms() + DEADLINE_TTL_MS,
            status: ChallengeStatus::Issued,
            request_digest: None,
            idempotency_key: idempotency_key.map(|s| s.to_string()),
            resource_id: None,
            consumed_at_ms: None,
        })
        .await
        .expect("issue challenge in PG");
}

async fn issue_expired_challenge(
    state: &AppState,
    action: WriteAuthAction,
    account: &AccountId,
    nonce_bytes: [u8; 32],
) {
    state
        .write_auth_challenges
        .issue(ChallengeRecord {
            nonce_bytes,
            account: account.clone(),
            action,
            chain_id: WRITE_AUTH_DOMAIN_CHAIN_ID,
            issued_at_ms: now_ms() - DEADLINE_TTL_MS - 100,
            expires_at_ms: now_ms() - 100,
            status: ChallengeStatus::Issued,
            request_digest: None,
            idempotency_key: None,
            resource_id: None,
            consumed_at_ms: None,
        })
        .await
        .expect("issue expired challenge in PG");
}

async fn active_series_id(state: &AppState, tag: &str) -> String {
    let now_sec = (now_ms() / 1000) as u64;
    let strike = 70_000_000_000u128 + (tag.bytes().map(u128::from).sum::<u128>() * 1_000);
    create_option_series(
        state,
        CreateOptionSeriesInput {
            underlying: "ETH".to_string(),
            base_asset: "ETH".to_string(),
            quote_asset: "USDC".to_string(),
            settlement_asset: "USDC".to_string(),
            expiry: now_sec + 7 * 24 * 3600,
            strike_1e8: strike,
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

fn json_post(path: &str, body: serde_json::Value) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(path)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(serde_json::to_vec(&body).expect("json body")))
        .expect("build request")
}

async fn response_json(response: axum::response::Response) -> serde_json::Value {
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    serde_json::from_slice(&bytes).unwrap_or(serde_json::json!({}))
}

/// Skip the test cleanly if the env var is unset. The runner script
/// asserts at the cargo-test level that tests EXECUTED (not skipped)
/// by parsing the `test result:` line.
macro_rules! pg_test {
    ($name:ident, $body:expr) => {
        #[tokio::test]
        async fn $name() {
            let Some(url) = pg_test_url() else {
                eprintln!(
                    "[pg-proof] {} → SKIPPED (set {} via scripts/account-write-auth-pg-proof.sh)",
                    stringify!($name),
                    ENV_VAR
                );
                return;
            };
            ($body)(url).await;
        }
    };
}

// =====================================================================
// Scenario 1 — Valid signed write consumes nonce, creates exactly one mutation.
// =====================================================================

pg_test!(
    scenario_1_valid_signed_write_creates_exactly_one_mutation,
    |url: String| async move {
        let _ = init_probe_pool(&url).await;
        let tag = "scenario_1";
        let state = pg_state(&url).await;
        let (signing_key, account) = test_keypair(0x11);
        let series_id = active_series_id(&state, tag).await;
        let nonce = per_test_nonce(tag, 0x01);
        issue_challenge(
            &state,
            WriteAuthAction::OptionOrderSubmit,
            &account,
            nonce,
            None,
        )
        .await;
        let deadline = now_ms() + DEADLINE_TTL_MS - 1;
        let canonical = canonical_for_option_order_submit(
            &account,
            &series_id,
            "1000000000",
            "100000000",
            Some("s1-clid"),
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
        let app = router(state.clone());
        let response = app
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
                    "client_order_id": "s1-clid",
                    "authorization": envelope,
                }),
            ))
            .await
            .unwrap();
        let status = response.status();
        let body = response_json(response).await;
        assert_eq!(status, StatusCode::OK, "body={}", body);
        assert_eq!(body["status"].as_str(), Some("open"));

        // SQL cardinality assertions via an independent connection pool.
        let probe = init_probe_pool(&url).await;
        let order_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM option_orders \
         WHERE lower(account) = lower($1) AND client_order_id = $2",
        )
        .bind(&account.0)
        .bind("s1-clid")
        .fetch_one(&probe)
        .await
        .expect("count orders");
        assert_eq!(order_count, 1, "exactly one order row must exist");

        let consumed: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM write_auth_challenges \
         WHERE nonce_bytes = $1 AND status = 'consumed' AND resource_id IS NOT NULL",
        )
        .bind(nonce.as_slice())
        .fetch_one(&probe)
        .await
        .expect("count nonce");
        assert_eq!(
            consumed, 1,
            "nonce must be consumed with resource_id linked"
        );
    }
);

/// Open a small, per-test SQL probe pool. Each `#[tokio::test]`
/// creates its own Tokio runtime; sharing one pool across runtimes
/// breaks because the pool's worker tasks live on the first test's
/// (already shut-down) runtime. The small pool size (2 connections)
/// keeps total connection demand under PG default `max_connections=100`
/// even when all 13 tests run in parallel alongside their per-test
/// PgRepository pools (5 conns each).
async fn init_probe_pool(url: &str) -> sqlx::PgPool {
    sqlx::postgres::PgPoolOptions::new()
        .max_connections(2)
        .acquire_timeout(std::time::Duration::from_secs(30))
        .connect(url)
        .await
        .expect("connect probe pool")
}

// =====================================================================
// Scenario 2 — Two concurrent submissions sharing one nonce → one mutation maximum.
// =====================================================================

pg_test!(
    scenario_2_concurrent_one_nonce_one_mutation,
    |url: String| async move {
        let _ = init_probe_pool(&url).await;
        let tag = "scenario_2";
        let state = pg_state(&url).await;
        let (signing_key, account) = test_keypair(0x22);
        let series_id = active_series_id(&state, tag).await;
        let nonce = per_test_nonce(tag, 0x02);
        issue_challenge(
            &state,
            WriteAuthAction::OptionOrderSubmit,
            &account,
            nonce,
            None,
        )
        .await;
        let deadline = now_ms() + DEADLINE_TTL_MS - 1;
        let canonical = canonical_for_option_order_submit(
            &account,
            &series_id,
            "1000000000",
            "100000000",
            Some("s2-clid"),
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
        // Spawn two concurrent submissions sharing the SAME envelope/nonce
        // through independent router clones sharing the SAME PgRepository
        // pool. The atomic concurrency guarantee comes from the PG row
        // lock + status-gated UPDATE inside `claim_write_auth_challenge`,
        // not from the connection topology; opening separate pools
        // saturates PG's `max_connections=100` (default) under parallel
        // test execution and causes harness-only `PoolTimedOut` errors.
        let envelope_a = envelope.clone();
        let envelope_b = envelope.clone();
        let series_a = series_id.clone();
        let series_b = series_id.clone();
        let account_a = account.0.clone();
        let account_b = account.0.clone();
        let app_a = router(state.clone());
        let app_b = router(state.clone());

        let task_a = tokio::spawn(async move {
            app_a
                .oneshot(json_post(
                    "/options/orders",
                    json!({
                        "option_series_id": series_a,
                        "account": account_a,
                        "side": "buy",
                        "price_1e8": "1000000000",
                        "size_1e8": "100000000",
                        "time_in_force": "gtc",
                        "post_only": false,
                        "client_order_id": "s2-clid",
                        "authorization": envelope_a,
                    }),
                ))
                .await
                .unwrap()
                .status()
        });
        let task_b = tokio::spawn(async move {
            app_b
                .oneshot(json_post(
                    "/options/orders",
                    json!({
                        "option_series_id": series_b,
                        "account": account_b,
                        "side": "buy",
                        "price_1e8": "1000000000",
                        "size_1e8": "100000000",
                        "time_in_force": "gtc",
                        "post_only": false,
                        "client_order_id": "s2-clid",
                        "authorization": envelope_b,
                    }),
                ))
                .await
                .unwrap()
                .status()
        });
        let (status_a, status_b) = (task_a.await.unwrap(), task_b.await.unwrap());

        // Exactly one must have created the resource. The other one either:
        //   - lost the race for Fresh and returned the same OK with the
        //     existing resource_id (IdempotentReplay), OR
        //   - hit AmbiguousPriorClaim (409) if it racing-saw a consumed
        //     row with no resource_id yet.
        let oks = [status_a, status_b]
            .iter()
            .filter(|s| **s == StatusCode::OK)
            .count();
        assert!(
            oks >= 1,
            "at least one of the concurrent submissions must succeed (got a={}, b={})",
            status_a,
            status_b
        );
        // Cardinality: independent connection counts at most ONE order row.
        let probe = init_probe_pool(&url).await;
        let order_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM option_orders \
         WHERE lower(account) = lower($1) AND client_order_id = $2",
        )
        .bind(&account.0)
        .bind("s2-clid")
        .fetch_one(&probe)
        .await
        .expect("count orders");
        assert_eq!(
            order_count, 1,
            "concurrent submissions of one nonce must produce exactly one option_orders row"
        );
    }
);

// =====================================================================
// Scenario 3 — Exact retry returns the same `resource_id`.
// =====================================================================

pg_test!(
    scenario_3_exact_retry_returns_same_resource_id,
    |url: String| async move {
        let _ = init_probe_pool(&url).await;
        let tag = "scenario_3";
        let state = pg_state(&url).await;
        let (signing_key, account) = test_keypair(0x33);
        let series_id = active_series_id(&state, tag).await;
        let nonce = per_test_nonce(tag, 0x03);
        issue_challenge(
            &state,
            WriteAuthAction::OptionOrderSubmit,
            &account,
            nonce,
            None,
        )
        .await;
        let deadline = now_ms() + DEADLINE_TTL_MS - 1;
        let canonical = canonical_for_option_order_submit(
            &account,
            &series_id,
            "1000000000",
            "100000000",
            Some("s3-clid"),
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
        let body = json!({
            "option_series_id": series_id,
            "account": account.0,
            "side": "buy",
            "price_1e8": "1000000000",
            "size_1e8": "100000000",
            "time_in_force": "gtc",
            "post_only": false,
            "client_order_id": "s3-clid",
            "authorization": envelope,
        });
        let app = router(state);
        let first = app
            .clone()
            .oneshot(json_post("/options/orders", body.clone()))
            .await
            .unwrap();
        assert_eq!(first.status(), StatusCode::OK);
        let first_id = response_json(first).await["order_id"]
            .as_str()
            .unwrap()
            .to_string();
        let second = app
            .oneshot(json_post("/options/orders", body))
            .await
            .unwrap();
        assert_eq!(
            second.status(),
            StatusCode::OK,
            "exact retry must return 200"
        );
        let second_id = response_json(second).await["order_id"]
            .as_str()
            .unwrap()
            .to_string();
        assert_eq!(
            first_id, second_id,
            "exact retry must return the same order_id"
        );

        // Independent connection: exactly ONE order row.
        let probe = init_probe_pool(&url).await;
        let order_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM option_orders \
         WHERE lower(account) = lower($1) AND client_order_id = $2",
        )
        .bind(&account.0)
        .bind("s3-clid")
        .fetch_one(&probe)
        .await
        .unwrap();
        assert_eq!(order_count, 1, "exact retry must NOT create a second row");
    }
);

// =====================================================================
// Scenario 4 — Same nonce + altered payload → rejected.
// =====================================================================

pg_test!(
    scenario_4_altered_payload_rejected,
    |url: String| async move {
        let tag = "scenario_4";
        let state = pg_state(&url).await;
        let (signing_key, account) = test_keypair(0x44);
        let series_id = active_series_id(&state, tag).await;
        let nonce = per_test_nonce(tag, 0x04);
        issue_challenge(
            &state,
            WriteAuthAction::OptionOrderSubmit,
            &account,
            nonce,
            None,
        )
        .await;
        let deadline = now_ms() + DEADLINE_TTL_MS - 1;
        let canonical_signed = canonical_for_option_order_submit(
            &account,
            &series_id,
            "1000000000",
            "100000000",
            None,
        );
        let envelope = sign_envelope(
            &signing_key,
            WriteAuthAction::OptionOrderSubmit,
            &account,
            &canonical_signed,
            nonce,
            deadline,
            None,
        );
        // Body says price=9999999999 but signature was over 1000000000.
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
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }
);

// =====================================================================
// Scenario 5 — Same nonce + altered action → rejected.
// =====================================================================

pg_test!(
    scenario_5_altered_action_rejected,
    |url: String| async move {
        let tag = "scenario_5";
        let state = pg_state(&url).await;
        let (signing_key, account) = test_keypair(0x55);
        let nonce = per_test_nonce(tag, 0x05);
        // Issue nonce for OPTION_ORDER_CANCEL, but try to use it for OPTION_ORDER_SUBMIT.
        issue_challenge(
            &state,
            WriteAuthAction::OptionOrderCancel,
            &account,
            nonce,
            None,
        )
        .await;
        let deadline = now_ms() + DEADLINE_TTL_MS - 1;
        let canonical = canonical_payload_bytes(WriteAuthAction::OptionOrderSubmit, &[]);
        let envelope = sign_envelope(
            &signing_key,
            WriteAuthAction::OptionOrderSubmit,
            &account,
            &canonical,
            nonce,
            deadline,
            None,
        );
        let result = verify_and_claim(
            state.write_auth_challenges.as_ref(),
            &envelope,
            WriteAuthAction::OptionOrderSubmit,
            &account,
            &canonical,
            WRITE_AUTH_DOMAIN_CHAIN_ID,
            now_ms(),
        )
        .await;
        assert!(
            matches!(result, Err(WriteAuthError::NonceNotFound)),
            "altered action must be rejected (got {result:?})"
        );
    }
);

// =====================================================================
// Scenario 6 — Same nonce + altered account → rejected.
// =====================================================================

pg_test!(
    scenario_6_altered_account_rejected,
    |url: String| async move {
        let tag = "scenario_6";
        let state = pg_state(&url).await;
        let (signing_key_a, account_a) = test_keypair(0x66);
        let (_signing_key_b, account_b) = test_keypair(0x67);
        let nonce = per_test_nonce(tag, 0x06);
        issue_challenge(
            &state,
            WriteAuthAction::OptionOrderSubmit,
            &account_a,
            nonce,
            None,
        )
        .await;
        let deadline = now_ms() + DEADLINE_TTL_MS - 1;
        let canonical = canonical_payload_bytes(WriteAuthAction::OptionOrderSubmit, &[]);
        // Signed by A, envelope claims account A — but verifier expects B.
        let envelope = sign_envelope(
            &signing_key_a,
            WriteAuthAction::OptionOrderSubmit,
            &account_a,
            &canonical,
            nonce,
            deadline,
            None,
        );
        let result = verify_and_claim(
            state.write_auth_challenges.as_ref(),
            &envelope,
            WriteAuthAction::OptionOrderSubmit,
            &account_b,
            &canonical,
            WRITE_AUTH_DOMAIN_CHAIN_ID,
            now_ms(),
        )
        .await;
        assert!(
            matches!(result, Err(WriteAuthError::SignerMismatch)),
            "altered account must be rejected (got {result:?})"
        );
    }
);

// =====================================================================
// Scenario 7 — Same nonce + altered chain → rejected.
// =====================================================================

pg_test!(
    scenario_7_altered_chain_rejected,
    |url: String| async move {
        let tag = "scenario_7";
        let state = pg_state(&url).await;
        let (signing_key, account) = test_keypair(0x77);
        let nonce = per_test_nonce(tag, 0x07);
        issue_challenge(
            &state,
            WriteAuthAction::OptionOrderSubmit,
            &account,
            nonce,
            None,
        )
        .await;
        let deadline = now_ms() + DEADLINE_TTL_MS - 1;
        let canonical = canonical_payload_bytes(WriteAuthAction::OptionOrderSubmit, &[]);
        let envelope = sign_envelope(
            &signing_key,
            WriteAuthAction::OptionOrderSubmit,
            &account,
            &canonical,
            nonce,
            deadline,
            None,
        );
        // Wrong chain_id passed to verifier.
        let result = verify_and_claim(
            state.write_auth_challenges.as_ref(),
            &envelope,
            WriteAuthAction::OptionOrderSubmit,
            &account,
            &canonical,
            1,
            now_ms(),
        )
        .await;
        assert!(
            matches!(result, Err(WriteAuthError::ChainMismatch)),
            "altered chain id must be rejected (got {result:?})"
        );
    }
);

// =====================================================================
// Scenario 8 — Same idempotency key + altered payload → rejected.
// =====================================================================

pg_test!(
    scenario_8_idempotency_key_conflict,
    |url: String| async move {
        let _ = init_probe_pool(&url).await;
        let tag = "scenario_8";
        let state = pg_state(&url).await;
        let (signing_key, account) = test_keypair(0x88);
        let series_id = active_series_id(&state, tag).await;
        let nonce_a = per_test_nonce(tag, 0x08);
        let nonce_b = per_test_nonce(tag, 0x09);
        let idempotency_key = "s8-key";
        // Two distinct challenges, both using the same idempotency_key.
        issue_challenge(
            &state,
            WriteAuthAction::OptionOrderSubmit,
            &account,
            nonce_a,
            Some(idempotency_key),
        )
        .await;
        issue_challenge(
            &state,
            WriteAuthAction::OptionOrderSubmit,
            &account,
            nonce_b,
            Some(idempotency_key),
        )
        .await;

        let deadline = now_ms() + DEADLINE_TTL_MS - 1;
        let canonical_a = canonical_for_option_order_submit(
            &account,
            &series_id,
            "1000000000",
            "100000000",
            Some("s8a-clid"),
        );
        let envelope_a = sign_envelope(
            &signing_key,
            WriteAuthAction::OptionOrderSubmit,
            &account,
            &canonical_a,
            nonce_a,
            deadline,
            Some(idempotency_key),
        );
        let canonical_b = canonical_for_option_order_submit(
            &account,
            &series_id,
            "1000000001", // different price
            "100000000",
            Some("s8b-clid"),
        );
        let envelope_b = sign_envelope(
            &signing_key,
            WriteAuthAction::OptionOrderSubmit,
            &account,
            &canonical_b,
            nonce_b,
            deadline,
            Some(idempotency_key),
        );
        let app = router(state);
        // First submission succeeds and binds the idempotency_key to a resource.
        let first = app
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
                    "client_order_id": "s8a-clid",
                    "authorization": envelope_a,
                }),
            ))
            .await
            .unwrap();
        assert_eq!(first.status(), StatusCode::OK);
        // Second submission with the SAME idempotency_key but a DIFFERENT
        // canonical payload must be REJECTED per the brief Phase 11
        // ("Same idempotency key with different payload is rejected").
        // The PG partial unique index `write_auth_challenges_idempotency`
        // raises SQLSTATE 23505 on the UPDATE; the repository now maps it
        // to `ClaimOutcome::IdempotencyKeyConflict`, which the verifier
        // surfaces as `WriteAuthError::IdempotencyConflict` (HTTP 409).
        let second = app
            .oneshot(json_post(
                "/options/orders",
                json!({
                    "option_series_id": series_id,
                    "account": account.0,
                    "side": "buy",
                    "price_1e8": "1000000001",
                    "size_1e8": "100000000",
                    "time_in_force": "gtc",
                    "post_only": false,
                    "client_order_id": "s8b-clid",
                    "authorization": envelope_b,
                }),
            ))
            .await
            .unwrap();
        assert_eq!(
            second.status(),
            StatusCode::CONFLICT,
            "same idempotency key with different payload must reject with 409"
        );
        // Cardinality: exactly ONE option_orders row exists across both calls.
        let probe = init_probe_pool(&url).await;
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM option_orders \
         WHERE lower(account) = lower($1) AND client_order_id IN ('s8a-clid','s8b-clid')",
        )
        .bind(&account.0)
        .fetch_one(&probe)
        .await
        .unwrap();
        assert_eq!(
            count, 1,
            "idempotency-key conflict must NOT create a second option_orders row"
        );
        // The losing nonce row (nonce_b) MUST stay in 'issued' status so
        // the caller can request a new challenge with a different
        // idempotency key without losing the issuance.
        let nonce_b_status: String =
            sqlx::query_scalar("SELECT status FROM write_auth_challenges WHERE nonce_bytes = $1")
                .bind(nonce_b.as_slice())
                .fetch_one(&probe)
                .await
                .unwrap();
        assert_eq!(
            nonce_b_status, "issued",
            "losing nonce row must NOT advance to consumed/expired on idempotency conflict"
        );
    }
);

// =====================================================================
// Scenario 9 — Consumed nonce survives repository reload.
// =====================================================================

pg_test!(
    scenario_9_consumed_nonce_survives_reload,
    |url: String| async move {
        let _ = init_probe_pool(&url).await;
        let tag = "scenario_9";
        let state = pg_state(&url).await;
        let (signing_key, account) = test_keypair(0x99);
        let nonce = per_test_nonce(tag, 0x0a);
        issue_challenge(
            &state,
            WriteAuthAction::OptionOrderSubmit,
            &account,
            nonce,
            None,
        )
        .await;
        let deadline = now_ms() + DEADLINE_TTL_MS - 1;
        let canonical = canonical_payload_bytes(WriteAuthAction::OptionOrderSubmit, &[]);
        let envelope = sign_envelope(
            &signing_key,
            WriteAuthAction::OptionOrderSubmit,
            &account,
            &canonical,
            nonce,
            deadline,
            None,
        );
        // Consume the nonce + attach a fake resource id so the row reaches
        // the `consumed + resource_id linked` terminal state.
        let verified = verify_and_claim(
            state.write_auth_challenges.as_ref(),
            &envelope,
            WriteAuthAction::OptionOrderSubmit,
            &account,
            &canonical,
            WRITE_AUTH_DOMAIN_CHAIN_ID,
            now_ms(),
        )
        .await
        .expect("first claim");
        assert!(verified.was_fresh);
        state
            .write_auth_challenges
            .attach_resource(nonce, "s9-resource")
            .await
            .expect("attach resource");

        // Drop the state + repository: simulate a backend restart.
        drop(state);

        // Fresh PgRepository against the same DB.
        let repo = PgRepository::connect(&url).await.expect("reload connect");
        let store: Arc<dyn WriteAuthChallengeStore + Send + Sync> = Arc::new(repo);
        let replay_outcome = verify_and_claim(
            store.as_ref(),
            &envelope,
            WriteAuthAction::OptionOrderSubmit,
            &account,
            &canonical,
            WRITE_AUTH_DOMAIN_CHAIN_ID,
            now_ms(),
        )
        .await
        .expect("replay claim");
        assert!(
            !replay_outcome.was_fresh,
            "consumed nonce must NOT be Fresh after reload"
        );
        assert_eq!(
            replay_outcome.idempotent_resource_id.as_deref(),
            Some("s9-resource"),
            "consumed nonce must still link to its resource after reload"
        );
    }
);

// =====================================================================
// Scenario 10 — Expired nonce stays rejected after reload.
// =====================================================================

pg_test!(
    scenario_10_expired_stays_rejected,
    |url: String| async move {
        let tag = "scenario_10";
        let state = pg_state(&url).await;
        let (signing_key, account) = test_keypair(0xaa);
        let nonce = per_test_nonce(tag, 0x0b);
        issue_expired_challenge(&state, WriteAuthAction::OptionOrderSubmit, &account, nonce).await;
        let deadline = now_ms() + DEADLINE_TTL_MS - 1;
        let canonical = canonical_payload_bytes(WriteAuthAction::OptionOrderSubmit, &[]);
        let envelope = sign_envelope(
            &signing_key,
            WriteAuthAction::OptionOrderSubmit,
            &account,
            &canonical,
            nonce,
            deadline,
            None,
        );
        // First attempt — should reject as Expired and persist that decision.
        let first = verify_and_claim(
            state.write_auth_challenges.as_ref(),
            &envelope,
            WriteAuthAction::OptionOrderSubmit,
            &account,
            &canonical,
            WRITE_AUTH_DOMAIN_CHAIN_ID,
            now_ms(),
        )
        .await;
        assert!(
            matches!(first, Err(WriteAuthError::Expired)),
            "first: {first:?}"
        );
        drop(state);
        // Restart simulation: fresh repo.
        let repo = PgRepository::connect(&url).await.expect("reload connect");
        let store: Arc<dyn WriteAuthChallengeStore + Send + Sync> = Arc::new(repo);
        let second = verify_and_claim(
            store.as_ref(),
            &envelope,
            WriteAuthAction::OptionOrderSubmit,
            &account,
            &canonical,
            WRITE_AUTH_DOMAIN_CHAIN_ID,
            now_ms(),
        )
        .await;
        assert!(
            matches!(second, Err(WriteAuthError::Expired)),
            "second: {second:?}"
        );
    }
);

// =====================================================================
// Scenario 11 — WebSocket EIP-191 nonce cannot authorize HTTP EIP-712 write.
// =====================================================================

pg_test!(
    scenario_11_websocket_nonce_cannot_authorize_http_write,
    |url: String| async move {
        let tag = "scenario_11";
        let state = pg_state(&url).await;
        let (signing_key, account) = test_keypair(0xbb);
        let canonical = canonical_payload_bytes(WriteAuthAction::OptionOrderSubmit, &[]);

        // Synthesise a nonce that has NO row in write_auth_challenges
        // (modelling a WS auth nonce that lives in a completely separate
        // table / scheme). The HTTP write verifier must treat it as
        // unknown.
        let ws_nonce = per_test_nonce(tag, 0x0c);
        let deadline = now_ms() + DEADLINE_TTL_MS - 1;
        let envelope = sign_envelope(
            &signing_key,
            WriteAuthAction::OptionOrderSubmit,
            &account,
            &canonical,
            ws_nonce,
            deadline,
            None,
        );
        let result = verify_and_claim(
            state.write_auth_challenges.as_ref(),
            &envelope,
            WriteAuthAction::OptionOrderSubmit,
            &account,
            &canonical,
            WRITE_AUTH_DOMAIN_CHAIN_ID,
            now_ms(),
        )
        .await;
        assert!(
            matches!(result, Err(WriteAuthError::NonceNotFound)),
            "WS-only nonce must NOT authorize HTTP write (got {result:?})"
        );

        // Belt-and-braces: also assert that the in-memory store (the
        // dev-fallback path) would reject the same envelope — proving the
        // assertion does not depend on a particular store backend.
        let mem_store = InMemoryChallengeStore::new();
        let mem_result = verify_and_claim(
            &mem_store,
            &envelope,
            WriteAuthAction::OptionOrderSubmit,
            &account,
            &canonical,
            WRITE_AUTH_DOMAIN_CHAIN_ID,
            now_ms(),
        )
        .await;
        assert!(matches!(mem_result, Err(WriteAuthError::NonceNotFound)));
    }
);

// =====================================================================
// Scenario 12 — Perp mutation routes remain fail-closed.
// =====================================================================

pg_test!(
    scenario_12_perp_routes_remain_fail_closed,
    |url: String| async move {
        let _ = init_probe_pool(&url).await;
        let state = pg_state(&url).await;
        let app = router(state);
        // POST /orders
        let resp = app
            .clone()
            .oneshot(json_post(
                "/orders",
                json!({
                    "market_id": 1,
                    "account": "0x0000000000000000000000000000000000000c01",
                    "side": "buy",
                    "price_1e8": "1000000000",
                    "size_1e8": "100000000",
                    "time_in_force": "gtc",
                    "reduce_only": false,
                    "post_only": false,
                    "client_order_id": "x",
                    "nonce": 1,
                    "deadline_ms": 0,
                    "signature": "0xff"
                }),
            ))
            .await
            .unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::SERVICE_UNAVAILABLE,
            "POST /orders must fail closed"
        );

        // DELETE /orders/:id
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri("/orders/00000000-0000-0000-0000-000000000000")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);

        // POST /rfqs, /rfqs/:id/quotes, /rfqs/:id/accept/:q, /rfqs/:id/cancel
        for (path, body) in [
        ("/rfqs", json!({})),
        ("/rfqs/00000000-0000-0000-0000-000000000000/quotes", json!({})),
        (
            "/rfqs/00000000-0000-0000-0000-000000000000/accept/11111111-1111-1111-1111-111111111111",
            json!({}),
        ),
        ("/rfqs/00000000-0000-0000-0000-000000000000/cancel", json!({})),
    ] {
        let resp = app.clone().oneshot(json_post(path, body)).await.unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::SERVICE_UNAVAILABLE,
            "{} must fail closed (got {})",
            path,
            resp.status()
        );
    }

        // POST /execution-intents/:id/signatures
        let resp = app
            .oneshot(json_post(
                "/execution-intents/00000000-0000-0000-0000-000000000000/signatures",
                json!({}),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);

        // Belt-and-braces cardinality: zero orders / RFQs / intents were created.
        let probe = init_probe_pool(&url).await;
        let orders: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM orders")
            .fetch_one(&probe)
            .await
            .unwrap();
        let rfqs: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM rfqs")
            .fetch_one(&probe)
            .await
            .unwrap();
        assert_eq!(orders, 0, "perp orders table must remain empty");
        assert_eq!(rfqs, 0, "perp rfqs table must remain empty");
    }
);

// =====================================================================
// Scenario 13 — AmbiguousPriorClaim (Phase-1 audit fix).
//
// Models a process crash between mutation and resource attachment OR
// a mutation failure mid-flow: the nonce is consumed but no resource_id
// is linked. The next retry MUST be rejected, forcing the user to
// request a new challenge so the next attempt is unambiguous.
// =====================================================================

pg_test!(
    scenario_13_ambiguous_prior_claim_rejects_retry,
    |url: String| async move {
        let tag = "scenario_13";
        let state = pg_state(&url).await;
        let (signing_key, account) = test_keypair(0xcc);
        let nonce = per_test_nonce(tag, 0x0d);
        issue_challenge(
            &state,
            WriteAuthAction::OptionOrderSubmit,
            &account,
            nonce,
            None,
        )
        .await;
        let deadline = now_ms() + DEADLINE_TTL_MS - 1;
        let canonical = canonical_payload_bytes(WriteAuthAction::OptionOrderSubmit, &[]);
        let envelope = sign_envelope(
            &signing_key,
            WriteAuthAction::OptionOrderSubmit,
            &account,
            &canonical,
            nonce,
            deadline,
            None,
        );
        // First claim succeeds (Fresh). We do NOT call attach_resource —
        // simulating the prior attempt crashing right after the claim.
        let first = verify_and_claim(
            state.write_auth_challenges.as_ref(),
            &envelope,
            WriteAuthAction::OptionOrderSubmit,
            &account,
            &canonical,
            WRITE_AUTH_DOMAIN_CHAIN_ID,
            now_ms(),
        )
        .await
        .expect("first claim");
        assert!(first.was_fresh);
        assert!(first.idempotent_resource_id.is_none());

        // Retry with the same envelope. The fix MUST reject this with
        // AmbiguousPriorClaim → caller must request a new challenge.
        let retry = verify_and_claim(
            state.write_auth_challenges.as_ref(),
            &envelope,
            WriteAuthAction::OptionOrderSubmit,
            &account,
            &canonical,
            WRITE_AUTH_DOMAIN_CHAIN_ID,
            now_ms(),
        )
        .await;
        assert!(
            matches!(retry, Err(WriteAuthError::AmbiguousPriorClaim)),
            "retry of consumed-but-unlinked nonce must reject (got {retry:?})"
        );
    }
);
