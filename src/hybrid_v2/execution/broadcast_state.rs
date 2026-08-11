//! Broadcast state machine + row type for the Hybrid V2 broadcast
//! pipeline (Part E of
//! `BACKEND-HYBRID-V2-BROADCAST-AND-CONFIRMATION-V1`).
//!
//! The broadcast state machine is DELIBERATELY separated from the
//! pre-broadcast [`crate::hybrid_v2::execution::state::ExecutionPhase`]
//! machine. The execution phases end at `ReadyForBroadcast` /
//! `BroadcastDisabled`; the broadcast machine picks up from there and
//! tracks the on-chain submission lifecycle in its own persisted table.
//!
//! Frozen safety:
//! - `NO_AUTOMATIC_NONCE_REPLACEMENT` / `NO_AUTOMATIC_FEE_BUMP_OR_RBF`
//!   — the matrix has no auto-resubmit edges. `SubmissionUnknown`
//!   only advances via hash-based investigation of the mempool /
//!   receipts, never by re-signing.
//! - `UNKNOWN_SUBMISSION_OUTCOME_NEVER_CAUSES_A_DIFFERENT_TRANSACTION_TO_BE_SIGNED`
//!   — the outbox persists BROADCASTING (and the local tx hash) BEFORE
//!   calling the network. A restart mid-call sees `Broadcasting` and
//!   knows to investigate via `transaction_by_hash`, never to re-sign.
//! - Terminal states (`MinedReverted`, `Dropped`,
//!   `CancelledBeforeBroadcast`, `ManualInterventionRequired`) have NO
//!   outgoing transitions.

use std::fmt;

// -----------------------------------------------------------------
//                          PHASE ENUM
// -----------------------------------------------------------------

/// Broadcast lifecycle phases for one submitted execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BroadcastPhase {
    /// Config-gated: broadcast_enabled = false OR the request is not
    /// yet at SignatureVerified. Initial state at row insert.
    BroadcastDisabled,
    /// Signature verified, gas + fee within policy, ready to submit.
    ReadyForBroadcast,
    /// In-flight: the outbox has persisted the tx hash and phase, then
    /// invoked `eth_sendRawTransaction`. A restart mid-call resumes
    /// here.
    Broadcasting,
    /// Network glitch: submit returned Timeout / Transport / Unavailable.
    /// Recovery is via `transaction_by_hash(envelope_hash)` — NEVER by
    /// re-signing or replacing the nonce.
    SubmissionUnknown,
    /// Provider accepted the raw tx.
    Submitted,
    /// Provider has the tx in its mempool but no receipt yet.
    Pending,
    /// Receipt observed with status = 1.
    MinedSuccess,
    /// Receipt observed with status = 0. Nonce consumed. Terminal.
    MinedReverted,
    /// Receipt observed; awaiting confirmation depth.
    Confirming,
    /// Confirmed at configured depth AND matched to indexer projection.
    Confirmed,
    /// The receipt's block was orphaned by a reorg.
    Reorged,
    /// No longer in mempool + no receipt. Terminal absent operator
    /// escalation.
    Dropped,
    /// Operator must intervene. Terminal.
    ManualInterventionRequired,
    /// Operator or policy cancelled the execution before broadcast.
    /// Terminal.
    CancelledBeforeBroadcast,
}

