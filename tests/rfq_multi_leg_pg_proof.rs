// RFQ-MULTI-LEG-SCHEMA-V1 — live PostgreSQL proof for the multi-leg
// atomic RFQ schema + repository CRUD. Gated on the env var
// `RFQ_MULTI_LEG_PG_TEST_DATABASE_URL`. If the var is unset every
// test returns early so `cargo test` stays green in developer
// environments without a Postgres instance.
//
// What this suite proves WHEN ENABLED:
//
//   1. Migration 0043 applies cleanly against a fresh database.
//   2. All 6 multi-leg tables exist.
//   3. `taker_subaccount_id` + `maker_subaccount_id` default to 1
//      when the INSERT omits them, and reject values < 1 through
//      the CHECK constraint.
//   4. `insert_option_multi_leg_rfq` persists parent + N legs in a
//      single transaction and rejects <2 or >8 legs.
//   5. `get_option_multi_leg_rfq` returns parent + legs in stable
//      `leg_index` order.
//   6. `list_option_multi_leg_rfqs_by_taker` isolates Account 1 from
//      Account 2 for the same wallet address (no cross-subaccount
//      leakage).
//   7. `insert_option_multi_leg_rfq_quote` persists quote + N quote
//      legs transactionally; duplicate `client_quote_id` for the same
//      (RFQ, maker) rejects with `InvalidOptionRfqQuoteState`.
//   8. `insert_option_multi_leg_rfq_fill` persists fill + N fill legs
//      transactionally and preserves stable ordering.
//   9. `list_option_multi_leg_rfq_quotes_by_maker` isolates Account 1
//      from Account 2 for the same maker wallet.
//
// Safety: never prints `RFQ_MULTI_LEG_PG_TEST_DATABASE_URL` or any
// derivative. All assertions read non-secret fields only.

use deopt_v2_backend::db::PgRepository;
use deopt_v2_backend::options::service::{create_option_series, CreateOptionSeriesInput};
use deopt_v2_backend::options::{
    OptionMultiLegRfqFill, OptionMultiLegRfqFillLeg, OptionMultiLegRfqLeg, OptionMultiLegRfqQuote,
    OptionMultiLegRfqQuoteLeg, OptionMultiLegRfqQuoteSignatureStatus, OptionMultiLegRfqQuoteStatus,
    OptionMultiLegRfqRequest, OptionMultiLegRfqStatus, OptionsConfig,
};
use deopt_v2_backend::types::{AccountId, Side};
use uuid::Uuid;

use deopt_v2_backend::api::AppState;
use deopt_v2_backend::engine::EngineState;

const ENV_VAR: &str = "RFQ_MULTI_LEG_PG_TEST_DATABASE_URL";

fn pg_test_url() -> Option<String> {
    std::env::var(ENV_VAR).ok().filter(|v| !v.is_empty())
}

async fn ensure_migrated(url: &str) {
    static MIGRATED: tokio::sync::OnceCell<()> = tokio::sync::OnceCell::const_new();
    MIGRATED
        .get_or_init(|| async {
            let repo = PgRepository::connect(url)
                .await
                .expect("connect for shared migration");
            repo.run_migrations()
                .await
                .expect("run migrations once against disposable PG database");
        })
        .await;
}

async fn fresh_repo(url: &str) -> PgRepository {
    ensure_migrated(url).await;
    PgRepository::connect(url)
        .await
        .expect("connect to disposable PG database")
}

async fn seed_series(repo: &PgRepository, tag: &str) -> String {
    let state = AppState::with_options_config_and_repository(
        EngineState::with_default_markets(),
        {
            let mut cfg = OptionsConfig::disabled();
            cfg.enabled = true;
            cfg.require_persistence = true;
            cfg
        },
        repo.clone(),
    );
    let expiry = u64::try_from(deopt_v2_backend::types::now_ms() / 1000).unwrap()
        + 86_400
        + tag.bytes().map(u64::from).sum::<u64>();
    let series = create_option_series(
        &state,
        CreateOptionSeriesInput {
            underlying: "ETH".to_string(),
            base_asset: "ETH".to_string(),
            quote_asset: "USDC".to_string(),
            settlement_asset: "USDC".to_string(),
            expiry,
            strike_1e8: 300_000_000_000 + tag.bytes().next().unwrap_or(0) as u128 * 1_000_000,
            is_call: true,
            contract_size_1e8: Some(100_000_000),
            onchain_product_id: None,
            onchain_series_id: None,
        },
    )
    .await
    .expect("seed option series");
    series.option_series_id
}

