//! Bounded gas + fee policy for Hybrid V2 execution (Part J of
//! `BACKEND-HYBRID-V2-SIGNER-AND-EXECUTION-V1`).
//!
//! Every derived quantity is bounded by an operator-supplied ceiling.
//! Overflow-checked integer arithmetic only — no floating-point in the
//! critical path. Zero estimates and obviously-anomalous fee history
//! entries fail closed.

use crate::hybrid_v2::execution::rpc::FeeHistory;
use crate::hybrid_v2::execution::simulator::SimulationOutcome;
use alloy_primitives::U256;
use thiserror::Error;

/// Static gas / fee policy. All fields are validated at startup via
/// [`GasFeePolicy::validate_config_startup`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GasFeePolicy {
    /// Absolute upper bound on gas_limit — never exceeded regardless
    /// of what the estimator returns.
    pub max_gas_limit: u64,
    /// Multiplier applied to `estimate` to derive gas_limit, in basis
    /// points. `12000` = 1.20x, `10000` = 1.00x.
    pub gas_limit_multiplier_bps: u64,
    pub max_fee_per_gas_wei: U256,
    pub max_priority_fee_per_gas_wei: U256,
    pub max_total_native_cost_wei: U256,
    /// If a provider's max-base-fee sample is more than this multiple
    /// of the median sample, reject as `ProviderFeeAnomaly`. `10` is
    /// the recommended default (10x median).
    pub abnormal_estimate_reject_threshold: u64,
}

/// Concrete gas + fee computation output. Fed to the signer + RPC.
#[derive(Debug, Clone, PartialEq)]
pub struct GasComputationOutcome {
    pub gas_limit: u64,
    pub max_fee_per_gas_wei: U256,
    pub max_priority_fee_per_gas_wei: U256,
    /// Bounded upper estimate `gas_limit * max_fee_per_gas`.
    pub estimated_total_cost_wei: U256,
}

#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum GasPolicyError {
    #[error("estimate {estimate} exceeds ceiling {ceiling}")]
    EstimateExceedsCeiling { estimate: u64, ceiling: u64 },
    #[error("estimate must not be zero")]
    EstimateZero,
    #[error("estimate multiplication overflowed u64")]
    EstimateOverflow,
    #[error("computed fee cap {computed} exceeds ceiling {ceiling}")]
    FeeCapExceeded { computed: U256, ceiling: U256 },
    #[error("computed total cost {computed} exceeds ceiling {ceiling}")]
    TotalCostExceeded { computed: U256, ceiling: U256 },
    #[error("abnormal estimate {estimate} exceeds threshold {threshold}")]
    AbnormalEstimate { estimate: u64, threshold: u64 },
    #[error("provider fee anomaly: {0}")]
    ProviderFeeAnomaly(String),
    #[error("gas policy misconfigured: {0}")]
    Misconfigured(String),
    #[error("simulation did not succeed, cannot compute gas")]
    SimulationNotSuccessful,
}

impl GasFeePolicy {
    /// Startup sanity check — rejects obviously-broken configuration
    /// (zero ceilings, missing multiplier, absurd fee caps).
    pub fn validate_config_startup(&self) -> Result<(), GasPolicyError> {
        if self.max_gas_limit == 0 {
            return Err(GasPolicyError::Misconfigured("max_gas_limit == 0".into()));
        }
        if self.gas_limit_multiplier_bps == 0 {
            return Err(GasPolicyError::Misconfigured(
                "gas_limit_multiplier_bps == 0".into(),
            ));
        }
        if self.gas_limit_multiplier_bps < 10_000 {
            return Err(GasPolicyError::Misconfigured(
                "gas_limit_multiplier_bps < 10000 (would shrink estimate)".into(),
            ));
        }
        if self.gas_limit_multiplier_bps > 100_000 {
            return Err(GasPolicyError::Misconfigured(
                "gas_limit_multiplier_bps > 100000 (>10x)".into(),
            ));
        }
        if self.max_fee_per_gas_wei == U256::ZERO {
            return Err(GasPolicyError::Misconfigured(
                "max_fee_per_gas_wei == 0".into(),
            ));
        }
        if self.max_priority_fee_per_gas_wei > self.max_fee_per_gas_wei {
            return Err(GasPolicyError::Misconfigured(
                "max_priority_fee_per_gas_wei > max_fee_per_gas_wei".into(),
            ));
        }
        if self.max_total_native_cost_wei == U256::ZERO {
            return Err(GasPolicyError::Misconfigured(
                "max_total_native_cost_wei == 0".into(),
            ));
        }
        if self.abnormal_estimate_reject_threshold < 2 {
            return Err(GasPolicyError::Misconfigured(
                "abnormal_estimate_reject_threshold too small (< 2)".into(),
            ));
        }
        Ok(())
    }

