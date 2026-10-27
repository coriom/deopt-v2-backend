//! Deterministic races for the internal, broadcast-disabled perps ledger.
use async_trait::async_trait;
use deopt_v2_backend::api::AppState;
use deopt_v2_backend::engine::EngineState;
use deopt_v2_backend::error::Result;
use deopt_v2_backend::perps::{
    cancel_perp_order_via_state,
    price_reader::{PerpOraclePriceReader, RawPriceRead},
    submit_perp_order_via_state, PerpOrderSide, PerpTimeInForce, PerpsReadConfig,
    SubmitPerpOrderInput,
};
use deopt_v2_backend::types::{now_ms, AccountId};
use std::sync::Arc;
use tokio::sync::{Barrier, Notify};
use tokio::time::{timeout, Duration};

const ONE: u128 = 100_000_000;

fn state() -> Arc<AppState> {
    let mut state = AppState::new(EngineState::with_default_markets());
    state.perps_read_config = PerpsReadConfig::enabled_in_memory_for_tests();
    Arc::new(state)
}

fn account(id: u64) -> AccountId {
    AccountId::new(format!("0x{id:040x}"))
}

fn order(id: u64, side: PerpOrderSide, client_id: &str) -> SubmitPerpOrderInput {
    SubmitPerpOrderInput {
        account: account(id),
        subaccount_id: 1,
        market_id: "ETH-PERP".to_owned(),
        side,
        price_1e8: 3_000 * ONE,
        size_1e8: ONE,
        time_in_force: PerpTimeInForce::Gtc,
        post_only: false,
        reduce_only: false,
        isolated_margin_1e8: 300 * ONE,
        client_order_id: Some(client_id.to_owned()),
        max_execution_price_1e8: 0,
        min_execution_price_1e8: 0,
    }
}

#[derive(Clone)]
struct Oracle {
    entered: Option<Arc<Barrier>>,
    release: Option<Arc<Notify>>,
    stale: bool,
}

#[async_trait]
impl PerpOraclePriceReader for Oracle {
    async fn read_price(
        &self,
        _: &deopt_v2_backend::perps::PerpsReadMarket,
    ) -> Result<RawPriceRead> {
        if let Some(entered) = &self.entered {
            entered.wait().await;
        }
        if let Some(release) = &self.release {
            release.notified().await;
        }
        Ok(RawPriceRead {
            price_1e8: 3_000 * ONE,
            updated_at_sec: if self.stale {
                1
            } else {
                (now_ms() / 1000) as u64
            },
            ok: true,
        })
    }
}

fn fast_oracle() -> Oracle {
    Oracle {
        entered: None,
        release: None,
        stale: false,
    }
}

#[tokio::test]
async fn slow_oracle_does_not_block_cancel_or_match_against_cancelled_liquidity() {
    let state = state();
    let maker = submit_perp_order_via_state(
        &state,
        &fast_oracle(),
        order(1, PerpOrderSide::Sell, "maker"),
    )
    .await
    .unwrap();
    let entered = Arc::new(Barrier::new(2));
    let release = Arc::new(Notify::new());
    let delayed = Oracle {
        entered: Some(entered.clone()),
        release: Some(release.clone()),
        stale: false,
    };
    let handle = tokio::spawn({
        let state = state.clone();
        async move {
            submit_perp_order_via_state(&state, &delayed, order(2, PerpOrderSide::Buy, "taker"))
                .await
        }
    });
    timeout(Duration::from_secs(2), entered.wait())
        .await
        .unwrap();
    timeout(
        Duration::from_millis(200),
        cancel_perp_order_via_state(&state, maker.order.id, &account(1)),
    )
    .await
    .expect("cancellation blocked by oracle lock")
    .unwrap();
    release.notify_one();
    let outcome = timeout(Duration::from_secs(2), handle)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(outcome.fills.is_empty());
    assert_eq!(
        state
            .perp_order_store
            .lock()
            .unwrap()
            .list_fills_for_account(&account(1))
            .len(),
        0
    );
}

#[tokio::test]
async fn concurrent_duplicate_client_id_commits_once() {
    let state = state();
    let mut handles = Vec::new();
    for _ in 0..24 {
        let state = state.clone();
        handles.push(tokio::spawn(async move {
            submit_perp_order_via_state(
                &state,
                &fast_oracle(),
                order(7, PerpOrderSide::Buy, "same"),
            )
            .await
            .is_ok()
        }));
    }
    let mut successes = 0;
    for handle in handles {
        if handle.await.unwrap() {
            successes += 1;
        }
    }
    assert_eq!(successes, 1);
    assert_eq!(
        state
            .perp_order_store
            .lock()
            .unwrap()
            .list_orders_for_account(&account(7))
            .len(),
        1
    );
}

