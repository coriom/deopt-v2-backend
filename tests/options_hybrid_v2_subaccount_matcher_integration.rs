//! OPTIONS-HYBRID-V2-PRODUCT-INTEGRATION-V1 — Package A.
//!
//! Locks in the canonical account identity `(owner, subaccount_id)`
//! through the Options DB matcher and read filters. Verifies:
//!
//!   * Same wallet acting through TWO subaccounts trades independently:
//!     buy from subaccount 1, sell from subaccount 2. The resulting
//!     fill row records `buyer_subaccount_id != seller_subaccount_id`
//!     even though `buyer == seller` (self-cross across subaccounts).
//!   * `OptionOrderFilter { account, subaccount_id: Some(N) }` returns
//!     only orders whose `subaccount_id == N`. Subaccount 1's orders
//!     never leak into subaccount 2's view.
//!   * `OptionFillFilter` scoped to `(account, subaccount_id=N)` is
//!     side-aware: for a same-wallet cross-subaccount fill, both
//!     subaccount views see the fill via their respective side, but
//!     `subaccount_id=99` (unused) sees no fills.
//!   * Two distinct wallets on the same series each see only their
//!     own orders and fills. Cross-wallet leakage rejected.
//!   * `OptionsConfig::validate_startup` panics closed when
//!     `require_persistence=true` and `persistence_enabled=false`.
//!     (Guards `NO_OPTIONS_MEMORY_FALLBACK_IN_PRODUCTION`.)
//!
//! These tests use in-memory `AppState`
//! (`OptionsConfig::enabled_in_memory_for_tests()`) and call the
//! service functions directly. They do NOT exercise the write-auth
//! HTTP boundary (that is covered by
//! `subaccounts_options_orders_history_tests.rs`), which is why the
//! `submit_option_order` call is invoked without EIP-712
//! authorization.
//!
//! Zero broadcast, zero real chain, zero external RPC.

use deopt_v2_backend::api::AppState;
use deopt_v2_backend::engine::EngineState;
use deopt_v2_backend::options::service::{
    create_option_series, list_option_fills, list_option_orders, submit_option_order,
    CreateOptionSeriesInput, SubmitOptionOrderInput,
};
use deopt_v2_backend::options::{
    OptionFillFilter, OptionOrderFilter, OptionOrderStatus, OptionsConfig,
};
use deopt_v2_backend::types::{now_ms, AccountId, Side, TimeInForce};

const VALID_SIGNATURE: &str = concat!(
    "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
);

fn state() -> AppState {
    AppState::with_options_config(
        EngineState::with_default_markets(),
        OptionsConfig::enabled_in_memory_for_tests(),
    )
}

fn wallet_a() -> AccountId {
    AccountId::new("0x00000000000000000000000000000000000000a1")
}

fn wallet_b() -> AccountId {
    AccountId::new("0x00000000000000000000000000000000000000b2")
}

fn future_expiry_sec() -> u64 {
    ((now_ms() / 1_000) + 60 * 60 * 24 * 30) as u64
}

fn series_input() -> CreateOptionSeriesInput {
    CreateOptionSeriesInput {
        underlying: "ETH".to_string(),
        base_asset: "ETH".to_string(),
        quote_asset: "USDC".to_string(),
        settlement_asset: "USDC".to_string(),
        expiry: future_expiry_sec(),
        strike_1e8: 300_000_000_000,
        is_call: true,
        contract_size_1e8: Some(100_000_000),
        onchain_product_id: None,
        onchain_series_id: None,
    }
}

async fn active_series_id(state: &AppState) -> String {
    create_option_series(state, series_input())
        .await
        .expect("series creation must succeed")
        .option_series_id
}

fn order_input(
    series_id: &str,
    account: AccountId,
    subaccount_id: u32,
    side: Side,
    price_1e8: u128,
    size_1e8: u128,
    client_order_id: &str,
    nonce: u64,
) -> SubmitOptionOrderInput {
    SubmitOptionOrderInput {
        option_series_id: series_id.to_string(),
        account,
        subaccount_id,
        side,
        price_1e8,
        size_1e8,
        time_in_force: TimeInForce::Gtc,
        post_only: false,
        client_order_id: Some(client_order_id.to_string()),
        nonce: Some(nonce),
        deadline_ms: Some(now_ms() + 60_000),
        signature: Some(VALID_SIGNATURE.to_string()),
        attached_tp_sl: None,
    }
}

// --------------------------------------------------------------------
// SAME-WALLET, TWO SUBACCOUNTS: THE MATCHER PERMITS THE CROSS AND THE
// FILL ROW RECORDS EACH SIDE'S SUBACCOUNT INDEPENDENTLY.
// --------------------------------------------------------------------

