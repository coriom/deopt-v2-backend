//! PERPS_BASE_SEPOLIA_CLOSED_TEST_COSIGN_ROUTE_V1 — PG-backed
//! integration coverage that exercises the SAME repository +
//! prepare/cosign helper chain the axum handlers use, against a
//! REAL disposable Postgres.
//!
//! Env-gated via `PERPS_CLOSED_TEST_E2E_PG_URL`. Zero required skips.

use deopt_v2_backend::api::perps_cosign::{
    cosign_load_and_verify, intent_to_v1_payload, prepare_trade_core, CosignTradeRequest,
    MarkPriceReader, NonceReader, PrepareTradeRequest, ReaderFuture, StoredTradeSignaturesView,
    PREPARE_DEADLINE_TTL_SEC,
};
use deopt_v2_backend::api::AppState;
use deopt_v2_backend::db::PgRepository;
use deopt_v2_backend::engine::EngineState;
use deopt_v2_backend::error::BackendError;
use deopt_v2_backend::execution::{
    perp_trade_v1_digest_bytes, ExecutionIntentStatus, StoredTradeSignatures,
};
use deopt_v2_backend::types::AccountId;

const PG_ENV_VAR: &str = "PERPS_CLOSED_TEST_E2E_PG_URL";
const CHAIN_ID: u64 = 84532;
const PME: &str = "0x774d96e5739bffadee91508b4d3d74f5be29f165";
const PENG: &str = "0xc6c592100723fe0c66343a16e95ec34cc0c2141c";
// Well-known Ganache test keys — used ONLY by this integration test to
// synthesize ephemeral signatures. No real trader keystore is unlocked.
const BUYER_KEY: &str = "0x4c0883a69102937d6231471b5dbb6204fe5129617082792ae468d01a3f362318";
const BUYER_ADDR: &str = "0x2c7536e3605d9c16a7a3d7b1898e529396a65c23";
const SELLER_KEY: &str = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
const SELLER_ADDR: &str = "0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266";

fn pg_url() -> Option<String> {
    std::env::var(PG_ENV_VAR).ok().filter(|v| !v.is_empty())
}

async fn ensure_migrated(url: &str) {
    static MIGRATED: tokio::sync::OnceCell<()> = tokio::sync::OnceCell::const_new();
    MIGRATED
        .get_or_init(|| async {
            let repo = PgRepository::connect(url).await.expect("connect");
            repo.run_migrations().await.expect("run migrations");
        })
        .await;
}

async fn fresh_repo(url: &str) -> PgRepository {
    ensure_migrated(url).await;
    PgRepository::connect(url).await.expect("connect")
}

fn state_with_closed_test(repo: PgRepository) -> AppState {
    let mut state = AppState::new(EngineState::with_default_markets());
    state.perps_closed_test_enabled = true;
    state.perps_public_trading_enabled = false;
    state.perps_closed_test_allowlist = vec![
        AccountId::new(BUYER_ADDR.to_string()),
        AccountId::new(SELLER_ADDR.to_string()),
    ];
    state.perps_read_config.chain_id = CHAIN_ID;
    state.execution_config.perp_matching_engine_address = AccountId::new(PME.to_string());
    state.execution_config.perp_engine_address = AccountId::new(PENG.to_string());
    state.persistence_enabled = true;
    state.repository = Some(repo);
    state
}

struct StaticNonceReader {
    buyer_addr: String,
    buyer: u128,
    seller_addr: String,
    seller: u128,
}
impl NonceReader for StaticNonceReader {
    fn read_pme_nonce<'a>(&'a self, trader: &'a AccountId) -> ReaderFuture<'a, u128> {
        let addr = trader.0.to_ascii_lowercase();
        let ba = self.buyer_addr.clone();
        let sa = self.seller_addr.clone();
        let bn = self.buyer;
        let sn = self.seller;
        Box::pin(async move {
            if addr == ba {
                Ok(bn)
            } else if addr == sa {
                Ok(sn)
            } else {
                Err(BackendError::Config(format!("unknown trader {addr}")))
            }
        })
    }
}