impl BroadcastPhase {
    /// Stable SCREAMING_SNAKE_CASE token used in the SQL `phase`
    /// column. Must remain byte-identical to the CHECK constraint
    /// values in `migrations/0051_hybrid_v2_broadcast_state.sql`.
    pub fn as_str(self) -> &'static str {
        match self {
            BroadcastPhase::BroadcastDisabled => "BROADCAST_DISABLED",
            BroadcastPhase::ReadyForBroadcast => "READY_FOR_BROADCAST",
            BroadcastPhase::Broadcasting => "BROADCASTING",
            BroadcastPhase::SubmissionUnknown => "SUBMISSION_UNKNOWN",
            BroadcastPhase::Submitted => "SUBMITTED",
            BroadcastPhase::Pending => "PENDING",
            BroadcastPhase::MinedSuccess => "MINED_SUCCESS",
            BroadcastPhase::MinedReverted => "MINED_REVERTED",
            BroadcastPhase::Confirming => "CONFIRMING",
            BroadcastPhase::Confirmed => "CONFIRMED",
            BroadcastPhase::Reorged => "REORGED",
            BroadcastPhase::Dropped => "DROPPED",
            BroadcastPhase::ManualInterventionRequired => "MANUAL_INTERVENTION_REQUIRED",
            BroadcastPhase::CancelledBeforeBroadcast => "CANCELLED_BEFORE_BROADCAST",
        }
    }

    pub fn parse(s: &str) -> Result<Self, BroadcastPhaseParseError> {
        Ok(match s {
            "BROADCAST_DISABLED" => BroadcastPhase::BroadcastDisabled,
            "READY_FOR_BROADCAST" => BroadcastPhase::ReadyForBroadcast,
            "BROADCASTING" => BroadcastPhase::Broadcasting,
            "SUBMISSION_UNKNOWN" => BroadcastPhase::SubmissionUnknown,
            "SUBMITTED" => BroadcastPhase::Submitted,
            "PENDING" => BroadcastPhase::Pending,
            "MINED_SUCCESS" => BroadcastPhase::MinedSuccess,
            "MINED_REVERTED" => BroadcastPhase::MinedReverted,
            "CONFIRMING" => BroadcastPhase::Confirming,
            "CONFIRMED" => BroadcastPhase::Confirmed,
            "REORGED" => BroadcastPhase::Reorged,
            "DROPPED" => BroadcastPhase::Dropped,
            "MANUAL_INTERVENTION_REQUIRED" => BroadcastPhase::ManualInterventionRequired,
            "CANCELLED_BEFORE_BROADCAST" => BroadcastPhase::CancelledBeforeBroadcast,
            other => return Err(BroadcastPhaseParseError(other.to_string())),
        })
    }

    /// True iff `self` is terminal — no outgoing edges.
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            BroadcastPhase::MinedReverted
                | BroadcastPhase::Dropped
                | BroadcastPhase::CancelledBeforeBroadcast
                | BroadcastPhase::ManualInterventionRequired
        )
    }

    /// Legal-edge matrix. Returns true iff `self -> next` is permitted.
    ///
    /// Self-loops are rejected — every legal transition advances the
    /// machine. `ManualInterventionRequired` / `CancelledBeforeBroadcast`
    /// / `MinedReverted` / `Dropped` are terminal.
    pub fn can_transition_to(self, next: BroadcastPhase) -> bool {
        use BroadcastPhase::*;
        if self == next {
            return false;
        }
        if self.is_terminal() {
            return false;
        }
        match (self, next) {
            // BROADCAST_DISABLED
            (BroadcastDisabled, ReadyForBroadcast) => true,
            (BroadcastDisabled, CancelledBeforeBroadcast) => true,
            // Startup / lazy race: the outbox may transition directly
            // BroadcastDisabled -> Broadcasting when broadcast is enabled
            // AND we go straight to the send path in one lock hold.
            (BroadcastDisabled, Broadcasting) => true,
            // Firewall reject before any submission — the outbox escalates
            // straight to manual intervention.
            (BroadcastDisabled, ManualInterventionRequired) => true,

            // READY_FOR_BROADCAST
            (ReadyForBroadcast, Broadcasting) => true,
            (ReadyForBroadcast, CancelledBeforeBroadcast) => true,
            (ReadyForBroadcast, ManualInterventionRequired) => true,

            // BROADCASTING
            (Broadcasting, Submitted) => true,
            (Broadcasting, SubmissionUnknown) => true,
            (Broadcasting, ManualInterventionRequired) => true,

            // SUBMISSION_UNKNOWN — recovery via hash lookup
            (SubmissionUnknown, Submitted) => true,
            (SubmissionUnknown, Pending) => true,
            (SubmissionUnknown, MinedSuccess) => true,
            (SubmissionUnknown, MinedReverted) => true,
            (SubmissionUnknown, Dropped) => true,
            (SubmissionUnknown, ManualInterventionRequired) => true,

            // SUBMITTED
            (Submitted, Pending) => true,
            (Submitted, MinedSuccess) => true,
            (Submitted, MinedReverted) => true,
            (Submitted, Dropped) => true,
            (Submitted, Reorged) => true,
            (Submitted, ManualInterventionRequired) => true,

            // PENDING
            (Pending, MinedSuccess) => true,
            (Pending, MinedReverted) => true,
            (Pending, Dropped) => true,
            (Pending, Reorged) => true,
            (Pending, ManualInterventionRequired) => true,

            // MINED_SUCCESS
            (MinedSuccess, Confirming) => true,
            (MinedSuccess, Reorged) => true,
            (MinedSuccess, ManualInterventionRequired) => true,

            // CONFIRMING
            (Confirming, Confirmed) => true,
            (Confirming, Reorged) => true,
            (Confirming, ManualInterventionRequired) => true,

            // CONFIRMED — very rare deep reorg
            (Confirmed, Reorged) => true,
            (Confirmed, ManualInterventionRequired) => true,

            // REORGED
            (Reorged, Pending) => true,
            (Reorged, Confirming) => true,
            (Reorged, ManualInterventionRequired) => true,

            _ => false,
        }
    }

    /// Enumerate the legal successors of `self`. Kept in sync with
    /// `can_transition_to`; tests prove agreement.
    pub fn legal_successors(self) -> &'static [BroadcastPhase] {
        use BroadcastPhase::*;
        match self {
            BroadcastDisabled => &[
                ReadyForBroadcast,
                CancelledBeforeBroadcast,
                Broadcasting,
                ManualInterventionRequired,
            ],
            ReadyForBroadcast => &[
                Broadcasting,
                CancelledBeforeBroadcast,
                ManualInterventionRequired,
            ],
            Broadcasting => &[Submitted, SubmissionUnknown, ManualInterventionRequired],
            SubmissionUnknown => &[
                Submitted,
                Pending,
                MinedSuccess,
                MinedReverted,
                Dropped,
                ManualInterventionRequired,
            ],
            Submitted => &[
                Pending,
                MinedSuccess,
                MinedReverted,
                Dropped,
                Reorged,
                ManualInterventionRequired,
            ],
            Pending => &[
                MinedSuccess,
                MinedReverted,
                Dropped,
                Reorged,
                ManualInterventionRequired,
            ],
            MinedSuccess => &[Confirming, Reorged, ManualInterventionRequired],
            Confirming => &[Confirmed, Reorged, ManualInterventionRequired],
            Confirmed => &[Reorged, ManualInterventionRequired],
            Reorged => &[Pending, Confirming, ManualInterventionRequired],
            MinedReverted | Dropped | CancelledBeforeBroadcast | ManualInterventionRequired => &[],
        }
    }

    pub const ALL: &'static [BroadcastPhase] = &[
        BroadcastPhase::BroadcastDisabled,
        BroadcastPhase::ReadyForBroadcast,
        BroadcastPhase::Broadcasting,
        BroadcastPhase::SubmissionUnknown,
        BroadcastPhase::Submitted,
        BroadcastPhase::Pending,
        BroadcastPhase::MinedSuccess,
        BroadcastPhase::MinedReverted,
        BroadcastPhase::Confirming,
        BroadcastPhase::Confirmed,
        BroadcastPhase::Reorged,
        BroadcastPhase::Dropped,
        BroadcastPhase::ManualInterventionRequired,
        BroadcastPhase::CancelledBeforeBroadcast,
    ];
}

