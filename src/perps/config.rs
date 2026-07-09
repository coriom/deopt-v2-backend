//! PERPS-MINIMAL-MARKET-AND-PRICE-V1 — read-only Perps configuration.
//!
//! **Off by default.** Every field defaults to a sentinel that keeps
//! the read endpoints returning `PerpsReadDisabled` (503) until an
//! operator explicitly wires the addresses via env. This mirrors the
//! posture of every other opt-in on-chain reader in the codebase
//! (`TradingViewsConfig::disabled()`, `PerpNonceSyncConfig::disabled()`).
//!
//! **Never activates mutations.** No field in this struct can flip the
//! Perps mutation gate. The `PerpsNotLive` handler-entry guards remain
//! in place regardless of what is configured here.

use crate::types::{AccountId, TimestampMs};
use serde::{Deserialize, Serialize};

const DEFAULT_BASE_SEPOLIA_CHAIN_ID: u64 = 84532;
const DEFAULT_STALE_AFTER_SEC: u64 = 60;
const DEFAULT_ETH_ONCHAIN_MARKET_ID: u64 = 1;
const DEFAULT_BTC_ONCHAIN_MARKET_ID: u64 = 2;

// PERPS-MARGIN-ORACLE-RISK-V1 defaults.
//
// Deviation: OracleRouter guarantees `mark == index` in V1, but we
// thread the guard through the same pre-submit gate as the freshness
// check so a future divergence flips from "pass deterministically" to
// "reject at threshold". Threshold defaults to 500 bps (5 %).
const DEFAULT_MAX_DEVIATION_BPS: u32 = 500;
// Sane bounds for the deviation config — anything outside these is a
// startup-refusal because it either lets clearly unsafe prices through
// or is so tight it can never pass. `10_000` bps == 100 %.
const MAX_ALLOWED_DEVIATION_BPS: u32 = 5_000;
// Same reasoning for staleness — anything under 1s risks flapping,
// anything over an hour has no legitimate trading use case in V1.
const MIN_STALE_AFTER_SEC: u64 = 1;
const MAX_STALE_AFTER_SEC: u64 = 3_600;

/// One row of the read-only market catalogue. The (base, quote)
/// addresses tell `OracleRouter.getPriceSafe(base, quote)` which feed
/// to query; the human symbol is what the frontend renders.
///
/// PERPS-ISOLATED-MARGIN-POSITION-ENGINE-V1 — per-market risk
/// parameters live here rather than in a separate config so the
/// operator wires them alongside the addresses. Defaults are the
/// conservative V1 values from the milestone brief; every field is
/// overrideable via `PERPS_{ETH,BTC}_*` env vars.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PerpsReadMarket {
    /// Stable human symbol (e.g. `"ETH-PERP"`).
    pub symbol: String,
    /// The on-chain `uint256` market id (as a u64 — both seeded markets
    /// fit; we keep the wire format decimal-string in the API).
    pub onchain_market_id: u64,
    /// Underlying symbol (e.g. `"ETH"`).
    pub base_asset_label: String,
    /// Quote symbol (e.g. `"mUSDC"`).
    pub quote_asset_label: String,
    /// Address of the underlying ERC-20 (e.g. mWETH).
    pub base_asset_address: AccountId,
    /// Address of the quote ERC-20 (e.g. mUSDC).
    pub quote_asset_address: AccountId,
    /// Maximum permitted leverage for opening or increasing a
    /// position on this market. Example: `10` for ETH-PERP allows up
    /// to 10× notional per unit of isolated margin.
    pub max_leverage: u32,
    /// Maintenance margin as a fraction of notional, in basis points.
    /// Example: `500` = 5%. When the position's equity (margin +
    /// unrealised PnL) falls below `notional * maintenance_margin_bps
    /// / 10_000`, the position is eligible for liquidation in a
    /// later milestone.
    pub maintenance_margin_bps: u32,
    /// PERPS-MARGIN-ORACLE-RISK-V1 — per-market risk caps. Every cap
    /// is `None` for the disabled config and populated on the closed-
    /// test config. Reads default to `u128::MAX` behaviour (no cap)
    /// when `None`, matching the pre-milestone posture.
    ///
    /// Max order size (base units, `1e8` scaled). E.g. 10 ETH → `10 * 1e8 = 1_000_000_000`.
    pub max_order_size_1e8: Option<u128>,
    /// Max order notional (quote units, `1e8` scaled). E.g. $100k → `100_000 * 1e8 = 10_000_000_000_000`.
    pub max_order_notional_1e8: Option<u128>,
    /// Max aggregate open notional per (wallet, subaccount) on this
    /// market. Cross-subaccount notional is not netted.
    pub max_subaccount_notional_1e8: Option<u128>,
    /// Max market-wide open interest, in base units (`1e8`). Sums the
    /// size of every active position across every wallet + subaccount.
    pub max_open_interest_1e8: Option<u128>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PerpsReadConfig {
    /// Master switch. When false, every read endpoint returns
    /// `PerpsReadDisabled` (503).
    pub enabled: bool,
    /// Chain id the reader is bound to. Requests that surface a
    /// different chain id are rejected with `PerpsChainIdMismatch`.
    /// Defaults to Base Sepolia (84532). Mainnet ids are rejected
    /// during `validate_startup`.
    pub chain_id: u64,
    /// JSON-RPC endpoint. When `None`, the reader treats every price
    /// call as `PerpsPriceUnavailable`. Never printed to logs.
    pub rpc_url: Option<String>,
    /// Address of the deployed `PerpMarketRegistry`.
    pub market_registry_address: Option<AccountId>,
    /// Address of the deployed `OracleRouter`.
    pub oracle_router_address: Option<AccountId>,
    /// Rows we surface. Populated from env at startup.
    pub markets: Vec<PerpsReadMarket>,
    /// A price whose `updatedAt` is older than this becomes `stale=true`
    /// on the wire but is still returned. Default 60s (matches the
    /// mock-feed `maxDelay` seen in the base-sepolia deployment).
    pub stale_after_sec: u64,
    /// PERPS-MARGIN-ORACLE-RISK-V1 — max absolute deviation, in bps,
    /// between the trusted `index` price and the mark used for
    /// execution. V1 uses `mark == index` from OracleRouter, so the
    /// guard passes deterministically today; the field is validated at
    /// startup so a future divergence path (e.g. a per-market mark
    /// smoother) rejects anything above threshold rather than silently
    /// letting stale-shape prices settle risk-taking mutations.
    pub oracle_max_deviation_bps: u32,
}

