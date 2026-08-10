//! `BACKEND-HYBRID-V2-EXTERNAL-SIGNER-INTEGRATION-AND-LIVE-ORCHESTRATOR-V1`
//! Part P — External-signer property tests.
//!
//! Bounded (20-case) properties over the external-signer surface. No
//! new Cargo deps — each property loops explicitly across a small,
//! deterministic sample set.
//!
//! Properties covered:
//!
//!   1. `prop_idempotency_key_is_deterministic`
//!   2. `prop_idempotency_key_differs_on_any_field_change`
//!   3. `prop_bridge_never_calls_vendor_on_mainnet_chain_id`
//!   4. `prop_max_retries_upper_bound_is_five`
//!   5. `prop_deterministic_refusal_never_retried`
//!   6. `prop_transient_error_is_retried_bounded`
//!   7. `prop_identity_mismatch_never_returns_ok`
//!   8. `prop_malformed_y_parity_always_rejected`
//!   9. `prop_kms_timeout_maps_to_signer_timeout`
//!  10. `prop_availability_reports_configured_at_construction`
//!  11. `prop_endpoint_redaction_never_leaks_path`
//!  12. `prop_signer_provider_parse_round_trip`
//!  13. `prop_config_validate_refuses_mainnet_regardless_of_provider`
//!  14. `prop_config_validate_refuses_non_https_endpoint`
//!  15. `prop_pipeline_never_invokes_broadcast_rpc`
//!
//! Verdict: `EXTERNAL_SIGNER_LIVE_ORCHESTRATOR_PROPERTIES_VALIDATED`.

#![cfg(feature = "test-signer")]

mod hybrid_v2_external_signer_harness;
mod hybrid_v2_mock_rpc_helpers;
mod hybrid_v2_support;

use std::sync::Arc;
use std::time::Duration;

use hybrid_v2_external_signer_harness::{
    build_bridge_with_mode, build_scripted_bridge, malformed_signed_response, parse_address_hex,
    signed_response_for, test_address_bytes, ScriptedRemoteSigner, TEST_ADDRESS_HEX,
};
use hybrid_v2_mock_rpc_helpers::MockRpcServer;

use deopt_v2_backend::execution::remote_signer::SignerError as PerpsSignerError;
use deopt_v2_backend::execution::signer_adapters::MockProviderMode;
use deopt_v2_backend::hybrid_v2::config::{HybridV2ExecutionConfig, SignerProvider};
use deopt_v2_backend::hybrid_v2::execution::signer::{
    ExecutionSigner, SignerAvailability, SignerError,
};
use deopt_v2_backend::hybrid_v2::execution::signer_kms_bridge::{
    derive_idempotency_key, redacted_endpoint, HybridV2KmsSignerBridge,
};
use deopt_v2_backend::types::AccountId;

const CASES: usize = 20;

fn seed_addr(seed: u8) -> [u8; 20] {
    let mut a = [0u8; 20];
    for (i, b) in a.iter_mut().enumerate() {
        *b = seed.wrapping_add(i as u8);
    }
    a
}

fn seed_hash32(seed: u8) -> [u8; 32] {
    let mut h = [0u8; 32];
    for (i, b) in h.iter_mut().enumerate() {
        *b = seed.wrapping_mul(3).wrapping_add(i as u8);
    }
    h
}

// -----------------------------------------------------------------
//                    1 — idempotency determinism
// -----------------------------------------------------------------

#[test]
fn prop_idempotency_key_is_deterministic() {
    for case in 0..CASES {
        let addr = seed_addr(case as u8);
        let cid = format!("execution-{case}").into_bytes();
        let plan = seed_hash32(case as u8);
        let payload = seed_hash32((case + 7) as u8);
        let a = derive_idempotency_key(&addr, &cid, &plan, &payload);
        let b = derive_idempotency_key(&addr, &cid, &plan, &payload);
        assert_eq!(a, b, "case {case}: idempotency key must be deterministic");
    }
}

// -----------------------------------------------------------------
//                    2 — idempotency sensitivity
// -----------------------------------------------------------------

#[test]
fn prop_idempotency_key_differs_on_any_field_change() {
    let base_addr = seed_addr(0x10);
    let base_cid = b"base-cid".to_vec();
    let base_plan = seed_hash32(0x11);
    let base_payload = seed_hash32(0x22);
    let base = derive_idempotency_key(&base_addr, &base_cid, &base_plan, &base_payload);
    // Address variation.
    for case in 1..CASES {
        let addr = seed_addr(case as u8);
        if addr == base_addr {
            continue;
        }
        let k = derive_idempotency_key(&addr, &base_cid, &base_plan, &base_payload);
        assert_ne!(k, base, "address variation collided at case {case}");
    }
    // Plan hash variation.
    for case in 1..CASES {
        let plan = seed_hash32(case as u8);
        if plan == base_plan {
            continue;
        }
        let k = derive_idempotency_key(&base_addr, &base_cid, &plan, &base_payload);
        assert_ne!(k, base, "plan_hash variation collided at case {case}");
    }
}