struct StaticMarkPriceReader {
    price: u128,
}
impl MarkPriceReader for StaticMarkPriceReader {
    fn read_mark_price<'a>(&'a self, _market_id: u128) -> ReaderFuture<'a, u128> {
        let p = self.price;
        Box::pin(async move { Ok(p) })
    }
}

fn mk_request() -> PrepareTradeRequest {
    PrepareTradeRequest {
        buyer: BUYER_ADDR.to_string(),
        seller: SELLER_ADDR.to_string(),
        market_id: "1".to_string(),
        size_delta_1e8: "1000000".to_string(),
        buyer_is_maker: false,
        max_execution_price_1e8: None,
        min_execution_price_1e8: None,
    }
}

fn sign_v1_with(key_hex: &str, digest: &[u8; 32]) -> String {
    use k256::ecdsa::SigningKey;
    let key_bytes = decode_hex_bytes(key_hex.strip_prefix("0x").unwrap());
    let sk = SigningKey::from_bytes(key_bytes.as_slice().into()).unwrap();
    let (sig, rec_id) = sk.sign_prehash_recoverable(digest).unwrap();
    let r = sig.r().to_bytes();
    let s = sig.s().to_bytes();
    let mut out = String::from("0x");
    for b in r.iter().chain(s.iter()) {
        out.push_str(&format!("{b:02x}"));
    }
    let v = 27u8 + rec_id.to_byte();
    out.push_str(&format!("{v:02x}"));
    out
}

