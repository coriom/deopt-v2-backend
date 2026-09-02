//! PERPS-FULLSTACK-RUNTIME-INTEGRATION-V1 Part B — impact-mid cache.
//!
//! Thread-safe in-memory publish/read surface for the impact-mid keeper.
//! The keeper writes an `ImpactMidState` (either `Available` with a
//! sample or `Unavailable` with a reason) per market_id after each
//! tick; downstream consumers (a future funding worker, or tests via
//! `ImpactMidCache::get`) read the latest state without blocking the
//! writer.
//!
//! **No IO.** Everything lives behind a single `Mutex<HashMap>`; the
//! backend already runs the funding worker off a `std::sync::Mutex`
//! and the pattern matches. No `.await` is required to publish or
//! read; the mutex is only held for the duration of the map lookup.
//!
//! **Idempotency contract.** `publish` returns `true` iff the state
//! actually changed (either the wire tag flipped, or the `mid_1e8`
//! moved, or the reason changed). Same-mid-at-same-tick is a no-op
//! for the returned bool — callers use this to decide whether to
//! record a state-change metric.
//!
//! **No secrets.** Every field is a `u128` / `i64` / short enum tag.
//! There is no wallet, RPC URL, DB URL, admin token, or signature.

use crate::perps::impact_mid::{ImpactMidSample, InsufficientDepth};
use crate::types::TimestampMs;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// The state published to the cache for a market at a specific tick.
/// Distinguishes `Available` from `Unavailable` so downstream consumers
/// see the exact reason for a missing sample (helpful for a future
/// funding worker: "index available, impact-mid stale" is a different
/// fault than "no bid depth").
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ImpactMidState {
    /// A fresh, sanity-checked sample is published. `sample.mid_1e8` is
    /// the reference value.
    Available {
        sample: ImpactMidSample,
        updated_at_ms: TimestampMs,
    },
    /// No usable sample — reason distinguishes shallow-book, stale
    /// oracle index, deviation-vs-index, non-monotonic book, and
    /// zero-notional config bug.
    Unavailable {
        reason: ImpactMidUnavailableReason,
        updated_at_ms: TimestampMs,
    },
}

impl ImpactMidState {
    /// Convenience for tests + downstream readers: extract the sample
    /// iff the state is `Available`.
    pub fn sample(&self) -> Option<ImpactMidSample> {
        match self {
            Self::Available { sample, .. } => Some(*sample),
            Self::Unavailable { .. } => None,
        }
    }

    /// The tick time regardless of variant, for readiness / heartbeat.
    pub fn updated_at_ms(&self) -> TimestampMs {
        match self {
            Self::Available { updated_at_ms, .. } | Self::Unavailable { updated_at_ms, .. } => {
                *updated_at_ms
            }
        }
    }
}

/// Reason a tick could not publish a sample. Maps 1:1 to a metrics
/// bucket in the keeper. Kept small and stable so a future dashboard
/// alert can hard-match on `reason`.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ImpactMidUnavailableReason {
    /// The orderbook cannot absorb the configured `impact_notional_1e8`
    /// on the ask side.
    InsufficientAskDepth,
    /// Same, bid side.
    InsufficientBidDepth,
    /// Best bid >= best ask (crossed or locked). See `InsufficientDepth::NonMonotonic`.
    NonMonotonic,
    /// Interior overflow. Should never happen for realistic books; kept
    /// distinct so an operator can see it in metrics if it does.
    Overflow,
    /// The oracle index price is stale or unreadable at tick time.
    StaleIndex,
    /// The oracle index is fresh, the impact mid was computed, but the
    /// two disagree by more than `max_index_deviation_bps` — refuse to
    /// publish because "something is wrong with the book".
    IndexDeviationExceeded {
        observed_bps: u32,
        threshold_bps: u32,
    },
    /// Configuration bug: `impact_notional_1e8 == 0`. The keeper
    /// refuses to enable in this state at startup, so this variant
    /// SHOULD be unreachable in production; kept explicit for
    /// completeness.
    ZeroNotional,
}

