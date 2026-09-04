//! SUBACCOUNTS-CORE-BACKEND-V1
//!
//! Real Derive-like subaccounts for DeOpt V2. A subaccount is an
//! internal trading identity that belongs to a connected wallet. Every
//! wallet owns one or more subaccounts; the default `Account 1` is
//! lazily created on the first authenticated interaction.
//!
//! This crate landed the identity model, the write-auth actions
//! (`SUBACCOUNT_CREATE` / `SUBACCOUNT_RENAME`), and their HTTP
//! surface. Broad migration of Options / RFQ / TWAP / Perps / fees /
//! `used_nonces` / `write_auth_challenges` to include `subaccount_id`
//! is deferred to the follow-up milestones. Trading routes still
//! resolve orders against the wallet address only.

use crate::error::{BackendError, Result};
use crate::types::{now_ms, AccountId, TimestampMs};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Mutex;

/// Default id assigned to the auto-created subaccount on lazy
/// initialization. `0` is reserved for future system use.
pub const DEFAULT_SUBACCOUNT_ID: u32 = 1;

/// Maximum stored `name` length (in unicode chars). Matches the SQL
/// CHECK on `migrations/0038_subaccounts.sql`.
pub const MAX_SUBACCOUNT_NAME_LEN: usize = 64;

/// Minimum stored `name` length after trimming. Matches the SQL CHECK.
pub const MIN_SUBACCOUNT_NAME_LEN: usize = 1;

/// A single subaccount record. `name` is `None` when the user hasn't
/// set a custom label; the frontend displays `Account {subaccount_id}`
/// in that case.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Subaccount {
    pub owner_address: AccountId,
    pub subaccount_id: u32,
    pub name: Option<String>,
    pub created_at_ms: TimestampMs,
    pub updated_at_ms: TimestampMs,
    pub archived_at_ms: Option<TimestampMs>,
}

impl Subaccount {
    /// Deterministic display label used by both the backend response
    /// DTO and the frontend fallback. Never leaks the raw address.
    pub fn display_name(&self) -> String {
        match &self.name {
            Some(name) => name.clone(),
            None => format!("Account {}", self.subaccount_id),
        }
    }
}

/// Errors specific to subaccount workflows. Distinct from
/// `BackendError` so callers can map to precise HTTP responses.
#[derive(Debug, Clone, thiserror::Error)]
pub enum SubaccountError {
    #[error("subaccount name must not be empty")]
    NameEmpty,
    #[error("subaccount name too long (max {MAX_SUBACCOUNT_NAME_LEN} characters)")]
    NameTooLong,
    #[error("subaccount name contains a disallowed control character")]
    NameHasControlChar,
    #[error("subaccount not found")]
    NotFound,
    #[error("owner address is malformed")]
    OwnerAddressMalformed,
    #[error("subaccount persistence failure: {0}")]
    Persistence(String),
}

impl From<SubaccountError> for BackendError {
    fn from(err: SubaccountError) -> Self {
        match err {
            SubaccountError::NameEmpty
            | SubaccountError::NameTooLong
            | SubaccountError::NameHasControlChar
            | SubaccountError::OwnerAddressMalformed => {
                BackendError::InvalidSubaccountRequest(err.to_string())
            }
            SubaccountError::NotFound => BackendError::SubaccountNotFound,
            SubaccountError::Persistence(msg) => BackendError::Persistence(msg),
        }
    }
}

/// Trim, validate, and normalize a user-supplied subaccount name.
///
/// Rules:
/// * leading/trailing whitespace is trimmed;
/// * empty (after trim) is rejected;
/// * more than `MAX_SUBACCOUNT_NAME_LEN` chars is rejected;
/// * ASCII control characters are rejected (defence-in-depth against
///   log-injection or UI-glitch labels).
pub fn normalize_subaccount_name(raw: &str) -> std::result::Result<String, SubaccountError> {
    let trimmed = raw.trim();
    if trimmed.chars().count() < MIN_SUBACCOUNT_NAME_LEN {
        return Err(SubaccountError::NameEmpty);
    }
    if trimmed.chars().count() > MAX_SUBACCOUNT_NAME_LEN {
        return Err(SubaccountError::NameTooLong);
    }
    if trimmed.chars().any(|c| c.is_control()) {
        return Err(SubaccountError::NameHasControlChar);
    }
    Ok(trimmed.to_string())
}

