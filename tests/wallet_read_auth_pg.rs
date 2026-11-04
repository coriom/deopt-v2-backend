//! Disposable PostgreSQL acceptance for wallet-scoped, read-only sessions.
use axum::body::Body;
use axum::http::{header::AUTHORIZATION, HeaderMap, HeaderValue, Request, StatusCode};
use axum::{extract::State, Json};
use deopt_v2_backend::api::{router, AppState};
use deopt_v2_backend::auth::wallet_read::{
    self, ChallengeRequest, VerifyRequest, WalletReadAuthConfig,
};
use deopt_v2_backend::db::{pool, PgRepository};
use deopt_v2_backend::engine::EngineState;
use deopt_v2_backend::signing::{Eip712Domain, SignatureVerificationMode};
use k256::ecdsa::SigningKey;
use tower::ServiceExt;

fn key(tag: u8) -> SigningKey {
    SigningKey::from_slice(&[tag; 32]).unwrap()
}
fn address(key: &SigningKey) -> String {
    let point = key.verifying_key().to_encoded_point(false);
    let hash = deopt_v2_backend::signing::eip712::keccak256(&point.as_bytes()[1..]);
    format!("0x{}", hex::encode(&hash[12..]))
}
fn sign(key: &SigningKey, message: &str) -> String {
    let digest = deopt_v2_backend::signing::personal_sign_digest(message);
    let (sig, recovery) = key.sign_prehash_recoverable(&digest).unwrap();
    let mut bytes = [0u8; 65];
    bytes[..32].copy_from_slice(&sig.r().to_bytes());
    bytes[32..64].copy_from_slice(&sig.s().to_bytes());
    bytes[64] = recovery.to_byte();
    format!("0x{}", hex::encode(bytes))
}