impl ImpactMidUnavailableReason {
    /// Bounded string label for observability. See
    /// `PerpsImpactMidObservability` for the counter shape.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::InsufficientAskDepth => "insufficient_ask_depth",
            Self::InsufficientBidDepth => "insufficient_bid_depth",
            Self::NonMonotonic => "non_monotonic",
            Self::Overflow => "overflow",
            Self::StaleIndex => "stale_index",
            Self::IndexDeviationExceeded { .. } => "index_deviation_exceeded",
            Self::ZeroNotional => "zero_notional",
        }
    }

    /// Map an `InsufficientDepth` from the math layer to the cache's
    /// reason enum. Kept as a helper so the keeper never open-codes
    /// the mapping.
    pub fn from_insufficient_depth(insufficient: InsufficientDepth) -> Self {
        match insufficient {
            InsufficientDepth::ZeroNotional => Self::ZeroNotional,
            InsufficientDepth::NoAskDepth => Self::InsufficientAskDepth,
            InsufficientDepth::NoBidDepth => Self::InsufficientBidDepth,
            InsufficientDepth::NonMonotonic => Self::NonMonotonic,
            InsufficientDepth::Overflow => Self::Overflow,
        }
    }
}

/// Thread-safe per-market impact-mid state cache. Cloneable via
/// `Arc`; every producer + consumer shares the same underlying
/// `Mutex<HashMap>`.
#[derive(Clone, Debug, Default)]
pub struct ImpactMidCache {
    inner: Arc<Mutex<HashMap<String, ImpactMidState>>>,
}

impl ImpactMidCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Publish a fresh state for `market_id`. Returns `true` iff the
    /// state changed (either the variant flipped or a numeric field
    /// moved); a same-value republish is a no-op that returns `false`
    /// so callers can decide whether to record a state-change metric.
    /// Timestamps are NOT compared — same numeric mid at a different
    /// timestamp is considered unchanged, keeping heartbeat churn out
    /// of any dependent dashboard.
    pub fn publish(&self, market_id: &str, new_state: ImpactMidState) -> bool {
        let mut inner = self
            .inner
            .lock()
            .expect("impact-mid cache mutex poisoned");
        let changed = match inner.get(market_id) {
            Some(existing) => !states_equivalent(existing, &new_state),
            None => true,
        };
        // Always write the fresh state so the timestamp reflects the
        // last tick — the return value is the change signal, but a
        // dashboard that renders "last updated" still sees a moving
        // heartbeat.
        inner.insert(market_id.to_string(), new_state);
        changed
    }

    /// Read the latest state for `market_id`. `None` when the keeper
    /// has not yet run a tick against that market.
    pub fn get(&self, market_id: &str) -> Option<ImpactMidState> {
        self.inner
            .lock()
            .expect("impact-mid cache mutex poisoned")
            .get(market_id)
            .copied()
    }

    /// Read a snapshot of every market's latest state. Used by
    /// readiness / diagnostic surfaces.
    pub fn snapshot(&self) -> HashMap<String, ImpactMidState> {
        self.inner
            .lock()
            .expect("impact-mid cache mutex poisoned")
            .clone()
    }

    /// Clear a single market — used only by tests today.
    #[cfg(test)]
    pub fn clear(&self, market_id: &str) {
        self.inner
            .lock()
            .expect("impact-mid cache mutex poisoned")
            .remove(market_id);
    }
}

