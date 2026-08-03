//! `BACKEND-HYBRID-V2-POSTGRES-READ-STORE-2B-MAIN-ROUTER-PG-PROOF-V1`
//!
//! End-to-end proof that the mounted Hybrid V2 HTTP read surface serves
//! canonical data from real PostgreSQL through the same top-level router
//! `main.rs` uses. Wire path:
//!   `AppState::new(EngineState::with_default_markets())
//!    .with_hybrid_v2_postgres(pool, entries)`
//!   -> `deopt_v2_backend::api::router(app_state)`
//!   -> merged `/hybrid-v2` sub-router -> handlers via `state.store()`
//!   (`Arc<dyn HybridV2ReadStore>`) -> PostgreSQL over sqlx.
//!
//! Gating: skips cleanly when neither `HYBRID_V2_PG_TEST_DATABASE_URL`
//! nor `PG_INTEGRATION_URL` is set. Panics loudly if
//! `DEOPT_REQUIRE_PG_INTEGRATION=1` and no URL is set.
//!
//! Isolation: each test drops + recreates schema `public` + runs the
//! full migration chain, so this file is `--test-threads=1` safe.
//!
//! Secrets: never prints the database URL, credentials, or role name.

use axum::body::{to_bytes, Body};
use axum::http::{Method, Request, StatusCode};
use deopt_v2_backend::api::hybrid_v2_read::DeploymentEntry;
use deopt_v2_backend::api::{router, AppState};
use deopt_v2_backend::engine::EngineState;
use deopt_v2_backend::hybrid_v2::events::{EventKind, HybridV2Event};
use deopt_v2_backend::hybrid_v2::manifest::{
    ActivationStatus, ManifestModuleAddresses, ManifestParams,
};
use deopt_v2_backend::hybrid_v2::persistence::{
    CanonicalBlockRef, HybridV2ProjectionStore, PostgresHybridV2ProjectionStore, ReadinessSnapshot,
    RuntimeCursorSnapshot,
};
use deopt_v2_backend::hybrid_v2::reducer::{
    apply, ApplyContext, ExecutionCompletion, MatchedExecutionRow, ProjectionState,
};
use deopt_v2_backend::hybrid_v2::runtime::JournaledLog;
use serde_json::{json, Value};
use sqlx::postgres::{PgPool, PgPoolOptions};
use std::sync::Arc;
use tower::ServiceExt;

// -----------------------------------------------------------------
//                       PG HARNESS
// -----------------------------------------------------------------

mod pg_harness {
    use super::*;

    pub const URL_ENV: &str = "HYBRID_V2_PG_TEST_DATABASE_URL";
    pub const ALT_URL_ENV: &str = "PG_INTEGRATION_URL";
    pub const REQUIRE_ENV: &str = "DEOPT_REQUIRE_PG_INTEGRATION";

    /// Resolve a disposable PG URL from env, panic if
    /// `DEOPT_REQUIRE_PG_INTEGRATION=1` was requested but no URL is set,
    /// otherwise return `None` so the caller skips.
    pub fn get_pg_url_or_skip_or_panic() -> Option<String> {
        let url = std::env::var(URL_ENV)
            .ok()
            .or_else(|| std::env::var(ALT_URL_ENV).ok())
            .filter(|v| !v.is_empty());
        if url.is_none() {
            let required = matches!(
                std::env::var(REQUIRE_ENV).ok().as_deref(),
                Some("1") | Some("true") | Some("TRUE")
            );
            if required {
                panic!(
                    "{} is enabled but neither {} nor {} is set — this \
                     Hybrid V2 main-router PG proof cannot execute in \
                     required-mode without a disposable PostgreSQL URL",
                    REQUIRE_ENV, URL_ENV, ALT_URL_ENV
                );
            }
        }
        url
    }

    /// Every test drops + recreates schema public + applies the entire
    /// migration chain against a disposable database. Never prints the
    /// URL / credentials / role name.
    pub async fn fresh_pool(url: &str) -> PgPool {
        let pool = PgPoolOptions::new()
            .max_connections(4)
            .acquire_timeout(std::time::Duration::from_secs(30))
            .idle_timeout(Some(std::time::Duration::from_secs(10)))
            .connect(url)
            .await
            .expect("connect to disposable PostgreSQL (env)");
        sqlx::query("DROP SCHEMA public CASCADE")
            .execute(&pool)
            .await
            .expect("drop schema public");
        sqlx::query("CREATE SCHEMA public")
            .execute(&pool)
            .await
            .expect("create schema public");
        sqlx::query("GRANT ALL ON SCHEMA public TO PUBLIC")
            .execute(&pool)
            .await
            .expect("grant schema public");
        let migrator = sqlx::migrate::Migrator::new(std::path::Path::new("./migrations"))
            .await
            .expect("load ./migrations");
        migrator
            .run(&pool)
            .await
            .expect("apply migration chain to fresh disposable database");
        pool
    }

