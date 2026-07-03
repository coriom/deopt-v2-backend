//! PERPS-MINIMAL-MARKET-AND-PRICE-V1 — read-only Perps service layer.
//!
//! Orchestrates the market and oracle readers behind a small set of
//! service functions used by the HTTP handlers and tests. Never
//! fabricates values on RPC failure: an oracle read that returns
//! `ok=false` surfaces as `PerpsPriceUnavailable`; an RPC transport
//! failure surfaces as `PerpsPriceUnavailable` with the reason.

use crate::error::{BackendError, Result};
use crate::perps::config::{PerpsReadConfig, PerpsReadMarket};
use crate::perps::margin;
use crate::perps::market_reader::PerpMarketRegistryReader;
use crate::perps::order_store::PerpOrderStore;
use crate::perps::orders::{PerpFill, PerpOrder, PerpOrderSide};
use crate::perps::positions::{PerpPosition, PerpPositionsStore};
use crate::perps::price_reader::{PerpOraclePriceReader, RawPriceRead};
use crate::perps::types::{
    PerpMarket, PerpMarketSource, PerpMarketStatus, PerpPriceSnapshot, PerpPriceSource,
};
use crate::types::{now_ms, AccountId, TimestampMs};
use serde::{Deserialize, Serialize};

/// Wire envelope for `GET /perps/markets`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PerpMarketListing {
    pub markets: Vec<PerpMarket>,
    pub chain_id: u64,
    /// Always `false` for this milestone.
    pub trading_enabled: bool,
}

/// Wire envelope for `GET /perps/markets/:market_id/price`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PerpMarketPriceResponse {
    #[serde(flatten)]
    pub snapshot: PerpPriceSnapshot,
    pub chain_id: u64,
}

pub fn ensure_read_enabled(cfg: &PerpsReadConfig) -> Result<()> {
    if !cfg.enabled {
        return Err(BackendError::PerpsReadDisabled);
    }
    Ok(())
}

/// Validate that this backend was configured for the same chain the
/// caller is asking about. Refuses mainnet chain ids explicitly.
pub fn ensure_chain_id_matches(cfg: &PerpsReadConfig, requested_chain_id: u64) -> Result<()> {
    if requested_chain_id == 1 || requested_chain_id == 8453 {
        return Err(BackendError::PerpsChainIdMismatch {
            expected: cfg.chain_id,
            got: requested_chain_id,
        });
    }
    if requested_chain_id != cfg.chain_id {
        return Err(BackendError::PerpsChainIdMismatch {
            expected: cfg.chain_id,
            got: requested_chain_id,
        });
    }
    Ok(())
}

/// List all configured Perps markets with their on-chain liveness
/// status.
pub async fn list_perp_markets<R: PerpMarketRegistryReader + ?Sized>(
    cfg: &PerpsReadConfig,
    reader: &R,
) -> Result<PerpMarketListing> {
    ensure_read_enabled(cfg)?;
    let mut markets = Vec::with_capacity(cfg.markets.len());
    for row in &cfg.markets {
        // A per-row read failure MUST NOT poison the list — we surface
        // the row as `PerpMarketStatus::Unknown` so the frontend can
        // still show that the market exists in the registry, with a
        // clearly non-tradeable status. Fabricating "ReadOnly" would
        // violate the "no fake data" rule.
        let status = reader
            .read_status(row)
            .await
            .unwrap_or(PerpMarketStatus::Unknown);
        markets.push(mint_perp_market(cfg, row, status));
    }
    Ok(PerpMarketListing {
        markets,
        chain_id: cfg.chain_id,
        trading_enabled: false,
    })
}

/// Fetch one market by symbol (e.g. `"ETH-PERP"`).
pub async fn get_perp_market<R: PerpMarketRegistryReader + ?Sized>(
    cfg: &PerpsReadConfig,
    reader: &R,
    symbol: &str,
) -> Result<PerpMarket> {
    ensure_read_enabled(cfg)?;
    let row = cfg
        .market_by_symbol(symbol)
        .ok_or_else(|| BackendError::PerpsMarketNotFound(symbol.to_string()))?;
    let status = reader
        .read_status(row)
        .await
        .unwrap_or(PerpMarketStatus::Unknown);
    Ok(mint_perp_market(cfg, row, status))
}

