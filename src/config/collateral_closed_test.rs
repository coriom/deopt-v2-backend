//! DEOPT_WETH_COLLATERAL_CLOSED_TEST_V1 — closed-test-only collateral
//! configuration overlay.
//!
//! This module exposes a `WETH_CLOSED_TEST` collateral configuration
//! that is deliberately activated ONLY inside disposable local /
//! closed-test environments. It exists so an operator running the
//! backend against a local Anvil (or a disposable Base Sepolia
//! closed-test lane) can prove the multi-collateral runtime path
//! end-to-end without ever touching the production `WETH` constant
//! in [`crate::config::collateral`] (which stays inert per the V1
//! posture).
//!
//! # Activation gate
//!
//! The overlay is inert unless the caller has explicitly opted in
//! via `is_closed_test_multicollateral_enabled()`, which reads the
//! `MULTICOLLATERAL_CLOSED_TEST_ENABLED=true` env var. The gate is
//! evaluated in exactly two places:
//!   * `active_collateral_registry()` — the runtime accessor every
//!     downstream call site must use instead of iterating
//!     `KNOWN_COLLATERAL` directly.
//!   * Startup validation, so a mistakenly-set flag on a production
//!     binary causes an audible refusal, not a silent activation.
//!
//! # Parameters
//!
//! `WETH_CLOSED_TEST` uses deliberately conservative TEST parameters
//! (`collateral_factor_bps = 8_000`, `liquidation_factor_bps =
//! 8_500`) inspired by the safe bounds in
//! `DEOPT_WETH_COLLATERAL_ACTIVATION_DESIGN_V1.md`. These values are
//! **NOT** production recommendations — they exist only so a closed-
//! test can exercise the risk-adjusted valuation code path with
//! plausible numbers.
//!
//! # Posture invariants
//!
//! * Production `WETH` in `crate::config::collateral` REMAINS
//!   `deposit_enabled = false, collateral_factor_bps = 0`.
//! * Production `assert_v1_single_collateral_invariant()` REMAINS
//!   passing.
//! * A binary that has NOT set `MULTICOLLATERAL_CLOSED_TEST_ENABLED=true`
//!   sees exactly the same collateral universe it saw before this
//!   module existed.
//! * A binary that HAS set the flag on a non-test build refuses at
//!   startup via `refuse_closed_test_on_forbidden_chain(chain_id)`.

use crate::config::chains::{BASE_MAINNET_CHAIN_ID, ETHEREUM_MAINNET_CHAIN_ID};
use crate::config::collateral::{
    CollateralConfig, COLLATERAL_FACTOR_BPS_MAX, KNOWN_COLLATERAL, WETH_SYMBOL,
};
use std::env;

/// Env var whose truthy value activates the closed-test multi-
/// collateral overlay. Any of `"true"`, `"1"`, `"yes"` (case-
/// insensitive) counts as opt-in; every other value (including
/// unset) means the overlay is inert.
pub const CLOSED_TEST_ENV_VAR: &str = "MULTICOLLATERAL_CLOSED_TEST_ENABLED";

/// Env var that OVERRIDES the closed-test WETH deposit cap (raw
/// units, 18 decimals). `None` when unset. Defaults to 1_000 WETH
/// (a plausible closed-test cap).
pub const CLOSED_TEST_WETH_CAP_ENV_VAR: &str = "MULTICOLLATERAL_CLOSED_TEST_WETH_DEPOSIT_CAP_1E18";

/// Default WETH deposit cap for closed test: 1_000 WETH in raw 1e18
/// units. Chosen so a closed-test operator can move meaningful
/// notional without being able to move production-scale value.
pub const CLOSED_TEST_WETH_DEFAULT_CAP_1E18: u128 = 1_000 * (10u128.pow(18));

/// TEST-ONLY WETH collateral configuration. Distinct from
/// `crate::config::collateral::WETH` (which stays inert).
///
/// Not a production parameter set. See module docs.
pub const WETH_CLOSED_TEST: CollateralConfig = CollateralConfig {
    asset_symbol: WETH_SYMBOL,
    decimals: 18,
    collateral_factor_bps: 8_000,
    liquidation_factor_bps: 8_500,
    deposit_enabled: true,
    withdrawal_enabled: true,
};

