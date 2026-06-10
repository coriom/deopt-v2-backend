//! M-P4c — Local/test-only execution-intent + tx-status fixture.
//!
//! Strictly local/test-only. Provides:
//!   * An in-memory store of synthetic execution intents.
//!   * A small status-machine for cycling intents through
//!     CREATED → PENDING → CONFIRMED | FAILED | REVERTED | STUCK.
//!   * HTTP handlers gated under `/admin/test/*` (admin Bearer required
//!     by the existing `admin_route_gate` middleware).
//!
//! ## Forbidden
//!
//! This module NEVER:
//!   * touches a public chain;
//!   * calls the signer / AWS / KMS;
//!   * broadcasts a transaction;
//!   * reads or writes `.env`;
//!   * mutates any production execution-transaction or
//!     option-execution-transaction row;
//!   * runs when `chain_id == 8453` (Base mainnet).
//!
//! ## Activation
//!
//! Default: **disabled**. `LocalTestFixturesConfig::disabled()` is the
//! only constructor used by the production startup path. Tests and the
//! `e2e:local` runbook construct an enabled config via
//! `LocalTestFixturesConfig::enabled_for_chain_id(chain_id)`; that
//! factory itself refuses mainnet (returns `disabled()`).
//!
//! Defence-in-depth: every HTTP handler additionally calls
//! `assert_enabled(state.chain_id)` which checks both the enabled flag
//! and the runtime chain_id. A disabled fixture or a mainnet chain_id
//! both surface as HTTP 404 — indistinguishable from a non-existent
//! route, by design.

use crate::api::AppState;
use crate::error::{BackendError, Result as BackendResult};
use crate::types::{now_ms, TimestampMs};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt::Write as _;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

/// Base mainnet chain id — fixtures must NEVER be enabled here.
pub const MAINNET_CHAIN_ID: u64 = 8453;

/// Hard guard for the local/test fixtures subsystem.
///
/// Disabled by default. The `enabled_for_chain_id` factory refuses
/// mainnet (returns `disabled()`); `assert_enabled` enforces the same
/// at every request as defence-in-depth.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LocalTestFixturesConfig {
    enabled: bool,
}

impl LocalTestFixturesConfig {
    /// Production default. Always returns the disabled config.
    pub fn disabled() -> Self {
        Self { enabled: false }
    }

