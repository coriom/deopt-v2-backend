//! `HybridV2ApiState` — shared state consumed by the read API router.
//!
//! `BACKEND-HYBRID-V2-POSTGRES-READ-STORE-2B-HANDLER-SWAP-V1` posture:
//!
//! * Handlers see an immutable per-deployment `DeploymentEntry` (metadata
//!   only) plus an `Arc<dyn HybridV2ReadStore>` that satisfies every
//!   data-bearing query. Handlers MUST NOT reach into `DeploymentEntry`
//!   for a runtime handle — they must go through `state.store()`.
//! * Production is wired via `HybridV2ApiState::with_postgres(pool, ...)`
//!   which constructs a `PostgresHybridV2ReadStore`. There is no
//!   production runtime-memory fallback.
//! * The legacy `HybridV2ApiState::new(vec![entry])` constructor remains
//!   for test/compatibility — it wraps the supplied runtime-carrying
//!   entries in a `RuntimeBackedHybridV2ReadStore`. Production code
//!   never selects this path.
//! * `HybridV2ApiState::empty()` binds an `EmptyReadStore` — canonical
//!   routes fail closed and `/deployments` returns `[]`.

use crate::hybrid_v2::manifest::ManifestParams;
use crate::hybrid_v2::read_store::{HybridV2ReadStore, PostgresHybridV2ReadStore, ReadStoreError};
use crate::hybrid_v2::runtime::IndexerRuntime;
use crate::hybrid_v2::runtime_backed_read_store::RuntimeBackedHybridV2ReadStore;
use sqlx::postgres::PgPool;
use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

/// Immutable per-deployment metadata surfaced to handlers.
///
/// Under the runtime-backed compatibility path the entry additionally
/// carries an `RwLock<IndexerRuntime>` in a crate-visible field
/// (`_runtime`) so the runtime-backed store can bind to it. Handlers
/// MUST NOT touch this field — they must go through `state.store()`.
pub struct DeploymentEntry {
    pub deployment_id: u64,
    pub manifest: ManifestParams,
    /// Test/compatibility backing runtime. Populated by
    /// `DeploymentEntry::new(runtime)` and consumed exclusively by
    /// `RuntimeBackedHybridV2ReadStore` plus the integration tests
    /// that mutate the runtime between assertions. `None` under the
    /// production PostgreSQL wiring. **Handlers MUST NOT read this
    /// field** — every canonical read must be routed through
    /// `state.store()`.
    pub runtime: Option<Arc<RwLock<IndexerRuntime>>>,
}

impl DeploymentEntry {
    /// Test/compatibility constructor. The supplied runtime is
    /// consumed and hidden behind `state.store()`. Handlers never
    /// see it directly.
    pub fn new(runtime: IndexerRuntime) -> Self {
        Self {
            deployment_id: runtime.deployment_id,
            manifest: runtime.manifest.clone(),
            runtime: Some(Arc::new(RwLock::new(runtime))),
        }
    }

    /// Production constructor — metadata only. The store passed to
    /// `HybridV2ApiState::with_store` / `with_postgres` supplies every
    /// data-bearing read; this entry only anchors the deployment id
    /// and immutable manifest.
    pub fn from_metadata(deployment_id: u64, manifest: ManifestParams) -> Self {
        Self {
            deployment_id,
            manifest,
            runtime: None,
        }
    }
}

/// Router state. Cloneable via `Arc` — every handler holds a shared reference.
#[derive(Clone)]
pub struct HybridV2ApiState {
    inner: Arc<HybridV2ApiStateInner>,
}

struct HybridV2ApiStateInner {
    deployments: BTreeMap<u64, Arc<DeploymentEntry>>,
    store: Arc<dyn HybridV2ReadStore>,
}