impl fmt::Display for BroadcastPhase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Unknown token when parsing a persisted broadcast `phase` column value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BroadcastPhaseParseError(pub String);

impl fmt::Display for BroadcastPhaseParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unknown BroadcastPhase token: {}", self.0)
    }
}

impl std::error::Error for BroadcastPhaseParseError {}

// -----------------------------------------------------------------
//                          ROW + PATCH
// -----------------------------------------------------------------

/// One row of `hybrid_v2_broadcast_state`. Every field maps 1:1 to the
/// column of the same name in migration 0051.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BroadcastStateRow {
    pub canonical_execution_id: String,
    /// 0x-prefixed 32-byte hex. `None` until the outbox derives the
    /// local tx hash. Immutable once set (SQL trigger + in-memory
    /// enforcement).
    pub tx_hash: Option<String>,
    /// `envelope_hash` as 0x-prefixed hex. Same value as `tx_hash` for
    /// EIP-1559 (both are `keccak256(raw_bytes)`); the pair is kept as
    /// two columns to make audits explicit. Immutable once set.
    pub envelope_hash: Option<String>,
    /// `envelope_bytes_hash` — `keccak256(raw_bytes)`. For EIP-1559
    /// this equals `envelope_hash` / `tx_hash`; retained for future
    /// tx-type extensibility.
    pub envelope_bytes_hash: Option<String>,
    pub phase: BroadcastPhase,
    pub submission_attempt_count: i32,
    pub first_submission_at_ms: Option<i64>,
    pub last_submission_at_ms: Option<i64>,
    pub provider_classification: Option<String>,
    pub receipt_tx_hash: Option<String>,
    pub receipt_block_number: Option<i64>,
    pub receipt_block_hash: Option<String>,
    /// `0` or `1`.
    pub receipt_status: Option<i16>,
    pub gas_used: Option<i64>,
    pub effective_gas_price_wei: Option<String>,
    pub actual_tx_cost_wei: Option<String>,
    pub confirmation_count: i32,
    pub canonicality_state: String,
    pub correlated_execution_status: Option<String>,
    pub last_checked_head: Option<i64>,
    pub last_checked_finalized_head: Option<i64>,
    pub reorg_count: i32,
    pub failure_class: Option<String>,
    pub failure_detail: Option<String>,
    pub terminal_at_ms: Option<i64>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

