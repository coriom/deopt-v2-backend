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

pub mod identity;
pub mod persistence;
pub mod plan;
pub mod preflight;
pub mod state;
pub mod target_policy;

pub use identity::{derive_canonical_execution_id, CanonicalExecutionId};
pub use persistence::{ExecutionRequestPatch, ExecutionRequestRow};
pub use plan::{ExecutionPlan, ExecutionPlanBuilder, OptionOrder, PlanError, SignedActionEnvelope};
pub use preflight::{PreflightChecker, PreflightRejection, TrustLevel};
pub use state::{ExecutionPhase, PhaseParseError, PhaseTransitionError};
pub use target_policy::{AllowedTarget, PolicyError, TargetPolicy};
