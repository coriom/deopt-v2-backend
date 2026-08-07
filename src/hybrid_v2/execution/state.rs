//! Execution phase state machine for the Hybrid V2 pre-broadcast
//! pipeline.
//!
//! Frozen safety:
//! - `BROADCAST_IS_DISABLED` — the only forward transition out of
//!   `ReadyForBroadcast` is `BroadcastDisabled` (terminal). There is
//!   deliberately NO `Broadcasted` state in this milestone; adding one
//!   would require a corresponding trait method that could send a
//!   transaction, which is banned at the RPC-provider layer.
//! - Every legal edge here is mirrored by the SQL update in
//!   `HybridV2ProjectionStore::update_execution_phase` (conditional
//!   `WHERE phase = $from`) so a rogue caller cannot skip states.
//! - Terminal states (`BroadcastDisabled`, `Cancelled`, `Stale`,
//!   `Failed`) have NO outgoing edges. A terminal row is immutable
//!   with respect to phase.
//!
//! Persistence: each variant maps to a stable SCREAMING_SNAKE_CASE
//! token used verbatim as the SQL `phase` column value and in the
//! CHECK constraint on `hybrid_v2_execution_requests`.

use std::fmt;

// -----------------------------------------------------------------
//                          PHASE ENUM
// -----------------------------------------------------------------

/// The 13 phases of a pre-broadcast Hybrid V2 execution. Ordered here
/// in typical forward progression for readability; the ordering has
/// no semantic weight — [`can_transition_to`] is the source of truth.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ExecutionPhase {
    Discovered,
    Validating,
    ReadyToSimulate,
    Simulating,
    SimulationSucceeded,
    SimulationFailed,
    AwaitingSignature,
    Signing,
    SignatureVerified,
    /// Logical-only. In this milestone broadcast is disabled: the only
    /// forward transition out of this state is `BroadcastDisabled`.
    ReadyForBroadcast,
    /// Terminal success state for the foundation package. Records that
    /// the pipeline advanced through simulation + signature but was
    /// deliberately not broadcast (RPC layer has no send_* method).
    BroadcastDisabled,
    /// Terminal — operator-initiated or safety-triggered cancellation.
    Cancelled,
    /// Terminal — age-based abandonment. The orchestrator ages out
    /// rows whose timestamps drift past a bounded window.
    Stale,
    /// Terminal — any pipeline failure not classified above.
    Failed,
}