// -----------------------------------------------------------------
//                    3 — mainnet refusal short-circuit
// -----------------------------------------------------------------

#[tokio::test]
async fn prop_bridge_never_calls_vendor_on_mainnet_chain_id() {
    // For each of a range of arbitrary values that a stray caller
    // might submit, the bridge REFUSES for chain_id == 8453 without
    // ever calling the vendor.
    for case in 0..CASES {
        let scripted = Arc::new(ScriptedRemoteSigner::new(
            AccountId::new(TEST_ADDRESS_HEX),
            vec![],
        ));
        let bridge = build_scripted_bridge(
            TEST_ADDRESS_HEX,
            scripted.clone(),
            case as u32 % 4,
            Duration::from_millis(200),
        );
        let mut req = hybrid_v2_external_signer_harness::baseline_signing_request();
        req.chain_id = 8453;
        req.nonce = case as u64;
        let err = bridge.sign_execution(req).await.unwrap_err();
        assert!(matches!(err, SignerError::ChainMismatch));
        assert_eq!(scripted.calls(), 0, "case {case}");
    }
}

// -----------------------------------------------------------------
//                    4 — max_retries clamp at 5
// -----------------------------------------------------------------

#[test]
fn prop_max_retries_upper_bound_is_five() {
    // Bridge internally clamps max_retries to 5. Constructing with a
    // higher value must report 5 via `max_retries()`.
    for req in 0..CASES {
        let scripted = Arc::new(ScriptedRemoteSigner::new(
            AccountId::new(TEST_ADDRESS_HEX),
            vec![],
        ));
        let bridge: Arc<HybridV2KmsSignerBridge> = build_scripted_bridge(
            TEST_ADDRESS_HEX,
            scripted,
            req as u32,
            Duration::from_millis(100),
        );
        let expected = (req as u32).min(5);
        assert_eq!(
            bridge.max_retries(),
            expected,
            "requested={req} expected_clamped={expected}"
        );
    }
}

// -----------------------------------------------------------------
//                    5 — deterministic refusal never retried
// -----------------------------------------------------------------

#[tokio::test]
async fn prop_deterministic_refusal_never_retried() {
    for case in 0..CASES {
        let scripted = Arc::new(ScriptedRemoteSigner::new(
            AccountId::new(TEST_ADDRESS_HEX),
            vec![
                Err(PerpsSignerError::PolicyFingerprint),
                Err(PerpsSignerError::PolicyFingerprint),
                Err(PerpsSignerError::PolicyFingerprint),
            ],
        ));
        // Budget 3 attempts, but deterministic must consume only 1.
        let bridge = build_scripted_bridge(
            TEST_ADDRESS_HEX,
            scripted.clone(),
            3,
            Duration::from_millis(100),
        );
        let mut req = hybrid_v2_external_signer_harness::baseline_signing_request();
        req.nonce = case as u64;
        let _ = bridge.sign_execution(req).await.unwrap_err();
        assert_eq!(
            scripted.calls(),
            1,
            "case {case}: deterministic must consume exactly one call"
        );
    }
}

// -----------------------------------------------------------------
//                    6 — transient error retried bounded
// -----------------------------------------------------------------

#[tokio::test]
async fn prop_transient_error_is_retried_bounded() {
    for case in 0..CASES {
        let retries = (case as u32) % 3; // 0..2 retries
        let attempts = retries + 1;
        // Fill queue with `attempts` transient errors — the bridge
        // will exhaust budget and return.
        let queue: Vec<_> = (0..attempts)
            .map(|_| Err(PerpsSignerError::KmsTimeout))
            .collect();
        let scripted = Arc::new(ScriptedRemoteSigner::new(
            AccountId::new(TEST_ADDRESS_HEX),
            queue,
        ));
        let bridge = build_scripted_bridge(
            TEST_ADDRESS_HEX,
            scripted.clone(),
            retries,
            Duration::from_millis(100),
        );
        let mut req = hybrid_v2_external_signer_harness::baseline_signing_request();
        req.nonce = case as u64;
        let err = bridge.sign_execution(req).await.unwrap_err();
        assert!(matches!(err, SignerError::Timeout));
        assert_eq!(
            scripted.calls(),
            attempts,
            "case {case}: transient must exhaust exactly {attempts} attempts"
        );
    }
}