    /// Derive the final gas + fee triple from the simulation outcome
    /// and a recent `eth_feeHistory` sample.
    pub fn compute(
        &self,
        sim: &SimulationOutcome,
        recent_fee_history: &FeeHistory,
    ) -> Result<GasComputationOutcome, GasPolicyError> {
        if !sim.eth_call_ok {
            return Err(GasPolicyError::SimulationNotSuccessful);
        }
        if sim.gas_estimate == 0 {
            return Err(GasPolicyError::EstimateZero);
        }

        // gas_limit = ceil(estimate * multiplier_bps / 10_000)
        // with checked mul + div.
        let scaled = (sim.gas_estimate as u128)
            .checked_mul(self.gas_limit_multiplier_bps as u128)
            .ok_or(GasPolicyError::EstimateOverflow)?;
        let rounded_up = scaled
            .checked_add(9_999)
            .ok_or(GasPolicyError::EstimateOverflow)?
            / 10_000u128;
        if rounded_up > u64::MAX as u128 {
            return Err(GasPolicyError::EstimateOverflow);
        }
        let gas_limit = rounded_up as u64;
        if gas_limit > self.max_gas_limit {
            return Err(GasPolicyError::EstimateExceedsCeiling {
                estimate: gas_limit,
                ceiling: self.max_gas_limit,
            });
        }

        // Fee history → median base fee.
        let median_base = median(&recent_fee_history.base_fee_per_gas);
        // Reject if the newest sample is more than N × median (provider
        // anomaly). Newest = last element.
        if let Some(newest) = recent_fee_history.base_fee_per_gas.last() {
            if median_base > U256::ZERO {
                let ratio_bound = median_base
                    .checked_mul(U256::from(self.abnormal_estimate_reject_threshold))
                    .ok_or_else(|| {
                        GasPolicyError::ProviderFeeAnomaly(
                            "median*threshold overflowed".to_string(),
                        )
                    })?;
                if *newest > ratio_bound {
                    return Err(GasPolicyError::ProviderFeeAnomaly(format!(
                        "newest base_fee {newest} > threshold*{}xmedian {ratio_bound}",
                        self.abnormal_estimate_reject_threshold
                    )));
                }
            }
        }

        // priority fee = min(median-reward, ceiling). Fallback to a
        // conservative flat priority if reward is empty.
        let priority = median_reward(&recent_fee_history.reward)
            .unwrap_or_else(|| self.max_priority_fee_per_gas_wei.min(U256::from(1u64)));
        let priority_capped = priority.min(self.max_priority_fee_per_gas_wei);

        // max_fee = min(base + priority, max_fee_per_gas_wei)
        let base_plus_priority = median_base.checked_add(priority_capped).ok_or_else(|| {
            GasPolicyError::ProviderFeeAnomaly("base+priority overflow".to_string())
        })?;
        let max_fee = base_plus_priority.min(self.max_fee_per_gas_wei);
        if max_fee > self.max_fee_per_gas_wei {
            return Err(GasPolicyError::FeeCapExceeded {
                computed: max_fee,
                ceiling: self.max_fee_per_gas_wei,
            });
        }

        // total_cost = gas_limit * max_fee.
        let total = U256::from(gas_limit)
            .checked_mul(max_fee)
            .ok_or_else(|| GasPolicyError::ProviderFeeAnomaly("gas*fee overflow".to_string()))?;
        if total > self.max_total_native_cost_wei {
            return Err(GasPolicyError::TotalCostExceeded {
                computed: total,
                ceiling: self.max_total_native_cost_wei,
            });
        }

        Ok(GasComputationOutcome {
            gas_limit,
            max_fee_per_gas_wei: max_fee,
            max_priority_fee_per_gas_wei: priority_capped,
            estimated_total_cost_wei: total,
        })
    }
}

fn median(xs: &[U256]) -> U256 {
    if xs.is_empty() {
        return U256::ZERO;
    }
    let mut v = xs.to_vec();
    v.sort_unstable();
    v[v.len() / 2]
}

fn median_reward(rewards: &[Vec<U256>]) -> Option<U256> {
    // Take the first-percentile column from each block (the low
    // reward), median it across blocks. Empty → None.
    let column: Vec<U256> = rewards
        .iter()
        .filter_map(|row| row.first().copied())
        .collect();
    if column.is_empty() {
        None
    } else {
        Some(median(&column))
    }
}