impl ExecutionPhase {
    /// Stable SCREAMING_SNAKE_CASE token used in the SQL `phase`
    /// column. Must remain byte-identical to the CHECK constraint
    /// values in `migrations/0049_hybrid_v2_execution.sql`.
    pub fn as_str(self) -> &'static str {
        match self {
            ExecutionPhase::Discovered => "DISCOVERED",
            ExecutionPhase::Validating => "VALIDATING",
            ExecutionPhase::ReadyToSimulate => "READY_TO_SIMULATE",
            ExecutionPhase::Simulating => "SIMULATING",
            ExecutionPhase::SimulationSucceeded => "SIMULATION_SUCCEEDED",
            ExecutionPhase::SimulationFailed => "SIMULATION_FAILED",
            ExecutionPhase::AwaitingSignature => "AWAITING_SIGNATURE",
            ExecutionPhase::Signing => "SIGNING",
            ExecutionPhase::SignatureVerified => "SIGNATURE_VERIFIED",
            ExecutionPhase::ReadyForBroadcast => "READY_FOR_BROADCAST",
            ExecutionPhase::BroadcastDisabled => "BROADCAST_DISABLED",
            ExecutionPhase::Cancelled => "CANCELLED",
            ExecutionPhase::Stale => "STALE",
            ExecutionPhase::Failed => "FAILED",
        }
    }

    /// Parse a persisted SCREAMING_SNAKE_CASE token. Returns
    /// `Err(PhaseParseError)` on any unknown value; the caller MUST
    /// treat this as a hard failure (fail closed).
    pub fn parse(s: &str) -> Result<Self, PhaseParseError> {
        Ok(match s {
            "DISCOVERED" => ExecutionPhase::Discovered,
            "VALIDATING" => ExecutionPhase::Validating,
            "READY_TO_SIMULATE" => ExecutionPhase::ReadyToSimulate,
            "SIMULATING" => ExecutionPhase::Simulating,
            "SIMULATION_SUCCEEDED" => ExecutionPhase::SimulationSucceeded,
            "SIMULATION_FAILED" => ExecutionPhase::SimulationFailed,
            "AWAITING_SIGNATURE" => ExecutionPhase::AwaitingSignature,
            "SIGNING" => ExecutionPhase::Signing,
            "SIGNATURE_VERIFIED" => ExecutionPhase::SignatureVerified,
            "READY_FOR_BROADCAST" => ExecutionPhase::ReadyForBroadcast,
            "BROADCAST_DISABLED" => ExecutionPhase::BroadcastDisabled,
            "CANCELLED" => ExecutionPhase::Cancelled,
            "STALE" => ExecutionPhase::Stale,
            "FAILED" => ExecutionPhase::Failed,
            other => return Err(PhaseParseError(other.to_string())),
        })
    }

    /// True iff `self` is a terminal state — no outgoing transitions
    /// permitted, and any observed transition attempt is a bug.
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            ExecutionPhase::BroadcastDisabled
                | ExecutionPhase::Cancelled
                | ExecutionPhase::Stale
                | ExecutionPhase::Failed
        )
    }

    /// Legal-edge matrix. Returns `true` iff `self -> next` is a legal
    /// transition. See the module docs for the closed set of edges.
    ///
    /// Terminal states permit no outgoing edges. A same-state
    /// self-loop is NOT permitted — the orchestrator must always
    /// advance forward or explicitly cancel/fail.
    pub fn can_transition_to(self, next: ExecutionPhase) -> bool {
        use ExecutionPhase::*;
        // Self-loops are always rejected. Every legal update advances
        // the machine.
        if self == next {
            return false;
        }
        // Terminal states are frozen.
        if self.is_terminal() {
            return false;
        }
        match (self, next) {
            (Discovered, Validating) => true,
            (Discovered, Cancelled) => true,
            (Discovered, Failed) => true,

            (Validating, ReadyToSimulate) => true,
            (Validating, Failed) => true,
            (Validating, Cancelled) => true,
            (Validating, Stale) => true,

            (ReadyToSimulate, Simulating) => true,
            (ReadyToSimulate, Cancelled) => true,
            (ReadyToSimulate, Stale) => true,

            (Simulating, SimulationSucceeded) => true,
            (Simulating, SimulationFailed) => true,
            (Simulating, Failed) => true,

            (SimulationSucceeded, AwaitingSignature) => true,
            (SimulationSucceeded, Cancelled) => true,
            (SimulationSucceeded, Stale) => true,

            (SimulationFailed, Failed) => true,

            (AwaitingSignature, Signing) => true,
            (AwaitingSignature, Cancelled) => true,
            (AwaitingSignature, Stale) => true,

            (Signing, SignatureVerified) => true,
            (Signing, Failed) => true,

            (SignatureVerified, ReadyForBroadcast) => true,
            (SignatureVerified, Failed) => true,

            // Broadcast is disabled in this milestone. There is no
            // `Broadcasted` state and no send_* method. The only
            // forward transition out of ReadyForBroadcast is the
            // terminal marker BroadcastDisabled.
            (ReadyForBroadcast, BroadcastDisabled) => true,

            _ => false,
        }
    }

    /// Yield the legal successors of `self`. Exhaustive; used by the
    /// state-machine table tests to prove the matrix is closed.
    pub fn legal_successors(self) -> &'static [ExecutionPhase] {
        use ExecutionPhase::*;
        match self {
            Discovered => &[Validating, Cancelled, Failed],
            Validating => &[ReadyToSimulate, Failed, Cancelled, Stale],
            ReadyToSimulate => &[Simulating, Cancelled, Stale],
            Simulating => &[SimulationSucceeded, SimulationFailed, Failed],
            SimulationSucceeded => &[AwaitingSignature, Cancelled, Stale],
            SimulationFailed => &[Failed],
            AwaitingSignature => &[Signing, Cancelled, Stale],
            Signing => &[SignatureVerified, Failed],
            SignatureVerified => &[ReadyForBroadcast, Failed],
            ReadyForBroadcast => &[BroadcastDisabled],
            BroadcastDisabled | Cancelled | Stale | Failed => &[],
        }
    }

    /// Every variant in canonical enumeration order. Used by parse /
    /// round-trip tests.
    pub const ALL: &'static [ExecutionPhase] = &[
        ExecutionPhase::Discovered,
        ExecutionPhase::Validating,
        ExecutionPhase::ReadyToSimulate,
        ExecutionPhase::Simulating,
        ExecutionPhase::SimulationSucceeded,
        ExecutionPhase::SimulationFailed,
        ExecutionPhase::AwaitingSignature,
        ExecutionPhase::Signing,
        ExecutionPhase::SignatureVerified,
        ExecutionPhase::ReadyForBroadcast,
        ExecutionPhase::BroadcastDisabled,
        ExecutionPhase::Cancelled,
        ExecutionPhase::Stale,
        ExecutionPhase::Failed,
    ];
}

impl fmt::Display for ExecutionPhase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

// -----------------------------------------------------------------
//                          ERRORS
// -----------------------------------------------------------------

/// Unknown token when parsing a persisted `phase` column value.
/// Callers MUST fail closed on this — an unknown phase indicates a
/// forward-compat mismatch, not a recoverable soft error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhaseParseError(pub String);

impl fmt::Display for PhaseParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unknown ExecutionPhase token: {}", self.0)
    }
}