    /// Returns an enabled config iff `chain_id != MAINNET_CHAIN_ID`.
    /// Mainnet always returns `disabled()`.
    pub fn enabled_for_chain_id(chain_id: u64) -> Self {
        if chain_id == MAINNET_CHAIN_ID {
            Self::disabled()
        } else {
            Self { enabled: true }
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Defence-in-depth runtime gate. Returns `BackendError::Config`
    /// when disabled OR when the runtime chain id is mainnet.
    /// Handlers map this error to HTTP 404 so a disabled fixture is
    /// indistinguishable from a non-existent route.
    pub fn assert_enabled(&self, chain_id: u64) -> BackendResult<()> {
        if chain_id == MAINNET_CHAIN_ID {
            return Err(BackendError::Config(
                "local test fixtures are forbidden on mainnet".to_string(),
            ));
        }
        if !self.enabled {
            return Err(BackendError::Config(
                "local test fixtures are disabled".to_string(),
            ));
        }
        Ok(())
    }
}

/// Synthetic intent status. Independent of the production
/// `OptionExecutionIntentStatus` and `ExecutionTransactionStatus`
/// vocabularies — the fixture's only purpose is to exercise the
/// frontend tx-polling UI through a small, well-defined set of
/// transitions. Production statuses are intentionally NOT reused so a
/// mistakenly-enabled fixture cannot impersonate a real execution row.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LocalTestIntentStatus {
    Created,
    Pending,
    Confirmed,
    Failed,
    Reverted,
    Stuck,
}

impl LocalTestIntentStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Created => "created",
            Self::Pending => "pending",
            Self::Confirmed => "confirmed",
            Self::Failed => "failed",
            Self::Reverted => "reverted",
            Self::Stuck => "stuck",
        }
    }

    pub fn parse(value: &str) -> BackendResult<Self> {
        match value {
            "created" => Ok(Self::Created),
            "pending" => Ok(Self::Pending),
            "confirmed" => Ok(Self::Confirmed),
            "failed" => Ok(Self::Failed),
            "reverted" => Ok(Self::Reverted),
            "stuck" => Ok(Self::Stuck),
            other => Err(BackendError::Config(format!(
                "invalid local test intent status: {other}"
            ))),
        }
    }

    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Confirmed | Self::Failed | Self::Reverted)
    }

    /// Allowed transitions:
    ///   Created -> Pending
    ///   Pending -> Confirmed | Failed | Reverted | Stuck
    ///   Stuck   -> Pending | Failed
    ///   Confirmed | Failed | Reverted are terminal.
    pub fn can_transition_to(self, to: LocalTestIntentStatus) -> bool {
        matches!(
            (self, to),
            (Self::Created, Self::Pending)
                | (
                    Self::Pending,
                    Self::Confirmed | Self::Failed | Self::Reverted | Self::Stuck
                )
                | (Self::Stuck, Self::Pending | Self::Failed)
        )
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct LocalTestIntentTransition {
    pub from: LocalTestIntentStatus,
    pub to: LocalTestIntentStatus,
    pub at_ms: TimestampMs,
}

#[derive(Clone, Debug, Serialize)]
pub struct LocalTestIntent {
    pub intent_id: Uuid,
    pub request_id: String,
    pub account: String,
    pub source_type: String,
    pub status: LocalTestIntentStatus,
    pub tx_hash: String,
    pub created_at_ms: TimestampMs,
    pub updated_at_ms: TimestampMs,
    pub transitions: Vec<LocalTestIntentTransition>,
    pub synthetic: bool,
}

/// Synthetic, clearly-marked tx hash. Deterministic per intent_id —
/// callers can recompute it without DB access for assertion purposes.
///
/// Format: `0xdeadbee5` prefix (4 bytes) + 12 zero bytes + 16 uuid
/// bytes = 32 bytes / 64 hex chars (66 with `0x`). The `deadbee5`
/// prefix is recognisable as synthetic on inspection; the embedded
/// uuid lets test code link a hash back to its origin intent.
pub fn synthetic_tx_hash(intent_id: &Uuid) -> String {
    let mut bytes = [0u8; 32];
    bytes[..4].copy_from_slice(&[0xde, 0xad, 0xbe, 0xe5]);
    bytes[16..32].copy_from_slice(intent_id.as_bytes());
    let mut out = String::with_capacity(66);
    out.push_str("0x");
    for b in &bytes {
        let _ = write!(out, "{:02x}", b);
    }
    out
}

/// In-memory only. Lives behind `Arc<Mutex<…>>` on `AppState`. Never
/// persisted, never round-tripped through PgRepository, never visible
/// to the production option_execution_transactions or
/// execution_transactions tables.
#[derive(Default, Debug)]
pub struct LocalTestIntentStore {
    inner: HashMap<Uuid, LocalTestIntent>,
}

impl LocalTestIntentStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn create(&mut self, account: String, source_type: String) -> LocalTestIntent {
        let intent_id = Uuid::new_v4();
        let now = now_ms();
        let tx_hash = synthetic_tx_hash(&intent_id);
        let intent = LocalTestIntent {
            intent_id,
            request_id: format!("test-{}", intent_id),
            account,
            source_type,
            status: LocalTestIntentStatus::Created,
            tx_hash,
            created_at_ms: now,
            updated_at_ms: now,
            transitions: Vec::new(),
            synthetic: true,
        };
        self.inner.insert(intent_id, intent.clone());
        intent
    }

    pub fn get(&self, intent_id: &Uuid) -> Option<&LocalTestIntent> {
        self.inner.get(intent_id)
    }

    pub fn len(&self) -> usize {
        self.inner.len()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Validates the transition, applies it, returns the updated
    /// intent. 404 if unknown; 400-shaped error if invalid transition.
    pub fn transition(
        &mut self,
        intent_id: &Uuid,
        to: LocalTestIntentStatus,
    ) -> BackendResult<LocalTestIntent> {
        let intent = self.inner.get_mut(intent_id).ok_or_else(|| {
            BackendError::Persistence(format!("local test intent not found: {intent_id}"))
        })?;
        if !intent.status.can_transition_to(to) {
            return Err(BackendError::Config(format!(
                "invalid local test intent transition: {} -> {}",
                intent.status.as_str(),
                to.as_str()
            )));
        }
        let from = intent.status;
        let now = now_ms();
        intent.status = to;
        intent.updated_at_ms = now;
        intent.transitions.push(LocalTestIntentTransition {
            from,
            to,
            at_ms: now,
        });
        Ok(intent.clone())
    }
}

// ──────────────────────────────────────────────────────────────────────
// HTTP handlers
// ──────────────────────────────────────────────────────────────────────

#[derive(Debug)]
pub struct LocalTestApiError {
    status: StatusCode,
    message: String,
}

impl LocalTestApiError {
    fn not_found() -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            message: "not found".to_string(),
        }
    }

    fn not_found_with(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            message: message.into(),
        }
    }

    fn bad_request(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: message.into(),
        }
    }

    fn internal() -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: "internal server error".to_string(),
        }
    }
}

