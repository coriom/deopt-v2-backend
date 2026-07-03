//! PERPS-PERSISTENCE-HISTORY-LIFECYCLE-V1 — Perps rejection classifier.
//!
//! Maps `BackendError` variants raised inside the internal execution
//! service to a `(reason_code, reason_source)` pair suitable for
//! surfacing in a WS `PerpOrderRejected` lifecycle frame OR (when the
//! public write path lands) a persistent rejected-attempts feed.
//!
//! Auth-level errors are intentionally NOT in the recordable set —
//! matching the Options rejection classifier, we only classify
//! trader-meaningful outcomes for a caller whose identity has already
//! been proven. Public Perps mutation routes remain fail-closed, so
//! this classifier only runs behind internal service calls today.

use crate::error::BackendError;
use crate::perps::orders::reason;

pub fn classify_perp_rejection(error: &BackendError) -> Option<(&'static str, &'static str)> {
    use reason::*;
    match error {
        BackendError::PerpZeroSize => Some((ZERO_SIZE, SOURCE_REQUEST_VALIDATION)),
        BackendError::PerpZeroPrice => Some((ZERO_PRICE, SOURCE_REQUEST_VALIDATION)),
        BackendError::PerpZeroMargin => Some((INSUFFICIENT_MARGIN, SOURCE_REQUEST_VALIDATION)),
        BackendError::PerpsMarketNotFound(_) => Some((UNKNOWN_MARKET, SOURCE_REQUEST_VALIDATION)),
        BackendError::PerpMarkPriceUnavailable(_) => Some((STALE_MARK_PRICE, SOURCE_RISK)),
        BackendError::PerpDuplicateClientOrderId(_) => {
            Some((DUPLICATE_CLIENT_ORDER_ID, SOURCE_REQUEST_VALIDATION))
        }
        BackendError::PerpPostOnlyWouldMatch => {
            Some((POST_ONLY_WOULD_MATCH, SOURCE_MATCHING_POLICY))
        }
        BackendError::PerpFokNotFillable => Some((FOK_NOT_FILLABLE, SOURCE_MATCHING_POLICY)),
        BackendError::PerpInvalidTifCombination(_) => {
            Some((INVALID_TIF_COMBINATION, SOURCE_REQUEST_VALIDATION))
        }
        BackendError::PerpUnsupportedTif(_) => Some((UNSUPPORTED_TIF, SOURCE_REQUEST_VALIDATION)),
        BackendError::PerpSelfTrade => Some((SELF_TRADE, SOURCE_MATCHING_POLICY)),
        BackendError::PerpReduceOnlyViolation => Some((REDUCE_ONLY_VIOLATION, SOURCE_RISK)),
        BackendError::PerpInsufficientMargin(_) => Some((INSUFFICIENT_MARGIN, SOURCE_RISK)),
        BackendError::PerpLeverageExceeded { .. } => Some((LEVERAGE_EXCEEDED, SOURCE_RISK)),
        BackendError::PerpPositionFlip => Some((POSITION_FLIP, SOURCE_RISK)),
        // Auth-level, config, internal errors: intentionally not
        // classified — caller identity may not be safely attributable.
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_matching_policy_errors() {
        assert_eq!(
            classify_perp_rejection(&BackendError::PerpPostOnlyWouldMatch),
            Some((
                reason::POST_ONLY_WOULD_MATCH,
                reason::SOURCE_MATCHING_POLICY
            ))
        );
        assert_eq!(
            classify_perp_rejection(&BackendError::PerpFokNotFillable),
            Some((reason::FOK_NOT_FILLABLE, reason::SOURCE_MATCHING_POLICY))
        );
        assert_eq!(
            classify_perp_rejection(&BackendError::PerpSelfTrade),
            Some((reason::SELF_TRADE, reason::SOURCE_MATCHING_POLICY))
        );
    }

    #[test]
    fn classifies_risk_errors() {
        assert_eq!(
            classify_perp_rejection(&BackendError::PerpReduceOnlyViolation),
            Some((reason::REDUCE_ONLY_VIOLATION, reason::SOURCE_RISK))
        );
        assert_eq!(
            classify_perp_rejection(&BackendError::PerpLeverageExceeded {
                market: "ETH-PERP".to_string(),
                reason: "test".to_string()
            }),
            Some((reason::LEVERAGE_EXCEEDED, reason::SOURCE_RISK))
        );
        assert_eq!(
            classify_perp_rejection(&BackendError::PerpMarkPriceUnavailable("stale".to_string())),
            Some((reason::STALE_MARK_PRICE, reason::SOURCE_RISK))
        );
        assert_eq!(
            classify_perp_rejection(&BackendError::PerpPositionFlip),
            Some((reason::POSITION_FLIP, reason::SOURCE_RISK))
        );
    }

    #[test]
    fn classifies_request_validation_errors() {
        assert_eq!(
            classify_perp_rejection(&BackendError::PerpZeroSize),
            Some((reason::ZERO_SIZE, reason::SOURCE_REQUEST_VALIDATION))
        );
        assert_eq!(
            classify_perp_rejection(&BackendError::PerpZeroPrice),
            Some((reason::ZERO_PRICE, reason::SOURCE_REQUEST_VALIDATION))
        );
        assert_eq!(
            classify_perp_rejection(&BackendError::PerpInvalidTifCombination(
                "po+ioc".to_string()
            )),
            Some((
                reason::INVALID_TIF_COMBINATION,
                reason::SOURCE_REQUEST_VALIDATION
            ))
        );
        assert_eq!(
            classify_perp_rejection(&BackendError::PerpsMarketNotFound("SOL-PERP".to_string())),
            Some((reason::UNKNOWN_MARKET, reason::SOURCE_REQUEST_VALIDATION))
        );
        assert_eq!(
            classify_perp_rejection(&BackendError::PerpDuplicateClientOrderId(
                "cli-1".to_string()
            )),
            Some((
                reason::DUPLICATE_CLIENT_ORDER_ID,
                reason::SOURCE_REQUEST_VALIDATION
            ))
        );
    }

    #[test]
    fn does_not_classify_auth_or_internal_errors() {
        // These SHOULD NOT surface as trader-visible rejections.
        assert!(classify_perp_rejection(&BackendError::MalformedSignature).is_none());
        assert!(classify_perp_rejection(&BackendError::MalformedAccountAddress).is_none());
        assert!(classify_perp_rejection(&BackendError::Config("test".to_string())).is_none());
        assert!(classify_perp_rejection(&BackendError::Persistence("test".to_string())).is_none());
    }
}