/// Read the opt-in flag from the environment. Truthy = `"true"` /
/// `"1"` / `"yes"` (case-insensitive). Anything else = false.
pub fn is_closed_test_multicollateral_enabled() -> bool {
    match env::var(CLOSED_TEST_ENV_VAR) {
        Ok(v) => matches!(v.trim().to_ascii_lowercase().as_str(), "true" | "1" | "yes"),
        Err(_) => false,
    }
}

/// Refuse the closed-test overlay on any production-shaped chain.
/// Called at startup so a mainnet binary with the flag mistakenly
/// set fails loudly. Base Sepolia (84532) is ALSO refused because
/// the milestone explicitly forbids activating WETH on Base Sepolia
/// during this closed test — the operator must target a disposable
/// local chain (Anvil, or a scratch fork).
pub fn refuse_closed_test_on_forbidden_chain(chain_id: u64) -> Result<(), String> {
    if !is_closed_test_multicollateral_enabled() {
        return Ok(());
    }
    match chain_id {
        BASE_MAINNET_CHAIN_ID | ETHEREUM_MAINNET_CHAIN_ID => Err(format!(
            "{}=true refused on mainnet chain id {} — closed-test WETH must run against a local disposable chain",
            CLOSED_TEST_ENV_VAR, chain_id
        )),
        // 84532 = Base Sepolia. Milestone forbids activating WETH here.
        84532 => Err(format!(
            "{}=true refused on Base Sepolia (84532) — the WETH closed test must target a local disposable chain, not the shared testnet",
            CLOSED_TEST_ENV_VAR
        )),
        _ => Ok(()),
    }
}

/// Returns the active collateral registry given the current
/// environment. In production posture this is exactly
/// `KNOWN_COLLATERAL`; in closed-test posture it appends
/// `WETH_CLOSED_TEST` as an additional active entry (the production
/// inert WETH entry is retained but never activated by this path).
pub fn active_collateral_registry() -> Vec<&'static CollateralConfig> {
    let mut out: Vec<&'static CollateralConfig> = KNOWN_COLLATERAL.iter().collect();
    if is_closed_test_multicollateral_enabled() {
        out.push(&WETH_CLOSED_TEST);
    }
    out
}

/// Same accessor as `active_collateral_registry()` but returns only
/// entries that are `deposit_enabled = true`. Downstream deposit /
/// withdraw / margin call sites should iterate this list, never
/// hard-code a symbol.
pub fn active_enabled_collateral() -> Vec<&'static CollateralConfig> {
    active_collateral_registry()
        .into_iter()
        .filter(|c| c.deposit_enabled)
        .collect()
}