    pub fn now_ms() -> i64 {
        1_700_000_000_000
    }

    /// Per-tag deployment_version derived from a small FNV-like hash so
    /// the `hybrid_v2_deployments_uniq` (chain_id, manifest_hash) and
    /// version index never collide across tests that share a database.
    pub fn tag_deployment_version(tag: &str) -> u16 {
        let mut h: u32 = 0x811c9dc5;
        for b in tag.bytes() {
            h ^= b as u32;
            h = h.wrapping_mul(0x01000193);
        }
        ((h & 0xffff) as u16).max(2)
    }

    pub fn base_manifest(chain_id: u64, tag: &str) -> ManifestParams {
        ManifestParams {
            chain_id,
            manifest_address: format!("0xdeadaddr{tag}"),
            manifest_hash: format!("0xdeadbeef{tag}"),
            module_addresses_hash: "0x".into(),
            critical_config_hash: "0x".into(),
            architecture_version: 1,
            storage_version: 1,
            event_version: 1,
            deployment_version: tag_deployment_version(tag),
            manifest_schema_version: 1,
            environment_tag: "TESTNET".into(),
            deployer: "0xdep".into(),
            deployment_block: 1,
            deployment_timestamp: 1_700_000_000,
            module_addresses: ManifestModuleAddresses {
                subaccount_registry: "0xreg".into(),
                collateral_vault: "0xvlt".into(),
                options_positions_ledger: "0xopl".into(),
                risk_module: "0xrsk".into(),
                margin_engine: "0xmg".into(),
                option_matching_engine: "0xmm".into(),
                escape_controller: "0xesc".into(),
                recovery_finalizer: "0xrec".into(),
                oracle_adapter: "0xoc".into(),
                options_risk_provider: "0xorp".into(),
                quote_token: "0xqt".into(),
                fees_manager_v2: None,
                option_execution_fee_adapter: None,
                protocol_timelock: None,
                governance: None,
                guardian: None,
            },
            protocol_fee_subkey: "0xpf".into(),
            rebate_budget_subkey: "0xrb".into(),
            insurance_fund_subkey: "0xif".into(),
            max_collateral_tokens: 8,
            max_active_series: 32,
            all_capabilities_mask: "0".into(),
            recovery_activation_delay_seconds: 3600,
            recovery_pause_max_duration_blocks: 100,
            activation_status: ActivationStatus::Pending,
        }
    }

    // Fixture identities shared across every test.
    pub const OWNER: &str = "0xaabbccddeeff00112233445566778899aabbccdd";
    pub const SUBKEY: &str = "0x0102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f20";
    pub const TOKEN: &str = "0xt00000000000000000000000000000000000t";
    pub const ENGINE: &str = "0xengine00000000000000000000000000000eng";
    pub const SERIES: &str = "0xseries0000000000000000000000000000000000000000000000000000000042";
    pub const ORDER_HASH: &str =
        "0xff11ff22ff33ff44ff55ff66ff77ff88ff99ffaaffbbffccffddffeeff000102";
    pub const EXEC_ID: &str = "0xexec00000000000000000000000000000000000000000000000000000000001";
    pub const EXEC_BUY: &str = "0xbuyer000000000000000000000000000000000000000000000000000000000f0";
    pub const EXEC_SELL: &str =
        "0xseller00000000000000000000000000000000000000000000000000000000f1";

    fn block_ref(n: u64) -> CanonicalBlockRef {
        CanonicalBlockRef {
            block_number: n,
            block_hash: format!("0xblk{n}"),
            parent_hash: if n == 1 {
                "0x0".into()
            } else {
                format!("0xblk{}", n - 1)
            },
            block_timestamp: 1_700_000_000 + n * 12,
        }
    }

