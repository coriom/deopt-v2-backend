//! PERPS-FULLSTACK-RUNTIME-INTEGRATION-V1 Part D — integration tests
//! for the closed-test signed `PerpOrderIntent` endpoint
//! (`POST /perps/orders/signed`).
//!
//! Coverage:
//! * Fail-closed layer 1: `perps_closed_test_enabled = false` → 503
//!   `PerpsNotLive` regardless of any other input.
//! * Fail-closed layer 2: `perps_public_trading_enabled = true` on
//!   mainnet path is not reached — the endpoint is closed-test-only
//!   for V1.
//! * Allowlist rejection: signed intent from a non-allowlisted trader
//!   → 503 `PerpsNotLive` (does NOT reveal allowlist membership via
//!   401 / 403).
//! * Signature failures: wrong signer, malformed signature, tampered
//!   payload → 401 `PerpsIntentSignatureInvalid`.
//! * Shape failures: wrong side bound (buy setting min) → 422
//!   `PerpsIntentSideBoundInconsistent`.
//! * Deadline in the past → 422 `PerpsIntentDeadlineExpired`.
//! * Nonce replay: the second submit with the same `(trader, nonce)`
//!   → 409 `PerpsIntentNonceReplay`.
//!
//! Happy-path buy/sell (allowlisted trader, PG-backed engine, in-memory
//! oracle reader) is env-gated on
//! `PERPS_SIGNED_INTENT_PG_URL` — matches the pattern in
//! `perps_public_route_enabled_flag_pg_proof.rs`. Without the env var,
//! the happy-path tests no-op so `cargo test` stays green in developer
//! environments that don't run Postgres.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use deopt_v2_backend::api::{router, AppState};
use deopt_v2_backend::db::PgRepository;
use deopt_v2_backend::engine::EngineState;
use deopt_v2_backend::error::BackendError;
use deopt_v2_backend::execution::{
    perp_order_intent_digest, PerpOrderIntent, PerpTradeDomain, PERP_ORDER_INTENT_SIDE_BUY,
    PERP_ORDER_INTENT_SIDE_SELL,
};
use deopt_v2_backend::perps::price_reader::{InMemoryPerpOraclePriceReader, RawPriceRead};
use deopt_v2_backend::perps::{
    InMemoryNonceLedger, PerpOrderIntentNonceLedger, PerpsReadConfig,
};
use deopt_v2_backend::signing::eip712::keccak256;
use deopt_v2_backend::types::{now_ms, AccountId};
use k256::ecdsa::signature::hazmat::PrehashSigner;
use k256::ecdsa::{RecoveryId, Signature, SigningKey};
use serde_json::json;
use tower::ServiceExt;

const ONE_1E8: u128 = 100_000_000;
const ETH_ONCHAIN_MARKET_ID: u128 = 1;

// ---------------------------------------------------------------------
// Setup helpers
// ---------------------------------------------------------------------

fn signing_key(seed_byte: u8) -> SigningKey {
    // 32 identical non-zero bytes — deterministic, distinct per seed.
    SigningKey::from_bytes(&[seed_byte; 32].into()).unwrap()
}

fn signer_address(key: &SigningKey) -> AccountId {
    let verifying = key.verifying_key();
    let encoded = verifying.to_encoded_point(false);
    let hash = keccak256(&encoded.as_bytes()[1..]);
    let mut hex = String::from("0x");
    for byte in &hash[12..] {
        hex.push_str(&format!("{byte:02x}"));
    }
    AccountId::new(hex)
}

fn sign_digest(key: &SigningKey, digest: &[u8; 32]) -> String {
    let (sig, recovery): (Signature, RecoveryId) = key.sign_prehash(digest).unwrap();
    let mut bytes = [0u8; 65];
    bytes[..64].copy_from_slice(&sig.to_bytes());
    bytes[64] = recovery.to_byte();
    let mut hex = String::from("0x");
    for byte in bytes {
        hex.push_str(&format!("{byte:02x}"));
    }
    hex
}