#[tokio::test]
async fn same_wallet_two_subaccounts_cross_matches_and_records_both_ids() {
    let state = state();
    let series = active_series_id(&state).await;
    let wallet = wallet_a();

    // Subaccount 1 places a resting sell at 1.00.
    let maker_out = submit_option_order(
        &state,
        order_input(
            &series,
            wallet.clone(),
            1,
            Side::Sell,
            1_000_000_000,
            100_000_000,
            "same-wallet-maker-sub1",
            10,
        ),
    )
    .await
    .expect("resting sell must accept");
    assert!(maker_out.fills.is_empty(), "resting order must not fill");
    assert_eq!(maker_out.order.subaccount_id, 1);

    // Subaccount 2 of the SAME wallet places an aggressive buy at
    // 1.00. The matcher must permit the cross (same-wallet policy
    // preserves existing self-trade behavior; NO_CROSS_SUBACCOUNT_
    // NETTING is a settlement invariant, not a matcher gate).
    let taker_out = submit_option_order(
        &state,
        order_input(
            &series,
            wallet.clone(),
            2,
            Side::Buy,
            1_000_000_000,
            100_000_000,
            "same-wallet-taker-sub2",
            11,
        ),
    )
    .await
    .expect("cross-subaccount taker must accept");
    assert_eq!(taker_out.order.subaccount_id, 2);
    assert_eq!(taker_out.fills.len(), 1, "one fill must be recorded");

    let fill = &taker_out.fills[0];
    assert_eq!(fill.buyer, wallet, "buyer identity");
    assert_eq!(fill.seller, wallet, "seller identity same wallet");
    assert_eq!(fill.buyer_subaccount_id, 2, "buyer sub is 2");
    assert_eq!(fill.seller_subaccount_id, 1, "seller sub is 1");
    assert_ne!(
        fill.buyer_subaccount_id, fill.seller_subaccount_id,
        "subaccount ids preserved distinctly on the fill row",
    );
    assert_eq!(fill.size_1e8, 100_000_000);
    assert_eq!(fill.price_1e8, 1_000_000_000);
}

// --------------------------------------------------------------------
// FILTER ISOLATION: SUBACCOUNT_ID=1 NEVER SEES SUBACCOUNT_ID=2 ORDERS.
// --------------------------------------------------------------------

#[tokio::test]
async fn order_list_scoped_by_subaccount_never_leaks_across_subaccounts() {
    let state = state();
    let series = active_series_id(&state).await;
    let wallet = wallet_a();

    // Place ONE order per subaccount for the SAME wallet.
    submit_option_order(
        &state,
        order_input(
            &series,
            wallet.clone(),
            1,
            Side::Buy,
            999_000_000,
            100_000_000,
            "wallet-a-sub1-buy",
            20,
        ),
    )
    .await
    .expect("sub1 buy must accept");
    submit_option_order(
        &state,
        order_input(
            &series,
            wallet.clone(),
            2,
            Side::Buy,
            998_000_000,
            100_000_000,
            "wallet-a-sub2-buy",
            21,
        ),
    )
    .await
    .expect("sub2 buy must accept");

    // Subaccount 1 view: only sees its own order.
    let sub1_orders = list_option_orders(
        &state,
        OptionOrderFilter {
            account: Some(wallet.clone()),
            subaccount_id: Some(1),
            ..OptionOrderFilter::default()
        },
    )
    .await
    .expect("sub1 list");
    assert_eq!(sub1_orders.len(), 1, "sub1 view returns exactly one order");
    assert_eq!(sub1_orders[0].subaccount_id, 1);
    assert_eq!(sub1_orders[0].price_1e8, 999_000_000);

    // Subaccount 2 view: only sees its own order.
    let sub2_orders = list_option_orders(
        &state,
        OptionOrderFilter {
            account: Some(wallet.clone()),
            subaccount_id: Some(2),
            ..OptionOrderFilter::default()
        },
    )
    .await
    .expect("sub2 list");
    assert_eq!(sub2_orders.len(), 1, "sub2 view returns exactly one order");
    assert_eq!(sub2_orders[0].subaccount_id, 2);
    assert_eq!(sub2_orders[0].price_1e8, 998_000_000);

    // Unused subaccount: empty view.
    let sub99_orders = list_option_orders(
        &state,
        OptionOrderFilter {
            account: Some(wallet.clone()),
            subaccount_id: Some(99),
            ..OptionOrderFilter::default()
        },
    )
    .await
    .expect("sub99 list");
    assert!(sub99_orders.is_empty(), "empty view for unused subaccount");

    // Admin/aggregate view (no subaccount_id filter): sees both.
    let admin_view = list_option_orders(
        &state,
        OptionOrderFilter {
            account: Some(wallet.clone()),
            subaccount_id: None,
            ..OptionOrderFilter::default()
        },
    )
    .await
    .expect("admin view");
    assert_eq!(
        admin_view.len(),
        2,
        "internal/admin view aggregates across subaccounts",
    );
}

