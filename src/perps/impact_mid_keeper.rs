//! PERPS-FULLSTACK-RUNTIME-INTEGRATION-V1 Part B — impact-mid keeper.
//!
//! A tokio-based worker that periodically walks the in-memory
//! `PerpOrderStore` for each configured market, computes the impact
//! mid at the operator-configured notional, sanity-checks the result
//! against the oracle index price, and publishes to an in-memory
//! `ImpactMidCache`. The cache is intended to be consumed by a future
//! funding worker (`FundingConfig.isEnabled=false` for this milestone).
//!
//! **Safety posture (unchanged by this module):**
//!
//! * Perps public trading remains fail-closed
//!   (`PERPS_PUBLIC_TRADING_ENABLED=false`).
//! * Perps funding worker + tick remain fail-closed
//!   (`PERPS_FUNDING_WORKER_ENABLED=false`,
//!   `PERPS_FUNDING_TICK_ENABLED=false`).
//! * `FundingConfig.isEnabled` on-chain STAYS `false`; this keeper
//!   NEVER broadcasts to `PerpEngine.updateImpactMid`. The value is
//!   written to the in-memory cache only.
//!
//! **What this module adds:**
//!
//! * `PerpsImpactMidKeeperConfig` — default-off periodic runner over a
//!   per-market impact-notional configuration. Layered gate:
//!   `enabled` starts the periodic loop; the tick is a no-op when
//!   disabled. Mainnet-refused on chain_id ∈ {1, 8453}.
//! * `spawn_perps_impact_mid_keeper` — tokio task spawner following
//!   the same shape as `spawn_perps_funding_worker`. Restart-safe
//!   (catches errors, logs, continues to next tick). Cancellable via
//!   a `tokio::sync::broadcast` shutdown channel.
//! * `run_perps_impact_mid_tick_once` — one-tick entry point exposed
//!   so the integration tests can drive the tick deterministically
//!   without spinning up the periodic loop.
//!
//! **What this module does NOT do:**
//!
//! * No on-chain broadcast to `PerpEngine.updateImpactMid`. That is
//!   the follow-up milestone; the keeper produces the value and the
//!   Solidity interface is the target for a future broadcaster.
//! * No mainnet path — `validate_startup(chain_id)` refuses enabled
//!   config on chain ids `1` and `8453`.
//! * No admin route. The keeper is periodic-only; downstream cache
//!   consumers read from the shared `ImpactMidCache`.

use crate::error::{BackendError, Result};
use crate::perps::config::{PerpsReadConfig, PerpsReadMarket};
use crate::perps::impact_mid::{impact_mid, Level};
use crate::perps::impact_mid_cache::{ImpactMidCache, ImpactMidState, ImpactMidUnavailableReason};
use crate::perps::impact_mid_publisher::{ImpactMidPublisher, PublishOutcome};
use crate::perps::order_store::PerpOrderStore;
use crate::perps::orderbook::{active_asks_sorted, active_bids_sorted};
use crate::perps::price_reader::PerpOraclePriceReader;
use crate::types::{now_ms, TimestampMs};
use std::sync::Arc;

// Bounds for tick interval configuration. Under `MIN_INTERVAL_MS` the
// loop starts to busy-spin; over `MAX_INTERVAL_MS` the "keeper" stops
// being meaningful (a market-reference number updated once an hour is
// not a keeper, it's a lifecycle event). Both bounds enforced by
// `validate_startup` even when the keeper is disabled so a dangerous
// value is caught early.
const MIN_TICK_INTERVAL_MS: u64 = 250;
const MAX_TICK_INTERVAL_MS: u64 = 3_600_000;

// Bounds for the deviation guard between impact-mid and the index
// price. Under 1 bp is unrealistically tight (the oracle itself has
// more noise than that); over 5000 bps (50%) accepts nonsense.
const MIN_MAX_INDEX_DEVIATION_BPS: u32 = 1;
const MAX_MAX_INDEX_DEVIATION_BPS: u32 = 5_000;

const BPS: u128 = 10_000;

// Defaults for the periodic keeper. `enabled=false` by construction —
// the keeper is opt-in via env.
const DEFAULT_TICK_INTERVAL_MS: u64 = 5_000;
/// Default per-market deviation threshold surfaced by the env parser.
/// Not consumed by this module itself (the config carries per-market
/// values); exported so `config/env.rs` can share the constant without
/// re-defining it.
pub const DEFAULT_MAX_INDEX_DEVIATION_BPS: u32 = 500;