/// Fetch the latest price snapshot for one market by symbol.
pub async fn get_perp_market_price<P: PerpOraclePriceReader + ?Sized>(
    cfg: &PerpsReadConfig,
    reader: &P,
    symbol: &str,
) -> Result<PerpMarketPriceResponse> {
    ensure_read_enabled(cfg)?;
    let row = cfg
        .market_by_symbol(symbol)
        .ok_or_else(|| BackendError::PerpsMarketNotFound(symbol.to_string()))?;
    let read = reader.read_price(row).await?;
    let snapshot = build_price_snapshot(cfg, row, read, now_ms())?;
    Ok(PerpMarketPriceResponse {
        snapshot,
        chain_id: cfg.chain_id,
    })
}

fn mint_perp_market(
    cfg: &PerpsReadConfig,
    row: &PerpsReadMarket,
    status: PerpMarketStatus,
) -> PerpMarket {
    PerpMarket {
        market_id: row.symbol.clone(),
        onchain_market_id: row.onchain_market_id.to_string(),
        base_asset: row.base_asset_label.clone(),
        quote_asset: row.quote_asset_label.clone(),
        status,
        chain_id: cfg.chain_id,
        source: PerpMarketSource::OnchainRegistry,
        trading_enabled: false,
    }
}

fn build_price_snapshot(
    cfg: &PerpsReadConfig,
    row: &PerpsReadMarket,
    read: RawPriceRead,
    now: TimestampMs,
) -> Result<PerpPriceSnapshot> {
    // The router's `ok` flag is the ground truth: `false` means feed
    // paused, stale, or a source read failed. We refuse to invent a
    // number; the caller sees 503 `PerpsPriceUnavailable`.
    if !read.ok {
        return Err(BackendError::PerpsPriceUnavailable(format!(
            "OracleRouter reported ok=false for {}",
            row.symbol
        )));
    }
    // A zero price with ok=true would be pathological, but we still
    // refuse to surface it — a zero mark on a live perp is worse than
    // a 503.
    if read.price_1e8 == 0 {
        return Err(BackendError::PerpsPriceUnavailable(format!(
            "OracleRouter returned zero price for {}",
            row.symbol
        )));
    }
    let oracle_timestamp_ms = read.updated_at_ms();
    let stale_after_ms = (cfg.stale_after_sec as i64).saturating_mul(1000);
    let stale = if oracle_timestamp_ms == 0 {
        true
    } else {
        now.saturating_sub(oracle_timestamp_ms) > stale_after_ms
    };
    Ok(PerpPriceSnapshot {
        market_id: row.symbol.clone(),
        // V1: mark == index. See module-level doc for the rationale
        // (funding-premium computation deferred).
        index_price_1e8: read.price_1e8.to_string(),
        mark_price_1e8: read.price_1e8.to_string(),
        oracle_timestamp_ms,
        source: PerpPriceSource::OracleRouter,
        stale,
        trading_enabled: false,
    })
}

// =====================================================================
// PERPS-ISOLATED-MARGIN-POSITION-ENGINE-V1 — positions view + list.
// =====================================================================

/// One row of the account-scoped positions response. Combines the
/// persistent position with a snapshot of the current mark and the
/// derived risk fields.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PerpPositionView {
    pub id: String,
    pub account: String,
    pub market_id: String,
    pub side: String,
    pub size_1e8: String,
    pub entry_price_1e8: String,
    pub margin_1e8: String,
    pub realized_pnl_1e8: String,
    pub status: String,
    /// Present when the oracle read was fresh; `null` when
    /// unavailable/stale so the frontend renders honestly.
    pub mark_price_1e8: Option<String>,
    pub notional_1e8: Option<String>,
    pub unrealized_pnl_1e8: Option<String>,
    pub initial_margin_requirement_1e8: String,
    pub maintenance_margin_requirement_1e8: Option<String>,
    pub margin_ratio_bps: Option<String>,
    pub estimated_liquidation_price_1e8: Option<String>,
    pub opened_at_ms: TimestampMs,
    pub updated_at_ms: TimestampMs,
    pub closed_at_ms: Option<TimestampMs>,
    /// True when the mark price used for the risk fields is stale
    /// (older than `PERPS_STALE_AFTER_SEC`) or absent altogether.
    pub price_stale: bool,
    pub trading_enabled: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PerpPositionListResponse {
    pub positions: Vec<PerpPositionView>,
    pub chain_id: u64,
    pub trading_enabled: bool,
}

