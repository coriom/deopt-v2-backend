//! DEOPT_MULTICHAIN_MULTICOLLATERAL_FOUNDATION_V1 — chain configuration
//! layer.
//!
//! Canonical chain-identity registry. Every chain the protocol may ever
//! be deployed against is represented here with a `ChainConfig` entry;
//! whether the protocol actually *runs* against a given chain is
//! controlled by the `enabled` flag.
//!
//! Runtime posture (V1):
//!   * `BASE_SEPOLIA` (84532) — `enabled = true`. The only production
//!     surface the platform will accept.
//!   * `BASE_MAINNET` (8453) — `enabled = false`. Represented so the
//!     mainnet-refusal guards in `HybridV2Config::validate` etc. have a
//!     canonical entry to reference; adding it to the registry does NOT
//!     activate it.
//!   * `ANVIL` (31337) — `enabled = false`. Local dev only; kept as a
//!     dev convenience, never enabled in a shipping binary.
//!
//! The registry deliberately does NOT hold RPC URLs, deployment
//! addresses, or private-key material. Those live in
//! `AppConfig`/`ExecutionConfig`/`HybridV2Config` and remain reachable
//! from the registry only via the `chain_id` foreign key so a future
//! multi-chain runtime can look them up per-chain.
//!
//! # Non-goals for V1
//!   * No cross-chain economic state. `chain_id` is embedded into every
//!     canonical identity that the platform persists so state on Chain A
//!     can never leak into state on Chain B.
//!   * No cross-chain messaging / bridging.
//!   * No shared collateral between chains.
//!
//! These are enforced by architecture (see
//! `DEOPT_MULTICHAIN_MULTICOLLATERAL_FOUNDATION_V1.md`) and by the
//! `enabled` flag being singular.

use std::fmt;

/// Well-known chain identifier for Base mainnet. The platform refuses
/// to accept this chain at every runtime gate.
pub const BASE_MAINNET_CHAIN_ID: u64 = 8453;

/// Well-known chain identifier for Base Sepolia — the sole enabled
/// chain for V1.
pub const BASE_SEPOLIA_CHAIN_ID: u64 = 84532;

/// Well-known chain identifier for local Anvil dev nodes.
pub const ANVIL_CHAIN_ID: u64 = 31337;

/// Well-known chain identifier for Ethereum mainnet. Kept as a
/// refused-value constant so mainnet gates have a canonical name.
pub const ETHEREUM_MAINNET_CHAIN_ID: u64 = 1;

/// Finality policy — how the runtime treats block confirmations.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FinalityPolicy {
    /// Depth-based finality. The runtime waits for N confirmations
    /// before treating a block as final.
    ConfirmationDepth(u32),
}

/// Oracle policy — which oracle families the runtime expects to be
/// available on a chain. The registry does NOT hold feed addresses.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OraclePolicy {
    /// Dual-source: primary + secondary with deviation cap. Matches
    /// the on-chain `OracleRouter` dual-source invariant.
    DualSource,
    /// No oracle expected — used for local Anvil dev where oracles
    /// are mocked or absent.
    None,
}

/// Canonical chain configuration entry.
///
/// Byte layout is stable — adding fields must be additive so the
/// registry lookup helpers do not break. `enabled` is intentionally
/// the *last* semantic gate; it must be explicit rather than derived.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChainConfig {
    pub chain_id: u64,
    pub name: &'static str,
    pub short_name: &'static str,
    /// Human-facing settlement asset symbol (e.g. "USDC"). Multiple
    /// chains may share the same symbol; the actual token address per
    /// chain lives in `CollateralConfig`.
    pub settlement_asset_symbol: &'static str,
    /// Testnet vs. mainnet — informational and used by belt-and-
    /// suspenders mainnet refusal gates.
    pub is_testnet: bool,
    /// Block-explorer URL prefix, e.g. `https://sepolia.basescan.org`.
    /// Optional so local chains without an explorer can omit it.
    pub explorer_url: Option<&'static str>,
    pub finality_policy: FinalityPolicy,
    pub oracle_policy: OraclePolicy,
    /// V1 posture flag. Only Base Sepolia is enabled. Adding a new
    /// enabled chain is an explicit operator + release-manager
    /// decision, never a silent default.
    pub enabled: bool,
}

impl ChainConfig {
    /// True iff this chain is the sole production surface the
    /// platform will accept in V1.
    #[inline]
    pub fn is_base_sepolia(&self) -> bool {
        self.chain_id == BASE_SEPOLIA_CHAIN_ID
    }

    /// True iff this chain is any known mainnet the platform refuses.
    #[inline]
    pub fn is_refused_mainnet(&self) -> bool {
        matches!(
            self.chain_id,
            BASE_MAINNET_CHAIN_ID | ETHEREUM_MAINNET_CHAIN_ID
        )
    }
}