/// Per-market config for the impact-mid keeper. The keeper only walks
/// markets present in this map — a market with no entry is skipped
/// silently (no cache write, no metric) so an operator can enable the
/// keeper for ETH-PERP without also standing up BTC-PERP.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PerpsImpactMidMarketConfig {
    /// Human symbol (e.g. `"ETH-PERP"`). Matches
    /// `PerpsReadConfig::market_by_symbol`.
    pub symbol: String,
    /// The taker notional (quote, `1e8`-scaled) at which the impact is
    /// measured. E.g. `$10k = 10_000 * 1e8`. Non-zero — validated at
    /// startup regardless of `enabled`.
    pub impact_notional_1e8: u128,
    /// Maximum absolute deviation, in bps, between the computed impact
    /// mid and the oracle index price. Sample is rejected + published
    /// as `IndexDeviationExceeded` above threshold.
    pub max_index_deviation_bps: u32,
}

/// Impact-mid keeper configuration. All defaults are safe
/// (`enabled=false`, empty markets vec). Enable via env:
///
/// * `PERPS_IMPACT_MID_KEEPER_ENABLED=true` (spawns the periodic loop)
/// * `PERPS_IMPACT_MID_KEEPER_INTERVAL_MS` (250..=3_600_000, default 5000)
/// * `PERPS_ETH_IMPACT_NOTIONAL_1E8=...` (non-zero to enable ETH-PERP)
/// * `PERPS_BTC_IMPACT_NOTIONAL_1E8=...`
/// * `PERPS_ETH_IMPACT_MAX_INDEX_DEVIATION_BPS=500` (per-market)
/// * `PERPS_BTC_IMPACT_MAX_INDEX_DEVIATION_BPS=500`
#[derive(Clone, Debug)]
pub struct PerpsImpactMidKeeperConfig {
    /// When `false`, `spawn_perps_impact_mid_keeper` returns immediately.
    /// The tick itself is a no-op when this flag is false (defensive
    /// second layer — the spawner already skipped, but the tick guard
    /// exists so `run_perps_impact_mid_tick_once` from tests cannot
    /// accidentally exercise a disabled keeper).
    pub enabled: bool,
    /// Milliseconds between periodic ticks. `MissedTickBehavior::Delay`
    /// applies — a slow tick does not queue subsequent ticks.
    pub tick_interval_ms: u64,
    /// Per-market rows. Empty in the disabled/default config; populated
    /// from env at startup.
    pub markets: Vec<PerpsImpactMidMarketConfig>,
    /// PERPS-CLOSED-TEST-HARDENING-V1 Part E — optional on-chain
    /// publisher handle. When `Some`, the keeper broadcasts each fresh
    /// sample to the on-chain `PerpEngine.updateImpactMid` writer after
    /// the cache write succeeds. `None` (V1 default) keeps the keeper
    /// cache-only — no broadcast, no signer, no RPC. Publish errors are
    /// LOGGED but never poison the keeper tick.
    pub publisher: Option<Arc<dyn ImpactMidPublisher>>,
}

impl PartialEq for PerpsImpactMidKeeperConfig {
    /// Equality ignores the `publisher` trait object — a runtime
    /// handle is not a value that can be meaningfully compared. All
    /// other fields (enabled, tick_interval_ms, markets) are compared
    /// structurally. Tests that need to prove "publisher wired" should
    /// call `.publisher.is_some()` directly.
    fn eq(&self, other: &Self) -> bool {
        self.enabled == other.enabled
            && self.tick_interval_ms == other.tick_interval_ms
            && self.markets == other.markets
    }
}

impl Eq for PerpsImpactMidKeeperConfig {}

