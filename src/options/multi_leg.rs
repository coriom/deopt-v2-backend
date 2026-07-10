//! RFQ-MULTI-LEG-SCHEMA-V1 — domain types for multi-leg atomic RFQ.
//!
//! Foundation-only. This module intentionally does NOT expose any HTTP
//! route, service function, EIP-712 canonical, or lifecycle payload.
//! It ships the structs + status enum re-exports + leg-count bounds
//! that the follow-up milestones (`RFQ-MULTI-LEG-CREATE-QUOTE-V1`,
//! `_ATOMIC-ACCEPT-V1`, `_FRONTEND-V1`, `_MM-GATEWAY-V1`,
//! `_READINESS-V1`) will build on.
//!
//! The 6 structs mirror the shape of the DB tables in
//! `migrations/0043_option_multi_leg_rfqs.sql`:
//!
//! * `OptionMultiLegRfqRequest`   / row of `option_multi_leg_rfqs`
//! * `OptionMultiLegRfqLeg`       / row of `option_multi_leg_rfq_legs`
//! * `OptionMultiLegRfqQuote`     / row of `option_multi_leg_rfq_quotes`
//! * `OptionMultiLegRfqQuoteLeg`  / row of `option_multi_leg_rfq_quote_legs`
//! * `OptionMultiLegRfqFill`      / row of `option_multi_leg_rfq_fills`
//! * `OptionMultiLegRfqFillLeg`   / row of `option_multi_leg_rfq_fill_legs`
//!
//! Status enums (`OptionMultiLegRfqStatus`,
//! `OptionMultiLegRfqQuoteStatus`, `OptionMultiLegRfqQuoteSignatureStatus`)
//! are aliased to the existing single-leg enums via `pub use` so both
//! flavors share the same wire tokens and downstream reason coverage
//! for free.

use crate::error::{BackendError, Result};
use crate::options::types::one_subaccount_id;
use crate::types::{AccountId, Price1e8, Side, Size1e8, TimestampMs};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub type OptionMultiLegRfqId = Uuid;
pub type OptionMultiLegRfqQuoteId = Uuid;
pub type OptionMultiLegRfqFillId = Uuid;

pub use crate::options::types::{
    OptionRfqQuoteSignatureStatus as OptionMultiLegRfqQuoteSignatureStatus,
    OptionRfqQuoteStatus as OptionMultiLegRfqQuoteStatus,
    OptionRfqStatus as OptionMultiLegRfqStatus,
};

/// Minimum number of legs per multi-leg RFQ. A "1-leg multi-leg RFQ"
/// is nonsensical — single-leg requests must go through the existing
/// `/options/rfqs` path. Guarded at the repository layer so schema-
/// level tables do not need a per-parent aggregate CHECK constraint.
pub const MIN_LEGS_PER_MULTI_LEG_RFQ: usize = 2;

/// Maximum number of legs per multi-leg RFQ. Covers every standard
/// options strategy (iron condor: 4, butterfly: 3, calendar: 2,
/// custom: 4-5) with room to spare. Prevents `O(n)` DOS-style leg
/// arrays from reaching the transactional accept path.
pub const MAX_LEGS_PER_MULTI_LEG_RFQ: usize = 8;

/// Parent-level record. Mirrors the `option_multi_leg_rfqs` row.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct OptionMultiLegRfqRequest {
    pub option_rfq_id: OptionMultiLegRfqId,
    pub taker: AccountId,
    #[serde(default = "one_subaccount_id")]
    pub taker_subaccount_id: u32,
    pub status: OptionMultiLegRfqStatus,
    pub created_at_ms: TimestampMs,
    pub expires_at_ms: TimestampMs,
    pub accepted_quote_id: Option<OptionMultiLegRfqQuoteId>,
    pub accepted_fill_id: Option<OptionMultiLegRfqFillId>,
}

