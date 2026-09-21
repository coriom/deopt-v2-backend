//! PERPS_V2_BACKEND_ANVIL_LIVE_E2E_INTEGRATION_V1 — real V2 backend
//! + Anvil + PostgreSQL integration coverage (non-broadcast lifecycle
//! + preflight negatives).
//!
//! The reusable V2 spawn harness (`V2E2eEnv`) now lives in
//! `tests/perps_v2_anvil_shared/mod.rs`; this file only owns the tests
//! themselves.
//!
//! # Hard rules
//!
//! * NO Base Sepolia write. NO public-chain deployment. NO
//!   `sendRawTransaction`. NO executor arming. NO real trader
//!   keystore. NO LocalKeystore. Anvil + local PostgreSQL only.

#![allow(dead_code)]

mod perps_v2_anvil_shared;

use deopt_v2_backend::execution::v2_readiness::{
    MigrationState, V2PreflightDenial, V2PreflightOutcome,
};
use deopt_v2_backend::execution::{
    decode_execute_trade_v2_calldata, encode_execute_trade_v2_calldata, execute_trade_v2_selector,
    perp_trade_v2_digest_bytes, ExecutionIntentStatus, PerpTradeDomain, PerpTradePayload,
    PerpTradeSignatureBundle, PerpsProtocolVersion,
};
use deopt_v2_backend::types::AccountId;

use perps_v2_anvil_shared::{
    build_close_candidate, decode_hex_bytes, hex_no_prefix, pg_url_or_ignore, sign_digest_65,
    spawn_or_ignore, V2SpawnOpts, CANDIDATE_PRICE_1E8, CANDIDATE_SIZE_1E8, MARKET_ID,
    POSITIVE_CLEARING_FLOOR_RAW,
};

// =====================================================================
// TEST 1 — POSITIVE lifecycle (readiness → prepare → PG → cosign →
// simulation_ok → zero-send + decoded-calldata proofs).
// =====================================================================

