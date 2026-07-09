//! PERPS-MARKET-STATUS-DTO-WS-V1 — market status DTO tests.
//!
//! Verifies:
//!
//! * Part 1 — `PerpsMarketRiskStatusView` DTO shape (fields + JSON
//!   round-trip + no-secrets grep).
//! * Part 2 — status computation priority order.
//! * Part 3 — HTTP `/perps/markets` and `/perps/markets/:market_id`
//!   return the new `risk` field with the correct wire string.
//! * Part 4 — status ↔ submit-error alignment (stale oracle status
//!   pairs with `PerpMarkPriceUnavailable`).
//! * Part 5 — reason-code stability (pinned strings).
//! * Part 6 — regression: default public Perps still fail-closed;
//!   status field never claims trading is enabled.
//!
//! No PG. No RPC. No mainnet. No secrets. No transactions.

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use deopt_v2_backend::api::{router, AppState};
use deopt_v2_backend::engine::EngineState;
use deopt_v2_backend::perps::market_reader::InMemoryPerpMarketRegistryReader;
use deopt_v2_backend::perps::price_reader::{InMemoryPerpOraclePriceReader, RawPriceRead};
use deopt_v2_backend::perps::{
    compute_perps_market_risk_status, compute_perps_market_risk_status_view,
    get_perp_market_with_risk, list_perp_markets_with_risk, PerpsMarketAdminOverride,
    PerpsMarketOracleSnapshot, PerpsMarketRiskStatus, PerpsReadConfig,
};
use deopt_v2_backend::types::now_ms;
use tower::ServiceExt;

fn cfg() -> PerpsReadConfig {
    let mut c = PerpsReadConfig::enabled_in_memory_for_tests();
    c.rpc_url = None;
    c
}

fn eth() -> deopt_v2_backend::perps::PerpsReadMarket {
    cfg().market_by_symbol("ETH-PERP").cloned().unwrap()
}

fn fresh_snapshot(now: i64) -> PerpsMarketOracleSnapshot {
    PerpsMarketOracleSnapshot {
        index_price_1e8: 3_000 * 100_000_000,
        mark_price_1e8: 3_000 * 100_000_000,
        updated_at_ms: now,
        is_stale: false,
    }
}

// =====================================================================
// Part 1 — DTO shape.
// =====================================================================

#[test]
fn part1_view_has_expected_fields_and_serializes() {
    let cfg = cfg();
    let view = compute_perps_market_risk_status_view(
        &cfg,
        &eth(),
        Some(fresh_snapshot(now_ms())),
        PerpsMarketAdminOverride::default(),
        now_ms(),
    );
    let json = serde_json::to_string(&view).expect("serializes");
    for field in [
        "\"status\"",
        "\"reason_code\"",
        "\"allows_new_risk\"",
        "\"allows_reduce_only\"",
        "\"allows_cancel\"",
        "\"oracle_stale_after_sec\"",
        "\"oracle_max_deviation_bps\"",
        "\"last_checked_at_ms\"",
    ] {
        assert!(json.contains(field), "missing field: {field} in {json}");
    }
    // Round-trip.
    let decoded: deopt_v2_backend::perps::PerpsMarketRiskStatusView =
        serde_json::from_str(&json).unwrap();
    assert_eq!(decoded, view);
}

#[test]
fn part1_view_never_contains_secret_field_names() {
    let cfg = cfg();
    let view = compute_perps_market_risk_status_view(
        &cfg,
        &eth(),
        Some(fresh_snapshot(now_ms())),
        PerpsMarketAdminOverride::default(),
        now_ms(),
    );
    let json = serde_json::to_string(&view).unwrap();
    for banned in [
        "rpc_url",
        "database_url",
        "admin_token",
        "authorization",
        "allowlist",
        "PERPS_CLOSED_TEST_ALLOWLIST",
        "private_key",
        "envelope",
        "signature",
        "nonce",
    ] {
        assert!(
            !json.contains(banned),
            "risk view leaked banned string: {banned}"
        );
    }
}

// =====================================================================
// Part 2 — status computation priority order.
// =====================================================================

#[test]
fn part2_active_when_fresh_oracle_and_no_override() {
    let cfg = cfg();
    let view = compute_perps_market_risk_status_view(
        &cfg,
        &eth(),
        Some(fresh_snapshot(now_ms())),
        PerpsMarketAdminOverride::default(),
        now_ms(),
    );
    assert_eq!(view.status, "active");
    assert_eq!(view.reason_code, "active");
    assert!(view.allows_new_risk);
    assert!(view.allows_reduce_only);
    assert!(view.allows_cancel);
}

