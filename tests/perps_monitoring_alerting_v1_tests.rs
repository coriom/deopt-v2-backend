//! PERPS-MONITORING-ALERTING-V1 — Perps observability + metrics tests.
//!
//! Verifies:
//!
//! * Part 1 — `PerpsObservability` counter behaviour + snapshot shape.
//! * Part 2 — `/metrics` endpoint includes the new Perps counter +
//!   gauge families.
//! * Part 3 — bounded reason-label cardinality (no wallet, no order id,
//!   no signature, no nonce, no raw error message).
//! * Part 4 — increments fire at the expected lifecycle points
//!   (kill-switch skip, tick failure, public fail-closed reject).
//! * Part 5 — no secrets in `/metrics` body.
//! * Part 6 — last-tick age gauge derivation.
//! * Part 7 — regression: default public Perps still fail-closed.
//!
//! No PG. No RPC. No mainnet. No secrets. No transactions.

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use deopt_v2_backend::api::{router, AppState};
use deopt_v2_backend::engine::EngineState;
use deopt_v2_backend::error::BackendError;
use deopt_v2_backend::perps::{
    cancel_reason_labels, run_perps_funding_tick_once, run_perps_liquidation_tick_once,
    submit_reason_labels, PerpsFundingWorkerConfig, PerpsLiquidationWorkerConfig,
    PerpsObservability, PerpsReadConfig, PerpsWorkerStaleOraclePolicy,
};
use tower::ServiceExt;

fn base_state() -> AppState {
    let mut state = AppState::new(EngineState::with_default_markets());
    let mut cfg = PerpsReadConfig::enabled_in_memory_for_tests();
    cfg.rpc_url = None;
    state.perps_read_config = cfg;
    state
}

async fn get_metrics(state: AppState) -> (StatusCode, String) {
    let router = router(state);
    let req = Request::builder()
        .method("GET")
        .uri("/metrics")
        .body(Body::empty())
        .unwrap();
    let resp = router.oneshot(req).await.unwrap();
    let status = resp.status();
    let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (status, String::from_utf8(body.to_vec()).unwrap())
}

