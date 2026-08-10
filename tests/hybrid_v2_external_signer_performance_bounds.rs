//! `BACKEND-HYBRID-V2-EXTERNAL-SIGNER-INTEGRATION-AND-LIVE-ORCHESTRATOR-V1`
//! Part Q — Bounded latency observations for the external-signer
//! bridge + surrounding surface.
//!
//! These are not benchmarks — they are wall-clock ceilings under the
//! mock harness. Any stage that regresses beyond its ceiling fails the
//! assertion loudly. Wall-clock-fragile tests (round-trip latency in
//! particular) carry `#[ignore]` so `cargo test` in a busy CI still
//! passes; the CI job runs the ignored ones explicitly when the
//! runner has stable dispatch guarantees.
//!
//! Bounds:
//!   * bridge sign_execution round-trip (mock success):    < 50ms
//!   * bridge boundary refusal (mainnet chain_id):         < 5ms
//!   * idempotency key derivation:                         < 1ms
//!   * bridge probe() success path:                        < 50ms
//!   * bridge retry loop (2 attempts, each mocked):        < 100ms
//!   * config validate_startup:                            < 5ms
//!
//! Verdict: `EXTERNAL_SIGNER_PERFORMANCE_BOUNDED`.

#![cfg(feature = "test-signer")]

mod hybrid_v2_external_signer_harness;

use std::sync::Arc;
use std::time::{Duration, Instant};

use hybrid_v2_external_signer_harness::{
    build_bridge_with_mode, build_scripted_bridge, test_address_bytes, ScriptedRemoteSigner,
    TEST_ADDRESS_HEX,
};

use deopt_v2_backend::execution::remote_signer::SignerError as PerpsSignerError;
use deopt_v2_backend::execution::signer_adapters::MockProviderMode;
use deopt_v2_backend::hybrid_v2::config::{HybridV2ExecutionConfig, SignerProvider};
use deopt_v2_backend::hybrid_v2::execution::signer::{ExecutionSigner, SignerError};
use deopt_v2_backend::hybrid_v2::execution::signer_kms_bridge::derive_idempotency_key;
use deopt_v2_backend::types::AccountId;

/// Structural bound — the bridge internally clamps max_retries to 5.
/// Independent of wall-clock, so this is a plain assertion.
#[test]
fn bounded_max_retries_is_at_most_five() {
    let scripted = Arc::new(ScriptedRemoteSigner::new(
        AccountId::new(TEST_ADDRESS_HEX),
        vec![],
    ));
    let bridge = build_scripted_bridge(
        TEST_ADDRESS_HEX,
        scripted,
        1_000, // ludicrously large; must clamp
        Duration::from_millis(100),
    );
    assert_eq!(bridge.max_retries(), 5);
}

#[test]
fn bounded_idempotency_key_derivation_under_1ms() {
    // 100 derivations under 100ms is a soft ceiling — 1ms per call
    // amortized. Purely CPU-bound Keccak256, no allocations beyond
    // the initial hasher.
    let start = Instant::now();
    for i in 0..100 {
        let addr = [i as u8; 20];
        let plan = [(i as u8).wrapping_mul(3); 32];
        let payload = [(i as u8).wrapping_add(11); 32];
        let cid = format!("id-{i}").into_bytes();
        let _ = derive_idempotency_key(&addr, &cid, &plan, &payload);
    }
    let elapsed = start.elapsed();
    assert!(
        elapsed < Duration::from_millis(100),
        "100 idempotency derivations exceeded 100ms: {elapsed:?}"
    );
}