/// Normalize an owner address to a lower-case, 0x-prefixed 42-char
/// hex string. Callers that already hold an `AccountId` from a parsed
/// route path can use the raw address; the store is responsible for
/// case-insensitive matching.
pub fn normalize_owner_address(address: &str) -> std::result::Result<String, SubaccountError> {
    let trimmed = address.trim();
    let hex = trimmed
        .strip_prefix("0x")
        .ok_or(SubaccountError::OwnerAddressMalformed)?;
    if hex.len() != 40 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(SubaccountError::OwnerAddressMalformed);
    }
    Ok(format!("0x{}", hex.to_ascii_lowercase()))
}

/// Persistence surface for subaccounts. Production wires this via
/// `PgRepository`; unit tests use `InMemorySubaccountStore` below.
#[async_trait]
pub trait SubaccountStore: Send + Sync {
    async fn list_by_owner(&self, owner: &AccountId) -> Result<Vec<Subaccount>>;

    /// Fetch a single row by composite key. Returns `Ok(None)` if the
    /// row does not exist; `Ok(Some(_))` if it does. Never lazily
    /// creates — callers wanting lazy-create semantics use the
    /// service-level `ensure_default_subaccount`.
    async fn get(&self, owner: &AccountId, subaccount_id: u32) -> Result<Option<Subaccount>>;

    /// Insert a new row. `subaccount_id` is chosen by the caller (the
    /// service allocates it as `MAX(subaccount_id)+1`, defaulting to
    /// 1). Fails if the composite key is already present.
    async fn insert(&self, record: Subaccount) -> Result<()>;

    /// Return the current maximum `subaccount_id` for `owner`, or
    /// `None` if no rows exist. Used by the allocator to hand out the
    /// next positive integer.
    async fn max_id_for_owner(&self, owner: &AccountId) -> Result<Option<u32>>;

    /// Set the `name` column. `updated_at_ms` is set by the caller.
    /// Returns `NotFound` if the composite key is unknown.
    async fn rename(
        &self,
        owner: &AccountId,
        subaccount_id: u32,
        new_name: Option<String>,
        updated_at_ms: TimestampMs,
    ) -> Result<()>;
}

// ===========================================================================
// Service layer
// ===========================================================================

/// Guarantee that `owner` has at least one subaccount. If none exist,
/// creates `Account 1` (no explicit name — the display label is
/// derived from `subaccount_id`). Idempotent.
pub async fn ensure_default_subaccount(
    store: &dyn SubaccountStore,
    owner: &AccountId,
) -> Result<Subaccount> {
    if let Some(existing) = store.get(owner, DEFAULT_SUBACCOUNT_ID).await? {
        return Ok(existing);
    }
    // Race-safe path: another writer might have inserted after our
    // get(). Try insert, treat any composite-key collision as success
    // and re-fetch the winner.
    let now = now_ms();
    let record = Subaccount {
        owner_address: owner.clone(),
        subaccount_id: DEFAULT_SUBACCOUNT_ID,
        name: None,
        created_at_ms: now,
        updated_at_ms: now,
        archived_at_ms: None,
    };
    match store.insert(record.clone()).await {
        Ok(()) => Ok(record),
        Err(BackendError::Persistence(_)) => match store.get(owner, DEFAULT_SUBACCOUNT_ID).await? {
            Some(row) => Ok(row),
            None => Err(BackendError::Persistence(
                "subaccount insert conflict but no row visible".to_string(),
            )),
        },
        Err(other) => Err(other),
    }
}

