use crate::error::{BackendError, Result};
use crate::execution::perp_trade::PerpsProtocolVersion;
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
    /// PERPS_CLOSE_PNL_BUG_RETIRED_AND_V2_ACCOUNTING_DESIGN — operator-
    /// authored administrative retirement of an intent that must never
    /// be signed / cosigned / simulated / armed / broadcast. Terminal.
    /// The row remains for audit; the executor's broadcastable filter
    /// (`WHERE status IN ('pending','calldata_ready','simulation_ok')`)
    /// naturally excludes it. Distinct from `Failed` (post-broadcast
    /// on-chain revert) and `SimulationFailed` (simulate-time revert):
    /// `Abandoned` denotes an intent that was withdrawn BEFORE the
    /// runtime touched it.
    Abandoned,
}

impl ExecutionIntentStatus {
    /// Whether the intent may still transition into an executable
    /// lifecycle state. `Abandoned`, `Confirmed`, and `Failed` are
    /// terminal for the executor; `Prepared` and `Submitted` are
    /// in-flight and MUST NOT be overwritten by an administrative
    /// retire (the reconciler is authoritative). Only administratively-
    /// retire-safe states return `true`.
    pub fn is_retire_eligible(self) -> bool {
        matches!(
            self,
            ExecutionIntentStatus::Pending
                | ExecutionIntentStatus::DryRun
                | ExecutionIntentStatus::CalldataReady
                | ExecutionIntentStatus::SimulationOk
                | ExecutionIntentStatus::SimulationFailed
        )
    }
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
    /// PERPS_V2_BACKEND_COMPAT_FOUNDATION_V1 — settlement protocol
    /// version this intent belongs to. Persisted as `TEXT` on
    /// `execution_intents.protocol_version` (migration
    /// `0064_execution_intents_protocol_version.sql`). All rows
    /// created BEFORE the migration back-fill to
    /// [`PerpsProtocolVersion::V1`] deterministically, matching the
    /// live Base Sepolia V1 deployment. Post-cosign this field is
    /// IMMUTABLE: it fixes the exact EIP-712 domain / typehash /
    /// verifying contract the trader signed, and flipping the runtime
    /// active-version MUST NOT retarget an already-signed intent.
    #[serde(default)]
    pub protocol_version: PerpsProtocolVersion,
    /// PERPS_V2_BACKEND_RPC_SIMULATION_INTEGRATION_V1 — V2
    /// trader-signed upper price bound (1e8 scale). `0` reproduces
    /// V1 strict-price semantics (no upper bound). Persisted in
    /// `execution_intents.max_execution_price_1e8` (migration
    /// `0065_execution_intents_v2_price_bounds.sql`). Participates
    /// in the V2 EIP-712 digest via
    /// [`PerpTradePayload::max_execution_price_1e8`]; any loss
    /// between prepare and cosign produces an on-chain signature
    /// verification failure. Immutable post-cosign.
    #[serde(default)]
    pub max_execution_price_1e8: u128,
    /// PERPS_V2_BACKEND_RPC_SIMULATION_INTEGRATION_V1 — V2
    /// trader-signed lower price bound (1e8 scale). See
    /// [`max_execution_price_1e8`] for lifecycle notes.
    #[serde(default)]
    pub min_execution_price_1e8: u128,
}

impl ExecutionIntent {
    /// PERPS_V2_BACKEND_ANVIL_LIVE_E2E_V1 (§0) — cross-version
    /// safety invariant. Because the backend uses a single 12-field
    /// [`PerpTradePayload`] for both V1 and V2, a V1 intent MUST
    /// carry `max_execution_price_1e8 == 0` AND
    /// `min_execution_price_1e8 == 0`. Any other combination
    /// implies the persisted row is malformed (or the row was
    /// tampered with) and MUST fail closed rather than being
    /// silently reinterpreted.
    ///
    /// V2 intents may carry any valid combination (including 0/0 —
    /// strict-price mode).
    ///
    /// Called by:
    ///   - [`ExecutionIntent::perp_trade_payload`] (every reload path)
    ///   - [`api::perps_cosign::prepare_trade_core`] (after payload
    ///     construction, before insert)
    ///
    /// The invariant is enforced at both prepare time and every
    /// reload so a mid-database mutation cannot bypass it.
    pub fn validate_version_invariants(&self) -> Result<()> {
        if self.protocol_version == PerpsProtocolVersion::V1
            && (self.max_execution_price_1e8 != 0 || self.min_execution_price_1e8 != 0)
        {
            return Err(BackendError::Config(format!(
                "V1 intent {} has non-zero V2 price bounds (max={}, min={}); \
                 V1 signatures do not commit to bounds — this row is malformed \
                 and cannot be reconstructed safely",
                self.intent_id, self.max_execution_price_1e8, self.min_execution_price_1e8
            )));
        }
        Ok(())
    }

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
        // PERPS_V2_BACKEND_ANVIL_LIVE_E2E_V1 (§0) — enforce the V1
        // bounds invariant on EVERY payload reconstruction. Cheaper
        // than repeating the check at every call site and prevents
        // any bypass path via a malformed row surviving through the
        // DB layer.
        self.validate_version_invariants()?;
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