impl axum::response::IntoResponse for LocalTestApiError {
    fn into_response(self) -> axum::response::Response {
        (
            self.status,
            Json(serde_json::json!({ "error": self.message })),
        )
            .into_response()
    }
}

#[derive(Debug, Deserialize)]
pub struct CreateLocalTestIntentRequest {
    /// Optional account hex (`0x` + 40 hex chars). Defaults to anvil[0]
    /// well-known public address when absent. Never accepted as a
    /// secret; treated as a label only.
    pub account: Option<String>,
    /// Optional source-type label (`option_orderbook_fill` |
    /// `option_rfq_fill`). Defaults to `option_orderbook_fill`.
    pub source_type: Option<String>,
}

/// Anvil[0] well-known public dev address. Public knowledge; no real
/// funds. Used as a friendly default so the Playwright wallet fixture
/// (which already pins this account) sees a matching intent owner.
pub const DEFAULT_TEST_ACCOUNT: &str = "0xf39Fd6e51aaD88F6F4ce6aB8827279cffFb92266";

fn map_account(input: Option<String>) -> Result<String, LocalTestApiError> {
    let s = input.unwrap_or_else(|| DEFAULT_TEST_ACCOUNT.to_string());
    if !s.starts_with("0x") || s.len() != 42 || !s[2..].chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(LocalTestApiError::bad_request(
            "account must be a 0x-prefixed 40-hex-char address",
        ));
    }
    Ok(s)
}

fn map_source_type(input: Option<String>) -> Result<String, LocalTestApiError> {
    match input.as_deref() {
        None | Some("option_orderbook_fill") => Ok("option_orderbook_fill".to_string()),
        Some("option_rfq_fill") => Ok("option_rfq_fill".to_string()),
        Some(other) => Err(LocalTestApiError::bad_request(format!(
            "invalid source_type: {other}"
        ))),
    }
}

fn assert_or_404(state: &AppState) -> Result<(), LocalTestApiError> {
    state
        .local_test_fixtures
        .assert_enabled(state.chain_id)
        .map_err(|_| LocalTestApiError::not_found())
}

