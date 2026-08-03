//! `BACKEND-HYBRID-V2-POSTGRES-READ-STORE-2B-HANDLER-SWAP-V1`
//!
//! **Test / compatibility only.** Not selectable by production wiring.
//!
//! Adapts one or more in-memory `IndexerRuntime`s to the
//! `HybridV2ReadStore` trait so the existing runtime-based tests
//! (`hybrid_v2_read_api_tests.rs`, `hybrid_v2_read_api_main_router_tests.rs`,
//! `hybrid_v2_read_api_property_tests.rs`) can keep constructing the
//! `HybridV2ApiState` via `HybridV2ApiState::new(vec![entry])` after the
//! stage-2B handler refactor.
//!
//! Behaviour is faithful to the pre-refactor runtime-driven read path:
//! - Non-history queries snapshot the projection state + cursor +
//!   readiness under a read lock and delegate to
//!   `InMemoryHybridV2ReadStore` (which is trait-conformant and
//!   parity-tested against `PostgresHybridV2ReadStore`).
//! - History queries acquire the read lock, walk canonical raw logs
//!   via `build_history`, and apply store-shaped keyset pagination.
//!
//! Every production selection MUST go through `PostgresHybridV2ReadStore`.
//! `HybridV2ApiState::with_postgres(pool, entries)` is the sole
//! production constructor — see `state.rs`.

use crate::api::hybrid_v2_read::history::{build_history, HistoryEvent, HistoryFilter};
use crate::hybrid_v2::read_store::{
    filter_stable_hash, CollateralRecord, DeploymentListRecord, DeploymentStatusRecord,
    FeeRebateRecord, HistoryConsistency, HistoryCursorKey, HistoryPage, HistoryPageAnchor,
    HistoryRecord, HistoryScope, HybridV2ReadStore, InMemoryHybridV2ReadStore,
    InMemoryStoreBuilder, MatchedExecutionRecord, OrderLifecycleRecord, PageAnchor, PositionRecord,
    ReadStoreError, RecoveryRecord, ReservationRecord, StorePage, SubaccountRecord,
    SubaccountSummaryRecord,
};
use crate::hybrid_v2::runtime::IndexerRuntime;
use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

/// Test/compatibility adapter. Never selected by production wiring.
pub struct RuntimeBackedHybridV2ReadStore {
    inner: Arc<RuntimeBackedInner>,
}

struct RuntimeBackedInner {
    deployments: BTreeMap<u64, Arc<RwLock<IndexerRuntime>>>,
}

impl RuntimeBackedHybridV2ReadStore {
    /// Build an adapter over the runtime handles carried by the
    /// supplied `DeploymentEntry`s. Metadata-only entries (no
    /// runtime) are silently skipped — every non-history query for
    /// their deployment id will resolve as absent, which matches the
    /// production behaviour when a deployment is registered but the
    /// backing store has no persisted rows yet.
    pub fn from_entries(
        entries: &[Arc<crate::api::hybrid_v2_read::state::DeploymentEntry>],
    ) -> Self {
        let mut map = BTreeMap::new();
        for entry in entries {
            if let Some(rt) = entry.runtime.as_ref() {
                map.insert(entry.deployment_id, rt.clone());
            }
        }
        Self {
            inner: Arc::new(RuntimeBackedInner { deployments: map }),
        }
    }

    fn runtime(&self, deployment_id: u64) -> Option<Arc<RwLock<IndexerRuntime>>> {
        self.inner.deployments.get(&deployment_id).cloned()
    }

    fn snapshot(
        &self,
        deployment_id: u64,
    ) -> Result<Option<InMemoryHybridV2ReadStore>, ReadStoreError> {
        let Some(rt_lock) = self.runtime(deployment_id) else {
            return Ok(None);
        };
        let rt = rt_lock.read().map_err(|_| ReadStoreError::Backend {
            detail: "runtime lock poisoned".into(),
        })?;
        let readiness = rt.readiness();
        let store = InMemoryStoreBuilder::new(deployment_id)
            .with_chain_id(rt.manifest.chain_id)
            .with_manifest(&rt.manifest.manifest_hash, &rt.manifest.manifest_address)
            .with_state(rt.projection().clone())
            .with_cursor(rt.cursor().clone())
            .with_readiness(
                readiness.ready,
                readiness.reason.as_ref().map(|r| format!("{:?}", r)),
                None,
            )
            .with_finalized_head(rt.metrics().finalized_block)
            .build();
        Ok(Some(store))
    }
}

