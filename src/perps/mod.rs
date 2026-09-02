//! PERPS-MINIMAL-MARKET-AND-PRICE-V1 — read-only Perps market registry
//! and mark/index price service.
//!
//! **Scope:** read-only. This module does NOT execute Perps orders, hold
//! Perps positions, compute Perps margin, accrue Perps funding, or
//! liquidate Perps. All Perps mutation HTTP routes remain fail-closed
//! via `BackendError::PerpsNotLive` — this module adds a distinct
//! read layer with its own error variants
//! (`PerpsReadDisabled`, `PerpsChainIdMismatch`, `PerpsMarketNotFound`,
//! `PerpsPriceUnavailable`).
//!
//! **On-chain contracts read from (Base Sepolia, chain id 84532):**
//!   - `PerpMarketRegistry` — market config, active flag, risk fields.
//!   - `OracleRouter` — index price + timestamp per (base, quote) pair.
//!
//! **What this module does NOT do:**
//!   - No mark-price computation with funding premium (mark == index
//!     for V1; a separate `PERPS-FUNDING-V1` milestone will land the
//!     real mark from `PerpEngineViews.getMarkPrice`).
//!   - No writes, no signing, no broadcasts.
//!   - No fabrication: if RPC or oracle is unreachable, the endpoint
//!     surfaces `PerpsPriceUnavailable` — it never returns a made-up
//!     number.

pub mod config;
pub mod execution;
pub mod execution_pg;
pub mod fills;
pub mod funding;
pub mod funding_pg;
pub mod impact_mid;
pub mod impact_mid_cache;
pub mod impact_mid_keeper;
pub mod intent_nonce_store;
pub mod lifecycle;
pub mod liquidation;
pub mod liquidation_pg;
pub mod margin;
pub mod market_reader;
pub mod observability;
pub mod order_store;
pub mod orderbook;
pub mod orders;
pub mod positions;
pub mod price_reader;
pub mod rejection;
pub mod service;
pub mod types;
pub mod workers;

pub use funding::{
    build_perp_funding_view, calculate_funding_payment_1e8, emit_perp_funding_lifecycle,
    list_perp_funding_events_for_account_view, prefetch_funding_indices, run_perp_funding_tick,
    FundingIndexRead, InMemoryPerpFundingIndexReader, PerpFundingEvent, PerpFundingEventView,
    PerpFundingEventsStore, PerpFundingIndexReader, PerpFundingListResponse,
    PerpFundingTickResponse,
};
pub use funding_pg::run_perp_funding_tick_via_repository;
pub use lifecycle::{
    emit_lifecycle_for_submit_outcome, emit_perp_fill_lifecycle, emit_perp_order_lifecycle,
    emit_perp_position_lifecycle, emit_perp_rejection_lifecycle,
};
pub use liquidation::{
    build_perp_liquidation_view, evaluate_perp_liquidation, liquidate_perp_position_internal,
    list_perp_liquidations_for_account_view, prefetch_mark_prices, run_perp_liquidation_tick,
    LiquidationEvaluation, PerpLiquidationEvent, PerpLiquidationEventView,
    PerpLiquidationListResponse, PerpLiquidationStatus, PerpLiquidationTickResponse,
    PerpLiquidationsStore,
};
pub use liquidation_pg::run_perp_liquidation_tick_via_repository;
pub use rejection::classify_perp_rejection;

pub use execution::{
    cancel_perp_order_internal, list_perp_fills_for_account, list_perp_orders_for_account,
    submit_perp_order_internal, SubmitPerpOrderInput, SubmitPerpOrderOutcome,
};
pub use execution_pg::{cancel_perp_order_via_repository, submit_perp_order_via_repository};
pub use order_store::PerpOrderStore;
pub use orders::{
    reason as perp_order_reason, PerpFill, PerpOrder, PerpOrderSide, PerpOrderStatus,
    PerpOrderType, PerpTimeInForce,
};

pub use config::{
    compute_perps_market_risk_status, compute_perps_market_risk_status_view,
    PerpsMarketAdminOverride, PerpsMarketOracleSnapshot, PerpsMarketRiskStatus,
    PerpsMarketRiskStatusView, PerpsReadConfig, PerpsReadMarket,
};
pub use execution::deviation_bps;
pub use fills::{apply_perp_fill_for_account, PerpFillInput, PerpFillOutcome};
pub use market_reader::{PerpMarketRegistryReader, PerpMarketRegistryRpcReader};
pub use observability::{
    cancel_reason_labels, submit_reason_labels, tick_age_seconds, PerpsObservability,
    PerpsObservabilitySnapshot,
};
pub use positions::{PerpPosition, PerpPositionStatus, PerpPositionsStore, PerpSide};
pub use price_reader::{PerpOraclePriceReader, PerpOracleRouterRpcReader};
pub use service::{
    cancel_perp_order_via_state, get_perp_market, get_perp_market_price, get_perp_market_with_risk,
    get_perp_position_view, list_perp_market_positions_for_account, list_perp_markets,
    list_perp_markets_with_risk, list_perp_positions_for_account, submit_perp_order_via_state,
    PerpFillListResponse, PerpFillView, PerpMarketListing, PerpMarketPriceResponse,
    PerpOrderListResponse, PerpOrderView, PerpPositionListResponse, PerpPositionView,
};
pub use types::{PerpMarket, PerpMarketStatus, PerpPriceSnapshot};
pub use workers::{
    run_perps_funding_tick_once, run_perps_liquidation_tick_once, spawn_perps_funding_worker,
    spawn_perps_liquidation_worker, PerpsFundingWorkerConfig, PerpsLiquidationWorkerConfig,
    PerpsWorkerStaleOraclePolicy, PerpsWorkerTickRecord,
};

pub use impact_mid::{
    impact_mid, impact_price_walk, ImpactMidSample, InsufficientDepth, Level as ImpactMidLevel,
};
pub use impact_mid_cache::{ImpactMidCache, ImpactMidState, ImpactMidUnavailableReason};
pub use impact_mid_keeper::{
    run_perps_impact_mid_tick_once, spawn_perps_impact_mid_keeper, PerpsImpactMidKeeperConfig,
    PerpsImpactMidMarketConfig,
};
pub use intent_nonce_store::PerpOrderIntentNonceStore;