impl PerpsReadConfig {
    /// The safe default: everything disabled, no addresses wired, no
    /// markets. Used by every existing `AppState` builder.
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            chain_id: DEFAULT_BASE_SEPOLIA_CHAIN_ID,
            rpc_url: None,
            market_registry_address: None,
            oracle_router_address: None,
            markets: Vec::new(),
            stale_after_sec: DEFAULT_STALE_AFTER_SEC,
            oracle_max_deviation_bps: DEFAULT_MAX_DEVIATION_BPS,
        }
    }

    /// A test-only preset that turns the reader on with two seeded
    /// markets and no RPC. Callers inject mock readers via the service
    /// signature; the concrete `PerpMarketRegistryRpcReader` /
    /// `PerpOracleRouterRpcReader` never runs in this mode.
    pub fn enabled_in_memory_for_tests() -> Self {
        Self {
            enabled: true,
            chain_id: DEFAULT_BASE_SEPOLIA_CHAIN_ID,
            rpc_url: None,
            market_registry_address: Some(AccountId::new(
                "0xb4fcf45e57b93274441def8f0f68bd30f6d677ec",
            )),
            oracle_router_address: Some(AccountId::new(
                "0xb416406f200b2ef3d7a86a5d5877ed41d9b1a581",
            )),
            markets: vec![
                PerpsReadMarket {
                    symbol: "ETH-PERP".to_string(),
                    onchain_market_id: DEFAULT_ETH_ONCHAIN_MARKET_ID,
                    base_asset_label: "ETH".to_string(),
                    quote_asset_label: "mUSDC".to_string(),
                    base_asset_address: AccountId::new(
                        "0x4deebc5f537f3b8ba0e3393807b4d699d72bdd02",
                    ),
                    quote_asset_address: AccountId::new(
                        "0x6eae407f5640b006fac9965182e238582a3b412e",
                    ),
                    max_leverage: 10,
                    maintenance_margin_bps: 500,
                    // PERPS-MARGIN-ORACLE-RISK-V1 — closed-test caps
                    // for ETH-PERP per the scope doc.
                    max_order_size_1e8: Some(10 * 100_000_000),
                    max_order_notional_1e8: Some(100_000 * 100_000_000),
                    max_subaccount_notional_1e8: Some(500_000 * 100_000_000),
                    max_open_interest_1e8: Some(50 * 100_000_000),
                },
                PerpsReadMarket {
                    symbol: "BTC-PERP".to_string(),
                    onchain_market_id: DEFAULT_BTC_ONCHAIN_MARKET_ID,
                    base_asset_label: "BTC".to_string(),
                    quote_asset_label: "mUSDC".to_string(),
                    base_asset_address: AccountId::new(
                        "0x9d871ac7595e8da271e866608e5145252047967c",
                    ),
                    quote_asset_address: AccountId::new(
                        "0x6eae407f5640b006fac9965182e238582a3b412e",
                    ),
                    max_leverage: 5,
                    maintenance_margin_bps: 750,
                    // BTC-PERP stays deferred in V1; caps set defensively.
                    max_order_size_1e8: Some(100_000_000),
                    max_order_notional_1e8: Some(100_000 * 100_000_000),
                    max_subaccount_notional_1e8: Some(500_000 * 100_000_000),
                    max_open_interest_1e8: Some(5 * 100_000_000),
                },
            ],
            stale_after_sec: DEFAULT_STALE_AFTER_SEC,
            oracle_max_deviation_bps: DEFAULT_MAX_DEVIATION_BPS,
        }
    }

    /// Validate at startup. Returns `Err(BackendError::Config)` on
    /// any misconfiguration that would silently produce nonsense
    /// downstream (e.g. mainnet chain id, RPC missing while enabled,
    /// registry address missing while enabled).
    pub fn validate_startup(&self) -> crate::error::Result<()> {
        // PERPS-MARGIN-ORACLE-RISK-V1 — bounds checks apply even in
        // the disabled config so a dangerous knob (e.g. 0 stale) is
        // rejected regardless of whether the reader is turned on.
        if self.oracle_max_deviation_bps == 0
            || self.oracle_max_deviation_bps > MAX_ALLOWED_DEVIATION_BPS
        {
            return Err(crate::error::BackendError::Config(format!(
                "PERPS_ORACLE_MAX_DEVIATION_BPS must be in (0, {MAX_ALLOWED_DEVIATION_BPS}], \
                 got {}",
                self.oracle_max_deviation_bps
            )));
        }
        if self.stale_after_sec < MIN_STALE_AFTER_SEC || self.stale_after_sec > MAX_STALE_AFTER_SEC
        {
            return Err(crate::error::BackendError::Config(format!(
                "PERPS_ORACLE_STALE_AFTER_SEC must be in [{MIN_STALE_AFTER_SEC}, \
                 {MAX_STALE_AFTER_SEC}], got {}",
                self.stale_after_sec
            )));
        }
        if !self.enabled {
            return Ok(());
        }
        if self.chain_id == 1 || self.chain_id == 8453 {
            return Err(crate::error::BackendError::Config(format!(
                "PERPS_READ_ENABLED must not run against mainnet chain id {} \
                 (this module is Base Sepolia read-only)",
                self.chain_id
            )));
        }
        if self.rpc_url.is_none() {
            return Err(crate::error::BackendError::Config(
                "PERPS_READ_ENABLED=true requires RPC_URL to be set (used by \
                 the market and oracle readers)"
                    .to_string(),
            ));
        }
        if self.market_registry_address.is_none() {
            return Err(crate::error::BackendError::Config(
                "PERPS_READ_ENABLED=true requires PERPS_MARKET_REGISTRY_ADDRESS".to_string(),
            ));
        }
        if self.oracle_router_address.is_none() {
            return Err(crate::error::BackendError::Config(
                "PERPS_READ_ENABLED=true requires PERPS_ORACLE_ROUTER_ADDRESS".to_string(),
            ));
        }
        if self.markets.is_empty() {
            return Err(crate::error::BackendError::Config(
                "PERPS_READ_ENABLED=true requires at least one market row \
                 (PERPS_ETH_MARKET_* or PERPS_BTC_MARKET_*)"
                    .to_string(),
            ));
        }
        for market in &self.markets {
            market.validate_startup()?;
        }
        Ok(())
    }

    /// Look up one seeded market row by human symbol.
    pub fn market_by_symbol(&self, symbol: &str) -> Option<&PerpsReadMarket> {
        self.markets.iter().find(|m| m.symbol == symbol)
    }
}