#[tokio::test]
async fn v2_backend_anvil_live_positive_lifecycle() {
    let label = "v2_positive_lifecycle";
    if pg_url_or_ignore(label).is_none() {
        return;
    }
    let Some(env) = spawn_or_ignore(label, V2SpawnOpts::default()).await else {
        return;
    };

    eprintln!(
        "V2E2E deployed addresses:\n\
         vault={}\n perpEngineV2={}\n perpMatchingEngineV2={}\n \
         perpClearingAccountV2={}\n oracleRouter={}\n \
         perpMarketRegistry={}\n usdc={}\n executor={}",
        env.contracts.vault,
        env.contracts.perp_engine_v2,
        env.contracts.perp_matching_engine_v2,
        env.contracts.perp_clearing_account_v2,
        env.contracts.oracle_router,
        env.contracts.perp_market_registry,
        env.contracts.usdc,
        env.contracts.executor
    );

    // ── (A) Preflight — Ready ─────────────────────────────────────
    let preflight = env.run_v2_preflight().await;
    match &preflight.outcome {
        V2PreflightOutcome::Ready => {}
        V2PreflightOutcome::Denied(d) => {
            panic!(
                "expected Ready; got Denied({:?}). report={:?}",
                d, preflight
            )
        }
    }
    assert_eq!(preflight.migration_state, Some(MigrationState::Sealed));
    assert!(
        preflight
            .migration_snapshot_hash
            .expect("snapshot hash present")
            != [0u8; 32]
    );
    assert_eq!(preflight.pme_is_executor, Some(true));
    assert_eq!(preflight.pme_paused, Some(false));
    assert!(
        preflight.clearing_balance_raw.expect("balance present") >= POSITIVE_CLEARING_FLOOR_RAW
    );

    // ── (B) HTTP prepare (real route, non-trivial bounds) ────────
    let request = build_close_candidate(&env);
    let expected_max_bound: u128 = request
        .max_execution_price_1e8
        .as_ref()
        .unwrap()
        .parse()
        .unwrap();
    let expected_min_bound: u128 = request
        .min_execution_price_1e8
        .as_ref()
        .unwrap()
        .parse()
        .unwrap();
    assert!(expected_max_bound > 0);
    assert!(expected_min_bound > 0);
    let prepared = env.http_prepare(&request).await.expect("http prepare");
    let uuid = uuid::Uuid::parse_str(&prepared.uuid).expect("prepared uuid");

    let domain = prepared
        .typed_data
        .get("domain")
        .expect("typedData.domain")
        .clone();
    assert_eq!(
        domain.get("name").and_then(|v| v.as_str()),
        Some("DeOptV2-PerpMatchingEngine")
    );
    assert_eq!(domain.get("version").and_then(|v| v.as_str()), Some("2"));
    assert_eq!(
        domain
            .get("verifyingContract")
            .and_then(|v| v.as_str())
            .map(|s| s.to_ascii_lowercase()),
        Some(env.contracts.perp_matching_engine_v2.clone())
    );
    let msg = prepared
        .typed_data
        .get("message")
        .expect("typedData.message");
    for field in [
        "intentId",
        "buyer",
        "seller",
        "marketId",
        "sizeDelta1e8",
        "executionPrice1e8",
        "maxExecutionPrice1e8",
        "minExecutionPrice1e8",
        "buyerIsMaker",
        "buyerNonce",
        "sellerNonce",
        "deadline",
    ] {
        assert!(
            msg.get(field).is_some(),
            "typedData.message missing field {field}: {}",
            prepared.typed_data
        );
    }
    assert_ne!(prepared.trade.max_execution_price_1e8, "0");
    assert_ne!(prepared.trade.min_execution_price_1e8, "0");
    let max_bound: u128 = prepared.trade.max_execution_price_1e8.parse().unwrap();
    let min_bound: u128 = prepared.trade.min_execution_price_1e8.parse().unwrap();
    let exec_price: u128 = prepared.trade.execution_price_1e8.parse().unwrap();
    assert!(min_bound <= exec_price && exec_price <= max_bound);
    assert_eq!(max_bound, expected_max_bound);
    assert_eq!(min_bound, expected_min_bound);

    // ── (C) Reload from Postgres and prove all 12 fields survive ─
    let intent = env
        .repository
        .get_execution_intent(uuid)
        .await
        .expect("pg reload")
        .expect("row exists");
    assert_eq!(intent.protocol_version, PerpsProtocolVersion::V2);
    assert_eq!(intent.status, ExecutionIntentStatus::Pending);
    assert_eq!(intent.buyer_nonce, Some(0));
    assert_eq!(intent.seller_nonce, Some(0));
    assert_eq!(u128::from(intent.size_1e8), CANDIDATE_SIZE_1E8);
    assert_eq!(u128::from(intent.price_1e8), CANDIDATE_PRICE_1E8);
    assert_eq!(intent.max_execution_price_1e8, max_bound);
    assert_eq!(intent.min_execution_price_1e8, min_bound);
    assert!(intent.buyer_is_maker == Some(true));

    let v2_verifying = AccountId::new(env.contracts.perp_matching_engine_v2.clone());
    let reload_domain =
        PerpTradeDomain::for_version(intent.protocol_version, env.chain_id, v2_verifying.clone());
    assert_eq!(reload_domain.version, "2");

    let payload_after: PerpTradePayload = intent.perp_trade_payload().expect("payload reconstruct");
    assert_eq!(
        format!("0x{}", hex_no_prefix(payload_after.intent_id.as_slice())),
        prepared.intent_id_hex
    );
    assert_eq!(
        payload_after.buyer.0.to_ascii_lowercase(),
        env.wallets.trader_b.address
    );
    assert_eq!(
        payload_after.seller.0.to_ascii_lowercase(),
        env.wallets.trader_a.address
    );
    assert_eq!(payload_after.market_id, MARKET_ID);
    assert_eq!(payload_after.size_delta_1e8, CANDIDATE_SIZE_1E8);
    assert_eq!(payload_after.execution_price_1e8, CANDIDATE_PRICE_1E8);
    assert_eq!(payload_after.max_execution_price_1e8, max_bound);
    assert_eq!(payload_after.min_execution_price_1e8, min_bound);
    assert!(payload_after.buyer_is_maker);
    assert_eq!(payload_after.buyer_nonce, 0);
    assert_eq!(payload_after.seller_nonce, 0);

    let digest_before = decode_hex_bytes(&prepared.digest);
    assert_eq!(digest_before.len(), 32);
    let digest_after =
        perp_trade_v2_digest_bytes(&payload_after, &reload_domain).expect("digest_after");
    assert_eq!(
        digest_before.as_slice(),
        digest_after.as_slice(),
        "V2 digest must survive PG round-trip byte-for-byte"
    );

    // ── (D) Active-version flip durability ───────────────────────
    let mut flipped = (*env.state).clone();
    flipped.execution_config.perps_active_engine_version = PerpsProtocolVersion::V1;
    let intent_after_flip = env
        .repository
        .get_execution_intent(uuid)
        .await
        .expect("reload during flip")
        .expect("row present during flip");
    assert_eq!(
        intent_after_flip.protocol_version,
        PerpsProtocolVersion::V2,
        "persisted intent's protocol_version must be immutable under runtime flip"
    );
    let flip_verifying = flipped
        .execution_config
        .perp_matching_engine_address_for(intent_after_flip.protocol_version)
        .expect("verifying for V2")
        .clone();
    let flip_domain = PerpTradeDomain::for_version(
        intent_after_flip.protocol_version,
        flipped.perps_read_config.chain_id,
        flip_verifying,
    );
    let payload_after_flip = intent_after_flip
        .perp_trade_payload()
        .expect("payload during flip");
    let digest_after_flip =
        perp_trade_v2_digest_bytes(&payload_after_flip, &flip_domain).expect("digest during flip");
    assert_eq!(
        digest_after_flip.as_slice(),
        digest_before.as_slice(),
        "runtime active-version flip MUST NOT alter the digest of an already-persisted V2 intent"
    );
    assert_eq!(payload_after_flip.max_execution_price_1e8, max_bound);
    assert_eq!(payload_after_flip.min_execution_price_1e8, min_bound);

    // ── (E) Ephemeral signing with local trader keys ─────────────
    let digest_bytes: [u8; 32] = digest_before
        .as_slice()
        .try_into()
        .expect("digest is 32 bytes");
    let buyer_sig = sign_digest_65(&env.wallets.trader_b.signer, &digest_bytes);
    let seller_sig = sign_digest_65(&env.wallets.trader_a.signer, &digest_bytes);
    let recovered_buyer =
        deopt_v2_backend::signing::recover_eip712_signer(&digest_bytes, &buyer_sig)
            .expect("recover buyer");
    assert_eq!(
        recovered_buyer.0.to_ascii_lowercase(),
        env.wallets.trader_b.address
    );
    let recovered_seller =
        deopt_v2_backend::signing::recover_eip712_signer(&digest_bytes, &seller_sig)
            .expect("recover seller");
    assert_eq!(
        recovered_seller.0.to_ascii_lowercase(),
        env.wallets.trader_a.address
    );

    // ── (F) Real HTTP cosign after PG reload ─────────────────────
    let cosign_response = env
        .http_cosign(uuid, &buyer_sig, &seller_sig)
        .await
        .expect("http cosign");
    assert!(cosign_response.calldata_ready);
    let signatures = env
        .repository
        .get_execution_intent_signatures(uuid)
        .await
        .expect("signatures reload");
    assert!(signatures.buyer_signature_present());
    assert!(signatures.seller_signature_present());

    let intent_after_cosign = env
        .repository
        .get_execution_intent(uuid)
        .await
        .expect("reload after cosign")
        .expect("row present after cosign");
    assert_eq!(
        intent_after_cosign.status,
        ExecutionIntentStatus::CalldataReady
    );

    let preflight_2 = env.run_v2_preflight().await;
    assert!(
        preflight_2.outcome.is_ready(),
        "preflight 2: {preflight_2:?}"
    );

    let nonce_before = env.executor_anvil_nonce().await.expect("executor nonce");

    // ── (H) Real HTTP simulate → simulation_ok ───────────────────
    let sim = env.http_simulate(uuid).await.expect("http simulate");
    assert_eq!(
        sim.get("simulation_status").and_then(|v| v.as_str()),
        Some("simulation_ok"),
        "sim body={sim}"
    );
    assert!(sim.get("error").map(|v| v.is_null()).unwrap_or(true));
    assert!(sim.get("revert_data").map(|v| v.is_null()).unwrap_or(true));
    assert!(sim
        .get("revert_selector")
        .map(|v| v.is_null())
        .unwrap_or(true));

    let intent_final = env
        .repository
        .get_execution_intent(uuid)
        .await
        .expect("reload final")
        .expect("row present final");
    assert_eq!(intent_final.status, ExecutionIntentStatus::SimulationOk);

    // ── (I) Independent V2 calldata decode ───────────────────────
    let stored_signatures = signatures.clone();
    let bundle = PerpTradeSignatureBundle::new(
        stored_signatures.buyer_sig.as_deref().unwrap(),
        stored_signatures.seller_sig.as_deref().unwrap(),
    )
    .expect("bundle build");
    let calldata =
        encode_execute_trade_v2_calldata(&payload_after, &bundle).expect("v2 calldata encode");
    assert_eq!(&calldata[..4], &execute_trade_v2_selector()[..]);

    let (decoded_tuple, _decoded_buyer_sig, _decoded_seller_sig) =
        decode_execute_trade_v2_calldata(&calldata).expect("v2 calldata decode");

    assert_eq!(decoded_tuple.intentId, payload_after.intent_id);
    let decoded_buyer_addr = format!("0x{}", hex_no_prefix(decoded_tuple.buyer.as_slice()));
    let decoded_seller_addr = format!("0x{}", hex_no_prefix(decoded_tuple.seller.as_slice()));
    assert_eq!(
        decoded_buyer_addr.to_ascii_lowercase(),
        payload_after.buyer.0.to_ascii_lowercase()
    );
    assert_eq!(
        decoded_seller_addr.to_ascii_lowercase(),
        payload_after.seller.0.to_ascii_lowercase()
    );
    assert_eq!(
        u128::try_from(decoded_tuple.marketId).expect("marketId fits u128"),
        payload_after.market_id
    );
    assert_eq!(decoded_tuple.sizeDelta1e8, payload_after.size_delta_1e8);
    assert_eq!(
        decoded_tuple.executionPrice1e8,
        payload_after.execution_price_1e8
    );
    assert_eq!(
        decoded_tuple.maxExecutionPrice1e8,
        payload_after.max_execution_price_1e8
    );
    assert_eq!(
        decoded_tuple.minExecutionPrice1e8,
        payload_after.min_execution_price_1e8
    );
    assert_eq!(decoded_tuple.buyerIsMaker, payload_after.buyer_is_maker);
    assert_eq!(
        u128::try_from(decoded_tuple.buyerNonce).expect("buyerNonce fits u128"),
        payload_after.buyer_nonce
    );
    assert_eq!(
        u128::try_from(decoded_tuple.sellerNonce).expect("sellerNonce fits u128"),
        payload_after.seller_nonce
    );
    assert_eq!(
        u128::try_from(decoded_tuple.deadline).expect("deadline fits u128"),
        payload_after.deadline
    );

    // ── (J) Zero-send proof ──────────────────────────────────────
    let nonce_after = env.executor_anvil_nonce().await.expect("post-nonce");
    assert_eq!(
        nonce_before, nonce_after,
        "executor Anvil nonce MUST NOT change after eth_call simulation"
    );

    let submitted = env
        .repository
        .find_submitted_transaction_by_intent(uuid)
        .await
        .expect("submitted lookup");
    assert!(
        submitted.is_none(),
        "no execution_transactions row expected after simulation-only lifecycle"
    );

    env.shutdown().await.expect("clean shutdown");
    eprintln!("V2E2E_POSITIVE_LIFECYCLE_OK");
}

