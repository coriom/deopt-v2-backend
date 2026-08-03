//! `BACKEND-PG-MIGRATION-CHAIN-INTEGRITY-V1` — permanent migration
//! integration test.
//!
//! Applies the complete repository migration chain to a real, disposable
//! PostgreSQL database and verifies critical schema invariants. Also
//! reruns migrations to prove they are a no-op the second time.
//!
//! ## Modes
//!
//! - **Default (no environment)**: the test returns early with a clear
//!   `SKIP` message. `cargo test` stays green in developer environments
//!   without PostgreSQL. This matches the historical pattern used by
//!   every other `*_pg_proof.rs` file in the repository.
//! - **Required (`DEOPT_REQUIRE_PG_INTEGRATION=1`)**: the test **fails
//!   loudly** when `PG_INTEGRATION_URL` (or the legacy
//!   `HYBRID_V2_PG_TEST_DATABASE_URL`) is unset. This is the CI mode —
//!   silent skipping is forbidden.
//!
//! ## Safety
//!
//! - The database URL is read once from the environment and never
//!   printed or logged.
//! - The test rejects URLs that do not look disposable — the URL's
//!   path segment must contain either `disposable` or `test`, and must
//!   not resemble a production database name.
//! - The test drops schema `public` and recreates it before applying
//!   migrations, so the database MUST be dedicated to this test run.
//!   Never point this test at an application database.

use sqlx::postgres::PgPoolOptions;
use sqlx::Row;

const URL_ENV: &str = "PG_INTEGRATION_URL";
const LEGACY_URL_ENV: &str = "HYBRID_V2_PG_TEST_DATABASE_URL";
const REQUIRE_ENV: &str = "DEOPT_REQUIRE_PG_INTEGRATION";

fn integration_url() -> Option<String> {
    std::env::var(URL_ENV)
        .ok()
        .or_else(|| std::env::var(LEGACY_URL_ENV).ok())
        .filter(|v| !v.is_empty())
}

fn required_mode() -> bool {
    matches!(
        std::env::var(REQUIRE_ENV).ok().as_deref(),
        Some("1") | Some("true") | Some("TRUE")
    )
}