fn i128_to_wire(v: i128) -> String {
    v.to_string()
}

/// Build a full view for one position given the current mark snapshot
/// (which may be `None` when unavailable/stale).
pub fn build_perp_position_view(
    cfg: &PerpsReadConfig,
    market: &PerpsReadMarket,
    position: &PerpPosition,
    mark_price_1e8: Option<u128>,
    price_stale: bool,
) -> PerpPositionView {
    let initial_margin = margin::initial_margin_requirement_1e8(
        position.size_1e8,
        position.entry_price_1e8,
        market.max_leverage,
    );
    let (mark_str, notional, unrealized, mm, ratio, liq) = if let Some(mark) = mark_price_1e8 {
        let notional = margin::notional_1e8(position.size_1e8, mark);
        let unrealized = margin::unrealized_pnl_1e8(position, mark);
        let mm = margin::maintenance_margin_requirement_1e8(
            position.size_1e8,
            mark,
            market.maintenance_margin_bps,
        );
        let ratio = margin::margin_ratio_bps(position, mark);
        let liq = margin::estimated_liquidation_price_1e8(position, market.maintenance_margin_bps);
        (
            Some(mark.to_string()),
            Some(notional.to_string()),
            Some(i128_to_wire(unrealized)),
            Some(mm.to_string()),
            Some(ratio.to_string()),
            liq.map(|v| v.to_string()),
        )
    } else {
        // Even without a mark, the estimated liq price can still be
        // computed because it depends only on entry, size, margin,
        // and maintenance bps. Surface it — the trader can still
        // reason about their risk band.
        let liq = margin::estimated_liquidation_price_1e8(position, market.maintenance_margin_bps);
        (None, None, None, None, None, liq.map(|v| v.to_string()))
    };
    let _ = cfg; // chain_id lives on the list envelope, not per row
    PerpPositionView {
        id: position.id.to_string(),
        account: position.account.0.clone(),
        market_id: position.market_id.clone(),
        side: position.side.as_str().to_string(),
        size_1e8: position.size_1e8.to_string(),
        entry_price_1e8: position.entry_price_1e8.to_string(),
        margin_1e8: position.margin_1e8.to_string(),
        realized_pnl_1e8: i128_to_wire(position.realized_pnl_1e8),
        status: position.status.as_str().to_string(),
        mark_price_1e8: mark_str,
        notional_1e8: notional,
        unrealized_pnl_1e8: unrealized,
        initial_margin_requirement_1e8: initial_margin.to_string(),
        maintenance_margin_requirement_1e8: mm,
        margin_ratio_bps: ratio,
        estimated_liquidation_price_1e8: liq,
        opened_at_ms: position.opened_at_ms,
        updated_at_ms: position.updated_at_ms,
        closed_at_ms: position.closed_at_ms,
        price_stale,
        trading_enabled: false,
    }
}

/// List all positions (open + closed) for one account. `price_stale`
/// is set per row based on the freshness of the mark price used to
/// compute the row's risk fields.
pub async fn list_perp_positions_for_account<P: PerpOraclePriceReader + ?Sized>(
    cfg: &PerpsReadConfig,
    store: &PerpPositionsStore,
    price_reader: &P,
    account: &AccountId,
) -> Result<PerpPositionListResponse> {
    ensure_read_enabled(cfg)?;
    let positions = store.list_for_account(account);
    let mut views = Vec::with_capacity(positions.len());
    for position in &positions {
        let (mark, stale) = fetch_mark_snapshot(cfg, price_reader, &position.market_id).await;
        let market = cfg
            .market_by_symbol(&position.market_id)
            .ok_or_else(|| BackendError::PerpsMarketNotFound(position.market_id.clone()))?;
        views.push(build_perp_position_view(cfg, market, position, mark, stale));
    }
    Ok(PerpPositionListResponse {
        positions: views,
        chain_id: cfg.chain_id,
        trading_enabled: false,
    })
}