impl OptionMultiLegRfqRequest {
    /// Same effective-status rule as `OptionRfqRequest`: an RFQ that
    /// is still `Open` past its `expires_at_ms` is effectively
    /// `Expired`, even before the sweeper updates the persisted row.
    pub fn effective_status(&self, now_ms: TimestampMs) -> OptionMultiLegRfqStatus {
        if self.status == OptionMultiLegRfqStatus::Open && now_ms >= self.expires_at_ms {
            OptionMultiLegRfqStatus::Expired
        } else {
            self.status
        }
    }
}

/// Leg composition record. Mirrors an `option_multi_leg_rfq_legs`
/// row. `leg_index` is client-supplied at create time and pinned by
/// the composite PK; the repository verifies contiguity `[0..N)`
/// before INSERT.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct OptionMultiLegRfqLeg {
    pub option_rfq_id: OptionMultiLegRfqId,
    pub leg_index: u32,
    pub option_series_id: String,
    pub side: Side,
    pub size_1e8: Size1e8,
    /// Ratio numerator. `1` is the default (equal-weight leg). Stored
    /// as `(num, den)` so verifiable rational ratios survive the
    /// package-price consistency check without float noise.
    pub ratio_num: u32,
    pub ratio_den: u32,
}

/// Maker quote parent record.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct OptionMultiLegRfqQuote {
    pub quote_id: OptionMultiLegRfqQuoteId,
    pub option_rfq_id: OptionMultiLegRfqId,
    pub mm_account: AccountId,
    #[serde(default = "one_subaccount_id")]
    pub maker_subaccount_id: u32,
    pub session_id: Option<String>,
    pub client_quote_id: Option<String>,
    /// Signed net package price. Represented as a decimal-string
    /// 1e8 integer on the wire so the JSON round-trips deterministic.
    /// Callers are responsible for encoding the sign via the string
    /// value (e.g. `"-50000000"` for a net credit).
    pub package_price_1e8: String,
    pub size_1e8: Size1e8,
    pub status: OptionMultiLegRfqQuoteStatus,
    pub created_at_ms: TimestampMs,
    pub expires_at_ms: TimestampMs,
    pub signature: Option<String>,
    pub quote_digest: Option<String>,
    pub quote_nonce: Option<String>,
    pub signature_status: OptionMultiLegRfqQuoteSignatureStatus,
    pub recovered_signer: Option<AccountId>,
}

impl OptionMultiLegRfqQuote {
    pub fn effective_status(&self, now_ms: TimestampMs) -> OptionMultiLegRfqQuoteStatus {
        if self.status == OptionMultiLegRfqQuoteStatus::Active && now_ms >= self.expires_at_ms {
            OptionMultiLegRfqQuoteStatus::Expired
        } else {
            self.status
        }
    }
}

/// Per-leg quote price. Mirrors an `option_multi_leg_rfq_quote_legs`
/// row.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct OptionMultiLegRfqQuoteLeg {
    pub quote_id: OptionMultiLegRfqQuoteId,
    pub leg_index: u32,
    pub price_1e8: Price1e8,
}

/// Accepted-fill parent record.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct OptionMultiLegRfqFill {
    pub fill_id: OptionMultiLegRfqFillId,
    pub option_rfq_id: OptionMultiLegRfqId,
    pub quote_id: OptionMultiLegRfqQuoteId,
    pub taker: AccountId,
    #[serde(default = "one_subaccount_id")]
    pub taker_subaccount_id: u32,
    pub mm_account: AccountId,
    #[serde(default = "one_subaccount_id")]
    pub maker_subaccount_id: u32,
    pub package_price_1e8: String,
    pub size_1e8: Size1e8,
    pub created_at_ms: TimestampMs,
}

/// Per-leg fill detail record. Mirrors an
/// `option_multi_leg_rfq_fill_legs` row.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct OptionMultiLegRfqFillLeg {
    pub fill_id: OptionMultiLegRfqFillId,
    pub leg_index: u32,
    pub option_series_id: String,
    pub side: Side,
    pub size_1e8: Size1e8,
    pub price_1e8: Price1e8,
}