impl PerpsReadMarket {
    /// PERPS-MARGIN-ORACLE-RISK-V1 — per-market cross-consistency
    /// checks. Called during `PerpsReadConfig::validate_startup`.
    pub fn validate_startup(&self) -> crate::error::Result<()> {
        if self.max_leverage == 0 {
            return Err(crate::error::BackendError::Config(format!(
                "perps market {}: max_leverage must be > 0",
                self.symbol
            )));
        }
        if self.maintenance_margin_bps == 0
            || self.maintenance_margin_bps >= crate::perps::margin::BPS as u32
        {
            return Err(crate::error::BackendError::Config(format!(
                "perps market {}: maintenance_margin_bps must be in (0, {}), got {}",
                self.symbol,
                crate::perps::margin::BPS,
                self.maintenance_margin_bps
            )));
        }
        // Cross-consistency: maintenance margin must be strictly less
        // than initial margin. Otherwise every fresh open is instantly
        // liquidatable, which is a footgun. Initial margin bps at
        // `max_leverage`x is `10_000 / max_leverage`.
        let initial_bps = crate::perps::margin::BPS / (self.max_leverage as u128);
        if (self.maintenance_margin_bps as u128) >= initial_bps {
            return Err(crate::error::BackendError::Config(format!(
                "perps market {}: maintenance_margin_bps {} must be < initial-margin-at-max-leverage bps {} (max_leverage {})",
                self.symbol, self.maintenance_margin_bps, initial_bps, self.max_leverage
            )));
        }
        for (name, cap) in [
            ("max_order_size_1e8", self.max_order_size_1e8),
            ("max_order_notional_1e8", self.max_order_notional_1e8),
            (
                "max_subaccount_notional_1e8",
                self.max_subaccount_notional_1e8,
            ),
            ("max_open_interest_1e8", self.max_open_interest_1e8),
        ] {
            if let Some(0) = cap {
                return Err(crate::error::BackendError::Config(format!(
                    "perps market {}: {} must be > 0 (or None to disable)",
                    self.symbol, name
                )));
            }
        }
        Ok(())
    }
}