/// Equivalence check that IGNORES `updated_at_ms` — two `Available`
/// states with the same sample-values are equivalent regardless of tick
/// time, and same for two `Unavailable` states with the same reason.
/// This is what gates the "changed" return of `publish`.
fn states_equivalent(a: &ImpactMidState, b: &ImpactMidState) -> bool {
    match (a, b) {
        (
            ImpactMidState::Available { sample: sa, .. },
            ImpactMidState::Available { sample: sb, .. },
        ) => sa == sb,
        (
            ImpactMidState::Unavailable { reason: ra, .. },
            ImpactMidState::Unavailable { reason: rb, .. },
        ) => ra == rb,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(mid_1e8: u128) -> ImpactMidSample {
        ImpactMidSample {
            mid_1e8,
            ask_impact_1e8: mid_1e8 + 1_000,
            bid_impact_1e8: mid_1e8 - 1_000,
        }
    }

    #[test]
    fn first_publish_returns_changed_true() {
        let cache = ImpactMidCache::new();
        let changed = cache.publish(
            "ETH-PERP",
            ImpactMidState::Available {
                sample: sample(3000 * 100_000_000),
                updated_at_ms: 100,
            },
        );
        assert!(changed, "first publish must report a change");
    }

    #[test]
    fn same_sample_republish_returns_false() {
        let cache = ImpactMidCache::new();
        let s = sample(3000 * 100_000_000);
        cache.publish(
            "ETH-PERP",
            ImpactMidState::Available {
                sample: s,
                updated_at_ms: 100,
            },
        );
        let changed = cache.publish(
            "ETH-PERP",
            ImpactMidState::Available {
                sample: s,
                updated_at_ms: 200, // newer tick, same mid → no change
            },
        );
        assert!(!changed);
        // But the stored state's timestamp updates so a "last updated"
        // dashboard still sees the heartbeat.
        let stored = cache.get("ETH-PERP").unwrap();
        assert_eq!(stored.updated_at_ms(), 200);
    }

    #[test]
    fn different_sample_republish_returns_true() {
        let cache = ImpactMidCache::new();
        cache.publish(
            "ETH-PERP",
            ImpactMidState::Available {
                sample: sample(3000 * 100_000_000),
                updated_at_ms: 100,
            },
        );
        let changed = cache.publish(
            "ETH-PERP",
            ImpactMidState::Available {
                sample: sample(3001 * 100_000_000),
                updated_at_ms: 200,
            },
        );
        assert!(changed);
    }

    #[test]
    fn variant_flip_returns_true() {
        let cache = ImpactMidCache::new();
        cache.publish(
            "ETH-PERP",
            ImpactMidState::Available {
                sample: sample(3000 * 100_000_000),
                updated_at_ms: 100,
            },
        );
        let changed = cache.publish(
            "ETH-PERP",
            ImpactMidState::Unavailable {
                reason: ImpactMidUnavailableReason::StaleIndex,
                updated_at_ms: 200,
            },
        );
        assert!(changed);
    }

    #[test]
    fn same_reason_republish_returns_false() {
        let cache = ImpactMidCache::new();
        cache.publish(
            "ETH-PERP",
            ImpactMidState::Unavailable {
                reason: ImpactMidUnavailableReason::InsufficientAskDepth,
                updated_at_ms: 100,
            },
        );
        let changed = cache.publish(
            "ETH-PERP",
            ImpactMidState::Unavailable {
                reason: ImpactMidUnavailableReason::InsufficientAskDepth,
                updated_at_ms: 200,
            },
        );
        assert!(!changed);
    }

    #[test]
    fn per_market_isolation() {
        let cache = ImpactMidCache::new();
        cache.publish(
            "ETH-PERP",
            ImpactMidState::Available {
                sample: sample(3000 * 100_000_000),
                updated_at_ms: 100,
            },
        );
        cache.publish(
            "BTC-PERP",
            ImpactMidState::Available {
                sample: sample(60_000 * 100_000_000),
                updated_at_ms: 100,
            },
        );
        assert_eq!(
            cache.get("ETH-PERP").and_then(|s| s.sample()).unwrap().mid_1e8,
            3000 * 100_000_000
        );
        assert_eq!(
            cache.get("BTC-PERP").and_then(|s| s.sample()).unwrap().mid_1e8,
            60_000 * 100_000_000
        );
    }

    #[test]
    fn unavailable_reason_labels_stable() {
        assert_eq!(
            ImpactMidUnavailableReason::InsufficientAskDepth.as_str(),
            "insufficient_ask_depth"
        );
        assert_eq!(
            ImpactMidUnavailableReason::InsufficientBidDepth.as_str(),
            "insufficient_bid_depth"
        );
        assert_eq!(
            ImpactMidUnavailableReason::NonMonotonic.as_str(),
            "non_monotonic"
        );
        assert_eq!(ImpactMidUnavailableReason::Overflow.as_str(), "overflow");
        assert_eq!(
            ImpactMidUnavailableReason::StaleIndex.as_str(),
            "stale_index"
        );
        assert_eq!(
            ImpactMidUnavailableReason::IndexDeviationExceeded {
                observed_bps: 600,
                threshold_bps: 500,
            }
            .as_str(),
            "index_deviation_exceeded"
        );
        assert_eq!(
            ImpactMidUnavailableReason::ZeroNotional.as_str(),
            "zero_notional"
        );
    }

    #[test]
    fn from_insufficient_depth_full_mapping() {
        use crate::perps::impact_mid::InsufficientDepth as ID;
        assert_eq!(
            ImpactMidUnavailableReason::from_insufficient_depth(ID::ZeroNotional),
            ImpactMidUnavailableReason::ZeroNotional
        );
        assert_eq!(
            ImpactMidUnavailableReason::from_insufficient_depth(ID::NoAskDepth),
            ImpactMidUnavailableReason::InsufficientAskDepth
        );
        assert_eq!(
            ImpactMidUnavailableReason::from_insufficient_depth(ID::NoBidDepth),
            ImpactMidUnavailableReason::InsufficientBidDepth
        );
        assert_eq!(
            ImpactMidUnavailableReason::from_insufficient_depth(ID::NonMonotonic),
            ImpactMidUnavailableReason::NonMonotonic
        );
        assert_eq!(
            ImpactMidUnavailableReason::from_insufficient_depth(ID::Overflow),
            ImpactMidUnavailableReason::Overflow
        );
    }
}