#[tokio::test]
async fn pg_wallet_session_replay_isolation_and_revocation() {
    let Ok(url) = std::env::var("DEOPT_AUTH_TEST_DATABASE_URL") else {
        return;
    };
    let repo = PgRepository::connect(&url).await.unwrap();
    if std::env::var("DEOPT_AUTH_TEST_SKIP_MIGRATIONS")
        .ok()
        .as_deref()
        == Some("1")
    {
        assert_eq!(
            std::env::var("DEOPT_AUTH_TEST_DISPOSABLE").ok().as_deref(),
            Some("1"),
            "existing-schema auth test requires explicit disposable DB confirmation"
        );
        pool::verify_read_only_schema(repo.pool()).await.unwrap();
    } else {
        pool::run_migrations(repo.pool()).await.unwrap();
    }
    let mut state = AppState::with_signature_mode_domain_and_repository(
        EngineState::with_default_markets(),
        SignatureVerificationMode::Strict,
        Eip712Domain::default(),
        Some(repo.clone()),
    );
    state.wallet_read_auth = WalletReadAuthConfig {
        enabled: true,
        origin: "https://deoptv2-frontend.vercel.app".into(),
    };
    let alice = key(3);
    let bob = key(4);
    let alice_wallet = address(&alice);
    let bob_wallet = address(&bob);
    let Json(challenge) = wallet_read::challenge(
        State(state.clone()),
        Json(ChallengeRequest {
            address: alice_wallet.clone(),
        }),
    )
    .await
    .unwrap();
    assert!(challenge.message.contains("Chain ID: 84532"));
    let before: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(attempts), 0)::BIGINT FROM wallet_read_rate WHERE scope=$1",
    )
    .bind(format!("verify:{alice_wallet}"))
    .fetch_one(repo.pool())
    .await
    .unwrap();
    let bad = wallet_read::verify(
        State(state.clone()),
        Json(VerifyRequest {
            nonce: challenge.nonce.clone(),
            signature: sign(&bob, &challenge.message),
        }),
    )
    .await;
    assert_eq!(bad.err().unwrap().0, StatusCode::UNAUTHORIZED);
    let Json(verified) = wallet_read::verify(
        State(state.clone()),
        Json(VerifyRequest {
            nonce: challenge.nonce.clone(),
            signature: sign(&alice, &challenge.message),
        }),
    )
    .await
    .unwrap();
    assert_eq!(verified.wallet, alice_wallet);
    let replay = wallet_read::verify(
        State(state.clone()),
        Json(VerifyRequest {
            nonce: challenge.nonce,
            signature: sign(&alice, &challenge.message),
        }),
    )
    .await;
    assert_eq!(replay.err().unwrap().0, StatusCode::UNAUTHORIZED);
    let after: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(attempts), 0)::BIGINT FROM wallet_read_rate WHERE scope=$1",
    )
    .bind(format!("verify:{alice_wallet}"))
    .fetch_one(repo.pool())
    .await
    .unwrap();
    assert_eq!(after - before, 3);

    // Two verifiers racing on one nonce must produce only one session.
    let Json(racing) = wallet_read::challenge(
        State(state.clone()),
        Json(ChallengeRequest {
            address: alice_wallet.clone(),
        }),
    )
    .await
    .unwrap();
    let request = VerifyRequest {
        nonce: racing.nonce,
        signature: sign(&alice, &racing.message),
    };
    let other_request = VerifyRequest {
        nonce: request.nonce.clone(),
        signature: request.signature.clone(),
    };
    let (first, second) = tokio::join!(
        wallet_read::verify(State(state.clone()), Json(request)),
        wallet_read::verify(State(state.clone()), Json(other_request))
    );
    assert_eq!(u8::from(first.is_ok()) + u8::from(second.is_ok()), 1);

    let app = router(state.clone()).layer(axum::middleware::from_fn_with_state(
        state.clone(),
        deopt_v2_backend::read_only_release::reject_http_writes,
    ));
    let path = format!("/accounts/{alice_wallet}/perps/canonical-fees");
    let anonymous = app
        .clone()
        .oneshot(Request::builder().uri(&path).body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(anonymous.status(), StatusCode::UNAUTHORIZED);
    let own = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(&path)
                .header(AUTHORIZATION, format!("Bearer {}", verified.token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(own.status(), StatusCode::OK);
    let other = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/accounts/{bob_wallet}/perps/canonical-fees"))
                .header(AUTHORIZATION, format!("Bearer {}", verified.token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(other.status(), StatusCode::FORBIDDEN);
    // Exercise the future Options route without the production read-only
    // middleware, which currently blocks all Options routes before the
    // handler. This proves the handler itself checks wallet ownership.
    let options_app = router(state.clone());
    for suffix in [
        "positions",
        "portfolio",
        "balances",
        "history",
        "history/v2",
    ] {
        let alice_path = format!("/accounts/{alice_wallet}/{suffix}");
        let bob_path = format!("/accounts/{bob_wallet}/{suffix}");
        let anonymous = options_app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(alice_path)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(anonymous.status(), StatusCode::UNAUTHORIZED, "{suffix}");
        let other = options_app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(bob_path)
                    .header(AUTHORIZATION, format!("Bearer {}", verified.token))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(other.status(), StatusCode::FORBIDDEN, "{suffix}");
    }
    let write = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/perps/orders")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(write.status(), StatusCode::METHOD_NOT_ALLOWED);

    let mut headers = HeaderMap::new();
    headers.insert(
        AUTHORIZATION,
        HeaderValue::from_str(&format!("Bearer {}", verified.token)).unwrap(),
    );
    assert_eq!(
        wallet_read::session_wallet(&state, &headers).await.unwrap(),
        alice_wallet
    );
    let mut wrong_origin = state.clone();
    wrong_origin.wallet_read_auth.origin = "https://other.example".into();
    assert_eq!(
        wallet_read::session_wallet(&wrong_origin, &headers)
            .await
            .err()
            .unwrap()
            .0,
        StatusCode::UNAUTHORIZED
    );
    let mut wrong_chain = state.clone();
    wrong_chain.chain_id = 1;
    assert_eq!(
        wallet_read::session_wallet(&wrong_chain, &headers)
            .await
            .err()
            .unwrap()
            .0,
        StatusCode::UNAUTHORIZED
    );
    sqlx::query("UPDATE wallet_read_sessions SET expires_at_ms=$1 WHERE wallet=$2")
        .bind(deopt_v2_backend::types::now_ms() - 1)
        .bind(&alice_wallet)
        .execute(repo.pool())
        .await
        .unwrap();
    assert_eq!(
        wallet_read::session_wallet(&state, &headers)
            .await
            .err()
            .unwrap()
            .0,
        StatusCode::UNAUTHORIZED
    );
    sqlx::query("UPDATE wallet_read_sessions SET expires_at_ms=$1 WHERE wallet=$2")
        .bind(verified.expires_at_ms)
        .bind(&alice_wallet)
        .execute(repo.pool())
        .await
        .unwrap();
    wallet_read::logout(State(state.clone()), headers.clone())
        .await
        .unwrap();
    assert_eq!(
        wallet_read::session_wallet(&state, &headers)
            .await
            .err()
            .unwrap()
            .0,
        StatusCode::UNAUTHORIZED
    );
    let after_restart = AppState::with_signature_mode_domain_and_repository(
        EngineState::with_default_markets(),
        SignatureVerificationMode::Strict,
        Eip712Domain::default(),
        Some(repo),
    );
    let mut after_restart = after_restart;
    after_restart.wallet_read_auth = state.wallet_read_auth;
    assert_eq!(
        wallet_read::session_wallet(&after_restart, &headers)
            .await
            .err()
            .unwrap()
            .0,
        StatusCode::UNAUTHORIZED
    );
}
