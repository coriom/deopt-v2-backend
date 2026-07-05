//! Snapshot generators for the public WebSocket channels.
//!
//! Each generator calls the existing HTTP handler / service so the WS
//! surface and the HTTP surface stay perfectly in lock-step — there is
//! no second implementation that could drift.
//!
//! Generators never fabricate live data. They either return the same
//! `data` block the HTTP envelope would carry, or a SOURCE_UNAVAILABLE
//! sentinel that the dispatcher converts into a clean error frame.

use super::protocol::Channel;
use crate::api::AppState;
use axum::extract::{Query, State};
use serde_json::Value;

// ORDER-LIFECYCLE-OBSERVABILITY-V1 — REST handler imports for the
// `account.orders`, `account.fills`, `account.conditional_orders`
// snapshot generators. Calling the service functions in-process keeps
// the WS and REST surfaces in lock-step.
use crate::options::service::{
    list_option_fills as list_option_fills_service,
    list_option_orders as list_option_orders_service,
};
use crate::options::{OptionFillFilter, OptionOrderFilter};
use crate::types::AccountId;

#[derive(Debug)]
pub enum SnapshotError {
    /// The underlying source returned an upstream error or has no data.
    SourceUnavailable(String),
    /// Channel doesn't have a snapshot generator wired yet.
    NotImplemented,
}

/// Run the matching HTTP handler in-process and return its `data`
/// payload as `serde_json::Value`. Public channels only — private
/// channels go through `build_snapshot_for_address`.
pub async fn build_snapshot(state: &AppState, channel: Channel) -> Result<Value, SnapshotError> {
    match channel {
        Channel::TradingHealth => snapshot_trading_health(state).await,
        Channel::OptionsProducts => snapshot_options_products(state).await,
        Channel::Leaderboard => snapshot_leaderboard(state).await,
        Channel::AccountPositions
        | Channel::AccountPortfolio
        | Channel::AccountBalances
        | Channel::AccountOrders
        | Channel::AccountFills
        | Channel::AccountConditionalOrders
        | Channel::AccountHistory
        | Channel::AccountIntentStatus
        | Channel::AccountSettlements
        | Channel::AccountLiquidations
        | Channel::AccountPerpOrders
        | Channel::AccountPerpFills
        | Channel::AccountPerpPositions
        | Channel::AccountPerpFunding
        | Channel::AccountRfqs => Err(SnapshotError::NotImplemented),
    }
}

/// Address-scoped snapshot for `account.*` channels. Reuses the
/// existing HTTP handlers so the REST and WS surfaces never drift.
///
/// V1 wires four channels with real sources (`positions`, `portfolio`,
/// `balances`, `history`) and surfaces honest empty arrays for the
/// remaining five (`orders`, `fills`, `intent_status`, `settlements`,
/// `liquidations`). No private data is fabricated; no global data is
/// returned inside an account channel.
pub async fn build_snapshot_for_address(
    state: &AppState,
    channel: Channel,
    address: &str,
) -> Result<Value, SnapshotError> {
    match channel {
        Channel::AccountPositions => snapshot_account_positions(state, address).await,
        Channel::AccountPortfolio => snapshot_account_portfolio(state, address).await,
        Channel::AccountBalances => snapshot_account_balances(state, address).await,
        Channel::AccountHistory => snapshot_account_history(state, address).await,
        // ORDER-LIFECYCLE-OBSERVABILITY-V1 — three account channels now
        // call their REST service in-process, returning the same DB-backed
        // collections the HTTP surface returns.
        Channel::AccountOrders => snapshot_account_orders(state, address).await,
        Channel::AccountFills => snapshot_account_fills(state, address).await,
        Channel::AccountConditionalOrders => {
            snapshot_account_conditional_orders(state, address).await
        }
        Channel::AccountIntentStatus => Ok(empty_account_collection("intents", address)),
        Channel::AccountSettlements => Ok(empty_account_collection("settlements", address)),
        Channel::AccountLiquidations => Ok(empty_account_collection("liquidations", address)),
        // PERPS-PERSISTENCE-HISTORY-LIFECYCLE-V1 — snapshot the
        // account's in-memory perp orders / fills / positions. The
        // REST endpoints exposed under `/accounts/:address/perps/*`
        // return the same shape.
        Channel::AccountPerpOrders => snapshot_account_perp_orders(state, address),
        Channel::AccountPerpFills => snapshot_account_perp_fills(state, address),
        Channel::AccountPerpPositions => Ok(empty_account_collection("perp_positions", address)),
        // PERPS-FUNDING-V1 — honest empty snapshot; the REST endpoint
        // under `/accounts/:address/perps/funding` returns the
        // durable/in-memory ledger.
        Channel::AccountPerpFunding => Ok(empty_account_collection("perp_funding", address)),
        // OPTIONS-RFQ-LIFECYCLE-WS-V1 — honest empty snapshot; the
        // frontend calls `GET /options/rfqs`, `GET /options/rfqs/{id}/quotes`,
        // and `GET /options/rfq-fills?account=…` on each resync bump so
        // this channel is used purely for delta notifications.
        Channel::AccountRfqs => Ok(empty_account_collection("rfqs", address)),
        // Public channels never route through this path.
        Channel::TradingHealth | Channel::OptionsProducts | Channel::Leaderboard => {
            Err(SnapshotError::NotImplemented)
        }
    }
}