pub async fn create_local_test_intent(
    State(state): State<AppState>,
    Json(body): Json<CreateLocalTestIntentRequest>,
) -> Result<Json<LocalTestIntent>, LocalTestApiError> {
    assert_or_404(&state)?;
    let account = map_account(body.account)?;
    let source_type = map_source_type(body.source_type)?;
    let mut store = state
        .local_test_intents
        .lock()
        .map_err(|_| LocalTestApiError::internal())?;
    let intent = store.create(account, source_type);
    Ok(Json(intent))
}

#[derive(Debug, Deserialize)]
pub struct TransitionLocalTestIntentRequest {
    pub to_status: String,
}

pub async fn transition_local_test_intent(
    State(state): State<AppState>,
    Path(intent_id): Path<String>,
    Json(body): Json<TransitionLocalTestIntentRequest>,
) -> Result<Json<LocalTestIntent>, LocalTestApiError> {
    assert_or_404(&state)?;
    let intent_uuid = Uuid::parse_str(&intent_id)
        .map_err(|_| LocalTestApiError::not_found_with("intent not found"))?;
    let to = LocalTestIntentStatus::parse(&body.to_status)
        .map_err(|e| LocalTestApiError::bad_request(e.to_string()))?;
    let mut store = state
        .local_test_intents
        .lock()
        .map_err(|_| LocalTestApiError::internal())?;
    match store.transition(&intent_uuid, to) {
        Ok(intent) => Ok(Json(intent)),
        Err(BackendError::Persistence(_)) => {
            Err(LocalTestApiError::not_found_with("intent not found"))
        }
        Err(e) => Err(LocalTestApiError::bad_request(e.to_string())),
    }
}

pub async fn get_local_test_intent(
    State(state): State<AppState>,
    Path(intent_id): Path<String>,
) -> Result<Json<LocalTestIntent>, LocalTestApiError> {
    assert_or_404(&state)?;
    let intent_uuid = Uuid::parse_str(&intent_id)
        .map_err(|_| LocalTestApiError::not_found_with("intent not found"))?;
    let store = state
        .local_test_intents
        .lock()
        .map_err(|_| LocalTestApiError::internal())?;
    store
        .get(&intent_uuid)
        .cloned()
        .map(Json)
        .ok_or_else(|| LocalTestApiError::not_found_with("intent not found"))
}

/// Frontend-facing read endpoint. Reads only the synthetic fixture
/// store — does NOT touch the production option_execution_transactions
/// or execution_transactions tables. Returns 404 when the fixture is
/// disabled (so frontend code falls back to its existing route
/// interception path).
///
/// The returned shape is a small, deliberately distinct envelope —
/// the `source: "local_test_fixture"` discriminator and the `synthetic:
/// true` flag are intentional defence-in-depth signals so a downstream
/// consumer can never mistake a fixture row for a real tx.
pub async fn get_local_test_tx_status(
    State(state): State<AppState>,
    Path(intent_id): Path<String>,
) -> Result<Json<serde_json::Value>, LocalTestApiError> {
    assert_or_404(&state)?;
    let intent_uuid = Uuid::parse_str(&intent_id)
        .map_err(|_| LocalTestApiError::not_found_with("intent not found"))?;
    let store = state
        .local_test_intents
        .lock()
        .map_err(|_| LocalTestApiError::internal())?;
    let intent = store
        .get(&intent_uuid)
        .ok_or_else(|| LocalTestApiError::not_found_with("intent not found"))?;
    Ok(Json(serde_json::json!({
        "source": "local_test_fixture",
        "synthetic": true,
        "intent_id": intent.intent_id,
        "request_id": intent.request_id,
        "account": intent.account,
        "source_type": intent.source_type,
        "status": intent.status.as_str(),
        "tx_hash": intent.tx_hash,
        "created_at_ms": intent.created_at_ms,
        "updated_at_ms": intent.updated_at_ms,
        "transitions": intent.transitions,
    })))
}