    fn journaled(n: u64, log_index: u32) -> JournaledLog {
        JournaledLog {
            block_number: n,
            block_hash: format!("0xblk{n}"),
            parent_hash: if n == 1 {
                "0x0".into()
            } else {
                format!("0xblk{}", n - 1)
            },
            block_timestamp: 1_700_000_000 + n * 12,
            tx_hash: format!("0xtx{n}"),
            tx_index: 0,
            log_index,
            emitter: "0xreg".into(),
            topics: vec![[0u8; 32]],
            data: vec![0; 32],
            is_canonical: true,
            orphaned_at_block: None,
        }
    }

    fn ctx(n: u64, log_index: u32) -> ApplyContext {
        ApplyContext {
            block_number: n,
            tx_hash: format!("0xtx{n}"),
            log_index,
            block_timestamp: 1_700_000_000 + n * 12,
        }
    }

    fn cursor_snap(n: u64) -> RuntimeCursorSnapshot {
        RuntimeCursorSnapshot {
            cursor_name: "indexer".into(),
            indexed_head_block: n,
            indexed_head_hash: format!("0xblk{n}"),
            indexed_head_parent: if n == 1 {
                "0x0".into()
            } else {
                format!("0xblk{}", n - 1)
            },
            observed_head_block: n,
            finalized_head_block: 0,
            last_error: None,
            reorg_count: 0,
            max_reorg_depth_seen: 0,
            decode_failures: 0,
            projection_failures: 0,
            unknown_canonical_events: 0,
            last_success_block: n,
        }
    }

    fn ready() -> ReadinessSnapshot {
        ReadinessSnapshot {
            ready: true,
            reason: None,
            reason_detail: None,
        }
    }

    // -------------------- event constructors --------------------

    /// Base event skeleton — every canonical HybridV2Event field is
    /// `None`/default here; helpers below overwrite only what they
    /// need. Keeps the fixture events concise without hiding intent.
    fn ev(kind: EventKind) -> HybridV2Event {
        HybridV2Event {
            kind,
            event_version: 1,
            subkey: None,
            owner: None,
            subaccount_id: None,
            token: None,
            engine: None,
            execution_id: None,
            order_hash: None,
            series_id: None,
            payload: json!({}),
        }
    }

    fn sub_created() -> HybridV2Event {
        let mut e = ev(EventKind::SubaccountCreated);
        e.subkey = Some(SUBKEY.into());
        e.owner = Some(OWNER.into());
        e.subaccount_id = Some(1);
        e
    }

    fn deposit() -> HybridV2Event {
        let mut e = ev(EventKind::Deposit);
        e.subkey = Some(SUBKEY.into());
        e.token = Some(TOKEN.into());
        e.payload = json!({ "amount": "1000" });
        e
    }

    fn collateral_locked() -> HybridV2Event {
        let mut e = ev(EventKind::CollateralLocked);
        e.subkey = Some(SUBKEY.into());
        e.token = Some(TOKEN.into());
        e.engine = Some(ENGINE.into());
        e.payload = json!({ "amount": "200" });
        e
    }

    fn position_opened() -> HybridV2Event {
        let mut e = ev(EventKind::OptionPositionOpened);
        e.subkey = Some(SUBKEY.into());
        e.series_id = Some(SERIES.into());
        e.payload = json!({ "long_delta_1e8": "100", "short_delta_1e8": "0" });
        e
    }

    fn order_filled() -> HybridV2Event {
        let mut e = ev(EventKind::OptionOrderFilled);
        e.subkey = Some(SUBKEY.into());
        e.owner = Some(OWNER.into());
        e.order_hash = Some(ORDER_HASH.into());
        e.series_id = Some(SERIES.into());
        e.payload = json!({
            "side": 0,
            "time_in_force": 0,
            "total_qty_1e8": "1000",
            "filled_delta_1e8": "100",
            "terminal": false
        });
        e
    }

    fn fee_charged() -> HybridV2Event {
        let mut e = ev(EventKind::OptionFeeCharged);
        e.subkey = Some(SUBKEY.into());
        e.token = Some(TOKEN.into());
        e.payload = json!({ "amount": "5", "fee_subkey": "0xpf" });
        e
    }

    fn pair_executed() -> HybridV2Event {
        let mut e = ev(EventKind::OptionOrderPairExecuted);
        e.execution_id = Some(EXEC_ID.into());
        e
    }