// --------------------------------------------------------------------
// FILL FILTER SIDE-AWARENESS: BOTH SIDES OF A SAME-WALLET CROSS-SUB
// FILL ARE VISIBLE VIA THEIR RESPECTIVE SUBACCOUNT VIEWS.
// --------------------------------------------------------------------

#[tokio::test]
async fn fill_filter_side_aware_for_same_wallet_cross_subaccount() {
    let state = state();
    let series = active_series_id(&state).await;
    let wallet = wallet_a();

    // Resting sell from sub 1.
    submit_option_order(
        &state,
        order_input(
            &series,
            wallet.clone(),
            1,
            Side::Sell,
            1_000_000_000,
            100_000_000,
            "cross-sub-sell",
            30,
        ),
    )
    .await
    .expect("resting sell must accept");
    // Aggressive buy from sub 2 crosses.
    let taker_out = submit_option_order(
        &state,
        order_input(
            &series,
            wallet.clone(),
            2,
            Side::Buy,
            1_000_000_000,
            100_000_000,
            "cross-sub-buy",
            31,
        ),
    )
    .await
    .expect("taker must accept and fill");
    assert_eq!(taker_out.fills.len(), 1);

    // Subaccount 1 view (seller side): sees the fill via seller_subaccount_id.
    let sub1_fills = list_option_fills(
        &state,
        OptionFillFilter {
            account: Some(wallet.clone()),
            subaccount_id: Some(1),
            ..OptionFillFilter::default()
        },
    )
    .await
    .expect("sub1 fill list");
    assert_eq!(sub1_fills.len(), 1, "sub1 sees the fill via seller side");
    assert_eq!(sub1_fills[0].seller_subaccount_id, 1);
    assert_eq!(sub1_fills[0].buyer_subaccount_id, 2);

    // Subaccount 2 view (buyer side): sees the fill via buyer_subaccount_id.
    let sub2_fills = list_option_fills(
        &state,
        OptionFillFilter {
            account: Some(wallet.clone()),
            subaccount_id: Some(2),
            ..OptionFillFilter::default()
        },
    )
    .await
    .expect("sub2 fill list");
    assert_eq!(sub2_fills.len(), 1, "sub2 sees the fill via buyer side");
    assert_eq!(sub2_fills[0].buyer_subaccount_id, 2);
    assert_eq!(sub2_fills[0].seller_subaccount_id, 1);

    // Unused subaccount 99: sees no fill.
    let sub99_fills = list_option_fills(
        &state,
        OptionFillFilter {
            account: Some(wallet.clone()),
            subaccount_id: Some(99),
            ..OptionFillFilter::default()
        },
    )
    .await
    .expect("sub99 fill list");
    assert!(sub99_fills.is_empty(), "unused subaccount sees no fills");
}

// --------------------------------------------------------------------
// CROSS-WALLET ISOLATION: SUBACCOUNT SCOPING NEVER RETURNS ANOTHER
// WALLET'S DATA.
// --------------------------------------------------------------------

#[tokio::test]
async fn cross_wallet_isolation_holds_under_subaccount_scoping() {
    let state = state();
    let series = active_series_id(&state).await;

    // Wallet A places sub 1 buy.
    submit_option_order(
        &state,
        order_input(
            &series,
            wallet_a(),
            1,
            Side::Buy,
            900_000_000,
            100_000_000,
            "wallet-a-buy",
            40,
        ),
    )
    .await
    .expect("wallet A buy");
    // Wallet B places sub 1 buy at same price.
    submit_option_order(
        &state,
        order_input(
            &series,
            wallet_b(),
            1,
            Side::Buy,
            900_000_000,
            100_000_000,
            "wallet-b-buy",
            41,
        ),
    )
    .await
    .expect("wallet B buy");

    // Wallet A + subaccount 1 view returns only wallet A's order.
    let a_view = list_option_orders(
        &state,
        OptionOrderFilter {
            account: Some(wallet_a()),
            subaccount_id: Some(1),
            ..OptionOrderFilter::default()
        },
    )
    .await
    .expect("wallet A list");
    assert_eq!(a_view.len(), 1);
    assert_eq!(a_view[0].account, wallet_a());

    // Wallet B + subaccount 1 view returns only wallet B's order.
    let b_view = list_option_orders(
        &state,
        OptionOrderFilter {
            account: Some(wallet_b()),
            subaccount_id: Some(1),
            ..OptionOrderFilter::default()
        },
    )
    .await
    .expect("wallet B list");
    assert_eq!(b_view.len(), 1);
    assert_eq!(b_view[0].account, wallet_b());
}

