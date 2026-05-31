//! V2G-F: classifier for the `deopt_option_fee_charged_v2_total{consumer}`
//! and `deopt_option_fee_rebated_v2_total{consumer}` metrics.
//!
//! Mirrors the V2F-P/V2F-Q [`perp_consumer`] classifier but for the
//! OPTION-side fee path, which is driven by the V2 MarginEngine
//! (`useFeesManagerV2 = true` since V2E-E). Every OPTION `FeeChargedV2`
//! / `FeeRebatedV2` should be emitted by FeesManagerV2 on behalf of the
//! NEW MarginEngine; any nonzero `consumer == "old"` would indicate a
//! regression (the V2 MarginEngine was re-pointed at the stranded
//! legacy address, or the legacy address was re-allowed as a fee
//! consumer).
//!
//! Like the PERP classifier, this module only ever returns one of three
//! low-cardinality bucket labels (`"new"`, `"old"`, `"unknown"`) — raw
//! addresses are explicitly never returned. The implementation
//! delegates to the same case-insensitive matcher used by the PERP
//! classifier so behaviour stays consistent across both products.
//!
//! [`perp_consumer`]: super::perp_consumer

use super::perp_consumer::classify_perp_fee_consumer;

/// Bucket labels emitted on `deopt_option_fee_charged_v2_total{consumer=...}`.
///
/// Re-exported aliases of the PERP buckets so both metrics share the
/// same label vocabulary.
pub use super::perp_consumer::{CONSUMER_NEW, CONSUMER_OLD, CONSUMER_UNKNOWN};

/// Classify the decoded `consumer` topic of an OPTION `FeeChargedV2`
/// or `FeeRebatedV2` log into one of the three low-cardinality bucket
/// labels.
///
/// `new` is the configured NEW MarginEngine address (the V2-fees
/// consumer). `old` is the optional legacy MarginEngine address used
/// only for observability — `None` means "OLD address not configured",
/// in which case every non-NEW consumer is bucketed as `"unknown"`.
///
/// The function never returns the raw `consumer` address.
pub fn classify_option_fee_consumer(
    consumer: &str,
    new: Option<&str>,
    old: Option<&str>,
) -> &'static str {
    classify_perp_fee_consumer(consumer, new, old)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NEW_MARGIN: &str = "0x287Cef479be5889eEfCa847F9e73C860898f48Cc";
    const OLD_MARGIN: &str = "0x6C5665De05e7314cB63cD77F82DFa86508A5b5F8";
    const ZERO: &str = "0x0000000000000000000000000000000000000000";

    #[test]
    fn matches_new_margin_engine() {
        assert_eq!(
            classify_option_fee_consumer(NEW_MARGIN, Some(NEW_MARGIN), Some(OLD_MARGIN)),
            "new"
        );
        assert_eq!(
            classify_option_fee_consumer(
                &NEW_MARGIN.to_ascii_lowercase(),
                Some(NEW_MARGIN),
                Some(OLD_MARGIN)
            ),
            "new"
        );
    }

    #[test]
    fn matches_old_margin_engine_when_configured() {
        assert_eq!(
            classify_option_fee_consumer(OLD_MARGIN, Some(NEW_MARGIN), Some(OLD_MARGIN)),
            "old"
        );
    }

    #[test]
    fn unknown_when_old_unset_and_consumer_is_not_new() {
        assert_eq!(
            classify_option_fee_consumer(OLD_MARGIN, Some(NEW_MARGIN), None),
            "unknown"
        );
        assert_eq!(
            classify_option_fee_consumer(
                "0xdeadbeef00000000000000000000000000000001",
                Some(NEW_MARGIN),
                None
            ),
            "unknown"
        );
    }

    #[test]
    fn unknown_when_no_addresses_configured() {
        assert_eq!(
            classify_option_fee_consumer(NEW_MARGIN, None, None),
            "unknown"
        );
    }

    #[test]
    fn zero_address_never_matches() {
        assert_eq!(
            classify_option_fee_consumer(NEW_MARGIN, Some(ZERO), Some(ZERO)),
            "unknown"
        );
        assert_eq!(
            classify_option_fee_consumer(ZERO, Some(NEW_MARGIN), Some(OLD_MARGIN)),
            "unknown"
        );
    }

    #[test]
    fn empty_consumer_resolves_to_unknown() {
        assert_eq!(
            classify_option_fee_consumer("", Some(NEW_MARGIN), Some(OLD_MARGIN)),
            "unknown"
        );
    }

    #[test]
    fn classifier_never_emits_raw_address() {
        let cases = [
            (NEW_MARGIN, Some(NEW_MARGIN), Some(OLD_MARGIN)),
            (OLD_MARGIN, Some(NEW_MARGIN), Some(OLD_MARGIN)),
            (
                "0xdeadbeef00000000000000000000000000000001",
                Some(NEW_MARGIN),
                Some(OLD_MARGIN),
            ),
            (NEW_MARGIN, None, None),
            (ZERO, Some(NEW_MARGIN), Some(OLD_MARGIN)),
            ("", Some(NEW_MARGIN), Some(OLD_MARGIN)),
        ];
        for (consumer, new, old) in cases {
            let label = classify_option_fee_consumer(consumer, new, old);
            assert!(
                matches!(label, "new" | "old" | "unknown"),
                "OPTION classifier leaked non-bucket label `{label}` for consumer `{consumer}`"
            );
        }
    }
}