/// Read the closed-test WETH deposit cap. Returns
/// `CLOSED_TEST_WETH_DEFAULT_CAP_1E18` if the env var is unset or
/// invalid. `None` when the closed-test overlay is not active.
pub fn closed_test_weth_deposit_cap_1e18() -> Option<u128> {
    if !is_closed_test_multicollateral_enabled() {
        return None;
    }
    let cap = env::var(CLOSED_TEST_WETH_CAP_ENV_VAR)
        .ok()
        .and_then(|s| s.parse::<u128>().ok())
        .unwrap_or(CLOSED_TEST_WETH_DEFAULT_CAP_1E18);
    Some(cap)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // Test helper — env-var manipulation across tests. Each test that
    // touches the env var takes this lock so tests don't race.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn with_flag_set<T>(value: &str, f: impl FnOnce() -> T) -> T {
        let _g = ENV_LOCK.lock().unwrap();
        let prev = env::var(CLOSED_TEST_ENV_VAR).ok();
        // SAFETY: env var mutation is safe within our serialised
        // test-only block.
        unsafe {
            env::set_var(CLOSED_TEST_ENV_VAR, value);
        }
        let out = f();
        unsafe {
            match prev {
                Some(v) => env::set_var(CLOSED_TEST_ENV_VAR, v),
                None => env::remove_var(CLOSED_TEST_ENV_VAR),
            }
        }
        out
    }

    fn with_flag_unset<T>(f: impl FnOnce() -> T) -> T {
        let _g = ENV_LOCK.lock().unwrap();
        let prev = env::var(CLOSED_TEST_ENV_VAR).ok();
        unsafe {
            env::remove_var(CLOSED_TEST_ENV_VAR);
        }
        let out = f();
        unsafe {
            if let Some(v) = prev {
                env::set_var(CLOSED_TEST_ENV_VAR, v);
            }
        }
        out
    }

    #[test]
    fn overlay_inert_by_default() {
        with_flag_unset(|| {
            assert!(!is_closed_test_multicollateral_enabled());
            let registry = active_collateral_registry();
            assert_eq!(registry.len(), KNOWN_COLLATERAL.len());
            // Only USDC is enabled — matches production V1 posture.
            let enabled = active_enabled_collateral();
            assert_eq!(enabled.len(), 1);
            assert_eq!(enabled[0].asset_symbol, "USDC");
        });
    }

    #[test]
    fn overlay_active_when_flag_true() {
        with_flag_set("true", || {
            assert!(is_closed_test_multicollateral_enabled());
            let enabled = active_enabled_collateral();
            assert_eq!(enabled.len(), 2, "USDC + WETH_CLOSED_TEST");
            let symbols: Vec<&str> = enabled.iter().map(|c| c.asset_symbol).collect();
            assert!(symbols.contains(&"USDC"));
            assert!(symbols.contains(&"WETH"));
        });
    }

    #[test]
    fn flag_accepts_multiple_truthy_forms() {
        for v in ["true", "TRUE", "True", "1", "yes", "YES"] {
            with_flag_set(v, || {
                assert!(
                    is_closed_test_multicollateral_enabled(),
                    "value {} should activate",
                    v
                );
            });
        }
    }

    #[test]
    fn flag_rejects_ambiguous_values() {
        for v in ["", "0", "no", "false", "maybe", "on"] {
            with_flag_set(v, || {
                assert!(
                    !is_closed_test_multicollateral_enabled(),
                    "value '{}' must NOT activate",
                    v
                );
            });
        }
    }

    #[test]
    fn refuse_closed_test_on_base_mainnet() {
        with_flag_set("true", || {
            let err = refuse_closed_test_on_forbidden_chain(BASE_MAINNET_CHAIN_ID)
                .expect_err("must refuse mainnet");
            assert!(err.contains("mainnet"));
        });
    }

    #[test]
    fn refuse_closed_test_on_base_sepolia() {
        with_flag_set("true", || {
            let err = refuse_closed_test_on_forbidden_chain(84532)
                .expect_err("must refuse base sepolia");
            assert!(err.contains("Base Sepolia"));
        });
    }

    #[test]
    fn allow_closed_test_on_local_anvil() {
        with_flag_set("true", || {
            refuse_closed_test_on_forbidden_chain(31337)
                .expect("anvil is a legitimate closed-test target");
        });
    }

    #[test]
    fn refusal_is_a_noop_when_flag_unset() {
        with_flag_unset(|| {
            for cid in [BASE_MAINNET_CHAIN_ID, 84532, 31337, 1] {
                refuse_closed_test_on_forbidden_chain(cid)
                    .expect("no refusal when flag not set");
            }
        });
    }

    #[test]
    fn production_weth_stays_inert() {
        // Even with the flag on, the production `WETH` constant
        // MUST remain deposit-disabled. The closed-test overlay
        // appends a SEPARATE `WETH_CLOSED_TEST` entry with the same
        // symbol.
        assert!(!crate::config::collateral::WETH.deposit_enabled);
        assert_eq!(crate::config::collateral::WETH.collateral_factor_bps, 0);
    }

    #[test]
    fn closed_test_weth_has_conservative_test_factors() {
        assert_eq!(WETH_CLOSED_TEST.decimals, 18);
        assert_eq!(WETH_CLOSED_TEST.collateral_factor_bps, 8_000);
        assert_eq!(WETH_CLOSED_TEST.liquidation_factor_bps, 8_500);
        // Never 100% — a haircut is required for the closed test.
        assert!(WETH_CLOSED_TEST.collateral_factor_bps < COLLATERAL_FACTOR_BPS_MAX);
    }

    #[test]
    fn deposit_cap_defaults_to_1000_weth() {
        with_flag_set("true", || {
            let cap = closed_test_weth_deposit_cap_1e18().unwrap();
            assert_eq!(cap, CLOSED_TEST_WETH_DEFAULT_CAP_1E18);
            assert_eq!(cap, 1_000u128 * 10u128.pow(18));
        });
    }

    #[test]
    fn deposit_cap_none_when_overlay_off() {
        with_flag_unset(|| {
            assert!(closed_test_weth_deposit_cap_1e18().is_none());
        });
    }
}