impl std::error::Error for PhaseParseError {}

/// Attempted transition from `from -> to` that is not in the legal
/// edge matrix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhaseTransitionError {
    pub from: ExecutionPhase,
    pub to: ExecutionPhase,
}

impl fmt::Display for PhaseTransitionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "illegal execution phase transition: {} -> {}",
            self.from.as_str(),
            self.to.as_str()
        )
    }
}

impl std::error::Error for PhaseTransitionError {}

// -----------------------------------------------------------------
//                          UNIT TESTS
// -----------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn as_str_and_parse_round_trip_all_variants() {
        for p in ExecutionPhase::ALL {
            assert_eq!(ExecutionPhase::parse(p.as_str()), Ok(*p));
        }
    }

    #[test]
    fn parse_rejects_unknown_tokens() {
        assert!(ExecutionPhase::parse("BROADCASTED").is_err());
        assert!(ExecutionPhase::parse("").is_err());
        assert!(ExecutionPhase::parse("discovered").is_err());
    }

    #[test]
    fn terminal_phases_are_frozen() {
        for term in [
            ExecutionPhase::BroadcastDisabled,
            ExecutionPhase::Cancelled,
            ExecutionPhase::Stale,
            ExecutionPhase::Failed,
        ] {
            assert!(term.is_terminal(), "{term:?} should be terminal");
            for next in ExecutionPhase::ALL {
                assert!(
                    !term.can_transition_to(*next),
                    "terminal {term:?} illegally transitions to {next:?}"
                );
            }
            assert!(term.legal_successors().is_empty());
        }
    }

    #[test]
    fn non_terminal_phases_advertise_at_least_one_forward_edge() {
        for p in ExecutionPhase::ALL {
            if p.is_terminal() {
                continue;
            }
            assert!(
                !p.legal_successors().is_empty(),
                "{p:?} should advertise at least one successor"
            );
        }
    }

    #[test]
    fn self_loops_are_rejected() {
        for p in ExecutionPhase::ALL {
            assert!(
                !p.can_transition_to(*p),
                "self-loop {p:?} should be rejected"
            );
        }
    }

    #[test]
    fn legal_edge_matrix_matches_successors_table() {
        // For every ordered pair, `can_transition_to` MUST agree with
        // `legal_successors`. This closes the door on drift between
        // the two representations.
        for from in ExecutionPhase::ALL {
            let successors: Vec<ExecutionPhase> = from.legal_successors().to_vec();
            for to in ExecutionPhase::ALL {
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
    fn broadcast_is_disabled_only_forward_from_ready_for_broadcast() {
        // The `BROADCAST_IS_DISABLED` invariant: from
        // `ReadyForBroadcast`, only `BroadcastDisabled` is reachable.
        let succs = ExecutionPhase::ReadyForBroadcast.legal_successors();
        assert_eq!(succs, &[ExecutionPhase::BroadcastDisabled]);
    }

    #[test]
    fn happy_path_walk_is_legal_end_to_end() {
        // The canonical "successful" walk under this milestone ends at
        // BroadcastDisabled (not Broadcasted).
        let walk = [
            ExecutionPhase::Discovered,
            ExecutionPhase::Validating,
            ExecutionPhase::ReadyToSimulate,
            ExecutionPhase::Simulating,
            ExecutionPhase::SimulationSucceeded,
            ExecutionPhase::AwaitingSignature,
            ExecutionPhase::Signing,
            ExecutionPhase::SignatureVerified,
            ExecutionPhase::ReadyForBroadcast,
            ExecutionPhase::BroadcastDisabled,
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

    #[test]
    fn simulation_failed_only_goes_to_failed() {
        // Regression guard: SimulationFailed is intentionally a
        // narrow funnel — the only next step is Failed, which is
        // terminal. This keeps the "post-simulation-failure retry"
        // story explicit (create a NEW execution row) instead of
        // recycling the failed one.
        let succs = ExecutionPhase::SimulationFailed.legal_successors();
        assert_eq!(succs, &[ExecutionPhase::Failed]);
    }

    #[test]
    fn discovered_forbids_direct_jump_to_simulating() {
        // Skipping validation is explicitly not allowed.
        assert!(!ExecutionPhase::Discovered.can_transition_to(ExecutionPhase::Simulating));
        assert!(!ExecutionPhase::Discovered.can_transition_to(ExecutionPhase::AwaitingSignature));
        assert!(!ExecutionPhase::Discovered.can_transition_to(ExecutionPhase::ReadyForBroadcast));
    }

    #[test]
    fn transition_error_is_serializable_debug() {
        let err = PhaseTransitionError {
            from: ExecutionPhase::Discovered,
            to: ExecutionPhase::ReadyForBroadcast,
        };
        let msg = format!("{err}");
        assert!(msg.contains("DISCOVERED"));
        assert!(msg.contains("READY_FOR_BROADCAST"));
    }
}