async fn snapshot_account_orders(state: &AppState, address: &str) -> Result<Value, SnapshotError> {
    let filter = OptionOrderFilter {
        option_series_id: None,
        account: Some(AccountId::new(address.to_lowercase())),
        status: None,
        side: None,
    };
    match list_option_orders_service(state, filter).await {
        Ok(orders) => Ok(serde_json::json!({
            "address": address,
            "source": "option_orders",
            "orders": orders,
        })),
        Err(err) => Err(SnapshotError::SourceUnavailable(format!(
            "account.orders upstream error: {err}"
        ))),
    }
}

async fn snapshot_account_fills(state: &AppState, address: &str) -> Result<Value, SnapshotError> {
    let filter = OptionFillFilter {
        option_series_id: None,
        account: Some(AccountId::new(address.to_lowercase())),
        order_id: None,
    };
    match list_option_fills_service(state, filter).await {
        Ok(fills) => Ok(serde_json::json!({
            "address": address,
            "source": "option_fills",
            "fills": fills,
        })),
        Err(err) => Err(SnapshotError::SourceUnavailable(format!(
            "account.fills upstream error: {err}"
        ))),
    }
}

async fn snapshot_account_conditional_orders(
    state: &AppState,
    address: &str,
) -> Result<Value, SnapshotError> {
    use crate::options::conditional_orders as cond;
    let filter = cond::ConditionalOrderFilter {
        account: Some(AccountId::new(address.to_lowercase())),
        ..Default::default()
    };
    match cond::list_conditional_orders(state, filter).await {
        Ok(orders) => Ok(serde_json::json!({
            "address": address,
            "source": "options_conditional_orders",
            "conditional_orders": orders,
        })),
        Err(err) => Err(SnapshotError::SourceUnavailable(format!(
            "account.conditional_orders upstream error: {err}"
        ))),
    }
}

fn empty_account_collection(key: &'static str, address: &str) -> Value {
    serde_json::json!({
        "address": address,
        "source": "empty",
        key: [],
    })
}

// PERPS-PERSISTENCE-HISTORY-LIFECYCLE-V1 — read the account's perp
// orders/fills from the in-memory store and hand the same shape back
// to the WS snapshot path that REST returns.
fn snapshot_account_perp_orders(state: &AppState, address: &str) -> Result<Value, SnapshotError> {
    let account = AccountId::new(address.to_string());
    let store = state
        .perp_order_store
        .lock()
        .map_err(|_| {
            SnapshotError::SourceUnavailable("perp_order_store lock poisoned".to_string())
        })?
        .clone();
    let response = crate::perps::service::list_perp_orders_for_account_view(
        &state.perps_read_config,
        &store,
        &account,
    );
    serde_json::to_value(response).map_err(|e| SnapshotError::SourceUnavailable(e.to_string()))
}

fn snapshot_account_perp_fills(state: &AppState, address: &str) -> Result<Value, SnapshotError> {
    let account = AccountId::new(address.to_string());
    let store = state
        .perp_order_store
        .lock()
        .map_err(|_| {
            SnapshotError::SourceUnavailable("perp_order_store lock poisoned".to_string())
        })?
        .clone();
    let response = crate::perps::service::list_perp_fills_for_account_view(
        &state.perps_read_config,
        &store,
        &account,
    );
    serde_json::to_value(response).map_err(|e| SnapshotError::SourceUnavailable(e.to_string()))
}

async fn snapshot_account_positions(
    state: &AppState,
    address: &str,
) -> Result<Value, SnapshotError> {
    match crate::api::trading::account_positions(
        State(state.clone()),
        axum::extract::Path(address.to_string()),
    )
    .await
    {
        Ok(env) => serde_json::to_value(&env.0.data).map_err(|e| {
            SnapshotError::SourceUnavailable(format!("account.positions serialization failed: {e}"))
        }),
        Err(_) => Err(SnapshotError::SourceUnavailable(
            "account.positions upstream error".to_string(),
        )),
    }
}