// =====================================================================
// NEGATIVES — five REAL preflight refusal branches.
// =====================================================================

#[tokio::test]
async fn v2_negative_migration_open_denies_preflight() {
    let label = "v2_negative_migration_open";
    if pg_url_or_ignore(label).is_none() {
        return;
    }
    let opts = V2SpawnOpts {
        seal_migration: false,
        ..V2SpawnOpts::default()
    };
    let Some(env) = spawn_or_ignore(label, opts).await else {
        return;
    };
    let report = env.run_v2_preflight().await;
    match &report.outcome {
        V2PreflightOutcome::Denied(V2PreflightDenial::MigrationOpen) => {}
        other => panic!("expected MigrationOpen; got {other:?}. report={report:?}"),
    }
    env.shutdown().await.expect("clean shutdown");
}

#[tokio::test]
async fn v2_negative_clearing_account_mismatch_denies_preflight() {
    let label = "v2_negative_clearing_mismatch";
    if pg_url_or_ignore(label).is_none() {
        return;
    }
    let Some(env) = spawn_or_ignore(label, V2SpawnOpts::default()).await else {
        return;
    };
    let bogus_clearing = AccountId::new(env.contracts.deployer.clone());
    let report = env
        .run_v2_preflight_with(
            &AccountId::new(env.contracts.perp_engine_v2.clone()),
            &AccountId::new(env.contracts.perp_matching_engine_v2.clone()),
            &bogus_clearing,
            env.opts.clearing_min_balance_raw,
        )
        .await;
    match &report.outcome {
        V2PreflightOutcome::Denied(V2PreflightDenial::ClearingAccountMismatch { .. }) => {}
        other => panic!("expected ClearingAccountMismatch; got {other:?}. report={report:?}"),
    }
    env.shutdown().await.expect("clean shutdown");
}