fn base_state() -> AppState {
    let mut state = AppState::new(EngineState::with_default_markets());
    let mut cfg = PerpsReadConfig::enabled_in_memory_for_tests();
    cfg.rpc_url = None;
    state.perps_read_config = cfg;
    state
}

fn state_with_closed_test(allow: &[AccountId]) -> AppState {
    let mut state = base_state();
    state.perps_closed_test_enabled = true;
    state.perps_closed_test_allowlist = allow.to_vec();
    // Seed a fresh oracle price for ETH-PERP so happy-path submits
    // don't hit `PerpMarkPriceUnavailable`. The endpoint uses this
    // in-memory reader only when the RPC-backed reader can't be
    // constructed (typical closed-test posture, no rpc_url).
    let reader = InMemoryPerpOraclePriceReader::new().with_price(
        "ETH-PERP",
        RawPriceRead {
            price_1e8: 3000 * ONE_1E8,
            updated_at_sec: (now_ms() / 1000) as u64,
            ok: true,
        },
    );
    state.perps_signed_intent_price_reader = Some(std::sync::Arc::new(reader));
    state
}

fn buy_intent(trader: AccountId) -> PerpOrderIntent {
    // A market buy: `limitPrice1e8 == 0`, capped at $3200.
    PerpOrderIntent {
        intent_id: intent_id_hex_from("test-buy-01"),
        trader,
        subaccount_id: 1,
        market_id: ETH_ONCHAIN_MARKET_ID,
        side: PERP_ORDER_INTENT_SIDE_BUY,
        size_1e8: ONE_1E8,
        limit_price_1e8: 0,
        max_exec_price_1e8: 3200 * ONE_1E8,
        min_exec_price_1e8: 0,
        nonce: 1,
        deadline: far_future_deadline(),
    }
}

fn sell_intent(trader: AccountId) -> PerpOrderIntent {
    PerpOrderIntent {
        intent_id: intent_id_hex_from("test-sell-01"),
        trader,
        subaccount_id: 1,
        market_id: ETH_ONCHAIN_MARKET_ID,
        side: PERP_ORDER_INTENT_SIDE_SELL,
        size_1e8: ONE_1E8,
        limit_price_1e8: 0,
        max_exec_price_1e8: 0,
        min_exec_price_1e8: 2800 * ONE_1E8,
        nonce: 1,
        deadline: far_future_deadline(),
    }
}

fn intent_id_hex_from(seed: &str) -> alloy_primitives::B256 {
    alloy_primitives::B256::from(keccak256(seed.as_bytes()))
}

fn far_future_deadline() -> u128 {
    // Year 2200-ish.
    9_999_999_999
}

fn domain_for(state: &AppState) -> PerpTradeDomain {
    PerpTradeDomain::new(
        state.perps_read_config.chain_id,
        state.execution_config.perp_matching_engine_address.clone(),
    )
}

fn intent_body_json(intent: &PerpOrderIntent, signature: &str) -> String {
    json!({
        "intent": {
            "intentId": hex_b256(&intent.intent_id),
            "trader": intent.trader.0,
            "subaccountId": intent.subaccount_id,
            "marketId": intent.market_id.to_string(),
            "side": intent.side,
            "size1e8": intent.size_1e8.to_string(),
            "limitPrice1e8": intent.limit_price_1e8.to_string(),
            "maxExecPrice1e8": intent.max_exec_price_1e8.to_string(),
            "minExecPrice1e8": intent.min_exec_price_1e8.to_string(),
            "nonce": intent.nonce.to_string(),
            "deadline": intent.deadline.to_string(),
        },
        "signature": signature,
    })
    .to_string()
}

fn hex_b256(b: &alloy_primitives::B256) -> String {
    let mut s = String::from("0x");
    for byte in b.as_slice() {
        s.push_str(&format!("{byte:02x}"));
    }
    s
}