/// PERPS-MARGIN-ORACLE-RISK-V1 — market safety status. Composed at
/// read time from `(config, current price snapshot)`. Never persisted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PerpsMarketRiskStatus {
    /// Fresh oracle within deviation threshold; new orders allowed.
    Active,
    /// Oracle mark is stale beyond `stale_after_sec` or unavailable;
    /// new risk-increasing orders reject.
    StaleOracle { reason: &'static str },
    /// Absolute deviation between `index` and `mark` exceeded the
    /// configured `oracle_max_deviation_bps`; new orders reject.
    DeviationExceeded {
        observed_bps: u32,
        threshold_bps: u32,
    },
    /// Reserved — operator-driven pause. Currently only reachable via
    /// tests / the internal admin surface (not yet an env-driven knob
    /// in V1). New orders reject; reduce-only cancel remains allowed.
    Paused,
}

impl PerpsMarketRiskStatus {
    /// True when new risk-increasing orders may proceed. Reduce-only
    /// callers ignore this flag; the fill applicator computes its own
    /// reduce-vs-increase view at commit time.
    pub fn allows_new_risk(&self) -> bool {
        matches!(self, PerpsMarketRiskStatus::Active)
    }

    /// Structured reason string for API/WS surfaces. Stable — pinned
    /// by `perps_margin_oracle_risk_v1_tests.rs` and
    /// `perps_market_status_dto_ws_v1_tests.rs`.
    pub fn reason_code(&self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::StaleOracle { .. } => "stale_oracle",
            Self::DeviationExceeded { .. } => "deviation_exceeded",
            Self::Paused => "paused",
        }
    }
}

// ---------------------------------------------------------------------
// PERPS-MARKET-STATUS-DTO-WS-V1 — public/API-safe status view.
// ---------------------------------------------------------------------