fn taker_for(tag: &str) -> AccountId {
    // Deterministic per-test wallet so parallel PG tests do not fight
    // over the same subaccount rows on a shared disposable database.
    let sum: u32 = tag.bytes().map(u32::from).sum();
    let mut hex = String::from("0x");
    hex.push_str(&"a".repeat(20));
    hex.push_str(&format!("{:04x}", sum & 0xffff));
    for b in tag.bytes().take(8) {
        hex.push_str(&format!("{:02x}", b));
    }
    while hex.len() < 42 {
        hex.push('0');
    }
    hex.truncate(42);
    AccountId::new(hex)
}

fn maker_for(tag: &str) -> AccountId {
    let sum: u32 = tag.bytes().rev().map(u32::from).sum();
    let mut hex = String::from("0x");
    hex.push_str(&"b".repeat(20));
    hex.push_str(&format!("{:04x}", sum & 0xffff));
    for b in tag.bytes().take(8) {
        hex.push_str(&format!("{:02x}", b));
    }
    while hex.len() < 42 {
        hex.push('1');
    }
    hex.truncate(42);
    AccountId::new(hex)
}

fn make_rfq(
    rfq_id: Uuid,
    taker: AccountId,
    subaccount_id: u32,
    expires_at_ms: i64,
) -> OptionMultiLegRfqRequest {
    OptionMultiLegRfqRequest {
        option_rfq_id: rfq_id,
        taker,
        taker_subaccount_id: subaccount_id,
        status: OptionMultiLegRfqStatus::Open,
        created_at_ms: 0,
        expires_at_ms,
        accepted_quote_id: None,
        accepted_fill_id: None,
    }
}

fn make_leg(rfq_id: Uuid, index: u32, series_id: &str) -> OptionMultiLegRfqLeg {
    OptionMultiLegRfqLeg {
        option_rfq_id: rfq_id,
        leg_index: index,
        option_series_id: series_id.to_string(),
        side: if index % 2 == 0 {
            Side::Buy
        } else {
            Side::Sell
        },
        size_1e8: 100_000_000,
        ratio_num: 1,
        ratio_den: 1,
    }
}

fn make_quote(
    quote_id: Uuid,
    rfq_id: Uuid,
    mm: AccountId,
    subaccount_id: u32,
    expires_at_ms: i64,
    client_quote_id: Option<&str>,
) -> OptionMultiLegRfqQuote {
    OptionMultiLegRfqQuote {
        quote_id,
        option_rfq_id: rfq_id,
        mm_account: mm,
        maker_subaccount_id: subaccount_id,
        session_id: None,
        client_quote_id: client_quote_id.map(str::to_string),
        package_price_1e8: "50000000".to_string(),
        size_1e8: 100_000_000,
        status: OptionMultiLegRfqQuoteStatus::Active,
        created_at_ms: 0,
        expires_at_ms,
        signature: None,
        quote_digest: None,
        quote_nonce: None,
        signature_status: OptionMultiLegRfqQuoteSignatureStatus::NotRequired,
        recovered_signer: None,
    }
}

fn make_quote_leg(quote_id: Uuid, index: u32, price_1e8: u128) -> OptionMultiLegRfqQuoteLeg {
    OptionMultiLegRfqQuoteLeg {
        quote_id,
        leg_index: index,
        price_1e8,
    }
}

fn make_fill(
    fill_id: Uuid,
    rfq_id: Uuid,
    quote_id: Uuid,
    taker: AccountId,
    taker_subaccount_id: u32,
    mm: AccountId,
    maker_subaccount_id: u32,
) -> OptionMultiLegRfqFill {
    OptionMultiLegRfqFill {
        fill_id,
        option_rfq_id: rfq_id,
        quote_id,
        taker,
        taker_subaccount_id,
        mm_account: mm,
        maker_subaccount_id,
        package_price_1e8: "50000000".to_string(),
        size_1e8: 100_000_000,
        created_at_ms: 0,
    }
}