/// List all subaccounts for `owner`, ensuring `Account 1` exists
/// first. This is the surface `GET /accounts/:address/subaccounts`
/// binds to. Never returns an empty vector for a well-formed owner.
pub async fn list_subaccounts(
    store: &dyn SubaccountStore,
    owner: &AccountId,
) -> Result<Vec<Subaccount>> {
    let _ = ensure_default_subaccount(store, owner).await?;
    let mut rows = store.list_by_owner(owner).await?;
    rows.sort_by_key(|r| r.subaccount_id);
    Ok(rows)
}

/// Fetch a single subaccount by composite key. Does NOT lazily create.
/// Returns `SubaccountError::NotFound` if missing.
pub async fn get_subaccount(
    store: &dyn SubaccountStore,
    owner: &AccountId,
    subaccount_id: u32,
) -> Result<Subaccount> {
    match store.get(owner, subaccount_id).await? {
        Some(row) => Ok(row),
        None => Err(SubaccountError::NotFound.into()),
    }
}

/// Allocate and insert the next positive subaccount for `owner`. The
/// allocation is `MAX(subaccount_id) + 1`, always ≥ 2 (because
/// `ensure_default_subaccount` runs first). Server assigns the id;
/// clients cannot pick it.
pub async fn create_subaccount(
    store: &dyn SubaccountStore,
    owner: &AccountId,
    name: Option<String>,
) -> Result<Subaccount> {
    // Ensure Account 1 exists so the newly-created row is never id 1
    // (avoids a race where two concurrent creators fight for id 1).
    let _ = ensure_default_subaccount(store, owner).await?;
    let normalized_name = match name {
        Some(raw) => Some(normalize_subaccount_name(&raw)?),
        None => None,
    };
    let next_id = store
        .max_id_for_owner(owner)
        .await?
        .map(|id| id + 1)
        .unwrap_or(DEFAULT_SUBACCOUNT_ID + 1);
    debug_assert!(next_id >= 1);
    let now = now_ms();
    let record = Subaccount {
        owner_address: owner.clone(),
        subaccount_id: next_id,
        name: normalized_name,
        created_at_ms: now,
        updated_at_ms: now,
        archived_at_ms: None,
    };
    store.insert(record.clone()).await?;
    Ok(record)
}

/// Ownership predicate: returns `true` iff `subaccount_id` is
/// registered against `owner` in the persistence layer.
///
/// PERPS-CLOSED-TEST-HARDENING-V1 Part C #15 — the closed-test
/// signed-intent submit path calls this AFTER signature verification
/// and BEFORE market resolution. The signed `PerpOrderIntent`
/// EIP-712 struct binds `subaccountId`; the signature attests the
/// declared trader authorized that specific subaccount. But without
/// this ownership predicate a valid signature from wallet W could
/// reference a subaccount owned by wallet W' ≠ W. The predicate
/// closes that gap: only subaccounts owned by W are authorized.
///
/// Behaviour:
///
/// * `DEFAULT_SUBACCOUNT_ID` (`1`) is lazily ensured before the
///   lookup — the default account is created on the first
///   authenticated interaction, matching the module-level doc, and
///   the signed-intent submit is an authenticated interaction (the
///   signature has already been verified when this is called).
/// * Any other `subaccount_id` MUST have been previously allocated
///   via `create_subaccount` for `owner`. A cross-owner id returns
///   `false` (deterministic reject) rather than propagating an
///   error, so the caller can map the negative result to a specific
///   HTTP status without leaking whether the id exists under a
///   different owner.
/// * `subaccount_id == 0` is reserved for future system use (see
///   `DEFAULT_SUBACCOUNT_ID` doc). It is never a legitimate
///   subaccount and this function returns `false` for it without
///   touching persistence.
/// * Persistence errors (connection loss, poisoned in-memory mutex)
///   propagate as `Err(_)` — the caller MUST treat that as a hard
///   reject rather than a `false` denial.
pub async fn is_owned_by(
    store: &dyn SubaccountStore,
    owner: &AccountId,
    subaccount_id: u32,
) -> Result<bool> {
    if subaccount_id == 0 {
        // `0` is reserved for future system use; never a legitimate
        // per-wallet subaccount. Cheap short-circuit before touching
        // the store.
        return Ok(false);
    }
    if subaccount_id == DEFAULT_SUBACCOUNT_ID {
        // Lazy-create the default subaccount on the first
        // authenticated interaction. The signed-intent path has
        // already verified the caller's signature by the time this
        // runs so the "authenticated interaction" precondition is met.
        let _ = ensure_default_subaccount(store, owner).await?;
        return Ok(true);
    }
    Ok(store.get(owner, subaccount_id).await?.is_some())
}