#[tokio::test]
async fn v2_negative_clearing_floor_denies_then_lifts() {
    let label = "v2_negative_clearing_floor";
    if pg_url_or_ignore(label).is_none() {
        return;
    }
    let Some(env) = spawn_or_ignore(label, V2SpawnOpts::default()).await else {
        return;
    };

    let ready = env.run_v2_preflight().await;
    let observed = ready
        .clearing_balance_raw
        .expect("clearing balance observed");
    let too_high_floor = observed + 1;
    let report_denied = env
        .run_v2_preflight_with(
            &AccountId::new(env.contracts.perp_engine_v2.clone()),
            &AccountId::new(env.contracts.perp_matching_engine_v2.clone()),
            &AccountId::new(env.contracts.perp_clearing_account_v2.clone()),
            too_high_floor,
        )
        .await;
    match &report_denied.outcome {
        V2PreflightOutcome::Denied(V2PreflightDenial::ClearingBalanceBelowFloor {
            observed: obs,
            floor,
        }) => {
            assert_eq!(*obs, observed);
            assert_eq!(*floor, too_high_floor);
        }
        other => {
            panic!("expected ClearingBalanceBelowFloor; got {other:?}. report={report_denied:?}")
        }
    }

    let report_ok = env
        .run_v2_preflight_with(
            &AccountId::new(env.contracts.perp_engine_v2.clone()),
            &AccountId::new(env.contracts.perp_matching_engine_v2.clone()),
            &AccountId::new(env.contracts.perp_clearing_account_v2.clone()),
            observed,
        )
        .await;
    assert!(
        report_ok.outcome.is_ready(),
        "expected Ready; got {report_ok:?}"
    );
    env.shutdown().await.expect("clean shutdown");
}

