//! `BACKEND-HYBRID-V2-SIGNER-AND-EXECUTION-V1` (Foundation package,
//! parts C–H) — Hybrid V2 pre-broadcast execution scaffolding.
//!
//! This module is intentionally SCOPED to the pre-broadcast surface:
//! identity, state machine, persistence, deterministic ABI plan,
//! target-and-selector policy, and pre-execution canonical validation.
//!
//! Frozen safety invariants enforced here:
//!   * `BROADCAST_IS_DISABLED` — no method in this module (or any it
//!     surfaces) issues `eth_sendTransaction` or
//!     `eth_sendRawTransaction`. The state machine's only forward
//!     terminal from `ReadyForBroadcast` is `BroadcastDisabled`.
//!   * `HYBRID_V2_CHAIN_SOURCE_IS_STRICTLY_READ_ONLY` — this module is
//!     read-only against chain sources; it depends on `ChainSource` /
//!     `ChainViewProvider` only for observation.
//!   * `BASE_MAINNET_8453_IS_FORBIDDEN` — target policy rejects chain
//!     id 8453 outright.
//!   * `CHAIN_STATE_IS_CANONICAL_POSTGRES_IS_A_REBUILDABLE_NON_CANONICAL_PROJECTION`
//!     — every persisted execution row is a projection of intent, not a
//!     canonical economic fact.
//!
//! Modules deliberately NOT included in the foundation package:
//!   * simulation (next package)
//!   * signer abstraction (later package)
//!   * orchestrator / admin routes (later package)
//!   * broadcast (permanently disabled at trait level for HV2)

pub mod gas_policy;
pub mod identity;
pub mod nonce;
pub mod persistence;
pub mod plan;
pub mod preflight;
pub mod rpc;
pub mod signature_verify;
pub mod signer;
pub mod signer_firewall;
pub mod signer_production;
pub mod simulator;
pub mod state;
pub mod target_policy;

#[cfg(any(test, feature = "test-signer"))]
pub mod signer_ephemeral;

pub use gas_policy::{GasComputationOutcome, GasFeePolicy, GasPolicyError};
pub use identity::{derive_canonical_execution_id, CanonicalExecutionId};
pub use nonce::{NonceReservation, NonceReserveError, NonceReserver};
pub use persistence::{ExecutionRequestPatch, ExecutionRequestRow};
pub use plan::{ExecutionPlan, ExecutionPlanBuilder, OptionOrder, PlanError, SignedActionEnvelope};
pub use preflight::{PreflightChecker, PreflightRejection, TrustLevel};
pub use rpc::{
    BlockTag, DecodedRevert, EthCallOutcome, EthCallRequest, ExecutionRpcClient, ExecutionRpcError,
    FeeHistory, HttpExecutionRpcClient, ALLOWED_METHODS, KNOWN_CUSTOM_ERROR_SELECTORS,
};
pub use signature_verify::{verify_signed_tx, SigVerifyError, VerifiedSignature};
pub use signer::{
    ExecutionSigner, SignedTx, SignerBackend, SignerError, SignerIdentity, SignerKind,
    SigningRequest,
};
pub use signer_firewall::{FirewallRejection, SignerPolicyFirewall};
pub use signer_production::ProductionSignerUnavailable;
pub use simulator::{ExecutionSimulator, SimulationError, SimulationOutcome};
pub use state::{ExecutionPhase, PhaseParseError, PhaseTransitionError};
pub use target_policy::{AllowedTarget, PolicyError, TargetPolicy};