pub fn shared_store() -> Arc<Mutex<LocalTestIntentStore>> {
    Arc::new(Mutex::new(LocalTestIntentStore::new()))
}

// ──────────────────────────────────────────────────────────────────────
// Tests
// ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // --- LocalTestFixturesConfig guards ---------------------------------

    #[test]
    fn local_test_fixtures_disabled_by_default() {
        let cfg = LocalTestFixturesConfig::default();
        assert!(!cfg.is_enabled());
    }

    #[test]
    fn local_test_fixtures_disabled_constructor_is_disabled() {
        let cfg = LocalTestFixturesConfig::disabled();
        assert!(!cfg.is_enabled());
    }

    #[test]
    fn local_test_fixtures_refuses_mainnet_chain_id() {
        let cfg = LocalTestFixturesConfig::enabled_for_chain_id(MAINNET_CHAIN_ID);
        assert!(
            !cfg.is_enabled(),
            "enabled_for_chain_id MUST refuse mainnet (8453)"
        );
    }

    #[test]
    fn local_test_fixtures_enabled_for_sepolia() {
        let cfg = LocalTestFixturesConfig::enabled_for_chain_id(84532);
        assert!(cfg.is_enabled());
    }

    #[test]
    fn local_test_fixtures_enabled_for_anvil() {
        let cfg = LocalTestFixturesConfig::enabled_for_chain_id(31337);
        assert!(cfg.is_enabled());
    }

    #[test]
    fn assert_enabled_returns_err_when_disabled_on_sepolia() {
        let cfg = LocalTestFixturesConfig::disabled();
        assert!(cfg.assert_enabled(84532).is_err());
    }

    #[test]
    fn assert_enabled_returns_err_on_mainnet_even_when_flag_is_true() {
        // Defence-in-depth: even if a caller manages to hand-construct
        // an enabled config and then runs on mainnet, the assert MUST
        // reject. We simulate the impossible-via-constructor state by
        // mutating via the factory path; the result is unchanged.
        let cfg = LocalTestFixturesConfig::enabled_for_chain_id(84532);
        // Run-time switch to mainnet chain id MUST be refused.
        assert!(cfg.assert_enabled(MAINNET_CHAIN_ID).is_err());
    }

    #[test]
    fn assert_enabled_returns_ok_when_enabled_on_sepolia() {
        let cfg = LocalTestFixturesConfig::enabled_for_chain_id(84532);
        assert!(cfg.assert_enabled(84532).is_ok());
    }

    // --- Status machine -------------------------------------------------

    #[test]
    fn status_parse_roundtrip() {
        for s in [
            LocalTestIntentStatus::Created,
            LocalTestIntentStatus::Pending,
            LocalTestIntentStatus::Confirmed,
            LocalTestIntentStatus::Failed,
            LocalTestIntentStatus::Reverted,
            LocalTestIntentStatus::Stuck,
        ] {
            assert_eq!(LocalTestIntentStatus::parse(s.as_str()).unwrap(), s);
        }
    }

    #[test]
    fn status_parse_rejects_unknown() {
        assert!(LocalTestIntentStatus::parse("simulated").is_err());
        assert!(LocalTestIntentStatus::parse("broadcast_confirmed").is_err());
    }

    #[test]
    fn status_terminal_set_is_correct() {
        assert!(LocalTestIntentStatus::Confirmed.is_terminal());
        assert!(LocalTestIntentStatus::Failed.is_terminal());
        assert!(LocalTestIntentStatus::Reverted.is_terminal());
        assert!(!LocalTestIntentStatus::Created.is_terminal());
        assert!(!LocalTestIntentStatus::Pending.is_terminal());
        assert!(!LocalTestIntentStatus::Stuck.is_terminal());
    }

    #[test]
    fn allowed_transitions_match_spec() {
        use LocalTestIntentStatus::*;
        assert!(Created.can_transition_to(Pending));
        assert!(!Created.can_transition_to(Confirmed));
        assert!(Pending.can_transition_to(Confirmed));
        assert!(Pending.can_transition_to(Failed));
        assert!(Pending.can_transition_to(Reverted));
        assert!(Pending.can_transition_to(Stuck));
        assert!(!Pending.can_transition_to(Created));
        assert!(Stuck.can_transition_to(Pending));
        assert!(Stuck.can_transition_to(Failed));
        assert!(!Stuck.can_transition_to(Confirmed));
        for terminal in [Confirmed, Failed, Reverted] {
            for to in [Created, Pending, Confirmed, Failed, Reverted, Stuck] {
                assert!(
                    !terminal.can_transition_to(to),
                    "terminal {} must reject {}",
                    terminal.as_str(),
                    to.as_str()
                );
            }
        }
    }

    // --- Store behaviour ------------------------------------------------

    #[test]
    fn store_create_returns_intent_with_request_id() {
        let mut store = LocalTestIntentStore::new();
        let intent = store.create(
            DEFAULT_TEST_ACCOUNT.to_string(),
            "option_orderbook_fill".to_string(),
        );
        assert!(intent.request_id.starts_with("test-"));
        assert_eq!(intent.status, LocalTestIntentStatus::Created);
        assert!(intent.synthetic);
        assert_eq!(store.len(), 1);
    }

    #[test]
    fn synthetic_tx_hash_is_clearly_marked() {
        let intent_id = Uuid::new_v4();
        let hash = synthetic_tx_hash(&intent_id);
        assert!(
            hash.starts_with("0xdeadbee5"),
            "synthetic hash must start with the deadbee5 marker"
        );
        assert_eq!(hash.len(), 66);
    }

    #[test]
    fn store_transition_full_cycle_created_to_confirmed() {
        let mut store = LocalTestIntentStore::new();
        let intent = store.create(
            "0x".to_string() + &"a".repeat(40),
            "option_orderbook_fill".to_string(),
        );
        store
            .transition(&intent.intent_id, LocalTestIntentStatus::Pending)
            .expect("created -> pending");
        let confirmed = store
            .transition(&intent.intent_id, LocalTestIntentStatus::Confirmed)
            .expect("pending -> confirmed");
        assert_eq!(confirmed.status, LocalTestIntentStatus::Confirmed);
        assert_eq!(confirmed.transitions.len(), 2);
    }

    #[test]
    fn store_transition_pending_to_failed() {
        let mut store = LocalTestIntentStore::new();
        let intent = store.create(
            "0x".to_string() + &"b".repeat(40),
            "option_rfq_fill".to_string(),
        );
        store
            .transition(&intent.intent_id, LocalTestIntentStatus::Pending)
            .unwrap();
        let failed = store
            .transition(&intent.intent_id, LocalTestIntentStatus::Failed)
            .unwrap();
        assert_eq!(failed.status, LocalTestIntentStatus::Failed);
    }

    #[test]
    fn store_transition_pending_to_reverted() {
        let mut store = LocalTestIntentStore::new();
        let intent = store.create(
            "0x".to_string() + &"c".repeat(40),
            "option_orderbook_fill".to_string(),
        );
        store
            .transition(&intent.intent_id, LocalTestIntentStatus::Pending)
            .unwrap();
        let r = store
            .transition(&intent.intent_id, LocalTestIntentStatus::Reverted)
            .unwrap();
        assert_eq!(r.status, LocalTestIntentStatus::Reverted);
    }

    #[test]
    fn store_transition_stuck_recovers_to_pending() {
        let mut store = LocalTestIntentStore::new();
        let intent = store.create(
            "0x".to_string() + &"d".repeat(40),
            "option_orderbook_fill".to_string(),
        );
        store
            .transition(&intent.intent_id, LocalTestIntentStatus::Pending)
            .unwrap();
        store
            .transition(&intent.intent_id, LocalTestIntentStatus::Stuck)
            .unwrap();
        let recovered = store
            .transition(&intent.intent_id, LocalTestIntentStatus::Pending)
            .unwrap();
        assert_eq!(recovered.status, LocalTestIntentStatus::Pending);
        assert_eq!(recovered.transitions.len(), 3);
    }

    #[test]
    fn store_rejects_invalid_transition_created_to_confirmed() {
        let mut store = LocalTestIntentStore::new();
        let intent = store.create(
            DEFAULT_TEST_ACCOUNT.to_string(),
            "option_orderbook_fill".to_string(),
        );
        let err = store
            .transition(&intent.intent_id, LocalTestIntentStatus::Confirmed)
            .unwrap_err();
        assert!(format!("{err}").contains("invalid"));
    }

    #[test]
    fn store_rejects_transition_from_terminal_state() {
        let mut store = LocalTestIntentStore::new();
        let intent = store.create(
            DEFAULT_TEST_ACCOUNT.to_string(),
            "option_orderbook_fill".to_string(),
        );
        store
            .transition(&intent.intent_id, LocalTestIntentStatus::Pending)
            .unwrap();
        store
            .transition(&intent.intent_id, LocalTestIntentStatus::Confirmed)
            .unwrap();
        let err = store
            .transition(&intent.intent_id, LocalTestIntentStatus::Pending)
            .unwrap_err();
        assert!(format!("{err}").contains("invalid"));
    }

    #[test]
    fn store_unknown_intent_returns_persistence_error() {
        let mut store = LocalTestIntentStore::new();
        let err = store
            .transition(&Uuid::new_v4(), LocalTestIntentStatus::Pending)
            .unwrap_err();
        match err {
            BackendError::Persistence(msg) => assert!(msg.contains("not found")),
            other => panic!("expected Persistence variant, got {other:?}"),
        }
    }

    // --- Account validation in handler helper ---------------------------

    #[test]
    fn map_account_accepts_valid_hex() {
        let s = map_account(Some(DEFAULT_TEST_ACCOUNT.to_string())).unwrap();
        assert_eq!(s, DEFAULT_TEST_ACCOUNT);
    }

    #[test]
    fn map_account_defaults_to_anvil_zero() {
        let s = map_account(None).unwrap();
        assert_eq!(s, DEFAULT_TEST_ACCOUNT);
    }

    #[test]
    fn map_account_rejects_garbage() {
        assert!(map_account(Some("0xZZZ".to_string())).is_err());
        assert!(map_account(Some("not-an-address".to_string())).is_err());
        assert!(map_account(Some("0x".to_string() + &"a".repeat(39))).is_err());
    }

    #[test]
    fn map_source_type_defaults_and_validates() {
        assert_eq!(map_source_type(None).unwrap(), "option_orderbook_fill");
        assert_eq!(
            map_source_type(Some("option_rfq_fill".to_string())).unwrap(),
            "option_rfq_fill"
        );
        assert!(map_source_type(Some("perp".to_string())).is_err());
        assert!(map_source_type(Some("execution".to_string())).is_err());
    }

    // --- Sanity checks: no secrets, no signer, no broadcast -------------

    #[test]
    fn synthetic_tx_hash_does_not_match_a_real_tx_hash_pattern() {
        // A real Ethereum tx hash is a 32-byte keccak digest with full
        // entropy. Our synthetic hash always carries the 0xdeadbee5
        // prefix and 24 trailing bytes that include 12 zero bytes —
        // both are easy to assert against in test code so a fixture
        // can never be confused with a real broadcast result.
        let h = synthetic_tx_hash(&Uuid::nil());
        assert!(h.starts_with("0xdeadbee5"));
        // 12 zero bytes after the 4-byte marker = 24 hex zeros.
        assert!(h[10..34].chars().all(|c| c == '0'));
    }
}