    /// Seed the fixture required by every test in this file. Populates:
    /// - one deployment identity,
    /// - one subaccount (owner + subkey),
    /// - one vault balance (+ reservation from the lock),
    /// - one active options position,
    /// - one open order,
    /// - one completed matched execution,
    /// - one fee event,
    /// - a canonical block + cursor + projection status row marking
    ///   `ready = true`.
    pub async fn seed_fixture(pool: &PgPool, tag: &str) -> i64 {
        let projection = PostgresHybridV2ProjectionStore::new(pool.clone());
        let manifest = base_manifest(84532, tag);
        let deployment_id = projection
            .upsert_deployment(&manifest, "PENDING", now_ms())
            .await
            .expect("upsert deployment for fixture");

        // Use the DB-assigned deployment_id — every route accepts it.
        let deployment_id_i64 = deployment_id;

        let mut state = ProjectionState::default();

        // Block 1 — subaccount + deposit + lock + position + order + fee.
        let events_b1 = [
            sub_created(),
            deposit(),
            collateral_locked(),
            position_opened(),
            order_filled(),
            fee_charged(),
        ];
        let mut raws_b1 = Vec::new();
        let mut decoded_b1 = Vec::new();
        for (i, e) in events_b1.iter().enumerate() {
            let c = ctx(1, i as u32);
            apply(&mut state, e, &c).expect("reducer apply block 1");
            raws_b1.push(journaled(1, i as u32));
            decoded_b1.push((e.clone(), c));
        }

        // Pre-populate a COMPLETE matched execution row into projection
        // state so the block-atomic persist has a paired-executed event
        // + a COMPLETE row to write.
        state.matched_executions.insert(
            EXEC_ID.into(),
            MatchedExecutionRow {
                buyer_order_hash: EXEC_BUY.into(),
                seller_order_hash: EXEC_SELL.into(),
                buyer_subkey: SUBKEY.into(),
                seller_subkey: SUBKEY.into(),
                series_id: SERIES.into(),
                matched_qty_1e8: "10".into(),
                premium_amount: "100".into(),
                fee_amount: "1".into(),
                rebate_amount: "0".into(),
                block_number: 1,
                tx_hash: "0xtx1".into(),
                completion_status: ExecutionCompletion::Complete,
            },
        );
        // Append the paired-execution event to block 1 so persistence
        // has an anchor for the completed matched execution row.
        let pair_ev = pair_executed();
        let pair_ctx = ctx(1, events_b1.len() as u32);
        apply(&mut state, &pair_ev, &pair_ctx).expect("reducer apply pair");
        raws_b1.push(journaled(1, events_b1.len() as u32));
        decoded_b1.push((pair_ev, pair_ctx));

        projection
            .persist_block_atomic(
                deployment_id_i64,
                &block_ref(1),
                &raws_b1,
                &decoded_b1,
                &state,
                &cursor_snap(1),
                &ready(),
                now_ms(),
            )
            .await
            .expect("persist seeded block 1");

        deployment_id_i64
    }

    /// Build an `AppState` wired to the given pool + the seeded
    /// deployment id via the production Postgres path.
    pub fn app_state_with_pg(pool: PgPool, deployment_id: i64, tag: &str) -> AppState {
        let manifest = base_manifest(84532, tag);
        let entry = Arc::new(DeploymentEntry::from_metadata(
            deployment_id as u64,
            manifest,
        ));
        AppState::new(EngineState::with_default_markets())
            .with_hybrid_v2_postgres(pool, vec![entry])
    }
}

// -----------------------------------------------------------------
//                       REQUEST/RESPONSE HELPERS
// -----------------------------------------------------------------

async fn get_json(app: axum::Router, uri: &str) -> (StatusCode, Value) {
    let response = app
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .expect("router oneshot");
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("collect body");
    let json: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, json)
}