impl HybridV2ApiState {
    /// Test/compatibility constructor. Every supplied entry must have
    /// been built via `DeploymentEntry::new(runtime)` so the internal
    /// runtime-backed store can bind to its `IndexerRuntime`. Callers
    /// mixing runtime-carrying and metadata-only entries will get an
    /// adapter that treats metadata-only entries as if their store
    /// data were empty for that deployment id.
    ///
    /// Handlers still route every read through `state.store()`; the
    /// runtime is only reachable via the adapter.
    pub fn new(entries: Vec<Arc<DeploymentEntry>>) -> Self {
        let store = Arc::new(RuntimeBackedHybridV2ReadStore::from_entries(&entries))
            as Arc<dyn HybridV2ReadStore>;
        Self::from_parts(entries, store)
    }

    /// Empty state. `state.store()` returns an `EmptyReadStore` that
    /// resolves every canonical query as absent — handlers combine
    /// this with `resolve_deployment` (which itself 404s on an empty
    /// registry) so canonical routes fail closed under an unconfigured
    /// backend.
    pub fn empty() -> Self {
        let store = Arc::new(EmptyReadStore) as Arc<dyn HybridV2ReadStore>;
        Self::from_parts(Vec::new(), store)
    }

    /// Production PostgreSQL constructor. The supplied deployment
    /// entries MUST be metadata-only (`DeploymentEntry::from_metadata`);
    /// no runtime is created and no runtime fallback exists. When the
    /// pool loses connectivity the mapped `ReadStoreError::Backend`
    /// surfaces as a structured 500 — no memory fallback.
    pub fn with_postgres(pool: PgPool, entries: Vec<Arc<DeploymentEntry>>) -> Self {
        let store = Arc::new(PostgresHybridV2ReadStore::new(pool)) as Arc<dyn HybridV2ReadStore>;
        Self::from_parts(entries, store)
    }

    /// Test constructor for custom stores (e.g. `InMemoryHybridV2ReadStore`
    /// parity fixtures). Never selected by production wiring.
    pub fn with_store(
        store: Arc<dyn HybridV2ReadStore>,
        entries: Vec<Arc<DeploymentEntry>>,
    ) -> Self {
        Self::from_parts(entries, store)
    }

    fn from_parts(entries: Vec<Arc<DeploymentEntry>>, store: Arc<dyn HybridV2ReadStore>) -> Self {
        let mut map = BTreeMap::new();
        for entry in entries {
            map.insert(entry.deployment_id, entry);
        }
        Self {
            inner: Arc::new(HybridV2ApiStateInner {
                deployments: map,
                store,
            }),
        }
    }

    pub fn get(&self, deployment_id: u64) -> Option<Arc<DeploymentEntry>> {
        self.inner.deployments.get(&deployment_id).cloned()
    }

    pub fn list(&self) -> Vec<Arc<DeploymentEntry>> {
        self.inner.deployments.values().cloned().collect()
    }

    /// Return the sole configured deployment when exactly one exists.
    /// Every subaccount route accepts an implicit deployment when
    /// exactly one is configured; otherwise callers must pass an
    /// explicit `deployment_id` query.
    pub fn resolve_single(&self) -> Option<Arc<DeploymentEntry>> {
        if self.inner.deployments.len() == 1 {
            self.inner.deployments.values().next().cloned()
        } else {
            None
        }
    }

    /// Shared read store — every data-bearing handler goes through this.
    pub fn store(&self) -> Arc<dyn HybridV2ReadStore> {
        self.inner.store.clone()
    }
}

// -----------------------------------------------------------------
//                       EMPTY READ STORE
// -----------------------------------------------------------------

/// Fails-closed placeholder for the unconfigured `HybridV2ApiState`.
/// Every canonical read reports the deployment as absent — combined
/// with the empty deployment registry this produces structured 404
/// (`DEPLOYMENT_NOT_FOUND`) / 503 responses at the handler layer.
struct EmptyReadStore;

#[async_trait::async_trait]
impl HybridV2ReadStore for EmptyReadStore {
    async fn list_deployments(
        &self,
    ) -> Result<Vec<crate::hybrid_v2::read_store::DeploymentListRecord>, ReadStoreError> {
        Ok(Vec::new())
    }

    async fn get_deployment_status(
        &self,
        _deployment_id: u64,
    ) -> Result<Option<crate::hybrid_v2::read_store::DeploymentStatusRecord>, ReadStoreError> {
        Ok(None)
    }