impl fmt::Display for ChainConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}({})", self.name, self.chain_id)
    }
}

/// Base Sepolia — the only chain enabled in V1.
pub const BASE_SEPOLIA: ChainConfig = ChainConfig {
    chain_id: BASE_SEPOLIA_CHAIN_ID,
    name: "Base Sepolia",
    short_name: "sepolia",
    settlement_asset_symbol: "USDC",
    is_testnet: true,
    explorer_url: Some("https://sepolia.basescan.org"),
    finality_policy: FinalityPolicy::ConfirmationDepth(1),
    oracle_policy: OraclePolicy::DualSource,
    enabled: true,
};

/// Base mainnet — represented so refusal guards can name it, but
/// permanently `enabled = false` in this binary.
pub const BASE_MAINNET: ChainConfig = ChainConfig {
    chain_id: BASE_MAINNET_CHAIN_ID,
    name: "Base mainnet",
    short_name: "mainnet",
    settlement_asset_symbol: "USDC",
    is_testnet: false,
    explorer_url: Some("https://basescan.org"),
    finality_policy: FinalityPolicy::ConfirmationDepth(3),
    oracle_policy: OraclePolicy::DualSource,
    enabled: false,
};

/// Local Anvil — dev convenience, never enabled in shipping binaries.
pub const ANVIL: ChainConfig = ChainConfig {
    chain_id: ANVIL_CHAIN_ID,
    name: "Anvil (local)",
    short_name: "anvil",
    settlement_asset_symbol: "USDC",
    is_testnet: true,
    explorer_url: None,
    finality_policy: FinalityPolicy::ConfirmationDepth(1),
    oracle_policy: OraclePolicy::None,
    enabled: false,
};

/// Ordered registry of every chain the codebase knows about.
///
/// Order is stable and load-bearing for the round-trip helpers.
pub const KNOWN_CHAINS: &[ChainConfig] = &[BASE_SEPOLIA, BASE_MAINNET, ANVIL];

/// Look up a chain by its numeric id. Returns `None` for unknown
/// chains — callers must decide whether unknown means "refuse" or
/// "treat as informational".
pub fn find_chain(chain_id: u64) -> Option<&'static ChainConfig> {
    KNOWN_CHAINS.iter().find(|c| c.chain_id == chain_id)
}

/// List every chain currently marked `enabled = true`. For V1 this
/// always returns exactly `[BASE_SEPOLIA]`.
pub fn enabled_chains() -> Vec<&'static ChainConfig> {
    KNOWN_CHAINS.iter().filter(|c| c.enabled).collect()
}

/// Assert the V1 invariant: exactly one chain is enabled, and it is
/// Base Sepolia. Called at startup to catch accidental registry drift.
pub fn assert_v1_single_chain_invariant() -> Result<(), &'static str> {
    let enabled: Vec<&ChainConfig> = enabled_chains();
    if enabled.len() != 1 {
        return Err("chain registry V1 invariant: exactly one chain must be enabled");
    }
    if enabled[0].chain_id != BASE_SEPOLIA_CHAIN_ID {
        return Err("chain registry V1 invariant: enabled chain must be Base Sepolia");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_sepolia_is_the_only_enabled_chain() {
        let enabled = enabled_chains();
        assert_eq!(enabled.len(), 1, "V1 must enable exactly one chain");
        assert_eq!(enabled[0].chain_id, BASE_SEPOLIA_CHAIN_ID);
        assert!(enabled[0].is_base_sepolia());
        assert!(!enabled[0].is_refused_mainnet());
    }

    #[test]
    fn base_mainnet_is_present_but_refused() {
        let cfg = find_chain(BASE_MAINNET_CHAIN_ID).expect("base mainnet must be in registry");
        assert!(!cfg.enabled, "base mainnet must never be enabled in V1");
        assert!(cfg.is_refused_mainnet());
    }

    #[test]
    fn ethereum_mainnet_is_refused_even_if_absent_from_registry() {
        // Ethereum mainnet is not in the registry today, but the
        // refusal helper on `ChainConfig` still classifies it as a
        // refused mainnet if a chain entry with id 1 were ever added.
        let synthetic = ChainConfig {
            chain_id: ETHEREUM_MAINNET_CHAIN_ID,
            name: "Ethereum",
            short_name: "eth",
            settlement_asset_symbol: "USDC",
            is_testnet: false,
            explorer_url: None,
            finality_policy: FinalityPolicy::ConfirmationDepth(12),
            oracle_policy: OraclePolicy::DualSource,
            enabled: false,
        };
        assert!(synthetic.is_refused_mainnet());
    }

    #[test]
    fn v1_invariant_holds() {
        assert!(assert_v1_single_chain_invariant().is_ok());
    }

    #[test]
    fn unknown_chain_lookup_returns_none() {
        assert!(find_chain(999_999).is_none());
    }
}