async fn method_status(app: axum::Router, method: Method, uri: &str) -> StatusCode {
    let response = app
        .oneshot(
            Request::builder()
                .method(method)
                .uri(uri)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("router oneshot");
    response.status()
}

/// Bootstrap sequence used by every test: return `None` when the PG
/// env isn't set (test skips), otherwise return a fully-seeded
/// (`pool`, `deployment_id`, `router`) tuple ready to serve requests.
async fn boot(tag: &str) -> Option<(PgPool, i64, axum::Router)> {
    let url = pg_harness::get_pg_url_or_skip_or_panic()?;
    let pool = pg_harness::fresh_pool(&url).await;
    let did = pg_harness::seed_fixture(&pool, tag).await;
    let app = router(pg_harness::app_state_with_pg(pool.clone(), did, tag));
    Some((pool, did, app))
}

// -----------------------------------------------------------------
//                          TESTS
// -----------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn deployments_route_lists_configured_deployment_through_pg_router() {
    let Some((_pool, did, app)) = boot("depl1").await else {
        eprintln!("SKIP hybrid_v2 pg main-router: no disposable URL");
        return;
    };
    let (status, body) = get_json(app, "/subaccounts/deployments").await;
    assert_eq!(status, StatusCode::OK);
    let arr = body.as_array().expect("deployments array");
    assert_eq!(arr.len(), 1, "expected exactly one configured deployment");
    let row = &arr[0];
    assert_eq!(row["deployment_id"].as_u64().unwrap(), did as u64);
    assert_eq!(row["chain_id"].as_u64().unwrap(), 84532);
    assert_eq!(row["ready"].as_bool(), Some(true));
}

#[tokio::test(flavor = "multi_thread")]
async fn deployment_status_readable_through_pg_router() {
    let Some((_pool, did, app)) = boot("stat1").await else {
        return;
    };
    let (status, body) = get_json(app, &format!("/subaccounts/deployments/{}/status", did)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["deployment_id"].as_u64().unwrap(), did as u64);
    assert_eq!(body["ready"].as_bool(), Some(true));
    assert_eq!(body["indexed_block"].as_u64().unwrap(), 1);
    assert!(body["metadata"].is_object());
}