// --------------------------------------------------------------------
// TWO DIFFERENT WALLETS TRADE — FILL PRESERVES BOTH WALLET AND
// SUBACCOUNT IDENTITY ON EACH SIDE.
// --------------------------------------------------------------------

#[tokio::test]
async fn two_wallets_trade_fill_preserves_both_wallet_and_subaccount() {
    let state = state();
    let series = active_series_id(&state).await;

    // Wallet A resting sell.
    submit_option_order(
        &state,
        order_input(
            &series,
            wallet_a(),
            1,
            Side::Sell,
            1_000_000_000,
            100_000_000,
            "wallet-a-sell",
            50,
        ),
    )
    .await
    .expect("A sell");
    // Wallet B aggressive buy.
    let taker = submit_option_order(
        &state,
        order_input(
            &series,
            wallet_b(),
            1,
            Side::Buy,
            1_000_000_000,
            100_000_000,
            "wallet-b-buy",
            51,
        ),
    )
    .await
    .expect("B buy");
    assert_eq!(taker.fills.len(), 1);
    let fill = &taker.fills[0];
    assert_eq!(fill.buyer, wallet_b());
    assert_eq!(fill.seller, wallet_a());
    assert_eq!(fill.buyer_subaccount_id, 1);
    assert_eq!(fill.seller_subaccount_id, 1);
    assert_ne!(fill.buyer, fill.seller, "distinct wallets on each side");
}

// --------------------------------------------------------------------
// FILTER RESPECTS ORDER STATUS FILTER × SUBACCOUNT COMBINATION.
// --------------------------------------------------------------------

#[tokio::test]
async fn filter_by_status_and_subaccount_combination() {
    let state = state();
    let series = active_series_id(&state).await;
    let wallet = wallet_a();

    // Sub 1 buy, remains open.
    submit_option_order(
        &state,
        order_input(
            &series,
            wallet.clone(),
            1,
            Side::Buy,
            900_000_000,
            100_000_000,
            "sub1-open",
            60,
        ),
    )
    .await
    .expect("sub1 open");
    // Sub 2 buy at same price, remains open.
    submit_option_order(
        &state,
        order_input(
            &series,
            wallet.clone(),
            2,
            Side::Buy,
            900_000_000,
            100_000_000,
            "sub2-open",
            61,
        ),
    )
    .await
    .expect("sub2 open");

    // Sub 1 view of only open orders returns 1 row.
    let sub1_open = list_option_orders(
        &state,
        OptionOrderFilter {
            account: Some(wallet.clone()),
            subaccount_id: Some(1),
            status: Some(OptionOrderStatus::Open),
            ..OptionOrderFilter::default()
        },
    )
    .await
    .expect("sub1 open list");
    assert_eq!(sub1_open.len(), 1);
    assert_eq!(sub1_open[0].subaccount_id, 1);
    assert_eq!(sub1_open[0].status, OptionOrderStatus::Open);
}

// --------------------------------------------------------------------
// STARTUP GUARD: OptionsConfig::validate_startup FAILS CLOSED WHEN
// require_persistence=true AND persistence_enabled=false. Guards
// NO_OPTIONS_MEMORY_FALLBACK_IN_PRODUCTION.
// --------------------------------------------------------------------

#[test]
fn options_config_startup_validation_fails_closed_in_production() {
    // Production-shaped config: enabled + require_persistence.
    // The `disabled()` constructor sets `require_persistence=true`;
    // we flip `enabled=true` to mimic the production posture.
    let mut cfg = OptionsConfig::disabled();
    cfg.enabled = true;
    assert!(cfg.enabled);
    assert!(cfg.require_persistence);

    // If persistence backend is unavailable, startup must reject.
    let err = cfg
        .validate_startup(false)
        .expect_err("must fail closed when persistence_enabled=false");
    let message = err.to_string();
    assert!(
        message.contains("persistence") || message.contains("options"),
        "error must mention persistence/options; got: {message}",
    );

    // With persistence enabled, startup passes.
    cfg.validate_startup(true)
        .expect("must accept when persistence_enabled=true");
}

// --------------------------------------------------------------------
// STARTUP GUARD: TEST CONFIG (enabled_in_memory_for_tests) IS ALLOWED
// TO OPERATE WITHOUT PERSISTENCE.
// --------------------------------------------------------------------

#[test]
fn options_config_test_mode_accepts_in_memory_operation() {
    let cfg = OptionsConfig::enabled_in_memory_for_tests();
    assert!(cfg.enabled);
    assert!(!cfg.require_persistence);
    cfg.validate_startup(false)
        .expect("test config must accept in-memory operation");
}