/// Convenience for the internal callers that already know which
/// market they care about.
pub async fn list_perp_market_positions_for_account<P: PerpOraclePriceReader + ?Sized>(
    cfg: &PerpsReadConfig,
    store: &PerpPositionsStore,
    price_reader: &P,
    account: &AccountId,
    market_id: &str,
) -> Result<PerpPositionListResponse> {
    let mut all = list_perp_positions_for_account(cfg, store, price_reader, account).await?;
    all.positions.retain(|p| p.market_id == market_id);
    Ok(all)
}

/// Fetch one position view by (account, market). Returns
/// `PerpPositionNotFound` when there is no active position for the
/// requested pair.
pub async fn get_perp_position_view<P: PerpOraclePriceReader + ?Sized>(
    cfg: &PerpsReadConfig,
    store: &PerpPositionsStore,
    price_reader: &P,
    account: &AccountId,
    market_id: &str,
) -> Result<PerpPositionView> {
    ensure_read_enabled(cfg)?;
    let position = store
        .get_active(account, market_id)
        .ok_or(BackendError::PerpPositionNotFound)?;
    let (mark, stale) = fetch_mark_snapshot(cfg, price_reader, market_id).await;
    let market = cfg
        .market_by_symbol(market_id)
        .ok_or_else(|| BackendError::PerpsMarketNotFound(market_id.to_string()))?;
    Ok(build_perp_position_view(
        cfg, market, &position, mark, stale,
    ))
}

// =====================================================================
// PERPS-PERSISTENCE-HISTORY-LIFECYCLE-V1 — orders + fills history.
// =====================================================================

/// Wire view of one Perps order for the account-scoped history
/// endpoint.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PerpOrderView {
    pub order_id: String,
    pub account: String,
    pub market_id: String,
    pub side: String,
    pub order_type: String,
    pub price_1e8: String,
    pub size_1e8: String,
    pub remaining_size_1e8: String,
    pub filled_size_1e8: String,
    pub time_in_force: String,
    pub post_only: bool,
    pub reduce_only: bool,
    pub isolated_margin_1e8: String,
    pub status: String,
    pub client_order_id: Option<String>,
    pub terminal_reason_code: Option<String>,
    pub terminal_reason_message: Option<String>,
    pub terminal_reason_source: Option<String>,
    pub created_at_ms: TimestampMs,
    pub updated_at_ms: TimestampMs,
    /// Always `false` in V1.
    pub trading_enabled: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PerpOrderListResponse {
    pub orders: Vec<PerpOrderView>,
    pub chain_id: u64,
    pub trading_enabled: bool,
}

/// Wire view of one Perps fill, from the perspective of the requesting
/// account. `side` reflects the *account's* side in the fill (buy if
/// they got size added long, sell if added short) — computed from
/// their role (taker/maker) + the taker_side field.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PerpFillView {
    pub fill_id: String,
    pub market_id: String,
    pub taker_order_id: String,
    pub maker_order_id: String,
    pub taker_account: String,
    pub maker_account: String,
    /// The requesting account's role in the fill.
    pub liquidity_role: String,
    /// The requesting account's effective side.
    pub side: String,
    pub price_1e8: String,
    pub size_1e8: String,
    pub created_at_ms: TimestampMs,
    /// Always `false` in V1.
    pub trading_enabled: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PerpFillListResponse {
    pub fills: Vec<PerpFillView>,
    pub chain_id: u64,
    pub trading_enabled: bool,
}

pub fn build_perp_order_view(order: &PerpOrder) -> PerpOrderView {
    PerpOrderView {
        order_id: order.id.to_string(),
        account: order.account.0.clone(),
        market_id: order.market_id.clone(),
        side: order.side.as_str().to_string(),
        order_type: order.order_type.as_str().to_string(),
        price_1e8: order.price_1e8.to_string(),
        size_1e8: order.size_1e8.to_string(),
        remaining_size_1e8: order.remaining_size_1e8.to_string(),
        filled_size_1e8: order.filled_size_1e8.to_string(),
        time_in_force: order.time_in_force.as_str().to_string(),
        post_only: order.post_only,
        reduce_only: order.reduce_only,
        isolated_margin_1e8: order.isolated_margin_1e8.to_string(),
        status: order.status.as_str().to_string(),
        client_order_id: order.client_order_id.clone(),
        terminal_reason_code: order.terminal_reason_code.clone(),
        terminal_reason_message: order.terminal_reason_message.clone(),
        terminal_reason_source: order.terminal_reason_source.clone(),
        created_at_ms: order.created_at_ms,
        updated_at_ms: order.updated_at_ms,
        trading_enabled: false,
    }
}