// -----------------------------------------------------------------
//                          UNIT TESTS
// -----------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn policy() -> GasFeePolicy {
        GasFeePolicy {
            max_gas_limit: 5_000_000,
            gas_limit_multiplier_bps: 12_000,
            max_fee_per_gas_wei: U256::from(50_000_000_000u64), // 50 gwei
            max_priority_fee_per_gas_wei: U256::from(2_000_000_000u64), // 2 gwei
            max_total_native_cost_wei: U256::from(10u64).pow(U256::from(18u64)), // 1 ETH
            abnormal_estimate_reject_threshold: 10,
        }
    }

    fn sim(gas: u64) -> SimulationOutcome {
        SimulationOutcome {
            simulation_block_number: 1,
            simulation_block_hash: [0; 32],
            gas_estimate: gas,
            eth_call_ok: true,
            revert: None,
            simulated_at_ms: 0,
        }
    }

    fn history() -> FeeHistory {
        FeeHistory {
            oldest_block: 0,
            base_fee_per_gas: vec![
                U256::from(1_000_000_000u64),
                U256::from(1_100_000_000u64),
                U256::from(1_050_000_000u64),
            ],
            reward: vec![
                vec![U256::from(500_000_000u64)],
                vec![U256::from(600_000_000u64)],
                vec![U256::from(550_000_000u64)],
            ],
            gas_used_ratio: vec![0.5, 0.6, 0.55],
        }
    }

    #[test]
    fn happy_path() {
        let out = policy().compute(&sim(100_000), &history()).unwrap();
        assert_eq!(out.gas_limit, 120_000); // 100_000 * 12000 / 10000
        assert!(out.max_fee_per_gas_wei <= policy().max_fee_per_gas_wei);
        assert!(out.estimated_total_cost_wei > U256::ZERO);
    }

    #[test]
    fn startup_validate_catches_zero_ceiling() {
        let mut p = policy();
        p.max_gas_limit = 0;
        assert!(p.validate_config_startup().is_err());

        let mut p = policy();
        p.gas_limit_multiplier_bps = 0;
        assert!(p.validate_config_startup().is_err());

        let mut p = policy();
        p.gas_limit_multiplier_bps = 9_999;
        assert!(p.validate_config_startup().is_err());

        let mut p = policy();
        p.max_fee_per_gas_wei = U256::ZERO;
        assert!(p.validate_config_startup().is_err());

        let mut p = policy();
        p.abnormal_estimate_reject_threshold = 1;
        assert!(p.validate_config_startup().is_err());
    }

    #[test]
    fn zero_estimate_rejected() {
        assert!(matches!(
            policy().compute(&sim(0), &history()),
            Err(GasPolicyError::EstimateZero)
        ));
    }

    #[test]
    fn overflow_rejected() {
        let mut p = policy();
        p.gas_limit_multiplier_bps = 100_000; // 10x
        let err = p.compute(&sim(u64::MAX), &history()).unwrap_err();
        assert!(matches!(err, GasPolicyError::EstimateOverflow));
    }

    #[test]
    fn ceiling_enforced() {
        // 500_000 * 1.20 = 600_000; ceiling 100 → reject.
        let mut p = policy();
        p.max_gas_limit = 100_000;
        let err = p.compute(&sim(500_000), &history()).unwrap_err();
        assert!(matches!(err, GasPolicyError::EstimateExceedsCeiling { .. }));
    }

    #[test]
    fn abnormal_provider_fee_rejected() {
        let mut h = history();
        // Push a base fee 100x larger than the median → provider anomaly.
        h.base_fee_per_gas.push(U256::from(500_000_000_000u64)); // 500 gwei
                                                                 // Also raise ceiling so it's the anomaly check that fires, not the cap.
        let mut p = policy();
        p.max_fee_per_gas_wei = U256::from(10_000_000_000_000u64);
        let err = p.compute(&sim(100_000), &h).unwrap_err();
        assert!(matches!(err, GasPolicyError::ProviderFeeAnomaly(_)));
    }

    #[test]
    fn simulation_failure_refused() {
        let bad_sim = SimulationOutcome {
            simulation_block_number: 1,
            simulation_block_hash: [0; 32],
            gas_estimate: 100_000,
            eth_call_ok: false,
            revert: None,
            simulated_at_ms: 0,
        };
        assert!(matches!(
            policy().compute(&bad_sim, &history()),
            Err(GasPolicyError::SimulationNotSuccessful)
        ));
    }

    #[test]
    fn total_cost_ceiling_enforced() {
        let mut p = policy();
        // Set the total cost ceiling absurdly small.
        p.max_total_native_cost_wei = U256::from(1u64);
        let err = p.compute(&sim(100_000), &history()).unwrap_err();
        assert!(matches!(err, GasPolicyError::TotalCostExceeded { .. }));
    }
}
