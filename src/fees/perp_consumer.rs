//! V2F-P: classifier for the `deopt_perp_fee_charged_v2_total{consumer}`
//! metric.
//!
//! After the V2F PerpEngine cutover, every PERP `FeeChargedV2` event must
//! be emitted by `FeesManagerV2` on behalf of the NEW PerpEngine
//! consumer. The OLD PerpEngine remains stranded under the A3 Base
//! Sepolia fallback and is **expected to emit zero new PERP V2 fees**.
//! Any nonzero `consumer == "old"` occurrence indicates a regression
//! (FeesManagerV2 was re-pointed at OLD, OLD was re-allowed as a fee
//! consumer, or an executor was routed at OLD).
//!
//! This module exposes a single pure function — `classify_perp_fee_consumer`
//! — that maps the decoded `consumer` topic of a PERP `FeeChargedV2`
//! event to one of three low-cardinality bucket labels:
//!
//! - `"new"` when the consumer matches the configured NEW PerpEngine.
//! - `"old"` when the consumer matches the configured OLD PerpEngine.
//! - `"unknown"` when neither configured address is set or matches.
//!
//! The raw address is never returned — that would explode label
//! cardinality and is explicitly disallowed by the V2F-P task spec.

/// Bucket labels emitted on `deopt_perp_fee_charged_v2_total{consumer=...}`.
pub const CONSUMER_NEW: &str = "new";
pub const CONSUMER_OLD: &str = "old";
pub const CONSUMER_UNKNOWN: &str = "unknown";

/// Classify the decoded `consumer` topic of a PERP `FeeChargedV2` log
/// into one of the three low-cardinality bucket labels.
///
/// `new` and `old` are the configured NEW / OLD PerpEngine addresses.
/// They are matched case-insensitively against `consumer`. An empty
/// or zero-address configuration is treated as "not configured" and
/// does not match anything.
///
/// The function never returns the raw `consumer` address — the
/// only outputs are `"new"`, `"old"`, or `"unknown"`.
pub fn classify_perp_fee_consumer(
    consumer: &str,
    new: Option<&str>,
    old: Option<&str>,
) -> &'static str {
    if address_matches(consumer, new) {
        CONSUMER_NEW
    } else if address_matches(consumer, old) {
        CONSUMER_OLD
    } else {
        CONSUMER_UNKNOWN
    }
}

fn address_matches(consumer: &str, configured: Option<&str>) -> bool {
    let consumer = consumer.trim();
    if consumer.is_empty() || is_zero_address(consumer) {
        return false;
    }
    let Some(configured) = configured else {
        return false;
    };
    let configured = configured.trim();
    if configured.is_empty() || is_zero_address(configured) {
        return false;
    }
    consumer.eq_ignore_ascii_case(configured)
}

fn is_zero_address(address: &str) -> bool {
    let trimmed = address
        .strip_prefix("0x")
        .or_else(|| address.strip_prefix("0X"))
        .unwrap_or(address);
    !trimmed.is_empty() && trimmed.bytes().all(|byte| byte == b'0')
}

#[cfg(test)]
mod tests {
    use super::*;

    const NEW_PERP: &str = "0xc6C592100723Fe0C66343A16e95eC34cC0c2141c";
    const OLD_PERP: &str = "0xB36395b67D0798ADA981731c9Fa5239F4362b53B";
    const ZERO: &str = "0x0000000000000000000000000000000000000000";

    #[test]
    fn matches_new_consumer_case_insensitively() {
        assert_eq!(
            classify_perp_fee_consumer(NEW_PERP, Some(NEW_PERP), Some(OLD_PERP)),
            "new"
        );
        assert_eq!(
            classify_perp_fee_consumer(
                &NEW_PERP.to_ascii_lowercase(),
                Some(NEW_PERP),
                Some(OLD_PERP)
            ),
            "new"
        );
        assert_eq!(
            classify_perp_fee_consumer(
                &NEW_PERP.to_ascii_uppercase(),
                Some(NEW_PERP),
                Some(OLD_PERP)
            ),
            "new"
        );
    }

    #[test]
    fn matches_old_consumer_case_insensitively() {
        assert_eq!(
            classify_perp_fee_consumer(OLD_PERP, Some(NEW_PERP), Some(OLD_PERP)),
            "old"
        );
        assert_eq!(
            classify_perp_fee_consumer(
                &OLD_PERP.to_ascii_lowercase(),
                Some(NEW_PERP),
                Some(OLD_PERP)
            ),
            "old"
        );
    }

    #[test]
    fn returns_unknown_when_no_addresses_configured() {
        assert_eq!(classify_perp_fee_consumer(NEW_PERP, None, None), "unknown");
        assert_eq!(classify_perp_fee_consumer(OLD_PERP, None, None), "unknown");
    }

    #[test]
    fn returns_unknown_when_old_unset_and_consumer_is_not_new() {
        assert_eq!(
            classify_perp_fee_consumer(
                "0xdeadbeef00000000000000000000000000000001",
                Some(NEW_PERP),
                None
            ),
            "unknown"
        );
    }

    #[test]
    fn returns_unknown_when_addresses_are_zero() {
        // Zero address as the configured target never matches.
        assert_eq!(
            classify_perp_fee_consumer(NEW_PERP, Some(ZERO), Some(ZERO)),
            "unknown"
        );
        // Zero address as the observed consumer never resolves.
        assert_eq!(
            classify_perp_fee_consumer(ZERO, Some(NEW_PERP), Some(OLD_PERP)),
            "unknown"
        );
    }

    #[test]
    fn returns_unknown_for_unrelated_consumer() {
        assert_eq!(
            classify_perp_fee_consumer(
                "0xdeadbeef00000000000000000000000000000001",
                Some(NEW_PERP),
                Some(OLD_PERP)
            ),
            "unknown"
        );
    }

    #[test]
    fn empty_consumer_resolves_to_unknown() {
        assert_eq!(
            classify_perp_fee_consumer("", Some(NEW_PERP), Some(OLD_PERP)),
            "unknown"
        );
    }

    #[test]
    fn classifier_never_emits_raw_address() {
        // Iterate over every input shape and assert the result is one of the
        // three allowed bucket labels — no raw address ever leaks through.
        let cases = [
            (NEW_PERP, Some(NEW_PERP), Some(OLD_PERP)),
            (OLD_PERP, Some(NEW_PERP), Some(OLD_PERP)),
            (
                "0xdeadbeef00000000000000000000000000000001",
                Some(NEW_PERP),
                Some(OLD_PERP),
            ),
            (NEW_PERP, None, None),
            (ZERO, Some(NEW_PERP), Some(OLD_PERP)),
            ("", Some(NEW_PERP), Some(OLD_PERP)),
        ];
        for (consumer, new, old) in cases {
            let label = classify_perp_fee_consumer(consumer, new, old);
            assert!(
                matches!(label, "new" | "old" | "unknown"),
                "classifier returned non-bucket label `{label}` for consumer `{consumer}`"
            );
        }
    }
}