impl PerpsImpactMidKeeperConfig {
    /// The safe default: disabled, no markets configured, safe interval,
    /// no publisher.
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            tick_interval_ms: DEFAULT_TICK_INTERVAL_MS,
            markets: Vec::new(),
            publisher: None,
        }
    }

    /// Attach an on-chain publisher. Consumes-and-returns for the
    /// builder-style wiring in `main.rs` / test setup.
    pub fn with_publisher(mut self, publisher: Arc<dyn ImpactMidPublisher>) -> Self {
        self.publisher = Some(publisher);
        self
    }

    /// Refuses obviously-invalid configuration at startup. Runs even
    /// when `enabled=false` so a dangerous knob (e.g. `interval_ms=0`)
    /// is caught before the operator later flips the enable flag.
    /// Mainnet is refused unconditionally when the keeper is enabled.
    pub fn validate_startup(&self, chain_id: u64) -> Result<()> {
        if self.tick_interval_ms < MIN_TICK_INTERVAL_MS
            || self.tick_interval_ms > MAX_TICK_INTERVAL_MS
        {
            return Err(BackendError::Config(format!(
                "PERPS_IMPACT_MID_KEEPER_INTERVAL_MS must be in [{MIN_TICK_INTERVAL_MS}, \
                 {MAX_TICK_INTERVAL_MS}], got {}",
                self.tick_interval_ms
            )));
        }
        for market in &self.markets {
            if market.impact_notional_1e8 == 0 {
                return Err(BackendError::Config(format!(
                    "PERPS impact-mid keeper: market {} impact_notional_1e8 must be > 0",
                    market.symbol
                )));
            }
            if market.max_index_deviation_bps < MIN_MAX_INDEX_DEVIATION_BPS
                || market.max_index_deviation_bps > MAX_MAX_INDEX_DEVIATION_BPS
            {
                return Err(BackendError::Config(format!(
                    "PERPS impact-mid keeper: market {} max_index_deviation_bps must be in \
                     [{MIN_MAX_INDEX_DEVIATION_BPS}, {MAX_MAX_INDEX_DEVIATION_BPS}], got {}",
                    market.symbol, market.max_index_deviation_bps
                )));
            }
        }
        if self.enabled && (chain_id == 1 || chain_id == 8453) {
            return Err(BackendError::Config(format!(
                "PERPS_IMPACT_MID_KEEPER_ENABLED refused on mainnet chain id {chain_id}"
            )));
        }
        Ok(())
    }

    /// Look up a per-market row by symbol.
    pub fn market_by_symbol(&self, symbol: &str) -> Option<&PerpsImpactMidMarketConfig> {
        self.markets.iter().find(|m| m.symbol == symbol)
    }
}

// ---------------------------------------------------------------------
// Tick loop + one-tick entry point.
// ---------------------------------------------------------------------

/// Spawn the periodic impact-mid keeper. No-op when `enabled=false`.
/// The shutdown channel — when supplied — cancels the loop on the
/// first message; when `None`, the loop lives for the process
/// lifetime (matches the pre-existing perps funding/liquidation worker
/// posture).
///
/// Inputs are handles (`Arc`) so the caller may share them with the
/// funding worker or a diagnostic surface. The keeper never mutates
/// the order store, positions store, or oracle reader — everything
/// downstream of the read is confined to the `ImpactMidCache`.
pub fn spawn_perps_impact_mid_keeper<P>(
    config: PerpsImpactMidKeeperConfig,
    read_config: PerpsReadConfig,
    order_store: Arc<std::sync::Mutex<PerpOrderStore>>,
    price_reader: Arc<P>,
    cache: ImpactMidCache,
    mut shutdown_rx: Option<tokio::sync::broadcast::Receiver<()>>,
) -> Option<tokio::task::JoinHandle<()>>
where
    P: PerpOraclePriceReader + Send + Sync + 'static,
{
    if !config.enabled {
        tracing::info!(
            worker = "perps_impact_mid_keeper",
            enabled = false,
            "perps impact-mid keeper disabled"
        );
        return None;
    }
    let interval_ms = config.tick_interval_ms;
    let handle = tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_millis(interval_ms));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        // Skip the immediate `tick.tick()` (which fires at t=0) so a
        // spammy restart loop never spawns a tick storm before the
        // process reaches a steady state.
        interval.tick().await;
        loop {
            let tick_fut = interval.tick();
            if let Some(rx) = shutdown_rx.as_mut() {
                tokio::select! {
                    _ = tick_fut => {}
                    _ = rx.recv() => {
                        tracing::info!(
                            worker = "perps_impact_mid_keeper",
                            "shutdown signal received; exiting periodic loop"
                        );
                        return;
                    }
                }
            } else {
                tick_fut.await;
            }
            // The tick catches its own errors — never let a panic
            // escape the task.
            run_perps_impact_mid_tick_once(
                &config,
                &read_config,
                &order_store,
                price_reader.as_ref(),
                &cache,
            )
            .await;
        }
    });
    Some(handle)
}