async fn post_signed(state: AppState, body: String) -> axum::response::Response {
    router(state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/perps/orders/signed")
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap()
}

async fn body_text(response: axum::response::Response) -> String {
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    String::from_utf8_lossy(&bytes).to_string()
}

// ---------------------------------------------------------------------
// 1. Layer 1: closed-test flag off → 503, regardless of body.
// ---------------------------------------------------------------------

#[tokio::test]
async fn closed_test_off_returns_perps_not_live() {
    let state = base_state();
    let key = signing_key(0x11);
    let trader = signer_address(&key);
    let intent = buy_intent(trader);
    let domain = domain_for(&state);
    let digest = perp_order_intent_digest(&intent, &domain).unwrap();
    let sig = sign_digest(&key, &digest);
    let body = intent_body_json(&intent, &sig);
    let response = post_signed(state, body).await;
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let text = body_text(response).await;
    assert!(text.contains("perps not live"), "got: {text}");
}

// ---------------------------------------------------------------------
// 2. Layer 2: public trading on → 503 (signed-intent is closed-test
//    only for V1).
// ---------------------------------------------------------------------

#[tokio::test]
async fn public_trading_on_still_returns_perps_not_live_on_signed_endpoint() {
    let key = signing_key(0x22);
    let trader = signer_address(&key);
    let mut state = state_with_closed_test(&[trader.clone()]);
    state.perps_public_trading_enabled = true;
    let intent = buy_intent(trader);
    let domain = domain_for(&state);
    let digest = perp_order_intent_digest(&intent, &domain).unwrap();
    let sig = sign_digest(&key, &digest);
    let body = intent_body_json(&intent, &sig);
    let response = post_signed(state, body).await;
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
}

// ---------------------------------------------------------------------
// 3. Allowlist rejection: closed-test on, valid signature, but caller
//    not on allowlist → 503 `PerpsNotLive` (not 401/403 — we do not
//    reveal allowlist membership via a distinct status).
// ---------------------------------------------------------------------

#[tokio::test]
async fn allowlist_rejects_non_listed_trader_as_perps_not_live() {
    let key = signing_key(0x33);
    let trader = signer_address(&key);
    // Empty allowlist — trader is not in.
    let state = state_with_closed_test(&[]);
    let intent = buy_intent(trader);
    let domain = domain_for(&state);
    let digest = perp_order_intent_digest(&intent, &domain).unwrap();
    let sig = sign_digest(&key, &digest);
    let body = intent_body_json(&intent, &sig);
    let response = post_signed(state, body).await;
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
}

// ---------------------------------------------------------------------
// 4. Bad signature (wrong signer) → 401 PerpsIntentSignatureInvalid.
// ---------------------------------------------------------------------

#[tokio::test]
async fn wrong_signer_rejected_as_signature_invalid() {
    let key = signing_key(0x44);
    let trader = signer_address(&key);
    let state = state_with_closed_test(&[trader.clone()]);
    let intent = buy_intent(trader);
    // Sign with a DIFFERENT key.
    let other_key = signing_key(0x55);
    let domain = domain_for(&state);
    let digest = perp_order_intent_digest(&intent, &domain).unwrap();
    let sig = sign_digest(&other_key, &digest);
    let body = intent_body_json(&intent, &sig);
    let response = post_signed(state, body).await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let text = body_text(response).await;
    assert!(text.contains("perp order intent signature is invalid"), "got: {text}");
}

// ---------------------------------------------------------------------
// 5. Tampered field (side flip) → signature no longer verifies to the
//    declared trader → 401.
// ---------------------------------------------------------------------

#[tokio::test]
async fn tampered_side_flips_signature_to_invalid() {
    let key = signing_key(0x66);
    let trader = signer_address(&key);
    let state = state_with_closed_test(&[trader.clone()]);
    let intent = buy_intent(trader.clone());
    let domain = domain_for(&state);
    let digest = perp_order_intent_digest(&intent, &domain).unwrap();
    let sig = sign_digest(&key, &digest);
    // Send a tampered intent alongside the pristine signature. Flip
    // side buy→sell. Recovered signer will NOT be the declared trader.
    let mut tampered = intent;
    tampered.side = PERP_ORDER_INTENT_SIDE_SELL;
    // Also flip the bounds so the shape check doesn't reject BEFORE
    // signature verification would (verify runs first in the handler,
    // so this doesn't matter for path — but we set the shape correctly
    // to prove the signature layer is the rejecter).
    tampered.max_exec_price_1e8 = 0;
    tampered.min_exec_price_1e8 = 2800 * ONE_1E8;
    let body = intent_body_json(&tampered, &sig);
    let response = post_signed(state, body).await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn malformed_signature_rejected_as_signature_invalid() {
    let key = signing_key(0x77);
    let trader = signer_address(&key);
    let state = state_with_closed_test(&[trader.clone()]);
    let intent = buy_intent(trader);
    let body = intent_body_json(&intent, "0xdead");
    let response = post_signed(state, body).await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

// ---------------------------------------------------------------------
// 6. Wrong side bound (buy setting min) → 422 side/bound inconsistent.
// ---------------------------------------------------------------------

#[tokio::test]
async fn buy_setting_min_bound_rejected_as_side_bound_inconsistent() {
    let key = signing_key(0x88);
    let trader = signer_address(&key);
    let state = state_with_closed_test(&[trader.clone()]);
    let mut intent = buy_intent(trader);
    // Buy with a min bound — this is the side/bound inconsistency
    // check. Also clear max so we don't accidentally still be valid.
    intent.min_exec_price_1e8 = 2000 * ONE_1E8;
    // Keep max set so the handler doesn't reject on "buy without max"
    // first — we want to prove that the min-set-on-buy rule fires.
    let domain = domain_for(&state);
    let digest = perp_order_intent_digest(&intent, &domain).unwrap();
    let sig = sign_digest(&key, &digest);
    let body = intent_body_json(&intent, &sig);
    let response = post_signed(state, body).await;
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let text = body_text(response).await;
    assert!(
        text.contains("side/bound"),
        "expected side/bound rejection; got: {text}"
    );
}

// ---------------------------------------------------------------------
// 7. Deadline in the past → 422 PerpsIntentDeadlineExpired.
// ---------------------------------------------------------------------

#[tokio::test]
async fn expired_deadline_rejected() {
    let key = signing_key(0x99);
    let trader = signer_address(&key);
    let state = state_with_closed_test(&[trader.clone()]);
    let mut intent = buy_intent(trader);
    intent.deadline = 1; // way in the past
    let domain = domain_for(&state);
    let digest = perp_order_intent_digest(&intent, &domain).unwrap();
    let sig = sign_digest(&key, &digest);
    let body = intent_body_json(&intent, &sig);
    let response = post_signed(state, body).await;
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let text = body_text(response).await;
    assert!(
        text.contains("deadline"),
        "expected deadline rejection; got: {text}"
    );
}

// ---------------------------------------------------------------------
// 8. Nonce replay: exercise the process-local nonce store directly.
//    The endpoint's Layer 8 delegates to `try_consume`; we prove the
//    replay-rejection behavior at the store level here so we don't
//    need PG to see it. The endpoint-integration proof is env-gated
//    (see PG happy-path section below).
// ---------------------------------------------------------------------

#[tokio::test]
async fn nonce_store_rejects_replay_within_process() {
    let store = InMemoryNonceLedger::new();
    let trader = AccountId::new("0x00000000000000000000000000000000000000ab");
    let hash = [1u8; 32];
    store.try_consume(&trader, 42, hash).await.unwrap();
    let err = store.try_consume(&trader, 42, hash).await.unwrap_err();
    assert!(matches!(err, BackendError::PerpsIntentNonceReplay));
}

// ---------------------------------------------------------------------
// 9. Nonce replay via the HTTP surface — we exercise the failure
//    path by pre-populating the nonce store with the exact
//    `(trader, nonce)` before submitting. Both submits then go
//    through the same path: signature verify (OK) → allowlist (OK)
//    → shape (OK) → deadline (OK) → nonce (REPLAY). This is
//    end-to-end at the router boundary but does not need PG because
//    the replay-reject happens before dispatch to the engine.
// ---------------------------------------------------------------------

#[tokio::test]
async fn nonce_replay_via_http_returns_conflict() {
    let key = signing_key(0xaa);
    let trader = signer_address(&key);
    let state = state_with_closed_test(&[trader.clone()]);
    let intent = buy_intent(trader.clone());
    // Pre-consume the nonce so the endpoint's try_consume fails.
    // Use a placeholder intent_hash; the in-memory ledger ignores it
    // (the endpoint recomputes the real hash before calling
    // try_consume, but the (trader, nonce) key alone gates replay).
    state
        .perp_order_intent_nonce_ledger
        .try_consume(&trader, intent.nonce, [0u8; 32])
        .await
        .unwrap();
    let domain = domain_for(&state);
    let digest = perp_order_intent_digest(&intent, &domain).unwrap();
    let sig = sign_digest(&key, &digest);
    let body = intent_body_json(&intent, &sig);
    let response = post_signed(state, body).await;
    assert_eq!(response.status(), StatusCode::CONFLICT);
}

// ---------------------------------------------------------------------
// 10. Happy path buy (PG + in-memory oracle reader). Env-gated on
//     `PERPS_SIGNED_INTENT_PG_URL`. Verifies the endpoint's full
//     dispatch to the internal engine when every gate is satisfied.
// ---------------------------------------------------------------------

const PG_ENV_VAR: &str = "PERPS_SIGNED_INTENT_PG_URL";

fn pg_test_url() -> Option<String> {
    std::env::var(PG_ENV_VAR).ok().filter(|v| !v.is_empty())
}

async fn ensure_migrated(url: &str) {
    static MIGRATED: tokio::sync::OnceCell<()> = tokio::sync::OnceCell::const_new();
    MIGRATED
        .get_or_init(|| async {
            let repo = PgRepository::connect(url)
                .await
                .expect("connect for shared migration");
            repo.run_migrations()
                .await
                .expect("run migrations once against disposable PG database");
        })
        .await;
}

async fn fresh_repo(url: &str) -> PgRepository {
    ensure_migrated(url).await;
    PgRepository::connect(url)
        .await
        .expect("connect to disposable PG database")
}

async fn state_with_closed_test_and_pg(allow: &[AccountId], url: &str) -> AppState {
    let mut state = state_with_closed_test(allow);
    let repo = fresh_repo(url).await;
    state.repository = Some(repo);
    state.persistence_enabled = true;
    state.database_configured = true;
    state
}

#[tokio::test]
async fn happy_path_buy_signed_submit_returns_ok() {
    let Some(url) = pg_test_url() else {
        return;
    };
    let key = signing_key(0xbb);
    let trader = signer_address(&key);
    let state = state_with_closed_test_and_pg(&[trader.clone()], &url).await;
    let intent = buy_intent(trader);
    let domain = domain_for(&state);
    let digest = perp_order_intent_digest(&intent, &domain).unwrap();
    let sig = sign_digest(&key, &digest);
    let body = intent_body_json(&intent, &sig);
    let response = post_signed(state, body).await;
    assert_eq!(response.status(), StatusCode::OK);
    let text = body_text(response).await;
    assert!(text.contains("\"status\":\"ok\""), "got: {text}");
    assert!(
        text.contains("\"closed_test_accepted\":true"),
        "got: {text}"
    );
}

#[tokio::test]
async fn happy_path_sell_signed_submit_returns_ok() {
    let Some(url) = pg_test_url() else {
        return;
    };
    let key = signing_key(0xcc);
    let trader = signer_address(&key);
    let state = state_with_closed_test_and_pg(&[trader.clone()], &url).await;
    let intent = sell_intent(trader);
    let domain = domain_for(&state);
    let digest = perp_order_intent_digest(&intent, &domain).unwrap();
    let sig = sign_digest(&key, &digest);
    let body = intent_body_json(&intent, &sig);
    let response = post_signed(state, body).await;
    assert_eq!(response.status(), StatusCode::OK);
}

// ---------------------------------------------------------------------
// PERPS-CLOSED-TEST-HARDENING-V1 Part C #15 — subaccount ownership
// gate. The signed intent's EIP-712 struct binds `subaccountId`, so
// tamper detection is signature-mediated. But nothing previously
// enforced that the signing WALLET owns the referenced subaccount.
// These tests prove the ownership check at the authoritative
// execution boundary (`perps_submit_signed_order`, Layer 6).
//
// Notes:
// * Layer 6 runs AFTER Layer 5 (allowlist), so both traders must be
//   allowlisted for the subaccount-owner check to surface as the
//   rejecter (otherwise the allowlist gate wins with `PerpsNotLive`).
// * The signed-intent test path uses `state_with_closed_test` which
//   defaults to `InMemorySubaccountStore` — the ownership store the
//   handler consults is the same `AppState.subaccounts` field.
// * We seed subaccounts by calling the module-level helpers directly
//   against `state.subaccounts.as_ref()`, mirroring the pattern in
//   `tests/perps_v2_write_auth_enforcement_v1_tests.rs`.
// ---------------------------------------------------------------------

async fn seed_default_subaccount(state: &AppState, owner: &AccountId) {
    let _ = deopt_v2_backend::subaccounts::ensure_default_subaccount(
        state.subaccounts.as_ref(),
        owner,
    )
    .await
    .expect("seed default subaccount");
}

async fn seed_second_subaccount(state: &AppState, owner: &AccountId) -> u32 {
    seed_default_subaccount(state, owner).await;
    let created = deopt_v2_backend::subaccounts::create_subaccount(
        state.subaccounts.as_ref(),
        owner,
        None,
    )
    .await
    .expect("allocate second subaccount");
    assert_eq!(created.subaccount_id, 2, "expected id=2, got {created:?}");
    created.subaccount_id
}

/// Part C #15.a — trader signs an intent naming THEIR OWN default
/// subaccount (id 1). The ownership gate must not reject. The
/// request may still fail later at engine dispatch (503 `PerpsNotLive`
/// when no PG is wired) but MUST NOT be 401 for subaccount reasons.
#[tokio::test]
async fn part_c_own_subaccount_accepted() {
    let key = signing_key(0xe0);
    let trader = signer_address(&key);
    let state = state_with_closed_test(&[trader.clone()]);
    // Lazy-created by the handler on Layer 6, but seeding explicitly
    // pins the invariant (id 1 is present) BEFORE the request runs.
    seed_default_subaccount(&state, &trader).await;
    let intent = buy_intent(trader);
    let domain = domain_for(&state);
    let digest = perp_order_intent_digest(&intent, &domain).unwrap();
    let sig = sign_digest(&key, &digest);
    let body = intent_body_json(&intent, &sig);
    let response = post_signed(state, body).await;
    // Without PG wired the handler returns `PerpsNotLive` at engine
    // dispatch (Layer 11). What matters is that we did NOT collapse
    // to 401 for subaccount reasons.
    assert_ne!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "own default subaccount must not be rejected as unauthorized"
    );
    let text = body_text(response).await;
    assert!(
        !text.contains("subaccount not owned"),
        "own default subaccount must not surface the ownership reject: {text}"
    );
}

/// Part C #15.b — trader1 (allowlisted) signs an intent naming a
/// subaccount that is owned by trader2 (also allowlisted, so the
/// allowlist gate lets both through). The ownership gate MUST reject
/// with 401 `PerpsIntentSubaccountUnauthorized`.
#[tokio::test]
async fn part_c_another_wallets_subaccount_rejected() {
    let key1 = signing_key(0xe1);
    let key2 = signing_key(0xe2);
    let trader1 = signer_address(&key1);
    let trader2 = signer_address(&key2);
    // Both are allowlisted so Layer 5 doesn't win.
    let state = state_with_closed_test(&[trader1.clone(), trader2.clone()]);
    // Seed subaccount 2 for trader2. trader1 does NOT own id 2.
    let id2 = seed_second_subaccount(&state, &trader2).await;
    assert_eq!(id2, 2);
    // Also ensure trader1 has a default so we're proving the gate on
    // id 2 specifically (not just "trader1 has no rows at all").
    seed_default_subaccount(&state, &trader1).await;
    // trader1 signs an intent referencing subaccount id 2 (trader2's).
    let intent = PerpOrderIntent {
        intent_id: intent_id_hex_from("part-c-cross-owner"),
        trader: trader1.clone(),
        subaccount_id: 2,
        market_id: ETH_ONCHAIN_MARKET_ID,
        side: PERP_ORDER_INTENT_SIDE_BUY,
        size_1e8: ONE_1E8,
        limit_price_1e8: 0,
        max_exec_price_1e8: 3200 * ONE_1E8,
        min_exec_price_1e8: 0,
        nonce: 1,
        deadline: far_future_deadline(),
    };
    let domain = domain_for(&state);
    let digest = perp_order_intent_digest(&intent, &domain).unwrap();
    let sig = sign_digest(&key1, &digest);
    let body = intent_body_json(&intent, &sig);
    let response = post_signed(state, body).await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let text = body_text(response).await;
    assert!(
        text.contains("subaccount not owned"),
        "expected subaccount ownership rejection; got: {text}"
    );
}

/// Part C #15.c — `subaccountId == 0` is reserved for future system
/// use (see `crate::subaccounts::DEFAULT_SUBACCOUNT_ID` doc). It is
/// never a legitimate per-wallet subaccount and the ownership gate
/// MUST reject it with 401.
#[tokio::test]
async fn part_c_malformed_subaccount_id_rejected() {
    let key = signing_key(0xe3);
    let trader = signer_address(&key);
    let state = state_with_closed_test(&[trader.clone()]);
    seed_default_subaccount(&state, &trader).await;
    let intent = PerpOrderIntent {
        intent_id: intent_id_hex_from("part-c-subaccount-zero"),
        trader: trader.clone(),
        // 0 is reserved; MUST be rejected.
        subaccount_id: 0,
        market_id: ETH_ONCHAIN_MARKET_ID,
        side: PERP_ORDER_INTENT_SIDE_BUY,
        size_1e8: ONE_1E8,
        limit_price_1e8: 0,
        max_exec_price_1e8: 3200 * ONE_1E8,
        min_exec_price_1e8: 0,
        nonce: 1,
        deadline: far_future_deadline(),
    };
    let domain = domain_for(&state);
    let digest = perp_order_intent_digest(&intent, &domain).unwrap();
    let sig = sign_digest(&key, &digest);
    let body = intent_body_json(&intent, &sig);
    let response = post_signed(state, body).await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let text = body_text(response).await;
    assert!(
        text.contains("subaccount not owned"),
        "expected subaccount ownership rejection for id=0; got: {text}"
    );
}

/// Part C #15.d — a trader that owns MULTIPLE subaccounts (ids 1 and
/// 2) can sign intents naming either one. Both are accepted at the
/// SIGNING path. Downstream position/order isolation is Part D
/// territory; here we only prove Layer 6 does not incorrectly deny.
#[tokio::test]
async fn part_c_same_wallet_multiple_subaccounts_isolated() {
    let key = signing_key(0xe4);
    let trader = signer_address(&key);
    let state = state_with_closed_test(&[trader.clone()]);
    let id2 = seed_second_subaccount(&state, &trader).await;
    assert_eq!(id2, 2);
    let domain = domain_for(&state);

    // Intent A: signed by trader for subaccount 1 (their default).
    let mut intent_a = buy_intent(trader.clone());
    intent_a.subaccount_id = 1;
    intent_a.nonce = 100;
    intent_a.intent_id = intent_id_hex_from("part-c-multi-sa-a");
    let digest_a = perp_order_intent_digest(&intent_a, &domain).unwrap();
    let sig_a = sign_digest(&key, &digest_a);
    let body_a = intent_body_json(&intent_a, &sig_a);
    let response_a = post_signed(state.clone(), body_a).await;
    assert_ne!(
        response_a.status(),
        StatusCode::UNAUTHORIZED,
        "own subaccount 1 must not be rejected as unauthorized"
    );

    // Intent B: signed by trader for subaccount 2 (also theirs).
    let mut intent_b = buy_intent(trader.clone());
    intent_b.subaccount_id = 2;
    intent_b.nonce = 101;
    intent_b.intent_id = intent_id_hex_from("part-c-multi-sa-b");
    let digest_b = perp_order_intent_digest(&intent_b, &domain).unwrap();
    let sig_b = sign_digest(&key, &digest_b);
    let body_b = intent_body_json(&intent_b, &sig_b);
    let response_b = post_signed(state, body_b).await;
    assert_ne!(
        response_b.status(),
        StatusCode::UNAUTHORIZED,
        "own subaccount 2 must not be rejected as unauthorized"
    );
}

/// Part C #15.e — layered fail-closed ordering: an UNLISTED trader
/// referencing another wallet's subaccount MUST still surface the
/// allowlist rejection (503 `PerpsNotLive`), NOT the ownership
/// rejection (401). Ensures Layer 5 (allowlist) runs BEFORE Layer 6
/// (subaccount ownership) and the endpoint never leaks whether a
/// subaccount exists to a non-allowlisted caller.
#[tokio::test]
async fn part_c_allowlist_gate_precedes_subaccount_gate() {
    let key1 = signing_key(0xe5);
    let key2 = signing_key(0xe6);
    let trader1 = signer_address(&key1);
    let trader2 = signer_address(&key2);
    // Only trader2 is allowlisted; trader1 is NOT.
    let state = state_with_closed_test(&[trader2.clone()]);
    // Seed subaccount 2 for trader2.
    let id2 = seed_second_subaccount(&state, &trader2).await;
    assert_eq!(id2, 2);
    // trader1 (unlisted) signs an intent referencing trader2's
    // subaccount 2.
    let intent = PerpOrderIntent {
        intent_id: intent_id_hex_from("part-c-allowlist-before-sub"),
        trader: trader1.clone(),
        subaccount_id: 2,
        market_id: ETH_ONCHAIN_MARKET_ID,
        side: PERP_ORDER_INTENT_SIDE_BUY,
        size_1e8: ONE_1E8,
        limit_price_1e8: 0,
        max_exec_price_1e8: 3200 * ONE_1E8,
        min_exec_price_1e8: 0,
        nonce: 1,
        deadline: far_future_deadline(),
    };
    let domain = domain_for(&state);
    let digest = perp_order_intent_digest(&intent, &domain).unwrap();
    let sig = sign_digest(&key1, &digest);
    let body = intent_body_json(&intent, &sig);
    let response = post_signed(state, body).await;
    // Allowlist gate wins first — 503 `PerpsNotLive`. If subaccount
    // gate had run first the response would be 401 and we'd leak the
    // existence of trader2's subaccount 2 to an unlisted probe.
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn happy_path_then_replay_returns_conflict() {
    let Some(url) = pg_test_url() else {
        return;
    };
    let key = signing_key(0xdd);
    let trader = signer_address(&key);
    let state = state_with_closed_test_and_pg(&[trader.clone()], &url).await;
    let intent = buy_intent(trader);
    let domain = domain_for(&state);
    let digest = perp_order_intent_digest(&intent, &domain).unwrap();
    let sig = sign_digest(&key, &digest);
    let body = intent_body_json(&intent, &sig);
    let first = post_signed(state.clone(), body.clone()).await;
    assert_eq!(first.status(), StatusCode::OK);
    let second = post_signed(state, body).await;
    assert_eq!(second.status(), StatusCode::CONFLICT);
}
