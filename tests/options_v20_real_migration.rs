//! Disposable PostgreSQL restore only. Never run against the hosted database.
use deopt_v2_backend::db::{pool, PgRepository};
use deopt_v2_backend::options::{
    onchain_option_series_key, OptionSeries, OptionSeriesSource, OptionSeriesStatus,
};

#[tokio::test]
#[ignore = "requires a verified, decrypted real backup in a disposable local PostgreSQL 16"]
async fn real_backup_migrates_to_dual_registry_schema() {
    let url = std::env::var("OPTIONS_V20_REAL_PG_URL").expect("isolated database URL");
    assert!(url.contains("127.0.0.1:"), "local PostgreSQL only");
    let repo = PgRepository::connect(&url)
        .await
        .expect("connect isolated restore");
    let before: (i64, i64) =
        sqlx::query_as("SELECT COUNT(*)::bigint, MAX(version)::bigint FROM _sqlx_migrations")
            .fetch_one(repo.pool())
            .await
            .expect("baseline migrations");
    assert!(
        before.1 >= 71 && before.1 <= 73,
        "unexpected baseline schema"
    );
    let historical_count: (i64,) = sqlx::query_as("SELECT COUNT(*)::bigint FROM option_series")
        .fetch_one(repo.pool())
        .await
        .expect("historical series");
    repo.run_migrations()
        .await
        .expect("migrate disposable database");
    let after: (i64, i64) =
        sqlx::query_as("SELECT COUNT(*)::bigint, MAX(version)::bigint FROM _sqlx_migrations")
            .fetch_one(repo.pool())
            .await
            .expect("migrated versions");
    assert_eq!(after.1, 74);
    assert_eq!(after.0, 74);
    pool::verify_read_only_schema(repo.pool())
        .await
        .expect("V20 read-only schema gate");
    let preserved: (i64,) = sqlx::query_as("SELECT COUNT(*)::bigint FROM option_series WHERE onchain_chain_id IS NULL AND onchain_registry_address IS NULL")
        .fetch_one(repo.pool()).await.expect("historical series preserved");
    assert_eq!(preserved.0, historical_count.0);
    let series = |registry: &str| OptionSeries {
        option_series_id: onchain_option_series_key(84532, registry, "1").unwrap(),
        underlying: "0x0000000000000000000000000000000000000001".into(),
        base_asset: "0x0000000000000000000000000000000000000001".into(),
        quote_asset: "0x0000000000000000000000000000000000000002".into(),
        settlement_asset: "0x0000000000000000000000000000000000000002".into(),
        expiry: 1_800_000_000,
        strike_1e8: 2250_00000000,
        is_call: true,
        contract_size_1e8: 100_000_000,
        status: OptionSeriesStatus::Disabled,
        source: OptionSeriesSource::Onchain,
        onchain_product_id: None,
        onchain_series_id: Some("1".into()),
        onchain_chain_id: Some(84532),
        onchain_registry_address: Some(registry.into()),
        created_at_ms: 1,
        updated_at_ms: 1,
    };
    let first = "0x1111111111111111111111111111111111111111";
    let second = "0x2222222222222222222222222222222222222222";
    repo.insert_option_series(&series(first))
        .await
        .expect("first registry option 1");
    repo.insert_option_series(&series(second))
        .await
        .expect("second registry option 1");
    let collision_count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*)::bigint FROM option_series WHERE onchain_chain_id=84532 AND onchain_series_id='1' AND onchain_registry_address IN ($1,$2)"
    ).bind(first).bind(second).fetch_one(repo.pool()).await.expect("registry collision query");
    assert_eq!(collision_count.0, 2);
    sqlx::query("DELETE FROM option_series WHERE onchain_chain_id=84532 AND onchain_series_id='1' AND onchain_registry_address IN ($1,$2)")
        .bind(first).bind(second).execute(repo.pool()).await.expect("remove disposable collision fixtures");
    println!(
        "REAL_0074_MIGRATIONS={}; HISTORICAL_SERIES={}; COLLIDING_OPTION_IDS=2",
        after.0, historical_count.0
    );
}