/// Execute a single impact-mid tick across every configured market.
/// Public so integration tests can drive the tick deterministically
/// without spinning up the periodic loop. Never panics; every failure
/// mode publishes an `ImpactMidState::Unavailable` for the affected
/// market and returns.
pub async fn run_perps_impact_mid_tick_once<P>(
    config: &PerpsImpactMidKeeperConfig,
    read_config: &PerpsReadConfig,
    order_store: &std::sync::Mutex<PerpOrderStore>,
    price_reader: &P,
    cache: &ImpactMidCache,
) where
    P: PerpOraclePriceReader + ?Sized,
{
    if !config.enabled {
        // Defensive: never do work when disabled, even if the tick was
        // triggered by a test.
        return;
    }
    let now = now_ms();
    for market_cfg in &config.markets {
        let market = match read_config.market_by_symbol(&market_cfg.symbol) {
            Some(m) => m.clone(),
            None => {
                tracing::warn!(
                    market = %market_cfg.symbol,
                    "perps impact-mid keeper: market not present in read config; skipping"
                );
                continue;
            }
        };

        // Phase 1: derive the levels from the order store. We copy
        // out of the store while holding the lock, then drop it before
        // the async price read so we never `.await` while holding a
        // `std::sync::Mutex` guard (matches the existing perps worker
        // pattern in `run_perps_funding_tick_once`).
        let (asks, bids) = {
            let guard = match order_store.lock() {
                Ok(g) => g,
                Err(_) => {
                    // Poisoned mutex → publish unavailable with a
                    // generic reason. Overflow is the closest match for
                    // "something structural went wrong" without inventing
                    // a new reason variant.
                    tracing::warn!(
                        market = %market.symbol,
                        "perps impact-mid keeper: order store mutex poisoned; publishing unavailable"
                    );
                    cache.publish(
                        &market.symbol,
                        ImpactMidState::Unavailable {
                            reason: ImpactMidUnavailableReason::Overflow,
                            updated_at_ms: now,
                        },
                    );
                    continue;
                }
            };
            let asks = orders_to_levels(&active_asks_sorted(&guard, &market.symbol));
            let bids = orders_to_levels(&active_bids_sorted(&guard, &market.symbol));
            (asks, bids)
        };

        // Phase 2: compute the impact-mid from the levels.
        let sample = match impact_mid(&asks, &bids, market_cfg.impact_notional_1e8) {
            Ok(sample) => sample,
            Err(insufficient) => {
                let reason =
                    ImpactMidUnavailableReason::from_insufficient_depth(insufficient);
                tracing::debug!(
                    market = %market.symbol,
                    reason = %reason.as_str(),
                    "perps impact-mid keeper: insufficient depth"
                );
                cache.publish(
                    &market.symbol,
                    ImpactMidState::Unavailable {
                        reason,
                        updated_at_ms: now,
                    },
                );
                continue;
            }
        };

        // Phase 3: read the oracle index. `RawPriceRead { ok: false }`
        // or a stale updated_at both count as `StaleIndex` — the
        // funding worker will treat both as "no reference" so we do
        // the same.
        let index_1e8 = match read_fresh_index(price_reader, &market, read_config, now).await {
            Some(index) => index,
            None => {
                tracing::debug!(
                    market = %market.symbol,
                    "perps impact-mid keeper: oracle index unavailable / stale"
                );
                cache.publish(
                    &market.symbol,
                    ImpactMidState::Unavailable {
                        reason: ImpactMidUnavailableReason::StaleIndex,
                        updated_at_ms: now,
                    },
                );
                continue;
            }
        };

        // Phase 4: deviation guard. `|mid - index| * BPS > index * threshold_bps`
        // ⇒ reject. Computed with `u128` throughout so a $60k BTC book
        // stays well below the overflow ceiling.
        let observed_bps = deviation_bps(index_1e8, sample.mid_1e8);
        if observed_bps > market_cfg.max_index_deviation_bps {
            tracing::warn!(
                market = %market.symbol,
                observed_bps,
                threshold_bps = market_cfg.max_index_deviation_bps,
                "perps impact-mid keeper: impact mid deviates from index beyond threshold"
            );
            cache.publish(
                &market.symbol,
                ImpactMidState::Unavailable {
                    reason: ImpactMidUnavailableReason::IndexDeviationExceeded {
                        observed_bps,
                        threshold_bps: market_cfg.max_index_deviation_bps,
                    },
                    updated_at_ms: now,
                },
            );
            continue;
        }

        // Phase 5: publish. `publish` returns whether the state
        // actually changed; we log the flip for operator visibility
        // but do NOT fail the tick on either outcome — a same-value
        // republish is a valid heartbeat.
        let changed = cache.publish(
            &market.symbol,
            ImpactMidState::Available {
                sample,
                updated_at_ms: now,
            },
        );
        tracing::debug!(
            market = %market.symbol,
            mid_1e8 = sample.mid_1e8,
            bid_impact_1e8 = sample.bid_impact_1e8,
            ask_impact_1e8 = sample.ask_impact_1e8,
            index_1e8 = index_1e8,
            observed_bps,
            changed,
            "perps impact-mid keeper: published sample"
        );

        // Phase 6: on-chain publisher (PERPS-CLOSED-TEST-HARDENING-V1
        // Part E). Optional; runs only when the keeper config carries
        // a `publisher`. The cache write already succeeded above — a
        // transient RPC error here must NOT poison the tick, so we
        // log at warn level and continue. The publisher path is
        // idempotent per (market_id, mid, timestamp) so a re-broadcast
        // on a later tick is safe.
        if let Some(publisher) = config.publisher.as_ref() {
            match publisher
                .publish(market.onchain_market_id, sample.mid_1e8, now)
                .await
            {
                Ok(PublishOutcome::Published { tx_hash, block_number }) => {
                    tracing::info!(
                        market = %market.symbol,
                        onchain_market_id = market.onchain_market_id,
                        mid_1e8 = sample.mid_1e8,
                        tx_hash = %tx_hash,
                        block_number,
                        "perps impact-mid keeper: on-chain publish confirmed"
                    );
                }
                Ok(PublishOutcome::Skipped { reason }) => {
                    tracing::debug!(
                        market = %market.symbol,
                        onchain_market_id = market.onchain_market_id,
                        mid_1e8 = sample.mid_1e8,
                        reason = %reason,
                        "perps impact-mid keeper: on-chain publish skipped"
                    );
                }
                Err(err) => {
                    tracing::warn!(
                        market = %market.symbol,
                        onchain_market_id = market.onchain_market_id,
                        mid_1e8 = sample.mid_1e8,
                        error = %err,
                        "perps impact-mid keeper: on-chain publish failed (cache write still succeeded)"
                    );
                }
            }
        }
    }
}