/// Convert a `HistoryEvent` (assembled by the runtime-side
/// `build_history`) into the store-domain `HistoryRecord`. The two
/// types are structurally identical — this is a plain field-by-field
/// mapping preserved so that runtime and Postgres paths return the
/// same wire JSON.
fn history_event_to_record(ev: HistoryEvent) -> HistoryRecord {
    HistoryRecord {
        event_id: ev.event_id,
        deployment_id: ev.deployment_id,
        chain_id: ev.chain_id,
        block_number: ev.block_number,
        block_hash: ev.block_hash,
        tx_hash: ev.tx_hash,
        tx_index: ev.tx_index,
        log_index: ev.log_index,
        timestamp_ms: ev.timestamp_ms,
        finalized: ev.finalized,
        direction: ev.direction,
        owner: ev.owner,
        subaccount_id: ev.subaccount_id,
        subkey: ev.subkey,
        related_order_hash: ev.related_order_hash,
        related_execution_id: ev.related_execution_id,
        payload: ev.payload,
    }
}

#[async_trait::async_trait]
impl HybridV2ReadStore for RuntimeBackedHybridV2ReadStore {
    async fn list_deployments(&self) -> Result<Vec<DeploymentListRecord>, ReadStoreError> {
        let mut out = Vec::new();
        for (id, rt_lock) in self.inner.deployments.iter() {
            let rt = rt_lock.read().map_err(|_| ReadStoreError::Backend {
                detail: "runtime lock poisoned".into(),
            })?;
            out.push(DeploymentListRecord {
                deployment_id: *id,
                chain_id: rt.manifest.chain_id,
                manifest_hash: rt.manifest.manifest_hash.clone(),
                manifest_address: rt.manifest.manifest_address.clone(),
                deployment_version: rt.manifest.deployment_version as u32,
                activation_status: format!("{:?}", rt.manifest.activation_status).to_uppercase(),
                max_collateral_tokens: rt.manifest.max_collateral_tokens as u32,
                max_active_series: rt.manifest.max_active_series,
            });
        }
        Ok(out)
    }

    async fn get_deployment_status(
        &self,
        deployment_id: u64,
    ) -> Result<Option<DeploymentStatusRecord>, ReadStoreError> {
        match self.snapshot(deployment_id)? {
            Some(s) => s.get_deployment_status(deployment_id).await,
            None => Ok(None),
        }
    }

    async fn list_subaccounts_by_owner(
        &self,
        deployment_id: u64,
        owner: &str,
    ) -> Result<Vec<SubaccountRecord>, ReadStoreError> {
        match self.snapshot(deployment_id)? {
            Some(s) => s.list_subaccounts_by_owner(deployment_id, owner).await,
            None => Ok(Vec::new()),
        }
    }

    async fn get_subaccount_summary(
        &self,
        deployment_id: u64,
        subkey: &str,
    ) -> Result<Option<SubaccountSummaryRecord>, ReadStoreError> {
        match self.snapshot(deployment_id)? {
            Some(s) => s.get_subaccount_summary(deployment_id, subkey).await,
            None => Ok(None),
        }
    }

    async fn list_collateral(
        &self,
        deployment_id: u64,
        subkey: &str,
    ) -> Result<Vec<CollateralRecord>, ReadStoreError> {
        match self.snapshot(deployment_id)? {
            Some(s) => s.list_collateral(deployment_id, subkey).await,
            None => Ok(Vec::new()),
        }
    }

    async fn list_reservations(
        &self,
        deployment_id: u64,
        subkey: &str,
    ) -> Result<Vec<ReservationRecord>, ReadStoreError> {
        match self.snapshot(deployment_id)? {
            Some(s) => s.list_reservations(deployment_id, subkey).await,
            None => Ok(Vec::new()),
        }
    }

    async fn list_positions(
        &self,
        deployment_id: u64,
        subkey: &str,
        page: &PageAnchor,
    ) -> Result<StorePage<PositionRecord>, ReadStoreError> {
        match self.snapshot(deployment_id)? {
            Some(s) => s.list_positions(deployment_id, subkey, page).await,
            None => Ok(StorePage {
                items: Vec::new(),
                next_anchor: None,
            }),
        }
    }

    async fn list_orders(
        &self,
        deployment_id: u64,
        subkey: &str,
        page: &PageAnchor,
    ) -> Result<StorePage<OrderLifecycleRecord>, ReadStoreError> {
        match self.snapshot(deployment_id)? {
            Some(s) => s.list_orders(deployment_id, subkey, page).await,
            None => Ok(StorePage {
                items: Vec::new(),
                next_anchor: None,
            }),
        }
    }

    async fn get_order(
        &self,
        deployment_id: u64,
        order_hash: &str,
    ) -> Result<Option<OrderLifecycleRecord>, ReadStoreError> {
        match self.snapshot(deployment_id)? {
            Some(s) => s.get_order(deployment_id, order_hash).await,
            None => Ok(None),
        }
    }