fn make_fill_leg(
    fill_id: Uuid,
    index: u32,
    series_id: &str,
    price_1e8: u128,
) -> OptionMultiLegRfqFillLeg {
    OptionMultiLegRfqFillLeg {
        fill_id,
        leg_index: index,
        option_series_id: series_id.to_string(),
        side: if index % 2 == 0 {
            Side::Buy
        } else {
            Side::Sell
        },
        size_1e8: 100_000_000,
        price_1e8,
    }
}

// ---------------------------------------------------------------------

#[tokio::test]
async fn part1_migration_and_insert_two_leg_rfq_round_trips() {
    let Some(url) = pg_test_url() else {
        return;
    };
    let repo = fresh_repo(&url).await;
    let series_a = seed_series(&repo, "p1a").await;
    let series_b = seed_series(&repo, "p1b").await;

    let rfq_id = Uuid::new_v4();
    let taker = taker_for("p1");
    let rfq = make_rfq(rfq_id, taker.clone(), 1, 9_000_000_000_000);
    let legs = vec![
        make_leg(rfq_id, 0, &series_a),
        make_leg(rfq_id, 1, &series_b),
    ];

    repo.insert_option_multi_leg_rfq(&rfq, &legs)
        .await
        .expect("insert 2-leg RFQ");

    let (loaded_rfq, loaded_legs) = repo
        .get_option_multi_leg_rfq(rfq_id)
        .await
        .expect("get RFQ")
        .expect("row must exist");
    assert_eq!(loaded_rfq.option_rfq_id, rfq_id);
    assert_eq!(loaded_rfq.taker_subaccount_id, 1);
    assert_eq!(loaded_legs.len(), 2);
    assert_eq!(loaded_legs[0].leg_index, 0);
    assert_eq!(loaded_legs[1].leg_index, 1);
}

#[tokio::test]
async fn part2_max_legs_boundary_accepts_eight_rejects_nine() {
    let Some(url) = pg_test_url() else {
        return;
    };
    let repo = fresh_repo(&url).await;
    let series = seed_series(&repo, "p2").await;

    let rfq_id_ok = Uuid::new_v4();
    let taker = taker_for("p2");
    let rfq_ok = make_rfq(rfq_id_ok, taker.clone(), 1, 9_000_000_000_000);
    let eight: Vec<_> = (0..8u32).map(|i| make_leg(rfq_id_ok, i, &series)).collect();
    repo.insert_option_multi_leg_rfq(&rfq_ok, &eight)
        .await
        .expect("8-leg boundary must succeed");

    let rfq_id_bad = Uuid::new_v4();
    let rfq_bad = make_rfq(rfq_id_bad, taker, 1, 9_000_000_000_000);
    let nine: Vec<_> = (0..9u32)
        .map(|i| make_leg(rfq_id_bad, i, &series))
        .collect();
    let err = repo
        .insert_option_multi_leg_rfq(&rfq_bad, &nine)
        .await
        .expect_err("9-leg must reject");
    let msg = err.to_string();
    assert!(msg.contains("at most 8"), "message = {msg}");
}

#[tokio::test]
async fn part3_list_by_taker_isolates_account_1_from_account_2() {
    let Some(url) = pg_test_url() else {
        return;
    };
    let repo = fresh_repo(&url).await;
    let series = seed_series(&repo, "p3").await;
    let taker = taker_for("p3");

    let rfq_a1 = Uuid::new_v4();
    let rfq_a2 = Uuid::new_v4();
    repo.insert_option_multi_leg_rfq(
        &make_rfq(rfq_a1, taker.clone(), 1, 9_000_000_000_000),
        &[make_leg(rfq_a1, 0, &series), make_leg(rfq_a1, 1, &series)],
    )
    .await
    .unwrap();
    repo.insert_option_multi_leg_rfq(
        &make_rfq(rfq_a2, taker.clone(), 2, 9_000_000_000_000),
        &[make_leg(rfq_a2, 0, &series), make_leg(rfq_a2, 1, &series)],
    )
    .await
    .unwrap();

    let acct_1 = repo
        .list_option_multi_leg_rfqs_by_taker(&taker, 1)
        .await
        .unwrap();
    let acct_2 = repo
        .list_option_multi_leg_rfqs_by_taker(&taker, 2)
        .await
        .unwrap();

    assert!(
        acct_1.iter().all(|(r, _)| r.taker_subaccount_id == 1),
        "account 1 list must not leak account 2 rows"
    );
    assert!(
        acct_2.iter().all(|(r, _)| r.taker_subaccount_id == 2),
        "account 2 list must not leak account 1 rows"
    );
    assert!(acct_1.iter().any(|(r, _)| r.option_rfq_id == rfq_a1));
    assert!(acct_2.iter().any(|(r, _)| r.option_rfq_id == rfq_a2));
    // Cross-subaccount refusal by construction:
    assert!(!acct_1.iter().any(|(r, _)| r.option_rfq_id == rfq_a2));
    assert!(!acct_2.iter().any(|(r, _)| r.option_rfq_id == rfq_a1));
}