/// Validate that a client-supplied leg array meets the schema-level
/// invariants: bounded count, `subaccount_id >= 1` (checked upstream),
/// contiguous `leg_index` sequence starting at 0, and consistent
/// `option_rfq_id` on every leg. Called by every repository INSERT
/// path; rejects before touching the DB.
pub fn validate_multi_leg_composition(
    option_rfq_id: OptionMultiLegRfqId,
    legs: &[OptionMultiLegRfqLeg],
) -> Result<()> {
    if legs.len() < MIN_LEGS_PER_MULTI_LEG_RFQ {
        return Err(BackendError::InvalidOptionRfqState(format!(
            "multi-leg RFQ requires at least {} legs, got {}",
            MIN_LEGS_PER_MULTI_LEG_RFQ,
            legs.len()
        )));
    }
    if legs.len() > MAX_LEGS_PER_MULTI_LEG_RFQ {
        return Err(BackendError::InvalidOptionRfqState(format!(
            "multi-leg RFQ supports at most {} legs, got {}",
            MAX_LEGS_PER_MULTI_LEG_RFQ,
            legs.len()
        )));
    }
    for (expected_index, leg) in legs.iter().enumerate() {
        if leg.option_rfq_id != option_rfq_id {
            return Err(BackendError::InvalidOptionRfqState(
                "multi-leg RFQ leg.option_rfq_id does not match parent".to_string(),
            ));
        }
        if leg.leg_index as usize != expected_index {
            return Err(BackendError::InvalidOptionRfqState(format!(
                "multi-leg RFQ leg_index must be contiguous from 0; expected {}, got {}",
                expected_index, leg.leg_index
            )));
        }
        if leg.ratio_num == 0 || leg.ratio_den == 0 {
            return Err(BackendError::InvalidOptionRfqState(
                "multi-leg RFQ leg ratio must be strictly positive".to_string(),
            ));
        }
        if leg.size_1e8 == 0 {
            return Err(BackendError::InvalidOptionRfqState(
                "multi-leg RFQ leg size must be strictly positive".to_string(),
            ));
        }
    }
    Ok(())
}

/// Same invariants for quote legs: bounded count matching the RFQ's
/// leg count, contiguous `leg_index`, consistent `quote_id`. The
/// consistency check between quote legs and RFQ legs (matching
/// `leg_index` order + no missing legs) is enforced by the caller
/// against the loaded RFQ leg array.
pub fn validate_multi_leg_quote_composition(
    quote_id: OptionMultiLegRfqQuoteId,
    expected_leg_count: usize,
    legs: &[OptionMultiLegRfqQuoteLeg],
) -> Result<()> {
    if legs.len() != expected_leg_count {
        return Err(BackendError::InvalidOptionRfqQuoteState(format!(
            "multi-leg quote must carry exactly {} legs, got {}",
            expected_leg_count,
            legs.len()
        )));
    }
    for (expected_index, leg) in legs.iter().enumerate() {
        if leg.quote_id != quote_id {
            return Err(BackendError::InvalidOptionRfqQuoteState(
                "multi-leg quote leg.quote_id does not match parent".to_string(),
            ));
        }
        if leg.leg_index as usize != expected_index {
            return Err(BackendError::InvalidOptionRfqQuoteState(format!(
                "multi-leg quote leg_index must be contiguous from 0; expected {}, got {}",
                expected_index, leg.leg_index
            )));
        }
        if leg.price_1e8 == 0 {
            return Err(BackendError::InvalidOptionRfqQuoteState(
                "multi-leg quote leg price must be strictly positive".to_string(),
            ));
        }
    }
    Ok(())
}

