use crate::error::{BackendError, Result};
use crate::execution::{intent_id_to_b256, PerpTradePayload};
use crate::types::{AccountId, MarketId, OrderId, Price1e8, Size1e8, TimestampMs};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionIntentStatus {
    Pending,
    DryRun,
    CalldataReady,
    SimulationOk,
    SimulationFailed,
    /// PERPS-BASE-SEPOLIA-BACKEND-BROADCAST-DURABILITY-PG-V1: raw
    /// signed transaction envelope, tx_hash, and executor nonce are
    /// persisted in the durable store, but the RPC has NOT been
    /// invoked (or the RPC outcome is unknown — see
    /// [`SendErrorClass::Ambiguous`]). The reconciler polls
    /// `transaction_receipt(tx_hash)` and either observes a receipt
    /// (advance to Submitted/Confirmed/Failed) or rebroadcasts the
    /// byte-identical raw envelope (bounded retry).
    Prepared,
    Submitted,
    Confirmed,
    Failed,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ExecutionIntent {
    pub intent_id: Uuid,
    pub market_id: MarketId,
    pub buyer: AccountId,
    pub seller: AccountId,
    pub price_1e8: Price1e8,
    pub size_1e8: Size1e8,
    pub buy_order_id: OrderId,
    pub sell_order_id: OrderId,
    pub buyer_is_maker: Option<bool>,
    pub buyer_nonce: Option<u64>,
    pub seller_nonce: Option<u64>,
    pub deadline_ms: Option<TimestampMs>,
    pub created_at_ms: TimestampMs,
    pub status: ExecutionIntentStatus,
}

impl ExecutionIntent {
    /// Canonical reconstruction of the deployed-V1 10-field
    /// `PerpTradePayload` from a persisted `ExecutionIntent`.
    ///
    /// **Deadline unit conversion.** `ExecutionIntent.deadline_ms` is
    /// storage/application milliseconds (Unix epoch × 1_000).
    /// The deployed PerpMatchingEngine V1 `PerpTrade.deadline` is
    /// Solidity Unix SECONDS (compared to `block.timestamp`). The
    /// boundary conversion `deadline_ms / 1_000` MUST happen exactly
    /// once when reconstructing the on-chain payload; this function
    /// owns that conversion.
    ///
    /// **Sub-second policy.** For canonical closed-test prepared
    /// intents `deadline_ms` is always `deadline_sec × 1_000` (see
    /// `prepare_trade_core::deadline_shadow_ms`). Any non-multiple of
    /// 1_000 that reaches this function is refused with
    /// [`BackendError::MissingExecutionMetadata`] rather than being
    /// silently truncated — an ambiguous deadline would produce a
    /// digest that does not match what the traders signed.
    ///
    /// This method is the shared primitive for BOTH the cosign
    /// verification path (`api::perps_cosign::intent_to_v1_payload`
    /// delegates to it) and the runtime transaction builder
    /// (`build_perp_execution_call_from_intent`). The equality
    /// invariant is asserted by unit tests
    /// `payload_equality_cosign_and_runtime_paths_match_*`.
    pub fn perp_trade_payload(&self) -> Result<PerpTradePayload> {
        let buyer_is_maker = self
            .buyer_is_maker
            .ok_or_else(|| BackendError::MissingExecutionMetadata("buyer_is_maker".to_string()))?;
        let buyer_nonce = self
            .buyer_nonce
            .ok_or_else(|| BackendError::MissingExecutionMetadata("buyer_nonce".to_string()))?;
        let seller_nonce = self
            .seller_nonce
            .ok_or_else(|| BackendError::MissingExecutionMetadata("seller_nonce".to_string()))?;
        let deadline_ms = self
            .deadline_ms
            .ok_or_else(|| BackendError::MissingExecutionMetadata("deadline".to_string()))?;
        // Reject sub-second (non-multiple-of-1000) deadlines: any such
        // value is ambiguous under the ms↔sec boundary and would
        // produce a signature the runtime cannot reproduce.
        if deadline_ms % 1_000 != 0 {
            return Err(BackendError::MissingExecutionMetadata(
                "deadline_ms is not a whole-second multiple; cannot reconstruct \
                 deployed V1 PerpTrade.deadline unambiguously"
                    .to_string(),
            ));
        }
        let deadline_ms_u128 = u128::try_from(deadline_ms)
            .map_err(|_| BackendError::MissingExecutionMetadata("deadline".to_string()))?;
        let deadline_sec = deadline_ms_u128 / 1_000;

        // PERPS-PRICING-AND-EXECUTION-SAFETY-CORE-V1 — legacy intent shape has no
        // user bounds. Pass `0, 0` (strict): validate_shape() reproduces V1 exact-
        // price behaviour when both bounds are zero. Once the higher intent model
        // carries user-signed bounds, thread them through instead of hard-coding 0.
        PerpTradePayload::new(
            intent_id_to_b256(&self.intent_id.to_string())?,
            self.buyer.clone(),
            self.seller.clone(),
            u128::from(self.market_id),
            self.size_1e8,
            self.price_1e8,
            0,
            0,
            buyer_is_maker,
            u128::from(buyer_nonce),
            u128::from(seller_nonce),
            deadline_sec,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{MarketId, OrderId};

    fn intent_with_deadline_ms(deadline_ms: TimestampMs) -> ExecutionIntent {
        ExecutionIntent {
            intent_id: Uuid::from_u128(0xdead_beef),
            market_id: MarketId::from(1u64),
            buyer: AccountId::new("0x0000000000000000000000000000000000000001"),
            seller: AccountId::new("0x0000000000000000000000000000000000000002"),
            price_1e8: 300_000_000_000,
            size_1e8: 1_000_000,
            buy_order_id: OrderId(Uuid::from_u128(0x11)),
            sell_order_id: OrderId(Uuid::from_u128(0x22)),
            buyer_is_maker: Some(false),
            buyer_nonce: Some(7),
            seller_nonce: Some(12),
            deadline_ms: Some(deadline_ms),
            created_at_ms: 1_789_000_000_000,
            status: ExecutionIntentStatus::CalldataReady,
        }
    }

    // PERPS_BASE_SEPOLIA_CLOSED_TEST_LIFECYCLE_DEADLINE_UNITS_FIX_V1

    #[test]
    fn deadline_conversion_ms_to_seconds() {
        // Prepared closed-test intent: deadline_ms == deadline_sec × 1_000.
        let intent = intent_with_deadline_ms(1_789_656_028_000);
        let payload = intent.perp_trade_payload().unwrap();
        assert_eq!(
            payload.deadline, 1_789_656_028,
            "PerpTrade.deadline MUST be Unix seconds (deadline_ms / 1_000)"
        );
        // Guard against a regression that keeps the ms value.
        assert_ne!(payload.deadline, 1_789_656_028_000);
    }

    #[test]
    fn deadline_conversion_zero_seconds_edge() {
        let intent = intent_with_deadline_ms(0);
        let payload = intent.perp_trade_payload().unwrap();
        assert_eq!(payload.deadline, 0);
    }

    #[test]
    fn deadline_conversion_rejects_sub_second_ambiguous_values() {
        // Any deadline_ms that is NOT a multiple of 1_000 is refused.
        // Silently truncating would produce a signature the runtime
        // cannot reproduce (this is exactly the shape of the earlier
        // real-trade `InvalidSignature` incident).
        let intent = intent_with_deadline_ms(1_789_656_028_500);
        let err = intent.perp_trade_payload().unwrap_err();
        assert!(matches!(err, BackendError::MissingExecutionMetadata(_)));
    }

    #[test]
    fn cosign_and_runtime_paths_produce_byte_identical_payload() {
        // The api::perps_cosign::intent_to_v1_payload delegates to
        // intent.perp_trade_payload; the two MUST be indistinguishable.
        let intent = intent_with_deadline_ms(1_789_656_028_000);
        let runtime = intent.perp_trade_payload().unwrap();
        let cosign = crate::api::perps_cosign::intent_to_v1_payload(&intent).unwrap();
        assert_eq!(runtime, cosign);
    }

    #[test]
    fn digest_matches_across_cosign_and_runtime_paths() {
        // Same domain, same reconstructed payload → same EIP-712 digest.
        use crate::execution::{perp_trade_v1_digest_bytes, PerpTradeDomain};
        let intent = intent_with_deadline_ms(1_789_656_028_000);
        let domain = PerpTradeDomain::new(
            84532,
            AccountId::new("0x774d96e5739bffadee91508b4d3d74f5be29f165"),
        );
        let runtime_payload = intent.perp_trade_payload().unwrap();
        let cosign_payload = crate::api::perps_cosign::intent_to_v1_payload(&intent).unwrap();
        let d_runtime = perp_trade_v1_digest_bytes(&runtime_payload, &domain).unwrap();
        let d_cosign = perp_trade_v1_digest_bytes(&cosign_payload, &domain).unwrap();
        assert_eq!(d_runtime, d_cosign);
    }

    #[test]
    fn missing_deadline_ms_is_refused() {
        let mut intent = intent_with_deadline_ms(1_789_656_028_000);
        intent.deadline_ms = None;
        let err = intent.perp_trade_payload().unwrap_err();
        assert!(
            matches!(err, BackendError::MissingExecutionMetadata(ref m) if m.contains("deadline")),
            "got: {err:?}"
        );
    }
}