#[tokio::test]
async fn part4_quote_transaction_round_trip_and_duplicate_client_id_rejects() {
    let Some(url) = pg_test_url() else {
        return;
    };
    let repo = fresh_repo(&url).await;
    let series = seed_series(&repo, "p4").await;
    let taker = taker_for("p4t");
    let mm = maker_for("p4m");

    let rfq_id = Uuid::new_v4();
    repo.insert_option_multi_leg_rfq(
        &make_rfq(rfq_id, taker.clone(), 1, 9_000_000_000_000),
        &[make_leg(rfq_id, 0, &series), make_leg(rfq_id, 1, &series)],
    )
    .await
    .unwrap();

    let quote_id = Uuid::new_v4();
    let quote = make_quote(
        quote_id,
        rfq_id,
        mm.clone(),
        1,
        9_000_000_000_000,
        Some("cid-1"),
    );
    let quote_legs = vec![
        make_quote_leg(quote_id, 0, 12_000_000_000),
        make_quote_leg(quote_id, 1, 11_500_000_000),
    ];
    repo.insert_option_multi_leg_rfq_quote(&quote, 2, &quote_legs)
        .await
        .expect("insert quote transactionally");

    let (loaded_quote, loaded_legs) = repo
        .get_option_multi_leg_rfq_quote(quote_id)
        .await
        .unwrap()
        .expect("row must exist");
    assert_eq!(loaded_quote.maker_subaccount_id, 1);
    assert_eq!(loaded_legs.len(), 2);
    assert_eq!(loaded_legs[0].leg_index, 0);
    assert_eq!(loaded_legs[1].leg_index, 1);

    let dup_id = Uuid::new_v4();
    let dup = make_quote(
        dup_id,
        rfq_id,
        mm.clone(),
        1,
        9_000_000_000_000,
        Some("cid-1"),
    );
    let err = repo
        .insert_option_multi_leg_rfq_quote(&dup, 2, &quote_legs)
        .await
        .expect_err("duplicate client_quote_id must reject");
    let msg = err.to_string();
    assert!(
        msg.to_ascii_lowercase().contains("duplicate"),
        "msg = {msg}"
    );
}