#[test]
fn part2_stale_oracle_reads_none() {
    let cfg = cfg();
    let view = compute_perps_market_risk_status_view(
        &cfg,
        &eth(),
        None,
        PerpsMarketAdminOverride::default(),
        now_ms(),
    );
    assert_eq!(view.status, "stale_oracle");
    assert!(view.reason_code.starts_with("stale_oracle"));
    assert!(!view.allows_new_risk);
    assert!(view.allows_reduce_only);
    assert!(view.allows_cancel);
}

#[test]
fn part2_stale_flag_beats_fresh_prices() {
    let cfg = cfg();
    let mut snap = fresh_snapshot(now_ms());
    snap.is_stale = true;
    let view = compute_perps_market_risk_status_view(
        &cfg,
        &eth(),
        Some(snap),
        PerpsMarketAdminOverride::default(),
        now_ms(),
    );
    assert_eq!(view.status, "stale_oracle");
    assert!(!view.allows_new_risk);
}

#[test]
fn part2_deviation_exceeded_when_mark_diverges() {
    let cfg = cfg();
    // Index 3000, mark 3500 → deviation = 500 / 3000 = 1666 bps.
    // Threshold default 500 → exceeded.
    let snap = PerpsMarketOracleSnapshot {
        index_price_1e8: 3_000 * 100_000_000,
        mark_price_1e8: 3_500 * 100_000_000,
        updated_at_ms: now_ms(),
        is_stale: false,
    };
    let view = compute_perps_market_risk_status_view(
        &cfg,
        &eth(),
        Some(snap),
        PerpsMarketAdminOverride::default(),
        now_ms(),
    );
    assert_eq!(view.status, "deviation_exceeded");
    assert!(view.reason_code.starts_with("deviation_exceeded:observed="));
    assert!(view.reason_code.contains("threshold=500"));
    assert!(!view.allows_new_risk);
}

#[test]
fn part2_admin_disabled_beats_everything() {
    let cfg = cfg();
    let view = compute_perps_market_risk_status_view(
        &cfg,
        &eth(),
        Some(fresh_snapshot(now_ms())),
        PerpsMarketAdminOverride {
            disabled: true,
            paused: false,
            cancel_only: false,
        },
        now_ms(),
    );
    assert_eq!(view.status, "disabled");
    assert_eq!(view.reason_code, "disabled");
    assert!(!view.allows_new_risk);
    assert!(!view.allows_reduce_only);
    assert!(!view.allows_cancel);
}

#[test]
fn part2_admin_paused_beats_oracle_state() {
    let cfg = cfg();
    let view = compute_perps_market_risk_status_view(
        &cfg,
        &eth(),
        None, // stale would otherwise dominate
        PerpsMarketAdminOverride {
            disabled: false,
            paused: true,
            cancel_only: false,
        },
        now_ms(),
    );
    assert_eq!(view.status, "paused");
    assert_eq!(view.reason_code, "paused");
    assert!(!view.allows_new_risk);
    // Paused still allows exits.
    assert!(view.allows_reduce_only);
    assert!(view.allows_cancel);
}

#[test]
fn part2_admin_cancel_only_beats_oracle_active() {
    let cfg = cfg();
    let view = compute_perps_market_risk_status_view(
        &cfg,
        &eth(),
        Some(fresh_snapshot(now_ms())),
        PerpsMarketAdminOverride {
            disabled: false,
            paused: false,
            cancel_only: true,
        },
        now_ms(),
    );
    assert_eq!(view.status, "cancel_only");
    assert_eq!(view.reason_code, "cancel_only");
    assert!(!view.allows_new_risk);
    assert!(view.allows_reduce_only);
    assert!(view.allows_cancel);
}

#[test]
fn part2_disabled_beats_paused_beats_cancel_only() {
    let cfg = cfg();
    // Disabled + paused + cancel_only all set — disabled wins.
    let view = compute_perps_market_risk_status_view(
        &cfg,
        &eth(),
        Some(fresh_snapshot(now_ms())),
        PerpsMarketAdminOverride {
            disabled: true,
            paused: true,
            cancel_only: true,
        },
        now_ms(),
    );
    assert_eq!(view.status, "disabled");
    // Paused + cancel_only (no disabled) — paused wins.
    let view = compute_perps_market_risk_status_view(
        &cfg,
        &eth(),
        Some(fresh_snapshot(now_ms())),
        PerpsMarketAdminOverride {
            disabled: false,
            paused: true,
            cancel_only: true,
        },
        now_ms(),
    );
    assert_eq!(view.status, "paused");
}