    async fn list_completed_executions(
        &self,
        deployment_id: u64,
        subkey: Option<&str>,
        page: &PageAnchor,
    ) -> Result<StorePage<MatchedExecutionRecord>, ReadStoreError> {
        match self.snapshot(deployment_id)? {
            Some(s) => {
                s.list_completed_executions(deployment_id, subkey, page)
                    .await
            }
            None => Ok(StorePage {
                items: Vec::new(),
                next_anchor: None,
            }),
        }
    }

    async fn list_fees(
        &self,
        deployment_id: u64,
        subkey: &str,
        page: &PageAnchor,
    ) -> Result<StorePage<FeeRebateRecord>, ReadStoreError> {
        match self.snapshot(deployment_id)? {
            Some(s) => s.list_fees(deployment_id, subkey, page).await,
            None => Ok(StorePage {
                items: Vec::new(),
                next_anchor: None,
            }),
        }
    }

    async fn get_recovery(
        &self,
        deployment_id: u64,
        subkey: &str,
    ) -> Result<Option<RecoveryRecord>, ReadStoreError> {
        match self.snapshot(deployment_id)? {
            Some(s) => s.get_recovery(deployment_id, subkey).await,
            None => Ok(None),
        }
    }

    async fn query_history(
        &self,
        deployment_id: u64,
        scope: &HistoryScope,
        filter: &HistoryFilter,
        page: &HistoryPageAnchor,
    ) -> Result<HistoryPage, ReadStoreError> {
        let Some(rt_lock) = self.runtime(deployment_id) else {
            return Ok(HistoryPage {
                items: Vec::new(),
                next_anchor: None,
            });
        };
        let rt = rt_lock.read().map_err(|_| ReadStoreError::Backend {
            detail: "runtime lock poisoned".into(),
        })?;

        // Compose the effective filter by merging scope into the caller's
        // filter — the store-boundary contract has scope + filter as
        // separate inputs so different routes (global / owner / subaccount)
        // stay type-tagged, but the runtime-side `build_history` collapses
        // them into a single `HistoryFilter`.
        let mut eff_filter = filter.clone();
        match scope {
            HistoryScope::Global => {}
            HistoryScope::Owner { owner } => {
                eff_filter.owner = Some(owner.clone());
            }
            HistoryScope::Subaccount { subkey } => {
                eff_filter.subkey = Some(subkey.clone());
            }
        }

        // Cursor filter/consistency binding — reject filter drift so the
        // runtime path fails closed the same way the Postgres path does.
        let expected_hash = filter_stable_hash(&eff_filter);
        if page.filter_hash != expected_hash {
            return Err(ReadStoreError::InvalidCursor {
                detail: "filter hash does not match cursor binding".into(),
            });
        }
        // Under indexed consistency, verify the anchor's head hash still
        // matches the runtime's canonical head. Empty means "first page"
        // sentinels which we accept.
        if page.consistency == HistoryConsistency::Indexed
            && !page.indexed_head_hash.is_empty()
            && !rt
                .cursor()
                .indexed_head_hash
                .eq_ignore_ascii_case(&page.indexed_head_hash)
        {
            return Err(ReadStoreError::StaleCursor {
                expected_hash: page.indexed_head_hash.clone(),
                actual_hash: rt.cursor().indexed_head_hash.clone(),
            });
        }

        let finalized_block = rt.metrics().finalized_block;
        let mut events = build_history(&rt, rt.manifest.chain_id, finalized_block, &eff_filter);
        // Canonical descending order by (block, tx_index, log_index, event_id).
        events.sort_by(|a, b| {
            (b.block_number, b.tx_index, b.log_index, &b.event_id).cmp(&(
                a.block_number,
                a.tx_index,
                a.log_index,
                &a.event_id,
            ))
        });

        let mut skipping = page.after.is_some();
        let mut out: Vec<HistoryRecord> = Vec::with_capacity(page.limit);
        for ev in events {
            if skipping {
                if let Some(after) = page.after.as_ref() {
                    if ev.block_number == after.block_number
                        && ev.tx_index == after.tx_index
                        && ev.log_index == after.log_index
                        && ev.event_id == after.event_id
                    {
                        skipping = false;
                    }
                }
                continue;
            }
            let rec = history_event_to_record(ev);
            if out.len() >= page.limit {
                break;
            }
            out.push(rec);
        }

        let next_anchor = if out.len() >= page.limit {
            out.last().map(|last| HistoryPageAnchor {
                limit: page.limit,
                after: Some(HistoryCursorKey {
                    block_number: last.block_number,
                    tx_index: last.tx_index,
                    log_index: last.log_index,
                    event_id: last.event_id.clone(),
                }),
                consistency: page.consistency,
                filter_hash: page.filter_hash.clone(),
                indexed_head_hash: page.indexed_head_hash.clone(),
            })
        } else {
            None
        };

        Ok(HistoryPage {
            items: out,
            next_anchor,
        })
    }
}