pub fn build_perp_fill_view(fill: &PerpFill, viewer: &AccountId) -> PerpFillView {
    let viewer_lower = viewer.0.to_lowercase();
    let is_taker = fill.taker_account.0.to_lowercase() == viewer_lower;
    let (liquidity_role, viewer_side) = if is_taker {
        ("taker", fill.taker_side)
    } else {
        ("maker", fill.taker_side.opposite())
    };
    PerpFillView {
        fill_id: fill.id.to_string(),
        market_id: fill.market_id.clone(),
        taker_order_id: fill.taker_order_id.to_string(),
        maker_order_id: fill.maker_order_id.to_string(),
        taker_account: fill.taker_account.0.clone(),
        maker_account: fill.maker_account.0.clone(),
        liquidity_role: liquidity_role.to_string(),
        side: viewer_side.as_str().to_string(),
        price_1e8: fill.price_1e8.to_string(),
        size_1e8: fill.size_1e8.to_string(),
        created_at_ms: fill.created_at_ms,
        trading_enabled: false,
    }
}

pub fn list_perp_orders_for_account_view(
    cfg: &PerpsReadConfig,
    store: &PerpOrderStore,
    account: &AccountId,
) -> PerpOrderListResponse {
    let rows = store.list_orders_for_account(account);
    let orders: Vec<PerpOrderView> = rows.iter().map(build_perp_order_view).collect();
    PerpOrderListResponse {
        orders,
        chain_id: cfg.chain_id,
        trading_enabled: false,
    }
}

pub fn list_perp_fills_for_account_view(
    cfg: &PerpsReadConfig,
    store: &PerpOrderStore,
    account: &AccountId,
) -> PerpFillListResponse {
    let rows = store.list_fills_for_account(account);
    let fills: Vec<PerpFillView> = rows
        .iter()
        .map(|f| build_perp_fill_view(f, account))
        .collect();
    PerpFillListResponse {
        fills,
        chain_id: cfg.chain_id,
        trading_enabled: false,
    }
}

// Silence unused-import warning; PerpOrderSide is referenced only
// indirectly via `PerpOrder::side`. Keeping the explicit `use` makes
// the file's public surface easy to grep.
#[allow(dead_code)]
const _PERP_ORDER_SIDE_MARKER: fn() -> PerpOrderSide = || PerpOrderSide::Buy;

// =====================================================================
// PERPS-PG-EXECUTION-INTEGRATION-V1 — top-level dispatcher.
// =====================================================================

/// Dispatch a Perps submit to the right backing store: PG when
/// `state.repository.is_some()`, otherwise the in-memory ledger.
///
/// **This is still an internal service.** The public Perps mutation
/// routes remain fail-closed with `PerpsNotLive`; this function is
/// called only by unit tests and (in a future milestone) by a
/// public route once the readiness gate closes.
pub async fn submit_perp_order_via_state<P: PerpOraclePriceReader + ?Sized>(
    state: &crate::api::AppState,
    price_reader: &P,
    input: crate::perps::execution::SubmitPerpOrderInput,
) -> Result<crate::perps::SubmitPerpOrderOutcome> {
    if let Some(repository) = state.repository.clone() {
        return crate::perps::submit_perp_order_via_repository(
            &state.perps_read_config,
            &repository,
            price_reader,
            &state.lifecycle_events,
            input,
        )
        .await;
    }
    // PERPS-PG-HARNESS-AND-REJECTION-EMIT-V1 — snapshot the input
    // before it moves into the internal call so we can emit
    // `PerpOrderRejected` on a classified error. Mirrors the PG-path
    // behaviour so both branches surface the same lifecycle contract.
    let rejection_snapshot = crate::perps::execution_pg::RejectionSnapshot::from_input(&input);
    let mut order_store = state
        .perp_order_store
        .lock()
        .map_err(|_| BackendError::Config("perp_order_store lock poisoned".to_string()))?;
    let mut positions_store = state
        .perp_positions_store
        .lock()
        .map_err(|_| BackendError::Config("perp_positions_store lock poisoned".to_string()))?;
    match crate::perps::submit_perp_order_internal(
        &state.perps_read_config,
        &mut order_store,
        &mut positions_store,
        price_reader,
        input,
    )
    .await
    {
        Ok(outcome) => {
            // In-memory path: emit the same lifecycle bundle the PG
            // path emits AFTER commit.
            crate::perps::emit_lifecycle_for_submit_outcome(
                &state.lifecycle_events,
                &positions_store,
                &outcome,
            );
            Ok(outcome)
        }
        Err(err) => {
            rejection_snapshot.emit_if_classified(&state.lifecycle_events, &err);
            Err(err)
        }
    }
}