/// Serialized public-safe view of a single Perps market's operational
/// risk status. Returned on `GET /perps/markets` and
/// `GET /perps/markets/:market_id`, and reusable as-is for a future
/// public-market WS status frame (no such WS channel exists yet — the
/// current WS surface is account-scoped only). Never carries RPC URLs,
/// DB URLs, admin tokens, allowlist detail, signatures, nonces, or any
/// internal-only config value.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PerpsMarketRiskStatusView {
    /// Stable string form of the status. One of `"active"`,
    /// `"stale_oracle"`, `"deviation_exceeded"`, `"paused"`,
    /// `"cancel_only"`, or `"disabled"`. Pinned by the test binary
    /// so future consumers can hard-match.
    pub status: String,
    /// Machine-readable reason code. Equals `status` for the simple
    /// statuses; may carry structured context (e.g. deviation
    /// observed vs threshold bps as a `":"`-separated tail — see
    /// `deviation_reason_code_from`).
    pub reason_code: String,
    /// True only when `status == "active"`. New risk-increasing
    /// orders (opens / increases) are gated on this.
    pub allows_new_risk: bool,
    /// True for every status except `"disabled"`. Reduce-only closes
    /// are permitted in stale/deviation/paused/cancel-only because
    /// they can only lower risk.
    pub allows_reduce_only: bool,
    /// True for every status except `"disabled"`. Cancels always
    /// allowed while the market is registered so a trader can exit
    /// a resting order regardless of oracle state.
    pub allows_cancel: bool,
    /// Configured `PERPS_STALE_AFTER_SEC`. Not a secret; exposed so
    /// operator tooling can correlate stale-oracle status with the
    /// configured threshold.
    pub oracle_stale_after_sec: u64,
    /// Configured `PERPS_ORACLE_MAX_DEVIATION_BPS`. Not a secret.
    pub oracle_max_deviation_bps: u32,
    /// Milliseconds since epoch when this snapshot was computed.
    /// Consumers may treat older snapshots as stale for their own
    /// caching purposes.
    pub last_checked_at_ms: TimestampMs,
}

/// PERPS-MARKET-STATUS-DTO-WS-V1 — one oracle read for status
/// computation. `None` means the read failed (RPC error, `ok=false`,
/// or a zero price which we refuse to interpret). `Some(read)` carries
/// the raw price, `updated_at_ms`, and derived staleness flag so the
/// helper never needs to re-consult the config for the threshold.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PerpsMarketOracleSnapshot {
    /// Router-reported index price (`1e8`).
    pub index_price_1e8: u128,
    /// Router-reported mark price. In V1, `mark == index`; a future
    /// per-market mark smoother will diverge this from the index and
    /// the deviation guard will start rejecting above threshold.
    pub mark_price_1e8: u128,
    /// Milliseconds since epoch when the oracle last updated. `0`
    /// when the oracle exposes a legitimate "never updated" state
    /// (which the caller MUST treat as stale — this helper does).
    pub updated_at_ms: TimestampMs,
    /// True when the update timestamp exceeds
    /// `PERPS_STALE_AFTER_SEC`. Callers that already computed
    /// staleness (e.g. via `prefetch_mark_prices`) pass their derived
    /// value here so the helper is fully deterministic given inputs.
    pub is_stale: bool,
}

/// PERPS-MARKET-STATUS-DTO-WS-V1 — manual/admin-driven flags. Neither
/// is env-reachable in V1; the enum variants exist so the compute
/// helper's priority order is complete and a future admin
/// pause/resume endpoint can flip these without a wire break.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PerpsMarketAdminOverride {
    /// When `true` the market is treated as `PerpsMarketRiskStatus::Paused`
    /// regardless of oracle state. Reduce-only closes and cancels are
    /// still allowed on the DTO (`allows_reduce_only`, `allows_cancel`).
    pub paused: bool,
    /// When `true` the market is `cancel_only` — no new risk
    /// (increase or open), reduce-only still allowed, cancels always
    /// allowed. V1 does not surface this yet; reserved for a future
    /// admin toggle.
    pub cancel_only: bool,
    /// When `true` the market is fully `disabled` — nothing allowed.
    /// Distinct from `PerpsReadConfig.enabled == false` (which
    /// disables the entire read layer). This flag is per-market and
    /// currently unreachable from env.
    pub disabled: bool,
}