async fn snapshot_account_portfolio(
    state: &AppState,
    address: &str,
) -> Result<Value, SnapshotError> {
    match crate::api::trading::account_portfolio(
        State(state.clone()),
        axum::extract::Path(address.to_string()),
    )
    .await
    {
        Ok(env) => serde_json::to_value(&env.0.data).map_err(|e| {
            SnapshotError::SourceUnavailable(format!("account.portfolio serialization failed: {e}"))
        }),
        Err(_) => Err(SnapshotError::SourceUnavailable(
            "account.portfolio upstream error".to_string(),
        )),
    }
}

async fn snapshot_account_balances(
    state: &AppState,
    address: &str,
) -> Result<Value, SnapshotError> {
    match crate::api::trading::account_balances(
        State(state.clone()),
        axum::extract::Path(address.to_string()),
    )
    .await
    {
        Ok(env) => serde_json::to_value(&env.0.data).map_err(|e| {
            SnapshotError::SourceUnavailable(format!("account.balances serialization failed: {e}"))
        }),
        Err(_) => Err(SnapshotError::SourceUnavailable(
            "account.balances upstream error".to_string(),
        )),
    }
}

async fn snapshot_account_history(state: &AppState, address: &str) -> Result<Value, SnapshotError> {
    let q = crate::api::trading::HistoryV2Query {
        tab: Some("trades".to_string()),
        range: Some("last_month".to_string()),
        page: Some(1),
        page_size: Some(100),
    };
    match crate::api::trading::account_history_v2(
        State(state.clone()),
        axum::extract::Path(address.to_string()),
        Query(q),
    )
    .await
    {
        Ok(env) => serde_json::to_value(&env.0.data).map_err(|e| {
            SnapshotError::SourceUnavailable(format!("account.history serialization failed: {e}"))
        }),
        Err(_) => Err(SnapshotError::SourceUnavailable(
            "account.history upstream error".to_string(),
        )),
    }
}

async fn snapshot_trading_health(state: &AppState) -> Result<Value, SnapshotError> {
    let response = crate::api::trading::trading_health(State(state.clone())).await;
    serde_json::to_value(&response.0.data).map_err(|e| {
        SnapshotError::SourceUnavailable(format!("trading.health serialization failed: {e}"))
    })
}

async fn snapshot_options_products(state: &AppState) -> Result<Value, SnapshotError> {
    let q = crate::api::trading::ListProductsQuery::default();
    match crate::api::trading::list_products(State(state.clone()), Query(q)).await {
        Ok(env) => serde_json::to_value(&env.0.data).map_err(|e| {
            SnapshotError::SourceUnavailable(format!("options.products serialization failed: {e}"))
        }),
        Err(_) => Err(SnapshotError::SourceUnavailable(
            "options.products upstream error".to_string(),
        )),
    }
}