/// Dispatch a Perps cancel to the right backing store.
pub async fn cancel_perp_order_via_state(
    state: &crate::api::AppState,
    order_id: uuid::Uuid,
    caller: &AccountId,
) -> Result<crate::perps::PerpOrder> {
    if let Some(repository) = state.repository.clone() {
        return crate::perps::cancel_perp_order_via_repository(
            &repository,
            &state.lifecycle_events,
            order_id,
            caller,
        )
        .await;
    }
    let mut order_store = state
        .perp_order_store
        .lock()
        .map_err(|_| BackendError::Config("perp_order_store lock poisoned".to_string()))?;
    let order = crate::perps::cancel_perp_order_internal(&mut order_store, order_id, caller)?;
    crate::perps::emit_perp_order_lifecycle(&state.lifecycle_events, &order);
    Ok(order)
}

async fn fetch_mark_snapshot<P: PerpOraclePriceReader + ?Sized>(
    cfg: &PerpsReadConfig,
    price_reader: &P,
    market_id: &str,
) -> (Option<u128>, bool) {
    let Some(market) = cfg.market_by_symbol(market_id) else {
        return (None, true);
    };
    match price_reader.read_price(market).await {
        Ok(read) if read.ok && read.price_1e8 > 0 => {
            let ms = read.updated_at_ms();
            let stale_after_ms = (cfg.stale_after_sec as i64).saturating_mul(1000);
            let stale = if ms == 0 {
                true
            } else {
                now_ms().saturating_sub(ms) > stale_after_ms
            };
            (Some(read.price_1e8), stale)
        }
        _ => (None, true),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::perps::market_reader::InMemoryPerpMarketRegistryReader;
    use crate::perps::price_reader::InMemoryPerpOraclePriceReader;

    fn cfg() -> PerpsReadConfig {
        PerpsReadConfig::enabled_in_memory_for_tests()
    }

    #[tokio::test]
    async fn disabled_config_returns_perps_read_disabled() {
        let cfg = PerpsReadConfig::disabled();
        let reader = InMemoryPerpMarketRegistryReader::new();
        let err = list_perp_markets(&cfg, &reader).await.unwrap_err();
        assert!(matches!(err, BackendError::PerpsReadDisabled));
    }

    #[tokio::test]
    async fn list_markets_returns_configured_rows_with_status() {
        let cfg = cfg();
        let reader = InMemoryPerpMarketRegistryReader::new()
            .with_status(1, PerpMarketStatus::ReadOnly)
            .with_status(2, PerpMarketStatus::Paused);
        let listing = list_perp_markets(&cfg, &reader).await.unwrap();
        assert_eq!(listing.chain_id, 84532);
        assert!(!listing.trading_enabled);
        assert_eq!(listing.markets.len(), 2);
        let eth = &listing.markets[0];
        assert_eq!(eth.market_id, "ETH-PERP");
        assert_eq!(eth.onchain_market_id, "1");
        assert_eq!(eth.status, PerpMarketStatus::ReadOnly);
        assert!(!eth.trading_enabled);
        let btc = &listing.markets[1];
        assert_eq!(btc.market_id, "BTC-PERP");
        assert_eq!(btc.status, PerpMarketStatus::Paused);
    }

    #[tokio::test]
    async fn list_markets_uses_unknown_status_on_reader_failure_without_fabricating() {
        let cfg = cfg();
        let reader = InMemoryPerpMarketRegistryReader::new().with_forced_error("rpc down");
        let listing = list_perp_markets(&cfg, &reader).await.unwrap();
        assert_eq!(listing.markets.len(), 2);
        for market in &listing.markets {
            assert_eq!(market.status, PerpMarketStatus::Unknown);
            assert!(!market.trading_enabled);
        }
    }

    #[tokio::test]
    async fn get_market_by_unknown_symbol_returns_not_found() {
        let cfg = cfg();
        let reader = InMemoryPerpMarketRegistryReader::new();
        let err = get_perp_market(&cfg, &reader, "NOPE-PERP")
            .await
            .unwrap_err();
        assert!(matches!(err, BackendError::PerpsMarketNotFound(_)));
    }

    #[tokio::test]
    async fn get_price_returns_normalized_1e8() {
        let cfg = cfg();
        let reader = InMemoryPerpOraclePriceReader::new().with_price(
            "ETH-PERP",
            RawPriceRead {
                price_1e8: 350_000_000_000,
                updated_at_sec: (now_ms() / 1000) as u64,
                ok: true,
            },
        );
        let resp = get_perp_market_price(&cfg, &reader, "ETH-PERP")
            .await
            .unwrap();
        assert_eq!(resp.snapshot.market_id, "ETH-PERP");
        assert_eq!(resp.snapshot.index_price_1e8, "350000000000");
        assert_eq!(resp.snapshot.mark_price_1e8, "350000000000");
        assert!(!resp.snapshot.stale);
        assert!(!resp.snapshot.trading_enabled);
    }

    #[tokio::test]
    async fn get_price_flags_stale_when_older_than_config() {
        let cfg = cfg();
        // updatedAt = 1 second past epoch → definitely > 60s ago
        let reader = InMemoryPerpOraclePriceReader::new().with_price(
            "ETH-PERP",
            RawPriceRead {
                price_1e8: 350_000_000_000,
                updated_at_sec: 1,
                ok: true,
            },
        );
        let resp = get_perp_market_price(&cfg, &reader, "ETH-PERP")
            .await
            .unwrap();
        assert!(resp.snapshot.stale);
    }

    #[tokio::test]
    async fn get_price_refuses_to_fabricate_when_router_reports_not_ok() {
        let cfg = cfg();
        let reader = InMemoryPerpOraclePriceReader::new().with_price(
            "ETH-PERP",
            RawPriceRead {
                price_1e8: 350_000_000_000,
                updated_at_sec: 1_782_000_000,
                ok: false,
            },
        );
        let err = get_perp_market_price(&cfg, &reader, "ETH-PERP")
            .await
            .unwrap_err();
        assert!(matches!(err, BackendError::PerpsPriceUnavailable(_)));
    }

    #[tokio::test]
    async fn get_price_refuses_zero_even_when_router_says_ok() {
        let cfg = cfg();
        let reader = InMemoryPerpOraclePriceReader::new().with_price(
            "ETH-PERP",
            RawPriceRead {
                price_1e8: 0,
                updated_at_sec: 1_782_000_000,
                ok: true,
            },
        );
        let err = get_perp_market_price(&cfg, &reader, "ETH-PERP")
            .await
            .unwrap_err();
        assert!(matches!(err, BackendError::PerpsPriceUnavailable(_)));
    }

    #[tokio::test]
    async fn get_price_propagates_reader_transport_error() {
        let cfg = cfg();
        let reader =
            InMemoryPerpOraclePriceReader::new().with_forced_error("rpc timeout at oracle router");
        let err = get_perp_market_price(&cfg, &reader, "ETH-PERP")
            .await
            .unwrap_err();
        assert!(matches!(err, BackendError::PerpsPriceUnavailable(_)));
    }

    #[test]
    fn chain_id_matcher_accepts_configured_chain_id() {
        let cfg = cfg();
        assert!(ensure_chain_id_matches(&cfg, 84532).is_ok());
    }

    #[test]
    fn chain_id_matcher_rejects_mainnet_ids() {
        let cfg = cfg();
        assert!(matches!(
            ensure_chain_id_matches(&cfg, 1).unwrap_err(),
            BackendError::PerpsChainIdMismatch { .. }
        ));
        assert!(matches!(
            ensure_chain_id_matches(&cfg, 8453).unwrap_err(),
            BackendError::PerpsChainIdMismatch { .. }
        ));
    }

    #[test]
    fn chain_id_matcher_rejects_mismatched_testnet_id() {
        let cfg = cfg();
        assert!(matches!(
            ensure_chain_id_matches(&cfg, 11155111).unwrap_err(),
            BackendError::PerpsChainIdMismatch { .. }
        ));
    }
}