/// Same invariants for fill legs: count matches parent, contiguous
/// `leg_index`, consistent `fill_id`.
pub fn validate_multi_leg_fill_composition(
    fill_id: OptionMultiLegRfqFillId,
    expected_leg_count: usize,
    legs: &[OptionMultiLegRfqFillLeg],
) -> Result<()> {
    if legs.len() != expected_leg_count {
        return Err(BackendError::InvalidOptionRfqQuoteState(format!(
            "multi-leg fill must carry exactly {} legs, got {}",
            expected_leg_count,
            legs.len()
        )));
    }
    for (expected_index, leg) in legs.iter().enumerate() {
        if leg.fill_id != fill_id {
            return Err(BackendError::InvalidOptionRfqQuoteState(
                "multi-leg fill leg.fill_id does not match parent".to_string(),
            ));
        }
        if leg.leg_index as usize != expected_index {
            return Err(BackendError::InvalidOptionRfqQuoteState(format!(
                "multi-leg fill leg_index must be contiguous from 0; expected {}, got {}",
                expected_index, leg.leg_index
            )));
        }
        if leg.size_1e8 == 0 {
            return Err(BackendError::InvalidOptionRfqQuoteState(
                "multi-leg fill leg size must be strictly positive".to_string(),
            ));
        }
        if leg.price_1e8 == 0 {
            return Err(BackendError::InvalidOptionRfqQuoteState(
                "multi-leg fill leg price must be strictly positive".to_string(),
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn taker() -> AccountId {
        AccountId::new("0x1111111111111111111111111111111111111111")
    }

    fn rfq_id() -> OptionMultiLegRfqId {
        uuid::uuid!("11111111-1111-1111-1111-111111111111")
    }

    fn valid_leg(index: u32) -> OptionMultiLegRfqLeg {
        OptionMultiLegRfqLeg {
            option_rfq_id: rfq_id(),
            leg_index: index,
            option_series_id: format!("series-{index}"),
            side: if index % 2 == 0 {
                Side::Buy
            } else {
                Side::Sell
            },
            size_1e8: 100_000_000,
            ratio_num: 1,
            ratio_den: 1,
        }
    }

    #[test]
    fn effective_status_flips_open_to_expired_after_deadline() {
        let rfq = OptionMultiLegRfqRequest {
            option_rfq_id: rfq_id(),
            taker: taker(),
            taker_subaccount_id: 1,
            status: OptionMultiLegRfqStatus::Open,
            created_at_ms: 0,
            expires_at_ms: 100,
            accepted_quote_id: None,
            accepted_fill_id: None,
        };
        assert_eq!(rfq.effective_status(50), OptionMultiLegRfqStatus::Open);
        assert_eq!(rfq.effective_status(100), OptionMultiLegRfqStatus::Expired);
        assert_eq!(rfq.effective_status(101), OptionMultiLegRfqStatus::Expired);
    }

    #[test]
    fn effective_status_does_not_downgrade_accepted() {
        let rfq = OptionMultiLegRfqRequest {
            option_rfq_id: rfq_id(),
            taker: taker(),
            taker_subaccount_id: 1,
            status: OptionMultiLegRfqStatus::Accepted,
            created_at_ms: 0,
            expires_at_ms: 100,
            accepted_quote_id: None,
            accepted_fill_id: None,
        };
        assert_eq!(rfq.effective_status(200), OptionMultiLegRfqStatus::Accepted);
    }

    #[test]
    fn validate_rejects_single_leg() {
        let legs = vec![valid_leg(0)];
        let err = validate_multi_leg_composition(rfq_id(), &legs).unwrap_err();
        assert!(matches!(err, BackendError::InvalidOptionRfqState(_)));
    }

    #[test]
    fn validate_accepts_two_through_eight_legs() {
        for n in 2..=8 {
            let legs: Vec<_> = (0..n as u32).map(valid_leg).collect();
            validate_multi_leg_composition(rfq_id(), &legs)
                .unwrap_or_else(|e| panic!("count {n} should validate: {e}"));
        }
    }

    #[test]
    fn validate_rejects_nine_legs() {
        let legs: Vec<_> = (0..9).map(valid_leg).collect();
        let err = validate_multi_leg_composition(rfq_id(), &legs).unwrap_err();
        assert!(matches!(err, BackendError::InvalidOptionRfqState(_)));
    }

    #[test]
    fn validate_rejects_non_contiguous_leg_index() {
        let mut legs = vec![valid_leg(0), valid_leg(1)];
        legs[1].leg_index = 5;
        let err = validate_multi_leg_composition(rfq_id(), &legs).unwrap_err();
        assert!(matches!(err, BackendError::InvalidOptionRfqState(_)));
    }

    #[test]
    fn validate_rejects_mismatched_rfq_id_on_leg() {
        let mut legs = vec![valid_leg(0), valid_leg(1)];
        legs[1].option_rfq_id = uuid::uuid!("22222222-2222-2222-2222-222222222222");
        let err = validate_multi_leg_composition(rfq_id(), &legs).unwrap_err();
        assert!(matches!(err, BackendError::InvalidOptionRfqState(_)));
    }

    #[test]
    fn validate_rejects_zero_ratio() {
        let mut legs = vec![valid_leg(0), valid_leg(1)];
        legs[1].ratio_num = 0;
        let err = validate_multi_leg_composition(rfq_id(), &legs).unwrap_err();
        assert!(matches!(err, BackendError::InvalidOptionRfqState(_)));
    }

    #[test]
    fn validate_rejects_zero_size() {
        let mut legs = vec![valid_leg(0), valid_leg(1)];
        legs[1].size_1e8 = 0;
        let err = validate_multi_leg_composition(rfq_id(), &legs).unwrap_err();
        assert!(matches!(err, BackendError::InvalidOptionRfqState(_)));
    }

    #[test]
    fn validate_quote_legs_require_count_match() {
        let quote_id = uuid::uuid!("33333333-3333-3333-3333-333333333333");
        let quote_leg = OptionMultiLegRfqQuoteLeg {
            quote_id,
            leg_index: 0,
            price_1e8: 10,
        };
        let err = validate_multi_leg_quote_composition(quote_id, 2, &[quote_leg]).unwrap_err();
        assert!(matches!(err, BackendError::InvalidOptionRfqQuoteState(_)));
    }

    #[test]
    fn validate_quote_legs_reject_zero_price() {
        let quote_id = uuid::uuid!("44444444-4444-4444-4444-444444444444");
        let legs = vec![
            OptionMultiLegRfqQuoteLeg {
                quote_id,
                leg_index: 0,
                price_1e8: 10,
            },
            OptionMultiLegRfqQuoteLeg {
                quote_id,
                leg_index: 1,
                price_1e8: 0,
            },
        ];
        let err = validate_multi_leg_quote_composition(quote_id, 2, &legs).unwrap_err();
        assert!(matches!(err, BackendError::InvalidOptionRfqQuoteState(_)));
    }

    #[test]
    fn validate_fill_legs_require_contiguity_and_positive_values() {
        let fill_id = uuid::uuid!("55555555-5555-5555-5555-555555555555");
        let legs = vec![
            OptionMultiLegRfqFillLeg {
                fill_id,
                leg_index: 0,
                option_series_id: "series-0".to_string(),
                side: Side::Buy,
                size_1e8: 100,
                price_1e8: 10,
            },
            OptionMultiLegRfqFillLeg {
                fill_id,
                leg_index: 2, // gap
                option_series_id: "series-2".to_string(),
                side: Side::Sell,
                size_1e8: 100,
                price_1e8: 10,
            },
        ];
        let err = validate_multi_leg_fill_composition(fill_id, 2, &legs).unwrap_err();
        assert!(matches!(err, BackendError::InvalidOptionRfqQuoteState(_)));
    }
}