async fn post_json(state: AppState, uri: &str, body: &str) -> StatusCode {
    let router = router(state);
    let req = Request::builder()
        .method("POST")
        .uri(uri)
        .header("Content-Type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let resp = router.oneshot(req).await.unwrap();
    resp.status()
}

// =====================================================================
// Part 1 — PerpsObservability behaviour.
// =====================================================================

#[test]
fn part1_snapshot_starts_zero() {
    let obs = PerpsObservability::new();
    let snap = obs.snapshot();
    assert_eq!(snap.funding_tick_ok_total, 0);
    assert_eq!(snap.liquidation_tick_ok_total, 0);
    assert_eq!(snap.perps_not_live_reject_total, 0);
    assert_eq!(snap.deviation_exceeded_total, 0);
    assert!(snap.submit_reject_by_reason.is_empty());
    assert!(snap.cancel_reject_by_reason.is_empty());
}

#[test]
fn part1_incrementers_are_idempotent_and_composable() {
    let obs = PerpsObservability::new();
    obs.record_funding_tick_ok(2);
    obs.record_funding_tick_ok(1);
    obs.record_funding_tick_failure();
    obs.record_liquidation_tick_ok(5, 3);
    obs.record_perps_not_live_reject();
    obs.record_perps_not_live_reject();
    obs.record_closed_test_access_denied();
    obs.record_deviation_exceeded();
    obs.record_bad_debt_event();
    let snap = obs.snapshot();
    assert_eq!(snap.funding_tick_ok_total, 2);
    assert_eq!(snap.funding_market_stale_skip_total, 3);
    assert_eq!(snap.funding_tick_failure_total, 1);
    assert_eq!(snap.liquidation_tick_ok_total, 1);
    assert_eq!(snap.liquidation_market_stale_skip_total, 5);
    assert_eq!(snap.liquidation_event_total, 3);
    assert_eq!(snap.perps_not_live_reject_total, 2);
    assert_eq!(snap.closed_test_access_denied_total, 1);
    assert_eq!(snap.deviation_exceeded_total, 1);
    assert_eq!(snap.bad_debt_event_total, 1);
}

// =====================================================================
// Part 2 — /metrics inventory.
// =====================================================================

#[tokio::test]
async fn part2_metrics_endpoint_lists_new_perps_gauges() {
    let state = base_state();
    let (status, body) = get_metrics(state).await;
    assert_eq!(status, StatusCode::OK);
    for name in [
        "deopt_perps_funding_tick_ok_total",
        "deopt_perps_funding_tick_failure_total",
        "deopt_perps_funding_tick_kill_switch_skip_total",
        "deopt_perps_funding_market_stale_skip_total",
        "deopt_perps_liquidation_tick_ok_total",
        "deopt_perps_liquidation_tick_failure_total",
        "deopt_perps_liquidation_tick_kill_switch_skip_total",
        "deopt_perps_liquidation_market_stale_skip_total",
        "deopt_perps_not_live_reject_total",
        "deopt_perps_closed_test_access_denied_total",
        "deopt_perps_v2_auth_failure_total",
        "deopt_perps_deviation_exceeded_total",
        "deopt_perps_liquidation_event_total",
        "deopt_perps_bad_debt_event_total",
        "deopt_perps_submit_reject_total",
        "deopt_perps_cancel_reject_total",
    ] {
        assert!(body.contains(name), "missing metric: {name}");
    }
}

#[tokio::test]
async fn part2_metrics_pre_seeds_bounded_reason_labels() {
    let state = base_state();
    let (_, body) = get_metrics(state).await;
    // Every submit and cancel reason label should appear pre-seeded at 0.
    for reason in submit_reason_labels() {
        let hit = format!("deopt_perps_submit_reject_total{{reason=\"{reason}\"}}");
        assert!(body.contains(&hit), "submit pre-seed missing: {reason}");
    }
    for reason in cancel_reason_labels() {
        let hit = format!("deopt_perps_cancel_reject_total{{reason=\"{reason}\"}}");
        assert!(body.contains(&hit), "cancel pre-seed missing: {reason}");
    }
}

// =====================================================================
// Part 3 — bounded label cardinality.
// =====================================================================

#[test]
fn part3_submit_reason_labels_are_bounded_and_sanitized() {
    let labels = submit_reason_labels();
    // Alphabet + underscore only.
    for l in labels {
        assert!(!l.is_empty());
        assert!(l
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_'));
        assert!(l.len() <= 64);
    }
    // Duplicates would defeat the bounded-cardinality guarantee.
    let mut seen = std::collections::BTreeSet::new();
    for l in labels {
        assert!(seen.insert(*l), "duplicate label: {l}");
    }
    // "other" MUST be part of the whitelist so unclassified errors
    // don't leak the raw message.
    assert!(labels.contains(&"other"));
}

#[test]
fn part3_cancel_reason_labels_are_bounded_and_sanitized() {
    let labels = cancel_reason_labels();
    for l in labels {
        assert!(!l.is_empty());
        assert!(l
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_'));
        assert!(l.len() <= 64);
    }
    let mut seen = std::collections::BTreeSet::new();
    for l in labels {
        assert!(seen.insert(*l), "duplicate label: {l}");
    }
    assert!(labels.contains(&"other"));
}

#[test]
fn part3_uncategorized_error_lands_in_other_bucket() {
    let obs = PerpsObservability::new();
    // A `Config` error is intentionally not on the classified list.
    obs.record_submit_reject(&BackendError::Config("secret-looking-message".to_string()));
    let snap = obs.snapshot();
    // The message MUST NOT leak; only the "other" bucket increments.
    assert_eq!(snap.submit_reject_by_reason.get("other"), Some(&1));
    for (k, _) in &snap.submit_reject_by_reason {
        assert!(!k.contains("secret-looking-message"));
    }
}

// =====================================================================
// Part 4 — increment behaviour at lifecycle points.
// =====================================================================

#[tokio::test]
async fn part4_funding_kill_switch_skip_increments_counter() {
    let state = base_state(); // both worker flags off by default
    run_perps_funding_tick_once(&state).await;
    let snap = state.perps_observability.snapshot();
    assert_eq!(snap.funding_tick_kill_switch_skip_total, 1);
    assert_eq!(snap.funding_tick_ok_total, 0);
    assert_eq!(snap.funding_tick_failure_total, 0);
}

#[tokio::test]
async fn part4_liquidation_kill_switch_skip_increments_counter() {
    let state = base_state();
    run_perps_liquidation_tick_once(&state).await;
    let snap = state.perps_observability.snapshot();
    assert_eq!(snap.liquidation_tick_kill_switch_skip_total, 1);
}

#[tokio::test]
async fn part4_funding_tick_ok_increments_ok_counter() {
    let mut state = base_state();
    state.perps_funding_worker_config = PerpsFundingWorkerConfig {
        worker_enabled: false,
        tick_enabled: true,
        interval_sec: 3600,
        max_markets_per_tick: 32,
        stale_oracle_policy: PerpsWorkerStaleOraclePolicy::Skip,
    };
    run_perps_funding_tick_once(&state).await;
    let snap = state.perps_observability.snapshot();
    assert_eq!(snap.funding_tick_ok_total, 1);
    assert_eq!(snap.funding_tick_kill_switch_skip_total, 0);
}

#[tokio::test]
async fn part4_liquidation_tick_failure_increments_failure_counter() {
    let mut state = base_state();
    // With tick_enabled=true but no configured RPC URL, the
    // liquidation worker attempts to build a mark-price reader and
    // fails → errored heartbeat + failure counter increments.
    state.perps_liquidation_worker_config = PerpsLiquidationWorkerConfig {
        worker_enabled: false,
        tick_enabled: true,
        interval_sec: 30,
        max_positions_per_tick: 500,
        stale_oracle_policy: PerpsWorkerStaleOraclePolicy::Skip,
    };
    run_perps_liquidation_tick_once(&state).await;
    let snap = state.perps_observability.snapshot();
    assert_eq!(snap.liquidation_tick_failure_total, 1);
}

#[tokio::test]
async fn part4_public_perps_submit_rejects_increment_counter() {
    let state = base_state(); // fail-closed defaults
    let status = post_json(
        state.clone(),
        "/perps/orders",
        "{\"market_id\":\"ETH-PERP\",\"account\":\"0x1\",\"side\":\"long\",\
         \"price_1e8\":\"1\",\"size_1e8\":\"1\",\"time_in_force\":\"ioc\",\
         \"isolated_margin_1e8\":\"1\"}",
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    let snap = state.perps_observability.snapshot();
    assert_eq!(snap.perps_not_live_reject_total, 1);
    assert_eq!(snap.closed_test_access_denied_total, 0);
}

#[tokio::test]
async fn part4_metrics_endpoint_reflects_incremented_counters() {
    let state = base_state();
    // Trigger one 503.
    let _ = post_json(
        state.clone(),
        "/perps/orders",
        "{\"market_id\":\"ETH-PERP\",\"account\":\"0x1\",\"side\":\"long\",\
         \"price_1e8\":\"1\",\"size_1e8\":\"1\",\"time_in_force\":\"ioc\",\
         \"isolated_margin_1e8\":\"1\"}",
    )
    .await;
    let (_, body) = get_metrics(state).await;
    // Gauge value == 1 for perps_not_live_reject_total.
    assert!(
        body.contains("deopt_perps_not_live_reject_total 1"),
        "counter value not reflected in metrics: {body}"
    );
}

// =====================================================================
// Part 5 — no secrets in /metrics body.
// =====================================================================

#[tokio::test]
async fn part5_metrics_labels_have_no_wallet_or_secret_values() {
    // Prometheus HELP text is allowed to mention words like "signature"
    // and "nonce" when describing what a counter tracks. What must NOT
    // appear anywhere in the body is a *value*: a wallet address, an
    // RPC URL, a DB URL, an admin token, an allowlist entry. Scan only
    // the non-HELP lines to make the assertion targeted.
    let state = base_state();
    let _ = post_json(
        state.clone(),
        "/perps/orders",
        "{\"market_id\":\"ETH-PERP\",\"account\":\"0xa11ce\",\"side\":\"long\",\
         \"price_1e8\":\"1\",\"size_1e8\":\"1\",\"time_in_force\":\"ioc\",\
         \"isolated_margin_1e8\":\"1\"}",
    )
    .await;
    let (_, body) = get_metrics(state).await;
    // Walk value lines only (skip `# HELP` and `# TYPE`).
    for line in body.lines() {
        if line.starts_with("# HELP") || line.starts_with("# TYPE") {
            continue;
        }
        for banned in [
            "0xa11ce",
            "0xdeadbeef",
            "private_key=",
            "rpc_url=",
            "database_url=",
            "admin_token=",
            "allowlist=",
        ] {
            assert!(
                !line.contains(banned),
                "metrics value line leaked '{banned}': {line}"
            );
        }
    }
}

// =====================================================================
// Part 6 — last-tick age derivation.
// =====================================================================

#[tokio::test]
async fn part6_last_funding_tick_age_gauge_emitted_after_tick() {
    let mut state = base_state();
    state.perps_funding_worker_config = PerpsFundingWorkerConfig {
        worker_enabled: false,
        tick_enabled: true,
        interval_sec: 3600,
        max_markets_per_tick: 32,
        stale_oracle_policy: PerpsWorkerStaleOraclePolicy::Skip,
    };
    run_perps_funding_tick_once(&state).await;
    let (_, body) = get_metrics(state).await;
    assert!(body.contains("deopt_perps_last_funding_tick_age_seconds"));
    assert!(body.contains("deopt_perps_last_funding_tick_ok"));
    assert!(body.contains("deopt_perps_last_funding_tick_executed 1"));
}

#[tokio::test]
async fn part6_last_liquidation_tick_ok_gauge_reports_zero_on_failure() {
    let mut state = base_state();
    state.perps_liquidation_worker_config = PerpsLiquidationWorkerConfig {
        worker_enabled: false,
        tick_enabled: true,
        interval_sec: 30,
        max_positions_per_tick: 500,
        stale_oracle_policy: PerpsWorkerStaleOraclePolicy::Skip,
    };
    run_perps_liquidation_tick_once(&state).await;
    let (_, body) = get_metrics(state).await;
    // ok should be 0 (RPC missing → error).
    assert!(body.contains("deopt_perps_last_liquidation_tick_ok 0"));
    // Failure counter also bumped.
    assert!(body.contains("deopt_perps_liquidation_tick_failure_total 1"));
}

// =====================================================================
// Part 7 — regression: default public Perps still fail-closed.
// =====================================================================

#[tokio::test]
async fn part7_default_perps_submit_returns_503() {
    let state = base_state();
    let status = post_json(
        state,
        "/perps/orders",
        "{\"market_id\":\"ETH-PERP\",\"account\":\"0x1\",\"side\":\"long\",\
         \"price_1e8\":\"1\",\"size_1e8\":\"1\",\"time_in_force\":\"ioc\",\
         \"isolated_margin_1e8\":\"1\"}",
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn part7_metrics_endpoint_ok_by_default() {
    let state = base_state();
    let (status, _body) = get_metrics(state).await;
    assert_eq!(status, StatusCode::OK);
}