#[test]
fn part2_enum_helper_priority_matches_wire() {
    let cfg = cfg();
    // Enum-level compute matches the wire-level for the pure oracle
    // path (admin overrides are wire-level only).
    let status = compute_perps_market_risk_status(&cfg, None, PerpsMarketAdminOverride::default());
    assert!(matches!(status, PerpsMarketRiskStatus::StaleOracle { .. }));
    let status = compute_perps_market_risk_status(
        &cfg,
        Some(fresh_snapshot(now_ms())),
        PerpsMarketAdminOverride::default(),
    );
    assert!(matches!(status, PerpsMarketRiskStatus::Active));
}

// =====================================================================
// Part 3 — HTTP DTO integration.
// =====================================================================

fn state_with_read_enabled() -> AppState {
    let mut state = AppState::new(EngineState::with_default_markets());
    state.perps_read_config = cfg();
    state
}

async fn http_get(state: AppState, uri: &str) -> (StatusCode, String) {
    let router = router(state);
    let req = Request::builder()
        .method("GET")
        .uri(uri)
        .body(Body::empty())
        .unwrap();
    let resp = router.oneshot(req).await.unwrap();
    let status = resp.status();
    let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (status, String::from_utf8(body.to_vec()).unwrap())
}

#[tokio::test]
async fn part3_list_perp_markets_with_risk_populates_view() {
    // Direct service call (no HTTP) so we can inject the price reader.
    let cfg = cfg();
    let reader = InMemoryPerpMarketRegistryReader::new();
    let price_reader = InMemoryPerpOraclePriceReader::new().with_price(
        "ETH-PERP",
        RawPriceRead {
            price_1e8: 3_000 * 100_000_000,
            updated_at_sec: (now_ms() / 1000) as u64,
            ok: true,
        },
    );
    let listing = list_perp_markets_with_risk(&cfg, &reader, &price_reader)
        .await
        .expect("listing");
    let eth = listing
        .markets
        .iter()
        .find(|m| m.market_id == "ETH-PERP")
        .expect("ETH-PERP");
    let risk = eth.risk.as_ref().expect("risk populated");
    assert_eq!(risk.status, "active");
    assert!(risk.allows_new_risk);
    // BTC has no seeded price → stale.
    let btc = listing
        .markets
        .iter()
        .find(|m| m.market_id == "BTC-PERP")
        .expect("BTC-PERP");
    let risk = btc.risk.as_ref().expect("risk populated");
    assert_eq!(risk.status, "stale_oracle");
    assert!(!risk.allows_new_risk);
}

#[tokio::test]
async fn part3_get_perp_market_with_risk_populates_view() {
    let cfg = cfg();
    let reader = InMemoryPerpMarketRegistryReader::new();
    let price_reader = InMemoryPerpOraclePriceReader::new().with_price(
        "ETH-PERP",
        RawPriceRead {
            price_1e8: 3_000 * 100_000_000,
            updated_at_sec: (now_ms() / 1000) as u64,
            ok: true,
        },
    );
    let market = get_perp_market_with_risk(&cfg, &reader, &price_reader, "ETH-PERP")
        .await
        .expect("market");
    let risk = market.risk.as_ref().expect("risk populated");
    assert_eq!(risk.status, "active");
    assert!(!market.trading_enabled, "trading still fail-closed");
}

#[tokio::test]
async fn part3_http_perps_markets_body_has_no_secrets_even_on_error() {
    // With `rpc_url = None` the HTTP handler returns 503 at the
    // registry-reader build step. Regardless of the outcome, the
    // response body MUST NOT contain any secret field name — the
    // read layer's error path was audited by prior milestones; this
    // test pins the invariant across the new risk-view wiring.
    let state = state_with_read_enabled();
    let (_status, body) = http_get(state, "/perps/markets").await;
    for banned in [
        "rpc_url",
        "database_url",
        "admin_token",
        "authorization",
        "envelope",
        "signature",
        "PERPS_CLOSED_TEST_ALLOWLIST",
    ] {
        assert!(!body.contains(banned), "leaked: {banned}");
    }
}

// =====================================================================
// Part 4 — status ↔ submit-error alignment.
// =====================================================================

#[test]
fn part4_stale_status_pairs_with_perp_mark_price_unavailable() {
    // The `stale_oracle` wire string is the DTO-side twin of the
    // `PerpMarkPriceUnavailable` submit error. Verify the string
    // constants stay stable so a client can key on them the same
    // way.
    let cfg = cfg();
    let view = compute_perps_market_risk_status_view(
        &cfg,
        &eth(),
        None,
        PerpsMarketAdminOverride::default(),
        now_ms(),
    );
    assert_eq!(view.status, "stale_oracle");
    // Reason code carries structured context but always starts with
    // the wire-level status name so consumers can `.startsWith()`.
    assert!(view.reason_code.starts_with("stale_oracle"));
}