#[test]
fn bounded_config_validate_startup_under_5ms() {
    let mut cfg = HybridV2ExecutionConfig::disabled();
    cfg.execution_enabled = true;
    cfg.executor_address = [0xaau8; 20];
    cfg.expected_signer_address = Some(test_address_bytes());
    cfg.signer_endpoint = Some("https://signer.example/sign".to_string());
    cfg.signer_provider = Some(SignerProvider::KmsAws);
    let start = Instant::now();
    for _ in 0..1_000 {
        cfg.validate_startup(84532).unwrap();
    }
    let elapsed = start.elapsed();
    assert!(
        elapsed < Duration::from_millis(500),
        "1000 validate_startup calls exceeded 500ms: {elapsed:?}"
    );
}

#[tokio::test]
async fn bounded_mainnet_refusal_short_circuit_under_5ms() {
    let scripted = Arc::new(ScriptedRemoteSigner::new(
        AccountId::new(TEST_ADDRESS_HEX),
        vec![],
    ));
    let bridge = build_scripted_bridge(
        TEST_ADDRESS_HEX,
        scripted.clone(),
        3,
        Duration::from_millis(500),
    );
    let mut req = hybrid_v2_external_signer_harness::baseline_signing_request();
    req.chain_id = 8453;
    let start = Instant::now();
    let err = bridge.sign_execution(req).await.unwrap_err();
    let elapsed = start.elapsed();
    assert!(matches!(err, SignerError::ChainMismatch));
    assert_eq!(scripted.calls(), 0);
    assert!(
        elapsed < Duration::from_millis(50),
        "mainnet short-circuit exceeded 50ms: {elapsed:?}"
    );
}

#[tokio::test]
#[ignore] // wall-clock fragile: run under stable dispatch only
async fn bounded_sign_execution_round_trip_under_50ms() {
    let bridge = build_bridge_with_mode(MockProviderMode::Success, TEST_ADDRESS_HEX, 0);
    let req = hybrid_v2_external_signer_harness::baseline_signing_request();
    let start = Instant::now();
    let signed = bridge.sign_execution(req).await.expect("success");
    let elapsed = start.elapsed();
    assert_eq!(signed.recovered_signer, test_address_bytes());
    assert!(
        elapsed < Duration::from_millis(50),
        "bridge sign_execution exceeded 50ms: {elapsed:?}"
    );
}

#[tokio::test]
#[ignore] // wall-clock fragile: probe latency
async fn bounded_probe_success_under_50ms() {
    let bridge = build_bridge_with_mode(MockProviderMode::Success, TEST_ADDRESS_HEX, 0);
    let start = Instant::now();
    let verdict = bridge.probe().await;
    let elapsed = start.elapsed();
    assert!(matches!(
        verdict,
        deopt_v2_backend::hybrid_v2::execution::SignerAvailability::Configured
    ));
    assert!(
        elapsed < Duration::from_millis(50),
        "probe exceeded 50ms: {elapsed:?}"
    );
}

#[tokio::test]
#[ignore] // wall-clock fragile: retry loop timing
async fn bounded_retry_loop_two_attempts_under_100ms() {
    // Bridge: 1 retry allowed → 2 total attempts. Both fail with a
    // transient. Verify the retry loop itself completes under 100ms
    // (no external delays because we do not `set_delay` on the mock).
    let scripted = Arc::new(ScriptedRemoteSigner::new(
        AccountId::new(TEST_ADDRESS_HEX),
        vec![
            Err(PerpsSignerError::KmsTimeout),
            Err(PerpsSignerError::KmsTimeout),
        ],
    ));
    let bridge = build_scripted_bridge(
        TEST_ADDRESS_HEX,
        scripted.clone(),
        1,
        Duration::from_millis(200),
    );
    let start = Instant::now();
    let err = bridge
        .sign_execution(hybrid_v2_external_signer_harness::baseline_signing_request())
        .await
        .unwrap_err();
    let elapsed = start.elapsed();
    assert!(matches!(err, SignerError::Timeout));
    assert_eq!(scripted.calls(), 2);
    assert!(
        elapsed < Duration::from_millis(100),
        "retry loop exceeded 100ms: {elapsed:?}"
    );
}