async fn snapshot_leaderboard(state: &AppState) -> Result<Value, SnapshotError> {
    let q = crate::api::trading::LeaderboardQuery {
        range: Some("last_month".to_string()),
        page: Some(1),
        page_size: Some(100),
    };
    match crate::api::trading::leaderboard(State(state.clone()), Query(q)).await {
        Ok(env) => serde_json::to_value(&env.0.data).map_err(|e| {
            SnapshotError::SourceUnavailable(format!("leaderboard serialization failed: {e}"))
        }),
        Err(_) => Err(SnapshotError::SourceUnavailable(
            "leaderboard upstream error".to_string(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::EngineState;

    fn test_state() -> AppState {
        let mut s = AppState::new(EngineState::new(Vec::new()));
        s.chain_id = 31337;
        s.network_name = "anvil".to_string();
        s.options_config.enabled = true;
        s
    }

    #[tokio::test]
    async fn trading_health_snapshot_is_serialisable() {
        let v = build_snapshot(&test_state(), Channel::TradingHealth)
            .await
            .expect("ok");
        assert!(v.is_object());
        assert!(v.get("overall_status").is_some());
        assert_eq!(v["chain_id"], 31337);
    }

    #[tokio::test]
    async fn options_products_snapshot_is_serialisable() {
        let v = build_snapshot(&test_state(), Channel::OptionsProducts)
            .await
            .expect("ok");
        assert!(v.is_object());
        assert!(v.get("products").is_some());
    }

    #[tokio::test]
    async fn leaderboard_snapshot_is_serialisable() {
        let v = build_snapshot(&test_state(), Channel::Leaderboard)
            .await
            .expect("ok");
        assert!(v.is_object());
        assert!(v.get("items").is_some());
    }

    #[tokio::test]
    async fn account_channels_are_not_implemented_via_public_build_snapshot() {
        // The public-channel `build_snapshot` always rejects
        // `account.*` — those go through `build_snapshot_for_address`.
        for c in [
            Channel::AccountPositions,
            Channel::AccountPortfolio,
            Channel::AccountBalances,
            Channel::AccountOrders,
            Channel::AccountFills,
            Channel::AccountConditionalOrders,
            Channel::AccountHistory,
            Channel::AccountIntentStatus,
            Channel::AccountSettlements,
            Channel::AccountLiquidations,
        ] {
            let r = build_snapshot(&test_state(), c).await;
            assert!(matches!(r, Err(SnapshotError::NotImplemented)));
        }
    }

    const TEST_ADDR: &str = "0x1234567890abcdef1234567890abcdef12345678";

    #[tokio::test]
    async fn account_positions_snapshot_is_serialisable_for_an_address() {
        let v = build_snapshot_for_address(&test_state(), Channel::AccountPositions, TEST_ADDR)
            .await
            .expect("positions ok");
        assert!(v.is_object());
        assert_eq!(v["address"], TEST_ADDR);
    }

    #[tokio::test]
    async fn account_portfolio_snapshot_is_serialisable_for_an_address() {
        let v = build_snapshot_for_address(&test_state(), Channel::AccountPortfolio, TEST_ADDR)
            .await
            .expect("portfolio ok");
        assert!(v.is_object());
    }

    #[tokio::test]
    async fn account_balances_snapshot_is_serialisable_for_an_address() {
        let v = build_snapshot_for_address(&test_state(), Channel::AccountBalances, TEST_ADDR)
            .await
            .expect("balances ok");
        assert!(v.is_object());
    }

    #[tokio::test]
    async fn account_history_snapshot_is_serialisable_for_an_address() {
        let v = build_snapshot_for_address(&test_state(), Channel::AccountHistory, TEST_ADDR)
            .await
            .expect("history ok");
        assert!(v.is_object());
        assert_eq!(v["tab"], "trades");
        assert_eq!(v["address"], TEST_ADDR);
    }

    #[tokio::test]
    async fn deferred_account_channels_return_honest_empty_arrays() {
        // ORDER-LIFECYCLE-OBSERVABILITY-V1 moved orders/fills/conditional_orders
        // off the empty-sentinel path; they now have real DB-backed sources
        // (see the dedicated tests below). The three remaining channels
        // still surface honest empty arrays until their sources land.
        for (channel, key) in [
            (Channel::AccountIntentStatus, "intents"),
            (Channel::AccountSettlements, "settlements"),
            (Channel::AccountLiquidations, "liquidations"),
        ] {
            let v = build_snapshot_for_address(&test_state(), channel, TEST_ADDR)
                .await
                .expect("empty ok");
            assert_eq!(v["address"], TEST_ADDR);
            assert_eq!(v["source"], "empty");
            assert_eq!(v[key].as_array().expect("array").len(), 0);
        }
    }

    #[tokio::test]
    async fn account_orders_snapshot_is_real_db_backed_collection() {
        let v = build_snapshot_for_address(&test_state(), Channel::AccountOrders, TEST_ADDR)
            .await
            .expect("orders ok");
        assert_eq!(v["address"], TEST_ADDR);
        assert_eq!(v["source"], "option_orders");
        assert!(v["orders"].is_array());
    }

    #[tokio::test]
    async fn account_fills_snapshot_is_real_db_backed_collection() {
        let v = build_snapshot_for_address(&test_state(), Channel::AccountFills, TEST_ADDR)
            .await
            .expect("fills ok");
        assert_eq!(v["address"], TEST_ADDR);
        assert_eq!(v["source"], "option_fills");
        assert!(v["fills"].is_array());
    }

    #[tokio::test]
    async fn account_conditional_orders_snapshot_is_real_db_backed_collection() {
        let v =
            build_snapshot_for_address(&test_state(), Channel::AccountConditionalOrders, TEST_ADDR)
                .await
                .expect("conditional ok");
        assert_eq!(v["address"], TEST_ADDR);
        assert_eq!(v["source"], "options_conditional_orders");
        assert!(v["conditional_orders"].is_array());
    }

    #[tokio::test]
    async fn public_channels_are_rejected_by_address_scoped_path() {
        for c in [
            Channel::TradingHealth,
            Channel::OptionsProducts,
            Channel::Leaderboard,
        ] {
            let r = build_snapshot_for_address(&test_state(), c, TEST_ADDR).await;
            assert!(matches!(r, Err(SnapshotError::NotImplemented)));
        }
    }
}