    async fn list_subaccounts_by_owner(
        &self,
        _deployment_id: u64,
        _owner: &str,
    ) -> Result<Vec<crate::hybrid_v2::read_store::SubaccountRecord>, ReadStoreError> {
        Ok(Vec::new())
    }

    async fn get_subaccount_summary(
        &self,
        _deployment_id: u64,
        _subkey: &str,
    ) -> Result<Option<crate::hybrid_v2::read_store::SubaccountSummaryRecord>, ReadStoreError> {
        Ok(None)
    }

    async fn list_collateral(
        &self,
        _deployment_id: u64,
        _subkey: &str,
    ) -> Result<Vec<crate::hybrid_v2::read_store::CollateralRecord>, ReadStoreError> {
        Ok(Vec::new())
    }

    async fn list_reservations(
        &self,
        _deployment_id: u64,
        _subkey: &str,
    ) -> Result<Vec<crate::hybrid_v2::read_store::ReservationRecord>, ReadStoreError> {
        Ok(Vec::new())
    }

    async fn list_positions(
        &self,
        _deployment_id: u64,
        _subkey: &str,
        _page: &crate::hybrid_v2::read_store::PageAnchor,
    ) -> Result<
        crate::hybrid_v2::read_store::StorePage<crate::hybrid_v2::read_store::PositionRecord>,
        ReadStoreError,
    > {
        Ok(crate::hybrid_v2::read_store::StorePage {
            items: Vec::new(),
            next_anchor: None,
        })
    }

    async fn list_orders(
        &self,
        _deployment_id: u64,
        _subkey: &str,
        _page: &crate::hybrid_v2::read_store::PageAnchor,
    ) -> Result<
        crate::hybrid_v2::read_store::StorePage<crate::hybrid_v2::read_store::OrderLifecycleRecord>,
        ReadStoreError,
    > {
        Ok(crate::hybrid_v2::read_store::StorePage {
            items: Vec::new(),
            next_anchor: None,
        })
    }

    async fn get_order(
        &self,
        _deployment_id: u64,
        _order_hash: &str,
    ) -> Result<Option<crate::hybrid_v2::read_store::OrderLifecycleRecord>, ReadStoreError> {
        Ok(None)
    }

    async fn list_completed_executions(
        &self,
        _deployment_id: u64,
        _subkey: Option<&str>,
        _page: &crate::hybrid_v2::read_store::PageAnchor,
    ) -> Result<
        crate::hybrid_v2::read_store::StorePage<
            crate::hybrid_v2::read_store::MatchedExecutionRecord,
        >,
        ReadStoreError,
    > {
        Ok(crate::hybrid_v2::read_store::StorePage {
            items: Vec::new(),
            next_anchor: None,
        })
    }

    async fn list_fees(
        &self,
        _deployment_id: u64,
        _subkey: &str,
        _page: &crate::hybrid_v2::read_store::PageAnchor,
    ) -> Result<
        crate::hybrid_v2::read_store::StorePage<crate::hybrid_v2::read_store::FeeRebateRecord>,
        ReadStoreError,
    > {
        Ok(crate::hybrid_v2::read_store::StorePage {
            items: Vec::new(),
            next_anchor: None,
        })
    }

    async fn get_recovery(
        &self,
        _deployment_id: u64,
        _subkey: &str,
    ) -> Result<Option<crate::hybrid_v2::read_store::RecoveryRecord>, ReadStoreError> {
        Ok(None)
    }

    async fn query_history(
        &self,
        _deployment_id: u64,
        _scope: &crate::hybrid_v2::read_store::HistoryScope,
        _filter: &crate::api::hybrid_v2_read::history::HistoryFilter,
        _page: &crate::hybrid_v2::read_store::HistoryPageAnchor,
    ) -> Result<crate::hybrid_v2::read_store::HistoryPage, ReadStoreError> {
        Ok(crate::hybrid_v2::read_store::HistoryPage {
            items: Vec::new(),
            next_anchor: None,
        })
    }
}
