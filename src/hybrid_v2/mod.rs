//! `BACKEND-SUBACCOUNT-CANONICAL-STATE-AND-INDEXER-V1` — Hybrid V2 canonical
//! projection layer.
//!
//! Frozen canonicality rule:
//! `CHAIN_STATE_IS_CANONICAL_POSTGRES_IS_A_REBUILDABLE_PROJECTION`.
//!
//! PostgreSQL may store raw chain logs, decoded events, deterministic
//! projections, derived query models, reconciliation status, and indexer
//! checkpoints. PostgreSQL must NEVER independently decide any canonical
//! economic fact (ownership / balances / reservations / positions /
//! order lifecycle / recovery state / fee accounting).
//!
//! Every projection row is traceable to a raw log OR to immutable
//! manifest data OR to a bounded canonical contract view used for
//! verification.
//!
//! Status: `EXPERIMENTAL — NOT SECURITY APPROVED`.

pub mod decoder;
pub mod events;
pub mod manifest;
pub mod reducer;
pub mod snapshot;
pub mod topics;

pub use events::{EventKind, HybridV2Event};
pub use manifest::{ManifestParams, ManifestValidationError, ManifestValidator, NetworkPolicy};
pub use snapshot::{PinnedSnapshots, SourceMetadata};
pub use topics::TopicCatalogue;
