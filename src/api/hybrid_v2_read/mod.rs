//! `BACKEND-SUBACCOUNT-READ-API-AND-HISTORY-V1` — public read API for the
//! Hybrid V2 canonical projection layer.
//!
//! Frozen rules:
//! - Every response carries `CanonicalityMetadata` (deployment id, chain id,
//!   manifest hash, indexed/finalized block, canonicality level, generated
//!   at). Public consumers rely on this to interpret data freshness.
//! - No mutations. No signing. No chain writes.
//! - Deterministic cursor pagination. Cursors bind to (deployment, filter,
//!   consistency) — a stale cursor is rejected explicitly.
//! - Uint256 amounts serialise as decimal strings.
//! - Every canonical field is traceable to a canonical event, manifest data,
//!   or a canonical chain view.

pub mod cursor;
pub mod errors;
pub mod handlers;
pub mod history;
pub mod metadata;
pub mod openapi;
pub mod router;
pub mod serialization;
pub mod state;

pub use cursor::{Cursor, CursorBinding, CursorError};
pub use errors::{ApiError, ApiErrorBody, ApiErrorCode};
pub use handlers::*;
pub use history::{HistoryDirection, HistoryEvent, HistoryEventPayload, HistoryFilter};
pub use metadata::{CanonicalityLevel, CanonicalityMetadata, ConsistencyMode};
pub use openapi::openapi_spec_json;
pub use router::build_hybrid_v2_read_router;
pub use state::{DeploymentEntry, EmptyReadStore, HybridV2ApiState};