fn decode_hex_bytes(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

fn bytes_to_hex(bytes: &[u8]) -> String {
    let mut s = String::from("0x");
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

// ================================================================
// 1. Prepare → PG insert → reload → digest equality
// ================================================================

#[tokio::test]
async fn prepare_persists_frozen_trade_and_digest_round_trips() {
    let Some(url) = pg_url() else {
        eprintln!(
            "IGNORED [prepare_persists_frozen_trade_and_digest_round_trips] (PG url not provided)"
        );
        return;
    };
    let repo = fresh_repo(&url).await;
    let state = state_with_closed_test(repo.clone());
    let req = mk_request();
    let nonce_reader = StaticNonceReader {
        buyer_addr: BUYER_ADDR.to_string(),
        buyer: 7,
        seller_addr: SELLER_ADDR.to_string(),
        seller: 12,
    };
    let mark_reader = StaticMarkPriceReader {
        price: 240_000_000_000,
    };
    let outcome = prepare_trade_core(
        &state,
        &req,
        &nonce_reader,
        &mark_reader,
        Some(1_700_000_000_000),
    )
    .await
    .expect("prepare");

    // Persist — same call as the axum handler makes.
    repo.insert_execution_intent_row(&outcome.execution_intent)
        .await
        .expect("insert intent");

    // Reload from PG.
    let reloaded = repo
        .get_execution_intent(outcome.uuid)
        .await
        .expect("reload query")
        .expect("row present");
    assert_eq!(reloaded.buyer.0.to_lowercase(), BUYER_ADDR);
    assert_eq!(reloaded.seller.0.to_lowercase(), SELLER_ADDR);
    assert_eq!(reloaded.buyer_nonce, Some(7));
    assert_eq!(reloaded.seller_nonce, Some(12));
    assert_eq!(reloaded.price_1e8, 240_000_000_000);
    assert_eq!(reloaded.size_1e8, 1_000_000);
    assert_eq!(reloaded.status, ExecutionIntentStatus::Pending);

    // Reconstruct payload from PG row and prove digest equality.
    let reloaded_payload = intent_to_v1_payload(&reloaded).unwrap();
    assert_eq!(
        reloaded_payload.deadline,
        1_700_000_000 + PREPARE_DEADLINE_TTL_SEC
    );
    let reloaded_digest = perp_trade_v1_digest_bytes(&reloaded_payload, &outcome.domain).unwrap();
    let original_digest = perp_trade_v1_digest_bytes(&outcome.payload, &outcome.domain).unwrap();
    assert_eq!(
        reloaded_digest, original_digest,
        "digest must be byte-identical after PG round-trip"
    );
}

// ================================================================
// 2. Cosign persistence — signatures land in execution_intent_signatures
// ================================================================

#[tokio::test]
async fn cosign_persists_both_signatures_and_advances_state() {
    let Some(url) = pg_url() else {
        eprintln!(
            "IGNORED [cosign_persists_both_signatures_and_advances_state] (PG url not provided)"
        );
        return;
    };
    let repo = fresh_repo(&url).await;
    let state = state_with_closed_test(repo.clone());
    let req = mk_request();
    let nonce_reader = StaticNonceReader {
        buyer_addr: BUYER_ADDR.to_string(),
        buyer: 0,
        seller_addr: SELLER_ADDR.to_string(),
        seller: 0,
    };
    let mark_reader = StaticMarkPriceReader {
        price: 240_000_000_000,
    };
    let outcome = prepare_trade_core(
        &state,
        &req,
        &nonce_reader,
        &mark_reader,
        Some(1_700_000_000_000),
    )
    .await
    .expect("prepare");
    repo.insert_execution_intent_row(&outcome.execution_intent)
        .await
        .expect("insert");

    // Sign the digest with ephemeral test keys.
    let digest = perp_trade_v1_digest_bytes(&outcome.payload, &outcome.domain).unwrap();
    let buyer_sig = sign_v1_with(BUYER_KEY, &digest);
    let seller_sig = sign_v1_with(SELLER_KEY, &digest);
    let cosign_req = CosignTradeRequest {
        buyer_signature: buyer_sig.clone(),
        seller_signature: seller_sig.clone(),
    };

    // Verify (same call the handler makes).
    let intent = repo
        .get_execution_intent(outcome.uuid)
        .await
        .unwrap()
        .unwrap();
    let verified = cosign_load_and_verify(
        &intent,
        &outcome.domain,
        &cosign_req,
        None,
        /*now_sec*/ 1_700_000_100,
    )
    .unwrap();
    assert_eq!(verified.buyer_signer.0.to_lowercase(), BUYER_ADDR);
    assert_eq!(verified.seller_signer.0.to_lowercase(), SELLER_ADDR);

    // Persist signatures — same PG call the handler makes.
    repo.upsert_execution_intent_signatures(
        outcome.uuid,
        Some(buyer_sig.clone()),
        Some(seller_sig.clone()),
        1_700_000_100_000,
    )
    .await
    .expect("upsert signatures");

    // Advance status.
    repo.update_execution_intent_status(
        outcome.uuid,
        ExecutionIntentStatus::CalldataReady,
        1_700_000_100_000,
    )
    .await
    .expect("update status");

    // Reload and assert.
    let reloaded = repo
        .get_execution_intent(outcome.uuid)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(reloaded.status, ExecutionIntentStatus::CalldataReady);
    let stored_sigs = repo
        .get_execution_intent_signatures(outcome.uuid)
        .await
        .unwrap();
    assert_eq!(stored_sigs.buyer_sig.as_deref(), Some(buyer_sig.as_str()));
    assert_eq!(stored_sigs.seller_sig.as_deref(), Some(seller_sig.as_str()));
}

// ================================================================
// 3. Worker-selection proof — unsigned intent NOT broadcastable
// ================================================================
//
// The worker's canonical selection query is
// `PgRepository::list_pending_execution_intents(limit)` which returns
// rows with status='pending'. That set INCLUDES our prepared row (it
// starts in Pending). What we PROVE here is the OTHER half of the
// invariant:
//
// For any Pending row selected by the worker, the subsequent path
// (`get_execution_intent_signatures` → `build_execution_transaction_request`)
// FAILS CLOSED with `MissingTradeSignatures` unless both sigs are
// persisted. This test loads the prepared row through the real
// repository and drives the same fail-closed path.

#[tokio::test]
async fn unsigned_pending_intent_fails_build_execution_transaction_request() {
    let Some(url) = pg_url() else {
        eprintln!("IGNORED [unsigned_pending_intent_fails_build_execution_transaction_request] (PG url not provided)");
        return;
    };
    use deopt_v2_backend::execution::build_execution_transaction_request;
    use deopt_v2_backend::execution::ExecutionConfig;

    let repo = fresh_repo(&url).await;
    let state = state_with_closed_test(repo.clone());
    let req = mk_request();
    let nonce_reader = StaticNonceReader {
        buyer_addr: BUYER_ADDR.to_string(),
        buyer: 0,
        seller_addr: SELLER_ADDR.to_string(),
        seller: 0,
    };
    let mark_reader = StaticMarkPriceReader {
        price: 240_000_000_000,
    };
    let outcome = prepare_trade_core(
        &state,
        &req,
        &nonce_reader,
        &mark_reader,
        Some(1_700_000_000_000),
    )
    .await
    .expect("prepare");
    repo.insert_execution_intent_row(&outcome.execution_intent)
        .await
        .expect("insert");

    // Reload signatures — none persisted yet.
    let sigs = repo
        .get_execution_intent_signatures(outcome.uuid)
        .await
        .unwrap();
    // Drive the same path the executor tick would.
    let intent = repo
        .get_execution_intent(outcome.uuid)
        .await
        .unwrap()
        .unwrap();
    let cfg = ExecutionConfig {
        perp_matching_engine_address: outcome.domain.verifying_contract.clone(),
        require_simulation_ok: false,
        executor_chain_id: CHAIN_ID,
        max_gas_limit: 1_000_000,
        max_fee_per_gas_wei: Some("1000000000".to_string()),
        max_priority_fee_per_gas_wei: Some("100000000".to_string()),
        ..ExecutionConfig::disabled()
    };
    // First: prove pre-cosign fails with MissingTradeSignatures.
    let err = build_execution_transaction_request(&cfg, &intent, &sigs).unwrap_err();
    assert!(matches!(err, BackendError::MissingTradeSignatures));

    // Then: cosign persists both sigs; retry succeeds.
    let digest = perp_trade_v1_digest_bytes(&outcome.payload, &outcome.domain).unwrap();
    let buyer_sig = sign_v1_with(BUYER_KEY, &digest);
    let seller_sig = sign_v1_with(SELLER_KEY, &digest);
    repo.upsert_execution_intent_signatures(
        outcome.uuid,
        Some(buyer_sig.clone()),
        Some(seller_sig.clone()),
        1_700_000_100_000,
    )
    .await
    .unwrap();
    repo.update_execution_intent_status(
        outcome.uuid,
        ExecutionIntentStatus::SimulationOk,
        1_700_000_100_000,
    )
    .await
    .unwrap();
    let intent_after = repo
        .get_execution_intent(outcome.uuid)
        .await
        .unwrap()
        .unwrap();
    let cfg_sim = ExecutionConfig {
        require_simulation_ok: true,
        ..cfg
    };
    let sigs_after = repo
        .get_execution_intent_signatures(outcome.uuid)
        .await
        .unwrap();
    let request = build_execution_transaction_request(&cfg_sim, &intent_after, &sigs_after)
        .expect("post-cosign build succeeds");
    // First 4 bytes = executeTrade V1 selector 0x7a708c4c.
    let selector = &request.calldata[..4];
    assert_eq!(
        format!(
            "{:02x}{:02x}{:02x}{:02x}",
            selector[0], selector[1], selector[2], selector[3]
        ),
        "7a708c4c"
    );
    assert_eq!(request.to.0.to_lowercase(), PME);
}

// ================================================================
// 4. Idempotent + conflicting cosign against real PG
// ================================================================

#[tokio::test]
async fn cosign_idempotent_and_conflict_semantics_against_pg() {
    let Some(url) = pg_url() else {
        eprintln!(
            "IGNORED [cosign_idempotent_and_conflict_semantics_against_pg] (PG url not provided)"
        );
        return;
    };
    let repo = fresh_repo(&url).await;
    let state = state_with_closed_test(repo.clone());
    let req = mk_request();
    let nonce_reader = StaticNonceReader {
        buyer_addr: BUYER_ADDR.to_string(),
        buyer: 0,
        seller_addr: SELLER_ADDR.to_string(),
        seller: 0,
    };
    let mark_reader = StaticMarkPriceReader {
        price: 240_000_000_000,
    };
    let outcome = prepare_trade_core(
        &state,
        &req,
        &nonce_reader,
        &mark_reader,
        Some(1_700_000_000_000),
    )
    .await
    .unwrap();
    repo.insert_execution_intent_row(&outcome.execution_intent)
        .await
        .unwrap();
    let digest = perp_trade_v1_digest_bytes(&outcome.payload, &outcome.domain).unwrap();
    let buyer_sig = sign_v1_with(BUYER_KEY, &digest);
    let seller_sig = sign_v1_with(SELLER_KEY, &digest);

    // Round 1 — no prior sigs, cosign succeeds.
    let intent1 = repo
        .get_execution_intent(outcome.uuid)
        .await
        .unwrap()
        .unwrap();
    let stored_prior = StoredTradeSignatures::default();
    let prior_view = StoredTradeSignaturesView::from(&stored_prior);
    let cosign_req_ok = CosignTradeRequest {
        buyer_signature: buyer_sig.clone(),
        seller_signature: seller_sig.clone(),
    };
    let verified = cosign_load_and_verify(
        &intent1,
        &outcome.domain,
        &cosign_req_ok,
        Some(&prior_view),
        1_700_000_100,
    )
    .unwrap();
    let bundle_buyer_hex = bytes_to_hex(&verified.bundle.buyer_sig);
    let bundle_seller_hex = bytes_to_hex(&verified.bundle.seller_sig);
    repo.upsert_execution_intent_signatures(
        outcome.uuid,
        Some(bundle_buyer_hex),
        Some(bundle_seller_hex),
        1_700_000_100_000,
    )
    .await
    .unwrap();

    // Round 2 — same bundle idempotent OK.
    let intent2 = repo
        .get_execution_intent(outcome.uuid)
        .await
        .unwrap()
        .unwrap();
    let stored_now = repo
        .get_execution_intent_signatures(outcome.uuid)
        .await
        .unwrap();
    let prior_view2 = StoredTradeSignaturesView::from(&stored_now);
    let cosign_req_idempo = CosignTradeRequest {
        buyer_signature: stored_now.buyer_sig.clone().unwrap(),
        seller_signature: stored_now.seller_sig.clone().unwrap(),
    };
    let _ok = cosign_load_and_verify(
        &intent2,
        &outcome.domain,
        &cosign_req_idempo,
        Some(&prior_view2),
        1_700_000_200,
    )
    .unwrap();

    // Round 3 — different buyer_sig → conflict rejected.
    let different = format!("0x{}", "cc".repeat(65));
    let cosign_req_conflict = CosignTradeRequest {
        buyer_signature: different,
        seller_signature: stored_now.seller_sig.clone().unwrap(),
    };
    let err = cosign_load_and_verify(
        &intent2,
        &outcome.domain,
        &cosign_req_conflict,
        Some(&prior_view2),
        1_700_000_300,
    )
    .unwrap_err();
    assert!(matches!(err, BackendError::BroadcastRejected(msg) if msg.contains("cosign_conflict")));
}