        // PERPS_V2_BACKEND_RPC_SIMULATION_INTEGRATION_V1 — thread the
        // persisted V2 bounds through. V1 intents persist `0, 0` (via
        // the DEFAULT in migration 0065) which reproduces the legacy
        // strict-price behaviour byte-for-byte. V2 intents persist the
        // trader-signed bounds so the reconstructed payload byte-matches
        // what the trader hashed.
        PerpTradePayload::new(
            intent_id_to_b256(&self.intent_id.to_string())?,
            self.buyer.clone(),
            self.seller.clone(),
            u128::from(self.market_id),
            self.size_1e8,
            self.price_1e8,
            self.max_execution_price_1e8,
            self.min_execution_price_1e8,
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
            protocol_version: crate::execution::perp_trade::PerpsProtocolVersion::V1,
            max_execution_price_1e8: 0,
            min_execution_price_1e8: 0,
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

    // PERPS_CLOSE_PNL_BUG_RETIRED_AND_V2_ACCOUNTING_DESIGN
    #[test]
    fn abandoned_is_terminal_and_not_retire_eligible() {
        // The `Abandoned` variant itself is a terminal state; a second
        // retire attempt must be refused (idempotency handled at the
        // repository transaction boundary; here we only care about the
        // enum-level property that `Abandoned.is_retire_eligible()` is
        // false).
        assert!(!ExecutionIntentStatus::Abandoned.is_retire_eligible());
    }

    #[test]
    fn retire_eligible_covers_administratively_safe_states_only() {
        // Administratively safe: nothing broadcast yet.
        assert!(ExecutionIntentStatus::Pending.is_retire_eligible());
        assert!(ExecutionIntentStatus::DryRun.is_retire_eligible());
        assert!(ExecutionIntentStatus::CalldataReady.is_retire_eligible());
        assert!(ExecutionIntentStatus::SimulationOk.is_retire_eligible());
        assert!(ExecutionIntentStatus::SimulationFailed.is_retire_eligible());
        // Post-broadcast or terminal: MUST NOT be overwritten.
        // Prepared / Submitted are in-flight — the reconciler owns them.
        assert!(!ExecutionIntentStatus::Prepared.is_retire_eligible());
        assert!(!ExecutionIntentStatus::Submitted.is_retire_eligible());
        assert!(!ExecutionIntentStatus::Confirmed.is_retire_eligible());
        assert!(!ExecutionIntentStatus::Failed.is_retire_eligible());
        assert!(!ExecutionIntentStatus::Abandoned.is_retire_eligible());
    }

    #[test]
    fn abandoned_status_roundtrips_through_string_form() {
        // Repository writes go via db::models::execution_status_to_str;
        // reads go via execution_status_from_str_public. Both must know
        // about `Abandoned` or a persisted retired intent cannot be
        // deserialized by the reconciler / audit paths.
        let s = crate::db::models::execution_status_to_str(ExecutionIntentStatus::Abandoned);
        assert_eq!(s, "abandoned");
        let back = crate::db::models::execution_status_from_str_public("abandoned").unwrap();
        assert_eq!(back, ExecutionIntentStatus::Abandoned);
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

    // ── PERPS_V2_BACKEND_ANVIL_LIVE_E2E_V1 §0 V1 bounds invariant ──

    #[test]
    fn v1_with_zero_bounds_is_valid() {
        let intent = intent_with_deadline_ms(1_789_656_028_000);
        assert_eq!(
            intent.protocol_version,
            crate::execution::perp_trade::PerpsProtocolVersion::V1
        );
        assert!(intent.validate_version_invariants().is_ok());
        assert!(intent.perp_trade_payload().is_ok());
    }

    #[test]
    fn v1_with_nonzero_max_bound_is_refused() {
        let mut intent = intent_with_deadline_ms(1_789_656_028_000);
        intent.max_execution_price_1e8 = 1;
        let err = intent.validate_version_invariants().unwrap_err();
        assert!(
            format!("{err}").contains("V1 intent")
                && format!("{err}").contains("non-zero V2 price bounds"),
            "got: {err}"
        );
        // Reload path also fails closed.
        assert!(intent.perp_trade_payload().is_err());
    }

    #[test]
    fn v1_with_nonzero_min_bound_is_refused() {
        let mut intent = intent_with_deadline_ms(1_789_656_028_000);
        intent.min_execution_price_1e8 = 1;
        assert!(intent.validate_version_invariants().is_err());
        assert!(intent.perp_trade_payload().is_err());
    }

    #[test]
    fn v2_with_nonzero_bounds_is_valid() {
        let mut intent = intent_with_deadline_ms(1_789_656_028_000);
        intent.protocol_version = crate::execution::perp_trade::PerpsProtocolVersion::V2;
        intent.max_execution_price_1e8 = 300_500_000_000;
        intent.min_execution_price_1e8 = 299_500_000_000;
        assert!(intent.validate_version_invariants().is_ok());
        let payload = intent.perp_trade_payload().unwrap();
        assert_eq!(payload.max_execution_price_1e8, 300_500_000_000);
        assert_eq!(payload.min_execution_price_1e8, 299_500_000_000);
    }

    #[test]
    fn v2_with_zero_bounds_is_valid() {
        // Strict-price V2 case: bounds = 0, 0 explicitly permitted.
        let mut intent = intent_with_deadline_ms(1_789_656_028_000);
        intent.protocol_version = crate::execution::perp_trade::PerpsProtocolVersion::V2;
        assert!(intent.validate_version_invariants().is_ok());
        assert!(intent.perp_trade_payload().is_ok());
    }
}