/// PERPS-MARKET-STATUS-DTO-WS-V1 — compute a public-safe market risk
/// status view for one market at a given moment.
///
/// **Priority order (highest first):**
///
/// 1. `disabled`   — market entirely off (`admin.disabled = true`).
/// 2. `paused`     — operator-driven pause (`admin.paused = true`).
/// 3. `cancel_only` — operator-driven cancel-only mode (`admin.cancel_only = true`).
/// 4. `stale_oracle` — oracle read missing or older than
///    `cfg.stale_after_sec`.
/// 5. `deviation_exceeded` — |index − mark| / index in bps exceeds
///    `cfg.oracle_max_deviation_bps`.
/// 6. `active`     — every other case (fresh oracle, deviation within
///    threshold, no admin override).
///
/// The priority puts `cancel_only` above oracle staleness so that an
/// operator-driven cancel-only status is not masked by a stale
/// oracle: the operator's decision is the ground truth.
///
/// **Reduce-only + cancel policy:**
///
/// * `allows_reduce_only = !disabled` — reduce-only can only lower
///   risk, so it stays enabled in every non-disabled status.
/// * `allows_cancel = !disabled` — cancel is a defensive action a
///   trader must always be able to take.
///
/// **Never uses:** RPC URLs, DB URLs, admin tokens, allowlist, or any
/// value not already surfaced through configured knobs.
pub fn compute_perps_market_risk_status_view(
    cfg: &PerpsReadConfig,
    market: &PerpsReadMarket,
    oracle: Option<PerpsMarketOracleSnapshot>,
    admin: PerpsMarketAdminOverride,
    now: TimestampMs,
) -> PerpsMarketRiskStatusView {
    let status = compute_status(cfg, oracle, admin);
    // Admin overrides take priority for the wire string because the
    // enum's `Paused` variant covers three distinct admin states
    // (`disabled`, `paused`, `cancel_only`). Intercepting here keeps
    // the enum minimal without ambiguity on the wire.
    let (status_str, reason_code) = if admin.disabled {
        ("disabled", "disabled".to_string())
    } else if admin.paused {
        ("paused", "paused".to_string())
    } else if admin.cancel_only {
        ("cancel_only", "cancel_only".to_string())
    } else {
        let (s, r) = status_wire(&status);
        (s, r)
    };
    let allows_reduce_only = !admin.disabled;
    let allows_cancel = !admin.disabled;
    let _ = market; // reserved: per-market override slot
    PerpsMarketRiskStatusView {
        status: status_str.to_string(),
        reason_code,
        allows_new_risk: matches!(status, PerpsMarketRiskStatus::Active)
            && !admin.disabled
            && !admin.paused
            && !admin.cancel_only,
        allows_reduce_only,
        allows_cancel,
        oracle_stale_after_sec: cfg.stale_after_sec,
        oracle_max_deviation_bps: cfg.oracle_max_deviation_bps,
        last_checked_at_ms: now,
    }
}

/// PERPS-MARKET-STATUS-DTO-WS-V1 — the internal compute step,
/// returning the raw enum. Kept public so
/// `compute_perps_market_risk_status_view` and any future admin
/// endpoint can share it without duplicating the priority ladder.
pub fn compute_perps_market_risk_status(
    cfg: &PerpsReadConfig,
    oracle: Option<PerpsMarketOracleSnapshot>,
    admin: PerpsMarketAdminOverride,
) -> PerpsMarketRiskStatus {
    compute_status(cfg, oracle, admin)
}

fn compute_status(
    cfg: &PerpsReadConfig,
    oracle: Option<PerpsMarketOracleSnapshot>,
    admin: PerpsMarketAdminOverride,
) -> PerpsMarketRiskStatus {
    // Priority 1: disabled (per-market admin flag). We surface this
    // as `Paused` in the enum today because the enum has no
    // `Disabled` variant; the wire string is still `"disabled"` via
    // `status_wire`. Keeping the enum minimal is intentional — we do
    // not want an enum variant that no computation path can produce
    // to leak into the public API.
    if admin.disabled {
        return PerpsMarketRiskStatus::Paused;
    }
    // Priority 2: admin-driven pause.
    if admin.paused {
        return PerpsMarketRiskStatus::Paused;
    }
    // Priority 3: cancel-only. We surface as `Paused` at the enum
    // level (same rationale as Disabled) and translate to
    // `"cancel_only"` on the wire.
    if admin.cancel_only {
        return PerpsMarketRiskStatus::Paused;
    }
    // Priority 4: stale oracle. `None` (reader failed / ok=false /
    // zero price) counts as stale.
    let Some(snapshot) = oracle else {
        return PerpsMarketRiskStatus::StaleOracle {
            reason: "oracle_read_unavailable",
        };
    };
    if snapshot.is_stale {
        return PerpsMarketRiskStatus::StaleOracle {
            reason: "oracle_stale",
        };
    }
    // Priority 5: deviation guard. V1 has `mark == index` so this
    // check passes deterministically; the guard is present so a
    // future divergence rejects at threshold.
    if snapshot.index_price_1e8 != 0 {
        let observed = compute_deviation_bps(snapshot.index_price_1e8, snapshot.mark_price_1e8);
        if observed > cfg.oracle_max_deviation_bps {
            return PerpsMarketRiskStatus::DeviationExceeded {
                observed_bps: observed,
                threshold_bps: cfg.oracle_max_deviation_bps,
            };
        }
    }
    // Priority 6: active.
    PerpsMarketRiskStatus::Active
}