#[tokio::test(flavor = "multi_thread")]
async fn owner_subaccounts_route_through_pg_router() {
    let Some((_pool, _did, app)) = boot("own1").await else {
        return;
    };
    let (status, body) = get_json(
        app,
        &format!("/accounts/{}/hybrid-v2/subaccounts", pg_harness::OWNER),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let subs = body["data"]["subaccounts"].as_array().expect("subaccounts");
    assert_eq!(subs.len(), 1);
    assert_eq!(subs[0]["subaccount_id"].as_u64().unwrap(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn subaccount_summary_route_through_pg_router() {
    let Some((_pool, _did, app)) = boot("sum1").await else {
        return;
    };
    let (status, body) = get_json(app, &format!("/subaccounts/{}", pg_harness::SUBKEY)).await;
    assert_eq!(status, StatusCode::OK);
    let data = &body["data"];
    assert_eq!(data["subkey"], pg_harness::SUBKEY);
    assert_eq!(data["subaccount_id"].as_u64().unwrap(), 1);
    assert!(data["balance_token_count"].as_u64().unwrap() >= 1);
    assert!(data["reservation_count"].as_u64().unwrap() >= 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn subaccount_collateral_route_through_pg_router() {
    let Some((_pool, _did, app)) = boot("col1").await else {
        return;
    };
    let (status, body) = get_json(
        app,
        &format!("/subaccounts/{}/collateral", pg_harness::SUBKEY),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let rows = body["data"].as_array().expect("collateral rows");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["token"], pg_harness::TOKEN);
    assert_eq!(rows[0]["balance"], "1000");
}

#[tokio::test(flavor = "multi_thread")]
async fn subaccount_reservations_route_through_pg_router() {
    let Some((_pool, _did, app)) = boot("res1").await else {
        return;
    };
    let (status, body) = get_json(
        app,
        &format!("/subaccounts/{}/reservations", pg_harness::SUBKEY),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let rows = body["data"].as_array().expect("reservation rows");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["token"], pg_harness::TOKEN);
    assert_eq!(rows[0]["engine"], pg_harness::ENGINE);
}

#[tokio::test(flavor = "multi_thread")]
async fn subaccount_positions_route_through_pg_router() {
    let Some((_pool, _did, app)) = boot("pos1").await else {
        return;
    };
    let (status, body) = get_json(
        app,
        &format!("/subaccounts/{}/positions", pg_harness::SUBKEY),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let rows = body["data"].as_array().expect("position rows");
    assert!(!rows.is_empty(), "expected at least one active position");
    assert!(rows.iter().any(|r| r["series_id"] == pg_harness::SERIES));
    assert!(rows.iter().any(|r| r["active"].as_bool() == Some(true)));
}

#[tokio::test(flavor = "multi_thread")]
async fn subaccount_orders_route_through_pg_router() {
    let Some((_pool, _did, app)) = boot("ord1").await else {
        return;
    };
    let (status, body) =
        get_json(app, &format!("/subaccounts/{}/orders", pg_harness::SUBKEY)).await;
    assert_eq!(status, StatusCode::OK);
    let rows = body["data"].as_array().expect("order rows");
    assert!(!rows.is_empty(), "expected at least one open order");
    assert!(rows
        .iter()
        .any(|r| r["order_hash"] == pg_harness::ORDER_HASH));
}

#[tokio::test(flavor = "multi_thread")]
async fn subaccount_executions_route_through_pg_router() {
    let Some((_pool, _did, app)) = boot("exe1").await else {
        return;
    };
    let (status, body) = get_json(
        app,
        &format!("/subaccounts/{}/executions", pg_harness::SUBKEY),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let rows = body["data"].as_array().expect("execution rows");
    assert!(
        !rows.is_empty(),
        "expected at least one completed execution"
    );
    // Wire format is `Vec<(execution_id: String, MatchedExecutionRow)>`
    // which serialises as `[[<id>, {row...}], ...]` — element 0 is the id.
    assert!(rows.iter().any(|r| r[0] == pg_harness::EXEC_ID));
}

#[tokio::test(flavor = "multi_thread")]
async fn subaccount_fees_route_through_pg_router() {
    let Some((_pool, _did, app)) = boot("fee1").await else {
        return;
    };
    let (status, body) = get_json(app, &format!("/subaccounts/{}/fees", pg_harness::SUBKEY)).await;
    assert_eq!(status, StatusCode::OK);
    let rows = body["data"].as_array().expect("fee rows");
    assert!(!rows.is_empty(), "expected at least one fee event");
    assert_eq!(rows[0]["amount"], "5");
}

#[tokio::test(flavor = "multi_thread")]
async fn subaccount_recovery_route_through_pg_router() {
    let Some((_pool, _did, app)) = boot("rec1").await else {
        return;
    };
    let (status, body) = get_json(
        app,
        &format!("/subaccounts/{}/recovery", pg_harness::SUBKEY),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["data"]["recovery_state"], "NORMAL");
    assert_eq!(body["data"]["finalized"], false);
}

#[tokio::test(flavor = "multi_thread")]
async fn order_lookup_route_through_pg_router() {
    let Some((_pool, _did, app)) = boot("olu1").await else {
        return;
    };
    let (status, body) = get_json(
        app,
        &format!("/hybrid-v2/orders/{}", pg_harness::ORDER_HASH),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    // `OrderLifecycleRow` — the wire type inherited from the pre-refactor
    // handler — does not carry `order_hash` in the response body; the
    // hash is the URL path parameter. The store lookup succeeded iff
    // the response has a decoded row with the seeded subkey/owner.
    assert_eq!(body["data"]["subkey"], pg_harness::SUBKEY);
    assert_eq!(body["data"]["owner"], pg_harness::OWNER);
}

#[tokio::test(flavor = "multi_thread")]
async fn subaccount_history_route_through_pg_router() {
    let Some((_pool, _did, app)) = boot("sh1").await else {
        return;
    };
    // No cursor — handler synthesises the first-page anchor; response
    // is a well-formed array (possibly with a `next_cursor`).
    let (status, body) = get_json(
        app,
        &format!("/subaccounts/{}/history?limit=10", pg_harness::SUBKEY),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={}", body);
    assert!(body["data"].is_array(), "history payload has data array");
}

#[tokio::test(flavor = "multi_thread")]
async fn global_history_route_through_pg_router() {
    let Some((_pool, _did, app)) = boot("gh1").await else {
        return;
    };
    let (status, body) = get_json(app, "/hybrid-v2/history?limit=10").await;
    assert_eq!(status, StatusCode::OK, "global history body={}", body);
    let items = body["data"].as_array().expect("history data");
    assert!(!items.is_empty(), "expected at least one history event");
}

#[tokio::test(flavor = "multi_thread")]
async fn owner_history_route_through_pg_router() {
    let Some((_pool, _did, app)) = boot("oh1").await else {
        return;
    };
    let (status, body) = get_json(
        app,
        &format!("/accounts/{}/hybrid-v2/history?limit=10", pg_harness::OWNER),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "owner history body={}", body);
    let items = body["data"].as_array().expect("history data");
    assert!(!items.is_empty(), "expected owner-scoped history items");
}

#[tokio::test(flavor = "multi_thread")]
async fn hybrid_v2_openapi_route_through_pg_router() {
    let Some((_pool, _did, app)) = boot("oapi").await else {
        return;
    };
    let (status, body) = get_json(app, "/hybrid-v2/openapi.json").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["openapi"], "3.1.0");
    assert!(body["paths"].is_object());
}

#[tokio::test(flavor = "multi_thread")]
async fn hybrid_v2_write_method_rejected_through_pg_router() {
    let Some((_pool, _did, app)) = boot("wr1").await else {
        return;
    };
    for method in [Method::POST, Method::PUT, Method::PATCH, Method::DELETE] {
        let s = method_status(
            app.clone(),
            method.clone(),
            &format!("/subaccounts/{}/collateral", pg_harness::SUBKEY),
        )
        .await;
        assert!(
            matches!(s, StatusCode::METHOD_NOT_ALLOWED | StatusCode::NOT_FOUND),
            "{} accepted on read-only collateral route: {}",
            method,
            s
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn existing_v1_history_route_remains_reachable_through_pg_router() {
    let Some((_pool, _did, app)) = boot("v1h").await else {
        return;
    };
    let addr = "0x000000000000000000000000000000000000abcd";
    let response = app
        .oneshot(
            Request::builder()
                .uri(&format!("/accounts/{}/history/v2", addr))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("oneshot");
    assert_ne!(
        response.status(),
        StatusCode::NOT_FOUND,
        "V1 /accounts/:addr/history/v2 was shadowed by the Hybrid V2 merge"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn health_route_remains_reachable_through_pg_router() {
    let Some((_pool, _did, app)) = boot("hlth").await else {
        return;
    };
    let response = app
        .oneshot(
            Request::builder()
                .uri("/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("oneshot");
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test(flavor = "multi_thread")]
async fn app_state_recreation_returns_identical_persisted_deployments() {
    let Some(url) = pg_harness::get_pg_url_or_skip_or_panic() else {
        return;
    };
    let pool = pg_harness::fresh_pool(&url).await;
    let tag = "rec2";
    let did = pg_harness::seed_fixture(&pool, tag).await;

    // First AppState — same seeded PG.
    let app_a = router(pg_harness::app_state_with_pg(pool.clone(), did, tag));
    let (sa, body_a) = get_json(app_a, "/subaccounts/deployments").await;
    assert_eq!(sa, StatusCode::OK);

    // Second AppState — brand new instance against the same pool.
    let app_b = router(pg_harness::app_state_with_pg(pool.clone(), did, tag));
    let (sb, body_b) = get_json(app_b, "/subaccounts/deployments").await;
    assert_eq!(sb, StatusCode::OK);

    assert_eq!(
        body_a, body_b,
        "same PG state must yield identical deployments across AppStates"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn postgres_unavailability_fails_closed_no_memory_fallback() {
    let Some(url) = pg_harness::get_pg_url_or_skip_or_panic() else {
        return;
    };
    let pool = pg_harness::fresh_pool(&url).await;
    let tag = "fc1";
    let did = pg_harness::seed_fixture(&pool, tag).await;

    // Wire the state to the pool, then close the pool so subsequent
    // queries fail. This proves that the wire path does NOT downgrade
    // to an in-memory fallback — reads must surface as a structured
    // 5xx with a decodable error body.
    let state = pg_harness::app_state_with_pg(pool.clone(), did, tag);
    let app = router(state);
    pool.close().await;

    // A canonical read that requires readiness + PG access.
    let (status, body) = get_json(
        app,
        &format!("/subaccounts/{}/collateral", pg_harness::SUBKEY),
    )
    .await;
    assert!(
        status.is_server_error(),
        "expected 5xx after pool close, got {}, body={}",
        status,
        body
    );
    // Structured error body — the runtime never returns a bare empty
    // response, and never leaks credentials.
    assert!(body["code"].is_string(), "missing code in body: {}", body);
    assert!(body["message"].is_string(), "missing message: {}", body);
    assert!(
        body["retryable"].is_boolean(),
        "missing retryable: {}",
        body
    );
    // Sanity: no URL / credential fragment leaks into the message. We
    // don't have access to the URL string here, so we assert the
    // message does not contain a stray 'postgres://' scheme fragment.
    let msg = body["message"].as_str().unwrap_or("");
    assert!(
        !msg.contains("postgres://") && !msg.contains("postgresql://"),
        "message must not leak connection strings: {}",
        msg
    );
}