#[tokio::test]
async fn v2_negative_executor_not_authorized_denies_then_lifts() {
    let label = "v2_negative_executor_auth";
    if pg_url_or_ignore(label).is_none() {
        return;
    }
    let Some(env) = spawn_or_ignore(label, V2SpawnOpts::default()).await else {
        return;
    };

    env.cast_set_executor(false)
        .await
        .expect("setExecutor(false)");
    let report_denied = env.run_v2_preflight().await;
    match &report_denied.outcome {
        V2PreflightOutcome::Denied(V2PreflightDenial::ExecutorNotAuthorized(addr)) => {
            assert_eq!(addr.0.to_ascii_lowercase(), env.contracts.executor);
        }
        other => panic!("expected ExecutorNotAuthorized; got {other:?}. report={report_denied:?}"),
    }

    env.cast_set_executor(true)
        .await
        .expect("setExecutor(true) restore");
    let report_ok = env.run_v2_preflight().await;
    assert!(
        report_ok.outcome.is_ready(),
        "expected Ready; got {report_ok:?}"
    );
    env.shutdown().await.expect("clean shutdown");
}

#[tokio::test]
async fn v2_negative_pme_engine_linkage_mismatch_denies_preflight() {
    let label = "v2_negative_pme_engine_linkage";
    if pg_url_or_ignore(label).is_none() {
        return;
    }
    let Some(env) = spawn_or_ignore(label, V2SpawnOpts::default()).await else {
        return;
    };
    let bogus_engine = AccountId::new(env.contracts.deployer.clone());
    let report = env
        .run_v2_preflight_with(
            &bogus_engine,
            &AccountId::new(env.contracts.perp_matching_engine_v2.clone()),
            &AccountId::new(env.contracts.perp_clearing_account_v2.clone()),
            env.opts.clearing_min_balance_raw,
        )
        .await;
    match &report.outcome {
        V2PreflightOutcome::Denied(V2PreflightDenial::PmeEngineLinkageMismatch { .. }) => {}
        V2PreflightOutcome::Denied(V2PreflightDenial::UpstreamRpcError(_)) => {}
        V2PreflightOutcome::Denied(V2PreflightDenial::ClearingAccountMismatch { .. }) => {}
        other => panic!(
            "expected PmeEngineLinkageMismatch / UpstreamRpcError / ClearingAccountMismatch \
             (fail-closed on bogus engine); got {other:?}. report={report:?}"
        ),
    }
    env.shutdown().await.expect("clean shutdown");
}