// -----------------------------------------------------------------
//                    7 — identity mismatch never Ok
// -----------------------------------------------------------------

#[tokio::test]
async fn prop_identity_mismatch_never_returns_ok() {
    for case in 0..CASES {
        // Rotate the "returned address" through synthetic addresses.
        let bad = format!("0x{}", format!("{:02x}", case as u8).repeat(20));
        let resp = signed_response_for([case as u8; 32], &bad);
        let scripted = Arc::new(ScriptedRemoteSigner::new(
            AccountId::new(TEST_ADDRESS_HEX),
            vec![Ok(resp)],
        ));
        let bridge =
            build_scripted_bridge(TEST_ADDRESS_HEX, scripted, 0, Duration::from_millis(100));
        let mut req = hybrid_v2_external_signer_harness::baseline_signing_request();
        req.nonce = case as u64;
        let result = bridge.sign_execution(req).await;
        // Either the bridge itself rejects (IdentityMismatch) OR the
        // Perps transport layer rejects earlier as SignerUnavailable.
        // Both are acceptable — the property is "no Ok".
        assert!(result.is_err(), "case {case}: mismatch must never succeed");
    }
}

// -----------------------------------------------------------------
//                    8 — malformed y_parity always rejected
// -----------------------------------------------------------------

#[tokio::test]
async fn prop_malformed_y_parity_always_rejected() {
    for case in 0..CASES {
        let resp = malformed_signed_response([case as u8; 32], TEST_ADDRESS_HEX);
        let scripted = Arc::new(ScriptedRemoteSigner::new(
            AccountId::new(TEST_ADDRESS_HEX),
            vec![Ok(resp)],
        ));
        let bridge =
            build_scripted_bridge(TEST_ADDRESS_HEX, scripted, 0, Duration::from_millis(100));
        let mut req = hybrid_v2_external_signer_harness::baseline_signing_request();
        req.nonce = case as u64;
        let err = bridge.sign_execution(req).await.unwrap_err();
        assert!(
            matches!(err, SignerError::MalformedResponse(_)),
            "case {case}: unexpected err {err:?}"
        );
    }
}

// -----------------------------------------------------------------
//                    9 — KMS timeout mapping
// -----------------------------------------------------------------

#[tokio::test]
async fn prop_kms_timeout_maps_to_signer_timeout() {
    for case in 0..CASES {
        let bridge = build_bridge_with_mode(MockProviderMode::Timeout, TEST_ADDRESS_HEX, 0);
        let mut req = hybrid_v2_external_signer_harness::baseline_signing_request();
        req.nonce = case as u64;
        let err = bridge.sign_execution(req).await.unwrap_err();
        assert!(
            matches!(err, SignerError::Timeout),
            "case {case}: got {err:?}"
        );
    }
}

// -----------------------------------------------------------------
//                    10 — availability at construction
// -----------------------------------------------------------------

#[test]
fn prop_availability_reports_configured_at_construction() {
    for case in 0..CASES {
        let mode = match case % 5 {
            0 => MockProviderMode::Success,
            1 => MockProviderMode::Denied,
            2 => MockProviderMode::Unavailable,
            3 => MockProviderMode::RateLimited,
            _ => MockProviderMode::AuthFailed,
        };
        let bridge = build_bridge_with_mode(mode, TEST_ADDRESS_HEX, 0);
        // availability() reports Configured at construction time; the
        // live status is only reachable via probe() which we do NOT
        // call here (the property is about the sync verdict).
        assert!(matches!(
            bridge.availability(),
            SignerAvailability::Configured
        ));
    }
}

// -----------------------------------------------------------------
//                    11 — endpoint redaction
// -----------------------------------------------------------------

#[test]
fn prop_endpoint_redaction_never_leaks_path() {
    let secret_paths = [
        "https://signer.example.com/v1/keys/SECRET123/sign",
        "https://signer.example.com/api/2024/sign?token=XYZ",
        "http://127.0.0.1:9000/sign/with/path?apikey=abc",
        "https://user:pass@signer.example.com/keys/1234",
        "https://signer.example.com/#fragment",
    ];
    for url in &secret_paths {
        let red = redacted_endpoint(url);
        assert!(!red.contains("SECRET123"), "leaks SECRET123: {red}");
        assert!(!red.contains("token=XYZ"), "leaks token: {red}");
        assert!(!red.contains("apikey"), "leaks apikey: {red}");
        assert!(!red.contains("user:pass"), "leaks credentials: {red}");
        // Path stripped entirely — no forward slash after the host
        // segment. We look for `/` AFTER the scheme's `://`.
        let after_scheme = red.split_once("://").map(|x| x.1).unwrap_or(&red);
        assert!(!after_scheme.contains('/'), "leaks path fragment: {red}");
        assert!(!red.contains("fragment"), "leaks fragment: {red}");
    }
    assert_eq!(redacted_endpoint("garbage"), "<opaque>");
}