fn assert_disposable_url(url: &str) {
    let looks_disposable = url.contains("disposable")
        || url.contains("test")
        || url.contains("ephemeral")
        || url.contains("_ci_")
        || url.contains("/ci_")
        // Repository-standard disposable-DB naming prefixes used by
        // scripts/*-pg-proof.sh and by the migration-integrity harness.
        || url.contains("deopt_mig_")
        || url.contains("deopt_hv2_")
        || url.contains("deopt_hybrid_v2_")
        || url.contains("deopt_perps_")
        || url.contains("deopt_rfq_")
        || url.contains("deopt_conditional_")
        || url.contains("deopt_write_auth_");
    assert!(
        looks_disposable,
        "PG_INTEGRATION_URL must reference a disposable database — accepted \
         markers include 'disposable', 'test', 'ephemeral', or one of the \
         repository-standard 'deopt_(mig|hv2|hybrid_v2|perps|rfq|conditional|write_auth)_*' \
         prefixes. This test drops schema public and MUST NOT run against a \
         production or long-lived database"
    );
    let bad_names = ["prod", "production", "mainnet", "canary", "release", "live"];
    for bad in bad_names {
        assert!(
            !url.to_lowercase().contains(bad),
            "PG_INTEGRATION_URL contains suspicious segment '{}' \
             — refusing to run migration integration test against this URL",
            bad
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn migration_chain_applies_cleanly_to_fresh_disposable_postgres() {
    let url = match integration_url() {
        Some(u) => u,
        None => {
            if required_mode() {
                panic!(
                    "{} is enabled but neither {} nor {} is set — \
                     migration integration test cannot execute in \
                     required-mode without a disposable PostgreSQL URL",
                    REQUIRE_ENV, URL_ENV, LEGACY_URL_ENV
                );
            } else {
                eprintln!(
                    "[migration-integration] SKIP: neither {} nor {} set \
                     (set {}=1 to force this test to fail loudly)",
                    URL_ENV, LEGACY_URL_ENV, REQUIRE_ENV
                );
                return;
            }
        }
    };
    assert_disposable_url(&url);

    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect(&url)
        .await
        .expect("connect to disposable PostgreSQL");

    // Reset schema public so we start from a genuinely empty state.
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
        .expect("load migrations directory");

    // First application: must succeed cleanly.
    migrator
        .run(&pool)
        .await
        .expect("first application of migration chain must succeed");

    // Second application: must be a no-op (sqlx checks checksums against
    // the recorded _sqlx_migrations rows).
    migrator
        .run(&pool)
        .await
        .expect("second application must be a checksum-clean no-op");

    // Verify migration count matches the number of files in ./migrations
    let installed: i64 = sqlx::query("SELECT COUNT(*) AS c FROM _sqlx_migrations")
        .fetch_one(&pool)
        .await
        .expect("query _sqlx_migrations")
        .try_get("c")
        .expect("read c");
    let file_count = std::fs::read_dir("./migrations")
        .expect("read migrations dir")
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.path()
                .extension()
                .and_then(|s| s.to_str())
                .map(|s| s.eq_ignore_ascii_case("sql"))
                .unwrap_or(false)
        })
        .count() as i64;
    assert_eq!(
        installed, file_count,
        "installed migration count {installed} != file count {file_count}"
    );

    // ------------------------------------------------------------------
    // Critical schema invariants
    // ------------------------------------------------------------------

    // 1. V1 subaccounts table exists with the case-insensitive uniqueness
    //    invariant enforced by an index (post-0038-repair). Insert two
    //    distinct-case owner values with the same subaccount_id and prove
    //    the second insert conflicts.
    sqlx::query(
        "INSERT INTO subaccounts (owner_address, subaccount_id, name, created_at_ms, updated_at_ms)
         VALUES ($1, $2, NULL, $3, $3)",
    )
    .bind("0xaabbccddeeff00112233445566778899aabbccdd")
    .bind(1_i32)
    .bind(1_700_000_000_000_i64)
    .execute(&pool)
    .await
    .expect("insert lowercase owner subaccount 1");

    let duplicate_lower = sqlx::query(
        "INSERT INTO subaccounts (owner_address, subaccount_id, name, created_at_ms, updated_at_ms)
         VALUES ($1, $2, NULL, $3, $3)",
    )
    .bind("0xaabbccddeeff00112233445566778899aabbccdd")
    .bind(1_i32)
    .bind(1_700_000_000_001_i64)
    .execute(&pool)
    .await;
    assert!(
        duplicate_lower.is_err(),
        "identical (owner, subaccount_id) must conflict on the primary key"
    );

    let mixed_case_variant = sqlx::query(
        "INSERT INTO subaccounts (owner_address, subaccount_id, name, created_at_ms, updated_at_ms)
         VALUES ($1, $2, NULL, $3, $3)",
    )
    .bind("0xAABBCCDDEEFF00112233445566778899AABBCCDD")
    .bind(1_i32)
    .bind(1_700_000_000_002_i64)
    .execute(&pool)
    .await;
    assert!(
        mixed_case_variant.is_err(),
        "mixed-case owner address with same subaccount_id must conflict on the case-insensitive unique index"
    );

    // Different subaccount_id for the same owner succeeds.
    sqlx::query(
        "INSERT INTO subaccounts (owner_address, subaccount_id, name, created_at_ms, updated_at_ms)
         VALUES ($1, $2, NULL, $3, $3)",
    )
    .bind("0xaabbccddeeff00112233445566778899aabbccdd")
    .bind(2_i32)
    .bind(1_700_000_000_003_i64)
    .execute(&pool)
    .await
    .expect("different subaccount_id for same owner must succeed");

    // Different owner with same subaccount_id succeeds.
    sqlx::query(
        "INSERT INTO subaccounts (owner_address, subaccount_id, name, created_at_ms, updated_at_ms)
         VALUES ($1, $2, NULL, $3, $3)",
    )
    .bind("0x00112233445566778899aabbccddeeff00112233")
    .bind(1_i32)
    .bind(1_700_000_000_004_i64)
    .execute(&pool)
    .await
    .expect("different owner with same subaccount_id must succeed");

    // 2. Hybrid V2 canonical projection tables from 0044/0045/0046 exist.
    let expected_hv2_tables = [
        "hybrid_v2_deployments",
        "hybrid_v2_cursors",
        "hybrid_v2_raw_logs",
        "hybrid_v2_decoded_events",
        "hybrid_v2_subaccounts",
        "hybrid_v2_vault_balances",
        "hybrid_v2_reservations",
        "hybrid_v2_collateral_universe",
        "hybrid_v2_capability_grants",
        "hybrid_v2_recovery_state",
        "hybrid_v2_positions",
        "hybrid_v2_active_series",
        "hybrid_v2_order_lifecycle",
        "hybrid_v2_matched_executions",
        "hybrid_v2_fee_events",
        "hybrid_v2_recovery_epochs",
        "hybrid_v2_escape_state",
        "hybrid_v2_projection_status",
        "hybrid_v2_reconciliation_status",
        "hybrid_v2_canonical_blocks",
        "hybrid_v2_pause_flags",
        "hybrid_v2_bad_debt",
        "hybrid_v2_recovery_withdrawal_counts",
        "hybrid_v2_recovery_pause",
        "hybrid_v2_runtime_metrics",
        "hybrid_v2_readiness",
        "hybrid_v2_reorg_events",
    ];
    for t in expected_hv2_tables {
        let exists: bool = sqlx::query(
            "SELECT EXISTS (SELECT 1 FROM information_schema.tables
             WHERE table_schema='public' AND table_name=$1)",
        )
        .bind(t)
        .fetch_one(&pool)
        .await
        .expect("query information_schema")
        .try_get(0)
        .expect("read bool");
        assert!(exists, "expected table {} missing after migration chain", t);
    }

    // 3. hybrid_v2_cursors has the additive columns from 0046.
    let extra_cursor_cols = [
        "reorg_count",
        "max_reorg_depth_seen",
        "decode_failures",
        "projection_failures",
        "unknown_canonical_events",
        "last_success_block",
    ];
    for col in extra_cursor_cols {
        let exists: bool = sqlx::query(
            "SELECT EXISTS (SELECT 1 FROM information_schema.columns
             WHERE table_schema='public'
               AND table_name='hybrid_v2_cursors'
               AND column_name=$1)",
        )
        .bind(col)
        .fetch_one(&pool)
        .await
        .expect("query information_schema for cursor column")
        .try_get(0)
        .expect("read bool");
        assert!(
            exists,
            "expected column hybrid_v2_cursors.{} missing (migration 0046)",
            col
        );
    }

    // 4. Migration count sanity — must be at least 46 (0001 through 0046).
    assert!(
        file_count >= 46,
        "expected at least 46 migrations, found {file_count}"
    );

    eprintln!(
        "[migration-integration] OK: {} migrations applied cleanly, second application no-op, \
         critical invariants pass",
        installed
    );
}