/// Convert a `Vec<PerpOrder>` (already sorted best-first by the
/// `orderbook` helpers) into the `Level` slice the math layer
/// consumes. Uses the ORDER's `remaining_size_1e8` (post-partial-fill
/// depth) because that is the real depth a taker would see.
fn orders_to_levels(orders: &[crate::perps::orders::PerpOrder]) -> Vec<Level> {
    orders
        .iter()
        .map(|o| Level::new(o.price_1e8, o.remaining_size_1e8))
        .collect()
}

/// Read the oracle index for `market` and return the `1e8`-scaled
/// price ONLY when it is fresh (`updated_at_ms` within
/// `read_config.stale_after_sec` of `now`) and `ok`. Returns `None`
/// on any read failure, stale timestamp, `ok=false`, or zero price —
/// the caller maps `None` to `StaleIndex`.
async fn read_fresh_index<P: PerpOraclePriceReader + ?Sized>(
    price_reader: &P,
    market: &PerpsReadMarket,
    read_config: &PerpsReadConfig,
    now: TimestampMs,
) -> Option<u128> {
    let read = match price_reader.read_price(market).await {
        Ok(read) => read,
        Err(_) => return None,
    };
    if !read.ok || read.price_1e8 == 0 {
        return None;
    }
    let stale_after_ms = (read_config.stale_after_sec as i64).saturating_mul(1000);
    let updated_ms = read.updated_at_ms();
    if updated_ms == 0 || now.saturating_sub(updated_ms) > stale_after_ms {
        return None;
    }
    Some(read.price_1e8)
}

/// Absolute deviation between the index and the impact mid, expressed
/// in basis points relative to the index. Saturates at `u32::MAX` on
/// unreasonable divergence — enough to trip any sane threshold.
fn deviation_bps(index_1e8: u128, mid_1e8: u128) -> u32 {
    if index_1e8 == 0 {
        // Callers already reject a zero index at the freshness gate;
        // this branch is defensive only.
        return u32::MAX;
    }
    let diff = if mid_1e8 >= index_1e8 {
        mid_1e8 - index_1e8
    } else {
        index_1e8 - mid_1e8
    };
    let bps = diff.saturating_mul(BPS) / index_1e8;
    u32::try_from(bps).unwrap_or(u32::MAX)
}