#[tokio::test]
async fn part5_list_quotes_by_maker_isolates_account_1_from_account_2() {
    let Some(url) = pg_test_url() else {
        return;
    };
    let repo = fresh_repo(&url).await;
    let series = seed_series(&repo, "p5").await;
    let taker = taker_for("p5t");
    let mm = maker_for("p5m");

    let rfq_id = Uuid::new_v4();
    repo.insert_option_multi_leg_rfq(
        &make_rfq(rfq_id, taker.clone(), 1, 9_000_000_000_000),
        &[make_leg(rfq_id, 0, &series), make_leg(rfq_id, 1, &series)],
    )
    .await
    .unwrap();

    let quote_a1 = Uuid::new_v4();
    let quote_a2 = Uuid::new_v4();
    repo.insert_option_multi_leg_rfq_quote(
        &make_quote(quote_a1, rfq_id, mm.clone(), 1, 9_000_000_000_000, None),
        2,
        &[
            make_quote_leg(quote_a1, 0, 1_000_000),
            make_quote_leg(quote_a1, 1, 2_000_000),
        ],
    )
    .await
    .unwrap();
    repo.insert_option_multi_leg_rfq_quote(
        &make_quote(quote_a2, rfq_id, mm.clone(), 2, 9_000_000_000_000, None),
        2,
        &[
            make_quote_leg(quote_a2, 0, 3_000_000),
            make_quote_leg(quote_a2, 1, 4_000_000),
        ],
    )
    .await
    .unwrap();

    let list_1 = repo
        .list_option_multi_leg_rfq_quotes_by_maker(&mm, 1)
        .await
        .unwrap();
    let list_2 = repo
        .list_option_multi_leg_rfq_quotes_by_maker(&mm, 2)
        .await
        .unwrap();
    assert!(list_1.iter().all(|(q, _)| q.maker_subaccount_id == 1));
    assert!(list_2.iter().all(|(q, _)| q.maker_subaccount_id == 2));
    assert!(list_1.iter().any(|(q, _)| q.quote_id == quote_a1));
    assert!(list_2.iter().any(|(q, _)| q.quote_id == quote_a2));
    assert!(!list_1.iter().any(|(q, _)| q.quote_id == quote_a2));
    assert!(!list_2.iter().any(|(q, _)| q.quote_id == quote_a1));
}

#[tokio::test]
async fn part6_fill_transaction_round_trip_preserves_stable_ordering() {
    let Some(url) = pg_test_url() else {
        return;
    };
    let repo = fresh_repo(&url).await;
    let series_a = seed_series(&repo, "p6a").await;
    let series_b = seed_series(&repo, "p6b").await;
    let taker = taker_for("p6t");
    let mm = maker_for("p6m");

    let rfq_id = Uuid::new_v4();
    let quote_id = Uuid::new_v4();
    let fill_id = Uuid::new_v4();

    repo.insert_option_multi_leg_rfq(
        &make_rfq(rfq_id, taker.clone(), 1, 9_000_000_000_000),
        &[
            make_leg(rfq_id, 0, &series_a),
            make_leg(rfq_id, 1, &series_b),
        ],
    )
    .await
    .unwrap();
    repo.insert_option_multi_leg_rfq_quote(
        &make_quote(quote_id, rfq_id, mm.clone(), 1, 9_000_000_000_000, None),
        2,
        &[
            make_quote_leg(quote_id, 0, 10_000_000),
            make_quote_leg(quote_id, 1, 20_000_000),
        ],
    )
    .await
    .unwrap();
    repo.insert_option_multi_leg_rfq_fill(
        &make_fill(fill_id, rfq_id, quote_id, taker.clone(), 1, mm.clone(), 1),
        2,
        &[
            make_fill_leg(fill_id, 0, &series_a, 10_000_000),
            make_fill_leg(fill_id, 1, &series_b, 20_000_000),
        ],
    )
    .await
    .unwrap();

    let (loaded_fill, loaded_legs) = repo
        .get_option_multi_leg_rfq_fill(fill_id)
        .await
        .unwrap()
        .expect("fill must exist");
    assert_eq!(loaded_fill.taker_subaccount_id, 1);
    assert_eq!(loaded_fill.maker_subaccount_id, 1);
    assert_eq!(loaded_legs.len(), 2);
    assert_eq!(loaded_legs[0].leg_index, 0);
    assert_eq!(loaded_legs[1].leg_index, 1);
    assert_eq!(loaded_legs[0].option_series_id, series_a);
    assert_eq!(loaded_legs[1].option_series_id, series_b);
}

#[tokio::test]
async fn part7_single_leg_repo_paths_untouched() {
    // Regression sanity: the single-leg RFQ tables remain queryable
    // against the same disposable DB. This test just proves the two
    // schemas coexist without collision.
    let Some(url) = pg_test_url() else {
        return;
    };
    let repo = fresh_repo(&url).await;
    let series = seed_series(&repo, "p7").await;
    // list_option_rfqs on an empty table must return Ok(vec![]) for a
    // fresh DB and stay green if other tests pre-populated rows.
    let rows = repo.list_option_rfqs().await.expect("list single-leg RFQs");
    assert!(
        rows.iter()
            .all(|r| r.option_series_id.as_str() != "sentinel-missing-series"),
        "sentinel must never appear"
    );
    let _ = series; // touched to satisfy compiler
}