#[tokio::test]
async fn quote_expiring_during_oracle_wait_cannot_open() {
    let state = state();
    let err = submit_perp_order_via_state(
        &state,
        &Oracle {
            stale: true,
            ..fast_oracle()
        },
        order(8, PerpOrderSide::Buy, "stale"),
    )
    .await
    .unwrap_err();
    assert!(err.to_string().contains("stale"));
    assert!(state
        .perp_order_store
        .lock()
        .unwrap()
        .list_orders_for_account(&account(8))
        .is_empty());
}

#[tokio::test]
async fn ordinary_opposite_order_still_requires_fresh_oracle() {
    let state = state();
    submit_perp_order_via_state(
        &state,
        &fast_oracle(),
        order(41, PerpOrderSide::Sell, "open-short-maker"),
    )
    .await
    .unwrap();
    submit_perp_order_via_state(
        &state,
        &fast_oracle(),
        order(42, PerpOrderSide::Buy, "open-long-taker"),
    )
    .await
    .unwrap();
    let before = state
        .perp_order_store
        .lock()
        .unwrap()
        .list_orders_for_account(&account(42))
        .len();
    let err = submit_perp_order_via_state(
        &state,
        &Oracle {
            stale: true,
            ..fast_oracle()
        },
        order(42, PerpOrderSide::Sell, "ordinary-opposite"),
    )
    .await
    .unwrap_err();
    assert!(err.to_string().contains("stale"));
    assert_eq!(
        state
            .perp_order_store
            .lock()
            .unwrap()
            .list_orders_for_account(&account(42))
            .len(),
        before
    );
}

#[tokio::test]
async fn oversized_reduce_only_order_is_rejected_before_resting() {
    let state = state();
    submit_perp_order_via_state(
        &state,
        &fast_oracle(),
        order(43, PerpOrderSide::Sell, "reduce-maker"),
    )
    .await
    .unwrap();
    submit_perp_order_via_state(
        &state,
        &fast_oracle(),
        order(44, PerpOrderSide::Buy, "reduce-long"),
    )
    .await
    .unwrap();
    let mut oversized = order(44, PerpOrderSide::Sell, "oversized-reduce");
    oversized.size_1e8 = 2 * ONE;
    oversized.reduce_only = true;
    assert!(
        submit_perp_order_via_state(&state, &fast_oracle(), oversized)
            .await
            .is_err()
    );
    assert_eq!(
        state
            .perp_order_store
            .lock()
            .unwrap()
            .list_orders_for_account(&account(44))
            .len(),
        1
    );
}

#[tokio::test]
async fn concurrent_accounts_cannot_overfill_resting_liquidity() {
    let state = state();
    for i in 0..16 {
        submit_perp_order_via_state(
            &state,
            &fast_oracle(),
            order(100 + i, PerpOrderSide::Sell, &format!("maker-{i}")),
        )
        .await
        .unwrap();
    }
    let mut handles = Vec::new();
    for i in 0..24 {
        let state = state.clone();
        handles.push(tokio::spawn(async move {
            submit_perp_order_via_state(
                &state,
                &fast_oracle(),
                order(200 + i, PerpOrderSide::Buy, &format!("taker-{i}")),
            )
            .await
            .unwrap()
            .fills
            .len()
        }));
    }
    let mut fills = 0;
    for handle in handles {
        fills += handle.await.unwrap();
    }
    assert_eq!(fills, 16);
    let store = state.perp_order_store.lock().unwrap();
    for i in 0..16 {
        assert_eq!(store.list_fills_for_account(&account(100 + i)).len(), 1);
    }
}

#[tokio::test]
async fn late_self_trade_rejection_leaves_earlier_maker_unchanged() {
    let state = state();
    let first = submit_perp_order_via_state(
        &state,
        &fast_oracle(),
        order(9, PerpOrderSide::Sell, "third-party"),
    )
    .await
    .unwrap();
    let mut self_order = order(10, PerpOrderSide::Sell, "self-maker");
    self_order.price_1e8 += ONE;
    let self_maker = submit_perp_order_via_state(&state, &fast_oracle(), self_order)
        .await
        .unwrap();
    let mut taker = order(10, PerpOrderSide::Buy, "self-taker");
    taker.price_1e8 += ONE;
    taker.size_1e8 = 2 * ONE;
    taker.isolated_margin_1e8 = 600 * ONE;
    let err = submit_perp_order_via_state(&state, &fast_oracle(), taker)
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        deopt_v2_backend::error::BackendError::PerpSelfTrade
    ));
    let store = state.perp_order_store.lock().unwrap();
    assert_eq!(store.get(first.order.id).unwrap().remaining_size_1e8, ONE);
    assert_eq!(
        store.get(self_maker.order.id).unwrap().remaining_size_1e8,
        ONE
    );
    assert!(store.list_fills_for_account(&account(9)).is_empty());
}