/// Diagnostic: how many entries the cache has after the keeper has
/// run. Kept private-ish (used by tests). Not exported through
/// `perps::mod`.
#[cfg(test)]
pub(crate) fn snapshot_len(cache: &ImpactMidCache) -> usize {
    cache.snapshot().len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_config_validates_ok() {
        assert!(PerpsImpactMidKeeperConfig::disabled()
            .validate_startup(84532)
            .is_ok());
    }

    #[test]
    fn interval_out_of_range_rejects() {
        let mut cfg = PerpsImpactMidKeeperConfig::disabled();
        cfg.tick_interval_ms = 0;
        assert!(cfg.validate_startup(84532).is_err());
        cfg.tick_interval_ms = MAX_TICK_INTERVAL_MS + 1;
        assert!(cfg.validate_startup(84532).is_err());
    }

    #[test]
    fn enabled_on_mainnet_refused() {
        let mut cfg = PerpsImpactMidKeeperConfig::disabled();
        cfg.enabled = true;
        assert!(cfg.validate_startup(1).is_err());
        assert!(cfg.validate_startup(8453).is_err());
    }

    #[test]
    fn disabled_on_mainnet_allowed() {
        // The keeper stays fail-closed on mainnet; the DISABLED config
        // must pass validation everywhere so the process boots without
        // requiring an operator opt-out on every deployment.
        let cfg = PerpsImpactMidKeeperConfig::disabled();
        assert!(cfg.validate_startup(1).is_ok());
        assert!(cfg.validate_startup(8453).is_ok());
    }

    #[test]
    fn zero_notional_market_rejects_regardless_of_enabled() {
        let mut cfg = PerpsImpactMidKeeperConfig::disabled();
        cfg.markets = vec![PerpsImpactMidMarketConfig {
            symbol: "ETH-PERP".to_string(),
            impact_notional_1e8: 0,
            max_index_deviation_bps: 500,
        }];
        // Even with enabled=false, a zero notional is a startup
        // refusal because it will silently break the tick if the
        // operator later flips enabled=true.
        assert!(cfg.validate_startup(84532).is_err());
    }

    #[test]
    fn deviation_bps_bounds_check() {
        let mut cfg = PerpsImpactMidKeeperConfig::disabled();
        cfg.markets = vec![PerpsImpactMidMarketConfig {
            symbol: "ETH-PERP".to_string(),
            impact_notional_1e8: 10_000_00000000,
            max_index_deviation_bps: 0,
        }];
        assert!(cfg.validate_startup(84532).is_err());
        cfg.markets[0].max_index_deviation_bps = MAX_MAX_INDEX_DEVIATION_BPS + 1;
        assert!(cfg.validate_startup(84532).is_err());
    }

    #[test]
    fn market_by_symbol_lookup() {
        let cfg = PerpsImpactMidKeeperConfig {
            enabled: false,
            tick_interval_ms: 5000,
            markets: vec![PerpsImpactMidMarketConfig {
                symbol: "ETH-PERP".to_string(),
                impact_notional_1e8: 10_000 * 100_000_000,
                max_index_deviation_bps: 500,
            }],
            publisher: None,
        };
        assert!(cfg.market_by_symbol("ETH-PERP").is_some());
        assert!(cfg.market_by_symbol("BTC-PERP").is_none());
    }

    #[test]
    fn disabled_config_has_no_publisher_by_default() {
        // PERPS-CLOSED-TEST-HARDENING-V1 Part E — the safe default
        // starts without an on-chain publisher; operators must opt in
        // explicitly via `with_publisher(...)` or env wiring.
        let cfg = PerpsImpactMidKeeperConfig::disabled();
        assert!(cfg.publisher.is_none());
    }

    #[test]
    fn with_publisher_attaches_handle() {
        let cfg = PerpsImpactMidKeeperConfig::disabled()
            .with_publisher(Arc::new(crate::perps::NoOpPublisher::new()));
        assert!(cfg.publisher.is_some());
        // PartialEq ignores the publisher — a `with_publisher`-derived
        // config still compares equal to the `disabled()` baseline on
        // the structural fields.
        assert_eq!(cfg, PerpsImpactMidKeeperConfig::disabled());
    }

    #[test]
    fn deviation_bps_math() {
        // 1% deviation → 100 bps.
        let index = 3_000u128 * 100_000_000;
        let mid = 3_030u128 * 100_000_000;
        assert_eq!(deviation_bps(index, mid), 100);
        // Zero index → saturate.
        assert_eq!(deviation_bps(0, mid), u32::MAX);
        // Same → 0.
        assert_eq!(deviation_bps(index, index), 0);
    }
}