/// Set the `name` of an existing subaccount. Empty / too-long names
/// are rejected. Unknown composite keys return `NotFound`.
pub async fn rename_subaccount(
    store: &dyn SubaccountStore,
    owner: &AccountId,
    subaccount_id: u32,
    new_name: String,
) -> Result<Subaccount> {
    let normalized = normalize_subaccount_name(&new_name)?;
    // Verify the row exists before writing so the error surface is
    // deterministic (rename of a missing subaccount returns 404 rather
    // than a silent no-op).
    let _existing = get_subaccount(store, owner, subaccount_id).await?;
    let updated_at_ms = now_ms();
    store
        .rename(
            owner,
            subaccount_id,
            Some(normalized.clone()),
            updated_at_ms,
        )
        .await?;
    get_subaccount(store, owner, subaccount_id).await
}

// ===========================================================================
// In-memory store — used by tests, and by the default in-process
// AppState when no PgRepository is configured. NOT production-safe.
// ===========================================================================

#[derive(Default)]
pub struct InMemorySubaccountStore {
    // Keyed by (LOWER(owner), subaccount_id). Owner is stored as-is
    // inside the record; the map key is normalized-lowercase for
    // case-insensitive matching.
    inner: Mutex<HashMap<(String, u32), Subaccount>>,
}

impl InMemorySubaccountStore {
    pub fn new() -> Self {
        Self::default()
    }
}

fn owner_key(owner: &AccountId) -> String {
    owner.0.to_ascii_lowercase()
}

#[async_trait]
impl SubaccountStore for InMemorySubaccountStore {
    async fn list_by_owner(&self, owner: &AccountId) -> Result<Vec<Subaccount>> {
        let guard = self
            .inner
            .lock()
            .map_err(|_| BackendError::Persistence("InMemorySubaccountStore poisoned".into()))?;
        let owner_lc = owner_key(owner);
        Ok(guard
            .iter()
            .filter_map(|((owner_lower, _), row)| {
                if owner_lower == &owner_lc {
                    Some(row.clone())
                } else {
                    None
                }
            })
            .collect())
    }

    async fn get(&self, owner: &AccountId, subaccount_id: u32) -> Result<Option<Subaccount>> {
        let guard = self
            .inner
            .lock()
            .map_err(|_| BackendError::Persistence("InMemorySubaccountStore poisoned".into()))?;
        Ok(guard.get(&(owner_key(owner), subaccount_id)).cloned())
    }