/// Absolute deviation between two `1e8`-scaled prices in basis
/// points, relative to `index`. Mirrors the internal
/// `execution::deviation_bps` helper but is decoupled so the config
/// module does not have to `use crate::perps::execution`.
fn compute_deviation_bps(index_1e8: u128, mark_1e8: u128) -> u32 {
    if index_1e8 == 0 {
        return 0;
    }
    let diff = if mark_1e8 >= index_1e8 {
        mark_1e8 - index_1e8
    } else {
        index_1e8 - mark_1e8
    };
    // `bps = diff / index * 10_000`. Saturate at `u32::MAX` for
    // divergence so extreme cases still cross the threshold.
    let bps = diff.saturating_mul(10_000) / index_1e8;
    u32::try_from(bps).unwrap_or(u32::MAX)
}

/// Wire form for the status. Priority-driven translation so the
/// runtime enum stays minimal but the wire string surface is
/// complete. Includes structured context on `deviation_exceeded`.
fn status_wire(status: &PerpsMarketRiskStatus) -> (&'static str, String) {
    match status {
        PerpsMarketRiskStatus::Active => ("active", "active".to_string()),
        PerpsMarketRiskStatus::StaleOracle { reason } => {
            ("stale_oracle", format!("stale_oracle:{reason}"))
        }
        PerpsMarketRiskStatus::DeviationExceeded {
            observed_bps,
            threshold_bps,
        } => (
            "deviation_exceeded",
            format!("deviation_exceeded:observed={observed_bps},threshold={threshold_bps}"),
        ),
        // The enum's `Paused` variant is intentionally re-used for
        // three admin-driven wire strings (`disabled`, `paused`,
        // `cancel_only`) so the internal enum stays minimal. The
        // wire caller MUST use `compute_perps_market_risk_status_view`
        // which decides the wire string from the admin override.
        // Direct enum conversion falls through to `"paused"`.
        PerpsMarketRiskStatus::Paused => ("paused", "paused".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_config_validates_ok() {
        assert!(PerpsReadConfig::disabled().validate_startup().is_ok());
    }

    #[test]
    fn enabled_without_rpc_fails_validation() {
        let mut cfg = PerpsReadConfig::enabled_in_memory_for_tests();
        cfg.rpc_url = None;
        assert!(cfg.validate_startup().is_err());
    }

    #[test]
    fn enabled_on_mainnet_fails_validation() {
        let mut cfg = PerpsReadConfig::enabled_in_memory_for_tests();
        cfg.rpc_url = Some("https://example.invalid".to_string());
        cfg.chain_id = 1;
        assert!(cfg.validate_startup().is_err());
        cfg.chain_id = 8453;
        assert!(cfg.validate_startup().is_err());
    }

    #[test]
    fn enabled_without_market_registry_fails_validation() {
        let mut cfg = PerpsReadConfig::enabled_in_memory_for_tests();
        cfg.rpc_url = Some("https://example.invalid".to_string());
        cfg.market_registry_address = None;
        assert!(cfg.validate_startup().is_err());
    }

    #[test]
    fn seeded_test_config_finds_markets_by_symbol() {
        let cfg = PerpsReadConfig::enabled_in_memory_for_tests();
        assert_eq!(
            cfg.market_by_symbol("ETH-PERP")
                .map(|m| m.onchain_market_id),
            Some(1)
        );
        assert_eq!(
            cfg.market_by_symbol("BTC-PERP")
                .map(|m| m.onchain_market_id),
            Some(2)
        );
        assert!(cfg.market_by_symbol("NOPE-PERP").is_none());
    }
}