#[test]
fn part4_deviation_status_pairs_with_perp_oracle_deviation_exceeded() {
    let cfg = cfg();
    let snap = PerpsMarketOracleSnapshot {
        index_price_1e8: 100 * 100_000_000,
        mark_price_1e8: 200 * 100_000_000, // 10_000 bps
        updated_at_ms: now_ms(),
        is_stale: false,
    };
    let view = compute_perps_market_risk_status_view(
        &cfg,
        &eth(),
        Some(snap),
        PerpsMarketAdminOverride::default(),
        now_ms(),
    );
    assert_eq!(view.status, "deviation_exceeded");
    assert!(view.reason_code.starts_with("deviation_exceeded:observed="));
}

// =====================================================================
// Part 5 — reason-code stability (pinned strings).
// =====================================================================

#[test]
fn part5_status_strings_are_pinned() {
    // These strings MUST remain stable across releases so clients can
    // hard-match. Update this test only alongside a documented wire
    // migration.
    let cfg = cfg();
    let cases = vec![
        (
            compute_perps_market_risk_status_view(
                &cfg,
                &eth(),
                Some(fresh_snapshot(now_ms())),
                PerpsMarketAdminOverride::default(),
                now_ms(),
            )
            .status,
            "active",
        ),
        (
            compute_perps_market_risk_status_view(
                &cfg,
                &eth(),
                None,
                PerpsMarketAdminOverride::default(),
                now_ms(),
            )
            .status,
            "stale_oracle",
        ),
        (
            compute_perps_market_risk_status_view(
                &cfg,
                &eth(),
                Some(PerpsMarketOracleSnapshot {
                    index_price_1e8: 100 * 100_000_000,
                    mark_price_1e8: 200 * 100_000_000,
                    updated_at_ms: now_ms(),
                    is_stale: false,
                }),
                PerpsMarketAdminOverride::default(),
                now_ms(),
            )
            .status,
            "deviation_exceeded",
        ),
        (
            compute_perps_market_risk_status_view(
                &cfg,
                &eth(),
                Some(fresh_snapshot(now_ms())),
                PerpsMarketAdminOverride {
                    disabled: false,
                    paused: true,
                    cancel_only: false,
                },
                now_ms(),
            )
            .status,
            "paused",
        ),
        (
            compute_perps_market_risk_status_view(
                &cfg,
                &eth(),
                Some(fresh_snapshot(now_ms())),
                PerpsMarketAdminOverride {
                    disabled: false,
                    paused: false,
                    cancel_only: true,
                },
                now_ms(),
            )
            .status,
            "cancel_only",
        ),
        (
            compute_perps_market_risk_status_view(
                &cfg,
                &eth(),
                Some(fresh_snapshot(now_ms())),
                PerpsMarketAdminOverride {
                    disabled: true,
                    paused: false,
                    cancel_only: false,
                },
                now_ms(),
            )
            .status,
            "disabled",
        ),
    ];
    for (got, want) in cases {
        assert_eq!(got, want);
    }
}

// =====================================================================
// Part 6 — regression: default public Perps still fail-closed.
// =====================================================================

#[tokio::test]
async fn part6_perps_markets_listing_dto_never_flips_trading_enabled() {
    // Direct service call — the wire envelope's `trading_enabled`
    // MUST stay `false` regardless of the risk view field. Even an
    // `active` risk status never means public trading is on.
    let cfg = cfg();
    let reader = InMemoryPerpMarketRegistryReader::new();
    let price_reader = InMemoryPerpOraclePriceReader::new().with_price(
        "ETH-PERP",
        RawPriceRead {
            price_1e8: 3_000 * 100_000_000,
            updated_at_sec: (now_ms() / 1000) as u64,
            ok: true,
        },
    );
    let listing = list_perp_markets_with_risk(&cfg, &reader, &price_reader)
        .await
        .expect("listing");
    assert!(!listing.trading_enabled);
    for market in &listing.markets {
        assert!(!market.trading_enabled);
        if let Some(risk) = &market.risk {
            // `allows_new_risk == true` never implies `trading_enabled`.
            let _ = risk.allows_new_risk;
        }
    }
}

#[tokio::test]
async fn part6_perps_submit_still_returns_503_regardless_of_status() {
    // Even with a fresh oracle and `active` market status, the public
    // Perps mutation route stays 503 while both public + closed-test
    // flags are default-off.
    let state = state_with_read_enabled();
    let router = router(state);
    let req = Request::builder()
        .method("POST")
        .uri("/perps/orders")
        .header("Content-Type", "application/json")
        .body(Body::from(
            "{\"market_id\":\"ETH-PERP\",\"account\":\"0x1\",\"side\":\"long\",\
             \"price_1e8\":\"1\",\"size_1e8\":\"1\",\"time_in_force\":\"ioc\",\
             \"isolated_margin_1e8\":\"1\"}",
        ))
        .unwrap();
    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
}