    async fn insert(&self, record: Subaccount) -> Result<()> {
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| BackendError::Persistence("InMemorySubaccountStore poisoned".into()))?;
        let key = (owner_key(&record.owner_address), record.subaccount_id);
        if guard.contains_key(&key) {
            return Err(BackendError::Persistence(format!(
                "subaccount {} already exists for owner",
                record.subaccount_id
            )));
        }
        guard.insert(key, record);
        Ok(())
    }

    async fn max_id_for_owner(&self, owner: &AccountId) -> Result<Option<u32>> {
        let guard = self
            .inner
            .lock()
            .map_err(|_| BackendError::Persistence("InMemorySubaccountStore poisoned".into()))?;
        let owner_lc = owner_key(owner);
        Ok(guard
            .iter()
            .filter(|((owner_lower, _), _)| owner_lower == &owner_lc)
            .map(|((_, id), _)| *id)
            .max())
    }

    async fn rename(
        &self,
        owner: &AccountId,
        subaccount_id: u32,
        new_name: Option<String>,
        updated_at_ms: TimestampMs,
    ) -> Result<()> {
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| BackendError::Persistence("InMemorySubaccountStore poisoned".into()))?;
        let key = (owner_key(owner), subaccount_id);
        match guard.get_mut(&key) {
            Some(row) => {
                row.name = new_name;
                row.updated_at_ms = updated_at_ms;
                Ok(())
            }
            None => Err(SubaccountError::NotFound.into()),
        }
    }
}