/// Sparse patch applied atomically with a phase transition. Only
/// `Some(_)` fields are written; `None` leaves the DB column untouched.
///
/// `tx_hash` and `envelope_hash` are additionally enforced immutable
/// once set (SQL trigger + in-memory rejection).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BroadcastStatePatch {
    pub submission_attempt_count: Option<i32>,
    pub first_submission_at_ms: Option<i64>,
    pub last_submission_at_ms: Option<i64>,
    pub provider_classification: Option<String>,
    pub receipt_tx_hash: Option<String>,
    pub receipt_block_number: Option<i64>,
    pub receipt_block_hash: Option<String>,
    pub receipt_status: Option<i16>,
    pub gas_used: Option<i64>,
    pub effective_gas_price_wei: Option<String>,
    pub actual_tx_cost_wei: Option<String>,
    pub confirmation_count: Option<i32>,
    pub canonicality_state: Option<String>,
    pub correlated_execution_status: Option<String>,
    pub last_checked_head: Option<i64>,
    pub last_checked_finalized_head: Option<i64>,
    pub reorg_count: Option<i32>,
    pub failure_class: Option<String>,
    pub failure_detail: Option<String>,
    pub terminal_at_ms: Option<i64>,
}

// -----------------------------------------------------------------
//                          UNIT TESTS
// -----------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn as_str_and_parse_round_trip_all_variants() {
        for p in BroadcastPhase::ALL {
            assert_eq!(BroadcastPhase::parse(p.as_str()), Ok(*p));
        }
    }

    #[test]
    fn parse_rejects_unknown_tokens() {
        assert!(BroadcastPhase::parse("").is_err());
        assert!(BroadcastPhase::parse("submitted").is_err());
        assert!(BroadcastPhase::parse("UNKNOWN_STATE").is_err());
    }

    #[test]
    fn terminal_phases_are_frozen() {
        for term in [
            BroadcastPhase::MinedReverted,
            BroadcastPhase::Dropped,
            BroadcastPhase::CancelledBeforeBroadcast,
            BroadcastPhase::ManualInterventionRequired,
        ] {
            assert!(term.is_terminal(), "{term:?} should be terminal");
            for next in BroadcastPhase::ALL {
                assert!(
                    !term.can_transition_to(*next),
                    "terminal {term:?} illegally transitions to {next:?}"
                );
            }
            assert!(term.legal_successors().is_empty());
        }
    }

    #[test]
    fn self_loops_are_rejected() {
        for p in BroadcastPhase::ALL {
            assert!(!p.can_transition_to(*p), "self-loop {p:?} rejected");
        }
    }

    #[test]
    fn legal_matrix_matches_successors_table() {
        for from in BroadcastPhase::ALL {
            let successors: Vec<BroadcastPhase> = from.legal_successors().to_vec();
            for to in BroadcastPhase::ALL {
                let allowed = from.can_transition_to(*to);
                let listed = successors.contains(to);
                assert_eq!(
                    allowed, listed,
                    "matrix / successors disagreement: {from:?} -> {to:?} (allowed={allowed}, listed={listed})"
                );
            }
        }
    }

    #[test]
    fn broadcasting_only_reaches_submitted_unknown_or_manual() {
        // Frozen safety: the send path may only emit one of these
        // three outcomes. No auto-retry, no fee-bump, no nonce
        // replacement.
        let succs = BroadcastPhase::Broadcasting.legal_successors();
        assert_eq!(
            succs,
            &[
                BroadcastPhase::Submitted,
                BroadcastPhase::SubmissionUnknown,
                BroadcastPhase::ManualInterventionRequired,
            ]
        );
    }

    #[test]
    fn submission_unknown_only_advances_via_hash_lookup_outcomes() {
        // Every legal successor from SubmissionUnknown is an OBSERVATION
        // outcome (hash lookup, receipt lookup, mempool lookup). None
        // of them require re-signing.
        let succs = BroadcastPhase::SubmissionUnknown.legal_successors();
        for s in [
            BroadcastPhase::Submitted,
            BroadcastPhase::Pending,
            BroadcastPhase::MinedSuccess,
            BroadcastPhase::MinedReverted,
            BroadcastPhase::Dropped,
            BroadcastPhase::ManualInterventionRequired,
        ] {
            assert!(succs.contains(&s), "SubmissionUnknown must reach {s:?}");
        }
    }

    #[test]
    fn ready_for_broadcast_never_direct_to_submitted() {
        // Every path to Submitted MUST pass through Broadcasting so
        // the outbox has a chance to persist the tx hash + phase
        // before the network call.
        assert!(!BroadcastPhase::ReadyForBroadcast.can_transition_to(BroadcastPhase::Submitted));
        assert!(!BroadcastPhase::BroadcastDisabled.can_transition_to(BroadcastPhase::Submitted));
    }

    #[test]
    fn reorged_never_direct_to_confirmed() {
        // Reorged rows must re-observe Pending / Confirming before
        // being promoted back to Confirmed.
        assert!(!BroadcastPhase::Reorged.can_transition_to(BroadcastPhase::Confirmed));
    }

    #[test]
    fn happy_path_end_to_end() {
        let walk = [
            BroadcastPhase::BroadcastDisabled,
            BroadcastPhase::ReadyForBroadcast,
            BroadcastPhase::Broadcasting,
            BroadcastPhase::Submitted,
            BroadcastPhase::Pending,
            BroadcastPhase::MinedSuccess,
            BroadcastPhase::Confirming,
            BroadcastPhase::Confirmed,
        ];
        for pair in walk.windows(2) {
            assert!(
                pair[0].can_transition_to(pair[1]),
                "happy-path edge {:?} -> {:?} must be legal",
                pair[0],
                pair[1]
            );
        }
    }
}