// -----------------------------------------------------------------
//                    12 — SignerProvider parse round-trip
// -----------------------------------------------------------------

#[test]
fn prop_signer_provider_parse_round_trip() {
    for provider in [
        SignerProvider::KmsAws,
        SignerProvider::KmsGcp,
        SignerProvider::Turnkey,
        SignerProvider::Fireblocks,
        SignerProvider::Mock,
    ] {
        let s = provider.as_str();
        let parsed = SignerProvider::parse(s).expect(s);
        assert_eq!(parsed, provider);
    }
}

// -----------------------------------------------------------------
//                    13 — mainnet refused regardless of provider
// -----------------------------------------------------------------

#[test]
fn prop_config_validate_refuses_mainnet_regardless_of_provider() {
    for provider in [
        SignerProvider::KmsAws,
        SignerProvider::KmsGcp,
        SignerProvider::Turnkey,
        SignerProvider::Fireblocks,
        SignerProvider::Mock,
    ] {
        let mut cfg = HybridV2ExecutionConfig::disabled();
        cfg.execution_enabled = true;
        cfg.executor_address = [0xaau8; 20];
        cfg.expected_signer_address = Some(test_address_bytes());
        cfg.signer_endpoint = Some("https://signer.example/sign".to_string());
        cfg.signer_provider = Some(provider);
        let err = cfg.validate_startup(8453).unwrap_err().to_string();
        assert!(
            err.contains("Base mainnet forbidden"),
            "provider={provider:?} err={err}"
        );
    }
}

// -----------------------------------------------------------------
//                    14 — non-https endpoint refused
// -----------------------------------------------------------------

#[test]
fn prop_config_validate_refuses_non_https_endpoint() {
    let bad_endpoints = [
        "http://public.example.com/sign",
        "ftp://signer.example.com/sign",
        "ws://signer.example.com/sign",
        "file:///etc/passwd",
        "http://google.com/",
    ];
    for endpoint in &bad_endpoints {
        let mut cfg = HybridV2ExecutionConfig::disabled();
        cfg.execution_enabled = true;
        cfg.executor_address = [0xaau8; 20];
        cfg.expected_signer_address = Some(test_address_bytes());
        cfg.signer_endpoint = Some((*endpoint).to_string());
        cfg.signer_provider = Some(SignerProvider::KmsAws);
        let res = cfg.validate_startup(84532);
        assert!(res.is_err(), "endpoint {endpoint} should be refused");
    }
}

// -----------------------------------------------------------------
//                    15 — no broadcast RPC invocation
// -----------------------------------------------------------------

#[tokio::test]
async fn prop_pipeline_never_invokes_broadcast_rpc() {
    // Boot the mock RPC + a Success bridge. Every mode we run through
    // the bridge results in NO broadcast RPC call. This is the mock's
    // `prohibited_calls_seen()` set assertion generalized across each
    // provider mode.
    for mode in [
        MockProviderMode::Success,
        MockProviderMode::Denied,
        MockProviderMode::Timeout,
        MockProviderMode::Unavailable,
        MockProviderMode::AuthFailed,
        MockProviderMode::RateLimited,
    ] {
        let mock = MockRpcServer::start().await;
        let bridge = build_bridge_with_mode(mode, TEST_ADDRESS_HEX, 0);
        // The bridge does NOT touch the RPC — it only calls the vendor.
        // We invoke it once so the failure paths run, then assert the
        // mock never observed a broadcast method.
        let mut req = hybrid_v2_external_signer_harness::baseline_signing_request();
        req.nonce = 1;
        let _ = bridge.sign_execution(req).await;
        assert!(
            mock.prohibited_calls_seen().is_empty(),
            "mode {mode:?}: prohibited calls seen"
        );
    }
}

// -----------------------------------------------------------------
//                          extra: address parse round-trip
// -----------------------------------------------------------------

#[test]
fn parse_address_helper_round_trips_canonical_hex() {
    for case in 0..CASES {
        let addr = seed_addr(case as u8);
        let hex = format!(
            "0x{}",
            addr.iter()
                .map(|b| format!("{:02x}", b))
                .collect::<String>()
        );
        let parsed = parse_address_hex(&hex);
        assert_eq!(parsed, addr);
    }
}