// ===========================================================================
// Unit tests — pure service logic against the in-memory store.
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn owner_a() -> AccountId {
        AccountId::new("0xAAAA000000000000000000000000000000000001")
    }
    fn owner_b() -> AccountId {
        AccountId::new("0xBBBB000000000000000000000000000000000002")
    }

    #[tokio::test]
    async fn list_ensures_default_for_new_owner() {
        let store = InMemorySubaccountStore::new();
        let rows = list_subaccounts(&store, &owner_a()).await.expect("list");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].subaccount_id, DEFAULT_SUBACCOUNT_ID);
        assert_eq!(rows[0].name, None);
        assert_eq!(rows[0].display_name(), "Account 1");
        assert!(rows[0].archived_at_ms.is_none());
    }

    #[tokio::test]
    async fn ensure_default_is_idempotent() {
        let store = InMemorySubaccountStore::new();
        let a = ensure_default_subaccount(&store, &owner_a())
            .await
            .expect("first ensure");
        let b = ensure_default_subaccount(&store, &owner_a())
            .await
            .expect("second ensure");
        assert_eq!(a, b);
        assert_eq!(store.list_by_owner(&owner_a()).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn create_allocates_next_id_starting_at_two() {
        let store = InMemorySubaccountStore::new();
        // Bootstrap default via list.
        let _ = list_subaccounts(&store, &owner_a()).await.unwrap();

        let second = create_subaccount(&store, &owner_a(), Some("MM Desk".to_string()))
            .await
            .expect("create second");
        assert_eq!(second.subaccount_id, 2);
        assert_eq!(second.name.as_deref(), Some("MM Desk"));
        assert_eq!(second.display_name(), "MM Desk");

        let third = create_subaccount(&store, &owner_a(), None)
            .await
            .expect("create third");
        assert_eq!(third.subaccount_id, 3);
        assert_eq!(third.name, None);
        assert_eq!(third.display_name(), "Account 3");
    }

    #[tokio::test]
    async fn get_missing_returns_not_found() {
        let store = InMemorySubaccountStore::new();
        let _ = list_subaccounts(&store, &owner_a()).await.unwrap();
        let err = get_subaccount(&store, &owner_a(), 42).await.unwrap_err();
        assert!(
            matches!(err, BackendError::SubaccountNotFound),
            "got: {err:?}"
        );
    }

    #[tokio::test]
    async fn rename_valid_name_wins() {
        let store = InMemorySubaccountStore::new();
        let _ = list_subaccounts(&store, &owner_a()).await.unwrap();
        let renamed = rename_subaccount(&store, &owner_a(), 1, "Trading".to_string())
            .await
            .expect("rename");
        assert_eq!(renamed.name.as_deref(), Some("Trading"));
        assert!(renamed.updated_at_ms >= renamed.created_at_ms);
    }

    #[tokio::test]
    async fn rename_trims_whitespace() {
        let store = InMemorySubaccountStore::new();
        let _ = list_subaccounts(&store, &owner_a()).await.unwrap();
        let renamed = rename_subaccount(&store, &owner_a(), 1, "  Alpha  ".to_string())
            .await
            .expect("rename");
        assert_eq!(renamed.name.as_deref(), Some("Alpha"));
    }

    #[tokio::test]
    async fn rename_empty_rejected() {
        let store = InMemorySubaccountStore::new();
        let _ = list_subaccounts(&store, &owner_a()).await.unwrap();
        let err = rename_subaccount(&store, &owner_a(), 1, "   ".to_string())
            .await
            .unwrap_err();
        assert!(
            matches!(err, BackendError::InvalidSubaccountRequest(_)),
            "got: {err:?}"
        );
    }

    #[tokio::test]
    async fn rename_too_long_rejected() {
        let store = InMemorySubaccountStore::new();
        let _ = list_subaccounts(&store, &owner_a()).await.unwrap();
        let long = "x".repeat(MAX_SUBACCOUNT_NAME_LEN + 1);
        let err = rename_subaccount(&store, &owner_a(), 1, long)
            .await
            .unwrap_err();
        assert!(
            matches!(err, BackendError::InvalidSubaccountRequest(_)),
            "got: {err:?}"
        );
    }

    #[tokio::test]
    async fn rename_control_char_rejected() {
        let store = InMemorySubaccountStore::new();
        let _ = list_subaccounts(&store, &owner_a()).await.unwrap();
        let err = rename_subaccount(&store, &owner_a(), 1, "hi\nthere".to_string())
            .await
            .unwrap_err();
        assert!(
            matches!(err, BackendError::InvalidSubaccountRequest(_)),
            "got: {err:?}"
        );
    }

    #[tokio::test]
    async fn rename_missing_returns_not_found() {
        let store = InMemorySubaccountStore::new();
        let _ = list_subaccounts(&store, &owner_a()).await.unwrap();
        let err = rename_subaccount(&store, &owner_a(), 42, "X".to_string())
            .await
            .unwrap_err();
        assert!(
            matches!(err, BackendError::SubaccountNotFound),
            "got: {err:?}"
        );
    }

    #[tokio::test]
    async fn create_name_too_long_rejected() {
        let store = InMemorySubaccountStore::new();
        let _ = list_subaccounts(&store, &owner_a()).await.unwrap();
        let long = "x".repeat(MAX_SUBACCOUNT_NAME_LEN + 1);
        let err = create_subaccount(&store, &owner_a(), Some(long))
            .await
            .unwrap_err();
        assert!(
            matches!(err, BackendError::InvalidSubaccountRequest(_)),
            "got: {err:?}"
        );
    }

    #[tokio::test]
    async fn subaccounts_are_isolated_by_owner() {
        let store = InMemorySubaccountStore::new();
        let _ = list_subaccounts(&store, &owner_a()).await.unwrap();
        let _ = list_subaccounts(&store, &owner_b()).await.unwrap();
        create_subaccount(&store, &owner_a(), Some("A2".to_string()))
            .await
            .unwrap();
        create_subaccount(&store, &owner_a(), Some("A3".to_string()))
            .await
            .unwrap();
        // Owner B still has just the default; renaming Owner A's row
        // must not touch Owner B.
        let rows_a = list_subaccounts(&store, &owner_a()).await.unwrap();
        let rows_b = list_subaccounts(&store, &owner_b()).await.unwrap();
        assert_eq!(rows_a.len(), 3);
        assert_eq!(rows_b.len(), 1);
        // The default subaccount cannot be id 0.
        for row in &rows_a {
            assert!(row.subaccount_id >= DEFAULT_SUBACCOUNT_ID);
        }
    }

    #[tokio::test]
    async fn owner_lookup_is_case_insensitive() {
        let store = InMemorySubaccountStore::new();
        let mixed = AccountId::new("0xAaAaBbBbCcCcDdDdEeEeFfFf0000000000000001");
        let lower = AccountId::new("0xaaaabbbbccccddddeeeeffff0000000000000001");
        let _ = list_subaccounts(&store, &mixed).await.unwrap();
        let rows = list_subaccounts(&store, &lower).await.unwrap();
        assert_eq!(rows.len(), 1);
    }

    // -----------------------------------------------------------------
    // PERPS-CLOSED-TEST-HARDENING-V1 Part C #15 — is_owned_by unit tests
    // -----------------------------------------------------------------

    #[tokio::test]
    async fn is_owned_by_default_id_is_lazy_true_for_any_owner() {
        let store = InMemorySubaccountStore::new();
        // No subaccounts exist yet. `is_owned_by` for the default id
        // must lazy-create and return true.
        assert!(is_owned_by(&store, &owner_a(), DEFAULT_SUBACCOUNT_ID)
            .await
            .expect("is_owned_by default"));
        // Post-condition: the row was actually written.
        let rows = store.list_by_owner(&owner_a()).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].subaccount_id, DEFAULT_SUBACCOUNT_ID);
    }

    #[tokio::test]
    async fn is_owned_by_zero_id_is_always_false() {
        let store = InMemorySubaccountStore::new();
        assert!(!is_owned_by(&store, &owner_a(), 0)
            .await
            .expect("is_owned_by 0"));
        // No lazy-create side effect for id 0.
        assert!(store.list_by_owner(&owner_a()).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn is_owned_by_returns_true_for_owned_non_default_id() {
        let store = InMemorySubaccountStore::new();
        let _ = list_subaccounts(&store, &owner_a()).await.unwrap();
        let created = create_subaccount(&store, &owner_a(), Some("Desk".to_string()))
            .await
            .unwrap();
        assert_eq!(created.subaccount_id, 2);
        assert!(is_owned_by(&store, &owner_a(), 2).await.unwrap());
    }

    #[tokio::test]
    async fn is_owned_by_rejects_cross_owner_id() {
        let store = InMemorySubaccountStore::new();
        // Owner A gets subaccount 2; owner B does NOT.
        let _ = list_subaccounts(&store, &owner_a()).await.unwrap();
        let _ = create_subaccount(&store, &owner_a(), None).await.unwrap();
        // Owner B has only the default subaccount 1 (lazy) — id 2 is
        // not theirs.
        assert!(!is_owned_by(&store, &owner_b(), 2).await.unwrap());
        // And owner B's default is still theirs.
        assert!(is_owned_by(&store, &owner_b(), DEFAULT_SUBACCOUNT_ID)
            .await
            .unwrap());
    }

    #[tokio::test]
    async fn is_owned_by_rejects_never_allocated_non_default_id() {
        let store = InMemorySubaccountStore::new();
        let _ = list_subaccounts(&store, &owner_a()).await.unwrap();
        // Only id 1 exists; id 42 was never allocated.
        assert!(!is_owned_by(&store, &owner_a(), 42).await.unwrap());
    }

    #[test]
    fn normalize_owner_rejects_malformed() {
        assert!(normalize_owner_address("not-hex").is_err());
        assert!(normalize_owner_address("0x1234").is_err());
        assert!(normalize_owner_address("0xZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZ").is_err());
        let ok = normalize_owner_address("0xAaBbCcDdEeFf00112233445566778899aabbccdd")
            .expect("normalize");
        assert_eq!(ok, "0xaabbccddeeff00112233445566778899aabbccdd");
    }

    #[test]
    fn normalize_name_trims_and_bounds() {
        assert_eq!(normalize_subaccount_name(" Trading ").unwrap(), "Trading");
        assert!(normalize_subaccount_name("   ").is_err());
        assert!(normalize_subaccount_name(&"x".repeat(MAX_SUBACCOUNT_NAME_LEN + 1)).is_err());
        assert!(normalize_subaccount_name("bad\tname").is_err());
    }
}
